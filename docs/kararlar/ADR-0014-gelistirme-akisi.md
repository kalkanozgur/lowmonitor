# ADR-0014: Geliştirme akışı — lokal build/test, tag tabanlı release
- Tarih: 2026-11-14
- Durum: Kabul

## Bağlam
Build ve testlerin lokalde yapılması, CI/CD'nin git üzerinden yürümesi isteniyor. Hedef Windows-only; private repoda Windows runner dakikaları 2× sayılır.

## Karar
- Kalite kapısı: `.githooks/pre-push` → `fmt --check`, `clippy -D warnings`, `test --workspace`. Etkinleştirme: `git config core.hooksPath .githooks`.
- GitHub Actions yalnızca `v*` tag push'unda `windows-latest` üzerinde release build alır, exe'leri GitHub Release'e ekler.
- Push/PR'da bulut CI yok.

## Sonuçlar
- \+ Bulut dakikası yalnızca release'te harcanır; hızlı geri bildirim lokalde.
- − Hook `--no-verify` ile atlanabilir; tag'deki kod bulutta test edilmez.
- Risk: lokal ve runner toolchain farkı → `rust-toolchain.toml` + `--locked` ile sabitlenir.
