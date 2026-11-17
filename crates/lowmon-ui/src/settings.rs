//! Ayarlar sekmesi: kullanıcı ayar dosyaları (`ai.toml`, `taskbar.toml`, `deck.toml`).
//! Yalnız değişen anahtar yazılır (`toml_edit`: yorum + sıra korunur); sonuç dosyanın kendi
//! ayrıştırıcısıyla doğrulanmadan diske gitmez. Servis `config.toml` (ProgramData) kullanıcıya kapalı.
use toml_edit::{DocumentMut, Item, Table, value};

/// Yazılacak değer (form alanları yalnız bunlar).
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Anahtarı sil (varsayılana dön; ör. blok monitörü "ortak")
    Remove,
}

/// `path` = tablo yolu + anahtar (`["sys", "warn_pct"]`); eksik tablolar oluşturulur.
pub fn set(text: &str, path: &[&str], v: &Val) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let (key, tables) = path.split_last().ok_or("boş yol")?;
    let mut t: &mut Table = doc.as_table_mut();
    for name in tables {
        let item = t.entry(name).or_insert_with(|| Item::Table(Table::new()));
        t = item
            .as_table_mut()
            .ok_or_else(|| format!("{name} tablo değil"))?;
    }
    // Var olan değerin süsü (satır sonu yorumu) korunur
    let decor = t
        .get(key)
        .and_then(Item::as_value)
        .map(|v| v.decor().clone());
    let mut new = match v {
        Val::Remove => {
            t.remove(key);
            return Ok(doc.to_string());
        }
        Val::Bool(b) => value(*b),
        Val::Int(i) => value(*i),
        Val::Float(f) => value(*f),
        Val::Str(s) => value(s.as_str()),
    };
    if let (Some(d), Some(nv)) = (decor, new.as_value_mut()) {
        *nv.decor_mut() = d;
    }
    t.insert(key, new);
    Ok(doc.to_string())
}

/// Değişiklikleri sırayla uygular, sonucu `check` ile (dosyanın kendi ayrıştırıcısı) doğrular.
pub fn apply(
    text: &str,
    changes: &[(&[&str], Val)],
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    let mut out = text.to_owned();
    for (p, v) in changes {
        out = set(&out, p, v)?;
    }
    check(&out)?;
    Ok(out)
}

/// Dosyayı değişikliklerle yazar (yoksa oluşturur); geçici dosya + rename.
pub fn write(
    path: &std::path::Path,
    changes: &[(&[&str], Val)],
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let out = apply(&text, changes, check)?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

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
    fn remove_key() {
        let out = set(
            "[ai]
monitor = \"left\" # yorum
bubble = true
",
            &["ai", "monitor"],
            &Val::Remove,
        )
        .unwrap();
        assert_eq!(
            out,
            "[ai]
bubble = true
"
        );
    }

    #[test]
    fn keeps_comments_and_order() {
        let src =
            "# baş yorum\nbind = \"192.168.1.5\"\nport = 47810 # port yorumu\ntoken = \"x\"\n";
        let out = set(src, &["port"], &Val::Int(47820)).unwrap();
        assert_eq!(
            out,
            "# baş yorum\nbind = \"192.168.1.5\"\nport = 47820 # port yorumu\ntoken = \"x\"\n"
        );
    }

    #[test]
    fn creates_tables() {
        let out = set("", &["sys", "cpu_temp"], &Val::Bool(false)).unwrap();
        assert_eq!(out, "[sys]\ncpu_temp = false\n");
        let out = set(&out, &["ai", "bubble"], &Val::Bool(true)).unwrap();
        let out = set(&out, &["font_pt"], &Val::Float(9.5)).unwrap();
        let v: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(v["sys"]["cpu_temp"].as_bool(), Some(false));
        assert_eq!(v["ai"]["bubble"].as_bool(), Some(true));
        assert_eq!(v["font_pt"].as_float(), Some(9.5));
    }

    #[test]
    fn check_blocks_bad_result() {
        let ok = |_: &str| Ok(());
        let deny = |_: &str| Err("geçersiz".to_string());
        assert!(apply("a = 1", &[(&["a"], Val::Int(2))], ok).is_ok());
        assert_eq!(
            apply("a = 1", &[(&["a"], Val::Int(2))], deny),
            Err("geçersiz".into())
        );
        assert!(set("{ bozuk", &["a"], &Val::Int(1)).is_err());
        assert!(set("x = 1", &["x", "y"], &Val::Int(1)).is_err());
    }
}
