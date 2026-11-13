// SPDX-License-Identifier: GPL-3.0-only
//! The zips of an install set, read entry by entry: every name checked to stay inside the zip's
//! root (a backslash is read as a separator, as some zip writers put it), each entry hashed or
//! written out without ever overwriting a file. The zip's own CRC-32 is checked by the reader
//! as each entry is read.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::digest::Hasher;
use crate::text::Said;

/// The most bytes a small member (a manifest, an index, an applications table) may have.
pub const SMALL_LIMIT: u64 = 16 << 20;

/// Whether `text` is a plain relative path: `/`-separated names, none empty, `.` or `..`, none
/// holding `\`, `:` or a control character.
pub fn safe_relative(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('/')
        && text.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.contains(['\\', ':'])
                && !segment.chars().any(char::is_control)
        })
}

/// The name of a zip entry as the wizard reads it: backslashes as `/`, a directory's trailing
/// `/` dropped; `None` when the name leaves the zip's root.
pub fn entry_name(raw: &str) -> Option<String> {
    let name = raw.replace('\\', "/");
    let name = name.trim_end_matches('/');
    safe_relative(name).then(|| name.to_owned())
}

/// One opened zip: its file entries by name, with their sizes.
pub struct Archive {
    path: PathBuf,
    zip: zip::ZipArchive<File>,
    files: BTreeMap<String, (usize, u64)>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self, Said> {
        let unreadable = |error: &dyn std::fmt::Display| {
            Said::new("zip 读不出", "the zip cannot be read")
                .with(format!("{}: {error}", path.display()))
        };
        let file = File::open(path).map_err(|error| unreadable(&error))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|error| unreadable(&error))?;
        let mut files = BTreeMap::new();
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|error| unreadable(&error))?;
            let raw = entry.name().to_owned();
            let Some(name) = entry_name(&raw) else {
                return Err(Said::new(
                    "zip 里有越出根目录的条目",
                    "the zip holds an entry that leaves its root",
                )
                .with(format!("{}: {raw}", path.display())));
            };
            if entry.is_dir() {
                continue;
            }
            if files.insert(name.clone(), (index, entry.size())).is_some() {
                return Err(Said::new("zip 里有重名条目", "the zip names one entry twice")
                    .with(format!("{}: {name}", path.display())));
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            zip,
            files,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every file entry's name.
    pub fn names(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    /// The uncompressed size of the entry `name`.
    pub fn size_of(&self, name: &str) -> Option<u64> {
        self.files.get(name).map(|(_, size)| *size)
    }

    fn entry(&mut self, name: &str) -> Result<zip::read::ZipFile<'_>, Said> {
        let Some((index, _)) = self.files.get(name).copied() else {
            return Err(Said::new("zip 里没有这一项", "the zip has no such entry")
                .with(format!("{}: {name}", self.path.display())));
        };
        let path = self.path.display().to_string();
        self.zip.by_index(index).map_err(|error| {
            Said::new("zip 条目读不出", "a zip entry cannot be read")
                .with(format!("{path}: {name}: {error}"))
        })
    }

    /// The bytes of a small entry, at most [`SMALL_LIMIT`].
    pub fn read_small(&mut self, name: &str) -> Result<Vec<u8>, Said> {
        let path = self.path.display().to_string();
        let entry = self.entry(name)?;
        let mut bytes = Vec::new();
        entry
            .take(SMALL_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                Said::new("zip 条目读不出", "a zip entry cannot be read")
                    .with(format!("{path}: {name}: {error}"))
            })?;
        if bytes.len() as u64 > SMALL_LIMIT {
            return Err(Said::new("zip 条目过大", "a zip entry is too large")
                .with(format!("{path}: {name}")));
        }
        Ok(bytes)
    }

    /// The SHA-256 and the size of the entry `name`, streamed.
    pub fn hash(&mut self, name: &str) -> Result<(String, u64), Said> {
        let path = self.path.display().to_string();
        let mut entry = self.entry(name)?;
        let mut hasher = Hasher::new();
        io::copy(&mut entry, &mut hasher).map_err(|error| {
            Said::new("zip 条目读不出", "a zip entry cannot be read")
                .with(format!("{path}: {name}: {error}"))
        })?;
        Ok(hasher.finish())
    }

    /// The entry `name` written to `target`, a file that must not exist yet (its folders are
    /// created); returns the bytes written. Nothing is ever overwritten.
    pub fn extract(&mut self, name: &str, target: &Path) -> Result<u64, Said> {
        let path = self.path.display().to_string();
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                Said::new("无法建文件夹", "cannot create the folder")
                    .with(format!("{}: {error}", parent.display()))
            })?;
        }
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|error| {
                Said::new("无法新建文件", "cannot create the file")
                    .with(format!("{}: {error}", target.display()))
            })?;
        let mut entry = self.entry(name)?;
        let written = io::copy(&mut entry, &mut out).map_err(|error| {
            Said::new("解压失败", "unpacking failed")
                .with(format!("{path}: {name} → {}: {error}", target.display()))
        })?;
        out.sync_all().map_err(|error| {
            Said::new("写盘失败", "the file could not be written to disk")
                .with(format!("{}: {error}", target.display()))
        })?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_stay_inside_the_zip() {
        assert_eq!(entry_name("runtime/a.exe").as_deref(), Some("runtime/a.exe"));
        // Compress-Archive of Windows PowerShell 5.1 writes backslashes.
        assert_eq!(
            entry_name("platform-tools\\adb.exe").as_deref(),
            Some("platform-tools/adb.exe")
        );
        assert_eq!(entry_name("packs/").as_deref(), Some("packs"));
        for bad in ["", "/etc/x", "../x", "a/../../x", "a//b", "C:/x", "a/./b", "a\u{0}b"] {
            assert_eq!(entry_name(bad), None, "{bad:?}");
        }
    }
}
