//! 1 s toplayıcılar (spek §4). Hepsi delta tabanlı; ilk örnek atılır.
mod cpu;
mod mem;

use lowmon_core::metrics::Sample;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct Collector {
    cpu: cpu::Cpu,
}

impl Collector {
    /// Örneği yerinde doldurur. `false` = ilk örnek veya okuma hatası → kullanma.
    pub fn collect(&mut self, s: &mut Sample) -> bool {
        s.ts_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let cpu_ok = self.cpu.read(s);
        mem::read(s) && cpu_ok
    }
}
