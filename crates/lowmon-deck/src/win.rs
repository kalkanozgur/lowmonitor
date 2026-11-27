//! Windows arka ucu: Core Audio (COM) + SMTC (WinRT). Polling yok; her şey olay tabanlı.
//!
//! COM nesneleri yalnız sahibi olan thread'de kullanılır. Windows callback'leri
//! (başka thread'lerde gelir) sadece kanala mesaj bırakır; COM çağrısı yapmaz.

use crate::{Backend, Hub};
use lowdeck_proto::{AudioCmd, AudioState, MediaCmd, MediaState};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
};
use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP, VK_PAUSE,
};
use windows::core::{PCWSTR, Result, implement};

enum AudioMsg {
    Cmd(AudioCmd),
    /// Varsayılan çıkış cihazı değişti.
    Rebind,
}

enum MediaMsg {
    Cmd(MediaCmd),
    /// Aktif SMTC oturumu değişti.
    Rebind,
    /// Parça/oynatma bilgisi değişti.
    Refresh,
}

pub struct WinBackend {
    audio: Sender<AudioMsg>,
    media: Sender<MediaMsg>,
}

impl Backend for WinBackend {
    fn audio(&self, cmd: AudioCmd) {
        let _ = self.audio.send(AudioMsg::Cmd(cmd));
    }
    fn media(&self, cmd: MediaCmd) {
        let _ = self.media.send(MediaMsg::Cmd(cmd));
    }
    /// cat-lock (yönetici) kısayolu: LL hook enjekte Pause'u da kabul eder (2026-11-16 denendi). UIPI SendInput'u
    /// yalnız yönetici ön plan penceresine engeller; hook yine görür. cat-lock yoksa tuş ön plandaki uygulamaya gider.
    fn lock(&self) {
        media_key(VK_PAUSE);
        log!("kilit: Pause gönderildi");
    }
    fn power(&self, cmd: lowdeck_proto::PowerCmd) -> (i32, u16) {
        crate::power::run(cmd)
    }
    fn events(&self, from_ms: u64, to_ms: u64) -> Vec<lowdeck_proto::EventItem> {
        crate::lowmon::query_events(from_ms, to_ms).unwrap_or_else(|e| {
            log!("lowmon olay sorgusu: {e}");
            Vec::new()
        })
    }
    fn history(
        &self,
        from_ms: u64,
        to_ms: u64,
        n: u16,
    ) -> (u64, Vec<Option<lowdeck_proto::HistPoint>>) {
        crate::lowmon::query_history(from_ms, to_ms, n).unwrap_or_else(|e| {
            log!("lowmon geçmiş sorgusu: {e}");
            (0, Vec::new())
        })
    }
    fn kill(&self, pid: u32, ctime: i64) -> i32 {
        crate::procs::kill(pid, ctime)
    }
    fn temp_history(
        &self,
        from_ms: u64,
        to_ms: u64,
        n: u16,
    ) -> (u64, Vec<Option<lowdeck_proto::TempPoint>>) {
        crate::report::temp_points(from_ms, to_ms, n)
    }
    fn ai_report(&self, view: u8) -> lowdeck_proto::AiReport {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        crate::report::ai_report(view, now, utc_offset_s())
    }
    fn ai_config(&self) -> (lowdeck_proto::AiConfig, String) {
        crate::aicfg::read(&lowmon_ai::config_path())
    }
    fn set_ai_config(&self, cfg: &lowdeck_proto::AiConfig) -> (lowdeck_proto::AiConfig, String) {
        crate::aicfg::set(&lowmon_ai::config_path(), cfg, tray_reload_ai)
    }
}

/// Tray `WM_RELOAD` (bit 0 = ai + taskbar), lowmon-ui Ayarlar ile aynı mesaj. Deck tray sürecinde: kendi penceresine post.
fn tray_reload_ai() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP};
    // SAFETY: sınıf adı sabit; pencere bulunmazsa çağrı yapılmaz.
    unsafe {
        if let Ok(h) = FindWindowW(windows::core::w!("LowMonitorTray"), None) {
            let _ = PostMessageW(Some(h), WM_APP + 6, WPARAM(1), LPARAM(0));
        }
    }
}

/// `GetTimeZoneInformation` dönüşü (winnt.h); windows crate'inde sabit yok.
const TIME_ZONE_ID_DAYLIGHT: u32 = 2;

/// Yerel = UTC + dönüş (saniye).
pub fn utc_offset_s() -> i64 {
    let mut tz = TIME_ZONE_INFORMATION::default();
    // SAFETY: tz yerel, fonksiyon yalnız ona yazar.
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = tz.Bias
        + if id == TIME_ZONE_ID_DAYLIGHT {
            tz.DaylightBias
        } else {
            tz.StandardBias
        };
    -(bias as i64) * 60
}

/// Süreç listesi (ADR-0009): abone yokken bekler (sıfır uyanma), varken 2 s'de bir tarar.
fn procs_thread(hub: Arc<Hub>) {
    let ncpu = std::thread::available_parallelism().map_or(1, |n| n.get() as u64);
    let mut tracker = crate::procs::Tracker::default();
    let mut buf = Vec::new();
    let t0 = std::time::Instant::now();
    loop {
        hub.want_procs.wait_on();
        tracker.reset();
        while hub.want_procs.get() {
            let ms = t0.elapsed().as_millis() as u64;
            let rows = tracker.update(
                ms,
                ncpu,
                crate::procs::scan(&mut buf),
                crate::procs::SortBy::Cpu,
            );
            hub.push_procs(rows);
            hub.want_procs
                .sleep_while_on(std::time::Duration::from_millis(crate::procs::SCAN_MS));
        }
        hub.clear_procs();
        buf = Vec::new(); // ~512 KiB tarama buffer'ı boşta tutulmaz
    }
}

pub fn start(hub: Arc<Hub>) -> WinBackend {
    let (atx, arx) = channel();
    let (mtx, mrx) = channel();
    spawn("lowdeck-audio", {
        let (hub, tx) = (hub.clone(), atx.clone());
        move || audio_thread(hub, tx, arx)
    });
    {
        let hub = hub.clone();
        std::thread::Builder::new()
            .name("lowdeck-procs".into())
            .spawn(move || procs_thread(hub))
            .expect("thread");
    }
    spawn("lowdeck-media", {
        let tx = mtx.clone();
        move || media_thread(hub, tx, mrx)
    });
    WinBackend {
        audio: atx,
        media: mtx,
    }
}

fn spawn(name: &str, f: impl FnOnce() -> Result<()> + Send + 'static) {
    let n = name.to_string();
    std::thread::Builder::new()
        .name(n.clone())
        .spawn(move || {
            if let Err(e) = f() {
                log!("{n}: {e}");
            }
        })
        .expect("thread");
}

fn com_init() -> Result<()> {
    // SAFETY: thread başına bir kez; MTA, callback'ler herhangi bir thread'den gelebilir.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
}

// ---------------- Ses ----------------

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCb(Arc<Hub>);

impl IAudioEndpointVolumeCallback_Impl for VolumeCb_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        // SAFETY: Windows çağrı süresince geçerli bir yapı işaretçisi verir.
        if let Some(d) = unsafe { data.as_ref() } {
            self.0
                .set_audio_level(to_percent(d.fMasterVolume), d.bMuted.as_bool());
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCb(Sender<AudioMsg>);

impl IMMNotificationClient_Impl for DeviceCb_Impl {
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.0.send(AudioMsg::Rebind);
        }
        Ok(())
    }
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

fn to_percent(scalar: f32) -> u8 {
    (scalar * 100.0).round().clamp(0.0, 100.0) as u8
}

struct Endpoint {
    vol: IAudioEndpointVolume,
    cb: IAudioEndpointVolumeCallback,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        // SAFETY: aynı thread'de kaydedilen callback'in kaydı silinir.
        let _ = unsafe { self.vol.UnregisterControlChangeNotify(&self.cb) };
    }
}

/// Varsayılan çıkış cihazına bağlanır, callback kaydeder, tam durumu Hub'a yazar.
fn bind_endpoint(en: &IMMDeviceEnumerator, hub: &Arc<Hub>) -> Result<Endpoint> {
    // SAFETY: COM çağrıları, bu thread'de init edilmiş MTA içinde.
    unsafe {
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let device = dev
            .OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_default();
        let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
        let cb: IAudioEndpointVolumeCallback = VolumeCb(hub.clone()).into();
        vol.RegisterControlChangeNotify(&cb)?;
        let volume = to_percent(vol.GetMasterVolumeLevelScalar()?);
        let muted = vol.GetMute()?.as_bool();
        hub.set_audio(AudioState {
            volume,
            muted,
            device,
        });
        Ok(Endpoint { vol, cb })
    }
}

fn audio_thread(hub: Arc<Hub>, tx: Sender<AudioMsg>, rx: Receiver<AudioMsg>) -> Result<()> {
    com_init()?;
    // SAFETY: MTA içinde COM nesnesi oluşturma.
    let en: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let dev_cb: IMMNotificationClient = DeviceCb(tx).into();
    // SAFETY: kayıt thread ömrü boyunca tutulur (dev_cb aşağıda yaşıyor).
    unsafe { en.RegisterEndpointNotificationCallback(&dev_cb)? };

    let mut ep = bind_endpoint(&en, &hub).ok(); // çıkış cihazı yoksa None; Rebind'de tekrar denenir
    for msg in rx {
        match msg {
            AudioMsg::Rebind => {
                drop(ep.take()); // önce eski callback kaydı silinir
                ep = bind_endpoint(&en, &hub).ok();
            }
            AudioMsg::Cmd(cmd) => {
                let Some(e) = &ep else { continue };
                // SAFETY: COM çağrıları bu thread'de. Sonuç VolumeCb ile push edilir.
                let _ = unsafe {
                    match cmd {
                        AudioCmd::SetVolume(v) => e.vol.SetMasterVolumeLevelScalar(
                            v.min(100) as f32 / 100.0,
                            std::ptr::null(),
                        ),
                        AudioCmd::Step(d) => e.vol.GetMasterVolumeLevelScalar().and_then(|cur| {
                            let next =
                                (to_percent(cur) as i16 + d as i16).clamp(0, 100) as f32 / 100.0;
                            e.vol.SetMasterVolumeLevelScalar(next, std::ptr::null())
                        }),
                        AudioCmd::ToggleMute => e
                            .vol
                            .GetMute()
                            .and_then(|m| e.vol.SetMute(!m.as_bool(), std::ptr::null())),
                    }
                };
            }
        }
    }
    // SAFETY: kayıt aynı nesneyle silinir.
    let _ = unsafe { en.UnregisterEndpointNotificationCallback(&dev_cb) };
    Ok(())
}

// ---------------- Medya (SMTC) ----------------

/// AUMID'den kısa uygulama adı: "Spotify.exe" → "Spotify",
/// "40087JoeFinApps.Trdo_kdbpvth5scec4!App" → "Trdo".
pub fn app_name(aumid: &str) -> &str {
    let s = aumid.split('!').next().unwrap_or(aumid);
    let s = s.split('_').next().unwrap_or(s);
    let s = s
        .strip_suffix(".exe")
        .or_else(|| s.strip_suffix(".EXE"))
        .unwrap_or(s);
    s.rsplit(['.', '\\']).next().unwrap_or(s)
}

struct Bound {
    session: Session,
    props_token: i64,
    play_token: i64,
}

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.props_token);
        let _ = self.session.RemovePlaybackInfoChanged(self.play_token);
    }
}

fn bind_session(mgr: &Manager, tx: &Sender<MediaMsg>) -> Option<Bound> {
    let session = mgr.GetCurrentSession().ok()?;
    let t1 = tx.clone();
    let props_token = session
        .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t1.send(MediaMsg::Refresh);
            Ok(())
        }))
        .ok()?;
    let t2 = tx.clone();
    let play_token = session
        .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t2.send(MediaMsg::Refresh);
            Ok(())
        }))
        .ok()?;
    Some(Bound {
        session,
        props_token,
        play_token,
    })
}

fn read_media(b: Option<&Bound>) -> MediaState {
    let Some(b) = b else {
        return MediaState::default();
    };
    let s = &b.session;
    let playing = s
        .GetPlaybackInfo()
        .and_then(|i| i.PlaybackStatus())
        .is_ok_and(|st| st == Status::Playing);
    let app = s
        .SourceAppUserModelId()
        .map(|h| app_name(&h.to_string_lossy()).to_string())
        .unwrap_or_default();
    let (title, artist) = s
        .TryGetMediaPropertiesAsync()
        .and_then(|op| op.join())
        .map(|p| {
            let title = p.Title().map(|h| h.to_string_lossy()).unwrap_or_default();
            let artist = p.Artist().map(|h| h.to_string_lossy()).unwrap_or_default();
            (title, artist)
        })
        .unwrap_or_default();
    MediaState {
        has_session: true,
        playing,
        app,
        title,
        artist,
    }
}

fn media_key(vk: VIRTUAL_KEY) {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    // SAFETY: yerel dizi, boyut doğru.
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn media_thread(hub: Arc<Hub>, tx: Sender<MediaMsg>, rx: Receiver<MediaMsg>) -> Result<()> {
    com_init()?;
    let mgr = Manager::RequestAsync()?.join()?;
    let t = tx.clone();
    let mgr_token = mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = t.send(MediaMsg::Rebind);
        Ok(())
    }))?;

    let mut bound = bind_session(&mgr, &tx);
    hub.set_media(read_media(bound.as_ref()));
    for msg in rx {
        match msg {
            MediaMsg::Rebind => {
                drop(bound.take()); // eski olay kayıtları silinir
                bound = bind_session(&mgr, &tx);
                hub.set_media(read_media(bound.as_ref()));
            }
            MediaMsg::Refresh => hub.set_media(read_media(bound.as_ref())),
            MediaMsg::Cmd(cmd) => match &bound {
                // Sonuç PlaybackInfoChanged/MediaPropertiesChanged ile push edilir.
                Some(b) => {
                    let s = &b.session;
                    let op = match cmd {
                        MediaCmd::PlayPause => s.TryTogglePlayPauseAsync(),
                        MediaCmd::Next => s.TrySkipNextAsync(),
                        MediaCmd::Prev => s.TrySkipPreviousAsync(),
                        MediaCmd::Stop => s.TryStopAsync(),
                    };
                    let _ = op.and_then(|o| o.join());
                }
                // SMTC oturumu yok: klasik medya tuşu (ör. SMTC desteklemeyen oynatıcı).
                None => media_key(match cmd {
                    MediaCmd::PlayPause => VK_MEDIA_PLAY_PAUSE,
                    MediaCmd::Next => VK_MEDIA_NEXT_TRACK,
                    MediaCmd::Prev => VK_MEDIA_PREV_TRACK,
                    MediaCmd::Stop => VK_MEDIA_STOP,
                }),
            },
        }
    }
    let _ = mgr.RemoveCurrentSessionChanged(mgr_token);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn app_names() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("40087JoeFinApps.Trdo_kdbpvth5scec4!App"), "Trdo");
        assert_eq!(
            app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(app_name("C:\\Program Files\\VLC\\vlc.exe"), "vlc");
        assert_eq!(app_name("chrome"), "chrome");
    }
}
