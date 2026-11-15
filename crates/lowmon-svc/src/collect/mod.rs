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
#[derive(Default)]
pub struct Collector {
    topn_every: u32,
    topn_count: usize,
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
    pub fn new(topn_every: u32, topn_count: usize) -> Self {
        Self {
            topn_every,
            topn_count,
            ..Default::default()
        }
    }

    /// `s` her tick, `top` her `topn_every` tick'te yerinde doldurulur. İlk örnek/tarama geçersizdir.
    pub fn collect(&mut self, s: &mut Sample, top: &mut Vec<TopEntry>) -> Collected {
        let tick = self.tick;
        self.tick = tick.wrapping_add(1);
        if tick.is_multiple_of(REDISCOVER_EVERY) {
            self.disks.rediscover();
            self.nics.rediscover();
        }
        let sample = self.sample(s);
        let mut top_ok = false;
        if self.topn_count > 0 && tick.is_multiple_of(self.topn_every) {
            let now = Instant::now();
            let elapsed_100ns = self
                .last_top
                .map_or(0, |l| (now.duration_since(l).as_nanos() / 100) as u64);
            self.last_top = Some(now);
            let ncpu = s.cpu_core.len() as u64;
            top_ok = self.topn.scan(elapsed_100ns, ncpu, self.topn_count, top) && !top.is_empty();
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

#[cfg(test)]
mod bench {
    use super::*;

    /// Çağrı başına duvar süresi (kernel dahil). `cargo test -p lowmon-svc --release cost_per_call -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cost_per_call() {
        fn t(name: &str, n: u32, mut f: impl FnMut()) -> f64 {
            f();
            let s = Instant::now();
            for _ in 0..n {
                f();
            }
            let us = s.elapsed().as_secs_f64() * 1e6 / n as f64;
            println!("{name:<14} {us:>9.1} µs/çağrı");
            us
        }
        let mut s = Sample::default();
        let mut top = Vec::new();
        let (mut c, mut d, mut ni, mut tn) = (
            cpu::Cpu::default(),
            disk::Disks::default(),
            net::Nics::default(),
            topn::TopN::default(),
        );
        d.rediscover();
        ni.rediscover();
        let cpu = t("cpu", 500, || _ = c.read(&mut s));
        let mem = t("mem", 500, || _ = mem::read(&mut s));
        let disk = t("disk", 500, || d.read(1000, &mut s.disk));
        let net = t("net", 500, || ni.read(1000, &mut s.net));
        let topn = t("topn", 100, || _ = tn.scan(50_000_000, 16, 5, &mut top));
        t("topn sorgu", 100, || _ = tn.query());
        let red_d = t("disk keşif", 20, || {
            d = disk::Disks::default();
            d.rediscover()
        });
        let red_n = t("nic keşif", 20, || ni.rediscover());
        let per_s = cpu + mem + disk + net + topn / 5.0 + (red_d + red_n) / 60.0;
        println!("tahmini toplam {:.0} µs/s", per_s);
    }
}
