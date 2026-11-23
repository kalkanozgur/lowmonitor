# ADR-0043: Tanılama raporu ve GitHub issue taslağı
- Tarih: 2026-11-24
- Durum: Kabul (kullanıcı, 2026-11-24: "GitHub issue taslağı")
- İlgili: ADR-0041/0042 (ağ tercihleri, güncelleme), `docs/destek/lowmon-diag.ps1`

## Bağlam
Arkadaşın PC'sindeki sorunları ayıklamak için betik + dosya taşıma zahmetliydi (Drive yüklemesi sorun çıkardı). İstek: menüden tek tıkla debug raporu, mümkünse GitHub'a güvenli gönderim.

## Karar
1. **Tanılama ▸** menüsü: "Tanılama raporu oluştur…", "Raporu GitHub'a gönder (issue taslağı)…", "Log klasörünü aç".
2. **Rapor** (`diag.rs`, yalnız okur): sürüm + güncelleme durumu, Windows sürümü/build, monitör sayısı, `TaskbarAl`/`TaskbarDa`, servis (`sc query`), `lowmon*` süreçleri, `tray.toml` / `taskbar.toml` / `lowvolume.toml` / `deck.toml`, `deck.log` son 120 + `lowvolume.log` son 40 satır. `%APPDATA%\LowMonitor\diag.txt`'e yazılır, Notepad'de açılır.
3. **Gizlilik:** `token`/`secret`/`password`/`sessionkey` geçen **satırlar çıkarılır**; kullanıcı adı `<kullanıcı>` olur. `ai.toml`, `tokens.bin`, `net.toml`, ProgramData `config.toml` rapora girmez.
4. **GitHub gönderimi = issue taslağı, yükleme yok:** rapor panoya kopyalanır, kullanıcı onaylarsa tarayıcıda `…/issues/new?title=…&body=…` açılır (URL'de rapor yok, yalnız başlık ve "yapıştırın" notu). Gönderme kullanıcının kendi hesabıyla, kendi yapıştırmasıyla. Binary'de token yok, LowMonitor GitHub'a kimlik doğrulamasız hiçbir veri göndermez.
5. Reddedilenler: depoya yazan token gömmek (binary'den çıkar, spam/suistimal); gizli gist için OAuth device flow (ayrı ADR, büyük iş).

## Sonuçlar
- (+) Arkadaşın PC'sinde betiksiz tek tık; kullanıcı raporu görür, onaylar, kendisi gönderir.
- (−) Depo herkese açık: gönderilen issue de herkese açık. Onay kutusu bunu açıkça söyler; hassas rapor için e-posta yolu (dosya `diag.txt`) kalır.
- (−) Maskeleme kaba (anahtar kelimeli satır silme + kullanıcı adı); telefon adları / IP'ler deck.log'da kalabilir, kullanıcı gözden geçirir.
- `shortcut:` rapor senkron toplanır (`sc.exe`, `tasklist.exe` birkaç yüz ms); UI thread'inde menü tıklamasında bir kez.
