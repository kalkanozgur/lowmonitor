//! lowmon-taskbar (ADR-0032 Faz 2, ADR-0033 Faz 2): taskbar'a gömülü renkli metin blokları.
//! Blok = 2 satır × N sütun hücre ızgarası (AI 2×1, sistem 2×4); bloklar bildirim alanının solunda
//! yan yana dizilir (blok 0 en sağda), her biri ayrı alt pencere. Her blok kendi monitörünün taskbar'ına
//! kurulur (ADR-0033 Faz 3); o monitörde taskbar yoksa ortak `monitor` ayarındakine döner.
//! Yöntem (kullanıcı kararı, C++ widget'ın topmost + 1 s yoklamasının yerine):
//! - Pencere `Shell_TrayWnd`'nin **alt penceresi**: taskbar'la hareket eder, tam ekranda onunla
//!   gizlenir; z-sırası kavgası ve zamanlayıcı yok.
//! - Bildirim alanı genişleyince `EVENT_OBJECT_LOCATIONCHANGE` (out-of-context) → sahibe `EV_MOVED`.
//! - Metin GDI+ ile PARGB bitmap'e (gerçek alfa, gri tonlu AA) → `UpdateLayeredWindow`.
//!
//! Sahip pencere olayları `PostMessage(owner, msg, EV_*, blok)` ile alır; `Bar` sahibin thread'inde yaşar.
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod bar;
#[cfg(windows)]
pub use bar::{Bar, light_theme};

/// Sol tık
pub const EV_LEFT: usize = 1;
/// Sağ tık
pub const EV_RIGHT: usize = 2;
/// Taskbar / bildirim alanı yer değiştirdi → `Bar::layout`
pub const EV_MOVED: usize = 3;

/// Bir hücre: metin + renk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    /// 0xRRGGBB
    pub rgb: u32,
}

/// Bir bloğun içeriği: iki satır, satır başına aynı sayıda hücre; sütunlar hizalanır.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    pub rows: [Vec<Line>; 2],
    /// Hücre metni sütunun sağına yaslanır (AI); değilse sola (sistem ızgarası)
    pub right: bool,
}

impl Grid {
    /// Tek sütun, sağa yaslı (AI bloğu).
    pub fn lines([a, b]: [Line; 2]) -> Self {
        Self {
            rows: [vec![a], vec![b]],
            right: true,
        }
    }
}

/// Sütun genişlikleri: iki satırın en genişi, önceki genişlikten (`keep`) dar olmaz. Sayılar değiştikçe
/// blok daralıp genişlemesin (yanındaki blok oynamasın) diye genişlik yalnız büyür.
pub fn columns(measured: [&[i32]; 2], keep: &[i32]) -> Vec<i32> {
    let n = measured[0].len().max(measured[1].len());
    (0..n)
        .map(|i| {
            let at = |v: &[i32]| v.get(i).copied().unwrap_or(0);
            at(measured[0]).max(at(measured[1])).max(at(keep))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Monitor {
    Primary,
    Left,
    Right,
    /// Ortak `monitor` ayarının seçtiği monitörden farklı olan (birden çoksa ona en yakını);
    /// yalnız blok ayarında anlamlı. Başka monitör yoksa blok ortak monitöre döner.
    Other,
}

/// `[ai]`: AI kullanım bloğu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ai {
    pub enabled: bool,
    /// Bloğun monitörü; yoksa ortak `monitor`
    pub monitor: Option<Monitor>,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            enabled: true,
            monitor: None,
        }
    }
}

/// `[sys]`: sistem ızgarası (ADR-0033). Eşik: değer ≥ uyarı → turuncu, ≥ kritik → kırmızı.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sys {
    pub enabled: bool,
    /// Bloğun monitörü; varsayılan `other` (ikinci monitör takılıysa orada, değilse AI'ın yanında)
    pub monitor: Option<Monitor>,
    /// CPU °C (ACPI termal bölge, PDH: +~4 MiB); kapalıysa hücre `—`
    pub cpu_temp: bool,
    /// CPU / GPU / MEM kullanımı (%)
    pub warn_pct: i32,
    pub crit_pct: i32,
    /// CPU / GPU sıcaklığı (°C)
    pub temp_warn_c: i32,
    pub temp_crit_c: i32,
    /// Disk sıcaklığı (°C)
    pub disk_warn_c: i32,
    pub disk_crit_c: i32,
}

impl Default for Sys {
    fn default() -> Self {
        Self {
            enabled: true,
            monitor: Some(Monitor::Other),
            cpu_temp: true,
            warn_pct: 75,
            crit_pct: 90,
            temp_warn_c: 80,
            temp_crit_c: 90,
            disk_warn_c: 55,
            disk_crit_c: 65,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Hangi monitörün taskbar'ı (C++ varsayılanı: en sağdaki)
    pub monitor: Monitor,
    /// Bildirim alanıyla aradaki boşluk (96 DPI px)
    pub gap_px: i32,
    /// Bildirim alanı olmayan (ikincil) taskbar'da sağdan ayrılan yer (96 DPI px)
    pub reserved_px: i32,
    pub font: String,
    pub font_pt: f32,
    pub ai: Ai,
    pub sys: Sys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitor: Monitor::Right,
            gap_px: 8,
            reserved_px: 110,
            font: "Segoe UI".into(),
            font_pt: 9.0,
            ai: Ai::default(),
            sys: Sys::default(),
        }
    }
}

impl Config {
    pub fn parse(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    /// Dosya yoksa varsayılan.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// `%APPDATA%\LowMonitor\taskbar.toml`
pub fn config_path() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("LowMonitor")
        .join("taskbar.toml")
}

/// Monitör seçimi; `mons` = (sol kenar, birincil mi). `base`: ortak `monitor` ayarı (`Other` buna göre;
/// `base` de `Other` ise birincil sayılır). `Other` için başka monitör yoksa `None`.
pub fn pick(mons: &[(i32, bool)], sel: Monitor, base: Monitor) -> Option<usize> {
    let by_left =
        |rev: bool| (0..mons.len()).max_by_key(|&i| if rev { -mons[i].0 } else { mons[i].0 });
    match sel {
        Monitor::Right => by_left(false),
        Monitor::Left => by_left(true),
        Monitor::Primary => mons.iter().position(|m| m.1).or_else(|| by_left(false)),
        Monitor::Other => {
            let base = if base == Monitor::Other {
                Monitor::Primary
            } else {
                base
            };
            let b = pick(mons, base, base)?;
            (0..mons.len())
                .filter(|&i| i != b)
                .min_by_key(|&i| (mons[i].0 - mons[b].0).abs())
        }
    }
}

/// 96 DPI px → `dpi` px
pub fn scale(px: i32, dpi: u32) -> i32 {
    (px as i64 * dpi as i64 / 96) as i32
}

/// Göstergenin sağ kenarı (taskbar istemci x'i). `notify_left`: bildirim alanının sol kenarı
/// (istemci x'i); taskbar dışındaysa / yoksa sağdan `reserved` ayrılır.
pub fn anchor(width: i32, notify_left: Option<i32>, gap: i32, reserved: i32) -> i32 {
    let edge = match notify_left {
        Some(l) if l > 0 && l < width => l,
        _ => width - reserved,
    };
    edge - gap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_pick() {
        let m = [(1920, false), (0, true), (-1280, false)];
        let r = Monitor::Right;
        assert_eq!(pick(&m, Monitor::Right, r), Some(0));
        assert_eq!(pick(&m, Monitor::Left, r), Some(2));
        assert_eq!(pick(&m, Monitor::Primary, r), Some(1));
        assert_eq!(
            pick(&[(0, false), (800, false)], Monitor::Primary, r),
            Some(1)
        );
        assert_eq!(pick(&[], Monitor::Right, r), None);
        // Other: ortak monitörden farklı, ona en yakın olan
        assert_eq!(pick(&m, Monitor::Other, Monitor::Right), Some(1));
        assert_eq!(pick(&m, Monitor::Other, Monitor::Left), Some(1));
        assert_eq!(pick(&m, Monitor::Other, Monitor::Primary), Some(2));
        assert_eq!(pick(&m, Monitor::Other, Monitor::Other), Some(2)); // taban birincil sayılır
        // bu PC: birincil 2560 (x=0) + solda dizüstü (x=-1920)
        let pc = [(-1920, false), (0, true)];
        assert_eq!(pick(&pc, Monitor::Other, Monitor::Right), Some(0));
        // tek monitör: başka yok
        assert_eq!(pick(&[(0, true)], Monitor::Other, Monitor::Right), None);
    }

    #[test]
    fn anchor_and_scale() {
        assert_eq!(anchor(1920, Some(1700), 8, 110), 1692);
        assert_eq!(anchor(1920, None, 8, 110), 1802);
        assert_eq!(anchor(1920, Some(2000), 8, 110), 1802); // taskbar dışı
        assert_eq!(scale(8, 144), 12);
        assert_eq!(scale(110, 96), 110);
    }

    #[test]
    fn column_widths() {
        // sütun = iki satırın en genişi; önceki genişlikten daralmaz
        assert_eq!(columns([&[40, 10], &[30, 25]], &[]), [40, 25]);
        assert_eq!(columns([&[40, 10], &[30, 25]], &[50, 5]), [50, 25]);
        assert_eq!(columns([&[40], &[30, 25]], &[0, 0, 9]), [40, 25]);
        assert_eq!(columns([&[], &[]], &[]), Vec::<i32>::new());
        let g = Grid::lines([
            Line {
                text: "a".into(),
                rgb: 1,
            },
            Line {
                text: "b".into(),
                rgb: 2,
            },
        ]);
        assert!(g.right && g.rows[0].len() == 1 && g.rows[1][0].text == "b");
    }

    #[test]
    fn config() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        let c = Config::parse("monitor = \"primary\"\nfont_pt = 10.5").unwrap();
        assert_eq!((c.monitor, c.font_pt), (Monitor::Primary, 10.5));
        assert!(Config::parse("x = 1").is_err());
        let c = Config::parse("[sys]\nenabled = false\ntemp_crit_c = 95\n[ai]\nenabled = false")
            .unwrap();
        assert!(!c.sys.enabled && !c.ai.enabled);
        assert_eq!((c.sys.monitor, c.ai.monitor), (Some(Monitor::Other), None));
        let d = Config::parse("[sys]\nmonitor = \"primary\"\n[ai]\nmonitor = \"other\"").unwrap();
        assert_eq!(
            (d.sys.monitor, d.ai.monitor),
            (Some(Monitor::Primary), Some(Monitor::Other))
        );
        assert_eq!((c.sys.temp_crit_c, c.sys.temp_warn_c), (95, 80));
        assert!(Config::parse("[sys]\nx = 1").is_err());
        let missing = std::env::temp_dir().join("lowmon-test-yok-taskbar.toml");
        assert_eq!(Config::load(&missing).unwrap(), Config::default());
    }
}
