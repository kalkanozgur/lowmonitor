//! Top-N süreç (CPU): NtQuerySystemInformation(SystemProcessInformation), spek §4.
use lowmon_core::metrics::{TopEntry, cpu_total_units};
use std::collections::HashMap;
use windows_sys::{
    Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation},
    Win32::{
        Foundation::STATUS_INFO_LENGTH_MISMATCH,
        System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION as Spi,
    },
};

/// Belgelenmemiş ama sabit alanlar `Reserved1` içinde (ntddk SYSTEM_PROCESS_INFORMATION).
const OFF_PRIVATE_WS: usize = 0;
const OFF_CREATE_TIME: usize = 24;
const OFF_USER_TIME: usize = 32;
const OFF_KERNEL_TIME: usize = 40;

/// Anahtar: pid + CreateTime → PID yeniden kullanımı karışmaz.
type Key = (u32, i64);

pub struct TopN {
    buf: Vec<u64>, // u64 → 8 bayt hizalı; tek sefer ayrılır, ×1.5 büyür, küçülmez
    prev: HashMap<Key, u64>,
    cur: HashMap<Key, u64>,
}

struct Cand<'a> {
    delta: u64,
    pid: u32,
    ws_kb: u64,
    name: &'a [u16], // buf içine bakar; String yalnızca ilk N için üretilir
}

impl Default for TopN {
    fn default() -> Self {
        Self {
            buf: vec![0; 512 * 1024 / 8],
            prev: HashMap::new(),
            cur: HashMap::new(),
        }
    }
}

fn field(r: &[u8; 48], off: usize) -> u64 {
    u64::from_le_bytes(r[off..off + 8].try_into().unwrap())
}

impl TopN {
    /// `elapsed_100ns` × `ncpu` = sistemin toplam CPU kapasitesi. İlk taramada `out` boş kalır.
    pub fn scan(
        &mut self,
        elapsed_100ns: u64,
        ncpu: u64,
        n: usize,
        out: &mut Vec<TopEntry>,
    ) -> bool {
        if !self.query() {
            return false;
        }
        self.cur.clear();
        let mut cand = Vec::new();
        let base = self.buf.as_ptr().cast::<u8>();
        let mut off = 0usize;
        loop {
            // SAFETY: çekirdek kayıtları 8 bayt hizalı yazar; off NextEntryOffset zinciriyle buffer içinde kalır.
            let p = unsafe { &*base.add(off).cast::<Spi>() };
            let pid = p.UniqueProcessId as usize as u32;
            if pid != 0 {
                let r = &p.Reserved1;
                let key = (pid, field(r, OFF_CREATE_TIME) as i64);
                let t = field(r, OFF_USER_TIME) + field(r, OFF_KERNEL_TIME);
                if let Some(&old) = self.prev.get(&key) {
                    let delta = t.saturating_sub(old);
                    if delta > 0 {
                        let name = if p.ImageName.Buffer.is_null() {
                            &[][..]
                        } else {
                            // SAFETY: ImageName.Buffer aynı buffer içinde, Length bayt cinsinden.
                            unsafe {
                                std::slice::from_raw_parts(
                                    p.ImageName.Buffer,
                                    p.ImageName.Length as usize / 2,
                                )
                            }
                        };
                        cand.push(Cand {
                            delta,
                            pid,
                            ws_kb: field(r, OFF_PRIVATE_WS) / 1024,
                            name,
                        });
                    }
                }
                self.cur.insert(key, t);
            }
            if p.NextEntryOffset == 0 {
                break;
            }
            off += p.NextEntryOffset as usize;
        }
        // Ölen süreçler cur'a girmedi → swap ile temizlenir.
        std::mem::swap(&mut self.prev, &mut self.cur);
        rank(cand, elapsed_100ns * ncpu, n, out);
        true
    }

    pub(super) fn query(&mut self) -> bool {
        loop {
            let mut len = 0u32;
            // SAFETY: buf bayt boyutuyla birlikte veriliyor.
            let st = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    self.buf.as_mut_ptr().cast(),
                    (self.buf.len() * 8) as u32,
                    &mut len,
                )
            };
            if st != STATUS_INFO_LENGTH_MISMATCH {
                return st >= 0;
            }
            let want = (len as usize).max(self.buf.len() * 8) * 3 / 2;
            self.buf = vec![0; want.div_ceil(8)];
        }
    }
}

fn rank(mut cand: Vec<Cand>, capacity_100ns: u64, n: usize, out: &mut Vec<TopEntry>) {
    cand.sort_unstable_by_key(|c| std::cmp::Reverse(c.delta));
    out.clear();
    out.extend(cand.into_iter().take(n).map(|c| TopEntry {
        pid: c.pid,
        cpu: cpu_total_units(c.delta, capacity_100ns),
        ws_kb: c.ws_kb,
        name: String::from_utf16_lossy(c.name),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(delta: u64, pid: u32) -> Cand<'static> {
        Cand {
            delta,
            pid,
            ws_kb: 0,
            name: &[],
        }
    }

    #[test]
    fn rank_orders_and_truncates() {
        let mut out = Vec::new();
        rank(vec![c(10, 1), c(50, 2), c(30, 3)], 100, 2, &mut out);
        assert_eq!(
            out.iter().map(|e| (e.pid, e.cpu)).collect::<Vec<_>>(),
            [(2, 5000), (3, 3000)]
        );
    }

    #[test]
    fn scan_sees_own_process() {
        let mut t = TopN::default();
        let mut out = Vec::new();
        assert!(t.scan(10_000_000, 1, 1000, &mut out));
        assert!(out.is_empty()); // ilk tarama
        let me = std::process::id();
        assert!(t.prev.keys().any(|k| k.0 == me));
        let spin = std::time::Instant::now();
        while spin.elapsed().as_millis() < 30 {} // kendimize CPU zamanı yaz
        assert!(t.scan(10_000_000, 1, 1000, &mut out));
        assert!(
            out.iter()
                .any(|e| e.pid == me && e.name.contains("lowmon_svc"))
        );
    }
}
