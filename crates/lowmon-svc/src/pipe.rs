//! Pipe sunucusu (spek §6): istemci başına okuyucu + yazıcı thread, overlapped I/O.
//! Abone yoksa `publish` kilidi alıp hemen döner; mesaj serileştirilmez.
use crate::{export::export, query::query, security::wide};
use lowmon_core::{
    crypto::MasterKey,
    proto::{
        self, CH_EVENT, CH_SAMPLE, ClientMsg, Control, ErrorCode, PROTO, RangeItem, ServerMsg,
    },
};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL,
        INVALID_HANDLE_VALUE, LocalFree, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        CheckTokenMembership, CreateWellKnownSid, PSECURITY_DESCRIPTOR, RevertToSelf,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, WinBuiltinAdministratorsSid,
    },
    Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile,
        WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, ImpersonateNamedPipeClient, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::{CreateEventW, GetCurrentThread, INFINITE, OpenThreadToken},
    },
};

/// Spek §6.1'den farkı: IU'ya GENERIC_WRITE yerine FILE_WRITE_DATA (0x12019b = okuma + yazma,
/// FILE_CREATE_PIPE_INSTANCE yok) → kullanıcı aynı adla sahte sunucu instance'ı açamaz. OW = sunucu süreci.
const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)(A;;0x12019b;;;IU)";
/// Aynı anda en fazla bu kadar istemci (thread sayısı sınırı).
const MAX_CLIENTS: u32 = 8;
/// Hello gelmezse bağlantı düşer: boşta instance tutulamaz.
const HELLO_TIMEOUT_MS: u32 = 5_000;
/// Yazamayan (okumayan) istemci düşürülür.
const WRITE_TIMEOUT_MS: u32 = 2_000;
/// İstemci başına bekleyen çerçeve; dolarsa yeni çerçeve atlanır.
const QUEUE: usize = 8;
const STACK: usize = 64 * 1024;
/// Export/QueryRange çerçeve boyutu (spek §6.3 RangeChunk).
const CHUNK: usize = 64 * 1024;

/// Pipe sunucusunun paylaşılan durumu.
pub struct Server {
    pub publ: Publisher,
    /// Export: `logs/` kökü ve segment anahtarı.
    pub root: PathBuf,
    pub key: MasterKey,
    /// Writer/toplayıcı döngü thread'inde: komut oraya gider, sonuç cevap kanalıyla döner.
    pub cmd: SyncSender<CmdReq>,
}

/// Toplayıcı döngüsünün uyguladığı komutlar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Export öncesi yazıcı tamponu diske.
    Flush,
    Control(Control),
}
pub type CmdReq = (Cmd, SyncSender<Result<(), ErrorCode>>);
/// Döngü her tick'te bakar; daha uzun beklenirse cevapsız sayılır.
const CMD_WAIT: Duration = Duration::from_secs(3);

/// Kuyruk dolu ya da döngü cevapsızsa `None`.
fn ask(srv: &Server, cmd: Cmd) -> Option<Result<(), ErrorCode>> {
    let (tx, rx) = sync_channel(1);
    srv.cmd.try_send((cmd, tx)).ok()?;
    rx.recv_timeout(CMD_WAIT).ok()
}

type Frame = Arc<[u8]>;

struct Sub {
    id: u64,
    channels: u8,
    rate_ms: u32,
    every: u32,
    count: u32,
    tx: SyncSender<Frame>,
}

struct Subs {
    list: Vec<Sub>,
    /// Örnekleme aralığı; ReloadConfig değiştirebilir (ADR-0027) → `every` yeniden hesaplanır.
    interval_ms: u32,
    /// Raised kural id'leri; `CH_EVENT` abonesine ilk çerçeve.
    alarms: Vec<String>,
}

#[derive(Clone)]
pub struct Publisher(Arc<Mutex<Subs>>);

impl Default for Publisher {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Subs {
            list: Vec::new(),
            interval_ms: 1000,
            alarms: Vec::new(),
        })))
    }
}

/// Kaç örnekte bir gönderilir (en az 1).
fn every(rate_ms: u32, interval_ms: u32) -> u32 {
    (rate_ms / interval_ms.max(1)).max(1)
}

impl Publisher {
    fn lock(&self) -> std::sync::MutexGuard<'_, Subs> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_interval(&self, interval_ms: u32) {
        let mut g = self.lock();
        g.interval_ms = interval_ms;
        for s in &mut g.list {
            s.every = every(s.rate_ms, interval_ms);
            s.count = s.count.min(s.every - 1);
        }
    }

    pub fn set_alarms(&self, alarms: Vec<String>) {
        self.lock().alarms = alarms;
    }

    /// `msg` yalnızca en az bir abonenin sırası geldiyse çağrılır ve bir kez serileştirilir.
    pub fn publish(&self, ch: u8, msg: impl FnOnce() -> ServerMsg) {
        let mut g = self.lock();
        let subs = &mut g.list;
        if subs.is_empty() {
            return;
        }
        let (mut msg, mut frame) = (Some(msg), None::<Option<Frame>>);
        subs.retain_mut(|s| {
            if s.channels & ch == 0 {
                return true;
            }
            if ch == CH_SAMPLE {
                s.count += 1;
                if s.count < s.every {
                    return true;
                }
                s.count = 0;
            }
            let f = frame.get_or_insert_with(|| {
                let m = msg.take().map(|f| f());
                m.and_then(|m| proto::encode(&m).ok()).map(Frame::from)
            });
            match f {
                None => true,
                Some(f) => !matches!(s.tx.try_send(f.clone()), Err(TrySendError::Disconnected(_))),
            }
        });
    }

    fn subscribe(&self, id: u64, channels: u8, rate_ms: u32, tx: SyncSender<Frame>) {
        let mut g = self.lock();
        g.list.retain(|s| s.id != id);
        // Kilit altında: anlık liste ile sonraki `Event`'ler arasında boşluk/çakışma yok
        if channels & CH_EVENT != 0
            && let Ok(f) = proto::encode(&ServerMsg::Alarms(g.alarms.clone()))
        {
            let _ = tx.try_send(f.into());
        }
        // count = every-1 → ilk örnek hemen gider
        let every = every(rate_ms, g.interval_ms);
        g.list.push(Sub {
            id,
            channels,
            rate_ms,
            every,
            count: every - 1,
            tx,
        });
    }

    fn remove(&self, id: u64) {
        self.lock().list.retain(|s| s.id != id);
    }
}

struct Handle(HANDLE);
// SAFETY: kernel handle'ı thread'ler arası geçerli; eşzamanlı overlapped I/O ayrı OVERLAPPED'larla.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: handle bu nesneye ait, bir kez kapanır.
        unsafe { CloseHandle(self.0) };
    }
}

/// Bir thread'in overlapped I/O ucu: kendi event'i, paylaşılan pipe handle'ı.
struct Io {
    pipe: Arc<Handle>,
    ev: Handle,
    timeout: u32,
}

impl Io {
    fn new(pipe: Arc<Handle>) -> io::Result<Self> {
        // SAFETY: isimsiz, manual-reset event.
        let ev = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if ev.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Io {
            pipe,
            ev: Handle(ev),
            timeout: INFINITE,
        })
    }

    fn op(&mut self, buf: *mut u8, len: usize, write: bool) -> io::Result<usize> {
        let h = self.pipe.0;
        let len = len.min(u32::MAX as usize) as u32;
        // SAFETY: ov ve buf bu fonksiyon dönene kadar yaşar; zaman aşımında I/O iptal edilip tamamlanması beklenir.
        unsafe {
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = self.ev.0;
            let ok = if write {
                WriteFile(h, buf, len, ptr::null_mut(), &mut ov)
            } else {
                ReadFile(h, buf, len, ptr::null_mut(), &mut ov)
            };
            if ok == 0 && GetLastError() != ERROR_IO_PENDING {
                return Err(io::Error::last_os_error());
            }
            let mut n = 0u32;
            if GetOverlappedResultEx(h, &ov, &mut n, self.timeout, 0) == 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(WAIT_TIMEOUT as i32) {
                    CancelIoEx(h, &ov);
                    GetOverlappedResult(h, &ov, &mut n, 1);
                    return Err(io::ErrorKind::TimedOut.into());
                }
                return Err(err);
            }
            Ok(n as usize)
        }
    }
}

impl Read for Io {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.op(buf.as_mut_ptr(), buf.len(), false)
    }
}

impl Write for Io {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // WriteFile tamponu okumaz-yazmaz; *mut yalnızca imza ortaklığı için
        self.op(buf.as_ptr().cast_mut(), buf.len(), true)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn create(name: &[u16], first: bool) -> io::Result<Handle> {
    let sddl = wide(SDDL);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: sd LocalFree ile bırakılır; sa yalnızca CreateNamedPipeW süresince kullanılır.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let open = PIPE_ACCESS_DUPLEX
            | FILE_FLAG_OVERLAPPED
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        let h = CreateNamedPipeW(
            name.as_ptr(),
            open,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            MAX_CLIENTS,
            64 * 1024,
            4 * 1024,
            0,
            &sa,
        );
        LocalFree(sd as HLOCAL);
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Handle(h))
    }
}

fn connect(pipe: &Arc<Handle>) -> io::Result<()> {
    let io = Io::new(pipe.clone())?;
    // SAFETY: ov bekleme bitene kadar yaşar.
    unsafe {
        let mut ov: OVERLAPPED = std::mem::zeroed();
        ov.hEvent = io.ev.0;
        if ConnectNamedPipe(pipe.0, &mut ov) == 0 {
            match GetLastError() {
                ERROR_PIPE_CONNECTED => return Ok(()),
                ERROR_IO_PENDING => {}
                _ => return Err(io::Error::last_os_error()),
            }
            let mut n = 0;
            if GetOverlappedResult(pipe.0, &ov, &mut n, 1) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

/// İlk instance burada açılır: aynı adla başka sunucu varsa (servis + `run`) hata döner.
pub fn spawn(name: &str, srv: Arc<Server>) -> io::Result<()> {
    let name = wide(name);
    let first = create(&name, true)?;
    thread::Builder::new()
        .name("pipe-accept".into())
        .stack_size(STACK)
        .spawn(move || {
            let ids = AtomicU64::new(0);
            let mut next = Some(first);
            loop {
                let pipe = match next.take().map_or_else(|| create(&name, false), Ok) {
                    Ok(p) => Arc::new(p),
                    // MAX_CLIENTS dolu veya geçici hata
                    Err(_) => {
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };
                if connect(&pipe).is_err() {
                    continue;
                }
                let (srv, id) = (srv.clone(), ids.fetch_add(1, Ordering::Relaxed));
                let _ = thread::Builder::new()
                    .name("pipe-client".into())
                    .stack_size(STACK)
                    .spawn(move || client(pipe, &srv, id));
            }
        })?;
    Ok(())
}

/// İstemci yükseltilmiş Administrators üyesi mi (spek §6.1). LocalService'te SeImpersonate yok →
/// token Identification seviyesinde gelir; üyelik sorgusu için yeterli.
fn client_is_admin(pipe: HANDLE) -> bool {
    // SAFETY: impersonation bu fonksiyonda başlar ve biter; geri dönülemezse süreç durdurulur.
    unsafe {
        if ImpersonateNamedPipeClient(pipe) == 0 {
            return false;
        }
        let mut tok = ptr::null_mut();
        let opened = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut tok) != 0;
        if RevertToSelf() == 0 {
            // İstemci kimliğiyle devam etmek yetki karışıklığı olur
            std::process::abort();
        }
        if !opened {
            return false;
        }
        let mut sid = [0u8; 68];
        let mut n = sid.len() as u32;
        let mut member = 0;
        let admin = CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut n,
        ) != 0
            && CheckTokenMembership(tok, sid.as_mut_ptr().cast(), &mut member) != 0
            && member != 0;
        CloseHandle(tok);
        admin
    }
}

/// Export çıktısını 64 KB'lık RangeChunk'lara böler; kuyruk doluysa bekler (backpressure).
struct Chunks<'a> {
    buf: Vec<u8>,
    tx: &'a SyncSender<Frame>,
}

impl Chunks<'_> {
    fn send(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let f = proto::encode(&ServerMsg::RangeChunk(std::mem::take(&mut self.buf)))?;
        self.tx
            .send(f.into())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

impl Write for Chunks<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = b.len().min(CHUNK - self.buf.len());
        self.buf.extend_from_slice(&b[..n]);
        if self.buf.len() == CHUNK {
            self.send()?;
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.send()
    }
}

/// QueryRange öğelerini kodlanmış toplamı ≤ 64 KB olan `Range` çerçevelerinde toplar; kuyruk doluysa bekler.
struct Batch<'a> {
    items: Vec<RangeItem>,
    size: usize,
    tx: &'a SyncSender<Frame>,
}

impl Batch<'_> {
    fn push(&mut self, it: RangeItem) -> io::Result<()> {
        let n = proto::encode(&it)?.len() - 4;
        if self.size + n > CHUNK {
            self.flush()?;
        }
        self.size += n;
        self.items.push(it);
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.items.is_empty() {
            return Ok(());
        }
        self.size = 0;
        let f = proto::encode(&ServerMsg::Range(std::mem::take(&mut self.items)))?;
        self.tx
            .send(f.into())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// Akış bitişi: Ack ya da Error{Io}. try_send dolu kuyrukta düşürürdü, beklemeli gönderilir.
fn finish(tx: &SyncSender<Frame>, r: io::Result<()>) {
    let end = match r {
        Ok(()) => ServerMsg::Ack,
        Err(_) => ServerMsg::Error {
            code: ErrorCode::Io,
        },
    };
    if let Ok(f) = proto::encode(&end) {
        let _ = tx.send(f.into());
    }
}

fn client(pipe: Arc<Handle>, srv: &Server, id: u64) {
    let publ = &srv.publ;
    let Ok(mut rd) = Io::new(pipe.clone()) else {
        return;
    };
    rd.timeout = HELLO_TIMEOUT_MS;
    if !matches!(
        proto::read(&mut rd),
        Ok(ClientMsg::Hello { proto: PROTO, .. })
    ) {
        let _ = proto::write(
            &mut rd,
            &ServerMsg::Error {
                code: ErrorCode::BadHello,
            },
        );
        return;
    }
    rd.timeout = INFINITE;
    let (tx, rx) = sync_channel::<Frame>(QUEUE);
    let Ok(mut wr) = Io::new(pipe.clone()) else {
        return;
    };
    wr.timeout = WRITE_TIMEOUT_MS;
    let writer = thread::Builder::new()
        .name("pipe-write".into())
        .stack_size(STACK)
        .spawn(move || {
            for f in rx {
                if wr.write_all(&f).is_err() {
                    // Okuyucunun bekleyen ReadFile'ı da düşsün → istemci kapanır
                    // SAFETY: handle wr.pipe ile canlı.
                    unsafe { CancelIoEx(wr.pipe.0, ptr::null()) };
                    break;
                }
            }
        });
    if writer.is_err() {
        return;
    }
    let reply = |m: &ServerMsg| {
        if let Ok(f) = proto::encode(m) {
            let _ = tx.try_send(f.into());
        }
    };
    reply(&ServerMsg::Ack);
    while let Ok(msg) = proto::read::<ClientMsg>(&mut rd) {
        match msg {
            ClientMsg::Subscribe { channels, rate_ms } => {
                publ.subscribe(id, channels, rate_ms, tx.clone());
                reply(&ServerMsg::Ack);
            }
            ClientMsg::Unsubscribe => {
                publ.remove(id);
                reply(&ServerMsg::Ack);
            }
            ClientMsg::Hello { .. } => reply(&ServerMsg::Error {
                code: ErrorCode::Unsupported,
            }),
            ClientMsg::Export { .. } | ClientMsg::Control(_) if !client_is_admin(pipe.0) => {
                reply(&ServerMsg::Error {
                    code: ErrorCode::Forbidden,
                })
            }
            ClientMsg::Control(c) => reply(&match ask(srv, Cmd::Control(c)) {
                Some(Ok(())) => ServerMsg::Ack,
                Some(Err(code)) => ServerMsg::Error { code },
                None => ServerMsg::Error {
                    code: ErrorCode::Io,
                },
            }),
            ClientMsg::Export { from_ms, to_ms } => {
                // Cevapsızsa Export son ≤ 60 s'siz devam eder
                let _ = ask(srv, Cmd::Flush);
                let mut out = Chunks {
                    buf: Vec::with_capacity(CHUNK),
                    tx: &tx,
                };
                let r =
                    export(&srv.root, &srv.key, from_ms, to_ms, &mut out).and_then(|_| out.flush());
                finish(&tx, r);
            }
            ClientMsg::QueryRange {
                from_ms,
                to_ms,
                resolution,
            } => {
                let mut b = Batch {
                    items: Vec::new(),
                    size: 0,
                    tx: &tx,
                };
                let r = query(&srv.root, &srv.key, from_ms, to_ms, resolution, &mut |it| {
                    b.push(it)
                });
                finish(&tx, r.and_then(|_| b.flush()));
            }
        }
    }
    // Abonelik + tx düşünce yazıcı thread biter; son Arc ile handle kapanır
    publ.remove(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lowmon_core::{
        metrics::{Sample, TopEntry},
        proto::{CH_TOP, Resolution, Role},
    };
    use std::{fs::File, os::windows::io::FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{CreateFileW, OPEN_EXISTING},
    };

    fn is_elevated() -> bool {
        std::process::Command::new("net")
            .arg("session")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn open(name: &str) -> File {
        let n = wide(name);
        // SAFETY: n NUL sonlu; dönen handle File'a devredilir.
        unsafe {
            let h = CreateFileW(
                n.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            );
            assert_ne!(h, INVALID_HANDLE_VALUE, "{}", io::Error::last_os_error());
            File::from_raw_handle(h)
        }
    }

    #[test]
    fn set_interval_rescales_existing_subscribers() {
        let publ = Publisher::default(); // 1000 ms
        let (tx, rx) = sync_channel(16);
        publ.subscribe(1, CH_SAMPLE, 4000, tx); // her 4. örnek
        publ.set_interval(2000); // artık her 2. örnek
        let mut sent = 0;
        for _ in 0..6 {
            publ.publish(CH_SAMPLE, || ServerMsg::Ack);
        }
        while rx.try_recv().is_ok() {
            sent += 1;
        }
        assert_eq!(sent, 3);
        assert_eq!(every(500, 1000), 1); // aralıktan hızlı istek → her örnek
    }

    #[test]
    fn event_subscriber_gets_alarms_snapshot_first() {
        let publ = Publisher::default();
        publ.set_alarms(vec!["cpu_high".into()]);
        let (tx, rx) = sync_channel(16);
        publ.subscribe(1, CH_EVENT, 1000, tx);
        let (tx2, rx2) = sync_channel(16);
        publ.subscribe(2, CH_SAMPLE, 1000, tx2);
        publ.publish(CH_EVENT, || ServerMsg::Ack);
        let got: Vec<ServerMsg> = rx
            .try_iter()
            .map(|f| proto::read(&mut &f[..]).unwrap())
            .collect();
        assert_eq!(
            got,
            [ServerMsg::Alarms(vec!["cpu_high".into()]), ServerMsg::Ack]
        );
        assert!(rx2.try_recv().is_err(), "Sample abonesi olay almamalı");
    }

    #[test]
    fn subscribe_receive_and_reject_bad_hello() {
        let name = format!(r"\\.\pipe\LowMonitor-test-{}", std::process::id());
        let publ = Publisher::default();
        let srv = |publ| {
            Arc::new(Server {
                publ,
                root: std::env::temp_dir(),
                key: [0; 32],
                cmd: sync_channel(1).0,
            })
        };
        spawn(&name, srv(publ.clone())).unwrap();
        assert!(
            spawn(&name, srv(Publisher::default())).is_err(),
            "ikinci sunucu açılmamalı"
        );

        let mut bad = open(&name);
        proto::write(&mut bad, &ClientMsg::Unsubscribe).unwrap();
        assert!(matches!(
            proto::read(&mut bad),
            Ok(ServerMsg::Error {
                code: ErrorCode::BadHello
            })
        ));

        let mut c = open(&name);
        proto::write(
            &mut c,
            &ClientMsg::Hello {
                proto: PROTO,
                role: Role::Tray,
            },
        )
        .unwrap();
        proto::write(
            &mut c,
            &ClientMsg::Subscribe {
                channels: CH_SAMPLE,
                rate_ms: 2000,
            },
        )
        .unwrap();
        assert_eq!(proto::read::<ServerMsg>(&mut c).unwrap(), ServerMsg::Ack);
        assert_eq!(proto::read::<ServerMsg>(&mut c).unwrap(), ServerMsg::Ack);
        // Test süreci yükseltilmemiş varsayılır (pre-push normal kabukta)
        if !is_elevated() {
            proto::write(
                &mut c,
                &ClientMsg::Export {
                    from_ms: 0,
                    to_ms: 1,
                },
            )
            .unwrap();
            assert_eq!(
                proto::read::<ServerMsg>(&mut c).unwrap(),
                ServerMsg::Error {
                    code: ErrorCode::Forbidden
                }
            );
            // Yetkisiz Control (spek §12): döngüye hiç ulaşmaz
            proto::write(&mut c, &ClientMsg::Control(Control::Pause)).unwrap();
            assert_eq!(
                proto::read::<ServerMsg>(&mut c).unwrap(),
                ServerMsg::Error {
                    code: ErrorCode::Forbidden
                }
            );
        }
        // QueryRange yetki istemez; boş aralık → doğrudan Ack
        proto::write(
            &mut c,
            &ClientMsg::QueryRange {
                from_ms: 1,
                to_ms: 1,
                resolution: Resolution::Raw,
            },
        )
        .unwrap();
        assert_eq!(proto::read::<ServerMsg>(&mut c).unwrap(), ServerMsg::Ack);
        // rate 2000/1000 → her 2. örnek; ilk örnek hemen; Top kanalına abone değil
        let mut called = 0;
        for ts in 1..=4 {
            publ.publish(CH_SAMPLE, || {
                called += 1;
                ServerMsg::Sample(Sample {
                    ts_unix_ms: ts,
                    ..Default::default()
                })
            });
            publ.publish(CH_TOP, || ServerMsg::Top(vec![TopEntry::default()]));
        }
        assert_eq!(called, 2, "sırası gelmeyen örnek serileştirilmemeli");
        for ts in [1, 3] {
            match proto::read::<ServerMsg>(&mut c).unwrap() {
                ServerMsg::Sample(s) => assert_eq!(s.ts_unix_ms, ts),
                m => panic!("beklenmeyen {m:?}"),
            }
        }
        drop(c);
        // Kopan istemci aboneliği düşürür → publish hiç serileştirmez
        for _ in 0..50 {
            if publ.lock().list.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        publ.publish(CH_SAMPLE, || panic!("abone yokken serileştirildi"));
    }
}
