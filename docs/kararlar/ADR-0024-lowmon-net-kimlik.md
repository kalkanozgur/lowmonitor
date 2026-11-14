# ADR-0024: lowmon-net eşleştirme, token ve deneme koruması
- Tarih: 2026-11-15
- Durum: Kabul (spek §16.2'yi uygular ve netleştirir; ADR-0023'ün "M6a yalnız loopback" kısıtını kaldırır)

## Bağlam
Spek §16.2: `lowmon-net pair` → 6 haneli kod 120 s → `POST /v1/pair` → 256-bit token; PC'de SHA-256 hash + DPAPI; Bearer ya da `?token=`; hatalı denemede 1 s gecikme, IP başına 10 hata → 10 dk ban; Origin izin listesi. Ağ thread'i tek; bloklayan bekleme herkesi durdurur.

## Karar
- **Kod kanalı:** `lowmon-net pair` kodu konsola yazar; SHA-256(kod) ‖ bitiş ms DPAPI (kullanıcı) ile `%APPDATA%\LowMonitor\pair.bin`'e. Çalışan sunucu `POST /v1/pair`'de dosyayı okur; doğru kodda siler (tek kullanımlık), süresi geçmişse siler. Ayrı IPC yok.
- **Token:** 32 bayt `getrandom` → 64 hex. Diskte yalnız SHA-256 hash'i (`tokens.bin`, DPAPI). Her el sıkışmada dosya yeniden okunur. Karşılaştırma sabit süreli.
- **İptal:** `lowmon-net unpair --all` iki dosyayı da siler. Açık bağlantılar ping turunda (≤ 30 s) token'ları geçersiz olduğu için düşer.
- **1 s gecikme:** IP başına kısıt olarak uygulanır. Hatalı denemeden sonra o IP'den 1 s içinde gelen bağlantı yanıtsız kapanır; diğer istemciler beklemez. 10 hata → 10 dk ban. Başarılı kimlik doğrulama sayacı sıfırlar.
- **Origin:** header varsa `allowed_origins`'te olmalı (yoksa 403). Header yoksa (yerel uygulama) yalnız token yeterli.
- **El sıkışma non-blocking:** bekleyen bağlantılar `WSAPoll`'a eklenir; 2 s'de tamamlanmayan, 8 KB'ı aşan istek düşer; en fazla 8 bekleyen. WS yükseltmesini `httparse` + `derive_accept_key` ile sunucu kendisi yapar, sonra `WebSocket::from_partially_read`.
- **Bind:** config'teki IP, `0.0.0.0` / `::` reddedilir. Firewall kuralı M6c'de.

## Sonuçlar
- \+ Düz token PC'de saklanmaz; DPAPI dosyası başka kullanıcıya/makineye taşınınca açılmaz.
- \+ Sessiz/yavaş bağlantı canlı yayını ve diğer el sıkışmaları bekletmez (testle doğrulandı).
- − Tüm token'lar tek seferde iptal edilir; tekil iptal yok.
- − `tokens.bin` atomik yazılmıyor (`shortcut:`); yarım dosya DPAPI'de reddedilir → yeniden eşleştirme gerekir.
