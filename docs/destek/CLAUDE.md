# LowMonitor — uzak PC tanılama (Claude için talimat)

Bu klasör, LowMonitor'ün kurulu olduğu bir PC'de sorun ayıklamak için. **Kod değiştirme, ayar değiştirme, servis durdurma yok** — yalnız oku ve raporla. Dil: Türkçe, kısa, tablolu.

## 1. Tanılama
1. `lowmon-diag.ps1`'i çalıştır: `powershell -ExecutionPolicy Bypass -File .\lowmon-diag.ps1`
2. Çıktı: `Masaüstü\lowmon-diag-<tarih>.txt`. Kullanıcıya dosyanın yolunu söyle.
3. Dosyayı oku ve şunları özetle (tablo):

| Kontrol | Nereye bak | Beklenen |
|---|---|---|
| Servis çalışıyor mu | "Servis ve surecler" | `lowmon-svc`, `lowmon-tray` Running |
| CPU °C yok mu | "CPU sicaklik sayaci" | Sayaç yoksa UI'daki `CPU: —` normal, hata değil |
| Widget çakışması | "Taskbar ogeleri": `WidgetsButton` x aralığı ile `LowMonitorTaskbar` blokları | Blok sağ kenarı widget sol kenarından küçük olmalı |
| Taskbar hizası | `TaskbarAl` | 0 = sola hizalı |
| Log hataları | "Log dosyalari", "Uygulama hatalari" | Tekrarlayan satırları not et |

## 2. Widget algılama testi (isteğe bağlı)
Yalnız `demo.exe` bu klasördeyse:
1. Kullanıcıya haber ver, sonra kurulu tray'i kapat: `Stop-Process -Name lowmon-tray`
2. `.\demo.exe 60` çalıştır. İki blok widget düğmesinin **soluna** yaslanmalı, üstüne binmemeli. Kullanıcıdan ekran fotoğrafı iste.
3. Bitince tray'i geri aç (kapatmadan önce `(Get-Process lowmon-tray).Path` ile yolu not et).

## 3. Rapor
- Kullanıcı logu kendine e-postalayacak: gönderilecek dosya yalnız `lowmon-diag-*.txt`.
- Göndermeden önce dosyada kişisel bilgi (kullanıcı adı, medya başlıkları) var mı kontrol et ve kullanıcıya söyle. E-postayı **sen gönderme**.
