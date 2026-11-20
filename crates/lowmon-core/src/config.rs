//! `%ProgramData%\LowMonitor\config.toml` (spek §7, alarmlar ADR-0028). Bilinmeyen alanlar yok sayılır.
use serde::Deserialize;
use std::collections::HashSet;

/// En fazla kural sayısı (ADR-0028: 32 kuralda < 5 µs/tick).
pub const MAX_ALARMS: usize = 32;

/// Kurulumda yazılan varsayılan dosya (spek §7).
pub const DEFAULT_TOML: &str = r#"# LowMonitor ayarları — değişiklik servis yeniden başlayınca geçerli olur
[sampling]
interval_ms = 1000
topn_every  = 5
topn_count  = 5

[retention]
raw_days    = 7
total_days  = 30

# UAC istemi (consent.exe) açılınca "uac" alarmı (ETW, ADR-0037); varsayılan kapalı
[events]
uac = false

[[alarm]]
id       = "cpu_high"
metric   = "cpu.total"
raise    = { above = 90.0, for_s = 30 }
clear    = { below = 80.0, for_s = 10 }

[[alarm]]
id       = "ram_high"
metric   = "mem.used_pct"
raise    = { above = 90.0, for_s = 60 }
clear    = { below = 85.0, for_s = 30 }
"#;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sampling: Sampling,
    pub retention: Retention,
    pub events: Events,
    /// `[[alarm]]` tabloları; dosya yoksa boş.
    pub alarm: Vec<AlarmRule>,
}

/// Alarm kuralı (ADR-0028).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlarmRule {
    pub id: String,
    pub metric: Metric,
    /// Disk: PhysicalDrive numarası; ağ: arayüz takma adı. Yoksa toplam.
    #[serde(default)]
    pub device: Option<String>,
    pub raise: Cond,
    pub clear: Cond,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Metric {
    #[serde(rename = "cpu.total")]
    CpuTotal,
    #[serde(rename = "mem.used_pct")]
    MemUsedPct,
    #[serde(rename = "mem.commit_pct")]
    MemCommitPct,
    #[serde(rename = "disk.r_bps")]
    DiskR,
    #[serde(rename = "disk.w_bps")]
    DiskW,
    #[serde(rename = "disk.bps")]
    DiskRw,
    #[serde(rename = "net.rx_bps")]
    NetRx,
    #[serde(rename = "net.tx_bps")]
    NetTx,
    #[serde(rename = "net.bps")]
    NetRxTx,
}

impl Metric {
    fn takes_device(self) -> bool {
        !matches!(self, Self::CpuTotal | Self::MemUsedPct | Self::MemCommitPct)
    }
}

/// Eşik: `above` ya da `below` (tam biri) + süre.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cond {
    #[serde(default)]
    pub above: Option<f64>,
    #[serde(default)]
    pub below: Option<f64>,
    #[serde(default)]
    pub for_s: u32,
}

impl Cond {
    pub fn hit(&self, v: f64) -> bool {
        self.above.is_some_and(|a| v > a) || self.below.is_some_and(|b| v < b)
    }
}

/// Kural doğrulaması; hata metni kullanıcıya gösterilebilir.
fn check_alarm(r: &AlarmRule) -> Result<(), String> {
    let id = &r.id;
    let id_ok = (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !id_ok {
        return Err(format!("alarm id \"{id}\": [a-z0-9_]{{1,32}} olmalı"));
    }
    if r.device.is_some() && !r.metric.takes_device() {
        return Err(format!("alarm {id}: bu metrik device almaz"));
    }
    let one = |c: &Cond| c.above.is_some() != c.below.is_some();
    if !one(&r.raise) || !one(&r.clear) {
        return Err(format!(
            "alarm {id}: raise/clear tam bir above ya da below içermeli"
        ));
    }
    if r.raise.for_s > 3600 || r.clear.for_s > 3600 {
        return Err(format!("alarm {id}: for_s 0..=3600 olmalı"));
    }
    // Histerezis: clear ters yönde ve raise eşiğini geçmeyen bir eşik olmalı
    let hysteresis = match (r.raise.above, r.raise.below, r.clear.above, r.clear.below) {
        (Some(ra), None, None, Some(cb)) => cb <= ra,
        (None, Some(rb), Some(ca), None) => ca >= rb,
        _ => false,
    };
    if !hysteresis {
        return Err(format!(
            "alarm {id}: clear ters yönde olmalı (raise.above → clear.below ≤ above)"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Sampling {
    pub interval_ms: u32,
    pub topn_every: u32,
    pub topn_count: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Retention {
    pub raw_days: u32,
    pub total_days: u32,
}

/// Opsiyonel olay kaynakları; hepsi kapalı başlar (kapalıyken thread/oturum açılmaz).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Events {
    pub uac: bool,
}

impl Default for Sampling {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            topn_every: 5,
            topn_count: 5,
        }
    }
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            raw_days: 7,
            total_days: 30,
        }
    }
}

impl Config {
    /// Ayrıştır + sınır kontrolü. Hata metni kullanıcıya gösterilebilir.
    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        let s = &c.sampling;
        let r = &c.retention;
        let checks = [
            (
                (100..=60_000).contains(&s.interval_ms),
                "sampling.interval_ms 100..=60000 olmalı",
            ),
            (
                (1..=3600).contains(&s.topn_every),
                "sampling.topn_every 1..=3600 olmalı",
            ),
            (s.topn_count <= 50, "sampling.topn_count <= 50 olmalı"),
            (
                r.raw_days >= 1 && r.raw_days <= r.total_days,
                "retention: 1 <= raw_days <= total_days olmalı",
            ),
            (r.total_days <= 3650, "retention.total_days <= 3650 olmalı"),
        ];
        if let Some((_, msg)) = checks.iter().find(|(ok, _)| !ok) {
            return Err((*msg).to_owned());
        }
        if c.alarm.len() > MAX_ALARMS {
            return Err(format!("en fazla {MAX_ALARMS} alarm"));
        }
        let mut ids = HashSet::new();
        for r in &c.alarm {
            check_alarm(r)?;
            if !ids.insert(r.id.as_str()) {
                return Err(format!("alarm id \"{}\" tekrar ediyor", r.id));
            }
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_to_defaults() {
        let c = Config::parse(DEFAULT_TOML).unwrap();
        assert_eq!(
            (&c.sampling, &c.retention),
            (&Sampling::default(), &Retention::default())
        );
        let ids: Vec<_> = c.alarm.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["cpu_high", "ram_high"]);
        assert!(Config::default().alarm.is_empty()); // dosya yoksa alarm yok
        assert!(!c.events.uac && !Config::default().events.uac);
        assert!(
            Config::parse(
                "[events]
uac = true
"
            )
            .unwrap()
            .events
            .uac
        );
    }

    fn alarm(body: &str) -> Result<Config, String> {
        Config::parse(&format!("[[alarm]]\n{body}\n"))
    }

    #[test]
    fn alarm_device_and_below_parse() {
        let c = alarm(
            "id = \"wan\"\nmetric = \"net.rx_bps\"\ndevice = \"Ethernet 2\"\n\
             raise = { above = 50_000_000, for_s = 20 }\nclear = { below = 10_000_000 }",
        )
        .unwrap();
        let r = &c.alarm[0];
        assert_eq!(
            (r.metric, r.device.as_deref()),
            (Metric::NetRx, Some("Ethernet 2"))
        );
        assert_eq!(r.clear.for_s, 0);
        // ters yön: düşük değer alarmı
        assert!(alarm(
            "id = \"lo\"\nmetric = \"cpu.total\"\nraise = { below = 5.0 }\nclear = { above = 10.0 }"
        )
        .is_ok());
    }

    #[test]
    fn rejects_bad_alarms() {
        let ok_conds = "raise = { above = 90.0 }\nclear = { below = 80.0 }";
        for (body, why) in [
            (format!("id = \"Cpu\"\nmetric = \"cpu.total\"\n{ok_conds}"), "büyük harf"),
            (format!("id = \"a\"\nmetric = \"gpu.load\"\n{ok_conds}"), "bilinmeyen metrik"),
            (
                format!("id = \"a\"\nmetric = \"cpu.total\"\ndevice = \"0\"\n{ok_conds}"),
                "device almaz",
            ),
            (
                "id = \"a\"\nmetric = \"cpu.total\"\nraise = { above = 90.0 }\nclear = { below = 95.0 }"
                    .into(),
                "histerezis ters",
            ),
            (
                "id = \"a\"\nmetric = \"cpu.total\"\nraise = { above = 90.0 }\nclear = { above = 80.0 }"
                    .into(),
                "aynı yön",
            ),
            (
                "id = \"a\"\nmetric = \"cpu.total\"\nraise = { above = 90.0, below = 1.0 }\nclear = { below = 80.0 }"
                    .into(),
                "iki eşik",
            ),
            (
                "id = \"a\"\nmetric = \"cpu.total\"\nraise = { above = 90.0, for_s = 4000 }\nclear = { below = 80.0 }"
                    .into(),
                "for_s",
            ),
            (format!("id = \"a\"\nmetric = \"cpu.total\"\nfoo = 1\n{ok_conds}"), "bilinmeyen alan"),
        ] {
            assert!(alarm(&body).is_err(), "{why} kabul edildi");
        }
        let dup = format!(
            "[[alarm]]\nid = \"a\"\nmetric = \"cpu.total\"\n{ok_conds}\n[[alarm]]\nid = \"a\"\nmetric = \"mem.used_pct\"\n{ok_conds}\n"
        );
        assert!(Config::parse(&dup).is_err(), "tekrar eden id");
    }

    #[test]
    fn partial_file_fills_defaults() {
        let c = Config::parse("[sampling]\ntopn_every = 10\n").unwrap();
        assert_eq!(c.sampling.topn_every, 10);
        assert_eq!(c.sampling.interval_ms, 1000);
        assert_eq!(c.retention, Retention::default());
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("[sampling]\ninterval_ms = 5\n").is_err());
        assert!(Config::parse("[retention]\nraw_days = 40\n").is_err());
        assert!(Config::parse("[sampling\n").is_err());
    }
}
