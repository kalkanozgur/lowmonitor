# ADR-0029: PC arayüzü LowDeck'te (agent köprüsü)
- Tarih: 2026-11-15
- Durum: Kabul; **ADR-0031 ile değişiyor** (agent → lowmon-tray, arayüz → lowmon-ui; taslak 2026-11-16) — ADR-0019'un açık maddesini ("bu repoda mı, ayrı repoda mı") kapatır; spek §2, §3, §8, §9, §13'ü değiştirir

## Bağlam
ADR-0019: UI Slint ile yazılacak, repo yeri açık. Bu repoda henüz `lowmon-ui` crate'i yok.
LowDeck (`kalkanozgur/lowdeck`) Slint ile yazılıyor, aynı uygulama masaüstünde de çalışıyor. `lowdeck-agent` kullanıcı oturumunda pipe'a Viewer olarak bağlanıp Sample + Top'u telefona aktarıyor (LowDeck ADR-0002 a, `lowmon-proto` git bağımlılığı).

Seçenekler (kullanıcıyla 2026-11-15):
| | Veri yolu | Artı | Eksi |
|---|---|---|---|
| A | LowDeck masaüstü → pipe; telefon → agent → pipe | En düşük gecikme | İki veri kaynağı; telefon kazanımı yok |
| **B** | PC ve telefon → agent → pipe | Tek istemci kodu; pipe'ı yalnız agent bilir; telefon da geçmiş/olay alabilir | Agent büyür; loopback ek atlama (~0,1 ms) |
| C | Ortak UI crate, iki uygulama | — | Bakım yükü en yüksek |

## Karar (B)
- PC arayüzü **LowDeck'in masaüstü uygulaması**dır; bu repoda `lowmon-ui` yazılmaz. LowMonitor headless çekirdek kalır: servis, tray, `lowmon-net`, `lwm-dump`, ileride `lowmon-elev`.
- Arayüz LowMonitor verisine yalnız **`lowdeck-agent` üzerinden** erişir (PC'de loopback, telefonda LAN). Pipe'a bağlanan tek LowDeck bileşeni agent'tır.
- **Sözleşme:** pipe protokolü (spek §6, `lowmon-proto`). Değişiklik kuralları aynı kalır: enum sonuna ekleme → `PROTO` aynı; kırıcı değişiklik → `PROTO` artar + LowDeck'e haber (agent'ın `lowmon-proto` rev'i güncellenir).
- **İlk teslimat** (kullanıcı kararı): Canlı + Olaylar. Olaylar = `CH_EVENT` canlı akışı + `QueryRange` ile `RangeItem::Event` geçmişi. Sonra Geçmiş grafiği. Ayarlar/Süreçler `lowmon-elev`'den sonra. Telefonda aynı ekranların olup olmayacağı sonra kararlaştırılacak.
- **Yetki:** agent yükseltilmemiş çalışır → `Control`, `Export` `Forbidden`. Admin işleri (ayar yazma, kill/öncelik) `lowmon-elev` (UAC) ile yapılır; agent'a yetki verilmez.
- **Tray "Arayüzü aç":** `HKCU\Software\LowMonitor` altındaki `UiExe` değerindeki exe'yi başlatır. Değer yoksa menü pasif kalır. Değeri LowDeck masaüstü uygulaması kendi yolu olarak yazar (ayrıntı LowDeck ADR'sinde).
- **Performans kuralları** (§9, Slint): yeniden çizim yalnız veri değişince ve ≤ 1 Hz; simge durumunda abonelik bırakılır; grafik verisi sabit kapasiteli ring buffer.

## Sonuçlar
- \+ Tek Slint kod tabanı (PC + telefon); LowMonitor'da GUI bağımlılığı yok, exe'ler küçük kalır.
- \+ Pipe istemci sayısı artmaz (agent tek bağlantı; 8 istemci sınırı rahat).
- − Arayüz için agent'ın çalışıyor olması gerekir. Agent yoksa arayüz "agent yok" der (LowDeck tarafı).
- − İki repo arasında sürüm uyumu elle izlenir (agent'ın `lowmon-proto` rev'i).
- − Spek §1'deki `lowmon-ui` tüketim satırı LowDeck masaüstü uygulamasına devredilir; ölçümü LowDeck'te yapılır.
