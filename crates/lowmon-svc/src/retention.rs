//! Saklama (spek §5.5): `raw_days`'ten eski ham segmenti dakikalık min/avg/max'a indir, `total_days`'ten eskiyi sil.
//!
//! Değiştirme sırası çökmeye dayanıklı: `.tmp` yaz → baştan doğrula → ham dosyayı sil → `.tmp`'yi yeniden adlandır.
//! Yarım kalan `.tmp` bir sonraki çalışmada tamamlanır (orijinal yoksa) ya da silinir.
use lowmon_core::{
    codec::{AggKind, BlockDecoder, BlockEncoder, Dict, EventKind, Record, SCHEMA, aggregate},
    crypto::{FLAG_DOWNSAMPLED, Header, MasterKey, Opener, Sealer},
    metrics::{Sample, TopEntry},
    time::parse_day,
};
use std::{fs, io, path::Path};

/// Özet segmentte bir blok = 1 saat.
const MINUTES_PER_BLOCK: u64 = 60;

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub deleted: u32,
    pub downsampled: u32,
    /// Doğrulanamayan ham segment: dokunulmaz (kanıt olarak kalır).
    pub skipped: u32,
}

pub fn run(
    root: &Path,
    master: &MasterKey,
    raw_days: u32,
    total_days: u32,
    today: u64,
) -> io::Result<Report> {
    let logs = root.join("logs");
    let mut report = Report::default();
    for entry in fs::read_dir(&logs)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        let Some(day) = parse_day(&name) else {
            continue;
        };
        let age = today.saturating_sub(day);
        if let Some(orig) = name.strip_suffix(".tmp") {
            // Önceki çalışma silme ile yeniden adlandırma arasında kesildi
            let orig = logs.join(orig);
            if orig.exists() {
                fs::remove_file(&path)?;
            } else {
                fs::rename(&path, orig)?;
            }
            continue;
        }
        if age > total_days as u64 {
            fs::remove_file(&path)?;
            report.deleted += 1;
        } else if age > raw_days as u64 && name.ends_with(".lwm") {
            match downsample(&path, master)? {
                Some(true) => report.downsampled += 1,
                Some(false) => {}
                None => report.skipped += 1,
            }
        }
    }
    Ok(report)
}

/// `Some(true)` = indirildi, `Some(false)` = zaten özet, `None` = doğrulanamadı (dokunulmadı).
fn downsample(path: &Path, master: &MasterKey) -> io::Result<Option<bool>> {
    let bytes = fs::read(path)?;
    let Ok((header, opener)) = Opener::new(master, &bytes) else {
        return Ok(None);
    };
    if header.flags & FLAG_DOWNSAMPLED != 0 {
        return Ok(Some(false));
    }
    let mut names = Vec::new();
    let mut records = Vec::new();
    for block in opener {
        let Ok(block) = block else { return Ok(None) };
        for r in BlockDecoder::new(&block, &mut names) {
            let Ok(r) = r else { return Ok(None) };
            records.push(r);
        }
    }
    let out = build(&header, master, &records).map_err(|e| io::Error::other(format!("{e:?}")))?;
    let tmp = path.with_extension("lwm.tmp");
    fs::write(&tmp, &out)?;
    // Silmeden önce yeni dosyayı baştan doğrula
    let check = fs::read(&tmp)?;
    let (_, o) = Opener::new(master, &check).map_err(|e| io::Error::other(format!("{e:?}")))?;
    if o.into_iter().any(|b| b.is_err()) {
        fs::remove_file(&tmp)?;
        return Err(io::Error::other("özet segment doğrulanamadı"));
    }
    fs::remove_file(path)?;
    fs::rename(&tmp, path)?;
    Ok(Some(true))
}

struct Out {
    bytes: Vec<u8>,
    sealer: Sealer,
    enc: BlockEncoder,
    dict: Dict,
    minutes: u64,
    /// Açık dakikanın olayları: özetten sonra yazılır (zamanları dakika başına iner).
    events: Vec<(EventKind, String, i64)>,
}

impl Out {
    fn flush(&mut self) -> Result<(), lowmon_core::crypto::CryptoError> {
        if !self.enc.is_empty() {
            let plain = self.enc.take();
            self.sealer.seal(&plain, &mut self.bytes)?;
        }
        Ok(())
    }

    fn minute(
        &mut self,
        bucket: &mut Vec<Sample>,
        top: &mut Option<Vec<TopEntry>>,
        minute: u64,
    ) -> Result<(), lowmon_core::crypto::CryptoError> {
        let aggs = aggregate(bucket, minute * 60_000);
        if let Some(aggs) = &aggs {
            for (kind, s) in AggKind::ALL.iter().zip(aggs) {
                self.enc.agg(*kind, s);
            }
            // Dakikanın ilk TopN'i korunur (spek: 1 dk çözünürlük)
            if let Some(t) = top.take() {
                self.enc.top(&t, &mut self.dict);
            }
        }
        // Olaylar özetlenmez, hepsi taşınır (ADR-0028)
        for (k, n, v) in std::mem::take(&mut self.events) {
            self.enc.event(k, &n, v, &mut self.dict);
        }
        if aggs.is_some() {
            self.minutes += 1;
            if self.minutes.is_multiple_of(MINUTES_PER_BLOCK) {
                self.flush()?;
            }
        }
        bucket.clear();
        *top = None;
        Ok(())
    }
}

fn build(
    src: &Header,
    master: &MasterKey,
    records: &[Record],
) -> Result<Vec<u8>, lowmon_core::crypto::CryptoError> {
    let mut header = Header::new(src.segment_id, src.created_unix, 60_000, SCHEMA)?;
    header.flags = FLAG_DOWNSAMPLED;
    let mut out = Out {
        bytes: header.to_bytes().to_vec(),
        sealer: Sealer::new(master, &header),
        enc: BlockEncoder::default(),
        dict: Dict::default(),
        minutes: 0,
        events: Vec::new(),
    };
    let (mut bucket, mut top, mut cur) = (Vec::new(), None, None);
    for r in records {
        match r {
            Record::Sample(s) => {
                let m = s.ts_unix_ms / 60_000;
                if cur.is_some_and(|c| c != m) {
                    out.minute(&mut bucket, &mut top, cur.unwrap())?;
                }
                cur = Some(m);
                bucket.push(s.clone());
            }
            Record::Top(t) if top.is_none() && cur.is_some() => top = Some(t.clone()),
            Record::Event { kind, name, value } if cur.is_some() => {
                out.events.push((*kind, name.clone(), *value))
            }
            // Açık dakika yok (ör. gap sonrası, ilk örnekten önce): hemen yaz
            Record::Event { kind, name, value } => {
                out.enc.event(*kind, name, *value, &mut out.dict)
            }
            Record::Gap {
                from_ms,
                to_ms,
                reason,
            } => {
                if let Some(c) = cur.take() {
                    out.minute(&mut bucket, &mut top, c)?;
                }
                out.enc.gap(*from_ms, *to_ms, *reason);
            }
            // Ham segmentteki Seal'ler yeni zincirde anlamsız; Agg ham segmentte olmaz
            Record::Top(_) | Record::Seal { .. } | Record::Agg { .. } => {}
        }
    }
    if let Some(c) = cur {
        out.minute(&mut bucket, &mut top, c)?;
    }
    out.flush()?;
    let (blocks, prev) = out.sealer.head();
    let prev = prev.to_vec();
    out.enc.seal(blocks, out.minutes, &prev);
    out.flush()?;
    Ok(out.bytes)
}

/// Açılıştan 5 dk sonra bir kez, sonra her gün yerel 03:00'te; IDLE öncelikli ayrı thread.
pub struct Scheduler {
    root: std::path::PathBuf,
    master: MasterKey,
    raw_days: u32,
    total_days: u32,
    tick: u64,
    last_local_day: Option<u16>,
    running: Option<std::thread::JoinHandle<()>>,
}

const FIRST_RUN_TICK: u64 = 300;

impl Scheduler {
    pub fn new(root: &Path, master: MasterKey, raw_days: u32, total_days: u32) -> Self {
        Self {
            root: root.to_owned(),
            master,
            raw_days,
            total_days,
            tick: 0,
            last_local_day: None,
            running: None,
        }
    }

    /// ReloadConfig: sonraki çalışmada geçerli (süren çalışma eski değerle biter).
    pub fn set_days(&mut self, raw_days: u32, total_days: u32) {
        (self.raw_days, self.total_days) = (raw_days, total_days);
    }

    /// Her tick çağrılır; 60 tick'te bir saat kontrolü yapar (maliyet ~0).
    pub fn tick(&mut self, now_ms: u64) {
        self.tick += 1;
        if self.tick != FIRST_RUN_TICK && !self.tick.is_multiple_of(60) {
            return;
        }
        let mut lt = windows_sys::Win32::Foundation::SYSTEMTIME::default();
        // SAFETY: çıktı yapısı yığında.
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut lt) };
        let due_daily = lt.wHour == 3 && self.last_local_day != Some(lt.wDay);
        if self.tick != FIRST_RUN_TICK && !due_daily {
            return;
        }
        if self.running.as_ref().is_some_and(|h| !h.is_finished()) {
            return; // öncekisi sürüyor
        }
        self.last_local_day = due_daily.then_some(lt.wDay).or(self.last_local_day);
        let (root, master, raw, total) = (
            self.root.clone(),
            self.master,
            self.raw_days,
            self.total_days,
        );
        let today = lowmon_core::time::day_of(now_ms);
        self.running = Some(std::thread::spawn(move || {
            use windows_sys::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_IDLE,
            };
            // SAFETY: kendi thread handle'ımız.
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_IDLE) };
            // shortcut: sonuç/hata loglanmıyor; M5'te Event olarak yazılır
            let _ = run(&root, &master, raw, total, today);
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Writer;
    use lowmon_core::{
        codec::GapReason,
        time::{DAY_MS, segment_name},
    };
    use std::path::PathBuf;

    const KEY: MasterKey = [5; 32];
    const TODAY: u64 = 20_400;

    fn root(name: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("lowmon-test-{}-ret-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        r
    }

    /// `day` gününe `minutes` dakikalık 1 s örnekli ham segment yazar; dakika başı bir TopN.
    fn raw_segment(r: &Path, day: u64, minutes: u64) {
        let t0 = day * DAY_MS + 3_600_000;
        let mut w = Writer::open(r, KEY, 1000, t0, 3_600_000).unwrap();
        for i in 0..minutes * 60 {
            let s = Sample {
                ts_unix_ms: t0 + i * 1000,
                cpu_total: (i % 60) as u16 * 100, // dakika içinde 0..5900
                cpu_core: vec![(i % 60) as u8],
                ..Default::default()
            };
            w.sample(&s).unwrap();
            if i % 60 == 0 {
                w.top(&[TopEntry {
                    pid: 1,
                    cpu: 7,
                    ws_kb: 1,
                    name: "x.exe".into(),
                }]);
            }
        }
        w.close().unwrap();
    }

    fn records(p: &Path) -> (Header, Vec<Record>) {
        let b = fs::read(p).unwrap();
        let (h, o) = Opener::new(&KEY, &b).unwrap();
        let mut names = Vec::new();
        let mut out = Vec::new();
        for blk in o {
            out.extend(BlockDecoder::new(&blk.unwrap(), &mut names).map(Result::unwrap));
        }
        (h, out)
    }

    #[test]
    fn downsamples_deletes_and_keeps_recent() {
        let r = root("main");
        let logs = r.join("logs");
        for (age, minutes) in [(3, 2), (10, 125), (40, 1)] {
            raw_segment(&r, TODAY - age, minutes);
        }
        fs::write(
            logs.join(format!("{}.bad-9", segment_name(TODAY - 40))),
            b"x",
        )
        .unwrap();
        let rep = run(&r, &KEY, 7, 30, TODAY).unwrap();
        assert_eq!(
            rep,
            Report {
                deleted: 2,
                downsampled: 1,
                skipped: 0
            }
        );
        assert!(!logs.join(segment_name(TODAY - 40)).exists());

        let (h, recs) = records(&logs.join(segment_name(TODAY - 10)));
        assert_eq!(h.flags & FLAG_DOWNSAMPLED, FLAG_DOWNSAMPLED);
        assert!(!recs.iter().any(|r| matches!(r, Record::Sample(_))));
        let aggs: Vec<_> = recs
            .iter()
            .filter_map(|r| match r {
                Record::Agg { kind, sample } => Some((*kind, sample.cpu_total)),
                _ => None,
            })
            .collect();
        assert_eq!(aggs.len(), 125 * 3);
        assert_eq!(
            &aggs[..3],
            &[
                (AggKind::Min, 0),
                (AggKind::Avg, 2950),
                (AggKind::Max, 5900)
            ]
        );
        assert_eq!(
            recs.iter().filter(|r| matches!(r, Record::Top(_))).count(),
            125
        );
        assert!(matches!(
            recs.last(),
            Some(Record::Seal {
                sample_count: 125,
                ..
            })
        ));

        // 3 günlük segment ham kaldı; ikinci çalışma bir şey yapmaz
        let (h3, _) = records(&logs.join(segment_name(TODAY - 3)));
        assert_eq!(h3.flags, 0);
        assert_eq!(run(&r, &KEY, 7, 30, TODAY).unwrap(), Report::default());
    }

    #[test]
    fn gaps_survive_downsampling() {
        let r = root("gap");
        let day = TODAY - 10;
        raw_segment(&r, day, 1);
        // Aynı güne temiz yeniden başlatma → Paused gap
        let mut w = Writer::open(&r, KEY, 1000, day * DAY_MS + 7_200_000, 3_600_000).unwrap();
        w.close().unwrap();
        run(&r, &KEY, 7, 30, TODAY).unwrap();
        let (_, recs) = records(&r.join("logs").join(segment_name(day)));
        assert!(recs.iter().any(|r| matches!(
            r,
            Record::Gap {
                reason: GapReason::Paused,
                ..
            }
        )));
    }

    #[test]
    fn events_survive_downsampling() {
        let r = root("event");
        let t0 = (TODAY - 10) * DAY_MS;
        let mut w = Writer::open(&r, KEY, 1000, t0, 3_600_000).unwrap();
        for i in 0..4 {
            w.sample(&Sample {
                ts_unix_ms: t0 + i * 1000,
                ..Default::default()
            })
            .unwrap();
            if i == 1 {
                w.event(EventKind::AlarmRaise, "cpu", 9000);
            }
        }
        w.close().unwrap();
        run(&r, &KEY, 7, 30, TODAY).unwrap();
        let (_, recs) = records(&r.join("logs").join(segment_name(TODAY - 10)));
        let ev = recs
            .iter()
            .position(|r| matches!(r, Record::Event { .. }))
            .expect("event kayboldu");
        assert_eq!(
            recs[ev],
            Record::Event {
                kind: EventKind::AlarmRaise,
                name: "cpu".into(),
                value: 9000
            }
        );
        // Dakikanın Agg'larından sonra gelir → okuyucu ts'i dakika sonu olarak görür
        assert!(matches!(
            recs[ev - 1],
            Record::Agg {
                kind: AggKind::Max,
                ..
            }
        ));
    }

    #[test]
    fn corrupt_raw_is_left_alone() {
        let r = root("bad");
        let p = r.join("logs").join(segment_name(TODAY - 10));
        raw_segment(&r, TODAY - 10, 2);
        let mut b = fs::read(&p).unwrap();
        b[100] ^= 1;
        fs::write(&p, &b).unwrap();
        assert_eq!(run(&r, &KEY, 7, 30, TODAY).unwrap().skipped, 1);
        assert_eq!(fs::read(&p).unwrap(), b);
    }

    #[test]
    fn leftover_tmp_is_completed() {
        let r = root("tmp");
        let logs = r.join("logs");
        raw_segment(&r, TODAY - 10, 1);
        let p = logs.join(segment_name(TODAY - 10));
        // Çökme simülasyonu: ham silinmiş, .tmp kalmış
        fs::rename(&p, p.with_extension("lwm.tmp")).unwrap();
        run(&r, &KEY, 7, 30, TODAY).unwrap();
        assert!(p.exists() && !p.with_extension("lwm.tmp").exists());
    }
}
