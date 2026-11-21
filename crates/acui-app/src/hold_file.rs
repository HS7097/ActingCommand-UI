// SPDX-License-Identifier: GPL-3.0-only
//! The console's record of the player hold requests it sent (Workflow #429 UI-3d,
//! `workbench::hold`): `acui-holds.json` in this program's own folder (`report::local_dir`,
//! `%LOCALAPPDATA%\ActingCommand` on Windows), beside its logs and never inside a state root. Each
//! request is put in before it is sent, and taken out once what it did is known, so that a request
//! whose answer never came is settled from a `Status` after the console, its connection or the
//! Runtime started again. Its text is `acui_source::workbench::hold::HoldFile`'s.
//!
//! A file that is not there is an empty record. One that is there and cannot be read or does not
//! parse is an error, said in red, and never written over: no request is sent while it is so, and
//! it is read again at every `Status` read online, so that mending it or moving it away is enough.
//! Every change reads the file again, changes it, checks that what it would write reads back, and
//! writes it to `acui-holds.json.tmp` beside it, which then replaces it, as `settings` writes
//! `acui.toml`; all under one lock of this process. Two consoles of one user share the file and
//! are not locked against each other: when both change it at the same moment, one's item may drop
//! out, which the README says. A record left empty is written as an empty list, not deleted.
//!
//! `store` runs on a request's own thread, `load` and `forget` on the event loop: the file holds a
//! handful of short items at most.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use acui_source::workbench::hold::{HoldFile, StoredAttempt};

use crate::report;

const FILE: &str = "acui-holds.json";
const TEMPORARY: &str = "acui-holds.json.tmp";

/// Every read, change and write of the file in this process, one at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// Where the file is. `Err` names the variables that are not set.
pub fn path() -> Result<PathBuf, &'static str> {
    Ok(report::local_dir()?.join(FILE))
}

/// The record as the file holds it: empty when there is no file.
pub fn load() -> Result<HoldFile, String> {
    let _one = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    read(&located()?)
}

/// `stored` put in the record, last, before its request is sent.
pub fn store(stored: &StoredAttempt) -> Result<(), String> {
    change(|file| {
        file.put(stored.clone());
        true
    })
    .map(drop)
}

/// `stored` taken out of the record: whether it was in. Nothing is written when it was not.
pub fn forget(stored: &StoredAttempt) -> Result<bool, String> {
    change(|file| file.take(stored))
}

fn located() -> Result<PathBuf, String> {
    path().map_err(|names| format!("no folder for {FILE}: {names} is not set"))
}

/// The file read again, `edit` applied, and the file written when `edit` says it changed it.
fn change(edit: impl FnOnce(&mut HoldFile) -> bool) -> Result<bool, String> {
    let _one = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let path = located()?;
    let mut file = read(&path)?;
    if !edit(&mut file) {
        return Ok(false);
    }
    let text = file.render();
    // What is written reads back, or nothing is written.
    HoldFile::parse(&text).map_err(|error| {
        format!("{}: what would be written does not read back: {error}", path.display())
    })?;
    write(&path, &text)?;
    Ok(true)
}

/// The record at `path`: empty when the file is not there; an error when it is there and cannot
/// be read or does not parse, which is then never written over.
fn read(path: &Path) -> Result<HoldFile, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => HoldFile::parse(&text).map_err(|error| format!("{}: {error}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(HoldFile::empty()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// `text` to the temporary file beside `path`, which then replaces it; a failure leaves the file
/// as it was, and takes the temporary file away again.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let temporary = path.with_file_name(TEMPORARY);
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&temporary, text))
        .and_then(|()| std::fs::rename(&temporary, path));
    match written {
        Ok(()) => Ok(()),
        Err(error) if temporary.exists() => match std::fs::remove_file(&temporary) {
            Ok(()) => Err(format!("{}: {error}", path.display())),
            Err(removing) => {
                Err(format!("{}: {error}; {}: {removing}", path.display(), temporary.display()))
            }
        },
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}
