//! Taskbar sistem ızgarası (ADR-0033 Faz 2): 2 satır × 4 sütun.
//! CPU %, MEM %, ağ ↑↓ pipe `Sample`'ından (`Live`); GPU % / °C, CPU °C, disk °C `lowmon-sensors`'tan.
//! Sensörler ayrı iş parçacığında, blok açıkken ya da deck sensör abonesi varken, 2 s'de bir okunur (PDH açılışı ~0,3 s,
//! disk sorgusu 5–35 ms bekletir); sonuç `READING`'e yazılır, çizim UI thread'inde.
use crate::{Live, ai};
use lowmon_ai::usage::Level;
use lowmon_sensors::{Reading, Sensors, Sources, log};
use lowmon_taskbar::{Grid, Line, Sys};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::time::Duration;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const WM_SYS: u32 = WM_APP + 5;
/// Bloklar sağdan sola: sistem ızgarası bildirim alanının yanında, AI onun solunda.
pub const BLOCK: usize = 0;
const PERIOD: Duration = Duration::from_secs(2);

static READING: Mutex<Option<Reading>> = Mutex::new(None);
/// Çalışan sensör iş parçacığının kuşağı; değişince eski iş parçacığı bir sonraki turda çıkar.
static GEN: AtomicU32 = AtomicU32::new(0);
/// Sensör iş parçacığı çalışıyor (tek kopya).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Izgara açık (`[sys] enabled`): iş parçacığı abone olmasa da sürer.
static GRID: AtomicBool = AtomicBool::new(false);
/// `[sys] cpu_temp` (PDH +4 MiB); ızgara ayarı yüklenmeden abone gelirse varsayılan açık.
static CPU_TEMP: AtomicBool = AtomicBool::new(true);
/// Sıcaklık alarmı eşikleri (`[sys] temp_warn_c` / `disk_warn_c`); ızgara kapalıyken de geçerli.
static TEMP_WARN: AtomicI32 = AtomicI32::new(80);
static DISK_WARN: AtomicI32 = AtomicI32::new(60);
/// Alarm eşiğin bu kadar altına inince temizlenir (eşikte titreşim olmasın).
const HYST_C: i32 = 3;
/// Deck olay adları (alarm kuralı id'si yerine); sıra `temp_edges` ile aynı.
const TEMP_NAMES: [&str; 3] = ["Temp.CPU", "Temp.GPU", "Temp.Disk"];

thread_local! {
    static CFG: Cell<Option<Sys>> = const { Cell::new(None) };
}

/// Izgara ayarını uygular ve sensör iş parçacığını yeniden kurar (`[sys] enabled = false` → ızgara yok;
/// iş parçacığı yalnız deck sensör abonesi varsa sürer, ADR-0033 / LowDeck `TOPIC_SENSORS`).
pub fn start(hwnd: HWND, cfg: Sys) {
    stop();
    CPU_TEMP.store(cfg.cpu_temp, Ordering::Relaxed);
    TEMP_WARN.store(cfg.temp_warn_c, Ordering::Relaxed);
    DISK_WARN.store(cfg.disk_warn_c, Ordering::Relaxed);
    if cfg.enabled {
        CFG.set(Some(cfg));
        GRID.store(true, Ordering::Relaxed);
    }
    sync(hwnd);
}

/// İş parçacığı gerekli mi: ızgara açık, deck'te sensör abonesi var ya da telefon olay bekliyor (sıcaklık alarmı).
fn wanted() -> bool {
    GRID.load(Ordering::Relaxed)
        || crate::DECK_HUB
            .get()
            .is_some_and(|h| h.wants_sensors() || h.phone_wants_events())
}

/// Sıcaklık alarmı kenarları: `(sıra, kalktı mı, °C)`. Eşikte kalkar, `HYST_C` altında iner; sensör yoksa durum korunur.
fn temp_edges(
    on: &mut [bool; 3],
    r: &Reading,
    warn: i32,
    disk_warn: i32,
) -> Vec<(usize, bool, i32)> {
    let vals = [(r.cpu_c, warn), (r.gpu_c, warn), (r.disk_c, disk_warn)];
    let mut out = Vec::new();
    for (i, (v, w)) in vals.into_iter().enumerate() {
        let Some(v) = v else { continue };
        let next = if on[i] { v > w - HYST_C } else { v >= w };
        if next != on[i] {
            on[i] = next;
            out.push((i, next, v));
        }
    }
    out
}

/// Kenarları deck'e olay olarak iter (telefon perdesi alarmda açılır; LM UI alarm bandı da görür).
/// shortcut: olaylar servis olay deposuna yazılmaz (geçmişte yok); gerekirse servise sensör alarmı kuralı.
fn push_temp(edges: &[(usize, bool, i32)]) {
    let Some(h) = crate::DECK_HUB.get() else {
        return;
    };
    let ts_ms = lowmon_core::time::now_ms();
    for &(i, raise, c) in edges {
        h.push_event(lowmon_deck::EventItem {
            ts_ms,
            kind: if raise { 0 } else { 1 },
            name: TEMP_NAMES[i].into(),
            value: i64::from(c) * 100,
        });
    }
}

/// İş parçacığı biterken açık alarmları temizle: okunmayan sıcaklık için alarm asılı kalmasın.
fn clear_temp(on: &mut [bool; 3]) {
    let edges: Vec<_> = (0..3).filter(|&i| on[i]).map(|i| (i, false, 0)).collect();
    *on = [false; 3];
    push_temp(&edges);
}

/// Gerekliyse ve çalışmıyorsa iş parçacığını başlatır (deck abonelik değişince de çağrılır).
/// Gereksizse iş parçacığı bir sonraki turda kendisi çıkar.
pub fn sync(hwnd: HWND) {
    if !wanted() || RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let generation = GEN.load(Ordering::Relaxed);
    let owner = hwnd as usize;
    let src = Sources {
        gpu: true,
        cpu_temp: CPU_TEMP.load(Ordering::Relaxed),
        disk_temp: true,
    };
    let spawned = std::thread::Builder::new()
        .name("sensors".into())
        .spawn(move || {
            let mut s = Sensors::open(src);
            // Sıcaklık geçmişi (ADR-0035): dakika bitince bir kayıt
            let (mut minute, log_path) = (log::Minute::default(), log::path());
            let mut hot = [false; 3];
            while GEN.load(Ordering::Relaxed) == generation {
                if !wanted() {
                    // Çıkmadan önce bayrağı bırak, sonra yeniden bak: arada abone geldiyse sync bizi görmedi
                    RUNNING.store(false, Ordering::Release);
                    if !wanted() || RUNNING.swap(true, Ordering::AcqRel) {
                        clear_temp(&mut hot);
                        return;
                    }
                }
                let r = s.read();
                push_temp(&temp_edges(
                    &mut hot,
                    &r,
                    TEMP_WARN.load(Ordering::Relaxed),
                    DISK_WARN.load(Ordering::Relaxed),
                ));
                *READING.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                let now_s = lowmon_core::time::now_ms() / 1000;
                if let Some(rec) = minute.add(now_s, &r)
                    && let Err(e) = log::write(&log_path, &rec)
                {
                    lowmon_deck::log::write(&format!("sys: sensör geçmişi yazılamadı: {e}"));
                }
                if let Some(h) = crate::DECK_HUB.get() {
                    h.set_sensors(to_proto(&r));
                }
                // SAFETY: pencere yok olduysa PostMessage yalnız başarısız döner.
                unsafe { PostMessageW(owner as HWND, WM_SYS, 0, 0) };
                std::thread::sleep(PERIOD);
            }
            clear_temp(&mut hot);
            // Kuşak değişti (stop/start): yeni iş parçacığı RUNNING'i zaten aldı ya da alacak
        });
    if let Err(e) = spawned {
        RUNNING.store(false, Ordering::Release);
        lowmon_deck::log::write(&format!("sys: sensör iş parçacığı açılamadı: {e}"));
    }
}

/// `lowmon-sensors` okuması → telefon/UI protokolü (aralık dışı değer kırpılır).
fn to_proto(r: &Reading) -> lowmon_deck::Sensors {
    let c = |v: Option<i32>| v.map(|v| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    lowmon_deck::Sensors {
        gpu_pct: r.gpu_pct.map(|v| v.min(100) as u8),
        gpu_c: c(r.gpu_c),
        cpu_c: c(r.cpu_c),
        disk_c: c(r.disk_c),
    }
}

pub fn stop() {
    GEN.fetch_add(1, Ordering::Relaxed);
    RUNNING.store(false, Ordering::Release);
    GRID.store(false, Ordering::Relaxed);
    CFG.set(None);
    *READING.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Son `Sample` + son sensör okumasıyla bloğu çizer. İçerik değişmediyse `Bar::show` iş yapmaz.
pub fn render() {
    let Some(cfg) = CFG.get() else { return };
    let live = *crate::LATEST.lock().unwrap_or_else(|e| e.into_inner());
    let reading = READING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_default();
    let g = grid(
        live.as_ref(),
        &reading,
        crate::TOTAL_KB.get(),
        &cfg,
        lowmon_taskbar::light_theme(),
    );
    ai::BAR.with_borrow_mut(|b| {
        if let Some(b) = b {
            b.show(BLOCK, g);
        }
    });
}

fn level(v: i32, warn: i32, crit: i32) -> Level {
    if v >= crit {
        Level::Critical
    } else if v >= warn {
        Level::Warn
    } else {
        Level::Normal
    }
}

/// "0.67 KB/s", "22.6 KB/s", "118 KB/s", "1.24 MB/s": 3 anlamlı basamak, en küçük birim KB.
pub fn rate(bytes_per_s: u64) -> String {
    let mut v = bytes_per_s as f64 / 1024.0;
    let mut unit = "KB/s";
    for u in ["MB/s", "GB/s"] {
        if v < 999.5 {
            break;
        }
        v /= 1024.0;
        unit = u;
    }
    let prec = if v < 9.995 {
        2
    } else if v < 99.95 {
        1
    } else {
        0
    };
    format!("{v:.prec$} {unit}")
}

/// Izgara içeriği. `live` yok (servis kapalı) ya da sensör yok → o hücre `—`, gri.
pub fn grid(live: Option<&Live>, r: &Reading, mem_total_kb: u64, c: &Sys, light: bool) -> Grid {
    let cell = |label: &str, v: Option<i32>, unit: &str, warn: i32, crit: i32| match v {
        Some(v) => Line {
            text: format!("{label}: {v} {unit}"),
            rgb: ai::color(level(v, warn, crit), light),
        },
        None => Line {
            text: format!("{label}: —"),
            rgb: ai::color(Level::Dim, light),
        },
    };
    let pct = |label, v| cell(label, v, "%", c.warn_pct, c.crit_pct);
    let net = |label: &str, v: Option<u64>| Line {
        text: format!("{label}: {}", v.map_or("—".into(), rate)),
        rgb: ai::color(
            if v.is_some() {
                Level::Normal
            } else {
                Level::Dim
            },
            light,
        ),
    };
    let cpu = live.map(|l| (l.cpu as f64 / 100.0).round() as i32);
    let mem = live
        .filter(|_| mem_total_kb > 0)
        .map(|l| (100.0 * l.ram_kb as f64 / mem_total_kb as f64).round() as i32);
    Grid {
        rows: [
            vec![
                pct("CPU", cpu),
                pct("GPU", r.gpu_pct.map(|v| v as i32)),
                pct("MEM", mem),
                net("↑", live.map(|l| l.tx_bps)),
            ],
            vec![
                cell("CPU", r.cpu_c, "°C", c.temp_warn_c, c.temp_crit_c),
                cell("GPU", r.gpu_c, "°C", c.temp_warn_c, c.temp_crit_c),
                cell("HDD", r.disk_c, "°C", c.disk_warn_c, c.disk_crit_c),
                net("↓", live.map(|l| l.rx_bps)),
            ],
        ],
        right: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(rate(0), "0.00 KB/s");
        assert_eq!(rate(686), "0.67 KB/s");
        assert_eq!(rate(23_142), "22.6 KB/s");
        assert_eq!(rate(120_832), "118 KB/s");
        assert_eq!(rate(1_023_000), "999 KB/s");
        assert_eq!(rate(1_024_000), "0.98 MB/s");
        assert_eq!(rate(1_300_234), "1.24 MB/s");
        assert_eq!(rate(3 * 1024 * 1024 * 1024), "3.00 GB/s");
    }

    #[test]
    fn temp_alarm_hysteresis() {
        let mut on = [false; 3];
        let r = |cpu, disk| Reading {
            cpu_c: Some(cpu),
            disk_c: disk,
            ..Reading::default()
        };
        assert!(temp_edges(&mut on, &r(94, None), 95, 60).is_empty());
        assert_eq!(
            temp_edges(&mut on, &r(95, Some(61)), 95, 60),
            [(0, true, 95), (2, true, 61)]
        );
        // eşik - 3 üstünde kalır; sensör kaybolunca disk alarmı korunur
        assert!(temp_edges(&mut on, &r(93, None), 95, 60).is_empty());
        assert_eq!(temp_edges(&mut on, &r(92, None), 95, 60), [(0, false, 92)]);
        assert_eq!(on, [false, false, true]);
    }

    #[test]
    fn grid_cells() {
        let live = Live {
            cpu: 1_549,
            ram_kb: 14_000_000,
            rx_bps: 23_142,
            tx_bps: 686,
        };
        let r = Reading {
            gpu_pct: Some(0),
            gpu_c: Some(92),
            cpu_c: Some(81),
            disk_c: None,
        };
        let c = Sys::default();
        let g = grid(Some(&live), &r, 16_000_000, &c, false);
        let text =
            |row: usize| -> Vec<&str> { g.rows[row].iter().map(|l| l.text.as_str()).collect() };
        assert_eq!(
            text(0),
            ["CPU: 15 %", "GPU: 0 %", "MEM: 88 %", "↑: 0.67 KB/s"]
        );
        assert_eq!(
            text(1),
            ["CPU: 81 °C", "GPU: 92 °C", "HDD: —", "↓: 22.6 KB/s"]
        );
        assert!(!g.right);
        let rgb = |row: usize, col: usize| g.rows[row][col].rgb;
        assert_eq!(rgb(0, 0), 0xFFFFFF);
        assert_eq!(rgb(0, 2), 0xF0A030); // MEM 88 ≥ 75
        assert_eq!(rgb(1, 0), 0xF0A030); // CPU 81 °C ≥ 80
        assert_eq!(rgb(1, 1), 0xE5484D); // GPU 92 °C ≥ 90
        assert_eq!(rgb(1, 2), 0x9A9A9A); // sensör yok
        // servis kapalı: CPU %, MEM, ağ boş; sensörler kalır
        let g = grid(None, &r, 16_000_000, &c, true);
        assert_eq!(g.rows[0][0].text, "CPU: —");
        assert_eq!(g.rows[0][3].text, "↑: —");
        assert_eq!(g.rows[1][1].text, "GPU: 92 °C");
        assert_eq!(g.rows[0][0].rgb, 0x707070);
    }
}
