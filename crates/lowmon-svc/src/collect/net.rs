//! Ağ rx/tx: GetIfEntry2 (aktif fiziksel arayüz indeksleri cache'li), keşif GetIfTable2.
use lowmon_core::metrics::{NetIo, per_sec};
use windows_sys::Win32::NetworkManagement::{
    IpHelper::{
        FreeMibTable, GetIfEntry2, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2,
        MIB_IF_TABLE2,
    },
    Ndis::IfOperStatusUp,
};

struct Nic {
    index: u32,
    prev: Option<(u64, u64)>,
}

#[derive(Default)]
pub struct Nics {
    nics: Vec<Nic>,
}

/// Donanım arayüzü, filtre kopyası değil (WFP/QoS kopyaları çift sayar), açık, loopback değil.
// shortcut: VPN / Hyper-V vEthernet sayılmaz (donanım değil); istenirse config'le açılır
fn counted(row: &MIB_IF_ROW2) -> bool {
    let flags = row.InterfaceAndOperStatusFlags._bitfield;
    let hardware = flags & 0b01 != 0;
    let filter = flags & 0b10 != 0;
    hardware && !filter && row.OperStatus == IfOperStatusUp && row.Type != IF_TYPE_SOFTWARE_LOOPBACK
}

impl Nics {
    /// `aliases`: sayılan arayüzlerin (ifIndex, takma ad) listesi, alarm `device` eşlemesi için.
    pub fn rediscover(&mut self, aliases: &mut Vec<(u32, String)>) {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        // SAFETY: başarılıysa tabloyu iphlpapi ayırır, aşağıda FreeMibTable ile bırakılır.
        if unsafe { GetIfTable2(&mut table) } != 0 {
            return;
        }
        // SAFETY: NumEntries kadar satır tabloda ardışık.
        let rows = unsafe {
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
        };
        aliases.clear();
        for r in rows.iter().filter(|r| counted(r)) {
            let end = r
                .Alias
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(r.Alias.len());
            aliases.push((r.InterfaceIndex, String::from_utf16_lossy(&r.Alias[..end])));
        }
        let up: Vec<u32> = aliases.iter().map(|a| a.0).collect();
        // SAFETY: GetIfTable2'nin ayırdığı tablo, bir kez bırakılır; rows artık kullanılmıyor.
        unsafe { FreeMibTable(table.cast()) };
        self.nics.retain(|n| up.contains(&n.index));
        for index in up {
            if !self.nics.iter().any(|n| n.index == index) {
                self.nics.push(Nic { index, prev: None });
            }
        }
    }

    pub fn read(&mut self, dt_ms: u64, out: &mut Vec<NetIo>) {
        out.clear();
        self.nics.retain_mut(|n| {
            let mut row = MIB_IF_ROW2 {
                InterfaceIndex: n.index,
                ..Default::default()
            };
            // SAFETY: row yığında, InterfaceIndex dolu.
            if unsafe { GetIfEntry2(&mut row) } != 0 {
                return false;
            }
            let cur = (row.InOctets, row.OutOctets);
            if let Some(p) = n.prev {
                out.push(NetIo {
                    if_index: n.index,
                    rx_bps: per_sec(p.0, cur.0, dt_ms),
                    tx_bps: per_sec(p.1, cur.1, dt_ms),
                });
            }
            n.prev = Some(cur);
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2_0;

    fn row(flags: u8, up: bool, ty: u32) -> MIB_IF_ROW2 {
        MIB_IF_ROW2 {
            InterfaceAndOperStatusFlags: MIB_IF_ROW2_0 { _bitfield: flags },
            OperStatus: if up { IfOperStatusUp } else { 2 },
            Type: ty,
            ..Default::default()
        }
    }

    #[test]
    fn filter_rules() {
        assert!(counted(&row(0b01, true, 6)));
        assert!(!counted(&row(0b11, true, 6))); // filtre kopyası
        assert!(!counted(&row(0b00, true, 6))); // sanal
        assert!(!counted(&row(0b01, false, 6))); // kapalı
        assert!(!counted(&row(0b01, true, IF_TYPE_SOFTWARE_LOOPBACK)));
    }

    #[test]
    fn discovers_active_nic() {
        let mut n = Nics::default();
        let mut a = Vec::new();
        n.rediscover(&mut a);
        assert!(!n.nics.is_empty(), "aktif fiziksel arayüz yok");
        assert_eq!(a.len(), n.nics.len());
        assert!(a.iter().all(|(_, name)| !name.is_empty()));
    }
}
