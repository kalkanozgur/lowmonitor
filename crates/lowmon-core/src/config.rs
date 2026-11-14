//! `%ProgramData%\LowMonitor\config.toml` (spek §7). Bilinmeyen alanlar (ör. `[[alarm]]`, M5) yok sayılır.
use serde::Deserialize;

/// Kurulumda yazılan varsayılan dosya (spek §7).
pub const DEFAULT_TOML: &str = r#"# LowMonitor ayarları — değişiklik servis yeniden başlayınca geçerli olur
[sampling]
interval_ms = 1000
topn_every  = 5
topn_count  = 5

[retention]
raw_days    = 7
total_days  = 30

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
        match checks.iter().find(|(ok, _)| !ok) {
            Some((_, msg)) => Err((*msg).to_owned()),
            None => Ok(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_to_defaults() {
        assert_eq!(Config::parse(DEFAULT_TOML), Ok(Config::default()));
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
