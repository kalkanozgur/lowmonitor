//! lwm-dump: `.lwm` segmentini doğrular ve CSV'ye döker (geliştirme / debug).
//!
//! Kullanım: `lwm-dump <segment.lwm> [--key <key.bin>]` → CSV stdout'a, özet stderr'e.
//! Anahtar varsayılanı: segmentin `logs/` klasörünün üstündeki `key.bin`.
//! Yalnızca çalıştıran hesabın DPAPI anahtarı çözülebilir (ADR-0016): `run` modu verisi evet,
//! servis verisi hayır (servis logları M3'te pipe `Export` ile).
use lowmon_core::{
    codec::{AggKind, BlockDecoder, Record},
    crypto::{FLAG_DOWNSAMPLED, MasterKey, Opener},
    dpapi::load_key,
    metrics::Sample,
};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const CSV_HEADER: &str = "type,ts_unix_ms,cpu_pct,ram_used_kb,commit_kb,cache_kb,disk_r_bps,disk_w_bps,net_rx_bps,net_tx_bps,cores,info";

fn sample_row(kind: &str, s: &Sample) -> String {
    let cores: Vec<String> = s
        .cpu_core
        .iter()
        .map(|&c| format!("{:.1}", c as f32 / 2.0))
        .collect();
    format!(
        "{kind},{},{:.2},{},{},{},{},{},{},{},{},",
        s.ts_unix_ms,
        s.cpu_total as f32 / 100.0,
        s.ram_used_kb,
        s.commit_kb,
        s.cache_kb,
        s.disk.iter().map(|d| d.r_bps).sum::<u64>(),
        s.disk.iter().map(|d| d.w_bps).sum::<u64>(),
        s.net.iter().map(|n| n.rx_bps).sum::<u64>(),
        s.net.iter().map(|n| n.tx_bps).sum::<u64>(),
        cores.join(" "),
    )
}

/// Sample/Agg dışındaki kayıtlar: ts + `info` sütunu.
fn other_row(kind: &str, ts: u64, info: &str) -> String {
    format!("{kind},{ts},,,,,,,,,,{}", info.replace(',', ";"))
}

/// Dosyayı baştan doğrular, CSV yazar. Dönüş: (blok sayısı, hata metni).
fn dump(
    master: &MasterKey,
    bytes: &[u8],
    out: &mut impl Write,
) -> io::Result<(u64, Option<String>)> {
    let (header, mut opener) = match Opener::new(master, bytes) {
        Ok(x) => x,
        Err(e) => return Ok((0, Some(format!("başlık: {e:?}")))),
    };
    eprintln!(
        "segment {} | oluşturma {} | aralık {} ms | {}",
        header.segment_id,
        header.created_unix,
        header.interval_ms,
        if header.flags & FLAG_DOWNSAMPLED != 0 {
            "dakikalık özet"
        } else {
            "ham"
        }
    );
    writeln!(out, "{CSV_HEADER}")?;
    let (mut names, mut last_ts) = (Vec::new(), 0);
    for block in opener.by_ref() {
        let block = match block {
            Ok(b) => b,
            Err(e) => return Ok((0, Some(format!("blok doğrulama: {e:?}")))),
        };
        for rec in BlockDecoder::new(&block, &mut names) {
            let row = match rec {
                Err(e) => return Ok((0, Some(format!("kayıt çözme: {e:?}")))),
                Ok(Record::Sample(s)) => {
                    last_ts = s.ts_unix_ms;
                    sample_row("sample", &s)
                }
                Ok(Record::Agg { kind, sample }) => {
                    last_ts = sample.ts_unix_ms;
                    let k = match kind {
                        AggKind::Min => "min",
                        AggKind::Avg => "avg",
                        AggKind::Max => "max",
                    };
                    sample_row(k, &sample)
                }
                Ok(Record::Top(t)) => {
                    let info: Vec<String> = t
                        .iter()
                        .map(|e| {
                            format!(
                                "{}:{}:{:.2}%:{}KB",
                                e.name,
                                e.pid,
                                e.cpu as f32 / 100.0,
                                e.ws_kb
                            )
                        })
                        .collect();
                    other_row("top", last_ts, &info.join(" "))
                }
                Ok(Record::Gap {
                    from_ms,
                    to_ms,
                    reason,
                }) => other_row("gap", from_ms, &format!("to={to_ms} reason={reason:?}")),
                Ok(Record::Seal {
                    block_count,
                    sample_count,
                    ..
                }) => other_row(
                    "seal",
                    last_ts,
                    &format!("blocks={block_count} samples={sample_count}"),
                ),
            };
            writeln!(out, "{row}")?;
        }
    }
    Ok((opener.head().0, None))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (seg, key) = match args.as_slice() {
        [seg] => {
            let seg = PathBuf::from(seg);
            let key = seg
                .parent()
                .and_then(Path::parent)
                .map(|r| r.join("key.bin"));
            (seg, key)
        }
        [seg, flag, key] if flag == "--key" => (PathBuf::from(seg), Some(PathBuf::from(key))),
        _ => {
            eprintln!("kullanım: lwm-dump <segment.lwm> [--key <key.bin>]");
            return ExitCode::from(2);
        }
    };
    let Some(key_path) = key else {
        eprintln!("key.bin konumu çıkarılamadı; --key verin");
        return ExitCode::from(2);
    };
    let master = match load_key(&key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!(
                "anahtar okunamadı ({}): {e}\nservis anahtarı bu hesapla çözülemez (ADR-0016)",
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
    match dump(&master, &bytes, &mut out).and_then(|r| out.flush().map(|_| r)) {
        Ok((blocks, None)) => {
            eprintln!("doğrulandı: {blocks} blok, zincir sağlam");
            ExitCode::SUCCESS
        }
        Ok((_, Some(err))) => {
            eprintln!("BÜTÜNLÜK HATASI: {err} (öncesindeki satırlar doğrulanmış veridir)");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("yazma hatası: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowmon_core::{
        codec::{BlockEncoder, Dict, GapReason},
        crypto::{Header, Sealer},
        metrics::TopEntry,
    };

    const KEY: MasterKey = [1; 32];

    fn segment() -> Vec<u8> {
        let h = Header::new(1, 0, 1000, 1).unwrap();
        let mut file = h.to_bytes().to_vec();
        let (mut s, mut enc, mut dict) = (
            Sealer::new(&KEY, &h),
            BlockEncoder::default(),
            Dict::default(),
        );
        enc.sample(&Sample {
            ts_unix_ms: 1000,
            cpu_total: 1234,
            cpu_core: vec![3, 200],
            ..Default::default()
        });
        enc.top(
            &[TopEntry {
                pid: 7,
                cpu: 50,
                ws_kb: 9,
                name: "a,b.exe".into(),
            }],
            &mut dict,
        );
        enc.gap(1000, 5000, GapReason::Sleep);
        s.seal(&enc.take(), &mut file).unwrap();
        file
    }

    #[test]
    fn csv_rows() {
        let mut out = Vec::new();
        let (blocks, err) = dump(&KEY, &segment(), &mut out).unwrap();
        assert_eq!((blocks, err), (1, None));
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(lines[1], "sample,1000,12.34,0,0,0,0,0,0,0,1.5 100.0,");
        assert_eq!(lines[2], "top,1000,,,,,,,,,,a;b.exe:7:0.50%:9KB"); // virgül CSV'yi bozmaz
        assert_eq!(lines[3], "gap,1000,,,,,,,,,,to=5000 reason=Sleep");
        assert!(lines.iter().all(|l| l.split(',').count() == 12));
    }

    #[test]
    fn tamper_is_reported() {
        let mut seg = segment();
        let n = seg.len();
        seg[n - 1] ^= 1;
        let (_, err) = dump(&KEY, &seg, &mut Vec::new()).unwrap();
        assert!(err.unwrap().contains("Auth"));
    }
}
