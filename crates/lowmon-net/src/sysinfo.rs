//! `/v1/info` için okunur adlar: disk modeli ve ağ arayüzü takma adı. Admin gerekmez.
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2},
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::{
        IO::DeviceIoControl,
        Ioctl::{
            IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
            STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
        },
    },
};

/// Tampondaki `off` konumundan NUL'a kadar ASCII (0 = alan yok).
fn ascii_at(buf: &[u8], off: u32) -> &str {
    let tail = buf
        .get(off as usize..)
        .filter(|_| off != 0)
        .unwrap_or_default();
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    std::str::from_utf8(&tail[..end]).unwrap_or("").trim()
}

/// `\\.\PhysicalDriveN` → "Vendor Model"; okunamazsa `PhysicalDriveN`.
pub fn disk_name(no: u32) -> String {
    let fallback = format!("PhysicalDrive{no}");
    let path: Vec<u16> = format!(r"\\.\{fallback}")
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
    if h == INVALID_HANDLE_VALUE {
        return fallback;
    }
    // SAFETY: geçerli handle'ın tek sahibi OwnedHandle olur.
    let h = unsafe { OwnedHandle::from_raw_handle(h) };
    let q = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buf = [0u8; 1024];
    let mut ret = 0u32;
    // SAFETY: girdi/çıktı tamponları ve boyutları doğru; senkron çağrı.
    let ok = unsafe {
        DeviceIoControl(
            h.as_raw_handle(),
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&raw const q).cast(),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut ret,
            ptr::null_mut(),
        )
    };
    if ok == 0 || (ret as usize) < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return fallback;
    }
    let buf = &buf[..ret as usize];
    // SAFETY: tampon en az descriptor kadar; hizasız okuma.
    let d: STORAGE_DEVICE_DESCRIPTOR = unsafe { ptr::read_unaligned(buf.as_ptr().cast()) };
    let name = format!(
        "{} {}",
        ascii_at(buf, d.VendorIdOffset),
        ascii_at(buf, d.ProductIdOffset)
    );
    match name.trim() {
        "" => fallback,
        n => n.to_owned(),
    }
}

/// IF index → takma ad ("Wi-Fi", "Ethernet"); okunamazsa `if<N>`.
pub fn iface_name(index: u32) -> String {
    // SAFETY: MIB_IF_ROW2 düz veri; sıfırla başlatılıp InterfaceIndex doldurulur.
    let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    row.InterfaceIndex = index;
    // SAFETY: row geçerli, çağrı yalnız bu yapıya yazar.
    if unsafe { GetIfEntry2(&mut row) } != 0 {
        return format!("if{index}");
    }
    let end = row
        .Alias
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(row.Alias.len());
    String::from_utf16_lossy(&row.Alias[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_or_fall_back() {
        assert_eq!(ascii_at(b"ab\0cd\0", 3), "cd");
        assert_eq!(ascii_at(b"ab", 0), "", "offset 0 = alan yok");
        assert_eq!(ascii_at(b"ab", 99), "");
        assert_ne!(disk_name(0), "", "sistem diski");
        assert_eq!(disk_name(99), "PhysicalDrive99");
        assert_eq!(iface_name(u32::MAX), format!("if{}", u32::MAX));
    }
}
