# ADR-0042: GitHub'dan sürüm denetimi ve güncelleme (tıkla-kur)
- Tarih: 2026-11-24
- Durum: Kabul (kullanıcı, 2026-11-23: "Sor → indir → doğrula → kur")
- İlgili: ADR-0041 (gizlilik tercihleri), ADR-0038 (Inno setup), spek §1 (async runtime yok)

## Bağlam
Arkadaşın PC'sine kurulumdan sonra yeni sürümleri elle indirmek zahmetli (Drive yüklemesi sorun çıkardı). İstek: PC açılışında ve 24 saatte bir denetim; sağ tık menüsünde "GitHub'dan güncelle". LowMonitor şimdiye dek internete hiç çıkmıyordu (Claude kullanım API'si hariç); bu yeni bir ağ davranışı.

## Karar
1. **Denetim:** tray açılıştan 60 sn sonra, sonra 24 saatte bir `GET api.github.com/repos/kalkanozgur/lowmonitor/releases/latest` (`lowmon_ai::win::Http`, WinHTTP, TLS sistemden; yeni crate yok, async runtime yok). Yanıtta kişisel veri yok: User-Agent `LowMonitor/<sürüm>`. `latest` ön sürümleri döndürmez; ön sürüm etiketleri (`-rc`) zaten sürüm karşılaştırmasında elenir.
2. **Thread:** tüm ağ işi `update-check` / `update-download` worker thread'lerinde; UI thread'i beklemez, sonuç `WM_UPDATE` ile gelir (ADR-0040'daki UIA kilitlenmesinden ders).
3. **Güvenlik:** setup yalnız `https://github.com/kalkanozgur/lowmonitor/releases/download/…` adresinden iner; dosya GitHub'ın asset `digest` alanındaki **SHA-256** ile eşleşmezse silinir/çalıştırılmaz; özet yoksa teklif edilmez. Boyut sınırı 64 MiB.
4. **Kullanıcı onayı:** "GitHub'dan güncelle…" yeni sürüm varsa onay kutusu (sürüm, boyut, UAC uyarısı) → indir → doğrula → setup'ı `ShellExecute open` ile başlatır (manifest yönetici ister, UAC setup'tan gelir; setup tray'i kapatıp yeniden açar). **Sessiz kurulum yok.** Otomatik denetimde yeni sürüm bulunursa yalnız bilgi balonu + menü satırı.
5. **Tercih:** `tray.toml` `update_check` (menüden "Otomatik denetle" ✓). Kapalıysa hiç ağ çağrısı olmaz; elle "GitHub'dan güncelle…" yine çalışır. Yeni kurulumda setup görevi varsayılan kapalı (ADR-0041); mevcut kurulumlarda açık.
6. Menü: "Güncelleme ▸" (durum satırı, GitHub'dan güncelle…, Sürüm notları, Otomatik denetle).

## Sonuçlar
- (+) Elle indirme yok; denetim yalnız tek küçük JSON isteği; indirme yalnız kullanıcı onayıyla.
- (−) Kod imzası yok: SHA-256 özeti aynı GitHub release'inden gelir; repo/hesap ele geçirilirse korumaz (imza açık konu, spek §15). Anonim API sınırı saatte 60 istek: 24 saatte bir yeterli.
- (−) Setup 7 MB belleğe iner (sınır 64 MiB), sonra yazılır; indirme sırasında ilerleme göstergesi yok (menü "indiriliyor…").
- (−) Elle test edilmemiş kısımlar: indirme + SHA-256 + setup başlatma (yalnız birim testleri ve canlı `releases/latest` denetimi).
- `shortcut:` GitHub API 403/429 (hız sınırı) metin olarak "GitHub HTTP 403" gösterilir, yeniden deneme yok.
