//! Ana anahtar (ADR-0016): yoksa üret → DPAPI (çalışan hesap kapsamı) → key.bin.
use crate::security;
use lowmon_core::{
    crypto::MasterKey,
    dpapi::{dpapi, load_key},
};
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
};

/// `owner_sid`: Some → key.bin DACL'ı yalnızca bu SID (servis); None → klasörden kalıtım (run modu).
pub fn load_or_create(path: &Path, owner_sid: Option<&str>) -> io::Result<MasterKey> {
    if path.exists() {
        return load_key(path);
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
    use std::fs;

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
