//! 1 s toplayıcılar (spek §4). Hepsi delta tabanlı; ilk örnek atılır.
mod cpu;
mod disk;
mod mem;
mod net;

use lowmon_core::metrics::Sample;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Disk / arayüz yeniden keşfi, tick cinsinden (spek §4: penceresiz servis, WM_DEVICECHANGE yok).
const REDISCOVER_EVERY: u32 = 60;

#[derive(Default)]
pub struct Collector {
    cpu: cpu::Cpu,
    disks: disk::Disks,
    nics: net::Nics,
    last: Option<Instant>,
    tick: u32,
}

impl Collector {
    /// Örneği yerinde doldurur. `false` = ilk örnek veya okuma hatası → kullanma.
    pub fn collect(&mut self, s: &mut Sample) -> bool {
        if self.tick.is_multiple_of(REDISCOVER_EVERY) {
            self.disks.rediscover();
            self.nics.rediscover();
        }
        self.tick = self.tick.wrapping_add(1);
        let now = Instant::now();
        let dt_ms = self
            .last
            .map_or(0, |l| now.duration_since(l).as_millis() as u64);
        self.last = Some(now);
        s.ts_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        self.disks.read(dt_ms, &mut s.disk);
        self.nics.read(dt_ms, &mut s.net);
        let cpu_ok = self.cpu.read(s);
        mem::read(s) && cpu_ok
    }
}
