//! Süreçler sekmesi (LM spek §9.4, §10): CPU delta hesabı, sıralama, korumalı süreçler.
//! Tarama Windows'ta `win::scan` (NtQuerySystemInformation, LM `topn.rs` ile aynı yöntem);
//! burada yalnız platformdan bağımsız mantık. Sekme görünürken 2 s'de bir çağrılır.

use std::collections::HashMap;

/// Tarama aralığı: yalnız Süreçler sekmesi açık ve pencere küçültülmemişken.
pub const SCAN_MS: u64 = 2000;

/// Ham tarama kaydı.
#[derive(Debug, Clone)]
pub struct RawProc {
    pub pid: u32,
    /// CreationTime (FILETIME, 100 ns); pid ile birlikte kimlik → PID yeniden kullanımı karışmaz.
    pub ctime: i64,
    /// Kullanıcı + çekirdek CPU zamanı, 100 ns.
    pub cpu_100ns: u64,
    /// Private working set, bayt.
    pub ws_bytes: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub pid: u32,
    pub ctime: i64,
    pub name: String,
    /// Sistem kapasitesinin 0.01 %'u (10000 = tüm çekirdekler dolu).
    pub cpu: u16,
    pub ws_mb: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Cpu,
    Ram,
}

/// CPU yumuşatma (EMA, α = 0,3 ≈ son 3 örnek): gösterilen değer ve sıralama bununla → liste zıplamaz.
pub fn ema(prev: Option<u16>, v: u16) -> u16 {
    prev.map_or(v, |p| (p as i32 + (v as i32 - p as i32) * 3 / 10) as u16)
}

/// Önceki taramanın CPU zamanları ve yumuşatılmış CPU; ölen süreçler her taramada düşer.
#[derive(Default)]
pub struct Tracker {
    prev: HashMap<(u32, i64), (u64, Option<u16>)>,
    prev_ms: Option<u64>,
}

impl Tracker {
    /// İlk taramada (ya da uzun aradan sonra `reset` ile) CPU 0 gösterilir; liste yine hemen dolar.
    pub fn update(
        &mut self,
        now_ms: u64,
        ncpu: u64,
        procs: Vec<RawProc>,
        sort: SortBy,
    ) -> Vec<Row> {
        let cap_100ns = self
            .prev_ms
            .map_or(0, |p| now_ms.saturating_sub(p) * 10_000 * ncpu.max(1));
        let mut next = HashMap::with_capacity(procs.len());
        let mut rows: Vec<Row> = procs
            .into_iter()
            .map(|p| {
                let key = (p.pid, p.ctime);
                // İlk ölçüm yumuşatılmaz (0'dan başlamasın); ilk taramada değer yok → None
                let cpu = match self.prev.get(&key) {
                    Some(&(old, smooth)) if cap_100ns > 0 => {
                        let raw = (p.cpu_100ns.saturating_sub(old).saturating_mul(10_000)
                            / cap_100ns)
                            .min(10_000);
                        Some(ema(smooth, raw as u16))
                    }
                    _ => None,
                };
                next.insert(key, (p.cpu_100ns, cpu));
                let cpu = cpu.unwrap_or(0);
                Row {
                    pid: p.pid,
                    ctime: p.ctime,
                    name: p.name,
                    cpu,
                    ws_mb: (p.ws_bytes >> 20) as u32,
                }
            })
            .collect();
        self.prev = next;
        self.prev_ms = Some(now_ms);
        sort_rows(&mut rows, sort);
        rows
    }

    /// Sekme kapanınca: bir sonraki açılışta eski delta ile yanlış CPU gösterilmesin.
    pub fn reset(&mut self) {
        self.prev.clear();
        self.prev_ms = None;
    }
}

pub fn sort_rows(rows: &mut [Row], sort: SortBy) {
    match sort {
        SortBy::Cpu => rows.sort_by(|a, b| b.cpu.cmp(&a.cpu).then(b.ws_mb.cmp(&a.ws_mb))),
        SortBy::Ram => rows.sort_by(|a, b| b.ws_mb.cmp(&a.ws_mb).then(b.cpu.cmp(&a.cpu))),
    }
}

/// Arama: `f` küçük harf; boş → hepsi. Ad içerir ya da PID içerir.
pub fn matches(r: &Row, f: &str) -> bool {
    f.is_empty() || r.name.to_lowercase().contains(f) || r.pid.to_string().contains(f)
}

/// Sonlandırılamaz (spek §10 yasak liste): sistemi düşürür ya da oturumu kapatır.
pub fn protected(pid: u32, name: &str) -> bool {
    const DENY: [&str; 8] = [
        "csrss.exe",
        "wininit.exe",
        "smss.exe",
        "lsass.exe",
        "services.exe",
        "winlogon.exe",
        "system",
        "registry",
    ];
    pid == 0
        || pid == 4
        || pid == std::process::id()
        || DENY.iter().any(|d| name.eq_ignore_ascii_case(d))
}

/// "12.3%"; < %0,05 → boş (liste sakin okunur; EMA kuyruğu "0.0%" göstermesin).
pub fn fmt_cpu(cpu: u16) -> String {
    if cpu < 5 {
        String::new()
    } else {
        format!("{:.1}%", cpu as f32 / 100.0)
    }
}

/// Yükseltilmiş kopyanın çıkış kodları (`--kill`), sonuç metni ana kopyada.
pub mod exit {
    pub const OK: i32 = 0;
    pub const FAILED: i32 = 1;
    /// Süreç yok ya da PID başka sürece geçmiş (CreationTime uyuşmuyor).
    pub const GONE: i32 = 2;
    pub const PROTECTED: i32 = 3;
    pub const DENIED: i32 = 5;
    /// UAC reddedildi / yükseltilmiş kopya başlatılamadı (ana kopyada üretilir).
    pub const CANCELLED: i32 = 1223;

    pub fn text(code: i32, name: &str) -> String {
        match code {
            OK => format!("{name} sonlandırıldı"),
            GONE => format!("{name}: süreç artık yok (ya da PID başka sürece geçti)"),
            PROTECTED => format!("{name}: korumalı süreç, sonlandırılamaz"),
            DENIED => format!("{name}: erişim reddedildi"),
            CANCELLED => format!("{name}: yönetici onayı verilmedi"),
            c => format!("{name}: sonlandırılamadı (kod {c})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ctime: i64, cpu_100ns: u64, ws_mb: u64) -> RawProc {
        RawProc {
            pid,
            ctime,
            cpu_100ns,
            ws_bytes: ws_mb << 20,
            name: format!("p{pid}.exe"),
        }
    }

    #[test]
    fn cpu_delta_and_reuse() {
        let mut t = Tracker::default();
        // İlk tarama: CPU 0, RAM'e göre değil CPU'ya göre (eşitlikte RAM) sıralı
        let r = t.update(
            1_000,
            4,
            vec![p(10, 1, 0, 50), p(11, 1, 0, 80)],
            SortBy::Cpu,
        );
        assert_eq!(
            r.iter().map(|r| (r.pid, r.cpu)).collect::<Vec<_>>(),
            [(11, 0), (10, 0)]
        );
        // 2 s, 4 çekirdek → kapasite 80 000 000 × 100 ns; pid 10: 20 000 000 → %25
        // pid 11 yeniden kullanılmış (ctime farklı) → CPU 0
        let r = t.update(
            3_000,
            4,
            vec![p(10, 1, 20_000_000, 50), p(11, 2, 70_000_000, 80)],
            SortBy::Cpu,
        );
        assert_eq!((r[0].pid, r[0].cpu, r[0].ws_mb), (10, 2_500, 50));
        assert_eq!((r[1].pid, r[1].cpu), (11, 0));
        let r = t.update(
            5_000,
            4,
            vec![p(10, 1, 20_000_000, 50), p(11, 2, 70_000_000, 80)],
            SortBy::Ram,
        );
        assert_eq!(r[0].pid, 11);
        t.reset();
        let r = t.update(9_000, 4, vec![p(10, 1, 99_000_000, 50)], SortBy::Cpu);
        assert_eq!(r[0].cpu, 0);
    }

    #[test]
    fn ema_smooths() {
        assert_eq!(ema(None, 500), 500);
        assert_eq!(ema(Some(1000), 0), 700);
        assert_eq!(ema(Some(0), 1000), 300);
        assert_eq!(ema(Some(10_000), 10_000), 10_000);
    }

    #[test]
    fn cpu_clamped() {
        let mut t = Tracker::default();
        t.update(0, 1, vec![p(1, 1, 0, 0)], SortBy::Cpu);
        let r = t.update(1_000, 1, vec![p(1, 1, u64::MAX, 0)], SortBy::Cpu);
        assert_eq!(r[0].cpu, 10_000);
    }

    #[test]
    fn search() {
        let r = Row {
            pid: 1234,
            ctime: 0,
            name: "Notepad.exe".into(),
            cpu: 0,
            ws_mb: 0,
        };
        assert!(matches(&r, ""));
        assert!(matches(&r, "note"));
        assert!(matches(&r, "23"));
        assert!(!matches(&r, "chrome"));
    }

    #[test]
    fn deny_list() {
        assert!(protected(4, "System"));
        assert!(protected(0, "Idle"));
        assert!(protected(700, "LSASS.EXE"));
        assert!(protected(std::process::id(), "x.exe"));
        assert!(!protected(1234, "notepad.exe"));
        assert!(exit::text(exit::GONE, "a").contains("artık yok"));
        assert_eq!(fmt_cpu(0), "");
        assert_eq!(fmt_cpu(4), "");
        assert_eq!(fmt_cpu(5), "0.1%");
        assert_eq!(fmt_cpu(1234), "12.3%");
    }
}
