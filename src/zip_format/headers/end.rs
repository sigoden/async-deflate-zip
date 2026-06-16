use crate::zip_format::binary::*;
use crate::zip_format::constants::*;

pub(crate) struct Eocdr {
    pub(crate) total_entries: u64,
    pub(crate) cd_size: u64,
    pub(crate) cd_offset: u64,
    pub(crate) comment: Option<Vec<u8>>,
}

impl Eocdr {
    pub(crate) fn write_to(&self, buf: &mut Vec<u8>) {
        let use_zip64 = archive_needs_zip64(self.total_entries, self.cd_size, self.cd_offset);

        let comment = self.comment.as_deref().unwrap_or_default();

        buf.clear();
        buf.reserve(22 + comment.len());
        put_u32(buf, EOCDR_SIG);
        put_u16(buf, 0);
        put_u16(buf, 0);
        let total = if use_zip64 {
            0xFFFF
        } else {
            self.total_entries as u16
        };
        put_u16(buf, total);
        put_u16(buf, total);
        put_u32(
            buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.cd_size as u32
            },
        );
        put_u32(
            buf,
            if use_zip64 {
                u32::MAX
            } else {
                self.cd_offset as u32
            },
        );
        put_u16(buf, comment.len() as u16);
        buf.extend_from_slice(comment);
    }
}

pub(crate) struct Zip64Eocdr {
    pub(crate) total_entries: u64,
    pub(crate) cd_size: u64,
    pub(crate) cd_offset: u64,
}

impl Zip64Eocdr {
    pub(crate) fn write_to(&self, buf: &mut Vec<u8>) {
        buf.clear();
        buf.reserve(56);
        put_u32(buf, EOCDR64_SIG);
        put_u64(buf, 44);
        put_u16(buf, VERSION_ZIP64);
        put_u16(buf, VERSION_ZIP64);
        put_u32(buf, 0);
        put_u32(buf, 0);
        put_u64(buf, self.total_entries);
        put_u64(buf, self.total_entries);
        put_u64(buf, self.cd_size);
        put_u64(buf, self.cd_offset);
    }
}

pub(crate) struct Zip64EocdrLocator {
    pub(crate) eocdr64_offset: u64,
}

impl Zip64EocdrLocator {
    pub(crate) fn write_to(&self, buf: &mut Vec<u8>) {
        buf.clear();
        buf.reserve(20);
        put_u32(buf, EOCDR64L_SIG);
        put_u32(buf, 0);
        put_u64(buf, self.eocdr64_offset);
        put_u32(buf, 1);
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
        let mut buf = Vec::new();
        eocdr.write_to(&mut buf);
        assert_eq!(read_u32(&buf, 0), EOCDR_SIG);
        assert_eq!(read_u16(&buf, 8), 10);
        assert_eq!(read_u16(&buf, 10), 10);
        assert_eq!(read_u32(&buf, 12), 5000);
        assert_eq!(read_u32(&buf, 16), 10000);
    }

    #[test]
    fn test_eocd_zip64_indicator() {
        let eocdr = Eocdr {
            total_entries: 70000,
            cd_size: 5_000_000_000,
            cd_offset: 6_000_000_000,
            comment: None,
        };
        let mut buf = Vec::new();
        eocdr.write_to(&mut buf);
        assert_eq!(read_u16(&buf, 8), 0xFFFF);
        assert_eq!(read_u16(&buf, 10), 0xFFFF);
        assert_eq!(read_u32(&buf, 12), u32::MAX);
        assert_eq!(read_u32(&buf, 16), u32::MAX);
    }

    #[test]
    fn test_eocd_zip64_eocdr() {
        let z64 = Zip64Eocdr {
            total_entries: 70000,
            cd_size: 5_000_000_000,
            cd_offset: 6_000_000_000,
        };
        let mut buf = Vec::new();
        z64.write_to(&mut buf);
        assert_eq!(buf.len(), 56);
        assert_eq!(read_u32(&buf, 0), EOCDR64_SIG);
        assert_eq!(read_u64(&buf, 4), 44);
        assert_eq!(read_u16(&buf, 12), VERSION_ZIP64);
        assert_eq!(read_u16(&buf, 14), VERSION_ZIP64);
        assert_eq!(read_u64(&buf, 24), 70000);
        assert_eq!(read_u64(&buf, 32), 70000);
        assert_eq!(read_u64(&buf, 40), 5_000_000_000);
        assert_eq!(read_u64(&buf, 48), 6_000_000_000);
    }

    #[test]
    fn test_eocd_zip64_locator() {
        let loc = Zip64EocdrLocator {
            eocdr64_offset: 6_000_000_000,
        };
        let mut buf = Vec::new();
        loc.write_to(&mut buf);
        assert_eq!(buf.len(), 20);
        assert_eq!(read_u32(&buf, 0), EOCDR64L_SIG);
        assert_eq!(read_u32(&buf, 4), 0);
        assert_eq!(read_u64(&buf, 8), 6_000_000_000);
        assert_eq!(read_u32(&buf, 16), 1);
    }
}
