// SPDX-License-Identifier: AGPL-3.0-only
//! The one file this program reads and writes itself: its own two preferences,
//! in the per-user config directory. Never inside a state root, and never
//! anything else — a state root is the read face's to touch, not this one's.
//!
//! `<config_dir>/ActingCommand/acui.toml`, with `lang` and `text_size`.

use std::path::PathBuf;

use crate::strings::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSize {
    Standard,
    Large,
    ExtraLarge,
}

impl TextSize {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Large => "large",
            Self::ExtraLarge => "extra-large",
        }
    }

    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "standard" => Some(Self::Standard),
            "large" => Some(Self::Large),
            "extra-large" => Some(Self::ExtraLarge),
            _ => None,
        }
    }

    pub const fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Large,
            2 => Self::ExtraLarge,
            _ => Self::Standard,
        }
    }

    pub const fn index(self) -> i32 {
        match self {
            Self::Standard => 0,
            Self::Large => 1,
            Self::ExtraLarge => 2,
        }
    }

    /// The one number every font size and row height in `app.slint` is scaled by.
    pub const fn factor(self) -> f32 {
        match self {
            Self::Standard => 1.0,
            Self::Large => 1.25,
            Self::ExtraLarge => 1.5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub language: Language,
    pub text_size: TextSize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { language: Language::Zh, text_size: TextSize::Standard }
    }
}

/// `%APPDATA%\ActingCommand\acui.toml` on Windows, `$XDG_CONFIG_HOME` (or
/// `$HOME/.config`) elsewhere. `None` when the platform states neither.
pub fn path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    Some(base.join("ActingCommand").join("acui.toml"))
}

/// Reads the two keys. A missing file, an unreadable one or an unknown value
/// all mean the same thing: use the default.
pub fn load() -> Settings {
    let mut settings = Settings::default();
    let Some(path) = path() else {
        return settings;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return settings;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "lang" => settings.language = Language::from_wire(value).unwrap_or(settings.language),
            "text_size" => {
                settings.text_size = TextSize::from_wire(value).unwrap_or(settings.text_size)
            }
            _ => {}
        }
    }
    settings
}

/// Writes the language, keeping whatever the file says about the text size.
pub fn save_language(language: Language) {
    save(Settings { language, ..load() });
}

/// Writes the text size, keeping whatever the file says about the language: the
/// language this run shows may be a `--lang` override, which is this run's alone
/// and must never reach the file.
pub fn save_text_size(text_size: TextSize) {
    save(Settings { text_size, ..load() });
}

/// Writes both keys. A console that cannot save a preference still runs, so a
/// failure here is reported and otherwise ignored.
fn save(settings: Settings) {
    let Some(path) = path() else {
        return;
    };
    let body = format!(
        "# ActingCommand 监控台 / ActingCommand Console\nlang = \"{}\"\ntext_size = \"{}\"\n",
        settings.language.wire(),
        settings.text_size.wire()
    );
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| std::fs::write(&path, body));
    if let Err(error) = written {
        eprintln!("{}: {error}", path.display());
    }
}
