# 2026-11-15 — Toplayıcı maliyeti (bench)

| Alan | Değer |
|---|---|
| Düzenek | XAN-LAPTOP, Ryzen 7 5800H (16 mantıksal), 265 süreç / 3962 thread |
| Araç | `cargo test -p lowmon-svc --release cost_per_call -- --ignored --nocapture` (duvar süresi, kernel dahil) |
| Koşul | Kullanıcı oturumu, yönetici değil, normal öncelik (servis: LocalService + EcoQoS + BELOW_NORMAL) |
| Commit | `b34ee5b` + bench |

## Sonuç (iki koşu)
| Parça | µs/çağrı | Sıklık | µs/s |
|---|---|---|---|
| Top-N tarama | 1860–2080 | 1/5 s | ~400 |
| ↳ yalnız `NtQuerySystemInformation` | ~2200 | | ≈ taramanın tamamı |
| net `GetIfEntry2` | 40–47 | 1 Hz | 45 |
| mem `K32GetPerformanceInfo` | 19 | 1 Hz | 19 |
| cpu | 10–19 | 1 Hz | 15 |
| disk | 3 | 1 Hz | 3 |
| NIC keşfi / disk keşfi | 870 / 45–63 | 1/60 s | 15 |
| **Toplam** | | | **~480–505** |

## Yorum
- Top-N maliyetin ~%83'ü ve kernel'de; ayrıştırma/HashMap ölçülemeyecek kadar küçük. Spek §1 tahmini (~400 µs/s) doğru.
- Servis soak'ında 1,45 ms/s (M6d soak ilk 5 dk) / 2,26 ms/s (M3 kısmi soak). ~1 ms/s fark toplayıcılarla açıklanmıyor; aday: EcoQoS frekansı, pipe/writer, uyanma. 24 saat soak sonrası EcoQoS açık/kapalı karşılaştırması.
- Ayar düğmesi: `topn_every` 5 → 10 ≈ −200 µs/s.
