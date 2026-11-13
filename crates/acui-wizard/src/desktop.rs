// SPDX-License-Identifier: GPL-3.0-only
//! What the window asks of Windows beyond the back end: the caller's console for the command
//! line (the release build is a window program), the shell's folder picker, the default
//! install location, the console started apart from the wizard, Explorer, and a message box
//! for a window that cannot open. On any other platform each fails loud or says it on stderr.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The variables a launcher hands down to name an A/B installation (Runtime
/// `crates/actingcommand-contract/src/installation.rs` `INSTALL_ROOT_ENV`,
/// `INSTALL_SELECTION_ENV`); the console started from here must not read another root's.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SELECTION_VARIABLES: [&str; 2] = ["ACTINGCOMMAND_INSTALL_ROOT", "ACTINGCOMMAND_INSTALL_SELECTION"];

/// Whether `path` may take a new installation as far as its own folder goes: absent, or an
/// empty folder.
fn unused(path: &Path) -> bool {
    match fs::metadata(path) {
        Err(error) => error.kind() == io::ErrorKind::NotFound,
        Ok(meta) if meta.is_dir() => fs::read_dir(path).map(|mut entries| entries.next().is_none()).unwrap_or(false),
        Ok(_) => false,
    }
}

/// `base` when it is unused, else the first of `base-2`, `base-3`, … that is (up to `-99`);
/// `base` again when none is, for the checks to say why.
pub fn first_unused(base: PathBuf) -> PathBuf {
    if unused(&base) {
        return base;
    }
    let name = base
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ActingCommand".to_owned());
    for number in 2..100u32 {
        let candidate = base.with_file_name(format!("{name}-{number}"));
        if unused(&candidate) {
            return candidate;
        }
    }
    base
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::{first_unused, SELECTION_VARIABLES};

    /// `DETACHED_PROCESS` and `CREATE_NEW_PROCESS_GROUP`: the console gets none of the wizard's
    /// console and none of its Ctrl+C, and outlives it.
    const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
    const DRIVE_FIXED: u32 = 3;
    const MB_OK: u32 = 0;
    const MB_ICONERROR: u32 = 0x10;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
        fn GetFileType(handle: *mut c_void) -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(window: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_TYPE_PIPE: u32 = 3;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The release build is a window program: run from a console or a script, its command line
    /// still writes there. Output the caller redirected (a pipe or a file) is kept; otherwise
    /// the parent's console is attached and `CONOUT$` becomes stdout and stderr. With neither,
    /// nothing is shown, and the exit code still says how it went. (The 0.11 acsetup's own.)
    pub fn attach_console() {
        // SAFETY: plain queries of this process's own standard handles.
        let redirected = |which: u32| unsafe {
            let handle = GetStdHandle(which);
            !handle.is_null()
                && handle as isize != -1
                && matches!(GetFileType(handle), FILE_TYPE_DISK | FILE_TYPE_PIPE)
        };
        let (out, err) = (redirected(STD_OUTPUT_HANDLE), redirected(STD_ERROR_HANDLE));
        if out && err {
            return;
        }
        // SAFETY: attaching to the parent's console changes nothing else here; it fails
        // harmlessly when this process already has one (a debug build).
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return;
        }
        // Read access too: Rust writes UTF-16 to a console only when it can ask its mode.
        let Ok(console) = std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$") else {
            return;
        };
        use std::os::windows::io::IntoRawHandle;
        // The console handle stays open for the life of the process.
        let handle = console.into_raw_handle();
        // SAFETY: `handle` is a valid console handle this process owns.
        unsafe {
            if !out {
                SetStdHandle(STD_OUTPUT_HANDLE, handle);
            }
            if !err {
                SetStdHandle(STD_ERROR_HANDLE, handle);
            }
        }
        // The prompt the caller's shell printed stays on its own line.
        if !out {
            use std::io::Write;
            let _ = writeln!(std::io::stdout());
        }
    }

    /// `X:\ActingCommand` on the first local fixed drive other than the system's, else
    /// `%LOCALAPPDATA%\ActingCommand`; with `-2`, `-3`, … when that is already in use.
    pub fn default_root() -> PathBuf {
        let system = std::env::var("SystemDrive")
            .ok()
            .and_then(|drive| drive.chars().next())
            .map(|letter| letter.to_ascii_uppercase())
            .unwrap_or('C');
        // SAFETY: a plain query of the drive bitmask.
        let mask = unsafe { GetLogicalDrives() };
        let fixed = (0u8..26).filter(|bit| mask & (1u32 << bit) != 0).map(|bit| char::from(b'A' + bit)).find(|letter| {
            if *letter == system {
                return false;
            }
            let root = format!("{letter}:\\");
            let name = wide(&root);
            // SAFETY: `name` is NUL-terminated UTF-16 that outlives the call.
            let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
            kind == DRIVE_FIXED && Path::new(&root).is_dir()
        });
        let base = match fixed {
            Some(letter) => PathBuf::from(format!("{letter}:\\ActingCommand")),
            None => match std::env::var_os("LOCALAPPDATA") {
                Some(local) => PathBuf::from(local).join("ActingCommand"),
                None => PathBuf::from(format!("{system}:\\ActingCommand")),
            },
        };
        first_unused(base)
    }

    /// The shell's folder picker, on the calling thread (a worker of the window's, in an
    /// apartment of its own); `None` when the person cancels.
    pub fn pick_folder(title: &str) -> Result<Option<PathBuf>, String> {
        use windows::core::HSTRING;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
            SIGDN_FILESYSPATH,
        };
        /// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: the person closed the picker.
        const CANCELLED: u32 = 0x8007_04C7;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                // SAFETY: paired with the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
        // SAFETY: plain COM initialisation of the calling thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .map_err(|error| error.to_string())?;
        let _apartment = Apartment;
        // SAFETY: every call goes through the interfaces the shell returned, with strings that
        // outlive the calls; the path the shell allocates is read once and freed once.
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(|error| error.to_string())?;
            let options = dialog.GetOptions().map_err(|error| error.to_string())?;
            dialog
                .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
                .map_err(|error| error.to_string())?;
            dialog.SetTitle(&HSTRING::from(title)).map_err(|error| error.to_string())?;
            if let Err(error) = dialog.Show(None) {
                if error.code().0 as u32 == CANCELLED {
                    return Ok(None);
                }
                return Err(error.to_string());
            }
            let item = dialog.GetResult().map_err(|error| error.to_string())?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|error| error.to_string())?;
            let text = name.to_string();
            CoTaskMemFree(Some(name.0 as *const c_void));
            text.map(|text| Some(PathBuf::from(text))).map_err(|error| error.to_string())
        }
    }

    /// `<root>\ui\acui.exe`, started in `<root>\ui`, apart from the wizard: no console, no
    /// standard handles, no A/B selection handed down. The child is never waited on.
    pub fn start_console(root: &Path) -> io::Result<()> {
        use std::os::windows::process::CommandExt;
        let ui = root.join("ui");
        let mut command = Command::new(ui.join("acui.exe"));
        for variable in SELECTION_VARIABLES {
            command.env_remove(variable);
        }
        command
            .current_dir(&ui)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED)
            .spawn()
            .map(|_child| ())
    }

    /// Explorer showing `path`: a folder opened, a file opened with its program.
    pub fn open_with_explorer(path: &Path) -> io::Result<()> {
        use std::os::windows::process::CommandExt;
        let windows = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        Command::new(windows.join("explorer.exe"))
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED)
            .spawn()
            .map(|_child| ())
    }

    /// A message box, for what the window itself cannot say.
    pub fn alert(title: &str, text: &str) {
        let (title, text) = (wide(title), wide(text));
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::first_unused;

    const WINDOWS_ONLY: &str = "只在 Windows 上可用 / available on Windows only";

    pub fn attach_console() {}

    pub fn default_root() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
        first_unused(home.join("ActingCommand"))
    }

    pub fn pick_folder(_title: &str) -> Result<Option<PathBuf>, String> {
        Err(WINDOWS_ONLY.to_owned())
    }

    pub fn start_console(_root: &Path) -> io::Result<()> {
        Err(io::Error::other(WINDOWS_ONLY))
    }

    pub fn open_with_explorer(_path: &Path) -> io::Result<()> {
        Err(io::Error::other(WINDOWS_ONLY))
    }

    pub fn alert(title: &str, text: &str) {
        eprintln!("{title}: {text}");
    }
}

pub use imp::{alert, attach_console, default_root, open_with_explorer, pick_folder, start_console};
