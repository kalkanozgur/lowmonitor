# ADR-0034: Claude Code etkinlik durumu: taskbar'da konuşma balonu (`lowmon-ai` etkinlik + `lowmon-hook`)
- Tarih: 2026-11-17
- Durum: Taslak (kullanıcı kararları alındı; uygulama parçalar halinde, her parça ayrı PR ve tek oturumda bitecek boyutta).

## Bağlam
Claude Code çalışırken ne yaptığı ("düşünüyor", "Bash çalışıyor", "girdi bekliyor") token harcamadan görünsün. Kaynak: Claude Code **hook**'ları; harness çalıştırır, modele bir şey gitmez. İlk plan: `~/.claude/plans/claude-thinking.md` (2026-11-17 02:48); bu ADR onun güncel hâli.

## Karar (kullanıcı, 2026-11-17)
1. **`lowmon-ai`'a bağlı.** Durum makinesi `lowmon-ai`'da; tray'deki AI kullanımının parçası. Claude açık değilken hook tetiklenmez → **sıfır iş**: zamanlayıcı, yoklama, pencere yok.
2. **PC yükteyken ek yük olmasın.** Hook yolu olabildiğince hafif, maliyeti ölçülür (Parça 2); bütçe aşılırsa sık olaylar (araç başı / sonu) kapatılır.
3. **Gösterim: AI kullanım bloğunun üstünde yarı saydam konuşma balonu.** Taskbar'ın dışında, bloğun hemen üstünde; etkinlik yokken gizli.
4. **Parçalara bölünmüş iş**: aşağıdaki sıra; her parça kendi başına derlenir, test edilir, merge edilir.

## Tasarım

### Olay → durum
| Hook | Durum | Balon metni (tr / en) |
|---|---|---|
| `UserPromptSubmit` | çalışıyor | "Düşünüyor…" / "Thinking…" |
| `PreToolUse` | çalışıyor | araç adı: "Bash…", "Edit…" (MCP araçlarında son parça) |
| `PreToolUse` + `AskUserQuestion` | bekliyor | "Soru bekliyor" / "Waiting for answer" |
| `PostToolUse` | çalışıyor | "Düşünüyor…" |
| `Notification` (izin / boşta) | bekliyor | "Girdi bekliyor" / "Waiting for input" |
| `Stop` | bitti | "Bitti" / "Done"; 5 s sonra balon gizlenir (tek seferlik zamanlayıcı) |
| `SessionEnd` | oturum silinir | — |

- Yalnız `hook_event_name`, `session_id`, `tool_name` okunur. Araç girdisi, komut, dosya yolu, ileti metni **okunmaz ve gösterilmez**.
- Birden çok oturum `session_id` ile tutulur; gösterilen: `bekliyor > çalışıyor > bitti`, eşitlikte en yeni.
- Bayat kayıt: 10 dk olay gelmeyen "çalışıyor" oturum düşer (çökme / kill). Kontrol, mevcut 30 s'lik AI zamanlayıcısında ve yalnız kayıt varken; ek zamanlayıcı yok.
- "Bekliyor" **sabit turuncu**; yanıp sönme yok (sürekli zamanlayıcı gerektirir; spek §9 "animasyon yok" ile uyumlu).

### Taşıma: hook → tray
- Hook komutu **`lowmon-hook.exe`**: ayrı, çok küçük exe (yalnız `kernel32` + `user32`). stdin'deki JSON'u okur, tray penceresini bulur (`FindWindowW("LowMonitorTray")`), `WM_COPYDATA` ile gönderir (`SendMessageTimeoutW`, 50 ms, `SMTO_ABORTIFHUNG`), çıkar.
- Neden tray exe'sinin `--hook` kipi değil: tray `gdiplus`, `winhttp`, `pdh`, `shell32` yükler; her araç çağrısında bu DLL'leri yüklemek gereksiz yük. Küçük exe'nin başlatma maliyeti Parça 2'de ölçülür.
- Tray kapalıysa / yanıt vermiyorsa hook sessizce 0 döner: Claude Code asla bloklanmaz, çıktı üretilmez.
- Dosya, yoklama, soket, yeni servis yok.

### Balon (`lowmon-taskbar`)
- Ayrı üst düzey katmanlı pencere (`WS_EX_LAYERED | TOPMOST | NOACTIVATE | TRANSPARENT`): tıklamalar altındaki pencereye geçer, odak almaz. Taskbar alt penceresi olamaz (taskbar sınırının dışına çizilemez).
- GDI+: yuvarlatılmış dikdörtgen + AI bloğuna bakan küçük kuyruk, arka plan ~%70 opak, tek satır metin; renk durumdan (çalışıyor: normal, bekliyor: turuncu, bitti: soluk).
- Konum: AI bloğunun ekran dikdörtgeninin üstü, sağa yaslı; blok yer değiştirince (`EV_MOVED`) birlikte taşınır. AI bloğu gizliyse (veri yok / `[ai] enabled = false`) balon da gösterilmez.
- Tam ekran: `SHQueryUserNotificationState` meşgul / tam ekran / sunum diyorsa balon gösterilmez (oyun ve videonun üstüne çıkmasın). Durum, olay geldiğinde ve 30 s'lik kontrolde bakılır; yoklama yok.
- Ayar: `taskbar.toml` `[ai] bubble = true`.

### Hook kurulumu
- `~/.claude/settings.json` Claude Code'a ait: `install.ps1` **sessizce yazmaz**. Tray menüsü "AI kullanımı ▸ Claude Code hook'larını kur / kaldır": yalnız kendi girdilerini (komutu `lowmon-hook.exe` olanlar) ekler / siler, ilk yazmadan önce `.lowmon-backup` alır, dosya geçerli JSON değilse dokunmaz.
- `install.ps1` yalnız `lowmon-hook.exe`'yi `%ProgramFiles%\LowMonitor`'a kopyalar.

## Parçalar (sıra; her biri ayrı PR)
| # | İş | Bitti sayılır | Dokunulan |
|---|---|---|---|
| 1 | `lowmon_ai::activity`: hook JSON ayrıştırma + durum makinesi (saf kod) | birim testleri: olay tablosu, öncelik, bayat kayıt, araç adı kısaltma | `crates/lowmon-ai/src/activity.rs` |
| 2 | `lowmon-hook` exe + tray `WM_COPYDATA` alıcısı (durumu log'a yazar) + **ölçüm** | elle `echo {...} \| lowmon-hook.exe` → tray log'unda durum; `docs/olcumler/`: başlatma süresi, CPU; karar: sık olaylar açık mı | `tools/lowmon-hook/`, `crates/lowmon-tray/src/ai.rs`, `main.rs` |
| 3 | Balon: `lowmon-taskbar` balon penceresi + tray bağlantısı + tam ekran kontrolü + 5 s gizleme | canlı: sahte olaylarla üç durum, ekran görüntüsü; tray önce-sonra ölçümü | `crates/lowmon-taskbar/src/`, `crates/lowmon-tray/src/ai.rs` |
| 4 | Hook kur / kaldır menüsü + `install.ps1` + spek §8 | gerçek Claude Code oturumunda izin istemi, araç, `Stop` gözlenir | `crates/lowmon-tray/src/`, `scripts/install.ps1`, `docs/spesifikasyon.md` |
| Sonra (ayrı karar) | Telefon (LowDeck: sona yeni `S2C` çerçevesi; `AiUsage`'a alan eklenmez, düzeni bozar), IoT webhook, klavye / kasa RGB | | |

## Parça 2 kararları ve notları (2026-11-17)
| Konu | Karar / uygulama |
|---|---|
| Olaylar (kullanıcı) | **Hepsi kurulur** (`PreToolUse` + `PostToolUse` dahil). Ölçüm: olay başına ~13 ms duvar, ~8 ms CPU (süreç başlatma); araç çağrısı başına ~16 ms CPU. Yoğun oturumda (30 araç / dk) ortalama ~8 ms/s |
| Hook exe'si | Alanları kendisi çıkarır (`"anahtar":"değer"` taraması, stdin'in ilk 8 KB'ı), tray'e yalnız üç alanlık küçük JSON gider: araç girdisi / çıktısı süreçten çıkmaz, `WM_COPYDATA` küçük kalır. stdin'in kalanı okunup atılır (yazan taraf kapalı boruya yazmasın) |
| İleti | `COPYDATASTRUCT.dwData = 0x4C4D4831` ("LMH1"), en çok 2048 bayt; tray başka `WM_COPYDATA`'yı yok sayar |
| Tray | Olay → `Activity::apply`; süresi dolanlar mevcut 30 s'lik AI turunda düşer (kayıt yoksa iş yok). Parça 2'de durum değişimi `deck.log`'a yazılır (`ai: etkinlik: …`); Parça 3'te yerini balon alır |
| Pencere alt sistemi | `lowmon-hook` konsol açmaz; stdin borusu yine okunur |

## Parça 3 notları (2026-11-17)
| Konu | Uygulama |
|---|---|
| Balon | `Bar::bubble(blok, Option<Line>)`: üst düzey katmanlı pencere (`TOPMOST`, `NOACTIVATE`, `TRANSPARENT`, `TOOLWINDOW`), ilk gösterimde kurulur. GDI+ yolu: dört köşe yayı + sağ altta kuyruk; zemin ~%70 opak (koyu tema `0x2B2B2B`, açık tema `0xF3F3F3`), 1 px ince kenar |
| Konum | Bloğun sağ-üst köşesine yaslı, 2 px boşluk; blok her yerleştiğinde izlenir, içerik ve konum aynıysa yeniden çizilmez |
| Gizlenme | Etkinlik yok, AI bloğu görünmüyor, `[ai] bubble = false` ya da tam ekran / sunum (`SHQueryUserNotificationState`) |
| Renk | çalışıyor: normal, bekliyor: turuncu, bitti: soluk (`ai::color`) |
| "Bitti" | `DONE_SEC` + 0,2 s'lik tek seferlik zamanlayıcı (`TIMER_BUBBLE`) kaydı düşürür; sürekli zamanlayıcı yok |
| Log | Parça 2'deki `ai: etkinlik: …` log satırı kaldırıldı |

## Sonuçlar
- (+) Claude kapalıyken maliyet sıfır; açıkken yalnız olay başına kısa ömürlü küçük süreç + bir pencere mesajı.
- (+) Sır riski yok: yalnız olay türü ve araç adı işlenir.
- (−) Her araç çağrısında (önce + sonra) bir süreç başlar. Ölçüm kabul edilemez çıkarsa `PreToolUse` / `PostToolUse` hook'ları kurulmaz; balon yalnız "çalışıyor / bekliyor / bitti" gösterir.
- (−) Balon üst düzey topmost pencere: tam ekran kontrolü olay anında yapılır, tam ekrana geçiş ile sonraki olay arasında balon kısa süre görünebilir.
- (−) Hook ayarı kullanıcının Claude Code dosyasında; Claude Code hook biçimi değişirse kurulum komutu güncellenir.
- Spek §8 (tray), §2 (bileşen tablosu: `lowmon-hook`) Parça 4 PR'ında güncellenir.

## Açık
- Hook'un `async` çalıştırılması (Claude Code destekliyorsa araç çağrısını hiç bekletmez): Parça 4'te belgeye bakılır.
- `Notification` olayının izin istemi ile boşta bekleme ayrımı (alan adı): Parça 4'te gerçek olayla doğrulanır.
