//! Oturum açılışında başlatma (spek §8): `HKCU\...\Run` değeri = tırnaklı exe yolu.
use crate::wide;
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR},
    System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    },
};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE: &str = "LowMonitorTray";

fn check(e: WIN32_ERROR) -> io::Result<()> {
    if e == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(e as i32))
    }
}

/// Kayıtlı komut satırı; değer yoksa None.
pub fn get(value: &str) -> io::Result<Option<String>> {
    get_in(RUN, value)
}

/// "Arayüzü aç" durumu (ADR-0031): tray ile aynı klasördeki `lowmon-ui.exe`.
#[derive(Debug, PartialEq)]
pub enum Ui {
    /// Exe yok (kurulmamış/silinmiş).
    Missing,
    Ready(String),
}

impl Ui {
    /// Menü satırı; yalnız `Ready` etkin.
    pub fn label(&self) -> &'static str {
        match self {
            Ui::Ready(_) => lowmon_ai::text::tr("Arayüzü aç", "Open interface"),
            Ui::Missing => lowmon_ai::text::tr(
                "Arayüzü aç (lowmon-ui.exe bulunamadı)",
                "Open interface (lowmon-ui.exe not found)",
            ),
        }
    }
}

/// Menü her açılışta denetler (yalnız sağ tıkta, periyodik iş yok).
pub fn ui() -> Ui {
    let exe = std::env::current_exe().map(|p| p.with_file_name("lowmon-ui.exe"));
    match exe {
        Ok(p) if p.is_file() => Ui::Ready(p.display().to_string()),
        _ => Ui::Missing,
    }
}

/// `HKCU\<key>` altındaki REG_SZ; değer yoksa None.
fn get_in(key: &str, value: &str) -> io::Result<Option<String>> {
    let (key, name) = (wide(key), wide(value));
    let mut buf = [0u16; 1024];
    let mut size = size_of_val(&buf) as u32;
    // SAFETY: buf ve size eşleşiyor; RRF_RT_REG_SZ sonucu NUL ile bitirir.
    let e = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(e)?;
    let n = (size as usize / 2).saturating_sub(1);
    Ok(Some(String::from_utf16_lossy(&buf[..n])))
}

pub fn set(value: &str, cmd: Option<&str>) -> io::Result<()> {
    let (key, name) = (wide(RUN), wide(value));
    // SAFETY: tüm dizgeler NUL sonlu; veri boyutu NUL dahil byte cinsinden.
    let e = unsafe {
        match cmd {
            Some(c) => {
                let data = wide(c);
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            }
            None => match RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) {
                ERROR_FILE_NOT_FOUND => 0,
                e => e,
            },
        }
    };
    check(e)
}

/// Bu exe için Run komut satırı.
pub fn this_exe() -> io::Result<String> {
    Ok(format!("\"{}\"", std::env::current_exe()?.display()))
}

/// Yalnızca bu exe'yi gösteriyorsa açık sayılır (taşınmış eski yol = kapalı, menüden yeniden açılır).
pub fn enabled() -> bool {
    matches!((get(VALUE), this_exe()), (Ok(Some(v)), Ok(me)) if v.eq_ignore_ascii_case(&me))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_delete() {
        let name = format!("LowMonitorTest{}", std::process::id());
        set(&name, Some(r#""C:\x y\a.exe""#)).unwrap();
        assert_eq!(get(&name).unwrap().as_deref(), Some(r#""C:\x y\a.exe""#));
        set(&name, None).unwrap();
        assert_eq!(get(&name).unwrap(), None);
        set(&name, None).unwrap(); // yokken silmek hata değil
    }
}
