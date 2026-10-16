mod binary;
mod constants;
mod extra_fields;
mod headers;
mod time;

pub(crate) use constants::*;
pub(crate) use extra_fields::*;
pub(crate) use headers::*;
pub(crate) use time::*;

/// Whether the UTF-8 (EFS) general-purpose flag must be set for an entry.
///
/// Bit 11 applies to both the filename and the comment, so an entry needs it
/// when either of them is non-ASCII (APPNOTE 4.4.4).
pub(crate) fn needs_utf8_flag(name: &str, comment: Option<&[u8]>) -> bool {
    !name.is_ascii() || comment.is_some_and(|c| !c.is_ascii())
}
