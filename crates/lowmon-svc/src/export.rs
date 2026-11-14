//! Pipe `Export` (spek §6.3): servis loglarını `[from, to)` için CSV'ye döker.
//! Servis anahtarı yalnızca serviste açılır (ADR-0016); bu yol admin'e dışa aktarım sağlar.
use lowmon_core::{
    crypto::MasterKey,
    csv,
    time::{day_of, parse_day},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

/// shortcut: yazıcının bellekteki son bloğu (≤ 60 örnek) dahil değil; flush Export'tan önce M4'te
pub fn export(
    root: &Path,
    master: &MasterKey,
    from: u64,
    to: u64,
    out: &mut impl Write,
) -> io::Result<()> {
    writeln!(out, "{}", csv::HEADER)?;
    if from >= to {
        return Ok(());
    }
    let days = day_of(from)..=day_of(to - 1);
    let mut segs: Vec<(u64, std::path::PathBuf)> = fs::read_dir(root.join("logs"))?
        .filter_map(|e| {
            let p = e.ok()?.path();
            let name = p.file_name()?.to_str()?;
            let day = parse_day(name)?;
            (name.len() == 14 && name.ends_with(".lwm") && days.contains(&day)).then_some((day, p))
        })
        .collect();
    segs.sort();
    for (_, p) in segs {
        // Açık segment yazılırken okunabilir; yarım son blok bütünlük hatası satırı olarak görünür
        let bytes = fs::read(&p)?;
        let o = csv::segment(master, &bytes, from, to, out)?;
        if let Some(e) = o.error {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            writeln!(
                out,
                "{}",
                csv::other_row("error", from, &format!("{name}: {e}"))
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Writer;
    use lowmon_core::{
        metrics::Sample,
        time::{DAY_MS, segment_name},
    };

    #[test]
    fn exports_range_across_days_and_flags_corruption() {
        let r = std::env::temp_dir().join(format!("lowmon-test-{}-export", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let key = [5; 32];
        let t0 = 20_000 * DAY_MS + DAY_MS - 3_000; // gün sonuna 3 s kala
        let mut w = Writer::open(&r, key, 1000, t0, 3_600_000).unwrap();
        for i in 0..6 {
            let s = Sample {
                ts_unix_ms: t0 + i * 1000,
                ..Default::default()
            };
            w.sample(&s).unwrap();
        }
        w.close().unwrap();

        let csv_of = |from, to| {
            let mut out = Vec::new();
            export(&r, &key, from, to, &mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        let all = csv_of(t0, t0 + 6_000);
        assert_eq!(all.lines().next(), Some(csv::HEADER));
        assert_eq!(
            all.matches("sample,").count(),
            6,
            "iki güne yayılan 6 örnek:\n{all}"
        );
        assert_eq!(csv_of(t0 + 3_000, t0 + 4_000).matches("sample,").count(), 1);
        assert_eq!(
            csv_of(5, 5).lines().count(),
            1,
            "boş aralık = yalnız başlık"
        );

        // İkinci günün dosyası bozulursa: ilk gün verisi gelir + error satırı
        let p2 = r.join("logs").join(segment_name(day_of(t0) + 1));
        let mut b = fs::read(&p2).unwrap();
        b[64 + 4 + 24 + 1] ^= 1; // başlık + len + nonce → ilk bloğun şifreli metni
        fs::write(&p2, b).unwrap();
        let bad = csv_of(t0, t0 + 6_000);
        assert_eq!(bad.matches("sample,").count(), 3);
        assert!(
            bad.lines()
                .any(|l| l.starts_with("error,") && l.contains("Auth"))
        );
        let _ = fs::remove_dir_all(&r);
    }
}
