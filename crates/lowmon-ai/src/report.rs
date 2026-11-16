//! Rapor verisi (ADR-0032 Faz 5; ai-usage-bar `report_builder` + `report_html`'in veri kısmı):
//! limit CSV'si örnekleri, son oturum dökümlerinin token dağılımı (önbellekli), bulut özetleri.
//! Çizim `lowmon-ui`'da; burada yalnız veri ve biçim.
use crate::breakdown::{Analyzer, GroupCost, Session, parse_summary_json, to_summary_json};
use crate::json;
use crate::usage::{SESSION_SEC, csv_file_name};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DAY: i64 = 86_400;
/// Bir hafta + pay
const LOOKBACK: Duration = Duration::from_secs(8 * 86_400);
/// Daha uzun boşluk (sorgu kapalı) çizgiyi koparır.
const GAP_SEC: i64 = 20 * 60;
/// Grafik viewbox genişliği
pub const CHART_W: f64 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub ts: i64,
    pub session_pct: Option<f64>,
    pub week_pct: Option<f64>,
}

/// `ts,session_pct,session_reset,week_pct,week_reset`; boş alan = yok. Başlık / bozuk satır → None.
pub fn parse_csv_row(line: &str) -> Option<Sample> {
    let f: Vec<&str> = line.trim_end_matches('\r').split(',').collect();
    let [ts, s, _, w, _] = f.as_slice() else {
        return None;
    };
    let opt = |v: &str| {
        if v.is_empty() {
            Some(None)
        } else {
            v.parse().ok().map(Some)
        }
    };
    Some(Sample {
        ts: ts.parse().ok()?,
        session_pct: opt(s)?,
        week_pct: opt(w)?,
    })
}

pub struct Paths {
    /// `%LOCALAPPDATA%\LowMonitor\ai`: CSV + `breakdown-cache.jsonl`
    pub data_dir: PathBuf,
    /// `~/.claude`
    pub claude_dir: PathBuf,
    /// Bulut oturum özetleri (`*_usage.json`)
    pub summary_dir: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct Data {
    pub now: i64,
    /// ts'e göre artan, son 8 gün
    pub samples: Vec<Sample>,
    pub sessions: Vec<Session>,
    /// Önbellekte olmayıp bu kez ayrıştırılan döküm sayısı
    pub parsed: usize,
}

fn load_samples(dir: &Path, now: i64) -> Vec<Sample> {
    let from = now - 8 * DAY;
    let mut names = vec![csv_file_name(from), csv_file_name(now)];
    names.dedup();
    let mut out: Vec<Sample> = names
        .iter()
        .filter_map(|n| std::fs::read_to_string(dir.join(n)).ok())
        .flat_map(|t| {
            t.lines()
                .filter_map(parse_csv_row)
                .filter(|s| s.ts >= from)
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort_by_key(|s| s.ts);
    out
}

struct Transcript {
    path: PathBuf,
    mtime: SystemTime,
    size: u64,
    project: String,
}

fn cache_key(t: &Transcript) -> String {
    let ms = t
        .mtime
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("{}|{ms}|{}", t.path.display(), t.size)
}

fn jsonl_files(dir: &Path) -> impl Iterator<Item = (PathBuf, std::fs::Metadata)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let m = e.metadata().ok()?;
            (m.is_file() && p.extension().is_some_and(|x| x == "jsonl")).then_some((p, m))
        })
}

/// En yeni `max` döküm; bir haftadan eskiler atılır ama en yenisi hep kalır ("Son oturum" boş kalmasın).
fn list_transcripts(claude_dir: &Path, max: usize) -> Vec<Transcript> {
    let mut all: Vec<Transcript> = std::fs::read_dir(claude_dir.join("projects"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|pd| pd.path().is_dir())
        .flat_map(|pd| {
            let project = pd.file_name().to_string_lossy().into_owned();
            jsonl_files(&pd.path())
                .map(|(path, m)| Transcript {
                    path,
                    mtime: m.modified().unwrap_or(UNIX_EPOCH),
                    size: m.len(),
                    project: project.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all.sort_by_key(|t| std::cmp::Reverse(t.mtime));
    let cutoff = SystemTime::now() - LOOKBACK;
    let keep = all
        .iter()
        .take(max)
        .enumerate()
        .take_while(|(i, t)| *i == 0 || t.mtime >= cutoff)
        .count();
    all.truncate(keep);
    all
}

fn analyze_file(path: &Path, mut a: Analyzer) -> Option<Session> {
    let mut r = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line).ok()? == 0 {
            break;
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        a.feed_line(&line);
    }
    Some(a.finish())
}

fn analyze_session(t: &Transcript) -> Option<Session> {
    let mut a = Analyzer::default();
    // Claude Code subagent dökümleri: <proje>/<oturum>/subagents/*.jsonl
    let sub_dir = t.path.with_extension("").join("subagents");
    for (p, _) in jsonl_files(&sub_dir) {
        if let Some(s) = analyze_file(&p, Analyzer::default()) {
            a.add_subagent(&s);
        }
    }
    let mut r = analyze_file(&t.path, a)?;
    r.project = t.project.clone();
    Some(r)
}

/// Satır başına `{"k":<anahtar>,"r":<özet>}`; anahtar = yol|mtime|boyut.
fn load_cache(file: &Path) -> HashMap<String, Session> {
    let text = std::fs::read(file).unwrap_or_default();
    text.split(|&b| b == b'\n')
        .filter_map(|line| {
            let root = json::root(line)?;
            let k = json::as_string(line, json::member(line, root, "k")?)?;
            let r = json::member(line, root, "r")?;
            Some((k, parse_summary_json(&line[r.begin..r.end])?))
        })
        .collect()
}

fn load_summaries(dir: &Path, out: &mut Vec<Session>) {
    let cutoff = SystemTime::now() - LOOKBACK;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let fresh = e
            .metadata()
            .is_ok_and(|m| m.is_file() && m.modified().is_ok_and(|t| t >= cutoff));
        if !fresh || !e.file_name().to_string_lossy().ends_with("_usage.json") {
            continue;
        }
        if let Some(mut r) = std::fs::read(e.path())
            .ok()
            .and_then(|t| parse_summary_json(&t))
        {
            r.project = format!("{} (bulut)", r.project).trim_start().to_string();
            out.push(r);
        }
    }
}

/// Raporu kurar. Dosya okur (ilk seferde onlarca MB döküm): arayüz iş parçacığında çağrılmaz.
pub fn build(p: &Paths, max_sessions: usize, now: i64) -> Data {
    let cache_file = p.data_dir.join("breakdown-cache.jsonl");
    let old = load_cache(&cache_file);
    let mut d = Data {
        now,
        samples: load_samples(&p.data_dir, now),
        ..Default::default()
    };
    let mut cache = String::new();
    for t in list_transcripts(&p.claude_dir, max_sessions) {
        let key = cache_key(&t);
        let s = match old.get(&key) {
            Some(s) => s.clone(),
            None => {
                let Some(s) = analyze_session(&t) else {
                    continue;
                };
                d.parsed += 1;
                s
            }
        };
        let _ = writeln!(
            cache,
            "{{\"k\":{},\"r\":{}}}",
            json::quote(&key),
            to_summary_json(&s)
        );
        d.sessions.push(s);
    }
    // Önbellek yazılamazsa rapor yine çıkar; sonraki sefer yeniden ayrıştırılır.
    let _ = std::fs::create_dir_all(&p.data_dir).and_then(|()| std::fs::write(&cache_file, cache));
    load_summaries(&p.summary_dir, &mut d.sessions);
    d
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    LastSession,
    Today,
    Week,
}

/// Görünümün başlangıcı (Unix s); `off` = yerel − UTC (s).
pub fn range_from(v: View, now: i64, off: i64) -> i64 {
    match v {
        View::LastSession => now - SESSION_SEC,
        View::Today => (now + off).div_euclid(DAY) * DAY - off,
        View::Week => now - 7 * DAY,
    }
}

/// Görünüme giren oturumlar, yeni → eski. "Son oturum" = en son biten tek oturum.
pub fn sessions_in(all: &[Session], v: View, from: i64) -> Vec<&Session> {
    let mut out: Vec<&Session> = all.iter().collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.ended_at.unwrap_or(0)));
    if v == View::LastSession {
        out.truncate(1);
    } else {
        out.retain(|s| s.ended_at.is_some_and(|e| e >= from));
    }
    out
}

/// Birkaç oturumun grup maliyetlerini toplar; büyükten küçüğe, paylar birleşik toplam üzerinden.
pub fn merge(parts: &[&Session]) -> Session {
    let mut out = Session::default();
    let mut by: BTreeMap<&str, f64> = BTreeMap::new();
    for p in parts {
        out.request_count += p.request_count;
        out.total_effective += p.total_effective;
        for g in &p.groups {
            *by.entry(&g.name).or_default() += g.effective;
        }
    }
    let total = out.total_effective;
    out.groups = by
        .into_iter()
        .map(|(name, effective)| GroupCost {
            name: name.to_string(),
            effective,
            share: if total > 0.0 { effective / total } else { 0.0 },
        })
        .collect();
    out.groups
        .sort_by(|a, b| b.effective.total_cmp(&a.effective));
    out
}

/// 812 → "812", 81200 → "81,2 K", 3040000 → "3,04 M" (ondalık virgül).
pub fn format_tokens(v: f64) -> String {
    let (n, unit) = if v < 1e3 {
        return format!("{v:.0}");
    } else if v < 1e6 {
        (v / 1e3, "K")
    } else {
        (v / 1e6, "M")
    };
    let prec = if n < 10.0 { 2 } else { 1 };
    format!("{n:.prec$} {unit}").replace('.', ",")
}

/// Limit çizgisi → SVG path, viewbox `CHART_W`×100: x zamana oranlı, yok / uzun boşluk çizgiyi koparır.
pub fn chart_path(
    samples: &[Sample],
    from: i64,
    to: i64,
    pick: impl Fn(&Sample) -> Option<f64>,
) -> String {
    let span = (to - from).max(1) as f64;
    let mut out = String::new();
    let mut prev: Option<i64> = None;
    for s in samples.iter().filter(|s| s.ts >= from && s.ts <= to) {
        let Some(v) = pick(s) else {
            prev = None;
            continue;
        };
        let cmd = if prev.is_some_and(|p| s.ts - p <= GAP_SEC) {
            'L'
        } else {
            'M'
        };
        let x = CHART_W * (s.ts - from) as f64 / span;
        let y = 100.0 - v.clamp(0.0, 100.0);
        let _ = write!(out, "{cmd}{x:.1} {y:.1}");
        prev = Some(s.ts);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(end: i64, total: f64, groups: &[(&str, f64)]) -> Session {
        Session {
            ended_at: Some(end),
            request_count: 1,
            total_effective: total,
            groups: groups
                .iter()
                .map(|(n, e)| GroupCost {
                    name: n.to_string(),
                    effective: *e,
                    share: e / total,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn csv_rows() {
        assert_eq!(
            parse_csv_row("1793527786,0.0,,80.0,1793703600\r"),
            Some(Sample {
                ts: 1_793_527_786,
                session_pct: Some(0.0),
                week_pct: Some(80.0)
            })
        );
        assert_eq!(parse_csv_row("5,,,,").unwrap().session_pct, None);
        assert!(parse_csv_row("ts,session_pct,session_reset,week_pct,week_reset").is_none());
        assert!(parse_csv_row("1,2,3").is_none());
        assert!(parse_csv_row("1,x,,,").is_none());
    }

    #[test]
    fn views() {
        // 2026-11-16 22:00 UTC, TSİ (+3) → yerel 17 Kas 01:00; yerel gece yarısı = 21:00 UTC
        let now = 1_794_866_400;
        assert_eq!(range_from(View::Today, now, 3 * 3600), now - 3600);
        assert_eq!(range_from(View::Week, now, 0), now - 7 * DAY);
        let all = [
            session(now - 10 * DAY, 1.0, &[]),
            session(now - 100, 1.0, &[]),
            session(now - 2 * DAY, 1.0, &[]),
        ];
        let ends = |v| -> Vec<i64> {
            sessions_in(&all, v, range_from(v, now, 0))
                .iter()
                .map(|s| now - s.ended_at.unwrap())
                .collect()
        };
        assert_eq!(ends(View::LastSession), [100]);
        assert_eq!(ends(View::Today), [100]);
        assert_eq!(ends(View::Week), [100, 2 * DAY]);
    }

    #[test]
    fn merges_and_formats() {
        let a = session(1, 100.0, &[("x", 60.0), ("y", 40.0)]);
        let b = session(2, 300.0, &[("y", 300.0)]);
        let m = merge(&[&a, &b]);
        assert_eq!((m.request_count, m.total_effective), (2, 400.0));
        assert_eq!(m.groups[0].name, "y");
        assert!((m.groups[0].share - 0.85).abs() < 1e-9);
        assert_eq!(format_tokens(812.0), "812");
        assert_eq!(format_tokens(8120.0), "8,12 K");
        assert_eq!(format_tokens(81_200.0), "81,2 K");
        assert_eq!(format_tokens(3_040_000.0), "3,04 M");
    }

    #[test]
    fn chart_breaks_on_gap_and_missing() {
        let s = |ts, v| Sample {
            ts,
            session_pct: v,
            week_pct: None,
        };
        let pts = [
            s(0, Some(10.0)),
            s(500, Some(20.0)),
            s(600, None),
            s(700, Some(30.0)),
            s(700 + GAP_SEC + 1, Some(150.0)),
        ];
        let p = chart_path(&pts, 0, 2000, |s| s.session_pct);
        assert_eq!(p, "M0.0 90.0L250.0 80.0M350.0 70.0M950.5 0.0");
        assert_eq!(chart_path(&pts, 0, 2000, |s| s.week_pct), "");
    }

    #[test]
    fn builds_from_files_and_caches() {
        let root = std::env::temp_dir().join(format!("lowmon-test-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("claude/projects/demo");
        std::fs::create_dir_all(proj.join("s1/subagents")).unwrap();
        std::fs::create_dir_all(root.join("sum")).unwrap();
        let fixture = include_str!("../fixtures/session_sample.jsonl");
        std::fs::write(proj.join("s1.jsonl"), fixture).unwrap();
        std::fs::write(proj.join("s1/subagents/a.jsonl"), fixture).unwrap();
        let cloud = session(1_793_383_200, 50.0, &[("Web", 50.0)]);
        std::fs::write(root.join("sum/x_usage.json"), to_summary_json(&cloud)).unwrap();
        let p = Paths {
            data_dir: root.join("data"),
            claude_dir: root.join("claude"),
            summary_dir: root.join("sum"),
        };
        std::fs::create_dir_all(&p.data_dir).unwrap();
        let now = 1_793_383_200;
        std::fs::write(
            p.data_dir.join(csv_file_name(now)),
            format!(
                "ts,a,b,c,d\r\n{},1.0,,2.0,\r\n{},3.0,,4.0,\r\n",
                now - 9 * DAY,
                now - 60
            ),
        )
        .unwrap();

        let d = build(&p, 30, now);
        assert_eq!((d.parsed, d.sessions.len(), d.samples.len()), (1, 2, 1));
        // döküm 3048 + subagent 3048
        assert!((d.sessions[0].total_effective - 6096.0).abs() < 1e-6);
        assert_eq!(d.sessions[0].project, "demo");
        assert_eq!(d.sessions[1].project, "(bulut)");
        // ikinci kez: önbellekten, sonuç aynı (özet 1 ondalığa yuvarlanır)
        let d2 = build(&p, 30, now);
        assert_eq!(d2.parsed, 0);
        assert!((d2.sessions[0].total_effective - 6096.0).abs() < 0.06);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
