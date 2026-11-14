//! lowmon-tray (spek §8): tray ikonu (CPU/RAM çift çubuk) + tooltip, pipe'tan 2 s'de bir Sample.
//! Spekten farkı: windows-rs yerine windows-sys (diğer crate'lerle aynı, ADR-0018).
#![cfg_attr(not(test), windows_subsystem = "windows")]

use lowmon_core::{
    metrics::Sample,
    proto::{self, CH_SAMPLE, ClientMsg, PIPE_NAME, PROTO, Role, ServerMsg},
};
use std::{
    cell::Cell,
    fs::File,
    io,
    os::windows::io::FromRawHandle,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, GENERIC_READ, GetLastError, HWND, INVALID_HANDLE_VALUE, LPARAM,
        LRESULT, POINT, WPARAM,
    },
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
        DeleteObject, GetDC, ReleaseDC,
    },
    Storage::FileSystem::{CreateFileW, FILE_WRITE_DATA, OPEN_EXISTING},
    System::{
        LibraryLoader::GetModuleHandleW,
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::CreateMutexW,
    },
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
            Shell_NotifyIconW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
            DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
            HICON, ICONINFO, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
            PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
            TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP,
            WM_DESTROY, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
        },
    },
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_DATA: u32 = WM_APP + 2;
const ID_UI: usize = 1;
const ID_MUTE: usize = 2;
const ID_EXIT: usize = 3;
const RATE_MS: u32 = 2_000;
const RETRY: Duration = Duration::from_secs(10);
// shortcut: sabit 16 px; yüksek DPI'da kabuk ölçekler (bulanık), SM_CXSMICON + DPI manifest M4'te
const N: usize = 16;

/// Tray için gereken özet; None = servise bağlı değil.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Live {
    cpu: u16, // 0.01 %
    ram_kb: u64,
    rx_bps: u64,
}

static LATEST: Mutex<Option<Live>> = Mutex::new(None);
static HWND_: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ICON: Cell<HICON> = const { Cell::new(ptr::null_mut()) };
    static TOTAL_KB: Cell<u64> = const { Cell::new(0) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn pct(part: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (part as f32 * 100.0 / total as f32).min(100.0)
    }
}

fn rate(bps: u64) -> String {
    if bps >= 1 << 20 {
        format!("{:.1} MB/s", bps as f32 / (1 << 20) as f32)
    } else {
        format!("{} KB/s", bps >> 10)
    }
}

fn tooltip(live: Option<Live>, total_kb: u64) -> String {
    match live {
        None => "LowMonitor: servise bağlı değil".into(),
        Some(l) => format!(
            "CPU {:.0}% · RAM {:.0}% · ↓{}",
            l.cpu as f32 / 100.0,
            pct(l.ram_kb, total_kb),
            rate(l.rx_bps)
        ),
    }
}

/// 16×16 ARGB (üstten aşağı): solda CPU, sağda RAM çubuğu; bağlı değilse yalnızca gri iz.
fn pixels(bars: Option<(f32, f32)>) -> [u32; N * N] {
    const TRACK: u32 = 0x6080_8080;
    let mut px = [0u32; N * N];
    let color = |v: f32, base: u32| if v >= 90.0 { 0xFFEF_4444 } else { base };
    for (x0, idx) in [(1usize, 0), (9, 1)] {
        let (v, c) = match bars {
            None => (0.0, 0),
            Some((cpu, _)) if idx == 0 => (cpu, color(cpu, 0xFF22_C55E)),
            Some((_, ram)) => (ram, color(ram, 0xFF3B_82F6)),
        };
        // En az 1 px: %0 bile "bağlı" olarak görünsün
        let h = if bars.is_some() {
            ((v / 100.0 * N as f32).round() as usize).clamp(1, N)
        } else {
            0
        };
        for y in 0..N {
            for x in x0..x0 + 6 {
                px[y * N + x] = if y >= N - h { c } else { TRACK };
            }
        }
    }
    px
}

fn make_icon(bars: Option<(f32, f32)>) -> HICON {
    let px = pixels(bars);
    // SAFETY: bits DIB'in N*N*4 B'lık tamponunu gösterir; GDI nesneleri ikon oluşunca silinir.
    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: N as i32,
            biHeight: -(N as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let dc = GetDC(ptr::null_mut());
        let mut bits = ptr::null_mut();
        let color = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        ReleaseDC(ptr::null_mut(), dc);
        if color.is_null() {
            return ptr::null_mut();
        }
        ptr::copy_nonoverlapping(px.as_ptr(), bits.cast::<u32>(), N * N);
        let mask_bits = [0u8; N * N / 8];
        let mask = CreateBitmap(N as i32, N as i32, 1, 1, mask_bits.as_ptr().cast());
        let ii = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

fn notify(hwnd: HWND, msg: u32) {
    let live = *LATEST.lock().unwrap_or_else(|e| e.into_inner());
    let total = TOTAL_KB.get();
    let icon = make_icon(live.map(|l| (l.cpu as f32 / 100.0, pct(l.ram_kb, total))));
    // SAFETY: nid tam doldurulmuş; eski ikon kabuk yeni ikonu aldıktan sonra yok edilir.
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        let tip: Vec<u16> = tooltip(live, total).encode_utf16().collect();
        let n = tip.len().min(nid.szTip.len() - 1);
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        Shell_NotifyIconW(msg, &nid);
        let old = ICON.replace(icon);
        if !old.is_null() {
            DestroyIcon(old);
        }
    }
}

fn menu(hwnd: HWND) {
    // SAFETY: menü bu fonksiyonda oluşturulup yok edilir.
    unsafe {
        let m = CreatePopupMenu();
        // M4/M5'e kadar pasif
        AppendMenuW(m, MF_STRING | MF_GRAYED, ID_UI, wide("Arayüzü aç").as_ptr());
        AppendMenuW(
            m,
            MF_STRING | MF_GRAYED,
            ID_MUTE,
            wide("Alarmları sessize al (1 saat)").as_ptr(),
        );
        AppendMenuW(m, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(m, MF_STRING, ID_EXIT, wide("Çıkış").as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Menü dışına tıklayınca kapanması için (KB135788)
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        if cmd as usize == ID_EXIT {
            DestroyWindow(hwnd);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_DATA => notify(hwnd, NIM_MODIFY),
        WM_TRAY if lp as u32 == WM_RBUTTONUP => menu(hwnd),
        // Explorer yeniden başladı → ikon kayboldu, yeniden ekle
        m if m == TASKBAR_CREATED.get() && m != 0 => notify(hwnd, NIM_ADD),
        WM_DESTROY => {
            // SAFETY: nid yalnızca kimlik alanlarıyla; ikon silinir.
            unsafe {
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1;
                Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
            }
        }
        // SAFETY: işlenmeyen mesajlar varsayılan işleyiciye.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    0
}

fn connect() -> io::Result<File> {
    let name = wide(PIPE_NAME);
    // GENERIC_WRITE değil: pipe DACL'ı IU'ya FILE_CREATE_PIPE_INSTANCE vermiyor (ADR-0018)
    // SAFETY: name NUL sonlu; dönen handle File'a devredilir.
    unsafe {
        let h = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | FILE_WRITE_DATA,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_handle(h))
    }
}

fn post(live: Option<Live>) {
    *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = live;
    // SAFETY: pencere yok olduysa PostMessage yalnızca başarısız döner.
    unsafe { PostMessageW(HWND_.load(Ordering::Relaxed) as HWND, WM_DATA, 0, 0) };
}

fn summarize(s: &Sample) -> Live {
    Live {
        cpu: s.cpu_total,
        ram_kb: s.ram_used_kb,
        rx_bps: s.net.iter().map(|n| n.rx_bps).sum(),
    }
}

fn session(f: &mut File) -> io::Result<()> {
    proto::write(
        f,
        &ClientMsg::Hello {
            proto: PROTO,
            role: Role::Tray,
        },
    )?;
    proto::write(
        f,
        &ClientMsg::Subscribe {
            channels: CH_SAMPLE,
            rate_ms: RATE_MS,
        },
    )?;
    loop {
        match proto::read::<ServerMsg>(f)? {
            ServerMsg::Sample(s) => post(Some(summarize(&s))),
            ServerMsg::Error { code } => return Err(io::Error::other(format!("{code:?}"))),
            _ => {}
        }
    }
}

/// Servis yoksa / koparsa ikon gri, 10 s'de bir yeniden dener (spek §8).
fn pipe_loop() {
    loop {
        if let Ok(mut f) = connect() {
            let _ = session(&mut f);
        }
        post(None);
        thread::sleep(RETRY);
    }
}

fn main() {
    let mutex = wide(r"Local\LowMonitorTray");
    // SAFETY: isimli mutex süreç ömrü boyunca açık kalır (tek örnek kilidi).
    unsafe {
        CreateMutexW(ptr::null(), 0, mutex.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut ms) != 0 {
            TOTAL_KB.set(ms.ullTotalPhys / 1024);
        }
        TASKBAR_CREATED.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));

        let class = wide("LowMonitorTray");
        let inst = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // Görünmez üst düzey pencere (HWND_MESSAGE değil: SetForegroundWindow + menü için)
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            inst,
            ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        HWND_.store(hwnd as usize, Ordering::Relaxed);
        notify(hwnd, NIM_ADD);
        let _ = thread::Builder::new()
            .name("pipe".into())
            .stack_size(64 * 1024)
            .spawn(pipe_loop);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_format() {
        let l = Live {
            cpu: 1234,
            ram_kb: 8 << 20,
            rx_bps: 3 << 19,
        };
        assert_eq!(tooltip(Some(l), 16 << 20), "CPU 12% · RAM 50% · ↓1.5 MB/s");
        assert_eq!(
            tooltip(Some(Live { rx_bps: 2048, ..l }), 0),
            "CPU 12% · RAM 0% · ↓2 KB/s"
        );
        assert!(tooltip(None, 1).contains("bağlı değil"));
    }

    #[test]
    fn bars() {
        let p = pixels(Some((50.0, 95.0)));
        let col = |x: usize| (0..N).filter(|y| p[y * N + x] >> 24 == 0xFF).count();
        assert_eq!((col(1), col(9), col(0)), (8, 15, 0));
        assert_eq!(p[(N - 1) * N + 9], 0xFFEF_4444, "≥%90 kırmızı");
        assert!(
            pixels(None).iter().all(|&c| c >> 24 != 0xFF),
            "gri: dolu çubuk yok"
        );
        assert_eq!(col(16 - 1), 0);
    }
}

#[cfg(test)]
/// Elle duman testi (servis/`run` açıkken): `cargo test -p lowmon-tray -- --ignored --nocapture`
mod live_debug {
    use super::*;
    #[test]
    #[ignore]
    fn live() {
        let mut f = connect().unwrap();
        eprintln!("bağlandı");
        proto::write(
            &mut f,
            &ClientMsg::Hello {
                proto: PROTO,
                role: Role::Tray,
            },
        )
        .unwrap();
        eprintln!("hello yazıldı");
        proto::write(
            &mut f,
            &ClientMsg::Subscribe {
                channels: CH_SAMPLE,
                rate_ms: RATE_MS,
            },
        )
        .unwrap();
        eprintln!("subscribe yazıldı");
        for _ in 0..4 {
            eprintln!(
                "{:?}",
                proto::read::<ServerMsg>(&mut f)
                    .map(|m| format!("{m:?}").chars().take(80).collect::<String>())
            );
        }
    }
}
