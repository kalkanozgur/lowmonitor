# TODO
## Şimdi
- [ ] Rust toolchain kur (rustup, stable-msvc) + MSVC Build Tools kontrolü
- [ ] M0: workspace `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, crate iskeletleri, boş servis (`install/start/stop/uninstall`)

## Sonra
- [ ] M1: CPU/RAM/Disk/Ağ/Top-N toplayıcı — 24 saatte CPU < %0.05, WS < 3 MB
- [ ] M2: şifreli + zincirli log, seal, retention, `lwm-dump`
- [ ] M3: pipe protokolü + tray
- [ ] M4: egui UI, geçmiş, müdahale + `lowmon-elev`
- [ ] M5: alarmlar + bildirim
- [ ] M6: `lowmon-net` (WS + eşleştirme + firewall)

## Fikirler / açık konular (spek §15)
- [ ] `lowmon-gpu` (NVML) tasarımı
- [ ] MQTT köprüsü: `lowmon-net` modülü mü, ayrı exe mi?
- [ ] `wss://` gerekli mi?
- [ ] `opt-level = "s"` vs `3`
- [ ] > 64 çekirdek (processor group)
- [ ] Kod imzalama
