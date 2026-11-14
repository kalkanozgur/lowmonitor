//! ACL yardımcıları (spek §11 adım 5): servis SID'i çözümleme ve SDDL'den korumalı DACL uygulama.
use std::{io, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{HLOCAL, LocalFree},
    Security::{
        ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, LookupAccountNameW,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SID_NAME_USE,
    },
};

pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain([0]).collect()
}

fn last_err<T>() -> io::Result<T> {
    Err(io::Error::last_os_error())
}

/// `NT SERVICE\<name>` → `S-1-5-80-…` (SDDL'de hesap adı değil SID dizesi gerekir).
pub fn service_sid(name: &str) -> io::Result<String> {
    let account = wide(format!(r"NT SERVICE\{name}"));
    let mut sid = [0u8; 68]; // SECURITY_MAX_SID_SIZE
    let (mut sid_len, mut dom_len) = (sid.len() as u32, 256u32);
    let mut dom = [0u16; 256];
    let mut use_: SID_NAME_USE = 0;
    // SAFETY: tamponlar ve uzunlukları eşleşiyor.
    let ok = unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
            dom.as_mut_ptr(),
            &mut dom_len,
            &mut use_,
        )
    };
    if ok == 0 {
        return last_err();
    }
    let mut s = ptr::null_mut();
    // SAFETY: sid geçerli; çıktı LocalFree ile bırakılır.
    if unsafe { ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &mut s) } == 0 {
        return last_err();
    }
    // SAFETY: s NUL sonlu UTF-16, LocalAlloc'lu.
    let out = unsafe {
        let len = (0..).take_while(|&i| *s.add(i) != 0).count();
        let v = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
        LocalFree(s as HLOCAL);
        v
    };
    Ok(out)
}

/// SDDL'deki DACL'ı uygular; kalıtım kesilir (PROTECTED) ve kalıtılabilir ACE'ler mevcut alt öğelere yayılır.
pub fn set_dacl(path: &Path, sddl: &str) -> io::Result<()> {
    let sddl = wide(sddl);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: sd çıktısı LocalFree ile bırakılır.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            ptr::null_mut(),
        )
    } == 0
    {
        return last_err();
    }
    let path = wide(path);
    let (mut present, mut defaulted, mut dacl) = (0, 0, ptr::null_mut::<ACL>());
    // SAFETY: sd geçerli; dacl sd içine işaret eder, sd bırakılmadan önce kullanılır.
    let err = unsafe {
        let err = if GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) == 0 {
            io::Error::last_os_error().raw_os_error().unwrap_or(1) as u32
        } else {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null(),
            )
        };
        LocalFree(sd as HLOCAL);
        err
    };
    if err == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(err as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dacl_applies_to_temp_file() {
        let p = std::env::temp_dir().join(format!("lowmon-test-{}-acl.txt", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        // Yalnızca SYSTEM + Administrators + sahip (OW): kendimizi kilitlemeden uygulanabildiğini doğrula
        set_dacl(&p, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)").unwrap();
        assert!(set_dacl(&p, "bozuk sddl").is_err());
        std::fs::remove_file(&p).unwrap();
    }
}
