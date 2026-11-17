# LowMonitor — Proje Spesifikasyonu

> Windows için çok düşük tüketimli kaynak izleme, loglama ve müdahale aracı.
> Dil: **Rust** · Hedef: `x86_64-pc-windows-msvc` · Durum: Tasarım tamamlandı, iskelet kurulacak
> Tarih: 2026-11-14 · Sahip: Ahmet Özgür Kalkan

---

## 1. Hedefler ve bütçe

> **ADR-0017:** Aşağıdaki sayılar kesin kabul sınırı değil, yön gösteren hedeftir; her milestone'da ölçülüp raporlanır.

| Metrik | Hedef (lowmon-svc) | Ölçüm yöntemi |
|---|---|---|
| CPU (ortalama, 24 saat) | < %0.05 **toplam CPU** (CPU süresi ÷ (süre × mantıksal çekirdek); ms/s da raporlanır — ADR-0026) | `scripts/soak.ps1` `cpu_s` farkı, WPR/WPA |
| Private WS + Private Bytes | < 3 MB (her biri; toplam WS paylaşılan DLL sayfası içerir, hedef değil — ADR-0015) | `Get-Counter "\Process(lowmon-svc)\Working Set - Private"`, Process Explorer → Private Bytes |
| Disk yazma | Dakikada 1 `WriteFile` (~1.5 KB) | Resource Monitor → Disk |
| Wake-up | 1/s (timer coalescing açık) | `powercfg /energy` |
| Exe boyutu | < 500 KB | Release build |

### Tahmini tüketim dökümü (M1'de ölçülerek doğrulanacak)

| Bileşen | CPU | RAM (WS) | Ne zaman |
|---|---|---|---|
| `lowmon-svc` | ~0.4 ms/s → tek çekirdeğin ~%0.04'ü; Görev Yöneticisi'nde %0.00–0.01 | 2–3 MB | Sürekli |
| `lowmon-tray` (deck + AI + taskbar + sistem ızgarası, ADR-0031/0032/0033) | ~1,7 ms/s (ölçüm `2026-11-17_tray-sys-taskbar`; sistem ızgarası kapalı ~1,0 ms/s) | ~9,3 MB private (sistem ızgarası kapalı ~6,7; CPU °C kapalı ~−4) | Sürekli |
| `lowmon-net` | ~20 µs/s (istemci varken) | 1.5–2.5 MB | İsteğe bağlı, sürekli |
| `lowmon-ui` (ADR-0031) | %0.3–1 (≤ 1 Hz çizim; LowDeck ölçümü Canlı ≈ 1 ms/s) | ~5 MB private (software renderer) | Sadece açıkken |

- **svc CPU:** okuma ~50 µs + kodlama ~10 µs + Top-N ortalaması ~400 µs/s (maliyetin ~%85'i). Şifreleme ~1 µs/dk.
- **svc RAM:** runtime + exe ~1 MB, `SystemProcessInformation` buffer'ı 0.5–1 MB, diğer < 100 KB.
- **İlk ayar düğmesi:** `topn_every` 5 → 10 s yapılırsa svc CPU'su yarıya iner.
- **Referans:** Görev Yöneticisi ~%1 / 30–50 MB, HWiNFO ~%0.5–1 / 30–60 MB.

**Prensipler**
- PDH, WMI, .NET, WebView yok. Doğrudan Win32/NT API. Tek istisna (ADR-0033): tray'de ACPI termal bölge sayacı PDH ile okunur (CPU °C; başka sürücüsüz yol yok). Servise PDH girmez.
- Async runtime (tokio) yok; tek toplayıcı thread + olay tabanlı pipe thread.
- Kimse dinlemiyorsa canlı akış **üretilmez**.
- En az yetki: servis LocalService, müdahale kullanıcı token'ı / UAC ile.

---

## 2. Bileşenler

| Exe / crate | Rol | Çalışma hesabı | Not |
|---|---|---|---|
| `lowmon-proto` (lib) | Metrik tipleri + pipe protokolü; yalnız serde + postcard | — | Dış istemciler bağımlanır (ADR-0022); `lowmon-deck` yol bağımlılığı |
| `lowmon-core` (lib) | Kayıt formatı, kripto, config; `lowmon-proto`'yu yeniden dışa verir | — | Tüm exe'ler paylaşır |
| `lowmon-svc` | Toplayıcı, log yazıcı, alarm motoru, pipe sunucu | LocalService + servis SID'i | Windows servisi |
| `lowmon-deck` (lib) | Telefon/arayüz sunucusu: hub, TCP dinleyici (LAN + loopback), UDP keşif, Core Audio/SMTC/kilit, pipe köprüsü (ADR-0031) | — | `windows` crate (COM/WinRT); `lowdeck-proto` LowDeck'ten git rev |
| `lowmon-ai` (lib) | Claude abonelik kullanımı (5 sa oturum + hafta): Claude Code OAuth (+ yenileme) / sessionKey, WinHTTP, tempo seviyesi, limit CSV'si; sorgu yalnız Claude Desktop/CLI/VS Code açıkken (ADR-0032) | — | `windows-sys` (WinHTTP, DPAPI, ToolHelp); JSON span tabanlı (kimlik dosyası bayt-korumalı yamalanır) |
| `lowmon-taskbar` (lib) | Taskbar'a gömülü renkli metin blokları (2 satır × N sütun hücre ızgarası; bloklar yan yana) (`Shell_TrayWnd` alt penceresi, GDI+); AI kullanımı, ileride trafik vb. (ADR-0032) | — | `windows-sys` (GDI+, WinEvent); katmanlı alt pencere için exe manifesti (supportedOS Win10) |
| `lowmon-tray` | Tray ikon, tooltip, alarm bildirimi; `lowmon-deck`'i ve AI işçisini süreç içinde çalıştırır | Kullanıcı | GUI framework yok |
| `lowmon-hook` | Claude Code hook komutu: stdin'deki olaydan üç alan (`hook_event_name`, `session_id`, `tool_name`) → tray'e `WM_COPYDATA`, çıkar (ADR-0034) | Kullanıcı (Claude Code başlatır) | Yalnız `kernel32` + `user32`; her zaman sessiz, çıkış 0 |
| `lowmon-ui` | Grafikler, olaylar, geçmiş, süreçler (ADR-0031; eski LowDeck masaüstü) | Kullanıcı (asInvoker), tray'deki deck'e loopback | Slint (Royalty-free 2.0, README atfı) |
| `lowmon-sensors` (lib) | Servisin ölçmediği sensörler, tray içinde (ADR-0033): GPU % + °C (D3DKMT, ayrık GPU), disk °C (`StorageDeviceTemperatureProperty`), CPU °C (ACPI termal bölge, PDH). Yalnız taskbar sistem bloğu açıkken okunur; geçmişe yazılmaz | — | `windows-sys` (D3DKMT, PDH, Ioctl) |
| `lowmon-gpu` | GPU / sıcaklık üreticisi (geçmiş + telefon; `lowmon-sensors`'ı kullanır) | Kullanıcı | **Sonra tasarlanacak** (ADR-0033 Faz 4) |
| `lowmon-net` | LAN veri yayıncısı (WebSocket), Mi Note 10 Lite vb. istemciler için | Kullanıcı | Bkz. §16 |

```
┌──────────────┐  named pipe (postcard)  ┌──────────────┐
│ lowmon-tray  │◄──────────────────────►│              │
├──────────────┤                         │  lowmon-svc  │──► %ProgramData%\LowMonitor\logs\*.lwm
│ lowdeck-agent│◄──────────────────────►│  (session 0) │      (XChaCha20-Poly1305 + zincir)
├──────────────┤                         │              │
│ lowmon-gpu   │───── producer ─────────►│              │
├──────────────┤                         │              │
│ lowmon-net   │◄──── subscriber ────────│              │──► WS (LAN) ──► telefon / diğer istemciler
└──────┬───────┘                         └──────────────┘
       │ ShellExecuteExW("runas")
       ▼
┌──────────────┐
│ lowmon-ui    │  yükseltilmiş kopya: --kill, --service-settings (ADR-0030)
└──────────────┘
```

---

## 3. Repo yapısı

```
LowMonitor/
├─ Cargo.toml                # workspace
├─ rust-toolchain.toml       # stable, msvc
├─ .cargo/config.toml        # target-feature=+crt-static
├─ crates/
│  ├─ lowmon-proto/              # src/{lib.rs, metrics.rs, proto.rs}
│  ├─ lowmon-core/
│  │  └─ src/{lib.rs, metrics.rs, record.rs, codec.rs, crypto.rs, ipc.rs, config.rs}
│  ├─ lowmon-svc/
│  │  └─ src/{main.rs, service.rs, install.rs, collect/{cpu,mem,disk,net,proc}.rs,
│  │          writer.rs, retention.rs, alarm.rs, pipe.rs}
│  ├─ lowmon-tray/
├─ docs/
│  ├─ adr/                   # mimari karar kayıtları
│  └─ log-format.md
├─ tools/
│  └─ lwm-dump/              # debug: log segmentini çözüp CSV'ye döken CLI (admin)
└─ tests/
```

### Workspace `Cargo.toml` (taslak)

```toml
[workspace]
resolver = "2"
members = ["crates/*", "tools/*"]

[workspace.dependencies]
windows          = { version = "0.6x", features = [] }   # crate bazında feature aç
windows-service  = "0.7"
serde            = { version = "1", default-features = false, features = ["derive"] }
postcard         = { version = "1", features = ["use-std"] }
chacha20poly1305 = { version = "0.10", features = ["std"] }   # XChaCha20Poly1305
hkdf             = "0.12"
sha2             = "0.10"
getrandom        = "0.2"
toml             = "0.8"
eframe           = { version = "0.3x", default-features = false, features = ["glow", "default_fonts"] }
egui_plot        = "0.3x"

[profile.release]
opt-level      = 3        # svc için "s" ile karşılaştırılacak
lto            = "fat"
codegen-units  = 1
panic          = "abort"
strip          = true
debug          = false

[profile.release-ui]      # lowmon-ui + bağımlılıkları "s" (ölçüm 2026-11-16: 10,52 → 8,90 MB)
inherits       = "release"
opt-level      = "s"
```

> Sürüm numaraları iskelet kurulurken `cargo add` ile güncel değerlere sabitlenecek.

---

## 4. lowmon-svc — Veri toplama

| Metrik | API | Periyot | Tahmini maliyet |
|---|---|---|---|
| CPU toplam + çekirdek | `NtQuerySystemInformation(SystemProcessorPerformanceInformation)` | 1 s | ~5 µs |
| RAM, commit, cache | `GlobalMemoryStatusEx`, `GetPerformanceInfo` | 1 s | ~5 µs |
| Disk okuma/yazma | `DeviceIoControl(IOCTL_DISK_PERFORMANCE)` → `\\.\PhysicalDriveN` | 1 s | ~10 µs/disk |
| Ağ rx/tx | `GetIfEntry2` (aktif arayüz indeksleri cache) | 1 s | ~10 µs/arayüz |
| Top-N süreç | `NtQuerySystemInformation(SystemProcessInformation)` | 5 s | ~1–3 ms |

**Uygulama kuralları**
- Disk handle'ları ve arayüz listesi başlangıçta açılır; `WM_DEVICECHANGE` yerine 60 s'de bir yeniden keşif (servis penceresiz).
- `SystemProcessInformation` buffer'ı tek sefer ayrılır, `STATUS_INFO_LENGTH_MISMATCH` gelirse ×1.5 büyütülür, küçültülmez.
- Top-N: CPU delta'sı için `pid + CreateTime` anahtarlı küçük hash map; ölen süreçler her taramada temizlenir.
- Tüm hesaplar delta tabanlı; ilk örnek atılır.

**Zamanlama ve güç**
```rust
// Kavramsal akış
let timer = CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_MANUAL_RESET, TIMER_ALL_ACCESS)?;
SetWaitableTimerEx(timer, &due, 1000 /*ms period*/, None, None, None, 100 /*TolerableDelay ms*/)?;
// EcoQoS
let state = PROCESS_POWER_THROTTLING_STATE {
    Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    StateMask:   PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
};
SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, &state, size_of_val(&state))?;
SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS)?;
// Döngü: WaitForMultipleObjects([timer, stop_event]) → collect → encode → (60 örnekte bir) flush
```

---

## 5. Log formatı (`.lwm`)

### 5.1 Genel
- Günlük segment: `%ProgramData%\LowMonitor\logs\YYYY-MM-DD.lwm`, append-only.
- 60 örnek = 1 blok → dakikada 1 yazma.
- Her blok **XChaCha20-Poly1305** ile şifreli (192-bit rastgele nonce → nonce çakışma riski pratikte yok).
- **Hash zinciri:** her bloğun AAD'si = önceki bloğun tag'i + blok indeksi. Blok silme, sıra değişimi, araya ekleme, bit flip → doğrulamada yakalanır.

### 5.2 Anahtar yönetimi
> **Güncel (ADR-0016):** anahtarı kurulum değil, servis ilk açılışta LocalService DPAPI'siyle üretir; Administrators'ın key.bin'e erişimi yok, yalnızca silebilir (`--purge`).

- Kurulumda 32 B master key (`getrandom`) → `CryptProtectData` (DPAPI) → `%ProgramData%\LowMonitor\key.bin`.
- `key.bin` ACL: sadece `NT SERVICE\LowMonitor` okuma; Administrators **yok** (log okumak için servis üzerinden geçilir).
- Segment anahtarı: `HKDF-SHA256(master, salt = header.salt, info = "lwm-seg-v1")`.

### 5.3 Dosya düzeni
```
Header (64 B, düz metin, ilk bloğun AAD'sine dahil)
  magic        [u8;4]  = "LWMN"
  version      u16     = 1
  flags        u16
  segment_id   u64     (monoton, state dosyasında tutulur)
  created_unix u64
  interval_ms  u32     (segment açılışındaki aralık, bilgi; çalışırken değişebilir — ADR-0027)
  schema_hash  u32     (kayıt şeması değişince değişir)
  salt         [u8;16]
  reserved     [u8;16]  (alanlar 48 B; 64 B için 16 — 2026-11-14 düzeltme)

Block (tekrarlanır)
  len          u32     (nonce + ciphertext + tag)
  nonce        [u8;24]
  ciphertext   [u8;len-40]
  tag          [u8;16]

Seal block (segment kapanırken, kayıt tipi 0xFF)
  block_count, sample_count, son zincir tag'i
```

### 5.4 Blok içi kayıt tipleri (şifre çözülmüş)
| Tip | Kod | İçerik | Kodlama |
|---|---|---|---|
| Sample | 0x01 | ts, cpu_total, cpu_core[n], ram_used, commit, cache, disk[i].r/w, net[i].rx/tx | ts delta + zigzag varint delta |
| TopN | 0x02 | (dict_id, pid, cpu ‱, ws_kb) × N | varint |
| Event | 0x03 | kind (AlarmRaise/AlarmClear/System), name (ProcDict id), değer (zigzag) — ADR-0028, SCHEMA 2 | varint |
| ProcDict | 0x04 | dict_id → exe adı (segment başına bir kez) | UTF-8 |
| Gap | 0x05 | servis duraklatıldı / uyku / boot | ts aralığı |
| Seal | 0xFF | bkz. 5.3 | — |

**Çözünürlükler:** CPU `u16` (0.01 %), çekirdek `u8` (0.5 %), bellek KB, disk/ağ byte/s.

### 5.5 Boyut ve saklama
| Yaş | Çözünürlük | Boyut |
|---|---|---|
| 0–7 gün | 1 s ham | ~2.5 MB/gün → ~18 MB |
| 8–30 gün | 1 dk (min/avg/max) | ~100 KB/gün → ~2.5 MB |
| > 30 gün | Silinir | — |
| **Toplam** | | **~20 MB** |

- Downsample her gün 03:00, `THREAD_PRIORITY_IDLE` thread'de; çıktı yeni zincirli segment, eski ham segment doğrulandıktan sonra silinir.

### 5.6 Dayanıklılık ve bilinen sınırlar
- `SERVICE_CONTROL_PRESHUTDOWN` ve `STOP`'ta tampon flush + Seal → kayıp yok.
- Çökmede en fazla 60 s kayıp; açılışta mühürsüz segment doğrulanır, `Gap` kaydıyla devam edilir.
- **Sınır:** Açık (mühürsüz) segmentin *son* blokları silinirse tek başına tespit edilemez.
  - Önlem: zincir başı (son tag + blok sayısı) `state.bin`'e ayrı ACL ile yazılır; tutarsızlık açılışta `Event` olarak loglanır.

---

## 6. IPC — `\\.\pipe\LowMonitor`

### 6.1 Güvenlik
- `CreateNamedPipeW` + `PIPE_REJECT_REMOTE_CLIENTS`.
- SDDL: `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)(A;;0x12019b;;;IU)` → SYSTEM/Admin/sunucu tam, interaktif kullanıcı okuma + FILE_WRITE_DATA (instance açamaz); ağ yok. İlk instance `FILE_FLAG_FIRST_PIPE_INSTANCE` (ADR-0018).
- Yetki kontrolü komut bazında: `ImpersonateNamedPipeClient` → `CheckTokenMembership(BUILTIN\Administrators)` → `RevertToSelf`.

### 6.2 Çerçeve
- `u32 LE uzunluk | postcard payload`, maks. 1 MB çerçeve; aşan bağlantı kapatılır.
- İlk mesaj `Hello { proto_version, role }` değilse bağlantı kapatılır.

### 6.3 Mesajlar
| Yön | Mesaj | Yetki |
|---|---|---|
| C→S | `Hello { proto, role: Viewer \| Tray \| Producer }` | Herkes |
| C→S | `Subscribe { channels, rate_ms }` / `Unsubscribe` | Herkes |
| C→S | `QueryRange { from, to, resolution: Raw \| Minute }` (Top-N dönmez, ADR-0020) | Herkes |
| C→S | `QueryEvents { from, to }`: yalnız `RangeItem::Event` (`Range` + `Ack`), PC arayüzü Olaylar ekranı (LowDeck ADR-0003) | Herkes |
| C→S | `Control(Pause \| Resume \| ReloadConfig)` — ayar değişikliği = Admin `config.toml` yazar + `ReloadConfig`; `interval_ms` dahil çalışırken uygulanır (ADR-0027) | Admin |
| C→S | `Export { from, to, format: Csv }` | Admin |
| C→S | `ProducerPush { channel_id, payload }` (GPU vb.) | Producer |
| S→C | `Sample`, `TopN`, `Event`, `Alarms` (CH_EVENT = 4 aboneliğinde aktif liste, `Ack`'tan önce), `RangeChunk` (Export), `Range` (QueryRange, Event dahil), `Ack`, `Error { code }` — ADR-0028 | — |

- Abone yoksa canlı mesaj serileştirilmez bile.
- `QueryRange` sonucu ≤ 64 KB'lık `Range(Vec<RangeItem>)` çerçeveleriyle akıtılır, `Ack` ile biter (ADR-0020). Export CSV'si `RangeChunk` ile.

---

## 7. Alarmlar

```toml
# %ProgramData%\LowMonitor\config.toml
[sampling]
interval_ms = 1000
topn_every  = 5
topn_count  = 5

[retention]
raw_days    = 7
total_days  = 30

[[alarm]]
id       = "cpu_high"
metric   = "cpu.total"
raise    = { above = 90.0, for_s = 30 }
clear    = { below = 80.0, for_s = 10 }

[[alarm]]
id       = "ram_high"
metric   = "mem.used_pct"
raise    = { above = 90.0, for_s = 60 }
clear    = { below = 85.0, for_s = 30 }
```

- Değerlendirme her tick'te, histerezis + süre sayacı (durum makinesi: `Normal → Pending → Raised → Clearing → Normal`); süre örnek ts'iyle ölçülür, tick sayılmaz.
- Metrikler, `device` seçimi (disk no / arayüz takma adı), gap/pause ve ReloadConfig davranışı, Event kapsamı (alarm + sistem olayları): [ADR-0028](kararlar/ADR-0028-alarm-ve-event.md).
- Raise/Clear → `Event` kaydı + pipe üzerinden tray'e push.
- Servis session 0'da → bildirimi **tray** gösterir: `Shell_NotifyIconW(NIM_MODIFY, NIF_INFO)` (Win10/11'de toast olarak görünür, WinRT gerekmez).

---

## 8. lowmon-tray

- `windows-sys` + gizli üst düzey pencere + `Shell_NotifyIconW` (ADR-0018).
- 16×16 ikon: CPU/RAM çift çubuk, `CreateDIBSection` + `CreateIconIndirect`, 2 s'de bir güncelleme.
- Tooltip: `CPU 12% · RAM 58% · ↓1.2 MB/s · 📱 telefon · ⚠ alarmlar · AI %33/%68` (telefon yalnız bağlıyken, AI yalnız veri varken).
- "Arayüzü aç" → tray ile aynı klasördeki `lowmon-ui.exe` (ADR-0031; `UiExe` kaydı kullanılmaz). Exe yoksa pasif, etiket nedeni söyler.
- Sağ tık menü: arayüzü aç, alarmları sessize al (1 saat), **Telefon ▸** (durum: `bind · port · token` + bağlı telefon ya da kapalı nedeni; ayar dosyasını aç, ayarı yeniden yükle, log'u aç, firewall kuralı ekle), **AI kullanımı ▸** (durum: kaynak + son sorgu / hata / Claude kapalı; şimdi yenile, `ai.toml`'u aç, ayarı yeniden yükle, sessionKey kaydet / sil, Claude Code hook'larını kur / kaldır), Windows ile başlat, çıkış.
- Dil (ADR-0032 "Dil kapsamı"): `ai.toml` `language = "en"` / `"tr"` tray menüsünü, bildirimleri, taskbar metnini ve `lowmon-ui`'ı birlikte seçer; varsayılan `tr`.
- Sistem ızgarası (`lowmon-sensors` + `lowmon-taskbar`, ADR-0033): taskbar'da bildirim alanının hemen solunda 2 × 4 hücre: `CPU: 15 %  GPU: 0 %  MEM: 88 %  ↑: 0.67 KB/s` / `CPU: 63 °C  GPU: 44 °C  HDD: 35 °C  ↓: 22.6 KB/s`. CPU %, MEM, ağ servisten (pipe `Sample`); GPU ve sıcaklıklar tray'de `lowmon-sensors` ile (2 s; disk 30 s). Renk: % ≥ 75 turuncu / ≥ 90 kırmızı; CPU-GPU °C ≥ 80 / ≥ 90; disk °C ≥ 55 / ≥ 65; veri yok `—` gri. Sol tık `lowmon-ui`, sağ tık tray menüsü. Ayar `taskbar.toml` `[sys]` (`enabled`, `cpu_temp`, eşikler); kapalıyken sensör okunmaz. Monitör: varsayılan `[sys] monitor = "other"` → ikinci monitör takılıysa (ve taskbar'ı varsa) ızgara oradadır, yoksa AI bloğunun sağında; `[sys]` / `[ai]` `monitor` = `primary` / `left` / `right` / `other`. AI bloğu ortak `monitor` ayarındaki taskbar'da (`[ai] enabled`).
- AI kullanımı (`lowmon-ai` + `lowmon-taskbar`, ADR-0032): taskbar'da bildirim alanının solunda `Oturum %33 · 4s16dk` / `Hafta %68 · 16s26dk`; renk tempo/eşik seviyesi (turuncu uyarı, kırmızı kritik, gri eski). Veri hiç yoksa gizli. Sol tık raporu açar (`lowmon-ui` AI sekmesi), sağ tık tray menüsü. Ayar `ai.toml` + `taskbar.toml` (monitör, boşluk, yazı tipi); log `deck.log` (`ai:` öneki).
- Claude Code etkinlik balonu (ADR-0034): AI bloğunun üstünde `Thinking…` / araç adı / `Waiting for input` (turuncu) / `Done` (5 s). Kaynak Claude Code hook'ları → `lowmon-hook.exe` → `WM_COPYDATA`; yoklama yok. Hook'lar `~/.claude/settings.json`'a yalnız menüden yazılır (`async`, yalnız kendi girdileri, ilk yazmada `.lowmon-backup`). `taskbar.toml` `[ai] bubble`.
- Telefon sunucusu (`lowmon-deck`, ADR-0031): ayar `%APPDATA%\LowMonitor\deck.toml` (ilk açılışta LowDeck `agent.cfg`'den taşınır), log `deck.log`. Port kullanımdaysa tray çalışır, telefon pasif (menüde neden). Medya/ses kontrolü yalnız telefondan (ADR-0031 §6). AI kullanımı `TOPIC_AI` ile telefona ve `lowmon-ui`'a gider (son durum önbellekli, değişince push; ADR-0032).
- Servis yoksa ikon gri, 10 s'de bir yeniden bağlanma denemesi.
- Otomatik başlatma: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

---

## 9. Arayüz (`lowmon-ui`)

> **ADR-0019 → ADR-0029 → ADR-0031:** UI Slint ile, bu repoda `lowmon-ui` (eski LowDeck masaüstü). Veriye yalnız tray'deki `lowmon-deck` üzerinden erişir (loopback, `lowdeck-proto`); port + token `deck.toml`'dan. Pipe'a bağlanan deck'tir. Medya/ses kontrolü yok, yalnız izler (ADR-0031 §6).

**Ekranlar** (sıra = teslim sırası). Üst çubuk: sekmeler; sağda bağlantı noktası (sorunsuzken yalnız yeşil nokta, metin üzerine gelince; kopuk / servis yok → turuncu nokta + metin) ve ⚙ (Ayarlar, sekme 6).
1. **Canlı:** CPU (toplam + çekirdek) + CPU °C, GPU % + °C, RAM/commit, disk (+ en sıcak disk °C), ağ (son 5 dk). Sıcaklık/GPU tray sensörlerinden deck `TOPIC_SENSORS` ile (ADR-0033 Faz 4a). — ilk teslimat. Üstte AI bandı: Claude oturum/hafta %, kalan süre, seviye rengi (ADR-0032; veri yoksa gizli).
2. **Olaylar:** `CH_EVENT` canlı + `QueryRange` `RangeItem::Event` geçmişi; aktif alarmlar; her olayın altında okunur açıklama (sistem olayları adına göre, alarm metrik önekine göre). — ilk teslimat
3. **Geçmiş:** zaman aralığı → `QueryRange`, otomatik çözünürlük (aralık > 6 saat → 1 dk); aralık seçici grafiklerin altında. CPU °C, GPU % + °C, disk en yüksek °C tray'in dakikalık dosyasından (`%LOCALAPPDATA%\LowMonitor\sensors.bin`, 7 g halka, ort + max; ADR-0035).
4. **Süreçler:** `lowmon-ui` süreçleri kendisi tarar (2 s, yalnız sekme açıkken; Top-N kullanılmaz) + müdahale (§10, ADR-0030).
5. **AI:** Claude kullanım raporu (ADR-0032 Faz 5): limit çizgisi (oturum + hafta %), token dökümü (araç grubu başına efektif maliyet), oturum tablosu; Son oturum / Bugün / Bu hafta. Veri `lowmon-ai` ile dosyalardan (`~/.claude/projects` dökümleri, limit CSV'si, bulut özetleri), yalnız sekme açılınca / Yenile ile, arka iş parçacığında; yalnız sayılar, araç ve proje klasörü adları (mesaj metni okunup saklanmaz). `lowmon-ui --ai` bu sekmeyle açar.
6. **Ayarlar:** ilk parti (yapıldı): kullanıcı dosyaları `ai.toml` (dil, sorgu aralığı, kimlik, rapor oturumu, CSV), `taskbar.toml` (monitör, yazı boyutu, AI / sistem bloğu, monitörleri, balon, CPU °C, eşikler), `deck.toml` (bind, port). Yalnız değişen anahtar yazılır (`toml_edit`, yorum + sıra korunur); sonuç dosyanın ayrıştırıcısından geçmezse yazılmaz; kaydedince tray'e `WM_APP + 6` (bit 0 ai + taskbar, bit 1 deck) → yeniden yükleme. Servis ayarları (`ProgramData\LowMonitor\config.toml`, kullanıcıya kapalı): "Servis ayarlarını düzenle" → UAC → `lowmon-ui.exe --service-settings`; yükseltilmiş kopya `[sampling]` (`interval_ms`, `topn_every`, `topn_count`) ve `[retention]` (`raw_days`, `total_days`) alanlarını aynı kurallarla (`toml_edit`, `Config::parse` doğrulaması) yerinde yazar (kökte BA dosya oluşturamaz, korumalı DACL korunur), ardından `Control::ReloadConfig` gönderir (ADR-0030 eki). Sonraki parti: alarm kuralları.

**Performans kuralları (Slint)**
- Yeniden çizim yalnız veri değişince, ≤ 1 Hz; sürekli timer/animasyon yok.
- Simge durumu → abonelik bırakılır; geri gelince yeniden abone.
- Grafik verisi sabit kapasiteli ring buffer.
- Grafik hover'ı (Canlı + Geçmiş): dikey çizgi + kutucuk (saat / kova aralığı + o andaki seriler). Metin Rust'ta en yakın örnekten (`Hover.text`); yeniden çizim yalnız fare hareketinde ve yeni veride.
- Yetki: arayüz ve tray yükseltilmemiş; Admin işleri UAC ile: servis ayarı yazma ve süreç müdahalesi `lowmon-ui`'nın yükseltilmiş kopyası (ADR-0030).

---

## 10. Müdahale (kill / priority / affinity / EcoQoS)

| İşlem | API | Gerekli hak |
|---|---|---|
| Sonlandır | `TerminateProcess` | `PROCESS_TERMINATE` |
| Öncelik | `SetPriorityClass` | `PROCESS_SET_INFORMATION` |
| Affinity | `SetProcessAffinityMask` | `PROCESS_SET_INFORMATION` |
| EcoQoS | `SetProcessInformation(ProcessPowerThrottling)` | `PROCESS_SET_INFORMATION` |

**Akış**
1. UI, kullanıcı token'ıyla `OpenProcess(gerekli_hak | PROCESS_QUERY_LIMITED_INFORMATION)` dener.
2. **PID reuse koruması:** `GetProcessTimes` → `CreationTime` snapshot'takiyle eşleşmezse **iptal**.
3. `ERROR_ACCESS_DENIED` → onay → `ShellExecuteExW(verb="runas", "lowmon-ui.exe --kill 1234 133...")` (ADR-0030; LowDeck ADR-0006).
4. Yükseltilmiş kopya pencere açmadan aynı kontrolleri (CreationTime + yasak liste) tekrar yapar, işlemi uygular, exit code ile sonucu döner, kapanır.
5. Her müdahale servise bildirilir → `Event` olarak loglanır (kim, ne, hangi süreç). **Ertelendi** (ADR-0030 madde 4).

İlk teslimat yalnız Sonlandır; öncelik / affinity / EcoQoS sonra.

**Yasak liste:** `csrss`, `wininit`, `smss`, `lsass`, `services`, `winlogon`, `System`, `Registry`, PID 0/4, arayüzün kendisi → kill butonu yok.

---

## 11. Kurulum

```powershell
# Admin PowerShell
.\lowmon-svc.exe install     # servis kaydı + SID + ACL + anahtar + config
.\lowmon-svc.exe start
.\lowmon-svc.exe uninstall [--purge]   # --purge: log + anahtar + config silinir
```

`install` adımları:
1. `CreateServiceW` — `SERVICE_AUTO_START` (delayed), hesap `NT AUTHORITY\LocalService`.
2. `ChangeServiceConfig2W(SERVICE_CONFIG_SERVICE_SID_INFO, SERVICE_SID_TYPE_UNRESTRICTED)`.
3. `SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO` → gereksiz privilege'lar düşürülür.
4. `SERVICE_CONFIG_PRESHUTDOWN_INFO` → 5 s.
5. `%ProgramData%\LowMonitor\{logs,state}` oluştur, ACL:
   - `NT SERVICE\LowMonitor`: Modify
   - `SYSTEM`: Full
   - `Administrators`: Read (logs), **erişim yok** (`key.bin`)
   - `Users`: erişim yok
6. Master key üret → DPAPI → `key.bin`.
7. Varsayılan `config.toml` yaz (yoksa).
8. Failure actions: 3 kez yeniden başlat (5 s / 30 s / 5 dk).

Tam kurulum/güncelleme `scripts\install.ps1` (yönetici, repo kökünden, release build sonrası):
1. Exe'leri kilitleyen süreçler durur: `lowmon-tray/ui/net/hook` + eski `lowdeck-agent/desktop`. (`lowmon-hook.exe` de kopyalanır; Claude Code ayarına dokunulmaz.)
2. Eski LowDeck PC izleri silinir (ADR-0031): HKCU Run `LowDeckAgent`, `HKCU\Software\LowMonitor` `UiExe`, firewall `LowDeck agent`. `agent.cfg` kalır (deck.toml göçü).
   Eski C++ ai-usage-bar (ADR-0032 Faz 4): `claude-usage-bar` süreci durur, HKCU Run `ClaudeUsageBar` silinir; `%LOCALAPPDATA%\ClaudeUsageBar\usage-*.csv` → `%LOCALAPPDATA%\LowMonitor\ai\` (ts'e göre birleşir), `session.bin` hedefte yoksa kopyalanır, eski klasör silinir (`config.ini` taşınmaz).
3. Servis kaldır → exe'ler `%ProgramFiles%\LowMonitor` → servis kur + başlat.
4. Firewall `LowMonitor deck`: TCP `deck.toml` portu (yoksa 47810) + UDP 47811, private, localsubnet, program = kurulu tray.
5. Tray autostart açılır, explorer üzerinden yükseltilmemiş başlatılır.

---

## 12. Test stratejisi

| Alan | Test |
|---|---|
| Codec | Round-trip property test (`proptest`): rastgele örnek → encode → decode eşitliği |
| Kripto/zincir | Bit flip, blok silme, blok yer değiştirme, header değişimi → hepsi hata vermeli |
| Retention | Sahte saatle 40 günlük veri → boyut ve çözünürlük doğrulaması |
| Alarm | Durum makinesi tablo testleri (histerezis sınırları, kısa spike'lar) |
| IPC | Bozuk çerçeve, 1 MB üstü, yetkisiz `Control` → reddedilmeli |
| Performans | 24 saat soak test; CPU/WS/handle sayısı log'lanır, handle sızıntısı = 0 |
| Müdahale | PID reuse simülasyonu, korunan süreç listesi |

---

## 13. Kilometre taşları

| # | Kapsam | Tamam kriteri |
|---|---|---|
| **M0** | Workspace, release profili, CI (`cargo fmt --check`, `clippy -D warnings`), boş servis | `install/start/stop/uninstall` çalışıyor |
| **M1** | CPU/RAM/Disk/Ağ/Top-N toplayıcı | 24 saatte CPU < %0.05 (toplam, ADR-0026), Private WS / Private Bytes < 3 MB |
| **M2** | Şifreli + zincirli log, seal, retention, `lwm-dump` | Kurcalama testlerinin tamamı yakalanıyor |
| **M3** | Pipe protokolü + tray | Abone yokken sıfır akış, tray yeniden bağlanıyor |
| **M4** | Geçmiş sorgusu, müdahale (UI ve müdahale LowDeck'te, ADR-0029/0030) | Yükseltilmiş süreç kill'i UAC ile çalışıyor |
| **M5** | Alarmlar + bildirim | Histerezis testleri geçiyor, toast görünüyor |
| **M6** | `lowmon-net` (WS + eşleştirme + firewall kuralı) | `websocat` ile token'lı bağlantı 1 Hz JSON alıyor; token'sız bağlantı reddediliyor |
| Sonra | `lowmon-gpu` (NVML), MQTT → Home Assistant | Ayrı tasarım |

---

## 14. Karar kaydı (ADR özeti)

| # | Karar | Gerekçe |
|---|---|---|
| ADR-001 | Rust, MSVC target | Bellek güvenliği, native performans, tek exe |
| ADR-002 | Servis / UI / tray ayrı süreçler | UI kapalıyken sıfır GUI maliyeti, yetki ayrımı |
| ADR-003 | PDH/WMI yerine NT API | ~10× düşük maliyet, ek DLL yükü yok |
| ADR-004 | LocalService + servis SID | En az yetki, ACL'lerde izolasyon |
| ADR-005 | Binary + XChaCha20-Poly1305 + hash zinciri | En hafif format + gizlilik + kurcalama tespiti |
| ADR-006 | UI log dosyasına doğrudan erişmez | Anahtar sadece serviste |
| ADR-007 | egui/eframe (glow) | Hızlı geliştirme, wgpu'dan hafif |
| ADR-008 | Müdahale UI'da, gerekirse UAC helper | Servis yetkisi düşük kalır |
| ADR-009 | 1 s / 30 gün, 7 gün sonrası 1 dk downsample | ~20 MB toplam disk |
| ADR-010 | Self-install (`lowmon-svc install`) | Portable, MSI gereksiz |
| ADR-011 | LAN yayını ayrı süreç (`lowmon-net`), servis ağa açılmaz | Ağ saldırı yüzeyi servisten izole, kapatılabilir |
| ADR-012 | WebSocket + JSON, 1 Hz push | Web/PWA istemcide doğrudan tüketilir; ~150 B/s, polling yok |
| ADR-013 | Telefon istemcisi ayrı proje | LowMonitor sadece API sözleşmesini garanti eder |

---

## 15. Açık konular

- [ ] `lowmon-gpu`: NVML metrik seti, örnekleme, producer kanal şeması
- [ ] MQTT köprüsü: `lowmon-net` içinde opsiyonel modül mü, ayrı exe mi?
- [ ] `lowmon-net` için TLS (`wss://`) gerekli mi? (LAN + token şu an yeterli kabul edildi)
- [ ] `opt-level = "s"` vs `3` — servis için ölçüm sonrası karar
- [ ] Çekirdek sayısı > 64 (processor group) desteği gerekli mi?
- [ ] Kod imzalama (UAC prompt'ta "Unknown publisher" görünmemesi için)

---

## 16. lowmon-net — LAN veri yayını (Mi Note 10 Lite ve diğer istemciler)

**Kapsam:** LowMonitor, yerel ağa minimum maliyetle canlı veri yayınlar. İstemci (telefondaki PWA/kiosk) **ayrı proje**; burada sadece sunucu ve API sözleşmesi tanımlanır.

### 16.1 Mimari
```
lowmon-svc ──pipe Subscribe(1 Hz)──► lowmon-net ──ws://<pc-ip>:47800/v1/live──► istemci(ler)
```
- Kullanıcı oturumunda çalışır (tray gibi `HKCU\...\Run`); servis ağa **hiç** açılmaz.
- Senkron `tungstenite` + `std::net::TcpListener`, tokio yok. Bağlantı başına thread yok: tek thread, non-blocking soketler + `WSAPoll`.
- Bağlı istemci yoksa pipe bağlantısı kapatılır → svc tarafında sıfır ek maliyet. İki thread: ağ (WSAPoll, boşta 30 s'de bir uyanır) + pipe okuyucu (ADR-0023).
- Maks. 4 eşzamanlı istemci.

### 16.2 Güvenlik
| Önlem | Detay |
|---|---|
| Bağlama | Sadece seçilen arayüzün IP'si (config), `0.0.0.0` değil |
| Firewall | `lowmon-svc install` sırasında sadece **Private** profil, sadece yerel alt ağ için `lowmon-net.exe` inbound kuralı (`netsh advfirewall`, ADR-0025); `uninstall` siler |
| Eşleştirme | `lowmon-net pair` → 6 haneli kod 120 s geçerli, tek kullanımlık (DPAPI `pair.bin` ile sunucuya geçer) → istemci `POST /v1/pair {code}` → 256-bit token döner (ADR-0024) |
| Token saklama | PC tarafında token'ın SHA-256 hash'i, DPAPI (kullanıcı kapsamı) |
| Kimlik doğrulama | WS el sıkışmasında `Authorization: Bearer <token>` veya `?token=` (tarayıcı WS header desteklemediği için) |
| Brute-force | Hatalı denemeden sonra o IP'ye 1 s yeni bağlantı yok, IP başına 10 hata → 10 dk ban |
| İptal | `lowmon-net unpair --all`: tüm token'lar silinir, açık bağlantılar ≤ 30 s'de düşer |
| Yetki | Sadece okuma. Müdahale komutları (kill/priority) LAN'dan **kabul edilmez** |
| Origin | `Origin` header varsa izin listesinde olmalı (config); yoksa yalnız token |

### 16.3 API sözleşmesi (v1)
| Endpoint | Yöntem | Açıklama |
|---|---|---|
| `/v1/pair` | POST | Eşleştirme kodu → token |
| `/v1/info` | GET | Host adı, çekirdek sayısı, toplam RAM, disk/arayüz listesi `[{id, name}]` (token gerekli; id'ler `/v1/live` ile aynı küme, ADR-0025) |
| `/v1/live` | WS | 1 Hz push, sunucu → istemci |

**`/v1/live` mesajı (~150 B, minified JSON):**
```json
{
  "v": 1,
  "ts": 1763128800,
  "cpu": 12.4,
  "cores": [8, 15, 3, 22, 9, 11, 4, 7],
  "ram": { "used_mb": 9300, "total_mb": 16384 },
  "disk": { "r_bps": 120000, "w_bps": 54000 },
  "net":  { "rx_bps": 1250000, "tx_bps": 84000 },
  "gpu":  null,
  "top":  [["chrome.exe", 4.1], ["Code.exe", 2.3], ["Discord.exe", 1.0]],
  "alarms": ["cpu_high"]
}
```
- `gpu` alanı `lowmon-gpu` gelene kadar `null`.
- İstemci → sunucu tek mesaj: `{"rate_ms": 1000 | 2000 | 5000}` (istemci ekranı kapalıyken 5000'e düşebilir).
- Ping/pong 30 s; 2 kayıp pong → bağlantı kapatılır.
- Şema değişince `v` artırılır, eski sürüm bir major boyunca desteklenir.

### 16.4 Tüketim
| Kalem | Değer |
|---|---|
| CPU | JSON serileştirme + `send` ≈ 20 µs/s → ~%0 |
| RAM | 1.5–2.5 MB |
| Ağ | ~150 B/s/istemci (~13 MB/gün) |

### 16.5 Telefon istemcisi için notlar (ayrı projeye girdi)
- Mi Note 10 Lite AMOLED → burn-in: siyah arka plan, 60 s'de bir 2–4 px piksel kaydırma, gece karartma.
- Ekran kapalıyken `rate_ms: 5000` göndermek pil ve ağ yükünü düşürür.
- Ön koşul: telefonun bootloop sorunu çözülmüş olmalı.

### 16.6 Config
`%APPDATA%\LowMonitor
et.toml` (kullanıcı kapsamı; servis config'i kullanıcıya okunamaz, ADR-0023). İlk açılışta varsayılanla oluşur.
```toml
[net]
enabled        = true
bind           = "192.168.1.20"   # PC'nin LAN IP'si
port           = 47800
max_clients    = 4
allowed_origins = ["http://192.168.1.30"]
```
