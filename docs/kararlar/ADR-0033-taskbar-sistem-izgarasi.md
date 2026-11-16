# ADR-0033: Taskbar'da sistem ızgarası: `lowmon-sensors` + çok bloklu `lowmon-taskbar`
- Tarih: 2026-11-17
- Durum: Taslak (kullanıcı kararları alındı; "Açık" maddeleri kartla sorulur; uygulama fazlar halinde, her faz ayrı PR).

## Bağlam
Kullanıcı taskbar'da, AI göstergesinin (ADR-0032) yanında 2 satır × 4 sütun bir sistem ızgarası istiyor:

```
CPU: 15 %   GPU:  0 %   MEM: 88 %   ↑: 0.67 KB/s
CPU: 63 °C  GPU: 44 °C  HDD: 35 °C  ↓: 22.6 KB/s
```

| Değer | Bugün | Kaynak |
|---|---|---|
| CPU %, MEM %, ağ ↑↓ | Var: `lowmon-svc` ölçer, tray pipe'tan 2 s'de bir `Sample` alır | değişmez |
| GPU %, GPU °C | Yok (spek §2: `lowmon-gpu` "sonra tasarlanacak") | NVML |
| HDD °C | Yok | NVMe / ATA sıcaklık sorgusu |
| CPU °C | Yok | ACPI termal bölge |

Donanım (bu PC, 2026-11-17): Ryzen 7 5800H, RTX 3060 Laptop + AMD tümleşik GPU, Micron NVMe SSD; `nvml.dll` System32'de.

## Karar (kullanıcı, 2026-11-17)
1. **Yerleşim:** sistem ızgarası ve AI bloğu **yan yana**. Sonraki adım: sistem ızgarası takılıysa **diğer monitöre** → her blok kendi monitör ayarını taşır.
2. **CPU °C: ACPI termal bölge.** Spek §1 "PDH/WMI yok" (ADR-003) kuralına **dar istisna**: yalnız bu tek sayaç, yalnız tray'de (servis değil).
3. **Dil:** taskbar etiketleri İngilizce (`ai.toml language = "en"` yapıldı). Tray menüsü + `lowmon-ui` en/tr ayrı iş (ayrı PR, bu ADR dışında).

## Tasarım

### Yeni crate: `lowmon-sensors` (lib)
Servisin ölçmediği üç kaynak. Her biri bağımsız: yoksa / hata verirse hücre `—` olur, diğerleri çalışır.

| Kaynak | Yöntem | Not |
|---|---|---|
| GPU %, °C (deneme: `nvidia-smi` 0 %, 46 °C) | `nvml.dll` çalışma anında yüklenir (`LoadLibraryW` + `GetProcAddress`; link bağımlılığı yok): `nvmlInit_v2`, `nvmlDeviceGetHandleByIndex_v2`, `nvmlDeviceGetUtilizationRates`, `nvmlDeviceGetTemperature` | Yalnız NVIDIA. DLL yoksa GPU hücreleri `—`. AMD tümleşik GPU gösterilmez (kaynak yok). Yükseltme gerekmez |
| HDD °C | `IOCTL_STORAGE_QUERY_PROPERTY` + `StorageDeviceTemperatureProperty` (`\\.\PhysicalDriveN`, erişim 0) | Birden çok diskte en sıcak olan. Yükseltmesiz çalışıyor (deneme: `PhysicalDrive0` 35 °C, uyarı 87 / kritik 90) |
| CPU °C | PDH: `\Thermal Zone Information(*)\Temperature` (Kelvin), en sıcak bölge | Yükseltmesiz çalışıyor (deneme: 64,9–75,9 °C, yükle değişiyor). Çip sıcaklığı değil, ACPI bölgesi: kaba olabilir |

Denenip bırakılan (2026-11-17): termal bölge aygıtı (`GUID_DEVICE_THERMAL_ZONE`) + `IOCTL_THERMAL_READ_TEMPERATURE`. Yükseltmesiz açılıyor ama veri dönmüyor (0 bayt). WMI `MSAcpi_ThermalZoneTemperature` yönetici ister. Çekirdek sürücüsü (WinRing0 türü) reddedildi: güvenlik riski.

### Nerede ölçülür: tray (Faz 1–2)
- `lowmon-sensors` tray'de, **yalnız sistem bloğu açıkken**, 2 s'de bir (pipe `Sample` ritmiyle) okunur. Servise dokunulmaz: svc bütçesi (< 3 MB, < 500 KB exe), segment biçimi ve `lowmon-proto` değişmez.
- Sonuç: GPU ve sıcaklıklar **yalnız taskbar'da**; geçmişe yazılmaz, telefona ve `lowmon-ui`'a gitmez. Bunlar ayrı faz (Açık 1); o zaman spek §2'deki üretici kanalı (`ProducerPush`) kullanılır, `lowmon-sensors` aynen taşınır.

### `lowmon-taskbar`: çok blok
- Bugün: tek pencere, `show([Line; 2])`. Yeni: **blok** = satır × sütun hücre ızgarası (`Cell { text, rgb }`), sütunlar hizalı (her sütun kendi en geniş hücresi kadar).
- AI bloğu 2×1, sistem bloğu 2×4. Aynı monitördeki bloklar bildirim alanının solunda yan yana dizilir (sıra ayardan). Farklı monitördeki blok o monitörün taskbar'ına ayrı pencere olarak kurulur.
- `taskbar.toml`: ortak `font`, `font_pt`, `gap_px`; blok başına `[ai]` / `[sys]`: `enabled`, `monitor` (`primary` / `left` / `right`), `order`. Eski tek-blok ayarı (`monitor`, `reserved_px`) AI bloğuna uygulanır.
- Çizim yöntemi değişmez (ADR-0032 Faz 2: `Shell_TrayWnd` alt penceresi, GDI+, `UpdateLayeredWindow`, WinEvent ile konum).

### Biçim ve renk
| Hücre | Biçim | Renk |
|---|---|---|
| `CPU:` / `GPU:` / `MEM:` % | `15 %` (tam sayı) | ≥ 90 kırmızı, ≥ 75 turuncu |
| `CPU:` / `GPU:` / `HDD:` °C | `63 °C` | CPU/GPU ≥ 90 kırmızı, ≥ 80 turuncu; HDD ≥ 65 kırmızı, ≥ 55 turuncu |
| `↑:` / `↓:` | `0.67 KB/s`, `22.6 KB/s`, `1.24 MB/s` (3 anlamlı basamak) | normal |
| Veri yok | `—` | gri |

Eşikler `taskbar.toml` `[sys]` altında değiştirilebilir. Renkler AI bloğuyla aynı palet (uyarı `0xF0A030`, kritik `0xE5484D`, gri `0x9A9A9A`).

## Fazlar
| Faz | İş | Doğrulama |
|---|---|---|
| 1 | `lowmon-sensors` crate + `examples/dump` (değerleri yazar) | Görev Yöneticisi / `nvidia-smi` ile karşılaştırma; **ölçüm**: süreç private + CPU, PDH ve NVML yükü ayrı ayrı |
| 2 | `lowmon-taskbar` çok blok + tray `sys` modülü + `taskbar.toml` | canlı taskbar, AI ile yan yana; tray private / CPU önce-sonra |
| 3 | Sistem bloğu diğer monitörde (takılıysa; takılı değilse birincil) | iki monitörle canlı |
| 4 (ayrı karar) | Geçmiş + telefon + `lowmon-ui`: üretici kanalı | Açık 1 |

## Sonuçlar
- (+) İstenen görüntü; servis, proto, segment biçimi değişmez. Bloklar genel: sonraki göstergeler aynı yapıyı kullanır.
- (−) **PDH istisnası:** tray `pdh.dll` yükler. Maliyet Faz 1'de ölçülür; tray bütçesi (spek §1: ~5,1 MB private, ~0,5–0,8 ms/s) güncellenir. Kabul edilemez çıkarsa CPU °C hücresi kapatılır (`[sys] cpu_temp = false`) ve karar yeniden açılır.
- (−) NVML süreç içine NVIDIA DLL'i yükler (bellek ölçülecek); yalnız NVIDIA.
- (−) CPU °C ACPI bölgesidir; HWiNFO'nun çip sıcaklığından farklı olabilir.
- (−) Faz 1–2'de GPU / sıcaklık geçmişi yok.
- Spek: §1 Prensipler (PDH istisnası), §1 bütçe (tray satırı), §2 bileşen tablosu (`lowmon-sensors`; `lowmon-gpu` satırı buna bağlanır), §8 tray (sistem bloğu, `taskbar.toml`) ilgili faz PR'ında güncellenir.

## Açık (kartla sorulur)
1. GPU / sıcaklık **geçmişe, telefona, `lowmon-ui`'a** da gitsin mi (Faz 4), yoksa yalnız taskbar mı?
2. Sistem bloğu tık davranışı: sol tık `lowmon-ui` Canlı sekmesi (öneri), sağ tık tray menüsü.
