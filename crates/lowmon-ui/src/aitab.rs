//! AI sekmesi (ADR-0032 Faz 5): limit çizgisi, token dökümü, oturum tablosu.
//! Veri `lowmon_ai::report` (dosyalardan, arka iş parçacığında); görünüm değişimi yalnız yeniden çizer.
use crate::{GroupRow, MainWindow, SessRow, fmt_time, unix_now};
use lowmon_ai::report::{self, Data, View, format_tokens};
use lowmon_ai::text::{Lang, lang, tr};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};

thread_local! {
    static DATA: RefCell<Option<Data>> = const { RefCell::new(None) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// Son çizilen grafik aralığı (unix s); hover bunu kullanır.
    static RANGE: Cell<(i64, i64)> = const { Cell::new((0, 0)) };
}

pub fn init(ui: &MainWindow, offset: i64) {
    let w = ui.as_weak();
    ui.on_ai_report(move |rebuild| {
        let Some(ui) = w.upgrade() else { return };
        if rebuild {
            refresh(&ui, offset);
        } else {
            render(&ui, offset);
        }
    });
}

/// Raporu arka planda kurar; sürerken ikinci istek yok sayılır.
pub fn refresh(ui: &MainWindow, offset: i64) {
    if BUSY.replace(true) {
        return;
    }
    ui.set_ai_rep_status(tr("Yükleniyor…", "Loading…").into());
    let w = ui.as_weak();
    std::thread::spawn(move || {
        // ai.toml bozuksa tray zaten hata gösterir; rapor varsayılanla çıkar.
        let cfg = lowmon_ai::Config::load(&lowmon_ai::config_path()).unwrap_or_default();
        let d = report::build(
            &lowmon_ai::report_paths(&cfg),
            cfg.report_sessions,
            unix_now(),
        );
        let _ = w.upgrade_in_event_loop(move |ui| {
            BUSY.set(false);
            DATA.set(Some(d));
            render(&ui, offset);
        });
    });
}

/// "%47,7" / "47.7%"
fn pct1(share: f64) -> String {
    let v = format!("{:.1}", 100.0 * share);
    if lang() == Lang::En {
        format!("{v}%")
    } else {
        format!("%{}", v.replace('.', ","))
    }
}

fn render(ui: &MainWindow, offset: i64) {
    DATA.with_borrow(|d| {
        let Some(d) = d else { return };
        let view =
            [View::LastSession, View::Today, View::Week][ui.get_ai_view().clamp(0, 2) as usize];
        let from = report::range_from(view, d.now, offset);
        let to = d.now.max(from + 60);
        RANGE.set((from, to));
        // viewbox genişliği report::CHART_W = main.slint'teki vb: 1000
        ui.set_ai_chart_s(report::chart_path(&d.samples, from, to, |s| s.session_pct).into());
        ui.set_ai_chart_w(report::chart_path(&d.samples, from, to, |s| s.week_pct).into());

        let sessions = report::sessions_in(&d.sessions, view, from);
        let total = report::merge(&sessions);
        let top = total.groups.first().map_or(1.0, |g| g.effective);
        let row = |name: &str, active: bool| GroupRow {
            name: lowmon_ai::breakdown::group_label(lang(), name).into(),
            pct: String::new().into(),
            tokens: String::new().into(),
            fill: 0.0,
            active,
        };
        let mut groups: Vec<GroupRow> = total
            .groups
            .iter()
            .filter(|g| g.effective > 0.0)
            .map(|g| GroupRow {
                pct: pct1(g.share).into(),
                tokens: format_tokens(g.effective, lang()).into(),
                fill: (g.effective / top) as f32,
                ..row(&g.name, true)
            })
            .collect();
        // Bu görünümde kullanılmayan ama yüklü raporda geçen gruplar: kaybolmasın, gri kalsın
        let mut idle: Vec<&str> = d
            .sessions
            .iter()
            .flat_map(|s| s.groups.iter().map(|g| g.name.as_str()))
            .filter(|n| {
                !total
                    .groups
                    .iter()
                    .any(|g| g.name == *n && g.effective > 0.0)
            })
            .collect();
        idle.sort_unstable();
        idle.dedup();
        groups.extend(idle.into_iter().map(|n| row(n, false)));
        let rows: Vec<SessRow> = sessions
            .iter()
            .map(|s| SessRow {
                // "MM-DD HH:MM"
                end: s
                    .ended_at
                    .map_or("—".into(), |e| {
                        fmt_time(e as u64 * 1000, offset)[5..16].to_string()
                    })
                    .into(),
                project: if s.project.is_empty() {
                    "—"
                } else {
                    &s.project
                }
                .into(),
                requests: s.request_count.to_string().into(),
                tokens: format_tokens(s.total_effective, lang()).into(),
                top: s
                    .groups
                    .first()
                    .map_or("—".into(), |g| {
                        format!(
                            "{} {}",
                            lowmon_ai::breakdown::group_label(lang(), &g.name),
                            pct1(g.share)
                        )
                    })
                    .into(),
            })
            .collect();
        ui.set_ai_rep_sub(
            format!(
                "{} {} · {} {} · {} {}",
                sessions.len(),
                tr("oturum", "sessions"),
                total.request_count,
                tr("istek", "requests"),
                tr("toplam", "total"),
                format_tokens(total.total_effective, lang())
            )
            .into(),
        );
        ui.set_ai_rep_status(
            if sessions.is_empty() && d.samples.is_empty() {
                tr("Veri yok", "No data")
            } else {
                ""
            }
            .into(),
        );
        ui.set_ai_groups(ModelRc::new(VecModel::from(groups)));
        ui.set_ai_sessions(ModelRc::new(VecModel::from(rows)));
    });
}

/// Limit grafiği hover kutucuğu: imlece en yakın örneğin tarih-saati + oturum / hafta %.
pub fn hover_text(frac: f32, offset: i64) -> String {
    let (from, to) = RANGE.get();
    DATA.with_borrow(|d| {
        let Some(d) = d else { return String::new() };
        let t = from + ((to - from) as f64 * frac.clamp(0.0, 1.0) as f64) as i64;
        let Some(s) = d
            .samples
            .iter()
            .filter(|s| s.ts >= from && s.ts <= to)
            .min_by_key(|s| (s.ts - t).abs())
        else {
            return String::new();
        };
        let p = |v: Option<f64>| v.map_or("—".into(), |v| format!("{v:.0} %"));
        format!(
            "{}
{} {}  {} {}",
            &fmt_time(s.ts as u64 * 1000, offset)[5..16],
            tr("oturum", "session"),
            p(s.session_pct),
            tr("hafta", "week"),
            p(s.week_pct)
        )
    })
}
