# ADR-0044: Telefondan PC güç komutları (uyku / kapat / yeniden başlat)
- Tarih: 2026-11-27
- Durum: Kabul (kullanıcı, 2026-11-27, kartla: komut seti + 10 s geri sayım; yetki kartı yanıtsız → önerilen seçenek)
- İlgili: LowDeck ADR-0012 (proto `C2S::Power` / `S2C::PowerResult`, telefon UI), ADR-0031 (deck tray'de), ADR-0036 (çoklu telefon)

## Bağlam
LowDeck telefonundan PC'yi uyutmak / kapatmak / yeniden başlatmak isteniyor. `lowmon-deck` tray sürecinde, kullanıcı oturumunda, yükseltilmemiş çalışır.

## Karar
1. **Yetki modeli:** UAC / servis / yükseltilmiş kopya yok. Standart kullanıcı token'ında `SeShutdownPrivilege` vardır ama kapalıdır; her komutta `AdjustTokenPrivileges` ile açılır (`ERROR_NOT_ALL_ASSIGNED` → 1314).
2. **API'ler** (`crates/lowmon-deck/src/power.rs`):
   | Komut | Çağrı | Not |
   |---|---|---|
   | Sleep | `SetSuspendState(FALSE, FALSE, FALSE)` | ayrı thread, 300 ms sonra (yanıt çerçevesi önce gider) |
   | Shutdown | `InitiateShutdownW(NULL, "LowDeck (telefon)", 10, SHUTDOWN_POWEROFF, OTHER\|PLANNED)` | Windows geri sayım bildirimi; `delay_s = 10` |
   | Restart | aynı, `SHUTDOWN_RESTART` | |
   | Cancel | `AbortSystemShutdownW(NULL)` | süren geri sayım yoksa 1116 |
   `ExitWindowsEx` iptal edilemediği için kullanılmaz. Uygulamalar zorla kapatılmaz (`SHUTDOWN_FORCE_*` yok): kaydedilmemiş belge kapanmayı bekletebilir.
3. **Ayar:** `deck.toml` `power` (bool, yoksa `true`). `false` → `S2C::PowerResult { code: POWER_DISABLED (-1) }`, backend çağrılmaz. Yeni oluşturulan `deck.toml`'a `power = true` satırı yazılır. Etkisi tray → Telefon → Ayarı yeniden yükle (`Running::reload` → `Hub::power_off`).
4. **Kayıt:** her komut `deck.log`'a `güç: <cmd> → <code>`.
5. Komut herhangi bir bağlı istemciden kabul edilir (token, ADR-0036 madde 3); `lowmon-ui` (loopback) şimdilik göndermez.

## Sonuçlar
- (+) Kod yalnız `lowmon-deck` (`power.rs`, `lib.rs`, `win.rs`) + `windows` özellikleri `Win32_Security`, `Win32_System_Power`, `Win32_System_Shutdown` (yalnız bağlama; yeni crate yok).
- (+) Yanlış dokunuşa karşı telefonda iki aşamalı onay + PC'de 10 s iptal edilebilir geri sayım.
- (−) Eski LowDeck APK'sı `Power` göndermez, etkilenmez; yeni APK + eski LM → bağlantı düşer: **LM sürümü önce**.
- (−) Uyandırma (Wake-on-LAN) yok; ayrı iş (MAC yayını + NIC/BIOS ayarı).
- (−) Tray menüsünde `power` anahtarı yok; kapatmak için `deck.toml` elle düzenlenir (ihtiyaç olursa menüye eklenir).
- Doğrulanacak (Windows): uyku/kapat/yeniden başlat/iptal gerçek PC'de; Modern Standby makinede `SetSuspendState` davranışı.
