//! Win32 tarafı: taskbar bulma, alt pencere, olay kancası, GDI+ çizim.
use crate::{Config, EV_LEFT, EV_MOVED, EV_RIGHT, Line, anchor, pick, scale};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::{
        Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
            MONITORINFO, MonitorFromWindow, ScreenToClient, SelectObject,
        },
        GdiPlus::*,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        HiDpi::{
            DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetThreadDpiAwarenessContext,
        },
        WindowsAndMessaging::*,
    },
};

/// Sahip pencere + mesaj; pencere yordamı ve olay kancası buradan okur.
// shortcut: süreç başına tek gösterge; ikinci gösterge (trafik) gelince pencere başına veri (GWLP_USERDATA)
static OWNER: AtomicUsize = AtomicUsize::new(0);
static MSG: AtomicU32 = AtomicU32::new(0);
/// İzlenen pencereler: taskbar, bildirim alanı
static WATCH: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
/// Bekleyen EV_MOVED (animasyon sırasında yüzlerce olay → tek mesaj)
static MOVED: AtomicBool = AtomicBool::new(false);

const CLASS: &str = "LowMonitorTaskbar";
/// PixelFormat32bppPARGB (gdipluspixelformats.h; windows-sys'te yok)
const PARGB: i32 = 0x000E_200B;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn post(ev: usize) {
    // SAFETY: sahip yok olduysa PostMessage yalnız başarısız döner.
    unsafe {
        PostMessageW(
            OWNER.load(Ordering::Relaxed) as HWND,
            MSG.load(Ordering::Relaxed),
            ev,
            0,
        )
    };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // Tık taskbar'ı (explorer) öne almasın
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_LBUTTONUP => {
            post(EV_LEFT);
            0
        }
        WM_RBUTTONUP => {
            post(EV_RIGHT);
            0
        }
        // SAFETY: işlenmeyen mesajlar varsayılan işleyiciye.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

unsafe extern "system" fn on_event(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    obj: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let h = hwnd as usize;
    if obj == OBJID_WINDOW
        && h != 0
        && WATCH.iter().any(|w| w.load(Ordering::Relaxed) == h)
        && !MOVED.swap(true, Ordering::Relaxed)
    {
        post(EV_MOVED);
    }
}

/// Bu kapsamda thread per-monitor-v2: taskbar (explorer) PMv2, koordinat/DPI gerçek piksel olsun.
/// Tray süreci DPI-habersiz kalır (menü/ikon değişmez).
struct Pmv2(DPI_AWARENESS_CONTEXT);

impl Pmv2 {
    fn enter() -> Self {
        // SAFETY: yalnız bu thread'in bağlamı; Drop'ta geri alınır.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for Pmv2 {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: önceki bağlam geri yüklenir.
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

/// Taskbar açık temada mı (`SystemUsesLightTheme`); okunamazsa koyu.
pub fn light_theme() -> bool {
    let mut v: u32 = 0;
    let mut n = size_of::<u32>() as u32;
    // SAFETY: v/n geçerli; dizgeler NUL sonlu.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut v).cast(),
            &mut n,
        )
    };
    r == 0 && v != 0
}

fn class_is(hwnd: HWND, names: &[&str]) -> bool {
    let mut buf = [0u16; 64];
    // SAFETY: buf 64 öğe.
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
    let cls = String::from_utf16_lossy(&buf[..n]);
    names.contains(&cls.as_str())
}

/// Seçili monitörün taskbar'ı.
fn find_taskbar(cfg: &Config) -> HWND {
    unsafe extern "system" fn mon(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> i32 {
        let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        // SAFETY: lp = &mut Vec, EnumDisplayMonitors süresince yaşar.
        unsafe {
            if GetMonitorInfoW(m, &mut mi) != 0 {
                (*(lp as *mut Vec<(HMONITOR, i32, bool)>)).push((
                    m,
                    mi.rcMonitor.left,
                    mi.dwFlags & MONITORINFOF_PRIMARY != 0,
                ));
            }
        }
        1
    }
    unsafe extern "system" fn win(h: HWND, lp: LPARAM) -> i32 {
        // SAFETY: lp = &mut (HMONITOR, HWND), EnumWindows süresince yaşar.
        let c = unsafe { &mut *(lp as *mut (HMONITOR, HWND)) };
        if class_is(h, &["Shell_TrayWnd", "Shell_SecondaryTrayWnd"])
            // SAFETY: h EnumWindows'tan.
            && unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) } == c.0
        {
            c.1 = h;
            return 0;
        }
        1
    }
    let mut mons: Vec<(HMONITOR, i32, bool)> = Vec::new();
    // SAFETY: geri çağrılar yalnız çağrı süresince, verilen işaretçilerle çalışır.
    unsafe {
        EnumDisplayMonitors(null_mut(), null(), Some(mon), (&raw mut mons) as LPARAM);
        let keys: Vec<(i32, bool)> = mons.iter().map(|m| (m.1, m.2)).collect();
        let Some(i) = pick(&keys, cfg.monitor) else {
            return null_mut();
        };
        let mut ctx: (HMONITOR, HWND) = (mons[i].0, null_mut());
        EnumWindows(Some(win), (&raw mut ctx) as LPARAM);
        ctx.1
    }
}

/// GDI+ ile çizilmiş PARGB DIB, bellek DC'sine seçili.
struct Image {
    dc: HDC,
    bmp: *mut core::ffi::c_void,
    old: *mut core::ffi::c_void,
    w: i32,
    h: i32,
}

impl Drop for Image {
    fn drop(&mut self) {
        // SAFETY: render'da oluşturulan nesneler, bir kez.
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

/// GDI+ nesnelerini Drop'ta siler (Status hataları sessiz: çizim olmaz, gösterge boş kalır).
struct Gp<T>(*mut T, unsafe extern "system" fn(*mut T) -> Status);

impl<T> Drop for Gp<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Gdip*Create ile alınmış, bir kez silinir.
            unsafe { (self.1)(self.0) };
        }
    }
}

fn draw(lines: &[Line; 2], h: i32, dpi: u32, cfg: &Config) -> Option<Image> {
    let text: Vec<Vec<u16>> = lines
        .iter()
        .map(|l| l.text.encode_utf16().collect())
        .collect();
    // SAFETY: tüm GDI+/GDI nesneleri bu fonksiyonda (Gp/Image ile) sahiplenilir; tampon DIB'in.
    unsafe {
        let mut fam = null_mut();
        if GdipCreateFontFamilyFromName(wide(&cfg.font).as_ptr(), null_mut(), &mut fam) != Ok {
            GdipGetGenericFontFamilySansSerif(&mut fam);
        }
        let fam = Gp(fam, GdipDeleteFontFamily);
        let mut font = null_mut();
        let px = cfg.font_pt * dpi as f32 / 72.0;
        GdipCreateFont(fam.0, px, FontStyleRegular, UnitPixel, &mut font);
        let font = Gp(font, GdipDeleteFont);
        let mut fmt = null_mut();
        GdipCreateStringFormat(StringFormatFlagsNoWrap, 0, &mut fmt);
        let fmt = Gp(fmt, GdipDeleteStringFormat);
        GdipSetStringFormatAlign(fmt.0, StringAlignmentFar);
        if font.0.is_null() || fmt.0.is_null() {
            return None;
        }

        // Ölçüm: ekran uyumlu DC üzerinde
        let dc = CreateCompatibleDC(null_mut());
        let mut g = null_mut();
        GdipCreateFromHDC(dc, &mut g);
        let g = Gp(g, GdipDeleteGraphics);
        GdipSetTextRenderingHint(g.0, TextRenderingHintAntiAliasGridFit);
        let big = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 10_000.0,
            Height: 1_000.0,
        };
        let (mut tw, mut lh) = (0f32, 0f32);
        for t in &text {
            let mut b = RectF::default();
            GdipMeasureString(
                g.0,
                t.as_ptr(),
                t.len() as i32,
                font.0,
                &big,
                fmt.0,
                &mut b,
                null_mut(),
                null_mut(),
            );
            tw = tw.max(b.Width);
            lh = lh.max(b.Height);
        }
        drop(g);
        let pad = scale(3, dpi);
        let w = tw.ceil() as i32 + 2 * pad;
        let h = h.max(1);

        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // üstten aşağı
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits = null_mut();
        let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let img = Image {
            dc,
            bmp,
            old: SelectObject(dc, bmp),
            w,
            h,
        };

        let mut bitmap: *mut GpBitmap = null_mut();
        GdipCreateBitmapFromScan0(w, h, w * 4, PARGB, bits.cast(), &mut bitmap);
        let bitmap = Gp(bitmap.cast::<GpImage>(), GdipDisposeImage);
        let mut g = null_mut();
        GdipGetImageGraphicsContext(bitmap.0, &mut g);
        let g = Gp(g, GdipDeleteGraphics);
        GdipSetTextRenderingHint(g.0, TextRenderingHintAntiAliasGridFit);
        // Alfa 1: saydam görünür ama tıklanabilir (alfa 0 tıkları taskbar'a geçirir)
        GdipGraphicsClear(g.0, 0x0100_0000);
        let y0 = ((h as f32 - 2.0 * lh) / 2.0).max(0.0);
        for (i, (t, l)) in text.iter().zip(lines).enumerate() {
            let mut brush: *mut GpSolidFill = null_mut();
            GdipCreateSolidFill(0xFF00_0000 | l.rgb, &mut brush);
            let brush = Gp(brush.cast::<GpBrush>(), GdipDeleteBrush);
            let r = RectF {
                X: 0.0,
                Y: y0 + i as f32 * lh,
                Width: (w - pad) as f32,
                Height: lh,
            };
            GdipDrawString(g.0, t.as_ptr(), t.len() as i32, font.0, &r, fmt.0, brush.0);
        }
        drop(g); // flush → bits
        drop(bitmap);
        Some(img)
    }
}

/// Taskbar göstergesi. Sahip pencerenin thread'inde oluşturulur ve kullanılır.
pub struct Bar {
    cfg: Config,
    gdip: usize,
    hwnd: HWND,
    taskbar: HWND,
    notify: HWND,
    hook: HWINEVENTHOOK,
    lines: Option<[Line; 2]>,
}

impl Bar {
    /// Olaylar `PostMessage(owner, msg, EV_*, 0)` ile gelir. Pencere ilk `show`'da kurulur.
    pub fn new(owner: HWND, msg: u32, cfg: Config) -> Self {
        OWNER.store(owner as usize, Ordering::Relaxed);
        MSG.store(msg, Ordering::Relaxed);
        let mut gdip = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let class = wide(CLASS);
        // SAFETY: input geçerli; sınıf adı süreç ömrünce statik değil ama RegisterClassW kopyalar.
        unsafe {
            GdiplusStartup(&mut gdip, &input, null_mut());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: GetModuleHandleW(null()),
                hCursor: LoadCursorW(null_mut(), IDC_HAND),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc); // ikinci Bar'da "zaten var" hatası zararsız
        }
        Self {
            cfg,
            gdip,
            hwnd: null_mut(),
            taskbar: null_mut(),
            notify: null_mut(),
            hook: null_mut(),
            lines: None,
        }
    }

    /// İki satırı gösterir (gerekirse pencereyi taskbar'a kurar).
    pub fn show(&mut self, lines: [Line; 2]) {
        self.lines = Some(lines);
        self.layout();
    }

    pub fn hide(&mut self) {
        self.lines = None;
        if !self.hwnd.is_null() {
            // SAFETY: kendi penceremiz.
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
    }

    /// Explorer yeniden başladı (`TaskbarCreated`): eski pencere taskbar'la gitti, yeniden kur.
    pub fn reset(&mut self) {
        self.detach();
        self.layout();
    }

    /// `EV_MOVED`, tema, DPI: yeniden konumla + çiz.
    pub fn layout(&mut self) {
        MOVED.store(false, Ordering::Relaxed);
        let Some(lines) = self.lines.clone() else {
            return;
        };
        let _dpi = Pmv2::enter();
        if !self.alive() {
            self.detach();
            if !self.attach() {
                return;
            }
        }
        // SAFETY: tutamaçlar alive() ile doğrulandı; yapılar yerel.
        unsafe {
            let mut cr: RECT = std::mem::zeroed();
            GetClientRect(self.taskbar, &mut cr);
            let (cw, ch) = (cr.right, cr.bottom);
            if ch <= 0 || ch > cw {
                ShowWindow(self.hwnd, SW_HIDE); // dikey taskbar desteklenmiyor (C++ ile aynı)
                return;
            }
            let dpi = GetDpiForWindow(self.taskbar).max(96);
            let mut nr: RECT = std::mem::zeroed();
            let notify_left = (!self.notify.is_null()
                && IsWindowVisible(self.notify) != 0
                && GetWindowRect(self.notify, &mut nr) != 0)
                .then(|| {
                    let mut p = POINT {
                        x: nr.left,
                        y: nr.top,
                    };
                    ScreenToClient(self.taskbar, &mut p);
                    p.x
                });
            let right = anchor(
                cw,
                notify_left,
                scale(self.cfg.gap_px, dpi),
                scale(self.cfg.reserved_px, dpi),
            );
            let Some(img) = draw(&lines, ch, dpi, &self.cfg) else {
                return;
            };
            let size = SIZE {
                cx: img.w,
                cy: img.h,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                null(),
                &size,
                img.dc,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
            // HWND_TOP: taskbar'ın XAML içerik penceresinin üstünde kal
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                right - img.w,
                0,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            );
            ShowWindow(self.hwnd, SW_SHOWNA);
        }
    }

    fn alive(&self) -> bool {
        // SAFETY: IsWindow geçersiz tutamaçta 0 döner.
        !self.hwnd.is_null() && unsafe { IsWindow(self.hwnd) != 0 && IsWindow(self.taskbar) != 0 }
    }

    fn attach(&mut self) -> bool {
        let tb = find_taskbar(&self.cfg);
        if tb.is_null() {
            return false;
        }
        let class = wide(CLASS);
        // SAFETY: tb geçerli üst düzey pencere; oluşturulan pencere/kanca detach'te bırakılır.
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE,
                class.as_ptr(),
                null(),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                tb,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if hwnd.is_null() {
                return false;
            }
            let notify = FindWindowExW(tb, null_mut(), wide("TrayNotifyWnd").as_ptr(), null());
            let mut pid = 0;
            let tid = GetWindowThreadProcessId(tb, &mut pid);
            self.hook = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                null_mut(),
                Some(on_event),
                pid,
                tid,
                WINEVENT_OUTOFCONTEXT,
            );
            WATCH[0].store(tb as usize, Ordering::Relaxed);
            WATCH[1].store(notify as usize, Ordering::Relaxed);
            (self.hwnd, self.taskbar, self.notify) = (hwnd, tb, notify);
        }
        true
    }

    fn detach(&mut self) {
        // SAFETY: kendi kancamız/penceremiz; explorer gittiyse DestroyWindow yalnız başarısız döner.
        unsafe {
            if !self.hook.is_null() {
                UnhookWinEvent(self.hook);
            }
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
        for w in &WATCH {
            w.store(0, Ordering::Relaxed);
        }
        (self.hwnd, self.taskbar, self.notify, self.hook) =
            (null_mut(), null_mut(), null_mut(), null_mut());
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.detach();
        if self.gdip != 0 {
            // SAFETY: GdiplusStartup'tan alınan jeton, tüm GDI+ nesneleri silindi.
            unsafe { GdiplusShutdown(self.gdip) };
        }
    }
}
