# ADR-0040: Taskbar göstergesi Win11 Widget düğmesinden kaçınır (UI Automation)
- Tarih: 2026-11-23
- Durum: Kabul (kullanıcı, 2026-11-23)
- İlgili: ADR-0032 (taskbar göstergesi), ADR-0033 (sensörler), ADR-0031 (`windows` crate)

## Bağlam
Win11'de taskbar sola hizalıyken ve Widget'lar açıkken (arkadaşın PC'si, build 26300, 1920×1080), hava durumu düğmesi bildirim alanının hemen soluna geçer (x 1608–1760, bildirim alanı 1772). Gösterge sağ kenarını yalnız `TrayNotifyWnd`'ye göre hesapladığı için 152 px üst üste bindi. Düğme ayrı bir HWND değil, taskbar'ın XAML ağacında bir öğe (`AutomationId = WidgetsButton`); `GetWindowRect` ile bulunamaz.

## Karar
1. **UI Automation** ile `WidgetsButton` öğesinin sol kenarı okunur (`crates/lowmon-taskbar/src/widget.rs`); `anchor()` bu kenarı yalnız taskbar'ın sağ yarısındaysa ve bildirim alanının solundaysa sınır sayar.
2. **Kapı:** `HKCU\...\Explorer\Advanced` altında `TaskbarAl == 0` (sola hizalı) **ve** `TaskbarDa != 0` (değer yoksa varsayılan = açık). Hayırsa UIA hiç yüklenmez; ortalı taskbar'da düğme solda durur, çakışma yok. Win10 / widget kapalı / ortalı taskbar'da maliyet sıfır.
3. **UIA ayrı worker thread'de** (düzeltme, v1.2.3). v1.2.2'de çağrı tray UI thread'indeydi ve explorer'ı kilitledi (`AppHangXProcB1`, 3 kez; sebep: UIA explorer'ın UI thread'ini beklerken explorer gösterge pencerelerine mesaj gönderiyor, pencereler tray UI thread'inde). Artık `widget::Finder` UIA'yı kendi MTA thread'inde çalıştırır; tray yalnız önbelleğe bakar (bloklamaz), sonuç değişince `EV_MOVED` ile yeniden yerleşir. Tam yerleşimde (attach, `EV_MOVED`, tema, DPI) yeniden okuma istenir.
4. COM arayüzü `windows-sys'te yok → `lowmon-taskbar'a `windows` crate'i (zaten workspace'te, deck kullanıyor). COM yalnız worker thread'inde başlatılır; tray UI thread'inde COM yok.

## Sonuçlar
- (+) Çakışma kalkar; ek süreç / thread yok; gate yüzünden çoğu PC'de ek bellek yok.
- (−) `lowmon-taskbar` artık COM'a bağlı (derleme + ikili boyutu artar; ölçülmedi).
- (−) Gate açıkken UIA tray yüklemesi (UIAutomationCore) kadar bellek ekler; ölçülmedi.
- `shortcut:` worker UIA'da takılırsa (explorer yanıtsız) okuma istekleri düşer, kenar eski kalır; tray ve explorer etkilenmez.
- **Kapı notu:** `TaskbarDa` değeri yoksa varsayılan = açık sayılır; bu, widget'ı politikayla kapalı PC'lerde de UIA'yı çalıştırır (sonuç: bulunamaz, zararsız).
- `shortcut:` düğme genişliği olaysız değişirse (hava durumu metni) kenar sonraki yeniden yerleşime kadar eski kalır.
- Doğrulama: `anchor` birim testi (arkadaşın ölçüleri). v1.2.2 gerçek PC'lerde denendi: geliştirici PC'sinde explorer kilitlenmesi (yukarıdaki düzeltme), widget üzerinde canlı deneme yapılmadı (kullanıcının PC'sinde Widgets anahtarı kurum politikasıyla kilitli); arkadaşın PC'sinde v1.2.2 ile doğrulanacak.
