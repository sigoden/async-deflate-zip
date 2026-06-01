use super::stored_entry::StoredEntry;

/// Look up a Central Directory entry by index in a raw ZIP buffer.
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

    let crc32 = u32::from_le_bytes(cd[16..20].try_into().unwrap());
    let compressed_size = u32::from_le_bytes(cd[20..24].try_into().unwrap()) as u64;
    let uncompressed_size = u32::from_le_bytes(cd[24..28].try_into().unwrap()) as u64;
    let name_len = u16::from_le_bytes(cd[28..30].try_into().unwrap()) as usize;
    let name = String::from_utf8_lossy(&cd[46..46 + name_len]).to_string();

    StoredEntry {
        name,
        crc32,
        compressed_size,
        uncompressed_size,
        local_header_offset: 0,
        is_directory: false,
        is_symlink: false,
        is_stored: false,
        mtime: None,
        unix_mtime: None,
        unix_permissions: None,
    }
}
