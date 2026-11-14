//! UTC gün hesabı: segment dosya adı `YYYY-MM-DD.lwm` (UTC; DST/saat dilimi değişimi segment bölmez).

pub const DAY_MS: u64 = 86_400_000;

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
    }
}
