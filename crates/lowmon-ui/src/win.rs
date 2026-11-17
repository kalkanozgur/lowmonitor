//! Windows: tek örnek, yerel saat farkı, süreç taraması + sonlandırma.

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SW_RESTORE, SetForegroundWindow, ShowWindow, WM_APP,
};
use windows::core::{PCWSTR, w};

/// Otomatik sıfırlanan isimli olay: `--ai` ile açılan ikinci kopya, çalışan kopyayı AI sekmesine geçirir.
const SHOW_AI: PCWSTR = w!(r"Local\LowMonitorUi.ShowAi");

/// `SHOW_AI` her tetiklendiğinde `f` (arka iş parçacığında) çağrılır.
pub fn on_show_ai(f: impl Fn() + Send + 'static) {
    // SAFETY: isimli olay süreç ömrü boyunca açık kalır; dizge sabit.
    let Ok(ev) = (unsafe { CreateEventW(None, false, false, SHOW_AI) }) else {
        return;
    };
    let ev = ev.0 as usize; // HANDLE Send değil
    std::thread::spawn(move || {
        // SAFETY: ev yukarıda açılan, hiç kapatılmayan olay handle'ı.
        while unsafe { WaitForSingleObject(HANDLE(ev as _), INFINITE) }.0 == 0 {
            f();
        }
    });
}

/// İkinci kopya mevcut pencereyi öne getirip çıkar (tray "Arayüzü aç" tekrar tıklanınca).
/// Ayarlar kaydedildi → tray yeniden yükler (`lowmon-tray` `WM_RELOAD`). bit 0 = ai + taskbar, bit 1 = deck.
/// Tray kapalıysa `false` (ayar sonraki açılışta okunur).
pub fn tray_reload(bits: usize) -> bool {
    // SAFETY: sınıf adı sabit; pencere bulunmazsa çağrı yapılmaz.
    unsafe {
        match FindWindowW(w!("LowMonitorTray"), None) {
            Ok(h) => PostMessageW(
                Some(h),
                WM_APP + 6,
                windows::Win32::Foundation::WPARAM(bits),
                windows::Win32::Foundation::LPARAM(0),
            )
            .is_ok(),
            Err(_) => false,
        }
    }
}

pub fn single_instance(ai: bool) -> bool {
    // SAFETY: isimli mutex süreç ömrü boyunca açık kalır (handle bilerek kapatılmaz); dizgeler sabit.
    unsafe {
        let _ = CreateMutexW(None, false, w!(r"Local\LowMonitorUi"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return true;
        }
        if let Ok(h) = FindWindowW(None, w!("LowMonitor")) {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        if ai && let Ok(ev) = CreateEventW(None, false, false, SHOW_AI) {
            let _ = SetEvent(ev);
        }
    }
    false
}

/// Yerel saat farkı deck'te (AI raporu da kullanır, LowDeck ADR-0009).
pub use lowmon_deck::utc_offset_s;

// ---------------- Süreçler (LM spek §9.4, §10) ----------------

use crate::procs::exit;
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{SHOW_WINDOW_CMD, SW_HIDE, SW_SHOWNORMAL};

/// Tarama ve sonlandırma deck'te (LowDeck ADR-0009 telefonla ortak).
pub use lowmon_deck::procs::{kill, scan};

/// Handle'ı kapsam sonunda kapatır.
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: ShellExecuteExW'den alınan geçerli handle, bir kez kapatılır.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Yükseltilmiş kopya: `lowmon-ui.exe --kill <pid> <ctime>` (UAC). Aynı kontroller orada tekrar yapılır.
/// Bloklar (UAC + çıkış beklenir): çağıran ayrı thread'de çalıştırır.
pub fn kill_elevated(pid: u32, ctime: i64) -> i32 {
    runas(&format!("--kill {pid} {ctime}"), SW_HIDE, true)
}

/// Yükseltilmiş kopya: `lowmon-ui.exe --service-settings` (UAC, ADR-0030 eki); çıkışı beklenmez.
/// UAC istemi sürerken bloklar: çağıran ayrı thread'de çalıştırır.
pub fn service_settings_elevated() -> i32 {
    runas("--service-settings", SW_SHOWNORMAL, false)
}

/// `ShellExecuteExW(runas)` ile bu exe; `wait` ise çıkış kodu, değilse başlatma sonucu.
fn runas(args: &str, show: SHOW_WINDOW_CMD, wait: bool) -> i32 {
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
    else {
        return exit::FAILED;
    };
    let file: Vec<u16> = exe.encode_utf16().chain([0]).collect();
    let args: Vec<u16> = args.encode_utf16().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: show.0,
        ..Default::default()
    };
    // SAFETY: info tam dolduruldu, dizgeler çağrı boyunca yaşar.
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return if e.code() == ERROR_CANCELLED.to_hresult() {
            exit::CANCELLED
        } else {
            exit::FAILED
        };
    }
    let h = Owned(info.hProcess);
    if !wait {
        return exit::OK;
    }
    let mut code = exit::FAILED as u32;
    // SAFETY: h, SEE_MASK_NOCLOSEPROCESS ile dönen süreç handle'ı.
    unsafe {
        WaitForSingleObject(h.0, INFINITE);
        let _ = GetExitCodeProcess(h.0, &mut code);
    }
    code as i32
}

/// Servise `Control::ReloadConfig` (yalnız yükseltilmiş kopyada geçer: Admin token, spek §6.1).
/// Yanıt toplayıcı döngüsünün bir tick'i kadar sürebilir: çağıran ayrı thread'de çalıştırır.
pub fn reload_service() -> Result<(), String> {
    use lowmon_core::proto::{self, ClientMsg, Control, PIPE_NAME, PROTO, Role, ServerMsg};
    use std::os::windows::fs::OpenOptionsExt;
    // GENERIC_READ | FILE_WRITE_DATA: GENERIC_WRITE pipe DACL'ında yok (ADR-0018, tray ile aynı)
    let mut f = std::fs::OpenOptions::new()
        .access_mode(0x8000_0000 | 0x0002)
        .open(PIPE_NAME)
        .map_err(|e| format!("pipe: {e}"))?;
    let hello = ClientMsg::Hello {
        proto: PROTO,
        role: Role::Viewer,
    };
    proto::write(&mut f, &hello).map_err(|e| e.to_string())?;
    // Başarılı Hello yanıtsız; ilk çerçeve Control'ün sonucu (ya da BadHello)
    proto::write(&mut f, &ClientMsg::Control(Control::ReloadConfig)).map_err(|e| e.to_string())?;
    match proto::read::<ServerMsg>(&mut f).map_err(|e| e.to_string())? {
        ServerMsg::Ack => Ok(()),
        ServerMsg::Error { code } => Err(format!("{code:?}")),
        _ => Err("?".into()),
    }
}
