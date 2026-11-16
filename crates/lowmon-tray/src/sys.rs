//! Taskbar sistem ızgarası (ADR-0033 Faz 2): 2 satır × 4 sütun.
//! CPU %, MEM %, ağ ↑↓ pipe `Sample`'ından (`Live`); GPU % / °C, CPU °C, disk °C `lowmon-sensors`'tan.
//! Sensörler ayrı iş parçacığında, yalnız blok açıkken, 2 s'de bir okunur (PDH açılışı ~0,3 s,
//! disk sorgusu 5–35 ms bekletir); sonuç `READING`'e yazılır, çizim UI thread'inde.
use crate::{Live, ai};
use lowmon_ai::usage::Level;
use lowmon_sensors::{Reading, Sensors, Sources};
use lowmon_taskbar::{Grid, Line, Sys};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use windows_sys::Win32::{
    Foundation::HWND,
    System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const WM_SYS: u32 = WM_APP + 5;
/// Bloklar sağdan sola: sistem ızgarası bildirim alanının yanında, AI onun solunda.
pub const BLOCK: usize = 0;
const PERIOD: Duration = Duration::from_secs(2);

static READING: Mutex<Option<Reading>> = Mutex::new(None);
/// Çalışan sensör iş parçacığının kuşağı; değişince eski iş parçacığı bir sonraki turda çıkar.
static GEN: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static CFG: Cell<Option<Sys>> = const { Cell::new(None) };
    static MEM_TOTAL_KB: Cell<u64> = const { Cell::new(0) };
}

/// Sensör iş parçacığını (yeniden) başlatır; `[sys] enabled = false` ise yalnız durdurur.
pub fn start(hwnd: HWND, cfg: Sys) {
    stop();
    if !cfg.enabled {
        return;
    }
    CFG.set(Some(cfg));
    let mut ms = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        // SAFETY: düz veri yapısı; sıfır geçerli başlangıç.
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: ms yerel, dwLength dolu.
    if unsafe { GlobalMemoryStatusEx(&mut ms) } != 0 {
        MEM_TOTAL_KB.set(ms.ullTotalPhys / 1024);
    }
    let generation = GEN.load(Ordering::Relaxed);
    let owner = hwnd as usize;
    let src = Sources {
        gpu: true,
        cpu_temp: cfg.cpu_temp,
        disk_temp: true,
    };
    let spawned = std::thread::Builder::new()
        .name("sensors".into())
        .spawn(move || {
            let mut s = Sensors::open(src);
            while GEN.load(Ordering::Relaxed) == generation {
                *READING.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.read());
                // SAFETY: pencere yok olduysa PostMessage yalnız başarısız döner.
                unsafe { PostMessageW(owner as HWND, WM_SYS, 0, 0) };
                std::thread::sleep(PERIOD);
            }
        });
    if let Err(e) = spawned {
        lowmon_deck::log::write(&format!("sys: sensör iş parçacığı açılamadı: {e}"));
    }
}

pub fn stop() {
    GEN.fetch_add(1, Ordering::Relaxed);
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
        MEM_TOTAL_KB.get(),
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
