# 2026-11-16 — Tray + AI kullanımı + taskbar göstergesi (ADR-0032 Faz 2)

## Düzenek
- PC: XAN-LAPTOP, Windows 11 Pro 26300, 2 monitör (gösterge sağdaki 2560×1440 birincil taskbar'da).
- Önce: kurulu `lowmon-tray` (`be15cc6`, deck dahil). Sonra: `feat/ai-taskbar` release `lowmon-tray` (deck + `lowmon-ai` + `lowmon-taskbar`).
- Claude Desktop + Claude Code CLI açık → AI sorgusu etkin (180 s aralık). Telefon bağlı değil.
- Araç: `Get-Process` — private, working set, `TotalProcessorTime` farkı; 60 s boşta (yeni sürümde açılıştan 20 s sonra başlar).
- Karşılaştırma: eski C++ `claude-usage-bar.exe` (ayrı süreç, Faz 4'te kaldırılacak).

## Sonuç
| | Önce: tray | Sonra: tray (AI + taskbar) | C++ widget (ayrı süreç) |
|---|---|---|---|
| private | 3,13 MiB | 5,13 MiB | 5,13 MiB |
| working set | 20,64 MiB | 29,77 MiB | 31,01 MiB |
| CPU (60 s ort.) | 0,78 ms/s | 0,52 ms/s | 1,30 ms/s |
| thread | 11 | 17 | 6 |
| release exe | 675 328 B | 822 784 B (+147 456) | 733 696 B |

## Notlar
- Tray +2,0 MiB private, +6 thread (AI işçisi, GDI+ arka plan thread'i, WinHTTP havuzu). Exe +144 KiB (ADR tahmini +150–250 KB).
- CPU farkı gürültü düzeyinde; C++ widget'ın 1 s zamanlayıcısı (topmost yeniden öne alma + yoklama) yeni gösterge yokken bile 1,3 ms/s. Yeni gösterge yalnız olayda (konum değişimi) ve 30 s'de bir metin için çizer.
- Faz 4'te C++ widget kalkınca net: −5,13 MiB private, −1 süreç; tray tek başına AI dahil 5,13 MiB.
