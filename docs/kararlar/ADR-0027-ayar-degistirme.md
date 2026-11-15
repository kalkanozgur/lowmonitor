# ADR-0027: Çalışırken ayar değiştirme (SetInterval / SetRetention)
- Tarih: 2026-11-15
- Durum: Kabul — seçenek A (spek §6 `Control` satırını ve §5 başlık `interval_ms` tanımını değiştirir)

## Bağlam
Spek §6 `Control(Pause | Resume | SetInterval | SetRetention | ReloadConfig)` tanımlar. M4'te (PR #12) Pause/Resume/ReloadConfig geldi; ReloadConfig `interval_ms` değişikliğini `Unsupported` ile reddediyor.

`interval_ms` kullanımı (kod taraması):
| Yer | Rol |
|---|---|
| Segment başlığı (`Header.interval_ms`) | Yalnız bilgi; okuyan tek yer `lwm-dump` çıktısı. Query/Export/downsample örnek `ts`'ini kullanır |
| `Writer` | Sleep gap eşiği = 5 × aralık |
| `Ticker` | Waitable timer periyodu |
| `pipe::Server` | Abonelik `rate_ms / interval_ms` (açılışta sabit) |

Dosya adı gün başına tek segment (`segment_name(day)`); gün içinde yeni segment açmak dosya düzenini değiştirir. Servis bugün de yeni aralıkla yeniden başlarsa bugünün segmentine eski başlıkla devam eder.

`config.toml` DACL'ı: SY ve BA tam, servis SID'i yalnız okuma (install).

## Seçenekler
| | Yaklaşım | Artı | Eksi |
|---|---|---|---|
| **A** | `SetInterval`/`SetRetention` mesajı yok. Admin istemci `config.toml`'u yazar → `ReloadConfig`. Aralık çalışırken uygulanır: timer yeniden kurulur, `Writer` eşiği ve pipe bölücüsü (`AtomicU32`) güncellenir, toplayıcı sıfırlanır; segment başlığı "segment açılışındaki aralık (bilgi)" olarak tanımlanır | Tek yol (dosya = tek doğruluk kaynağı), servis ACL'ı değişmez, yeni segment yok, protokol küçülür | Başlıktaki aralık gün içinde eskiyebilir; Admin yazımı UI'da yükseltme ister (`lowmon-elev` op'u) |
| B | Spekteki gibi `SetInterval`/`SetRetention` mesajları; servis `config.toml`'a yazar | İstemci dosya biçimini bilmez | Servis SID'ine config yazma hakkı (ACL gevşer); iki yol (dosya + mesaj) çakışabilir; TOML yeniden yazımı yorumları siler |
| C | Aralık yalnız yeniden başlatmada; ReloadConfig `Unsupported` kalır | Kod yok | Ayar değişince servis yeniden başlatma (Admin + kısa kesinti, `Gap(Paused)`) |

## Karar
**A** (kullanıcı, 2026-11-15). Spek §6: `Control(Pause | Resume | ReloadConfig)`; ayar değişikliği = Admin'in `config.toml` yazması + `ReloadConfig`. Aralık değişikliği log'a `Gap` yazmaz (örnek ts'leri sürekli); başlık alanı belgede "açılış aralığı" olarak işaretlenir.

## Sonuçlar
- \+ Servis config'e yazamaz kalır (en az yetki).
- \+ Protokolden iki varyant düşer; ReloadConfig tek uygulama noktası.
- − UI/tray'in config yazması yükseltme ister (`lowmon-elev --op write-config`, M4 kalanı).
- − `lwm-dump` başlık aralığını "açılış" diye etiketlemeli.
