//! "Tanılama ▸ Rapor" (ADR-0043): sorun gideren için tek metin dosyası. Yalnız okur; token içeren
//! satırlar çıkarılır, kullanıcı adı maskelenir. E-postayı kullanıcı kendisi gönderir (önce Notepad'de görür).
use lowmon_ai::win::wide;
use std::{fmt::Write as _, os::windows::process::CommandExt, path::PathBuf, process::Command};
use windows_sys::Win32::{
    System::{
        Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
            RegGetValueW,
        },
        SystemInformation::GetLocalTime,
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS},
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LOG_LINES: usize = 120;

/// `%APPDATA%\LowMonitor`
pub fn dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("LowMonitor")
}

fn reg_str(hive: HKEY, key: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut n = std::mem::size_of_val(&buf) as u32;
    // SAFETY: buf/n geçerli; dizgeler NUL sonlu.
    let r = unsafe {
        RegGetValueW(
            hive,
            wide(key).as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut n,
        )
    };
    (r == 0).then(|| String::from_utf16_lossy(&buf[..(n as usize / 2).saturating_sub(1)]))
}

fn reg_dword(hive: HKEY, key: &str, name: &str) -> Option<u32> {
    let (mut v, mut n) = (0u32, 4u32);
    // SAFETY: v/n geçerli; dizgeler NUL sonlu.
    let r = unsafe {
        RegGetValueW(
            hive,
            wide(key).as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut v).cast(),
            &mut n,
        )
    };
    (r == 0).then_some(v)
}

fn run(exe: &str, args: &[&str]) -> String {
    Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("çalışmadı: {e}"))
}

/// Son `n` satır.
fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Token / anahtar içeren satırları çıkarır; `user` geçen yerleri maskeler.
fn redact(text: &str, user: &str) -> String {
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            let low = l.to_ascii_lowercase();
            if ["token", "secret", "password", "sessionkey"]
                .iter()
                .any(|k| low.contains(k))
            {
                "<gizli satır çıkarıldı>".to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    out.push(String::new());
    let joined = out.join("\n");
    if user.is_empty() {
        joined
    } else {
        joined.replace(user, "<kullanıcı>")
    }
}

fn file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| format!("(okunamadı: {e})"))
}

fn local_time() -> String {
    // SAFETY: parametresiz, yalnız dönüş değeri.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute
    )
}

fn report() -> String {
    const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    const ADV: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
    let d = dir();
    let local =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("LowMonitor");
    let dw =
        |n| reg_dword(HKEY_CURRENT_USER, ADV, n).map_or("(yok)".to_string(), |v| v.to_string());
    let mut s = String::new();
    let _ = writeln!(s, "LowMonitor tanılama · {}", local_time());
    let _ = writeln!(s, "\n=== Sürüm ===\ntray {}", crate::update::VERSION);
    let _ = writeln!(
        s,
        "güncelleme: {}",
        crate::update::describe(&crate::update::status(), lowmon_ai::text::tr)
    );
    let _ = writeln!(
        s,
        "\n=== Windows ===\n{} {} build {}",
        reg_str(HKEY_LOCAL_MACHINE, NT, "ProductName").unwrap_or_default(),
        reg_str(HKEY_LOCAL_MACHINE, NT, "DisplayVersion").unwrap_or_default(),
        reg_str(HKEY_LOCAL_MACHINE, NT, "CurrentBuild").unwrap_or_default(),
    );
    // SAFETY: parametre sabit, yan etkisiz.
    let monitors = unsafe { GetSystemMetrics(SM_CMONITORS) };
    let _ = writeln!(
        s,
        "monitör: {monitors} · TaskbarAl={} (0=sol) TaskbarDa={} (yok=widget açık)",
        dw("TaskbarAl"),
        dw("TaskbarDa")
    );
    let _ = writeln!(
        s,
        "\n=== Servis ===\n{}",
        run("sc.exe", &["query", "LowMonitor"])
    );
    let _ = writeln!(
        s,
        "\n=== Süreçler ===\n{}",
        run("tasklist.exe", &["/FI", "IMAGENAME eq lowmon*", "/NH"])
    );
    for name in ["tray.toml", "taskbar.toml", "lowvolume.toml", "deck.toml"] {
        let _ = writeln!(s, "\n=== {name} ===\n{}", file(&d.join(name)));
    }
    let _ = writeln!(
        s,
        "\n=== deck.log (son {LOG_LINES}) ===\n{}",
        tail(&file(&d.join("deck.log")), LOG_LINES)
    );
    let _ = writeln!(
        s,
        "\n=== lowvolume.log (son 40) ===\n{}",
        tail(&file(&local.join("lowvolume.log")), 40)
    );
    redact(&s, &std::env::var("USERNAME").unwrap_or_default())
}

/// Raporu `%APPDATA%\LowMonitor\diag.txt`'e yazar (üzerine); (yol, metin) döner.
pub fn write_report() -> std::io::Result<(PathBuf, String)> {
    let path = dir().join("diag.txt");
    std::fs::create_dir_all(dir())?;
    let text = report();
    std::fs::write(&path, &text)?;
    Ok((path, text))
}

/// Yüzde kodlama (RFC 3986 ayrılmamış karakterler dışı).
fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Hazır başlıklı "yeni issue" adresi; rapor URL'ye girmez (panodan yapıştırılır): sınır ve sızıntı yok.
pub fn issue_url() -> String {
    format!(
        "https://github.com/kalkanozgur/lowmonitor/issues/new?title={}&body={}",
        urlenc(&format!(
            "Tanılama raporu: LowMonitor v{}",
            crate::update::VERSION
        )),
        urlenc(
            "Tanılama raporu panoya kopyalanmıştı; buraya yapıştırın (Ctrl+V). Göndermeden önce kişisel bilgi olmadığını kontrol edin.\n\nSorun: "
        ),
    )
}

/// Metni panoya (CF_UNICODETEXT) koyar; başarıyı döner.
pub fn copy_to_clipboard(text: &str) -> bool {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
    };
    const CF_UNICODETEXT: u32 = 13;
    let w = wide(text);
    let bytes = w.len() * 2;
    // SAFETY: GlobalAlloc bloğu yazılıp kilidi açıldıktan sonra SetClipboardData'ya devredilir (başarıda
    // pano sahibi olur, başarısızlıkta burada serbest bırakılır); pano her durumda kapatılır.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        let mut ok = false;
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if !h.is_null() {
            let p = GlobalLock(h);
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p.cast::<u16>(), w.len());
                GlobalUnlock(h);
                EmptyClipboard();
                ok = !SetClipboardData(CF_UNICODETEXT, h).is_null();
            }
            if !ok {
                GlobalFree(h);
            }
        }
        CloseClipboard();
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_and_user() {
        let t = "bind = \"1.2.3.4\"\ntoken = \"abc123\"\nC:\\Users\\ahmet\\x\nSessionKey: z";
        let r = redact(t, "ahmet");
        assert!(r.contains("bind = \"1.2.3.4\""));
        assert!(!r.contains("abc123") && !r.contains("z\n"));
        assert!(!r.contains("ahmet"));
        assert!(r.contains(r"C:\Users\<kullanıcı>\x"));
    }

    #[test]
    fn issue_url_is_encoded_and_has_no_report() {
        assert_eq!(urlenc("a b/ç"), "a%20b%2F%C3%A7");
        let u = issue_url();
        assert!(u.starts_with("https://github.com/kalkanozgur/lowmonitor/issues/new?title="));
        assert!(!u.contains(' ') && !u.contains('\n') && u.len() < 600);
    }

    #[test]
    fn tail_keeps_last_lines() {
        assert_eq!(tail("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail("a", 5), "a");
        assert_eq!(tail("", 3), "");
    }

    #[test]
    fn report_has_sections_and_no_token() {
        let r = report();
        for s in [
            "=== Sürüm ===",
            "=== Windows ===",
            "=== Servis ===",
            "=== deck.toml ===",
        ] {
            assert!(r.contains(s), "{s}");
        }
        assert!(!r.to_ascii_lowercase().contains("token ="));
    }
}
