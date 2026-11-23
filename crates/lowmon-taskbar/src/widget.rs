//! Win11 "Widget'lar" düğmesinin konumu (ADR-0040). Düğme ayrı HWND değil, taskbar'ın XAML
//! ağacında bir öğe: yalnız UI Automation görür (`AutomationId = WidgetsButton`).
//!
//! UIA çağrısı explorer'ın UI thread'ini bekler; gösterge pencereleri ise taskbar'ın alt pencereleri
//! (explorer onlara mesaj gönderir). Çağrı tray'in UI thread'inde olursa iki taraf birbirini
//! bekleyip explorer'ı kilitleyebilir (v1.2.2'de görüldü, `AppHangXProcB1`). Bu yüzden UIA yalnız
//! ayrı bir worker thread'de çalışır; UI thread'i önbelleğe bakar ve hiç beklemez.
use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, SyncSender, sync_channel},
    },
};
use windows::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::ScreenToClient,
    System::{
        Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
        Variant::VARIANT,
    },
    UI::{
        Accessibility::{
            CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_AutomationIdPropertyId,
        },
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
    },
};

/// Taskbar HWND → son bulunan düğme sol kenarı (taskbar istemci x'i); `None` = yok / henüz okunmadı.
type Cache = Arc<Mutex<HashMap<usize, Option<i32>>>>;

pub struct Finder {
    cache: Cache,
    tx: SyncSender<usize>,
}

impl Finder {
    /// `on_change`: worker thread'inden, bir konum değiştiğinde çağrılır (sahibe mesaj atmak için).
    pub fn new(on_change: fn()) -> Self {
        let cache = Cache::default();
        let (tx, rx) = sync_channel(8);
        let c = cache.clone();
        let _ = std::thread::Builder::new()
            .name("widget-uia".into())
            .stack_size(256 * 1024)
            .spawn(move || worker(&rx, &c, on_change));
        Self { cache, tx }
    }

    /// Önbellekteki değer, bloklamaz. İlk sorguda ya da `refresh` iken worker'a okuma isteği gider;
    /// sonuç gelince `on_change` çağrılır.
    // shortcut: worker UIA'da takılırsa (explorer yanıtsız) istekler düşer, kenar eski kalır; tray etkilenmez
    pub fn get(&self, taskbar: *mut c_void, refresh: bool) -> Option<i32> {
        let key = taskbar as usize;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let new = !cache.contains_key(&key);
        let known = *cache.entry(key).or_insert(None);
        drop(cache);
        if new || refresh {
            let _ = self.tx.try_send(key); // kuyruk doluysa düşer; sonraki yerleşim yeniden dener
        }
        known
    }
}

fn worker(rx: &Receiver<usize>, cache: &Cache, on_change: fn()) {
    // SAFETY: yalnız bu thread'in bağlamı / COM durumu; thread bitince serbest kalır.
    let uia: Option<IUIAutomation> = unsafe {
        // taskbar PMv2: UIA dikdörtgeni ve ScreenToClient gerçek piksel olsun
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    };
    let Some(uia) = uia else { return };
    while let Ok(first) = rx.recv() {
        let mut keys = vec![first];
        while let Ok(k) = rx.try_recv() {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        let mut changed = false;
        for k in keys {
            let v = find(&uia, k as *mut c_void);
            let old = cache.lock().unwrap_or_else(|e| e.into_inner()).insert(k, v);
            changed |= old != Some(v);
        }
        if changed {
            on_change();
        }
    }
}

/// Düğmenin sol kenarı, taskbar istemci x'i.
fn find(uia: &IUIAutomation, taskbar: *mut c_void) -> Option<i32> {
    let hwnd = HWND(taskbar);
    // SAFETY: taskbar tutamacı kapanmışsa çağrılar yalnız hata döner; dönen arayüzler Drop'ta bırakılır.
    unsafe {
        let root = uia.ElementFromHandle(hwnd).ok()?;
        let cond = uia
            .CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from("WidgetsButton"))
            .ok()?;
        let r = root
            .FindFirst(TreeScope_Descendants, &cond)
            .ok()?
            .CurrentBoundingRectangle()
            .ok()?;
        let mut p = POINT {
            x: r.left,
            y: r.top,
        };
        (r.right > r.left && ScreenToClient(hwnd, &mut p).as_bool()).then_some(p.x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_never_blocks_and_unknown_window_is_none() {
        let f = Finder::new(|| {});
        // geçersiz tutamaç: ilk çağrı anında None döner, worker hata yutar ve ayakta kalır
        assert_eq!(f.get(0x1234 as *mut c_void, false), None);
        assert_eq!(f.get(0x1234 as *mut c_void, true), None);
    }
}
