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

pub(crate) struct Zip64Extra {
    pub(crate) uncompressed_size: u64,
    pub(crate) compressed_size: u64,
    pub(crate) offset: u64,
}

impl ExtraField for Zip64Extra {
    fn serialize(&self, buf: &mut Vec<u8>) {
        put_u16(buf, 0x0001);
        put_u16(buf, 24);
        put_u64(buf, self.uncompressed_size);
        put_u64(buf, self.compressed_size);
        put_u64(buf, self.offset);
    }
}
