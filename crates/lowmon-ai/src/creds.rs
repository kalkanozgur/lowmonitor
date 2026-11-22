//! Claude Code OAuth kimliği (`~/.claude/.credentials.json`):
//! `{"claudeAiOauth":{"accessToken":"…","refreshToken":"…","expiresAt":1767000000000,…}}`.
//! Dosya Claude Code'un; geri yazarken diğer her bayt aynen kalmalı (`patch`).
use crate::json::{self, Span, num_member, str_member};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OAuth {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix ms; 0 = bilinmiyor
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refresh {
    pub access_token: String,
    /// yok → eskisi kalır
    pub refresh_token: Option<String>,
    pub expires_in_s: i64,
}

fn oauth_object(doc: &[u8]) -> Option<Span> {
    let root = json::root(doc).filter(|r| json::is_object(doc, *r))?;
    json::member(doc, root, "claudeAiOauth").filter(|o| json::is_object(doc, *o))
}

pub fn parse(doc: &[u8]) -> Option<OAuth> {
    let o = oauth_object(doc)?;
    let c = OAuth {
        access_token: str_member(doc, o, "accessToken").unwrap_or_default(),
        refresh_token: str_member(doc, o, "refreshToken").unwrap_or_default(),
        expires_at_ms: num_member(doc, o, "expiresAt") as i64,
    };
    (!c.access_token.is_empty()).then_some(c)
}

/// `doc`'un accessToken / refreshToken / expiresAt değiştirilmiş kopyası; biri yoksa `None`.
pub fn patch(doc: &[u8], c: &OAuth) -> Option<Vec<u8>> {
    let o = oauth_object(doc)?;
    let mut edits = [
        (
            json::member(doc, o, "accessToken")?,
            json::quote(&c.access_token),
        ),
        (
            json::member(doc, o, "refreshToken")?,
            json::quote(&c.refresh_token),
        ),
        (
            json::member(doc, o, "expiresAt")?,
            c.expires_at_ms.to_string(),
        ),
    ];
    // Sondan başa: önceki ofsetler geçerli kalsın.
    edits.sort_by_key(|(s, _)| std::cmp::Reverse(s.begin));
    let mut out = doc.to_vec();
    for (s, text) in edits {
        out.splice(s.begin..s.end, text.into_bytes());
    }
    Some(out)
}

/// Token `skew_ms` içinde bitiyorsa (ya da bitiş bilinmiyorsa) true.
pub fn needs_refresh(c: &OAuth, now_ms: i64, skew_ms: i64) -> bool {
    c.expires_at_ms <= 0 || c.expires_at_ms - skew_ms <= now_ms
}

pub fn refresh_request(refresh_token: &str, client_id: &str) -> String {
    format!(
        r#"{{"grant_type":"refresh_token","refresh_token":{},"client_id":{}}}"#,
        json::quote(refresh_token),
        json::quote(client_id)
    )
}

pub fn parse_refresh(doc: &[u8]) -> Option<Refresh> {
    let root = json::root(doc).filter(|r| json::is_object(doc, *r))?;
    let access_token = str_member(doc, root, "access_token").unwrap_or_default();
    let expires_in_s = num_member(doc, root, "expires_in") as i64;
    if access_token.is_empty() || expires_in_s <= 0 {
        return None;
    }
    let refresh_token = str_member(doc, root, "refresh_token").filter(|s| !s.is_empty());
    Some(Refresh {
        access_token,
        refresh_token,
        expires_in_s,
    })
}

pub fn apply_refresh(old: &OAuth, r: &Refresh, now_ms: i64) -> OAuth {
    OAuth {
        access_token: r.access_token.clone(),
        refresh_token: r
            .refresh_token
            .clone()
            .unwrap_or_else(|| old.refresh_token.clone()),
        expires_at_ms: now_ms + r.expires_in_s * 1000,
    }
}

/// Panodan gelen claude.ai `sessionKey` çerezi: boşluk kırpılır, `sessionKey=` öneki atılır;
/// `sk-ant-` ile başlamıyorsa None (C++ widget ile aynı kural).
pub fn parse_session_key(text: &str) -> Option<&str> {
    let t = text.trim();
    let t = t.strip_prefix("sessionKey=").unwrap_or(t).trim();
    (t.starts_with("sk-ant-") && !t.contains(char::is_whitespace)).then_some(t)
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_key_from_clipboard() {
        assert_eq!(
            parse_session_key(
                "  sk-ant-sid01-abc
"
            ),
            Some("sk-ant-sid01-abc")
        );
        assert_eq!(parse_session_key("sessionKey=sk-ant-x"), Some("sk-ant-x"));
        assert_eq!(parse_session_key("merhaba"), None);
        assert_eq!(parse_session_key("sk-ant-a b"), None);
        assert_eq!(parse_session_key(""), None);
    }

    use super::*;

    const FILE: &str = "{\n  \"claudeAiOauth\": {\n    \"accessToken\": \"sk-ant-oat01-OLD\",\n    \"refreshToken\": \"sk-ant-ort01-OLD\",\n    \"expiresAt\": 1767000000000,\n    \"scopes\": [\"user:inference\", \"user:profile\"],\n    \"subscriptionType\": \"max\"\n  },\n  \"other\": {\"accessToken\": \"must-not-change\"}\n}\n";

    fn oauth(a: &str, r: &str, e: i64) -> OAuth {
        OAuth {
            access_token: a.into(),
            refresh_token: r.into(),
            expires_at_ms: e,
        }
    }

    #[test]
    fn parses() {
        assert_eq!(
            parse(FILE.as_bytes()),
            Some(oauth(
                "sk-ant-oat01-OLD",
                "sk-ant-ort01-OLD",
                1_767_000_000_000
            ))
        );
        assert!(parse(br#"{"claudeAiOauth":{}}"#).is_none());
        assert!(parse(br#"{"somethingElse":1}"#).is_none());
        assert!(parse(b"not json").is_none());
    }

    #[test]
    fn patch_replaces_only_three_values() {
        let fresh = oauth("sk-ant-oat01-NEW", "sk-ant-ort01-NEW", 1_767_999_999_999);
        let out = String::from_utf8(patch(FILE.as_bytes(), &fresh).unwrap()).unwrap();
        let expected = FILE
            .replace("sk-ant-oat01-OLD", "sk-ant-oat01-NEW")
            .replace("sk-ant-ort01-OLD", "sk-ant-ort01-NEW")
            .replace("1767000000000", "1767999999999");
        assert_eq!(out, expected);
        assert_eq!(parse(out.as_bytes()), Some(fresh));
        assert!(
            patch(
                br#"{"claudeAiOauth":{"accessToken":"a"}}"#,
                &oauth("b", "c", 1)
            )
            .is_none()
        );
    }

    #[test]
    fn needs_refresh_skew() {
        let mut c = oauth("a", "r", 10_000_000);
        assert!(!needs_refresh(&c, 1_000_000, 300_000));
        assert!(needs_refresh(&c, 9_800_000, 300_000));
        c.expires_at_ms = 0;
        assert!(needs_refresh(&c, 0, 0));
    }

    #[test]
    fn refresh_roundtrip() {
        assert_eq!(
            refresh_request("r\"1", "client"),
            r#"{"grant_type":"refresh_token","refresh_token":"r\"1","client_id":"client"}"#
        );
        let r = parse_refresh(
            br#"{"token_type":"Bearer","access_token":"A2","refresh_token":"R2","expires_in":28800}"#,
        )
        .unwrap();
        assert_eq!(
            apply_refresh(&oauth("A1", "R1", 0), &r, 1_000),
            oauth("A2", "R2", 1_000 + 28_800_000)
        );
        let r = parse_refresh(br#"{"access_token":"A2","expires_in":60}"#).unwrap();
        assert_eq!(
            apply_refresh(&oauth("A1", "R1", 0), &r, 0).refresh_token,
            "R1"
        );
        assert!(parse_refresh(br#"{"error":"invalid_grant"}"#).is_none());
    }
}
