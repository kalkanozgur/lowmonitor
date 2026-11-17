//! Süreçler sekmesi: mantık + tarama `lowmon_deck::procs`'ta (LowDeck ADR-0009 telefonla ortak); burada metin biçimi.

pub use lowmon_deck::procs::*;

/// "12.3%"; < %0,05 → boş (liste sakin okunur; EMA kuyruğu "0.0%" göstermesin).
pub fn fmt_cpu(cpu: u16) -> String {
    if cpu < 5 {
        String::new()
    } else {
        format!("{:.1}%", cpu as f32 / 100.0)
    }
}

/// Yükseltilmiş kopyanın çıkış kodları (`--kill`), sonuç metni ana kopyada.
pub mod exit {
    pub use lowmon_deck::procs::exit::*;

    pub fn text(code: i32, name: &str) -> String {
        use lowmon_ai::text::tr;
        match code {
            OK => format!("{name} {}", tr("sonlandırıldı", "ended")),
            GONE => format!(
                "{name}: {}",
                tr(
                    "süreç artık yok (ya da PID başka sürece geçti)",
                    "process is gone (or the PID now belongs to another process)"
                )
            ),
            PROTECTED => format!(
                "{name}: {}",
                tr(
                    "korumalı süreç, sonlandırılamaz",
                    "protected process, cannot be ended"
                )
            ),
            DENIED => format!("{name}: {}", tr("erişim reddedildi", "access denied")),
            CANCELLED => format!(
                "{name}: {}",
                tr(
                    "yönetici onayı verilmedi",
                    "administrator approval was not given"
                )
            ),
            c => format!(
                "{name}: {} ({} {c})",
                tr("sonlandırılamadı", "could not be ended"),
                tr("kod", "code")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        assert!(exit::text(exit::GONE, "a").contains("artık yok"));
        assert_eq!(fmt_cpu(0), "");
        assert_eq!(fmt_cpu(4), "");
        assert_eq!(fmt_cpu(5), "0.1%");
        assert_eq!(fmt_cpu(1234), "12.3%");
    }
}
