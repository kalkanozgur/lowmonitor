# LowMonitor
Windows için çok düşük tüketimli kaynak izleme, loglama ve müdahale aracı (Rust, `x86_64-pc-windows-msvc`).
Ayrıntılar: [docs/spesifikasyon.md](docs/spesifikasyon.md)

| Alan | Değer |
|---|---|
| Durum | Tasarım tamam → M0 (iskelet) |
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
