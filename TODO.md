# TODO
## Şimdi
- [x] Rust toolchain kur (rustup, 1.100.0-msvc)
- [x] M0: workspace, toolchain, crt-static, boş servis (install/start/stop/uninstall)

- [ ] Soak (M3 exe `f8e1d7f`) bitişi ~2026-11-15 22:42 → `2026-11-14_m3-soak.csv` analizi, M1 kabul kriteri (CPU < %0,05, Private < 3 MB, sızıntı yok)
- [ ] Soak sonrası: aynı koşullarda `opt-level` 3 / "s" / "z" karşılaştırması (CPU + exe boyutu)
- [ ] Servis modunda ilk gece 03:00 retention çalışmasının sonucu (export ile kontrol)
- [ ] Dakikalık özet segmentinin gerçek boyutu (7 gün sonra veya elle tetikleyerek)
- [ ] Ctrl+C ile `run` modunda temiz kapanış testi
- [ ] Tray kurulumu: exe'yi `Program Files\LowMonitor`'a kopyalama + `--autostart on` (şu an `target\release`'ten çalışıyor)

- [ ] Slint UI pipe istemcisi: erişim modu ve bağlantı kuralları → [#1](https://github.com/kalkanozgur/lowmonitor/issues/1)

## Kısayollar (`shortcut:` borcu)
- state.bin atomik yazılmıyor; anahtar bellekten silinmiyor (zeroize yok)
- Hatalı config.toml sessizce varsayılana düşüyor; yazma/retention/pipe hataları loglanmıyor (Event M5'te)
- Export yazıcının bellekteki son ≤ 60 s'sini içermiyor
- Tray ikonu sabit 16 px (DPI); menüden autostart kayıt hatası sessiz
- VPN/vEthernet sayılmıyor; ≤ 64 çekirdek; testler %TEMP%'te `lowmon-test-*` bırakıyor

## Sonra
- [~] M1: CPU/RAM/Disk/Ağ/Top-N toplayıcı — 24 saatte CPU < %0.05, Private WS / Private Bytes < 3 MB — kod tamam, 24 saat soak bekliyor
- [~] M2: şifreli + zincirli log, seal, retention, `lwm-dump` — kod tamam; servis kurulum testi soak sonrası
- [x] M3: pipe protokolü + tray (canlı Sample, tray, HKCU Run, Admin kontrolü, Export)
- [ ] M4: protokol tarafı: `QueryRange`, `Control`, Export öncesi flush, `lowmon-elev` (UI Slint ile ayrıca, ADR-0019)
- [ ] M5: alarmlar + bildirim
- [ ] M6: `lowmon-net` (WS + eşleştirme + firewall), telefondan izleme (Mi Note 10 Lite istemcisi)

## Fikirler / açık konular (spek §15)
- [ ] `lowmon-gpu` (NVML) tasarımı
- [ ] MQTT köprüsü: `lowmon-net` modülü mü, ayrı exe mi?
- [ ] `wss://` gerekli mi?
- [ ] `opt-level = "s"` vs `3`
- [ ] > 64 çekirdek (processor group)
- [ ] Kod imzalama
