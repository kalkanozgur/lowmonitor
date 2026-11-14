# ADR-0025: `/v1/info` kaynağı ve firewall kuralı
- Tarih: 2026-11-15
- Durum: Kabul (spek §16.2 firewall ve §16.3 `/v1/info` uygulaması)

## Bağlam
Spek: `/v1/info` host adı, çekirdek sayısı, toplam RAM, disk/arayüz listesi döner; `install` sırasında `INetFwPolicy2` ile yalnız Private profil + yerel alt ağ için inbound kural. `INetFwPolicy2` COM'dur, `windows-sys` COM içermez. `lowmon-net` kullanıcı oturumunda yükseltilmemiş çalışır.

## Karar
- **`/v1/info`** (token gerekli): id'ler servisin ölçtüğü son `Sample`'dan (pipe açıksa önbellek, değilse tek seferlik pipe okuması), böylece `/v1/live` ile aynı disk/arayüz kümesi döner. Adlar yerelde: disk `IOCTL_STORAGE_QUERY_PROPERTY` (vendor + model), arayüz `GetIfEntry2` takma adı. Okunamazsa `PhysicalDriveN` / `if<N>`. Servis yoksa 503.
- **Firewall:** `netsh advfirewall firewall add rule name="LowMonitor net" dir=in action=allow program=<svc klasörü>\lowmon-net.exe profile=private protocol=TCP localport=47800 remoteip=localsubnet`. `lowmon-svc install` ekler (önce aynı adlı kuralı siler), `uninstall` siler. Kural eklenemezse kurulum sürer, uyarı yazılır.
- Port kuralda sabit 47800.

## Sonuçlar
- \+ Yeni bağımlılık yok; kural program yoluna bağlı, başka süreç portu kullanamaz.
- \+ `/v1/info` ile `/v1/live` id'leri her zaman uyumlu.
- − `net.toml`'da port değişirse kural elle güncellenir.
- − İstemcisizken `/v1/info` ağ thread'ini tek örnek süresince (ölçüm: 170 ms) bloklar.
- − `lowmon-net.exe` servis exe'siyle aynı klasörde olmalı (M6d kurulumu bunu sağlar).
