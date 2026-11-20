# ADR-0036: Çoklu telefon: LAN yuvasında ≤ 4 istemci
- Tarih: 2026-11-20
- Durum: Kabul (kullanıcı, 2026-11-20)
- İlgili: ADR-0031 (deck tray'de), LowDeck ADR-0003 (yuvalar)

## Bağlam
`lowmon-deck` hub'ında LAN (telefon) için tek yuva vardı; yeni bağlantı eskisini kapatıyordu. İki telefon (M21 + Mi Note) LowDeck açıkken 1 s'lik yeniden bağlanmayla birbirini sürekli düşürdü (`deck.log`'da `+ bağlandı` / `- koptu` döngüsü, iki farklı IP). İki telefonun adı da `toco`, adla ayırt edilemiyor.

## Karar
1. **Üst sınır 4 telefon** (`MAX_PHONES`, `lowmon-net` `max_clients` üst sınırıyla aynı). Doluyken yeni bağlantı reddedilir (kapatılır, `deck.log`'a yazılır); mevcutlar düşmez → döngü yok.
2. **Aynı IP = aynı telefon:** aynı uzak IP'den gelen yeni LAN bağlantısı o IP'deki eski oturumu kapatır (yarım kalmış oturum, Wi-Fi uyanışı).
3. **Abonelik istemci başına** (zaten öyleydi): push'lar her istemciye kendi konularına göre; komutlar (ses/medya/kill) herhangi birinden.
4. **`phone_wants_events`** = herhangi bir telefon olaylara abone; `phone_name` = adlar `", "` ile.
5. Loopback yuvası (PC arayüzü) değişmez: tek istemci, yeni eskisini kapatır.

## Sonuçlar
- (+) Protokol ve LowDeck değişmez; kod yalnız `lowmon-deck/src/lib.rs` (istemci dizisi → `Vec`).
- (−) Aynı IP arkasında (NAT, misafir ağı) iki telefon birbirini düşürür; ev LAN'ında olmaz.
- (−) 5. telefona "dolu" hata kodu yok (proto'da yok); 1 s'de bir yeniden dener, diğerlerini etkilemez.
