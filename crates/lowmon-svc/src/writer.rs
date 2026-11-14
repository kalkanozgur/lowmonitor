//! Segment yazıcı (spek §5): günlük UTC dosyası, 60 örnek = 1 şifreli blok, Seal, açılış kurtarma, state.bin.
//!
//! Dizin düzeni: `<root>/logs/YYYY-MM-DD.lwm`, `<root>/state/state.bin`.
//! Açılışta bugünün segmenti baştan doğrulanır; zincir + isim sözlüğü geri kurulur ve sona eklenir.
use lowmon_core::{
    codec::{BlockDecoder, BlockEncoder, Dict, GapReason, Record, SCHEMA},
    crypto::{CryptoError, Header, MasterKey, Opener, Sealer},
    metrics::{Sample, TopEntry},
    time::{day_of, segment_name},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const SAMPLES_PER_BLOCK: u32 = 60;
/// Bu süreden kısa uptime → açılış (Boot), değilse servis duraklatılmış (Paused).
const BOOT_UPTIME_MS: u64 = 120_000;

pub struct Writer {
    root: PathBuf,
    master: MasterKey,
    interval_ms: u32,
    seg: Segment,
    out: Vec<u8>,
}

struct Segment {
    day: u64,
    id: u64,
    file: File,
    sealer: Sealer,
    enc: BlockEncoder,
    dict: Dict,
    in_block: u32,
    samples: u64,
    last_ts: Option<u64>,
}

/// state.bin: son segmentin zincir başı (spek §5.6). Ayrı ACL ile korunur (install).
#[derive(Debug, PartialEq)]
struct State {
    segment_id: u64,
    blocks: u64,
    prev: Vec<u8>,
}

const STATE_MAGIC: &[u8; 4] = b"LWMS";

impl State {
    fn encode(&self) -> Vec<u8> {
        let mut b = STATE_MAGIC.to_vec();
        b.extend_from_slice(&self.segment_id.to_le_bytes());
        b.extend_from_slice(&self.blocks.to_le_bytes());
        b.push(self.prev.len() as u8); // prev ≤ 64 B (başlık) ya da 16 B (tag)
        b.extend_from_slice(&self.prev);
        b
    }

    fn decode(b: &[u8]) -> Option<Self> {
        let u64_at = |i: usize| Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?));
        if b.get(..4)? != STATE_MAGIC {
            return None;
        }
        let len = *b.get(20)? as usize;
        Some(Self {
            segment_id: u64_at(4)?,
            blocks: u64_at(12)?,
            prev: b.get(21..21 + len)?.to_vec(),
        })
    }
}

fn crypto_err(e: CryptoError) -> io::Error {
    io::Error::other(format!("{e:?}"))
}

/// Var olan segmenti doğrulama sonucu.
struct Resumed {
    header: Header,
    sealer: Sealer,
    names: Vec<String>,
    samples: u64,
    last_ts: Option<u64>,
    clean: bool,    // son kayıt Seal ve kuyruk sağlam
    verified: bool, // state.bin zincir başı segmentte bulundu
    torn_at: Option<usize>,
}

enum Walk {
    Ok(Resumed),
    /// Başlık okunamadı veya ortada doğrulanmayan blok: dosya karantinaya.
    Corrupt(Option<u64>),
}

fn walk(master: &MasterKey, bytes: &[u8], state: Option<&State>) -> Walk {
    let Ok((header, mut opener)) = Opener::new(master, bytes) else {
        return Walk::Corrupt(None);
    };
    let corrupt = Walk::Corrupt(Some(header.segment_id));
    let st = state.filter(|s| s.segment_id == header.segment_id);
    let matches = |o: &Opener| st.is_some_and(|s| o.head() == (s.blocks, s.prev.as_slice()));
    let mut verified = matches(&opener);
    let (mut names, mut samples, mut last_ts, mut sealed, mut torn) =
        (Vec::new(), 0, None, false, false);
    loop {
        match opener.next() {
            None => break,
            Some(Ok(plain)) => {
                for rec in BlockDecoder::new(&plain, &mut names) {
                    let Ok(rec) = rec else { return corrupt };
                    sealed = matches!(rec, Record::Seal { .. });
                    if let Record::Sample(s) = rec {
                        samples += 1;
                        last_ts = Some(s.ts_unix_ms);
                    }
                }
                verified |= matches(&opener);
            }
            Some(Err(CryptoError::Truncated { .. })) => {
                torn = true;
                break;
            }
            Some(Err(_)) => return corrupt,
        }
    }
    let torn_at = torn.then(|| opener.good_end());
    Walk::Ok(Resumed {
        header,
        sealer: opener.into_sealer(),
        names,
        samples,
        last_ts,
        clean: sealed && !torn,
        verified,
        torn_at,
    })
}

impl Writer {
    /// Bugünün segmentini açar (varsa doğrulayıp devam eder, yoksa oluşturur).
    pub fn open(
        root: &Path,
        master: MasterKey,
        interval_ms: u32,
        now_ms: u64,
        uptime_ms: u64,
    ) -> io::Result<Self> {
        fs::create_dir_all(root.join("logs"))?;
        fs::create_dir_all(root.join("state"))?;
        let state = fs::read(root.join("state").join("state.bin"))
            .ok()
            .and_then(|b| State::decode(&b));
        let day = day_of(now_ms);
        let path = root.join("logs").join(segment_name(day));
        let mut next_id = state.as_ref().map_or(1, |s| s.segment_id + 1);
        let mut gaps = Vec::new();
        let seg = match path.exists().then(|| fs::read(&path)).transpose()? {
            None => Self::create(root, &master, interval_ms, day, next_id, now_ms)?,
            Some(bytes) => match walk(&master, &bytes, state.as_ref()) {
                Walk::Ok(r) => {
                    if let Some(len) = r.torn_at {
                        OpenOptions::new()
                            .write(true)
                            .open(&path)?
                            .set_len(len as u64)?;
                    }
                    let from = r.last_ts.unwrap_or(r.header.created_unix * 1000);
                    let reason = match (r.clean, uptime_ms < BOOT_UPTIME_MS) {
                        (false, _) => GapReason::Crash,
                        (true, true) => GapReason::Boot,
                        (true, false) => GapReason::Paused,
                    };
                    gaps.push((from, reason));
                    if !r.verified {
                        gaps.push((from, GapReason::Integrity));
                    }
                    Segment {
                        day,
                        id: r.header.segment_id,
                        file: OpenOptions::new().append(true).open(&path)?,
                        sealer: r.sealer,
                        enc: BlockEncoder::default(),
                        dict: Dict::from_names(r.names),
                        in_block: 0,
                        samples: r.samples,
                        last_ts: None, // aradaki süre kurtarma gap'iyle kapsanıyor
                    }
                }
                Walk::Corrupt(id) => {
                    next_id = next_id.max(id.map_or(0, |i| i + 1));
                    fs::rename(&path, path.with_extension(format!("lwm.bad-{next_id}")))?;
                    gaps.push((now_ms, GapReason::Integrity));
                    Self::create(root, &master, interval_ms, day, next_id, now_ms)?
                }
            },
        };
        let mut w = Self {
            root: root.to_owned(),
            master,
            interval_ms,
            seg,
            out: Vec::new(),
        };
        for (from, reason) in gaps {
            w.seg.enc.gap(from, now_ms, reason);
        }
        Ok(w)
    }

    fn create(
        root: &Path,
        master: &MasterKey,
        interval_ms: u32,
        day: u64,
        id: u64,
        now_ms: u64,
    ) -> io::Result<Segment> {
        let header = Header::new(id, now_ms / 1000, interval_ms, SCHEMA).map_err(crypto_err)?;
        let path = root.join("logs").join(segment_name(day));
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)?;
        file.write_all(&header.to_bytes())?;
        let seg = Segment {
            day,
            id,
            file,
            sealer: Sealer::new(master, &header),
            enc: BlockEncoder::default(),
            dict: Dict::default(),
            in_block: 0,
            samples: 0,
            last_ts: None,
        };
        save_state(root, &seg)?;
        Ok(seg)
    }

    pub fn sample(&mut self, s: &Sample) -> io::Result<()> {
        let day = day_of(s.ts_unix_ms);
        // Saat geri giderse mevcut segmentte kalınır; yalnızca ileri gün dönümü döndürür.
        if day > self.seg.day {
            self.close()?;
            self.seg = Self::create(
                &self.root,
                &self.master,
                self.interval_ms,
                day,
                self.seg.id + 1,
                s.ts_unix_ms,
            )?;
        }
        let seg = &mut self.seg;
        if let Some(last) = seg.last_ts
            && s.ts_unix_ms > last + 5 * self.interval_ms as u64
        {
            seg.enc.gap(last, s.ts_unix_ms, GapReason::Sleep);
        }
        seg.enc.sample(s);
        seg.in_block += 1;
        seg.samples += 1;
        seg.last_ts = Some(s.ts_unix_ms);
        if seg.in_block >= SAMPLES_PER_BLOCK {
            self.flush()?;
        }
        Ok(())
    }

    pub fn top(&mut self, top: &[TopEntry]) {
        self.seg.enc.top(top, &mut self.seg.dict);
    }

    /// Tamponu tek blok olarak yazar (tek WriteFile) ve zincir başını state.bin'e kaydeder.
    fn flush(&mut self) -> io::Result<()> {
        let seg = &mut self.seg;
        if seg.enc.is_empty() {
            return Ok(());
        }
        let plain = seg.enc.take();
        self.out.clear();
        seg.sealer.seal(&plain, &mut self.out).map_err(crypto_err)?;
        seg.file.write_all(&self.out)?;
        seg.in_block = 0;
        save_state(&self.root, seg)
    }

    /// STOP / PRESHUTDOWN / gün dönümü: kalan örnekler + Seal bloğu.
    pub fn close(&mut self) -> io::Result<()> {
        self.flush()?;
        let seg = &mut self.seg;
        let (blocks, prev) = seg.sealer.head();
        let prev = prev.to_vec();
        seg.enc.seal(blocks, seg.samples, &prev);
        self.flush()
    }
}

fn save_state(root: &Path, seg: &Segment) -> io::Result<()> {
    let (blocks, prev) = seg.sealer.head();
    let st = State {
        segment_id: seg.id,
        blocks,
        prev: prev.to_vec(),
    };
    // shortcut: atomik değil (yaz-yeniden adlandır yok); bozuk state.bin → Gap(Integrity), veri kaybı yok
    fs::write(root.join("state").join("state.bin"), st.encode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowmon_core::{crypto::HEADER_LEN, time::DAY_MS};

    const KEY: MasterKey = [3; 32];
    const T0: u64 = 20_000 * DAY_MS + 1_000; // gün başı + 1 s
    const LONG_UP: u64 = 3_600_000;

    fn root(name: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("lowmon-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        r
    }

    fn smp(ts: u64) -> Sample {
        Sample {
            ts_unix_ms: ts,
            cpu_total: 100,
            cpu_core: vec![1, 2],
            ..Default::default()
        }
    }

    fn seg_path(r: &Path, ts: u64) -> PathBuf {
        r.join("logs").join(segment_name(day_of(ts)))
    }

    /// Tüm segmenti doğrulayıp kayıtları döker; herhangi bir hata testte panik.
    fn records(path: &Path) -> Vec<Record> {
        let bytes = fs::read(path).unwrap();
        let (_, opener) = Opener::new(&KEY, &bytes).unwrap();
        let mut names = Vec::new();
        let mut out = Vec::new();
        for block in opener {
            for r in BlockDecoder::new(&block.unwrap(), &mut names) {
                out.push(r.unwrap());
            }
        }
        out
    }

    fn samples(recs: &[Record]) -> usize {
        recs.iter()
            .filter(|r| matches!(r, Record::Sample(_)))
            .count()
    }

    fn gaps(recs: &[Record]) -> Vec<GapReason> {
        recs.iter()
            .filter_map(|r| match r {
                Record::Gap { reason, .. } => Some(*reason),
                _ => None,
            })
            .collect()
    }

    fn write_n(w: &mut Writer, from: u64, n: u64) -> u64 {
        for i in 0..n {
            w.sample(&smp(from + i * 1000)).unwrap();
        }
        from + n * 1000
    }

    #[test]
    fn blocks_of_60_and_seal() {
        let r = root("blocks");
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        write_n(&mut w, T0, 130);
        w.close().unwrap();
        let recs = records(&seg_path(&r, T0));
        assert_eq!(samples(&recs), 130);
        assert!(gaps(&recs).is_empty());
        match recs.last().unwrap() {
            Record::Seal {
                block_count,
                sample_count,
                ..
            } => assert_eq!((*block_count, *sample_count), (3, 130)),
            other => panic!("son kayıt Seal değil: {other:?}"),
        }
    }

    #[test]
    fn clean_restart_appends_with_paused_gap_and_keeps_dict() {
        let r = root("restart");
        let top = [TopEntry {
            pid: 1,
            cpu: 5,
            ws_kb: 10,
            name: "a.exe".into(),
        }];
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        let t = write_n(&mut w, T0, 50);
        w.top(&top);
        w.close().unwrap();
        let mut w = Writer::open(&r, KEY, 1000, t + 600_000, LONG_UP).unwrap();
        w.top(&top);
        write_n(&mut w, t + 600_000, 10);
        w.close().unwrap();
        let recs = records(&seg_path(&r, T0));
        assert_eq!(samples(&recs), 60);
        assert_eq!(gaps(&recs), [GapReason::Paused]);
        let tops = recs
            .iter()
            .filter(|r| matches!(r, Record::Top(t) if t[0].name == "a.exe"))
            .count();
        assert_eq!(tops, 2);
    }

    #[test]
    fn boot_when_uptime_short() {
        let r = root("boot");
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        let t = write_n(&mut w, T0, 5);
        w.close().unwrap();
        let mut w = Writer::open(&r, KEY, 1000, t + 60_000, 30_000).unwrap();
        w.close().unwrap();
        assert_eq!(gaps(&records(&seg_path(&r, T0))), [GapReason::Boot]);
    }

    #[test]
    fn crash_loses_only_unflushed_samples() {
        let r = root("crash");
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        let t = write_n(&mut w, T0, 70);
        drop(w); // close yok = çökme
        let mut w = Writer::open(&r, KEY, 1000, t, LONG_UP).unwrap();
        w.close().unwrap();
        let recs = records(&seg_path(&r, T0));
        assert_eq!(samples(&recs), 60);
        assert_eq!(gaps(&recs), [GapReason::Crash]);
    }

    #[test]
    fn torn_tail_is_trimmed() {
        let r = root("torn");
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        let t = write_n(&mut w, T0, 60);
        drop(w);
        let p = seg_path(&r, T0);
        OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(&[100, 0, 0, 0, 1, 2])
            .unwrap(); // geçerli len, eksik gövde = yarım yazma
        let mut w = Writer::open(&r, KEY, 1000, t, LONG_UP).unwrap();
        write_n(&mut w, t, 10);
        w.close().unwrap();
        let recs = records(&p); // panik yok = yarım blok kırpıldı
        assert_eq!(samples(&recs), 70);
        assert_eq!(gaps(&recs), [GapReason::Crash]);
    }

    #[test]
    fn tail_cut_at_block_boundary_is_flagged_by_state() {
        let r = root("cut");
        let p = seg_path(&r, T0);
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        let t = write_n(&mut w, T0, 60);
        let after_first = fs::metadata(&p).unwrap().len();
        let t = write_n(&mut w, t, 60);
        drop(w);
        OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_len(after_first)
            .unwrap();
        let mut w = Writer::open(&r, KEY, 1000, t, LONG_UP).unwrap();
        w.close().unwrap();
        assert_eq!(gaps(&records(&p)), [GapReason::Crash, GapReason::Integrity]);
    }

    #[test]
    fn corrupt_segment_is_quarantined() {
        let r = root("corrupt");
        let p = seg_path(&r, T0);
        let mut w = Writer::open(&r, KEY, 1000, T0, LONG_UP).unwrap();
        let t = write_n(&mut w, T0, 120);
        drop(w);
        let mut bytes = fs::read(&p).unwrap();
        bytes[HEADER_LEN + 40] ^= 1; // blok 0 içi
        fs::write(&p, &bytes).unwrap();
        let mut w = Writer::open(&r, KEY, 1000, t, LONG_UP).unwrap();
        w.close().unwrap();
        assert!(p.with_extension("lwm.bad-2").exists());
        let recs = records(&p);
        assert_eq!(gaps(&recs), [GapReason::Integrity]);
        assert_eq!(Header::parse(&fs::read(&p).unwrap()).unwrap().segment_id, 2);
    }

    #[test]
    fn day_rollover_seals_previous_segment() {
        let r = root("rollover");
        let start = T0 + DAY_MS - 31_000; // gece yarısından 30 s önce
        let mut w = Writer::open(&r, KEY, 1000, start, LONG_UP).unwrap();
        write_n(&mut w, start, 60);
        w.close().unwrap();
        let first = records(&seg_path(&r, start));
        let second = records(&seg_path(&r, start + 60_000));
        assert!(matches!(first.last(), Some(Record::Seal { .. })));
        assert_eq!(samples(&first) + samples(&second), 60);
        assert!(samples(&first) >= 30 && samples(&second) >= 29);
        let id = |ts| {
            Header::parse(&fs::read(seg_path(&r, ts)).unwrap())
                .unwrap()
                .segment_id
        };
        assert_eq!(id(start + 60_000), id(start) + 1);
    }

    #[test]
    fn state_round_trip() {
        let s = State {
            segment_id: 7,
            blocks: 3,
            prev: vec![1; 16],
        };
        assert_eq!(State::decode(&s.encode()), Some(s));
        assert_eq!(State::decode(b"junk"), None);
    }
}
