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

#[cfg(test)]
mod tests {
    use crate::zip_format::binary::test_utils::*;
    use crate::zip_format::*;

    #[test]
    fn test_eocd_normal_case() {
        let eocdr = Eocdr {
            total_entries: 10,
            cd_size: 5000,
            cd_offset: 10000,
            comment: None,
        };
        let data = eocdr.serialize();
        assert_eq!(read_u32(&data, 0), EOCDR_SIG);
        assert_eq!(read_u16(&data, 8), 10);
        assert_eq!(read_u16(&data, 10), 10);
        assert_eq!(read_u32(&data, 12), 5000);
        assert_eq!(read_u32(&data, 16), 10000);
    }

    #[test]
    fn test_eocd_zip64_indicator() {
        let eocdr = Eocdr {
            total_entries: 70000,
            cd_size: 5_000_000_000,
            cd_offset: 6_000_000_000,
            comment: None,
        };
        let data = eocdr.serialize();
        assert_eq!(read_u16(&data, 8), 0xFFFF);
        assert_eq!(read_u16(&data, 10), 0xFFFF);
        assert_eq!(read_u32(&data, 12), u32::MAX);
        assert_eq!(read_u32(&data, 16), u32::MAX);
    }

    #[test]
    fn test_eocd_zip64_eocdr() {
        let z64 = Zip64Eocdr {
            total_entries: 70000,
            cd_size: 5_000_000_000,
            cd_offset: 6_000_000_000,
        };
        let data = z64.serialize();
        assert_eq!(data.len(), 56);
        assert_eq!(read_u32(&data, 0), EOCDR64_SIG);
        assert_eq!(read_u64(&data, 4), 44);
        assert_eq!(read_u16(&data, 12), VERSION_ZIP64);
        assert_eq!(read_u16(&data, 14), VERSION_ZIP64);
        assert_eq!(read_u64(&data, 24), 70000);
        assert_eq!(read_u64(&data, 32), 70000);
        assert_eq!(read_u64(&data, 40), 5_000_000_000);
        assert_eq!(read_u64(&data, 48), 6_000_000_000);
    }

    #[test]
    fn test_eocd_zip64_locator() {
        let loc = Zip64EocdrLocator {
            eocdr64_offset: 6_000_000_000,
        };
        let data = loc.serialize();
        assert_eq!(data.len(), 20);
        assert_eq!(read_u32(&data, 0), EOCDR64L_SIG);
        assert_eq!(read_u32(&data, 4), 0);
        assert_eq!(read_u64(&data, 8), 6_000_000_000);
        assert_eq!(read_u32(&data, 16), 1);
    }
}
