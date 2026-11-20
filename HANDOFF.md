# HANDOFF — LowMonitor
**Güncelleme:** 2026-11-20 (akşam) · **Güncelleyen:** Claude Code · **Kaynak durumu:** v1.0.0 + Unreleased (çoklu telefon, UAC bildirimi)

## Durum
v1.0.0 yayında (LowDeck v1.0.0 ile birlikte). Rust workspace: svc, tray, ui, net, deck, taskbar, ai, sensors, core, proto + lwm-dump, lowmon-hook. Kod M0–M6 kapsamında; **nihai 24 saat kabul soak'u henüz yok**. Kurulu build = main `7bfd931` (2026-11-19 15:40; sürüm numarası öncesi, içerik v1.0.0 ile aynı).

## Son yapılanlar (2026-11-19, PR #67–#69)
- Tray IP değişimini izler, telefon dinleyicisi yeni IP'ye geçer (#67)
- Tray menüsünden taskbar sistem / AI göstergesi aç-kapa (#68)
- Tray sıcaklık eşiği → deck alarm olayı (`Temp.CPU|GPU|Disk`); LowDeck perdesi alarmda açılır, meşgulken 10 dk'da bir 15 s, kopunca otomatik keşif (LowDeck #40) (#69)
- README EN + TR, sürüm 1.0.0

## Açık sorun / blokaj
- Açılışta ~2 dk servis yok: servis `Auto (Delayed Start)`; UI/deck `os error 2` loglar, servis gelince kendiliğinden düzelir (kullanıcı sorun saymadı)
- Canlı doğrulanmadı: sıcaklık alarmı (eşik aşılmadı), telefon perdesinin alarmla / arada açılması, tray çalışırken IP değişimi, taskbar aç-kapa menüsü
- CPU: servis 1,45–2,26 ms/s, toplayıcılar ~0,5 ms/s → ~1 ms/s fark açık
- Sıcaklık olayları servis olay deposuna yazılmaz (Geçmiş'te yok)
- UAC + güvenlik duvarı istemi bildirimi canlı doğrulandı (ADR-0037 + eki, 2026-11-20); kullanıcı makinesinde açık. Açıkken CPU ölçülmedi
- Çoklu telefon canlı doğrulandı (ADR-0036, 2026-11-20); kurulu build = main `bc4b0ce`

## Sıradaki adımlar (sırayla; kullanıcı onaylı plan, 2026-11-20)
1. **Inno Setup kurucusu (ADR-0038 yaz):** `installer/lowmonitor.iss` — bileşenler açıklamalı + varsayılan seçili: tray+taskbar `fixed` (zorunlu, gri), servis (`lowmon-svc install/start`, kaldırmada `uninstall`, `%ProgramData%` korunur), `lowmon-ui` (release-ui), `lowmon-net` (+firewall), `lowmon-hook`, `lwm-dump`, telefon firewall (TCP deck portu + UDP 47811, private+localsubnet). Önce süreçleri kapat + eski LowDeck/ai-usage-bar izleri (`scripts/install.ps1` 19–48). Tray autostart `[Run]` `runasoriginaluser` (HKCU doğru kullanıcı). Kaldırma: süreçler, servis, firewall, autostart. Lokal Inno kurulumu indirme ister → **önce kullanıcı onayı + UAC haberi**; CI: `windows-latest`'te `iscc`, `release.yml`'e `LowMonitor-Setup-<sürüm>.exe`. `install.ps1` geliştirici yolu kalır.
2. **v1.1.0:** workspace sürümü, CHANGELOG `[1.1.0]`, README EN+TR (setup exe), PR → merge → `v1.1.0` tag (önce haber) → release.
3. **Temiz kurulum:** kullanıcı çalıştırır (UAC; Claude oturumundan çalışınca HKCU/AppData sanallaşır): eski kurulumu kaldır → setup exe. Sonra doğrula: servis, tray, deck, balon 2–3 satır, `Claude.Input` telefonda, UAC/firewall alarmları (`[events] uac` config'te kalır).
4. Ayrı oturum: LowDeck telefonda Claude etkinlik listesi (TODO). Sonra HA bildirimi, soak.

## Not (kullanıcı)
- UAC / güvenlik duvarı penceresi açabilecek her adımdan önce chatte haber ver (deck testleri LAN'a bağlanır → firewall istemi). Bu pencerelere asla kendin tıklama.

## Dokunma
- `docs/spesifikasyon.md` — tek doğruluk kaynağı; spekten sapma önce ADR
- `target/`, `graphify-out/`, `lowmonitor_full.txt` — üretilmiş/büyük dosyalar, okuma dışı
- `.claude/worktrees/*` — başka oturumların çalışma alanı, bu oturumdan değiştirme
- main'e doğrudan push yok (pre-push hook); kurulu `Program Files` exe'leri elle değiştirme

## Kritik dosyalar
- `CLAUDE.md` — kurallar · `TODO.md` — milestone durumu · `docs/spesifikasyon.md` — spek
- `docs/devlog/2026-11.md` — gelişim günlüğü · `docs/kararlar/ADR-*.md` — kararlar (son: ADR-0037)
- `crates/lowmon-proto/src/proto.rs` — pipe protokolü (`PROTO` sürümü) · `lowdeck-proto` (LowDeck repo, git `rev`) — telefon protokolü
- `crates/lowmon-tray/src/main.rs` (menü, IP izleme), `crates/lowmon-tray/src/sys.rs` (sensörler, sıcaklık alarmı), `crates/lowmon-deck/src/lib.rs` (hub, dinleyiciler, keşif)

## Doğrulama
- `cargo fmt --check` · `cargo clippy --workspace -- -D warnings` · `cargo test --workspace`
- Kurulum: `cargo build --release --workspace` + `cargo build --profile release-ui -p lowmon-ui` → yönetici pwsh `scripts\install.ps1 -Src target\release -UiSrc target\release-ui`

## Kısıtlar / kararlar
- Yasak: tokio/async runtime, PDH, WMI, .NET, WebView (spek §1; istisna ADR-0033 ACPI PDH)
- Rust 1.100.0-msvc, `crt-static`; her `unsafe` üstünde `// SAFETY:`
- Bütçe: CPU < %0,05 toplam, Private WS/Bytes < 3 MB (24 saat)
- Bilinen borçlar (TODO.md "Kısayollar"): firewall portu sabit 47800, `tokens.bin`/`state.bin` atomik değil, ≤ 64 çekirdek

---
## Geri dönüş (Claude Code'a döndüğünde)
> HANDOFF.md'yi oku. Aşağıdaki çıktı <Gemini/OpenCode>'den geldi. Uygulamadan önce mevcut kodla çelişkileri kontrol et, sonra uygula, devlog'a ekle, HANDOFF.md'yi güncelle.
