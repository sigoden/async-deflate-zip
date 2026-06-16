use crate::zip_format::binary::*;
use crate::zip_format::constants::*;

pub(crate) struct Eocdr {
    pub(crate) total_entries: u64,
    pub(crate) cd_size: u64,
    pub(crate) cd_offset: u64,
    pub(crate) comment: Option<Vec<u8>>,
}

impl Eocdr {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let use_zip64 =
            self.total_entries > 0xFFFF || self.cd_size > U32_MAX || self.cd_offset > U32_MAX;

        let comment = self.comment.as_deref().unwrap_or_default();

        let mut buf = Vec::with_capacity(22 + comment.len());
        put_u32(&mut buf, EOCDR_SIG);
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 0);
        let total = if use_zip64 {
            0xFFFF
        } else {
            self.total_entries as u16
        };
        put_u16(&mut buf, total);
        put_u16(&mut buf, total);
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.cd_size as u32
            },
        );
        put_u32(
            &mut buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.cd_offset as u32
            },
        );
        put_u16(&mut buf, comment.len() as u16);
        buf.extend_from_slice(comment);
        buf
    }
}

pub(crate) struct Zip64Eocdr {
    pub(crate) total_entries: u64,
    pub(crate) cd_size: u64,
    pub(crate) cd_offset: u64,
}

impl Zip64Eocdr {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(56);
        put_u32(&mut buf, EOCDR64_SIG);
        put_u64(&mut buf, 44);
        put_u16(&mut buf, VERSION_ZIP64);
        put_u16(&mut buf, VERSION_ZIP64);
        put_u32(&mut buf, 0);
        put_u32(&mut buf, 0);
        put_u64(&mut buf, self.total_entries);
        put_u64(&mut buf, self.total_entries);
        put_u64(&mut buf, self.cd_size);
        put_u64(&mut buf, self.cd_offset);
        buf
    }
}

pub(crate) struct Zip64EocdrLocator {
    pub(crate) eocdr64_offset: u64,
}

impl Zip64EocdrLocator {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20);
        put_u32(&mut buf, EOCDR64L_SIG);
        put_u32(&mut buf, 0);
        put_u64(&mut buf, self.eocdr64_offset);
        put_u32(&mut buf, 1);
        buf
    }
}
