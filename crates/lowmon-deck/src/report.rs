//! LowDeck ADR-0009 sorguları: sıcaklık/GPU geçmişi (`sensors.bin`) ve AI raporu (`lowmon-ai::report`).
//! Dosya okur; istemcinin okuma thread'inde çağrılır (diğer istemciler etkilenmez).

use lowdeck_proto::{
    AI_GROUPS_MAX, AI_LINE_POINTS, AI_NAME_MAX, AI_SESSIONS_MAX, AiGroup, AiReport, AiSession,
    TempPoint,
};
use lowmon_ai::breakdown::group_label;
use lowmon_ai::report::{self, Data, View};
use lowmon_sensors::log::{self, Rec};

/// `QueryHistory` kovalarıyla hizalı: aynı `from/to/n` → aynı `step`.
pub fn temp_points(from_ms: u64, to_ms: u64, n: u16) -> (u64, Vec<Option<TempPoint>>) {
    let n = n.max(1) as usize;
    let step = (to_ms.saturating_sub(from_ms) / n as u64).max(1);
    let recs = log::read(
        &log::path(),
        (from_ms / 60_000) as u32,
        (from_ms + step * n as u64).div_ceil(60_000) as u32,
    )
    .unwrap_or_default();
    (step, to_points(&log::buckets(&recs, from_ms, step, n)))
}

fn to_points(b: &[Option<Rec>]) -> Vec<Option<TempPoint>> {
    let c = |v: Option<f32>| v.map(|v| v.round() as i16);
    b.iter()
        .map(|r| {
            r.map(|r| TempPoint {
                cpu_c: c(r.cpu.avg),
                gpu_pct: r.gpu_pct.avg.map(|v| v.round().clamp(0.0, 100.0) as u8),
                gpu_c: c(r.gpu_c.avg),
                disk_c_max: c(r.disk.max),
            })
        })
        .collect()
}

/// lowmon-ui AI sekmesiyle aynı veri. `view` 0 son oturum, 1 bugün, 2 hafta; `off` = yerel − UTC (s).
pub fn ai_report(view: u8, now: i64, off: i64) -> AiReport {
    let cfg = lowmon_ai::Config::load(&lowmon_ai::config_path()).unwrap_or_default();
    let d = report::build(&lowmon_ai::report_paths(&cfg), cfg.report_sessions, now);
    ai_report_from(&d, view, off)
}

fn cut(s: &str) -> String {
    s.chars().take(AI_NAME_MAX).collect()
}

/// 0..1 → 0.1 %.
fn permille(share: f64) -> u16 {
    (share * 1000.0).round().clamp(0.0, u16::MAX as f64) as u16
}

pub fn ai_report_from(d: &Data, view: u8, off: i64) -> AiReport {
    let v = [View::LastSession, View::Today, View::Week][view.min(2) as usize];
    let from = report::range_from(v, d.now, off);
    let to = d.now.max(from + 60);
    // Kova ortalaması; % → 0.1 %
    let line = |f: fn(&report::Sample) -> Option<f64>| {
        let mut sum = vec![(0.0, 0u32); AI_LINE_POINTS];
        for s in d.samples.iter().filter(|s| s.ts >= from && s.ts <= to) {
            if let Some(p) = f(s) {
                let i = ((s.ts - from) as usize * AI_LINE_POINTS / (to - from) as usize)
                    .min(AI_LINE_POINTS - 1);
                sum[i].0 += p;
                sum[i].1 += 1;
            }
        }
        sum.into_iter()
            .map(|(s, n)| (n > 0).then(|| (s / n as f64 * 10.0).round().clamp(0.0, 65535.0) as u16))
            .collect()
    };
    let lang = lowmon_ai::text::lang();
    let sessions = report::sessions_in(&d.sessions, v, from);
    let total = report::merge(&sessions);
    AiReport {
        from_s: from.max(0) as u64,
        to_s: to.max(0) as u64,
        session_line: line(|s| s.session_pct),
        week_line: line(|s| s.week_pct),
        session_count: sessions.len().min(u16::MAX as usize) as u16,
        request_count: total.request_count as u32,
        total_tokens: total.total_effective as u64,
        groups: total
            .groups
            .iter()
            .take(AI_GROUPS_MAX)
            .map(|g| AiGroup {
                name: cut(&group_label(lang, &g.name)),
                share: permille(g.share),
                tokens: g.effective as u64,
            })
            .collect(),
        sessions: sessions
            .iter()
            .take(AI_SESSIONS_MAX)
            .map(|s| {
                let top = s.groups.first();
                AiSession {
                    ended_at: s.ended_at.unwrap_or(0).max(0) as u64,
                    project: cut(&s.project),
                    requests: s.request_count as u32,
                    tokens: s.total_effective as u64,
                    top: top.map_or(String::new(), |g| cut(&group_label(lang, &g.name))),
                    top_share: top.map_or(0, |g| permille(g.share)),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowdeck_proto::{MAX_FRAME, S2C};
    use lowmon_ai::breakdown::{GroupCost, Session};
    use lowmon_sensors::log::Agg;

    #[test]
    fn temp_point_units() {
        let a = |v| Agg {
            avg: Some(v),
            max: Some(v + 10.0),
        };
        let r = Rec {
            ts_min: 0,
            cpu: a(45.4),
            gpu_c: Agg::default(),
            disk: a(30.0),
            gpu_pct: a(12.6),
        };
        let p = to_points(&[Some(r), None]);
        assert_eq!(
            p[0],
            Some(TempPoint {
                cpu_c: Some(45),
                gpu_pct: Some(13),
                gpu_c: None,
                disk_c_max: Some(40),
            })
        );
        assert_eq!(p[1], None);
    }

    /// Uzun adlar ve çok oturum → kırpılır, tek çerçeveye sığar; çizgi kovaları doğru yerde.
    #[test]
    fn ai_report_fits() {
        let now = 1_800_000_000;
        let sess = |i: i64| Session {
            project: "p".repeat(200),
            ended_at: Some(now - i * 60),
            request_count: 3,
            total_effective: 1000.0,
            groups: (0..20)
                .map(|g| GroupCost {
                    name: format!("{}{g}", "g".repeat(100)),
                    effective: 50.0,
                    share: 0.05,
                })
                .collect(),
            ..Default::default()
        };
        let d = Data {
            now,
            samples: vec![
                report::Sample {
                    ts: now - 7 * 86_400,
                    session_pct: Some(10.0),
                    week_pct: None,
                },
                report::Sample {
                    ts: now,
                    session_pct: Some(55.5),
                    week_pct: Some(20.0),
                },
            ],
            sessions: (0..50).map(sess).collect(),
            parsed: 0,
        };
        let r = ai_report_from(&d, 2, 3 * 3600);
        assert_eq!(r.session_count, 50);
        assert_eq!(r.sessions.len(), AI_SESSIONS_MAX);
        assert_eq!(r.groups.len(), AI_GROUPS_MAX);
        assert_eq!(r.session_line.len(), AI_LINE_POINTS);
        assert_eq!(r.session_line[0], Some(100));
        assert_eq!(r.session_line[AI_LINE_POINTS - 1], Some(555));
        assert_eq!(r.week_line[0], None);
        let mut buf = Vec::new();
        lowdeck_proto::write_frame(&mut buf, &S2C::AiReport(r)).unwrap();
        assert!(buf.len() <= MAX_FRAME);
    }
}
