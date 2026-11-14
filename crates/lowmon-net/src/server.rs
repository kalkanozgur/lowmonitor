//! WS sunucusu (spek §16.1): tek thread, non-blocking soketler + `WSAPoll`; pipe okuyucu ayrı thread.
//! İstemci yokken pipe bağlantısı kapalıdır → svc tarafında abonelik ve serileştirme yok.
use crate::live::{live_json, parse_rate};
use lowmon_proto::proto::{self, CH_SAMPLE, CH_TOP, ClientMsg, PIPE_NAME, PROTO, Role, ServerMsg};
use std::{
    fs::File,
    io::{self, ErrorKind},
    net::{TcpListener, TcpStream},
    os::windows::io::{AsRawSocket, FromRawHandle},
    ptr,
    sync::{Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    Message, WebSocket,
    handshake::server::{ErrorResponse, Request, Response},
    http::StatusCode,
    protocol::WebSocketConfig,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
    Networking::WinSock::{POLLRDNORM, SOCKET, SOCKET_ERROR, WSAPOLLFD, WSAPoll},
    Storage::FileSystem::{CreateFileW, FILE_WRITE_DATA, OPEN_EXISTING},
};

/// Ping aralığı; 2 kayıp pong → bağlantı kapanır (spek §16.3).
const PING: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_secs(10);
/// Pipe'tan 1 Hz alınır, istemci başına `rate_ms / 1000`'de bir gönderilir.
const PIPE_RATE_MS: u32 = 1000;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

struct Client {
    ws: WebSocket<TcpStream>,
    every: u32,
    count: u32,
    missed: u8,
}

#[derive(Default)]
pub struct Shared {
    clients: Mutex<Vec<Client>>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Vec<Client>> {
        self.clients.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sırası gelen istemcilere gönderir; yazamayanı düşürür. Dönüş: istemci kaldı mı.
    pub fn broadcast(&self, text: &str) -> bool {
        let msg = Message::text(text);
        let mut g = self.lock();
        g.retain_mut(|c| {
            let due = c.count % c.every == 0;
            c.count = c.count.wrapping_add(1);
            !due || alive(c.ws.send(msg.clone()))
        });
        !g.is_empty()
    }
}

/// WouldBlock: mesaj tungstenite tamponunda kaldı (sınır `max_write_buffer_size`), bağlantı canlı.
fn alive<T>(r: tungstenite::Result<T>) -> bool {
    match r {
        Ok(_) => true,
        Err(tungstenite::Error::Io(e)) => e.kind() == ErrorKind::WouldBlock,
        Err(_) => false,
    }
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(4096)) // istemci yalnız {"rate_ms": …} yollar
        .max_frame_size(Some(4096))
        .write_buffer_size(0) // ~150 B mesaj: hemen yaz
        .max_write_buffer_size(64 * 1024) // okumayan istemci için tampon sınırı
}

fn not_found() -> ErrorResponse {
    let mut r = ErrorResponse::new(None);
    *r.status_mut() = StatusCode::NOT_FOUND;
    r
}

/// shortcut: el sıkışması 2 s'ye kadar bloklayarak yapılır (loopback, M6a); LAN'da yavaş istemci diğerlerini bekletebilir, M6b'de non-blocking handshake
fn accept_all(listener: &TcpListener, shared: &Shared, max: usize) {
    while let Ok((s, _)) = listener.accept() {
        if shared.lock().len() >= max {
            continue; // düşen soket kapanır
        }
        let ok = s.set_nonblocking(false).is_ok()
            && s.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).is_ok()
            && s.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).is_ok();
        if !ok {
            continue;
        }
        // İmza tungstenite'in Callback'i; ErrorResponse boyutu bizim elimizde değil
        #[allow(clippy::result_large_err)]
        let check = |req: &Request, resp: Response| {
            if req.uri().path() == "/v1/live" {
                Ok(resp)
            } else {
                Err(not_found())
            }
        };
        let Ok(ws) = tungstenite::accept_hdr_with_config(s, check, Some(ws_config())) else {
            continue;
        };
        if ws.get_ref().set_nonblocking(true).is_err() {
            continue;
        }
        shared.lock().push(Client {
            ws,
            every: 1,
            count: 0,
            missed: 0,
        });
        shared.wake.notify_one();
    }
}

/// Bekleyen tüm mesajları okur. `false`: kapandı/hata → istemci düşer.
fn read_client(c: &mut Client) -> bool {
    loop {
        match c.ws.read() {
            Ok(Message::Text(t)) => {
                if let Some(ms) = parse_rate(&t) {
                    c.every = ms / PIPE_RATE_MS;
                    c.count = 0;
                }
            }
            Ok(Message::Pong(_)) => c.missed = 0,
            Ok(Message::Close(_)) => return false,
            Ok(_) => {} // Ping'e tungstenite kendisi Pong kuyruklar
            Err(e) => return alive::<()>(Err(e)),
        }
    }
}

fn ping(c: &mut Client) -> bool {
    if c.missed >= 2 {
        return false;
    }
    c.missed += 1;
    alive(c.ws.send(Message::Ping(Default::default())))
}

fn poll_fd(fd: SOCKET) -> WSAPOLLFD {
    WSAPOLLFD {
        fd,
        events: POLLRDNORM,
        revents: 0,
    }
}

/// Kabul + okuma + ping döngüsü; boşta yalnız ping aralığında uyanır.
pub fn serve(listener: &TcpListener, shared: &Shared, max: usize) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let mut last_ping = Instant::now();
    loop {
        let mut fds = vec![poll_fd(listener.as_raw_socket() as SOCKET)];
        fds.extend(
            shared
                .lock()
                .iter()
                .map(|c| poll_fd(c.ws.get_ref().as_raw_socket() as SOCKET)),
        );
        let wait = PING.saturating_sub(last_ping.elapsed()).as_millis() as i32;
        // SAFETY: fds geçerli WSAPOLLFD dizisi, uzunluğu doğru; soketler bu çağrı boyunca açık.
        if unsafe { WSAPoll(fds.as_mut_ptr(), fds.len() as u32, wait) } == SOCKET_ERROR {
            return Err(io::Error::last_os_error());
        }
        if fds[0].revents != 0 {
            accept_all(listener, shared, max);
        }
        let ready: Vec<SOCKET> = fds[1..]
            .iter()
            .filter(|f| f.revents != 0)
            .map(|f| f.fd)
            .collect();
        if !ready.is_empty() {
            shared.lock().retain_mut(|c| {
                !ready.contains(&(c.ws.get_ref().as_raw_socket() as SOCKET)) || read_client(c)
            });
        }
        if last_ping.elapsed() >= PING {
            last_ping = Instant::now();
            shared.lock().retain_mut(ping);
        }
    }
}

fn connect_pipe() -> io::Result<File> {
    let name: Vec<u16> = PIPE_NAME.encode_utf16().chain([0]).collect();
    // GENERIC_WRITE değil: pipe DACL'ı IU'ya FILE_CREATE_PIPE_INSTANCE vermiyor (ADR-0018)
    // SAFETY: name NUL sonlu; dönen handle File'a devredilir.
    unsafe {
        let h = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | FILE_WRITE_DATA,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_handle(h))
    }
}

/// `Ok`: istemci kalmadı, pipe kapatılabilir. `Err`: servis yok / koptu.
fn session(f: &mut File, shared: &Shared, total_mb: u64) -> io::Result<()> {
    proto::write(
        f,
        &ClientMsg::Hello {
            proto: PROTO,
            role: Role::Viewer,
        },
    )?;
    proto::write(
        f,
        &ClientMsg::Subscribe {
            channels: CH_SAMPLE | CH_TOP,
            rate_ms: PIPE_RATE_MS,
        },
    )?;
    let mut top = Vec::new();
    loop {
        match proto::read::<ServerMsg>(f)? {
            ServerMsg::Sample(s) => {
                if !shared.broadcast(&live_json(&s, &top, total_mb)) {
                    return Ok(());
                }
            }
            ServerMsg::Top(t) => top = t,
            ServerMsg::Error { code } => return Err(io::Error::other(format!("{code:?}"))),
            _ => {}
        }
    }
}

/// İstemci varken pipe'a bağlı kalır; servis yoksa 10 s'de bir dener.
pub fn pipe_loop(shared: &Shared, total_mb: u64) {
    loop {
        {
            let mut g = shared.lock();
            while g.is_empty() {
                g = shared.wake.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        }
        let r = connect_pipe().and_then(|mut f| session(&mut f, shared, total_mb));
        if r.is_err() {
            thread::sleep(RETRY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn live_path_rate_and_broadcast() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let shared = Arc::new(Shared::default());
        let s2 = shared.clone();
        thread::spawn(move || serve(&l, &s2, 1));

        let tcp = TcpStream::connect(addr).unwrap();
        assert!(
            tungstenite::client(format!("ws://{addr}/yanlis"), tcp).is_err(),
            "yalnız /v1/live"
        );
        let tcp = TcpStream::connect(addr).unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://{addr}/v1/live"), tcp).unwrap();
        ws.send(Message::text(r#"{"rate_ms":2000}"#)).unwrap();
        // max_clients = 1: ikinci istemci el sıkışamaz
        let tcp = TcpStream::connect(addr).unwrap();
        assert!(tungstenite::client(format!("ws://{addr}/v1/live"), tcp).is_err());

        // Rate mesajı işlenene kadar bekle, sonra 4 yayından 2'si gelmeli (her 2. örnek)
        for _ in 0..100 {
            if shared.lock().first().is_some_and(|c| c.every == 2) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        for i in 0..4 {
            assert!(shared.broadcast(&i.to_string()));
        }
        let got: Vec<String> = (0..2)
            .map(|_| ws.read().unwrap().into_text().unwrap().to_string())
            .collect();
        assert_eq!(got, ["0", "2"]);

        drop(ws);
        for _ in 0..100 {
            if !shared.broadcast("x") {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("kopan istemci düşmedi");
    }
}
