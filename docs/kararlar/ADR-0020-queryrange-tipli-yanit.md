# ADR-0020: QueryRange yanıtı tipli `Range` mesajı, Top-N yok
- Tarih: 2026-11-15
- Durum: Kabul (spek §6.3'teki "QueryRange → RangeChunk" maddesinin yerini alır)

## Bağlam
Spek §6.3'e göre `QueryRange` sonucu 64 KB'lık `RangeChunk(Vec<u8>)` çerçeveleriyle akacaktı. Export bu çerçeveyle CSV gönderiyor. UI (ADR-0019) için CSV parse etmek ya da iki aşamalı decode gereksiz iş. Yetki "Herkes" olduğu için süreç adları/PID'ler de herkese açık olurdu.

## Karar
- Yeni mesaj: `ServerMsg::Range(Vec<RangeItem>)`. Öğelerin kodlanmış toplamı ≤ 64 KB. Akış `Ack` (ya da `Error { Io }`) ile biter.
- `RangeItem = Sample | Agg { kind, sample } | Gap { from_ms, to_ms, reason }`.
- `resolution: Raw | Minute`:
  - `Raw`: dosyada ne varsa döner (ham günde 1 s `Sample`, downsample edilmiş günde `Agg`).
  - `Minute`: ham günlerde servis `aggregate()` ile anında min/avg/max üretir, eski günlerde kayıtlı `Agg`.
- Top-N kayıtları dönmez; süreç geçmişi yalnız admin `Export`'ta.
- Doğrulanamayan segment kuyruğu `Gap { reason: Integrity }` olarak gelir.
- `RangeChunk` yalnız Export (CSV) için kalır.

## Sonuçlar
- \+ İstemci tek postcard decode yapar; tipler `lowmon-core::proto`'da ortak.
- \+ Yetkisiz istemci süreç geçmişini göremez.
- − Postcard enum sırası sözleşme: yeni varyant yalnız sona eklenir, araya eklenirse `PROTO` artar.
- − Otomatik çözünürlük (> 6 saat → Minute) istemcide seçilir.
