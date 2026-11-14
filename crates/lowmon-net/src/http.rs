//! Minimal HTTP/1.1: istek başı ayrıştırma + kısa yanıtlar. Yalnız `/v1/live` (WS) ve `/v1/pair` (POST).
use serde::Deserialize;

/// İstek başı + gövde sınırı; aşan bağlantı kapanır.
pub const MAX_REQ: usize = 8 * 1024;
const MAX_BODY: usize = 256;

#[derive(Debug, PartialEq)]
pub struct Req {
    pub method: String,
    pub path: String,
    /// `Authorization: Bearer …` ya da `?token=…` (tarayıcı WS header'ı gönderemez).
    pub token: Option<String>,
    pub origin: Option<String>,
    /// Geçerli WS yükseltme isteğinde `Sec-WebSocket-Key`.
    pub ws_key: Option<String>,
    pub head_len: usize,
    pub body_len: usize,
}

#[derive(Debug, PartialEq)]
pub enum Parse {
    Partial,
    Bad,
    Done(Req),
}

pub fn parse(buf: &[u8]) -> Parse {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    let head_len = match req.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Parse::Partial,
        Err(_) => return Parse::Bad,
    };
    let get = |name: &str| {
        req.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .and_then(|h| std::str::from_utf8(h.value).ok())
            .map(str::trim)
    };
    let has = |name: &str, needle: &str| {
        get(name).is_some_and(|v| v.split(',').any(|p| p.trim().eq_ignore_ascii_case(needle)))
    };
    let Ok(body_len) = get("content-length").map_or(Ok(0), str::parse::<usize>) else {
        return Parse::Bad;
    };
    if body_len > MAX_BODY {
        return Parse::Bad;
    }
    let target = req.path.unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let token = get("authorization")
        .and_then(|a| a.strip_prefix("Bearer "))
        .or_else(|| query.split('&').find_map(|kv| kv.strip_prefix("token=")))
        .map(str::to_owned);
    let ws_key = (has("upgrade", "websocket")
        && has("connection", "upgrade")
        && get("sec-websocket-version") == Some("13"))
    .then(|| get("sec-websocket-key").map(str::to_owned))
    .flatten();
    Parse::Done(Req {
        method: req.method.unwrap_or("").to_owned(),
        path: path.to_owned(),
        token,
        origin: get("origin").map(str::to_owned),
        ws_key,
        head_len,
        body_len,
    })
}

pub fn reply(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

pub fn upgrade(key: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        tungstenite::handshake::derive_accept_key(key.as_bytes())
    )
    .into_bytes()
}

/// `POST /v1/pair` gövdesi: `{"code": "123456"}`.
pub fn pair_code(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Pair {
        code: String,
    }
    serde_json::from_slice::<Pair>(body).ok().map(|p| p.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ws_and_pair_requests() {
        let ws = b"GET /v1/live?x=1&token=abc HTTP/1.1\r\nHost: h\r\nUpgrade: websocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: k==\r\nOrigin: http://p\r\n\r\n";
        let Parse::Done(r) = parse(ws) else {
            panic!("ws")
        };
        assert_eq!(
            (r.path.as_str(), r.token.as_deref(), r.ws_key.as_deref()),
            ("/v1/live", Some("abc"), Some("k=="))
        );
        assert_eq!(r.origin.as_deref(), Some("http://p"));
        assert_eq!(parse(&ws[..20]), Parse::Partial);

        let bearer = b"GET /v1/live HTTP/1.1\r\nAuthorization: Bearer t1\r\n\r\n";
        let Parse::Done(r) = parse(bearer) else {
            panic!("bearer")
        };
        assert_eq!(
            (r.token.as_deref(), r.ws_key),
            (Some("t1"), None),
            "upgrade yok"
        );

        let post = b"POST /v1/pair HTTP/1.1\r\nContent-Length: 16\r\n\r\n{\"code\":\"12345\"}";
        let Parse::Done(r) = parse(post) else {
            panic!("post")
        };
        assert_eq!(r.body_len, 16);
        assert_eq!(pair_code(&post[r.head_len..]).as_deref(), Some("12345"));
        assert_eq!(pair_code(b"{}"), None);

        assert_eq!(parse(b"\x00\x01 garbage\r\n\r\n"), Parse::Bad);
        assert_eq!(
            parse(b"POST / HTTP/1.1\r\nContent-Length: 999999\r\n\r\n"),
            Parse::Bad
        );
    }
}
