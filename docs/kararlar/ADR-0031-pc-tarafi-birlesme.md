# ADR-0031: PC tarafı tek üründe: lowdeck-agent → lowmon-tray, masaüstü → lowmon-ui
- Tarih: 2026-11-16
- Durum: Taslak (kullanıcı kararları alındı; uygulama fazlar halinde, her faz ayrı PR). ADR-0029'u değiştirir.

## Bağlam
Kullanıcı oturumunda bugün iki tepsi ikonu ve iki kurulum var:

| Süreç | Repo | Görev | Ölçüm (2026-11-16, ~2 sa) |
|---|---|---|---|
| `lowmon-svc` | LM | toplayıcı, pipe | private 2,9 MiB |
| `lowmon-tray` | LM | ikon, alarm balonu, "Arayüzü aç" (`UiExe`) | 303 KB exe, private 3,1 MiB, 2,1 s CPU |
| `lowdeck-agent` | LowDeck | tepsi; TCP 47810 (telefon + loopback), Core Audio, SMTC, tuş kilidi, UDP keşif 47811, pipe köprüsü | ~290 KB exe, private 3,3 MiB, 0,1 s CPU |
| `lowdeck-desktop` | LowDeck | Slint PC arayüzü (Canlı, Olaylar, Geçmiş, Süreçler) | 8,2 MB exe, private ~5 MiB (software renderer) |

ADR-0029 (B): arayüz LowDeck'te, veri yalnız agent üzerinden. Sonuç: iki tepsi, iki autostart, iki release, `lowmon-proto` rev'i elle senkron.

## Karar (kullanıcı, 2026-11-16)
1. **Agent işlevleri `lowmon-tray` içine taşınır** → tek süreç, tek tepsi ikonu. Kod yeni kütüphane crate'i `lowmon-deck`'te (hub, dinleyiciler, keşif, Core Audio/SMTC/kilit backend'i, log, pipe köprüsü); tray onu başlatır ve menüsüne öğelerini ekler.
2. **Masaüstü arayüzü LM'ye taşınır: `lowmon-ui`** (Slint, software renderer). Slint bağımlılığı yalnız bu crate'te. Tray "Arayüzü aç" → aynı klasördeki `lowmon-ui.exe` (`UiExe` kaydı kalkar ya da yalnız geçiş için okunur).
3. **`lowdeck-proto` LowDeck repo'sunda kalır** (telefon sahibi); LM onu git `rev` ile kullanır. Pipe köprüsü artık aynı repodaki `lowmon-proto`'yu yol bağımlılığıyla kullanır → rev senkronu tek yöne iner.
4. **Ayar `%APPDATA%\LowMonitor\deck.toml`** (`net.toml` yanında). İlk açılışta `%APPDATA%\LowDeck\agent.cfg` varsa `bind`, `port`, `token` aynen taşınır → telefon ayarı değişmez. Eski dosya silinmez (geri dönüş).
5. LowDeck repo'sunda `tools/lowdeck-agent` ve `apps/lowdeck-desktop` kaldırılır, `release.yml` `pc` job'u ve LowDeck ADR-0002/0003/0004 "yerine geçti: LM ADR-0031" notu alır. `mock-server` ve `cli` LowDeck'te kalır (telefon geliştirme).
6. **Medya ve ses kontrolü yalnız telefondan** (kullanıcı, 2026-11-16): `lowmon-ui` `C2S::Media` / `C2S::Audio` göndermez, medya/ses durumunu göstermez; yalnız izler (metrik, olay, geçmiş, süreçler). Tuş kilidi kapsam dışı. Uygulama UI tarafında; sunucu (`lowmon-deck`) loopback yuvasını engellemez (bugünkü `lowdeck-desktop` da göndermiyor).

## Fazlar (her biri ayrı PR, ölçümle)
| Faz | Repo | İş | Doğrulama |
|---|---|---|---|
| 1 | LM | `lowmon-deck` crate: agent kodu taşınır (tray'den bağımsız, `start(cfg) -> Running`), `lowdeck-proto` git rev, testler taşınır | `cargo test`, clippy; `cli` ile loopback uçtan uca |
| 2 | LM | `lowmon-tray` deck'i başlatır; menü birleşir (telefon satırı, ayar dosyası, firewall, log); tooltip'e telefon durumu; `deck.toml` + `agent.cfg` göçü | tray exe boyutu, private bytes, boşta CPU önce/sonra; telefon gerçek bağlantı |
| 3 | LM | `lowmon-ui` (lowdeck-desktop taşınır; `start_agent` kalkar, ayar `deck.toml`'dan) ; tray "Arayüzü aç" sabit yol | Canlı/Olaylar/Geçmiş/Süreçler ekran görüntüsü, CPU/bellek |
| 4 | LM | `install.ps1` + release: ui dahil; firewall kuralı `lowmon-tray.exe` için (TCP port + UDP 47811); eski `lowdeck-agent` Run kaydı ve kuralı temizlenir | temiz kurulum + LowDeck'ten geçiş senaryosu |
| 5 | LowDeck | agent + desktop crate'leri silinir, CI/release sadeleşir, ADR notları | CI yeşil, APK değişmez |

## Sonuçlar
- \+ Tek tepsi, tek autostart, tek kurulum, tek PC sürümü; `lowmon-proto` senkron sorunu kalkar.
- \+ Bir süreç azalır (~3 MiB private, ayrı mesaj döngüsü).
- − `lowmon-tray` büyür: `windows` crate (Core Audio COM + SMTC WinRT; `windows-sys`'te yok). Tahmin 303 → ~600 KB; Faz 2'de ölçülür.
- − Tray çökerse telefon bağlantısı da düşer (bugün ayrı süreçler).
- − Firewall kuralının programı değişir (`lowmon-tray.exe`): kullanıcı kuralı bir kez yeniden ekler (Faz 4'te otomatik).
- − LM workspace'ine Slint girer (yalnız `lowmon-ui`); derleme süresi artar, svc/tray/net exe'leri etkilenmez.

## Açık
- `lowmon-net` (WS/JSON, LAN) ile deck sunucusu (TCP/postcard, LAN) örtüşüyor: bu ADR'de dokunulmaz, ayrı karar.
- Tray'in alarm ikonu/balonu ile ui'nin alarm bandı aynı kaynaktan (pipe) beslenir; tray'deki pipe aboneliği deck'in aboneliğiyle birleştirilebilir (Faz 2'de ölçüme göre).
