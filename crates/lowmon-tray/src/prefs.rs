//! `%APPDATA%\LowMonitor\tray.toml` (ADR-0041): tray'in kalıcı tercihleri. Dosya ya da alan yoksa
//! eski davranış (hepsi açık); yeni kurulumda setup seçimine göre yazılır (`--setup`).
use serde::Deserialize;
use std::{io, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Prefs {
    /// Alarm balonları (tray bildirimi)
    pub balloons: bool,
    /// Açılışta ve 24 saatte bir yeni sürüm kontrolü
    pub update_check: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            balloons: true,
            update_check: true,
        }
    }
}

pub fn path() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("LowMonitor")
        .join("tray.toml")
}

/// Dosya yok / bozuk → varsayılan (açık).
pub fn load() -> Prefs {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

fn text(p: &Prefs) -> String {
    format!(
        "# LowMonitor tray tercihleri. Değişiklik tray yeniden başlayınca geçerli olur.\n\
         # balloons: alarm balonları; update_check: açılışta ve 24 saatte bir yeni sürüm kontrolü\n\
         balloons = {}\nupdate_check = {}\n",
        p.balloons, p.update_check
    )
}

/// Dosya yoksa yazar; varsa dokunmaz (güncelleme kullanıcının seçimini ezmesin).
pub fn create(p: &Prefs) -> io::Result<()> {
    let path = path();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => io::Write::write_all(&mut f, text(p).as_bytes()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        assert_eq!(toml::from_str::<Prefs>("").unwrap(), Prefs::default());
        let off = Prefs {
            balloons: false,
            update_check: false,
        };
        assert_eq!(toml::from_str::<Prefs>(&text(&off)).unwrap(), off);
        // tek alan yazılmışsa diğeri varsayılan (açık) kalır
        assert!(
            toml::from_str::<Prefs>("balloons = false")
                .unwrap()
                .update_check
        );
    }
}
