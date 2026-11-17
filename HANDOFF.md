# HANDOFF — LowMonitor
**Güncelleme:** 2026-11-17 · **Güncelleyen:** Claude (sohbet; TODO.md + devlog 2026-11-15 sonuna göre) · **Kaynak durumu:** main, devlog son girdisi 2026-11-15

## Durum
Rust workspace (svc, tray, net, core, proto, lwm-dump) çalışıyor; M0 ve M3 tamam, M1/M2/M4/M5/M6 kod tarafında büyük ölçüde tamam, **nihai kabul soak'u henüz yok**. main `836c06a` kurulu. PC arayüzü bu repoda değil, LowDeck'te (ADR-0029).

## Son yapılanlar (2026-11-15)
- M5 alarmlar: durum makinesi, REC_EVENT, pipe `CH_EVENT`/`Alarms`, tray balonu, lowmon-net `alarms` — Event yolu canlı doğrulandı
- M6: telefon kabul testi ✅ (Samsung M21, LAN); `install.ps1` + firewall
- pipe `QueryEvents` (LowDeck ADR-0003), tray "Arayüzü aç" → `UiExe` (`ShellExecuteW`)
- Soak kullanıcı kararıyla 12:33'te durduruldu (taban çizgisi ~1 sa 20 dk): `docs/olcumler/2026-11-15_m6d-soak.csv`

## Açık sorun / blokaj
- CPU: toplayıcılar ~0,5 ms/s, servis 1,45–2,26 ms/s → ~1 ms/s fark açık (EcoQoS açık/kapalı karşılaştırması soak sonrası)
- Tray "Aktif alarm: …" bağlanış balonu canlı görülmedi
- Worktree'ler ve dallar incelenmedi: `.claude/worktrees/{devam-1a0c49,graphify-98adac,desktop-mascot-animations-6f1445}`, dallar `feat/ai-bubble`, `feat/ai-hook-setup`, `feat/ui-hover`, `feat/ui-settings`. Bunlarda `lowmon-ui/ai/deck/taskbar/sensors` crate'leri var; main'den ilerideler. Devlog'da 2026-11-16/17 girdisi yok.

## Sıradaki adımlar (sırayla)
1. Durumu doğrula: `git status`, `git branch -a`, `git worktree list`; hangi dalın güncel olduğunu belirle, devlog boşluğunu doldur, bu dosyayı düzelt
2. Nihai kabul soak'u: yönetici pwsh ile `scripts/soak.ps1` (kabul: CPU < %0,05 toplam, ADR-0026; Private < 3 MB; sızıntı yok)
3. `lowmon-elev` (kill/priority/affinity + `--op write-config`) ve uçtan uca Admin testi

## Dokunma
- `docs/spesifikasyon.md` — tek doğruluk kaynağı; spekten sapma önce ADR
- `target/`, `graphify-out/`, `lowmonitor_full.txt` — üretilmiş/büyük dosyalar, okuma dışı
- `.claude/worktrees/*` — başka oturumların çalışma alanı, bu oturumdan değiştirme
- main'e doğrudan push yok (pre-push hook); kurulu `Program Files` exe'leri elle değiştirme

## Kritik dosyalar
- `CLAUDE.md` — kurallar · `TODO.md` — milestone durumu · `docs/spesifikasyon.md` — spek
- `docs/devlog/2026-11.md` — gelişim günlüğü · `docs/kararlar/ADR-*.md` — kararlar (son: ADR-0029)
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
