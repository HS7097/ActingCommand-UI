// SPDX-License-Identifier: GPL-3.0-only
//! The Windows-only facts the wizard needs: where a per-user install goes,
//! how much room its volume has, where the console keeps its settings, where
//! the per-user Startup folder is, and how to start the console detached.
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

    /// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\ActingCommand.cmd`.
    pub fn startup_launcher_path() -> Result<PathBuf, String> {
        Ok(env_dir("APPDATA")?
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join("ActingCommand.cmd"))
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

    pub fn free_space(_path: &Path) -> Result<u64, String> {
        Err(WINDOWS_ONLY.into())
    }

    pub fn spawn_detached(_exe: &Path, _cwd: &Path) -> io::Result<()> {
        Err(io::Error::other(WINDOWS_ONLY))
    }
}

pub use imp::*;
