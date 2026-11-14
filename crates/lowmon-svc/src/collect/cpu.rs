//! CPU toplam + çekirdek: NtQuerySystemInformation(SystemProcessorPerformanceInformation).
use lowmon_core::metrics::{Sample, cpu_core_units, cpu_total_units};
use windows_sys::{
    Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessorPerformanceInformation,
    },
    Win32::System::WindowsProgramming::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION as Sppi,
};

// shortcut: tek processor group (≤ 64 çekirdek); fazlası gerekirse SystemProcessorPerformanceInformationEx (spek §15)
const MAX_CPUS: usize = 64;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Times {
    idle: u64,
    total: u64, // kernel + user; kernel idle'ı içerir
}

#[derive(Default)]
pub struct Cpu {
    raw: Vec<Sppi>,
    prev: Vec<Times>,
    cur: Vec<Times>,
}

impl Cpu {
    pub fn read(&mut self, s: &mut Sample) -> bool {
        if self.raw.is_empty() {
            self.raw = vec![Sppi::default(); MAX_CPUS];
        }
        let mut len = 0u32;
        // SAFETY: raw MAX_CPUS elemanlık, boyut bayt cinsinden doğru veriliyor.
        let st = unsafe {
            NtQuerySystemInformation(
                SystemProcessorPerformanceInformation,
                self.raw.as_mut_ptr().cast(),
                size_of_val(self.raw.as_slice()) as u32,
                &mut len,
            )
        };
        if st < 0 {
            return false;
        }
        let n = len as usize / size_of::<Sppi>();
        self.cur.clear();
        self.cur.extend(self.raw[..n].iter().map(|p| Times {
            idle: p.IdleTime as u64,
            total: (p.KernelTime + p.UserTime) as u64,
        }));
        let valid = self.prev.len() == n;
        if valid {
            fill(&self.prev, &self.cur, s);
        }
        std::mem::swap(&mut self.prev, &mut self.cur);
        valid
    }
}

fn fill(prev: &[Times], cur: &[Times], s: &mut Sample) {
    s.cpu_core.clear();
    let (mut busy_sum, mut total_sum) = (0, 0);
    for (p, c) in prev.iter().zip(cur) {
        let total = c.total.wrapping_sub(p.total);
        let busy = total.saturating_sub(c.idle.wrapping_sub(p.idle));
        s.cpu_core.push(cpu_core_units(busy, total));
        busy_sum += busy;
        total_sum += total;
    }
    s.cpu_total = cpu_total_units(busy_sum, total_sum);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(idle: u64, total: u64) -> Times {
        Times { idle, total }
    }

    #[test]
    fn delta_math() {
        let mut s = Sample::default();
        // çekirdek 0: %25 meşgul, çekirdek 1: tamamen boşta
        fill(&[t(0, 0), t(0, 0)], &[t(75, 100), t(100, 100)], &mut s);
        assert_eq!(s.cpu_core, vec![50, 0]);
        assert_eq!(s.cpu_total, 1250);
        // ilerleme yoksa 0, bölme hatası yok
        fill(&[t(5, 5)], &[t(5, 5)], &mut s);
        assert_eq!((s.cpu_total, s.cpu_core.as_slice()), (0, &[0][..]));
    }

    #[test]
    fn first_read_is_dropped_then_valid() {
        let (mut cpu, mut s) = (Cpu::default(), Sample::default());
        assert!(!cpu.read(&mut s));
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(cpu.read(&mut s));
        assert!(!s.cpu_core.is_empty() && s.cpu_total <= 10_000);
    }
}
