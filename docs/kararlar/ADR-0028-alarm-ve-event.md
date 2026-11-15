# ADR-0028: Alarm motoru ve Event kaydı (M5)
- Tarih: 2026-11-15
- Durum: Kabul (spek §5.4 Event, §6 kanal/mesaj, §7 alarmlar, §8 tray'i netleştirir)

## Bağlam
Spek §7: `[[alarm]]` kuralları (`raise`/`clear` + `for_s`), her tick değerlendirme, `Normal → Pending → Raised → Clearing` durum makinesi, Raise/Clear → `Event` kaydı + tray balonu. Açık kalanlar: metrik adları, cihaz seçimi, süre ölçümü, gap/pause davranışı, Event'in kapsamı, istemcilere aktarım.

Kod durumu: codec'te `REC_EVENT (0x03)` yok; dört `shortcut:` "Event M5'te" diyor (hatalı config, pipe açılamadı, yazma hatası, retention sonucu). `Sample`'da toplam RAM / commit limiti yok. Aralık çalışırken değişebilir (ADR-0027) → tick sayarak süre ölçmek yanlış.

Kullanıcı kararları (2026-11-15): metrikler = temel 5 + cihaz başına; Event = alarm + sistem olayları.

## Karar

### Kural biçimi (`config.toml`)
```toml
[[alarm]]
id     = "cpu_high"               # benzersiz, [a-z0-9_]{1,32}
metric = "cpu.total"
raise  = { above = 90.0, for_s = 30 }
clear  = { below = 80.0, for_s = 10 }

[[alarm]]
id     = "disk0_write"
metric = "disk.w_bps"
device = "0"                      # PhysicalDrive numarası; yoksa tüm diskler toplamı
raise  = { above = 200_000_000, for_s = 60 }
clear  = { below = 50_000_000,  for_s = 30 }

[[alarm]]
id     = "wan_rx"
metric = "net.rx_bps"
device = "Ethernet 2"             # arayüz takma adı (ifIndex açılışta değişebilir)
raise  = { above = 50_000_000, for_s = 20 }
clear  = { below = 10_000_000, for_s = 20 }
```
| Metrik | Birim | `device` |
|---|---|---|
| `cpu.total` | % | — |
| `mem.used_pct`, `mem.commit_pct` | % (toplam RAM / commit limiti serviste bir kez + keşifte okunur) | — |
| `disk.r_bps`, `disk.w_bps`, `disk.bps` (r+w) | byte/s | PhysicalDrive no; yoksa toplam |
| `net.rx_bps`, `net.tx_bps`, `net.bps` (rx+tx) | byte/s | arayüz takma adı; yoksa toplam |

- `above` yerine `below` da yazılabilir (ör. boş disk alanı ileride). Histerezis zorunlu: `raise.above` için `clear.below ≤ raise.above`.
- Doğrulama `Config::parse`'ta: benzersiz id, bilinen metrik, ≤ 32 kural, `for_s` 0..=3600. Hata → config hatası (açılışta varsayılan + `Event`, ReloadConfig'te `Io` ve mevcut kurallar kalır).

### Durum makinesi
- Süre örnek `ts`'iyle ölçülür (tick sayılmaz): Pending başlangıç ts'i + `for_s` ≤ şimdiki ts → Raised.
- `Normal → Pending` (eşik aşıldı) `→ Raised` (süre doldu) `→ Clearing` (clear eşiği) `→ Normal` (süre doldu). Pending'de eşik geri dönerse Normal; Clearing'de raise eşiği yeniden aşılırsa Raised.
- Değer yoksa (cihaz kayboldu, ilk örnek): durum donar, sayaç ilerlemez.
- Gap (Pause/Resume, uyku, > 5 aralık): Pending → Normal, Clearing → Raised; Raised korunur.
- Servis yeniden başlarsa durum saklanmaz; kural yeniden değerlendirilir.
- ReloadConfig: id + metric + device + eşikler aynı kalan kuralın durumu korunur, değişen sıfırlanır; kaldırılan Raised kural için `AlarmClear` yazılır.
- Maliyet hedefi: 32 kuralda < 5 µs/tick (bench'e eklenir).

### Event kaydı (`REC_EVENT = 0x03`, `SCHEMA` 1 → 2)
```
Event { kind: u8, name: dict_id, value: zigzag i64 }   // zaman = bloktaki önceki Sample'ın ts'i (Top gibi)
```
| kind | name örnekleri | value |
|---|---|---|
| AlarmRaise / AlarmClear | kural id'si | değer × 100 (% için 0,01, byte/s için ×100 taşmaz) |
| System | `config_invalid`, `pipe_unavailable`, `write_error`, `retention_ok`, `retention_error`, `integrity`, `control_pause`, `control_resume`, `control_reload`, `service_start` | koda göre (ör. silinen segment sayısı) |

- İsimler ProcDict sözlüğünü paylaşır (segment kapsamlı, ilk kullanımda yazılır).
- Aynı System olayı dakikada en fazla 1 kez yazılır (disk dolu sırasında taşkın yok).
- Okuyucular (query, export, downsample, `lwm-dump`) Event'i tanır; downsample Event'leri olduğu gibi taşır.
- Gap(Integrity) kalır; ayrıca `integrity` System olayı yazılır.

### İstemcilere aktarım (pipe, enum sonuna ekleme → `PROTO` aynı)
- Yeni kanal `CH_EVENT = 4`; `ServerMsg::Event { ts_ms, kind, name, value }`.
- `CH_EVENT` aboneliğinde önce `ServerMsg::Alarms(Vec<String>)` (şu an Raised olanlar) → istemci bağlanınca durumu bilir.
- `RangeItem::Event { … }`: QueryRange olay geçmişini de döner (UI "Olaylar" sekmesi).
- Tray: `CH_EVENT` abonesi; AlarmRaise → `Shell_NotifyIconW(NIF_INFO)` balonu; AlarmClear balon göstermez, tooltip güncellenir. "1 saat sessiz" tray belleğinde.
- `lowmon-net`: `CH_EVENT` abonesi; `/v1/live` JSON'undaki `alarms` aktif listeden.

## Sonuçlar
- \+ Dört `shortcut:` kapanır (hatalar artık log'da).
- \+ Aralık değişikliğinden etkilenmeyen süre ölçümü.
- \+ Olay geçmişi aynı şifreli/zincirli log'da; ayrı depo yok.
- − `SCHEMA` 2: eski `lwm-dump` yeni segmenti okuyamaz (aynı sürümle dağıtılır).
- − Arayüz takma adı kullanıcı tarafından değiştirilirse kural cihazı bulamaz (durum donar, `Event` yok); `/v1/info` adlarıyla aynı kaynak.
- − Servis yeniden başlayınca aktif alarm tekrar `for_s` sonra Raise olur (yinelenen Raise kaydı).
