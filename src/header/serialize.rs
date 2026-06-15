use std::time::SystemTime;

/// Write a `u16` in little-endian order into a byte buffer.
pub(crate) fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write a `u32` in little-endian order into a byte buffer.
pub(crate) fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write a `u64` in little-endian order into a byte buffer.
pub(crate) fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Write a `u8` into a byte buffer.
pub(crate) fn put_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}

/// Convert a `SystemTime` to MS-DOS date/time format.
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

/// Build the Info-ZIP extended timestamp extra field (ID `0x5455`) with mtime only.
pub(crate) fn build_extended_timestamp_extra(mtime: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    put_u16(&mut buf, 0x5455);
    put_u16(&mut buf, 5);
    put_u8(&mut buf, 1);
    put_u32(&mut buf, mtime.min(u32::MAX as u64) as u32);
    buf
}

/// Build the Unix UID/GID extra field (ID `0x7875`).
pub(crate) fn build_unix_uid_gid_extra(uid: u32, gid: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(15);
    put_u16(&mut buf, 0x7875);
    put_u16(&mut buf, 11);
    put_u8(&mut buf, 1);
    put_u8(&mut buf, 4);
    put_u32(&mut buf, uid);
    put_u8(&mut buf, 4);
    put_u32(&mut buf, gid);
    buf
}

/// Build the Unicode extra field (ID `0x7075`).
pub(crate) fn build_unicode_extra_field(name: &str) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let data_size = 1 + name_bytes.len();
    let mut buf = Vec::with_capacity(4 + data_size);
    put_u16(&mut buf, 0x7075);
    put_u16(&mut buf, data_size as u16);
    put_u8(&mut buf, 1);
    buf.extend_from_slice(name_bytes);
    buf
}

/// Convert a `SystemTime` to MS-DOS date/time and Unix timestamp pair.
pub(crate) fn mtime_to_ms_dos_and_unix(mtime: SystemTime) -> ((u16, u16), u64) {
    let (time, date) = system_time_to_ms_dos(mtime);
    let secs = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    ((time, date), secs)
}

/// Build a ZIP64 extra field for a Local File Header.
pub(crate) fn build_zip64_extra_lfh() -> Vec<u8> {
    let mut data = Vec::with_capacity(4);
    put_u16(&mut data, 0x0001);
    put_u16(&mut data, 0);
    data
}
