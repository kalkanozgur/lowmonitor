# ADR-0022: Dış sözleşme için ayrı `lowmon-proto` crate'i
- Tarih: 2026-11-15
- Durum: Kabul

## Bağlam
LowDeck (`E:\Documents\Projects\lowdeck`, ayrı repo) telefonda LowMonitor metriklerini gösterecek. Seçilen yol: `lowdeck-agent` `\.\pipe\LowMonitor`'a `Viewer` olarak bağlanır, Sample/Top'u kendi TCP protokolüyle telefona iter (LowDeck ADR-0002 seçenek a). M6 `lowmon-net` (WS + JSON) ayrıca yapılır; üçüncü taraf istemciler için.
Agent'ın pipe tiplerine ihtiyacı var. `lowmon-core` chacha20poly1305, hkdf, sha2, toml, windows-sys çekiyor; LowDeck CI ubuntu + windows'ta derliyor.

## Karar
- `crates/lowmon-proto`: `metrics` (Sample, TopEntry, DiskIo, NetIo, AggKind, GapReason, birim yardımcıları) + `proto` (pipe çerçevesi, ClientMsg, ServerMsg, RangeItem). Bağımlılık yalnız `serde` + `postcard`.
- `lowmon-core` bunları `pub use lowmon_proto::{metrics, proto}` ile yeniden dışa verir; iç yollar değişmez.
- LowDeck git dependency ile alır (`tag` ya da `rev` sabit). Repo private olduğu için LowDeck CI'da okuma erişimi (deploy key / fine-grained token) gerekir; kullanıcı ekler.
- Sözleşme kuralı aynen: enum varyantı yalnız sona eklenir, mevcut alan değişirse `PROTO` artar.

## Sonuçlar
- \+ Dış istemci tek doğruluk kaynağından derlenir; wire uyumsuzluğu derleme zamanında yakalanır.
- 2026-11-15: repo public; LowDeck CI için secret gerekmez. Lisans MIT OR Apache-2.0.
- − `lowmon-proto`'daki her değişiklik LowDeck'te `rev` güncellemesi ister.
