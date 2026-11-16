# ADR-0032: AI kullanım göstergesi LowMonitor'da: `lowmon-ai` + `lowmon-taskbar`
- Tarih: 2026-11-16
- Durum: Taslak (kullanıcı kararları alındı; uygulama fazlar halinde, her faz ayrı PR).

## Bağlam
`ai-usage-bar` (kullanıcının ayrı reposu, C++20 / Win32, public, lisans dosyası yok; tek sahip) Claude abonelik kullanımını (5 sa oturum + haftalık limit) taskbar'da, tepsinin solunda iki satır gösteriyor. Bugün ayrı süreç, ayrı autostart (`HKCU\…\Run\ClaudeUsageBar`), ayrı ayar (`%LOCALAPPDATA%\ClaudeUsageBar\config.ini`).

| Parça | Satır (yaklaşık) | Not |
|---|---|---|
| `core/` json_lite, time_util, api, credentials, display, process_kind | ~900 | Platformdan bağımsız, GoogleTest'li |
| `core/` usage_math (tempo, %100 tahmini, CSV) | ~370 | |
| `core/` usage_breakdown + report_html | ~1 400 | `.jsonl` dökümü → token dağılımı, HTML rapor |
| `win/` http_client (WinHTTP), usage_fetcher (OAuth → yenileme → sessionKey), secret_store (DPAPI), process_watch | ~670 | |
| `win/` renderer (UpdateLayeredWindow), taskbar (konum, tam ekran), app (pencere, timer, menü) | ~760 | |
| `win/` report_builder, config, log, util | ~580 | |

Ölçüm (çalışan exe, 2026-11-16): ~31 MiB WS (Görev Yöneticisi), CPU ölçülmedi.

## Karar (kullanıcı, 2026-11-16)
1. **Rust'a taşınır, tray içinde çalışır** (ayrı süreç yok; `lowmon-deck` gibi kütüphane). Servis değil: LocalService kullanıcının `~/.claude` kimliğine erişemez.
2. **İki yeni crate:**
   - `lowmon-ai` (lib): kullanım verisi. Kimlik (Claude Code OAuth `.credentials.json`, süresi dolunca yenileme; yedek sessionKey DPAPI), WinHTTP istemcisi, `/api/oauth/usage` ayrıştırma, tempo/seviye hesabı, Claude süreç tespiti, limit CSV'si. Çıktı: `Snapshot { session, week, source, error, stale }` + olay (değişince).
   - `lowmon-taskbar` (lib): **genel** taskbar metin göstergesi (layered pencere, per-pixel alpha, DPI, tepsi solu konumu, tam ekranda gizlenme, tema). Veri bilmez; `set_lines([Line; N])` alır. İleride trafik monitörü gibi başka eklentiler aynı göstergeye satır ekler.
3. **Telefon:** `lowmon-ai` verisi LowDeck telefonda görünür. `lowdeck-proto`'ya sona yeni `S2C` varyantı (LowDeck repo, sonra LM'de rev güncellemesi); deck yalnız değişince push eder (spek §1: "kimse dinlemiyorsa üretilmez").
4. **Rapor ve token dökümü** (`usage_breakdown`, `report_html`, `report_builder`) **sonraki faz**; ilk fazlarda taşınmaz.
5. Eski `ai-usage-bar` geçişte durdurulur: autostart kaydı silinir, ayar ve `session.bin` okunup taşınır (aynı kullanıcı → DPAPI çözülür), dosyalar silinmez (geri dönüş).

## Faz 1 kararları (kullanıcı, 2026-11-16)
| Konu | Karar |
|---|---|
| Sorgu ne zaman | Claude Desktop, Claude Code CLI ya da VS Code (`Code.exe`) çalışırken; hepsi kapalıyken istek yok, veri "eski" |
| Veri yeri | `%LOCALAPPDATA%\LowMonitor\ai\` (CSV, `session.bin` — aynı DPAPI entropisi, eski blob aynen çözülür); ayar `%APPDATA%\LowMonitor\ai.toml`. Eski `%LOCALAPPDATA%\ClaudeUsageBar\` geçişten sonra **silinir** (Faz 4; widget durdurulduktan sonra) |
| Dil | Seçilebilir `language = "tr" \| "en"` (varsayılan tr) |
| JSON | `json_lite` portu (span tabanlı): kimlik dosyasında 3 alan bayt-korumalı değişir; serde_json bunu vermez |
| Taskbar çizimi | C++'taki yöntemler (kullanıcı beğenmedi) Faz 2'de yeniden tasarlanır; o fazda kartla sorulur |

## Fazlar (her biri ayrı PR, ölçümle)
| Faz | Repo | İş | Doğrulama |
|---|---|---|---|
| 1 | LM | `lowmon-ai` crate: core mantığı + WinHTTP + OAuth/yenileme + sessionKey + süreç tespiti + CSV; C++ testleri Rust'a | `cargo test`; örnek exe ile canlı `/usage` (C++ widget ile aynı sayılar) |
| 2 | LM | `lowmon-taskbar` crate + tray entegrasyonu: "AI ▸" menüsü (durum, şimdi yenile, ayar, sessionKey kaydet/sil), `ai.toml` + `config.ini` göçü, tooltip'e kısa satır | tray exe boyutu, private, boşta CPU önce/sonra; C++ widget ile görsel karşılaştırma |
| 3 | LowDeck + LM | proto varyantı + telefon kartı (LowDeck PR), deck push + `lowmon-ui` kartı (LM PR) | telefonda gerçek veri |
| 4 | LM | `install.ps1`: eski widget süreci + `ClaudeUsageBar` Run kaydı temizliği, `%LOCALAPPDATA%\ClaudeUsageBar` CSV/`session.bin` kopyası → klasör silinir; spek, README | geçiş senaryosu |
| 5 | LM | rapor / token dökümü (`lowmon-ui` sekmesi ya da HTML; o fazda karar) | |

## Sonuçlar
- (+) Tek süreç, tek kurulum, tek dil/toolchain; veri telefona ve ui'a gider.
- (+) `lowmon-taskbar` genel: sonraki göstergeler (ağ trafiği vb.) aynı pencereyi kullanır.
- (−) Tray internete çıkar (yalnız `api.anthropic.com`, Claude çalışırken; sessionKey yolunda `claude.ai`). Tray exe büyür (WinHTTP/DPAPI sistem DLL'i; tahmin +150–250 KB, Faz 2'de ölçülür).
- (−) ToS gri alanı (Claude Code OAuth client'ıyla yenileme) LM'ye girer. Koruma aynen taşınır: Claude Code CLI çalışırken yenileme yapılmaz; token/başlık log'a yazılmaz; tamponlar sıfırlanır.
- (−) Usage endpoint'leri belgelenmemiş; değişirse ayrıştırıcı güncellenir.
- Spek §1 bütçesine tray satırı Faz 2 ölçümüyle güncellenir; §2 bileşen tablosuna iki crate eklenir (Faz 1/2 PR'larında).

## Açık (fazında kartla sorulur)
- Taskbar çizim yöntemi (Faz 2).
- Proto varyantının alanları (oturum/hafta %, reset, seviye, kaynak, hata).
