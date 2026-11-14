//! Fiziksel disk r/w: IOCTL_DISK_PERFORMANCE (\\.\PhysicalDriveN, erişim hakkı 0 → admin gerekmez).
use lowmon_core::metrics::{DiskIo, per_sec};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::{
        IO::DeviceIoControl,
        Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE},
    },
};

const MAX_DRIVES: u32 = 32; // numaralar boşluklu olabilir, hepsi denenir

struct Drive {
    no: u32,
    handle: OwnedHandle,
    prev: Option<(u64, u64)>,
}

#[derive(Default)]
pub struct Disks {
    drives: Vec<Drive>,
}

impl Disks {
    /// Yeni takılan diskleri ekler; mevcutların delta durumu korunur.
    pub fn rediscover(&mut self) {
        for no in 0..MAX_DRIVES {
            if !self.drives.iter().any(|d| d.no == no)
                && let Some(handle) = open(no)
            {
                self.drives.push(Drive {
                    no,
                    handle,
                    prev: None,
                });
            }
        }
    }

    /// Okunamayan (çıkarılmış) disk listeden düşer; ilk okumada çıktı yok.
    pub fn read(&mut self, dt_ms: u64, out: &mut Vec<DiskIo>) {
        out.clear();
        self.drives.retain_mut(|d| {
            let Some(cur) = query(&d.handle) else {
                return false;
            };
            if let Some(p) = d.prev {
                out.push(DiskIo {
                    drive: d.no,
                    r_bps: per_sec(p.0, cur.0, dt_ms),
                    w_bps: per_sec(p.1, cur.1, dt_ms),
                });
            }
            d.prev = Some(cur);
            true
        });
    }
}

fn open(no: u32) -> Option<OwnedHandle> {
    let path: Vec<u16> = format!(r"\\.\PhysicalDrive{no}")
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: path NUL sonlu; erişim 0 = yalnızca IOCTL, okuma/yazma yok.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    // SAFETY: geçerli handle'ın tek sahibi OwnedHandle olur.
    (h != INVALID_HANDLE_VALUE).then(|| unsafe { OwnedHandle::from_raw_handle(h) })
}

fn query(h: &OwnedHandle) -> Option<(u64, u64)> {
    let mut p = DISK_PERFORMANCE::default();
    let mut ret = 0u32;
    // SAFETY: çıktı tamponu p, boyutu doğru; senkron çağrı.
    let ok = unsafe {
        DeviceIoControl(
            h.as_raw_handle(),
            IOCTL_DISK_PERFORMANCE,
            ptr::null(),
            0,
            (&raw mut p).cast(),
            size_of::<DISK_PERFORMANCE>() as u32,
            &mut ret,
            ptr::null_mut(),
        )
    };
    (ok != 0).then_some((p.BytesRead as u64, p.BytesWritten as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_disk_without_admin() {
        let mut d = Disks::default();
        d.rediscover();
        assert!(!d.drives.is_empty(), "PhysicalDrive bulunamadı");
        let mut out = Vec::new();
        d.read(1000, &mut out);
        assert!(out.is_empty()); // ilk okuma atılır
        d.read(1000, &mut out);
        assert_eq!(out.len(), d.drives.len());
    }
}
