//! Canlı sekmesi 5 dk geçmişi (LM spek §9): sabit kapasiteli ring buffer + SVG path üretimi.
//! Init sonrası tahsis yok; path `String`'i çağıran tarafından yeniden kullanılır.

use std::fmt::Write;

/// Pencere = kapasite: deck 1 Hz push eder → 300 örnek = 5 dk.
pub const WINDOW_S: u64 = 300;
const CAP: usize = WINDOW_S as usize;
/// Ardışık iki örnek arası bundan büyükse çizgi kopar (küçültme/kopma boşluğu; mock 2 s'de bir).
const GAP_S: u64 = 5;
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Sample {
    pub ts_s: u64,
    /// Toplam CPU, 0..10000 (%·100).
    pub cpu: u16,
    /// RAM doluluk, 0..1.
    pub ram: f32,
    pub dr: u32,
    pub dw: u32,
    pub rx: u32,
    pub tx: u32,
}

pub struct History {
    buf: [Sample; CAP],
    /// Sonraki yazılacak indeks.
    head: usize,
    len: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            buf: [Sample::default(); CAP],
            head: 0,
            len: 0,
        }
    }
}

impl History {
    pub fn push(&mut self, s: Sample) {
        self.buf[self.head] = s;
        self.head = (self.head + 1) % CAP;
        self.len = (self.len + 1).min(CAP);
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Eskiden yeniye.
    pub fn iter(&self) -> impl Iterator<Item = &Sample> {
        let start = (self.head + CAP - self.len) % CAP;
        (0..self.len).map(move |i| &self.buf[(start + i) % CAP])
    }

    /// Penceredeki (son `WINDOW_S`) örnekler.
    fn recent(&self, now_s: u64) -> impl Iterator<Item = &Sample> {
        self.iter()
            .filter(move |s| now_s.saturating_sub(s.ts_s) <= WINDOW_S)
    }

    /// Penceredeki en büyük değer (oto-ölçek için).
    pub fn max(&self, now_s: u64, f: impl Fn(&Sample) -> u32) -> u32 {
        self.recent(now_s).map(f).max().unwrap_or(0)
    }

    /// SVG path, viewbox `WINDOW_S`×100: x = zaman (sağ kenar = şimdi), y = 100·(1 − v), v ∈ 0..1.
    /// Boşlukta yeni `M`; boş pencerede `out` boş kalır.
    pub fn path(&self, now_s: u64, f: impl Fn(&Sample) -> f32, out: &mut String) {
        out.clear();
        let mut prev: Option<u64> = None;
        for s in self.recent(now_s) {
            let x = WINDOW_S - now_s.saturating_sub(s.ts_s).min(WINDOW_S);
            let y = 100.0 * (1.0 - f(s).clamp(0.0, 1.0));
            let cmd = match prev {
                Some(p) if s.ts_s.saturating_sub(p) <= GAP_S => 'L',
                _ => 'M',
            };
            let _ = write!(out, "{cmd}{x} {y:.1}");
            prev = Some(s.ts_s);
        }
        // Tek noktalı alt yol çizilmez: yatay 1 birimlik çizgi olarak görünür kılınır.
        if !out.is_empty() && !out.contains('L') && out.matches('M').count() == 1 {
            let _ = write!(out, "h1");
        }
    }
}

/// Kova serisi → SVG path, viewbox `n`×100: x = kova indeksi, `None` → çizgi kopar.
/// Tek noktalı alt yol 1 birimlik yatay çizgi olur (görünsün).
pub fn series_path(vals: impl Iterator<Item = Option<f32>>, out: &mut String) {
    out.clear();
    let mut it = vals.enumerate().peekable();
    let mut in_seg = false;
    while let Some((i, v)) = it.next() {
        let Some(v) = v else {
            in_seg = false;
            continue;
        };
        let y = 100.0 * (1.0 - v.clamp(0.0, 1.0));
        let _ = write!(out, "{}{i} {y:.1}", if in_seg { 'L' } else { 'M' });
        let next_some = matches!(it.peek(), Some((_, Some(_))));
        if !in_seg && !next_some {
            let _ = write!(out, "h1");
        }
        in_seg = true;
    }
}

/// Disk/ağ ölçeği (KB/s): 1024·{1, 2, 5}·10^k, en az 1024 (= "1.0M"; boşta gürültü tepe gibi görünmesin).
/// 1024 tabanı `rate()` etiketini yuvarlak tutar: 2.0M, 5.0M, 10.0M, 1.0G…
pub fn nice_scale(max_kbps: u32) -> u32 {
    let mut base: u32 = 1024;
    loop {
        for m in [1, 2, 5] {
            let v = base.saturating_mul(m);
            if v >= max_kbps || v == u32::MAX {
                return v;
            }
        }
        base = base.saturating_mul(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ts_s: u64, cpu: u16) -> Sample {
        Sample {
            ts_s,
            cpu,
            ..Default::default()
        }
    }

    fn cpu(s: &Sample) -> f32 {
        s.cpu as f32 / 10_000.0
    }

    #[test]
    fn ring_wraps() {
        let mut h = History::default();
        for t in 0..305 {
            h.push(s(t, 0));
        }
        let ts: Vec<u64> = h.iter().map(|s| s.ts_s).collect();
        assert_eq!(ts.len(), 300);
        assert_eq!((ts[0], ts[299]), (5, 304));
        assert!(ts.windows(2).all(|w| w[0] < w[1]));
        h.clear();
        assert_eq!(h.iter().count(), 0);
    }

    #[test]
    fn path_gaps() {
        let mut h = History::default();
        let mut out = String::new();
        for t in [100, 102, 104] {
            h.push(s(t, 5_000));
        }
        h.path(104, cpu, &mut out);
        assert_eq!(out, "M296 50.0L298 50.0L300 50.0");
        h.push(s(114, 10_000)); // 10 s boşluk
        h.path(114, cpu, &mut out);
        assert_eq!(out.matches('M').count(), 2);
        assert!(out.ends_with("M300 0.0"));
    }

    #[test]
    fn path_window() {
        let mut h = History::default();
        let mut out = String::new();
        h.push(s(0, 0));
        h.push(s(400, 0));
        h.path(400, cpu, &mut out);
        assert_eq!(out, "M300 100.0h1"); // 400 s önceki örnek pencere dışı
        h.path(1000, cpu, &mut out);
        assert!(out.is_empty());
        assert_eq!(h.max(400, |s| s.ts_s as u32), 400);
    }

    #[test]
    fn series() {
        let mut out = String::new();
        series_path(
            [Some(0.5), Some(1.0), None, Some(0.0), None].into_iter(),
            &mut out,
        );
        assert_eq!(out, "M0 50.0L1 0.0M3 100.0h1");
        series_path([None, None].into_iter(), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn scale() {
        assert_eq!(nice_scale(0), 1024);
        assert_eq!(nice_scale(1500), 2048);
        assert_eq!(nice_scale(4999), 5120);
        assert_eq!(nice_scale(12_000), 20_480);
        assert_eq!(nice_scale(u32::MAX), u32::MAX);
    }
}
