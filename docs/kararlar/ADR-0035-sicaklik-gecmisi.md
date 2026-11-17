# ADR-0035: Sıcaklık / GPU geçmişi: tray'in dakikalık halka dosyası
- Tarih: 2026-11-17
- Durum: Kabul (kullanıcı kararları alındı)
- İlgili: ADR-0033 Faz 4b, ADR-0005 (geçmiş), spek §9

## Bağlam
GPU % ve sıcaklıklar (CPU, GPU, disk) yalnız tray'deki `lowmon-sensors` iş parçacığında okunur (ADR-0033). Servis bunları bilmez; `.lwm` segment biçimi ve pipe değişmeden `lowmon-ui` Geçmiş sekmesinde gösterilmeleri isteniyor.

## Karar (kullanıcı, 2026-11-17)
1. **Yazan tray, kullanıcı dosyası.** `%LOCALAPPDATA%\LowMonitor\sensors.bin`; servis, proto, segment biçimi değişmez.
2. **Çözünürlük: 1 dk ortalama + max.** Tray 2 s okumaları dakikada toplar, dakika bitince tek kayıt yazar.
3. **Gösterim Canlı ile aynı:** CPU kartına °C çizgisi, yeni GPU kartı (% + °C), disk başlığında aralığın en yüksek °C'si.

## Tasarım
- **Halka:** 7 g × 1440 = 10 080 yuva × 20 B = 201 600 B, ilk yazmada tam boy. Yuva = `ts_min % 10080`; başlık yok.
- **Kayıt (LE):** `ts_min u32` (Unix dakika) · CPU / GPU / disk °C ort + max `i16` (0,1 °C) · GPU % ort + max `u8` · 2 B boş. Veri yok: `i16::MIN` / `u8::MAX`.
- **Okuyucu** (`lowmon-ui`): dosyayı bir kez okur, `ts_min` aralık dışı (eski tur, boş yuva = 0) kayıtları eler. Geçmiş kovalarına: ortalamaların ortalaması, max'ların max'ı; dakikadan kısa kova kendi dakikasını kullanır.
- **Kod:** `lowmon_sensors::log` (yalnız std; `Minute` toplayıcı, `write`, `read`, testli); tray `sys.rs` sensör iş parçacığı yazar, `lowmon-ui` `history::temp_buckets` + `draw_history` okur.
- **Kesinti:** iş parçacığı yalnız ızgara açık ya da sensör abonesi varken çalışır (ADR-0033 Faz 4a); tray kapalı / ızgara kapalıyken boşluk kalır, grafik orada kopar. Yarım dakika yazılmaz.
- **Eşzamanlılık:** tek yazar (tray tek örnek). Okuyucu yazmayla çakışırsa en kötü bir kayıt yırtılır; 20 B'lik yazma tek `WriteFile`, pratikte atomik.

## Sonuçlar
- (+) Servis, pipe, `.lwm`, LowDeck proto değişmez; dakikada bir 20 B yazma.
- (+) Sabit 197 KiB; temizlik gerekmez.
- (−) Geçmiş en fazla 7 g (Geçmiş sekmesinin en uzun aralığı); servis geçmişinin 30 g'ı yok.
- (−) Veri kullanıcıya özgü ve şifresiz (sıcaklıklar hassas sayılmadı; `.lwm` gibi hash zinciri yok).
- (−) Tray yalnız oturum açıkken çalışır: oturum dışı süre boş.
- Spek: §9 Geçmiş satırı; ADR-0033 faz tablosu 4b → bu ADR.
