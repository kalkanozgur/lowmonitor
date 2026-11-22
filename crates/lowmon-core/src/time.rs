//! UTC gün hesabı: segment dosya adı `YYYY-MM-DD.lwm` (UTC; DST/saat dilimi değişimi segment bölmez).

pub const DAY_MS: u64 = 86_400_000;

/// Şu an, Unix ms (UTC). Saat epoch'tan gerideyse 0.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Unix ms → epoch'tan beri UTC gün.
pub fn day_of(ts_ms: u64) -> u64 {
    ts_ms / DAY_MS
}

/// Epoch günü → (yıl, ay, gün). H. Hinnant `civil_from_days`.
pub fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + u64::from(m <= 2), m, d)
}

/// (yıl, ay, gün) → epoch günü; `civil` tersi (Hinnant days_from_civil). 1970 öncesi yok.
pub fn days_from_civil(y: u64, m: u64, d: u64) -> Option<u64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe).checked_sub(719_468)
}

/// `YYYY-MM-DD` ile başlayan dosya adından gün (ör. `2026-11-14.lwm`, `2026-11-14.lwm.bad-3`).
pub fn parse_day(name: &str) -> Option<u64> {
    let num = |r: std::ops::Range<usize>| name.get(r)?.parse::<u64>().ok();
    if name.get(4..5)? != "-" || name.get(7..8)? != "-" {
        return None;
    }
    let ymd = (num(0..4)?, num(5..7)?, num(8..10)?);
    let day = days_from_civil(ymd.0, ymd.1, ymd.2)?;
    (civil(day) == ymd).then_some(day) // 2026-02-31 gibi geçersizler elenir
}

pub fn segment_name(day: u64) -> String {
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}.lwm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(segment_name(day_of(1_763_128_800_000)), "2025-11-14.lwm");
        assert_eq!(civil(day_of(951_782_400_000)), (2000, 2, 29)); // artık yıl
        assert_eq!(civil(day_of(1_798_675_200_000)), (2026, 12, 31));
        for day in [0, 10_957, 20_000, 20_406] {
            assert_eq!(parse_day(&segment_name(day)), Some(day));
        }
        assert_eq!(
            parse_day("2026-11-14.lwm.bad-3"),
            parse_day("2026-11-14.lwm")
        );
        assert_eq!(parse_day("2026-02-31.lwm"), None);
        assert_eq!(parse_day("state.bin"), None);
    }
}
