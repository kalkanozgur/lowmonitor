//! Pipe sunucusu (spek §6): istemci başına okuyucu + yazıcı thread, overlapped I/O.
//! Abone yoksa `publish` kilidi alıp hemen döner; mesaj serileştirilmez.
use crate::security::wide;
use lowmon_core::proto::{self, CH_SAMPLE, ClientMsg, ErrorCode, PROTO, ServerMsg};
use std::{
    io::{self, Read, Write},
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
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile,
        WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::{CreateEventW, INFINITE},
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

type Frame = Arc<[u8]>;

struct Sub {
    id: u64,
    channels: u8,
    every: u32,
    count: u32,
    tx: SyncSender<Frame>,
}

#[derive(Clone, Default)]
pub struct Publisher(Arc<Mutex<Vec<Sub>>>);

impl Publisher {
    /// `msg` yalnızca en az bir abonenin sırası geldiyse çağrılır ve bir kez serileştirilir.
    pub fn publish(&self, ch: u8, msg: impl FnOnce() -> ServerMsg) {
        let mut subs = self.0.lock().unwrap_or_else(|e| e.into_inner());
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

    fn subscribe(&self, id: u64, channels: u8, every: u32, tx: SyncSender<Frame>) {
        let mut subs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| s.id != id);
        // count = every-1 → ilk örnek hemen gider
        let every = every.max(1);
        subs.push(Sub {
            id,
            channels,
            every,
            count: every - 1,
            tx,
        });
    }

    fn remove(&self, id: u64) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|s| s.id != id);
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
pub fn spawn(name: &str, publ: Publisher, interval_ms: u32) -> io::Result<()> {
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
                let (publ, id) = (publ.clone(), ids.fetch_add(1, Ordering::Relaxed));
                let _ = thread::Builder::new()
                    .name("pipe-client".into())
                    .stack_size(STACK)
                    .spawn(move || client(pipe, publ, id, interval_ms));
            }
        })?;
    Ok(())
}

fn client(pipe: Arc<Handle>, publ: Publisher, id: u64, interval_ms: u32) {
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
                publ.subscribe(id, channels, rate_ms / interval_ms.max(1), tx.clone());
                reply(&ServerMsg::Ack);
            }
            ClientMsg::Unsubscribe => {
                publ.remove(id);
                reply(&ServerMsg::Ack);
            }
            ClientMsg::Hello { .. } => reply(&ServerMsg::Error {
                code: ErrorCode::Unsupported,
            }),
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
        proto::{CH_TOP, Role},
    };
    use std::{fs::File, os::windows::io::FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{CreateFileW, OPEN_EXISTING},
    };

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
    fn subscribe_receive_and_reject_bad_hello() {
        let name = format!(r"\\.\pipe\LowMonitor-test-{}", std::process::id());
        let publ = Publisher::default();
        spawn(&name, publ.clone(), 1000).unwrap();
        assert!(
            spawn(&name, Publisher::default(), 1000).is_err(),
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
            if publ.0.lock().unwrap().is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        publ.publish(CH_SAMPLE, || panic!("abone yokken serileştirildi"));
    }
}
