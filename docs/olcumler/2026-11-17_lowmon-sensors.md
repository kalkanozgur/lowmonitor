# 2026-11-17 — lowmon-sensors (ADR-0033 Faz 1)

## Düzenek
- PC: XAN-LAPTOP, Windows 11 Pro 26300; Ryzen 7 5800H, RTX 3060 Laptop + AMD tümleşik GPU, Micron NVMe SSD. Şebekede, boşta (arka planda derleme yok).
- `crates/lowmon-sensors/examples/dump` release build (238 080 B), 2 s'de bir okuma, 60 s; her kaynak ayrı süreçte, aynı anda.
- Araç: `Get-Process` (private, working set, iş parçacığı), okuma süresi örnek içinde `Instant` (duvar süresi).
- CPU süresi raporlanmadı: 60 s'de 16–62 ms, zamanlayıcı çözünürlüğü (15,6 ms) düzeyinde; tray'de Faz 2'de önce-sonra ölçülecek.

## Sonuç
| Kaynak | private | iş parçacığı | açılış | ilk okuma | sonraki okuma (ort.) |
|---|---|---|---|---|---|
| hiçbiri (taban) | 1,07 MiB | 1 | 0 ms | — | — |
| GPU, D3DKMT | 1,06 MiB (+0) | 1 | 3 ms | 1,4 ms | 1,6 ms |
| CPU °C, PDH termal bölge | 5,29 MiB (+4,2) | 3 | 287 ms | 0,3 ms | 0,14 ms |
| disk °C | 1,06 MiB (+0) | 1 | 0 ms | 23 ms | sorgu 5–35 ms bekler (G/Ç); 30 s'de bir |
| hepsi | 5,50 MiB (+4,4) | 3 | 276 ms | 66 ms | 1,7 ms |

GPU kaynağı karşılaştırması (ilk prototip, aynı düzenek):

| | private | okuma | değer |
|---|---|---|---|
| NVML (`nvml.dll`) | 20,55 MiB (+19,8) | 14 µs | 0 %, 47 °C |
| D3DKMT, tüm düğümler (3 adaptör, 40 düğüm) | 1,01 MiB | 9–14 ms | 0 %, 46,4 °C |
| D3DKMT, ayrık GPU + yalnız çalışmış düğümler | 1,06 MiB | 1,6 ms | 0 %, 46–47 °C |

## Notlar
- NVML bellek yükü tray bütçesinin (~5,1 MB private) dört katı → D3DKMT seçildi (kullanıcı kararı).
- D3DKMT düğüm sorgusu 0,2–0,9 ms; 13 düğümün çoğu hiç çalışmamış → yalnız `RunningTime != 0` olanlar okunur.
- PDH: tray private ~5,1 → ~9,3 MiB olur (tahmin; Faz 2'de ölçülür). Açılış ve okumalar arayüz iş parçacığı dışında yapılmalı.
- Değerler: GPU `nvidia-smi` ile aynı; HDD 35 °C (uyarı 87 / kritik 90); CPU °C PDH sayacıyla aynı (74–85 °C, yükle değişiyor). Tümleşik GPU'nun D3DKMT sıcaklığı aynı anda 52–54 °C.
