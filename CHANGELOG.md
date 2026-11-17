# Changelog
Format: Keep a Changelog. Sürümleme: SemVer (CLAUDE.md > Build / test / CI).

## [Unreleased]
### Değişti
- Taskbar AI göstergesine sol tık artık raporu açar (önceden: şimdi yenile; menüde duruyor)
- `lowmon-ui` `release-ui` profiliyle (`opt-level = "s"`) derlenir: exe 10,5 → 8,9 MB; `install.ps1` `-UiSrc target\release-ui`
- Tray "Arayüzü aç": `UiExe` kaydı var ama exe yoksa (taşınmış/silinmiş) menü pasif ve etiket nedeni söyler; önceden tıklama sessizce boşa gidiyordu
### Eklendi
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
