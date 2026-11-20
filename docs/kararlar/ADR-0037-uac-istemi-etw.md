# ADR-0037: UAC istemi algılama: servis içinde ETW (Kernel-Process)
- Tarih: 2026-11-20
- Durum: Kabul (kullanıcı, 2026-11-20)
- İlgili: ADR-0028 (alarm ve event), ADR-0004 (LocalService), ADR-0036 (çoklu telefon); TODO "Home Assistant olay bildirimi"

## Bağlam
Kullanıcı PC başında değilken bir UAC istemi açıldığında telefondan haberdar olmak istiyor (yalnız bildirim; telefondan onay **yok** — güvenlik). Polling yasak; çekirdek logger'ın tüketimi artmamalı; özellik kapalı başlamalı.

## Karar (kullanıcı, 2026-11-20)
1. **Kaynak:** ETW `Microsoft-Windows-Kernel-Process` ({22FB2CD6-0E7B-422B-A0C7-2FAD1FD0E716}), anahtar kelime `0x10` (process), olay kimliği süzgeci 1 (başla) + 2 (bitir). Özel gerçek zamanlı oturum `LowMonitor-UAC`, `FlushTimer = 1 s`.
2. **Yer:** `lowmon-svc` (`uac.rs`), ayrı thread `ProcessTrace`'te bloklu. Görüntü adı `\consent.exe` olan başlangıçların PID'i tutulur; açık istem 0 → 1 olunca `true`, 1 → 0 olunca `false` kanala gider.
3. **Çıkış:** mevcut alarm yolu (ADR-0028): `AlarmRaise` / `AlarmClear` `name = "uac"`, aktif alarm listesine `uac` eklenir → `.lwm` olay kaydı (Geçmiş), pipe `Event` → tray balonu, deck → telefon (alarm bandı, perde). HA webhook / MQTT ayrı adım.
4. **Bayrak:** `config.toml` `[events] uac = false` (varsayılan). Kapalıyken oturum ve thread açılmaz → idle tüketim değişmez. `ReloadConfig` açar/kapatır (kapanırken açık `uac` alarmı temizlenir). Arayüz: `lowmon-ui` Ayarlar ▸ servis ayarları onay kutusu (yükseltilmiş kopya, ADR-0030 eki).
5. **Yetki:** LocalService gerçek zamanlı oturum açabilir/tüketebilir (Microsoft StartTrace / OpenTrace belgeleri); grup üyeliği eklenmedi. Açılamazsa bir kez `uac_unavailable` sistem olayı (değer = Win32 hata kodu).

## Tasarım ayrıntıları
- Önceki çalıştırmadan kalan oturum (`ERROR_ALREADY_EXISTS`) önce durdurulur.
- Başlangıç olay verisi TDH ile ayrıştırılmaz: PID ilk `u32` (tüm sürümlerde), görüntü adı veri içinde UTF-16 `\consent.exe\0` aranarak bulunur (iki hizalama). `shortcut:` olay sürümü değişse de çalışır; yanlış pozitif için adın NUL ile bitmesi şart.
- Gecikme: ETW tamponu ≤ ~1 s + servis döngüsü ≤ 1 tick.
- Servis durunca `Drop`: `ControlTraceW(STOP)` → `ProcessTrace` döner, thread birleştirilir.

## Sonuçlar
- (+) Yeni crate yok (yalnız `windows-sys` özellikleri `Win32_System_Diagnostics_Etw`, `Win32_System_Time`); yeni süreç yok; telefon/proto değişmez.
- (+) UAC olayları Geçmiş'te kalır.
- (−) Açıkken sistemdeki her süreç başla/bitir olayında thread uyanır (kısa tarama); ölçüm canlı kurulumda.
- (−) Canlı doğrulanmadı: LocalService yetkisi, gecikme, güvenli masaüstü kapalıyken davranış (consent.exe yine başlar, algılanmalı).
- Spek: §7 config örneği + olay listesi.

## Ek (2026-11-20): güvenlik duvarı izni penceresi
- **Tespit:** "Allow public and private networks…" penceresini `PickerHost.exe FirewallNotificationDialogServer -Embedding` açar (DcomLaunch svchost'tan), pencere kapanınca biter. Deneme exe'siyle (0.0.0.0 dinleme) iki kez gözlendi.
- **Karar (kullanıcı):** aynı ETW oturumunda görüntü adı `\PickerHost.exe` → `firewall` alarmı; ayar ayrı değil, `[events] uac` ikisini birlikte açar (anahtar adı geriye uyum için aynı). Arayüz etiketi "Güvenlik istemlerini bildir: UAC, güvenlik duvarı".
- **Sınır (`shortcut:`):** ETW Kernel-Process başlangıç olayında komut satırı yok; PickerHost Store uygulamalarının dosya seçicisini de barındırır → nadiren yanlış `firewall` alarmı. Gerekirse tray (kullanıcı olarak) komut satırını doğrular.
- Kanal `(ad, açık mı)`; servis açık alarm adlarını listede tutar, ReloadConfig'te kapatılınca hepsini temizler.
