//! Örnek tipi ve çözünürlükleri (spek §5.4).

/// CPU toplamı 0.01 % birimi (10000 = %100).
pub type CpuTotal = u16;
/// Çekirdek yükü 0.5 % birimi (200 = %100).
pub type CpuCore = u8;

/// Tek bir 1 s örneği. Collector her tick'te aynı örneği yeniden doldurur (ayırma yok).
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiskIo {
    pub drive: u32,
    pub r_bps: u64,
    pub w_bps: u64,
}

/// Ağ arayüzü (IF index) hızları, byte/s.
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NetIo {
    pub if_index: u32,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// Top-N satırı (spek §5.4 TopN). `cpu` = sistem kapasitesinin 0.01 %'u.
// shortcut: isim String; segment başına ProcDict (dict_id) M2 codec'inde
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopEntry {
    pub pid: u32,
    pub cpu: CpuTotal,
    pub ws_kb: u64,
    pub name: String,
}

/// Kayıtlı aralık boşluğunun nedeni (spek §5.4 Gap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GapReason {
    Paused = 0,
    Sleep = 1,
    Boot = 2,
    /// Önceki çalışma mühürsüz bitti (çökme / elektrik kesintisi).
    Crash = 3,
    /// state.bin ile segment uyuşmuyor (kuyruk kesilmiş olabilir).
    Integrity = 4,
}

impl GapReason {
    pub const ALL: [GapReason; 5] = [
        Self::Paused,
        Self::Sleep,
        Self::Boot,
        Self::Crash,
        Self::Integrity,
    ];
}

/// Event kaydı türü (spek §5.4, ADR-0028).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EventKind {
    AlarmRaise = 0,
    AlarmClear = 1,
    /// Servis olayı: `name` = olay kodu (`config_invalid`, `write_error`, …).
    System = 2,
}

impl EventKind {
    pub const ALL: [EventKind; 3] = [Self::AlarmRaise, Self::AlarmClear, Self::System];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AggKind {
    Min = 0,
    Avg = 1,
    Max = 2,
}

impl AggKind {
    pub const ALL: [AggKind; 3] = [Self::Min, Self::Avg, Self::Max];
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
