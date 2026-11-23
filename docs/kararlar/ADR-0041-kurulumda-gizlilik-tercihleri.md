# ADR-0041: Kurulumda gizlilik ve ağ tercihleri (varsayılan kapalı)
- Tarih: 2026-11-23
- Durum: Kabul (kullanıcı, 2026-11-23); güncelleme kontrolü (Parça 3) ayrı PR'da bu ADR'ye eklenir
- İlgili: ADR-0038 (Inno setup), ADR-0025 (LAN firewall), ADR-0031 (deck tray içinde), ADR-0037 (güvenlik istemi bildirimi)

## Bağlam
Arkadaşın PC'sine kurulum: telefon sunucusu (LAN), alarm balonları ve (yeni) güncelleme kontrolü kurulumda kapalı gelsin, istenirse açılsın. Önceden hepsi koşulsuz açıktı (deck LAN dinleme, firewall kuralları, balonlar); kapatma anahtarı yoktu.

## Karar
1. **Setup'ta "Gizlilik ve ağ" görevleri** (Inno `[Tasks]`): `lan`, `balloons`, `update`. Hepsi işaretsiz (kapalı); yalnız **temiz kurulumda** sorulur (hedefte `lowmon-tray.exe` yoksa). Güncellemede sorulmaz, mevcut davranış korunur.
2. **LAN kapalı =** `deck.toml` `bind = "127.0.0.1"` (deck bu durumda yalnız loopback dinler, UDP keşfi açmaz, PC arayüzü çalışır → deck'e yeni anahtar yok); telefon firewall kuralları eklenmez; `lowmon-svc install --no-lan` "LowMonitor net" kuralını eklemez / siler.
3. **Tray tercihleri:** `%APPDATA%\LowMonitor\tray.toml` (`balloons`, `update_check`). Dosya ya da alan yoksa **açık** (eski kurulumlar değişmez). Setup `lowmon-tray --setup lan=… balloons=… update=…` ile yazar; var olan dosyaya dokunmaz.
4. **LAN seçimi kalıcı:** `HKLM\Software\LowMonitor\Lan` (0/1). Güncellemede `svc install` bu değere göre `--no-lan` verir; değer yoksa açık.
5. **UAC / güvenlik istemi bildirimi** (`config.toml` `[events] uac`) zaten varsayılan kapalı; ayrı görev eklenmedi.

## Sonuçlar
- (+) Temiz kurulum varsayılan olarak ağa kapalı; mevcut kurulumlar güncellemede bozulmaz.
- (+) Yeni deck / servis kodu yok: loopback `bind` ve `--no-lan` mevcut mekanizmalara dayanıyor.
- (−) Kapalıyken açmak elle: `deck.toml` `bind`'i LAN IP'ye çevir + tray menüsünden firewall kuralı; sihirbaz yok.
- (−) Menüdeki "Sessize al" işareti kalıcı `balloons = false`'u göstermez.
- `shortcut:` temiz kurulum tespiti `{autopf}\LowMonitor\lowmon-tray.exe` varlığına bakar; başka klasöre taşınmış kurulum "temiz" sayılır.
- Doğrulama: `prefs` ve `create_loopback` birim testleri; Inno betiği derlendi; sihirbaz akışı lokal kurulumla elle denenecek.

## Ek (2026-11-24): çalışma anında anahtarlar
Setup seçimleri artık tray menüsünden de değişir (sihirbaz gerekmez): "Bildirimler ▸ Alarm balonları" (`tray.toml` `balloons`), "Telefon ▸ Yerel ağda dinle" (`deck.toml` `bind` loopback ↔ LAN IP, dinleyici yeniden açılır; firewall kuralı ayrı, "Firewall kuralı ekle"), "Güncelleme ▸ Otomatik denetle" (ADR-0042). Menü gruplandı: Göstergeler ▸, Bildirimler ▸, Telefon ▸, AI ▸, Ses ▸, Windows ile başlat, Güncelleme ▸. Önceki "Menüdeki Sessize al işareti kalıcı balonu göstermez" sorunu kalktı (ayrı menü maddeleri).
