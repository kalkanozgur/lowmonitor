//! `lowmon-ai`: Claude abonelik kullanımı (5 sa oturum + hafta), tray içinde (ADR-0032).
//! ai-usage-bar (C++) portu. Sorgu yalnız Claude Desktop, Claude Code CLI ya da VS Code
//! çalışırken yapılır; hepsi kapalıyken ağ isteği yok, son değer "eski" işaretli kalır.
//! Rapor / token dökümü (`breakdown`, `report`) `lowmon-ui` AI sekmesinde gösterilir (Faz 5).
pub mod activity;
pub mod api;
pub mod breakdown;
pub mod creds;
#[cfg(windows)]
pub mod fetch;
pub mod json;
pub mod report;
pub mod text;
pub mod usage;
#[cfg(windows)]
pub mod win;

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    /// OAuth, olmazsa sessionKey
    #[default]
    Auto,
    OAuth,
    Session,
}

/// `%APPDATA%\LowMonitor\ai.toml`. Yoksa varsayılanlar (dosyayı Faz 2'de tray oluşturur / taşır).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: text::Lang,
    /// Sorgu aralığı, ≥ 30 s
    pub refresh_sec: u64,
    pub auth: Auth,
    /// Süresi dolan Claude Code token'ını yenile (yalnız CLI kapalıyken)
    pub oauth_refresh: bool,
    /// Standart dışı Claude Desktop kurulum klasörünün bir parçası
    pub desktop_path_hint: String,
    /// Her başarılı sorguyu `usage-YYYY-MM.csv`'ye yaz
    pub csv_log: bool,
    /// Rapor için çözümlenen en yeni oturum dökümü sayısı (1..200)
    pub report_sessions: usize,
    /// Bulut oturum özetleri (`*_usage.json`); boş = Belgeler\Claude-Logs
    pub summary_dir: String,
    pub colors: usage::Colors,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: text::Lang::Tr,
            refresh_sec: 180,
            auth: Auth::Auto,
            oauth_refresh: true,
            desktop_path_hint: String::new(),
            csv_log: true,
            report_sessions: 30,
            summary_dir: String::new(),
            colors: usage::Colors::default(),
        }
    }
}

impl Config {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut c: Self = toml::from_str(s).map_err(|e| e.to_string())?;
        c.refresh_sec = c.refresh_sec.max(30);
        c.report_sessions = c.report_sessions.clamp(1, 200);
        Ok(c)
    }

    /// Dosya yoksa varsayılan; bozuksa hata (sessizce varsayılana dönmez).
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

fn env_dir(var: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(var).unwrap_or_default()).join("LowMonitor")
}

/// `%APPDATA%\LowMonitor\ai.toml`
pub fn config_path() -> PathBuf {
    env_dir("APPDATA").join("ai.toml")
}

/// `%LOCALAPPDATA%\LowMonitor\ai` (CSV, `session.bin`)
pub fn data_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("ai")
}

/// Rapor girdilerinin yerleri (`ai.toml` `summary_dir` boşsa Belgeler\Claude-Logs).
pub fn report_paths(cfg: &Config) -> report::Paths {
    #[cfg(windows)]
    let docs = win::documents_dir();
    #[cfg(not(windows))]
    let docs: Option<PathBuf> = None;
    report::Paths {
        data_dir: data_dir(),
        claude_dir: claude_dir(),
        summary_dir: if cfg.summary_dir.is_empty() {
            docs.unwrap_or_default().join("Claude-Logs")
        } else {
            PathBuf::from(&cfg.summary_dir)
        },
    }
}

/// `CLAUDE_CONFIG_DIR` ya da `%USERPROFILE%\.claude`.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".claude")
        })
}

/// Son durum; tray / taskbar / deck okur.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub snapshot: Option<api::Snapshot>,
    /// Son başarılı sorgu (Unix s); 0 = hiç
    pub fetched_at: i64,
    /// "OAuth" | "Web"
    pub source: &'static str,
    /// Son sorgu hatası; boş = başarılı. Sır içermez.
    pub error: String,
    /// Claude/VS Code çalışıyor → sorgu açık
    pub active: bool,
}

impl State {
    /// Veri eski: son sorgu hatalı, sorgu kapalı ya da 2 aralıktan eski.
    pub fn stale(&self, now: i64, refresh_sec: u64) -> bool {
        !self.error.is_empty() || !self.active || now - self.fetched_at > 2 * refresh_sec as i64
    }
}

#[cfg(windows)]
pub use worker::{Running, start};

#[cfg(windows)]
mod worker {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct Shared {
        state: State,
        wake: bool,
        stop: bool,
    }

    pub struct Running {
        inner: Arc<(Mutex<Shared>, Condvar)>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Running {
        pub fn state(&self) -> State {
            self.inner
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .state
                .clone()
        }

        /// Beklemeyi keser, hemen sorgular (Claude kapalı olsa da).
        pub fn refresh_now(&self) {
            let (m, cv) = &*self.inner;
            m.lock().unwrap_or_else(|e| e.into_inner()).wake = true;
            cv.notify_one();
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            let (m, cv) = &*self.inner;
            m.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
            cv.notify_one();
            // WinHTTP zaman aşımı ≤ 15 s; çıkışta beklemeyelim.
            drop(self.thread.take());
        }
    }

    /// İşçi thread'i başlatır. `on_change` durum değişince (işçi thread'inden) çağrılır.
    pub fn start(cfg: Config, on_change: Box<dyn Fn(&State) + Send>, log: fn(&str)) -> Running {
        let inner = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let i2 = inner.clone();
        let thread = std::thread::Builder::new()
            .name("lowmon-ai".into())
            .stack_size(256 * 1024)
            .spawn(move || run(cfg, &i2, &*on_change, log))
            .ok();
        Running { inner, thread }
    }

    fn run(
        cfg: Config,
        inner: &(Mutex<Shared>, Condvar),
        on_change: &dyn Fn(&State),
        log: fn(&str),
    ) {
        let dir = data_dir();
        let mut f = fetch::Fetcher::new(cfg.clone(), dir.join("session.bin"), log);
        let mut forced = false;
        loop {
            let r = win::running(&cfg.desktop_path_hint);
            let active = r.desktop || r.cli || r.vscode;
            let mut next = inner
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .state
                .clone();
            next.active = active;
            if active || forced {
                let now = fetch::now_ms() / 1000;
                match f.fetch() {
                    Ok((snap, src)) => {
                        next.snapshot = Some(snap);
                        next.fetched_at = now;
                        next.source = if src == fetch::Source::OAuth {
                            "OAuth"
                        } else {
                            "Web"
                        };
                        next.error.clear();
                        if cfg.csv_log
                            && let Err(e) = usage::append_csv(&dir, now, &snap)
                        {
                            log(&format!("ai: CSV yazılamadı: {e}"));
                        }
                    }
                    Err(e) => {
                        if next.error != e {
                            log(&format!("ai: sorgu başarısız: {e}"));
                        }
                        next.error = e;
                    }
                }
            }
            let mut g = inner.0.lock().unwrap_or_else(|e| e.into_inner());
            if g.state != next {
                g.state = next;
                let s = g.state.clone();
                drop(g);
                on_change(&s);
                g = inner.0.lock().unwrap_or_else(|e| e.into_inner());
            }
            let (g2, _) = inner
                .1
                .wait_timeout_while(g, Duration::from_secs(cfg.refresh_sec), |s| {
                    !s.wake && !s.stop
                })
                .unwrap_or_else(|e| e.into_inner());
            g = g2;
            if g.stop {
                return;
            }
            forced = std::mem::take(&mut g.wake);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        let c =
            Config::parse("language = \"en\"\nrefresh_sec = 5\n[colors]\nmode = \"threshold\"\n")
                .unwrap();
        assert_eq!(c.language, text::Lang::En);
        assert_eq!(c.refresh_sec, 30);
        assert_eq!(c.colors.mode, usage::ColorMode::Threshold);
        assert_eq!(c.colors.crit_pct, 90.0);
        assert!(Config::parse("bogus = 1").is_err());
    }

    #[test]
    fn stale() {
        let mut s = State {
            fetched_at: 1000,
            active: true,
            ..Default::default()
        };
        assert!(!s.stale(1300, 180));
        assert!(s.stale(1400, 180));
        s.error = "x".into();
        assert!(s.stale(1001, 180));
    }
}
