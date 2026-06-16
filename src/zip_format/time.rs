use std::time::SystemTime;

pub(crate) fn system_time_to_ms_dos(t: SystemTime) -> (u16, u16) {
    let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let dt = time::OffsetDateTime::from(t).to_offset(local_offset);
    let y = dt.year().clamp(1980, 2107);
    let m = u8::from(dt.month()) as u16;
    let day = dt.day() as u16;
    let hour = dt.hour() as u16;
    let minute = dt.minute() as u16;
    let dos_sec = (dt.second() as u16).div_ceil(2);
    let date = ((y - 1980) as u16) << 9 | m << 5 | day;
    let time = hour << 11 | minute << 5 | dos_sec;
    (time, date)
}

pub(crate) fn mtime_to_ms_dos_and_unix(mtime: SystemTime) -> ((u16, u16), u64) {
    let (time, date) = system_time_to_ms_dos(mtime);
    let secs = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    ((time, date), secs)
}
