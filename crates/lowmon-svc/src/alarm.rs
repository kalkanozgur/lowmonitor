//! Alarm motoru (spek §7, ADR-0028): histerezis + süre, `Normal → Pending → Raised → Clearing → Normal`.
//! Süre örnek ts'iyle ölçülür (aralık çalışırken değişebilir, ADR-0027); tick sayılmaz.
use lowmon_core::{
    config::{AlarmRule, Metric},
    metrics::Sample,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Normal,
    Pending { since: u64 },
    Raised,
    Clearing { since: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Raise,
    Clear,
}

/// Değer çıkarmak için örnekte olmayan bilgiler (serviste açılışta + keşifte doldurulur).
#[derive(Debug, Default, Clone)]
pub struct Ctx {
    pub ram_total_kb: u64,
    pub commit_limit_kb: u64,
    /// (ifIndex, takma ad)
    pub aliases: Vec<(u32, String)>,
}

fn pct(part: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| part as f64 * 100.0 / total as f64)
}

/// Kuralın bu örnekteki değeri; cihaz bulunamazsa / toplam bilinmiyorsa `None` (durum donar).
pub fn value(rule: &AlarmRule, s: &Sample, ctx: &Ctx) -> Option<f64> {
    let dev = rule.device.as_deref();
    let disk = |f: fn(u64, u64) -> u64| -> Option<f64> {
        let want = match dev {
            None => None,
            Some(d) => Some(d.parse::<u32>().ok()?), // sayı değil → değer yok
        };
        let mut it = s
            .disk
            .iter()
            .filter(|d| want.is_none_or(|w| d.drive == w))
            .peekable();
        it.peek()?; // istenen disk yok (ya da ilk örnek)
        Some(it.map(|d| f(d.r_bps, d.w_bps)).sum::<u64>() as f64)
    };
    let net = |f: fn(u64, u64) -> u64| -> Option<f64> {
        let want = match dev {
            None => None,
            Some(a) => Some(ctx.aliases.iter().find(|(_, n)| n == a)?.0),
        };
        let mut it = s
            .net
            .iter()
            .filter(|n| want.is_none_or(|w| n.if_index == w))
            .peekable();
        it.peek()?;
        Some(it.map(|n| f(n.rx_bps, n.tx_bps)).sum::<u64>() as f64)
    };
    match rule.metric {
        Metric::CpuTotal => Some(s.cpu_total as f64 / 100.0),
        Metric::MemUsedPct => pct(s.ram_used_kb, ctx.ram_total_kb),
        Metric::MemCommitPct => pct(s.commit_kb, ctx.commit_limit_kb),
        Metric::DiskR => disk(|r, _| r),
        Metric::DiskW => disk(|_, w| w),
        Metric::DiskRw => disk(|r, w| r + w),
        Metric::NetRx => net(|r, _| r),
        Metric::NetTx => net(|_, t| t),
        Metric::NetRxTx => net(|r, t| r + t),
    }
}

pub struct Alarm {
    pub rule: AlarmRule,
    pub state: State,
}

impl Alarm {
    pub fn new(rule: AlarmRule) -> Self {
        Self {
            rule,
            state: State::Normal,
        }
    }

    /// Bir örnek ilerletir; `v = None` → durum donar.
    pub fn step(&mut self, ts: u64, v: Option<f64>) -> Option<Transition> {
        let v = v?;
        let (raise, clear) = (&self.rule.raise, &self.rule.clear);
        self.state = match self.state {
            State::Normal if raise.hit(v) => State::Pending { since: ts },
            State::Pending { .. } if !raise.hit(v) => State::Normal,
            State::Raised if clear.hit(v) => State::Clearing { since: ts },
            // Clear eşiğinden çıktı (geri yükseldi) → süre baştan
            State::Clearing { .. } if !clear.hit(v) => State::Raised,
            s => s,
        };
        let due = |since: u64, for_s: u32| ts.saturating_sub(since) >= for_s as u64 * 1000;
        match self.state {
            State::Pending { since } if due(since, raise.for_s) => {
                self.state = State::Raised;
                Some(Transition::Raise)
            }
            State::Clearing { since } if due(since, clear.for_s) => {
                self.state = State::Normal;
                Some(Transition::Clear)
            }
            _ => None,
        }
    }

    /// Gap (pause, uyku, > 5 aralık): yarım kalan süreler geçersiz; Raised korunur.
    pub fn gap(&mut self) {
        self.state = match self.state {
            State::Pending { .. } => State::Normal,
            State::Clearing { .. } => State::Raised,
            s => s,
        };
    }

    pub fn raised(&self) -> bool {
        matches!(self.state, State::Raised | State::Clearing { .. })
    }
}

/// Geçiş olayı: (kural id, geçiş, değer).
pub type Fired = (String, Transition, f64);

#[derive(Default)]
pub struct Engine {
    alarms: Vec<Alarm>,
}

impl Engine {
    pub fn new(rules: &[AlarmRule]) -> Self {
        Self {
            alarms: rules.iter().cloned().map(Alarm::new).collect(),
        }
    }

    pub fn step(&mut self, s: &Sample, ctx: &Ctx, out: &mut Vec<Fired>) {
        out.clear();
        for a in &mut self.alarms {
            let v = value(&a.rule, s, ctx);
            if let Some(t) = a.step(s.ts_unix_ms, v) {
                out.push((a.rule.id.clone(), t, v.unwrap_or_default()));
            }
        }
    }

    pub fn gap(&mut self) {
        self.alarms.iter_mut().for_each(Alarm::gap);
    }

    /// Şu an Raised (ya da Clearing) olan kural id'leri.
    // shortcut: `ServerMsg::Alarms` anlık listesi M5 adım 4'te; o zaman allow kalkar
    #[allow(dead_code)]
    pub fn active(&self) -> Vec<String> {
        self.alarms
            .iter()
            .filter(|a| a.raised())
            .map(|a| a.rule.id.clone())
            .collect()
    }

    /// ReloadConfig: aynen kalan kuralın durumu korunur, değişen sıfırlanır.
    /// Dönüş: artık Raised olmayan (kaldırılan/değişen) kural id'leri → `AlarmClear` yazılır.
    pub fn reload(&mut self, rules: &[AlarmRule]) -> Vec<String> {
        let mut old = std::mem::take(&mut self.alarms);
        self.alarms = rules
            .iter()
            .map(|r| match old.iter().position(|a| &a.rule == r) {
                Some(i) => old.swap_remove(i),
                None => Alarm::new(r.clone()),
            })
            .collect();
        old.into_iter()
            .filter(Alarm::raised)
            .map(|a| a.rule.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowmon_core::{
        config::{Cond, Config},
        metrics::{DiskIo, NetIo},
    };

    fn rule(
        id: &str,
        raise_above: f64,
        raise_for: u32,
        clear_below: f64,
        clear_for: u32,
    ) -> AlarmRule {
        AlarmRule {
            id: id.into(),
            metric: Metric::CpuTotal,
            device: None,
            raise: Cond {
                above: Some(raise_above),
                below: None,
                for_s: raise_for,
            },
            clear: Cond {
                above: None,
                below: Some(clear_below),
                for_s: clear_for,
            },
        }
    }

    /// (ts_s, değer) dizisini sürer, geçişleri (ts_s, geçiş) olarak döner.
    fn run(a: &mut Alarm, seq: &[(u64, Option<f64>)]) -> Vec<(u64, Transition)> {
        seq.iter()
            .filter_map(|&(t, v)| a.step(t * 1000, v).map(|x| (t, x)))
            .collect()
    }

    fn steady(from: u64, to: u64, v: f64) -> Vec<(u64, Option<f64>)> {
        (from..=to).map(|t| (t, Some(v))).collect()
    }

    #[test]
    fn raise_after_for_s_then_clear_with_hysteresis() {
        let mut a = Alarm::new(rule("c", 90.0, 30, 80.0, 10));
        let mut seq = steady(0, 30, 95.0); // 0..30 s üstte → 30. s'de Raise
        seq.extend(steady(31, 40, 85.0)); // histerezis bandı: Raised kalır
        seq.extend(steady(41, 51, 70.0)); // 41'de Clearing, 51'de Clear
        assert_eq!(
            run(&mut a, &seq),
            [(30, Transition::Raise), (51, Transition::Clear)]
        );
        assert_eq!(a.state, State::Normal);
    }

    #[test]
    fn short_spike_does_not_raise() {
        let mut a = Alarm::new(rule("c", 90.0, 30, 80.0, 10));
        let mut seq = steady(0, 20, 95.0);
        seq.push((21, Some(50.0))); // düşüş sayacı sıfırlar
        seq.extend(steady(22, 50, 95.0)); // 22 + 30 = 52'ye kadar Raise yok
        assert!(run(&mut a, &seq).is_empty());
        assert_eq!(a.state, State::Pending { since: 22_000 });
    }

    #[test]
    fn clearing_resets_when_value_returns_to_band() {
        let mut a = Alarm::new(rule("c", 90.0, 0, 80.0, 10));
        let mut seq = vec![(0, Some(95.0))]; // for_s 0 → hemen
        seq.extend(steady(1, 5, 70.0)); // Clearing
        seq.push((6, Some(85.0))); // bant → Raised, sayaç sıfır
        seq.extend(steady(7, 16, 70.0)); // 7 + 10 = 17'ye kadar Clear yok
        assert_eq!(run(&mut a, &seq), [(0, Transition::Raise)]);
        assert_eq!(a.state, State::Clearing { since: 7_000 });
        assert_eq!(run(&mut a, &[(17, Some(70.0))]), [(17, Transition::Clear)]);
    }

    #[test]
    fn missing_value_freezes_and_gap_drops_partial_timers() {
        let mut a = Alarm::new(rule("c", 90.0, 30, 80.0, 10));
        run(&mut a, &steady(0, 10, 95.0));
        assert!(run(&mut a, &[(50, None)]).is_empty()); // değer yok → donar, Raise yok
        assert_eq!(a.state, State::Pending { since: 0 });
        a.gap();
        assert_eq!(a.state, State::Normal);
        a.state = State::Clearing { since: 0 };
        a.gap();
        assert_eq!(a.state, State::Raised); // Raised korunur
    }

    #[test]
    fn low_value_alarm() {
        let mut r = rule("lo", 0.0, 5, 0.0, 5);
        r.raise = Cond {
            above: None,
            below: Some(10.0),
            for_s: 5,
        };
        r.clear = Cond {
            above: Some(20.0),
            below: None,
            for_s: 5,
        };
        let mut a = Alarm::new(r);
        let mut seq = steady(0, 5, 5.0);
        seq.extend(steady(6, 11, 25.0));
        assert_eq!(
            run(&mut a, &seq),
            [(5, Transition::Raise), (11, Transition::Clear)]
        );
    }

    #[test]
    fn values_by_metric_and_device() {
        let s = Sample {
            cpu_total: 1250,
            ram_used_kb: 750,
            commit_kb: 200,
            disk: vec![
                DiskIo {
                    drive: 0,
                    r_bps: 10,
                    w_bps: 1,
                },
                DiskIo {
                    drive: 2,
                    r_bps: 20,
                    w_bps: 2,
                },
            ],
            net: vec![NetIo {
                if_index: 7,
                rx_bps: 100,
                tx_bps: 5,
            }],
            ..Default::default()
        };
        let ctx = Ctx {
            ram_total_kb: 1000,
            commit_limit_kb: 0, // bilinmiyor
            aliases: vec![(7, "Ethernet 2".into())],
        };
        let v = |m: Metric, d: Option<&str>| {
            let mut r = rule("x", 1.0, 0, 0.0, 0);
            (r.metric, r.device) = (m, d.map(Into::into));
            value(&r, &s, &ctx)
        };
        assert_eq!(v(Metric::CpuTotal, None), Some(12.5));
        assert_eq!(v(Metric::MemUsedPct, None), Some(75.0));
        assert_eq!(v(Metric::MemCommitPct, None), None);
        assert_eq!(v(Metric::DiskRw, None), Some(33.0));
        assert_eq!(v(Metric::DiskW, Some("2")), Some(2.0));
        assert_eq!(v(Metric::DiskR, Some("1")), None); // yok
        assert_eq!(v(Metric::DiskR, Some("x")), None); // sayı değil
        assert_eq!(v(Metric::NetRxTx, Some("Ethernet 2")), Some(105.0));
        assert_eq!(v(Metric::NetRx, Some("Wi-Fi")), None);
    }

    #[test]
    fn engine_reload_keeps_unchanged_and_reports_dropped() {
        let (a, b) = (rule("a", 90.0, 0, 80.0, 0), rule("b", 50.0, 0, 40.0, 0));
        let mut e = Engine::new(&[a.clone(), b.clone()]);
        let mut out = Vec::new();
        let s = Sample {
            ts_unix_ms: 1000,
            cpu_total: 9500,
            ..Default::default()
        };
        e.step(&s, &Ctx::default(), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(e.active(), ["a", "b"]);
        let mut b2 = b.clone();
        b2.raise.above = Some(60.0); // değişti → sıfırlanır
        assert_eq!(e.reload(&[a.clone(), b2]), ["b"]);
        assert_eq!(e.active(), ["a"]);
        assert_eq!(e.reload(&[]), ["a"]);
    }

    #[test]
    fn default_config_rules_run() {
        let c = Config::parse(lowmon_core::config::DEFAULT_TOML).unwrap();
        let mut e = Engine::new(&c.alarm);
        let ctx = Ctx {
            ram_total_kb: 100,
            ..Default::default()
        };
        let mut out = Vec::new();
        let mut fired = Vec::new();
        for t in 0..=60 {
            let s = Sample {
                ts_unix_ms: t * 1000,
                cpu_total: 9900,
                ram_used_kb: 95,
                ..Default::default()
            };
            e.step(&s, &ctx, &mut out);
            fired.extend(out.iter().map(|(id, tr, _)| (t, id.clone(), *tr)));
        }
        assert_eq!(
            fired,
            [
                (30, "cpu_high".into(), Transition::Raise),
                (60, "ram_high".into(), Transition::Raise)
            ]
        );
    }
}
