//! Sensörleri 2 s'de bir yazar (ADR-0033 Faz 1 doğrulama + yük ölçümü).
//! `dump [saniye=10] [gpu] [cpu] [disk]`: kaynak adı verilmezse hepsi; verilirse yalnız onlar.
#[cfg(windows)]
fn main() {
    use lowmon_sensors::{Sensors, Sources};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(10);
    let picked: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| a.parse::<u64>().is_err())
        .collect();
    let on = |name| picked.is_empty() || picked.contains(&name);
    let src = Sources {
        gpu: on("gpu"),
        cpu_temp: on("cpu"),
        disk_temp: on("disk"),
    };
    let t0 = std::time::Instant::now();
    let mut s = Sensors::open(src);
    println!(
        "pid {} · {src:?} · açılış {} ms",
        std::process::id(),
        t0.elapsed().as_millis()
    );
    let show = |v: Option<i64>, unit| v.map_or("—".to_string(), |v| format!("{v} {unit}"));
    for _ in 0..secs.div_ceil(2).max(1) {
        let t = std::time::Instant::now();
        let r = s.read();
        println!(
            "GPU: {}  GPU: {}  CPU: {}  HDD: {}   ({} µs)",
            show(r.gpu_pct.map(i64::from), "%"),
            show(r.gpu_c.map(i64::from), "°C"),
            show(r.cpu_c.map(i64::from), "°C"),
            show(r.disk_c.map(i64::from), "°C"),
            t.elapsed().as_micros()
        );
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

#[cfg(not(windows))]
fn main() {}
