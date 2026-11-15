# ADR-0019: Arayüz egui yerine Slint, ayrı geliştirilecek
- Tarih: 2026-11-14
- Durum: Kabul (ADR-0007'nin, spek §9'un ve M4'ün UI kısmının yerini alır); repo yeri [ADR-0029](ADR-0029-pc-arayuzu-lowdeck.md) ile kapandı: LowDeck

## Bağlam
Spek §9'da `lowmon-ui` egui/eframe (glow) ile yapılacaktı. Kullanıcı arayüzü Slint ile, yine Rust'ta, kendisi ayrıca geliştirecek.

## Karar
- `lowmon-ui` Slint ile yazılacak. Kullanıcı geliştirecek; bu repoda mı, ayrı repoda mı olacağı açık.
- UI ile servis arasındaki arayüz pipe protokolü (`lowmon-core::proto`, spek §6) olarak kalır. UI istemcisi `Hello { role: Viewer }` ile bağlanır.
- M4'te bu taraf protokolün eksiklerini kapatır:
  - `QueryRange`;
  - `Control` komutları;
  - Export öncesi yazıcıyı diske boşaltma;
  - `lowmon-elev` (süreç sonlandırma ve öncelik değiştirme).
- Telefondan izleme M6'daki `lowmon-net` ile yapılır (spek §16); bu karardan etkilenmez.

## Sonuçlar
- \+ Kullanıcının tercih ettiği araç; arayüz sözleşmesi zaten tek ve pipe protokolünde.
- − Spek §9'daki egui performans kuralları Slint için yeniden yazılmalı. Bunlar: tekrar çizimi ≤ 1 Hz tutmak, simge durumundayken `Unsubscribe`, ring buffer.
- − Exe boyutu ve bellek ölçümü Slint'in render arka ucuna göre değişir (software / femtovg / skia).
