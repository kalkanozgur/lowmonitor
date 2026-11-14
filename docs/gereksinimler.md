# Gereksinimler — LowMonitor
Kaynak: [spesifikasyon.md](spesifikasyon.md). Çelişkide spek geçerlidir.

## Amaç
Windows'ta sürekli çalışan, ölçülebilir derecede düşük tüketimli kaynak izleme, şifreli loglama, alarm ve süreç müdahalesi.

## Fonksiyonel gereksinimler
- CPU (toplam + çekirdek), RAM/commit/cache, disk r/w, ağ rx/tx 1 s; Top-N süreç 5 s (§4)
- Şifreli + hash zincirli günlük log, 7 gün ham / 30 gün 1 dk downsample (§5)
- Named pipe IPC, abone yoksa sıfır akış (§6)
- Histerezisli alarmlar, tray bildirimi (§7–8)
- egui UI: canlı, geçmiş, süreçler, olaylar, ayarlar (§9)
- Müdahale: kill / priority / affinity / EcoQoS, PID reuse koruması, UAC helper (§10)
- Self-install servis (§11); LAN WebSocket yayını (§16)

## Performans bütçesi (lowmon-svc)
| Metrik | Hedef |
|---|---|
| CPU (24 saat ort.) | < %0.05 |
| Working set | < 3 MB |
| Disk yazma | Dakikada 1 (~1.5 KB) |
| Wake-up | 1/s (coalescing) |
| Exe boyutu | < 500 KB |

## Çevre
- Windows 10/11 x64, MSVC target, statik CRT
- Servis: LocalService + servis SID; UI/tray kullanıcı oturumu

## Kabul kriterleri
Kilometre taşı bazında spek §13 "Tamam kriteri" sütunu.

## TBD
- Desteklenen en düşük Windows sürümü (EcoQoS: Win11 / Win10 21H2+?)
- Spek §15 açık konuları
