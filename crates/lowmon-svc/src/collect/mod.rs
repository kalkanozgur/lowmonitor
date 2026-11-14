//! 1 s toplayıcılar (spek §4). Hepsi delta tabanlı; ilk örnek atılır.
mod cpu;
mod disk;
mod mem;
mod net;
mod topn;

use lowmon_core::metrics::{Sample, TopEntry};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Disk / arayüz yeniden keşfi, tick cinsinden (spek §4: penceresiz servis, WM_DEVICECHANGE yok).
const REDISCOVER_EVERY: u32 = 60;
// shortcut: sabit; config.toml [sampling] topn_every / topn_count M2'de
const TOPN_EVERY: u32 = 5;
const TOPN_COUNT: usize = 5;

#[derive(Default)]
pub struct Collector {
    cpu: cpu::Cpu,
    disks: disk::Disks,
    nics: net::Nics,
    topn: topn::TopN,
    last: Option<Instant>,
    last_top: Option<Instant>,
    tick: u32,
}

/// Bu tick'te geçerli çıktı üretenler.
pub struct Collected {
    pub sample: bool,
    pub top: bool,
}

impl Collector {
    /// `s` her tick, `top` her TOPN_EVERY tick'te yerinde doldurulur. İlk örnek/tarama geçersizdir.
    pub fn collect(&mut self, s: &mut Sample, top: &mut Vec<TopEntry>) -> Collected {
        let tick = self.tick;
        self.tick = tick.wrapping_add(1);
        if tick.is_multiple_of(REDISCOVER_EVERY) {
            self.disks.rediscover();
            self.nics.rediscover();
        }
        let sample = self.sample(s);
        let mut top_ok = false;
        if tick.is_multiple_of(TOPN_EVERY) {
            let now = Instant::now();
            let elapsed_100ns = self
                .last_top
                .map_or(0, |l| (now.duration_since(l).as_nanos() / 100) as u64);
            self.last_top = Some(now);
            let ncpu = s.cpu_core.len() as u64;
            top_ok = self.topn.scan(elapsed_100ns, ncpu, TOPN_COUNT, top) && !top.is_empty();
        }
        Collected {
            sample,
            top: top_ok,
        }
    }

    fn sample(&mut self, s: &mut Sample) -> bool {
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
