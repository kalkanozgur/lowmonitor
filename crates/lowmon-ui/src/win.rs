//! Windows: tek örnek, yerel saat farkı, süreç taraması + sonlandırma.

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent};
use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SW_RESTORE, SetForegroundWindow, ShowWindow, WM_APP,
};
use windows::core::{PCWSTR, w};

/// Otomatik sıfırlanan isimli olay: `--ai` ile açılan ikinci kopya, çalışan kopyayı AI sekmesine geçirir.
const SHOW_AI: PCWSTR = w!(r"Local\LowMonitorUi.ShowAi");

/// `SHOW_AI` her tetiklendiğinde `f` (arka iş parçacığında) çağrılır.
pub fn on_show_ai(f: impl Fn() + Send + 'static) {
    // SAFETY: isimli olay süreç ömrü boyunca açık kalır; dizge sabit.
    let Ok(ev) = (unsafe { CreateEventW(None, false, false, SHOW_AI) }) else {
        return;
    };
    let ev = ev.0 as usize; // HANDLE Send değil
    std::thread::spawn(move || {
        // SAFETY: ev yukarıda açılan, hiç kapatılmayan olay handle'ı.
        while unsafe { WaitForSingleObject(HANDLE(ev as _), INFINITE) }.0 == 0 {
            f();
        }
    });
}

/// İkinci kopya mevcut pencereyi öne getirip çıkar (tray "Arayüzü aç" tekrar tıklanınca).
/// Ayarlar kaydedildi → tray yeniden yükler (`lowmon-tray` `WM_RELOAD`). bit 0 = ai + taskbar, bit 1 = deck.
/// Tray kapalıysa `false` (ayar sonraki açılışta okunur).
pub fn tray_reload(bits: usize) -> bool {
    // SAFETY: sınıf adı sabit; pencere bulunmazsa çağrı yapılmaz.
    unsafe {
        match FindWindowW(w!("LowMonitorTray"), None) {
            Ok(h) => PostMessageW(
                Some(h),
                WM_APP + 6,
                windows::Win32::Foundation::WPARAM(bits),
                windows::Win32::Foundation::LPARAM(0),
            )
            .is_ok(),
            Err(_) => false,
        }
    }
}

pub fn single_instance(ai: bool) -> bool {
    // SAFETY: isimli mutex süreç ömrü boyunca açık kalır (handle bilerek kapatılmaz); dizgeler sabit.
    unsafe {
        let _ = CreateMutexW(None, false, w!(r"Local\LowMonitorUi"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return true;
        }
        if let Ok(h) = FindWindowW(None, w!("LowMonitor")) {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        if ai && let Ok(ev) = CreateEventW(None, false, false, SHOW_AI) {
            let _ = SetEvent(ev);
        }
    }
    false
}

/// `GetTimeZoneInformation` dönüşü (winnt.h); windows crate'inde sabit yok.
const TIME_ZONE_ID_DAYLIGHT: u32 = 2;

/// Yerel = UTC + dönüş (saniye).
pub fn utc_offset_s() -> i64 {
    let mut tz = TIME_ZONE_INFORMATION::default();
    // SAFETY: tz yerel, fonksiyon yalnız ona yazar.
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = tz.Bias
        + if id == TIME_ZONE_ID_DAYLIGHT {
            tz.DaylightBias
        } else {
            tz.StandardBias
        };
    -(bias as i64) * 60
}

// ---------------- Süreçler (LM spek §9.4, §10) ----------------

use crate::procs::{RawProc, exit};
use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_CANCELLED, FILETIME, HANDLE,
    STATUS_INFO_LENGTH_MISMATCH,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
    TerminateProcess, WaitForSingleObject,
};
use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION as Spi;
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// `Reserved1` içindeki belgelenmemiş ama sabit alanlar (ntddk; LM `topn.rs` ile aynı).
const OFF_PRIVATE_WS: usize = 0;
const OFF_CREATE_TIME: usize = 24;
const OFF_USER_TIME: usize = 32;
const OFF_KERNEL_TIME: usize = 40;

fn field(r: &[u8; 48], off: usize) -> u64 {
    u64::from_le_bytes(r[off..off + 8].try_into().unwrap_or_default())
}

/// Tek sistem çağrısıyla tüm süreçler (süreç açmadan). `buf` u64 → 8 bayt hizalı, yeniden kullanılır, ×1.5 büyür.
pub fn scan(buf: &mut Vec<u64>) -> Vec<RawProc> {
    if buf.is_empty() {
        *buf = vec![0; 512 * 1024 / 8];
    }
    loop {
        let mut len = 0u32;
        // SAFETY: buf bayt boyutuyla birlikte veriliyor.
        let st = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut len,
            )
        };
        if st == STATUS_INFO_LENGTH_MISMATCH {
            *buf = vec![0; ((len as usize).max(buf.len() * 8) * 3 / 2).div_ceil(8)];
            continue;
        }
        if st.is_err() {
            return Vec::new();
        }
        break;
    }
    let mut out = Vec::with_capacity(400);
    let base = buf.as_ptr().cast::<u8>();
    let mut off = 0usize;
    loop {
        // SAFETY: çekirdek kayıtları 8 bayt hizalı yazar; off NextEntryOffset zinciriyle buffer içinde kalır.
        let p = unsafe { &*base.add(off).cast::<Spi>() };
        let pid = p.UniqueProcessId.0 as usize as u32;
        let name = if p.ImageName.Buffer.is_null() {
            if pid == 0 {
                "Idle".into()
            } else {
                String::new()
            }
        } else {
            // SAFETY: ImageName.Buffer aynı buffer içinde, Length bayt cinsinden.
            String::from_utf16_lossy(unsafe {
                std::slice::from_raw_parts(p.ImageName.Buffer.0, p.ImageName.Length as usize / 2)
            })
        };
        let r = &p.Reserved1;
        if pid != 0 {
            out.push(RawProc {
                pid,
                ctime: field(r, OFF_CREATE_TIME) as i64,
                cpu_100ns: field(r, OFF_USER_TIME) + field(r, OFF_KERNEL_TIME),
                ws_bytes: field(r, OFF_PRIVATE_WS),
                name,
            });
        }
        if p.NextEntryOffset == 0 {
            break;
        }
        off += p.NextEntryOffset as usize;
    }
    out
}

/// Handle'ı kapsam sonunda kapatır.
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: OpenProcess'ten alınan geçerli handle, bir kez kapatılır.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn file_name(h: HANDLE) -> String {
    let mut buf = [0u16; 512];
    let mut n = buf.len() as u32;
    // SAFETY: buf ve n eşleşiyor; fonksiyon n'e yazılan karakter sayısını koyar.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut n,
        )
    };
    if ok.is_err() {
        return String::new();
    }
    let full = String::from_utf16_lossy(&buf[..n as usize]);
    full.rsplit('\\').next().unwrap_or_default().to_string()
}

/// Spek §10 akışı: aç → CreationTime eşleşmesi (PID reuse) → yasak liste → TerminateProcess. Dönüş `exit::*`.
pub fn kill(pid: u32, ctime: i64) -> i32 {
    // SAFETY: yalnız sayı argümanları; dönen handle Owned ile kapatılır.
    let h = match unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
    } {
        Ok(h) => Owned(h),
        Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => return exit::DENIED,
        Err(_) => return exit::GONE, // ERROR_INVALID_PARAMETER: süreç yok
    };
    let (mut c, mut x, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: h geçerli; çıktılar yerel.
    if unsafe { GetProcessTimes(h.0, &mut c, &mut x, &mut k, &mut u) }.is_err() {
        return exit::FAILED;
    }
    let created = ((c.dwHighDateTime as u64) << 32 | c.dwLowDateTime as u64) as i64;
    if created != ctime {
        return exit::GONE;
    }
    if crate::procs::protected(pid, &file_name(h.0)) {
        return exit::PROTECTED;
    }
    // SAFETY: h PROCESS_TERMINATE hakkıyla açıldı.
    match unsafe { TerminateProcess(h.0, 1) } {
        Ok(()) => exit::OK,
        Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => exit::DENIED,
        Err(_) => exit::FAILED,
    }
}

/// Yükseltilmiş kopya: `lowdeck-desktop.exe --kill <pid> <ctime>` (UAC). Aynı kontroller orada tekrar yapılır.
/// Bloklar (UAC + çıkış beklenir): çağıran ayrı thread'de çalıştırır.
pub fn kill_elevated(pid: u32, ctime: i64) -> i32 {
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
    else {
        return exit::FAILED;
    };
    let file: Vec<u16> = exe.encode_utf16().chain([0]).collect();
    let args: Vec<u16> = format!("--kill {pid} {ctime}")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: info tam dolduruldu, dizgeler çağrı boyunca yaşar.
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return if e.code() == ERROR_CANCELLED.to_hresult() {
            exit::CANCELLED
        } else {
            exit::FAILED
        };
    }
    let h = Owned(info.hProcess);
    let mut code = exit::FAILED as u32;
    // SAFETY: h, SEE_MASK_NOCLOSEPROCESS ile dönen süreç handle'ı.
    unsafe {
        WaitForSingleObject(h.0, INFINITE);
        let _ = GetExitCodeProcess(h.0, &mut code);
    }
    code as i32
}
