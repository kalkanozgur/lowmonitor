//! LowDeck ADR-0010: telefondan PC `ai.toml` (dil, aralık, kimlik, CSV, rapor oturumu).

use crate::tomlw::{Val, write};
use lowdeck_proto::{AI_TEXT_MAX, AiConfig};
use lowmon_ai::{Auth, text::Lang};

fn to_proto(c: &lowmon_ai::Config) -> AiConfig {
    AiConfig {
        lang: (c.language == Lang::En) as u8,
        refresh_sec: c.refresh_sec.min(u32::MAX as u64) as u32,
        auth: match c.auth {
            Auth::Auto => 0,
            Auth::OAuth => 1,
            Auth::Session => 2,
        },
        csv_log: c.csv_log,
        report_sessions: c.report_sessions.min(u16::MAX as usize) as u16,
    }
}

fn cut(e: String) -> String {
    e.chars().take(AI_TEXT_MAX).collect()
}

/// Dosyadaki ayar; bozuksa varsayılan + hata.
pub fn read(path: &std::path::Path) -> (AiConfig, String) {
    match lowmon_ai::Config::load(path) {
        Ok(c) => (to_proto(&c), String::new()),
        Err(e) => (to_proto(&Default::default()), cut(e)),
    }
}

/// Denetler, yalnız değişen anahtarları yazar. Dönen: yazıldı mı (tray yeniden yüklensin), hata.
pub fn save(path: &std::path::Path, new: &AiConfig) -> Result<bool, String> {
    new.check()?;
    let old = to_proto(&lowmon_ai::Config::load(path)?);
    let mut ch: Vec<(&[&str], Val)> = Vec::new();
    if old.lang != new.lang {
        ch.push((
            &["language"],
            Val::Str(["tr", "en"][new.lang as usize].into()),
        ));
    }
    if old.refresh_sec != new.refresh_sec {
        ch.push((&["refresh_sec"], Val::Int(new.refresh_sec as i64)));
    }
    if old.auth != new.auth {
        ch.push((
            &["auth"],
            Val::Str(["auto", "oauth", "session"][new.auth as usize].into()),
        ));
    }
    if old.csv_log != new.csv_log {
        ch.push((&["csv_log"], Val::Bool(new.csv_log)));
    }
    if old.report_sessions != new.report_sessions {
        ch.push((&["report_sessions"], Val::Int(new.report_sessions as i64)));
    }
    write(path, &ch, |s| lowmon_ai::Config::parse(s).map(|_| ()))?;
    Ok(!ch.is_empty())
}

/// `SetAiConfig` yanıtı: kaydet, gerekirse `reload`, dosyadan yeniden oku.
pub fn set(path: &std::path::Path, new: &AiConfig, reload: impl FnOnce()) -> (AiConfig, String) {
    match save(path, new) {
        Ok(changed) => {
            if changed {
                reload();
            }
            read(path)
        }
        Err(e) => (read(path).0, cut(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_only_changed_keeps_comments() {
        let dir = std::env::temp_dir().join(format!("lowdeck-aicfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("ai.toml");
        std::fs::write(
            &p,
            "# yorum\nrefresh_sec = 180 # aralık\nlanguage = \"tr\"\n",
        )
        .unwrap();
        let (mut c, e) = read(&p);
        assert_eq!((c.refresh_sec, c.lang, e.as_str()), (180, 0, ""));
        c.refresh_sec = 60;
        c.auth = 1;
        let mut reloaded = false;
        let (got, e) = set(&p, &c, || reloaded = true);
        assert_eq!((got, e.as_str(), reloaded), (c.clone(), "", true));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("refresh_sec = 60 # aralık") && text.contains("# yorum"),
            "{text}"
        );
        // Aynı değer → yazma ve reload yok
        let (_, _) = set(&p, &c, || panic!("değişiklik yokken reload"));
        // Geçersiz → dosya değişmez, hata döner
        let (got, e) = set(
            &p,
            &AiConfig {
                refresh_sec: 5,
                ..c.clone()
            },
            || panic!(),
        );
        assert_eq!(got.refresh_sec, 60);
        assert!(e.contains("refresh_sec"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
