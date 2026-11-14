//! DPAPI (çalışan hesap kapsamı) ve key.bin okuma; servis ve araçlar paylaşır (ADR-0016).
use crate::crypto::MasterKey;
use std::{io, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{HLOCAL, LocalFree},
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

/// `protect`: true = şifrele, false = çöz. Kapsam: çalışan hesap (servis için LocalService).
pub fn dpapi(data: &[u8], protect: bool) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let fl = CRYPTPROTECT_UI_FORBIDDEN;
    // SAFETY: input çağrı boyunca canlı (DPAPI girdiyi değiştirmez); çıktı LocalFree ile bırakılır.
    unsafe {
        let ok = if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                fl,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                fl,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as HLOCAL);
        Ok(v)
    }
}

/// key.bin → ana anahtar. Başka hesabın (ör. servis) anahtarıysa DPAPI hatası döner.
pub fn load_key(path: &Path) -> io::Result<MasterKey> {
    let key = dpapi(&std::fs::read(path)?, false)?;
    key.try_into()
        .map_err(|_| io::Error::other("key.bin: anahtar 32 B değil"))
}
