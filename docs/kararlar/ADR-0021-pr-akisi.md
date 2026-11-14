# ADR-0021: main'e değişiklik yalnız GitHub PR + squash merge ile
- Tarih: 2026-11-15
- Durum: Kabul (ADR-0014'ü genişletir; lokal kalite kapısı ve tag tabanlı release aynen)

## Bağlam
Şimdiye kadar main'e doğrudan commit/push yapılıyordu. Değişikliklerin incelenebilir bir kaydı ve onay adımı isteniyor. Repo private ve hesap GitHub Free: ruleset / branch protection API'si 403 dönüyor.

## Karar
- Her değişiklik ayrı dalda; push → PR → **squash merge**. PR başlığı Conventional Commits formatında, main'deki tek commit'in başlığı olur.
- Repo ayarı: yalnız squash merge açık; squash başlığı = PR başlığı, gövde = dal commit mesajları; merge sonrası dal silinir.
- main koruması lokal: `.githooks/pre-push` `refs/heads/main`'e push'u reddeder. Tag push'u serbest (release akışı ADR-0014).
- PR'da bulut CI yok (ADR-0014). PR açıklaması lokal test sonucunu içerir.
- Merge kullanıcının açık onayıyla `gh pr merge --squash`.

## Sonuçlar
- \+ main'de değişiklik başına tek, okunur commit; PR'da inceleme kaydı.
- − Koruma yalnız lokal: `--no-verify` veya GitHub web arayüzü main'e doğrudan yazabilir.
- 2026-11-15: repo public oldu; `main-pr-only` ruleset'i aktif (PR zorunlu, onay 0, yalnız squash, force-push/silme yasak, bypass yok). Lokal hook ek katman olarak kalır.
