// SPDX-License-Identifier: GPL-3.0-only
//! The install log, `<root>\install-wizard.log`: one line per thing the wizard did or found,
//! each with its UTC time, a failure with its reason. Lines said before the root exists (the
//! checks of the install set, the plan and the emulators) are kept and written first once the
//! log is opened. The salt is never written here. The person's profile folder (`USERPROFILE`,
//! which holds their user name) is written as `%USERPROFILE%`, so a log sent to whoever helps
//! names no user ([`masked`]).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The log's name in the root.
pub const LOG_FILE: &str = "install-wizard.log";

/// What the profile folder is written as.
pub const PROFILE_MARK: &str = "%USERPROFILE%";

pub struct InstallLog {
    path: Option<PathBuf>,
    file: Option<File>,
    pending: Vec<String>,
    profile: Option<String>,
}

impl InstallLog {
    /// A log with no file yet: its lines wait for [`InstallLog::open`].
    pub fn pending() -> Self {
        Self {
            path: None,
            file: None,
            pending: Vec::new(),
            profile: std::env::var("USERPROFILE").ok().filter(|profile| profile.trim().len() > 3),
        }
    }

    /// Opens `<root>\install-wizard.log` for appending (creating it) and writes the lines said
    /// before it.
    pub fn open(&mut self, root: &Path) -> io::Result<()> {
        let path = root.join(LOG_FILE);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        for line in self.pending.drain(..) {
            file.write_all(line.as_bytes())?;
        }
        file.flush()?;
        self.path = Some(path);
        self.file = Some(file);
        Ok(())
    }

    /// Where the log is, once it is open.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// One line, stamped and flushed at once: the log is whole whenever a person is told to
    /// read it.
    pub fn line(&mut self, text: &str) -> io::Result<()> {
        let text = match &self.profile {
            Some(profile) => masked(text, profile),
            None => text.to_owned(),
        };
        let line = format!("{} {}\n", utc_stamp(unix_ms()), text.replace('\n', " | "));
        match &mut self.file {
            Some(file) => {
                file.write_all(line.as_bytes())?;
                file.flush()
            }
            None => {
                self.pending.push(line);
                Ok(())
            }
        }
    }
}

/// `text` with each mention of the folder `profile` written [`PROFILE_MARK`]: as it is and as
/// JSON writes it (each `\` doubled), its ASCII letters in any case, as Windows compares names,
/// and only where the folder's name ends (`C:\Users\Al` is not masked inside `C:\Users\Alice`).
pub fn masked(text: &str, profile: &str) -> String {
    let profile = profile.trim_end_matches(['\\', '/']);
    let mut out = text.to_owned();
    for needle in [profile.replace('\\', "\\\\"), profile.to_owned()] {
        if needle.is_empty() {
            continue;
        }
        let lowered = out.to_ascii_lowercase();
        let wanted = needle.to_ascii_lowercase();
        let mut result = String::with_capacity(out.len());
        let mut from = 0;
        let mut search = 0;
        while let Some(found) = lowered[search..].find(&wanted) {
            let start = search + found;
            let end = start + wanted.len();
            let ends_there = out[end..]
                .chars()
                .next()
                .map_or(true, |next| !(next.is_alphanumeric() || matches!(next, '-' | '_' | '.')));
            if ends_there {
                result.push_str(&out[from..start]);
                result.push_str(PROFILE_MARK);
                from = end;
            }
            search = end;
        }
        result.push_str(&out[from..]);
        out = result;
    }
    out
}

/// Milliseconds since the Unix epoch.
pub fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// `YYYY-MM-DDThh:mm:ss.mmmZ` of `unix_ms` (proleptic Gregorian, UTC).
pub fn utc_stamp(unix_ms: u128) -> String {
    let millis = (unix_ms % 1000) as u32;
    let seconds = unix_ms / 1000;
    let days = (seconds / 86_400) as i64;
    let rest = (seconds % 86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The civil date of a day count since 1970-01-01: the day count shifted to 0000-03-01, split
/// into 400-year eras of 146 097 days, then the year of the era, the day of a March-based year,
/// and the month from it (the usual days-to-civil conversion of the proleptic Gregorian calendar).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month_number = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let month = month_number as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_folder_is_masked_as_it_is_and_as_json_writes_it() {
        let profile = r"C:\Users\Someone";
        assert_eq!(
            masked(r"shortcut created: C:\Users\Someone\Desktop\a.lnk", profile),
            r"shortcut created: %USERPROFILE%\Desktop\a.lnk"
        );
        // Another case of its letters, the verbatim prefix, and JSON's doubled backslashes.
        assert_eq!(masked(r"\\?\c:\users\SOMEONE\AC", profile), r"\\?\%USERPROFILE%\AC");
        assert_eq!(
            masked(r#"{"log":"C:\\Users\\Someone\\AC\\x.log"}"#, profile),
            r#"{"log":"%USERPROFILE%\\AC\\x.log"}"#
        );
        // Only where the folder's name ends; the folder itself at the end of the text.
        assert_eq!(masked(r"C:\Users\Someone2\AC", profile), r"C:\Users\Someone2\AC");
        assert_eq!(masked(r"home C:\Users\Someone", profile), r"home %USERPROFILE%");
        // A name outside ASCII stays as it is around the mark.
        assert_eq!(masked(r"安装到 C:\Users\用户\AC", r"C:\Users\用户"), r"安装到 %USERPROFILE%\AC");
        // Nothing else changes.
        assert_eq!(masked(r"D:\ActingCommand", profile), r"D:\ActingCommand");
    }
}
