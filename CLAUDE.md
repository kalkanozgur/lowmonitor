# Proje kuralları — LowMonitor

## Genel
- Dil: Türkçe doküman, kodda İngilizce tanımlayıcı + Türkçe yorum serbest.
- Tarih formatı ISO: YYYY-MM-DD. Dosya adları küçük harf, kebab-case.
- Tek doğruluk kaynağı: `docs/spesifikasyon.md`. Spekten sapan karar → önce ADR, sonra spek güncellemesi.
- Her anlamlı değişiklikten sonra: devlog girdisi + gerekiyorsa CHANGELOG.
- Geri dönüşü pahalı kararlar → `docs/kararlar/ADR-XXXX-<konu>.md`.

## Rust
- Workspace: `crates/*`, `tools/*`; bağımlılık sürümleri `[workspace.dependencies]`'te.
- Yasak: tokio/async runtime, PDH, WMI, .NET, WebView (spek §1 Prensipler).
- `cargo fmt` + `cargo clippy -D warnings` temiz olmadan push yok (pre-push hook).
- `unsafe` blokları minimum; her birinin üstünde `// SAFETY:` yorumu.
- Performans bütçesi (spek §1) aşan değişiklik ölçümle birlikte gelir.

## Build / test / CI
- Build ve testler **lokalde**: `cargo test --workspace`.
- GitHub Actions yalnızca `v*` tag'inde release build alır; push/PR'da CI yok.
- Değişiklik akışı (ADR-0021): dal → push → PR → **squash merge**. main'e doğrudan push yok (pre-push hook reddeder).
  - PR başlığı Conventional Commits formatında; squash commit'in başlığı olur.
  - PR açıklaması: ne değişti, lokal test sonucu (`cargo test --workspace` özeti), açık kalanlar.
  - Merge kullanıcı onayıyla (`gh pr merge --squash`); dal merge sonrası silinir.
- Sürümleme: SemVer, `vMAJOR.MINOR.PATCH` git tag; tag öncesi CHANGELOG güncellenir.

## Ar-Ge / ölçüm
- Performans ölçümleri: `docs/olcumler/YYYY-MM-DD_<konu>.md` (düzenek, araç, süre, sonuç).

## Commit mesajları
Conventional Commits, kapsam = crate: `feat(svc): …`, `fix(core): …`, `docs: …`, `ci: …`, `chore: …`
