use super::binary::*;

pub(crate) trait ExtraField {
    fn serialize(&self, buf: &mut Vec<u8>);
}

pub(crate) struct ExtendedTimestampExtra {
    mtime: u32,
}

impl ExtendedTimestampExtra {
    pub(crate) fn new(mtime: u64) -> Self {
        Self {
            mtime: mtime.min(u32::MAX as u64) as u32,
        }
    }
}

impl ExtraField for ExtendedTimestampExtra {
    fn serialize(&self, buf: &mut Vec<u8>) {
        put_u16(buf, 0x5455);
        put_u16(buf, 5);
        put_u8(buf, 1);
        put_u32(buf, self.mtime);
    }
}

pub(crate) struct UnixUidGidExtra {
    uid: u32,
    gid: u32,
}

impl UnixUidGidExtra {
    pub(crate) fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }
}

impl ExtraField for UnixUidGidExtra {
    fn serialize(&self, buf: &mut Vec<u8>) {
        put_u16(buf, 0x7875);
        put_u16(buf, 11);
        put_u8(buf, 1);
        put_u8(buf, 4);
        put_u32(buf, self.uid);
        put_u8(buf, 4);
        put_u32(buf, self.gid);
    }
}

pub(crate) struct UnicodePathExtra {
    name: Vec<u8>,
}

impl UnicodePathExtra {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.as_bytes().to_vec(),
        }
    }
}

impl ExtraField for UnicodePathExtra {
    fn serialize(&self, buf: &mut Vec<u8>) {
        put_u16(buf, 0x7075);
        put_u16(buf, (self.name.len() + 1) as u16);
        put_u8(buf, 1);
        buf.extend_from_slice(&self.name);
    }
}

pub(crate) enum Zip64Extra {
    LocalFileHeader,
    CentralDirectory {
        uncompressed_size: u64,
        compressed_size: u64,
        offset: u64,
    },
}

impl ExtraField for Zip64Extra {
    fn serialize(&self, buf: &mut Vec<u8>) {
        match self {
            Zip64Extra::LocalFileHeader => {
                put_u16(buf, 0x0001);
                put_u16(buf, 0);
            }
            Zip64Extra::CentralDirectory {
                uncompressed_size,
                compressed_size,
                offset,
            } => {
                put_u16(buf, 0x0001);
                put_u16(buf, 24);
                put_u64(buf, *uncompressed_size);
                put_u64(buf, *compressed_size);
                put_u64(buf, *offset);
            }
        }
    }
}
