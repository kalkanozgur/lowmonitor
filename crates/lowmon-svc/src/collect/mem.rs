//! RAM / commit / cache: tek çağrı K32GetPerformanceInfo (psapi.dll yüklenmez).
use lowmon_core::metrics::Sample;
use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};

/// Dönüş: alarm yüzdeleri için (toplam RAM KB, commit limiti KB); okunamazsa `None`.
pub fn read(s: &mut Sample) -> Option<(u64, u64)> {
    let mut pi = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: pi yığında, cb doğru.
    if unsafe { K32GetPerformanceInfo(&mut pi, pi.cb) } == 0 {
        return None;
    }
    let kb = |pages: usize| (pages as u64 * pi.PageSize as u64) / 1024;
    s.ram_used_kb = kb(pi.PhysicalTotal - pi.PhysicalAvailable);
    s.commit_kb = kb(pi.CommitTotal);
    s.cache_kb = kb(pi.SystemCache);
    Some((kb(pi.PhysicalTotal), kb(pi.CommitLimit)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_plausible_values() {
        let mut s = Default::default();
        let (total, limit) = super::read(&mut s).unwrap();
        assert!(s.ram_used_kb > 0 && s.commit_kb > 0);
        assert!(total >= s.ram_used_kb && limit >= s.commit_kb);
    }
}
