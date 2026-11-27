# HANDOFF — LowMonitor
**Güncelleme:** 2026-11-27 18:15 · **Güncelleyen:** Claude Code · **Kaynak durumu:** v1.4.0 sürüm PR'ı (#89, #90, #92, #93 telefondan güç komutları)

## Durum
v1.3.0 yayında (tag, GitHub Release; v1.2.2 yayından kaldırıldı, v1.2.3 hiç çıkmadı). main = `aafd2da` (#90). **Kurulu build = yerel setup `1.3.0-local.aafd2da`** (release profili, 2026-11-24 17:34; v1.3.0 + #89 + #90). Rust workspace: svc, tray, ui, net, deck, taskbar, ai, sensors, core, proto + lwm-dump, lowmon-hook; Inno Setup kurucusu (ADR-0038).

**24 sa kabul soak'u sürüyor** (yeni build): Task Scheduler "LowMonitor soak" (SYSTEM, `-AtStartup` + elle başlatıldı), `scripts/soak.ps1 -Until 2026-11-25T17:35:08` (svc + tray + ui, `pid` sütunu, reboot sonrası aynı CSV'ye ekler). CSV ve script **`lowmonitor-memory-compression-pid4164-d2284c` worktree'sinde, dal `test/soak-24h`** — soak bitene kadar o worktree'nin dalını değiştirme (görev diskteki `soak.ps1`'i okur).

## Son yapılanlar (2026-11-27)
- Telefondan uyku / kapat / yeniden başlat / iptal (#93, ADR-0044; LowDeck #46/#47, ADR-0012). `lowmon-deck/src/power.rs`, `deck.toml` `power` (yoksa açık). M21 ile gerçek PC'de Sleep, Restart, Cancel doğrulandı; Shutdown denenmedi
- Test için kurulu tray kapatılıp `E:\Documents\Projects\lowmonitor-power` worktree'sinin release tray'i elle açıldı (detached `1d32dcf`). v1.4.0 kurulunca worktree silinebilir (`git worktree remove ..\lowmonitor-power`); E: diskinde ~0,8 GB boş kalmıştı
- v1.4.0 sürüm PR'ı; tag'i kullanıcı atar

## Önceki (2026-11-23/24)
- v1.3.0 (#86): kurulumda gizlilik tercihleri (ADR-0041), gruplu tray menüsü, GitHub'dan güncelleme (ADR-0042), Tanılama menüsü (ADR-0043), Win11 widget UIA worker (explorer kilitlenmesi düzeltmesi, ADR-0040)
- AI balonu 10 sn değişmezse ~240 ms'de solar, "bekliyor" solmaz (#89, ADR-0034 eki)
- Tray ikonu uygulama paletinde: tema rengi, ≥%75 amber, ≥%90 kırmızı (#90). Fade + ikon canlı denendi, kullanıcı onayladı (koyu tema)
- Memory Compression şişmesi incelendi: LowMonitor kaynaklı değil (3 süreç ~25 MB özel bellek, sabit); RAM baskısı (commit 18,3 GB / 14,2 GB fiziksel)

## Açık sorun / blokaj
- Soak verisi: v1.3.0 kısmi koşu `docs/olcumler/2026-11-24_v1.3.0-soak.csv` (16:10–17:31; 16:43:59–17:16:13 tray satırları geçici `target
elease` exe'si). Yeni koşu `2026-11-24_v1.3.0-local-soak.csv`. İkisi de takip edilmeyen dosya (worktree'de)
- Reboot'lu soak parçalıdır: sızıntı analizi `pid` değişimine göre parça parça; kabul raporunda en uzun kesintisiz aralık ayrıca verilir. Açılışta svc Delayed Start → ilk ~2 dk svc `NA`
- Açık temada tray ikonu ve balon solması canlı denenmedi
- GitHub'dan güncelleme yolunun (indir + SHA-256 + setup) gerçek release ile denenmesi bekliyor
- Tanılama maskelemesi fazla geniş ("OAuth token yenilendi" satırları da çıkıyor)
- Ana checkout'ta yerel `main` eski (`7afa6a6`); orada `git pull` gerekir

## Sıradaki adımlar
1. Soak bitince (2026-11-25 17:35 sonrası): CSV analizi → `docs/olcumler/2026-11-25_v1.3.0-local-soak.md` (düzenek, araç, süre, parçalar, bütçe: CPU < %0,05, Private WS < 3 MB) + iki CSV → `test/soak-24h` dalıyla PR; Task Scheduler görevini sil (yönetici)
2. Sürüm: #89 + #90 için v1.3.1 (ya da v1.4.0) — CHANGELOG `[Unreleased]` → sürüm, tag (kullanıcı kararı/onayı)
3. Açık tema canlı kontrolü (ikon + balon); ardından TODO "Şimdi" listesinden sıradaki (LowDeck Claude etkinlik listesi / HA bildirimi)

## Not (kullanıcı)
- UAC / güvenlik duvarı penceresi açabilecek her adımdan önce chatte haber ver (deck testleri LAN'a bağlanır → firewall istemi). Bu pencerelere asla kendin tıklama.

## Dokunma
- `docs/spesifikasyon.md` — tek doğruluk kaynağı; spekten sapma önce ADR
- `target/`, `graphify-out/`, `lowmonitor_full.txt` — üretilmiş/büyük dosyalar, okuma dışı
- `.claude/worktrees/*` — başka oturumların çalışma alanı, bu oturumdan değiştirme (özellikle soak süren `lowmonitor-memory-compression-pid4164-d2284c`)
- main'e doğrudan push yok (pre-push hook); kurulu `Program Files` exe'leri elle değiştirme

## Kritik dosyalar
- `CLAUDE.md` — kurallar · `TODO.md` — milestone durumu · `docs/spesifikasyon.md` — spek
- `docs/devlog/2026-11.md` — gelişim günlüğü · `docs/kararlar/ADR-*.md` — kararlar (son: ADR-0043)
- `crates/lowmon-proto/src/proto.rs` — pipe protokolü (`PROTO` sürümü) · `lowdeck-proto` (LowDeck repo, git `rev`) — telefon protokolü
- `crates/lowmon-tray/src/main.rs` (menü, IP izleme), `crates/lowmon-tray/src/sys.rs` (sensörler, sıcaklık alarmı), `crates/lowmon-deck/src/lib.rs` (hub, dinleyiciler, keşif)

## Doğrulama
- `cargo fmt --check` · `cargo clippy --workspace -- -D warnings` · `cargo test --workspace`
- Kurulum: `cargo build --release --workspace` + `cargo build --profile release-ui -p lowmon-ui` → `& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" /DMyVersion=<sürüm> installer\lowmonitor.iss` → `target\setup\LowMonitor-Setup-<sürüm>.exe`'yi `explorer.exe` ile başlat (Claude araç sürecinden değil; UAC kullanıcıda)

## Kısıtlar / kararlar
- Yasak: tokio/async runtime, PDH, WMI, .NET, WebView (spek §1; istisna ADR-0033 ACPI PDH)
- Rust 1.100.0-msvc, `crt-static`; her `unsafe` üstünde `// SAFETY:`
- Bütçe: CPU < %0,05 toplam, Private WS/Bytes < 3 MB (24 saat)
- Bilinen borçlar (TODO.md "Kısayollar"): firewall portu sabit 47800, `tokens.bin`/`state.bin` atomik değil, ≤ 64 çekirdek

---
## Geri dönüş (Claude Code'a döndüğünde)
> HANDOFF.md'yi oku. Aşağıdaki çıktı <Gemini/OpenCode>'den geldi. Uygulamadan önce mevcut kodla çelişkileri kontrol et, sonra uygula, devlog'a ekle, HANDOFF.md'yi güncelle.
