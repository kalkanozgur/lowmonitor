# Changelog
Format: Keep a Changelog. Sürümleme: SemVer (CLAUDE.md > Build / test / CI).

## [Unreleased]
### Eklendi
- Proje iskeleti, spesifikasyon, ADR-0001..0014, pre-push hook, tag tabanlı release workflow
- `lowmon-svc`: boş Windows servisi, `install | uninstall | start | stop` komutları
- `lowmon-svc`: `\\.\pipe\LowMonitor` sunucusu (Hello/Subscribe/Unsubscribe, canlı Sample/Top)
- `lowmon-tray`: CPU/RAM çubuk ikonu, tooltip, sağ tık menüsü, servis yoksa gri + 10 s'de yeniden bağlanma
