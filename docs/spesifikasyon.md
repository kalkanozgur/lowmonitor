# LowMonitor — Proje Spesifikasyonu

> Windows için çok düşük tüketimli kaynak izleme, loglama ve müdahale aracı.
> Dil: **Rust** · Hedef: `x86_64-pc-windows-msvc` · Durum: Tasarım tamamlandı, iskelet kurulacak
> Tarih: 2026-11-14 · Sahip: Ahmet Özgür Kalkan

---

## 1. Hedefler ve bütçe

> **ADR-0017:** Aşağıdaki sayılar kesin kabul sınırı değil, yön gösteren hedeftir; her milestone'da ölçülüp raporlanır.

| Metrik | Hedef (lowmon-svc) | Ölçüm yöntemi |
|---|---|---|
| CPU (ortalama, 24 saat) | < %0.05 | `Get-Counter "\Process(lowmon-svc)\% Processor Time"`, WPR/WPA |
| Private WS + Private Bytes | < 3 MB (her biri; toplam WS paylaşılan DLL sayfası içerir, hedef değil — ADR-0015) | `Get-Counter "\Process(lowmon-svc)\Working Set - Private"`, Process Explorer → Private Bytes |
| Disk yazma | Dakikada 1 `WriteFile` (~1.5 KB) | Resource Monitor → Disk |
| Wake-up | 1/s (timer coalescing açık) | `powercfg /energy` |
| Exe boyutu | < 500 KB | Release build |

### Tahmini tüketim dökümü (M1'de ölçülerek doğrulanacak)

| Bileşen | CPU | RAM (WS) | Ne zaman |
|---|---|---|---|
| `lowmon-svc` | ~0.4 ms/s → tek çekirdeğin ~%0.04'ü; Görev Yöneticisi'nde %0.00–0.01 | 2–3 MB | Sürekli |
| `lowmon-tray` | ~50 µs / 2 s → ~%0 | 1.5–2 MB | Sürekli |
| `lowmon-net` | ~20 µs/s (istemci varken) | 1.5–2.5 MB | İsteğe bağlı, sürekli |
| `lowmon-ui` | %0.3–1 (1 Hz repaint) | 50–90 MB (çoğu GPU sürücüsü) | Sadece açıkken |

- **svc CPU:** okuma ~50 µs + kodlama ~10 µs + Top-N ortalaması ~400 µs/s (maliyetin ~%85'i). Şifreleme ~1 µs/dk.
- **svc RAM:** runtime + exe ~1 MB, `SystemProcessInformation` buffer'ı 0.5–1 MB, diğer < 100 KB.
- **İlk ayar düğmesi:** `topn_every` 5 → 10 s yapılırsa svc CPU'su yarıya iner.
- **Referans:** Görev Yöneticisi ~%1 / 30–50 MB, HWiNFO ~%0.5–1 / 30–60 MB.

**Prensipler**
- PDH, WMI, .NET, WebView yok. Doğrudan Win32/NT API.
- Async runtime (tokio) yok; tek toplayıcı thread + olay tabanlı pipe thread.
- Kimse dinlemiyorsa canlı akış **üretilmez**.
- En az yetki: servis LocalService, müdahale kullanıcı token'ı / UAC ile.

---

## 2. Bileşenler

| Exe / crate | Rol | Çalışma hesabı | Not |
|---|---|---|---|
| `lowmon-proto` (lib) | Metrik tipleri + pipe protokolü; yalnız serde + postcard | — | Dış istemciler (lowdeck-agent) bağımlanır (ADR-0022) |
| `lowmon-core` (lib) | Kayıt formatı, kripto, config; `lowmon-proto`'yu yeniden dışa verir | — | Tüm exe'ler paylaşır |
| `lowmon-svc` | Toplayıcı, log yazıcı, alarm motoru, pipe sunucu | LocalService + servis SID'i | Windows servisi |
| `lowmon-tray` | Tray ikon, tooltip, alarm bildirimi | Kullanıcı | GUI framework yok |
| `lowmon-ui` | Grafikler, geçmiş, müdahale | Kullanıcı (asInvoker) | egui/eframe (glow) |
| `lowmon-elev` | Yükseltilmiş tek seferlik işlem | Admin (UAC) | İşini yapıp çıkar |
| `lowmon-gpu` | NVML üretici | Kullanıcı | **Sonra tasarlanacak** |
| `lowmon-net` | LAN veri yayıncısı (WebSocket), Mi Note 10 Lite vb. istemciler için | Kullanıcı | Bkz. §16 |

```
┌──────────────┐  named pipe (postcard)  ┌──────────────┐
│ lowmon-tray  │◄──────────────────────►│              │
├──────────────┤                         │  lowmon-svc  │──► %ProgramData%\LowMonitor\logs\*.lwm
│ lowmon-ui    │◄──────────────────────►│  (session 0) │      (XChaCha20-Poly1305 + zincir)
├──────────────┤                         │              │
│ lowmon-gpu   │───── producer ─────────►│              │
├──────────────┤                         │              │
│ lowmon-net   │◄──── subscriber ────────│              │──► WS (LAN) ──► telefon / diğer istemciler
└──────┬───────┘                         └──────────────┘
       │ ShellExecuteExW("runas")
       ▼
┌──────────────┐
│ lowmon-elev  │  kill / priority / affinity / EcoQoS (yükseltilmiş hedefler için)
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
│  ├─ lowmon-ui/
│  └─ lowmon-elev/
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
  interval_ms  u32
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
| Event | 0x03 | alarm_id, state (raise/clear), değer | varint |
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
| C→S | `Control(Pause \| Resume \| SetInterval \| SetRetention \| ReloadConfig)` | Admin |
| C→S | `Export { from, to, format: Csv }` | Admin |
| C→S | `ProducerPush { channel_id, payload }` (GPU vb.) | Producer |
| S→C | `Sample`, `TopN`, `Event`, `RangeChunk` (Export), `Range` (QueryRange), `Ack`, `Error { code }` | — |

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

- Değerlendirme her tick'te, histerezis + süre sayacı (durum makinesi: `Normal → Pending → Raised → Clearing → Normal`).
- Raise/Clear → `Event` kaydı + pipe üzerinden tray'e push.
- Servis session 0'da → bildirimi **tray** gösterir: `Shell_NotifyIconW(NIM_MODIFY, NIF_INFO)` (Win10/11'de toast olarak görünür, WinRT gerekmez).

---

## 8. lowmon-tray

- `windows-sys` + gizli üst düzey pencere + `Shell_NotifyIconW` (ADR-0018).
- 16×16 ikon: CPU/RAM çift çubuk, `CreateDIBSection` + `CreateIconIndirect`, 2 s'de bir güncelleme.
- Tooltip: `CPU 12% · RAM 58% · ↓1.2 MB/s`.
- Sol tık → `lowmon-ui.exe` (named mutex ile tek örnek, açıksa `SetForegroundWindow`).
- Sağ tık menü: UI aç, alarmları sessize al (1 saat), çıkış.
- Servis yoksa ikon gri, 10 s'de bir yeniden bağlanma denemesi.
- Otomatik başlatma: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

---

## 9. lowmon-ui (egui)

> **ADR-0019:** UI Slint ile, kullanıcı tarafından ayrıca geliştirilecek. Aşağıdaki ekran listesi geçerli; egui'ye özgü maddeler Slint karşılığıyla yorumlanır.

**Ekranlar**
1. **Canlı:** CPU (toplam + çekirdek ısı haritası), RAM/commit, disk, ağ grafikleri (son 5 dk).
2. **Geçmiş:** zaman aralığı seçici → `QueryRange`, otomatik çözünürlük (aralık > 6 saat → 1 dk).
3. **Süreçler:** Top-N tablosu + müdahale butonları.
4. **Olaylar:** alarm geçmişi.
5. **Ayarlar:** servis kontrolü, retention, alarm kuralları (Admin gerekli).

**Performans kuralları**
- `eframe` glow backend, `request_repaint_after(1s)`; sürekli repaint yok.
- Minimize → `Unsubscribe`; restore → yeniden abone.
- Grafik verisi ring buffer (`VecDeque`, sabit kapasite).

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
3. `ERROR_ACCESS_DENIED` → onay diyaloğu → `ShellExecuteExW(verb="runas", "lowmon-elev.exe --op kill --pid 1234 --ctime 133...")`.
4. `lowmon-elev` aynı kontrolü tekrar yapar, işlemi uygular, exit code ile sonucu döner, kapanır.
5. Her müdahale servise bildirilir → `Event` olarak loglanır (kim, ne, hangi süreç).

**Yasak liste:** `csrss`, `wininit`, `smss`, `lsass`, `services`, `winlogon`, `System`, PID 0/4 → kill butonu devre dışı.

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
| **M1** | CPU/RAM/Disk/Ağ/Top-N toplayıcı | 24 saatte CPU < %0.05, Private WS / Private Bytes < 3 MB |
| **M2** | Şifreli + zincirli log, seal, retention, `lwm-dump` | Kurcalama testlerinin tamamı yakalanıyor |
| **M3** | Pipe protokolü + tray | Abone yokken sıfır akış, tray yeniden bağlanıyor |
| **M4** | UI (Slint, ADR-0019), geçmiş sorgusu, müdahale + `lowmon-elev` | Yükseltilmiş süreç kill'i UAC ile çalışıyor |
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
| Firewall | `install` sırasında sadece **Private** profil, sadece yerel alt ağ için inbound kural (`INetFwPolicy2`) |
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
| `/v1/info` | GET | Host adı, çekirdek sayısı, toplam RAM, disk/arayüz listesi (token gerekli) |
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
