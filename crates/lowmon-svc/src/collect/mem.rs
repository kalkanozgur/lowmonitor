//! RAM / commit / cache: tek çağrı K32GetPerformanceInfo (psapi.dll yüklenmez).
use lowmon_core::metrics::Sample;
use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};

pub fn read(s: &mut Sample) -> bool {
    let mut pi = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: pi yığında, cb doğru.
    if unsafe { K32GetPerformanceInfo(&mut pi, pi.cb) } == 0 {
        return false;
    }
    let kb = |pages: usize| (pages as u64 * pi.PageSize as u64) / 1024;
    s.ram_used_kb = kb(pi.PhysicalTotal - pi.PhysicalAvailable);
    s.commit_kb = kb(pi.CommitTotal);
    s.cache_kb = kb(pi.SystemCache);
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_plausible_values() {
        let mut s = Default::default();
        assert!(super::read(&mut s));
        assert!(s.ram_used_kb > 0 && s.commit_kb > 0);
    }
}
