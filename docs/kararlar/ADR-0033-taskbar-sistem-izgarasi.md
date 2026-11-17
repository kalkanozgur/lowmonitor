# ADR-0033: Taskbar'da sistem ızgarası: `lowmon-sensors` + çok bloklu `lowmon-taskbar`
- Tarih: 2026-11-17
- Durum: Taslak (kullanıcı kararları alındı; uygulama fazlar halinde, her faz ayrı PR).

## Bağlam
Kullanıcı taskbar'da, AI göstergesinin (ADR-0032) yanında 2 satır × 4 sütun bir sistem ızgarası istiyor:

```
CPU: 15 %   GPU:  0 %   MEM: 88 %   ↑: 0.67 KB/s
CPU: 63 °C  GPU: 44 °C  HDD: 35 °C  ↓: 22.6 KB/s
```

| Değer | Bugün | Kaynak |
|---|---|---|
| CPU %, MEM %, ağ ↑↓ | Var: `lowmon-svc` ölçer, tray pipe'tan 2 s'de bir `Sample` alır | değişmez |
| GPU %, GPU °C | Yok (spek §2: `lowmon-gpu` "sonra tasarlanacak") | D3DKMT (Faz 1 kararı; taslakta NVML'di) |
| HDD °C | Yok | NVMe / ATA sıcaklık sorgusu |
| CPU °C | Yok | ACPI termal bölge |

Donanım (bu PC, 2026-11-17): Ryzen 7 5800H, RTX 3060 Laptop + AMD tümleşik GPU, Micron NVMe SSD; `nvml.dll` System32'de.

## Karar (kullanıcı, 2026-11-17)
1. **Yerleşim:** sistem ızgarası ve AI bloğu **yan yana**. Sonraki adım: sistem ızgarası takılıysa **diğer monitöre** → her blok kendi monitör ayarını taşır.
2. **CPU °C: ACPI termal bölge.** Spek §1 "PDH/WMI yok" (ADR-003) kuralına **dar istisna**: yalnız bu tek sayaç, yalnız tray'de (servis değil).
3. **Kapsam: önce taskbar.** GPU / sıcaklık Faz 1–3'te yalnız taskbar'da; geçmiş, telefon ve `lowmon-ui` sonra ayrı faz (Faz 4, o zaman ayrıca kararlaştırılır).
4. **Tık:** sistem bloğuna sol tık `lowmon-ui`'ı Canlı sekmesiyle açar / öne getirir; sağ tık tray menüsü.
5. **Dil:** taskbar etiketleri İngilizce (`ai.toml language = "en"` yapıldı). Tray menüsü + `lowmon-ui` en/tr ayrı iş (ayrı PR, bu ADR dışında).

## Tasarım

### Yeni crate: `lowmon-sensors` (lib)
Servisin ölçmediği üç kaynak. Her biri bağımsız: yoksa / hata verirse hücre `—` olur, diğerleri çalışır.

| Kaynak | Yöntem | Not |
|---|---|---|
| GPU %, °C | D3DKMT (`gdi32`; Görev Yöneticisi'nin yolu, üretici DLL'i yok): `D3DKMTQueryStatistics` düğüm `RunningTime` farkı → en yoğun motorun kullanımı; `D3DKMTQueryAdapterInfo(KMTQAITYPE_ADAPTERPERFDATA)` → sıcaklık (0,1 °C) | Hibrit sistemde ayrık GPU (`HybridDiscrete`), yoksa ilk donanım adaptörü. Yalnız hiç çalışmış düğümler sorgulanır (sorgu başına 0,2–0,9 ms), 60 s'de bir yeniden taranır. Yükseltme gerekmez |
| HDD °C | `IOCTL_STORAGE_QUERY_PROPERTY` + `StorageDeviceTemperatureProperty` (`\\.\PhysicalDriveN`, erişim 0) | Birden çok diskte en sıcak olan. Yükseltmesiz çalışıyor (deneme: `PhysicalDrive0` 35 °C, uyarı 87 / kritik 90) |
| CPU °C | PDH: `\Thermal Zone Information(*)\Temperature` (Kelvin), en sıcak bölge | Yükseltmesiz çalışıyor (deneme: 64,9–75,9 °C, yükle değişiyor). Çip sıcaklığı değil, ACPI bölgesi: kaba olabilir |

Denenip bırakılan (2026-11-17): termal bölge aygıtı (`GUID_DEVICE_THERMAL_ZONE`) + `IOCTL_THERMAL_READ_TEMPERATURE`. Yükseltmesiz açılıyor ama veri dönmüyor (0 bayt). WMI `MSAcpi_ThermalZoneTemperature` yönetici ister. Çekirdek sürücüsü (WinRing0 türü) reddedildi: güvenlik riski.

### Nerede ölçülür: tray (Faz 1–2)
- `lowmon-sensors` tray'de, **yalnız sistem bloğu açıkken**, 2 s'de bir (pipe `Sample` ritmiyle) okunur. Servise dokunulmaz: svc bütçesi (< 3 MB, < 500 KB exe), segment biçimi ve `lowmon-proto` değişmez.
- Sonuç: GPU ve sıcaklıklar **yalnız taskbar'da**; geçmişe yazılmaz, telefona ve `lowmon-ui`'a gitmez. Bunlar ayrı faz (Faz 4, Karar 3); o zaman spek §2'deki üretici kanalı (`ProducerPush`) kullanılır, `lowmon-sensors` aynen taşınır.

### `lowmon-taskbar`: çok blok
- Bugün: tek pencere, `show([Line; 2])`. Yeni: **blok** = satır × sütun hücre ızgarası (`Cell { text, rgb }`), sütunlar hizalı (her sütun kendi en geniş hücresi kadar).
- AI bloğu 2×1, sistem bloğu 2×4. Aynı monitördeki bloklar bildirim alanının solunda yan yana dizilir (sıra ayardan). Farklı monitördeki blok o monitörün taskbar'ına ayrı pencere olarak kurulur.
- `taskbar.toml`: ortak `font`, `font_pt`, `gap_px`; blok başına `[ai]` / `[sys]`: `enabled`, `monitor` (`primary` / `left` / `right`; Faz 3). Eski tek-blok ayarı (`monitor`, `reserved_px`) AI bloğuna uygulanır.
- Çizim yöntemi değişmez (ADR-0032 Faz 2: `Shell_TrayWnd` alt penceresi, GDI+, `UpdateLayeredWindow`, WinEvent ile konum).

### Biçim ve renk
| Hücre | Biçim | Renk |
|---|---|---|
| `CPU:` / `GPU:` / `MEM:` % | `15 %` (tam sayı) | ≥ 90 kırmızı, ≥ 75 turuncu |
| `CPU:` / `GPU:` / `HDD:` °C | `63 °C` | CPU/GPU ≥ 90 kırmızı, ≥ 80 turuncu; HDD ≥ 65 kırmızı, ≥ 55 turuncu |
| `↑:` / `↓:` | `0.67 KB/s`, `22.6 KB/s`, `1.24 MB/s` (3 anlamlı basamak) | normal |
| Veri yok | `—` | gri |

Eşikler `taskbar.toml` `[sys]` altında değiştirilebilir. Renkler AI bloğuyla aynı palet (uyarı `0xF0A030`, kritik `0xE5484D`, gri `0x9A9A9A`).

## Faz 1 kararları (kullanıcı, 2026-11-17)
| Konu | Karar |
|---|---|
| GPU kaynağı | **D3DKMT** (NVML değil). Ölçüm: NVML sürece +19,8 MiB private (tray bütçesi ~5 MB), okuma 14 µs; D3DKMT +0 MiB, okuma ~1,6 ms. Değerler aynı (0 %, 46 °C). Ek kazanç: üreticiden bağımsız |
| CPU °C | **ACPI termal bölge (PDH) kalır.** Aynı anda tümleşik GPU'nun D3DKMT sıcaklığı 52–54 °C, ACPI bölgesi 74–85 °C; hangisinin çip sıcaklığına yakın olduğu doğrulanmadı |
| Okuma ritmi | GPU + CPU °C her okumada (2 s); disk °C 15 okumada bir (30 s): sorgu aygıt G/Ç'si, 5–35 ms bekletir. Okuma arayüz iş parçacığında yapılmaz |

## Faz 2 notları (2026-11-17)
| Konu | Uygulama |
|---|---|
| Bloklar | `Bar::new(.., blocks)`, `show(blok, Grid)` / `hide(blok)`; her blok ayrı alt pencere, blok numarası tıkla birlikte gelir (`PostMessage(owner, msg, EV_*, blok)`). Blok 0 = sistem (sağda), blok 1 = AI (solunda) |
| Sıra | Sabit (sistem sağda). `order` ayarı eklenmedi; istenirse sonra |
| Monitör | Faz 2'de iki blok da ortak `monitor` ayarındaki taskbar'daydı; blok başına monitör Faz 3'te geldi |
| `taskbar.toml` | `[ai] enabled`; `[sys] enabled`, `cpu_temp`, `warn_pct` / `crit_pct`, `temp_warn_c` / `temp_crit_c`, `disk_warn_c` / `disk_crit_c`. Eski düz ayarlar (`monitor`, `gap_px`, `reserved_px`, `font`, `font_pt`) aynen geçerli |
| Genişlik | Sütun genişliği yalnız büyür (sayılar değiştikçe blok daralıp genişlemesin, yanındaki AI bloğu oynamasın); DPI değişince sıfırlanır |
| Çizim ritmi | Sistem bloğu pipe `Sample` ritminde (2 s) çizilir; sensör iş parçacığı yalnız değeri günceller (servis kapalıysa çizimi o tetikler). İçerik aynıysa çizim atlanır; bir blok değişince diğeri yeniden çizilmez |
| Tık | Sistem bloğu sol tık `lowmon-ui`'ı açar / öne getirir (yeni açılışta Canlı; zaten açıksa bulunduğu sekmede kalır). AI bloğu sol tık AI sekmesi |
| Ölçüm | `docs/olcumler/2026-11-17_tray-sys-taskbar.md`: tray private 6,74 → 9,25 MiB, CPU 1,04 → 1,74 ms/s, exe 836 608 → 879 104 B |

## Faz 3 kararları (kullanıcı, 2026-11-17)
| Konu | Karar / uygulama |
|---|---|
| Varsayılan | **Sistem ızgarası diğer monitörde**, AI ortak monitörde kalır. Ayar yazmadan geçerli (`[sys] monitor` varsayılanı `other`) |
| `other` | Ortak `monitor` ayarının seçtiği monitörden farklı olan; birden çoksa ona en yakını |
| Geri dönüş | Başka monitör yoksa ya da o monitörde taskbar yoksa ("görev çubuğunu tüm ekranlarda göster" kapalı) blok ortak monitöre döner: AI'ın sağında, Faz 2'deki gibi |
| Ayar | `taskbar.toml`: `[sys] monitor` / `[ai] monitor` = `primary` / `left` / `right` / `other`; `[ai]`'de yoksa ortak `monitor` |
| Tak / çıkar | `WM_DISPLAYCHANGE` → bloklar hemen ve 3 s sonra bir kez daha yeniden kurulur (explorer ikincil taskbar'ı mesajdan sonra açar / kapatır) |
| İkincil taskbar | Bildirim alanı yok → blok sağdan `reserved_px` (saat) kadar içeride durur; DPI taskbar başına |

## Fazlar
| Faz | İş | Doğrulama |
|---|---|---|
| 1 | `lowmon-sensors` crate + `examples/dump` (değerleri yazar) | Görev Yöneticisi / `nvidia-smi` ile karşılaştırma; **ölçüm** `docs/olcumler/2026-11-17_lowmon-sensors.md`: kaynak başına private, okuma süresi, açılış |
| 2 | `lowmon-taskbar` çok blok + tray `sys` modülü + `taskbar.toml` | canlı taskbar, AI ile yan yana; tray private / CPU önce-sonra |
| 3 | Sistem bloğu diğer monitörde (takılıysa; takılı değilse birincil) | iki monitörle canlı |
| 4a | Canlı `lowmon-ui` + telefon: deck `TOPIC_SENSORS` (aşağıdaki ek) | 2026-11-17 |
| 4b | Sıcaklık / GPU geçmişi: tray dakikalık halka dosyası (ADR-0035) | 2026-11-17 |

## Sonuçlar
- (+) İstenen görüntü; servis, proto, segment biçimi değişmez. Bloklar genel: sonraki göstergeler aynı yapıyı kullanır.
- (−) **PDH istisnası:** tray `pdh.dll` yükler. Ölçüm (Faz 1): +4,2 MiB private, +2 iş parçacığı, açılış ~0,28 s, okuma 0,14 ms; tray bütçesi (spek §1: ~5,1 MB private, ~0,5–0,8 ms/s) güncellenir. Kabul edilemez çıkarsa CPU °C hücresi kapatılır (`[sys] cpu_temp = false`) ve karar yeniden açılır.
- (−) GPU okuması ~1,6 ms (duvar); tray CPU'su Faz 2'de önce-sonra ölçülür. Ayrık GPU'yu 2 s'de bir sorgulamak dizüstünde GPU'nun uykuya geçmesini engelleyebilir: doğrulanmadı, Faz 2'de pilde bakılır.
- (−) CPU °C ACPI bölgesidir; HWiNFO'nun çip sıcaklığından farklı olabilir.
- (−) Faz 1–2'de GPU / sıcaklık geçmişi yok.
- Spek: §1 Prensipler (PDH istisnası), §1 bütçe (tray satırı), §2 bileşen tablosu (`lowmon-sensors`; `lowmon-gpu` satırı buna bağlanır), §8 tray (sistem bloğu, `taskbar.toml`) ilgili faz PR'ında güncellenir.

## Ek (2026-11-17): Faz 4a — sensörler deck üzerinden (kullanıcı kararı)
- Karar 3 / sonuç satırındaki "üretici kanalı (`ProducerPush`)" yerine: tray'in zaten okuduğu `lowmon-sensors` değeri deck'e verilir; LowDeck `TOPIC_SENSORS = 8` + `S2C::Sensors` (LowDeck #30, ek varyant, `PROTO_VERSION` 4). `lowmon-ui` ve telefon aynı mesajı alır; servis ve pipe değişmez.
- Sensör iş parçacığı: ızgara açıkken **ya da** en az bir deck sensör abonesi varken çalışır; ikisi de yoksa kendisi çıkar (`sys::sync`, abonelik değişince `WM_DATA`). Maliyet ızgara açıkken değişmez; ızgara kapalıyken yalnız UI/telefon açıkken +2,5 MiB / +0,7 ms/s.
- `lowmon-ui` Canlı: CPU kartında % + CPU °C (aynı 0..100 ekseni), yeni GPU kartı (% + °C), disk °C disk kartı başlığında.
- Geçmiş (4b): tray kullanıcı dosyası, ADR-0035.

