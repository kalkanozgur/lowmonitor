# Changelog
Format: Keep a Changelog. Sürümleme: SemVer (CLAUDE.md > Build / test / CI).

## [Unreleased]
### Değişti
- Tray "Arayüzü aç": `UiExe` kaydı var ama exe yoksa (taşınmış/silinmiş) menü pasif ve etiket nedeni söyler; önceden tıklama sessizce boşa gidiyordu
### Eklendi
- `lowmon-deck` crate: LowDeck agent sunucusu kütüphane olarak (hub, dinleyiciler, UDP keşif, Core Audio/SMTC/kilit, pipe köprüsü); `start(cfg) -> Running`. Tray entegrasyonu Faz 2 (ADR-0031)
- `lowmon-net` (M6c): `/v1/info` (host, çekirdek, toplam RAM, disk modeli, arayüz adı); `lowmon-svc install` firewall kuralı ekler, `uninstall` siler (ADR-0025)
- `lowmon-net` (M6b): `lowmon-net pair` / `POST /v1/pair` eşleştirme, token (SHA-256 + DPAPI), Bearer / `?token=`, IP başına 1 s kısıt + 10 hata → 10 dk ban, Origin listesi, `unpair --all`, LAN IP'ye bind (ADR-0024)
- `lowmon-net` (M6a): `ws://127.0.0.1:47800/v1/live` 1 Hz JSON yayını, `rate_ms` 1000/2000/5000, ping/pong, en fazla 4 istemci; ayar `%APPDATA%\LowMonitor
et.toml` (ADR-0023)
- Lisans: MIT OR Apache-2.0
- `lowmon-proto` crate'i: metrik tipleri + pipe protokolü, yalnız serde + postcard (ADR-0022)
- Proje iskeleti, spesifikasyon, ADR-0001..0014, pre-push hook, tag tabanlı release workflow
- `lowmon-svc`: boş Windows servisi, `install | uninstall | start | stop` komutları
- `lowmon-svc`: `\\.\pipe\LowMonitor` sunucusu (Hello/Subscribe/Unsubscribe, canlı Sample/Top)
- `lowmon-tray`: CPU/RAM çubuk ikonu, tooltip, sağ tık menüsü, servis yoksa gri + 10 s'de yeniden bağlanma
- `lowmon-tray`: "Windows ile başlat" menü seçeneği ve `--autostart on|off` (HKCU Run)
- Pipe `Export` (admin) ve `lwm-dump --export`: servis loglarını CSV olarak dışa aktarma
- Pipe `QueryRange` (herkes): `Raw | Minute` çözünürlükte metrik geçmişi, tipli `Range` çerçeveleri (ADR-0020)
