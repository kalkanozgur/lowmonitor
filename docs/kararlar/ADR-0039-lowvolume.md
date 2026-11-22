# ADR-0039: lowvolume tray içinde (taskbar tekerleği → ses)
- Tarih: 2026-11-22
- Durum: Kabul (kullanıcı, 2026-11-22)
- İlgili: ADR-0031 (deck tray süreci içinde), lowvolume repo `docs/spec.md` + ADR-0001

## Bağlam
Volumouse benzeri davranış: imleç taskbar üstündeyken tekerlek sistem sesini değiştirir. Ayrı repo `kalkanozgur/lowvolume` (lib + standalone exe). İstek: standalone da çalışsın; LowMonitor kuruluysa LowMonitor üzerinden çalışsın. İlk taslaktaki ayrı süreç + named pipe + süreç izleme gereksiz bulundu.

## Karar
1. **Süreç içi:** `lowmon-tray`, `lowvolume` lib'ini git bağımlılığı olarak çeker (`tag = "v0.1.0"`, workspace bağımlılığı) ve UI thread'inde `lowvolume::start` ile başlatır. Hook ve worker lowvolume'un kendi thread'lerinde; tray mesaj döngüsü hook'u geciktirmez.
2. **Tek örnek:** lowvolume `Local\LowMonitor.Volume` mutex'i alır. Standalone `lowvolume.exe` zaten çalışıyorsa tray modülü başlatmaz, menüde "standalone çalışıyor" gösterir; "Yeniden yükle" yeniden dener.
3. **Menü:** "Ses (taskbar tekerleği) ▸": durum satırı (pasif), Etkin (✓), ayar dosyasını aç, ayarı yeniden yükle.
4. **Ayar:** `%APPDATA%\LowMonitor\lowvolume.toml` (ailenin dizini); bozuksa varsayılanlar + `deck.log`'a satır.
5. **Ses:** `SendInput(VK_VOLUME_UP/DOWN)` → Windows yerel göstergesi; WASAPI/COM kodu yok, adım %2 sabit.
6. Bağımlılık sürümleri (`windows-sys` 0.61, `serde`, `toml`) workspace ile aynı; yeni transitif bağımlılık yok.

## Sonuçlar
- (+) Ek süreç, IPC ve kurulum bileşeni yok; installer değişmez (tray exe'si içinde).
- (+) RAM maliyeti iki thread yığını + birkaç KB.
- (−) lowvolume'da panic (panic=abort) tray'i düşürür → lowvolume callback/worker panic'siz, hatalar `Result`.
- (−) lowvolume sürüm yükseltmesi lowmonitor'da tag değişikliği + PR ister.
- `shortcut:` "Ayar dosyasını aç" dosya yoksa Notepad'in "oluşturulsun mu" sorusuna bırakır (standalone exe yorumlu şablon yazar); şablon lib'e taşınınca düzeltilir.
- `shortcut:` "Etkin" yalnız çalışma anında; kalıcı hâl `lowvolume.toml` `[general] enabled`.
