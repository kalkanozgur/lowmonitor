# LowMonitor
Windows için çok düşük tüketimli kaynak izleme, loglama ve müdahale aracı (Rust, `x86_64-pc-windows-msvc`); masaüstü arayüzü, taskbar göstergeleri, Claude kullanım takibi ve telefon eşlikçisi ([LowDeck](https://github.com/kalkanozgur/lowdeck)) ile.
Spek: [docs/spesifikasyon.md](docs/spesifikasyon.md) (tek doğruluk kaynağı)

English: [README.md](README.md)

| Alan | Değer |
|---|---|
| Durum | Prototip: M0/M3 tamam, M1–M6 kodda büyük ölçüde tamam; nihai kabul soak'u bekliyor |
| Modüller | yazılım (Rust workspace) |
| Başlangıç | 2026-11-14 |
| Bütçe | CPU < %0,05 toplam, Private WS / Bytes < 3 MB (24 saat) |

## Bileşenler
| Süreç / crate | Görev |
|---|---|
| `lowmon-svc` | Windows servisi (SYSTEM): CPU / RAM / disk / ağ / Top-N toplayıcı, şifreli + zincirli log, saklama, alarmlar, `\\.\pipe\LowMonitor` |
| `lowmon-tray` | Kullanıcı oturumu: tray ikonu, taskbar göstergeleri, Claude kullanımı, telefon sunucusu (`lowmon-deck`), sensör iş parçacığı |
| `lowmon-ui` | Masaüstü arayüzü (Slint): Canlı, Olaylar, Geçmiş, Süreçler, AI, Ayarlar; veri tray'den loopback ile |
| `lowmon-net` | İsteğe bağlı LAN HTTP / WebSocket API (`/v1/info`, canlı JSON), token ile |
| `lowmon-deck` | Tray içindeki telefon / arayüz sunucusu: TCP 47810 + UDP 47811 keşif, ses / medya / kilit, süreçler, geçmiş, AI raporu |
| `lowmon-taskbar` | Taskbar'a gömülü iki bloklu gösterge (sistem ızgarası + AI), monitör başına |
| `lowmon-ai` | Claude abonelik kullanımı (5 sa oturum + hafta), OAuth / sessionKey, CSV geçmişi, Claude Code etkinliği |
| `lowmon-sensors` | GPU % / °C (D3DKMT), disk °C, CPU °C (ACPI termal bölge) |
| `lowmon-core`, `lowmon-proto` | Kayıt biçimi, kripto, ayar; pipe protokolü ve metrik tipleri |
| `tools/lwm-dump`, `tools/lowmon-hook` | Log dışa aktarma / inceleme; Claude Code hook komutu |

## Özellikler
- Canlı ve geçmiş grafikler (CPU, RAM, disk, ağ, GPU, sıcaklıklar), Top-N süreçler, süreç sonlandırma
- Alarm kuralları → olaylar, tray balonları, telefonda alarm bandı; opsiyonel UAC / güvenlik duvarı istemi alarmları (`[events] uac = true`, ETW, yalnız bildirim); `taskbar.toml` eşiklerinden tray sıcaklık alarmları (`Temp.CPU/GPU/Disk`)
- Taskbar: sistem ızgarası (CPU / GPU / MEM / ↑, °C / ↓) ve Claude kullanım göstergesi; ikisi de tray menüsünden açılıp kapanır
- AI göstergesinin üstünde Claude Code etkinlik balonu (hook'lar tray menüsünden kurulur)
- Claude Code etkinlik balonu: proje, güvenli özet (Bash açıklaması, dosya adı), Claude girdi bekleyince `Claude.Input` alarmı
- Telefon (LowDeck): keşif, PC IP'si değişince otomatik yeniden bağlanma, alarmda açılan siyah perde
- Tray IP değişimini (DHCP) izler, telefon dinleyicisini kendiliğinden yeni IP'ye bağlar
- İngilizce / Türkçe arayüz (`ai.toml` `language`)

## Klasör yapısı
```
lowmonitor/
├─ Cargo.toml              # workspace, ortak bağımlılık sürümleri
├─ crates/                 # lowmon-core, -proto, -svc, -tray, -ui, -net, -deck, -taskbar, -ai, -sensors
├─ tools/                  # lwm-dump, lowmon-hook
├─ scripts/                # install.ps1, soak.ps1
├─ .githooks/pre-push      # lokal kalite kapısı
├─ .github/workflows/      # yalnızca v* tag → release
└─ docs/
   ├─ spesifikasyon.md     # tek doğruluk kaynağı
   ├─ gereksinimler.md
   ├─ devlog/YYYY-MM.md
   ├─ kararlar/ADR-*.md    # kararlar (son: ADR-0035)
   ├─ olcumler/            # performans ölçümleri
   └─ referans/
```

## Hızlı başlangıç
Derleme (repo kökünden):
```powershell
cargo build --release --workspace
cargo build --profile release-ui -p lowmon-ui
```
Kurulum / güncelleme **yönetici PowerShell**'inden (`C:\Program Files\LowMonitor`'a kopyalar; servis, firewall kuralları, tray autostart; `%ProgramData%\LowMonitor` verisi korunur):
```powershell
pwsh -File scripts\install.ps1 -Src target\release -UiSrc target\release-ui
```
Kurulum: GitHub Releases'tan `LowMonitor-Setup-<sürüm>.exe` (bileşenler seçilebilir; tray + taskbar zorunlu). Geliştirici yolu: `scripts\install.ps1`.

Kullanıcı ayarları: `%APPDATA%\LowMonitor\` (`deck.toml`, `taskbar.toml`, `ai.toml`, `net.toml`); servis ayarları: `%ProgramData%\LowMonitor\config.toml`.

## Geliştirme akışı
| Aşama | Nerede | Ne |
|---|---|---|
| Build + test | Lokal | `cargo build`, `cargo test --workspace` |
| Push öncesi | Lokal hook | `fmt --check` + `clippy -D warnings` + `test` |
| Değişiklik | GitHub | dal → PR → squash merge (ADR-0021); main'e doğrudan push yok |
| Release | GitHub Actions | `git tag vX.Y.Z && git push --tags` → release build + exe'ler GitHub Release'e |

Klon sonrası hook'u bir kez etkinleştir:
```powershell
git config core.hooksPath .githooks
```

## Yapay zeka ajanı devri
Son durum ve sıradaki adımlar `HANDOFF.md`'de; kurallar `CLAUDE.md`'de.

## Lisans
MIT ya da Apache-2.0, seçim sizin ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

`lowmon-ui` [Slint](https://slint.dev) ile yapıldı ([Royalty-free 2.0](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md) lisansı; atıf şartı):

<a href="https://slint.dev"><img alt="Made with Slint" src="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg" height="60"></a>
