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
//! `acui.toml`. Every read and every change runs under one lock of this process and under the lock
//! file `acui-holds.json.lock` beside it, which two consoles of one user share: on Windows it is held
//! open with no sharing, so another console's read or change waits for it (both locks together at
//! most `ACROSS_WAIT`, then an error, said in red as any other; within `ACROSS_WAIT` of a wait that
//! ran out, one try without waiting), and the system lets it go when the console that held it
//! ends, so a console that died holds nothing. No item of another console's change is lost: a
//! change reads the file inside the lock. Elsewhere than Windows, where the console does not run,
//! the lock file locks nothing. A record left empty is written as an empty list, not deleted; the
//! lock file stays, empty.
//!
//! `store` runs on a request's own thread, `load` and `forget` on the event loop: the file holds a
//! handful of short items at most.

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use acui_source::workbench::hold::{HoldFile, StoredAttempt};

use crate::report;

const FILE: &str = "acui-holds.json";
const TEMPORARY: &str = "acui-holds.json.tmp";
const ACROSS: &str = "acui-holds.json.lock";

/// How long one read or change waits, for both locks together: another console's take
/// milliseconds.
const ACROSS_WAIT: Duration = Duration::from_secs(5);
/// How often it tries again meanwhile.
const ACROSS_RETRY: Duration = Duration::from_millis(25);

/// Every read, change and write of the file in this process, one at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// When a wait for the locks last ran out. Within `ACROSS_WAIT` of it a read or change tries once
/// and does not wait, so that the reads and changes one `Status` makes on the event loop wait
/// `ACROSS_WAIT` once between them, not once each, while something keeps the lock.
static TIMED_OUT: Mutex<Option<Instant>> = Mutex::new(None);

/// The lock file, held for one read or change against this user's other consoles; closing it,
/// when this is dropped or the process ends, lets it go.
struct Across {
    _held: File,
}

/// Both locks of one read or change, this process's and the lock file beside `path`, within one
/// wait: an error once it has run out, or when the lock file cannot be opened for any other
/// reason. They are taken in that order and let go the other way round, the lock file first, as
/// a tuple drops its first member first.
fn both(path: &Path) -> Result<(Across, MutexGuard<'static, ()>), String> {
    let deadline = deadline();
    let one = this_process(deadline)?;
    let across = across(path, deadline)?;
    *TIMED_OUT.lock().unwrap_or_else(PoisonError::into_inner) = None;
    Ok((across, one))
}

/// `ACROSS_WAIT` from now, or now when a wait ran out less than `ACROSS_WAIT` ago.
fn deadline() -> Instant {
    let now = Instant::now();
    let last = *TIMED_OUT.lock().unwrap_or_else(PoisonError::into_inner);
    match last {
        Some(at) if now.duration_since(at) < ACROSS_WAIT => now,
        _ => now + ACROSS_WAIT,
    }
}

/// A wait that ran out, remembered for `deadline`.
fn timed_out() {
    *TIMED_OUT.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
}

/// This process's lock, by `deadline`.
fn this_process(deadline: Instant) -> Result<MutexGuard<'static, ()>, String> {
    loop {
        match LOCK.try_lock() {
            Ok(one) => return Ok(one),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(ACROSS_RETRY);
            }
            Err(TryLockError::WouldBlock) => {
                timed_out();
                return Err(format!(
                    "{FILE}: another read or change of it in this console has run for over {} s",
                    ACROSS_WAIT.as_secs()
                ));
            }
        }
    }
}

/// The lock file beside `path`, by `deadline`.
fn across(path: &Path, deadline: Instant) -> Result<Across, String> {
    let lock = path.with_file_name(ACROSS);
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    loop {
        match open_alone(&lock) {
            Ok(held) => return Ok(Across { _held: held }),
            Err(error) if held_by_another(&error) && Instant::now() < deadline => {
                std::thread::sleep(ACROSS_RETRY);
            }
            Err(error) if held_by_another(&error) => {
                timed_out();
                return Err(format!(
                    "{}: another program, perhaps another console, has held it for over {} s: \
                     {error}",
                    lock.display(),
                    ACROSS_WAIT.as_secs()
                ));
            }
            Err(error) => return Err(format!("{}: {error}", lock.display())),
        }
    }
}

/// The lock file opened with no sharing: no other process opens it until this handle closes.
#[cfg(windows)]
fn open_alone(lock: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0).open(lock)
}

/// `ERROR_SHARING_VIOLATION`: another process has the lock file open.
#[cfg(windows)]
fn held_by_another(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(32)
}

/// Elsewhere than Windows, where the console does not run, the lock file is only opened.
#[cfg(not(windows))]
fn open_alone(lock: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock)
}

#[cfg(not(windows))]
fn held_by_another(_error: &std::io::Error) -> bool {
    false
}

/// Where the file is. `Err` names the variables that are not set.
pub fn path() -> Result<PathBuf, &'static str> {
    Ok(report::local_dir()?.join(FILE))
}

/// The record as the file holds it: empty when there is no file.
pub fn load() -> Result<HoldFile, String> {
    let path = located()?;
    let _locks = both(&path)?;
    read(&path)
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
    let path = located()?;
    let _locks = both(&path)?;
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
