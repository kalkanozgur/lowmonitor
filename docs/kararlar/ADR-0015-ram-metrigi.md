# ADR-0015: RAM hedefi private metriklerle ölçülür
- Tarih: 2026-11-14
- Durum: Kabul (spek §1 tablosunu değiştirir)

## Bağlam
Boş döngüdeki `lowmon-svc run` ölçümü: toplam WS 4.736 KB, Private WS 460 KB, Private Bytes 720 KB. Toplam WS'nin ~%90'ı ntdll/kernel32 gibi paylaşılan DLL sayfaları; süreç bunları kontrol etmez, sistem genelinde tek kopya.

## Karar
"< 3 MB" hedefi **Private WS** ve **Private Bytes** için ayrı ayrı uygulanır. Toplam WS raporlanır ama kabul kriteri değildir. Kozmetik `EmptyWorkingSet` yapılmaz.

## Sonuçlar
- \+ Hedef sürecin gerçek maliyetini ölçer; hile (trim) gerektirmez.
- − Görev Yöneticisi "Bellek" sütunu (Private WS) ile Process Explorer WS farklı görünür; dokümanda belirtildi.
