//! Sensör geçmişi (ADR-0035): dakikalık ortalama + max, 7 günlük sabit boyutlu halka dosyası.
//! Tray sensör iş parçacığı yazar (`Minute`), `lowmon-ui` Geçmiş sekmesi okur (`read`).
//! Kayıt yeri = `ts_min % SLOTS`; başlık yok. Okuyucu `ts_min`'e bakar: eski tur / boş yuva elenir.
use crate::Reading;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 7 gün × 1440 dk.
pub const SLOTS: u64 = 7 * 1440;
const REC: usize = 20;
const NONE_C: i16 = i16::MIN;
const NONE_P: u8 = u8::MAX;

/// `%LOCALAPPDATA%\LowMonitor\sensors.bin`
pub fn path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("LowMonitor")
        .join("sensors.bin")
}

/// Bir dakika: sıcaklıklar 0,1 °C, GPU %; `None` = o dakika hiç okunamadı.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rec {
    pub ts_min: u32,
    pub cpu: Agg,
    pub gpu_c: Agg,
    pub disk: Agg,
    pub gpu_pct: Agg,
}

/// Ortalama + max (birim alana göre). `None` = veri yok.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Agg {
    pub avg: Option<f32>,
    pub max: Option<f32>,
}

impl Rec {
    fn encode(&self) -> [u8; REC] {
        let c = |v: Option<f32>| {
            v.map_or(NONE_C, |v| {
                (v * 10.0).round().clamp(-32767.0, 32767.0) as i16
            })
        };
        let p = |v: Option<f32>| v.map_or(NONE_P, |v| v.round().clamp(0.0, 100.0) as u8);
        let mut b = [0u8; REC];
        b[0..4].copy_from_slice(&self.ts_min.to_le_bytes());
        for (i, a) in [self.cpu, self.gpu_c, self.disk].iter().enumerate() {
            b[4 + i * 4..6 + i * 4].copy_from_slice(&c(a.avg).to_le_bytes());
            b[6 + i * 4..8 + i * 4].copy_from_slice(&c(a.max).to_le_bytes());
        }
        b[16] = p(self.gpu_pct.avg);
        b[17] = p(self.gpu_pct.max);
        b
    }

    fn decode(b: &[u8]) -> Self {
        let c = |o: usize| match i16::from_le_bytes([b[o], b[o + 1]]) {
            NONE_C => None,
            v => Some(v as f32 / 10.0),
        };
        let p = |o: usize| (b[o] != NONE_P).then_some(b[o] as f32);
        let agg = |o: usize| Agg {
            avg: c(o),
            max: c(o + 2),
        };
        Rec {
            ts_min: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            cpu: agg(4),
            gpu_c: agg(8),
            disk: agg(12),
            gpu_pct: Agg {
                avg: p(16),
                max: p(17),
            },
        }
    }
}

/// Bir alanın dakika içi toplamı.
#[derive(Clone, Copy, Default)]
struct Acc {
    sum: f32,
    n: u32,
    max: Option<f32>,
}

impl Acc {
    fn add(&mut self, v: Option<i32>) {
        if let Some(v) = v {
            let v = v as f32;
            self.sum += v;
            self.n += 1;
            self.max = Some(self.max.map_or(v, |m| m.max(v)));
        }
    }
    fn agg(&self) -> Agg {
        Agg {
            avg: (self.n > 0).then(|| self.sum / self.n as f32),
            max: self.max,
        }
    }
}

/// Okumaları dakikaya toplar; dakika değişince biten dakikayı döner.
#[derive(Default)]
pub struct Minute {
    ts_min: u32,
    acc: [Acc; 4],
}

impl Minute {
    /// `now_s` Unix saniye. Dakika değiştiyse önceki dakikanın kaydı (en az bir okuma varsa).
    pub fn add(&mut self, now_s: u64, r: &Reading) -> Option<Rec> {
        let m = (now_s / 60) as u32;
        let done = (m != self.ts_min).then(|| self.take()).flatten();
        if m != self.ts_min {
            self.ts_min = m;
            self.acc = Default::default();
        }
        for (a, v) in
            self.acc
                .iter_mut()
                .zip([r.cpu_c, r.gpu_c, r.disk_c, r.gpu_pct.map(|p| p as i32)])
        {
            a.add(v);
        }
        done
    }

    fn take(&self) -> Option<Rec> {
        if self.ts_min == 0 || self.acc.iter().all(|a| a.n == 0) {
            return None;
        }
        let [cpu, gpu_c, disk, gpu_pct] = self.acc.map(|a| a.agg());
        Some(Rec {
            ts_min: self.ts_min,
            cpu,
            gpu_c,
            disk,
            gpu_pct,
        })
    }
}

/// Kaydı yuvasına yazar; dosya yoksa oluşturur (seyrek değil, `set_len` ile tam boy).
pub fn write(path: &Path, r: &Rec) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let len = SLOTS * REC as u64;
    if f.metadata()?.len() != len {
        f.set_len(len)?;
    }
    f.seek(SeekFrom::Start((r.ts_min as u64 % SLOTS) * REC as u64))?;
    f.write_all(&r.encode())
}

/// `[from_min, to_min)` aralığındaki kayıtlar, zamana göre sıralı. Dosya yoksa boş.
pub fn read(path: &Path, from_min: u32, to_min: u32) -> std::io::Result<Vec<Rec>> {
    let mut buf = Vec::new();
    match File::open(path) {
        Ok(mut f) => f.read_to_end(&mut buf)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut v: Vec<Rec> = buf
        .as_chunks::<REC>()
        .0
        .iter()
        .map(|b| Rec::decode(b))
        .filter(|r| r.ts_min != 0 && (from_min..to_min).contains(&r.ts_min))
        .collect();
    v.sort_by_key(|r| r.ts_min);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(cpu: i32, gpu_pct: Option<u32>) -> Reading {
        Reading {
            gpu_pct,
            gpu_c: None,
            cpu_c: Some(cpu),
            disk_c: Some(40),
        }
    }

    #[test]
    fn minute_aggregates_and_roundtrips() {
        let mut m = Minute::default();
        assert_eq!(m.add(60_000, &rd(60, Some(10))), None); // ilk dakika
        assert_eq!(m.add(60_030, &rd(70, Some(30))), None);
        let r = m.add(60_060, &rd(50, None)).expect("dakika bitti");
        assert_eq!(r.ts_min, 1000);
        assert_eq!(
            r.cpu,
            Agg {
                avg: Some(65.0),
                max: Some(70.0)
            }
        );
        assert_eq!(
            r.gpu_pct,
            Agg {
                avg: Some(20.0),
                max: Some(30.0)
            }
        );
        assert_eq!(r.gpu_c, Agg::default());
        assert_eq!(Rec::decode(&r.encode()), r);
    }

    #[test]
    fn ring_file_overwrites_and_filters() {
        let p = std::env::temp_dir().join(format!("lowmon-sensors-log-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert!(read(&p, 0, u32::MAX).unwrap().is_empty());
        let rec = |ts_min| Rec {
            ts_min,
            cpu: Agg {
                avg: Some(55.5),
                max: Some(61.0),
            },
            ..Default::default()
        };
        write(&p, &rec(100)).unwrap();
        write(&p, &rec(101)).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), SLOTS * REC as u64);
        // Aynı yuva bir tur sonra: eski kayıt gider
        write(&p, &rec(100 + SLOTS as u32)).unwrap();
        let v = read(&p, 0, u32::MAX).unwrap();
        assert_eq!(
            v.iter().map(|r| r.ts_min).collect::<Vec<_>>(),
            [101, 100 + SLOTS as u32]
        );
        assert_eq!(v[0].cpu.avg, Some(55.5));
        assert_eq!(read(&p, 101, 102).unwrap().len(), 1);
        let _ = std::fs::remove_file(&p);
    }
}
