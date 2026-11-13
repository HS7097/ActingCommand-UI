// SPDX-License-Identifier: GPL-3.0-only
//! The name a person gives an emulator instance, its alias, and the rules it must keep.
//!
//! The configuration takes any alias of 1 to 256 bytes without a control character (Runtime
//! `crates/actingcommand-contract/src/runtime.rs` `validate_instance_alias`). The schedule is
//! stricter: when the standard-pack composer copies one of a game's rules, pools, activity
//! profiles or timeline events to two or more instances, each copy's id becomes `<id>@<alias>`
//! (Runtime `contracts/standard-pack-schedule.md`, step 4 "Ids"), and such an id holds no
//! whitespace and is at most 128 bytes (`contracts/scheduling/v2/common.schema.json` `itemId`;
//! `crates/policy/src/validation.rs` `validate_item_identifier`, which tests whitespace with
//! `char::is_whitespace`). So an alias holds no whitespace either, and is at most as long as
//! the game's longest id in its `schedule.json` leaves room for: 128 bytes less that id and the
//! `@`. The rule holds whether or not a second instance is bound today, so adding one later
//! does not make an existing alias unusable.

use serde_json::Value;

use crate::text::Said;

/// The Runtime's bound on an alias (`MAX_INSTANCE_ALIAS_BYTES`).
pub const MAX_ALIAS_BYTES: usize = 256;
/// The Runtime's bound on a task, pool, profile or event id, an instance copy's included.
pub const MAX_ITEM_ID_BYTES: usize = 128;
/// The four lists of `schedule.json` whose items carry an `id`.
pub const SCHEDULE_LISTS: [&str; 4] = ["rules", "pools", "activity", "timeline"];

/// Why an alias cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasIssue {
    Empty,
    Whitespace,
    Control,
    /// Longer than `limit` bytes.
    TooLong { limit: usize },
}

impl AliasIssue {
    /// The reason in words a person reads, without the name itself.
    pub fn reason(self) -> Said {
        match self {
            Self::Empty => Said::new("不能空着", "it is empty"),
            Self::Whitespace => Said::new("名字里不能有空格", "it cannot contain spaces"),
            Self::Control => Said::new(
                "名字里有看不见的控制字符，请重新输入",
                "it contains an invisible control character; type it again",
            ),
            Self::TooLong { limit } => Said::new(
                format!("太长了，最多 {limit} 个英文字母或数字，或者 {} 个汉字", limit / 3),
                format!("it is too long: at most {limit} letters or digits, or {} Chinese characters", limit / 3),
            ),
        }
    }
}

/// The most bytes an alias may take beside a game whose longest schedule item id is
/// `longest_item_id` bytes (0: the game has no schedule item, so no copy is ever named).
pub fn alias_limit(longest_item_id: usize) -> usize {
    if longest_item_id == 0 {
        MAX_ALIAS_BYTES
    } else {
        MAX_ITEM_ID_BYTES
            .saturating_sub(longest_item_id + 1)
            .min(MAX_ALIAS_BYTES)
    }
}

/// What is wrong with `alias` for a game whose alias limit is `limit` bytes, or `None`.
pub fn alias_issue(alias: &str, limit: usize) -> Option<AliasIssue> {
    if alias.is_empty() {
        return Some(AliasIssue::Empty);
    }
    if alias.chars().any(char::is_whitespace) {
        return Some(AliasIssue::Whitespace);
    }
    if alias.chars().any(char::is_control) {
        return Some(AliasIssue::Control);
    }
    let limit = limit.min(MAX_ALIAS_BYTES);
    if alias.len() > limit {
        return Some(AliasIssue::TooLong { limit });
    }
    None
}

/// The whole sentence for an alias that cannot be used: "名字 {名字} 用不了：{原因}", or, for an
/// empty one, that an instance has no name.
pub fn alias_problem(alias: &str, limit: usize) -> Option<Said> {
    let issue = alias_issue(alias, limit)?;
    if issue == AliasIssue::Empty {
        return Some(Said::new("有实例的名字空着，请给它起一个名字", "An instance has no name; give it one"));
    }
    let shown = if issue == AliasIssue::Control {
        alias.escape_debug().to_string()
    } else {
        alias.to_owned()
    };
    let reason = issue.reason();
    Some(Said::new(
        format!("名字 {shown} 用不了：{}", reason.zh),
        format!("The name {shown} cannot be used: {}", reason.en),
    ))
}

/// The bytes of the longest `id` among the rules, pools, activity profiles and timeline events
/// of a game's `schedule.json` (a UTF-8 byte-order mark allowed); 0 when it has none. Each list
/// may be absent (Runtime `contracts/standard-pack-schedule.md`: absent means empty); one that is
/// present is an array of objects with a string `id`. The Runtime's own reader checks the rest.
pub fn longest_item_id(bytes: &[u8]) -> Result<usize, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let unreadable = |detail: String| Said::new("schedule.json 读不懂", "schedule.json does not parse").with(detail);
    let document: Value = serde_json::from_slice(bytes).map_err(|error| unreadable(error.to_string()))?;
    let Some(object) = document.as_object() else {
        return Err(unreadable("not an object".to_owned()));
    };
    let mut longest = 0usize;
    for list in SCHEDULE_LISTS {
        let Some(items) = object.get(list) else {
            continue;
        };
        let items = items
            .as_array()
            .ok_or_else(|| unreadable(format!("{list} is not an array")))?;
        for (at, item) in items.iter().enumerate() {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| unreadable(format!("{list}[{at}] has no string id")))?;
            longest = longest.max(id.len());
        }
    }
    Ok(longest)
}

/// The alias the window offers for MuMu instance `index` playing a game: `<name>-<index>`, the
/// name being the game's display name `label`, else its folder name `folder`, without
/// whitespace or control characters, and cut at a character so the whole stays within `limit`
/// bytes.
pub fn default_alias(label: &str, folder: &str, index: u16, limit: usize) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .filter(|character| !character.is_whitespace() && !character.is_control())
            .collect()
    };
    let mut name = clean(label);
    if name.is_empty() {
        name = clean(folder);
    }
    if name.is_empty() {
        name = "instance".to_owned();
    }
    let suffix = format!("-{index}");
    let room = limit.min(MAX_ALIAS_BYTES).saturating_sub(suffix.len());
    if room == 0 {
        return index.to_string();
    }
    let mut end = 0;
    for (at, character) in name.char_indices() {
        let next = at + character.len_utf8();
        if next > room {
            break;
        }
        end = next;
    }
    if end == 0 {
        return index.to_string();
    }
    format!("{}{suffix}", &name[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_keeps_the_schedule_ids_rules() {
        assert_eq!(alias_issue("碧蓝航线-5", 97), None);
        assert_eq!(alias_issue("主号", 6), None);
        assert_eq!(alias_issue("", 97), Some(AliasIssue::Empty));
        for spaced in ["a b", "a\u{3000}b", "a\u{a0}b", " a", "a\tb", "a\nb"] {
            assert_eq!(alias_issue(spaced, 97), Some(AliasIssue::Whitespace), "{spaced:?}");
        }
        assert_eq!(alias_issue("a\u{7}b", 97), Some(AliasIssue::Control));
        assert_eq!(alias_issue("a\u{85}b", 97), Some(AliasIssue::Whitespace));
        assert_eq!(alias_issue("a\u{9f}b", 97), Some(AliasIssue::Control));
        // Bytes, not characters: three Chinese characters are nine bytes.
        assert_eq!(alias_issue("主号号", 9), None);
        assert_eq!(alias_issue("主号号", 8), Some(AliasIssue::TooLong { limit: 8 }));
        assert_eq!(alias_issue(&"a".repeat(97), 97), None);
        assert_eq!(alias_issue(&"a".repeat(98), 97), Some(AliasIssue::TooLong { limit: 97 }));
        // The configuration's own bound stays.
        assert_eq!(
            alias_issue(&"a".repeat(257), 1000),
            Some(AliasIssue::TooLong { limit: 256 })
        );
        // An alias with `@` still leaves the id's first `@` between the id and the alias.
        assert_eq!(alias_issue("main@cn", 97), None);
    }

    #[test]
    fn the_limit_leaves_room_for_the_longest_id_and_the_at_sign() {
        assert_eq!(alias_limit(0), 256);
        assert_eq!(alias_limit(30), 97);
        assert_eq!(alias_limit(126), 1);
        assert_eq!(alias_limit(127), 0);
        assert_eq!(alias_limit(300), 0);
        // An id of the longest length with the longest alias is exactly 128 bytes.
        let id = "x".repeat(40);
        let alias = "y".repeat(alias_limit(id.len()));
        assert_eq!(format!("{id}@{alias}").len(), MAX_ITEM_ID_BYTES);
    }

    #[test]
    fn every_problem_is_one_sentence_in_both_languages() {
        assert_eq!(alias_problem("主号", 97), None);
        let empty = alias_problem("", 97).expect("empty");
        assert!(!empty.zh.is_empty() && !empty.en.is_empty());
        let spaced = alias_problem("main one", 97).expect("spaced");
        assert!(spaced.zh.starts_with("名字 main one 用不了："), "{}", spaced.zh);
        assert!(spaced.en.starts_with("The name main one cannot be used: "), "{}", spaced.en);
        let long = alias_problem(&"a".repeat(10), 9).expect("long");
        assert!(long.zh.contains("最多 9 个") && long.en.contains("at most 9 "), "{long}");
        let control = alias_problem("a\u{7}", 97).expect("control");
        assert!(control.en.contains("a\\u{7}"), "{control}");
    }

    #[test]
    fn the_longest_id_is_read_from_all_four_lists() {
        let schedule = br#"{
            "schema_version": "actingcommand.schedule.v1",
            "rules": [{"id": "azurlane.daily", "package_id": "p"}, {"id": "azurlane.commission.collect", "package_id": "p"}],
            "pools": [{"id": "azurlane.oil"}],
            "activity": [{"id": "azurlane.activity.a-rather-long-profile-name"}],
            "timeline": [{"id": "azurlane.reset"}]
        }"#;
        assert_eq!(longest_item_id(schedule), Ok("azurlane.activity.a-rather-long-profile-name".len()));
        let mut with_mark = b"\xef\xbb\xbf".to_vec();
        with_mark.extend_from_slice(br#"{"schema_version": "actingcommand.schedule.v1", "rules": [{"id": "abc"}]}"#);
        assert_eq!(longest_item_id(&with_mark), Ok(3));
        assert_eq!(longest_item_id(br#"{"schema_version": "actingcommand.schedule.v1"}"#), Ok(0));
        let broken: [&[u8]; 5] = [
            b"not json",
            br#"[1]"#,
            br#"{"rules": {"id": "x"}}"#,
            br#"{"pools": [{"name": "x"}]}"#,
            br#"{"timeline": [{"id": 3}]}"#,
        ];
        for broken in broken {
            assert!(longest_item_id(broken).is_err(), "{}", String::from_utf8_lossy(broken));
        }
    }

    #[test]
    fn the_default_alias_is_the_game_and_the_index_within_the_limit() {
        assert_eq!(default_alias("碧蓝航线", "azurlane", 5, 97), "碧蓝航线-5");
        assert_eq!(default_alias("Azur Lane", "azurlane", 12, 97), "AzurLane-12");
        assert_eq!(default_alias(" \t", "azurlane", 0, 97), "azurlane-0");
        assert_eq!(default_alias("", "", 3, 97), "instance-3");
        // Cut at a character: "碧蓝" is 6 bytes, "-5" two more.
        assert_eq!(default_alias("碧蓝航线", "azurlane", 5, 9), "碧蓝-5");
        assert_eq!(default_alias("碧蓝航线", "azurlane", 5, 8), "碧蓝-5");
        assert_eq!(default_alias("碧蓝航线", "azurlane", 5, 7), "碧-5");
        assert_eq!(default_alias("碧蓝航线", "azurlane", 5, 4), "5");
        for (label, limit) in [("碧蓝航线", 97), ("Azur Lane", 5), ("明日方舟", 10), ("x", 256)] {
            for index in [0u16, 7, 65535] {
                let alias = default_alias(label, "folder", index, limit);
                if limit >= 2 + index.to_string().len() {
                    assert_eq!(alias_issue(&alias, limit), None, "{alias} within {limit}");
                }
            }
        }
    }
}
