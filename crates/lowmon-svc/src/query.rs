//! Pipe `QueryRange` (spek §6.3): `[from, to)` metrikleri, yetki "Herkes".
//! Top-N dönmez: süreç geçmişi yalnızca admin `Export`'ta.
use crate::export::segments;
use lowmon_core::{
    codec::{AggKind, BlockDecoder, GapReason, Record, aggregate},
    crypto::{MasterKey, Opener},
    metrics::Sample,
    proto::{RangeItem, Resolution},
    time::DAY_MS,
};
use std::{fs, io, path::Path};

const MINUTE_MS: u64 = 60_000;

/// shortcut: yazıcının bellekteki son bloğu (≤ 60 örnek) dahil değil (Export ile aynı; flush M4'te)
pub fn query(
    root: &Path,
    master: &MasterKey,
    from: u64,
    to: u64,
    res: Resolution,
    emit: &mut impl FnMut(RangeItem) -> io::Result<()>,
) -> io::Result<()> {
    if from >= to {
        return Ok(());
    }
    for (day, p) in segments(root, from, to)? {
        let bytes = fs::read(&p)?;
        let mut last = day * DAY_MS;
        if !segment(master, &bytes, from, to, res, &mut last, emit)? {
            // Açık segmentte yarım son blok da buraya düşer; doğrulanmış kısım zaten gitti
            let (g0, g1) = (last.max(from), to.min((day + 1) * DAY_MS));
            if g0 < g1 {
                emit(RangeItem::Gap {
                    from_ms: g0,
                    to_ms: g1,
                    reason: GapReason::Integrity,
                })?;
            }
        }
    }
    Ok(())
}

/// Pipe `QueryEvents` (LowDeck ADR-0003): yalnız `RangeItem::Event`; segment yine çözülür.
pub fn query_events(
    root: &Path,
    master: &MasterKey,
    from: u64,
    to: u64,
    emit: &mut impl FnMut(RangeItem) -> io::Result<()>,
) -> io::Result<()> {
    query(
        root,
        master,
        from,
        to,
        Resolution::Raw,
        &mut |it| match it {
            RangeItem::Event { .. } => emit(it),
            _ => Ok(()),
        },
    )
}

/// Segmenti baştan doğrulayarak akıtır. `false`: doğrulama/çözme hatası, `last` = son geçerli ts.
fn segment(
    master: &MasterKey,
    bytes: &[u8],
    from: u64,
    to: u64,
    res: Resolution,
    last: &mut u64,
    emit: &mut impl FnMut(RangeItem) -> io::Result<()>,
) -> io::Result<bool> {
    let Ok((_, opener)) = Opener::new(master, bytes) else {
        return Ok(false);
    };
    let (mut names, mut minute) = (Vec::new(), Vec::<Sample>::new());
    let mut ok = true;
    'blocks: for block in opener {
        let Ok(block) = block else {
            ok = false;
            break;
        };
        for rec in BlockDecoder::new(&block, &mut names) {
            match rec {
                Err(_) => {
                    ok = false;
                    break 'blocks;
                }
                Ok(Record::Sample(s)) => {
                    *last = s.ts_unix_ms;
                    if !(from..to).contains(&s.ts_unix_ms) {
                        continue;
                    }
                    match res {
                        Resolution::Raw => emit(RangeItem::Sample(s))?,
                        Resolution::Minute => {
                            let m = s.ts_unix_ms / MINUTE_MS;
                            if minute
                                .first()
                                .is_some_and(|f| f.ts_unix_ms / MINUTE_MS != m)
                            {
                                flush(&mut minute, emit)?;
                            }
                            minute.push(s);
                        }
                    }
                }
                Ok(Record::Agg { kind, sample }) => {
                    *last = sample.ts_unix_ms + MINUTE_MS;
                    if (from..to).contains(&sample.ts_unix_ms) {
                        emit(RangeItem::Agg { kind, sample })?;
                    }
                }
                Ok(Record::Gap {
                    from_ms,
                    to_ms,
                    reason,
                }) if from_ms < to && to_ms > from => emit(RangeItem::Gap {
                    from_ms,
                    to_ms,
                    reason,
                })?,
                Ok(Record::Event { kind, name, value }) if (from..to).contains(last) => {
                    emit(RangeItem::Event {
                        ts_ms: *last,
                        kind,
                        name,
                        value,
                    })?
                }
                Ok(_) => {} // Top, Seal, kapsam dışı Gap/Event
            }
        }
    }
    flush(&mut minute, emit)?;
    Ok(ok)
}

/// Biriken dakikayı min/avg/max olarak yollar.
fn flush(
    minute: &mut Vec<Sample>,
    emit: &mut impl FnMut(RangeItem) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(aggs) = minute
        .first()
        .and_then(|f| aggregate(minute, f.ts_unix_ms / MINUTE_MS * MINUTE_MS))
    {
        for (kind, sample) in AggKind::ALL.into_iter().zip(aggs) {
            emit(RangeItem::Agg { kind, sample })?;
        }
    }
    minute.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Writer;
    use lowmon_core::time::{day_of, segment_name};

    #[test]
    fn raw_minute_and_corruption() {
        let r = std::env::temp_dir().join(format!("lowmon-test-{}-query", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let key = [6; 32];
        let t0 = 20_000 * DAY_MS + DAY_MS - 3_000; // 23:59:57 → iki gün, iki dakika
        let mut w = Writer::open(&r, key, 1000, t0, 3_600_000).unwrap();
        for i in 0..6 {
            let s = Sample {
                ts_unix_ms: t0 + i * 1000,
                cpu_total: i as u16 * 100,
                ..Default::default()
            };
            w.sample(&s).unwrap();
        }
        w.close().unwrap();

        let run = |from, to, res| {
            let mut v = Vec::new();
            query(&r, &key, from, to, res, &mut |it| {
                v.push(it);
                Ok(())
            })
            .unwrap();
            v
        };
        let raw = run(t0, t0 + 6_000, Resolution::Raw);
        assert_eq!(raw.len(), 6, "{raw:?}");
        assert!(raw.iter().all(|i| matches!(i, RangeItem::Sample(_))));
        assert!(run(5, 5, Resolution::Raw).is_empty());

        let min = run(t0, t0 + 6_000, Resolution::Minute);
        let aggs: Vec<(AggKind, u64, u16)> = min
            .iter()
            .map(|i| match i {
                RangeItem::Agg { kind, sample } => (*kind, sample.ts_unix_ms, sample.cpu_total),
                m => panic!("beklenmeyen {m:?}"),
            })
            .collect();
        let (m0, m1) = (t0 - 57_000, t0 + 3_000); // dakika başları
        assert_eq!(
            aggs,
            [
                (AggKind::Min, m0, 0),
                (AggKind::Avg, m0, 100),
                (AggKind::Max, m0, 200),
                (AggKind::Min, m1, 300),
                (AggKind::Avg, m1, 400),
                (AggKind::Max, m1, 500),
            ]
        );

        // İkinci gün bozulursa: ilk gün verisi + o günün kalanı Integrity gap
        let p2 = r.join("logs").join(segment_name(day_of(t0) + 1));
        let mut b = fs::read(&p2).unwrap();
        b[64 + 4 + 24 + 1] ^= 1;
        fs::write(&p2, b).unwrap();
        let bad = run(t0, t0 + 6_000, Resolution::Raw);
        assert_eq!(bad.len(), 4, "{bad:?}");
        assert_eq!(
            bad[3],
            RangeItem::Gap {
                from_ms: t0 + 3_000,
                to_ms: t0 + 6_000,
                reason: GapReason::Integrity
            }
        );
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn event_gets_previous_sample_ts() {
        let r = std::env::temp_dir().join(format!("lowmon-test-{}-qevent", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let key = [7; 32];
        let t0 = 20_000 * DAY_MS;
        let mut w = Writer::open(&r, key, 1000, t0, 3_600_000).unwrap();
        for i in 0..3 {
            w.sample(&Sample {
                ts_unix_ms: t0 + i * 1000,
                ..Default::default()
            })
            .unwrap();
            if i == 1 {
                w.event(lowmon_core::metrics::EventKind::AlarmClear, "ram", 42);
            }
        }
        w.close().unwrap();
        let mut v = Vec::new();
        query(&r, &key, t0, t0 + 3_000, Resolution::Raw, &mut |it| {
            v.push(it);
            Ok(())
        })
        .unwrap();
        assert_eq!(v.len(), 4, "{v:?}");
        assert_eq!(
            v[2],
            RangeItem::Event {
                ts_ms: t0 + 1000,
                kind: lowmon_core::metrics::EventKind::AlarmClear,
                name: "ram".into(),
                value: 42
            }
        );
        let mut ev = Vec::new();
        query_events(&r, &key, t0, t0 + 3_000, &mut |it| {
            ev.push(it);
            Ok(())
        })
        .unwrap();
        assert_eq!(ev, [v[2].clone()], "yalnız olay");
        let _ = fs::remove_dir_all(&r);
    }
}
