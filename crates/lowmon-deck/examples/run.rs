//! Tray'siz deneme (Faz 1 doğrulaması): `cargo run -p lowmon-deck --example run -- 127.0.0.1 47820 dev`
//! İstemci: LowDeck `cargo run -p lowdeck-client --example cli -- 127.0.0.1 47820 dev`. Debug build log'u stdout'a yazar.

#[cfg(windows)]
fn main() {
    let mut a = std::env::args().skip(1);
    let cfg = lowmon_deck::Config {
        bind: a.next().unwrap_or_else(|| "127.0.0.1".into()),
        port: a.next().and_then(|s| s.parse().ok()).unwrap_or(47820),
        token: a.next().unwrap_or_else(|| "dev".into()),
    };
    let _running = lowmon_deck::start(cfg).unwrap_or_else(|e| panic!("{e}"));
    loop {
        std::thread::park();
    }
}

#[cfg(not(windows))]
fn main() {}
