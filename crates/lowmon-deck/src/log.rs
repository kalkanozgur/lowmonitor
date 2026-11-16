//! Log dosyası (Faz 4): konsolsuz release exe'de `println!` kaybolur.
//! `%APPDATA%\LowDeck\agent.log`, `MAX_BYTES`'ı aşınca `agent.log.1`'e döner → toplam ≤ 2 × `MAX_BYTES`.
//! Yazma hatası sessiz: log yüzünden agent durmaz. Token log'a yazılmaz.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAX_BYTES: u64 = 1 << 20;

/// `log!("…{x}")`: zaman damgalı satır; debug build'de ayrıca stdout.
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(&format!($($t)*)) };
}

struct Log {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    max: u64,
}

impl Log {
    fn open(path: PathBuf, max: u64) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        let size = file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map_or(0, |m| m.len());
        Self {
            path,
            file,
            size,
            max,
        }
    }

    fn write(&mut self, line: &[u8]) {
        if self.size + line.len() as u64 > self.max {
            self.file = None; // Windows: açık dosya yeniden adlandırılamaz
            let old = rotated(&self.path);
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&self.path, &old);
            *self = Self::open(std::mem::take(&mut self.path), self.max);
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line).is_ok()
        {
            self.size += line.len() as u64;
        }
    }
}

fn rotated(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".1");
    p.into()
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Açılışta bir kez; klasör yoksa oluşturur.
pub fn init(path: PathBuf) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut g) = LOG.lock() {
        *g = Some(Log::open(path, MAX_BYTES));
    }
}

pub fn path() -> Option<PathBuf> {
    LOG.lock().ok()?.as_ref().map(|l| l.path.clone())
}

pub fn write(msg: &str) {
    #[cfg(debug_assertions)]
    println!("{msg}");
    let line = format!("{} {msg}\n", stamp());
    if let Ok(mut g) = LOG.lock()
        && let Some(l) = g.as_mut()
    {
        l.write(line.as_bytes());
    }
}

/// `YYYY-MM-DD HH:MM:SS.mmm`, yerel saat.
#[cfg(windows)]
fn stamp() -> String {
    // SAFETY: parametresiz, yalnız dönüş değeri.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// Windows dışı (test/geliştirme): UTC.
#[cfg(not(windows))]
fn stamp() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let (s, milli) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, sod) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // H. Hinnant civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{milli:03}Z",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_limit() {
        let dir = std::env::temp_dir().join(format!("lowdeck-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("agent.log");
        let mut l = Log::open(p.clone(), 25);
        for _ in 0..3 {
            l.write(b"0123456789\n"); // 11 B: 2 satır sığar, 3. döndürür
        }
        assert_eq!(std::fs::read(&p).unwrap().len(), 11);
        assert_eq!(std::fs::read(rotated(&p)).unwrap().len(), 22);
        l.write(b"0123456789\n");
        l.write(b"0123456789\n"); // ikinci döndürme: eski .1 silinir
        assert_eq!(std::fs::read(&p).unwrap().len(), 11);
        assert_eq!(std::fs::read(rotated(&p)).unwrap().len(), 22);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stamp_shape() {
        let s = stamp();
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[19..20], ".");
    }
}
