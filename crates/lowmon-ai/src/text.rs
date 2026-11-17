//! Kullanıcıya görünen metin: en/tr (ADR-0032, `ai.toml` `language`).
//! Aynı ayar tray menüsünün ve `lowmon-ui`'ın dilini de seçer: süreç açılışta `set_lang` çağırır,
//! metinler yerinde `tr("Türkçe", "English")` çiftiyle yazılır.
use crate::api::{Snapshot, Window};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Tr,
    En,
}

/// Sürecin arayüz dili (0 = tr, 1 = en); `set_lang` çağrılmadıysa Türkçe.
static LANG: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_lang(l: Lang) {
    LANG.store((l == Lang::En) as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(std::sync::atomic::Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Tr
    }
}

/// Sürecin diline göre iki metinden biri.
pub fn tr(tr: &'static str, en: &'static str) -> &'static str {
    pick(lang(), tr, en)
}

pub fn pick<'a>(lang: Lang, tr: &'a str, en: &'a str) -> &'a str {
    if lang == Lang::En { en } else { tr }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    Session,
    Week,
    OAuthNoLogin,
    OAuthCredRead,
    OAuthExpired,
    OAuthDenied,
    Unexpected,
    Unreachable,
    RefreshPrefix,
    WebNoKey,
    WebNoOrg,
    CookieOrCloudflare,
    NoCredentials,
}

pub fn t(lang: Lang, m: Msg) -> &'static str {
    use Msg::*;
    let (en, tr) = match m {
        Session => ("Session", "Oturum"),
        Week => ("Week", "Hafta"),
        OAuthNoLogin => (
            "OAuth: not signed in to Claude Code",
            "OAuth: Claude Code girişi yok",
        ),
        OAuthCredRead => (
            "OAuth: cannot read credentials file",
            "OAuth: kimlik dosyası okunamadı",
        ),
        OAuthExpired => ("OAuth: token expired", "OAuth: token süresi dolmuş"),
        OAuthDenied => ("OAuth: access denied", "OAuth: yetki reddedildi"),
        Unexpected => ("unexpected response", "beklenmeyen yanıt"),
        Unreachable => ("unreachable", "erişilemedi"),
        RefreshPrefix => ("OAuth refresh: ", "OAuth yenileme: "),
        WebNoKey => ("Web: no sessionKey saved", "Web: sessionKey kayıtlı değil"),
        WebNoOrg => (
            "Web: organization not found",
            "Web: organizasyon bulunamadı",
        ),
        CookieOrCloudflare => (
            " (invalid cookie or Cloudflare)",
            " (çerez geçersiz veya Cloudflare)",
        ),
        NoCredentials => ("no credentials", "kimlik bilgisi yok"),
    };
    if lang == Lang::En { en } else { tr }
}

/// "23%" / "%23"
pub fn pct(lang: Lang, v: f64) -> String {
    let v = v.round() as i64;
    if lang == Lang::En {
        format!("{v}%")
    } else {
        format!("%{v}")
    }
}

/// Geri sayım: "2d05h" / "2g05s", "3h07m" / "3s07dk", "12m" / "12dk". Negatif → 0.
pub fn remaining(lang: Lang, secs: i64) -> String {
    let s = secs.max(0);
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    let (ud, uh, um) = if lang == Lang::En {
        ("d", "h", "m")
    } else {
        ("g", "s", "dk")
    };
    if d > 0 {
        format!("{d}{ud}{h:02}{uh}")
    } else if h > 0 {
        format!("{h}{uh}{m:02}{um}")
    } else {
        format!("{m}{um}")
    }
}

/// Taskbar satırları: "Oturum %23 · 2s14dk" / "Hafta %41 · 3g05s".
pub fn lines(s: &Snapshot, now: i64, lang: Lang) -> [String; 2] {
    let one = |label, w: &Option<Window>| match w {
        None => format!("{} —", t(lang, label)),
        Some(w) => format!(
            "{} {} · {}",
            t(lang, label),
            pct(lang, w.utilization),
            w.resets_at.map_or("—".into(), |r| remaining(lang, r - now))
        ),
    };
    [
        one(Msg::Session, &s.five_hour),
        one(Msg::Week, &s.seven_day),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let now = 1_000_000;
        let s = Snapshot {
            five_hour: Some(Window {
                utilization: 23.4,
                resets_at: Some(now + 2 * 3600 + 14 * 60),
            }),
            seven_day: Some(Window {
                utilization: 81.0,
                resets_at: Some(now + 3 * 86_400 + 5 * 3600),
            }),
            ..Default::default()
        };
        assert_eq!(
            lines(&s, now, Lang::En),
            ["Session 23% · 2h14m", "Week 81% · 3d05h"]
        );
        assert_eq!(
            lines(&s, now, Lang::Tr),
            ["Oturum %23 · 2s14dk", "Hafta %81 · 3g05s"]
        );
        let s = Snapshot {
            seven_day: Some(Window {
                utilization: 10.0,
                resets_at: None,
            }),
            ..Default::default()
        };
        assert_eq!(lines(&s, now, Lang::En), ["Session —", "Week 10% · —"]);
        assert_eq!(remaining(Lang::Tr, -5), "0dk");
        assert_eq!(remaining(Lang::En, 12 * 60 + 59), "12m");
    }
}
