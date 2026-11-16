//! Kullanım uçları: OAuth `GET api.anthropic.com/api/oauth/usage`,
//! Web `GET claude.ai/api/organizations/{org}/usage`. İkisi aynı şekli döner:
//! `{"five_hour":{"utilization":23.0,"resets_at":"…"},"seven_day":{…},…}`.
use crate::json::{self, Span};
use lowmon_core::time::days_from_civil;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Window {
    /// 0..100
    pub utilization: f64,
    /// Unix saniye
    pub resets_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub seven_day_sonnet: Option<Window>,
    pub seven_day_opus: Option<Window>,
}

fn window(doc: &[u8], root: Span, key: &str) -> Option<Window> {
    let v = json::member(doc, root, key).filter(|v| json::is_object(doc, *v))?; // yok ya da null: plan bu pencereyi içermiyor
    Some(Window {
        utilization: json::member(doc, v, "utilization")
            .and_then(|u| json::as_number(doc, u))
            .unwrap_or(0.0),
        resets_at: json::member(doc, v, "resets_at")
            .and_then(|r| json::as_string(doc, r))
            .and_then(|s| parse_iso8601(&s)),
    })
}

/// `five_hour` ya da `seven_day` yoksa `None` (ör. hata yükü, Cloudflare sayfası).
pub fn parse_usage(doc: &[u8]) -> Option<Snapshot> {
    let root = json::root(doc).filter(|r| json::is_object(doc, *r))?;
    if json::member(doc, root, "five_hour").is_none()
        && json::member(doc, root, "seven_day").is_none()
    {
        return None;
    }
    Some(Snapshot {
        five_hour: window(doc, root, "five_hour"),
        seven_day: window(doc, root, "seven_day"),
        seven_day_sonnet: window(doc, root, "seven_day_sonnet"),
        seven_day_opus: window(doc, root, "seven_day_opus"),
    })
}

/// `GET claude.ai/api/organizations`: `capabilities`'inde "chat" olan ilk org, yoksa ilk uuid.
pub fn pick_chat_org(doc: &[u8]) -> Option<String> {
    let root = json::root(doc).filter(|r| json::is_array(doc, *r))?;
    let mut first = None;
    for org in json::elements(doc, root) {
        let Some(uuid) = json::member(doc, org, "uuid")
            .and_then(|u| json::as_string(doc, u))
            .filter(|u| !u.is_empty())
        else {
            continue;
        };
        let chat = json::member(doc, org, "capabilities").is_some_and(|caps| {
            json::elements(doc, caps).any(|c| json::as_string(doc, c).as_deref() == Some("chat"))
        });
        if chat {
            return Some(uuid);
        }
        first.get_or_insert(uuid);
    }
    first
}

fn digits(s: &[u8], pos: usize, n: usize) -> Option<i64> {
    let d = s.get(pos..pos + n)?;
    d.iter().try_fold(0i64, |acc, &c| {
        c.is_ascii_digit().then(|| acc * 10 + i64::from(c - b'0'))
    })
}

/// `YYYY-MM-DDTHH:MM:SS[.kesir][Z|±HH:MM|±HHMM]` → Unix saniye (UTC). Bölge yoksa UTC.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    let (y, mo, d) = (digits(b, 0, 4)?, digits(b, 5, 2)?, digits(b, 8, 2)?);
    let (h, mi, se) = (digits(b, 11, 2)?, digits(b, 14, 2)?, digits(b, 17, 2)?);
    if !at(4, b'-')
        || !at(7, b'-')
        || !(at(10, b'T') || at(10, b' '))
        || !at(13, b':')
        || !at(16, b':')
    {
        return None;
    }
    if h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut pos = 19;
    if at(pos, b'.') {
        pos += 1;
        while b.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
    }
    let mut offset = 0;
    match b.get(pos) {
        None => {}
        Some(b'Z' | b'z') => pos += 1,
        Some(&z @ (b'+' | b'-')) => {
            let oh = digits(b, pos + 1, 2)?;
            let mut mpos = pos + 3;
            if at(mpos, b':') {
                mpos += 1;
            }
            let om = digits(b, mpos, 2)?;
            offset = (oh * 3600 + om * 60) * if z == b'+' { 1 } else { -1 };
            pos = mpos + 2;
        }
        Some(_) => return None,
    }
    if pos != b.len() {
        return None;
    }
    let days = days_from_civil(y as u64, mo as u64, d as u64)? as i64;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_oauth_usage() {
        let s = parse_usage(
            br#"{
            "five_hour": {"utilization": 35.0, "resets_at": "2026-02-06T22:00:00+00:00"},
            "seven_day": {"utilization": 14.0, "resets_at": "2026-02-12T20:00:00+00:00"},
            "seven_day_sonnet": {"utilization": 39.0, "resets_at": "2026-02-09T14:00:00+00:00"},
            "seven_day_opus": null,
            "extra_usage": {"is_enabled": true, "utilization": null}
        }"#,
        )
        .unwrap();
        let f = s.five_hour.unwrap();
        assert_eq!(f.utilization, 35.0);
        assert_eq!(f.resets_at, Some(1_770_415_200));
        assert_eq!(s.seven_day.unwrap().utilization, 14.0);
        assert!(s.seven_day_sonnet.is_some());
        assert!(s.seven_day_opus.is_none());
    }

    #[test]
    fn null_utilization_is_zero() {
        let s = parse_usage(br#"{"five_hour":{"utilization":null,"resets_at":null}}"#).unwrap();
        assert_eq!(
            s.five_hour,
            Some(Window {
                utilization: 0.0,
                resets_at: None
            })
        );
        assert!(s.seven_day.is_none());
    }

    #[test]
    fn rejects_error_payloads() {
        assert!(
            parse_usage(br#"{"type":"error","error":{"type":"authentication_error"}}"#).is_none()
        );
        assert!(parse_usage(b"<html>Just a moment...</html>").is_none());
        assert!(parse_usage(b"").is_none());
    }

    #[test]
    fn picks_chat_org() {
        let doc = br#"[{"uuid":"api-org","capabilities":["api"]},{"uuid":"chat-org","capabilities":["api","chat"]}]"#;
        assert_eq!(pick_chat_org(doc).as_deref(), Some("chat-org"));
        assert_eq!(
            pick_chat_org(br#"[{"uuid":"only"}]"#).as_deref(),
            Some("only")
        );
        assert!(pick_chat_org(b"[]").is_none());
        assert!(pick_chat_org(br#"{"uuid":"x"}"#).is_none());
    }

    #[test]
    fn iso8601() {
        assert_eq!(parse_iso8601("2026-02-06T22:00:00Z"), Some(1_770_415_200));
        assert_eq!(
            parse_iso8601("2026-02-06T22:00:00.123456+00:00"),
            Some(1_770_415_200)
        );
        assert_eq!(
            parse_iso8601("2026-02-07T01:00:00+0300"),
            Some(1_770_415_200)
        );
        assert_eq!(
            parse_iso8601("2026-02-06 19:00:00-03:00"),
            Some(1_770_415_200)
        );
        assert_eq!(parse_iso8601("2026-02-06T22:00:00"), Some(1_770_415_200));
        assert_eq!(parse_iso8601("2026-13-06T22:00:00Z"), None);
        assert_eq!(parse_iso8601("2026-02-06T22:00:00Zx"), None);
        assert_eq!(parse_iso8601("garbage"), None);
    }
}
