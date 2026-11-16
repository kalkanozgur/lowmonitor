# 2026-11-17 — lowmon-ui AI raporu (ADR-0032 Faz 5)

## Düzenek
- PC: XAN-LAPTOP, Windows 11 Pro 26300. `lowmon-ui`: `feat/ai-report`, `release-ui` profili (`opt-level = "s"`).
- Veri: gerçek `~/.claude/projects` (en yeni 30 döküm, subagent'lar dahil) + limit CSV'lerinin kopyası (2 dosya, 2355 satır); önbellek boş (soğuk).
- Araç: `Stopwatch` (süreç başlatma → `breakdown-cache.jsonl` yazıldı), `Get-Process`; `lowmon-ui --ai`.

## Sonuç
| | önce (`2026-11-16`, #37) | sonra |
|---|---|---|
| `lowmon-ui.exe` | 8 903 680 B | 9 168 896 B (+265 KiB) |
| private (AI sekmesi açık, rapor kurulu) | — | 6,3 MiB |
| Soğuk rapor (süreç başlangıcı dahil) | — | 771 ms, CPU 0,28 s |

## Notlar
- Rapor yalnız sekme açılınca / Yenile ile, arka iş parçacığında kurulur; sekme kapalıyken ek iş yok (timer yok). Sonraki kurulumlar önbellekten (değişen döküm yeniden ayrıştırılır).
- Tray'e kod eklenmedi (yalnız `ShellExecuteW` argümanı + menü satırı); tray ölçümü tekrarlanmadı.
