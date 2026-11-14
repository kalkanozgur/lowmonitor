//! Örnek tipi ve çözünürlükleri (spek §5.4).

/// CPU toplamı 0.01 % birimi (10000 = %100).
pub type CpuTotal = u16;
/// Çekirdek yükü 0.5 % birimi (200 = %100).
pub type CpuCore = u8;

/// Tek bir 1 s örneği. Collector her tick'te aynı örneği yeniden doldurur (ayırma yok).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Sample {
    pub ts_unix_ms: u64,
    pub cpu_total: CpuTotal,
    pub cpu_core: Vec<CpuCore>,
    pub ram_used_kb: u64,
    pub commit_kb: u64,
    pub cache_kb: u64,
    pub disk: Vec<DiskIo>,
    pub net: Vec<NetIo>,
}

/// Fiziksel disk (`\\.\PhysicalDriveN`) hızları, byte/s.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct DiskIo {
    pub drive: u32,
    pub r_bps: u64,
    pub w_bps: u64,
}

/// Ağ arayüzü (IF index) hızları, byte/s.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct NetIo {
    pub if_index: u32,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// Sayaç farkı → byte/s. Sayaç geri giderse (aygıt yeniden takıldı) 0.
pub fn per_sec(prev: u64, cur: u64, dt_ms: u64) -> u64 {
    (cur.saturating_sub(prev) * 1000)
        .checked_div(dt_ms)
        .unwrap_or(0)
}

/// Oran → CPU toplam birimi; `busy <= total` varsayılır, total 0 ise 0.
pub fn cpu_total_units(busy: u64, total: u64) -> CpuTotal {
    (busy.min(total) * 10_000).checked_div(total).unwrap_or(0) as CpuTotal
}

/// Oran → çekirdek birimi (0.5 % adım, yuvarlamalı).
pub fn cpu_core_units(busy: u64, total: u64) -> CpuCore {
    (busy.min(total) * 400)
        .checked_div(total)
        .map_or(0, |x| x.div_ceil(2)) as CpuCore
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units() {
        assert_eq!(cpu_total_units(0, 0), 0);
        assert_eq!(cpu_total_units(1, 4), 2500);
        assert_eq!(cpu_total_units(9, 4), 10_000); // taşma kırpılır
        assert_eq!(cpu_core_units(1, 2), 100);
        assert_eq!(cpu_core_units(1, 1), 200);
        assert_eq!(cpu_core_units(1, 400), 1); // %0.25 → 0.5 % adımına yuvarlanır
        assert_eq!(per_sec(100, 600, 500), 1000);
        assert_eq!(per_sec(600, 100, 500), 0); // sayaç sıfırlandı
        assert_eq!(per_sec(0, 10, 0), 0);
    }
}
