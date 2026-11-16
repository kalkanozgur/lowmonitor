//! AI kullanımı (ADR-0032 Faz 2): `lowmon-ai` işçisi + `lowmon-taskbar` göstergesi, UI thread'inde.
//! İşçi durum değişince `WM_AI` gönderir; kalan süre metni için 30 s'lik zamanlayıcı yeniden çizer.
use lowmon_ai::{Config, Running, State, api::Window, text, usage, usage::Level};
use lowmon_deck::{AI_TEXT_MAX, AiUsage, AiWindow};
use lowmon_taskbar::{Bar, EV_LEFT, EV_MOVED, EV_RIGHT, Line};
use std::{cell::RefCell, ptr};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, HMENU, KillTimer, MF_GRAYED, MF_SEPARATOR, MF_STRING,
        PostMessageW, SetTimer, WM_APP,
    },
};

pub const WM_AI: u32 = WM_APP + 3;
pub const WM_BAR: u32 = WM_APP + 4;
pub const TIMER: usize = 0xA1;
pub const ID_NOW: usize = 20;
pub const ID_CFG: usize = 21;
pub const ID_RELOAD: usize = 22;
const TICK_MS: u32 = 30_000;

thread_local! {
    static AI: RefCell<Option<(Config, Running)>> = const { RefCell::new(None) };
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
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
    let owner = hwnd as usize;
    // SAFETY: pencere yok olduysa PostMessage yalnız başarısız döner.
    let on_change = Box::new(move |_: &State| unsafe {
        PostMessageW(owner as HWND, WM_AI, 0, 0);
    });
    let run = lowmon_ai::start(cfg.clone(), on_change, log);
    AI.set(Some((cfg, run)));
    BAR.set(Some(Bar::new(hwnd, WM_BAR, tb)));
    // SAFETY: kendi penceremiz; aynı kimlik yeniden kurulursa süre güncellenir.
    unsafe { SetTimer(hwnd, TIMER, TICK_MS, None) };
    render();
}

pub fn stop(hwnd: HWND) {
    // SAFETY: kendi penceremiz.
    unsafe { KillTimer(hwnd, TIMER) };
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
            let Some(s) = &st.snapshot else {
                b.hide();
                return;
            };
            let [t1, t2] = text::lines(s, now, cfg.language);
            let (v1, v2) = usage::levels(s, now, &cfg.colors, st.stale(now, cfg.refresh_sec));
            let light = lowmon_taskbar::light_theme();
            b.show([
                Line {
                    text: t1,
                    rgb: color(v1, light),
                },
                Line {
                    text: t2,
                    rgb: color(v2, light),
                },
            ]);
        });
    });
}

/// Taskbar olayı; sağ tıkta `true` (çağıran tray menüsünü açar).
pub fn on_bar(ev: usize) -> bool {
    match ev {
        EV_MOVED => BAR
            .with_borrow_mut(|b| b.as_mut().map(Bar::layout))
            .unwrap_or(()),
        EV_LEFT => refresh_now(),
        _ => {}
    }
    ev == EV_RIGHT
}

/// Explorer yeniden başladı.
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
        return clip(&format!("Ayar hatası: {e}"), 80);
    }
    if !st.error.is_empty() {
        return clip(&format!("Hata: {}", st.error), 80);
    }
    let closed = if st.active { "" } else { " · Claude kapalı" };
    if st.fetched_at == 0 {
        return if st.active {
            "Sorgulanıyor…".into()
        } else {
            "Claude kapalı · sorgu yok".into()
        };
    }
    format!(
        "{} · {} dk önce{closed}",
        st.source,
        (now - st.fetched_at).max(0) / 60
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
            (ID_NOW, "Şimdi yenile"),
            (ID_CFG, "Ayar dosyasını aç"),
            (ID_RELOAD, "Ayarı yeniden yükle"),
        ] {
            AppendMenuW(sub, MF_STRING, id, crate::wide(t).as_ptr());
        }
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
