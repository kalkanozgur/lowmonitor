//! Win32 katmanı: D3DKMT (GPU), disk sıcaklık sorgusu, PDH termal bölge.
use crate::{
    ADAPTER_SOFTWARE, Reading, Sources, kelvin_to_c, parse_disk_temp, pick_adapter, usage_pct,
};
use std::ptr;
use std::time::Instant;
use windows_sys::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTER_PERFDATA, D3DKMT_ADAPTERINFO, D3DKMT_ADAPTERTYPE, D3DKMT_CLOSEADAPTER,
    D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO, D3DKMT_QUERYSTATISTICS,
    D3DKMT_QUERYSTATISTICS_ADAPTER, D3DKMT_QUERYSTATISTICS_NODE, D3DKMTCloseAdapter,
    D3DKMTEnumAdapters2, D3DKMTQueryAdapterInfo, D3DKMTQueryStatistics, KMTQAITYPE_ADAPTERPERFDATA,
    KMTQAITYPE_ADAPTERTYPE,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_PROPERTY_QUERY,
    StorageDeviceTemperatureProperty,
};
use windows_sys::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA, PdhAddEnglishCounterW,
    PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------- GPU: D3DKMT (Görev Yöneticisi'nin yolu; üretici DLL'i yok) ----------------

/// Çalışmış düğümler bu kadar okumada bir yeniden taranır (yeni başlayan motorlar görünsün).
const RESCAN_EVERY: u32 = 30;

/// Seçilen adaptör: hibrit sistemde ayrık GPU, yoksa ilk donanım adaptörü (`pick_adapter`).
struct Gpu {
    adapter: D3DKMT_ADAPTERINFO,
    node_count: u32,
    /// (düğüm, son `RunningTime`); yalnız hiç çalışmış düğümler (sorgu başına ~0,2–0,9 ms)
    nodes: Vec<(u32, i64)>,
    at: Instant,
    reads: u32,
}

impl Gpu {
    fn open() -> Option<Self> {
        // SAFETY: yapılar sıfırlanıp belgelenmiş alanları doldurulur; tamponlar boyutlarıyla verilir.
        // Seçilmeyen adaptör tutamaçları hemen, seçilen `Drop`'ta kapatılır.
        unsafe {
            let mut infos = [std::mem::zeroed::<D3DKMT_ADAPTERINFO>(); 16];
            let mut e = D3DKMT_ENUMADAPTERS2 {
                NumAdapters: infos.len() as u32,
                pAdapters: infos.as_mut_ptr(),
            };
            if D3DKMTEnumAdapters2(&mut e) != 0 {
                return None;
            }
            let infos = &infos[..e.NumAdapters as usize];
            let types: Vec<u32> = infos
                .iter()
                .map(|a| {
                    let mut t = D3DKMT_ADAPTERTYPE::default();
                    let mut q = D3DKMT_QUERYADAPTERINFO {
                        hAdapter: a.hAdapter,
                        Type: KMTQAITYPE_ADAPTERTYPE,
                        pPrivateDriverData: (&raw mut t).cast(),
                        PrivateDriverDataSize: size_of::<D3DKMT_ADAPTERTYPE>() as u32,
                    };
                    // sorgu başarısızsa yazılım aygıtı say (seçilmesin)
                    if D3DKMTQueryAdapterInfo(&mut q) == 0 {
                        t.Anonymous.Value
                    } else {
                        ADAPTER_SOFTWARE
                    }
                })
                .collect();
            let pick = pick_adapter(&types);
            for (i, a) in infos.iter().enumerate() {
                if Some(i) != pick {
                    D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER {
                        hAdapter: a.hAdapter,
                    });
                }
            }
            let adapter = infos[pick?];
            let mut q: D3DKMT_QUERYSTATISTICS = std::mem::zeroed();
            q.Type = D3DKMT_QUERYSTATISTICS_ADAPTER;
            q.AdapterLuid = adapter.AdapterLuid;
            let node_count = if D3DKMTQueryStatistics(&q) == 0 {
                q.QueryResult.AdapterInformation.NodeCount
            } else {
                0
            };
            let mut g = Self {
                adapter,
                node_count,
                nodes: Vec::new(),
                at: Instant::now(),
                reads: 0,
            };
            g.rescan();
            Some(g)
        }
    }

    fn running(&self, node: u32) -> Option<i64> {
        // SAFETY: yapı sıfırlanır, sorgu alanları doldurulur; sonuç yalnız başarıda okunur.
        unsafe {
            let mut q: D3DKMT_QUERYSTATISTICS = std::mem::zeroed();
            q.Type = D3DKMT_QUERYSTATISTICS_NODE;
            q.AdapterLuid = self.adapter.AdapterLuid;
            q.Anonymous.QueryNode.NodeId = node;
            (D3DKMTQueryStatistics(&q) == 0)
                .then_some(q.QueryResult.NodeInformation.GlobalInformation.RunningTime)
        }
    }

    /// Tüm düğümlere bakar; çalışmış olanları (eski sayaçlarını koruyarak) listeye alır.
    fn rescan(&mut self) {
        let old = std::mem::take(&mut self.nodes);
        self.nodes = (0..self.node_count)
            .filter_map(|n| {
                let now = self.running(n).filter(|&r| r != 0)?;
                let prev = old.iter().find(|(id, _)| *id == n).map_or(now, |&(_, r)| r);
                Some((n, prev))
            })
            .collect();
    }

    /// (en yoğun motorun kullanımı %, °C). İlk okumada önceki örnek yok → 0 %.
    fn read(&mut self) -> (Option<u32>, Option<i32>) {
        self.reads += 1;
        if self.reads.is_multiple_of(RESCAN_EVERY) {
            self.rescan();
        }
        let now = Instant::now();
        let dt = now.duration_since(self.at).as_nanos() as f64 / 100.0; // RunningTime birimi: 100 ns
        self.at = now;
        let mut busiest = 0i64;
        for i in 0..self.nodes.len() {
            let (node, prev) = self.nodes[i];
            if let Some(cur) = self.running(node) {
                busiest = busiest.max(cur.wrapping_sub(prev));
                self.nodes[i].1 = cur;
            }
        }
        // SAFETY: çıktı yapısı yerel, boyutuyla verilir; adaptör tutamacı açık.
        let temp = unsafe {
            let mut d: D3DKMT_ADAPTER_PERFDATA = std::mem::zeroed();
            let mut q = D3DKMT_QUERYADAPTERINFO {
                hAdapter: self.adapter.hAdapter,
                Type: KMTQAITYPE_ADAPTERPERFDATA,
                pPrivateDriverData: (&raw mut d).cast(),
                PrivateDriverDataSize: size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
            };
            // Temperature: 0,1 °C; 0 = sürücü bildirmiyor
            (D3DKMTQueryAdapterInfo(&mut q) == 0 && d.Temperature != 0)
                .then(|| (d.Temperature as f64 / 10.0).round() as i32)
        };
        (Some(usage_pct(busiest, dt)), temp)
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: tutamaç `open`'da alındı, başka yerde kapatılmaz.
        unsafe {
            D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER {
                hAdapter: self.adapter.hAdapter,
            });
        }
    }
}

// ---------------- Disk sıcaklığı ----------------

/// Bakılan en çok fiziksel disk (`\\.\PhysicalDrive0..`).
const MAX_DISKS: u32 = 8;

/// Açılabilen diskler; tutamaçlar açılışta alınır (erişim 0: yükseltme gerekmez, diski uyandırmaz).
struct Disks(Vec<HANDLE>);

impl Disks {
    fn open() -> Self {
        let handles = (0..MAX_DISKS)
            .filter_map(|n| {
                // SAFETY: dizge NUL sonlu; dönen tutamaç `Drop`'ta kapatılır.
                let h = unsafe {
                    CreateFileW(
                        wide(&format!(r"\\.\PhysicalDrive{n}")).as_ptr(),
                        0,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        ptr::null(),
                        OPEN_EXISTING,
                        0,
                        ptr::null_mut(),
                    )
                };
                (h != INVALID_HANDLE_VALUE).then_some(h)
            })
            .collect();
        Self(handles)
    }

    /// En sıcak disk; sıcaklık bildirmeyen diskler (USB, sanal) atlanır.
    fn read(&self) -> Option<i32> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceTemperatureProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        self.0
            .iter()
            .filter_map(|&h| {
                let mut out = [0u8; 512];
                let mut ret = 0u32;
                // SAFETY: giriş / çıkış tamponları boyutlarıyla veriliyor; h açık tutamaç.
                let ok = unsafe {
                    DeviceIoControl(
                        h,
                        IOCTL_STORAGE_QUERY_PROPERTY,
                        (&raw const query).cast(),
                        size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                        out.as_mut_ptr().cast(),
                        out.len() as u32,
                        &mut ret,
                        ptr::null_mut(),
                    )
                };
                (ok != 0).then(|| parse_disk_temp(&out[..ret as usize]))?
            })
            .max()
    }
}

impl Drop for Disks {
    fn drop(&mut self) {
        for &h in &self.0 {
            // SAFETY: tutamaçlar `open`'da alındı, başka yerde kapatılmaz.
            unsafe { CloseHandle(h) };
        }
    }
}

// ---------------- CPU sıcaklığı: ACPI termal bölge (PDH; ADR-0033 istisnası) ----------------

struct Thermal {
    query: HANDLE,
    counter: HANDLE,
    /// Sayaç dizisi tamponu (u64 → 8 bayt hizalı), yeniden kullanılır.
    buf: Vec<u64>,
}

impl Thermal {
    fn open() -> Option<Self> {
        let (mut query, mut counter) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: çıktı işaretçileri yerel; yol NUL sonlu. Başarısızlıkta açılan sorgu kapatılır.
        unsafe {
            if PdhOpenQueryW(ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let path = wide(r"\Thermal Zone Information(*)\Temperature");
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
        }
        Some(Self {
            query,
            counter,
            buf: Vec::new(),
        })
    }

    /// En sıcak bölge (°C).
    fn read(&mut self) -> Option<i32> {
        // SAFETY: query / counter `open`'da alındı. Tampon bayt boyutuyla verilir; PDH_MORE_DATA'da
        // istenen boyuta büyütülüp yeniden denenir. Öğeler tamponun başında, 8 bayt hizalı.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let (mut size, mut count) = ((self.buf.len() * 8) as u32, 0u32);
            let mut st = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                self.buf.as_mut_ptr().cast(),
            );
            if st == PDH_MORE_DATA {
                self.buf.resize((size as usize).div_ceil(8), 0);
                size = (self.buf.len() * 8) as u32;
                st = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    self.buf.as_mut_ptr().cast(),
                );
            }
            if st != 0 {
                return None;
            }
            let items = std::slice::from_raw_parts(
                self.buf.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                count as usize,
            );
            items
                .iter()
                .filter(|i| i.FmtValue.CStatus == 0)
                .filter_map(|i| kelvin_to_c(i.FmtValue.Anonymous.doubleValue))
                .max()
        }
    }
}

impl Drop for Thermal {
    fn drop(&mut self) {
        // SAFETY: sorgu bu nesneye ait; sayaç sorguyla birlikte kapanır.
        unsafe { PdhCloseQuery(self.query) };
    }
}

// ---------------- Hepsi ----------------

/// Disk sıcaklığı bu kadar okumada bir yenilenir: sorgu ~5 ms bekletir (aygıt G/Ç), değer yavaş değişir.
const DISK_EVERY: u32 = 15;

/// Açık kaynaklar; `Send` değil (tek iş parçacığında kurulup okunur).
pub struct Sensors {
    gpu: Option<Gpu>,
    disks: Option<Disks>,
    thermal: Option<Thermal>,
    disk_c: Option<i32>,
    reads: u32,
}

impl Sensors {
    /// Seçilen kaynakları açar; açılamayan kaynak sessizce `None` kalır (`Reading` alanı `None`).
    /// PDH açılışı ~0,3 s sürer: arayüz iş parçacığında çağrılmaz.
    pub fn open(s: Sources) -> Self {
        Self {
            gpu: s.gpu.then(Gpu::open).flatten(),
            disks: s.disk_temp.then(Disks::open),
            thermal: s.cpu_temp.then(Thermal::open).flatten(),
            disk_c: None,
            reads: 0,
        }
    }

    /// Bir okuma (~1 ms; disk yenilenen okumada ~5 ms daha bekler): arayüz iş parçacığında çağrılmaz.
    pub fn read(&mut self) -> Reading {
        if self.reads.is_multiple_of(DISK_EVERY) {
            self.disk_c = self.disks.as_ref().and_then(Disks::read);
        }
        self.reads = self.reads.wrapping_add(1);
        let (gpu_pct, gpu_c) = self.gpu.as_mut().map_or((None, None), Gpu::read);
        Reading {
            gpu_pct,
            gpu_c,
            cpu_c: self.thermal.as_mut().and_then(Thermal::read),
            disk_c: self.disk_c,
        }
    }
}
