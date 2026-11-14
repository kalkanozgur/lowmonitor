//! lowmon-net için gelen bağlantı kuralı (spek §16.2, ADR-0025): yalnız Private profil + yerel alt ağ.
//! `install` yükseltilmiş çalıştığı için kuralı o ekler, `uninstall` siler. `netsh` her Windows'ta var.
use std::{io, path::Path, process::Command};

pub const RULE: &str = "LowMonitor net";
/// shortcut: port sabit; net.toml'da port değişirse kural elle güncellenir (`netsh … set rule`)
pub const PORT: u16 = 47800;

fn add_args(exe: &Path) -> Vec<String> {
    [
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={RULE}"),
        "dir=in",
        "action=allow",
        &format!("program={}", exe.display()),
        "enable=yes",
        "profile=private",
        "protocol=TCP",
        &format!("localport={PORT}"),
        "remoteip=localsubnet",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn netsh(args: &[String]) -> io::Result<bool> {
    Ok(Command::new("netsh").args(args).output()?.status.success())
}

/// Eski kuralı siler; `exe` verilirse yenisini ekler (tekrar çalıştırılabilir).
pub fn set(exe: Option<&Path>) -> io::Result<()> {
    let delete = [
        "advfirewall",
        "firewall",
        "delete",
        "rule",
        &format!("name={RULE}"),
    ]
    .map(str::to_owned);
    netsh(&delete)?; // kural yoksa başarısız döner; önemsiz
    match exe {
        Some(exe) if !netsh(&add_args(exe))? => Err(io::Error::other("netsh add rule başarısız")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_is_private_subnet_only() {
        let a = add_args(Path::new(r"C:\Program Files\LowMonitor\lowmon-net.exe"));
        for want in [
            "profile=private",
            "remoteip=localsubnet",
            "dir=in",
            "localport=47800",
            r"program=C:\Program Files\LowMonitor\lowmon-net.exe",
        ] {
            assert!(a.iter().any(|x| x == want), "{want} yok: {a:?}");
        }
    }
}
