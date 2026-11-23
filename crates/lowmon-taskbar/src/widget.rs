//! Win11 "Widget'lar" düğmesinin konumu (ADR-0040). Düğme ayrı HWND değil, taskbar'ın XAML
//! ağacında bir öğe: yalnız UI Automation görür (`AutomationId = WidgetsButton`).
use std::ffi::c_void;
use windows::Win32::{
    Foundation::HWND,
    System::{
        Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx},
        Variant::VARIANT,
    },
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_AutomationIdPropertyId,
    },
};

pub struct Uia(IUIAutomation);

impl Uia {
    pub fn new() -> Option<Self> {
        // SAFETY: parametreler geçerli. COM başlatma sonucu yok sayılır: S_OK, S_FALSE ve
        // RPC_E_CHANGED_MODE'da (thread zaten başka kipte) COM yine kullanılabilir.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .ok()
                .map(Self)
        }
    }

    /// Düğmenin sol kenarı (ekran px'i, çağıran thread'in DPI bağlamında); yoksa `None`.
    // shortcut: explorer yanıt vermezse UIA çağrısı thread'i bekletir (zaman aşımı yok); sorun çıkarsa ayrı thread'e al
    pub fn widget_left(&self, taskbar: *mut c_void) -> Option<i32> {
        // SAFETY: taskbar çağıranca doğrulanmış pencere; dönen arayüzler Drop'ta bırakılır.
        unsafe {
            let root = self.0.ElementFromHandle(HWND(taskbar)).ok()?;
            let cond = self
                .0
                .CreatePropertyCondition(
                    UIA_AutomationIdPropertyId,
                    &VARIANT::from("WidgetsButton"),
                )
                .ok()?;
            let r = root
                .FindFirst(TreeScope_Descendants, &cond)
                .ok()?
                .CurrentBoundingRectangle()
                .ok()?;
            (r.right > r.left).then_some(r.left)
        }
    }
}
