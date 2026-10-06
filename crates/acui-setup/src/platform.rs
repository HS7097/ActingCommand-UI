// SPDX-License-Identifier: GPL-3.0-only
//! The Windows-only facts the wizard needs: where a per-user install goes,
//! how much room its volume has, where the console keeps its settings, where
//! the Startup folder, the Start menu and the desktop are — asked of the
//! shell, so a redirected or OneDrive desktop is found where Explorer finds
//! it — how to write a shortcut, and how to start the console detached — and,
//! for the ADB server check after an upgrade, which processes listen on a
//! loopback port, their image paths, and how to end one. Those three are
//! asked of the system directly (iphlpapi and kernel32), no new dependency.
//!
//! On any other platform every one of them fails loud — acsetup v1 is
//! Windows-only — and the crate still compiles, so the workspace builds on
//! both CI legs.

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// Windows `DETACHED_PROCESS`: the console gets no console window and
    /// inherits none of the wizard's, so it outlives the wizard.
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    fn env_dir(name: &str) -> Result<PathBuf, String> {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                format!(
                    "环境变量 %{name}% 缺失或不是绝对路径 / %{name}% is missing or not absolute"
                )
            })
    }

    /// `%LOCALAPPDATA%\Programs\ActingCommand`: per-user, no elevation.
    pub fn default_install_root() -> Result<PathBuf, String> {
        Ok(env_dir("LOCALAPPDATA")?
            .join("Programs")
            .join("ActingCommand"))
    }

    /// `%USERPROFILE%\Downloads` when it exists; the person names the folder
    /// either way.
    pub fn default_download_dir() -> Option<PathBuf> {
        env_dir("USERPROFILE")
            .ok()
            .map(|home| home.join("Downloads"))
            .filter(|dir| dir.is_dir())
    }

    /// `%APPDATA%\ActingCommand\acui.toml`, the console's own file.
    pub fn console_settings_path() -> Result<PathBuf, String> {
        Ok(env_dir("APPDATA")?.join("ActingCommand").join("acui.toml"))
    }

    /// A known folder's path as the shell has it, whether or not the folder
    /// exists yet: whoever writes into it creates it.
    fn known_folder(id: &windows::core::GUID, name: &str) -> Result<PathBuf, String> {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath};
        // SAFETY: `id` is a valid KNOWNFOLDERID; the returned buffer is owned
        // by the caller, read once and freed with CoTaskMemFree as documented.
        let path =
            unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) }.map_err(|error| {
                format!("找不到{name}文件夹 / cannot find the {name} folder: {error}")
            })?;
        let text = unsafe { path.to_string() };
        unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
        text.map(PathBuf::from).map_err(|error| {
            format!("{name}文件夹路径无法读取 / the {name} folder's path is unreadable: {error}")
        })
    }

    /// `ActingCommand.cmd` in the per-user Startup folder.
    pub fn startup_launcher_path() -> Result<PathBuf, String> {
        Ok(known_folder(
            &windows::Win32::UI::Shell::FOLDERID_Startup,
            "启动 / Startup",
        )?
        .join("ActingCommand.cmd"))
    }

    /// `ActingCommand.lnk` in the per-user Start menu's Programs.
    pub fn start_menu_shortcut_path() -> Result<PathBuf, String> {
        Ok(known_folder(
            &windows::Win32::UI::Shell::FOLDERID_Programs,
            "开始菜单 / Start menu",
        )?
        .join("ActingCommand.lnk"))
    }

    /// `ActingCommand.lnk` on the person's desktop.
    pub fn desktop_shortcut_path() -> Result<PathBuf, String> {
        Ok(known_folder(
            &windows::Win32::UI::Shell::FOLDERID_Desktop,
            "桌面 / desktop",
        )?
        .join("ActingCommand.lnk"))
    }

    /// A shortcut at `lnk` to `target`, started in `workdir`, written through
    /// the shell's own IShellLink on this thread, in a single-threaded
    /// apartment entered and left here.
    pub fn create_shortcut(
        lnk: &Path,
        target: &Path,
        workdir: &Path,
        description: &str,
    ) -> Result<(), String> {
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        use windows::core::{HSTRING, Interface};
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                // SAFETY: paired with the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
        let failed = |error: &dyn std::fmt::Display| {
            format!(
                "无法创建快捷方式 / cannot create the shortcut {}: {error}",
                lnk.display()
            )
        };
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(&error))?;
        }
        // SAFETY: plain COM initialisation of the calling thread.
        let entered =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        entered.ok().map_err(|error| failed(&error))?;
        let _apartment = Apartment;
        // SAFETY: every call goes through the interfaces the shell returned,
        // with strings that outlive the calls.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| failed(&error))?;
            link.SetPath(&HSTRING::from(target.as_os_str()))
                .map_err(|error| failed(&error))?;
            link.SetWorkingDirectory(&HSTRING::from(workdir.as_os_str()))
                .map_err(|error| failed(&error))?;
            link.SetDescription(&HSTRING::from(description))
                .map_err(|error| failed(&error))?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)
                .map_err(|error| failed(&error))?;
            link.cast::<IPersistFile>()
                .and_then(|file| file.Save(&HSTRING::from(lnk.as_os_str()), true))
                .map_err(|error| failed(&error))?;
        }
        Ok(())
    }

    /// Bytes available to this user on the volume holding `path`, asked of
    /// the deepest ancestor that exists (the root itself usually does not yet).
    pub fn free_space(path: &Path) -> Result<u64, String> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                free_to_caller: *mut u64,
                total: *mut u64,
                total_free: *mut u64,
            ) -> i32;
        }
        let existing = path.ancestors().find(|dir| dir.is_dir()).ok_or_else(|| {
            format!(
                "路径所在的卷不存在 / no existing volume for: {}",
                path.display()
            )
        })?;
        let wide: Vec<u16> = existing
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free = 0u64;
        // SAFETY: `wide` is NUL-terminated UTF-16 and outlives the call; `free`
        // is a valid out pointer; the two other out pointers may be null by the
        // documented contract of GetDiskFreeSpaceExW.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(format!(
                "GetDiskFreeSpaceExW: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(free)
    }

    /// The console, detached: no window of the wizard's, no inherited
    /// handles; the child handle is dropped, never waited on or killed.
    pub fn spawn_detached(exe: &Path, cwd: &Path, snapshot: &acui_installation::Snapshot) -> io::Result<()> {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(exe);
        snapshot.apply_to(&mut command).map_err(io::Error::other)?;
        command
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .map(|_child| ())
    }

    /// Native resource occupancy includes loaded images and consumers whose
    /// launcher has exited. This session queries only; it never shuts down or
    /// restarts a process. `allowed` names processes the caller will close
    /// itself (the Runtime owner, in a pre-check); every other user blocks.
    pub fn slot_files_unused(paths: &[PathBuf], allowed: &[u32]) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::RestartManager::{
            CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
            RmStartSession,
        };
        use windows::core::{PCWSTR, PWSTR};
        if paths.is_empty() {
            return Ok(());
        }
        let mut session = 0;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: both outputs are writable and the key includes its terminator.
        let started = unsafe { RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) };
        if started.0 != 0 {
            return Err(format!(
                "Slot occupancy is unconfirmed: RmStartSession {}",
                started.0
            ));
        }
        let result = (|| {
            let names: Vec<Vec<u16>> = paths
                .iter()
                .map(|path| path.as_os_str().encode_wide().chain(Some(0)).collect())
                .collect();
            let names: Vec<PCWSTR> = names.iter().map(|name| PCWSTR(name.as_ptr())).collect();
            // SAFETY: all UTF-16 buffers and the array remain live through the call.
            let registered = unsafe { RmRegisterResources(session, Some(&names), None, None) };
            if registered.0 != 0 {
                return Err(format!(
                    "Slot occupancy is unconfirmed: RmRegisterResources {}",
                    registered.0
                ));
            }
            let mut processes = vec![RM_PROCESS_INFO::default(); 4096];
            let mut count = processes.len() as u32;
            let mut needed = 0;
            let mut reboot = 0;
            // SAFETY: processes holds count writable records; all outputs are valid.
            let listed = unsafe {
                RmGetList(
                    session,
                    &mut needed,
                    &mut count,
                    Some(processes.as_mut_ptr()),
                    &mut reboot,
                )
            };
            if listed.0 != 0 || count as usize > processes.len() {
                return Err(format!(
                    "Slot occupancy is unconfirmed: RmGetList {}, required {needed}, reboot reasons {reboot}",
                    listed.0
                ));
            }
            let affected = &processes[..count as usize];
            let own = std::process::id();
            // RmRebootReasonDetectedSelf: this process holds the deny-sharing
            // handles itself. It is the one reason ignored, and only while this
            // process is in fact listed; every other reason still blocks.
            const DETECTED_SELF: u32 = 0x10;
            let self_listed = affected
                .iter()
                .any(|process| process.Process.dwProcessId == own);
            if reboot & !DETECTED_SELF != 0 || (reboot & DETECTED_SELF != 0 && !self_listed) {
                return Err(format!(
                    "Slot occupancy is unconfirmed: RmGetList 0, required {needed}, reboot reasons {reboot}"
                ));
            }
            let occupied: Vec<_> = affected
                .iter()
                .filter(|process| {
                    process.Process.dwProcessId != own
                        && !allowed.contains(&process.Process.dwProcessId)
                })
                .map(|process| {
                    format!(
                        "{} ({})",
                        process.Process.dwProcessId,
                        app_name(&process.strAppName)
                    )
                })
                .collect();
            if !occupied.is_empty() {
                return Err(format!(
                    "Program slot is occupied by process(es) {}; materialization is blocked",
                    occupied.join(", ")
                ));
            }
            Ok(())
        })();
        // SAFETY: the session was successfully created and is closed exactly once.
        let ended = unsafe { RmEndSession(session) };
        if ended.0 != 0 {
            return Err(format!(
                "{}RmEndSession failed: {}",
                result
                    .err()
                    .map(|error| format!("{error}; "))
                    .unwrap_or_default(),
                ended.0
            ));
        }
        result
    }

    /// The application name Restart Manager reports, up to its terminating NUL.
    fn app_name(name: &[u16]) -> String {
        let end = name.iter().position(|unit| *unit == 0).unwrap_or(name.len());
        String::from_utf16_lossy(&name[..end])
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
        fn GetFileType(handle: *mut std::ffi::c_void) -> u32;
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const HANDLE_FLAG_INHERIT: u32 = 1;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_TYPE_PIPE: u32 = 3;

    /// acsetup is a GUI-subsystem program; run from a console or a script it
    /// still writes there. Output the caller redirected (a pipe or a file, as Git
    /// Bash and `Start-Process -Redirect…` give) is kept; otherwise the parent's
    /// console is attached and `CONOUT$` becomes stdout/stderr. With neither,
    /// output reaches only the log; the exit code is set either way.
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
        // SAFETY: attaching to the parent's console changes nothing else here.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return;
        }
        // Read access too: Rust writes UTF-16 to a console only when it can ask
        // the console's mode.
        let Ok(console) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONOUT$")
        else {
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
        // The prompt the caller's shell printed stays on its own line; a
        // redirected stdout (a caller's pipe) is never written to here.
        if !out {
            use std::io::Write;
            let _ = writeln!(std::io::stdout());
        }
    }

    /// This process's standard handles are never inherited by its children. A
    /// Runtime started here outlives acsetup and would otherwise keep a caller's
    /// pipe (Git Bash, `$(...)`) open; every child acsetup waits for gets its own
    /// explicit pipes or files.
    pub fn private_std_handles() {
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: only this process's own standard handles are queried and,
            // when valid, marked not inheritable.
            unsafe {
                let handle = GetStdHandle(which);
                if !handle.is_null() && handle as isize != -1 {
                    SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
    }

    static GUARDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static HANDLER: std::sync::Once = std::sync::Once::new();

    unsafe extern "system" fn interrupted(kind: u32) -> i32 {
        // CTRL_C_EVENT 0 and CTRL_BREAK_EVENT 1, while files are being written.
        if kind <= 1 && GUARDED.load(std::sync::atomic::Ordering::SeqCst) {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "正在写入，不能中断 / Writing; this cannot be interrupted now"
            );
            return 1;
        }
        0
    }

    /// While `on`, Ctrl+C and Ctrl+Break are refused, as the wizard refuses to
    /// close while it writes. Closing the console window cannot be refused.
    pub fn guard_interrupts(on: bool) {
        GUARDED.store(on, std::sync::atomic::Ordering::SeqCst);
        HANDLER.call_once(|| {
            // SAFETY: registers a handler that only reads an atomic and writes stderr.
            unsafe {
                SetConsoleCtrlHandler(
                    Some(interrupted as unsafe extern "system" fn(u32) -> i32),
                    1,
                );
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    /// The loud stop on every platform but Windows.
    const WINDOWS_ONLY: &str = "acsetup v1 is Windows-only";

    pub fn default_install_root() -> Result<PathBuf, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn default_download_dir() -> Option<PathBuf> {
        None
    }

    pub fn console_settings_path() -> Result<PathBuf, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn startup_launcher_path() -> Result<PathBuf, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn start_menu_shortcut_path() -> Result<PathBuf, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn desktop_shortcut_path() -> Result<PathBuf, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn create_shortcut(
        _lnk: &Path,
        _target: &Path,
        _workdir: &Path,
        _description: &str,
    ) -> Result<(), String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn free_space(_path: &Path) -> Result<u64, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn spawn_detached(_exe: &Path, _cwd: &Path, _snapshot: &acui_installation::Snapshot) -> io::Result<()> {
        Err(io::Error::other(WINDOWS_ONLY))
    }

    pub fn slot_files_unused(_paths: &[PathBuf], _allowed: &[u32]) -> Result<(), String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn attach_console() {}

    pub fn private_std_handles() {}

    pub fn guard_interrupts(_on: bool) {}
}

pub use imp::*;

/// Ctrl+C refused for as long as this lives (`guard_interrupts`).
pub struct InterruptGuard;

impl InterruptGuard {
    pub fn start() -> Self {
        guard_interrupts(true);
        InterruptGuard
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        guard_interrupts(false);
    }
}
