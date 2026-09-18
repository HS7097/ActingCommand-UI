// SPDX-License-Identifier: GPL-3.0-only
//! The one file this program reads and writes itself: its own preferences and
//! the three paths the launcher needs, in the per-user config directory. Never
//! inside a state root, and never anything else — a state root is the read
//! face's to touch, not this one's.
//!
//! `<config_dir>/ActingCommand/acui.toml`, with `lang` and `text_size`, and the
//! optional `state_root`, `actingd_config` and `actingd_exe` (absolute paths,
//! written as TOML literal strings in single quotes).

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

#[derive(Debug, Clone)]
pub struct Settings {
    pub language: Language,
    pub text_size: TextSize,
    /// The state root to open when `--state-root` is not given.
    pub state_root: Option<PathBuf>,
    /// The actingd configuration file the launcher passes as `--config`.
    pub actingd_config: Option<PathBuf>,
    /// The actingd executable the launcher spawns.
    pub actingd_exe: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::Zh,
            text_size: TextSize::Standard,
            state_root: None,
            actingd_config: None,
            actingd_exe: None,
        }
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

/// Reads the keys. A missing file, an unreadable one or an unknown value all
/// mean the same thing: use the default. A path key is kept as written; whether
/// it is absolute is checked where it is used, so the report names the key.
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
        let value = unquote(value.trim());
        match key.trim() {
            "lang" => settings.language = Language::from_wire(value).unwrap_or(settings.language),
            "text_size" => {
                settings.text_size = TextSize::from_wire(value).unwrap_or(settings.text_size)
            }
            "state_root" => settings.state_root = path_value(value),
            "actingd_config" => settings.actingd_config = path_value(value),
            "actingd_exe" => settings.actingd_exe = path_value(value),
            _ => {}
        }
    }
    settings
}

/// Strips one pair of matching quotes, `"…"` or `'…'`. No escapes are
/// processed: a Windows path goes in single quotes, as TOML's literal string.
fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(b'"'), Some(b'"')) | (Some(b'\''), Some(b'\'')) if bytes.len() >= 2 => {
            &value[1..value.len() - 1]
        }
        _ => value,
    }
}

fn path_value(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// Writes the language, keeping whatever the file says about everything else.
pub fn save_language(language: Language) {
    save(Settings { language, ..load() });
}

/// Writes the text size, keeping whatever the file says about everything else:
/// the language this run shows may be a `--lang` override, which is this run's
/// alone and must never reach the file.
pub fn save_text_size(text_size: TextSize) {
    save(Settings { text_size, ..load() });
}

/// Writes every key the file had, so a path a person put there survives a
/// preference change. A console that cannot save a preference still runs, so a
/// failure here is reported and otherwise ignored.
fn save(settings: Settings) {
    let Some(path) = path() else {
        return;
    };
    let mut body = format!(
        "# ActingCommand 监控台 / ActingCommand Console\nlang = \"{}\"\ntext_size = \"{}\"\n",
        settings.language.wire(),
        settings.text_size.wire()
    );
    for (key, value) in [
        ("state_root", &settings.state_root),
        ("actingd_config", &settings.actingd_config),
        ("actingd_exe", &settings.actingd_exe),
    ] {
        if let Some(value) = value {
            body.push_str(&format!("{key} = '{}'\n", value.display()));
        }
    }
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| std::fs::write(&path, body));
    if let Err(error) = written {
        eprintln!("{}: {error}", path.display());
    }
}
