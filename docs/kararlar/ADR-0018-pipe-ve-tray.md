# ADR-0018: Pipe güvenliği, iş parçacığı modeli ve tray için windows-sys
- Tarih: 2026-11-14
- Durum: Kabul (spek §6.1 ve §8'den sapar)

## Bağlam
- Spek §6.1 SDDL'i `(A;;GRGW;;;IU)`. GENERIC_WRITE, pipe'larda FILE_CREATE_PIPE_INSTANCE (0x4) içerir: interaktif kullanıcı aynı adla yeni instance açıp sahte sunucu olabilir (pipe squatting) ve diğer istemcilerin (UI, net) mesajlarını alabilir.
- Async runtime yasak (§1); tek toplayıcı thread ve olay tabanlı pipe thread'i öngörülmüştü.
- §8 tray için `windows-rs` diyor; diğer tüm crate'ler `windows-sys` kullanıyor.

## Karar
1. **SDDL:** `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)(A;;0x12019b;;;IU)`.
   - IU: okuma + FILE_WRITE_DATA. FILE_CREATE_PIPE_INSTANCE yok.
   - OW: sunucu sürecinin sahibi. LocalService ve `run` modu da yeni instance açabilsin diye.
   - İlk instance `FILE_FLAG_FIRST_PIPE_INSTANCE` ile açılır: ad başkasındaysa sunucu başlamaz.
   - İstemci `GENERIC_READ | FILE_WRITE_DATA` ile açar.
2. **Thread modeli:**
   - Bir kabul thread'i; bağlantı başına bir okuyucu ve bir yazıcı thread'i, her biri 64 KB stack.
   - Overlapped I/O: aynı handle'da eşzamanlı okuma ve yazma birbirini kilitlemez.
   - Toplayıcı çerçeveyi `sync_channel(8)` kuyruğuna `try_send` ile bırakır. Yavaş istemci toplayıcıyı bekletmez; kuyruk doluysa çerçeve atlanır.
   - Sınırlar: en fazla 8 istemci, Hello için 5 s, yazma için 2 s zaman aşımı.
3. **Tray:** `windows-sys` + gizli üst düzey pencere.
   - `HWND_MESSAGE` kullanılmadı: menünün dışına tıklayınca kapanması `SetForegroundWindow` gerektiriyor.

## Sonuçlar
- \+ Abone yoksa `publish` yalnızca mutex alıp döner; serileştirme yok (M3 kabul kriteri).
- \+ Tray 270 KB, ~1 MB Private WS, ~0,5 ms/s CPU.
- − İstemci başına 2 thread, en fazla 16 thread × 64 KB.
- − svc exe 590 → 663 KB (postcard + serde + pipe). ADR-0017 önceliğinde exe boyutu en sonda.
- Açık:
  - Komut bazlı Admin kontrolü (`ImpersonateNamedPipeClient`) Control/Export ile birlikte M4'te gelecek.
  - Producer rolü henüz yok.
