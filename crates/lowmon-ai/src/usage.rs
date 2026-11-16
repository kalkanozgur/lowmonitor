//! Tempo, renk seviyesi ve limit CSV'si (ai-usage-bar `usage_math` + `display` seviye kısmı).
//! CSV satırı: `ts,session_pct,session_reset,week_pct,week_reset`; boş alan = yanıtta yoktu.
use crate::api::{Snapshot, Window};
use lowmon_core::time::civil;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;

pub const SESSION_SEC: i64 = 5 * 3600;
pub const WEEK_SEC: i64 = 7 * 24 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warn,
    Critical,
    /// veri yok ya da eski
    Dim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    /// %100'e pencere bitmeden ulaşacak hızdaysa turuncu, `crit_pct`'de kırmızı
    Pace,
    /// `warn_pct` turuncu, `crit_pct` kırmızı
    Threshold,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub mode: ColorMode,
    pub warn_pct: f64,
    pub crit_pct: f64,
    /// eşit harcamanın kaç puan önündeyse turuncu
    pub pace_margin: f64,
    /// bunun altında tempo uyarısı yok (pencere başı gürültüsü)
    pub pace_min_pct: f64,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            mode: ColorMode::Pace,
            warn_pct: 75.0,
            crit_pct: 90.0,
            pace_margin: 5.0,
            pace_min_pct: 25.0,
        }
    }
}

/// Eşit harcamada olması gereken yüzde ve fark (> 0: tempo önde).
pub fn pace(utilization: f64, resets_at: i64, window_sec: i64, now: i64) -> (f64, f64) {
    let remaining = (resets_at - now).clamp(0, window_sec);
    let expected = 100.0 * (window_sec - remaining) as f64 / window_sec as f64;
    (expected, utilization - expected)
}

pub fn level(w: &Window, window_sec: i64, now: i64, c: &Colors) -> Level {
    if w.utilization >= c.crit_pct {
        return Level::Critical;
    }
    match (c.mode, w.resets_at) {
        (ColorMode::Pace, Some(r)) => {
            let (_, delta) = pace(w.utilization, r, window_sec, now);
            if w.utilization >= c.pace_min_pct && delta >= c.pace_margin {
                Level::Warn
            } else {
                Level::Normal
            }
        }
        // tempo modu ama reset bilinmiyor → eşik
        _ if w.utilization >= c.warn_pct => Level::Warn,
        _ => Level::Normal,
    }
}

/// (oturum, hafta) seviyeleri; `stale` ikisini de soluklaştırır.
pub fn levels(s: &Snapshot, now: i64, c: &Colors, stale: bool) -> (Level, Level) {
    let one = |w: &Option<Window>, sec| match w {
        Some(w) if !stale => level(w, sec, now, c),
        _ => Level::Dim,
    };
    (one(&s.five_hour, SESSION_SEC), one(&s.seven_day, WEEK_SEC))
}

pub const CSV_HEADER: &str = "ts,session_pct,session_reset,week_pct,week_reset";

pub fn csv_row(ts: i64, s: &Snapshot) -> String {
    let pct = |w: &Option<Window>| {
        w.map(|w| format!("{:.1}", w.utilization))
            .unwrap_or_default()
    };
    let reset = |w: &Option<Window>| {
        w.and_then(|w| w.resets_at)
            .map(|r| r.to_string())
            .unwrap_or_default()
    };
    format!(
        "{ts},{},{},{},{}",
        pct(&s.five_hour),
        reset(&s.five_hour),
        pct(&s.seven_day),
        reset(&s.seven_day)
    )
}

/// Aylık dosya adı (UTC): `usage-2026-10.csv`.
pub fn csv_file_name(ts: i64) -> String {
    let (y, m, _) = civil(ts.max(0) as u64 / 86_400);
    format!("usage-{y}-{m:02}.csv")
}

/// Bir satır ekler; dosya yeni ya da boşsa önce başlık.
pub fn append_csv(dir: &Path, ts: i64, s: &Snapshot) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(csv_file_name(ts)))?;
    if f.metadata()?.len() == 0 {
        write!(f, "{CSV_HEADER}\r\n")?;
    }
    write!(f, "{}\r\n", csv_row(ts, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(u: f64, r: Option<i64>) -> Window {
        Window {
            utilization: u,
            resets_at: r,
        }
    }

    #[test]
    fn pace_math() {
        // 5 sa pencerenin yarısı geçti → beklenen %50
        let (e, d) = pace(60.0, 1000 + SESSION_SEC / 2, SESSION_SEC, 1000);
        assert!((e - 50.0).abs() < 1e-9 && (d - 10.0).abs() < 1e-9);
        // reset geçmiş → tam pencere geçti sayılır
        assert_eq!(pace(10.0, 0, SESSION_SEC, 100).0, 100.0);
    }

    #[test]
    fn levels_pace_and_threshold() {
        let c = Colors::default();
        let now = 1_000_000;
        let half = Some(now + SESSION_SEC / 2);
        assert_eq!(level(&w(91.0, half), SESSION_SEC, now, &c), Level::Critical);
        assert_eq!(level(&w(56.0, half), SESSION_SEC, now, &c), Level::Warn); // 6 puan önde
        assert_eq!(level(&w(54.0, half), SESSION_SEC, now, &c), Level::Normal);
        assert_eq!(
            level(&w(80.0, Some(now + 60)), SESSION_SEC, now, &c),
            Level::Normal
        ); // yavaş
        assert_eq!(
            level(&w(20.0, Some(now + SESSION_SEC)), SESSION_SEC, now, &c),
            Level::Normal
        ); // min altı
        assert_eq!(level(&w(80.0, None), SESSION_SEC, now, &c), Level::Warn); // reset yok → eşik
        let t = Colors {
            mode: ColorMode::Threshold,
            ..c
        };
        assert_eq!(
            level(&w(80.0, Some(now + 60)), SESSION_SEC, now, &t),
            Level::Warn
        );
        let s = Snapshot {
            five_hour: Some(w(10.0, None)),
            ..Default::default()
        };
        assert_eq!(levels(&s, now, &c, false), (Level::Normal, Level::Dim));
        assert_eq!(levels(&s, now, &c, true), (Level::Dim, Level::Dim));
    }

    #[test]
    fn csv() {
        let s = Snapshot {
            five_hour: Some(w(23.0, Some(1_793_397_600))),
            seven_day: Some(w(41.04, None)),
            ..Default::default()
        };
        assert_eq!(
            csv_row(1_793_383_200, &s),
            "1793383200,23.0,1793397600,41.0,"
        );
        assert_eq!(csv_row(5, &Snapshot::default()), "5,,,,");
        assert_eq!(csv_file_name(1_793_383_200), "usage-2026-10.csv");
        let dir = std::env::temp_dir().join(format!("lowmon-test-ai-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        append_csv(&dir, 1_793_383_200, &s).unwrap();
        append_csv(&dir, 1_793_383_380, &s).unwrap();
        let text = std::fs::read_to_string(dir.join("usage-2026-10.csv")).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(text.starts_with(CSV_HEADER));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
