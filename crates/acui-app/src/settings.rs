// SPDX-License-Identifier: GPL-3.0-only
//! The one file this program reads and writes itself: its preferences and
//! the three paths the launcher needs, in the per-user config directory. Never
//! inside a state root, and never anything else — a state root is the read
//! face's to touch, not this one's.
//!
//! `<config_dir>/ActingCommand/acui.toml`, with `lang`, `text_size` and
//! `theme`, and the optional `state_root`, `actingd_config` and `actingd_exe`
//! (absolute paths, written as TOML literal strings in single quotes).
//!
//! The file is kept line by line: a save reads it again, changes the last line
//! of its one key (or appends one), keeps every other line byte for byte —
//! comments, unknown keys such as `install_root`, lines it cannot parse — and
//! replaces the file through a temporary one beside it. What the file holds that
//! this program cannot take is a `SettingsIssue`, never silence; a file that is
//! there but cannot be read is never written over.

use std::io::ErrorKind;
use std::path::PathBuf;

use crate::strings::{fill, Labels, Language};

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

    /// The one number every font size and row height in `app.slint` and its
    /// kit is scaled by.
    pub const fn factor(self) -> f32 {
        match self {
            Self::Standard => 1.0,
            Self::Large => 1.25,
            Self::ExtraLarge => 1.5,
        }
    }
}

/// The look the windows are drawn in, as `theme` spells it: picked in any
/// window, applied to every window at once and saved, like the text size. A
/// value that is neither is an issue: the windows are drawn flat, and the line
/// is kept as written, like every line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Flat,
    Aero,
}

impl Theme {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Aero => "aero",
        }
    }

    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "flat" => Some(Self::Flat),
            "aero" => Some(Self::Aero),
            _ => None,
        }
    }

    pub const fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Aero,
            _ => Self::Flat,
        }
    }

    pub const fn index(self) -> i32 {
        match self {
            Self::Flat => 0,
            Self::Aero => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub language: Language,
    pub text_size: TextSize,
    pub theme: Theme,
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
            theme: Theme::Flat,
            state_root: None,
            actingd_config: None,
            actingd_exe: None,
        }
    }
}

/// What the file holds that this program could not take, or why a save did
/// not happen. `is_error` sorts them: the file or its directory unusable and a
/// save that failed are errors; a value or a line that does not parse is a
/// warning, its default used and its line kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIssue {
    /// The platform states no config directory: nothing is read, and nothing
    /// can be saved.
    NoConfigDir,
    /// The file is there but could not be read (permissions, not UTF-8): the
    /// defaults are used and no save is made this run.
    Unreadable { path: PathBuf, error: String },
    /// A known key whose value is none of its spellings; counted from 1.
    UnknownValue { path: PathBuf, line: usize, key: &'static str, value: String },
    /// A line that is not blank, not a comment and has no `=`; counted from 1.
    Malformed { path: PathBuf, line: usize },
    /// A save refused because the file could not be read at the start.
    SaveBlocked { path: PathBuf },
    /// A save that was tried and failed; the file is as it was.
    SaveFailed { path: PathBuf, error: String },
}

/// The variables a config directory comes from, for the line that says none is set.
const CONFIG_DIR_NAMES: &str =
    if cfg!(windows) { "%APPDATA%" } else { "$XDG_CONFIG_HOME / $HOME" };

/// The first line of a file this program creates.
const HEADER: &str = "# ActingCommand 监控台 / ActingCommand Console";

impl SettingsIssue {
    pub fn is_error(&self) -> bool {
        !matches!(self, Self::UnknownValue { .. } | Self::Malformed { .. })
    }

    pub fn text(&self, labels: &Labels) -> String {
        match self {
            Self::NoConfigDir => fill(labels.settings_no_dir, &[CONFIG_DIR_NAMES]),
            Self::Unreadable { path, error } => {
                fill(labels.settings_unreadable, &[&path.display().to_string(), error])
            }
            Self::UnknownValue { path, line, key, value } => fill(
                labels.settings_unknown_value,
                &[&path.display().to_string(), &line.to_string(), *key, value],
            ),
            Self::Malformed { path, line } => fill(
                labels.settings_malformed,
                &[&path.display().to_string(), &line.to_string()],
            ),
            Self::SaveBlocked { path } => {
                fill(labels.settings_save_blocked, &[&path.display().to_string()])
            }
            Self::SaveFailed { path, error } => {
                fill(labels.settings_save_failed, &[&path.display().to_string(), error])
            }
        }
    }
}

/// The file as this run found it: where it is, and whether it may be written.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: Option<PathBuf>,
    /// Set when the file was there but could not be read at the start: a save
    /// would rest on nothing read, so none is made.
    save_blocked: bool,
}

/// What `load` found: the settings, the file, and the file's problems in line order.
pub struct Loaded {
    pub settings: Settings,
    pub file: SettingsFile,
    pub issues: Vec<SettingsIssue>,
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

/// Reads the keys. A missing file is the first run: the defaults, and no issue.
/// Anything else that keeps a key from being read is an issue, and the key
/// keeps its default; a key given twice takes its last line. A path key is kept
/// as written; whether it is absolute is checked where it is used, so the
/// report names the key. Unknown keys are not read and not an issue.
pub fn load() -> Loaded {
    let mut settings = Settings::default();
    let mut issues = Vec::new();
    let Some(path) = path() else {
        issues.push(SettingsIssue::NoConfigDir);
        let file = SettingsFile { path: None, save_blocked: false };
        return Loaded { settings, file, issues };
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => {
            issues.push(SettingsIssue::Unreadable { path: path.clone(), error: error.to_string() });
            let file = SettingsFile { path: Some(path), save_blocked: true };
            return Loaded { settings, file, issues };
        }
    };
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let (key, value) = match parse_line(line) {
            Line::Blank => continue,
            Line::Malformed => {
                issues.push(SettingsIssue::Malformed { path: path.clone(), line: number });
                continue;
            }
            Line::Entry(key, value) => (key, value),
        };
        let unknown = |key: &'static str| SettingsIssue::UnknownValue {
            path: path.clone(),
            line: number,
            key,
            value: value.to_owned(),
        };
        match key {
            "lang" => match Language::from_wire(value) {
                Some(language) => settings.language = language,
                None => issues.push(unknown("lang")),
            },
            "text_size" => match TextSize::from_wire(value) {
                Some(text_size) => settings.text_size = text_size,
                None => issues.push(unknown("text_size")),
            },
            "theme" => match Theme::from_wire(value) {
                Some(theme) => settings.theme = theme,
                None => issues.push(unknown("theme")),
            },
            "state_root" => settings.state_root = path_value(value),
            "actingd_config" => settings.actingd_config = path_value(value),
            "actingd_exe" => settings.actingd_exe = path_value(value),
            _ => {}
        }
    }
    let file = SettingsFile { path: Some(path), save_blocked: false };
    Loaded { settings, file, issues }
}

/// One line as the parser reads it.
enum Line<'a> {
    /// Empty, white space only, or a `#` comment.
    Blank,
    /// The trimmed key, and the trimmed value with one pair of quotes stripped.
    Entry(&'a str, &'a str),
    /// Anything else: no `=`.
    Malformed,
}

/// A byte-order mark before the first line is not part of its key.
fn parse_line(line: &str) -> Line<'_> {
    let line = line.trim_start_matches('\u{feff}').trim();
    if line.is_empty() || line.starts_with('#') {
        return Line::Blank;
    }
    match line.split_once('=') {
        Some((key, value)) => Line::Entry(key.trim(), unquote(value.trim())),
        None => Line::Malformed,
    }
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

impl SettingsFile {
    /// Writes the language, and nothing else.
    pub fn save_language(&self, language: Language) -> Result<(), SettingsIssue> {
        self.save_key("lang", &format!("\"{}\"", language.wire()))
    }

    /// Writes the text size, and nothing else: the language this run shows may
    /// be a `--lang` override, which is this run's alone and must never reach
    /// the file.
    pub fn save_text_size(&self, text_size: TextSize) -> Result<(), SettingsIssue> {
        self.save_key("text_size", &format!("\"{}\"", text_size.wire()))
    }

    /// Writes the theme, and nothing else, for the same reason.
    pub fn save_theme(&self, theme: Theme) -> Result<(), SettingsIssue> {
        self.save_key("theme", &format!("\"{}\"", theme.wire()))
    }

    /// Sets one key to `value` (TOML as written, quotes included). The file is
    /// read again first, since something else may have written it since the
    /// start; only the key's last line changes, or a line is appended. The
    /// result goes to `acui.toml.tmp` beside it, which then replaces the file;
    /// a failure leaves the file as it was.
    fn save_key(&self, key: &str, value: &str) -> Result<(), SettingsIssue> {
        let Some(path) = &self.path else {
            return Err(SettingsIssue::NoConfigDir);
        };
        if self.save_blocked {
            return Err(SettingsIssue::SaveBlocked { path: path.clone() });
        }
        let failed = |error: String| SettingsIssue::SaveFailed { path: path.clone(), error };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            // Unread, it is not written over.
            Err(error) => return Err(failed(error.to_string())),
        };
        let body = with_key(&text, key, value);
        let temporary = path.with_extension("toml.tmp");
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&temporary, body))
            .and_then(|()| std::fs::rename(&temporary, path));
        match written {
            Ok(()) => Ok(()),
            Err(error) if temporary.exists() => match std::fs::remove_file(&temporary) {
                Ok(()) => Err(failed(error.to_string())),
                Err(removing) => {
                    Err(failed(format!("{error}; {}: {removing}", temporary.display())))
                }
            },
            Err(error) => Err(failed(error.to_string())),
        }
    }
}

/// `text` with `key = value` in place of the key's last line, that line's own
/// line end kept, or appended after the last line when no line has the key.
/// Every other line is kept byte for byte. A new file starts with `HEADER`.
fn with_key(text: &str, key: &str, value: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let entry = format!("{key} = {value}");
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let last = lines
        .iter()
        .rposition(|line| matches!(parse_line(line), Line::Entry(found, _) if found == key));
    let mut body = String::with_capacity(text.len() + entry.len() + HEADER.len() + 4);
    match last {
        Some(at) => {
            for (index, line) in lines.iter().enumerate() {
                if index == at {
                    body.push_str(&entry);
                    body.push_str(line_end(line));
                } else {
                    body.push_str(line);
                }
            }
        }
        None => {
            if text.is_empty() {
                body.push_str(HEADER);
                body.push_str(newline);
            } else {
                body.push_str(text);
                if !text.ends_with('\n') {
                    body.push_str(newline);
                }
            }
            body.push_str(&entry);
            body.push_str(newline);
        }
    }
    body
}

/// The line end a line from `split_inclusive('\n')` carries, if any.
fn line_end(line: &str) -> &'static str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}
