//! Güvenlik istemi algılama (ADR-0037): ETW `Microsoft-Windows-Kernel-Process` gerçek zamanlı oturumu,
//! `consent.exe` (UAC) ve `PickerHost.exe` (güvenlik duvarı izni penceresi) başla/bitir. Polling yok: thread `ProcessTrace`'te bloklu, yalnız süreç başla/bitir olaylarında uyanır.
//! Config `[events] uac = false` iken hiç başlatılmaz.
use crate::security::wide;
use std::{
    mem::size_of,
    ptr,
    sync::mpsc::{Receiver, Sender, channel},
    thread::JoinHandle,
};
use windows_sys::{
    Win32::{Foundation::ERROR_ALREADY_EXISTS, System::Diagnostics::Etw::*},
    core::GUID,
};

/// {22FB2CD6-0E7B-422B-A0C7-2FAD1FD0E716}
const KERNEL_PROCESS: GUID = GUID::from_u128(0x22fb2cd6_0e7b_422b_a0c7_2fad1fd0e716);
const KEYWORD_PROCESS: u64 = 0x10;
const ID_START: u16 = 1;
const ID_STOP: u16 = 2;
const SESSION: &str = "LowMonitor-UAC";

/// Çalışan oturum; düşünce oturum durur, thread biter.
pub struct Uac {
    /// (alarm adı, açık mı): o türden ilk istem açılınca `true`, sonuncusu kapanınca `false`.
    pub rx: Receiver<(&'static str, bool)>,
    thread: Option<JoinHandle<()>>,
}

/// İzlenen görüntü adı (UTF-16 aranır, NUL sonlu) → alarm adı.
/// shortcut: PickerHost başka sistem seçicilerini de (Store dosya seçici) barındırır; ETW olayında komut satırı
/// (`FirewallNotificationDialogServer`) yok → nadiren yanlış `firewall` alarmı. Gerekirse tray komut satırını doğrular.
const WATCH: [(&[u8], &str); 2] = [
    (b"\\consent.exe\0", "uac"),
    (b"\\pickerhost.exe\0", "firewall"),
];

/// Callback durumu: açık istemlerin (PID, alarm adı).
struct State {
    tx: Sender<(&'static str, bool)>,
    open: Vec<(u32, &'static str)>,
}

/// `EVENT_TRACE_PROPERTIES` + oturum adı tek tamponda (u64: hizalama).
fn props() -> Vec<u64> {
    let name = wide(SESSION);
    let total = size_of::<EVENT_TRACE_PROPERTIES>() + name.len() * 2;
    let mut buf = vec![0u64; total.div_ceil(8)];
    let p = buf.as_mut_ptr().cast::<EVENT_TRACE_PROPERTIES>();
    // SAFETY: buf en az `total` bayt, u64 hizalı, sıfırlı (yapı için geçerli); ad yapının hemen arkasına sığar.
    unsafe {
        (*p).Wnode.BufferSize = total as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        (*p).Wnode.ClientContext = 1; // QPC
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).FlushTimer = 1; // s: gecikme ≤ ~1 s
        // Olay seyrek (yalnız consent/PickerHost başla-bitir): varsayılan çekirdek başına tampon
        // Private Bytes'ı ~2 MB şişiriyordu (soak 2026-11-25); 4 KB × 2–4 tampon yeter.
        (*p).BufferSize = 4; // KB
        (*p).MinimumBuffers = 2;
        (*p).MaximumBuffers = 4;
        (*p).LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    }
    buf
}

fn stop_session() {
    let mut p = props();
    let name = wide(SESSION);
    // SAFETY: p geçerli özellik tamponu; ad NUL sonlu. Oturum yoksa hata döner, yok sayılır.
    unsafe {
        ControlTraceW(
            CONTROLTRACE_HANDLE { Value: 0 },
            name.as_ptr(),
            p.as_mut_ptr().cast(),
            EVENT_TRACE_CONTROL_STOP,
        );
    }
}

/// Oturumu açar; hata = Win32 kodu (5: yetki yok).
pub fn start() -> Result<Uac, u32> {
    let name = wide(SESSION);
    let mut h = CONTROLTRACE_HANDLE { Value: 0 };
    let mut p = props();
    // SAFETY: h, p, name çağrı boyunca canlı.
    let mut r = unsafe { StartTraceW(&mut h, name.as_ptr(), p.as_mut_ptr().cast()) };
    if r == ERROR_ALREADY_EXISTS {
        stop_session(); // önceki çalıştırmadan (çökme) kalan oturum
        p = props();
        // SAFETY: aynı.
        r = unsafe { StartTraceW(&mut h, name.as_ptr(), p.as_mut_ptr().cast()) };
    }
    if r != 0 {
        return Err(r);
    }
    #[repr(C)]
    struct Ids {
        head: EVENT_FILTER_EVENT_ID,
        second: u16,
    }
    let ids = Ids {
        head: EVENT_FILTER_EVENT_ID {
            FilterIn: true,
            Reserved: 0,
            Count: 2,
            Events: [ID_START],
        },
        second: ID_STOP,
    };
    let mut filter = EVENT_FILTER_DESCRIPTOR {
        Ptr: (&raw const ids) as u64,
        Size: size_of::<Ids>() as u32,
        Type: EVENT_FILTER_TYPE_EVENT_ID,
    };
    let params = ENABLE_TRACE_PARAMETERS {
        Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
        EnableProperty: 0,
        ControlFlags: 0,
        SourceId: GUID::from_u128(0),
        EnableFilterDesc: &mut filter,
        FilterDescCount: 1,
    };
    // SAFETY: oturum açık; params/filter/ids çağrı boyunca canlı.
    let r = unsafe {
        EnableTraceEx2(
            h,
            &KERNEL_PROCESS,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER,
            TRACE_LEVEL_INFORMATION as u8,
            KEYWORD_PROCESS,
            0,
            0,
            &params,
        )
    };
    if r != 0 {
        stop_session();
        return Err(r);
    }
    let (tx, rx) = channel();
    let state = Box::into_raw(Box::new(State {
        tx,
        open: Vec::new(),
    })) as usize;
    let thread = std::thread::Builder::new()
        .name("uac".into())
        .spawn(move || consume(state))
        .map_err(|_| {
            stop_session();
            1u32
        })?;
    Ok(Uac {
        rx,
        thread: Some(thread),
    })
}

/// Oturum durana kadar bloklar; sonra durumu serbest bırakır.
fn consume(state: usize) {
    let mut name = wide(SESSION);
    // SAFETY: sıfırlı EVENT_TRACE_LOGFILEW geçerli (işaretçiler null, callback'ler None).
    let mut lf: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
    lf.LoggerName = name.as_mut_ptr();
    lf.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
    lf.Anonymous2.EventRecordCallback = Some(on_event);
    lf.Context = state as *mut _;
    // SAFETY: lf ve name ProcessTrace dönene kadar canlı; callback yalnız bu thread'de çağrılır.
    unsafe {
        let t = OpenTraceW(&mut lf);
        if t.Value != u64::MAX {
            ProcessTrace(&t, 1, ptr::null(), ptr::null());
            CloseTrace(t);
        }
        drop(Box::from_raw(state as *mut State));
    }
}

unsafe extern "system" fn on_event(e: *mut EVENT_RECORD) {
    // SAFETY: ETW geçerli kayıt verir; UserContext = consume'daki State (yalnız bu thread kullanır);
    // UserData UserDataLength bayt.
    let (st, id, data) = unsafe {
        let e = &*e;
        let data: &[u8] = if e.UserData.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(e.UserData.cast(), e.UserDataLength as usize)
        };
        (
            &mut *(e.UserContext as *mut State),
            e.EventHeader.EventDescriptor.Id,
            data,
        )
    };
    let Some(pid) = data
        .get(..4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    else {
        return;
    };
    let (name, opened) = match id {
        ID_START => match watched(data) {
            Some(n) => {
                st.open.push((pid, n));
                (n, true)
            }
            None => return,
        },
        ID_STOP => match st.open.iter().position(|o| o.0 == pid) {
            Some(i) => (st.open.remove(i).1, false),
            None => return,
        },
        _ => return,
    };
    // Yalnız kenar: o türden ilk açılış / son kapanış.
    if st.open.iter().filter(|o| o.1 == name).count() == usize::from(opened) {
        let _ = st.tx.send((name, opened));
    }
}

/// Başlangıç olayında izlenen görüntü adı varsa alarm adı.
fn watched(data: &[u8]) -> Option<&'static str> {
    WATCH.iter().find(|w| has_image(data, w.0)).map(|w| w.1)
}

/// Veride `...\<ad>.exe\0` var mı (UTF-16 LE, büyük/küçük harf duyarsız).
/// shortcut: TDH ile alan ayrıştırmak yerine veri içinde arar; olay sürümleri arası alan sırası değişse de çalışır.
fn has_image(data: &[u8], pat: &[u8]) -> bool {
    // Alan sırası bilinmediğinden iki hizalama da denenir (pratikte çift ofset).
    [data, data.get(1..).unwrap_or_default()]
        .into_iter()
        .any(|d| {
            let u: Vec<u16> = d
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            u.windows(pat.len()).any(|w| {
                w.iter()
                    .zip(pat)
                    .all(|(&c, &p)| c < 128 && (c as u8).to_ascii_lowercase() == p)
            })
        })
}

impl Drop for Uac {
    fn drop(&mut self) {
        stop_session();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str, offset: usize) -> Vec<u8> {
        let mut v = vec![0xAAu8; offset * 2]; // alan öncesi rastgele ikili veri yerine
        v.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        v
    }

    fn is_uac(d: &[u8]) -> bool {
        watched(d) == Some("uac")
    }

    #[test]
    fn detects_firewall_dialog() {
        let d = utf16(
            r"\Device\HarddiskVolume3\Windows\System32\PickerHost.exe",
            7,
        );
        assert_eq!(watched(&d), Some("firewall"));
        assert_eq!(watched(&utf16(r"\Device\X\notepad.exe", 1)), None);
    }

    #[test]
    fn detects_consent_only() {
        assert!(is_uac(&utf16(
            r"\Device\HarddiskVolume3\Windows\System32\consent.exe",
            20
        )));
        assert!(is_uac(&utf16(r"\DEVICE\X\WINDOWS\SYSTEM32\CONSENT.EXE", 3)));
        assert!(!is_uac(&utf16(r"\Device\X\myconsent.exe", 3)));
        assert!(!is_uac(&utf16(r"\Device\X\consent.exe.bak", 3)));
        assert!(!is_uac(&utf16(r"\Device\X\notepad.exe", 3)));
        assert!(!is_uac(&[]));
        let mut odd = vec![0u8];
        odd.extend(utf16(r"\Device\X\consent.exe", 2));
        assert!(is_uac(&odd), "tek ofset");
    }
}
