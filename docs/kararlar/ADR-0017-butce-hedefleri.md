# ADR-0017: Performans bütçesi kesin sınır değil, hedef
- Tarih: 2026-11-14
- Durum: Kabul (spek §1, §5.5 ve §13 kabul kriterlerinin yorumunu değiştirir)

## Bağlam
M2d ölçümleri: exe 590 KB (hedef < 500 KB), log ~4,9 MB/gün (spek tahmini ~2,5 MB). İlk sayılar tasarım aşamasındaki tahminlerdi. Kullanıcı: "ilk belirlediğim limitler biraz küçük kalmış olabilir; program genelinde performans ve minimum tüketime odaklanalım."

## Karar
- Spek §1 tablosu ve §5.5 boyut tahminleri **yön gösteren hedeflerdir**; aşılmaları tek başına bir milestone'u başarısız saymaz.
- Her milestone'da ölçüm yapılır ve raporlanır. Bir metrik kötüleşirse sebebi ve maliyeti açıklanır; ucuz bir iyileştirme varsa uygulanır.
- Öncelik sırası: CPU / uyanma sayısı → private bellek → disk yazma sayısı → log boyutu → exe boyutu.

## Sonuçlar
- \+ Gereksiz mikro-optimizasyon (ör. toml yerine el yazımı parser) yapılmaz.
- − Sınırların kendiliğinden gevşemesi riski; her ölçüm devlog'a yazılarak takip edilir.
