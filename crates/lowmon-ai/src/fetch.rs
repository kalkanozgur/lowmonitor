//! Kullanım sorgusu: önce Claude Code OAuth (gerekirse yenileme), yedek claude.ai sessionKey.
use crate::api::{self, Snapshot};
use crate::creds;
use crate::text::{Msg, t};
use crate::win::{self, Http};
use crate::{Auth, Config, claude_dir};
use std::path::PathBuf;

/// Claude Code'un genel OAuth istemcisi; yenilediğimiz token ona verildi.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_HOSTS: [&str; 2] = ["platform.claude.com", "console.anthropic.com"];
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    OAuth,
    Web,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

pub struct Fetcher {
    cfg: Config,
    session_file: PathBuf,
    http: Http,
    web: Option<Http>,
    org_id: String,
    log: fn(&str),
}

fn wide_header(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Fetcher {
    pub fn new(cfg: Config, session_file: PathBuf, log: fn(&str)) -> Self {
        let ua = format!("LowMonitor/{}", env!("CARGO_PKG_VERSION"));
        Self {
            cfg,
            session_file,
            http: Http::new(&ua),
            web: None,
            org_id: String::new(),
            log,
        }
    }

    fn l(&self, m: Msg) -> &'static str {
        t(self.cfg.language, m)
    }

    pub fn fetch(&mut self) -> Result<(Snapshot, Source), String> {
        let (mut oauth_err, mut web_err) = (String::new(), String::new());
        if self.cfg.auth != Auth::Session {
            match self.fetch_oauth() {
                Ok(s) => return Ok((s, Source::OAuth)),
                Err(e) => oauth_err = e,
            }
        }
        if self.cfg.auth != Auth::OAuth {
            match self.fetch_session() {
                Ok(s) => return Ok((s, Source::Web)),
                Err(e) => web_err = e,
            }
        }
        let err = match (oauth_err.is_empty(), web_err.is_empty()) {
            (true, true) => self.l(Msg::NoCredentials).to_string(),
            (false, true) => oauth_err,
            (true, false) => web_err,
            (false, false) => format!("{oauth_err} | {web_err}"),
        };
        Err(err)
    }

    fn fetch_oauth(&mut self) -> Result<Snapshot, String> {
        let path = claude_dir().join(".credentials.json");
        let Ok(mut text) = std::fs::read(&path) else {
            return Err(self.l(Msg::OAuthNoLogin).into());
        };
        let parsed = creds::parse(&text);
        win::wipe(&mut text);
        let mut c = parsed.ok_or_else(|| self.l(Msg::OAuthCredRead).to_string())?;
        let mut refreshed = false;
        // Çalışan CLI'nin altında refresh token döndürmek onu oturumdan düşürür; CLI zaten kendisi yeniler.
        let may_refresh = |c: &creds::OAuth, refreshed: bool, hint: &str, on: bool| {
            on && !c.refresh_token.is_empty() && !refreshed && !win::running(hint).cli
        };
        let (hint, on) = (self.cfg.desktop_path_hint.clone(), self.cfg.oauth_refresh);
        if creds::needs_refresh(&c, now_ms(), REFRESH_SKEW_MS) {
            if !may_refresh(&c, refreshed, &hint, on) {
                return Err(self.l(Msg::OAuthExpired).into());
            }
            c = self.refresh(&c)?;
            refreshed = true;
        }
        for _ in 0..2 {
            let mut h = wide_header(&format!(
                "Authorization: Bearer {}\r\nanthropic-beta: oauth-2025-04-20\r\nAccept: application/json",
                c.access_token
            ));
            let resp = self
                .http
                .request("GET", "api.anthropic.com", "/api/oauth/usage", &h, &[]);
            win::wipe(&mut h);
            let resp = resp.map_err(|e| format!("OAuth: {e}"))?;
            match resp.status {
                200 => {
                    return api::parse_usage(&resp.body)
                        .ok_or_else(|| format!("OAuth: {}", self.l(Msg::Unexpected)));
                }
                401 if may_refresh(&c, refreshed, &hint, on) => {
                    c = self.refresh(&c)?;
                    refreshed = true;
                }
                s => return Err(format!("OAuth: HTTP {s}")),
            }
        }
        Err(self.l(Msg::OAuthDenied).into())
    }

    fn refresh(&self, current: &creds::OAuth) -> Result<creds::OAuth, String> {
        let mut body = creds::refresh_request(&current.refresh_token, CLIENT_ID).into_bytes();
        let headers = wide_header("Content-Type: application/json\r\nAccept: application/json");
        let mut last = self.l(Msg::Unreachable).to_string();
        let prefix = self.l(Msg::RefreshPrefix);
        for host in TOKEN_HOSTS {
            let resp = match self
                .http
                .request("POST", host, "/v1/oauth/token", &headers, &body)
            {
                Ok(r) => r,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            if resp.status == 404 || resp.status >= 500 {
                last = format!("HTTP {}", resp.status);
                continue; // eski host'u dene
            }
            win::wipe(&mut body);
            if resp.status != 200 {
                // 400 invalid_grant: refresh token iptal → kullanıcı `claude` ile yeniden giriş yapmalı
                return Err(format!("{prefix}HTTP {}", resp.status));
            }
            let r = creds::parse_refresh(&resp.body)
                .ok_or_else(|| format!("{prefix}{}", self.l(Msg::Unexpected)))?;
            let fresh = creds::apply_refresh(current, &r, now_ms());
            self.persist(current, &fresh);
            return Ok(fresh);
        }
        win::wipe(&mut body);
        Err(format!("{prefix}{last}"))
    }

    /// Claude Code yeni refresh token'la çalışmaya devam etsin diye dosyaya yazılır (üç alan, gerisi aynen).
    fn persist(&self, used: &creds::OAuth, fresh: &creds::OAuth) {
        let path = claude_dir().join(".credentials.json");
        let Ok(mut latest) = std::fs::read(&path) else {
            return;
        };
        if creds::parse(&latest).is_some_and(|d| d.refresh_token != used.refresh_token) {
            (self.log)("ai: kimlik dosyası yenileme sırasında değişti; dokunulmadı");
            win::wipe(&mut latest);
            return;
        }
        let mut backup = path.as_os_str().to_owned();
        backup.push(".lowmon-backup");
        let backup = PathBuf::from(backup);
        if !backup.exists() {
            let _ = std::fs::copy(&path, &backup);
        }
        match creds::patch(&latest, fresh) {
            Some(mut p) => {
                let r = win::write_atomic(&path, &p);
                win::wipe(&mut p);
                (self.log)(if r.is_ok() {
                    "ai: OAuth token yenilendi"
                } else {
                    "ai: yenilenen kimlik yazılamadı"
                });
            }
            None => (self.log)("ai: yenilenen kimlik yazılamadı (alan yok)"),
        }
        win::wipe(&mut latest);
    }

    fn fetch_session(&mut self) -> Result<Snapshot, String> {
        let key = std::fs::read(&self.session_file)
            .ok()
            .and_then(|b| win::dpapi(&b, false));
        let Some(mut key) = key.filter(|k| !k.is_empty()) else {
            return Err(self.l(Msg::WebNoKey).into());
        };
        let mut h = wide_header(&format!(
            "Cookie: sessionKey={}\r\nAccept: application/json\r\nReferer: https://claude.ai/settings/usage",
            String::from_utf8_lossy(&key)
        ));
        win::wipe(&mut key);
        let r = self.fetch_session_with(&h);
        win::wipe(&mut h);
        r
    }

    fn fetch_session_with(&mut self, h: &[u16]) -> Result<Snapshot, String> {
        let lang = self.cfg.language;
        let web = self.web.get_or_insert_with(|| Http::new(BROWSER_UA));
        let fail = |s: u32| {
            let cf = if s == 403 {
                t(lang, Msg::CookieOrCloudflare)
            } else {
                ""
            };
            format!("Web: HTTP {s}{cf}")
        };
        if self.org_id.is_empty() {
            let r = web
                .request("GET", "claude.ai", "/api/organizations", h, &[])
                .map_err(|e| format!("Web: {e}"))?;
            if r.status != 200 {
                return Err(fail(r.status));
            }
            self.org_id =
                api::pick_chat_org(&r.body).ok_or_else(|| t(lang, Msg::WebNoOrg).to_string())?;
        }
        let path = format!("/api/organizations/{}/usage", self.org_id);
        let r = web
            .request("GET", "claude.ai", &path, h, &[])
            .map_err(|e| format!("Web: {e}"))?;
        if r.status != 200 {
            if r.status == 404 {
                self.org_id.clear(); // eski org: bir sonrakinde yeniden bul
            }
            return Err(fail(r.status));
        }
        api::parse_usage(&r.body).ok_or_else(|| format!("Web: {}", t(lang, Msg::Unexpected)))
    }
}

/// sessionKey'i DPAPI ile şifreleyip kaydeder (menü: "panodan kaydet", Faz 2).
pub fn save_session_key(file: &std::path::Path, key: &str) -> std::io::Result<()> {
    let enc = win::dpapi(key.trim().as_bytes(), true).ok_or_else(std::io::Error::last_os_error)?;
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    win::write_atomic(file, &enc)
}
