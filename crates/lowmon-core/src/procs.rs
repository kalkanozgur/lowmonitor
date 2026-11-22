//! Süreç anlık görüntüsü: tek `NtQuerySystemInformation(SystemProcessInformation)` çağrısı, süreç açmadan
//! (spek §4). lowmon-svc Top-N ve lowmon-deck Süreçler (ADR-0009) aynı kodu kullanır.
use windows_sys::{
    Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation},
    Win32::{
        Foundation::STATUS_INFO_LENGTH_MISMATCH,
        System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION as Spi,
    },
};

/// `Reserved1` içindeki belgelenmemiş ama sabit alanlar (ntddk SYSTEM_PROCESS_INFORMATION).
const OFF_PRIVATE_WS: usize = 0;
const OFF_CREATE_TIME: usize = 24;
const OFF_USER_TIME: usize = 32;
const OFF_KERNEL_TIME: usize = 40;

/// Görüntüdeki bir süreç; `name` tampona bakar (String yalnız gerekince üretilir).
pub struct Proc<'a> {
    pub pid: u32,
    /// CreationTime (FILETIME, 100 ns); pid ile birlikte kimlik → PID yeniden kullanımı karışmaz.
    pub ctime: i64,
    /// Kullanıcı + çekirdek CPU zamanı, 100 ns.
    pub cpu_100ns: u64,
    /// Private working set, bayt.
    pub ws_bytes: u64,
    /// UTF-16 görüntü adı; System gibi adsız süreçlerde boş.
    pub name: &'a [u16],
}

fn field(r: &[u8; 48], off: usize) -> u64 {
    u64::from_le_bytes(r[off..off + 8].try_into().unwrap_or_default())
}

/// `buf`'u doldurur. u64 → 8 bayt hizalı; boşsa 512 KB ayrılır, yetmezse ×1.5 büyür, küçülmez. Hata → `false`.
pub fn snapshot(buf: &mut Vec<u64>) -> bool {
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
        if st != STATUS_INFO_LENGTH_MISMATCH {
            return st >= 0;
        }
        *buf = vec![0; ((len as usize).max(buf.len() * 8) * 3 / 2).div_ceil(8)];
    }
}

/// Başarılı `snapshot` sonrası `buf`'taki her süreç için `f` (pid 0 = Idle hariç).
pub fn each<'a>(buf: &'a [u64], mut f: impl FnMut(Proc<'a>)) {
    if buf.is_empty() {
        return;
    }
    let base = buf.as_ptr().cast::<u8>();
    let mut off = 0usize;
    loop {
        // SAFETY: çekirdek kayıtları 8 bayt hizalı yazar; off NextEntryOffset zinciriyle buffer içinde kalır.
        let p = unsafe { &*base.add(off).cast::<Spi>() };
        let pid = p.UniqueProcessId as usize as u32;
        if pid != 0 {
            let r = &p.Reserved1;
            let name = if p.ImageName.Buffer.is_null() {
                &[][..]
            } else {
                // SAFETY: ImageName.Buffer aynı buffer içinde, Length bayt cinsinden.
                unsafe {
                    std::slice::from_raw_parts(p.ImageName.Buffer, p.ImageName.Length as usize / 2)
                }
            };
            f(Proc {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_own_process() {
        let mut buf = Vec::new();
        assert!(snapshot(&mut buf));
        let me = std::process::id();
        let mut found = false;
        each(&buf, |p| {
            if p.pid == me {
                found = String::from_utf16_lossy(p.name).contains("lowmon_core") && p.ctime > 0;
            }
        });
        assert!(found);
    }
}
