//! WS sunucusu (spek §16.1–16.2): tek ağ thread'i, non-blocking soketler + `WSAPoll`; pipe okuyucu ayrı thread.
//! El sıkışma da non-blocking (ADR-0024): yavaş/sessiz bağlantı diğerlerini bekletmez, 2 s'de düşer.
//! İstemci yokken pipe bağlantısı kapalıdır → svc tarafında abonelik ve serileştirme yok.
use crate::{
    auth::{Guard, Hash, Store},
    http::{self, MAX_REQ, Parse},
    live::{live_json, parse_rate},
};
use lowmon_proto::proto::{self, CH_SAMPLE, CH_TOP, ClientMsg, PIPE_NAME, PROTO, Role, ServerMsg};
use std::{
    fs::File,
    io::{self, ErrorKind, Read, Write},
    net::{IpAddr, TcpListener, TcpStream},
    os::windows::io::{AsRawSocket, FromRawHandle},
    ptr,
    sync::{Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
    Networking::WinSock::{POLLRDNORM, SOCKET, SOCKET_ERROR, WSAPOLLFD, WSAPoll},
    Storage::FileSystem::{CreateFileW, FILE_WRITE_DATA, OPEN_EXISTING},
};

/// Ping aralığı; 2 kayıp pong → bağlantı kapanır (spek §16.3). Token iptali de bu aralıkta uygulanır.
const PING: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_secs(10);
/// Pipe'tan 1 Hz alınır, istemci başına `rate_ms / 1000`'de bir gönderilir.
const PIPE_RATE_MS: u32 = 1000;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
/// Aynı anda el sıkışmada bekleyen bağlantı sınırı.
const MAX_PENDING: usize = 8;

/// Ağ ayarı + token deposu (ağ thread'ine özel).
pub struct Ctx {
    pub store: Store,
    pub origins: Vec<String>,
    pub max_clients: usize,
}

struct Client {
    ws: WebSocket<TcpStream>,
    /// Bağlanırken kullanılan token'ın hash'i; `unpair` sonrası düşürmek için.
    token: Hash,
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

    /// Token'ı artık kayıtlı olmayan istemcileri düşürür (`unpair --all`).
    fn revalidate(&self, valid: &[Hash]) {
        self.lock().retain(|c| valid.contains(&c.token));
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
        .write_buffer_size(0) // ~350 B mesaj: hemen yaz
        .max_write_buffer_size(64 * 1024) // okumayan istemci için tampon sınırı
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

struct Pending {
    s: TcpStream,
    ip: IpAddr,
    buf: Vec<u8>,
    since: Instant,
}

enum Step {
    Wait,
    Close,
    /// 101 yazıldı; `rest` = başlıktan sonra gelmiş baytlar.
    Upgrade {
        token: Hash,
        rest: Vec<u8>,
    },
}

/// Kısa yanıt; yazılamazsa bağlantı zaten kapanıyor.
fn send(s: &mut TcpStream, b: &[u8]) -> Step {
    let _ = s.write_all(b);
    Step::Close
}

/// Bekleyen isteğe gelen baytları okur ve tamamlanmışsa yanıtlar.
fn step(p: &mut Pending, ctx: &Ctx, guard: &mut Guard, clients: usize, now: Instant) -> Step {
    let mut tmp = [0u8; 1024];
    loop {
        match p.s.read(&mut tmp) {
            Ok(0) => return Step::Close,
            Ok(n) if p.buf.len() + n <= MAX_REQ => p.buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            _ => return Step::Close, // hata ya da çok büyük istek
        }
    }
    let r = match http::parse(&p.buf) {
        Parse::Partial => return Step::Wait,
        Parse::Bad => return send(&mut p.s, &http::reply("400 Bad Request", "")),
        Parse::Done(r) if p.buf.len() < r.head_len + r.body_len => return Step::Wait,
        Parse::Done(r) => r,
    };
    // Origin yoksa (yerel uygulama) yalnız token; varsa izin listesinde olmalı (ADR-0024)
    if r.origin.as_ref().is_some_and(|o| !ctx.origins.contains(o)) {
        return send(&mut p.s, &http::reply("403 Forbidden", ""));
    }
    match (r.method.as_str(), r.path.as_str(), &r.ws_key) {
        ("GET", "/v1/live", Some(key)) => {
            let Some(token) = r.token.as_deref().and_then(|t| ctx.store.check(t)) else {
                guard.fail(p.ip, now);
                return send(&mut p.s, &http::reply("401 Unauthorized", ""));
            };
            guard.ok(p.ip);
            if clients >= ctx.max_clients {
                return send(&mut p.s, &http::reply("503 Service Unavailable", ""));
            }
            if p.s.write_all(&http::upgrade(key)).is_err() {
                return Step::Close;
            }
            Step::Upgrade {
                token,
                rest: p.buf[r.head_len..].to_vec(),
            }
        }
        ("POST", "/v1/pair", _) => {
            let body = &p.buf[r.head_len..r.head_len + r.body_len];
            match http::pair_code(body).map(|c| ctx.store.redeem(&c, now_ms())) {
                Some(Ok(Some(tok))) => {
                    guard.ok(p.ip);
                    send(
                        &mut p.s,
                        &http::reply("200 OK", &format!(r#"{{"token":"{tok}"}}"#)),
                    )
                }
                Some(Err(_)) => send(&mut p.s, &http::reply("500 Internal Server Error", "")),
                _ => {
                    guard.fail(p.ip, now);
                    send(&mut p.s, &http::reply("401 Unauthorized", ""))
                }
            }
        }
        _ => send(&mut p.s, &http::reply("404 Not Found", "")),
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

fn sock(s: &TcpStream) -> SOCKET {
    s.as_raw_socket() as SOCKET
}

/// Kabul + el sıkışma + okuma + ping döngüsü; boşta yalnız ping aralığında uyanır.
pub fn serve(listener: &TcpListener, shared: &Shared, ctx: &Ctx) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let (mut guard, mut pending) = (Guard::default(), Vec::<Pending>::new());
    let mut last_ping = Instant::now();
    loop {
        let mut fds = vec![poll_fd(sock_l(listener))];
        fds.extend(pending.iter().map(|p| poll_fd(sock(&p.s))));
        fds.extend(shared.lock().iter().map(|c| poll_fd(sock(c.ws.get_ref()))));
        let mut wait = PING.saturating_sub(last_ping.elapsed());
        if let Some(p) = pending.iter().map(|p| p.since).min() {
            wait = wait.min(HANDSHAKE_TIMEOUT.saturating_sub(p.elapsed()));
        }
        // SAFETY: fds geçerli WSAPOLLFD dizisi, uzunluğu doğru; soketler bu çağrı boyunca açık.
        if unsafe { WSAPoll(fds.as_mut_ptr(), fds.len() as u32, wait.as_millis() as i32) }
            == SOCKET_ERROR
        {
            return Err(io::Error::last_os_error());
        }
        let now = Instant::now();
        let ready: Vec<SOCKET> = fds[1..]
            .iter()
            .filter(|f| f.revents != 0)
            .map(|f| f.fd)
            .collect();

        for mut p in std::mem::take(&mut pending) {
            if !ready.contains(&sock(&p.s)) {
                if now.duration_since(p.since) < HANDSHAKE_TIMEOUT {
                    pending.push(p);
                }
                continue;
            }
            let n = shared.lock().len();
            match step(&mut p, ctx, &mut guard, n, now) {
                Step::Wait if now.duration_since(p.since) < HANDSHAKE_TIMEOUT => pending.push(p),
                Step::Wait | Step::Close => {}
                Step::Upgrade { token, rest } => {
                    let ws = WebSocket::from_partially_read(
                        p.s,
                        rest,
                        tungstenite::protocol::Role::Server,
                        Some(ws_config()),
                    );
                    shared.lock().push(Client {
                        ws,
                        token,
                        every: 1,
                        count: 0,
                        missed: 0,
                    });
                    shared.wake.notify_one();
                }
            }
        }
        shared
            .lock()
            .retain_mut(|c| !ready.contains(&sock(c.ws.get_ref())) || read_client(c));

        if fds[0].revents != 0 {
            while let Ok((s, addr)) = listener.accept() {
                // Kısıtlı/banlı IP ve dolu kuyruk: soket hemen kapanır
                if guard.allowed(addr.ip(), now)
                    && pending.len() < MAX_PENDING
                    && s.set_nonblocking(true).is_ok()
                {
                    pending.push(Pending {
                        s,
                        ip: addr.ip(),
                        buf: Vec::new(),
                        since: now,
                    });
                }
            }
        }
        if last_ping.elapsed() >= PING {
            last_ping = Instant::now();
            shared.revalidate(&ctx.store.hashes().unwrap_or_default());
            shared.lock().retain_mut(ping);
        }
    }
}

fn sock_l(l: &TcpListener) -> SOCKET {
    l.as_raw_socket() as SOCKET
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
    use crate::auth::temp_store;
    use std::sync::Arc;
    use tungstenite::client::IntoClientRequest;

    fn raw(addr: std::net::SocketAddr, req: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn pair_req(code: &str) -> String {
        let body = format!(r#"{{"code":"{code}"}}"#);
        format!(
            "POST /v1/pair HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    const WS_HEAD: &str = "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";

    #[test]
    fn auth_pair_origin_limits_and_revoke() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let store_dir = temp_store("server").dir;
        let shared = Arc::new(Shared::default());
        let s2 = shared.clone();
        let dir2 = store_dir.clone();
        thread::spawn(move || {
            let ctx = Ctx {
                store: Store { dir: dir2 },
                origins: vec!["http://ok".into()],
                max_clients: 1,
            };
            serve(&l, &s2, &ctx)
        });
        let store = Store { dir: store_dir };

        assert!(raw(addr, "GET /yok HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        let r = raw(addr, &format!("GET /v1/live HTTP/1.1\r\n{WS_HEAD}\r\n"));
        assert!(r.starts_with("HTTP/1.1 401"), "token yok: {r}");
        assert_eq!(
            raw(addr, &pair_req("000000")),
            "",
            "hatalı denemeden sonra 1 s içinde bağlantı yanıtsız kapanır"
        );
        thread::sleep(Duration::from_millis(1100));

        let code = store.start_pair(now_ms()).unwrap();
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert!(raw(addr, &pair_req(wrong)).starts_with("HTTP/1.1 401"));
        thread::sleep(Duration::from_millis(1100));
        let r = raw(addr, &pair_req(&code));
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
        let token = r.split(r#""token":""#).nth(1).unwrap()[..64].to_owned();

        let mut bad = format!("ws://{addr}/v1/live?token={token}")
            .into_client_request()
            .unwrap();
        bad.headers_mut()
            .insert("Origin", "http://kotu".parse().unwrap());
        let tcp = TcpStream::connect(addr).unwrap();
        assert!(tungstenite::client(bad, tcp).is_err(), "izinsiz Origin");

        let mut req = format!("ws://{addr}/v1/live")
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        req.headers_mut()
            .insert("Origin", "http://ok".parse().unwrap());
        let tcp = TcpStream::connect(addr).unwrap();
        let (mut ws, _) = tungstenite::client(req, tcp).unwrap();

        // max_clients = 1 → 503
        let r = raw(
            addr,
            &format!("GET /v1/live?token={token} HTTP/1.1\r\n{WS_HEAD}\r\n"),
        );
        assert!(r.starts_with("HTTP/1.1 503"), "{r}");

        ws.send(Message::text(r#"{"rate_ms":2000}"#)).unwrap();
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

        // unpair --all → ping turunda düşer
        store.clear().unwrap();
        shared.revalidate(&store.hashes().unwrap());
        assert!(!shared.broadcast("x"), "iptal edilen token düşmeli");
        let _ = std::fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn silent_connection_times_out() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        thread::spawn(move || {
            let ctx = Ctx {
                store: temp_store("silent"),
                origins: vec![],
                max_clients: 1,
            };
            serve(&l, &Shared::default(), &ctx)
        });
        let t = Instant::now();
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Sessiz bağlantı beklerken başka istek hemen yanıtlanır
        assert!(raw(addr, "GET /yok HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        assert!(t.elapsed() < Duration::from_millis(500));
        let mut b = [0u8; 1];
        assert_eq!(s.read(&mut b).unwrap_or(0), 0, "sunucu kapatmalı");
        let el = t.elapsed();
        assert!(
            el >= Duration::from_millis(1500) && el < Duration::from_secs(4),
            "{el:?}"
        );
    }
}
