//! `/v1/live` mesajı (spek §16.3): minified JSON, sabit şema `v = 1`.
use lowmon_proto::metrics::{Sample, TopEntry};
use serde::{Deserialize, Serialize};

/// İstemcinin seçebileceği gönderim aralıkları (spek §16.3).
pub const RATES_MS: [u32; 3] = [1000, 2000, 5000];

#[derive(Serialize)]
struct Live<'a> {
    v: u8,
    ts: u64,
    cpu: f32,
    cores: Vec<u8>,
    ram: Ram,
    disk: Disk,
    net: Net,
    /// `lowmon-gpu` gelene kadar null.
    gpu: Option<()>,
    top: Vec<(&'a str, f32)>,
    /// shortcut: alarm motoru M5'te; o zamana kadar hep boş
    alarms: [&'a str; 0],
}

#[derive(Serialize)]
struct Ram {
    used_mb: u64,
    total_mb: u64,
}

#[derive(Serialize)]
struct Disk {
    r_bps: u64,
    w_bps: u64,
}

#[derive(Serialize)]
struct Net {
    rx_bps: u64,
    tx_bps: u64,
}

/// 0.01 % birimi → bir ondalıklı yüzde.
fn pct(units: u16) -> f32 {
    (units as f32 / 10.0).round() / 10.0
}

pub fn live_json(s: &Sample, top: &[TopEntry], total_mb: u64) -> String {
    let live = Live {
        v: 1,
        ts: s.ts_unix_ms / 1000,
        cpu: pct(s.cpu_total),
        cores: s.cpu_core.iter().map(|&c| c / 2).collect(), // 0.5 % birimi → tam yüzde
        ram: Ram {
            used_mb: s.ram_used_kb / 1024,
            total_mb,
        },
        disk: Disk {
            r_bps: s.disk.iter().map(|d| d.r_bps).sum(),
            w_bps: s.disk.iter().map(|d| d.w_bps).sum(),
        },
        net: Net {
            rx_bps: s.net.iter().map(|n| n.rx_bps).sum(),
            tx_bps: s.net.iter().map(|n| n.tx_bps).sum(),
        },
        gpu: None,
        top: top.iter().map(|t| (t.name.as_str(), pct(t.cpu))).collect(),
        alarms: [],
    };
    serde_json::to_string(&live).unwrap_or_default()
}

/// İstemci → sunucu tek mesaj: `{"rate_ms": 1000 | 2000 | 5000}`. Başka her şey yok sayılır.
pub fn parse_rate(text: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct Rate {
        rate_ms: u32,
    }
    let r: Rate = serde_json::from_str(text).ok()?;
    RATES_MS.contains(&r.rate_ms).then_some(r.rate_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowmon_proto::metrics::{DiskIo, NetIo};

    #[test]
    fn json_matches_spec_shape() {
        let s = Sample {
            ts_unix_ms: 1_763_128_800_123,
            cpu_total: 1237,
            cpu_core: vec![16, 31],
            ram_used_kb: 9300 * 1024,
            disk: vec![
                DiskIo {
                    drive: 0,
                    r_bps: 100,
                    w_bps: 4,
                },
                DiskIo {
                    drive: 1,
                    r_bps: 20,
                    w_bps: 1,
                },
            ],
            net: vec![NetIo {
                if_index: 3,
                rx_bps: 7,
                tx_bps: 8,
            }],
            ..Default::default()
        };
        let top = [TopEntry {
            pid: 1,
            cpu: 410,
            ws_kb: 1,
            name: "a\"b.exe".into(),
        }];
        assert_eq!(
            live_json(&s, &top, 16384),
            r#"{"v":1,"ts":1763128800,"cpu":12.4,"cores":[8,15],"ram":{"used_mb":9300,"total_mb":16384},"disk":{"r_bps":120,"w_bps":5},"net":{"rx_bps":7,"tx_bps":8},"gpu":null,"top":[["a\"b.exe",4.1]],"alarms":[]}"#
        );
    }

    #[test]
    fn rate_accepts_only_listed_values() {
        assert_eq!(parse_rate(r#"{"rate_ms":5000}"#), Some(5000));
        assert_eq!(parse_rate(r#"{"rate_ms":10}"#), None);
        assert_eq!(parse_rate("rate"), None);
    }
}
