//! AI kullanımı (ADR-0032 Faz 2): `lowmon-ai` işçisi + `lowmon-taskbar` göstergesi, UI thread'inde.
//! Gösterge iki blokludur (ADR-0033): sistem ızgarası (`sys`, blok 0) ve AI (blok 1). Aynı taskbar'da
//! sistem sağda, AI solunda; varsayılan ayarda sistem ızgarası ikinci monitör takılıysa oradadır.
//! İşçi durum değişince `WM_AI` gönderir; kalan süre metni için 30 s'lik zamanlayıcı yeniden çizer.
use lowmon_ai::{Config, Running, State, api::Window, text, text::tr, usage, usage::Level};
use lowmon_deck::{AI_TEXT_MAX, AiUsage, AiWindow};
use lowmon_taskbar::{Bar, EV_LEFT, EV_MOVED, EV_RIGHT, Grid, Line};
use std::{cell::RefCell, ptr};
use windows_sys::Win32::{
    Foundation::HWND,
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalUnlock},
    },
    UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, HMENU, KillTimer, MB_ICONERROR, MB_ICONINFORMATION,
        MB_ICONWARNING, MF_GRAYED, MF_SEPARATOR, MF_STRING, MessageBoxW, PostMessageW, SetTimer,
        WM_APP,
    },
};

pub const WM_AI: u32 = WM_APP + 3;
pub const WM_BAR: u32 = WM_APP + 4;
pub const TIMER: usize = 0xA1;
pub const ID_NOW: usize = 20;
pub const ID_CFG: usize = 21;
pub const ID_RELOAD: usize = 22;
pub const ID_KEY_SAVE: usize = 23;
pub const ID_KEY_DEL: usize = 24;
pub const ID_REPORT: usize = 25;
const TICK_MS: u32 = 30_000;
/// AI bloğu: sistem ızgarasının (`sys::BLOCK` = 0) solunda.
const BLOCK: usize = 1;

thread_local! {
    static AI: RefCell<Option<(Config, Running)>> = const { RefCell::new(None) };
    /// Taskbar göstergesi (iki blok); `sys` de buradan çizer.
    pub static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
    /// `[ai] enabled`
    static SHOW: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// ai.toml / taskbar.toml okuma hatası (varsayılanla devam edilir)
    static CFG_ERR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn log(m: &str) {
    lowmon_deck::log::write(&format!("ai: {m}"));
}

fn now() -> i64 {
    lowmon_ai::fetch::now_ms() / 1000
}

/// İşçiyi ve göstergeyi (yeniden) başlatır; ayarlar dosyadan okunur.
pub fn start(hwnd: HWND) {
    AI.set(None); // eski işçi durur
    BAR.set(None); // eski pencere/kanca bırakılır
    let mut errs = Vec::new();
    let cfg = Config::load(&lowmon_ai::config_path()).unwrap_or_else(|e| {
        errs.push(e);
        Config::default()
    });
    let tb = lowmon_taskbar::Config::load(&lowmon_taskbar::config_path()).unwrap_or_else(|e| {
        errs.push(format!("taskbar.toml: {e}"));
        lowmon_taskbar::Config::default()
    });
    for e in &errs {
        log(e);
    }
    CFG_ERR.set(errs.into_iter().next());
    text::set_lang(cfg.language); // tray menüsü ve iletiler de bu dilde
    let owner = hwnd as usize;
    // SAFETY: pencere yok olduysa PostMessage yalnız başarısız döner.
    let on_change = Box::new(move |_: &State| unsafe {
        PostMessageW(owner as HWND, WM_AI, 0, 0);
    });
    let run = lowmon_ai::start(cfg.clone(), on_change, log);
    AI.set(Some((cfg, run)));
    SHOW.set(tb.ai.enabled);
    let sys = tb.sys;
    let monitors = [sys.monitor, tb.ai.monitor]; // sıra: sys::BLOCK, BLOCK
    BAR.set(Some(Bar::new(hwnd, WM_BAR, tb, &monitors)));
    crate::sys::start(hwnd, sys);
    // SAFETY: kendi penceremiz; aynı kimlik yeniden kurulursa süre güncellenir.
    unsafe { SetTimer(hwnd, TIMER, TICK_MS, None) };
    render();
}

pub fn stop(hwnd: HWND) {
    // SAFETY: kendi penceremiz.
    unsafe { KillTimer(hwnd, TIMER) };
    crate::sys::stop();
    BAR.set(None);
    AI.set(None);
}

/// Seviye → renk (C++ widget'la aynı palet).
pub fn color(level: Level, light: bool) -> u32 {
    match level {
        Level::Critical => 0xE5484D,
        Level::Warn => 0xF0A030,
        Level::Dim if light => 0x707070,
        Level::Dim => 0x9A9A9A,
        Level::Normal if light => 0x1A1A1A,
        Level::Normal => 0xFFFFFF,
    }
}

/// Telefon / `lowmon-ui` için (deck `TOPIC_AI`); veri yoksa None.
pub fn usage_of(st: &State, cfg: &Config, now: i64) -> Option<AiUsage> {
    let s = st.snapshot.as_ref()?;
    // Ham seviye; "eski" ayrı alan (istemci soluk gösterir)
    let (l1, l2) = usage::levels(s, now, &cfg.colors, false);
    let win = |w: &Option<Window>, l: Level| {
        w.map(|w| AiWindow {
            pct: (w.utilization * 10.0).round().clamp(0.0, u16::MAX as f64) as u16,
            resets_at: w.resets_at.unwrap_or(0).max(0) as u64,
            level: match l {
                Level::Warn => 1,
                Level::Critical => 2,
                _ => 0,
            },
        })
    };
    Some(AiUsage {
        session: win(&s.five_hour, l1),
        week: win(&s.seven_day, l2),
        fetched_at: st.fetched_at.max(0) as u64,
        stale: st.stale(now, cfg.refresh_sec),
        source: clip(st.source, AI_TEXT_MAX),
        error: clip(&st.error, AI_TEXT_MAX),
    })
}

/// Son durumla göstergeyi çizer (veri hiç yoksa gizler) ve deck'e yayınlar (değiştiyse gider).
pub fn render() {
    let now = now();
    AI.with_borrow(|a| {
        let Some((cfg, run)) = a else { return };
        let st = run.state();
        let u = usage_of(&st, cfg, now);
        crate::DECK.with_borrow(|d| {
            if let Some(d) = d {
                d.hub.set_ai(u);
            }
        });
        BAR.with_borrow_mut(|b| {
            let Some(b) = b else { return };
            let Some(s) = st.snapshot.as_ref().filter(|_| SHOW.get()) else {
                b.hide(BLOCK);
                return;
            };
            let [t1, t2] = text::lines(s, now, cfg.language);
            let (v1, v2) = usage::levels(s, now, &cfg.colors, st.stale(now, cfg.refresh_sec));
            let light = lowmon_taskbar::light_theme();
            b.show(
                BLOCK,
                Grid::lines([
                    Line {
                        text: t1,
                        rgb: color(v1, light),
                    },
                    Line {
                        text: t2,
                        rgb: color(v2, light),
                    },
                ]),
            );
        });
    });
}

/// Gösterge tıklamasında çağıranın yapacağı iş.
#[derive(Debug, PartialEq, Eq)]
pub enum Click {
    None,
    /// sağ tık → tray menüsü
    Menu,
    /// AI bloğuna sol tık → `lowmon-ui` AI sekmesi (rapor)
    Report,
    /// sistem ızgarasına sol tık → `lowmon-ui` Canlı sekmesi
    Live,
}

/// Taskbar olayı; `block` = tıklanan blok (`EV_MOVED`'da anlamsız).
pub fn on_bar(ev: usize, block: usize) -> Click {
    match ev {
        EV_MOVED => {
            BAR.with_borrow_mut(|b| b.as_mut().map(Bar::layout));
            Click::None
        }
        EV_LEFT if block == crate::sys::BLOCK => Click::Live,
        EV_LEFT => Click::Report,
        EV_RIGHT => Click::Menu,
        _ => Click::None,
    }
}

/// Explorer yeniden başladı ya da monitör düzeni değişti: bloklar taskbar'larına yeniden kurulur.
pub fn taskbar_created() {
    BAR.with_borrow_mut(|b| b.as_mut().map(Bar::reset))
        .unwrap_or(())
}

pub fn refresh_now() {
    AI.with_borrow(|a| {
        if let Some((_, r)) = a {
            r.refresh_now();
        }
    });
}

/// Tooltip için "%33/%68" (oturum/hafta); veri yoksa None.
pub fn short() -> Option<String> {
    AI.with_borrow(|a| {
        let (cfg, run) = a.as_ref()?;
        short_of(run.state().snapshot.as_ref()?, cfg.language)
    })
}

fn short_of(s: &lowmon_ai::api::Snapshot, lang: text::Lang) -> Option<String> {
    let p = |w: &Option<lowmon_ai::api::Window>| {
        w.map_or("–".into(), |w| text::pct(lang, w.utilization))
    };
    (s.five_hour.is_some() || s.seven_day.is_some())
        .then(|| format!("{}/{}", p(&s.five_hour), p(&s.seven_day)))
}

fn session_file() -> std::path::PathBuf {
    lowmon_ai::data_dir().join("session.bin")
}

/// Panodaki metin (CF_UNICODETEXT); geçerli bir anahtar okunduysa pano temizlenir.
/// Dönen tampon sır içerebilir: çağıran sıfırlar.
fn take_clipboard_key(hwnd: HWND) -> Option<String> {
    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: pano bu fonksiyonda açılıp kapanır; kilitli bellek NUL sonlu UTF-16.
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return None;
        }
        let mut buf: Vec<u16> = Vec::new();
        let h = GetClipboardData(CF_UNICODETEXT);
        if !h.is_null() {
            let p = GlobalLock(h) as *const u16;
            if !p.is_null() {
                let n = (0..).take_while(|&i| *p.add(i) != 0).count();
                buf.extend_from_slice(std::slice::from_raw_parts(p, n));
                GlobalUnlock(h);
            }
        }
        let text = String::from_utf16_lossy(&buf);
        lowmon_ai::win::wipe(&mut buf);
        let key = lowmon_ai::creds::parse_session_key(&text).map(str::to_owned);
        let mut text = text.into_bytes();
        lowmon_ai::win::wipe(&mut text);
        if key.is_some() {
            EmptyClipboard(); // sır panoda kalmasın
        }
        CloseClipboard();
        key
    }
}

fn message(hwnd: HWND, text: &str, flags: u32) {
    // SAFETY: NUL sonlu dizgeler; modal kutu.
    unsafe {
        MessageBoxW(
            hwnd,
            crate::wide(text).as_ptr(),
            crate::wide("LowMonitor AI").as_ptr(),
            flags,
        )
    };
}

/// "Panodan sessionKey kaydet" (C++ widget ile aynı akış).
pub fn save_key(hwnd: HWND) {
    let Some(key) = take_clipboard_key(hwnd) else {
        message(
            hwnd,
            tr(
                "Panoda sessionKey yok. claude.ai çerezlerinden sessionKey değerini (sk-ant-…) kopyalayıp tekrar deneyin.",
                "No sessionKey on the clipboard. Copy the sessionKey value (sk-ant-…) from the claude.ai cookies and try again.",
            ),
            MB_ICONWARNING,
        );
        return;
    };
    let r = lowmon_ai::fetch::save_session_key(&session_file(), &key);
    let mut key = key.into_bytes();
    lowmon_ai::win::wipe(&mut key);
    match r {
        Ok(()) => {
            message(
                hwnd,
                tr(
                    "sessionKey kaydedildi (DPAPI).",
                    "sessionKey saved (DPAPI).",
                ),
                MB_ICONINFORMATION,
            );
            refresh_now();
        }
        Err(e) => {
            log(&format!("sessionKey kaydedilemedi: {e}"));
            message(
                hwnd,
                &format!(
                    "{}: {e}",
                    tr("sessionKey kaydedilemedi", "Could not save sessionKey")
                ),
                MB_ICONERROR,
            );
        }
    }
}

/// "sessionKey'i sil"
pub fn delete_key() {
    if let Err(e) = std::fs::remove_file(session_file()) {
        log(&format!("sessionKey silinemedi: {e}"));
    }
    refresh_now();
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n - 1).chain(['…']).collect()
    } else {
        s.to_owned()
    }
}

/// Menüdeki durum satırı.
pub fn status(st: &State, now: i64, cfg_err: Option<&str>) -> String {
    if let Some(e) = cfg_err {
        return clip(&format!("{}: {e}", tr("Ayar hatası", "Config error")), 80);
    }
    if !st.error.is_empty() {
        return clip(&format!("{}: {}", tr("Hata", "Error"), st.error), 80);
    }
    let closed = if st.active {
        ""
    } else {
        tr(" · Claude kapalı", " · Claude closed")
    };
    if st.fetched_at == 0 {
        return if st.active {
            tr("Sorgulanıyor…", "Querying…").into()
        } else {
            tr("Claude kapalı · sorgu yok", "Claude closed · not querying").into()
        };
    }
    format!(
        "{} · {} {}{closed}",
        st.source,
        (now - st.fetched_at).max(0) / 60,
        tr("dk önce", "min ago")
    )
}

/// "AI kullanımı ▸"; dönen menü üst menüyle birlikte yok edilir.
pub fn menu() -> HMENU {
    let st = AI.with_borrow(|a| a.as_ref().map(|(_, r)| r.state()).unwrap_or_default());
    let line = CFG_ERR.with_borrow(|e| status(&st, now(), e.as_deref()));
    // SAFETY: dizgeler çağrı boyunca yaşar; menü tutamacı çağırana devredilir.
    unsafe {
        let sub = CreatePopupMenu();
        AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, crate::wide(&line).as_ptr());
        AppendMenuW(sub, MF_SEPARATOR, 0, ptr::null());
        for (id, t) in [
            (
                ID_REPORT,
                tr("Rapor (token dökümü)", "Report (token breakdown)"),
            ),
            (ID_NOW, tr("Şimdi yenile", "Refresh now")),
            (ID_CFG, tr("Ayar dosyasını aç", "Open config file")),
            (ID_RELOAD, tr("Ayarı yeniden yükle", "Reload config")),
        ] {
            AppendMenuW(sub, MF_STRING, id, crate::wide(t).as_ptr());
        }
        // sessionKey: OAuth olmazsa claude.ai çerezi (DPAPI, `session.bin`)
        AppendMenuW(sub, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            sub,
            MF_STRING,
            ID_KEY_SAVE,
            crate::wide(tr(
                "Panodan sessionKey kaydet",
                "Save sessionKey from clipboard",
            ))
            .as_ptr(),
        );
        let has = if session_file().exists() {
            0
        } else {
            MF_GRAYED
        };
        AppendMenuW(
            sub,
            MF_STRING | has,
            ID_KEY_DEL,
            crate::wide(tr("sessionKey'i sil", "Delete sessionKey")).as_ptr(),
        );
        sub
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(color(Level::Normal, false), 0xFFFFFF);
        assert_eq!(color(Level::Normal, true), 0x1A1A1A);
        assert_eq!(color(Level::Critical, true), 0xE5484D);
        assert_eq!(color(Level::Dim, false), 0x9A9A9A);
    }

    #[test]
    fn short_line() {
        use lowmon_ai::api::{Snapshot, Window};
        let w = |u| {
            Some(Window {
                utilization: u,
                resets_at: None,
            })
        };
        let s = Snapshot {
            five_hour: w(33.4),
            seven_day: w(68.0),
            ..Default::default()
        };
        assert_eq!(short_of(&s, text::Lang::Tr).as_deref(), Some("%33/%68"));
        let s = Snapshot {
            five_hour: None,
            ..s
        };
        assert_eq!(short_of(&s, text::Lang::En).as_deref(), Some("–/68%"));
        assert_eq!(short_of(&Snapshot::default(), text::Lang::Tr), None);
    }

    #[test]
    fn usage_for_deck() {
        use lowmon_ai::api::Snapshot;
        let cfg = Config::default();
        let now = 1_000_000;
        let mut st = State {
            snapshot: Some(Snapshot {
                five_hour: Some(Window {
                    utilization: 33.44,
                    resets_at: Some(now + 4 * 3600),
                }),
                seven_day: Some(Window {
                    utilization: 95.0,
                    resets_at: None,
                }),
                ..Default::default()
            }),
            fetched_at: now - 10,
            source: "OAuth",
            active: true,
            ..Default::default()
        };
        let u = usage_of(&st, &cfg, now).unwrap();
        let s = u.session.unwrap();
        assert_eq!(
            (s.pct, s.resets_at, s.level),
            (334, now as u64 + 4 * 3600, 1)
        ); // beklenen %20 → tempo önde
        let w = u.week.unwrap();
        assert_eq!((w.pct, w.resets_at, w.level), (950, 0, 2));
        assert!(!u.stale && u.error.is_empty() && u.source == "OAuth");
        st.active = false;
        assert!(usage_of(&st, &cfg, now).unwrap().stale);
        st.snapshot = None;
        assert_eq!(usage_of(&st, &cfg, now), None);
    }

    #[test]
    fn status_line() {
        let mut st = State::default();
        assert_eq!(status(&st, 100, None), "Claude kapalı · sorgu yok");
        st.active = true;
        assert_eq!(status(&st, 100, None), "Sorgulanıyor…");
        st.fetched_at = 1000;
        st.source = "OAuth";
        assert_eq!(status(&st, 1130, None), "OAuth · 2 dk önce");
        st.active = false;
        assert_eq!(status(&st, 1000, None), "OAuth · 0 dk önce · Claude kapalı");
        st.error = "x".repeat(100);
        assert_eq!(status(&st, 1000, None).chars().count(), 80);
        assert!(status(&st, 0, Some("ai.toml: bozuk")).starts_with("Ayar hatası"));
    }
}
