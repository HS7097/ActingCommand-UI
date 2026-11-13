// SPDX-License-Identifier: GPL-3.0-only
//! The console's settings file of this root, `<root>\acui.toml`, which the console reads in its
//! root mode. The wizard writes only `lang`, the language it ran in; every other key keeps the
//! console's default by being absent, and no path key is written (the console takes its paths
//! from the root). The format is the console's own (`crates/acui-app/src/settings.rs`: its
//! header line, `key = "value"`). The global `%APPDATA%\ActingCommand\acui.toml` is never
//! written.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::text::{Lang, Said};

pub const SETTINGS_FILE: &str = "acui.toml";
/// The first line of a settings file the console creates (`settings.rs` `HEADER`).
const HEADER: &str = "# ActingCommand 监控台 / ActingCommand Console";

/// The file's whole text.
pub fn settings_text(lang: Lang) -> String {
    format!("{HEADER}\nlang = \"{}\"\n", lang.wire())
}

/// Writes `<root>\acui.toml`, a new file.
pub fn write_settings(root: &Path, lang: Lang) -> Result<PathBuf, Said> {
    let path = root.join(SETTINGS_FILE);
    let failed = |error: &dyn std::fmt::Display| {
        Said::new("写不进监控台设置文件", "the console's settings file cannot be written")
            .with(format!("{}: {error}", path.display()))
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| failed(&error))?;
    file.write_all(settings_text(lang).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| failed(&error))?;
    Ok(path)
}
