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

/// A path's text as Windows compares it: without the verbatim prefix, `\` for `/`, ASCII case
/// folded, no trailing separator.
pub fn fold(text: &str) -> String {
    strip_verbatim(text.trim())
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// Two paths' texts compared as Windows compares them: separators alike, ASCII case folded,
/// trailing separators ignored.
pub fn same_path(left: &str, right: &str) -> bool {
    fold(left) == fold(right)
}

/// Whether the path `inner` is `outer` itself or lies below it, compared as [`same_path`]
/// compares (`X:\ab` is not below `X:\a`).
pub fn within(inner: &str, outer: &str) -> bool {
    let (inner, outer) = (fold(inner), fold(outer));
    if outer.is_empty() {
        return false;
    }
    inner == outer || inner.strip_prefix(&outer).is_some_and(|rest| rest.starts_with('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_paths_and_network_paths_are_told_apart() {
        for absolute in [r"X:\Root", "x:/Root", r"X:\"] {
            assert!(drive_absolute(absolute), "{absolute}");
        }
        for not in ["X:", "X:Root", r"\Root", "Root", "", r"\\server\share", "1:\\Root"] {
            assert!(!drive_absolute(not), "{not}");
        }
        for network in [r"\\server\share", r"\\?\X:\Root", "//server/share"] {
            assert!(network_or_device(network), "{network}");
        }
        assert!(!network_or_device(r"X:\Root"));
        assert!(!network_or_device(r"\Root"));
    }

    #[test]
    fn segments_skip_the_drive_and_empty_names() {
        assert_eq!(segments(r"X:\Root\Games"), vec!["Root", "Games"]);
        assert_eq!(segments("X:/Root//Games/"), vec!["Root", "Games"]);
        assert_eq!(segments("X:"), Vec::<&str>::new());
        assert_eq!(segments(r"X:\"), Vec::<&str>::new());
        assert_eq!(segments(r"Root\Games"), vec!["Root", "Games"]);
        assert_eq!(last_segment(r"X:\Root\Games\"), Some("Games"));
        assert_eq!(last_segment(r"X:\"), None);
    }

    #[test]
    fn the_verbatim_prefix_is_dropped_only_before_a_drive_or_unc() {
        assert_eq!(strip_verbatim(r"\\?\X:\Root"), r"X:\Root");
        assert_eq!(strip_verbatim(r"\\?\UNC\server\share"), r"\\server\share");
        assert_eq!(strip_verbatim(r"\\?\Volume{1}\x"), r"\\?\Volume{1}\x");
        assert_eq!(strip_verbatim(r"X:\Root"), r"X:\Root");
    }

    #[test]
    fn paths_compare_as_windows_compares_them() {
        assert_eq!(fold(r" \\?\X:/Root/Games\ "), r"x:\root\games");
        assert!(same_path(r"X:\Root\Games", "x:/root/games/"));
        assert!(same_path(r"\\?\X:\Root", r"X:\ROOT\"));
        assert!(!same_path(r"X:\Root", r"X:\Root2"));
        assert!(within(r"X:\Root", r"X:\Root"));
        assert!(within(r"x:/root/games/a", r"X:\Root\"));
        assert!(!within(r"X:\Root2", r"X:\Root"));
        assert!(!within(r"X:\Ro", r"X:\Root"));
        assert!(!within(r"X:\Root", ""));
    }
}
