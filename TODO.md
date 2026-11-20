# TODO
## Şimdi
- [x] v1.0.0 (2026-11-19): sürüm numarası, CHANGELOG, README EN + TR; LowDeck v1.0.0 ile birlikte
- [ ] **Sonraki sürüm (kullanıcı, 2026-11-19): tek exe kurulum.** `install.ps1` + elle build yerine tek kurulum exe'si (GitHub Release'e eklenir):
  - Modül listesi, her biri kısa açıklamalı ve **varsayılan seçili**: servis (`lowmon-svc`), arayüz (`lowmon-ui`), LAN API (`lowmon-net`), Claude Code hook (`lowmon-hook`), `lwm-dump`, telefon firewall kuralı (TCP deck + UDP 47811)
  - **Tray + taskbar zorunlu** (seçimden çıkarılamaz, gri işaretli)
  - Mevcut `install.ps1` davranışı korunur: süreçleri kapat, `Program Files\LowMonitor`'a kopyala, servis kur/başlat, firewall, tray autostart, eski LowDeck / ai-usage-bar izleri, `%ProgramData%` verisi korunur
  - Açık kararlar (ADR gerekir): araç (Inno Setup / WiX / kendi Rust kurucusu), UAC tek sefer, kaldırma (uninstall) ve güncelleme yolu, MSIX AppData sanallaştırma tuzağı (HKCU yazmaları yükseltilmemiş bağlamda)
- [x] **Çoklu telefon (2026-11-20, ADR-0036):** `lowmon-deck` LAN yuvası ≤ 4 telefon, aynı IP eskisini kapatır, doluysa ret; `phone_wants_events` herhangi biri. Canlı ✅ 2026-11-20 (iki telefon 192.168.1.177 + .140 birlikte bağlı, döngü yok)
- [x] **UAC istemi bildirimi (2026-11-20, ADR-0037):** canlı ✅ (LocalService ek yetkisiz ETW açtı; istem → telefon bildirimi; olay kaydında `uac` kalk/in, kapanıştan ~3 s sonra iner). Kalan: açıkken CPU ölçümü; `lowmon-ui` servis ayarlarında aç/kapa ✅ (ReloadConfig ile canlı)
- [ ] **Sonra (kullanıcı, 2026-11-20): Home Assistant olay bildirimi.** Opsiyonel modül, feature flag ile kapalı başlar; kapalıyken idle tüketim değişmez, çekirdek logger tüketimi artmaz. Kod yazmadan önce plan + dosya listesi + onay; yeni bağımlılık gerekçeli.
  - Kapsam: (1) UAC istemi algılama: ETW `Microsoft-Windows-Kernel-Process`, `consent.exe` başla/bitir (polling yok); (2) eşik alarmları CPU/RAM/disk/sıcaklık, config'ten, debounce/cooldown; (3) çıkış: HA webhook HTTP POST `http://<HA_IP>:8123/api/webhook/<id>` (URL config'te), JSON `{event, ts, detail}`, bağlantı yalnız olay anında. MQTT sonra.
  - Kapsam dışı: firewall dialog algılama (süreç henüz tespit edilmedi), telefon istemcisi, GPU.
  - Ön analiz (2026-11-20): eşik + debounce/cooldown **zaten var** (servis `lowmon-core` `AlarmRule`, ADR-0028; sıcaklık tray `lowmon-tray/src/sys.rs` `temp_edges`) → yeni motor değil, mevcut olay akışına (`Hub::push_event`) webhook çıkışı. HTTP istemcisi **zaten var** (`lowmon-ai` WinHTTP, `crates/lowmon-ai/src/win.rs`) → yeni bağımlılık yok; en doğal yer tray (servisin LocalService ağ yetkisine dokunulmaz).
  - Açık kararlar (ADR): ETW gerçek zamanlı oturumu yönetici / "Performance Log Users" ister — servis LocalService (ADR-0004), tray yükseltilmemiş → servis SID'ine grup, ayrı yükseltilmiş süreç ya da SYSTEM görev? Feature flag = Cargo feature mı config anahtarı mı (ikisi)? Spek §"Sonra" `MQTT → Home Assistant` satırıyla birleşir
- [ ] v1 sonrası canlı doğrulama: tray taskbar aç-kapa menüsü (#68), tray çalışırken IP değişimi (#67), sıcaklık alarmı (#69; eşiği geçici düşürerek), telefon perdesinin alarmla / arada açılması (LowDeck #40)
- [ ] Sıcaklık alarmları servis olay deposuna yazılmıyor (Geçmiş'te yok); gerekirse servis tarafı sensör kuralı (yeni ADR)
- [x] Rust toolchain kur (rustup, 1.100.0-msvc)
- [x] M0: workspace, toolchain, crt-static, boş servis (install/start/stop/uninstall)

- [~] 24 saat soak sürüyor (**taban çizgisi**, `b34ee5b`; nihai kabul soak'u M5 dahil sürüm adayında): Task Scheduler "LowMonitor soak" (SYSTEM), `docs/olcumler/2026-11-15_m6d-soak.csv`, bitiş ~2026-11-16 11:13 (ilk koşu 2,76 sa'te kesildi, [kısmi ölçüm](docs/olcumler/2026-11-15_m3-kismi-soak.md)): yönetici pwsh ile `scripts/soak.ps1`, M1 kabulü (CPU < %0,05 toplam — ADR-0026, Private < 3 MB, sızıntı yok)
- [~] CPU: toplayıcılar ~0,5 ms/s (Top-N %83, [bench](docs/olcumler/2026-11-15_toplayici-bench.md)); servis 1,45–2,26 ms/s → ~1 ms/s fark açık. Soak sonrası EcoQoS açık/kapalı karşılaştır
- [ ] Soak sonrası: aynı koşullarda `opt-level` 3 / "s" / "z" karşılaştırması (CPU + exe boyutu)
- [ ] Servis modunda ilk gece 03:00 retention çalışmasının sonucu (export ile kontrol)
- [ ] Dakikalık özet segmentinin gerçek boyutu (7 gün sonra veya elle tetikleyerek)
- [ ] Ctrl+C ile `run` modunda temiz kapanış testi
- [x] Tray kurulumu: `scripts/install.ps1` Program Files'a kopyalıyor + `--autostart on` (M6d)

- [x] PC tarafı birleşmesi (ADR-0031, 2026-11-16): `lowmon-deck` (#31), tray içinde deck + "Telefon ▸" + `deck.toml` göçü (#33), `lowmon-ui` 4 sekme (#34), `install.ps1` deck firewall + eski LowDeck temizliği (#35), LowDeck'ten agent/masaüstü silindi (LowDeck #28). "Arayüzü aç" gerçek menüden doğrulandı ✅; pipe istemci kuralları → [#1](https://github.com/kalkanozgur/lowmonitor/issues/1) (artık deck için)
- [x] Telefonla gerçek bağlantı (2026-11-16, Samsung M21 `toco`, LowDeck APK `c7ef7f6`): keşif (UDP 47811) ✅, LAN bağlantısı (token göçü, ret yok) ✅, ses +/− ✅, play/pause ✅ (kullanıcı), PC paneli ✅
- [ ] Telefon → PC komut gecikmesi < 50 ms ölçülmedi (deck komutları loglamıyor; PC → telefon ping ort. 83 ms, telefon Wi-Fi güç tasarrufu şüphesi)
- [ ] `lowmon-ui` Süreçler: UAC ile sonlandırma (`--kill`, başka kullanıcı/SYSTEM süreci) canlı doğrulanmadı (2026-11-16 atlandı; öneri: SYSTEM zamanlanmış görevde `ping -t`)
- [ ] `lowdeck-proto` rev'i: LowDeck'te proto değişirse `lowmon-deck`/`lowmon-ui` git rev'i güncellenir; `lowmon-proto` `PROTO` artarsa deck `servis: BadHello` loglar
- [x] Kurulu `lowmon-ui` `release-ui` profilinde (8 903 680 B; `install.ps1`, 2026-11-16 17:04)

- [ ] ADR-0032 AI kullanımı: ~~Faz 1 `lowmon-ai`~~ ✅ → ~~2 `lowmon-taskbar` + tray~~ ✅ → ~~3 telefon/ui~~ ✅ (gerçek telefon APK'sı bekliyor) → ~~4 kurulum/geçiş + sessionKey menüsü~~ ✅ (kurulu build yenilenince geçiş canlı çalışır) → ~~5 rapor (`lowmon-ui` AI sekmesi)~~ ✅. Kurulum yenilendi 2026-11-17 (eski widget kapandı). Kalan: gerçek telefon APK'sı, tray sol tık / sessionKey menüsü fareyle
- [ ] Tray 2026-11-17 00:10 yeniden başlatmadan sonra çalışmıyordu (Run kaydı `LowMonitorTray` yerinde; neden bilinmiyor, elle açıldı). Bir sonraki açılışta izle
- [ ] Rafta (2026-11-17, kullanıcı): `lowmon-ui` veriyi doğrudan servisten (`lowmon-svc` pipe) alsın; şu an yalnız tray'deki deck üzerinden (ADR-0031), tray kapalıyken boş. Seçenekler: hibrit (metrik/olay/geçmiş pipe, AI bandı deck) ya da tamamen doğrudan; yeni ADR gerekir
- [ ] `install.ps1` Claude masaüstü oturumundan çalıştırılınca HKCU ve AppData yazmaları sanallaşıyor (2026-11-17: `ClaudeUsageBar` Run kaydı gerçekte silinmemiş, eski widget açılışta geri gelmiş; elle düzeltildi). Kurulumu normal yönetici PowerShell'inden çalıştır ya da betiğe uyarı ekle
- [ ] ADR-0034 Claude Code etkinlik balonu (parçalar ADR'de): ~~0 ADR taslağı~~ ✅ → ~~1 `lowmon_ai::activity` durum makinesi~~ ✅ → ~~2 `lowmon-hook` exe + tray alıcısı + ölçüm~~ ✅ → ~~3 balon penceresi~~ ✅ → ~~4 hook kur / kaldır menüsü + `install.ps1` + spek~~ ✅ (kurulumdan sonra menüden "Claude Code hook'larını kur"). Sonra (ayrı karar): telefon, IoT, RGB
- [ ] ADR-0033 taskbar sistem ızgarası: ~~Faz 1 `lowmon-sensors` (D3DKMT GPU, disk °C, ACPI termal bölge + ölçüm)~~ ✅ → ~~2 çok bloklu `lowmon-taskbar` + tray~~ ✅ → ~~3 diğer monitör~~ ✅ (monitör tak / çıkar canlı denenmedi) → 4 geçmiş/telefon (ayrı karar)
- [x] Dil: tray menüsü + `lowmon-ui` en/tr (`ai.toml` `language`, 2026-11-17). Çevrilmeyen: `lowmon-deck` / servis hata metinleri, loglar, telefon uygulaması
  - Faz 2 canlı doğrulanmadı: explorer yeniden başlatma, bildirim alanı genişleyince yeniden konum, açık tema, gösterge tıkları gerçek fareyle

## Kısayollar (`shortcut:` borcu)
- Firewall kuralı portu sabit 47800; `tokens.bin` atomik değil; istemcisiz `/v1/info` ağ thread'ini ≤ 1 örnek bloklar
- state.bin atomik yazılmıyor; anahtar bellekten silinmiyor (zeroize yok)
- ~~Export yazıcının bellekteki son ≤ 60 s'sini içermiyor~~ → M4 flush (döngü ≤ 1 s içinde, 3 s zaman aşımı); QueryRange hâlâ son ≤ 60 s'siz (her kullanıcı tetikleyebileceği için flush yok)
- Tray ikonu sabit 16 px (DPI); menüden autostart kayıt hatası sessiz
- `install.ps1`: farklı admin hesabıyla yükseltilirse HKCU/`%APPDATA%` (autostart, UiExe temizliği, deck.toml portu) o hesabınki
- VPN/vEthernet sayılmıyor; ≤ 64 çekirdek; testler %TEMP%'te `lowmon-test-*` bırakıyor

## Sonra
- [~] M1: CPU/RAM/Disk/Ağ/Top-N toplayıcı — 24 saatte CPU < %0.05, Private WS / Private Bytes < 3 MB — kod tamam, 24 saat soak bekliyor
- [~] M2: şifreli + zincirli log, seal, retention, `lwm-dump` — kod tamam; servis kurulum testi soak sonrası
- [x] M3: pipe protokolü + tray (canlı Sample, tray, HKCU Run, Admin kontrolü, Export)
- [~] M4: `QueryRange` ✅, Export öncesi flush ✅, `Control` Pause/Resume/ReloadConfig ✅ (2026-11-15). ReloadConfig'te çalışırken `interval_ms` + `lwm-dump` etiketi ✅. Kalan: servis ayarları `lowmon-ui --service-settings` (ADR-0030 eki; `lowmon-elev` iptal), uçtan uca Admin testi (soak sonrası). UI `lowmon-ui` (ADR-0031)
- [~] M5: alarmlar + bildirim — tasarım ✅ [ADR-0028](docs/kararlar/ADR-0028-alarm-ve-event.md) (2026-11-15); kod: config ayrıştırma + durum makinesi + tablo testleri ✅ (adım 1); REC_EVENT (SCHEMA 2) ✅ (adım 2); döngü + sistem olayları ✅ (adım 3); pipe CH_EVENT/Alarms ✅ (adım 4a); tray balonu ✅ (adım 4b); lowmon-net `alarms` ✅ (adım 4c); Event yolu balonu canlı görüldü ✅; kalan: bağlanış balonu canlı gözlemi, nihai kabul soak'u, lowmon-net `alarms`
- [~] M6: M6a–c kod tamam; M6d kurulum (`install.ps1`, firewall) + **telefon kabul testi ✅ 2026-11-15** (Samsung M21, LAN). Kalan: `lowmon-net` HKCU Run/otomatik başlatma (şimdilik elle, konsol), uzun süreli ölçüm; `bind` sabit IP → DHCP rezervasyonu
- [x] LowDeck entegrasyonu (yol a, ADR-0022; ADR-0031 ile deck tray'de): `lowdeck-agent` pipe Viewer → telefona CPU/RAM/disk/ağ/Top-N; LowDeck'te ayrı oturum — telefonda doğrulandı 2026-11-15

## Fikirler / açık konular (spek §15)
- [ ] Rafta (2026-11-15, LowDeck'ten): `lowmon-ui` Süreçler'de öncelik / EcoQoS / affinity
- [ ] `lowmon-gpu` (NVML) tasarımı
- [ ] MQTT köprüsü: `lowmon-net` modülü mü, ayrı exe mi?
- [ ] `wss://` gerekli mi?
- [~] `opt-level = "s"` vs `3`: `lowmon-ui` → `release-ui` ("s", [ölçüm](docs/olcumler/2026-11-16_lowmon-ui-opt.md)); svc/tray soak sonrası
- [ ] > 64 çekirdek (processor group)
- [ ] Kod imzalama
