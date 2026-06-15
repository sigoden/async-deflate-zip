use std::time::SystemTime;

use super::*;

#[test]
fn test_local_file_header() {
    let lfh = LocalFileHeader::new("test.txt", METHOD_DEFLATE, false, SystemTime::UNIX_EPOCH);
    let data = lfh.serialize().unwrap();
    assert_eq!(data.len(), 47);
    assert_eq!(&data[0..4], &0x04034b50u32.to_le_bytes());
    assert_eq!(&data[8..10], &METHOD_DEFLATE.to_le_bytes());
    assert!(data[6] & (1 << 3) != 0);
    assert_eq!(u16::from_le_bytes(data[4..6].try_into().unwrap()), 20);
    let extra_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
    assert_eq!(extra_len, 9);

    let lfh = LocalFileHeader::new("bigfile.bin", METHOD_DEFLATE, true, SystemTime::UNIX_EPOCH);
    let data = lfh.serialize().unwrap();
    assert_eq!(
        u16::from_le_bytes(data[4..6].try_into().unwrap()),
        VERSION_ZIP64
    );
    let name_len = u16::from_le_bytes(data[26..28].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
    assert_eq!(extra_len, 13);
    let extra_start = 30 + name_len;
    let extra = &data[extra_start..extra_start + extra_len];
    assert_eq!(u16::from_le_bytes(extra[0..2].try_into().unwrap()), 0x5455);
    assert_eq!(u16::from_le_bytes(extra[9..11].try_into().unwrap()), 0x0001);

    let lfh = LocalFileHeader::new("bigdir", METHOD_STORED, true, SystemTime::UNIX_EPOCH);
    let data = lfh.serialize().unwrap();
    assert_eq!(
        u16::from_le_bytes(data[4..6].try_into().unwrap()),
        VERSION_ZIP64
    );
}

#[test]
fn test_data_descriptor() {
    let dd = DataDescriptor {
        crc32: 0x12345678,
        compressed_size: 100,
        uncompressed_size: 200,
        zip64: false,
    };
    let data = dd.serialize();
    assert_eq!(data.len(), 16);
    assert_eq!(&data[0..4], &0x08074b50u32.to_le_bytes());
    assert_eq!(&data[4..8], &0x12345678u32.to_le_bytes());
    assert_eq!(&data[8..12], &100u32.to_le_bytes());
    assert_eq!(&data[12..16], &200u32.to_le_bytes());

    let dd = DataDescriptor {
        crc32: 0x12345678,
        compressed_size: 5_000_000_000,
        uncompressed_size: 10_000_000_000,
        zip64: true,
    };
    let data = dd.serialize();
    assert_eq!(data.len(), 24);
    assert_eq!(&data[4..8], &0x12345678u32.to_le_bytes());
    assert_eq!(&data[8..16], &5_000_000_000u64.to_le_bytes());
    assert_eq!(&data[16..24], &10_000_000_000u64.to_le_bytes());

    let dd = DataDescriptor {
        crc32: 0xDEADBEEF,
        compressed_size: 100,
        uncompressed_size: 200,
        zip64: true,
    };
    let data = dd.serialize();
    assert_eq!(data.len(), 24);
    assert_eq!(&data[4..8], &0xDEADBEEFu32.to_le_bytes());
    assert_eq!(&data[8..16], &100u64.to_le_bytes());
    assert_eq!(&data[16..24], &200u64.to_le_bytes());
}

#[test]
fn test_central_dir_entry() {
    let cde = CentralDirEntry {
        version_made_by: VERSION_DEFLATE,
        version_needed: VERSION_DEFLATE,
        flags: FLAG_DATA_DESC,
        method: METHOD_DEFLATE,
        time: 0,
        date: 0,
        crc32: 0xDEADBEEF,
        compressed_size: 500,
        uncompressed_size: 1000,
        name: b"test.txt".to_vec(),
        extra: Vec::new(),
        local_header_offset: 0,
        external_file_attributes: 0,
        comment: None,
    };
    let data = cde.serialize().unwrap();
    assert_eq!(&data[0..4], &0x02014b50u32.to_le_bytes());
    assert_eq!(&data[16..20], &0xDEADBEEFu32.to_le_bytes());
    assert_eq!(&data[20..24], &500u32.to_le_bytes());
    assert_eq!(&data[24..28], &1000u32.to_le_bytes());

    let cde = CentralDirEntry {
        version_made_by: VERSION_DEFLATE,
        version_needed: VERSION_DEFLATE,
        flags: FLAG_DATA_DESC,
        method: METHOD_DEFLATE,
        time: 0,
        date: 0,
        crc32: 0,
        compressed_size: 5_000_000_000,
        uncompressed_size: 10_000_000_000,
        name: b"big_file.bin".to_vec(),
        extra: Vec::new(),
        local_header_offset: 0,
        external_file_attributes: 0,
        comment: None,
    };
    let data = cde.serialize().unwrap();
    assert_eq!(&data[0..4], &0x02014b50u32.to_le_bytes());
    assert_eq!(&data[20..24], &u32::MAX.to_le_bytes());
    assert_eq!(&data[24..28], &u32::MAX.to_le_bytes());
    assert_eq!(&data[6..8], &VERSION_ZIP64.to_le_bytes());
    let name_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(data[30..32].try_into().unwrap()) as usize;
    assert_eq!(extra_len, 28);
    let extra_start = 46 + name_len;
    let extra = &data[extra_start..extra_start + extra_len];
    assert_eq!(u16::from_le_bytes(extra[0..2].try_into().unwrap()), 0x0001);
    assert_eq!(u16::from_le_bytes(extra[2..4].try_into().unwrap()), 24);
    assert_eq!(
        u64::from_le_bytes(extra[4..12].try_into().unwrap()),
        10_000_000_000
    );
    assert_eq!(
        u64::from_le_bytes(extra[12..20].try_into().unwrap()),
        5_000_000_000
    );
    assert_eq!(u64::from_le_bytes(extra[20..28].try_into().unwrap()), 0);
}

#[test]
fn test_eocdr() {
    let eocdr = Eocdr {
        total_entries: 10,
        cd_size: 5000,
        cd_offset: 10000,
        comment: None,
    };
    let data = eocdr.serialize();
    assert_eq!(&data[0..4], &0x06054b50u32.to_le_bytes());
    assert_eq!(&data[8..10], &10u16.to_le_bytes());
    assert_eq!(&data[10..12], &10u16.to_le_bytes());
    assert_eq!(&data[12..16], &5000u32.to_le_bytes());
    assert_eq!(&data[16..20], &10000u32.to_le_bytes());

    let eocdr = Eocdr {
        total_entries: 70000,
        cd_size: 5_000_000_000,
        cd_offset: 6_000_000_000,
        comment: None,
    };
    let data = eocdr.serialize();
    assert_eq!(&data[8..10], &0xFFFFu16.to_le_bytes());
    assert_eq!(&data[10..12], &0xFFFFu16.to_le_bytes());
    assert_eq!(&data[12..16], &u32::MAX.to_le_bytes());
    assert_eq!(&data[16..20], &u32::MAX.to_le_bytes());
}

#[test]
fn test_zip64_eocdr() {
    let z64 = Zip64Eocdr {
        total_entries: 70000,
        cd_size: 5_000_000_000,
        cd_offset: 6_000_000_000,
    };
    let data = z64.serialize();
    assert_eq!(data.len(), 56);
    assert_eq!(&data[0..4], &0x06064b50u32.to_le_bytes());
    assert_eq!(&data[4..12], &44u64.to_le_bytes());
    assert_eq!(&data[12..14], &VERSION_ZIP64.to_le_bytes());
    assert_eq!(&data[14..16], &VERSION_ZIP64.to_le_bytes());
    assert_eq!(&data[24..32], &70000u64.to_le_bytes());
    assert_eq!(&data[32..40], &70000u64.to_le_bytes());
    assert_eq!(&data[40..48], &5_000_000_000u64.to_le_bytes());
    assert_eq!(&data[48..56], &6_000_000_000u64.to_le_bytes());
}

#[test]
fn test_zip64_eocdr_locator() {
    let loc = Zip64EocdrLocator {
        eocdr64_offset: 6_000_000_000,
    };
    let data = loc.serialize();
    assert_eq!(data.len(), 20);
    assert_eq!(&data[0..4], &0x07064b50u32.to_le_bytes());
    assert_eq!(&data[4..8], &0u32.to_le_bytes());
    assert_eq!(&data[8..16], &6_000_000_000u64.to_le_bytes());
    assert_eq!(&data[16..20], &1u32.to_le_bytes());
}

#[test]
fn test_central_dir_entry_zip64_with_extra_field() {
    let timestamp_extra = build_extended_timestamp_extra(1700000000);
    let cde = CentralDirEntry {
        version_made_by: VERSION_UNIX,
        version_needed: VERSION_DEFLATE,
        flags: FLAG_DATA_DESC,
        method: METHOD_DEFLATE,
        time: 0,
        date: 0,
        crc32: 0x12345678,
        compressed_size: 5_000_000_000,
        uncompressed_size: 10_000_000_000,
        name: b"big_with_ts.bin".to_vec(),
        extra: timestamp_extra.clone(),
        local_header_offset: 0,
        external_file_attributes: 0,
        comment: None,
    };
    let data = cde.serialize().unwrap();

    assert_eq!(&data[20..24], &u32::MAX.to_le_bytes());
    assert_eq!(&data[24..28], &u32::MAX.to_le_bytes());

    let name_len = u16::from_le_bytes(data[28..30].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(data[30..32].try_into().unwrap()) as usize;

    assert_eq!(
        extra_len, 37,
        "expected 37-byte combined extra, got {extra_len}"
    );

    let extra_start = 46 + name_len;
    let extra_data = &data[extra_start..extra_start + extra_len];

    assert_eq!(&extra_data[0..2], b"UT");
    assert_eq!(
        u16::from_le_bytes(extra_data[9..11].try_into().unwrap()),
        0x0001
    );
    assert_eq!(
        u16::from_le_bytes(extra_data[11..13].try_into().unwrap()),
        24
    );
}

#[test]
fn test_local_file_header_name_too_long() {
    let name = "a".repeat(65536);
    let lfh = LocalFileHeader::new(&name, METHOD_STORED, false, SystemTime::UNIX_EPOCH);
    let result = lfh.serialize();
    assert!(result.is_err(), "expected Err for oversized filename");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("filename too long"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_central_dir_entry_name_too_long() {
    let cde = CentralDirEntry {
        version_made_by: VERSION_DEFLATE,
        version_needed: VERSION_DEFLATE,
        flags: FLAG_DATA_DESC,
        method: METHOD_STORED,
        time: 0,
        date: 0,
        crc32: 0,
        compressed_size: 0,
        uncompressed_size: 0,
        name: "a".repeat(65536).into_bytes(),
        extra: Vec::new(),
        local_header_offset: 0,
        external_file_attributes: 0,
        comment: None,
    };
    let result = cde.serialize();
    assert!(result.is_err(), "expected Err for oversized filename");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("filename too long"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_system_time_to_ms_dos() {
    let (time, date) = system_time_to_ms_dos(SystemTime::now());
    let hour = time >> 11;
    let min = (time >> 5) & 0x3F;
    assert!(hour <= 23);
    assert!(min <= 59);
    let year = (date >> 9) + 1980;
    let month = (date >> 5) & 0x0F;
    let day = date & 0x1F;
    assert!(year >= 2026);
    assert!((1..=12).contains(&month));
    assert!((1..=31).contains(&day));
}
