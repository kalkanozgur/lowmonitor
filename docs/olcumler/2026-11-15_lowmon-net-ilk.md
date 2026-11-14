# lowmon-net M6a ilk ölçüm (2026-11-15)
| Alan | Değer |
|---|---|
| Düzenek | Geliştirme PC'si, 16 mantıksal çekirdek, 14 GB RAM; `lowmon-svc` `f8e1d7f` (soak sürüyor) |
| Exe | `lowmon-net` release, 646 144 B (opt-level 3, LTO fat, strip) |
| Araç | PowerShell `ClientWebSocket`, 1 istemci, ~15 s; `Get-Process` |
| Sonuç | Private 992 KB, WS 6.5 MB, 4 thread. Mesaj 360–370 B. 1 Hz akış, Top-N 5 s'de bir dolu, `rate_ms: 2000` sonrası 2.0 s aralık. |
| Not | Uzun süreli CPU ve boşta (istemcisiz) ölçüm M6d'de. |
