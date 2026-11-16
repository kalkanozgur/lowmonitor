# 2026-11-16 — lowmon-ui opt-level karşılaştırması

| Alan | Değer |
|---|---|
| Düzenek | Win 11 Pro 26300, kurulu tray (deck) + svc çalışıyor; ui explorer'dan açılır, Canlı sekmesi, 15 s ısınma |
| Araç | `measure.ps1` (TotalProcessorTime farkı, PrivateMemorySize64) |
| Süre | Varyant başına 120 s, tek koşu |

| Varyant | Exe (B) | CPU / 120 s | Private | Thread |
|---|---|---|---|---|
| O3 (`release`) | 10 521 600 | 78 ms | 5,16 MiB | 5 |
| Yalnız `lowmon-ui` crate'i "s" | 9 999 872 (−5 %) | 62 ms | 5,11 MiB | 5 |
| ui + tüm bağımlılıklar "s" (`release-ui`) | 8 903 680 (−15 %) | 125 ms | 5,28 MiB | 7 |

## Sonuç
- CPU farkları ≤ 1 ms/s, tek koşuda gürültü düzeyinde; private bellek aynı.
- Karar (kullanıcı): `release-ui` profili (`inherits = "release"`, `opt-level = "s"`) yalnız `lowmon-ui` için; svc/tray/net O3 kalır.
- Not: önceki ölçüm (656 ms / 120 s, `2026-11-16_lowmon-ui.md`) pencere öndeyken; bu koşuda pencere arkadaydı (çizim azaldı). Karşılaştırma aynı koşulda.
