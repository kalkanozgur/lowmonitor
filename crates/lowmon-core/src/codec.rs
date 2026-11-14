//! Blok içi kayıt kodlaması (spek §5.4), şifre çözülmüş düz metin.
//!
//! - Her kayıt: `tip u8 | payload`; payload varint'lerle kendini sınırlar.
//! - Sample: tüm alanlar düzleştirilir, önceki örneğin aynı indeksine göre zigzag varint delta.
//!   Delta durumu her blokta sıfırlanır → blok tek başına çözülebilir.
//! - ProcDict: segment kapsamlı isim sözlüğü; yeni isim ilk kullanıldığı TopN'den hemen önce yazılır.
//! - Seal: STOP/PRESHUTDOWN ve gün dönümünde son blok; ardından (yeniden başlatmada) yeni bloklar gelebilir.
// shortcut: Event (0x03) M5'te; o zamana kadar bütünlük sorunları Gap(Integrity) olarak yazılır
use crate::metrics::{DiskIo, NetIo, Sample, TopEntry};
use std::collections::HashMap;

pub const REC_SAMPLE: u8 = 0x01;
pub const REC_TOPN: u8 = 0x02;
pub const REC_PROCDICT: u8 = 0x04;
pub const REC_GAP: u8 = 0x05;
/// Downsample (spek §5.5): dakika başına min/avg/max örnek.
pub const REC_AGG: u8 = 0x06;
pub const REC_SEAL: u8 = 0xFF;

/// Kayıt şeması değişince artar; Header.schema_hash'e yazılır.
pub const SCHEMA: u32 = 1;

pub use crate::metrics::{AggKind, GapReason};

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Sample(Sample),
    /// `sample.ts_unix_ms` = dakika başı.
    Agg {
        kind: AggKind,
        sample: Sample,
    },
    /// Zamanı, bloktaki bir önceki Sample'ın ts'idir.
    Top(Vec<TopEntry>),
    Gap {
        from_ms: u64,
        to_ms: u64,
        reason: GapReason,
    },
    /// `prev`: mühürden önceki zincir başı (son tag; blok yoksa başlık).
    Seal {
        block_count: u64,
        sample_count: u64,
        prev: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    Overflow,
    BadRecord(u8),
    BadUtf8,
    UnknownName(u32),
}

type Result<T> = std::result::Result<T, DecodeError>;

fn put_var(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push(v as u8 | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// Wrapping fark → zigzag: küçük ± değişimler küçük varint olur.
fn zig(delta: u64) -> u64 {
    let d = delta as i64;
    ((d << 1) ^ (d >> 63)) as u64
}

fn unzig(z: u64) -> u64 {
    ((z >> 1) as i64 ^ -((z & 1) as i64)) as u64
}

fn narrow<T: TryFrom<u64>>(v: u64) -> Result<T> {
    T::try_from(v).map_err(|_| DecodeError::Overflow)
}

fn flatten(s: &Sample, out: &mut Vec<u64>) {
    out.clear();
    out.extend([s.ts_unix_ms, s.cpu_total as u64, s.cpu_core.len() as u64]);
    out.extend(s.cpu_core.iter().map(|&c| c as u64));
    out.extend([s.ram_used_kb, s.commit_kb, s.cache_kb, s.disk.len() as u64]);
    for d in &s.disk {
        out.extend([d.drive as u64, d.r_bps, d.w_bps]);
    }
    out.push(s.net.len() as u64);
    for n in &s.net {
        out.extend([n.if_index as u64, n.rx_bps, n.tx_bps]);
    }
}

/// Yazıcı tarafı isim sözlüğü; segment başında sıfırlanır.
#[derive(Default)]
pub struct Dict {
    ids: HashMap<String, u32>,
}

impl Dict {
    /// Yeniden açılan segmentte çözücünün topladığı isimlerden sözlüğü kurar.
    pub fn from_names(names: Vec<String>) -> Self {
        Self {
            ids: names.into_iter().zip(0..).collect(),
        }
    }
}

#[derive(Default)]
pub struct BlockEncoder {
    buf: Vec<u8>,
    prev: Vec<u64>,
    cur: Vec<u64>,
}

impl BlockEncoder {
    pub fn sample(&mut self, s: &Sample) {
        self.buf.push(REC_SAMPLE);
        self.sample_body(s);
    }

    pub fn agg(&mut self, kind: AggKind, s: &Sample) {
        self.buf.push(REC_AGG);
        self.buf.push(kind as u8);
        self.sample_body(s);
    }

    /// Sample ve Agg aynı delta durumunu paylaşır.
    fn sample_body(&mut self, s: &Sample) {
        flatten(s, &mut self.cur);
        for (i, &v) in self.cur.iter().enumerate() {
            let base = self.prev.get(i).copied().unwrap_or(0);
            put_var(&mut self.buf, zig(v.wrapping_sub(base)));
        }
        std::mem::swap(&mut self.prev, &mut self.cur);
    }

    pub fn top(&mut self, top: &[TopEntry], dict: &mut Dict) {
        let mut ids = Vec::with_capacity(top.len());
        for e in top {
            let next = dict.ids.len() as u32;
            let id = *dict.ids.entry(e.name.clone()).or_insert_with(|| {
                self.buf.push(REC_PROCDICT);
                put_var(&mut self.buf, next as u64);
                put_var(&mut self.buf, e.name.len() as u64);
                self.buf.extend_from_slice(e.name.as_bytes());
                next
            });
            ids.push(id);
        }
        self.buf.push(REC_TOPN);
        put_var(&mut self.buf, top.len() as u64);
        for (e, id) in top.iter().zip(ids) {
            for v in [id as u64, e.pid as u64, e.cpu as u64, e.ws_kb] {
                put_var(&mut self.buf, v);
            }
        }
    }

    pub fn gap(&mut self, from_ms: u64, to_ms: u64, reason: GapReason) {
        self.buf.push(REC_GAP);
        put_var(&mut self.buf, from_ms);
        put_var(&mut self.buf, to_ms);
        self.buf.push(reason as u8);
    }

    pub fn seal(&mut self, block_count: u64, sample_count: u64, prev: &[u8]) {
        self.buf.push(REC_SEAL);
        put_var(&mut self.buf, block_count);
        put_var(&mut self.buf, sample_count);
        put_var(&mut self.buf, prev.len() as u64);
        self.buf.extend_from_slice(prev);
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Bloğu verir ve delta durumunu sıfırlar (sonraki blok bağımsız).
    pub fn take(&mut self) -> Vec<u8> {
        self.prev.clear();
        std::mem::take(&mut self.buf)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.pos).ok_or(DecodeError::Truncated)?;
        self.pos += 1;
        Ok(v)
    }

    fn var(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(DecodeError::Overflow)
    }

    /// Eleman sayısı; her eleman ≥ 1 bayt olduğundan kalan baytı aşamaz (dev ayırma yok).
    fn count(&mut self) -> Result<usize> {
        let n = self.var()?;
        if n > (self.b.len() - self.pos) as u64 {
            return Err(DecodeError::Truncated);
        }
        Ok(n as usize)
    }

    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let s = self
            .b
            .get(self.pos..self.pos + n)
            .ok_or(DecodeError::Truncated)?;
        self.pos += n;
        Ok(s)
    }
}

/// Bir bloğun kayıtlarını sırayla verir. `names` segment boyunca bloklar arasında paylaşılır.
pub struct BlockDecoder<'a> {
    r: Reader<'a>,
    prev: Vec<u64>,
    cur: Vec<u64>,
    names: &'a mut Vec<String>,
}

impl<'a> BlockDecoder<'a> {
    pub fn new(block: &'a [u8], names: &'a mut Vec<String>) -> Self {
        Self {
            r: Reader { b: block, pos: 0 },
            prev: Vec::new(),
            cur: Vec::new(),
            names,
        }
    }

    fn field(&mut self) -> Result<u64> {
        let base = self.prev.get(self.cur.len()).copied().unwrap_or(0);
        let v = base.wrapping_add(unzig(self.r.var()?));
        self.cur.push(v);
        Ok(v)
    }

    fn sample(&mut self) -> Result<Sample> {
        self.cur.clear();
        let mut s = Sample {
            ts_unix_ms: self.field()?,
            cpu_total: narrow(self.field()?)?,
            ..Default::default()
        };
        let n = self.count_field()?;
        for _ in 0..n {
            s.cpu_core.push(narrow(self.field()?)?);
        }
        s.ram_used_kb = self.field()?;
        s.commit_kb = self.field()?;
        s.cache_kb = self.field()?;
        for _ in 0..self.count_field()? {
            s.disk.push(DiskIo {
                drive: narrow(self.field()?)?,
                r_bps: self.field()?,
                w_bps: self.field()?,
            });
        }
        for _ in 0..self.count_field()? {
            s.net.push(NetIo {
                if_index: narrow(self.field()?)?,
                rx_bps: self.field()?,
                tx_bps: self.field()?,
            });
        }
        std::mem::swap(&mut self.prev, &mut self.cur);
        Ok(s)
    }

    fn count_field(&mut self) -> Result<u64> {
        let n = self.field()?;
        if n > (self.r.b.len() - self.r.pos) as u64 {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }

    fn top(&mut self) -> Result<Vec<TopEntry>> {
        let n = self.r.count()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let id: u32 = narrow(self.r.var()?)?;
            let name = self
                .names
                .get(id as usize)
                .ok_or(DecodeError::UnknownName(id))?
                .clone();
            out.push(TopEntry {
                pid: narrow(self.r.var()?)?,
                cpu: narrow(self.r.var()?)?,
                ws_kb: self.r.var()?,
                name,
            });
        }
        Ok(out)
    }

    fn proc_dict(&mut self) -> Result<()> {
        let id = self.r.var()?;
        // Sözlük sıralı büyür; atlama veya tekrar = bozuk veri
        if id != self.names.len() as u64 {
            return Err(DecodeError::BadRecord(REC_PROCDICT));
        }
        let len = self.r.count()?;
        let name = std::str::from_utf8(self.r.bytes(len)?).map_err(|_| DecodeError::BadUtf8)?;
        self.names.push(name.to_owned());
        Ok(())
    }

    fn record(&mut self) -> Result<Option<Record>> {
        loop {
            if self.r.pos == self.r.b.len() {
                return Ok(None);
            }
            match self.r.u8()? {
                REC_SAMPLE => return self.sample().map(|s| Some(Record::Sample(s))),
                REC_AGG => {
                    let kind = *AggKind::ALL
                        .get(self.r.u8()? as usize)
                        .ok_or(DecodeError::BadRecord(REC_AGG))?;
                    return self
                        .sample()
                        .map(|sample| Some(Record::Agg { kind, sample }));
                }
                REC_TOPN => return self.top().map(|t| Some(Record::Top(t))),
                REC_PROCDICT => self.proc_dict()?,
                REC_GAP => {
                    let from_ms = self.r.var()?;
                    let to_ms = self.r.var()?;
                    let reason = *GapReason::ALL
                        .get(self.r.u8()? as usize)
                        .ok_or(DecodeError::BadRecord(REC_GAP))?;
                    return Ok(Some(Record::Gap {
                        from_ms,
                        to_ms,
                        reason,
                    }));
                }
                REC_SEAL => {
                    let block_count = self.r.var()?;
                    let sample_count = self.r.var()?;
                    let len = self.r.count()?;
                    let prev = self.r.bytes(len)?.to_vec();
                    return Ok(Some(Record::Seal {
                        block_count,
                        sample_count,
                        prev,
                    }));
                }
                t => return Err(DecodeError::BadRecord(t)),
            }
        }
    }
}

impl Iterator for BlockDecoder<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        let r = self.record().transpose();
        if matches!(r, Some(Err(_))) {
            self.r.pos = self.r.b.len(); // hatadan sonra dur
        }
        r
    }
}

/// Dakikalık özet: düzeni (çekirdek sayısı, disk/arayüz id'leri) ilk örnekle aynı olan örneklerden
/// alan bazında min / avg / max. Boş girdi → None. ts = `minute_ts`.
pub fn aggregate(samples: &[Sample], minute_ts: u64) -> Option<[Sample; 3]> {
    let first = samples.first()?;
    let same_shape = |s: &&Sample| {
        s.cpu_core.len() == first.cpu_core.len()
            && s.disk
                .iter()
                .map(|d| d.drive)
                .eq(first.disk.iter().map(|d| d.drive))
            && s.net
                .iter()
                .map(|n| n.if_index)
                .eq(first.net.iter().map(|n| n.if_index))
    };
    let rows: Vec<Vec<u64>> = samples
        .iter()
        .filter(same_shape)
        .map(|s| {
            let mut v = Vec::new();
            flatten(s, &mut v);
            v
        })
        .collect();
    let n = rows[0].len();
    let (mut mn, mut mx, mut sum) = (rows[0].clone(), rows[0].clone(), vec![0u128; n]);
    for r in &rows {
        for i in 0..n {
            mn[i] = mn[i].min(r[i]);
            mx[i] = mx[i].max(r[i]);
            sum[i] += r[i] as u128;
        }
    }
    let avg: Vec<u64> = sum
        .iter()
        .map(|&x| (x / rows.len() as u128) as u64)
        .collect();
    // Sayaç / id alanları özetlenmez: düzen ilk örnekten kopyalanır, yalnız ölçüm alanları yazılır.
    let rebuild = |vals: &[u64]| {
        let mut out = first.clone();
        let mut it = vals.iter().copied();
        let mut next = || it.next().unwrap_or(0);
        out.ts_unix_ms = minute_ts;
        next();
        out.cpu_total = next() as u16;
        next();
        for c in &mut out.cpu_core {
            *c = next() as u8;
        }
        out.ram_used_kb = next();
        out.commit_kb = next();
        out.cache_kb = next();
        next();
        for d in &mut out.disk {
            next();
            d.r_bps = next();
            d.w_bps = next();
        }
        next();
        for x in &mut out.net {
            next();
            x.rx_bps = next();
            x.tx_bps = next();
        }
        out
    };
    Some([rebuild(&mn), rebuild(&avg), rebuild(&mx)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample() -> impl Strategy<Value = Sample> {
        (
            any::<u64>(),
            any::<u16>(),
            prop::collection::vec(any::<u8>(), 0..64),
            (any::<u64>(), any::<u64>(), any::<u64>()),
            prop::collection::vec((any::<u32>(), any::<u64>(), any::<u64>()), 0..4),
            prop::collection::vec((any::<u32>(), any::<u64>(), any::<u64>()), 0..4),
        )
            .prop_map(|(ts, cpu, cores, (ram, commit, cache), disk, net)| Sample {
                ts_unix_ms: ts,
                cpu_total: cpu,
                cpu_core: cores,
                ram_used_kb: ram,
                commit_kb: commit,
                cache_kb: cache,
                disk: disk
                    .into_iter()
                    .map(|(drive, r_bps, w_bps)| DiskIo {
                        drive,
                        r_bps,
                        w_bps,
                    })
                    .collect(),
                net: net
                    .into_iter()
                    .map(|(if_index, rx_bps, tx_bps)| NetIo {
                        if_index,
                        rx_bps,
                        tx_bps,
                    })
                    .collect(),
            })
    }

    fn record() -> impl Strategy<Value = Record> {
        let top = prop::collection::vec(
            ("[a-z]{1,6}\\.exe", any::<u32>(), any::<u16>(), any::<u64>()).prop_map(
                |(name, pid, cpu, ws_kb)| TopEntry {
                    pid,
                    cpu,
                    ws_kb,
                    name,
                },
            ),
            0..6,
        );
        let gap =
            (any::<u64>(), any::<u64>(), 0usize..5).prop_map(|(from_ms, to_ms, r)| Record::Gap {
                from_ms,
                to_ms,
                reason: GapReason::ALL[r],
            });
        let seal = (
            any::<u64>(),
            any::<u64>(),
            prop::collection::vec(any::<u8>(), 0..64),
        )
            .prop_map(|(block_count, sample_count, prev)| Record::Seal {
                block_count,
                sample_count,
                prev,
            });
        let agg = (0usize..3, sample()).prop_map(|(k, sample)| Record::Agg {
            kind: AggKind::ALL[k],
            sample,
        });
        prop_oneof![
            sample().prop_map(Record::Sample),
            agg,
            top.prop_map(Record::Top),
            gap,
            seal
        ]
    }

    fn encode(enc: &mut BlockEncoder, dict: &mut Dict, r: &Record) {
        match r {
            Record::Sample(s) => enc.sample(s),
            Record::Agg { kind, sample } => enc.agg(*kind, sample),
            Record::Top(t) => enc.top(t, dict),
            Record::Gap {
                from_ms,
                to_ms,
                reason,
            } => enc.gap(*from_ms, *to_ms, *reason),
            Record::Seal {
                block_count,
                sample_count,
                prev,
            } => enc.seal(*block_count, *sample_count, prev),
        }
    }

    proptest! {
        /// İki blok, paylaşılan segment sözlüğü: encode → decode eşitliği.
        #[test]
        fn round_trip(a in prop::collection::vec(record(), 0..20), b in prop::collection::vec(record(), 0..20)) {
            let (mut enc, mut dict, mut names) = (BlockEncoder::default(), Dict::default(), Vec::new());
            let mut blocks = Vec::new();
            for recs in [&a, &b] {
                for r in recs.iter() {
                    encode(&mut enc, &mut dict, r);
                }
                blocks.push(enc.take());
            }
            for (recs, block) in [&a, &b].into_iter().zip(&blocks) {
                let got: Vec<Record> = BlockDecoder::new(block, &mut names).collect::<Result<_>>().unwrap();
                prop_assert_eq!(&got, recs);
            }
        }

        /// Rastgele bayt: panik yok, sonsuz döngü yok.
        #[test]
        fn garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
            let mut names = Vec::new();
            for _ in BlockDecoder::new(&bytes, &mut names) {}
        }
    }

    #[test]
    fn steady_sample_is_small() {
        let s = Sample {
            ts_unix_ms: 1_763_128_800_000,
            cpu_total: 1234,
            cpu_core: vec![20; 16],
            ram_used_kb: 9_300_000,
            commit_kb: 12_000_000,
            cache_kb: 1_000_000,
            disk: vec![DiskIo {
                drive: 0,
                r_bps: 1000,
                w_bps: 5000,
            }],
            net: vec![NetIo {
                if_index: 7,
                rx_bps: 1200,
                tx_bps: 80,
            }],
        };
        let mut next = s.clone();
        next.ts_unix_ms += 1000;
        let mut enc = BlockEncoder::default();
        enc.sample(&s);
        let first = enc.len();
        enc.sample(&next);
        // Değişmeyen alanlar 1 bayt, ts +1000 ms 2 bayt
        assert!(
            enc.len() - first <= 40,
            "delta örnek {} bayt",
            enc.len() - first
        );
    }

    #[test]
    fn aggregate_min_avg_max() {
        let s = |cpu, core, ts| Sample {
            ts_unix_ms: ts,
            cpu_total: cpu,
            cpu_core: vec![core],
            ram_used_kb: cpu as u64,
            ..Default::default()
        };
        let odd = Sample {
            cpu_core: vec![1, 2],
            ..s(9999, 0, 3)
        }; // farklı düzen → dışarıda
        let [mn, avg, mx] = aggregate(&[s(10, 4, 1), s(30, 8, 2), odd], 60_000).unwrap();
        assert_eq!((mn.cpu_total, avg.cpu_total, mx.cpu_total), (10, 20, 30));
        assert_eq!((mn.cpu_core[0], avg.cpu_core[0], mx.cpu_core[0]), (4, 6, 8));
        assert_eq!(avg.ram_used_kb, 20);
        assert!(
            [&mn, &avg, &mx]
                .iter()
                .all(|x| x.ts_unix_ms == 60_000 && x.cpu_core.len() == 1)
        );
        assert!(aggregate(&[], 0).is_none());
    }

    #[test]
    fn truncated_block_errors() {
        let mut enc = BlockEncoder::default();
        enc.sample(&Sample {
            cpu_core: vec![1, 2, 3],
            ..Default::default()
        });
        let block = enc.take();
        let mut names = Vec::new();
        let r: Vec<_> = BlockDecoder::new(&block[..block.len() - 1], &mut names).collect();
        assert_eq!(r, vec![Err(DecodeError::Truncated)]);
    }
}
