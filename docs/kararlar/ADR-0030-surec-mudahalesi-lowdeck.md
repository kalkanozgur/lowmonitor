# ADR-0030: Süreç listesi ve müdahale LowDeck masaüstünde; `lowmon-elev` yalnız ayar yazma için
- Tarih: 2026-11-15
- Durum: Kabul (LowDeck ADR-0006'nın LowMonitor tarafı; spek §2, §9.4, §10, §13'ü değiştirir)

## Bağlam
Spek §9.4: "Süreçler: Top-N tablosu + müdahale (`lowmon-elev`, §10)". ADR-0029 ile arayüz LowDeck'e taşındı. Durum:
| Konu | Durum |
|---|---|
| Top-N | `topn_count = 5`, `topn_every = 5 s`; `TopEntry`'de CreationTime yok |
| `lowmon-elev` | yazılmadı |
| LowDeck masaüstü | izlenen PC'de, kullanıcı oturumunda çalışıyor |

Top-N'i süreç tablosuna yetecek kadar büyütmek svc CPU'sunu artırır (Top-N maliyetin ~%85'i, spek §1); tablo ise yalnız sekme açıkken gerekli.

## Karar
1. **Süreç listesi:** LowDeck masaüstü kendisi tarar (`NtQuerySystemInformation(SystemProcessInformation)`, `lowmon-svc` `topn.rs` ile aynı yöntem), 2 s'de bir, **yalnız Süreçler sekmesi açık ve pencere küçültülmemişken**. LM Top-N değişmez (tray, telefon, log için kalır).
2. **Müdahale** (§10 akışı korunur: OpenProcess → CreationTime eşleşmesi → yasak liste): LowDeck masaüstünde. `ERROR_ACCESS_DENIED` → `ShellExecuteExW(runas, lowdeck-desktop.exe --kill <pid> <ctime>)`; yükseltilmiş kopya aynı kontrolleri yapar, sonuç çıkış koduyla. İlk teslimat yalnız **Sonlandır**; öncelik/affinity/EcoQoS sonra.
3. **`lowmon-elev`:** süreç müdahalesinden çıkarıldı; yalnız §9.5 ayar yazma (`--op write-config`) için planlı kalır.
4. **Olay kaydı** (§10 madde 5, müdahalenin servise bildirilmesi): ertelendi; pipe'a yazma komutu gerektirir, ayrı karar.

## Sonuçlar
- \+ Svc yükü değişmez; tablo tüm süreçleri 2 s tazelikle gösterir.
- \+ `lowmon-elev` kapsamı küçüldü; LowDeck'te yeni exe yok.
- − Müdahale log'a düşmüyor (madde 4).
- − Tarama mantığı iki yerde (svc Top-N, LowDeck): `Reserved1` ofsetleri iki repoda aynı tutulmalı.

## Ek (2026-11-17): servis ayarları da `lowmon-ui`'nın yükseltilmiş kopyasında; `lowmon-elev` iptal
Bağlam: ADR-0031 ile PC arayüzü `lowmon-ui` oldu ve `--kill` için yükseltilmiş kopya deseni hazır. `ProgramData\LowMonitor\config.toml` kullanıcıya kapalı: okumak da yükseltme ister; ayrı `lowmon-elev` ya iki UAC (oku + yaz) ya da pipe'a yeni okuma mesajı (proto sürümü, LowDeck sözleşmesi) gerektirirdi.

Karar:
1. Ayarlar sekmesinde "Servis ayarlarını düzenle" → `ShellExecuteExW(runas, lowmon-ui.exe --service-settings)`; tek UAC.
2. Yükseltilmiş kopya Ayarlar sekmesinde açılır, `config.toml`'u okur; yalnız değişen anahtarı `toml_edit` ile yazar (yorum + sıra korunur), sonuç `lowmon_core::Config::parse`'tan geçmezse yazmaz; ardından pipe'a `Control::ReloadConfig` gönderir (Admin yetkisi rolden değil yükseltilmiş token'dan, spek §6.1), sonucu gösterir.
3. İlk parti: `[sampling]` `interval_ms`, `topn_every`, `topn_count`; `[retention]` `raw_days`, `total_days`. Alarm kuralları sonraki parti.
4. `lowmon-elev` planı kaldırıldı (madde 3'ün yerine geçer).

Sonuçlar:
- \+ Yeni exe yok; tek UAC; proto değişmez.
- − Yükseltilmiş kopya tam arayüzdür: açık kaldıkça Süreçler'de sonlandırma UAC'sız çalışır. Pencere başlığı "(Yönetici)" gösterir.
