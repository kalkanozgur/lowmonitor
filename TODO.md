# TODO
## Şimdi
- [x] Rust toolchain kur (rustup, 1.100.0-msvc)
- [x] M0: workspace, toolchain, crt-static, boş servis (install/start/stop/uninstall)

## Sonra
- [~] M1: CPU/RAM/Disk/Ağ/Top-N toplayıcı — 24 saatte CPU < %0.05, Private WS / Private Bytes < 3 MB — kod tamam, 24 saat soak bekliyor
- [~] M2: şifreli + zincirli log, seal, retention, `lwm-dump` — kod tamam; servis kurulum testi soak sonrası
- [~] M3: pipe protokolü + tray — canlı Sample + tray ikonu/tooltip/menü/yeniden bağlanma tamam; HKCU Run otomatik başlatma, QueryRange/Export, Admin kontrolü açık
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
