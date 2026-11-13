// SPDX-License-Identifier: GPL-3.0-only
//! Whether the Runtime a state root names is the one of the installation folder this console
//! belongs to (root mode), before anything is connected to it. In a root the state root is the
//! one its configuration names, used as written, so a configuration copied from another
//! installation leads the console to that installation's Runtime, which may be the one this
//! machine relies on. A Runtime is taken for the folder's only when the program file of the
//! process its `runtime-info.json` names, as the system gives it and resolves it, is one of the
//! folder's (`acui_source::place::runs_from_root`): `<root>\runtime\<file>`, or a kept build's
//! `<root>\install\builds\<commit>\runtime\<file>`. One that is not, or whose program file
//! cannot be had or resolved, is not connected to at all, to read or to ask: every connection
//! passes `admit` first (`acui_source::guard_connections`, set at the start), with the pid the
//! file states, read before anything connects. A pid no process has is a Runtime that is gone:
//! nothing is connected to it either, and it counts as not running. The workbench says what the
//! check found (`refusal`).
//!
//! The one call std does not make, the program file of another process, is the system's:
//! `OpenProcess` with the least right that reads it and `QueryFullProcessImageNameW` on
//! Windows, linked from kernel32 by name as `report` links its own; `/proc/<pid>/exe` elsewhere.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use acui_rows::outcome::CONSOLE_RUNTIME_NOT_THIS_ROOT;
use acui_source::place::runs_from_root;
use acui_source::{Admission, ClientFailure, RuntimeFacts};

use crate::install::plain;
use crate::report;
use crate::strings::{fill, Labels};

/// Why a Runtime is not taken for the installation folder's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotThisRoot {
    /// Its program file is elsewhere: as the system resolves it.
    Elsewhere { pid: u32, program: PathBuf },
    /// What kept the console from telling, as the system said it.
    Unknown { pid: u32, error: String },
}

impl NotThisRoot {
    /// In plain words, with the folder `root`: for the workbench, the ledger windows and a
    /// press's line.
    pub fn text(&self, labels: &Labels, root: &Path) -> String {
        let root = root.display().to_string();
        match self {
            Self::Elsewhere { pid, program } => fill(
                labels.runtime_not_this_root,
                &[&pid.to_string(), &program.display().to_string(), &root],
            ),
            Self::Unknown { pid, error } => {
                fill(labels.runtime_root_unverified, &[&pid.to_string(), &root, error])
            }
        }
    }

    /// For the technical log.
    fn detail(&self, root: &Path) -> String {
        match self {
            Self::Elsewhere { pid, program } => format!(
                "the Runtime of pid {pid} runs {}, which is no program of {}",
                program.display(),
                root.display()
            ),
            Self::Unknown { pid, error } => format!(
                "whether the Runtime of pid {pid} is one of {} cannot be told: {error}",
                root.display()
            ),
        }
    }
}

/// What the check found of one Runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// Its program file is one of the installation folder's.
    Ours,
    /// No process has its pid: it does not run.
    Gone,
    NotOurs(NotThisRoot),
}

/// The Runtime last judged and what the check found of it: asked of the system once per pid
/// and owner epoch, which a restart changes (`admit`).
struct Judged {
    facts: RuntimeFacts,
    verdict: Verdict,
}

static JUDGED: Mutex<Option<Judged>> = Mutex::new(None);

/// The guard every connection passes in root mode (`acui_source::guard_connections`): whether
/// the Runtime `facts` names, as the state root's `runtime-info.json` states it before anything
/// connects, may be connected to. The system is asked once per pid and owner epoch; what it
/// says of a new one goes into the technical log once.
pub fn admit(root: &Path, facts: &RuntimeFacts) -> Admission {
    let mut judged = JUDGED.lock().unwrap_or_else(PoisonError::into_inner);
    let known = judged
        .as_ref()
        .filter(|judged| judged.facts == *facts)
        .map(|judged| judged.verdict.clone());
    let verdict = match known {
        Some(verdict) => verdict,
        None => {
            let verdict = check(root, facts.pid);
            log(root, facts, &verdict);
            *judged = Some(Judged { facts: facts.clone(), verdict: verdict.clone() });
            verdict
        }
    };
    match verdict {
        Verdict::Ours => Admission::Admitted,
        Verdict::Gone => Admission::Gone,
        Verdict::NotOurs(_) => Admission::NotThisRoot,
    }
}

/// Why the Runtime last judged is not taken for the installation folder's; `None` when it is,
/// when it is gone, and before any was judged.
pub fn refusal() -> Option<NotThisRoot> {
    let judged = JUDGED.lock().unwrap_or_else(PoisonError::into_inner);
    match judged.as_ref().map(|judged| &judged.verdict) {
        Some(Verdict::NotOurs(why)) => Some(why.clone()),
        _ => None,
    }
}

/// Whether `failure` is the console's own word that the Runtime its state root names is not
/// the installation folder's (`admit`): nothing was connected, and a press that failed so says
/// that is why.
pub fn not_this_root(failure: &ClientFailure) -> bool {
    failure.code == CONSOLE_RUNTIME_NOT_THIS_ROOT
}

/// What a new judgment found, into the technical log: a Runtime not taken for the installation
/// folder's as a problem, the folder's own and one that is gone quietly.
fn log(root: &Path, facts: &RuntimeFacts, verdict: &Verdict) {
    let (pid, epoch) = (facts.pid, &facts.owner_epoch);
    match verdict {
        Verdict::Ours => report::log_quiet(
            "identity",
            &format!("the Runtime of pid {pid}, owner epoch {epoch}, is one of {}", root.display()),
        ),
        Verdict::Gone => report::log_quiet(
            "identity",
            &format!(
                "no process has pid {pid}, which runtime-info.json names (owner epoch {epoch}): \
                 no Runtime runs there, and nothing is connected"
            ),
        ),
        Verdict::NotOurs(why) => report::log(
            "identity",
            &format!("nothing is connected (owner epoch {epoch}): {}", why.detail(root)),
        ),
    }
}

/// Whether the Runtime of process `pid` is one of the installation folder `root`'s: its
/// program file and `root`, each as the system resolves it, by `runs_from_root`; or whether no
/// process has that pid.
fn check(root: &Path, pid: u32) -> Verdict {
    match platform::program(pid) {
        Ok(Some(program)) => match ours(root, pid, program) {
            Ok(()) => Verdict::Ours,
            Err(why) => Verdict::NotOurs(why),
        },
        Ok(None) => Verdict::Gone,
        Err(error) => Verdict::NotOurs(NotThisRoot::Unknown { pid, error: error.to_string() }),
    }
}

/// Whether `program`, the program file of process `pid`, is one of `root`'s.
fn ours(root: &Path, pid: u32, program: PathBuf) -> Result<(), NotThisRoot> {
    let unknown = |error: String| NotThisRoot::Unknown { pid, error };
    let resolved = |path: &Path| match path.canonicalize() {
        Ok(canonical) => Ok(plain(&canonical)),
        Err(error) => Err(unknown(format!("{}: {error}", path.display()))),
    };
    let program = resolved(program.as_path())?;
    let folder = resolved(root)?;
    let (Some(program_text), Some(folder_text)) = (program.to_str(), folder.to_str()) else {
        let error = format!("{} or {} is not Unicode", program.display(), folder.display());
        return Err(unknown(error));
    };
    if runs_from_root(program_text, folder_text) {
        Ok(())
    } else {
        Err(NotThisRoot::Elsewhere { pid, program })
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(
            process: *mut c_void,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    /// The least right that lets `QueryFullProcessImageNameW` read a process's program file.
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    /// The longest path the system gives, in UTF-16 units.
    const PATH_UNITS: u32 = 32_768;
    /// What `OpenProcess` says of a pid no process has.
    const ERROR_INVALID_PARAMETER: i32 = 87;

    /// The program file process `pid` runs, in its Win32 form; `None` when no process has
    /// that pid.
    pub fn program(pid: u32) -> std::io::Result<Option<PathBuf>> {
        // SAFETY: no pointer goes in; a handle that comes back is closed below, once.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
                return Ok(None);
            }
            return Err(error);
        }
        let mut name = vec![0u16; PATH_UNITS as usize];
        let mut size = PATH_UNITS;
        // SAFETY: `name` holds `size` units, the most the call writes, and the call sets
        // `size` to the units it wrote; `process` is the open handle above.
        let queried =
            unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) };
        // Taken before the handle is closed, which may set an error of its own.
        let failed = (queried == 0).then(std::io::Error::last_os_error);
        // SAFETY: the handle `OpenProcess` gave, not used after this.
        unsafe { CloseHandle(process) };
        if let Some(error) = failed {
            return Err(error);
        }
        name.truncate(size as usize);
        Ok(Some(PathBuf::from(OsString::from_wide(&name))))
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io::ErrorKind;
    use std::path::PathBuf;

    /// The program file process `pid` runs, as `/proc` gives it; `None` when no process has
    /// that pid.
    pub fn program(pid: u32) -> std::io::Result<Option<PathBuf>> {
        match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(program) => Ok(Some(program)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}
