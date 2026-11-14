# TODO
## Şimdi
- [x] Rust toolchain kur (rustup, 1.100.0-msvc)
- [x] M0: workspace, toolchain, crt-static, boş servis (install/start/stop/uninstall)

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
