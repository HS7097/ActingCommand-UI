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
                format!("环境变量 %{name}% 缺失或不是绝对路径 / %{name}% is missing or not absolute")
            })
    }

    /// `%LOCALAPPDATA%\Programs\ActingCommand`: per-user, no elevation.
    pub fn default_install_root() -> Result<PathBuf, String> {
        Ok(env_dir("LOCALAPPDATA")?.join("Programs").join("ActingCommand"))
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
        use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY};
        // SAFETY: `id` is a valid KNOWNFOLDERID; the returned buffer is owned
        // by the caller, read once and freed with CoTaskMemFree as documented.
        let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) }
            .map_err(|error| format!("找不到{name}文件夹 / cannot find the {name} folder: {error}"))?;
        let text = unsafe { path.to_string() };
        unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
        text.map(PathBuf::from)
            .map_err(|error| format!("{name}文件夹路径无法读取 / the {name} folder's path is unreadable: {error}"))
    }

    /// `ActingCommand.cmd` in the per-user Startup folder.
    pub fn startup_launcher_path() -> Result<PathBuf, String> {
        Ok(known_folder(&windows::Win32::UI::Shell::FOLDERID_Startup, "启动 / Startup")?.join("ActingCommand.cmd"))
    }

    /// `ActingCommand.lnk` in the per-user Start menu's Programs.
    pub fn start_menu_shortcut_path() -> Result<PathBuf, String> {
        Ok(known_folder(&windows::Win32::UI::Shell::FOLDERID_Programs, "开始菜单 / Start menu")?.join("ActingCommand.lnk"))
    }

    /// `ActingCommand.lnk` on the person's desktop.
    pub fn desktop_shortcut_path() -> Result<PathBuf, String> {
        Ok(known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop, "桌面 / desktop")?.join("ActingCommand.lnk"))
    }

    /// A shortcut at `lnk` to `target`, started in `workdir`, written through
    /// the shell's own IShellLink on this thread, in a single-threaded
    /// apartment entered and left here.
    pub fn create_shortcut(lnk: &Path, target: &Path, workdir: &Path, description: &str) -> Result<(), String> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                // SAFETY: paired with the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
        let failed = |error: &dyn std::fmt::Display| {
            format!("无法创建快捷方式 / cannot create the shortcut {}: {error}", lnk.display())
        };
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(&error))?;
        }
        // SAFETY: plain COM initialisation of the calling thread.
        let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        entered.ok().map_err(|error| failed(&error))?;
        let _apartment = Apartment;
        // SAFETY: every call goes through the interfaces the shell returned,
        // with strings that outlive the calls.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|error| failed(&error))?;
            link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|error| failed(&error))?;
            link.SetWorkingDirectory(&HSTRING::from(workdir.as_os_str())).map_err(|error| failed(&error))?;
            link.SetDescription(&HSTRING::from(description)).map_err(|error| failed(&error))?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0).map_err(|error| failed(&error))?;
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
            format!("路径所在的卷不存在 / no existing volume for: {}", path.display())
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
            return Err(format!("GetDiskFreeSpaceExW: {}", io::Error::last_os_error()));
        }
        Ok(free)
    }

    /// The console, detached: no window of the wizard's, no inherited
    /// handles; the child handle is dropped, never waited on or killed.
    pub fn spawn_detached(exe: &Path, cwd: &Path) -> io::Result<()> {
        use std::os::windows::process::CommandExt;
        Command::new(exe)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .map(|_child| ())
    }

    type Handle = *mut std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;

    /// An open process handle, closed when dropped.
    struct Process(Handle);

    impl Process {
        fn open(pid: u32, access: u32) -> io::Result<Self> {
            // SAFETY: plain OpenProcess; a null handle is the documented failure.
            let handle = unsafe { OpenProcess(access, 0, pid) };
            match handle.is_null() {
                true => Err(io::Error::last_os_error()),
                false => Ok(Process(handle)),
            }
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful OpenProcess, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The processes listening on TCP `port` of 127.0.0.1 — or of every IPv4
    /// address, which takes 127.0.0.1 in — from the system's own table of
    /// listeners with their owners (GetExtendedTcpTable), by PID, each once.
    pub fn loopback_listeners(port: u16) -> Result<Vec<u32>, String> {
        #[link(name = "iphlpapi")]
        extern "system" {
            fn GetExtendedTcpTable(
                table: *mut std::ffi::c_void,
                size: *mut u32,
                order: i32,
                family: u32,
                class: i32,
                reserved: u32,
            ) -> u32;
        }
        const AF_INET: u32 = 2;
        const TCP_TABLE_OWNER_PID_LISTENER: i32 = 3;
        const NO_ERROR: u32 = 0;
        const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
        // MIB_TCPTABLE_OWNER_PID: a DWORD count, then rows of six DWORDs —
        // state, local address, local port, remote address, remote port,
        // owning PID — addresses and ports in network byte order. A buffer of
        // u32 keeps them aligned.
        let mut wanted = 0u32;
        // The table may grow between the size asked and the copy.
        for _ in 0..4 {
            let mut table = vec![0u32; (wanted as usize).div_ceil(4).max(1)];
            let mut size = (table.len() * 4) as u32;
            // SAFETY: `table` is a writable buffer of `size` bytes, 4-aligned;
            // the call writes at most `size` bytes and says the size it needs
            // when that is too little.
            let code = unsafe {
                GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0)
            };
            match code {
                NO_ERROR => {
                    let count = table[0] as usize;
                    let rows = table
                        .get(1..1 + count * 6)
                        .ok_or_else(|| format!("GetExtendedTcpTable: {count} rows do not fit the table it returned"))?;
                    let mut pids = Vec::new();
                    for row in rows.chunks_exact(6) {
                        let address = row[1].to_ne_bytes();
                        let row_port = row[2].to_ne_bytes();
                        let loopback = address == [127, 0, 0, 1] || address == [0, 0, 0, 0];
                        if loopback && u16::from_be_bytes([row_port[0], row_port[1]]) == port && !pids.contains(&row[5]) {
                            pids.push(row[5]);
                        }
                    }
                    return Ok(pids);
                }
                ERROR_INSUFFICIENT_BUFFER => wanted = size,
                other => {
                    return Err(format!("GetExtendedTcpTable: {}", io::Error::from_raw_os_error(other as i32)))
                }
            }
        }
        Err("GetExtendedTcpTable: 表一直在变大 / the table kept growing".into())
    }

    /// The full path of the image a running process was started from.
    pub fn process_image(pid: u32) -> Result<PathBuf, String> {
        use std::os::windows::ffi::OsStringExt;
        let process = Process::open(pid, PROCESS_QUERY_LIMITED_INFORMATION).map_err(|error| error.to_string())?;
        let mut name = vec![0u16; 32_768];
        let mut size = name.len() as u32;
        // SAFETY: `name` holds `size` UTF-16 units; on success `size` is the
        // count written, without the terminating NUL.
        let ok = unsafe { QueryFullProcessImageNameW(process.0, 0, name.as_mut_ptr(), &mut size) };
        if ok == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&name[..size as usize])))
    }

    /// Ends the process `pid`, then waits up to `wait` for it to be gone.
    pub fn end_process(pid: u32, wait: std::time::Duration) -> Result<(), String> {
        let process = Process::open(pid, PROCESS_TERMINATE | SYNCHRONIZE).map_err(|error| error.to_string())?;
        // SAFETY: the handle was opened with PROCESS_TERMINATE.
        if unsafe { TerminateProcess(process.0, 1) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let milliseconds = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the handle was opened with SYNCHRONIZE.
        match unsafe { WaitForSingleObject(process.0, milliseconds) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(format!(
                "已请求结束，{} 秒内仍未退出 / asked to end, still there after {} s",
                wait.as_secs(),
                wait.as_secs()
            )),
            _ => Err(io::Error::last_os_error().to_string()),
        }
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

    pub fn create_shortcut(_lnk: &Path, _target: &Path, _workdir: &Path, _description: &str) -> Result<(), String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn free_space(_path: &Path) -> Result<u64, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn spawn_detached(_exe: &Path, _cwd: &Path) -> io::Result<()> {
        Err(io::Error::other(WINDOWS_ONLY))
    }

    pub fn loopback_listeners(_port: u16) -> Result<Vec<u32>, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn process_image(_pid: u32) -> Result<PathBuf, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn end_process(_pid: u32, _wait: std::time::Duration) -> Result<(), String> {
        Err(WINDOWS_ONLY.into())
    }
}

pub use imp::*;
