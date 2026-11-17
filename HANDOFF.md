# HANDOFF — LowMonitor
**Güncelleme:** 2026-11-17 · **Güncelleyen:** Claude Code (devam oturumu; `git log`, TODO.md, devlog 2026-11-17 sonuna göre) · **Kaynak durumu:** main `d05144d`

## Durum
Rust workspace: svc, tray, net, core, proto, lwm-dump + `lowmon-ui` (Slint PC arayüzü, artık bu repoda), `lowmon-ai`, `lowmon-taskbar`, `lowmon-sensors`, `lowmon-deck`, `lowmon-hook`. M0/M3 tamam, M1/M2/M4/M5/M6 kodda büyük ölçüde tamam, **nihai kabul soak'u henüz yok**. Kurulu build = main `d05144d` (2026-11-17 15:37).

## Son yapılanlar (2026-11-16/17, PR #50–#57)
- ADR-0032 AI kullanımı: `lowmon-ai`, taskbar AI bloğu, telefon/ui, rapor sekmesi; eski C++ widget kaldırıldı
- ADR-0033 Faz 1–3: `lowmon-sensors` (GPU, disk °C, ACPI termal), taskbar 2×4 sistem ızgarası, ikinci monitör
- ADR-0034 Parça 0–4: Claude Code etkinlik balonu, `lowmon-hook.exe`, tray menüsünden hook kur / kaldır
- Dil: `ai.toml` `language` (tray + `lowmon-ui` + taskbar)
- `lowmon-ui`: grafiklerde fare ile zaman damgası (#56), Ayarlar sekmesi ilk parti — `ai/taskbar/deck.toml`, `toml_edit`, kayıtta tray'e `WM_APP+6` (#57)

## Açık sorun / blokaj
- CPU: toplayıcılar ~0,5 ms/s, servis 1,45–2,26 ms/s → ~1 ms/s fark açık
- Canlı doğrulanmadı: Ayarlar'dan font / port / dil değişikliğinin tray'e yansıması, Notification matcher (izin istemi), monitör tak/çıkar, gerçek telefon APK'sı (AI bölümü)
- Tray 2026-11-17 00:10 yeniden başlatmadan sonra açılmadı (nedeni bilinmiyor)
- `install.ps1` Claude masaüstü oturumundan çalışırsa HKCU/AppData yazmaları MSIX sanallaştırmasına düşer → normal yönetici pwsh'tan çalıştır

## Sıradaki adımlar (sırayla)
1. Ayarlar 2. parti: servis ayarları (örnekleme, saklama, alarm kuralları) `lowmon-elev --op write-config` ile
2. Nihai kabul soak'u: yönetici pwsh ile `scripts/soak.ps1` (kabul: CPU < %0,05 toplam, ADR-0026; Private < 3 MB; sızıntı yok)
3. Rafta karar: `lowmon-ui` veriyi doğrudan servisten alsın (yeni ADR)

## Dokunma
- `docs/spesifikasyon.md` — tek doğruluk kaynağı; spekten sapma önce ADR
- `target/`, `graphify-out/`, `lowmonitor_full.txt` — üretilmiş/büyük dosyalar, okuma dışı
- `.claude/worktrees/*` — başka oturumların çalışma alanı, bu oturumdan değiştirme
- main'e doğrudan push yok (pre-push hook); kurulu `Program Files` exe'leri elle değiştirme

## Kritik dosyalar
- `CLAUDE.md` — kurallar · `TODO.md` — milestone durumu · `docs/spesifikasyon.md` — spek
- `docs/devlog/2026-11.md` — gelişim günlüğü · `docs/kararlar/ADR-*.md` — kararlar (son: ADR-0034)
- `crates/lowmon-proto/src/proto.rs` — pipe protokolü (LowDeck sözleşmesi, `PROTO` sürümü)

## Doğrulama
- `cargo fmt --check` · `cargo clippy --workspace -- -D warnings` · `cargo test --workspace` (son kayıt: 87 test)

## Kısıtlar / kararlar
- Yasak: tokio/async runtime, PDH, WMI, .NET, WebView (spek §1)
- Rust 1.100.0-msvc, `crt-static`; her `unsafe` üstünde `// SAFETY:`
- Bütçe: CPU < %0,05 toplam, Private WS/Bytes < 3 MB (24 saat)
- Bilinen borçlar (TODO.md "Kısayollar"): firewall portu sabit 47800, `tokens.bin`/`state.bin` atomik değil, ≤ 64 çekirdek

---
## Geri dönüş (Claude Code'a döndüğünde)
> HANDOFF.md'yi oku. Aşağıdaki çıktı <Gemini/OpenCode>'den geldi. Uygulamadan önce mevcut kodla çelişkileri kontrol et, sonra uygula, devlog'a ekle, HANDOFF.md'yi güncelle.
