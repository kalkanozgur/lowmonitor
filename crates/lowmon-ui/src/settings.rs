//! Ayarlar sekmesi: kullanıcı ayar dosyaları (`ai.toml`, `taskbar.toml`, `deck.toml`).
//! Yalnız değişen anahtar yazılır (`toml_edit`: yorum + sıra korunur); sonuç dosyanın kendi
//! ayrıştırıcısıyla doğrulanmadan diske gitmez. Servis `config.toml` (ProgramData) kullanıcıya kapalı.
/// Yazma çekirdeği deck'te (LowDeck ADR-0010: telefon da `ai.toml` yazar).
pub use lowmon_deck::tomlw::{Val, apply, write};

// ---------------- Form ↔ dosyalar ----------------

use crate::SettingsForm;
use lowmon_ai::{Auth, text::Lang};
use lowmon_taskbar::Monitor;

const MONS: [Monitor; 4] = [
    Monitor::Primary,
    Monitor::Left,
    Monitor::Right,
    Monitor::Other,
];

fn mon_name(m: Monitor) -> &'static str {
    match m {
        Monitor::Primary => "primary",
        Monitor::Left => "left",
        Monitor::Right => "right",
        Monitor::Other => "other",
    }
}

/// Blok monitörü: 0 = ortak (`None`), 1..4 = `MONS`.
fn block_idx(m: Option<Monitor>) -> i32 {
    m.and_then(|m| MONS.iter().position(|x| *x == m))
        .map_or(0, |i| i as i32 + 1)
}

/// Üç dosyayı okur; bozuk / eksik dosyada o bölüm varsayılanla dolar, hata metni döner.
pub fn load() -> (SettingsForm, Vec<String>) {
    let mut errs = Vec::new();
    let ai = lowmon_ai::Config::load(&lowmon_ai::config_path()).unwrap_or_else(|e| {
        errs.push(e);
        Default::default()
    });
    let tb = lowmon_taskbar::Config::load(&lowmon_taskbar::config_path()).unwrap_or_else(|e| {
        errs.push(format!("taskbar.toml: {e}"));
        Default::default()
    });
    let (bind, port) = match lowmon_deck::load(&lowmon_deck::config_path()) {
        Ok(d) => (d.bind, d.port.to_string()),
        Err(e) => {
            errs.push(e.to_string());
            (String::new(), String::new())
        }
    };
    let s = |v: &dyn ToString| -> slint::SharedString { v.to_string().into() };
    let f = SettingsForm {
        lang: (ai.language == Lang::En) as i32,
        refresh: s(&ai.refresh_sec),
        auth: match ai.auth {
            Auth::Auto => 0,
            Auth::OAuth => 1,
            Auth::Session => 2,
        },
        csv: ai.csv_log,
        report: s(&ai.report_sessions),
        // Ortak monitörde `other` anlamsız: listede yok, sağ sayılır
        monitor: MONS[..3].iter().position(|m| *m == tb.monitor).unwrap_or(2) as i32,
        font_pt: s(&tb.font_pt),
        ai_on: tb.ai.enabled,
        ai_mon: block_idx(tb.ai.monitor),
        bubble: tb.ai.bubble,
        sys_on: tb.sys.enabled,
        // `[sys] monitor` yoksa `other`: "ortak" seçeneği yok, 0..3 = MONS
        sys_mon: MONS
            .iter()
            .position(|m| Some(*m) == tb.sys.monitor)
            .unwrap_or(3) as i32,
        cpu_temp: tb.sys.cpu_temp,
        warn: s(&tb.sys.warn_pct),
        crit: s(&tb.sys.crit_pct),
        twarn: s(&tb.sys.temp_warn_c),
        tcrit: s(&tb.sys.temp_crit_c),
        dwarn: s(&tb.sys.disk_warn_c),
        dcrit: s(&tb.sys.disk_crit_c),
        bind: bind.into(),
        port: port.into(),
    };
    (f, errs)
}

fn int(name: &str, s: &str, lo: i64, hi: i64) -> Result<i64, String> {
    match s.trim().parse::<i64>() {
        Ok(v) if (lo..=hi).contains(&v) => Ok(v),
        _ => Err(format!("{name}: {lo}–{hi}")),
    }
}

fn pair(name: &str, w: &str, c: &str, hi: i64) -> Result<(i64, i64), String> {
    let (w, c) = (int(name, w, 1, hi)?, int(name, c, 1, hi)?);
    if w >= c {
        return Err(format!(
            "{name}: {}",
            crate::tr("uyarı < kritik olmalı", "warn must be < critical")
        ));
    }
    Ok((w, c))
}

type Changes = Vec<(&'static [&'static str], Val)>;

/// Eski ve yeni form farkı → dosya başına değişiklik listesi (ai, taskbar, deck). Geçersiz değer → hata.
pub fn diff(old: &SettingsForm, new: &SettingsForm) -> Result<[Changes; 3], String> {
    let mut ai: Changes = Vec::new();
    let mut tb: Changes = Vec::new();
    let mut deck: Changes = Vec::new();
    macro_rules! ch {
        ($list:ident, $field:ident, $path:expr, $val:expr) => {
            if old.$field != new.$field {
                $list.push(($path, $val));
            }
        };
    }
    ch!(
        ai,
        lang,
        &["language"],
        Val::Str(["tr", "en"][new.lang.clamp(0, 1) as usize].into())
    );
    let refresh = int("refresh_sec", &new.refresh, 30, 86_400)?;
    ch!(ai, refresh, &["refresh_sec"], Val::Int(refresh));
    ch!(
        ai,
        auth,
        &["auth"],
        Val::Str(["auto", "oauth", "session"][new.auth.clamp(0, 2) as usize].into())
    );
    ch!(ai, csv, &["csv_log"], Val::Bool(new.csv));
    let report = int("report_sessions", &new.report, 1, 200)?;
    ch!(ai, report, &["report_sessions"], Val::Int(report));

    ch!(
        tb,
        monitor,
        &["monitor"],
        Val::Str(mon_name(MONS[new.monitor.clamp(0, 2) as usize]).into())
    );
    let pt = match new.font_pt.trim().parse::<f64>() {
        Ok(v) if (6.0..=24.0).contains(&v) => v,
        _ => return Err("font_pt: 6–24".into()),
    };
    ch!(tb, font_pt, &["font_pt"], Val::Float(pt));
    let block = |i: i32| match i {
        1..=4 => Val::Str(mon_name(MONS[i as usize - 1]).into()),
        _ => Val::Remove,
    };
    ch!(tb, ai_on, &["ai", "enabled"], Val::Bool(new.ai_on));
    ch!(tb, ai_mon, &["ai", "monitor"], block(new.ai_mon));
    ch!(tb, bubble, &["ai", "bubble"], Val::Bool(new.bubble));
    ch!(tb, sys_on, &["sys", "enabled"], Val::Bool(new.sys_on));
    ch!(
        tb,
        sys_mon,
        &["sys", "monitor"],
        Val::Str(mon_name(MONS[new.sys_mon.clamp(0, 3) as usize]).into())
    );
    ch!(tb, cpu_temp, &["sys", "cpu_temp"], Val::Bool(new.cpu_temp));
    let (w, c) = pair("%", &new.warn, &new.crit, 100)?;
    ch!(tb, warn, &["sys", "warn_pct"], Val::Int(w));
    ch!(tb, crit, &["sys", "crit_pct"], Val::Int(c));
    let (w, c) = pair("CPU-GPU °C", &new.twarn, &new.tcrit, 150)?;
    ch!(tb, twarn, &["sys", "temp_warn_c"], Val::Int(w));
    ch!(tb, tcrit, &["sys", "temp_crit_c"], Val::Int(c));
    let (w, c) = pair("Disk °C", &new.dwarn, &new.dcrit, 150)?;
    ch!(tb, dwarn, &["sys", "disk_warn_c"], Val::Int(w));
    ch!(tb, dcrit, &["sys", "disk_crit_c"], Val::Int(c));

    if old.bind != new.bind {
        let b = new.bind.trim();
        if b.parse::<std::net::IpAddr>().is_err() {
            return Err(format!(
                "bind: {}",
                crate::tr("geçerli bir IP adresi değil", "not a valid IP address")
            ));
        }
        deck.push((&["bind"], Val::Str(b.into())));
    }
    let port = int("port", &new.port, 1024, 65_535)?;
    ch!(deck, port, &["port"], Val::Int(port));
    Ok([ai, tb, deck])
}

/// Değişen dosyaları yazar; hangi dosyaların yazıldığını döner (tray'e yeniden yükleme için).
pub fn save(old: &SettingsForm, new: &SettingsForm) -> Result<[bool; 3], String> {
    let [ai, tb, deck] = diff(old, new)?;
    write(&lowmon_ai::config_path(), &ai, |s| {
        lowmon_ai::Config::parse(s).map(|_| ())
    })?;
    write(&lowmon_taskbar::config_path(), &tb, |s| {
        lowmon_taskbar::Config::parse(s).map(|_| ())
    })?;
    write(&lowmon_deck::config_path(), &deck, |s| {
        toml::from_str::<lowmon_deck::Config>(s)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    Ok([!ai.is_empty(), !tb.is_empty(), !deck.is_empty()])
}

// ---------------- Servis: ProgramData\LowMonitor\config.toml (yükseltilmiş kopya, ADR-0030 eki) ----------------

use crate::SvcForm;

/// `%ProgramData%\LowMonitor\config.toml` (`lowmon-svc` ile aynı yol).
pub fn svc_config_path() -> std::path::PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into());
    std::path::PathBuf::from(base)
        .join("LowMonitor")
        .join("config.toml")
}

/// Dosya yoksa varsayılanlar; okunamaz / bozuksa hata (yazmaya izin verilmez).
pub fn svc_load(path: &std::path::Path) -> Result<SvcForm, String> {
    let c = match std::fs::read_to_string(path) {
        Ok(t) => lowmon_core::config::Config::parse(&t)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(e.to_string()),
    };
    let s = |v: &dyn ToString| v.to_string().into();
    Ok(SvcForm {
        interval: s(&c.sampling.interval_ms),
        topn_every: s(&c.sampling.topn_every),
        topn_count: s(&c.sampling.topn_count),
        raw_days: s(&c.retention.raw_days),
        total_days: s(&c.retention.total_days),
        uac: c.events.uac,
    })
}

/// Değişen alanlar; sınırlar `Config::parse` ile aynı (çapraz kural raw ≤ total yazmadan önce parse'ta).
pub fn svc_diff(old: &SvcForm, new: &SvcForm) -> Result<Changes, String> {
    let mut c: Changes = Vec::new();
    macro_rules! ch {
        ($field:ident, $path:expr, $lo:expr, $hi:expr) => {
            let v = int($path[1], &new.$field, $lo, $hi)?;
            if old.$field != new.$field {
                c.push(($path, Val::Int(v)));
            }
        };
    }
    ch!(interval, &["sampling", "interval_ms"], 100, 60_000);
    ch!(topn_every, &["sampling", "topn_every"], 1, 3600);
    ch!(topn_count, &["sampling", "topn_count"], 0, 50);
    ch!(raw_days, &["retention", "raw_days"], 1, 3650);
    ch!(total_days, &["retention", "total_days"], 1, 3650);
    if old.uac != new.uac {
        c.push((&["events", "uac"], Val::Bool(new.uac)));
    }
    Ok(c)
}

/// Yazıldıysa `true` (servise ReloadConfig gönderilmeli).
pub fn svc_save(path: &std::path::Path, old: &SvcForm, new: &SvcForm) -> Result<bool, String> {
    let c = svc_diff(old, new)?;
    if c.is_empty() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("config.toml: {e}"))?;
    let out = apply(&text, &c, |s| {
        lowmon_core::config::Config::parse(s).map(|_| ())
    })?;
    // Yerinde yaz (tmp + rename değil): klasörde BA'nın dosya oluşturma hakkı yok ve
    // config.toml'un korumalı DACL'ı (svc install) korunmalı. Doğrulama yazmadan önce.
    std::fs::write(path, out).map_err(|e| format!("config.toml: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SettingsForm {
        SettingsForm {
            refresh: "60".into(),
            report: "20".into(),
            font_pt: "9".into(),
            warn: "75".into(),
            crit: "90".into(),
            twarn: "80".into(),
            tcrit: "90".into(),
            dwarn: "55".into(),
            dcrit: "65".into(),
            bind: "127.0.0.1".into(),
            port: "47810".into(),
            ai_mon: 2,
            sys_mon: 3,
            ..Default::default()
        }
    }

    #[test]
    fn diff_only_changed() {
        let b = base();
        let [ai, tb, deck] = diff(&b, &b).unwrap();
        assert!(ai.is_empty() && tb.is_empty() && deck.is_empty());
        let n = SettingsForm {
            lang: 1,
            ai_mon: 0,
            sys_mon: 0,
            ..base()
        };
        let [ai, tb, deck] = diff(&b, &n).unwrap();
        assert_eq!(ai, vec![(&["language"][..], Val::Str("en".into()))]);
        assert_eq!(
            tb,
            vec![
                (&["ai", "monitor"][..], Val::Remove),
                (&["sys", "monitor"][..], Val::Str("primary".into()))
            ]
        );
        assert!(deck.is_empty());
    }

    #[test]
    fn diff_rejects_bad_values() {
        let b = base();
        let bad = |n: SettingsForm| diff(&b, &n).is_err();
        assert!(bad(SettingsForm {
            warn: "95".into(),
            ..base()
        }));
        assert!(bad(SettingsForm {
            port: "80".into(),
            ..base()
        }));
        assert!(bad(SettingsForm {
            refresh: "10".into(),
            ..base()
        }));
        assert!(bad(SettingsForm {
            font_pt: "x".into(),
            ..base()
        }));
        assert!(bad(SettingsForm {
            bind: "abc".into(),
            ..base()
        }));
    }

    #[test]
    fn svc_roundtrip_keeps_comments_and_validates() {
        let dir = std::env::temp_dir().join(format!("lowmon-ui-svc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        // Dosya yok → varsayılanlar
        let old = svc_load(&path).unwrap();
        assert_eq!(
            (old.interval.as_str(), old.raw_days.as_str()),
            ("1000", "7")
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            "# elle
[sampling]
interval_ms = 1000 # yorum
",
        )
        .unwrap();
        let old = svc_load(&path).unwrap();
        let mut new = old.clone();
        new.interval = "2000".into();
        assert!(svc_save(&path, &old, &new).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# elle") && text.contains("interval_ms = 2000 # yorum"));
        assert_eq!(svc_load(&path).unwrap().interval, "2000");
        // Değişiklik yok → yazılmaz
        assert!(!svc_save(&path, &new, &new).unwrap());
        // UAC aç/kapa: [events] tablosu yoksa oluşturulur
        let mut on = new.clone();
        on.uac = true;
        assert!(svc_save(&path, &new, &on).unwrap());
        assert!(svc_load(&path).unwrap().uac);
        on.uac = false;
        assert!(
            svc_save(
                &path,
                &{
                    let mut o = on.clone();
                    o.uac = true;
                    o
                },
                &on
            )
            .unwrap()
        );
        let text = std::fs::read_to_string(&path).unwrap();
        // Alan sınırı
        let mut bad = new.clone();
        bad.topn_count = "51".into();
        assert!(svc_save(&path, &new, &bad).is_err());
        // Çapraz kural (raw ≤ total) parse'ta; dosya değişmez
        let mut bad = new.clone();
        bad.raw_days = "40".into();
        assert!(svc_save(&path, &new, &bad).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        // Bozuk dosya → okuma hatası
        std::fs::write(
            &path,
            "[sampling]
interval_ms = 5
",
        )
        .unwrap();
        assert!(svc_load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
