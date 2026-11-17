# 2026-11-17 — Tray: taskbar sistem ızgarası (ADR-0033 Faz 2)

## Düzenek
- PC: XAN-LAPTOP, Windows 11 Pro 26300; Ryzen 7 5800H, RTX 3060 Laptop. Şebekede; Claude masaüstü + tarayıcı açık, derleme yok.
- Önce: kurulu tray (`a5d0dba`; deck + AI bloğu). Sonra: `feat/taskbar-sys` release build, explorer üzerinden başlatıldı (AI + sistem bloğu, sensör iş parçacığı, PDH açık).
- Araç: `Get-Process` farkı, 90 s; `lowmon-ui` açık (deck'e bağlı), telefon bağlı değil.

## Sonuç
| | private | working set | CPU / 90 s | iş parçacığı | exe |
|---|---|---|---|---|---|
| Önce | 6,74 MiB | 40,92 MiB | 94 ms (1,04 ms/s) | 16 | 836 608 B |
| Sonra | 9,25 MiB | 36,32 MiB | 156 ms (1,74 ms/s) | 18 | 879 104 B |
| Fark | +2,51 MiB | | +0,7 ms/s | +2 | +42 KiB |

## Notlar
- CPU süresi 15,6 ms çözünürlüklü (94 ms = 6 tik, 156 ms = 10 tik): fark ±0,2 ms/s belirsiz. Uzun (≥ 10 dk) ölçüm yapılmadı.
- Bellek farkı Faz 1 tahmininden (+4,4 MiB) küçük: "önce" değeri 40 dk çalışmış süreçten, "sonra" 10 s'lik süreçten.
- Artışın kaynağı (Faz 1 ölçümü): PDH ~4 MiB; GPU okuması ~1,6 ms / 2 s; iki blok çizimi (GDI+) 2 s'de bir, yalnız içerik değişince.
- Azaltma düğmeleri: `[sys] cpu_temp = false` (PDH yüklenmez), `[sys] enabled = false` (sensör iş parçacığı ve blok yok).
