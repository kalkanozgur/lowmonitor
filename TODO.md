# TODO
## Şimdi
- [x] Rust toolchain kur (rustup, 1.100.0-msvc)
- [x] M0: workspace, toolchain, crt-static, boş servis (install/start/stop/uninstall)

- [~] 24 saat soak sürüyor: Task Scheduler "LowMonitor soak" (SYSTEM), `docs/olcumler/2026-11-15_m6d-soak.csv`, bitiş ~2026-11-16 11:13 (ilk koşu 2,76 sa'te kesildi, [kısmi ölçüm](docs/olcumler/2026-11-15_m3-kismi-soak.md)): yönetici pwsh ile `scripts/soak.ps1`, M1 kabulü (CPU < %0,05 toplam — ADR-0026, Private < 3 MB, sızıntı yok)
- [~] CPU: toplayıcılar ~0,5 ms/s (Top-N %83, [bench](docs/olcumler/2026-11-15_toplayici-bench.md)); servis 1,45–2,26 ms/s → ~1 ms/s fark açık. Soak sonrası EcoQoS açık/kapalı karşılaştır
- [ ] Soak sonrası: aynı koşullarda `opt-level` 3 / "s" / "z" karşılaştırması (CPU + exe boyutu)
- [ ] Servis modunda ilk gece 03:00 retention çalışmasının sonucu (export ile kontrol)
- [ ] Dakikalık özet segmentinin gerçek boyutu (7 gün sonra veya elle tetikleyerek)
- [ ] Ctrl+C ile `run` modunda temiz kapanış testi
- [x] Tray kurulumu: `scripts/install.ps1` Program Files'a kopyalıyor + `--autostart on` (M6d)

- [ ] Slint UI pipe istemcisi: erişim modu ve bağlantı kuralları → [#1](https://github.com/kalkanozgur/lowmonitor/issues/1)

## Kısayollar (`shortcut:` borcu)
- Firewall kuralı portu sabit 47800; `tokens.bin` atomik değil; istemcisiz `/v1/info` ağ thread'ini ≤ 1 örnek bloklar
- state.bin atomik yazılmıyor; anahtar bellekten silinmiyor (zeroize yok)
- Hatalı config.toml sessizce varsayılana düşüyor; yazma/retention/pipe hataları loglanmıyor (Event M5'te)
- Export yazıcının bellekteki son ≤ 60 s'sini içermiyor
- Tray ikonu sabit 16 px (DPI); menüden autostart kayıt hatası sessiz
- VPN/vEthernet sayılmıyor; ≤ 64 çekirdek; testler %TEMP%'te `lowmon-test-*` bırakıyor

## Sonra
- [~] M1: CPU/RAM/Disk/Ağ/Top-N toplayıcı — 24 saatte CPU < %0.05, Private WS / Private Bytes < 3 MB — kod tamam, 24 saat soak bekliyor
- [~] M2: şifreli + zincirli log, seal, retention, `lwm-dump` — kod tamam; servis kurulum testi soak sonrası
- [x] M3: pipe protokolü + tray (canlı Sample, tray, HKCU Run, Admin kontrolü, Export)
- [ ] M4: protokol tarafı: ~~`QueryRange`~~ (kod tamam, servise kurulmadı), `Control`, Export öncesi flush, `lowmon-elev` (UI Slint ile ayrıca, ADR-0019)
- [ ] M5: alarmlar + bildirim
- [~] M6: M6a (`/v1/live`), M6b (pair/token/ban/Origin, LAN bind), M6c (`/v1/info`, firewall) kod tamam; M6d HKCU Run/kurulum/ölçüm. `lowmon-net` spek §16 tam (WS + info + pair, token/DPAPI, ban, Origin, firewall, HKCU Run), PR'lara bölünür (M6a–d)
- [ ] LowDeck entegrasyonu (yol a, ADR-0022): `lowdeck-agent` pipe Viewer → telefona CPU/RAM/disk/ağ/Top-N; LowDeck'te ayrı oturum

## Fikirler / açık konular (spek §15)
- [ ] `lowmon-gpu` (NVML) tasarımı
- [ ] MQTT köprüsü: `lowmon-net` modülü mü, ayrı exe mi?
- [ ] `wss://` gerekli mi?
- [ ] `opt-level = "s"` vs `3`
- [ ] > 64 çekirdek (processor group)
- [ ] Kod imzalama
