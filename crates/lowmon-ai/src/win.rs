//! Win32 katmanı: WinHTTP (TLS sistemden), DPAPI (sessionKey), Claude/VS Code süreç tespiti,
//! atomik dosya yazma. Token ve başlıklar log'a yazılmaz; tamponlar düşmeden sıfırlanır.
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HLOCAL, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Networking::WinHttp::*;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// Belgeler klasörü (yönlendirilmiş olabilir: `E:\Documents`).
pub fn documents_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut p: *mut u16 = ptr::null_mut();
    // SAFETY: p çıktı işaretçisi; başarıda NUL sonlu dizgeyi gösterir, her durumda CoTaskMemFree ile bırakılır.
    unsafe {
        let hr = SHGetKnownFolderPath(&FOLDERID_Documents, 0, ptr::null_mut(), &mut p);
        let out = (hr >= 0 && !p.is_null()).then(|| {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(
                p, len,
            )))
        });
        CoTaskMemFree(p.cast());
        out
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Sırrı tutan tamponu bırakmadan önce sıfırlar (derleyici silemesin diye volatile).
pub fn wipe<T: Copy + Default>(v: &mut [T]) {
    for x in v.iter_mut() {
        // SAFETY: x geçerli, hizalı bir &mut T.
        unsafe { ptr::write_volatile(x, T::default()) };
    }
}

const MAX_BODY: usize = 1 << 20;

pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: WinHTTP'den alınmış, bir kez kapatılan tutamaç.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

// SAFETY: WinHTTP oturum tutamacı thread'ler arası kullanılabilir; tek thread'den kullanıyoruz.
unsafe impl Send for Handle {}

fn last_err(at: &str) -> String {
    // SAFETY: yan etkisiz.
    format!("{at} failed (WinHTTP {})", unsafe { GetLastError() })
}

/// Bir User-Agent'lı WinHTTP oturumu; bağlantılar oturum boyunca havuzlanır.
pub struct Http(Handle);

impl Http {
    pub fn new(user_agent: &str) -> Self {
        let ua = wide(user_agent);
        // SAFETY: ua NUL sonlu; null proxy adları geçerli.
        let mut s = unsafe {
            WinHttpOpen(
                ua.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            )
        };
        if s.is_null() {
            // 8.1 öncesi
            // SAFETY: aynı.
            s = unsafe {
                WinHttpOpen(
                    ua.as_ptr(),
                    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                    ptr::null(),
                    ptr::null(),
                    0,
                )
            };
        }
        if !s.is_null() {
            let flags: u32 = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
            // SAFETY: s geçerli oturum; flags çağrı boyunca canlı.
            unsafe {
                WinHttpSetTimeouts(s, 10_000, 10_000, 15_000, 15_000);
                WinHttpSetOption(
                    s,
                    WINHTTP_OPTION_DECOMPRESSION,
                    (&raw const flags).cast(),
                    4,
                );
            }
        }
        Self(Handle(s))
    }

    /// HTTPS isteği (yanıt gövdesi ≤ 1 MiB). `headers` CRLF ayrılmış; çağıran sırrı içeren başlığı sonra `wipe` eder.
    pub fn request(
        &self,
        method: &str,
        host: &str,
        path: &str,
        headers: &[u16],
        body: &[u8],
    ) -> Result<Response, String> {
        self.request_max(method, host, path, headers, body, MAX_BODY)
    }

    /// `request`; yanıt gövdesi sınırı `max` bayt (kurulum dosyası gibi büyük indirmeler için).
    pub fn request_max(
        &self,
        method: &str,
        host: &str,
        path: &str,
        headers: &[u16],
        body: &[u8],
        max: usize,
    ) -> Result<Response, String> {
        if self.0.0.is_null() {
            return Err(last_err("WinHttpOpen"));
        }
        let (host, path, method) = (wide(host), wide(path), wide(method));
        // SAFETY: tüm dizgiler NUL sonlu ve çağrı boyunca canlı; tutamaçlar Handle ile kapanır.
        unsafe {
            let conn = Handle(WinHttpConnect(
                self.0.0,
                host.as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ));
            if conn.0.is_null() {
                return Err(last_err("WinHttpConnect"));
            }
            let req = Handle(WinHttpOpenRequest(
                conn.0,
                method.as_ptr(),
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ));
            if req.0.is_null() {
                return Err(last_err("WinHttpOpenRequest"));
            }
            // headers NUL sonlu değil: uzunluk açık verilir.
            let (hp, hl) = if headers.is_empty() {
                (ptr::null(), 0)
            } else {
                (headers.as_ptr(), headers.len() as u32)
            };
            let bp = if body.is_empty() {
                ptr::null()
            } else {
                body.as_ptr().cast()
            };
            if WinHttpSendRequest(req.0, hp, hl, bp, body.len() as u32, body.len() as u32, 0) == 0 {
                return Err(last_err("WinHttpSendRequest"));
            }
            if WinHttpReceiveResponse(req.0, ptr::null_mut()) == 0 {
                return Err(last_err("WinHttpReceiveResponse"));
            }
            let mut status = 0u32;
            let mut len = 4u32;
            WinHttpQueryHeaders(
                req.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                (&raw mut status).cast(),
                &mut len,
                ptr::null_mut(),
            );
            let mut out = Vec::new();
            loop {
                let mut avail = 0u32;
                if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                    return Err(last_err("WinHttpQueryDataAvailable"));
                }
                if avail == 0 {
                    break;
                }
                let old = out.len();
                if old + avail as usize > max {
                    return Err("response too large".into());
                }
                out.resize(old + avail as usize, 0);
                let mut read = 0u32;
                if WinHttpReadData(req.0, out.as_mut_ptr().add(old).cast(), avail, &mut read) == 0 {
                    return Err(last_err("WinHttpReadData"));
                }
                out.truncate(old + read as usize);
            }
            Ok(Response { status, body: out })
        }
    }
}

/// sessionKey şifresi; aynı hesaptaki diğer DPAPI kullanıcıları blob'u doğrudan kullanamasın.
/// C++ widget ile aynı → eski `session.bin` aynen çözülür.
const ENTROPY: &[u8] = b"ClaudeUsageBar.sessionKey.v1";

fn blob(d: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: d.len() as u32,
        pbData: d.as_ptr() as *mut u8,
    }
}

/// DPAPI (kullanıcı kapsamı) + sabit entropi. `protect` = şifrele.
pub fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    let (input, ent) = (blob(data), blob(ENTROPY));
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    // SAFETY: girdiler çağrı boyunca canlı; çıktı LocalFree ile bırakılır, önce sıfırlanır.
    unsafe {
        let ok = if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                &ent,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &ent,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return None;
        }
        let s = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let v = s.to_vec();
        wipe(s);
        LocalFree(out.pbData as HLOCAL);
        Some(v)
    }
}

/// Geçici dosya + `ReplaceFileW` (hedefin ACL'i korunur), olmazsa `MoveFileExW`.
pub fn write_atomic(target: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".lowmon-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    let (t, n) = (
        wide(&target.to_string_lossy()),
        wide(&tmp.to_string_lossy()),
    );
    // SAFETY: yollar NUL sonlu.
    let ok = unsafe {
        ReplaceFileW(
            t.as_ptr(),
            n.as_ptr(),
            ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            ptr::null(),
            ptr::null(),
        ) != 0
            || MoveFileExW(
                n.as_ptr(),
                t.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) != 0
    };
    if ok {
        Ok(())
    } else {
        let e = std::io::Error::last_os_error();
        let _ = std::fs::remove_file(&tmp);
        Err(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Other,
    /// Claude Desktop
    Desktop,
    /// Claude Code CLI (VS Code eklentisinin `claude.exe`'si dahil)
    Cli,
    /// VS Code (`Code.exe`)
    VsCode,
}

/// Exe yolundan tür. İki ürün de `claude.exe`; ayrımı kurulum yeri yapar.
/// `hint`: kullanıcının eklediği Desktop klasörü (büyük/küçük harf duyarsız alt dizgi).
pub fn classify(path: &str, hint: &str) -> Kind {
    let p = path.to_ascii_lowercase();
    let file = p.rsplit(['\\', '/']).next().unwrap_or("");
    if file == "code.exe" {
        return Kind::VsCode;
    }
    if file != "claude.exe" {
        return Kind::Other;
    }
    let hint = hint.to_ascii_lowercase();
    if [
        "\\anthropicclaude\\",
        "\\windowsapps\\claude_",
        "\\programs\\claude\\",
    ]
    .iter()
    .any(|n| p.contains(n))
        || (!hint.is_empty() && p.contains(&hint))
    {
        Kind::Desktop
    } else {
        Kind::Cli
    }
}

fn image_path(pid: u32) -> String {
    // SAFETY: tutamaç kapatılır; buf uzunluğu len ile verilir.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buf[..len as usize])
        }
    }
}

/// Çalışan Claude/VS Code süreçleri.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Running {
    pub desktop: bool,
    pub cli: bool,
    pub vscode: bool,
}

pub fn running(hint: &str) -> Running {
    let mut r = Running::default();
    // SAFETY: anlık görüntü tutamacı kapatılır; pe.dwSize ayarlı.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return r;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut pe);
        while ok != 0 {
            let n = pe
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(pe.szExeFile.len());
            let name = String::from_utf16_lossy(&pe.szExeFile[..n]).to_ascii_lowercase();
            if name == "claude.exe" {
                let path = image_path(pe.th32ProcessID);
                // Yolu okunamayan (ör. yükseltilmiş) claude.exe → CLI say: canlı token'ı asla döndürme.
                match if path.is_empty() {
                    Kind::Cli
                } else {
                    classify(&path, hint)
                } {
                    Kind::Desktop => r.desktop = true,
                    _ => r.cli = true,
                }
            } else if name == "code.exe" {
                r.vscode = true;
            }
            ok = Process32NextW(snap, &mut pe);
        }
        CloseHandle(snap);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        let d = r"C:\Users\me\AppData\Local\AnthropicClaude\app-1.2.3\claude.exe";
        assert_eq!(classify(d, ""), Kind::Desktop);
        assert_eq!(
            classify(
                r"C:\Program Files\WindowsApps\Claude_1.2.3.0_x64__pzs8sxrjxfjjc\app\claude.exe",
                ""
            ),
            Kind::Desktop
        );
        assert_eq!(
            classify(r"C:\Users\me\.local\bin\claude.exe", ""),
            Kind::Cli
        );
        assert_eq!(
            classify(
                r"C:\Users\me\.vscode\extensions\anthropic.claude-code-2.0\resources\native-binary\claude.exe",
                ""
            ),
            Kind::Cli
        );
        assert_eq!(
            classify(r"D:\Apps\MyClaude\claude.exe", "myclaude"),
            Kind::Desktop
        );
        assert_eq!(
            classify(r"C:\Program Files\Microsoft VS Code\Code.exe", ""),
            Kind::VsCode
        );
        assert_eq!(classify(r"C:\x\notepad.exe", ""), Kind::Other);
    }

    #[test]
    fn dpapi_roundtrip() {
        let enc = dpapi(b"sk-ant-sid01-test", true).unwrap();
        assert_ne!(enc, b"sk-ant-sid01-test");
        assert_eq!(dpapi(&enc, false).unwrap(), b"sk-ant-sid01-test");
    }

    #[test]
    fn atomic_write() {
        let p = std::env::temp_dir().join(format!("lowmon-test-ai-atomic-{}", std::process::id()));
        write_atomic(&p, b"a").unwrap();
        write_atomic(&p, b"bb").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"bb");
        std::fs::remove_file(&p).unwrap();
    }
}
