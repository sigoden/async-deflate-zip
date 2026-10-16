use std::time::SystemTime;

/// Unix timestamp of the earliest DOS-representable date (1980-01-01T00:00:00Z).
const DOS_MIN_UNIX: i64 = 315_532_800;

/// Unix timestamp of the latest DOS-representable date (2107-12-31T23:59:59Z).
const DOS_MAX_UNIX: i64 = 4_354_819_199;

/// Convert a Unix timestamp (in seconds) into a DOS date/time pair.
///
/// Timestamps outside the DOS-representable range (1980..=2107) are clamped so
/// that conversion never panics, even for `SystemTime` values the `time` crate
/// cannot represent.
fn unix_secs_to_ms_dos_signed(secs: i64) -> (u16, u16) {
    let secs = secs.clamp(DOS_MIN_UNIX, DOS_MAX_UNIX);
    let dt = time::OffsetDateTime::from_unix_timestamp(secs)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .to_offset(time::UtcOffset::UTC);
    let y = dt.year();
    let m = u8::from(dt.month()) as u16;
    let day = dt.day() as u16;
    let hour = dt.hour() as u16;
    let minute = dt.minute() as u16;
    let dos_sec = (dt.second() as u16) / 2;
    let date = ((y - 1980) as u16) << 9 | m << 5 | day;
    let time = hour << 11 | minute << 5 | dos_sec;
    (time, date)
}

pub(crate) fn system_time_to_ms_dos(t: SystemTime) -> (u16, u16) {
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs().min(i64::MAX as u64) as i64,
        Err(e) => -(e.duration().as_secs().min(i64::MAX as u64) as i64),
    };
    unix_secs_to_ms_dos_signed(secs)
}

pub(crate) fn system_time_to_unix_secs(t: SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn unix_secs_to_ms_dos(secs: u64) -> (u16, u16) {
    unix_secs_to_ms_dos_signed(secs.min(i64::MAX as u64) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn test_system_time_to_ms_dos() {
        let (time, date) = system_time_to_ms_dos(SystemTime::UNIX_EPOCH);

        let hour = time >> 11;
        let min = (time >> 5) & 0x3F;
        let sec = (time & 0x1F) * 2;
        assert!(hour <= 23);
        assert!(min <= 59);
        assert!(sec <= 59);

        let year = (date >> 9) + 1980;
        let month = (date >> 5) & 0x0F;
        let day = date & 0x1F;
        assert!((1980..=2107).contains(&year));
        assert!((1..=12).contains(&month));
        assert!((1..=31).contains(&day));

        assert_eq!(year, 1980);
    }

    fn date_year(date: u16) -> i32 {
        ((date >> 9) as i32) + 1980
    }

    #[test]
    fn test_system_time_to_ms_dos_clamps_out_of_range() {
        let far_future = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100_000_000_000);
        let (_, date) = system_time_to_ms_dos(far_future);
        assert_eq!(date_year(date), 2107);

        let far_past = SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(2_000_000_000);
        let (_, date) = system_time_to_ms_dos(far_past);
        assert_eq!(date_year(date), 1980);
    }

    #[test]
    fn test_unix_secs_to_ms_dos_out_of_range() {
        let (_, date) = unix_secs_to_ms_dos(u64::MAX);
        assert_eq!(date_year(date), 2107);

        // The signed helper also clamps values before the Unix epoch.
        let (_, date) = unix_secs_to_ms_dos_signed(i64::MIN);
        assert_eq!(date_year(date), 1980);
    }
}
