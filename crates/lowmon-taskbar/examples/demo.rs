//! Elle deneme: `cargo run -p lowmon-taskbar --example demo [saniye] [gap_px]` (varsayılan 30, 8).
//! Taskbar'a iki satır koyar; sağ tık ya da süre dolunca çıkar. Sol tık rengi değiştirir.
use lowmon_taskbar::{Bar, Config, EV_LEFT, EV_MOVED, EV_RIGHT, Line};
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const WM_BAR: u32 = WM_APP + 4;

thread_local! {
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
    static N: RefCell<u32> = const { RefCell::new(0) };
}

fn lines(n: u32) -> [Line; 2] {
    let fg = if lowmon_taskbar::light_theme() {
        0x1A1A1A
    } else {
        0xFFFFFF
    };
    [
        Line {
            text: format!("Oturum %{} · 2s14dk", 23 + n),
            rgb: if n % 2 == 1 { 0xF0A030 } else { fg },
        },
        Line {
            text: "Hafta %64 · 3g05s".into(),
            rgb: if n % 3 == 2 { 0xE5484D } else { fg },
        },
    ]
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_BAR => BAR.with_borrow_mut(|b| {
            let b = b.as_mut().unwrap();
            match wp {
                EV_MOVED => b.layout(),
                EV_LEFT => b.show(lines(N.with_borrow_mut(|n| {
                    *n += 1;
                    *n
                }))),
                EV_RIGHT => unsafe { PostQuitMessage(0) },
                _ => {}
            }
        }),
        WM_TIMER => unsafe { PostQuitMessage(0) },
        // SAFETY: varsayılan işleyici.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    0
}

fn main() {
    let secs: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let class: Vec<u16> = "LowMonitorTaskbarDemo\0".encode_utf16().collect();
    // SAFETY: demo; tutamaçlar süreç ömrünce.
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(null()),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            wc.hInstance,
            null(),
        );
        let mut cfg = Config::default();
        if let Some(g) = std::env::args().nth(2).and_then(|s| s.parse().ok()) {
            cfg.gap_px = g;
        }
        let mut bar = Bar::new(owner, WM_BAR, cfg);
        bar.show(lines(0));
        BAR.set(Some(bar));
        SetTimer(owner, 1, secs * 1000, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
        BAR.set(None);
    }
}
