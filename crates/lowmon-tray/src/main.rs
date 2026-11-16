//! lowmon-tray (spek §8): tray ikonu (CPU/RAM çift çubuk) + tooltip, pipe'tan 2 s'de bir Sample.
//! Alarm (ADR-0028): `CH_EVENT` abonesi; AlarmRaise → balon (1 saat sessize alınabilir).
//! Telefon sunucusu (ADR-0031): `lowmon-deck` aynı süreçte; menüde "Telefon ▸", tooltip'te bağlı telefon.
//! AI kullanımı (ADR-0032): `ai` modülü; taskbar göstergesi + menüde "AI kullanımı ▸".
//! Spekten farkı: windows-rs yerine windows-sys (diğer crate'lerle aynı, ADR-0018).
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod ai;
mod autostart;

use lowmon_core::{
    metrics::{EventKind, Sample},
    proto::{self, CH_EVENT, CH_SAMPLE, ClientMsg, PIPE_NAME, PROTO, Role, ServerMsg},
};
use std::{
    cell::{Cell, RefCell},
    fs::File,
    io,
    os::windows::io::FromRawHandle,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
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
        SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::CreateMutexW,
    },
    UI::{
        Shell::{
            NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE,
            NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
            DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
            HICON, HMENU, ICONINFO, MB_ICONERROR, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
            MF_STRING, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
            RegisterWindowMessageW, SW_HIDE, SW_SHOWNORMAL, SetForegroundWindow, TPM_NONOTIFY,
            TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY,
            WM_DISPLAYCHANGE, WM_NULL, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW,
        },
    },
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_DATA: u32 = WM_APP + 2;
const ID_UI: usize = 1;
const ID_MUTE: usize = 2;
const ID_EXIT: usize = 3;
const ID_AUTO: usize = 4;
const ID_CFG: usize = 5;
const ID_RELOAD: usize = 6;
const ID_LOG: usize = 7;
const ID_FIREWALL: usize = 8;
const RATE_MS: u32 = 2_000;
const RETRY: Duration = Duration::from_secs(10);
const MUTE_MS: u64 = 3_600_000;
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
/// Şu an Raised kural id'leri (servisten `Alarms` + olaylar).
static ALARMS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Gösterilmeyi bekleyen balon metni.
static BALLOON: Mutex<Option<String>> = Mutex::new(None);
/// Sessiz bitişi (GetTickCount64 ms); 0 = sessiz değil. Yalnız tray belleğinde.
static MUTE_UNTIL: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static ICON: Cell<HICON> = const { Cell::new(ptr::null_mut()) };
    static TOTAL_KB: Cell<u64> = const { Cell::new(0) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    /// Telefon sunucusu; yalnız UI thread'inde (menü, tooltip).
    static DECK: RefCell<Option<lowmon_deck::Running>> = const { RefCell::new(None) };
    /// Son dinleme hatası (port kullanımda vb.); menüde gösterilir.
    static DECK_ERR: RefCell<Option<String>> = const { RefCell::new(None) };
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

fn tooltip(live: Option<Live>, total_kb: u64, alarms: &[String], phone: Option<&str>) -> String {
    let mut s: String = match live {
        None => "LowMonitor: servise bağlı değil".into(),
        Some(l) => format!(
            "CPU {:.0}% · RAM {:.0}% · ↓{}",
            l.cpu as f32 / 100.0,
            pct(l.ram_kb, total_kb),
            rate(l.rx_bps)
        ),
    };
    if let Some(p) = phone {
        s += &format!(" · 📱 {p}");
    }
    if !alarms.is_empty() && live.is_some() {
        s += &format!(" · ⚠ {}", alarms.join(", "));
    }
    s
}

/// Olayı aktif listeye uygular; balon gösterilecekse metnini döner (sessizlik çağıranda).
fn on_event(alarms: &mut Vec<String>, kind: EventKind, name: &str, value: i64) -> Option<String> {
    match kind {
        EventKind::AlarmRaise => {
            if !alarms.iter().any(|a| a == name) {
                alarms.push(name.to_owned());
            }
            Some(format!("{name}: {:.1}", value as f64 / 100.0))
        }
        EventKind::AlarmClear => {
            alarms.retain(|a| a != name);
            None
        }
        EventKind::System => None,
    }
}

/// Bağlanışta aktif liste boş değilse tek balon.
fn snapshot_text(alarms: &[String]) -> Option<String> {
    (!alarms.is_empty()).then(|| format!("Aktif alarm: {}", alarms.join(", ")))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn muted() -> bool {
    // SAFETY: parametresiz, yan etkisiz.
    let now = unsafe { GetTickCount64() };
    now < MUTE_UNTIL.load(Ordering::Relaxed)
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
        let phone = DECK.with_borrow(|d| d.as_ref().and_then(|d| d.hub.phone_name()));
        let mut tip = tooltip(live, total, &lock(&ALARMS), phone.as_deref());
        if let Some(a) = ai::short() {
            tip += &format!(" · AI {a}");
        }
        let tip: Vec<u16> = tip.encode_utf16().collect();
        let n = tip.len().min(nid.szTip.len() - 1);
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        if let Some(text) = lock(&BALLOON).take() {
            nid.uFlags |= NIF_INFO;
            nid.dwInfoFlags = NIIF_WARNING;
            for (dst, src) in [
                (&mut nid.szInfoTitle[..], "LowMonitor alarmı"),
                (&mut nid.szInfo[..], &text),
            ] {
                let w: Vec<u16> = src.encode_utf16().collect();
                let n = w.len().min(dst.len() - 1);
                dst[..n].copy_from_slice(&w[..n]);
            }
        }
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
        let ui = autostart::ui();
        let grayed = if matches!(ui, autostart::Ui::Ready(_)) {
            0
        } else {
            MF_GRAYED
        };
        AppendMenuW(m, MF_STRING | grayed, ID_UI, wide(ui.label()).as_ptr());
        AppendMenuW(
            m,
            MF_STRING | if muted() { MF_CHECKED } else { 0 },
            ID_MUTE,
            wide("Alarmları sessize al (1 saat)").as_ptr(),
        );
        AppendMenuW(
            m,
            MF_STRING | MF_POPUP,
            phone_menu() as usize,
            wide("Telefon").as_ptr(),
        );
        AppendMenuW(
            m,
            MF_STRING | MF_POPUP,
            ai::menu() as usize,
            wide("AI kullanımı").as_ptr(),
        );
        let checked = if autostart::enabled() { MF_CHECKED } else { 0 };
        AppendMenuW(
            m,
            MF_STRING | checked,
            ID_AUTO,
            wide("Windows ile başlat").as_ptr(),
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
        DestroyMenu(m); // alt menüyü de yok eder
        match cmd as usize {
            ID_EXIT => {
                DestroyWindow(hwnd);
            }
            // Uygulama tek örnek: zaten açıksa kendi penceresini öne getirip çıkar.
            // Exe yoksa menü zaten pasif (etiket nedeni söyler); menü açıkken silinmesi ihmal.
            ID_UI => {
                if let autostart::Ui::Ready(exe) = ui {
                    // ShellExecuteW: std::process::Command exe'ye ~75 KB ekliyordu.
                    ShellExecuteW(
                        hwnd,
                        ptr::null(),
                        wide(&exe).as_ptr(),
                        ptr::null(),
                        ptr::null(),
                        SW_SHOWNORMAL,
                    );
                }
            }
            ID_MUTE => {
                let until = if muted() {
                    0
                } else {
                    GetTickCount64() + MUTE_MS
                };
                MUTE_UNTIL.store(until, Ordering::Relaxed);
            }
            ID_CFG => open_notepad(hwnd, &lowmon_deck::config_path()),
            ai::ID_NOW => ai::refresh_now(),
            ai::ID_CFG => open_notepad(hwnd, &lowmon_ai::config_path()),
            ai::ID_RELOAD => ai::start(hwnd),
            ai::ID_KEY_SAVE => ai::save_key(hwnd),
            ai::ID_KEY_DEL => ai::delete_key(),
            ID_LOG => open_notepad(hwnd, &lowmon_deck::config_path().with_file_name("deck.log")),
            ID_RELOAD => {
                deck_reload(true);
                notify(hwnd, NIM_MODIFY);
            }
            // UAC tek kez: cmd.exe yükseltilir, tray yükseltilmemiş kalır. Sonuç dönmez (runas asenkron).
            ID_FIREWALL => {
                let port =
                    DECK.with_borrow(|d| d.as_ref().and_then(|d| d.config()).map(|c| c.port));
                let exe = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.to_str().map(String::from));
                if let (Some(port), Some(exe)) = (port, exe) {
                    let args = wide(&lowmon_deck::firewall_args(port, &exe));
                    ShellExecuteW(
                        hwnd,
                        wide("runas").as_ptr(),
                        wide("cmd.exe").as_ptr(),
                        args.as_ptr(),
                        ptr::null(),
                        SW_HIDE,
                    );
                }
            }
            ID_AUTO => {
                // shortcut: kayıt hatası sessiz; menü bir sonraki açılışta gerçek durumu gösterir
                let cmd = (!autostart::enabled()).then(autostart::this_exe);
                let _ = match cmd {
                    Some(Ok(c)) => autostart::set(autostart::VALUE, Some(&c)),
                    Some(Err(e)) => Err(e),
                    None => autostart::set(autostart::VALUE, None),
                };
            }
            _ => {}
        }
    }
}

/// "Telefon ▸": durum satırları (pasif) + ayar/log/yeniden yükle/firewall.
/// Dönen menü üst menüye eklenir, onunla birlikte yok edilir.
fn phone_menu() -> HMENU {
    let (lines, listening) = DECK.with_borrow(|d| {
        let d = d.as_ref();
        match d.and_then(|d| d.config()) {
            Some(c) => {
                let who = d.and_then(|d| d.hub.phone_name());
                let who = who.map_or("Bekliyor".into(), |n| format!("Bağlı: {n}"));
                (
                    vec![format!("⚙  {}  ·  {}  ·  {}", c.bind, c.port, c.token), who],
                    true,
                )
            }
            None => {
                let e = DECK_ERR.with_borrow(|e| e.clone()).unwrap_or_default();
                (
                    vec![format!("Kapalı: {}", e.lines().next().unwrap_or(""))],
                    false,
                )
            }
        }
    });
    // SAFETY: dizgeler çağrı boyunca yaşar; menü tutamacı çağırana devredilir.
    unsafe {
        let sub = CreatePopupMenu();
        for l in &lines {
            AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, wide(l).as_ptr());
        }
        AppendMenuW(sub, MF_SEPARATOR, 0, ptr::null());
        for (id, text) in [
            (ID_CFG, "Ayar dosyasını aç"),
            (ID_RELOAD, "Ayarı yeniden yükle"),
            (ID_LOG, "Log'u aç"),
        ] {
            AppendMenuW(sub, MF_STRING, id, wide(text).as_ptr());
        }
        let fw = if listening { 0 } else { MF_GRAYED };
        AppendMenuW(
            sub,
            MF_STRING | fw,
            ID_FIREWALL,
            wide("Firewall kuralı ekle (yönetici)…").as_ptr(),
        );
        sub
    }
}

fn open_notepad(hwnd: HWND, path: &std::path::Path) {
    // SAFETY: NUL sonlu dizgeler çağrı boyunca yaşar.
    unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide("notepad.exe").as_ptr(),
            wide(&path.display().to_string()).as_ptr(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// `deck.toml`'u (yoksa `agent.cfg` göçü) okuyup dinlemeyi (yeniden) açar. Hata menüde görünür;
/// `show` (kullanıcı tıkladı) ise ayrıca kutu. Açılışta kutu yok: port kurulu agent'ta olabilir (ADR-0031).
fn deck_reload(show: bool) {
    let r = lowmon_deck::load_or_create(&lowmon_deck::config_path(), &lowmon_deck::legacy_path())
        .map_err(|e| e.to_string())
        .and_then(|c| DECK.with_borrow_mut(|d| d.as_mut().map_or(Ok(()), |d| d.reload(c))));
    let err = r.err();
    if let Some(e) = &err {
        lowmon_deck::log::write(&format!("telefon dinlenemedi: {e}"));
        if show {
            // SAFETY: NUL sonlu dizgeler; sahipsiz modal kutu.
            unsafe {
                MessageBoxW(
                    ptr::null_mut(),
                    wide(e).as_ptr(),
                    wide("LowMonitor telefon").as_ptr(),
                    MB_ICONERROR,
                )
            };
        }
    }
    DECK_ERR.replace(err);
}

/// WM_SETTINGCHANGE lParam'ı (NUL sonlu UTF-16 ya da NULL) `name` mi?
fn setting_is(lp: LPARAM, name: &str) -> bool {
    if lp == 0 {
        return false;
    }
    let p = lp as *const u16;
    // SAFETY: sistem lParam'ı NUL sonlu dizge; en fazla name.len()+1 öğe okunur.
    let got = (0..=name.len())
        .map(|i| unsafe { *p.add(i) })
        .take_while(|&c| c != 0);
    got.eq(name.encode_utf16())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_DATA => notify(hwnd, NIM_MODIFY),
        WM_TRAY if lp as u32 == WM_RBUTTONUP => menu(hwnd),
        ai::WM_AI | WM_DISPLAYCHANGE => ai::render(),
        WM_TIMER if wp == ai::TIMER => ai::render(),
        ai::WM_BAR => {
            if ai::on_bar(wp) {
                menu(hwnd);
            }
        }
        // Taskbar açık/koyu tema değişti → metin rengi
        WM_SETTINGCHANGE if setting_is(lp, "ImmersiveColorSet") => ai::render(),
        // Explorer yeniden başladı → ikon ve taskbar göstergesi kayboldu, yeniden kur
        m if m == TASKBAR_CREATED.get() && m != 0 => {
            notify(hwnd, NIM_ADD);
            ai::taskbar_created();
        }
        WM_DESTROY => {
            ai::stop(hwnd);
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
    post_redraw();
}

fn post_redraw() {
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
            channels: CH_SAMPLE | CH_EVENT,
            rate_ms: RATE_MS,
        },
    )?;
    loop {
        match proto::read::<ServerMsg>(f)? {
            ServerMsg::Sample(s) => post(Some(summarize(&s))),
            ServerMsg::Alarms(a) => {
                // Bağlanmadan önce Raise olmuş alarmlar (tray geç açıldı / servis yeniden başladı)
                if let Some(text) = snapshot_text(&a).filter(|_| !muted()) {
                    *lock(&BALLOON) = Some(text);
                }
                *lock(&ALARMS) = a;
                let live = *lock(&LATEST);
                post(live);
            }
            ServerMsg::Event {
                kind, name, value, ..
            } => {
                let text = on_event(&mut lock(&ALARMS), kind, &name, value);
                if let Some(text) = text.filter(|_| !muted()) {
                    *lock(&BALLOON) = Some(text);
                }
                let live = *lock(&LATEST);
                post(live); // tooltip / balon hemen
            }
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
        lock(&ALARMS).clear();
        post(None);
        thread::sleep(RETRY);
    }
}

/// `--autostart on|off`: kurulum betiği için; pencere açmaz. Çıkış kodu 0 başarı, 1 kayıt hatası, 2 kullanım.
fn cli(args: &[String]) -> Option<i32> {
    let on = match args {
        [] => return None,
        [a, v] if a == "--autostart" && v == "on" => true,
        [a, v] if a == "--autostart" && v == "off" => false,
        _ => return Some(2),
    };
    let r = if on {
        autostart::this_exe().and_then(|c| autostart::set(autostart::VALUE, Some(&c)))
    } else {
        autostart::set(autostart::VALUE, None)
    };
    Some(i32::from(r.is_err()))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }
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
        lowmon_deck::log::init(lowmon_deck::config_path().with_file_name("deck.log"));
        let deck = lowmon_deck::start();
        // Telefon bağlandı/koptu → tooltip (UI thread'inde WM_DATA ile)
        let _ = deck.hub.on_client.set(Box::new(post_redraw));
        DECK.set(Some(deck));
        deck_reload(false);
        notify(hwnd, NIM_ADD);
        ai::start(hwnd);
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
        assert_eq!(
            tooltip(Some(l), 16 << 20, &[], None),
            "CPU 12% · RAM 50% · ↓1.5 MB/s"
        );
        assert_eq!(
            tooltip(
                Some(l),
                16 << 20,
                &["cpu_high".into(), "wan_rx".into()],
                Some("toco")
            ),
            "CPU 12% · RAM 50% · ↓1.5 MB/s · 📱 toco · ⚠ cpu_high, wan_rx"
        );
        assert_eq!(
            tooltip(Some(Live { rx_bps: 2048, ..l }), 0, &[], None),
            "CPU 12% · RAM 0% · ↓2 KB/s"
        );
        assert_eq!(
            tooltip(None, 1, &["x".into()], Some("toco")),
            "LowMonitor: servise bağlı değil · 📱 toco"
        );
    }

    #[test]
    fn events_update_active_list() {
        let mut a = vec!["old".to_string()];
        let b = on_event(&mut a, EventKind::AlarmRaise, "cpu_high", 9350);
        assert_eq!(b.as_deref(), Some("cpu_high: 93.5"));
        on_event(&mut a, EventKind::AlarmRaise, "cpu_high", 9400); // yinelenen Raise
        assert_eq!(a, ["old", "cpu_high"]);
        assert_eq!(on_event(&mut a, EventKind::AlarmClear, "old", 0), None);
        assert_eq!(on_event(&mut a, EventKind::System, "write_error", 0), None);
        assert_eq!(a, ["cpu_high"]);
        assert_eq!(snapshot_text(&[]), None);
        assert_eq!(snapshot_text(&a).as_deref(), Some("Aktif alarm: cpu_high"));
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
                channels: CH_SAMPLE | CH_EVENT,
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
