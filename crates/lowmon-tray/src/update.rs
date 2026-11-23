//! GitHub'dan sürüm denetimi ve güncelleme (ADR-0042). Ağ yalnız ayrı thread'lerde (WinHTTP, TLS
//! sistemden; `lowmon_ai::win::Http`), UI thread'i hiç beklemez: bitince pencereye mesaj gelir.
//! Yalnız bu deponun release'i: API yanıtı `GET /repos/…/releases/latest` (ön sürümler gelmez),
//! indirme adresi `github.com/kalkanozgur/lowmonitor/releases/download/` ile başlamalı, dosya
//! GitHub'ın verdiği SHA-256 özetiyle doğrulanmadan çalıştırılmaz. Sessiz kurulum yok: kullanıcı onaylar.
use lowmon_ai::{
    json,
    win::{Http, wide},
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Mutex, thread};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

const API_HOST: &str = "api.github.com";
const API_PATH: &str = "/repos/kalkanozgur/lowmonitor/releases/latest";
const ASSET_PREFIX: &str = "https://github.com/kalkanozgur/lowmonitor/releases/download/";
const MAX_SETUP: usize = 64 << 20;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// `v1.2.3`
    pub tag: String,
    /// Sürüm notları sayfası
    pub page: String,
    pub setup_url: String,
    /// Küçük harfli 64 karakter hex
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading,
    Failed(String),
}

static STATE: Mutex<Status> = Mutex::new(Status::Idle);
/// İndirilip doğrulanmış kurulum dosyası (WM_UPDATE_READY'de çalıştırılır).
static READY: Mutex<Option<PathBuf>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn status() -> Status {
    lock(&STATE).clone()
}

pub fn take_ready() -> Option<PathBuf> {
    lock(&READY).take()
}

/// `v1.2.3` / `1.2.3` → sayılar; ön sürüm (`-rc1`) ve bozuk biçim `None` (teklif edilmez).
fn parse_ver(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim_start_matches('v').split('.');
    let v = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    it.next().is_none().then_some(v)
}

pub fn is_newer(current: &str, tag: &str) -> bool {
    matches!((parse_ver(current), parse_ver(tag)), (Some(c), Some(t)) if t > c)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `releases/latest` yanıtından kurulum dosyası: ad `LowMonitor-Setup-*.exe`, adres bu depodan,
/// özet `sha256:<64 hex>`. Biri eksik / uyumsuzsa `None` (özetsiz dosya çalıştırılmaz).
pub fn parse_release(doc: &[u8]) -> Option<Release> {
    let root = json::root(doc)?;
    let tag = json::str_member(doc, root, "tag_name")?;
    let page = json::str_member(doc, root, "html_url")?;
    let assets = json::member(doc, root, "assets")?;
    json::elements(doc, assets).find_map(|a| {
        let name = json::str_member(doc, a, "name")?;
        if !(name.starts_with("LowMonitor-Setup-") && name.ends_with(".exe")) {
            return None;
        }
        let setup_url = json::str_member(doc, a, "browser_download_url")?;
        let digest = json::str_member(doc, a, "digest")?;
        let sha256 = digest.strip_prefix("sha256:")?.to_ascii_lowercase();
        (setup_url.starts_with(ASSET_PREFIX)
            && sha256.len() == 64
            && sha256.bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| Release {
            tag: tag.clone(),
            page: page.clone(),
            setup_url,
            sha256,
            size: json::num_member(doc, a, "size") as u64,
        })
    })
}

fn http() -> Http {
    Http::new(&format!("LowMonitor/{VERSION}"))
}

/// Son sürüm bu sürümden yeniyse döner. Bloklar: yalnız worker thread'inden çağrılır.
fn check() -> Result<Option<Release>, String> {
    let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n"
        .encode_utf16()
        .collect();
    let r = http().request("GET", API_HOST, API_PATH, &headers, &[])?;
    if r.status != 200 {
        return Err(format!("GitHub HTTP {}", r.status));
    }
    let rel =
        parse_release(&r.body).ok_or("sürüm bilgisi okunamadı (kurulum dosyası / özet yok)")?;
    Ok(is_newer(VERSION, &rel.tag).then_some(rel))
}

/// Kurulum dosyasını `%TEMP%`'e indirir ve özeti doğrular. Bloklar: worker thread'i.
fn download(rel: &Release) -> Result<PathBuf, String> {
    let path = rel
        .setup_url
        .strip_prefix("https://github.com")
        .ok_or("beklenmeyen indirme adresi")?;
    let r = http().request_max("GET", "github.com", path, &[], &[], MAX_SETUP)?;
    if r.status != 200 {
        return Err(format!("indirme HTTP {}", r.status));
    }
    if hex(&Sha256::digest(&r.body)) != rel.sha256 {
        return Err("SHA-256 özeti uyuşmuyor, dosya silindi".into());
    }
    let out = std::env::temp_dir().join(format!(
        "LowMonitor-Setup-{}.exe",
        rel.tag.trim_start_matches('v')
    ));
    std::fs::write(&out, &r.body).map_err(|e| e.to_string())?;
    Ok(out)
}

fn post(hwnd: usize, msg: u32, wp: usize) {
    // SAFETY: pencere yok olduysa PostMessage yalnız başarısız döner.
    unsafe { PostMessageW(hwnd as HWND, msg, wp as WPARAM, 0 as LPARAM) };
}

/// Arka planda denetler; bitince `msg` (wParam = `manual`) gelir. Zaten çalışıyorsa bir şey yapmaz.
pub fn start_check(hwnd: HWND, msg: u32, manual: bool) {
    {
        let mut s = lock(&STATE);
        if matches!(*s, Status::Checking | Status::Downloading) {
            return;
        }
        *s = Status::Checking;
    }
    let h = hwnd as usize;
    let spawned = thread::Builder::new()
        .name("update-check".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            *lock(&STATE) = match check() {
                Ok(Some(r)) => Status::Available(r),
                Ok(None) => Status::UpToDate,
                Err(e) => Status::Failed(e),
            };
            post(h, msg, usize::from(manual));
        });
    if spawned.is_err() {
        *lock(&STATE) = Status::Failed("thread açılamadı".into());
    }
}

/// Arka planda indirip doğrular; bitince `msg` gelir (`take_ready` doluysa kurulum başlatılabilir).
pub fn start_download(hwnd: HWND, msg: u32, rel: Release) {
    *lock(&STATE) = Status::Downloading;
    let h = hwnd as usize;
    let spawned = thread::Builder::new()
        .name("update-download".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            match download(&rel) {
                Ok(p) => {
                    *lock(&READY) = Some(p);
                    *lock(&STATE) = Status::Available(rel);
                }
                Err(e) => *lock(&STATE) = Status::Failed(e),
            }
            post(h, msg, 0);
        });
    if spawned.is_err() {
        *lock(&STATE) = Status::Failed("thread açılamadı".into());
    }
}

/// Menü / ileti için kısa durum metni (`tr` çağıran tarafta).
pub fn describe(s: &Status, tr: fn(&'static str, &'static str) -> &'static str) -> String {
    let v = format!("{} {VERSION}", tr("Sürüm", "Version"));
    match s {
        Status::Idle => v,
        Status::Checking => format!("{v} · {}", tr("denetleniyor…", "checking…")),
        Status::UpToDate => format!("{v} · {}", tr("güncel", "up to date")),
        Status::Available(r) => format!("{v} · {} {}", r.tag, tr("mevcut", "available")),
        Status::Downloading => format!("{v} · {}", tr("indiriliyor…", "downloading…")),
        Status::Failed(e) => format!(
            "{v} · {}: {}",
            tr("denetlenemedi", "check failed"),
            e.lines().next().unwrap_or("")
        ),
    }
}

/// Setup'ı çalıştırır: manifest yönetici ister, UAC `ShellExecute` ile gelir. Tray'i setup kapatır.
pub fn launch(hwnd: HWND, path: &std::path::Path) {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    // SAFETY: NUL sonlu dizgeler çağrı boyunca yaşar.
    unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide(&path.display().to_string()).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "915fbc9b9e4edb3b88cad6e3ed935569b0f25f80291d8742eb4f484a43be9836";

    fn sample(tag: &str, url: &str, digest: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/kalkanozgur/lowmonitor/releases/tag/{tag}",
"prerelease":false,"assets":[
{{"name":"lowmon-tray.exe","browser_download_url":"{url}x","digest":"sha256:{SHA}","size":5}},
{{"name":"LowMonitor-Setup-9.9.9.exe","browser_download_url":"{url}","digest":"{digest}","size":6995007}}]}}"#
        )
    }

    #[test]
    fn versions() {
        assert!(is_newer("1.2.2", "v1.2.3"));
        assert!(is_newer("1.2.9", "v1.3.0"));
        assert!(is_newer("1.9.0", "v2.0.0"));
        assert!(!is_newer("1.2.3", "v1.2.3"));
        assert!(!is_newer("1.3.0", "v1.2.9"));
        assert!(!is_newer("1.2.2", "v1.3.0-rc1")); // ön sürüm teklif edilmez
        assert!(!is_newer("1.2.2", "latest"));
        assert!(!is_newer("1.2.2", "v1.2"));
    }

    #[test]
    fn parses_setup_asset_only() {
        let url = format!("{ASSET_PREFIX}v9.9.9/LowMonitor-Setup-9.9.9.exe");
        let r = parse_release(
            sample("v9.9.9", &url, &format!("sha256:{}", SHA.to_uppercase())).as_bytes(),
        )
        .expect("release");
        assert_eq!(r.tag, "v9.9.9");
        assert_eq!(r.setup_url, url);
        assert_eq!(r.sha256, SHA); // büyük harf özet normalize
        assert_eq!(r.size, 6_995_007);
    }

    #[test]
    fn rejects_foreign_url_or_bad_digest() {
        let ok = format!("{ASSET_PREFIX}v9.9.9/LowMonitor-Setup-9.9.9.exe");
        let d = format!("sha256:{SHA}");
        // başka depo / başka sunucu
        assert!(
            parse_release(
                sample(
                    "v9.9.9",
                    "https://evil.example/LowMonitor-Setup-9.9.9.exe",
                    &d
                )
                .as_bytes()
            )
            .is_none()
        );
        assert!(
            parse_release(
                sample(
                    "v9.9.9",
                    "https://github.com/other/repo/releases/download/v1/LowMonitor-Setup-9.9.9.exe",
                    &d
                )
                .as_bytes()
            )
            .is_none()
        );
        // özet yok / bozuk / kısa
        assert!(parse_release(sample("v9.9.9", &ok, "md5:abc").as_bytes()).is_none());
        assert!(parse_release(sample("v9.9.9", &ok, "sha256:abcd").as_bytes()).is_none());
        assert!(
            parse_release(sample("v9.9.9", &ok, &format!("sha256:{}zz", &SHA[..62])).as_bytes())
                .is_none()
        );
        // bozuk JSON
        assert!(parse_release(b"{").is_none());
    }

    #[test]
    fn hex_digest() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Gerçek GitHub: `cargo test -p lowmon-tray -- --ignored live_check` (ağ gerekir, dosya indirmez).
    #[test]
    #[ignore = "ağ"]
    fn live_check() {
        let r = check().expect("GitHub'a ulaşılamadı");
        println!("{r:?}");
    }
}
