//! `lowmon-hook`: Claude Code hook komutu (ADR-0034). stdin'deki hook JSON'undan yalnız
//! `hook_event_name`, `session_id`, `tool_name`, proje (`cwd` son parçası) ve güvenli özeti (`detail`,
//! ADR-0034 eki) çıkarır, tray penceresine `WM_COPYDATA` ile küçük bir JSON olarak iletir ve çıkar.
//! Komut metni, tam yol, ileti ve araç çıktısı bu süreçten dışarı çıkmaz.
//! Her zaman sessiz ve çıkış kodu 0: tray kapalıysa da Claude Code bloklanmaz, hata görmez.
//! Pencere alt sistemi: konsol (conhost) açılmaz; stdin borusu yine okunur.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Read;

/// Alanlar belgenin başındadır; gerisi (araç girdisi / çıktısı, MB'larca olabilir) okunup atılır.
const HEAD: usize = 8 * 1024;
/// Tray'in kabul ettiği `COPYDATASTRUCT.dwData` ("LMH1"); `lowmon-tray` ile aynı olmalı.
const MAGIC: usize = 0x4C4D_4831;
const TRAY_CLASS: &str = "LowMonitorTray";

/// `"key"` + `:` + dizgi değerinin ham (kaçışlı) içeriği; ilk geçtiği yer.
// shortcut: tam JSON ayrıştırma yok (belge 8 KB'ta kesik olabilir). Dizgi içinde tırnak kaçışlı olduğundan
// `"key":` yalnız nesne anahtarı olabilir; üst düzey alanlar iç içe nesnelerden önce gelir. Claude Code
// alan sırasını değiştirirse tam ayrıştırıcıya geç.
fn field<'a>(doc: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let pat = format!("\"{key}\"");
    let mut at = find(doc, pat.as_bytes())? + pat.len();
    let skip_ws = |i: &mut usize| {
        while doc.get(*i).is_some_and(|c| c.is_ascii_whitespace()) {
            *i += 1;
        }
    };
    skip_ws(&mut at);
    if doc.get(at) != Some(&b':') {
        return None;
    }
    at += 1;
    skip_ws(&mut at);
    if doc.get(at) != Some(&b'"') {
        return None;
    }
    let start = at + 1;
    let mut i = start;
    while let Some(&c) = doc.get(i) {
        match c {
            b'\\' => i += 1,
            b'"' => return Some(&doc[start..i]),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Ham (kaçışlı) yolun son parçası: `E:\\p\\lowmonitor` → `lowmonitor`.
fn last_part(raw: &[u8]) -> &[u8] {
    raw.split(|&c| c == b'\\' || c == b'/')
        .rfind(|p| !p.is_empty())
        .unwrap_or_default()
}

/// Güvenli özet (ADR-0034 eki): Bash/Agent açıklaması, dosya araçlarında dosya adı, arama deseni.
/// Komut metni ve tam yol hiç okunmaz. Uzun değer gönderilmez (tray kısaltır; kaçış yarıda kesilmesin).
fn detail<'a>(doc: &'a [u8], tool: &[u8]) -> Option<&'a [u8]> {
    const MAX_RAW: usize = 300;
    let v = match tool {
        b"Bash" | b"Agent" | b"Task" => field(doc, "description")?,
        b"Read" | b"Edit" | b"Write" | b"MultiEdit" | b"NotebookEdit" => {
            last_part(field(doc, "file_path").or_else(|| field(doc, "notebook_path"))?)
        }
        b"Grep" | b"Glob" => field(doc, "pattern")?,
        _ => return None,
    };
    (!v.is_empty() && v.len() <= MAX_RAW).then_some(v)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Tray'e gidecek küçük JSON; olay türü ya da oturum yoksa `None` (hiçbir şey gönderilmez).
fn message(doc: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(b"{\"hook_event_name\":\"");
    out.extend_from_slice(field(doc, "hook_event_name")?);
    out.extend_from_slice(b"\",\"session_id\":\"");
    out.extend_from_slice(field(doc, "session_id")?);
    if let Some(p) = field(doc, "cwd").map(last_part).filter(|p| !p.is_empty()) {
        out.extend_from_slice(b"\",\"project\":\"");
        out.extend_from_slice(p);
    }
    if let Some(tool) = field(doc, "tool_name") {
        out.extend_from_slice(b"\",\"tool_name\":\"");
        out.extend_from_slice(tool);
        if let Some(d) = detail(doc, tool) {
            out.extend_from_slice(b"\",\"detail\":\"");
            out.extend_from_slice(d);
        }
    }
    out.extend_from_slice(b"\"}");
    Some(out)
}

#[cfg(windows)]
fn send(msg: &[u8]) {
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA,
    };
    let class: Vec<u16> = TRAY_CLASS.encode_utf16().chain([0]).collect();
    let cds = COPYDATASTRUCT {
        dwData: MAGIC,
        cbData: msg.len() as u32,
        lpData: msg.as_ptr() as *mut _,
    };
    // SAFETY: sınıf adı NUL sonlu; `cds` ve `msg` çağrı boyunca yaşar (SendMessage eşzamanlı, alıcı kopyalar).
    // Tray yoksa FindWindowW null döner; takılıysa 50 ms'de vazgeçilir.
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if !hwnd.is_null() {
            SendMessageTimeoutW(
                hwnd,
                WM_COPYDATA,
                0,
                (&raw const cds) as isize,
                SMTO_ABORTIFHUNG,
                50,
                std::ptr::null_mut(),
            );
        }
    }
}

#[cfg(not(windows))]
fn send(_: &[u8]) {}

fn main() {
    let mut stdin = std::io::stdin().lock();
    let mut head = vec![0u8; HEAD];
    let mut n = 0;
    while n < HEAD {
        match stdin.read(&mut head[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    if let Some(msg) = message(&head[..n]) {
        send(&msg);
    }
    // Kalanı boşalt: yazan taraf (Claude Code) kapalı boruya yazıp hata almasın.
    let mut sink = [0u8; 16 * 1024];
    while matches!(stdin.read(&mut sink), Ok(k) if k > 0) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(doc: &str) -> Option<String> {
        message(doc.as_bytes()).map(|m| String::from_utf8(m).unwrap())
    }

    #[test]
    fn extracts_three_fields() {
        let doc = r#"{"session_id":"abc","transcript_path":"C:\\Users\\x\\t.jsonl","cwd":"E:\\p","hook_event_name" : "PreToolUse","tool_name":"Bash","tool_input":{"command":"echo \"tool_name\":\"Evil\" sk-ant-secret"}}"#;
        assert_eq!(
            msg(doc).as_deref(),
            Some(
                r#"{"hook_event_name":"PreToolUse","session_id":"abc","project":"p","tool_name":"Bash"}"#
            )
        );
        // araç yok
        assert_eq!(
            msg(r#"{"session_id":"s","hook_event_name":"Stop"}"#).as_deref(),
            Some(r#"{"hook_event_name":"Stop","session_id":"s"}"#)
        );
    }

    #[test]
    fn safe_detail_only() {
        let bash = r#"{"session_id":"s","cwd":"E:\\Projects\\lowmonitor","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"curl -H \"x-api-key: sk-ant-secret\" x","description":"Release build al"}}"#;
        let m = msg(bash).unwrap();
        assert!(
            m.contains(r#""project":"lowmonitor""#) && m.contains(r#""detail":"Release build al""#)
        );
        assert!(!m.contains("sk-ant") && !m.contains("curl"), "{m}");
        let edit = r#"{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"C:\\Users\\xan\\secret\\main.rs","old_string":"a"}}"#;
        let m = msg(edit).unwrap();
        assert!(
            m.contains(r#""detail":"main.rs""#) && !m.contains("xan"),
            "{m}"
        );
        // bilinmeyen araç / çok uzun açıklama → ayrıntı yok
        let mcp = r#"{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"mcp__x__y","tool_input":{"description":"z"}}"#;
        assert!(!msg(mcp).unwrap().contains("detail"));
        let long = format!(
            r#"{{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"description":"{}"}}}}"#,
            "a".repeat(400)
        );
        assert!(!msg(&long).unwrap().contains("detail"));
    }

    #[test]
    fn truncated_or_incomplete() {
        // 8 KB'ta kesilmiş belge: baştaki alanlar yeter
        let doc = r#"{"session_id":"s","hook_event_name":"PostToolUse","tool_name":"Read","tool_response":"aaaa"#;
        assert_eq!(
            msg(doc).as_deref(),
            Some(r#"{"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Read"}"#)
        );
        assert_eq!(msg(r#"{"hook_event_name":"Stop"}"#), None);
        assert_eq!(msg(r#"{"session_id":"s"}"#), None);
        assert_eq!(msg(r#"{"session_id":"s","hook_event_name":"St"#), None);
        assert_eq!(msg(r#"{"session_id":5,"hook_event_name":"Stop"}"#), None);
        assert_eq!(msg(""), None);
    }

    #[test]
    fn escaped_quote_in_value() {
        let doc = r#"{"session_id":"a\"b","hook_event_name":"Stop"}"#;
        assert_eq!(
            msg(doc).as_deref(),
            Some(r#"{"hook_event_name":"Stop","session_id":"a\"b"}"#)
        );
    }
}
