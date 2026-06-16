use std::sync::OnceLock;
use std::time::SystemTime;

static LOCAL_OFFSET: OnceLock<time::UtcOffset> = OnceLock::new();

fn local_offset() -> time::UtcOffset {
    *LOCAL_OFFSET
        .get_or_init(|| time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
}

pub(crate) fn system_time_to_ms_dos(t: SystemTime) -> (u16, u16) {
    let dt = time::OffsetDateTime::from(t).to_offset(local_offset());
    let y = dt.year().clamp(1980, 2107);
    let m = u8::from(dt.month()) as u16;
    let day = dt.day() as u16;
    let hour = dt.hour() as u16;
    let minute = dt.minute() as u16;
    let dos_sec = (dt.second() as u16) / 2;
    let date = ((y - 1980) as u16) << 9 | m << 5 | day;
    let time = hour << 11 | minute << 5 | dos_sec;
    (time, date)
}

pub(crate) fn system_time_to_unix_secs(t: SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn unix_secs_to_ms_dos(secs: u64) -> (u16, u16) {
    let duration = std::time::Duration::from_secs(secs);
    let system_time = std::time::UNIX_EPOCH + duration;
    system_time_to_ms_dos(system_time)
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
}
