// SPDX-License-Identifier: GPL-3.0-only
//! Whether a running Runtime is the one of the installation folder this console belongs to
//! (root mode), before anything is asked of it. In a root the state root is the one its
//! configuration names, used as written, so a configuration copied from another installation
//! leads the console to that installation's Runtime, which may be the one this machine relies
//! on. A Runtime is taken for the folder's only when the program file of the process its
//! `runtime-info.json` names, as the system gives it and resolves it, is one of the folder's
//! (`acui_source::place::runs_from_root`): `<root>\runtime\<file>`, or a kept build's
//! `<root>\install\builds\<commit>\runtime\<file>`. One that is not, or whose program file
//! cannot be had or resolved, is asked nothing: every person's request passes `allowed` first
//! (`acui_source::guard_requests`, set at the start), and Start checks a Runtime it finds up
//! itself (`launcher`). Reads are not checked; the workbench says what the check found.
//!
//! The one call std does not make, the program file of another process, is the system's:
//! `OpenProcess` with the least right that reads it and `QueryFullProcessImageNameW` on
//! Windows, linked from kernel32 by name as `report` links its own; `/proc/<pid>/exe` elsewhere.

use std::path::{Path, PathBuf};

use acui_source::place::runs_from_root;

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

/// Whether the Runtime of process `pid` is one of the installation folder `root`'s: its
/// program file and `root`, each as the system resolves it, by `runs_from_root`.
pub fn check(root: &Path, pid: u32) -> Result<(), NotThisRoot> {
    let unknown = |error: String| NotThisRoot::Unknown { pid, error };
    let resolved = |path: &Path| match path.canonicalize() {
        Ok(canonical) => Ok(plain(&canonical)),
        Err(error) => Err(unknown(format!("{}: {error}", path.display()))),
    };
    let program = platform::program(pid).map_err(|error| unknown(error.to_string()))?;
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

/// `check`, as every person's request asks it (`acui_source::guard_requests`): a Runtime not
/// taken for `root`'s goes into the technical log, each time a request is kept from it.
pub fn allowed(root: &Path, pid: u32) -> bool {
    match check(root, pid) {
        Ok(()) => true,
        Err(why) => {
            report::log("identity", &format!("a request was not sent: {}", why.detail(root)));
            false
        }
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

    /// The program file process `pid` runs, in its Win32 form.
    pub fn program(pid: u32) -> std::io::Result<PathBuf> {
        // SAFETY: no pointer goes in; a handle that comes back is closed below, once.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(std::io::Error::last_os_error());
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
        Ok(PathBuf::from(OsString::from_wide(&name)))
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::PathBuf;

    /// The program file process `pid` runs, as `/proc` gives it.
    pub fn program(pid: u32) -> std::io::Result<PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/exe"))
    }
}
