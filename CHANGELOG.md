# Changelog
Format: Keep a Changelog. Sürümleme: SemVer (CLAUDE.md > Build / test / CI).

## [Unreleased]

## [1.2.1] - 2026-11-22
İkincil taskbar'da ses tekerleği düzeltmesi ve kod temizliği.
### Düzeltildi
- Taskbar tekerleği → ses, %100 dışı ölçekli (ör. %125 ikincil) monitörün taskbar'ında çalışmıyordu; `lowvolume` v0.1.1 (hook callback'inde DPI bağlamı)
### Değişti
- Tekrar eden kod birleştirildi (davranış aynı): süreç taraması `lowmon_core::procs` (svc Top-N + deck/ui Süreçler), `lowmon_core::time::now_ms`, tarih hesabı `civil`, tek `wide()` / toplam RAM okuması, `json::str_member`/`num_member`, tray `settings.json` yazımı `write_atomic`

## [1.2.0] - 2026-11-22
Taskbar tekerleğiyle ses kontrolü (lowvolume) ve Claude balonu düzeni.
### Eklendi
- Taskbar üstünde fare tekerleği → sistem sesi (Windows ses göstergesiyle), tray menüsünde "Ses (taskbar tekerleği) ▸"; `lowvolume` v0.1.0 kütüphanesi tray sürecinde, ayar `lowvolume.toml`. Standalone `lowvolume.exe` çalışıyorsa çakışmaz (ADR-0039)
### Değişti
- Claude balonu 3 satır: proje adının ilk 2 kelimesi / durum / kısa ayrıntı (≤ 32 karakter); tek satırdaki uzun proje adı balonu taşırıyordu

## [1.1.0] - 2026-11-20
Çoklu telefon, güvenlik istemi bildirimleri (UAC, güvenlik duvarı), Claude etkinlik ayrıntısı ve tek setup exe.
### Eklendi
- Tek setup exe (Inno Setup): `LowMonitor-Setup-<sürüm>.exe`, bileşenler açıklamalı ve varsayılan seçili, tray + taskbar zorunlu; kaldırma Windows "Uygulamalar"dan, `%ProgramData%` verisi korunur (ADR-0038)
- Claude Code balonu 2–3 satır: proje adı (`cwd`), güvenli özet (Bash açıklaması, dosya adı, desen; komut ve tam yol yok), diğer oturum sayısı (ADR-0034 eki)
- Claude girdi bekleyince (izin istemi, soru) `Claude.Input` alarmı: telefon alarm bandı + perde, LM UI alarm bandı
- Güvenlik duvarı izni penceresi bildirimi: aynı ayar (`[events] uac`) açıkken `PickerHost.exe` → `firewall` alarmı (telefon, tray, Geçmiş). Ayarlar etiketi "Güvenlik istemlerini bildir" (ADR-0037 eki)
- UAC istemi bildirimi: `config.toml` `[events] uac = true` iken servis ETW (Kernel-Process) ile `consent.exe`'yi izler; istem açıkken `uac` alarmı (Geçmiş, tray balonu, telefon alarm bandı / perde). Varsayılan kapalı; yalnız bildirim. `lowmon-ui` Ayarlar ▸ servis ayarlarında onay kutusu, servis yeniden başlamadan uygular (ADR-0037)
- Çoklu telefon: `lowmon-deck` LAN yuvası ≤ 4 telefonu aynı anda tutar; aynı IP'den yeniden bağlanan eski oturumunu kapatır, 5. reddedilir. Tray menüsünde bağlı telefon adları virgülle (ADR-0036)

## [1.0.0] - 2026-11-19
İlk sürüm: servis, tray, `lowmon-ui`, `lowmon-net`, telefon sunucusu (LowDeck v1.0.0 ile), taskbar göstergeleri, Claude kullanımı. Nihai 24 saat kabul soak'u bu sürümden sonra (TODO.md).
### Eklendi
- Tray sıcaklık alarmı: CPU/GPU `temp_warn_c`, disk `disk_warn_c` aşılınca deck'e `Temp.CPU|GPU|Disk` olayı, 3 °C altında temizlenir; telefon olaylara aboneyken sensörler okunur (LowDeck perdesi bu olaylarla açılır) (#69)
- Tray menüsü: "Taskbar: sistem göstergesi" / "Taskbar: AI göstergesi" aç-kapa (`taskbar.toml` `[sys|ai] enabled`, yorumlar korunur) (#68)
- Tray IP değişimini izler (`NotifyUnicastIpAddressChange`): `deck.toml` `bind` IP'si kaybolunca telefon dinleyicisi şu anki LAN IP'sine geçer, eski IP geri gelince ona döner (#67)
- README İngilizce + `README.tr.md`
- `lowmon-deck`: telefondan PC AI ayarı (LowDeck ADR-0010): `QueryAiConfig` / `SetAiConfig` → `ai.toml` (dil, sorgu aralığı, kimlik, CSV, rapor oturumu); yalnız değişen anahtar yazılır, tray yeniden yükler. `toml_edit` yazma çekirdeği `lowmon-ui`'dan deck'e taşındı (`tomlw`)
- `lowmon-deck` telefona lowmon-ui verileri (LowDeck ADR-0009, proto v4 içinde): süreç listesi (`TOPIC_PROCS`, ilk 50, sıralama/süzgeç sunucuda, abone yokken tarama yok), süreç sonlandırma (`Kill`, CreationTime + korumalı liste; yetki = token, UAC yok), sıcaklık/GPU geçmişi (`QueryTempHistory`, `sensors.bin`), AI raporu (`QueryAiReport`, 100 kova + ilk 8 grup + 20 oturum). Süreç tarama/sonlandırma kodu `lowmon-ui`'dan deck'e taşındı, arayüz aynı kodu kullanır
- `lowmon-ui` Geçmiş: CPU kartında °C çizgisi, yeni GPU kartı (% + °C), disk başlığında aralığın en yüksek °C'si; hover ort + max gösterir. Tray sensör iş parçacığı dakikalık ort + max'ı `%LOCALAPPDATA%\LowMonitor\sensors.bin`'e yazar (7 g sabit halka, 197 KiB; ADR-0035)
- `lowmon-ui` Canlı: CPU kartında sıcaklık çizgisi (°C), yeni GPU kartı (% + °C), disk kartı başlığında en sıcak disk °C. Kaynak tray sensörleri; deck yeni `TOPIC_SENSORS` konusuyla yayınlar (telefon da abone olabilir). Tray sensör iş parçacığı taskbar ızgarası kapalıyken de abone varsa çalışır (ADR-0033 Faz 4a, LowDeck #30)
- `lowmon-ui` Ayarlar sekmesi: `ai.toml`, `taskbar.toml`, `deck.toml` formu; yalnız değişen anahtar yazılır (yorumlar korunur), değerler doğrulanır, kaydedince tray yeniden yükler. Servis ayarları sonraki partide
- `lowmon-ui` servis ayarları: Ayarlar ▸ "Servis ayarlarını düzenle (yönetici)" → UAC → `lowmon-ui --service-settings`; örnekleme (`interval_ms`, `topn_every`, `topn_count`) ve saklama (`raw_days`, `total_days`) `config.toml`'a yerinde yazılır, servis `ReloadConfig` ile yeniden başlamadan uygular. `lowmon-elev` planı kaldırıldı (ADR-0030 eki)
### Düzeltildi
- Ayarlar: değişen anahtarın üstündeki yorum satırı siliniyordu (`toml_edit` `insert` anahtar süsünü atıyor → yerinde değiştirme)
- `lowmon-ui` testleri: dil süreç genelinde; paralel testler birbirinin dilini değiştirip ara sıra düşüyordu (kilit)
### Değişti
- `lowmon-ui` AI sekmesi: seçili görünümde kullanılmayan gruplar listeden kaybolmaz, gri kalır; LİMİT grafiğinde fareyle tarih-saat + oturum / hafta %
- `lowmon-ui` üst çubuk: Ayarlar sekmesi yerine sağ üstte ⚙; "Connected" yazısı yalnız sorun varken (ya da noktanın üzerine gelince) görünür. Olaylar her satırda okunur açıklama gösterir; Geçmiş'te aralık seçici grafiklerin altına taşındı
- `lowmon-ui` grafikleri: fareyle üzerine gelince dikey çizgi + kutucuk; Canlı'da saat + değer(ler), Geçmiş'te kova aralığı (dakikadan kısa kovada saniyeli, 7 g'de tarihli) + ort/max, RAM, R/W, ↓/↑
- Taskbar AI göstergesine sol tık artık raporu açar (önceden: şimdi yenile; menüde duruyor)
- `lowmon-ui` `release-ui` profiliyle (`opt-level = "s"`) derlenir: exe 10,5 → 8,9 MB; `install.ps1` `-UiSrc target\release-ui`
- Tray "Arayüzü aç": `UiExe` kaydı var ama exe yoksa (taşınmış/silinmiş) menü pasif ve etiket nedeni söyler; önceden tıklama sessizce boşa gidiyordu
### Eklendi
- Tray "AI kullanımı ▸ Claude Code hook'larını kur / kaldır": `~/.claude/settings.json`'a yalnız `lowmon-hook` girdilerini ekler / siler (`async`, anahtar sırası korunur, ilk yazmada `.lowmon-backup`, geçersiz JSON'a dokunmaz); `install.ps1` `lowmon-hook.exe`'yi kurar (ADR-0034 Parça 4)
- Claude Code etkinlik balonu: AI bloğunun üstünde yarı saydam konuşma balonu (`Thinking…`, araç adı, `Waiting for input`, `Done`); tam ekranda gizlenir; `taskbar.toml` `[ai] bubble`. Hook kurulumu Parça 4'te (ADR-0034 Parça 3)
- `lowmon-hook.exe` (Claude Code hook komutu) + tray alıcısı: hook olayı → `WM_COPYDATA` → etkinlik durumu; şimdilik yalnız `deck.log`'a yazılır, balon Parça 3'te (ADR-0034 Parça 2)
- `lowmon_ai::activity`: Claude Code hook olaylarından etkinlik durumu (çalışıyor / bekliyor / bitti), oturum önceliği, bayat kayıt; henüz hiçbir yere bağlı değil (ADR-0034 Parça 1)
- İngilizce arayüz: `ai.toml` `language = "en"` artık tray menüsünü, tooltip / bildirim / ileti kutularını ve `lowmon-ui`'ın tüm sekmelerini de İngilizce yapar (önceden yalnız taskbar metni); sayılar dile göre biçimlenir (ADR-0032 Dil kapsamı)
- Taskbar blokları monitör başına: sistem ızgarası varsayılan olarak ikinci monitörün taskbar'ında (yoksa AI'ın yanında); `taskbar.toml` `[sys] monitor` / `[ai] monitor` (`other` dahil). Monitör takılıp çıkarılınca bloklar yeniden kurulur (ADR-0033 Faz 3)
- Taskbar sistem ızgarası: `CPU / GPU / MEM / ↑` ve `CPU °C / GPU °C / HDD °C / ↓` (2 × 4), AI bloğunun sağında; eşik renkleri, sol tık `lowmon-ui`. `lowmon-taskbar` çok bloklu hücre ızgarası oldu; `taskbar.toml` `[sys]` / `[ai]` (ADR-0033 Faz 2)
- `lowmon-sensors` crate: GPU % + °C (D3DKMT, üretici DLL'i yok), disk °C, CPU °C (ACPI termal bölge); taskbar sistem ızgarası için, tray entegrasyonu Faz 2 (ADR-0033 Faz 1)
- `lowmon-ui` "AI" sekmesi: Claude limit çizgisi, token dökümü (araç grubu başına), oturum tablosu; Son oturum / Bugün / Bu hafta. Taskbar göstergesine sol tık ve "AI kullanımı ▸ Rapor" bu sekmeyi açar (`lowmon-ui --ai`). `ai.toml`: `report_sessions`, `summary_dir` (ADR-0032 Faz 5)
- Tray "AI kullanımı ▸": "Panodan sessionKey kaydet" (DPAPI, pano boşaltılır) / "sessionKey'i sil". `install.ps1` eski C++ ai-usage-bar'ı kapatır (süreç, `ClaudeUsageBar` autostart), limit CSV'lerini ve `session.bin`'i `%LOCALAPPDATA%\LowMonitor\ai`'ye taşır, eski klasörü siler (ADR-0032 Faz 4)
- AI kullanımı telefonda (LowDeck "AI" bölümü) ve `lowmon-ui` Canlı sekmesinde (bant): deck `TOPIC_AI` yayını, son durum önbellekli; `lowdeck-proto` rev `ec2ce3d` (ADR-0032 Faz 3)
- Tray: Claude kullanımı taskbar'da (`Oturum %33 · 4s16dk` / `Hafta %68 · …`, tempo rengi), "AI kullanımı ▸" menüsü, tooltip'te `AI %33/%68`. Yeni `lowmon-taskbar` crate: taskbar'a gömülü gösterge (alt pencere + GDI+, yoklama yok); ayar `%APPDATA%\LowMonitor\taskbar.toml` (ADR-0032 Faz 2)
- `lowmon-ai` crate: Claude abonelik kullanımı (oturum + hafta %, reset, tempo seviyesi), Claude Code OAuth / sessionKey, CSV geçmişi; ai-usage-bar'ın Rust portu, tray entegrasyonu Faz 2 (ADR-0032 Faz 1)
- `install.ps1`: "LowMonitor deck" firewall kuralı (TCP deck portu + UDP 47811) ve eski LowDeck PC izlerinin temizliği (agent/desktop süreçleri, `LowDeckAgent` autostart, `UiExe`, "LowDeck agent" kuralı) (ADR-0031 Faz 4)
- `lowmon-ui`: PC arayüzü (Canlı, Olaylar, Geçmiş, Süreçler) bu repoda; LowDeck masaüstünden taşındı, veri tray'deki deck'ten, ayar `deck.toml`. Tray "Arayüzü aç" yanındaki `lowmon-ui.exe`'yi açar, `install.ps1` onu da kurar (ADR-0031 Faz 3)
- `lowmon-tray` telefon sunucusunu (`lowmon-deck`) başlatır: "Telefon ▸" menüsü, tooltip'te bağlı telefon, `%APPDATA%\LowMonitor\deck.toml` (ilk açılışta LowDeck `agent.cfg`'den taşınır). Port kullanımdaysa tray çalışmaya devam eder (ADR-0031 Faz 2)
- `lowmon-deck` crate: LowDeck agent sunucusu kütüphane olarak (hub, dinleyiciler, UDP keşif, Core Audio/SMTC/kilit, pipe köprüsü); `start(cfg) -> Running`. Tray entegrasyonu Faz 2 (ADR-0031)
- `lowmon-net` (M6c): `/v1/info` (host, çekirdek, toplam RAM, disk modeli, arayüz adı); `lowmon-svc install` firewall kuralı ekler, `uninstall` siler (ADR-0025)
- `lowmon-net` (M6b): `lowmon-net pair` / `POST /v1/pair` eşleştirme, token (SHA-256 + DPAPI), Bearer / `?token=`, IP başına 1 s kısıt + 10 hata → 10 dk ban, Origin listesi, `unpair --all`, LAN IP'ye bind (ADR-0024)
- `lowmon-net` (M6a): `ws://127.0.0.1:47800/v1/live` 1 Hz JSON yayını, `rate_ms` 1000/2000/5000, ping/pong, en fazla 4 istemci; ayar `%APPDATA%\LowMonitor\net.toml` (ADR-0023)
- Lisans: MIT OR Apache-2.0
- `lowmon-proto` crate'i: metrik tipleri + pipe protokolü, yalnız serde + postcard (ADR-0022)
- Proje iskeleti, spesifikasyon, ADR-0001..0014, pre-push hook, tag tabanlı release workflow
- `lowmon-svc`: boş Windows servisi, `install | uninstall | start | stop` komutları
- `lowmon-svc`: `\\.\pipe\LowMonitor` sunucusu (Hello/Subscribe/Unsubscribe, canlı Sample/Top)
- `lowmon-tray`: CPU/RAM çubuk ikonu, tooltip, sağ tık menüsü, servis yoksa gri + 10 s'de yeniden bağlanma
- `lowmon-tray`: "Windows ile başlat" menü seçeneği ve `--autostart on|off` (HKCU Run)
- Pipe `Export` (admin) ve `lwm-dump --export`: servis loglarını CSV olarak dışa aktarma
- Pipe `QueryRange` (herkes): `Raw | Minute` çözünürlükte metrik geçmişi, tipli `Range` çerçeveleri (ADR-0020)
