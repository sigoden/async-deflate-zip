use super::stored_entry::StoredEntry;

/// Parse the `0x5455` extended timestamp extra field for Unix mtime.
///
/// Returns `None` if the field is absent or malformed.
fn parse_ut_extra(extra: &[u8]) -> Option<u64> {
    let mut i = 0;
    while i + 4 <= extra.len() {
        let tag = u16::from_le_bytes(extra[i..i + 2].try_into().unwrap());
        let data_size = u16::from_le_bytes(extra[i + 2..i + 4].try_into().unwrap()) as usize;
        if i + 4 + data_size > extra.len() {
            break;
        }
        if tag == 0x5455 && data_size >= 5 {
            let flags = extra[i + 4];
            if flags & 1 != 0 {
                // mtime present as u32 at offset i+5
                let secs = u32::from_le_bytes(extra[i + 5..i + 9].try_into().unwrap()) as u64;
                return Some(secs);
            }
        }
        i += 4 + data_size;
    }
    None
}

/// Look up a Central Directory entry by index in a raw ZIP buffer.
///
/// Parses all `StoredEntry` fields from the Central Directory record.
/// For non-ZIP64 entries (the common test case), `local_header_offset`
/// is read from the fixed 32-bit field. For ZIP64 entries, the real
/// offset would be in the ZIP64 extra field — this helper does not
//currently parse that (test archives are small).
pub(crate) fn lookup_entry(buf: &[u8], index: usize) -> StoredEntry {
    let sig = b"PK\x01\x02";
    let pos: Vec<usize> = buf
        .windows(4)
        .enumerate()
        .filter(|(_, w)| w == sig)
        .map(|(i, _)| i)
        .collect();
    let pos = *pos.get(index).expect("entry not found");
    let cd = &buf[pos..];

    let method = u16::from_le_bytes(cd[10..12].try_into().unwrap());
    let crc32 = u32::from_le_bytes(cd[16..20].try_into().unwrap());
    let compressed_size = u32::from_le_bytes(cd[20..24].try_into().unwrap()) as u64;
    let uncompressed_size = u32::from_le_bytes(cd[24..28].try_into().unwrap()) as u64;
    let name_len = u16::from_le_bytes(cd[28..30].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(cd[30..32].try_into().unwrap()) as usize;
    let external_file_attributes = u32::from_le_bytes(cd[38..42].try_into().unwrap());
    let local_header_offset = u32::from_le_bytes(cd[42..46].try_into().unwrap()) as u64;
    let name = String::from_utf8_lossy(&cd[46..46 + name_len]).to_string();

    // Parse file type and permissions from external_file_attributes
    let file_type = (external_file_attributes >> 16) & 0o170000;
    let is_directory = file_type == 0o040000;
    let is_symlink = file_type == 0o120000;
    let unix_permissions = if external_file_attributes != 0 {
        Some((external_file_attributes >> 16) & 0o7777)
    } else {
        None
    };

    let is_stored = method == 0;

    // Parse extended timestamp from extra field
    let extra_start = 46 + name_len;
    let extra = &buf[pos + extra_start..pos + extra_start + extra_len];
    let unix_mtime = parse_ut_extra(extra);
    // mtime is reconstructed only if we have enough info — keep as None
    // for now since we don't parse MS-DOS time from the CD header fields
    // back into a SystemTime.
    let mtime = None;

    StoredEntry {
        name,
        crc32,
        compressed_size,
        uncompressed_size,
        local_header_offset,
        is_directory,
        is_symlink,
        is_stored,
        mtime,
        unix_mtime,
        unix_permissions,
    }
}
