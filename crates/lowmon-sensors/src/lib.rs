//! `lowmon-sensors`: servisin ölçmediği sensörler, tray içinde okunur (ADR-0033).
//! Üç bağımsız kaynak; biri yoksa / hata verirse o alan `None`, diğerleri çalışır:
//! - GPU % ve °C: D3DKMT (Görev Yöneticisi'nin yolu; üretici DLL'i yok). Hibrit sistemde ayrık GPU,
//!   en yoğun motorun kullanımı. NVML denendi: aynı değer, ama sürece +19,8 MiB (ölçüm 2026-11-17)
//! - disk °C: `IOCTL_STORAGE_QUERY_PROPERTY` + `StorageDeviceTemperatureProperty`, en sıcak disk
//! - CPU °C: ACPI termal bölge, PDH sayacı. Spek §1 "PDH yok" kuralına ADR-0033 istisnası:
//!   yalnız bu sayaç, yalnız tray. Çip sıcaklığı değil, kaba olabilir.
//!
//! Yükseltme gerekmez. Yalnız okunurken maliyet var: çağıran gösterge kapalıyken `read` çağırmaz.
#[cfg(windows)]
mod win;

/// Bir okuma; `None` = kaynak yok ya da okunamadı.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub gpu_pct: Option<u32>,
    pub gpu_c: Option<i32>,
    pub cpu_c: Option<i32>,
    pub disk_c: Option<i32>,
}

/// Açılacak kaynaklar (ölçümde ve ayarda tek tek kapatılabilsin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sources {
    pub gpu: bool,
    pub cpu_temp: bool,
    pub disk_temp: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            gpu: true,
            cpu_temp: true,
            disk_temp: true,
        }
    }
}

#[cfg(windows)]
pub use win::Sensors;

/// `D3DKMT_ADAPTERTYPE` bitleri (d3dkmthk.h).
pub const ADAPTER_SOFTWARE: u32 = 1 << 2;
const ADAPTER_HYBRID_DISCRETE: u32 = 1 << 4;

/// Gösterilecek adaptör: ayrık (hibrit) GPU, yoksa ilk donanım adaptörü; yazılım aygıtları atlanır.
pub fn pick_adapter(types: &[u32]) -> Option<usize> {
    let hw = |t: &&u32| **t & ADAPTER_SOFTWARE == 0;
    types
        .iter()
        .position(|t| hw(&t) && t & ADAPTER_HYBRID_DISCRETE != 0)
        .or_else(|| types.iter().position(|t| hw(&t)))
}

/// En yoğun motorun çalışma süresi farkı / geçen süre (ikisi de 100 ns) → 0..100.
pub fn usage_pct(running_delta: i64, elapsed: f64) -> u32 {
    if elapsed <= 0.0 {
        return 0;
    }
    (100.0 * running_delta.max(0) as f64 / elapsed)
        .round()
        .min(100.0) as u32
}

/// PDH termal bölge değeri Kelvin; makul aralık dışı (0, bozuk bölge) → `None`.
pub fn kelvin_to_c(k: f64) -> Option<i32> {
    let c = k - 273.15;
    (1.0..150.0).contains(&c).then(|| c.round() as i32)
}

/// `STORAGE_TEMPERATURE_DATA_DESCRIPTOR`: 24 bayt başlık (`InfoCount` u16 @12), ardından 16 baytlık
/// `STORAGE_TEMPERATURE_INFO` girdileri (`Temperature` i16 °C @2). En sıcak sensör.
pub fn parse_disk_temp(buf: &[u8]) -> Option<i32> {
    let count = u16::from_le_bytes(buf.get(12..14)?.try_into().ok()?) as usize;
    (0..count)
        .filter_map(|i| {
            let at = 24 + 16 * i + 2;
            Some(i16::from_le_bytes(buf.get(at..at + 2)?.try_into().ok()?) as i32)
        })
        // 0 ve altı: sensör bildirilmiyor (bazı sürücüler boş girdi döndürür)
        .filter(|&t| t > 0)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_choice() {
        let (render, post, sw, disc, integ) = (1, 1 << 3, ADAPTER_SOFTWARE, 1 << 4, 1 << 5);
        // bu PC: AMD tümleşik, NVIDIA ayrık, Basic Render
        assert_eq!(
            pick_adapter(&[render | post | integ, render | disc, render | sw]),
            Some(1)
        );
        // tek GPU'lu masaüstü
        assert_eq!(pick_adapter(&[render | sw, render | post]), Some(1));
        assert_eq!(pick_adapter(&[render | sw]), None);
        assert_eq!(pick_adapter(&[]), None);
    }

    #[test]
    fn usage() {
        assert_eq!(usage_pct(5_000_000, 20_000_000.0), 25);
        assert_eq!(usage_pct(30_000_000, 20_000_000.0), 100);
        assert_eq!(usage_pct(-5, 20_000_000.0), 0);
        assert_eq!(usage_pct(5, 0.0), 0);
    }

    #[test]
    fn kelvin() {
        assert_eq!(kelvin_to_c(348.0), Some(75));
        assert_eq!(kelvin_to_c(338.05), Some(65));
        assert_eq!(kelvin_to_c(0.0), None);
        assert_eq!(kelvin_to_c(273.15), None);
        assert_eq!(kelvin_to_c(5000.0), None);
    }

    #[test]
    fn disk_descriptor() {
        // Gerçek yanıt düzeni (Micron NVMe, 2026-11-17): 2 sensör, 35 °C ve 41 °C
        let mut b = vec![0u8; 56];
        b[12] = 2;
        b[26..28].copy_from_slice(&35i16.to_le_bytes());
        b[42..44].copy_from_slice(&41i16.to_le_bytes());
        assert_eq!(parse_disk_temp(&b), Some(41));
        // ikinci girdi tampon dışında → yalnız ilki
        assert_eq!(parse_disk_temp(&b[..40]), Some(35));
        b[12] = 0;
        assert_eq!(parse_disk_temp(&b), None);
        assert_eq!(parse_disk_temp(&[0; 8]), None);
        // boş girdi (0 °C) sayılmaz
        let mut z = vec![0u8; 40];
        z[12] = 1;
        assert_eq!(parse_disk_temp(&z), None);
    }
}
