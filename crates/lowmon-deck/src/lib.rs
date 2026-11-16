//! LowDeck PC sunucusu (LowDeck ADR-0002/0003, LM ADR-0031). Kullanıcı oturumunda, tray süreci içinde çalışır.
//! Tray'den bağımsız: `start(cfg) -> Running`; menü/ikon tray'in işi (`Hub::on_client`).
//!
//! Thread modeli (async yok, polling yok):
//! - accept: yeni bağlantı eskisinin yerine geçer (telefon tek istemci).
//! - client: telefondan okur, komutları audio/media thread'lerine kanalla iletir.
//! - audio / media (`win.rs`): COM/WinRT nesnelerinin sahibi, `recv`'de bloklu;
//!   Windows olayları (volume callback, SMTC event) kanala mesaj bırakır.
//! - lowmon (`lowmon.rs`): telefon metriklere aboneyken `\\.\pipe\LowMonitor`'a Viewer
//!   olarak bağlanır; abone yokken pipe kapalı, thread `Want`'ta uyur.
//! - `Hub`: son durumu tutar, yalnız gerçekten değişince istemciye push eder.

// Windows dışında yalnız testler derlenir (CI Linux); gerisi kullanılmaz.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use lowdeck_proto::*;
use std::io::{BufReader, BufWriter};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[macro_use]
pub mod log;
mod lowmon;
#[cfg(windows)]
mod win;

#[cfg(not(test))]
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const HELLO_TIMEOUT: Duration = Duration::from_millis(300);
/// İstemci 25 s'de bir ping atar; bu sürede hiçbir şey gelmezse bağlantı ölü.
const READ_TIMEOUT: Duration = Duration::from_secs(90);
/// Yavaş/kaybolmuş istemci Hub kilidini bu süreden uzun tutamaz.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------- Hub ----------------

#[derive(Default)]
pub struct Hub {
    inner: Mutex<HubInner>,
    /// Bağlı telefon değişince çağrılır (tray ikonu). Kilit dışında çağrılır.
    pub on_client: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Bağlı telefon metriklere abone mi (lowmon thread bunu bekler).
    pub want: Want,
}

/// Koşul değişkenli bayrak: lowmon thread abonelik yokken burada uyur (sıfır uyanma).
#[derive(Default)]
pub struct Want {
    on: Mutex<bool>,
    cv: std::sync::Condvar,
}

impl Want {
    fn set(&self, v: bool) {
        let mut g = self.on.lock().unwrap_or_else(|e| e.into_inner());
        if *g != v {
            *g = v;
            self.cv.notify_all();
        }
    }

    pub fn get(&self) -> bool {
        *self.on.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `true` olana kadar bekler.
    pub fn wait_on(&self) {
        let g = self.on.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.cv
                .wait_while(g, |on| !*on)
                .unwrap_or_else(|e| e.into_inner()),
        );
    }

    /// `d` kadar bekler, abonelik biterse erken döner.
    pub fn sleep_while_on(&self, d: Duration) {
        let g = self.on.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.cv
                .wait_timeout_while(g, d, |on| *on)
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}

struct Client {
    id: u64,
    name: String,
    w: BufWriter<TcpStream>,
    metrics: bool,
    events: bool,
}

/// Yuva = dinleyici (ADR-0003): LAN (telefon) ve loopback (PC arayüzü) birbirini düşürmez.
pub const SLOT_LAN: usize = 0;
pub const SLOT_LOCAL: usize = 1;

#[derive(Default)]
struct HubInner {
    audio: AudioState,
    media: MediaState,
    next_id: u64,
    clients: [Option<Client>; 2],
    /// LowMonitor aktif alarm listesi (pipe `Alarms` + olaylar); yeni olay abonesine ilk çerçeve.
    alarms: Vec<String>,
}

impl HubInner {
    /// `pick` uyan istemcilere yazar; false: en az biri yazma hatasıyla düşürüldü.
    fn send_to(&mut self, msg: &S2C, pick: impl Fn(&Client) -> bool) -> bool {
        let mut ok = true;
        for slot in &mut self.clients {
            if let Some(c) = slot.as_mut().filter(|c| pick(c))
                && write_frame(&mut c.w, msg).is_err()
            {
                let _ = c.w.get_ref().shutdown(Shutdown::Both);
                *slot = None;
                ok = false;
            }
        }
        ok
    }

    fn by_id(&mut self, id: u64) -> Option<&mut Client> {
        self.clients.iter_mut().flatten().find(|c| c.id == id)
    }
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, HubInner> {
        // Zehirlenmiş kilit: içerik yalnız önbellek + soket, kullanmaya devam etmek güvenli.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// İstemci ya da aboneliği değişti: tray + lowmon'a bildir. Kilit dışında çağrılır.
    fn changed(&self) {
        let want = self
            .lock()
            .clients
            .iter()
            .flatten()
            .any(|c| c.metrics || c.events);
        self.want.set(want);
        if let Some(f) = self.on_client.get() {
            f();
        }
    }

    fn subscribe(&self, id: u64, t: Topics) {
        let mut g = self.lock();
        let alarms = S2C::Alarms(g.alarms.clone());
        if let Some(c) = g.by_id(id) {
            let new_events = t & TOPIC_EVENTS != 0 && !c.events;
            c.metrics = t & TOPIC_METRICS != 0;
            c.events = t & TOPIC_EVENTS != 0;
            if new_events && write_frame(&mut c.w, &alarms).is_err() {
                let _ = c.w.get_ref().shutdown(Shutdown::Both);
            }
        }
        drop(g);
        self.changed();
    }

    /// Yalnız abone istemcilere; önbelleğe alınmaz (eski örnek göstermek yanıltıcı).
    pub fn push_metrics(&self, msg: S2C) {
        if !self.lock().send_to(&msg, |c| c.metrics) {
            self.changed();
        }
    }

    /// Pipe `Alarms`: önbellek + olay abonelerine.
    pub fn set_alarms(&self, alarms: Vec<String>) {
        let mut g = self.lock();
        g.alarms = alarms.clone();
        if !g.send_to(&S2C::Alarms(alarms), |c| c.events) {
            drop(g);
            self.changed();
        }
    }

    /// Pipe `Event`: aktif listeyi günceller + olay abonelerine.
    pub fn push_event(&self, e: EventItem) {
        let mut g = self.lock();
        match e.kind {
            0 if !g.alarms.contains(&e.name) => g.alarms.push(e.name.clone()),
            1 => g.alarms.retain(|a| *a != e.name),
            _ => {}
        }
        if !g.send_to(&S2C::Event(e), |c| c.events) {
            drop(g);
            self.changed();
        }
    }

    /// Tray için: bağlı telefonun adı (LAN yuvası; PC arayüzü sayılmaz).
    pub fn phone_name(&self) -> Option<String> {
        self.lock().clients[SLOT_LAN]
            .as_ref()
            .map(|c| c.name.clone())
    }

    fn push(&self, msg: S2C) {
        let ok = self.lock().send_to(&msg, |_| true);
        if !ok {
            self.changed();
        }
    }

    pub fn set_audio(&self, a: AudioState) {
        let mut g = self.lock();
        if g.audio != a {
            g.audio = a.clone();
            drop(g);
            self.push(S2C::Audio(a));
        }
    }

    /// Volume callback cihaz adını vermez; ad önbellekten korunur.
    pub fn set_audio_level(&self, volume: u8, muted: bool) {
        let device = self.lock().audio.device.clone();
        self.set_audio(AudioState {
            volume,
            muted,
            device,
        });
    }

    pub fn set_media(&self, m: MediaState) {
        let mut g = self.lock();
        if g.media != m {
            g.media = m.clone();
            drop(g);
            self.push(S2C::Media(m));
        }
    }

    /// Welcome + tam durumu kilit altında yazar: arada push kaçmaz/araya girmez.
    /// Aynı yuvadaki önceki istemci kapatılır. Dönen id ile `reply`/`detach` yapılır.
    fn attach(&self, slot: usize, name: String, mut w: BufWriter<TcpStream>) -> Option<u64> {
        let mut g = self.lock();
        let hello = [
            S2C::Welcome {
                proto: PROTO_VERSION,
                server: server_name(),
            },
            S2C::Audio(g.audio.clone()),
            S2C::Media(g.media.clone()),
        ];
        for m in &hello {
            write_frame(&mut w, m).ok()?;
        }
        if let Some(old) = g.clients[slot].take() {
            let _ = old.w.get_ref().shutdown(Shutdown::Both);
        }
        g.next_id += 1;
        let id = g.next_id;
        g.clients[slot] = Some(Client {
            id,
            name,
            w,
            metrics: false,
            events: false,
        });
        drop(g);
        self.changed();
        Some(id)
    }

    fn reply(&self, id: u64, msg: &S2C) {
        if !self.lock().send_to(msg, |c| c.id == id) {
            self.changed();
        }
    }

    /// Bağlı istemcileri düşürür (ayar yeniden yüklendi).
    fn kick(&self) {
        let mut g = self.lock();
        let mut any = false;
        for c in g.clients.iter_mut().filter_map(Option::take) {
            let _ = c.w.get_ref().shutdown(Shutdown::Both);
            any = true;
        }
        drop(g);
        if any {
            self.changed();
        }
    }

    fn detach(&self, id: u64) {
        let mut g = self.lock();
        let mut found = false;
        for slot in &mut g.clients {
            if slot.as_ref().is_some_and(|c| c.id == id) {
                *slot = None;
                found = true;
            }
        }
        drop(g);
        if found {
            self.changed();
        }
    }
}

fn server_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "pc".into())
}

/// Komutların gideceği arka uç (Windows'ta audio/media thread kanalları).
pub trait Backend: Send + Sync {
    fn audio(&self, cmd: AudioCmd);
    fn media(&self, cmd: MediaCmd);
    /// Tuş kilidi (ADR-0008); varsayılan: yok.
    fn lock(&self) {}
    /// LowMonitor olay geçmişi (pipe `QueryEvents`); hata/servis yok → boş.
    fn events(&self, _from_ms: u64, _to_ms: u64) -> Vec<EventItem> {
        Vec::new()
    }
    /// LowMonitor metrik geçmişi, `n` kova (ADR-0005): (step_ms, kovalar); hata/servis yok → boş.
    fn history(&self, _from_ms: u64, _to_ms: u64, _n: u16) -> (u64, Vec<Option<HistPoint>>) {
        (0, Vec::new())
    }
}

// ---------------- Bağlantı ----------------

/// Yuva yerel adresten: loopback dinleyici → PC arayüzü, diğeri → telefon (ADR-0003).
fn handle(
    stream: TcpStream,
    token: &str,
    hub: &Hub,
    backend: &dyn Backend,
) -> Result<(), FrameError> {
    let local = stream.local_addr().is_ok_and(|a| a.ip().is_loopback());
    handle_in(
        stream,
        if local { SLOT_LOCAL } else { SLOT_LAN },
        token,
        hub,
        backend,
    )
}

fn handle_in(
    stream: TcpStream,
    slot: usize,
    token: &str,
    hub: &Hub,
    backend: &dyn Backend,
) -> Result<(), FrameError> {
    let peer = stream.peer_addr().ok();
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(WRITE_TIMEOUT)).ok();
    // Timeout'lar okuyan handle'a: Windows'ta try_clone'lu handle SO_RCVTIMEO'yu paylaşmıyor.
    let mut rd = BufReader::new(stream.try_clone()?);
    rd.get_ref().set_read_timeout(Some(HELLO_TIMEOUT)).ok();
    let mut buf = [0u8; MAX_FRAME];

    match read_frame::<_, C2S>(&mut rd, &mut buf)? {
        C2S::Hello {
            proto,
            token: t,
            name,
        } => {
            let err = if proto != PROTO_VERSION {
                Some(ErrorCode::BadVersion)
            } else if t != token {
                // shortcut: sabit zamanlı karşılaştırma ve hatalı deneme cezası yok; LAN + eşleştirme gelince (PROTOCOL §5).
                Some(ErrorCode::BadToken)
            } else {
                None
            };
            if let Some(e) = err {
                log!("- reddedildi: {peer:?} ({e:?})");
                write_frame(&mut BufWriter::new(stream), &S2C::Error(e))?;
                return Ok(());
            }
            log!("+ bağlandı: {name} {peer:?}");
            rd.get_ref().set_read_timeout(Some(READ_TIMEOUT)).ok();
            let Some(id) = hub.attach(slot, name, BufWriter::new(stream)) else {
                return Ok(());
            };
            session(&mut rd, &mut buf, id, hub, backend)
        }
        _ => Ok(()),
    }
}

fn session(
    rd: &mut BufReader<TcpStream>,
    buf: &mut [u8; MAX_FRAME],
    id: u64,
    hub: &Hub,
    backend: &dyn Backend,
) -> Result<(), FrameError> {
    let r = (|| loop {
        match read_frame::<_, C2S>(rd, buf)? {
            C2S::Ping(n) => hub.reply(id, &S2C::Pong(n)),
            // Sonuç Windows olayı ile (gerçekten değişirse) push edilir.
            C2S::Audio(cmd) => backend.audio(cmd),
            C2S::Media(cmd) => backend.media(cmd),
            C2S::Lock => backend.lock(),
            C2S::Subscribe(t) => hub.subscribe(id, t),
            // Bu istemcinin okuma thread'inde bloklu; diğer istemciler etkilenmez.
            C2S::QueryEvents { from_ms, to_ms } => {
                for chunk in backend.events(from_ms, to_ms).chunks(EVENTS_PER_FRAME) {
                    hub.reply(id, &S2C::Events(chunk.to_vec()));
                }
                hub.reply(id, &S2C::Events(Vec::new())); // bitti
            }
            C2S::QueryHistory {
                from_ms,
                to_ms,
                points,
            } => {
                let (step_ms, v) =
                    backend.history(from_ms, to_ms, points.clamp(1, HIST_POINTS_MAX));
                for chunk in v.chunks(HIST_PER_FRAME) {
                    hub.reply(
                        id,
                        &S2C::History {
                            from_ms,
                            step_ms,
                            points: chunk.to_vec(),
                        },
                    );
                }
                hub.reply(
                    id,
                    &S2C::History {
                        from_ms,
                        step_ms,
                        points: Vec::new(),
                    },
                ); // bitti
            }
            C2S::Hello { .. } => hub.reply(id, &S2C::Error(ErrorCode::Unsupported)),
        }
    })();
    hub.detach(id);
    r
}

// ---------------- Ayar ----------------

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Config {
    /// Bu PC'nin LAN IP'si (telefonunki değil).
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub token: String,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn appdata() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into())
}

/// `%APPDATA%\LowMonitor\deck.toml` (ADR-0031 §4), `net.toml` yanında.
pub fn config_path() -> PathBuf {
    appdata().join("LowMonitor").join("deck.toml")
}

/// LowDeck agent ayarı: yalnız ilk açılışta göç için okunur, silinmez (geri dönüş).
pub fn legacy_path() -> PathBuf {
    appdata().join("LowDeck").join("agent.cfg")
}

/// `agent.cfg` biçimi `key=value`; bind ve token yoksa `None`.
fn parse_legacy(text: &str) -> Option<Config> {
    let mut c = Config {
        bind: String::new(),
        port: DEFAULT_PORT,
        token: String::new(),
    };
    for (k, v) in text.lines().filter_map(|l| l.split_once('=')) {
        let v = v.trim().to_string();
        match k.trim() {
            "bind" => c.bind = v,
            "port" => c.port = v.parse().unwrap_or(DEFAULT_PORT),
            "token" => c.token = v,
            _ => {}
        }
    }
    (!c.bind.is_empty() && !c.token.is_empty()).then_some(c)
}

/// `deck.toml`'u okur (oluşturmaz): arayüz için, ayarı tray yazar.
pub fn load(path: &std::path::Path) -> std::io::Result<Config> {
    let bad = |e: String| std::io::Error::other(format!("{}: {e}", path.display()));
    let c: Config =
        toml::from_str(&std::fs::read_to_string(path)?).map_err(|e| bad(e.to_string()))?;
    if c.bind.is_empty() || c.token.is_empty() {
        return Err(bad("bind ve token zorunlu".into()));
    }
    Ok(c)
}

/// `deck.toml` varsa okur. Yoksa `legacy`'den (agent.cfg) taşır → telefon ayarı değişmez;
/// o da yoksa LAN IP'si + rastgele token ile oluşturur.
pub fn load_or_create(path: &std::path::Path, legacy: &std::path::Path) -> std::io::Result<Config> {
    if path.exists() {
        return load(path);
    }
    let old = std::fs::read_to_string(legacy)
        .ok()
        .and_then(|t| parse_legacy(&t));
    let migrated = old.is_some();
    let c = old.unwrap_or_else(|| Config {
        bind: lan_ip(),
        port: DEFAULT_PORT,
        token: random_token(),
    });
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    // {:?}: tırnak/ters bölü kaçışı TOML temel dizgesiyle uyumlu (IP ve hex token için yeterli).
    std::fs::write(
        path,
        format!(
            "# bind: BU PC'nin LAN IP'si (telefonunki değil). Telefon ⚙ bu adres + port + token.\n\
             # Değişiklikten sonra tray → Telefon → Ayarı yeniden yükle.\n\
             bind = {:?}\nport = {}\ntoken = {:?}\n",
            c.bind, c.port, c.token
        ),
    )?;
    if migrated {
        log!("ayar taşındı: {} → {}", legacy.display(), path.display());
    } else {
        log!("ayar oluşturuldu: {}", path.display());
    }
    Ok(c)
}

/// Varsayılan rotanın yerel adresi. UDP `connect` paket göndermez; 192.0.2.1 TEST-NET (RFC 5737).
fn lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

/// shortcut: std RandomState (işletim sistemi rastgelesiyle tohumlanır) → 64 bit; eşleştirme gelince 256-bit CSPRNG.
fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    format!("{:016x}", h.finish())
}

// ---------------- Çalışma ----------------

/// Çalışan deck: hub + arka uç + dinleyiciler. Arka uç ve LowMonitor thread'leri süreç boyunca yaşar.
pub struct Running {
    pub hub: Arc<Hub>,
    backend: Arc<dyn Backend>,
    listeners: Option<Listeners>,
}

/// Hub, Windows arka ucu (Core Audio/SMTC/kilit) ve LowMonitor köprüsünü başlatır; dinleme yok.
/// Dinleyiciler `reload(cfg)` ile açılır: başlangıç hatası (port kullanımda) arka ucu yeniden kurmaz.
#[cfg(windows)]
pub fn start() -> Running {
    log!(
        "lowmon-deck {} başladı (proto v{PROTO_VERSION})",
        env!("CARGO_PKG_VERSION")
    );
    let hub = Arc::new(Hub::default());
    let backend: Arc<dyn Backend> = Arc::new(win::start(hub.clone()));
    lowmon::start(hub.clone());
    Running {
        hub,
        backend,
        listeners: None,
    }
}

impl Running {
    /// Geçerli ayar (`bind` DHCP yüzünden `lan_ip()`'ye düşmüş olabilir); dinleme durmuşsa `None`.
    pub fn config(&self) -> Option<&Config> {
        self.listeners.as_ref().map(|l| &l.cfg)
    }

    /// Dinlemeyi (yeniden) aç. "Ayarı yeniden yükle": eskiyi durdur, istemcileri düşür, yeniyi aç; olmazsa eskiye dön.
    pub fn reload(&mut self, new: Config) -> Result<(), String> {
        let old = self.listeners.take().map(|r| {
            let cfg = r.cfg.clone();
            r.stop();
            cfg
        });
        self.hub.kick(); // telefon yeni adres/token ile yeniden bağlansın
        match listen(new.clone(), &self.hub, &self.backend) {
            Ok(r) => {
                self.listeners = Some(r);
                Ok(())
            }
            Err(e) => {
                self.listeners = old.and_then(|c| listen(c, &self.hub, &self.backend).ok());
                let state = if self.listeners.is_some() {
                    "Eski ayarla devam ediliyor."
                } else {
                    "Dinleme durdu."
                };
                Err(format!(
                    "{}

{state}",
                    bind_error(&new, &e)
                ))
            }
        }
    }

    /// Dinleyicileri kapatır (portlar hemen boşalır).
    pub fn stop(mut self) {
        if let Some(l) = self.listeners.take() {
            l.stop();
        }
    }
}

/// Çalışan dinleyiciler (LAN + loopback). `stop` ile kapatılır (yeniden yükleme).
struct Listeners {
    cfg: Config,
    /// İlki `bind` (LAN), varsa ikincisi loopback.
    addrs: Vec<std::net::SocketAddr>,
    /// Keşif soketi açıksa: `stop`'ta uyandırmak için kendine datagram.
    udp_wake: Option<SocketAddr>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Listeners {
    fn stop(self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // accept() bloklu: kendine bağlanarak uyandır, sonra thread (ve dinleyici) bitsin.
        for a in &self.addrs {
            let _ = TcpStream::connect_timeout(a, Duration::from_secs(1));
        }
        if let Some(a) = self.udp_wake {
            let _ = UdpSocket::bind(("127.0.0.1", 0)).and_then(|u| u.send_to(&[0], a));
        }
        for t in self.threads {
            let _ = t.join();
        }
    }
}

/// 0.0.0.0 değil: seçilen arayüz (ADR-0002) + PC arayüzü için 127.0.0.1 (ADR-0003, firewall gerekmez).
/// Loopback açılamazsa telefon tarafı yine çalışır.
fn listen(
    mut cfg: Config,
    hub: &Arc<Hub>,
    backend: &Arc<dyn Backend>,
) -> std::io::Result<Listeners> {
    let lan = match TcpListener::bind((cfg.bind.as_str(), cfg.port)) {
        // DHCP adresi değiştirdi (10049): şu anki LAN IP'sine düş; keşif telefona yeni adresi bildirir. cfg dosyası değişmez.
        Err(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable => {
            let ip = lan_ip();
            log!("{} bu PC'de yok, {ip} kullanılıyor", cfg.bind);
            cfg.bind = ip;
            TcpListener::bind((cfg.bind.as_str(), cfg.port))?
        }
        r => r?,
    };
    let lan_addr = lan.local_addr()?;
    let mut listeners = vec![lan];
    if !lan_addr.ip().is_loopback() {
        match TcpListener::bind(("127.0.0.1", lan_addr.port())) {
            Ok(l) => listeners.push(l),
            Err(e) => log!("loopback dinlenemedi: {e}"),
        }
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let token = Arc::<str>::from(cfg.token.as_str());
    let (mut addrs, mut threads) = (Vec::new(), Vec::new());
    for listener in listeners {
        addrs.push(listener.local_addr()?);
        let (token, hub, backend, s) = (token.clone(), hub.clone(), backend.clone(), stop.clone());
        threads.push(
            std::thread::Builder::new()
                .name("lowdeck-accept".into())
                .spawn(move || accept_loop(listener, token, hub, backend, s))?,
        );
    }
    // Keşif (ADR-0007): yalnız LAN bind'inde (loopback'te telefon yok). Sorgu gelmezse uyanma yok.
    let mut udp_wake = None;
    if !lan_addr.ip().is_loopback() {
        match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
            Ok(u) => {
                let beacon = Beacon {
                    proto: PROTO_VERSION,
                    host: cfg.bind.clone(),
                    port: lan_addr.port(),
                    name: server_name(),
                };
                let s = stop.clone();
                threads.push(
                    std::thread::Builder::new()
                        .name("lowdeck-discovery".into())
                        .stack_size(64 * 1024)
                        .spawn(move || discovery_loop(&u, &beacon, &s))?,
                );
                udp_wake = Some(SocketAddr::from(([127, 0, 0, 1], DISCOVERY_PORT)));
            }
            Err(e) => log!("keşif dinlenemedi (UDP {DISCOVERY_PORT}): {e}"),
        }
    }
    log!(
        "dinleniyor: {addrs:?}, keşif: {}",
        if udp_wake.is_some() {
            "açık"
        } else {
            "kapalı"
        }
    );
    Ok(Listeners {
        cfg,
        addrs,
        udp_wake,
        stop,
        threads,
    })
}

/// Keşif yanıtlayıcı: yalnız `DISCOVER_REQ`'e, kaynağa tek datagram. Yanıt arası ≥ 100 ms (yansıtma sınırı).
fn discovery_loop(sock: &UdpSocket, beacon: &Beacon, stop: &std::sync::atomic::AtomicBool) {
    let reply = encode_beacon(beacon);
    let mut buf = [0u8; 16];
    let mut last: Option<std::time::Instant> = None;
    loop {
        // Windows'ta önceki yanıtın ICMP "port ulaşılamaz"ı recv_from hatası olarak dönebilir: yok sayılır.
        let r = sock.recv_from(&mut buf);
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let Ok((n, src)) = r else { continue };
        if buf[..n] != DISCOVER_REQ
            || last.is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            continue;
        }
        last = Some(std::time::Instant::now());
        if sock.send_to(&reply, src).is_ok() {
            log!("keşif: {src} sordu");
        }
    }
}

/// Gelen bağlantı kuralı (Faz 4): yalnız Private profil + yerel alt ağ, bu exe ve port.
/// `cmd.exe /c` parametresi; önce aynı adlı kural silinir → tekrar çalıştırmak güvenli (port değişince de).
pub fn firewall_args(port: u16, exe: &str) -> String {
    const RULE: &str = "netsh advfirewall firewall";
    // Ayrı ad: kurulu lowdeck-agent kuralını silmesin; eskisi Faz 4 kurulumunda temizlenir.
    const NAME: &str = "name=\"LowMonitor deck\"";
    let add = |proto: &str, port: u16| {
        format!(
            "{RULE} add rule {NAME} dir=in action=allow protocol={proto} localport={port} profile=private \
             remoteip=localsubnet program=\"{exe}\""
        )
    };
    // TCP: bağlantı; UDP: keşif (ADR-0007). Aynı ad → tek `delete` ikisini de siler.
    format!(
        "/c {RULE} delete rule {NAME} >nul 2>&1 & {} & {}",
        add("TCP", port),
        add("UDP", DISCOVERY_PORT)
    )
}

/// Kullanıcıya gösterilecek bind hatası; en sık hata: bind'a telefonun IP'sini yazmak.
pub fn bind_error(cfg: &Config, e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(10049) => format!(
            "{} bu PC'nin adresi değil (bu PC: {}).\nbind = PC'nin LAN IP'si, telefonunki değil.",
            cfg.bind,
            lan_ip()
        ),
        Some(10048) => format!(
            "Port {} kullanımda (başka bir program ya da ikinci agent).",
            cfg.port
        ),
        _ => format!("{}:{} dinlenemedi: {e}", cfg.bind, cfg.port),
    }
}

fn accept_loop(
    listener: TcpListener,
    token: Arc<str>,
    hub: Arc<Hub>,
    backend: Arc<dyn Backend>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    for conn in listener.incoming() {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let Ok(s) = conn else { continue };
        let (hub, backend, token) = (hub.clone(), backend.clone(), token.clone());
        let _ = std::thread::Builder::new()
            .name("lowdeck-client".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                if let Err(e) = handle(s, &token, &hub, &*backend) {
                    log!("- koptu ({e})");
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    struct Null;
    impl Backend for Null {
        fn audio(&self, _: AudioCmd) {}
        fn media(&self, _: MediaCmd) {}
    }

    /// Hello → tam durum; değişmeyen durum push edilmez; yeni bağlantı eskisini kapatır.
    #[test]
    fn hub_flow() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let hub = Arc::new(Hub::default());
        hub.set_audio(AudioState {
            volume: 40,
            muted: false,
            device: "H".into(),
        });
        {
            let hub = hub.clone();
            std::thread::spawn(move || {
                for s in l.incoming() {
                    let hub = hub.clone();
                    std::thread::spawn(move || handle(s.unwrap(), "t", &hub, &Null));
                }
            });
        }
        let connect = || {
            let s = TcpStream::connect(addr).unwrap();
            let mut w = BufWriter::new(s.try_clone().unwrap());
            write_frame(
                &mut w,
                &C2S::Hello {
                    proto: PROTO_VERSION,
                    token: "t".into(),
                    name: "x".into(),
                },
            )
            .unwrap();
            BufReader::new(s)
        };
        let mut buf = [0u8; MAX_FRAME];
        let mut a = connect();
        assert!(matches!(
            read_frame(&mut a, &mut buf).unwrap(),
            S2C::Welcome { .. }
        ));
        assert_eq!(
            read_frame::<_, S2C>(&mut a, &mut buf).unwrap(),
            S2C::Audio(hub.lock().audio.clone())
        );
        assert!(matches!(
            read_frame(&mut a, &mut buf).unwrap(),
            S2C::Media(_)
        ));

        hub.set_audio_level(40, false); // değişmedi → push yok
        hub.set_audio_level(55, true);
        let got: S2C = read_frame(&mut a, &mut buf).unwrap();
        assert_eq!(
            got,
            S2C::Audio(AudioState {
                volume: 55,
                muted: true,
                device: "H".into()
            })
        );

        let mut b = connect();
        assert!(matches!(
            read_frame(&mut b, &mut buf).unwrap(),
            S2C::Welcome { .. }
        ));
        assert!(matches!(
            read_frame::<_, S2C>(&mut a, &mut buf),
            Err(FrameError::Closed | FrameError::Io(_))
        ));
    }

    /// Metrik yalnız abone istemciye gider; `want` abonelik ve bağlantıyı izler.
    #[test]
    fn metrics_subscription() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let hub = Arc::new(Hub::default());
        {
            let hub = hub.clone();
            std::thread::spawn(move || handle(l.accept().unwrap().0, "t", &hub, &Null));
        }
        let s = TcpStream::connect(addr).unwrap();
        let mut w = BufWriter::new(s.try_clone().unwrap());
        write_frame(
            &mut w,
            &C2S::Hello {
                proto: PROTO_VERSION,
                token: "t".into(),
                name: "x".into(),
            },
        )
        .unwrap();
        let mut rd = BufReader::new(s.try_clone().unwrap());
        let mut buf = [0u8; MAX_FRAME];
        for _ in 0..3 {
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap();
        }
        hub.push_metrics(S2C::Metrics(None)); // abone değil → gitmez
        let wait = |want: bool| {
            for _ in 0..100 {
                if hub.want.get() == want {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("want != {want}");
        };
        write_frame(&mut w, &C2S::Subscribe(TOPIC_METRICS)).unwrap();
        wait(true);
        hub.push_metrics(S2C::Top(vec![]));
        assert_eq!(
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap(),
            S2C::Top(vec![])
        );
        write_frame(&mut w, &C2S::Subscribe(0)).unwrap();
        wait(false);
        write_frame(&mut w, &C2S::Subscribe(TOPIC_METRICS)).unwrap();
        wait(true);
        s.shutdown(Shutdown::Both).unwrap();
        wait(false);
    }

    /// Test sunucusu: her bağlantı verilen yuvaya, verilen arka uçla.
    fn serve(hub: &Arc<Hub>, slot: usize, backend: Arc<dyn Backend>) -> std::net::SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let hub = hub.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let (hub, backend) = (hub.clone(), backend.clone());
                std::thread::spawn(move || handle_in(s.unwrap(), slot, "t", &hub, &*backend));
            }
        });
        addr
    }

    /// Hello + Welcome/Audio/Media okunmuş istemci.
    fn client(addr: std::net::SocketAddr) -> (BufWriter<TcpStream>, BufReader<TcpStream>) {
        let s = TcpStream::connect(addr).unwrap();
        let mut w = BufWriter::new(s.try_clone().unwrap());
        write_frame(
            &mut w,
            &C2S::Hello {
                proto: PROTO_VERSION,
                token: "t".into(),
                name: "x".into(),
            },
        )
        .unwrap();
        let mut rd = BufReader::new(s);
        let mut buf = [0u8; MAX_FRAME];
        for _ in 0..3 {
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap();
        }
        (w, rd)
    }

    /// ADR-0003: telefon (LAN) ve PC arayüzü (loopback) birbirini düşürmez; yuva içinde yeni eskiyi kapatır.
    #[test]
    fn slots_are_independent() {
        let hub = Arc::new(Hub::default());
        let (lan, local) = (
            serve(&hub, SLOT_LAN, Arc::new(Null)),
            serve(&hub, SLOT_LOCAL, Arc::new(Null)),
        );
        let (_wa, mut a) = client(lan);
        let (_wb, mut b) = client(local);
        hub.set_audio_level(10, false);
        let mut buf = [0u8; MAX_FRAME];
        for rd in [&mut a, &mut b] {
            assert!(matches!(
                read_frame::<_, S2C>(rd, &mut buf).unwrap(),
                S2C::Audio(_)
            ));
        }
        let (_wc, _c) = client(local); // loopback yuvasında yeni istemci
        assert!(matches!(
            read_frame::<_, S2C>(&mut b, &mut buf),
            Err(FrameError::Closed | FrameError::Io(_))
        ));
        hub.set_audio_level(20, false);
        assert!(
            matches!(
                read_frame::<_, S2C>(&mut a, &mut buf).unwrap(),
                S2C::Audio(_)
            ),
            "LAN istemcisi düşmemeli"
        );
    }

    /// Olay aboneliği: önce önbellekteki aktif liste, sonra olaylar; liste olaylarla güncellenir.
    #[test]
    fn events_subscription() {
        let hub = Arc::new(Hub::default());
        hub.set_alarms(vec!["cpu_high".into()]);
        let (mut w, mut rd) = client(serve(&hub, SLOT_LOCAL, Arc::new(Null)));
        let mut buf = [0u8; MAX_FRAME];
        write_frame(&mut w, &C2S::Subscribe(TOPIC_EVENTS)).unwrap();
        assert_eq!(
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap(),
            S2C::Alarms(vec!["cpu_high".into()])
        );
        // Alarms çerçevesi kilit altında, want ondan hemen sonra → kısa bekle
        assert!(
            (0..100).any(|_| hub.want.get() || {
                std::thread::sleep(Duration::from_millis(10));
                false
            }),
            "olay aboneliği pipe'ı açtırmalı"
        );
        hub.push_metrics(S2C::Metrics(None)); // metrik abonesi değil → gitmez
        let e = EventItem {
            ts_ms: 5,
            kind: 0,
            name: "ram".into(),
            value: 9_000,
        };
        hub.push_event(e.clone());
        assert_eq!(
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap(),
            S2C::Event(e)
        );
        hub.push_event(EventItem {
            kind: 1,
            name: "cpu_high".into(),
            ..Default::default()
        });
        assert_eq!(hub.lock().alarms, ["ram"]);
    }

    #[test]
    fn discovery_replies_only_to_query() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let b = Beacon {
            proto: PROTO_VERSION,
            host: "192.168.1.20".into(),
            port: 47810,
            name: "pc".into(),
        };
        let t = {
            let (stop, b) = (stop.clone(), b.clone());
            std::thread::spawn(move || discovery_loop(&sock, &b, &stop))
        };
        let c = UdpSocket::bind("127.0.0.1:0").unwrap();
        c.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut buf = [0u8; 256];
        c.send_to(b"nope", addr).unwrap(); // yanıtlanmaz
        c.send_to(&DISCOVER_REQ, addr).unwrap();
        let (n, _) = c.recv_from(&mut buf).unwrap();
        assert_eq!(decode_beacon(&buf[..n]), Some(b));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        c.send_to(&[0], addr).unwrap();
        t.join().unwrap();
    }

    #[test]
    fn firewall_rule() {
        let a = firewall_args(
            47810,
            r"C:\Users\a b\AppData\Local\LowDeck\lowdeck-agent.exe",
        );
        assert!(
            a.starts_with("/c netsh advfirewall firewall delete rule name=\"LowMonitor deck\"")
        );
        assert!(a.contains(" & netsh advfirewall firewall add rule name=\"LowMonitor deck\" dir=in action=allow protocol=TCP "));
        assert!(a.contains("localport=47810 profile=private remoteip=localsubnet"));
        assert!(a.contains(" & netsh advfirewall firewall add rule name=\"LowMonitor deck\" dir=in action=allow protocol=UDP localport=47811 "));
        assert_eq!(a.matches("program=").count(), 2);
        assert!(a.ends_with(r#"program="C:\Users\a b\AppData\Local\LowDeck\lowdeck-agent.exe""#));
    }

    struct History(usize);
    impl Backend for History {
        fn audio(&self, _: AudioCmd) {}
        fn media(&self, _: MediaCmd) {}
        fn events(&self, from_ms: u64, _: u64) -> Vec<EventItem> {
            (0..self.0)
                .map(|i| EventItem {
                    ts_ms: from_ms + i as u64,
                    ..Default::default()
                })
                .collect()
        }
        fn history(&self, _: u64, _: u64, n: u16) -> (u64, Vec<Option<HistPoint>>) {
            (1000, vec![Some(HistPoint::default()); usize::from(n)])
        }
    }

    /// QueryHistory: istek kırpılır (HIST_POINTS_MAX), HIST_PER_FRAME'lik parçalar + boş bitiş.
    #[test]
    fn query_history_chunks_and_clamps() {
        let hub = Arc::new(Hub::default());
        let (mut w, mut rd) = client(serve(&hub, SLOT_LOCAL, Arc::new(History(0))));
        let mut buf = [0u8; MAX_FRAME];
        write_frame(
            &mut w,
            &C2S::QueryHistory {
                from_ms: 7,
                to_ms: 9,
                points: u16::MAX,
            },
        )
        .unwrap();
        let mut n = 0;
        loop {
            match read_frame::<_, S2C>(&mut rd, &mut buf).unwrap() {
                S2C::History { points, .. } if points.is_empty() => break,
                S2C::History {
                    from_ms,
                    step_ms,
                    points,
                } => {
                    assert_eq!((from_ms, step_ms), (7, 1000));
                    assert!(points.len() <= HIST_PER_FRAME);
                    n += points.len();
                }
                m => panic!("beklenmeyen {m:?}"),
            }
        }
        assert_eq!(n, usize::from(HIST_POINTS_MAX));
    }

    struct Locks(std::sync::atomic::AtomicU32);
    impl Backend for Locks {
        fn audio(&self, _: AudioCmd) {}
        fn media(&self, _: MediaCmd) {}
        fn lock(&self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// C2S::Lock (ADR-0008) backend'e iner; yanıt yok, bağlantı açık kalır.
    #[test]
    fn lock_reaches_backend() {
        let hub = Arc::new(Hub::default());
        let b = Arc::new(Locks(Default::default()));
        let (mut w, mut rd) = client(serve(&hub, SLOT_LOCAL, b.clone()));
        let mut buf = [0u8; MAX_FRAME];
        write_frame(&mut w, &C2S::Lock).unwrap();
        write_frame(&mut w, &C2S::Ping(9)).unwrap();
        loop {
            if let S2C::Pong(9) = read_frame::<_, S2C>(&mut rd, &mut buf).unwrap() {
                break;
            }
        }
        assert_eq!(b.0.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    /// QueryEvents: EVENTS_PER_FRAME'lik parçalar + boş bitiş çerçevesi.
    #[test]
    fn query_events_chunks_and_terminates() {
        let hub = Arc::new(Hub::default());
        let (mut w, mut rd) = client(serve(
            &hub,
            SLOT_LOCAL,
            Arc::new(History(EVENTS_PER_FRAME * 2 + 3)),
        ));
        let mut buf = [0u8; MAX_FRAME];
        write_frame(
            &mut w,
            &C2S::QueryEvents {
                from_ms: 100,
                to_ms: 200,
            },
        )
        .unwrap();
        let mut sizes = Vec::new();
        loop {
            match read_frame::<_, S2C>(&mut rd, &mut buf).unwrap() {
                S2C::Events(v) if v.is_empty() => break,
                S2C::Events(v) => sizes.push(v.len()),
                m => panic!("beklenmeyen {m:?}"),
            }
        }
        assert_eq!(sizes, [EVENTS_PER_FRAME, EVENTS_PER_FRAME, 3]);
    }

    /// Hello zaman aşımı oturuma taşmamalı (Windows'ta try_clone'lu handle timeout'u paylaşmıyor).
    #[test]
    fn idle_session_survives_hello_timeout() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let hub = Hub::default();
            let _ = handle(l.accept().unwrap().0, "t", &hub, &Null);
        });
        let s = TcpStream::connect(addr).unwrap();
        let mut w = BufWriter::new(s.try_clone().unwrap());
        write_frame(
            &mut w,
            &C2S::Hello {
                proto: PROTO_VERSION,
                token: "t".into(),
                name: "x".into(),
            },
        )
        .unwrap();
        let mut rd = BufReader::new(s);
        let mut buf = [0u8; MAX_FRAME];
        for _ in 0..3 {
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap(); // Welcome, Audio, Media
        }
        std::thread::sleep(HELLO_TIMEOUT * 2);
        write_frame(&mut w, &C2S::Ping(7)).unwrap();
        assert_eq!(
            read_frame::<_, S2C>(&mut rd, &mut buf).unwrap(),
            S2C::Pong(7)
        );
    }

    /// Yeniden yükleme: stop() dinleyiciyi gerçekten kapatır, aynı port hemen tekrar açılır.
    #[test]
    fn reload_rebinds_same_port() {
        let hub = Arc::new(Hub::default());
        let backend: Arc<dyn Backend> = Arc::new(Null);
        let cfg = Config {
            bind: "127.0.0.1".into(),
            port: 0,
            token: "t".into(),
        };
        let r = listen(cfg, &hub, &backend).unwrap();
        let port = r.addrs[0].port();
        r.stop();
        let again = Config {
            bind: "127.0.0.1".into(),
            port,
            token: "t".into(),
        };
        listen(again, &hub, &backend)
            .expect("aynı port yeniden açılmalı")
            .stop();
    }

    /// Running::reload: yeni ayar açılamazsa eski ayarla dinlemeye döner.
    #[test]
    fn reload_falls_back_to_old() {
        let busy = TcpListener::bind("127.0.0.1:0").unwrap();
        let cfg = Config {
            bind: "127.0.0.1".into(),
            port: 0,
            token: "t".into(),
        };
        let mut r = Running {
            hub: Arc::new(Hub::default()),
            backend: Arc::new(Null),
            listeners: None,
        };
        r.reload(cfg).unwrap();
        let old = r.config().unwrap().clone();
        let bad = Config {
            port: busy.local_addr().unwrap().port(),
            ..old.clone()
        };
        assert!(r.reload(bad).unwrap_err().contains("Eski ayarla devam"));
        assert_eq!(r.config(), Some(&old));
        r.stop();
    }

    /// deck.toml yoksa agent.cfg'den taşınır (eski dosya kalır), sonra deck.toml okunur; ikisi de yoksa yeni.
    #[test]
    fn config_migrates_from_agent_cfg() {
        let dir = std::env::temp_dir().join(format!("lowmon-deck-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (deck, legacy) = (
            dir.join("LowMonitor").join("deck.toml"),
            dir.join("agent.cfg"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &legacy,
            "# yorum
bind=192.168.1.20
port=47812
token=abc
",
        )
        .unwrap();
        let want = Config {
            bind: "192.168.1.20".into(),
            port: 47812,
            token: "abc".into(),
        };
        assert_eq!(load_or_create(&deck, &legacy).unwrap(), want);
        assert!(legacy.exists(), "eski dosya silinmez");
        std::fs::write(
            &legacy,
            "bind=x
token=y
",
        )
        .unwrap(); // artık okunmaz
        assert_eq!(load_or_create(&deck, &legacy).unwrap(), want);
        std::fs::write(
            &deck,
            "bind = \"10.0.0.2\"
token = \"t\"
",
        )
        .unwrap();
        assert_eq!(load_or_create(&deck, &legacy).unwrap().port, DEFAULT_PORT);
        std::fs::write(
            &deck,
            "bind = \"\"
token = \"t\"
",
        )
        .unwrap();
        assert!(load_or_create(&deck, &legacy).is_err());
        let (fresh, none) = (dir.join("new.toml"), dir.join("yok.cfg"));
        let c = load_or_create(&fresh, &none).unwrap();
        assert_eq!((c.port, c.token.len()), (DEFAULT_PORT, 16));
        assert_eq!(load_or_create(&fresh, &none).unwrap(), c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn bind_error_explains_wrong_ip() {
        let cfg = Config {
            bind: "192.0.2.1".into(),
            port: 47810,
            token: "t".into(),
        };
        let e = std::io::Error::from_raw_os_error(10049); // WSAEADDRNOTAVAIL
        assert!(
            bind_error(&cfg, &e).contains("bu PC'nin adresi değil"),
            "{}",
            bind_error(&cfg, &e)
        );
    }

    #[test]
    fn stale_bind_falls_back_to_lan_ip() {
        let hub = Arc::new(Hub::default());
        let backend: Arc<dyn Backend> = Arc::new(Null);
        // TEST-NET adresi bu PC'de yok → AddrNotAvailable → lan_ip()
        let cfg = Config {
            bind: "192.0.2.1".into(),
            port: 0,
            token: "t".into(),
        };
        let r = listen(cfg, &hub, &backend).expect("lan_ip'ye düşmeli");
        assert_eq!(r.cfg.bind, lan_ip());
        r.stop();
    }

    #[test]
    fn bad_token() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let hub = Hub::default();
            let _ = handle(l.accept().unwrap().0, "t", &hub, &Null);
        });
        let s = TcpStream::connect(addr).unwrap();
        write_frame(
            &mut BufWriter::new(s.try_clone().unwrap()),
            &C2S::Hello {
                proto: PROTO_VERSION,
                token: "x".into(),
                name: "x".into(),
            },
        )
        .unwrap();
        let mut buf = [0u8; MAX_FRAME];
        assert_eq!(
            read_frame::<_, S2C>(&mut BufReader::new(s), &mut buf).unwrap(),
            S2C::Error(ErrorCode::BadToken)
        );
    }
}
