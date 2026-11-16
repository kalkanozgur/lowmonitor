//! Tek sorgu: `cargo run -p lowmon-ai --example fetch [en|tr]`. Yalnız sayılar yazılır, sır yok.
//! Session yolu için C++ widget'ın `session.bin`'i de denenebilir: `LOWMON_AI_SESSION=<yol>`.
fn main() {
    let mut cfg = lowmon_ai::Config::load(&lowmon_ai::config_path()).expect("ai.toml");
    if let Some(l) = std::env::args().nth(1) {
        cfg.language = if l == "en" {
            lowmon_ai::text::Lang::En
        } else {
            lowmon_ai::text::Lang::Tr
        };
    }
    let session = std::env::var_os("LOWMON_AI_SESSION")
        .map(Into::into)
        .unwrap_or_else(|| lowmon_ai::data_dir().join("session.bin"));
    println!(
        "süreçler: {:?}",
        lowmon_ai::win::running(&cfg.desktop_path_hint)
    );
    let lang = cfg.language;
    let colors = cfg.colors;
    let mut f = lowmon_ai::fetch::Fetcher::new(cfg, session, |m| eprintln!("log: {m}"));
    let t0 = std::time::Instant::now();
    match f.fetch() {
        Ok((s, src)) => {
            let now = lowmon_ai::fetch::now_ms() / 1000;
            println!("kaynak {src:?}, {} ms", t0.elapsed().as_millis());
            println!("csv: {}", lowmon_ai::usage::csv_row(now, &s));
            let [a, b] = lowmon_ai::text::lines(&s, now, lang);
            println!(
                "{a}\n{b}\nseviye {:?}",
                lowmon_ai::usage::levels(&s, now, &colors, false)
            );
        }
        Err(e) => println!("hata: {e}"),
    }
}
