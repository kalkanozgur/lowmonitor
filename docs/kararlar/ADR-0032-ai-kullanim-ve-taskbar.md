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
| Taskbar çizimi | C++'taki yöntemler (kullanıcı beğenmedi) Faz 2'de yeniden tasarlanır; o fazda kartla sorulur → aşağıda |

## Faz 2 kararları (kullanıcı, 2026-11-16)
| Konu | C++ | Karar |
|---|---|---|
| Yerleşim | Taskbar üstünde yüzen `WS_EX_TOPMOST` pencere, her 1 s `SetWindowPos(TOPMOST)` | `Shell_TrayWnd`'nin **alt penceresi** (`WS_CHILD`, katmanlı): taskbar'la hareket eder, tam ekranda onunla gizlenir; z-sırası kavgası yok |
| Takip | 1 s zamanlayıcıyla konum/DPI/tam ekran yoklaması | `EVENT_OBJECT_LOCATIONCHANGE` (taskbar + `TrayNotifyWnd`, out-of-context) → yeniden konum; `TaskbarCreated` → yeniden kur. Metin için 30 s zamanlayıcı (kalan süre) |
| Metin | GDI beyaz çizim, kanalların max'ı alfa, boş piksel alfa=1 | GDI+ `AntiAliasGridFit` → PARGB bitmap (gerçek alfa); yalnız arka plan alfa=1 (tıklanabilir) |
| DPI | süreç DPI-habersiz | yalnız gösterge işlemleri per-monitor-v2 thread bağlamında; tray menüsü/ikonu değişmez |
| Ayar | `config.ini` | `taskbar.toml` (monitor `right`/`left`/`primary`, `gap_px`, `reserved_px`, `font`, `font_pt`); yoksa C++ varsayılanları |

Gereken: exe manifesti (supportedOS Win10) — katmanlı alt pencere Win8+ uyumluluğu ister; `lowmon-tray/build.rs` gömer.

## Faz 3 kararları (kullanıcı, 2026-11-16)
| Konu | Karar |
|---|---|
| Proto | `TOPIC_AI = 4` + `S2C::Ai(Option<AiUsage>)` sona eklendi (LowDeck); yalnız abone olana gider → `PROTO_VERSION` 4 kalır |
| Veri | Ham değer + seviye: `pct` (0,1 %), `resets_at` (Unix s), `level` (tray'in tempo/eşik ayarıyla 0/1/2), `stale`, `source`, `error`. Metin ve geri sayımı istemci kendi saatiyle üretir |
| Telefon | PC bölümünün üstünde ayrı "AI" bölümü; geri sayım her metrik push'unda yenilenir (ek timer yok) |
| `lowmon-ui` | Canlı sekmesinin üstünde AI bandı (veri yoksa gizli); küçültülünce `TOPIC_AI` düşer |
| Deck | Son durum önbellekte; yeni aboneye ilk çerçeve, sonra yalnız değişiklik. Metrik pipe'ını açtırmaz |

## Faz 4 kararları (kullanıcı, 2026-11-16)
| Konu | Karar |
|---|---|
| `config.ini` | **Taşınmaz**: kullanıcının dosyasındaki her değer varsayılan; eski klasörle birlikte silinir |
| sessionKey | "AI kullanımı ▸" menüsünde "Panodan sessionKey kaydet" / "sessionKey'i sil". Pano `sk-ant-…` ya da `sessionKey=sk-ant-…` ise DPAPI ile `session.bin`'e yazılır, pano boşaltılır; değilse pano dokunulmaz. Anahtar loga/ekrana çıkmaz |
| Geçiş (`install.ps1`) | `claude-usage-bar` süreci durur, HKCU Run `ClaudeUsageBar` silinir; `usage-*.csv` hedefle birleşir (ts tekil + sıralı, CRLF), `session.bin` hedefte yoksa kopyalanır, `%LOCALAPPDATA%\ClaudeUsageBar` silinir. Eski exe klasörüne dokunulmaz |

## Faz 5 kararları (kullanıcı, 2026-11-17)
| Konu | Karar |
|---|---|
| Gösterim | `lowmon-ui` 5. sekme "AI": limit çizgisi (oturum + hafta %), token dökümü (grup çubukları), oturum tablosu; görünüm Son oturum / Bugün / Bu hafta. HTML üretici taşınmaz |
| Veri | `lowmon_ai::breakdown` (analizör, C++ ile aynı formül ve testler) + `lowmon_ai::report` (döküm listesi, `breakdown-cache.jsonl`, CSV). Arayüz dosyaları kendisi okur (arka iş parçacığı, sekme açılınca / Yenile); deck ve proto değişmez |
| Sol tık | Taskbar göstergesine sol tık raporu açar (`lowmon-ui --ai`; çalışan kopya isimli olayla AI sekmesine geçer). "Şimdi yenile" menüde kalır |
| Bulut özetleri | `*_usage.json` rapora girer; klasör `ai.toml` `summary_dir`, boşsa Belgeler\Claude-Logs. Proje adına "(bulut)" eklenir |
| Taşınmayan | Tahmin kartları (%100'e varış, oturum→hafta oranı), grafik eksen etiketleri / imleç, CSV budama (`csv_keep_months`), İngilizce grup adları (arayüz Türkçe) |

## Dil kapsamı (kullanıcı, 2026-11-17)
| Konu | Karar |
|---|---|
| Kapsam | en/tr seçimi **her şeyi** kapsar: taskbar metni, tray menüsü / tooltip / bildirim / ileti kutuları, `lowmon-ui` tüm sekmeler |
| Ayar | Tek yer: `%APPDATA%\LowMonitor\ai.toml` `language = "en"` / `"tr"` (varsayılan `tr`). Tray açılışta ve "Ayarı yeniden yükle"de, `lowmon-ui` açılışta okur |
| Yöntem | `lowmon_ai::text::set_lang` + metin yerinde `tr("Türkçe", "English")`; Slint'te `Lang.t("Türkçe", "English")`. Çeviri dosyası / çalışma anı kitaplığı yok |
| Sayılar | İngilizcede ondalık nokta ve `47.7%`; Türkçede virgül ve `%47,7`. Token dökümü grup adları İngilizcede çevrilir (kimlikler veride Türkçe kalır) |
| Çevrilmeyen | Log dosyaları (`deck.log`), `lowmon-deck` ve servis hata metinleri (menüde "Off: …" ardından Türkçe gelebilir), telefon uygulaması (LowDeck, ayrı repo) |

## Fazlar (her biri ayrı PR, ölçümle)
| Faz | Repo | İş | Doğrulama |
|---|---|---|---|
| 1 | LM | `lowmon-ai` crate: core mantığı + WinHTTP + OAuth/yenileme + sessionKey + süreç tespiti + CSV; C++ testleri Rust'a | `cargo test`; örnek exe ile canlı `/usage` (C++ widget ile aynı sayılar) |
| 2 | LM | `lowmon-taskbar` crate + tray entegrasyonu: "AI ▸" menüsü (durum, şimdi yenile, ayar), tooltip'e kısa satır. sessionKey kaydet/sil ve `config.ini` göçü Faz 4'e | tray exe boyutu, private, boşta CPU önce/sonra; C++ widget ile görsel karşılaştırma |
| 3 | LowDeck + LM | proto varyantı + telefon kartı (LowDeck PR), deck push + `lowmon-ui` kartı (LM PR) | telefonda gerçek veri |
| 4 | LM | sessionKey kaydet/sil, `config.ini` → `ai.toml`/`taskbar.toml` göçü; `install.ps1`: eski widget süreci + `ClaudeUsageBar` Run kaydı temizliği, `%LOCALAPPDATA%\ClaudeUsageBar` CSV/`session.bin` kopyası → klasör silinir; spek, README | geçiş senaryosu |
| 5 | LM | rapor / token dökümü: `lowmon-ui` AI sekmesi (Faz 5 kararları) | `cargo test` (C++ testleri), canlı sekme, ölçüm `2026-11-17_lowmon-ui-ai-raporu` |

## Sonuçlar
- (+) Tek süreç, tek kurulum, tek dil/toolchain; veri telefona ve ui'a gider.
- (+) `lowmon-taskbar` genel: sonraki göstergeler (ağ trafiği vb.) aynı pencereyi kullanır.
- (−) Tray internete çıkar (yalnız `api.anthropic.com`, Claude çalışırken; sessionKey yolunda `claude.ai`). Tray exe büyür (WinHTTP/DPAPI sistem DLL'i; tahmin +150–250 KB, Faz 2'de ölçülür).
- (−) ToS gri alanı (Claude Code OAuth client'ıyla yenileme) LM'ye girer. Koruma aynen taşınır: Claude Code CLI çalışırken yenileme yapılmaz; token/başlık log'a yazılmaz; tamponlar sıfırlanır.
- (−) Usage endpoint'leri belgelenmemiş; değişirse ayrıştırıcı güncellenir.
- Spek §1 bütçesine tray satırı Faz 2 ölçümüyle güncellenir; §2 bileşen tablosuna iki crate eklenir (Faz 1/2 PR'larında).

## Açık (fazında kartla sorulur)
