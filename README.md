# LowMonitor
Windows için çok düşük tüketimli kaynak izleme, loglama ve müdahale aracı (Rust, `x86_64-pc-windows-msvc`).
Ayrıntılar: [docs/spesifikasyon.md](docs/spesifikasyon.md)

| Alan | Değer |
|---|---|
| Durum | M0 tamam → M1 (toplayıcı) |
| Modüller | yazilim (Rust workspace) |
| Başlangıç | 2026-11-14 |

## Klasör yapısı
```
lowmonitor/
├─ Cargo.toml              # workspace (M0)
├─ crates/                 # lowmon-core, -svc, -tray, -ui, -elev, -net (M0+)
├─ tools/lwm-dump/         # M2
├─ tests/
├─ .githooks/pre-push      # lokal kalite kapısı
├─ .github/workflows/      # yalnızca v* tag → release
└─ docs/
   ├─ spesifikasyon.md     # tek doğruluk kaynağı
   ├─ gereksinimler.md
   ├─ devlog/YYYY-MM.md
   ├─ kararlar/ADR-*.md
   └─ referans/
```

## Geliştirme akışı
| Aşama | Nerede | Ne |
|---|---|---|
| Build + test | Lokal | `cargo build`, `cargo test --workspace` |
| Push öncesi | Lokal hook | `fmt --check` + `clippy -D warnings` + `test` |
| Release | GitHub Actions | `git tag vX.Y.Z && git push --tags` → release build + exe'ler GitHub Release'e |

Klon sonrası hook'u bir kez etkinleştir:
```powershell
git config core.hooksPath .githooks
```

## Lisans
MIT ya da Apache-2.0, seçim sizin ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

`lowmon-ui` [Slint](https://slint.dev) ile yapıldı ([Royalty-free 2.0](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md) lisansı; atıf şartı):

<a href="https://slint.dev"><img alt="Made with Slint" src="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg" height="60"></a>
