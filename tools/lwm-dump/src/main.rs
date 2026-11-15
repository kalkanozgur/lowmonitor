//! lwm-dump: `.lwm` segmentini doğrular ve CSV'ye döker (geliştirme / debug).
//!
//! - `lwm-dump <segment.lwm> [--key <key.bin>]`: dosyadan. Anahtar varsayılanı `logs/`'un üstündeki `key.bin`.
//!   Yalnızca çalıştıran hesabın DPAPI anahtarı çözülür (ADR-0016): `run` verisi evet, servis verisi hayır.
//! - `lwm-dump --export <YYYY-MM-DD> [<YYYY-MM-DD>]`: servis loglarını pipe `Export` ile (UTC gün aralığı,
//!   dahil). Yükseltilmiş (admin) oturum gerekir.
//!
//! CSV stdout'a, özet stderr'e. Çıkış: 0 sağlam, 1 bütünlük hatası, 2 kullanım/IO, 3 anahtar yok / yetki yok.
use lowmon_core::{
    crypto::FLAG_DOWNSAMPLED,
    csv,
    dpapi::load_key,
    proto::{self, ClientMsg, ErrorCode, PIPE_NAME, PROTO, Role, ServerMsg},
    time::{DAY_MS, parse_day},
};
use std::{
    fs::OpenOptions,
    io::{self, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "kullanım: lwm-dump <segment.lwm> [--key <key.bin>]\n          lwm-dump --export <YYYY-MM-DD> [<YYYY-MM-DD>]";

fn file_mode(seg: PathBuf, key: Option<PathBuf>) -> ExitCode {
    let Some(key_path) = key else {
        eprintln!("key.bin konumu çıkarılamadı; --key verin");
        return ExitCode::from(2);
    };
    let master = match load_key(&key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!(
                "anahtar okunamadı ({}): {e}\nservis anahtarı bu hesapla çözülemez (ADR-0016); servis logları için --export",
                key_path.display()
            );
            return ExitCode::from(3);
        }
    };
    let bytes = match std::fs::read(&seg) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", seg.display());
            return ExitCode::from(2);
        }
    };
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let r = writeln!(out, "{}", csv::HEADER)
        .and_then(|_| csv::segment(&master, &bytes, 0, u64::MAX, &mut out))
        .and_then(|o| out.flush().map(|_| o));
    match r {
        Ok(o) => {
            if let Some(h) = o.header {
                eprintln!(
                    "segment {} | oluşturma {} | açılış aralığı {} ms | {}", // ADR-0027: gün içinde değişebilir
                    h.segment_id,
                    h.created_unix,
                    h.interval_ms,
                    if h.flags & FLAG_DOWNSAMPLED != 0 {
                        "dakikalık özet"
                    } else {
                        "ham"
                    }
                );
            }
            match o.error {
                None => {
                    eprintln!("doğrulandı: {} blok, zincir sağlam", o.blocks);
                    ExitCode::SUCCESS
                }
                Some(err) => {
                    eprintln!("BÜTÜNLÜK HATASI: {err} (öncesindeki satırlar doğrulanmış veridir)");
                    ExitCode::from(1)
                }
            }
        }
        Err(e) => {
            eprintln!("yazma hatası: {e}");
            ExitCode::from(2)
        }
    }
}

fn export(from_ms: u64, to_ms: u64) -> io::Result<ExitCode> {
    // GENERIC_READ | FILE_WRITE_DATA: pipe DACL'ı FILE_CREATE_PIPE_INSTANCE vermez (ADR-0018)
    let mut pipe = OpenOptions::new()
        .access_mode(0x8000_0000 | 0x2)
        .open(PIPE_NAME)?;
    proto::write(
        &mut pipe,
        &ClientMsg::Hello {
            proto: PROTO,
            role: Role::Viewer,
        },
    )?;
    proto::write(&mut pipe, &ClientMsg::Export { from_ms, to_ms })?;
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut acks = 0;
    loop {
        match proto::read::<ServerMsg>(&mut pipe)? {
            ServerMsg::RangeChunk(b) => out.write_all(&b)?,
            // 1. Ack = Hello, 2. Ack = Export bitti
            ServerMsg::Ack => {
                acks += 1;
                if acks == 2 {
                    out.flush()?;
                    eprintln!("export tamam");
                    return Ok(ExitCode::SUCCESS);
                }
            }
            ServerMsg::Error { code } => {
                out.flush()?;
                eprintln!("servis hatası: {code:?}");
                return Ok(ExitCode::from(if code == ErrorCode::Forbidden {
                    3
                } else {
                    1
                }));
            }
            _ => {}
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, from, rest @ ..] if flag == "--export" && rest.len() <= 1 => {
            let to = rest.first().unwrap_or(from);
            let (Some(a), Some(b)) = (parse_day(from), parse_day(to)) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            match export(a * DAY_MS, (b + 1) * DAY_MS) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("pipe: {e} (servis çalışıyor mu?)");
                    ExitCode::from(2)
                }
            }
        }
        [seg] => {
            let seg = PathBuf::from(seg);
            let key = seg
                .parent()
                .and_then(Path::parent)
                .map(|r| r.join("key.bin"));
            file_mode(seg, key)
        }
        [seg, flag, key] if flag == "--key" => {
            file_mode(PathBuf::from(seg), Some(PathBuf::from(key)))
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
