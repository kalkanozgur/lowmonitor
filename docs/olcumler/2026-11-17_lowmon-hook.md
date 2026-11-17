# 2026-11-17 — lowmon-hook başlatma maliyeti (ADR-0034 Parça 2)

## Düzenek
- PC: XAN-LAPTOP, Windows 11 Pro 26300; Ryzen 7 5800H. Şebekede; Claude masaüstü açık, derleme yok.
- `tools/lowmon-hook` release build (209 920 B). Her çalıştırma ayrı süreç: `System.Diagnostics.Process`, stdin yönlendirilmiş, hook JSON'u yazılıp kapatılır, çıkış beklenir.
- Ölçülen: duvar süresi (başlat → çıkış) ve sürecin CPU süresi (`TotalProcessorTime`). Araç: `scratchpad/hookbench.ps1` (PowerShell 7).

## Sonuç
| Durum | n | duvar ort. | ortanca | p95 | CPU ort. |
|---|---|---|---|---|---|
| hook, tray kapalı (ileti gitmez) | 100 | 13,8 ms | 13,2 | 14,3 | 7,97 ms |
| hook, tray açık (ileti işlenir) | 100 | 13,1 ms | 13,2 | 14,3 | 7,34 ms |
| hook, 1 MB girdi (`tool_response`) | 30 | 53,1 ms | 52,4 | 57,0 | 8,33 ms |
| referans: `cmd /c exit` | 100 | 31,8 ms | 31,2 | 37,2 | 12,50 ms |
| referans: `lowmon-tray.exe` başlat + çık | 30 | 17,5 ms | 17,2 | 20,0 | 17,19 ms |

Tray tarafı: 230 ileti için toplam 31 ms CPU (≈ 0,13 ms / ileti; 15,6 ms çözünürlükte iki tik).

## Notlar
- Maliyetin neredeyse tamamı Windows'un süreç başlatması; hook'un kendi işi (8 KB okuma, tarama, bir `SendMessage`) ölçüm gürültüsünün altında. Tray'in açık / kapalı olması fark etmiyor.
- Ayrı küçük exe, tray exe'sini başlatmaya göre CPU'da ~2,3 kat ucuz (7,3 ms / 17,2 ms): ADR'deki "ayrı exe" kararını doğruluyor.
- 1 MB girdide duvar süresi artıyor (boru üzerinden okuma), CPU artmıyor.
- Araç çağrısı başına 2 olay ≈ 16 ms CPU. Dakikada 30 araç çağrısı → ~0,5 s CPU / dk ≈ 8 ms/s ≈ 16 iş parçacığının %0,05'i.
- Ölçülmedi: Claude Code hook komutunu bir kabukla sarıyorsa onun maliyeti (referans `cmd /c exit` ≈ 12,5 ms CPU) eklenir; Parça 4'te gerçek oturumda bakılır.
