//! Pipe protokolü (spek §6): çerçeve = `u32 LE uzunluk | postcard`, maks. 1 MB.
//! shortcut: Producer yok; Control'de SetInterval/SetRetention yok (ADR-0027: config + ReloadConfig); Export yalnızca CSV, format alanı yok
//! Yeni varyantlar enum sonuna eklenir → postcard indeksleri değişmez, `PROTO` artmaz.
use crate::metrics::{AggKind, EventKind, GapReason, Sample, TopEntry};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

pub const PIPE_NAME: &str = r"\\.\pipe\LowMonitor";
pub const PROTO: u16 = 1;
pub const MAX_FRAME: usize = 1 << 20;
/// `Subscribe.channels` bitleri.
pub const CH_SAMPLE: u8 = 1;
pub const CH_TOP: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Viewer,
    Tray,
    Producer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello {
        proto: u16,
        role: Role,
    },
    /// `rate_ms` yalnızca Sample kanalına uygulanır; Top üretildikçe gider.
    Subscribe {
        channels: u8,
        rate_ms: u32,
    },
    Unsubscribe,
    /// Admin. `[from_ms, to_ms)` CSV olarak `RangeChunk` ile akar, `Ack` ile biter.
    Export {
        from_ms: u64,
        to_ms: u64,
    },
    /// Herkes. `[from_ms, to_ms)` metrikleri `Range` çerçeveleriyle akar, `Ack` ile biter. Top-N dönmez.
    QueryRange {
        from_ms: u64,
        to_ms: u64,
        resolution: Resolution,
    },
    /// Admin. Toplayıcı döngüsü ≤ 1 tick içinde uygular; `Ack` ya da `Error`.
    Control(Control),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    /// Toplama/yazma/yayın durur (kalıcı değil: servis yeniden başlarsa devam eder).
    Pause,
    /// Log'a `Gap(Paused)` yazılır, ilk örnek atılır.
    Resume,
    /// `config.toml` yeniden okunur; `interval_ms` dahil hepsi çalışırken uygulanır (ADR-0027).
    /// Dosya okunamaz/hatalıysa `Io`, mevcut ayar kalır. Ayar değişikliği = Admin dosyayı yazar + bu komut.
    ReloadConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// Dosyada ne varsa: ham günlerde 1 s `Sample`, downsample edilmiş günlerde `Agg`.
    Raw,
    /// Dakika başına min/avg/max `Agg`; ham günlerde serviste anında hesaplanır.
    Minute,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RangeItem {
    Sample(Sample),
    /// `sample.ts_unix_ms` = dakika başı.
    Agg {
        kind: AggKind,
        sample: Sample,
    },
    /// Doğrulanamayan segment kuyruğu da `Integrity` gap olarak gelir.
    Gap {
        from_ms: u64,
        to_ms: u64,
        reason: GapReason,
    },
    /// Alarm / sistem olayı (ADR-0028). `ts_ms`: önceki örneğin zamanı (özet günlerde dakika sonu).
    Event {
        ts_ms: u64,
        kind: EventKind,
        name: String,
        value: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// İlk mesaj Hello değil veya sürüm uyuşmuyor.
    BadHello,
    Unsupported,
    /// Komut yükseltilmiş Administrators üyeliği ister.
    Forbidden,
    /// Sunucu tarafında okuma/yazma hatası.
    Io,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    Sample(Sample),
    Top(Vec<TopEntry>),
    /// En fazla 64 KB.
    RangeChunk(Vec<u8>),
    Ack,
    /// `QueryRange` sonucu; öğelerin kodlanmış toplamı ≤ 64 KB.
    Range(Vec<RangeItem>),
    Error {
        code: ErrorCode,
    },
}

fn invalid(e: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

/// Mesaj → tam çerçeve (uzunluk önekiyle).
pub fn encode<T: Serialize>(msg: &T) -> io::Result<Vec<u8>> {
    let mut buf = postcard::to_extend(msg, vec![0u8; 4]).map_err(invalid)?;
    let n = u32::try_from(buf.len() - 4).map_err(invalid)?;
    buf[..4].copy_from_slice(&n.to_le_bytes());
    Ok(buf)
}

pub fn write<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    w.write_all(&encode(msg)?)
}

/// Bir çerçeve okur; 1 MB'ı aşan uzunluk okunmadan reddedilir (bellek şişirme yok).
pub fn read<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(invalid(format!("çerçeve çok büyük: {n} B")));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    postcard::from_bytes(&buf).map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{DiskIo, NetIo};

    #[test]
    fn roundtrip() {
        let s = Sample {
            ts_unix_ms: 1_700_000_000_000,
            cpu_total: 1234,
            cpu_core: vec![1, 200],
            ram_used_kb: 8_000_000,
            disk: vec![DiskIo {
                drive: 0,
                r_bps: 5,
                w_bps: 6,
            }],
            net: vec![NetIo {
                if_index: 3,
                rx_bps: 7,
                tx_bps: 8,
            }],
            ..Default::default()
        };
        let mut wire = Vec::new();
        write(&mut wire, &ServerMsg::Sample(s.clone())).unwrap();
        write(&mut wire, &ClientMsg::Unsubscribe).unwrap();
        let mut r = &wire[..];
        assert_eq!(read::<ServerMsg>(&mut r).unwrap(), ServerMsg::Sample(s));
        assert_eq!(read::<ClientMsg>(&mut r).unwrap(), ClientMsg::Unsubscribe);
        assert!(r.is_empty());
    }

    #[test]
    fn rejects_oversize_and_garbage() {
        let big = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert!(read::<ClientMsg>(&mut &big[..]).is_err());
        let junk = [3, 0, 0, 0, 0xff, 0xff, 0xff];
        assert!(read::<ClientMsg>(&mut &junk[..]).is_err());
    }
}
