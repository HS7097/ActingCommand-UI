// SPDX-License-Identifier: GPL-3.0-only
//! Where the wizard may install: a new single-location root. Each rule has its own sentence,
//! shown under the folder box; [`text_issues`] are the rules about the path's text alone,
//! [`check_root`] adds those that look at the disk.
//!
//! - an absolute drive path (a network or device path is refused: the Runtime's starts go
//!   through WMI and `cmd.exe`);
//! - not a drive's root, and its last name is not `A` or `B` in any case (Runtime
//!   `crates/actingcommand-contract/src/installation.rs` `process_slot_lock` reads a program
//!   whose root's folder is named so as an A/B slot);
//! - no `'` (the console's settings file holds paths as TOML literal strings) and no `%`
//!   (`cmd.exe` expands it);
//! - absent, or an empty folder;
//! - inside no other root, holding no other root: no ancestor with `runtime\BUILD-MANIFEST.json`
//!   or `install\active.json` (an empty folder holds nothing, so it holds no root);
//! - enough free space on its drive.

use std::fs;
use std::io;
use std::path::Path;

use crate::platform;
use crate::text::Said;
use crate::winpath::{drive_absolute, last_segment, network_or_device};

/// One rule a chosen folder breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootIssue {
    Empty,
    NotAbsolute,
    NetworkPath,
    DriveRoot,
    SlotName,
    Quote,
    Percent,
    NotAFolder,
    NotEmpty,
    Unreadable(String),
    InsideRoot(String),
    NoSpace { needed: u64, free: u64 },
    SpaceUnknown(String),
}

impl RootIssue {
    pub fn said(&self) -> Said {
        match self {
            Self::Empty => Said::new("请选一个文件夹", "Choose a folder"),
            Self::NotAbsolute => Said::new(
                "要写完整路径，从盘符开始，例如 D:\\ActingCommand",
                "Give the full path from the drive letter, for example D:\\ActingCommand",
            ),
            Self::NetworkPath => Said::new(
                "不能装到网络路径或设备路径上，请选本机硬盘上的文件夹",
                "A network or device path cannot be used; choose a folder on a local drive",
            ),
            Self::DriveRoot => Said::new(
                "不能直接装在盘符根目录，请在里面建一个文件夹",
                "Not the root of a drive; choose a folder inside it",
            ),
            Self::SlotName => Said::new(
                "最后一级文件夹不能叫 A 或 B（会被当成 A/B 安装的程序槽）",
                "The last folder may not be named A or B (it would be read as an A/B program slot)",
            ),
            Self::Quote => Said::new("路径里不能有单引号 '", "The path may not contain a single quote '"),
            Self::Percent => Said::new("路径里不能有百分号 %", "The path may not contain a percent sign %"),
            Self::NotAFolder => Said::new(
                "这个位置已经有一个文件，不是文件夹",
                "Something that is not a folder is already at this path",
            ),
            Self::NotEmpty => Said::new(
                "这个文件夹里已经有东西；请选一个不存在的或空的文件夹",
                "The folder is not empty; choose a new or an empty folder",
            ),
            Self::Unreadable(detail) => {
                Said::new("这个位置读不出", "The path cannot be read").with(detail)
            }
            Self::InsideRoot(other) => Said::new(
                "这个位置在另一套 ActingCommand 的安装文件夹里面",
                "The path is inside another ActingCommand installation",
            )
            .with(other),
            // In the units the window uses: the window shows this one when the install's own
            // checks stop on it.
            Self::NoSpace { needed, free } => Said::new(
                format!("磁盘空间不够：需要约 {}，只剩 {}", plain_size(*needed), plain_size(*free)),
                format!(
                    "Not enough disk space: about {} needed, {} free",
                    plain_size(*needed),
                    plain_size(*free)
                ),
            ),
            Self::SpaceUnknown(detail) => {
                Said::new("剩余空间读不出", "The free space cannot be read").with(detail)
            }
        }
    }
}

/// `n` bytes in MiB or GiB, as the log and the command line write them.
pub fn size_text(bytes: u64) -> String {
    sized(bytes, "GiB", "MiB")
}

/// `n` bytes as the window says them: counted in 1024s like [`size_text`], named GB and MB as
/// Explorer names them, so the numbers match what a person sees there.
pub fn plain_size(bytes: u64) -> String {
    sized(bytes, "GB", "MB")
}

fn sized(bytes: u64, large: &str, small: &str) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let mib = bytes as f64 / MIB;
    if mib >= 1024.0 {
        format!("{:.1} {large}", mib / 1024.0)
    } else {
        format!("{mib:.0} {small}")
    }
}

/// The rules about the path's text alone.
pub fn text_issues(text: &str) -> Vec<RootIssue> {
    let text = text.trim();
    if text.is_empty() {
        return vec![RootIssue::Empty];
    }
    let mut issues = Vec::new();
    if network_or_device(text) {
        issues.push(RootIssue::NetworkPath);
    } else if !drive_absolute(text) {
        issues.push(RootIssue::NotAbsolute);
    } else {
        match last_segment(text) {
            None => issues.push(RootIssue::DriveRoot),
            Some(last) if last.eq_ignore_ascii_case("A") || last.eq_ignore_ascii_case("B") => {
                issues.push(RootIssue::SlotName)
            }
            Some(_) => {}
        }
    }
    if text.contains('\'') {
        issues.push(RootIssue::Quote);
    }
    if text.contains('%') {
        issues.push(RootIssue::Percent);
    }
    issues
}

/// Whether `dir` is an installation root: a single-location root's Runtime manifest, or an A/B
/// root's selection (Runtime `contracts/install-cli.md`, "single-location root" and the A/B
/// marker).
pub fn is_installation_root(dir: &Path) -> bool {
    dir.join("runtime").join("BUILD-MANIFEST.json").is_file()
        || dir.join("install").join("active.json").is_file()
}

/// Every rule `root` breaks, with `needed` bytes of free space required; an empty list is a
/// usable root.
pub fn check_root(root: &Path, needed: u64) -> Vec<RootIssue> {
    let text = root.to_string_lossy().into_owned();
    let mut issues = text_issues(&text);
    if !issues.is_empty() {
        return issues;
    }
    match fs::metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => issues.push(RootIssue::Unreadable(format!("{}: {error}", root.display()))),
        Ok(meta) if !meta.is_dir() => issues.push(RootIssue::NotAFolder),
        Ok(_) => match fs::read_dir(root) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    issues.push(RootIssue::NotEmpty);
                }
            }
            Err(error) => {
                issues.push(RootIssue::Unreadable(format!("{}: {error}", root.display())))
            }
        },
    }
    for ancestor in root.ancestors().skip(1) {
        if is_installation_root(ancestor) {
            issues.push(RootIssue::InsideRoot(ancestor.display().to_string()));
            break;
        }
    }
    match platform::free_space(root) {
        Ok(free) if free < needed => issues.push(RootIssue::NoSpace { needed, free }),
        Ok(_) => {}
        Err(reason) => issues.push(RootIssue::SpaceUnknown(reason.to_string())),
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_drive_folder_passes_the_text_rules() {
        for good in [
            "D:\\ActingCommand",
            "d:/ActingCommand/测试",
            "E:\\AC Test\\root\\",
            "C:\\Users\\x\\AB",
            "D:\\a\\Bee",
        ] {
            assert_eq!(text_issues(good), Vec::new(), "{good}");
        }
    }

    #[test]
    fn each_text_rule_has_its_issue() {
        assert_eq!(text_issues("  "), vec![RootIssue::Empty]);
        assert_eq!(text_issues("ActingCommand"), vec![RootIssue::NotAbsolute]);
        assert_eq!(text_issues("\\ActingCommand"), vec![RootIssue::NotAbsolute]);
        assert_eq!(text_issues("D:ActingCommand"), vec![RootIssue::NotAbsolute]);
        assert_eq!(text_issues("\\\\server\\share\\ac"), vec![RootIssue::NetworkPath]);
        assert_eq!(text_issues("\\\\?\\D:\\ac"), vec![RootIssue::NetworkPath]);
        assert_eq!(text_issues("D:\\"), vec![RootIssue::DriveRoot]);
        assert_eq!(text_issues("D:/"), vec![RootIssue::DriveRoot]);
        assert_eq!(text_issues("D:\\ac\\A"), vec![RootIssue::SlotName]);
        assert_eq!(text_issues("D:\\ac\\b\\"), vec![RootIssue::SlotName]);
        assert_eq!(text_issues("D:\\Alice's"), vec![RootIssue::Quote]);
        assert_eq!(text_issues("D:\\100%"), vec![RootIssue::Percent]);
        assert_eq!(
            text_issues("D:\\it's 100%\\a"),
            vec![RootIssue::SlotName, RootIssue::Quote, RootIssue::Percent]
        );
    }

    #[test]
    fn every_issue_says_something_in_both_languages() {
        for issue in [
            RootIssue::Empty,
            RootIssue::NotAbsolute,
            RootIssue::NetworkPath,
            RootIssue::DriveRoot,
            RootIssue::SlotName,
            RootIssue::Quote,
            RootIssue::Percent,
            RootIssue::NotAFolder,
            RootIssue::NotEmpty,
            RootIssue::Unreadable("x".into()),
            RootIssue::InsideRoot("X:\\other".into()),
            RootIssue::NoSpace { needed: 3 << 30, free: 1 << 30 },
            RootIssue::SpaceUnknown("x".into()),
        ] {
            let said = issue.said();
            assert!(!said.zh.is_empty() && !said.en.is_empty(), "{issue:?}");
        }
        assert_eq!(size_text(3 << 30), "3.0 GiB");
        assert_eq!(size_text(512 << 20), "512 MiB");
    }

    #[test]
    fn the_window_counts_in_1024s_and_says_gb_and_mb() {
        assert_eq!(plain_size(3 << 30), "3.0 GB");
        assert_eq!(plain_size((3 << 30) / 2), "1.5 GB");
        assert_eq!(plain_size(512 << 20), "512 MB");
        assert_eq!(plain_size(1 << 30), "1.0 GB");
    }
}
