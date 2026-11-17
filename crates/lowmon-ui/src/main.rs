//! LowMonitor PC arayüzü (ADR-0031, eski LowDeck masaüstü): Canlı, Olaylar, Geçmiş, Süreçler, AI.
//! Veri yalnız tray'deki deck'ten loopback (127.0.0.1); port + token `%APPDATA%\LowMonitor\deck.toml`'dan.
//! Medya/ses kontrolü yok (ADR-0031 §6): yalnız izler.
//! Kullanım: `lowmon-ui [port token]` (argümanlar deck.toml'u ezer; mock-server için `47810 dev`).
//! `lowmon-ui --ai`: AI sekmesiyle açılır; çalışan kopya varsa onu AI sekmesine geçirir (taskbar göstergesi sol tık).
//! `lowmon-ui --kill <pid> <ctime>`: yükseltilmiş kopya (UAC), pencere açmaz, sonuç = çıkış kodu.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod aitab;
mod history;
mod procs;
#[cfg(windows)]
mod win;

use history::{History, Sample, nice_scale, series_path};
use lowdeck_client::{Client, Config, Event};
use lowdeck_proto::{
    AiUsage, AiWindow, C2S, EventItem, HIST_POINTS_MAX, HistPoint, Metrics, S2C, TOPIC_AI,
    TOPIC_EVENTS, TOPIC_METRICS, TopItem,
};
use lowmon_ai::text::tr;
use slint::{Model, ModelRc, VecModel};
use std::collections::HashMap;
use std::rc::Rc;

const HISTORY_MS: u64 = 24 * 3_600_000;
/// Liste üst sınırı: eski olaylar sondan atılır (bellek sabit).
const EVENTS_MAX: usize = 1000;

fn main() -> Result<(), slint::PlatformError> {
    // Tek örnek kontrolünden önce: yükseltilmiş kopya mutex'i görüp çıkmasın.
    if let Some(code) = kill_mode() {
        std::process::exit(code);
    }
    let ai = std::env::args().any(|a| a == "--ai");
    #[cfg(windows)]
    if !win::single_instance(ai) {
        return Ok(());
    }

    // Dil: tray ile aynı ayar (`ai.toml` `language`); dosya bozuksa Türkçe.
    let lang = lowmon_ai::Config::load(&lowmon_ai::config_path())
        .map(|c| c.language)
        .unwrap_or_default();
    lowmon_ai::text::set_lang(lang);
    let ui = MainWindow::new()?;
    ui.global::<Lang>()
        .set_english(lang == lowmon_ai::text::Lang::En);
    let events = Rc::new(VecModel::<EventRow>::default());
    ui.set_events(ModelRc::from(events.clone()));
    let offset = local_offset_s();
    procs_init(&ui, offset);
    window_events(&ui);
    aitab::init(&ui, offset);
    #[cfg(windows)]
    {
        let w = ui.as_weak();
        win::on_show_ai(move || {
            let _ = w.upgrade_in_event_loop(|ui| ui.set_tab(4));
        });
    }
    if ai {
        ui.set_tab(4);
    }

    match target() {
        Some((port, token)) => start_client(&ui, events, port, token, offset),
        // deck.toml'u tray ilk açılışta yazar; arayüz oluşturmaz (token tray'inkiyle aynı olmalı).
        None => ui.set_status(
            tr(
                "deck.toml yok: LowMonitor tray'ini bir kez çalıştırın",
                "deck.toml missing: run the LowMonitor tray once",
            )
            .into(),
        ),
    }
    ui.run()
}

/// İstemciyi başlatır; `Rc<Client>` kancaların (küçültme, Yenile, bağlanınca geçmiş) closure'larında yaşar.
fn start_client(
    ui: &MainWindow,
    events: Rc<VecModel<EventRow>>,
    port: u16,
    token: String,
    offset: i64,
) {
    let weak = ui.as_weak();
    let client = Rc::new(Client::start(
        Config {
            host: "127.0.0.1".into(),
            port,
            token,
            name: tr("LowMonitor arayüzü", "LowMonitor interface").into(),
        },
        move |ev| {
            let _ = weak.upgrade_in_event_loop(move |ui| apply(&ui, ev, offset));
        },
    ));
    client.subscribe(topics(MINIMIZED.with(|m| m.get())));
    CLIENT.with(|c| *c.borrow_mut() = Some(client.clone()));

    // Geçmiş sekmesi: düğme / sekmeye geçiş → tek sorgu (ADR-0005); otomatik yenileme yok.
    {
        let (client, weak) = (client.clone(), ui.as_weak());
        ui.on_query_history(move |span| {
            let Some(ui) = weak.upgrade() else { return };
            let now = now_ms();
            let from_ms = now.saturating_sub(HIST_SPANS_MS[span.clamp(0, 3) as usize]);
            if client.send(&C2S::QueryHistory {
                from_ms,
                to_ms: now,
                points: HIST_POINTS_MAX,
            }) {
                HQ.with(|h| {
                    let mut h = h.borrow_mut();
                    h.pending += 1;
                    h.pts.clear();
                });
                ui.set_hist_status(tr("Yükleniyor…", "Loading…").into());
            } else {
                ui.set_hist_status(tr("Tray'e bağlı değil", "Not connected to tray").into());
            }
        });
    }

    let query = move || {
        events.set_vec(Vec::new());
        let now = now_ms();
        client.send(&C2S::QueryEvents {
            from_ms: now.saturating_sub(HISTORY_MS),
            to_ms: u64::MAX,
        });
    };
    ui.on_refresh_events(query.clone());
    // Her bağlanışta geçmiş yeniden istenir: `apply` Connected'ı görünce bu kancayı çağırır.
    QUERY.with(|q| *q.borrow_mut() = Some(Box::new(query)));
}

/// Küçültülünce metrik + AI düşer (deck abone kalmazsa pipe'ı kapatır); olaylar sürer: alarm bandı + liste güncel.
fn topics(minimized: bool) -> lowdeck_proto::Topics {
    if minimized {
        TOPIC_EVENTS
    } else {
        TOPIC_METRICS | TOPIC_EVENTS | TOPIC_AI
    }
}

/// Pencere olayları tek kancada (winit kancası tek): küçültme → abonelik + süreç taraması durur.
fn window_events(ui: &MainWindow) {
    use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |_, ev| {
        // Windows'ta küçültme = 0×0 Resized; yalnız durum değişince işlem
        if let WindowEvent::Resized(s) = ev {
            let minimized = s.width == 0 && s.height == 0;
            if MINIMIZED.with(|m| m.replace(minimized)) != minimized {
                CLIENT.with(|c| c.borrow().as_ref().map(|c| c.subscribe(topics(minimized))));
                if let Some(ui) = weak.upgrade() {
                    procs_sync(&ui);
                }
            }
        }
        EventResult::Propagate
    });
}

// ---------------- Süreçler (LM spek §9.4, §10) ----------------

struct ProcUi {
    tracker: procs::Tracker,
    rows: Vec<procs::Row>,
    /// Tarama buffer'ı (Windows), yeniden kullanılır.
    buf: Vec<u64>,
    selected: Option<(u32, i64)>,
    /// Arama: ad (büyük/küçük harf duyarsız) ya da PID içerir.
    filter: String,
    sort: procs::SortBy,
    confirm: bool,
    /// Yükseltilmiş kopya bekleniyor.
    busy: bool,
    open: bool,
    status: String,
    timer: slint::Timer,
}

impl Default for ProcUi {
    fn default() -> Self {
        Self {
            tracker: Default::default(),
            rows: Vec::new(),
            buf: Vec::new(),
            selected: None,
            filter: String::new(),
            sort: procs::SortBy::Cpu,
            confirm: false,
            busy: false,
            open: false,
            status: String::new(),
            timer: slint::Timer::default(),
        }
    }
}

/// `--kill <pid> <ctime>` ise sonuç kodu (yükseltilmiş kopya).
fn kill_mode() -> Option<i32> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [flag, pid, ctime] = a.as_slice() else {
        return None;
    };
    if flag != "--kill" {
        return None;
    }
    let (Ok(pid), Ok(ctime)) = (pid.parse::<u32>(), ctime.parse::<i64>()) else {
        return Some(procs::exit::FAILED);
    };
    Some(kill_now(pid, ctime))
}

fn kill_now(pid: u32, ctime: i64) -> i32 {
    #[cfg(windows)]
    return win::kill(pid, ctime);
    #[cfg(not(windows))]
    {
        let _ = (pid, ctime);
        procs::exit::FAILED
    }
}

fn procs_init(ui: &MainWindow, offset: i64) {
    ui.set_procs(ModelRc::new(VecModel::<ProcRow>::default()));
    let w = ui.as_weak();
    ui.on_tab_changed(move |t| {
        let Some(ui) = w.upgrade() else { return };
        PROCS.with(|p| {
            let mut p = p.borrow_mut();
            p.open = t == 3;
            p.confirm = false;
        });
        procs_sync(&ui);
        if t == 4 {
            aitab::refresh(&ui, offset);
        }
    });
    let w = ui.as_weak();
    ui.on_proc_sort_changed(move |i| {
        let Some(ui) = w.upgrade() else { return };
        PROCS.with(|p| {
            let mut p = p.borrow_mut();
            p.sort = if i == 1 {
                procs::SortBy::Ram
            } else {
                procs::SortBy::Cpu
            };
            let sort = p.sort;
            procs::sort_rows(&mut p.rows, sort);
        });
        procs_render(&ui);
    });
    let w = ui.as_weak();
    ui.on_proc_filter_changed(move |f| {
        let Some(ui) = w.upgrade() else { return };
        PROCS.with(|p| p.borrow_mut().filter = f.to_lowercase());
        procs_render(&ui);
    });
    let w = ui.as_weak();
    // Liste filtrelenebildiği için satır indeksi değil PID gelir.
    ui.on_proc_select(move |pid| {
        let Some(ui) = w.upgrade() else { return };
        PROCS.with(|p| {
            let mut p = p.borrow_mut();
            let Some(r) = p.rows.iter().find(|r| r.pid as i32 == pid).cloned() else {
                return;
            };
            p.selected = Some((r.pid, r.ctime));
            p.confirm = false;
            let lock = if procs::protected(r.pid, &r.name) {
                tr("  ·  korumalı", "  ·  protected")
            } else {
                ""
            };
            p.status = format!("{}  ·  PID {}{lock}", r.name, r.pid);
        });
        procs_render(&ui);
    });
    let w = ui.as_weak();
    ui.on_proc_kill(move || {
        if let Some(ui) = w.upgrade() {
            proc_kill(&ui);
        }
    });
}

/// Tarama yalnız sekme açık ve pencere küçültülmemişken (tek periyodik iş, 2 s); kapanınca timer durur.
fn procs_sync(ui: &MainWindow) {
    let active = PROCS.with(|p| p.borrow().open) && !MINIMIZED.with(|m| m.get());
    let running = PROCS.with(|p| p.borrow().timer.running());
    if active && !running {
        procs_refresh(ui);
        let w = ui.as_weak();
        PROCS.with(|p| {
            p.borrow().timer.start(
                slint::TimerMode::Repeated,
                std::time::Duration::from_millis(procs::SCAN_MS),
                move || {
                    if let Some(ui) = w.upgrade() {
                        procs_refresh(&ui);
                    }
                },
            )
        });
    } else if !active && running {
        PROCS.with(|p| {
            let mut p = p.borrow_mut();
            p.timer.stop();
            p.tracker.reset();
        });
    }
}

fn procs_refresh(ui: &MainWindow) {
    // Fare liste üzerindeyken ya da onay beklerken liste dondurulur: seçilecek satır kaymasın.
    // Tarama da atlanır; sonraki taramada CPU deltası daha uzun aralıktan hesaplanır (doğru).
    if ui.get_proc_hover() || PROCS.with(|p| p.borrow().confirm) {
        return;
    }
    let ncpu = std::thread::available_parallelism().map_or(1, |n| n.get() as u64);
    PROCS.with(|p| {
        let mut p = p.borrow_mut();
        let raw = scan(&mut p.buf);
        let sort = p.sort;
        p.rows = p.tracker.update(now_ms(), ncpu, raw, sort);
        // Seçili süreç öldüyse seçim düşer
        if let Some(sel) = p.selected
            && !p.rows.iter().any(|r| (r.pid, r.ctime) == sel)
        {
            p.selected = None;
            p.confirm = false;
        }
    });
    procs_render(ui);
}

fn procs_render(ui: &MainWindow) {
    PROCS.with(|p| {
        let p = p.borrow();
        let rows: Vec<ProcRow> = p
            .rows
            .iter()
            .filter(|r| procs::matches(r, &p.filter))
            .map(|r| ProcRow {
                pid: r.pid as i32,
                name: r.name.as_str().into(),
                cpu: procs::fmt_cpu(r.cpu).into(),
                ws: format!("{} MB", r.ws_mb).into(),
                protected: procs::protected(r.pid, &r.name),
                selected: p.selected == Some((r.pid, r.ctime)),
            })
            .collect();
        let rows_len = rows.len();
        set_vec(ui.get_procs(), rows, |v| ui.set_procs(v));
        let sel = p
            .selected
            .and_then(|s| p.rows.iter().find(|r| (r.pid, r.ctime) == s));
        ui.set_proc_can_kill(!p.busy && sel.is_some_and(|r| !procs::protected(r.pid, &r.name)));
        ui.set_proc_confirm(p.confirm);
        let status = match (p.status.is_empty(), p.filter.is_empty()) {
            (false, _) => p.status.clone(),
            (true, true) => format!("{} {}", p.rows.len(), tr("süreç", "processes")),
            (true, false) => format!(
                "{} / {} {}",
                rows_len,
                p.rows.len(),
                tr("süreç", "processes")
            ),
        };
        ui.set_proc_status(status.into());
    });
}

/// İki aşama: ilk tık onay ister, ikincisi sonlandırır. Erişim reddinde yükseltilmiş kopya (UAC) ayrı thread'de beklenir.
fn proc_kill(ui: &MainWindow) {
    let Some((pid, ctime, name)) = PROCS.with(|p| {
        let p = p.borrow();
        let s = p.selected?;
        p.rows
            .iter()
            .find(|r| (r.pid, r.ctime) == s)
            .map(|r| (r.pid, r.ctime, r.name.clone()))
    }) else {
        return;
    };
    let ask = PROCS.with(|p| {
        let mut p = p.borrow_mut();
        if p.busy {
            return None;
        }
        p.confirm = !p.confirm;
        if p.confirm {
            p.status = format!(
                "{name} (PID {pid}) {}",
                tr("sonlandırılsın mı?", ": end this process?")
            );
        }
        Some(p.confirm)
    });
    match ask {
        None => return,
        Some(true) => return procs_render(ui),
        Some(false) => {}
    }
    let code = kill_now(pid, ctime);
    #[cfg(windows)]
    if code == procs::exit::DENIED {
        PROCS.with(|p| {
            let mut p = p.borrow_mut();
            p.busy = true;
            p.status = format!(
                "{name}: {}",
                tr(
                    "yönetici onayı isteniyor…",
                    "asking for administrator approval…"
                )
            );
        });
        procs_render(ui);
        let w = ui.as_weak();
        std::thread::spawn(move || {
            let code = win::kill_elevated(pid, ctime);
            let _ = w.upgrade_in_event_loop(move |ui| {
                PROCS.with(|p| {
                    let mut p = p.borrow_mut();
                    p.busy = false;
                    p.status = procs::exit::text(code, &name);
                });
                procs_refresh(&ui);
            });
        });
        return;
    }
    PROCS.with(|p| p.borrow_mut().status = procs::exit::text(code, &name));
    procs_refresh(ui);
}

#[cfg(windows)]
fn scan(buf: &mut Vec<u64>) -> Vec<procs::RawProc> {
    win::scan(buf)
}

/// Geliştirme (Linux + mock): sabit sahte süreçler, CPU zamanı saatle artar.
#[cfg(not(windows))]
fn scan(_: &mut Vec<u64>) -> Vec<procs::RawProc> {
    let t = now_ms() * 10_000; // 100 ns
    [
        ("System", 4, 1),
        ("explorer.exe", 1200, 5),
        ("firefox.exe", 3400, 160),
        ("Code.exe", 5100, 70),
        ("lowmon-tray.exe", 880, 1),
        ("lsass.exe", 700, 2),
    ]
    .iter()
    .enumerate()
    .map(|(i, &(name, pid, w))| procs::RawProc {
        pid,
        ctime: 1,
        cpu_100ns: t / 100 * w,
        ws_bytes: ((i as u64 + 1) * 90) << 20,
        name: name.into(),
    })
    .collect()
}

/// Geçmiş aralıkları (UI düğme sırası): 1 sa, 6 sa, 24 sa, 7 g.
const HIST_SPANS_MS: [u64; 4] = [3_600_000, 6 * 3_600_000, 24 * 3_600_000, 7 * 24 * 3_600_000];

/// Geçmiş yanıt tamponu. `pending` = yanıtı bitmemiş sorgu sayısı: yalnız sonuncusunun parçaları tutulur
/// (öncekilerin parçaları `pending > 1` iken gelir, atılır).
#[derive(Default)]
struct HistRx {
    pending: u32,
    from_ms: u64,
    step_ms: u64,
    pts: Vec<Option<HistPoint>>,
    /// Son `Metrics`'ten; RAM grafiği ölçeği (yoksa geçmişteki max).
    ram_total_mb: u32,
}

thread_local! {
    static CLIENT: std::cell::RefCell<Option<Rc<Client>>> = const { std::cell::RefCell::new(None) };
    /// Son AI durumu: geri sayım her metrik örneğinde yenilenir (ek timer yok).
    static LAST_AI: std::cell::RefCell<Option<AiUsage>> = const { std::cell::RefCell::new(None) };
    static MINIMIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PROCS: std::cell::RefCell<ProcUi> = std::cell::RefCell::new(ProcUi::default());
    static HQ: std::cell::RefCell<HistRx> = std::cell::RefCell::new(HistRx::default());
    /// Top-5 yumuşatılmış CPU (ad, aynı adın sırası) → değer; yalnız son listedekiler tutulur.
    static TOP_EMA: std::cell::RefCell<HashMap<(String, u8), u16>> = std::cell::RefCell::new(HashMap::new());
    static QUERY: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
    /// Canlı grafik geçmişi + path buffer'ı (yalnız event loop thread'i).
    static HIST: std::cell::RefCell<(History, String)> = std::cell::RefCell::new((History::default(), String::new()));
}

/// (port, token): argümanlar, yoksa `deck.toml` (`bind` LAN adresi; arayüz loopback'e bağlanır).
fn target() -> Option<(u16, String)> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<String> = a.into_iter().filter(|x| x != "--ai").collect();
    if let [port, token] = a.as_slice() {
        return Some((port.parse().ok()?, token.clone()));
    }
    let c = lowmon_deck::load(&lowmon_deck::config_path()).ok()?;
    Some((c.port, c.token))
}

fn apply(ui: &MainWindow, ev: Event, offset: i64) {
    match ev {
        Event::Connecting => {
            ui.set_connected(false);
            ui.set_status(tr("Bağlanıyor…", "Connecting…").into());
        }
        Event::Connected { server } => {
            ui.set_connected(true);
            ui.set_status(format!("{}: {server}", tr("Bağlı", "Connected")).into());
            QUERY.with(|q| q.borrow().as_ref().map(|f| f()));
        }
        Event::Disconnected { reason } => {
            ui.set_connected(false);
            ui.set_status(
                format!(
                    "{}: {reason}",
                    tr("Tray'e bağlı değil", "Not connected to tray")
                )
                .into(),
            );
            ui.set_lm_state(0);
            ui.set_ai_state(0);
            ui.set_alarms("".into());
            clear_graphs(ui);
            HQ.with(|h| h.borrow_mut().pending = 0); // yanıtlar kayboldu
            ui.set_hist_status("".into());
        }
        Event::Msg(S2C::Metrics(None)) => {
            ui.set_lm_state(2);
            clear_graphs(ui);
            set_vec(ui.get_lm_cores(), Vec::new(), |v| ui.set_lm_cores(v));
            set_vec(ui.get_lm_top(), Vec::new(), |v| ui.set_lm_top(v));
        }
        Event::Msg(S2C::Metrics(Some(m))) => {
            show_metrics(ui, &m);
            LAST_AI.with_borrow(|a| show_ai(ui, a.as_ref(), unix_now()));
        }
        Event::Msg(S2C::Ai(a)) => {
            show_ai(ui, a.as_ref(), unix_now());
            LAST_AI.set(a);
        }
        Event::Msg(S2C::Top(t)) => set_vec(
            ui.get_lm_top(),
            smooth_top(&t).iter().map(top_row).collect(),
            |v| ui.set_lm_top(v),
        ),
        Event::Msg(S2C::Alarms(a)) => ui.set_alarms(a.join(", ").into()),
        Event::Msg(S2C::Event(e)) => {
            // Aktif liste deck'te de tutulur; burada yalnız gösterim için güncellenir.
            let mut a: Vec<String> = ui
                .get_alarms()
                .split(", ")
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            match e.kind {
                0 if !a.contains(&e.name) => a.push(e.name.clone()),
                1 => a.retain(|x| *x != e.name),
                _ => {}
            }
            ui.set_alarms(a.join(", ").into());
            prepend(ui, &[e], offset);
        }
        // Parçalar eskiden yeniye gelir; her biri başa eklenince liste yeniden eskiye sıralı kalır.
        Event::Msg(S2C::Events(v)) => prepend(ui, &v, offset),
        Event::Msg(S2C::History {
            from_ms,
            step_ms,
            points,
        }) => {
            let done = HQ.with(|h| {
                let mut h = h.borrow_mut();
                if h.pending == 0 {
                    return false;
                }
                if points.is_empty() {
                    h.pending -= 1;
                    return h.pending == 0;
                }
                if h.pending == 1 {
                    (h.from_ms, h.step_ms) = (from_ms, step_ms);
                    h.pts.extend(points);
                }
                false
            });
            if done {
                draw_history(ui, offset);
            }
        }
        Event::Msg(_) => {}
    }
}

/// Geçmiş sorgusu sürerken gelen canlı olay geçmişte de olabilir: baştaki satırlarda aynısı varsa eklenmez.
fn prepend(ui: &MainWindow, v: &[EventItem], offset: i64) {
    let model = ui.get_events();
    let Some(m) = model.as_any().downcast_ref::<VecModel<EventRow>>() else {
        return;
    };
    for e in v {
        add(m, event_row(e, offset));
    }
    while m.row_count() > EVENTS_MAX {
        m.remove(m.row_count() - 1);
    }
}

/// Çift kontrolü için bakılan baş satır sayısı (sorgu süresince gelebilecek canlı olaylardan fazla).
const DEDUP_ROWS: usize = 50;

/// Başa ekler; aynı saniye + tür + ad baştaki satırlarda varsa (aynı olay) eklemez.
fn add(m: &VecModel<EventRow>, row: EventRow) {
    if !m
        .iter()
        .take(DEDUP_ROWS)
        .any(|r| r.time == row.time && r.kind == row.kind && r.name == row.name)
    {
        m.insert(0, row);
    }
}

fn event_row(e: &EventItem, offset: i64) -> EventRow {
    EventRow {
        time: fmt_time(e.ts_ms, offset).into(),
        kind: e.kind as i32,
        name: e.name.as_str().into(),
        value: fmt_event_value(e).into(),
    }
}

/// Alarm değeri × 100 gelir (LM); sistem olayının değeri kod → gösterilmez.
// shortcut: birim addan tahmin (cpu_/ram_ → %); LM yeni alarm türü eklerse birim buraya
fn fmt_event_value(e: &EventItem) -> String {
    if e.kind == 2 {
        return String::new();
    }
    let unit = if e.name.starts_with("cpu") || e.name.starts_with("ram") {
        "%"
    } else {
        ""
    };
    format!("{:.1}{unit}", e.value as f64 / 100.0)
}

/// Tamamlanan geçmiş yanıtından 7 çizgi + ölçek etiketleri.
fn draw_history(ui: &MainWindow, offset: i64) {
    HQ.with(|h| {
        let h = h.borrow();
        let pts = &h.pts;
        let mut buf = String::new();
        let mut line = |f: &dyn Fn(&HistPoint) -> f32,
                        set: &dyn Fn(&MainWindow, slint::SharedString)| {
            series_path(pts.iter().map(|p| p.as_ref().map(f)), &mut buf);
            set(ui, buf.as_str().into());
        };
        let max = |f: &dyn Fn(&HistPoint) -> u32| pts.iter().flatten().map(f).max().unwrap_or(0);
        line(
            &|p| p.cpu_avg as f32 / 10_000.0,
            &MainWindow::set_hist_cpu_avg,
        );
        line(
            &|p| p.cpu_max as f32 / 10_000.0,
            &MainWindow::set_hist_cpu_max,
        );
        let ram = if h.ram_total_mb > 0 {
            h.ram_total_mb
        } else {
            max(&|p| p.ram_mb).max(1)
        } as f32;
        line(&|p| p.ram_mb as f32 / ram, &MainWindow::set_hist_ram);
        let disk = nice_scale(max(&|p| p.disk_r_kbps.max(p.disk_w_kbps))) as f32;
        line(
            &|p| p.disk_r_kbps as f32 / disk,
            &MainWindow::set_hist_disk_r,
        );
        line(
            &|p| p.disk_w_kbps as f32 / disk,
            &MainWindow::set_hist_disk_w,
        );
        let net = nice_scale(max(&|p| p.net_rx_kbps.max(p.net_tx_kbps))) as f32;
        line(
            &|p| p.net_rx_kbps as f32 / net,
            &MainWindow::set_hist_net_rx,
        );
        line(
            &|p| p.net_tx_kbps as f32 / net,
            &MainWindow::set_hist_net_tx,
        );
        ui.set_hist_ram_scale(format!("{:.0} GB", ram / 1024.0).into());
        ui.set_hist_disk_scale(rate(disk as u32).into());
        ui.set_hist_net_scale(rate(net as u32).into());
        ui.set_hist_n(pts.len().max(1) as f32);
        let to_ms = h.from_ms + h.step_ms * pts.len() as u64;
        ui.set_hist_range(
            format!(
                "{} – {}",
                fmt_time(h.from_ms, offset),
                fmt_time(to_ms, offset)
            )
            .into(),
        );
        let empty = pts.iter().all(Option::is_none);
        ui.set_hist_status(
            if empty {
                tr(
                    "Veri yok (LowMonitor servisi?)",
                    "No data (LowMonitor service?)",
                )
            } else {
                ""
            }
            .into(),
        );
    });
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// AI bandı (ADR-0032); `None` = tray'de veri yok → band gizli.
fn show_ai(ui: &MainWindow, a: Option<&AiUsage>, now: i64) {
    let Some(a) = a else {
        ui.set_ai_state(2);
        return;
    };
    ui.set_ai_state(1);
    // 3 = soluk (eski veri), aksi halde tray'in seviyesi
    let level = |w: &Option<AiWindow>| {
        if a.stale {
            3
        } else {
            w.map_or(0, |w| w.level.min(2) as i32)
        }
    };
    ui.set_ai_session(ai_line(tr("Oturum", "Session"), &a.session, now).into());
    ui.set_ai_week(ai_line(tr("Hafta", "Week"), &a.week, now).into());
    ui.set_ai_session_level(level(&a.session));
    ui.set_ai_week_level(level(&a.week));
    let note = if !a.error.is_empty() {
        a.error.clone()
    } else if a.stale {
        format!(
            "{} · {} {}",
            tr("eski", "stale"),
            (now - a.fetched_at as i64).max(0) / 60,
            tr("dk önce", "min ago")
        )
    } else {
        a.source.clone()
    };
    ui.set_ai_note(note.into());
}

/// "Oturum %33 · 4s16dk" (taskbar ile aynı biçim).
fn ai_line(label: &str, w: &Option<AiWindow>, now: i64) -> String {
    use lowmon_ai::text::{lang, pct, remaining};
    match w {
        None => format!("{label} —"),
        Some(w) => {
            let left = if w.resets_at == 0 {
                "—".into()
            } else {
                remaining(lang(), w.resets_at as i64 - now)
            };
            format!("{label} {} · {left}", pct(lang(), w.pct as f64 / 10.0))
        }
    }
}

fn show_metrics(ui: &MainWindow, m: &Metrics) {
    ui.set_lm_state(1);
    HQ.with(|h| h.borrow_mut().ram_total_mb = m.ram_total_mb);
    ui.set_lm_cpu(format!("{:.0}%", m.cpu as f32 / 100.0).into());
    ui.set_lm_ram(
        format!(
            "{:.1} / {:.0} GB",
            m.ram_used_mb as f32 / 1024.0,
            m.ram_total_mb as f32 / 1024.0
        )
        .into(),
    );
    ui.set_lm_disk(format!("R {}  W {}", rate(m.disk_r_kbps), rate(m.disk_w_kbps)).into());
    ui.set_lm_net(format!("↓ {}  ↑ {}", rate(m.net_rx_kbps), rate(m.net_tx_kbps)).into());
    let cores = m.cores.iter().map(|&c| c as f32 / 200.0).collect();
    set_vec(ui.get_lm_cores(), cores, |v| ui.set_lm_cores(v));
    graphs(ui, m, now_ms() / 1000);
}

/// Örneği geçmişe ekler, 6 çizgi + 2 ölçek etiketini günceller (Metrics başına bir kez, ≈ 1 Hz).
fn graphs(ui: &MainWindow, m: &Metrics, now_s: u64) {
    HIST.with(|h| {
        let (h, buf) = &mut *h.borrow_mut();
        h.push(Sample {
            ts_s: now_s,
            cpu: m.cpu,
            ram: m.ram_used_mb as f32 / m.ram_total_mb.max(1) as f32,
            dr: m.disk_r_kbps,
            dw: m.disk_w_kbps,
            rx: m.net_rx_kbps,
            tx: m.net_tx_kbps,
        });
        let mut line = |f: &dyn Fn(&Sample) -> f32, set: &dyn Fn(slint::SharedString)| {
            h.path(now_s, f, buf);
            set(buf.as_str().into());
        };
        line(&|s| s.cpu as f32 / 10_000.0, &|v| ui.set_lm_cpu_path(v));
        line(&|s| s.ram, &|v| ui.set_lm_ram_path(v));
        // Disk/ağ: iki çizgi ortak ölçekte (penceredeki max, 1-2-5 yuvarlama).
        let disk = nice_scale(h.max(now_s, |s| s.dr.max(s.dw))) as f32;
        line(&|s| s.dr as f32 / disk, &|v| ui.set_lm_disk_r_path(v));
        line(&|s| s.dw as f32 / disk, &|v| ui.set_lm_disk_w_path(v));
        let net = nice_scale(h.max(now_s, |s| s.rx.max(s.tx))) as f32;
        line(&|s| s.rx as f32 / net, &|v| ui.set_lm_net_rx_path(v));
        line(&|s| s.tx as f32 / net, &|v| ui.set_lm_net_tx_path(v));
        ui.set_lm_disk_scale(rate(disk as u32).into());
        ui.set_lm_net_scale(rate(net as u32).into());
    });
}

fn clear_graphs(ui: &MainWindow) {
    HIST.with(|h| h.borrow_mut().0.clear());
    for set in [
        MainWindow::set_lm_cpu_path,
        MainWindow::set_lm_ram_path,
        MainWindow::set_lm_disk_r_path,
        MainWindow::set_lm_disk_w_path,
        MainWindow::set_lm_net_rx_path,
        MainWindow::set_lm_net_tx_path,
    ] {
        set(ui, "".into());
    }
}

/// Top-5'i EMA ile yumuşatıp ona göre sıralar (anlık sıralamada isimler her saniye yer değiştiriyordu).
// shortcut: aynı adlı süreçler (ör. iki claude.exe) LM'den PID'siz gelir; anahtar = (ad, geliş sırası), sıra değişirse değerleri karışabilir
fn smooth_top(t: &[TopItem]) -> Vec<TopItem> {
    TOP_EMA.with(|m| {
        let mut m = m.borrow_mut();
        let mut next = HashMap::with_capacity(t.len());
        let mut out: Vec<TopItem> = t
            .iter()
            .map(|it| {
                let n = next
                    .keys()
                    .filter(|(name, _): &&(String, u8)| *name == it.name)
                    .count() as u8;
                let key = (it.name.clone(), n);
                let cpu = procs::ema(m.get(&key).copied(), it.cpu);
                next.insert(key, cpu);
                TopItem { cpu, ..it.clone() }
            })
            .collect();
        *m = next;
        out.sort_by_key(|i| std::cmp::Reverse(i.cpu));
        out
    })
}

fn top_row(t: &TopItem) -> TopRow {
    TopRow {
        name: t.name.as_str().into(),
        cpu: format!("{:.1}%", t.cpu as f32 / 100.0).into(),
        ws: format!("{} MB", t.ws_mb).into(),
    }
}

fn rate(kbps: u32) -> String {
    match kbps {
        0..1024 => format!("{kbps}K"),
        1024..1_048_576 => format!("{:.1}M", kbps as f32 / 1024.0),
        _ => format!("{:.1}G", kbps as f32 / 1_048_576.0),
    }
}

fn set_vec<T: Clone + 'static>(cur: ModelRc<T>, v: Vec<T>, set: impl FnOnce(ModelRc<T>)) {
    match cur.as_any().downcast_ref::<VecModel<T>>() {
        Some(m) => m.set_vec(v),
        None => set(ModelRc::new(VecModel::from(v))),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// shortcut: yerel saat farkı açılışta bir kez alınır; yaz saatine geçişte uygulama yeniden açılana kadar 1 saat kayar
fn local_offset_s() -> i64 {
    #[cfg(windows)]
    return win::utc_offset_s();
    #[cfg(not(windows))]
    0
}

/// `YYYY-MM-DD HH:MM:SS` (yerel). Gün → takvim: H. Hinnant `civil_from_days`.
fn fmt_time(ts_ms: u64, offset_s: i64) -> String {
    let s = ts_ms as i64 / 1000 + offset_s;
    let (days, sod) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_line_format() {
        let w = Some(AiWindow {
            pct: 334,
            resets_at: 1_000 + 4 * 3600 + 16 * 60,
            level: 1,
        });
        assert_eq!(ai_line("Oturum", &w, 1_000), "Oturum %33 · 4s16dk");
        assert_eq!(ai_line("Hafta", &None, 0), "Hafta —");
        let w = Some(AiWindow {
            pct: 1000,
            resets_at: 0,
            level: 2,
        });
        assert_eq!(ai_line("Hafta", &w, 0), "Hafta %100 · —");
    }

    #[test]
    fn time_format() {
        assert_eq!(fmt_time(0, 0), "1970-01-01 00:00:00");
        // 2025-11-15 13:40:05 UTC, TSİ +3
        assert_eq!(fmt_time(1_763_214_005_000, 3 * 3600), "2025-11-15 16:40:05");
        assert_eq!(fmt_time(951_782_400_000, 0), "2000-02-29 00:00:00");
    }

    #[test]
    fn dedup() {
        let m = VecModel::default();
        let e = |ts, kind| {
            event_row(
                &EventItem {
                    ts_ms: ts,
                    kind,
                    name: "cpu".into(),
                    value: 1,
                },
                0,
            )
        };
        add(&m, e(1_000, 0)); // canlı
        add(&m, e(500, 2));
        add(&m, e(1_400, 0)); // geçmişten aynı olay (aynı saniye)
        add(&m, e(1_000, 1)); // farklı tür
        assert_eq!(m.row_count(), 3);
    }

    #[test]
    fn event_value() {
        let ev = |kind, name: &str, value| {
            fmt_event_value(&EventItem {
                ts_ms: 0,
                kind,
                name: name.into(),
                value,
            })
        };
        assert_eq!(ev(0, "cpu_high", 5917), "59.2%");
        assert_eq!(ev(1, "ram_high", 10000), "100.0%");
        assert_eq!(ev(0, "disk_x", 250), "2.5");
        assert_eq!(ev(2, "service_start", 0), "");
    }
}
