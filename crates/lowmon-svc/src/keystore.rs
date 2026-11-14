//! Ana anahtar (ADR-0016): yoksa üret → DPAPI (çalışan hesap kapsamı) → key.bin.
use crate::security;
use lowmon_core::crypto::MasterKey;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{HLOCAL, LocalFree},
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

/// `protect`: true = şifrele, false = çöz. Kapsam: çalışan hesap (servis için LocalService).
fn dpapi(data: &[u8], protect: bool) -> io::Result<Vec<u8>> {
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

/// `owner_sid`: Some → key.bin DACL'ı yalnızca bu SID (servis); None → klasörden kalıtım (run modu).
pub fn load_or_create(path: &Path, owner_sid: Option<&str>) -> io::Result<MasterKey> {
    if let Ok(blob) = fs::read(path) {
        let key = dpapi(&blob, false)?;
        return key
            .try_into()
            .map_err(|_| io::Error::other("key.bin: anahtar 32 B değil"));
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| io::Error::other(e.to_string()))?;
    let blob = dpapi(&key, true)?;
    // create_new: iki süreç aynı anda üretirse biri kaybeder, mevcut anahtar ezilmez
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(&blob)?;
    if let Some(sid) = owner_sid {
        security::set_dacl(path, &format!("D:P(A;;FA;;;{sid})"))?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_load_same_key() {
        let dir = std::env::temp_dir().join(format!("lowmon-test-{}-key", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("key.bin");
        let a = load_or_create(&p, None).unwrap();
        assert_ne!(
            fs::read(&p).unwrap()[..],
            a[..],
            "anahtar düz metin yazılmış"
        );
        assert_eq!(load_or_create(&p, None).unwrap(), a);
        let _ = fs::remove_dir_all(&dir);
    }
}
