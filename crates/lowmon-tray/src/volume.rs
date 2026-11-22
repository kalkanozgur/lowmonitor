//! lowvolume (ADR-0039): taskbar üstünde tekerlek → sistem sesi; aynı süreçte, menüde "Ses ▸".

use crate::wide;
use lowmon_ai::text::tr;
use std::cell::RefCell;
use std::ptr;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, HMENU, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
};

pub const ID_ENABLED: usize = 40;
pub const ID_CFG: usize = 41;
pub const ID_RELOAD: usize = 42;

thread_local! {
    /// Yalnız UI thread'inde. Err = çalışmıyor (neden menüde).
    static VOL: RefCell<Result<lowvolume::Handle, String>> = const { RefCell::new(Err(String::new())) };
}

fn load() -> lowvolume::Config {
    let (cfg, err) = lowvolume::Config::load(&lowvolume::Config::path());
    if let Some(e) = err {
        lowmon_deck::log::write(&format!("lowvolume.toml: {e}"));
    }
    cfg
}

/// Tray açılışında ve çalışmıyorken "Yeniden yükle"de. Standalone `lowvolume.exe` çalışıyorsa hook'u o tutar.
pub fn start() {
    let r = lowvolume::start(&load()).map_err(|e| match e {
        lowvolume::Error::AlreadyRunning => tr(
            "Standalone lowvolume çalışıyor",
            "Standalone lowvolume running",
        )
        .to_string(),
        e => {
            lowmon_deck::log::write(&format!("lowvolume: {e}"));
            e.to_string()
        }
    });
    VOL.set(r);
}

/// Çalışıyorsa config'i uygular; değilse (standalone kapandıysa) yeniden başlatmayı dener.
pub fn reload() {
    let running = VOL.with_borrow(|v| v.as_ref().map(|h| h.reload(&load())).is_ok());
    if !running {
        start();
    }
}

/// WM_DESTROY'da: hook'u süreç çıkışından önce kaldırır. Aksi hâlde TLS yıkıcısı, ExitProcess'in
/// sonlandırdığı thread'leri join etmeye çalışır → "threads should not terminate unexpectedly".
pub fn stop() {
    VOL.set(Err(String::new()));
}

pub fn toggle() {
    VOL.with_borrow(|v| {
        if let Ok(h) = v {
            h.set_enabled(!h.enabled());
        }
    });
}

/// "Ses ▸" alt menüsü; üst menüyle birlikte yok edilir.
pub fn menu() -> HMENU {
    let (status, enabled) = VOL.with_borrow(|v| match v {
        Ok(h) if h.enabled() => (tr("Çalışıyor", "Running").to_string(), Some(true)),
        Ok(_) => (tr("Devre dışı", "Disabled").to_string(), Some(false)),
        Err(e) => (format!("{}: {e}", tr("Kapalı", "Off")), None),
    });
    let enabled_flags = match enabled {
        Some(true) => MF_STRING | MF_CHECKED,
        Some(false) => MF_STRING,
        None => MF_STRING | MF_GRAYED,
    };
    // SAFETY: dizgeler çağrı boyunca yaşar; menü tutamacı çağırana devredilir.
    unsafe {
        let sub = CreatePopupMenu();
        AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, wide(&status).as_ptr());
        AppendMenuW(sub, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            sub,
            enabled_flags,
            ID_ENABLED,
            wide(tr("Etkin", "Enabled")).as_ptr(),
        );
        for (id, text) in [
            (ID_CFG, tr("Ayar dosyasını aç", "Open config file")),
            (ID_RELOAD, tr("Ayarı yeniden yükle", "Reload config")),
        ] {
            AppendMenuW(sub, MF_STRING, id, wide(text).as_ptr());
        }
        sub
    }
}
