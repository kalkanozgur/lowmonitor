//! Segment → CSV (lwm-dump ve servis `Export` paylaşır). 12 sütun; `info` içindeki virgül → `;`.
use crate::{
    codec::{AggKind, BlockDecoder, Record},
    crypto::{Header, MasterKey, Opener},
    metrics::Sample,
};
use std::io::{self, Write};

pub const HEADER: &str = "type,ts_unix_ms,cpu_pct,ram_used_kb,commit_kb,cache_kb,disk_r_bps,disk_w_bps,net_rx_bps,net_tx_bps,cores,info";

fn sample_row(kind: &str, s: &Sample) -> String {
    let cores: Vec<String> = s
        .cpu_core
        .iter()
        .map(|&c| format!("{:.1}", c as f32 / 2.0))
        .collect();
    format!(
        "{kind},{},{:.2},{},{},{},{},{},{},{},{},",
        s.ts_unix_ms,
        s.cpu_total as f32 / 100.0,
        s.ram_used_kb,
        s.commit_kb,
        s.cache_kb,
        s.disk.iter().map(|d| d.r_bps).sum::<u64>(),
        s.disk.iter().map(|d| d.w_bps).sum::<u64>(),
        s.net.iter().map(|n| n.rx_bps).sum::<u64>(),
        s.net.iter().map(|n| n.tx_bps).sum::<u64>(),
        cores.join(" "),
    )
}

/// Sample/Agg dışındaki satırlar: ts + `info` sütunu.
pub fn other_row(kind: &str, ts: u64, info: &str) -> String {
    format!("{kind},{ts},,,,,,,,,,{}", info.replace(',', ";"))
}

pub struct Outcome {
    pub header: Option<Header>,
    pub blocks: u64,
    /// Doğrulama/çözme hatası; öncesinde yazılan satırlar doğrulanmış veridir.
    pub error: Option<String>,
}

/// Segmenti baştan doğrular; zaman damgası `[from, to)` içindeki satırları yazar (başlık satırı hariç).
pub fn segment(
    master: &MasterKey,
    bytes: &[u8],
    from: u64,
    to: u64,
    out: &mut impl Write,
) -> io::Result<Outcome> {
    let fail = |header, e: String| {
        Ok(Outcome {
            header,
            blocks: 0,
            error: Some(e),
        })
    };
    let (header, mut opener) = match Opener::new(master, bytes) {
        Ok(x) => x,
        Err(e) => return fail(None, format!("başlık: {e:?}")),
    };
    let (mut names, mut last_ts) = (Vec::new(), 0);
    for block in opener.by_ref() {
        let block = match block {
            Ok(b) => b,
            Err(e) => return fail(Some(header), format!("blok doğrulama: {e:?}")),
        };
        for rec in BlockDecoder::new(&block, &mut names) {
            let (ts, row) = match rec {
                Err(e) => return fail(Some(header), format!("kayıt çözme: {e:?}")),
                Ok(Record::Sample(s)) => {
                    last_ts = s.ts_unix_ms;
                    (last_ts, sample_row("sample", &s))
                }
                Ok(Record::Agg { kind, sample }) => {
                    last_ts = sample.ts_unix_ms;
                    let k = match kind {
                        AggKind::Min => "min",
                        AggKind::Avg => "avg",
                        AggKind::Max => "max",
                    };
                    (last_ts, sample_row(k, &sample))
                }
                Ok(Record::Top(t)) => {
                    let info: Vec<String> = t
                        .iter()
                        .map(|e| {
                            format!(
                                "{}:{}:{:.2}%:{}KB",
                                e.name,
                                e.pid,
                                e.cpu as f32 / 100.0,
                                e.ws_kb
                            )
                        })
                        .collect();
                    (last_ts, other_row("top", last_ts, &info.join(" ")))
                }
                Ok(Record::Gap {
                    from_ms,
                    to_ms,
                    reason,
                }) => (
                    from_ms,
                    other_row("gap", from_ms, &format!("to={to_ms} reason={reason:?}")),
                ),
                Ok(Record::Seal {
                    block_count,
                    sample_count,
                    ..
                }) => (
                    last_ts,
                    other_row(
                        "seal",
                        last_ts,
                        &format!("blocks={block_count} samples={sample_count}"),
                    ),
                ),
            };
            if (from..to).contains(&ts) {
                writeln!(out, "{row}")?;
            }
        }
    }
    Ok(Outcome {
        header: Some(header),
        blocks: opener.head().0,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::{BlockEncoder, Dict, GapReason},
        crypto::Sealer,
        metrics::TopEntry,
    };

    const KEY: MasterKey = [1; 32];

    fn seg() -> Vec<u8> {
        let h = Header::new(1, 0, 1000, 1).unwrap();
        let mut file = h.to_bytes().to_vec();
        let (mut s, mut enc, mut dict) = (
            Sealer::new(&KEY, &h),
            BlockEncoder::default(),
            Dict::default(),
        );
        enc.sample(&Sample {
            ts_unix_ms: 1000,
            cpu_total: 1234,
            cpu_core: vec![3, 200],
            ..Default::default()
        });
        enc.top(
            &[TopEntry {
                pid: 7,
                cpu: 50,
                ws_kb: 9,
                name: "a,b.exe".into(),
            }],
            &mut dict,
        );
        enc.gap(1000, 5000, GapReason::Sleep);
        enc.sample(&Sample {
            ts_unix_ms: 6000,
            ..Default::default()
        });
        s.seal(&enc.take(), &mut file).unwrap();
        file
    }

    #[test]
    fn rows_and_range() {
        let mut out = Vec::new();
        let o = segment(&KEY, &seg(), 0, u64::MAX, &mut out).unwrap();
        assert_eq!((o.blocks, o.error), (1, None));
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "sample,1000,12.34,0,0,0,0,0,0,0,1.5 100.0,");
        assert_eq!(lines[1], "top,1000,,,,,,,,,,a;b.exe:7:0.50%:9KB"); // virgül CSV'yi bozmaz
        assert_eq!(lines[2], "gap,1000,,,,,,,,,,to=5000 reason=Sleep");
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l.split(',').count() == 12));

        let mut out = Vec::new();
        segment(&KEY, &seg(), 2000, 7000, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap().lines().count(),
            1,
            "yalnızca ts=6000"
        );
    }

    #[test]
    fn tamper_is_reported() {
        let mut s = seg();
        let n = s.len();
        s[n - 1] ^= 1;
        let o = segment(&KEY, &s, 0, u64::MAX, &mut Vec::new()).unwrap();
        assert!(o.error.unwrap().contains("Auth"));
    }
}
