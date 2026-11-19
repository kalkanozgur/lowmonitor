# HANDOFF — LowMonitor
**Güncelleme:** 2026-11-19 · **Güncelleyen:** Claude Code · **Kaynak durumu:** v1.0.0 (main, tag sonrası)

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
- İki telefon açıkken tek LAN yuvasında birbirini düşürür (M21 + Mi Note, 2026-11-19); geçici çözüm: birinde LowDeck'i kapat

## Sıradaki adımlar (sırayla)
1. Sonraki sürüm (kullanıcı, 2026-11-19): **çoklu telefon** (`lowmon-deck` LAN yuvası, ADR) ve **tek exe kurulum** (modüller açıklamalı, varsayılan seçili; tray + taskbar zorunlu; ADR) — ayrıntı `TODO.md`
2. Nihai kabul soak'u: yönetici pwsh ile `scripts/soak.ps1` (CPU < %0,05 toplam, ADR-0026; Private < 3 MB; sızıntı yok) → `docs/olcumler/`
3. Canlı doğrulama turu: taskbar menüsü, IP değişimi (DHCP yenile), sıcaklık alarmı (eşiği geçici düşürerek), telefon perdesi
4. Rafta karar: `lowmon-ui` veriyi doğrudan servisten alsın (yeni ADR)

## Dokunma
- `docs/spesifikasyon.md` — tek doğruluk kaynağı; spekten sapma önce ADR
- `target/`, `graphify-out/`, `lowmonitor_full.txt` — üretilmiş/büyük dosyalar, okuma dışı
- `.claude/worktrees/*` — başka oturumların çalışma alanı, bu oturumdan değiştirme
- main'e doğrudan push yok (pre-push hook); kurulu `Program Files` exe'leri elle değiştirme

## Kritik dosyalar
- `CLAUDE.md` — kurallar · `TODO.md` — milestone durumu · `docs/spesifikasyon.md` — spek
- `docs/devlog/2026-11.md` — gelişim günlüğü · `docs/kararlar/ADR-*.md` — kararlar (son: ADR-0035)
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
