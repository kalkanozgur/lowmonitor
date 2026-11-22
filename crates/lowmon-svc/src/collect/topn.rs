//! Top-N süreç (CPU): `lowmon_core::procs` anlık görüntüsü, spek §4.
use lowmon_core::metrics::{TopEntry, cpu_total_units};
use lowmon_core::procs;
use std::collections::HashMap;

/// Anahtar: pid + CreateTime → PID yeniden kullanımı karışmaz.
type Key = (u32, i64);

#[derive(Default)]
pub struct TopN {
    buf: Vec<u64>, // procs::snapshot tamponu; ilk taramada ayrılır, küçülmez
    prev: HashMap<Key, u64>,
    cur: HashMap<Key, u64>,
}

struct Cand<'a> {
    delta: u64,
    pid: u32,
    ws_kb: u64,
    name: &'a [u16], // buf içine bakar; String yalnızca ilk N için üretilir
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
        if !procs::snapshot(&mut self.buf) {
            return false;
        }
        self.cur.clear();
        let mut cand = Vec::new();
        let (prev, cur) = (&self.prev, &mut self.cur);
        procs::each(&self.buf, |p| {
            let key = (p.pid, p.ctime);
            if let Some(&old) = prev.get(&key) {
                let delta = p.cpu_100ns.saturating_sub(old);
                if delta > 0 {
                    cand.push(Cand {
                        delta,
                        pid: p.pid,
                        ws_kb: p.ws_bytes / 1024,
                        name: p.name,
                    });
                }
            }
            cur.insert(key, p.cpu_100ns);
        });
        // Ölen süreçler cur'a girmedi → swap ile temizlenir.
        std::mem::swap(&mut self.prev, &mut self.cur);
        rank(cand, elapsed_100ns * ncpu, n, out);
        true
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
