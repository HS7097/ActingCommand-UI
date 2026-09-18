// SPDX-License-Identifier: GPL-3.0-only
//! The install log: `<root>\acsetup-<unix_ms>.log`, one human-readable line
//! per thing the wizard did or found, appended as it happens. It is created
//! at the verify step, the first moment the wizard writes anything at all,
//! and it never holds the salt.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct InstallLog {
    path: PathBuf,
    file: File,
}

impl InstallLog {
    /// A new `<root>\acsetup-<unix_ms>.log`; the root must exist.
    pub fn create(root: &Path, unix_ms: u128) -> io::Result<Self> {
        let path = root.join(format!("acsetup-{unix_ms}.log"));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One line, flushed at once: the log must be complete at the moment the
    /// window tells a person to read it.
    pub fn line(&mut self, text: &str) -> io::Result<()> {
        self.file.write_all(text.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()
    }
}

/// Milliseconds since the Unix epoch: the one run's mark on its log and its
/// staging directory.
pub fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}
