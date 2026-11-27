//! PC güç komutları (ADR-0044; LowDeck ADR-0012): uyku / kapat / yeniden başlat / iptal.
//!
//! UAC gerekmez: standart kullanıcı token'ında `SeShutdownPrivilege` vardır ama kapalıdır,
//! `AdjustTokenPrivileges` ile açılır. Kapat/yeniden başlat `InitiateShutdownW` ile `SHUTDOWN_DELAY_S`
//! geri sayımla başlar (Windows bildirimi; `AbortSystemShutdownW` ile iptal). `ExitWindowsEx` iptal
//! edilemediği için kullanılmaz.

use lowdeck_proto::{POWER_OK, PowerCmd};
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::{
    AbortSystemShutdownW, InitiateShutdownW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
    SHTDN_REASON_MINOR_OTHER, SHUTDOWN_POWEROFF, SHUTDOWN_RESTART,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, w};

/// Kapat / yeniden başlat geri sayımı (kullanıcı kararı 2026-11-27).
pub const SHUTDOWN_DELAY_S: u16 = 10;
/// Uykuya geçmeden önce `PowerResult` çerçevesinin gitmesi için bekleme.
const SLEEP_AFTER: Duration = Duration::from_millis(300);
/// `ERROR_PRIVILEGE_NOT_HELD`
const PRIVILEGE_NOT_HELD: i32 = 1314;

/// (sonuç kodu, geri sayım s): `POWER_OK` ya da Win32 hata kodu.
pub fn run(cmd: PowerCmd) -> (i32, u16) {
    if let Err(code) = enable_shutdown_privilege() {
        return (code, 0);
    }
    match cmd {
        PowerCmd::Sleep => {
            // Yanıt `session`'da bu dönüşten sonra yazılır; uyku ayrı thread'de, kısa gecikmeyle.
            let r = std::thread::Builder::new()
                .name("lowdeck-sleep".into())
                .stack_size(64 * 1024)
                .spawn(|| {
                    std::thread::sleep(SLEEP_AFTER);
                    // SAFETY: argümansız powrprof çağrısı; bhibernate=false → uyku (S3 / Modern Standby).
                    if !unsafe { SetSuspendState(false, false, false) } {
                        // SAFETY: aynı thread'deki son Win32 hatası.
                        log!("uyku başarısız: {}", unsafe { GetLastError() }.0);
                    }
                });
            match r {
                Ok(_) => (POWER_OK, 0),
                Err(e) => (e.raw_os_error().unwrap_or(-2), 0),
            }
        }
        PowerCmd::Shutdown | PowerCmd::Restart => {
            let flags = if cmd == PowerCmd::Shutdown {
                SHUTDOWN_POWEROFF
            } else {
                SHUTDOWN_RESTART
            };
            // SAFETY: null makine adı = bu PC; mesaj statik UTF-16 dizgesi.
            let r = unsafe {
                InitiateShutdownW(
                    PCWSTR::null(),
                    w!("LowDeck (telefon)"),
                    u32::from(SHUTDOWN_DELAY_S),
                    flags,
                    SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED,
                )
            };
            if r == 0 {
                (POWER_OK, SHUTDOWN_DELAY_S)
            } else {
                (r as i32, 0)
            }
        }
        // SAFETY: null makine adı = bu PC.
        PowerCmd::Cancel => match unsafe { AbortSystemShutdownW(PCWSTR::null()) } {
            Ok(()) => (POWER_OK, 0),
            Err(e) => (win32(&e), 0),
        },
    }
}

/// Süreç token'ında `SeShutdownPrivilege`'ı açar (zaten açıksa etkisiz).
fn enable_shutdown_privilege() -> Result<(), i32> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess sahte handle döner; token çıkış parametresi yerel değişken.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    }
    .map_err(|e| win32(&e))?;
    let r = (|| {
        let mut luid = LUID::default();
        // SAFETY: yerel sistem; ad statik sabit; luid yerel değişken.
        unsafe { LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid) }
            .map_err(|e| win32(&e))?;
        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        // SAFETY: token geçerli (yukarıda açıldı); tp çağrı süresince yaşar; önceki durum istenmiyor.
        unsafe { AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None) }
            .map_err(|e| win32(&e))?;
        // Başarılı dönüşte bile ayrıcalık token'da yoksa ERROR_NOT_ALL_ASSIGNED.
        // SAFETY: aynı thread'deki son Win32 hatası.
        if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
            return Err(PRIVILEGE_NOT_HELD);
        }
        Ok(())
    })();
    // SAFETY: OpenProcessToken'ın açtığı handle, bir kez kapatılır.
    let _ = unsafe { CloseHandle(token) };
    r
}

/// `HRESULT_FROM_WIN32` → Win32 kodu (telefon metni Win32 kodlarını tanır); diğerleri ham HRESULT.
fn win32(e: &windows::core::Error) -> i32 {
    let h = e.code().0 as u32;
    if h & 0xFFFF_0000 == 0x8007_0000 {
        (h & 0xFFFF) as i32
    } else {
        h as i32
    }
}
