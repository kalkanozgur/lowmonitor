//! LowMonitor pipe istemcisi (ADR-0002 "Metrikler", seçenek a).
//!
//! Telefon `Subscribe(TOPIC_METRICS)` dediğinde `\\.\pipe\LowMonitor`'a Viewer olarak
//! bağlanır, `Sample` + `Top`'u kompakt `lowdeck_proto` tiplerine çevirip push eder.
//! Abone yoksa pipe kapalı, thread `Want` koşul değişkeninde uyur (sıfır uyanma).
//! Servis yoksa abonelik sürdükçe 10 s'de bir yeniden dener (LowMonitor tray gibi).

use crate::Hub;
use lowdeck_proto::{EVENT_NAME_MAX, EventItem, HistPoint, Metrics, S2C, TopItem};
use lowmon_proto::metrics::{AggKind, EventKind};
use lowmon_proto::metrics::{Sample, TopEntry};
use lowmon_proto::proto::RangeItem;
use std::sync::Arc;
use std::time::Duration;

/// LowMonitor örnek aralığı 1 s; telefona 2 s yeterli (yarı trafik).
const RATE_MS: u32 = 2000;
const RETRY: Duration = Duration::from_secs(10);
const TOP_N: usize = 5;
/// Ad kırpılır: çerçeve sınırı (4 KB) asla aşılmaz, telefonda zaten elide.
const NAME_MAX: usize = 32;

fn kbps(bytes_per_s: u64) -> u32 {
    (bytes_per_s / 1024).min(u32::MAX as u64) as u32
}

pub fn metrics_of(s: &Sample, ram_total_kb: u64) -> Metrics {
    Metrics {
        cpu: s.cpu_total,
        cores: s.cpu_core.clone(),
        ram_used_mb: (s.ram_used_kb / 1024) as u32,
        ram_total_mb: (ram_total_kb / 1024) as u32,
        disk_r_kbps: kbps(s.disk.iter().map(|d| d.r_bps).sum()),
        disk_w_kbps: kbps(s.disk.iter().map(|d| d.w_bps).sum()),
        net_rx_kbps: kbps(s.net.iter().map(|n| n.rx_bps).sum()),
        net_tx_kbps: kbps(s.net.iter().map(|n| n.tx_bps).sum()),
    }
}

/// LowMonitor olayı → kompakt öğe; ad kırpılır (çerçeve sınırı).
pub fn event_of(ts_ms: u64, kind: EventKind, name: &str, value: i64) -> EventItem {
    let kind = match kind {
        EventKind::AlarmRaise => 0,
        EventKind::AlarmClear => 1,
        EventKind::System => 2,
    };
    EventItem {
        ts_ms,
        kind,
        name: name.chars().take(EVENT_NAME_MAX).collect(),
        value,
    }
}

/// CPU'ya göre azalan ilk `TOP_N`.
pub fn top_of(t: &[TopEntry]) -> Vec<TopItem> {
    let mut v: Vec<&TopEntry> = t.iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.cpu));
    v.iter()
        .take(TOP_N)
        .map(|e| TopItem {
            name: e.name.chars().take(NAME_MAX).collect(),
            cpu: e.cpu,
            ws_mb: (e.ws_kb / 1024) as u32,
        })
        .collect()
}

/// Bundan uzun aralıkta dakika özeti istenir (LM spek §9.3).
pub const MINUTE_ABOVE_MS: u64 = 6 * 3_600_000;

#[derive(Default, Clone, Copy)]
struct Acc {
    n: u64,
    cpu: u64,
    ram_kb: u64,
    dr: u64,
    dw: u64,
    rx: u64,
    tx: u64,
    /// `Max` özeti ya da ham örneklerin en büyüğü; yoksa ortalama kullanılır.
    max: Option<u16>,
}

/// Geçmiş kovaları (ADR-0005): pipe `Range` öğeleri akış halinde eklenir, bellek = kova sayısı.
pub struct Buckets {
    from: u64,
    step: u64,
    acc: Vec<Acc>,
}

impl Buckets {
    pub fn new(from_ms: u64, to_ms: u64, n: u16) -> Self {
        let n = u64::from(n.max(1));
        let step = to_ms.saturating_sub(from_ms).div_ceil(n).max(1);
        Self {
            from: from_ms,
            step,
            acc: vec![Acc::default(); n as usize],
        }
    }

    fn slot(&mut self, ts_ms: u64) -> Option<&mut Acc> {
        let i = ts_ms.checked_sub(self.from)? / self.step;
        self.acc.get_mut(usize::try_from(i).ok()?)
    }

    /// `Sample` ortalamaya ve max'a; `Agg{Avg}` ortalamaya; `Agg{Max}` yalnız CPU max'a. Gap/Event/Min yok sayılır.
    pub fn add(&mut self, item: &RangeItem) {
        let (s, avg, max) = match item {
            RangeItem::Sample(s) => (s, true, true),
            RangeItem::Agg {
                kind: AggKind::Avg,
                sample,
            } => (sample, true, false),
            RangeItem::Agg {
                kind: AggKind::Max,
                sample,
            } => (sample, false, true),
            _ => return,
        };
        let Some(a) = self.slot(s.ts_unix_ms) else {
            return;
        };
        if avg {
            a.n += 1;
            a.cpu += u64::from(s.cpu_total);
            a.ram_kb += s.ram_used_kb;
            a.dr += s.disk.iter().map(|d| d.r_bps).sum::<u64>();
            a.dw += s.disk.iter().map(|d| d.w_bps).sum::<u64>();
            a.rx += s.net.iter().map(|n| n.rx_bps).sum::<u64>();
            a.tx += s.net.iter().map(|n| n.tx_bps).sum::<u64>();
        }
        if max {
            a.max = Some(a.max.unwrap_or(0).max(s.cpu_total));
        }
    }

    /// (step_ms, kovalar); ortalaması olmayan kova `None`.
    pub fn finish(self) -> (u64, Vec<Option<HistPoint>>) {
        let v = self
            .acc
            .iter()
            .map(|a| {
                let n = a.n;
                (n > 0).then(|| {
                    let cpu_avg = (a.cpu / n) as u16;
                    HistPoint {
                        cpu_avg,
                        cpu_max: a.max.unwrap_or(cpu_avg).max(cpu_avg),
                        ram_mb: (a.ram_kb / n / 1024) as u32,
                        disk_r_kbps: kbps(a.dr / n),
                        disk_w_kbps: kbps(a.dw / n),
                        net_rx_kbps: kbps(a.rx / n),
                        net_tx_kbps: kbps(a.tx / n),
                    }
                })
            })
            .collect();
        (self.step, v)
    }
}

#[cfg(windows)]
pub fn start(hub: Arc<Hub>) {
    let _ = std::thread::Builder::new()
        .name("lowdeck-lowmon".into())
        .stack_size(64 * 1024)
        .spawn(move || run(&hub));
}

#[cfg(not(windows))]
pub fn start(_: Arc<Hub>) {}

/// Pipe'ı aç; tüm örnekler meşgulse (ERROR_PIPE_BUSY 231: abonelik + sorgu aynı anda)
/// 100 ms arayla ≤ 10 kez yeniden dener. Servis yoksa (2) hemen döner.
#[cfg(windows)]
fn open_pipe() -> std::io::Result<std::fs::File> {
    let open = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lowmon_proto::proto::PIPE_NAME)
    };
    for _ in 0..10 {
        match open() {
            Err(e) if e.raw_os_error() == Some(231) => {
                std::thread::sleep(Duration::from_millis(100))
            }
            r => return r,
        }
    }
    open()
}

#[cfg(windows)]
fn run(hub: &Hub) {
    loop {
        hub.want.wait_on();
        let r = open_pipe().and_then(|f| session(f, hub));
        // Abonelik bittiyse sessizce bekle; sürüyorsa servis yok/koptu.
        if hub.want.get() {
            if let Err(e) = r {
                log!("lowmon: {e}");
            }
            hub.push_metrics(S2C::Metrics(None));
            hub.set_alarms(Vec::new()); // bilinmiyor; yeniden bağlanınca pipe `Alarms` gönderir
            hub.want.sleep_while_on(RETRY);
        }
    }
}

/// Abonelik bitince bir sonraki çerçevede (≤ RATE_MS) döner; `File` düşünce pipe kapanır.
#[cfg(windows)]
fn session(mut f: std::fs::File, hub: &Hub) -> std::io::Result<()> {
    use lowmon_proto::proto::{
        self, CH_EVENT, CH_SAMPLE, CH_TOP, ClientMsg, PROTO, Role, ServerMsg,
    };
    proto::write(
        &mut f,
        &ClientMsg::Hello {
            proto: PROTO,
            role: Role::Viewer,
        },
    )?;
    proto::write(
        &mut f,
        &ClientMsg::Subscribe {
            channels: CH_SAMPLE | CH_TOP | CH_EVENT,
            rate_ms: RATE_MS,
        },
    )?;
    let total = ram_total_kb();
    while hub.want.get() {
        match proto::read::<ServerMsg>(&mut f)? {
            ServerMsg::Sample(s) => hub.push_metrics(S2C::Metrics(Some(metrics_of(&s, total)))),
            ServerMsg::Top(t) => hub.push_metrics(S2C::Top(top_of(&t))),
            ServerMsg::Alarms(a) => hub.set_alarms(a),
            ServerMsg::Event {
                ts_ms,
                kind,
                name,
                value,
            } => hub.push_event(event_of(ts_ms, kind, &name, value)),
            ServerMsg::Error { code } => {
                return Err(std::io::Error::other(format!("servis: {code:?}")));
            }
            _ => {} // Ack
        }
    }
    Ok(())
}

/// Pipe `QueryEvents` (LM `3241c3a`): ayrı kısa bağlantı, yalnız olaylar. Servis yok / hata → `Err`.
// shortcut: üst sınır yok; aylık olay sayısı küçük (alarm geçişi + dakikada ≤ 1 sistem olayı), gerekirse sınır eklenir
#[cfg(windows)]
pub fn query_events(from_ms: u64, to_ms: u64) -> std::io::Result<Vec<EventItem>> {
    use lowmon_proto::proto::{self, ClientMsg, PROTO, RangeItem, Role, ServerMsg};
    let mut f = open_pipe()?;
    proto::write(
        &mut f,
        &ClientMsg::Hello {
            proto: PROTO,
            role: Role::Viewer,
        },
    )?;
    proto::write(&mut f, &ClientMsg::QueryEvents { from_ms, to_ms })?;
    let (mut out, mut acks) = (Vec::new(), 0);
    loop {
        match proto::read::<ServerMsg>(&mut f)? {
            ServerMsg::Range(items) => out.extend(items.into_iter().filter_map(|it| match it {
                RangeItem::Event {
                    ts_ms,
                    kind,
                    name,
                    value,
                } => Some(event_of(ts_ms, kind, &name, value)),
                _ => None,
            })),
            // İlk Ack = Hello yanıtı, ikincisi sorgu sonu
            ServerMsg::Ack => {
                acks += 1;
                if acks == 2 {
                    return Ok(out);
                }
            }
            ServerMsg::Error { code } => {
                return Err(std::io::Error::other(format!("servis: {code:?}")));
            }
            _ => {}
        }
    }
}

/// Pipe `QueryRange` → kovalar (ADR-0005). Ayrı kısa bağlantı; > 6 sa dakika özeti. Servis yok / hata → `Err`.
#[cfg(windows)]
pub fn query_history(
    from_ms: u64,
    to_ms: u64,
    n: u16,
) -> std::io::Result<(u64, Vec<Option<HistPoint>>)> {
    use lowmon_proto::proto::{self, ClientMsg, PROTO, Resolution, Role, ServerMsg};
    let resolution = if to_ms.saturating_sub(from_ms) > MINUTE_ABOVE_MS {
        Resolution::Minute
    } else {
        Resolution::Raw
    };
    let mut f = open_pipe()?;
    proto::write(
        &mut f,
        &ClientMsg::Hello {
            proto: PROTO,
            role: Role::Viewer,
        },
    )?;
    proto::write(
        &mut f,
        &ClientMsg::QueryRange {
            from_ms,
            to_ms,
            resolution,
        },
    )?;
    let (mut b, mut acks) = (Buckets::new(from_ms, to_ms, n), 0);
    loop {
        match proto::read::<ServerMsg>(&mut f)? {
            ServerMsg::Range(items) => items.iter().for_each(|i| b.add(i)),
            // İlk Ack = Hello yanıtı, ikincisi sorgu sonu
            ServerMsg::Ack => {
                acks += 1;
                if acks == 2 {
                    return Ok(b.finish());
                }
            }
            ServerMsg::Error { code } => {
                return Err(std::io::Error::other(format!("servis: {code:?}")));
            }
            _ => {}
        }
    }
}

/// Sample'da toplam yok; fiziksel RAM oturumda değişmez, bağlantı başına bir kez.
#[cfg(windows)]
fn ram_total_kb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: m yerel ve dwLength doğru; fonksiyon yalnız m'ye yazar.
    unsafe { GlobalMemoryStatusEx(&mut m) }.map_or(0, |_| m.ullTotalPhys / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowmon_proto::metrics::{DiskIo, NetIo};

    #[test]
    fn convert() {
        let s = Sample {
            cpu_total: 1234,
            cpu_core: vec![10, 200],
            ram_used_kb: 8 * 1024 * 1024,
            disk: vec![
                DiskIo {
                    drive: 0,
                    r_bps: 1024,
                    w_bps: 0,
                },
                DiskIo {
                    drive: 1,
                    r_bps: 1024,
                    w_bps: 4096,
                },
            ],
            net: vec![NetIo {
                if_index: 3,
                rx_bps: 10 * 1024,
                tx_bps: 1023,
            }],
            ..Default::default()
        };
        let m = metrics_of(&s, 16 * 1024 * 1024);
        assert_eq!(
            m,
            Metrics {
                cpu: 1234,
                cores: vec![10, 200],
                ram_used_mb: 8192,
                ram_total_mb: 16384,
                disk_r_kbps: 2,
                disk_w_kbps: 4,
                net_rx_kbps: 10,
                net_tx_kbps: 0,
            }
        );
        let e = |name: &str, cpu| TopEntry {
            pid: 1,
            cpu,
            ws_kb: 2048,
            name: name.into(),
        };
        let long = "x".repeat(100);
        let t = top_of(&[
            e("a", 1),
            e(&long, 9),
            e("c", 5),
            e("d", 0),
            e("e", 3),
            e("f", 2),
        ]);
        assert_eq!(t.len(), TOP_N);
        assert_eq!((t[0].name.len(), t[0].cpu, t[0].ws_mb), (NAME_MAX, 9, 2));
        assert_eq!(t.iter().map(|i| i.cpu).collect::<Vec<_>>(), [9, 5, 3, 2, 1]);
    }

    #[test]
    fn buckets() {
        let smp = |ts, cpu, ram_kb, r_bps| Sample {
            ts_unix_ms: ts,
            cpu_total: cpu,
            ram_used_kb: ram_kb,
            disk: vec![DiskIo {
                drive: 0,
                r_bps,
                w_bps: 0,
            }],
            ..Default::default()
        };
        // 0..4000 ms, 4 kova (1 s): kova 0 iki örnek, kova 1 boş (gap), kova 2 Agg, kova 3 yalnız Max (ortalama yok)
        let mut b = Buckets::new(0, 4000, 4);
        b.add(&RangeItem::Sample(smp(0, 1000, 2048, 2048)));
        b.add(&RangeItem::Sample(smp(500, 3000, 4096, 0)));
        b.add(&RangeItem::Gap {
            from_ms: 1000,
            to_ms: 2000,
            reason: lowmon_proto::metrics::GapReason::Paused,
        });
        b.add(&RangeItem::Agg {
            kind: AggKind::Avg,
            sample: smp(2000, 2000, 1024, 0),
        });
        b.add(&RangeItem::Agg {
            kind: AggKind::Max,
            sample: smp(2000, 9000, 0, 0),
        });
        b.add(&RangeItem::Agg {
            kind: AggKind::Min,
            sample: smp(2000, 1, 0, 0),
        });
        b.add(&RangeItem::Agg {
            kind: AggKind::Max,
            sample: smp(3000, 5000, 0, 0),
        });
        b.add(&RangeItem::Sample(smp(4000, 100, 0, 0))); // aralık dışı
        let (step, v) = b.finish();
        assert_eq!(step, 1000);
        let p0 = v[0].unwrap();
        assert_eq!(
            (p0.cpu_avg, p0.cpu_max, p0.ram_mb, p0.disk_r_kbps),
            (2000, 3000, 3, 1)
        );
        assert_eq!(v[1], None);
        assert_eq!((v[2].unwrap().cpu_avg, v[2].unwrap().cpu_max), (2000, 9000));
        assert_eq!(v[3], None);
    }

    #[test]
    fn bucket_step_rounds_up() {
        let (step, v) = Buckets::new(0, 1001, 10).finish();
        assert_eq!((step, v.len()), (101, 10));
        let (step, _) = Buckets::new(5, 5, 0).finish();
        assert_eq!(step, 1);
    }

    #[test]
    fn event_kinds_and_name_limit() {
        let e = event_of(7, EventKind::AlarmClear, &"y".repeat(100), -3);
        assert_eq!(
            (e.ts_ms, e.kind, e.name.len(), e.value),
            (7, 1, EVENT_NAME_MAX, -3)
        );
        assert_eq!(event_of(0, EventKind::AlarmRaise, "a", 0).kind, 0);
        assert_eq!(event_of(0, EventKind::System, "a", 0).kind, 2);
    }
}
