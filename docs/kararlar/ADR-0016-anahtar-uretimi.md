# ADR-0016: Ana anahtarı servis ilk açılışta üretir
- Tarih: 2026-11-14
- Durum: Kabul (spek §5.2 ve §11 adım 6'yı değiştirir)

## Bağlam
DPAPI kullanıcı kapsamlıdır. Kurulumda admin'in `CryptProtectData` ile koruduğu blob'u LocalService hesabıyla çalışan servis çözemez.

## Karar
- `install` yalnızca `%ProgramData%\LowMonitor\{logs,state}` klasörlerini, ACL'leri ve `config.toml`'u kurar.
- `key.bin` yoksa servis ilk açılışta 32 B anahtar üretir (`getrandom`), LocalService DPAPI'siyle (kullanıcı kapsamı) korur ve DACL'ı yalnızca `NT SERVICE\LowMonitor` olacak şekilde ayarlar.
- `run` modu: `%LOCALAPPDATA%\LowMonitor-dev\` (ya da `--dir`), anahtar çalıştıran kullanıcının DPAPI'siyle korunur.

## Sonuçlar
- \+ Admin de dahil başka hiçbir hesap blob'u çözemez.
- − Anahtar yedeklenemez; LocalService profili bozulursa eski loglar okunamaz.
- Root DACL'ında Administrators için Read + FILE_DELETE_CHILD: `uninstall --purge` key.bin'i silebilsin.
