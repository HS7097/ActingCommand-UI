// SPDX-License-Identifier: GPL-3.0-only
//! The one file this program reads and writes itself: its preferences and
//! the three paths the launcher needs, in the per-user config directory. Never
//! inside a state root, and never anything else — a state root is the read
//! face's to touch, not this one's.
//!
//! `<config_dir>/ActingCommand/acui.toml`, with `lang`, `text_size` and
//! `theme`, and the optional `state_root`, `actingd_config` and `actingd_exe`
//! (absolute paths, written as TOML literal strings in single quotes), and
//! `preview_frame_gap_seconds`, the least time between two frames the workbench's
//! picture reads (`workbench::preview::frame_gap`), which no box here writes. And
//! `resume_elsewhere_reasons`, which no box writes either: a one-line array of the
//! pause reasons no window here offers a resume for, since only whoever set such a
//! pause lifts it (`ResumeElsewhere`); none by default. A line of it that cannot be
//! taken, or a file that cannot be read, is an error, and until it is fixed no pause
//! but this console's own, its "pause all" and its "pause this account", offers a resume.
//!
//! The file is kept line by line: a save reads it again, changes the last line
//! of its one key (or appends one), keeps every other line byte for byte —
//! comments, unknown keys such as `install_root`, lines it cannot parse — and
//! replaces the file through a temporary one beside it. What the file holds that
//! this program cannot take is a `SettingsIssue`, never silence; a file that is
//! there but cannot be read is never written over.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Duration;

use acui_source::workbench::pause_reason_valid;

use crate::ledger::resume::ResumeElsewhere;
use crate::strings::{fill, Labels, Language};
use crate::workbench::preview::{self, MIN_FRAME_GAP};

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
    /// The least time between two frames the workbench's picture reads.
    pub preview_frame_gap: Duration,
    /// The pauses no window here offers a resume for.
    pub resume_elsewhere: ResumeElsewhere,
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
            preview_frame_gap: MIN_FRAME_GAP,
            resume_elsewhere: ResumeElsewhere::default(),
        }
    }
}

/// The key of the pause reasons no window here offers a resume for.
const RESUME_ELSEWHERE: &str = "resume_elsewhere_reasons";

/// What the file holds that this program could not take, or why a save did
/// not happen. `is_error` sorts them: the file or its directory unusable, a line
/// of `resume_elsewhere_reasons` that cannot be taken and a save that failed are
/// errors; any other value or line that does not parse is a warning, its default
/// used and its line kept.
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
    /// A line of `resume_elsewhere_reasons` that is not a one-line array of reasons the
    /// Runtime takes, with its value as written (the whole line when it has no `=` or
    /// its key's quotes do not pair); counted from 1. Until it is fixed no pause but this
    /// console's own, its "pause all" and its "pause this account", offers a resume
    /// (`ResumeElsewhere::Unusable`).
    UnusableReasons { path: PathBuf, line: usize, value: String },
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
            Self::UnusableReasons { path, line, value } => fill(
                labels.settings_reasons_unusable,
                &[&path.display().to_string(), &line.to_string(), value],
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
/// report names the key. Unknown keys are not read and not an issue. The pause
/// reasons fail closed instead: a file there that cannot be read, no config
/// directory to look in, or any line of their key that cannot be taken leaves
/// them unknown (`ResumeElsewhere::Unusable`), whatever another line of it says.
pub fn load() -> Loaded {
    let mut settings = Settings::default();
    let mut issues = Vec::new();
    let Some(path) = path() else {
        issues.push(SettingsIssue::NoConfigDir);
        settings.resume_elsewhere = ResumeElsewhere::Unusable;
        let file = SettingsFile { path: None, save_blocked: false };
        return Loaded { settings, file, issues };
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => {
            issues.push(SettingsIssue::Unreadable { path: path.clone(), error: error.to_string() });
            settings.resume_elsewhere = ResumeElsewhere::Unusable;
            let file = SettingsFile { path: Some(path), save_blocked: true };
            return Loaded { settings, file, issues };
        }
    };
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let (key, value) = match parse_line(line) {
            Line::Blank => continue,
            // It names the reasons' key, quoted or not, which is all it says: their line,
            // unusable.
            Line::Malformed
                if bare(line).trim_start_matches(['"', '\'']).starts_with(RESUME_ELSEWHERE) =>
            {
                settings.resume_elsewhere = ResumeElsewhere::Unusable;
                issues.push(SettingsIssue::UnusableReasons {
                    path: path.clone(),
                    line: number,
                    value: bare(line).to_owned(),
                });
                continue;
            }
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
            // A whole number of seconds in range, or the default (`MIN_FRAME_GAP`, 5 s) and an
            // issue: also when an earlier line of the key was in range.
            "preview_frame_gap_seconds" => match preview::frame_gap(value) {
                Some(gap) => settings.preview_frame_gap = gap,
                None => {
                    settings.preview_frame_gap = MIN_FRAME_GAP;
                    issues.push(unknown("preview_frame_gap_seconds"));
                }
            },
            // Reasons the Runtime takes, or none known and an issue, which no other line of
            // the key undoes. The key in quotes is the same key in TOML, so it is this one too;
            // the value as written, before its quotes are stripped, since an array in quotes is
            // a string, not an array.
            _ if unquote(key) == RESUME_ELSEWHERE => {
                let written = bare(line).split_once('=').map_or("", |(_, value)| value.trim());
                match reasons(written) {
                    Some(_) if settings.resume_elsewhere == ResumeElsewhere::Unusable => {}
                    Some(listed) => settings.resume_elsewhere = ResumeElsewhere::Listed(listed),
                    None => {
                        settings.resume_elsewhere = ResumeElsewhere::Unusable;
                        issues.push(SettingsIssue::UnusableReasons {
                            path: path.clone(),
                            line: number,
                            value: written.to_owned(),
                        });
                    }
                }
            }
            // The key in quotes that do not pair is not TOML, and names this key all the same:
            // its line, unusable, shown whole, since the key is what is wrong.
            _ if key.trim_matches(|c| c == '"' || c == '\'') == RESUME_ELSEWHERE => {
                settings.resume_elsewhere = ResumeElsewhere::Unusable;
                issues.push(SettingsIssue::UnusableReasons {
                    path: path.clone(),
                    line: number,
                    value: bare(line).to_owned(),
                });
            }
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
    let line = bare(line);
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

/// A line without the white space round it, or a byte-order mark before it.
fn bare(line: &str) -> &str {
    line.trim_start_matches('\u{feff}').trim()
}

fn path_value(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// A one-line TOML array of strings, `["a-reason", 'b.reason']`, a trailing comma
/// allowed as in TOML, each item a name the Runtime takes as a pause's reason
/// (`pause_reason_valid`); no escapes, as no such name needs one. `None` when the
/// value is not one, or any item is not.
fn reasons(value: &str) -> Option<Vec<String>> {
    let items = value.strip_prefix('[')?.strip_suffix(']')?.trim();
    if items.is_empty() {
        return Some(Vec::new());
    }
    let items = items.strip_suffix(',').unwrap_or(items);
    items
        .split(',')
        .map(|item| {
            let item = item.trim();
            let reason = unquote(item);
            let quoted = reason.len() + 2 == item.len();
            (quoted && pause_reason_valid(reason)).then(|| reason.to_owned())
        })
        .collect()
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
