# ADR-0038: Tek setup exe: Inno Setup
- Tarih: 2026-11-20
- Durum: Kabul (kullanıcı, 2026-11-19 / 2026-11-20)
- İlgili: ADR-0010 (servis kendini kurar), spek §11, `scripts/install.ps1`, MSIX AppData sanallaştırma notu

## Bağlam
Kurulum `cargo build` + yönetici `install.ps1` ile yapılıyordu. İstek: GitHub Release'te tek kurulum exe'si; modüller kısa açıklamalı ve varsayılan seçili, **tray + taskbar zorunlu** (gri). Claude oturumundan çalışan `install.ps1`'in HKCU/AppData yazmaları sanallaşıyordu.

## Karar
1. **Araç: Inno Setup 6** (`installer/lowmonitor.iss`). Bileşen sayfası, `fixed` zorunlu bileşen, kaldırma ve güncelleme hazır; tek UAC. Yeni Rust kodu yok. WiX (MSI) ve kendi Rust kurucumuz daha çok iş.
2. **Bileşenler** (hepsi varsayılan seçili): tray + taskbar (`fixed`), servis, `lowmon-ui` (`release-ui`), `lowmon-net`, `lowmon-hook`, `lwm-dump`, telefon firewall kuralı (TCP 47810 + UDP 47811, private + localsubnet).
3. **Adımlar `install.ps1` ile aynı:** `PrepareToInstall` süreçleri kapatır, eski servisi `uninstall` eder (exe kilidi kalkana kadar bekler), eski LowDeck / ai-usage-bar Run kayıtlarını ve "LowDeck agent" kuralını siler. Sonra `lowmon-svc install` + `start` (servis "LowMonitor net" kuralını kendisi ekler).
4. **HKCU doğru kullanıcıda:** tray autostart ve eski Run kayıtları `runasoriginaluser` / `ExecAsOriginalUser` ile kurulumu başlatan kullanıcıya yazılır (yükseltilmiş hesap / sanallaştırma tuzağı yok).
5. **Kaldırma:** süreçler, autostart, servis `uninstall`, "LowMonitor deck" kuralı. `%ProgramData%\LowMonitor` (anahtar, log, config) ve kullanıcı ayarları **silinmez**.
6. **Sürüm:** `ISCC /DMyVersion=<sürüm>`; çıktı `target\setup\LowMonitor-Setup-<sürüm>.exe`. Release CI (`release.yml`) tag'de derler ve release'e ekler.
7. `install.ps1` geliştirici yolu olarak kalır.

## Sonuçlar
- (+) Tek exe, bileşen seçimi, Windows "Uygulamalar"dan kaldırma.
- (−) `shortcut:` deck firewall portu sabit 47810 (deck.toml'da değiştiyse tray menüsünden kural yenilenir); ai-usage-bar CSV taşıma yok (v1.0 kurulumları zaten taşıdı).
- (−) Kod imzası yok: SmartScreen / UAC "Unknown publisher" (spek §15 açık konu).
