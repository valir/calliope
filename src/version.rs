//! Pure version helpers, shared by `build.rs` (via `#[path]`) and the crate.

/// Formats `YY.MM.BBBB`, zero-padded to 2/2/4 digits.
pub fn format_version(yy: u32, mm: u32, build: u32) -> String {
    format!("{yy:02}.{mm:02}.{build:04}")
}

/// Converts a Unix timestamp to the UTC `(year % 100, month)`.
pub fn yymm_from_unix(secs: i64) -> (u32, u32) {
    // Howard Hinnant's civil_from_days algorithm.
    let days = secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year.rem_euclid(100) as u32, m as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_pads_fields() {
        assert_eq!(format_version(26, 10, 42), "26.10.0042");
        assert_eq!(format_version(7, 1, 0), "07.01.0000");
        assert_eq!(format_version(26, 12, 9999), "26.12.9999");
    }

    #[test]
    fn yymm_from_unix_dates() {
        assert_eq!(yymm_from_unix(0), (70, 1));
        assert_eq!(yymm_from_unix(1_790_000_000), (26, 9));
        assert_eq!(yymm_from_unix(1_798_761_599), (26, 12));
        assert_eq!(yymm_from_unix(1_798_761_600), (27, 1));
    }
}
