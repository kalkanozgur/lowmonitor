# ADR-0023: lowmon-net ayar yeri, iş parçacıkları ve aşamalı LAN açılışı
- Tarih: 2026-11-15
- Durum: Kabul (spek §16.1 ve §16.6'yı netleştirir)

## Bağlam
Spek §16.6 `[net]` bölümünü `%ProgramData%\LowMonitor\config.toml`'da gösteriyor. Bu dosyanın DACL'ı yalnız SY, BA ve servis SID'ine açık; kullanıcı oturumundaki `lowmon-net` okuyamaz. Spek §16.1 "tek thread, WSAPoll" diyor; pipe okuma ise bloklayan bir çağrı. Eşleştirme/token M6b'de geliyor.

## Karar
- Ayar `%APPDATA%\LowMonitor\net.toml`; yoksa varsayılanla oluşturulur, hatalıysa süreç çıkış kodu 2 ile kapanır (ağ ayarında sessiz varsayılan yok). M6b token hash'leri de aynı klasörde (DPAPI kullanıcı kapsamı).
- İki thread: ağ thread'i (dinleyici + istemciler, `WSAPoll`, zaman aşımı = ping aralığı 30 s) ve pipe okuyucu. İstemci listesi `Mutex` arkasında; pipe thread'i yayını doğrudan non-blocking soketlere yazar.
- Pipe bağlantısının ömrü = en az bir istemci. Son istemci düşünce bir sonraki örnekte pipe kapanır; ilk istemcide `Condvar` ile yeniden açılır. Servis yoksa 10 s'de bir dener.
- Pipe'tan 1 Hz abone olunur; istemci başına `rate_ms / 1000` örnekte bir gönderilir.
- LAN'a açılış aşamalı: M6a'da `bind` yalnız loopback kabul edilir (değilse çıkış kodu 2). LAN bind M6b'de kimlik doğrulamayla birlikte açılır.
- WS sınırları: istemci mesajı ≤ 4 KB, yazma tamponu ≤ 64 KB (okumayan istemci düşer), el sıkışma 2 s zaman aşımlı ve bloklayan (M6b'de LAN için non-blocking'e çevrilecek).

## Sonuçlar
- \+ Servis config'inin ACL'ı gevşetilmedi; kullanıcı kendi ağ ayarını admin olmadan değiştirir.
- \+ Boşta: ağ thread'i 30 s'de bir, pipe thread'i hiç uyanmaz; svc'de abonelik yok.
- − Ayar iki dosyada (servis / kullanıcı).
- − Gerçek mesaj ~360 B (16 çekirdek + Top-5); spek §16.4'teki ~150 B tahmini 8 çekirdekli, Top-3 örneğe göre.
