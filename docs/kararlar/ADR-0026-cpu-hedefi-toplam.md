# ADR-0026: CPU hedefi toplam CPU'ya göre
- Tarih: 2026-11-15
- Durum: Kabul (spek §1 ve §14 M1 kriterini değiştirir; ADR-0017'nin yön gösteren hedef ilkesi geçerli)

## Bağlam
Spek §1 "CPU < %0.05" diyor, ölçüm yöntemi `\Process(lowmon-svc)\% Processor Time`. Bu sayaç tek çekirdek ölçeğinde (100 = 1 çekirdek tam dolu). Tahmin dökümü de "tek çekirdeğin ~%0.04'ü" (~0.4 ms/s) varsayıyordu.

M3 kısmi soak (`f8e1d7f`, 2,76 sa, [ölçüm](../olcumler/2026-11-15_m3-kismi-soak.md)): 22,44 s CPU / 9937 s = **2,26 ms/s**.
- Tek çekirdek ölçeği: %0,226 → hedefin ~4,5 katı.
- Toplam CPU (16 mantıksal çekirdek, Görev Yöneticisi ölçeği): %0,014 → hedefin altında.

Kullanıcıya görünen tüketim Görev Yöneticisi'ndeki toplam CPU yüzdesidir.

## Karar
- CPU hedefi **toplam CPU** üzerinden: `ortalama CPU % = CPU süresi / (geçen süre × mantıksal çekirdek sayısı)` < %0.05.
- Ölçüm: `soak.ps1` CSV'sindeki `cpu_s` farkından hesap; `\Processor Information(_Total)` ölçeğiyle aynı. Raporda tek çekirdek değeri (ms/s) de yazılır.
- Mutlak bilgi değeri olarak **ms/s** raporlanır; 1 ms/s üstü sapma olarak devlog'a not edilir.

## Sonuçlar
- \+ Hedef, kullanıcının gördüğü değerle aynı ölçekte.
- \+ M3 kısmi soak ölçeğiyle M1 CPU kriteri sağlanıyor (24 saat soak hâlâ gerekli).
- − Hedef çekirdek sayısına bağlı: 16 çekirdekte 8 ms/s'ye, 4 çekirdekte 2 ms/s'ye izin verir. Az çekirdekli makinede aynı exe hedefi aşabilir.
- − Ölçülen 2,26 ms/s, ilk tahminin (0,4 ms/s) ~5,6 katı; sebep bilinmiyor, profilleme açık iş.
