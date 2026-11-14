//! Periyodik uyanma + durdurma olayı (spek §4 "Zamanlama ve güç").
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, CreateEventW, CreateWaitableTimerExW, GetCurrentProcess,
        INFINITE, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetEvent, SetPriorityClass, SetProcessInformation,
        SetWaitableTimerEx, TIMER_ALL_ACCESS, WaitForMultipleObjects,
    },
};

/// İşletim sistemi birleştirme (coalescing) payı.
const TOLERABLE_DELAY_MS: u32 = 100;

pub struct Ticker {
    timer: HANDLE,
    stop: HANDLE,
}

/// Başka thread'den (servis kontrol handler'ı) durdurmak için.
#[derive(Clone, Copy)]
pub struct StopHandle(usize);

// SAFETY: Event handle'ı süreç genelinde geçerli; SetEvent thread-safe.
unsafe impl Send for StopHandle {}

impl StopHandle {
    pub fn signal(self) {
        // SAFETY: handle Ticker yaşadığı sürece açık; Ticker run() bitene kadar düşmez.
        unsafe { SetEvent(self.0 as HANDLE) };
    }
}

fn check(h: HANDLE) -> io::Result<HANDLE> {
    if h.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(h)
    }
}

impl Ticker {
    pub fn new(period_ms: u32) -> io::Result<Self> {
        // SAFETY: tüm pointer argümanları null (varsayılan güvenlik, isimsiz nesne).
        unsafe {
            // Flags 0 = auto-reset: her periyotta tek uyanma, el ile reset yok.
            let timer = check(CreateWaitableTimerExW(
                ptr::null(),
                ptr::null(),
                0,
                TIMER_ALL_ACCESS,
            ))?;
            let stop = check(CreateEventW(ptr::null(), 1, 0, ptr::null()))?;
            let due: i64 = -(period_ms as i64) * 10_000; // göreli, 100 ns birimi
            if SetWaitableTimerEx(
                timer,
                &due,
                period_ms as i32,
                None,
                ptr::null(),
                ptr::null(),
                TOLERABLE_DELAY_MS,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { timer, stop })
        }
    }

    pub fn stop_handle(&self) -> StopHandle {
        StopHandle(self.stop as usize)
    }

    /// `true` = tick, `false` = durdurma istendi.
    pub fn wait(&self) -> bool {
        let handles = [self.stop, self.timer];
        // SAFETY: handles dizisi çağrı boyunca canlı.
        let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        r == WAIT_OBJECT_0 + 1
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        // SAFETY: handle'lar new() içinde açıldı, bir kez kapanır.
        unsafe {
            CloseHandle(self.timer);
            CloseHandle(self.stop);
        }
    }
}

/// EcoQoS + BELOW_NORMAL; hata kritik değil (eski Windows'ta EcoQoS yok).
pub fn enter_eco_mode() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    // SAFETY: state çağrı boyunca canlı, boyutu doğru.
    unsafe {
        let p = GetCurrentProcess();
        SetProcessInformation(
            p,
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of_val(&state) as u32,
        );
        SetPriorityClass(p, BELOW_NORMAL_PRIORITY_CLASS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn ticks_then_stops() {
        let t = Ticker::new(50).unwrap();
        let start = Instant::now();
        assert!(t.wait());
        assert!(t.wait());
        assert!(start.elapsed() >= Duration::from_millis(90));
        t.stop_handle().signal();
        assert!(!t.wait());
    }
}
