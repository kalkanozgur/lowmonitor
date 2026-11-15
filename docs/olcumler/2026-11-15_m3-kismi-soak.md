# M3 Kısmi Soak Ölçümü — `lowmon-svc` (`f8e1d7f`)

- **Tarih:** 2026-11-15
- **İlgili Belgeler:** `docs/spesifikasyon.md` (§1), `TODO.md`
- **Ham Veri:** `docs/olcumler/2026-11-14_m3-soak.csv` (34 örnek, 5 dakika aralık)

---

## 1. Düzenek ve Araçlar

| Alan | Değer |
|---|---|
| **Hedef İkili** | `lowmon-svc.exe` (commit `f8e1d7f`, release build, ~707 KB) |
| **Çalışma Modu** | Windows Servisi (`NT SERVICE\LowMonitor`, `LocalService` + Restricted SID) |
| **Aktif Alt Sistemler** | M1 toplayıcılar (1 Hz CPU/RAM/Disk/Ağ, 0,2 Hz Top-N), M2 şifreli + zincirli `.lwm` günlük yazıcı, M3 Named Pipe sunucusu (`\\.\pipe\LowMonitor`) |
| **Ölçüm Aracı** | `scripts/soak.ps1` (`Get-Process` / Win32 sayaç sorgusu, 300 s örnekleme) |
| **Ölçüm Aralığı** | `2026-11-14 22:42:37` – `2026-11-15 01:28:14` (**165,6 dakika / 9937 s**) |
| **Sonlanma Nedeni** | `soak.ps1` 01:28'de durdu (reboot 10:31'de, sebep reboot değil; muhtemelen başlatan oturum kapandı) |

---

## 2. Sonuçlar

| Metrik | Başlangıç (22:42) | Min | Max | Son (01:28) | Hedef (Spek §1) | Durum |
|---|---|---|---|---|---|---|
| **CPU (ortalama)** | — | — | — | **2,26 ms/s (%0,0141 @ 16 çekirdek)** | < %0,05 | Geçti |
| **Private WS** | 1812 KB (1,77 MB) | 1628 KB (1,59 MB) | 2052 KB (2,00 MB) | **1748 KB (1,71 MB)** | < 3 MB | Geçti |
| **Private Bytes** | 2152 KB (2,10 MB) | 2152 KB (2,10 MB) | 2768 KB (2,70 MB) | **2676 KB (2,61 MB)** | < 3 MB | Geçti |
| **Toplam WS** | 8168 KB (7,98 MB) | 2920 KB (2,85 MB) | 8528 KB (8,33 MB) | **3064 KB (2,99 MB)** | — | Bilgi |
| **Handle Sayısı** | 86 | 82 | 86 | **82** | Artış yok | Geçti |
| **Thread Sayısı** | 7 | 3 | 7 | **3** | — | Geçti |

---

## 3. Gözlemler

1. **CPU Bütçesi:** 9937 saniyede toplam `22,437 s` (`22,453 s - 0,016 s`) CPU süresi tüketildi; saniyede ortalama **2,26 ms** (%0,0141). M2 şifreleme/disk blok yazımı ve M3 pipe sunucusu ek yükü hedef sınırın (< %0,05) yaklaşık üçte biri seviyesinde kaldı.
2. **Bellek Kararlılığı:** `Private Bytes` ilk 15 dakikada `2768 KB` tepe değerine ulaştıktan sonra kalan 2,5 saat boyunca **2676 KB** taban çizgisinde sabit kaldı. Periyodik görülen `+80 KB` (`2676 → 2756 KB`) artışları Windows thread pool iş parçacıklarının geçici açılıp kapanmasıyla örtüşüyor ve her seferinde taban değere geri dönüyor.
3. **Working Set Davranışı:** İşletim sisteminin çalışma kümesi kırpmasıyla (working set trimming) toplam `WS` 8,33 MB'tan 2,99 MB'a geriledi; `Private WS` ise ~1,71 MB seviyesinde dengelendi.
4. **Handle ve Thread Sızıntısı:** Ölçüm sonunda handle sayısı başlangıcın altında (`86 → 82`), aktif thread sayısı ise `3` olarak sabitlendi; herhangi bir kaynak sızıntısı gözlenmedi.

---

## 4. Sonraki Adım

- Kesintisiz **24 saatlik** kabul ölçümü için `scripts/soak.ps1 -Out "docs\olcumler\2026-11-15_m3-soak.csv"` komutuyla tam süreli ölçüm tekrarlanacak.