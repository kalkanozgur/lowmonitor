//! lowmon-net (spek §16): LAN canlı veri yayıncısı, kullanıcı oturumunda çalışır.
//! Ayar `%APPDATA%\LowMonitor\net.toml` (ADR-0023): servis config'i kullanıcıya okunamaz.
//!
//! Kullanım: `lowmon-net` (sunucu) · `lowmon-net pair` (6 haneli kod, 120 s) · `lowmon-net unpair --all`.
mod auth;
mod http;
mod live;
mod server;
mod sysinfo;

use serde::Deserialize;
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    thread,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const DEFAULT_TOML: &str = r#"# lowmon-net ayarları (spek §16.6) — değişiklik yeniden başlatınca geçerli olur
[net]
enabled         = true
bind            = "127.0.0.1"   # LAN için PC'nin LAN IP'si (ör. "192.168.1.20"); 0.0.0.0 kabul edilmez
port            = 47800
max_clients     = 4
allowed_origins = []            # tarayıcı istemcileri için, ör. ["http://192.168.1.30"]
"#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
struct Net {
    enabled: bool,
    bind: IpAddr,
    port: u16,
    max_clients: usize,
    allowed_origins: Vec<String>,
}

impl Default for Net {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: Ipv4Addr::LOCALHOST.into(),
            port: 47800,
            max_clients: 4,
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    net: Net,
}

fn parse(text: &str) -> Result<Net, String> {
    let n = toml::from_str::<File>(text).map_err(|e| e.to_string())?.net;
    if n.port == 0 {
        return Err("net.port 0 olamaz".into());
    }
    if !(1..=4).contains(&n.max_clients) {
        return Err("net.max_clients 1..=4 olmalı".into());
    }
    // Spek §16.2: yalnız seçilen arayüz
    if n.bind.is_unspecified() {
        return Err("net.bind: 0.0.0.0 / :: olamaz, arayüz IP'si yazın".into());
    }
    Ok(n)
}

fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").unwrap_or_else(|| ".".into());
    PathBuf::from(base).join("LowMonitor")
}

fn total_ram_mb() -> u64 {
    // SAFETY: MEMORYSTATUSEX sıfırla başlatılıp dwLength doldurulur; çağrı yalnız bu yapıya yazar.
    unsafe {
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) == 0 {
            return 0;
        }
        m.ullTotalPhys / (1024 * 1024)
    }
}

fn main() -> ExitCode {
    let store = auth::Store { dir: config_dir() };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {}
        ["pair"] => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64);
            return match store.start_pair(now) {
                Ok(code) => {
                    println!(
                        "Eşleştirme kodu: {code} ({} s geçerli)",
                        auth::PAIR_TTL_MS / 1000
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("pair: {e}");
                    ExitCode::from(1)
                }
            };
        }
        ["unpair", "--all"] => {
            return match store.clear() {
                Ok(()) => {
                    println!("Tüm token'lar silindi; açık bağlantılar 30 s içinde kapanır.");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("unpair: {e}");
                    ExitCode::from(1)
                }
            };
        }
        _ => {
            eprintln!("kullanım: lowmon-net [pair | unpair --all]");
            return ExitCode::from(2);
        }
    }
    let path = store.dir.join("net.toml");
    if !path.exists() {
        let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
        let _ = fs::write(&path, DEFAULT_TOML);
    }
    let cfg = match fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| parse(&t))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    if !cfg.enabled {
        return ExitCode::SUCCESS;
    }
    let listener = match TcpListener::bind((cfg.bind, cfg.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}:{} dinlenemiyor: {e}", cfg.bind, cfg.port);
            return ExitCode::from(1);
        }
    };
    eprintln!("lowmon-net ws://{}:{}/v1/live", cfg.bind, cfg.port);
    let shared = Arc::new(server::Shared::default());
    let (s2, total) = (shared.clone(), total_ram_mb());
    thread::Builder::new()
        .name("pipe".into())
        .stack_size(64 * 1024)
        .spawn(move || server::pipe_loop(&s2, total))
        .expect("pipe thread");
    let ctx = server::Ctx {
        store,
        origins: cfg.allowed_origins,
        max_clients: cfg.max_clients,
        host: std::env::var("COMPUTERNAME").unwrap_or_default(),
        total_mb: total,
    };
    match server::serve(&listener, &shared, &ctx) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("poll: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_and_limits() {
        assert_eq!(parse(DEFAULT_TOML), Ok(Net::default()));
        assert_eq!(parse("").unwrap().port, 47800);
        assert_eq!(
            parse("[net]\nbind = \"192.168.1.20\"\n").unwrap().bind,
            IpAddr::from([192, 168, 1, 20])
        );
        assert!(parse("[net]\nbind = \"0.0.0.0\"\n").is_err());
        assert!(parse("[net]\nmax_clients = 9\n").is_err());
        assert!(parse("[net]\nport = 0\n").is_err());
        assert!(parse("[net]\nbind = \"yok\"\n").is_err());
    }
}
