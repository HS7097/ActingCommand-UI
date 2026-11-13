// SPDX-License-Identifier: GPL-3.0-only
//! Windows path text, read the same on every platform so the rules about a root can be judged
//! (and tested) anywhere: `X:\a\b` and `X:/a/b` are one path, `\\?\` is the verbatim prefix
//! `std::fs::canonicalize` returns on Windows.

/// Whether `text` is a drive-absolute path, `X:\…` or `X:/…`.
pub fn drive_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Whether `text` starts like a UNC or device path (`\\server\…`, `\\?\…`, `//…`).
pub fn network_or_device(text: &str) -> bool {
    text.starts_with("\\\\") || text.starts_with("//")
}

/// The path's names after its drive, `\` and `/` alike, empty names skipped.
pub fn segments(text: &str) -> Vec<&str> {
    let body = if drive_absolute(text) || (text.len() == 2 && text.as_bytes()[1] == b':') {
        &text[2..]
    } else {
        text
    };
    body.split(['\\', '/'])
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// The last name of the path, `None` for a drive root.
pub fn last_segment(text: &str) -> Option<&str> {
    segments(text).last().copied()
}

/// `text` without the verbatim prefix: `\\?\C:\x` is `C:\x`, `\\?\UNC\s\h` is `\\s\h`; any other
/// text as it is.
pub fn strip_verbatim(text: &str) -> String {
    if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{rest}");
    }
    match text.strip_prefix("\\\\?\\") {
        Some(rest) if drive_absolute(rest) => rest.to_owned(),
        _ => text.to_owned(),
    }
}

/// Two paths' texts compared as Windows compares them: separators alike, ASCII case folded,
/// trailing separators ignored.
pub fn same_path(left: &str, right: &str) -> bool {
    let fold = |text: &str| -> String {
        strip_verbatim(text)
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    fold(left) == fold(right)
}
