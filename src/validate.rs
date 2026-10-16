use crate::error::ZipError;

/// Maximum length of a ZIP field (entry name or comment) in bytes.
const MAX_FIELD_LEN: usize = u16::MAX as usize;

/// Validate a ZIP entry name.
///
/// A valid name must not be empty, must not exceed 65535 bytes,
/// and must not contain a NUL byte.
pub(crate) fn validate_entry_name(name: &str) -> Result<(), ZipError> {
    let len = name.len();
    if len == 0 {
        return Err(ZipError::InvalidInput {
            reason: "entry name is empty",
        });
    }
    if len > MAX_FIELD_LEN {
        return Err(ZipError::FieldTooLong {
            field: "entry name",
            len,
            max: MAX_FIELD_LEN,
        });
    }
    if name.as_bytes().contains(&0) {
        return Err(ZipError::InvalidInput {
            reason: "entry name contains NUL byte",
        });
    }
    Ok(())
}

/// Validate both an entry name and optional comment.
///
/// This is a convenience wrapper around [`validate_entry_name`] and
/// [`validate_comment`].
pub(crate) fn validate_input(name: &str, comment: Option<&[u8]>) -> Result<(), ZipError> {
    validate_entry_name(name)?;
    if let Some(c) = comment {
        validate_comment(c)?;
    }
    Ok(())
}

/// Validate a ZIP entry or archive comment.
///
/// A valid comment must not exceed 65535 bytes.
pub(crate) fn validate_comment(comment: &[u8]) -> Result<(), ZipError> {
    let len = comment.len();
    if len > MAX_FIELD_LEN {
        return Err(ZipError::FieldTooLong {
            field: "comment",
            len,
            max: MAX_FIELD_LEN,
        });
    }
    Ok(())
}

/// Validate a symbolic link target.
///
/// The target is stored as the entry's data, so it is not length-limited like
/// a name. It must not be empty — an empty symlink target cannot be created —
/// and must not contain a NUL byte, which the underlying OS symlink call would
/// reject.
pub(crate) fn validate_symlink_target(target: &str) -> Result<(), ZipError> {
    if target.is_empty() {
        return Err(ZipError::InvalidInput {
            reason: "symlink target is empty",
        });
    }
    if target.as_bytes().contains(&0) {
        return Err(ZipError::InvalidInput {
            reason: "symlink target contains NUL byte",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_entry_name_ok() {
        assert!(validate_entry_name("hello.txt").is_ok());
        assert!(validate_entry_name("a").is_ok());
        assert!(validate_entry_name("path/to/file.txt").is_ok());
    }

    #[test]
    fn test_validate_entry_name_empty() {
        let err = validate_entry_name("").unwrap_err();
        assert!(
            matches!(&err, ZipError::InvalidInput { reason } if *reason == "entry name is empty"),
            "expected InvalidInput, got {err}"
        );
    }

    #[test]
    fn test_validate_entry_name_nul() {
        let err = validate_entry_name("bad\0name").unwrap_err();
        assert!(
            matches!(&err, ZipError::InvalidInput { reason } if *reason == "entry name contains NUL byte"),
            "expected InvalidInput, got {err}"
        );
    }

    #[test]
    fn test_validate_entry_name_too_long() {
        let name = "a".repeat(MAX_FIELD_LEN + 1);
        let err = validate_entry_name(&name).unwrap_err();
        assert!(
            matches!(&err, ZipError::FieldTooLong { field, len, max }
                if *field == "entry name" && *len == MAX_FIELD_LEN + 1 && *max == MAX_FIELD_LEN),
            "expected FieldTooLong, got {err}"
        );
    }

    #[test]
    fn test_validate_input_long_comment() {
        let comment = vec![b'x'; MAX_FIELD_LEN + 1];
        let err = validate_input("file.txt", Some(&comment)).unwrap_err();
        assert!(matches!(err, ZipError::FieldTooLong { .. }));
    }

    #[test]
    fn test_validate_comment_ok() {
        assert!(validate_comment(b"").is_ok());
        assert!(validate_comment(b"hello").is_ok());
    }

    #[test]
    fn test_validate_comment_too_long() {
        let comment = vec![b'x'; MAX_FIELD_LEN + 1];
        let err = validate_comment(&comment).unwrap_err();
        assert!(
            matches!(&err, ZipError::FieldTooLong { field, len, max }
                if *field == "comment" && *len == MAX_FIELD_LEN + 1 && *max == MAX_FIELD_LEN),
            "expected FieldTooLong, got {err}"
        );
    }

    #[test]
    fn test_validate_symlink_target_ok() {
        assert!(validate_symlink_target("target.txt").is_ok());
        assert!(validate_symlink_target("../dir/file").is_ok());
        assert!(validate_symlink_target("/absolute/path").is_ok());
    }

    #[test]
    fn test_validate_symlink_target_empty() {
        let err = validate_symlink_target("").unwrap_err();
        assert!(
            matches!(&err, ZipError::InvalidInput { reason } if *reason == "symlink target is empty"),
            "expected InvalidInput, got {err}"
        );
    }

    #[test]
    fn test_validate_symlink_target_nul() {
        let err = validate_symlink_target("bad\0target").unwrap_err();
        assert!(
            matches!(&err, ZipError::InvalidInput { reason } if *reason == "symlink target contains NUL byte"),
            "expected InvalidInput, got {err}"
        );
    }
}
