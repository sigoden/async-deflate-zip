use std::borrow::Cow;

fn is_windows_drive_letter(component: &str) -> bool {
    component.len() == 2
        && component.as_bytes()[0].is_ascii_alphabetic()
        && component.as_bytes()[1] == b':'
}

pub(crate) fn sanitize_path(path: &str) -> Cow<'_, str> {
    if !path.contains('\\') {
        let mut clean = true;
        for component in path.split('/') {
            match component {
                "" | "." | ".." => {
                    clean = false;
                    break;
                }
                _ if is_windows_drive_letter(component) => {
                    clean = false;
                    break;
                }
                _ => {}
            }
        }
        if clean {
            return Cow::Borrowed(path);
        }
    }

    let path = path.replace('\\', "/");
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            _ if is_windows_drive_letter(component) => {}
            _ => {
                components.push(component);
            }
        }
    }
    Cow::Owned(components.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_path() {
        let cases = [
            // normal paths
            ("foo.txt", "foo.txt"),
            ("foo/bar.txt", "foo/bar.txt"),
            ("foo/bar/baz.txt", "foo/bar/baz.txt"),
            // dot components
            ("./foo.txt", "foo.txt"),
            ("./foo/bar.txt", "foo/bar.txt"),
            ("foo/./bar.txt", "foo/bar.txt"),
            // parent traversal that escapes root -> remaining components
            ("../foo.txt", "foo.txt"),
            ("../foo/bar.txt", "foo/bar.txt"),
            ("../../foo.txt", "foo.txt"),
            ("foo/../bar.txt", "bar.txt"),
            ("foo/../../bar.txt", "bar.txt"),
            // absolute paths
            ("/foo/bar.txt", "foo/bar.txt"),
            ("///foo/bar.txt", "foo/bar.txt"),
            // mixed slashes
            ("foo//bar.txt", "foo/bar.txt"),
            ("foo///bar///baz.txt", "foo/bar/baz.txt"),
            // windows style
            (r"foo\bar.txt", "foo/bar.txt"),
            (r"..\..\foo.txt", "foo.txt"),
            // windows absolute
            (r"C:\foo\bar.txt", "foo/bar.txt"),
            (r"D:\tmp\test.txt", "tmp/test.txt"),
            // only dangerous
            ("..", ""),
            ("../..", ""),
            (".", ""),
            ("././.", ""),
            ("/", ""),
            // dots and traversal mixed
            ("./../foo.txt", "foo.txt"),
            ("foo/..", ""),
            ("foo/./../bar.txt", "bar.txt"),
        ];

        for (input, expected) in cases {
            assert_eq!(sanitize_path(input), expected, "input={input:?}");
        }
    }
}
