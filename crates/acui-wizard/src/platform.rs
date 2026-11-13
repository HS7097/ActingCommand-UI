// SPDX-License-Identifier: GPL-3.0-only
//! What the wizard asks of Windows itself: a volume's free space, where the desktop and the
//! Start menu are (asked of the shell, so a redirected or OneDrive desktop is found where
//! Explorer finds it), a shortcut written through the shell's own IShellLink, the running
//! processes with their image paths, and the uninstall entries of the registry, read only.
//!
//! On any other platform each of them fails loud, and the crate still builds there.

use std::path::PathBuf;

use crate::text::Said;

/// One running process.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    /// The image's file name, as the process list has it.
    pub name: String,
    /// The image's full path, `Err` with the reason when it cannot be read (another user's or an
    /// elevated process).
    pub path: Result<PathBuf, String>,
}

/// One uninstall entry under `HKLM`, `HKLM\…\WOW6432Node` or `HKCU`
/// `SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`.
#[derive(Debug, Clone)]
pub struct UninstallEntry {
    pub key_path: String,
    pub install_location: Option<String>,
    pub display_icon: Option<String>,
    pub display_version: Option<String>,
}

#[cfg(not(windows))]
fn windows_only() -> Said {
    Said::new("只在 Windows 上可用", "available on Windows only")
}

#[cfg(windows)]
mod imp {
    use super::{ProcessInfo, UninstallEntry};
    use crate::text::Said;
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    type Handle = *mut c_void;

    /// `PROCESSENTRY32W`; the system fills every member, the wizard reads three.
    #[repr(C)]
    #[allow(dead_code)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        priority_class_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
        fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(key: Handle, sub_key: *const u16, options: u32, sam: u32, result: *mut Handle) -> i32;
        fn RegEnumKeyExW(
            key: Handle,
            index: u32,
            name: *mut u16,
            name_length: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_length: *mut u32,
            last_write: *mut u64,
        ) -> i32;
        fn RegQueryValueExW(
            key: Handle,
            value_name: *const u16,
            reserved: *mut u32,
            value_type: *mut u32,
            data: *mut u8,
            data_length: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: Handle) -> i32;
    }

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_NO_MORE_FILES: i32 = 18;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_NO_MORE_ITEMS: i32 = 259;
    const KEY_READ: u32 = 0x0002_0019;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const MAX_VALUE_BYTES: u32 = 32 * 1024;

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn wide_str(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn text_of(buffer: &[u16]) -> String {
        let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    /// Bytes this user may use on the volume of `path`, asked of the deepest folder of it that
    /// exists (the root itself usually does not yet).
    pub fn free_space(path: &Path) -> Result<u64, Said> {
        let existing = path.ancestors().find(|dir| dir.is_dir()).ok_or_else(|| {
            Said::new("路径所在的盘不存在", "the path's drive does not exist").with(path.display())
        })?;
        let directory = wide(existing.as_os_str());
        let mut free = 0u64;
        // SAFETY: `directory` is NUL-terminated UTF-16 that outlives the call; `free` is a valid
        // out pointer; the two others may be null (GetDiskFreeSpaceExW's documented contract).
        let ok = unsafe {
            GetDiskFreeSpaceExW(directory.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut())
        };
        if ok == 0 {
            return Err(Said::new("剩余空间读不出", "the free space cannot be read")
                .with(format!("{}: {}", existing.display(), io::Error::last_os_error())));
        }
        Ok(free)
    }

    /// Every running process with its image path where it can be read.
    pub fn processes() -> Result<Vec<ProcessInfo>, Said> {
        let failed = |stage: &str| {
            Said::new("进程列表读不出", "the process list cannot be read")
                .with(format!("{stage}: {}", io::Error::last_os_error()))
        };
        // SAFETY: a plain snapshot request; the handle is checked and closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot.is_null() || snapshot as isize == -1 {
            return Err(failed("CreateToolhelp32Snapshot"));
        }
        let mut entry = ProcessEntry32W {
            size: std::mem::size_of::<ProcessEntry32W>() as u32,
            usage: 0,
            process_id: 0,
            default_heap_id: 0,
            module_id: 0,
            threads: 0,
            parent_process_id: 0,
            priority_class_base: 0,
            flags: 0,
            exe_file: [0; 260],
        };
        let mut list = Vec::new();
        // SAFETY: `entry` is a valid PROCESSENTRY32W with its size set; `snapshot` is open.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        let mut result = Ok(());
        if !more {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES) {
                result = Err(Said::new("进程列表读不出", "the process list cannot be read")
                    .with(format!("Process32FirstW: {error}")));
            }
        }
        while more {
            let pid = entry.process_id;
            list.push(ProcessInfo {
                pid,
                name: text_of(&entry.exe_file),
                path: image_path(pid),
            });
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
            if !more {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_NO_MORE_FILES) {
                    result = Err(Said::new("进程列表读不出", "the process list cannot be read")
                        .with(format!("Process32NextW: {error}")));
                }
            }
        }
        // SAFETY: closes the snapshot this function opened, once.
        unsafe { CloseHandle(snapshot) };
        result.map(|()| list)
    }

    fn image_path(pid: u32) -> Result<PathBuf, String> {
        if pid == 0 {
            return Err("system idle process".to_owned());
        }
        // SAFETY: a query-only handle to `pid`; checked and closed below.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut buffer = vec![0u16; 32 * 1024];
        let mut size = buffer.len() as u32;
        // SAFETY: `buffer` holds `size` UTF-16 units; `size` is updated to the length written.
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) };
        let error = io::Error::last_os_error();
        // SAFETY: closes the handle opened above, once.
        unsafe { CloseHandle(process) };
        if ok == 0 {
            return Err(error.to_string());
        }
        Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])))
    }

    struct Key(Handle);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful RegOpenKeyExW and is closed once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn hive(label: &str) -> Handle {
        // HKEY_LOCAL_MACHINE and HKEY_CURRENT_USER: the sign-extended predefined handles.
        let value: i32 = if label == "HKLM" { 0x8000_0002u32 as i32 } else { 0x8000_0001u32 as i32 };
        value as isize as Handle
    }

    fn open(parent: Handle, sub_key: &str, key_path: &str) -> Result<Option<Key>, Said> {
        let mut handle: Handle = std::ptr::null_mut();
        let name = wide_str(sub_key);
        // SAFETY: `name` is NUL-terminated UTF-16 and the out pointer is valid.
        let status = unsafe { RegOpenKeyExW(parent, name.as_ptr(), 0, KEY_READ, &mut handle) };
        match status {
            ERROR_SUCCESS => Ok(Some(Key(handle))),
            ERROR_FILE_NOT_FOUND => Ok(None),
            code => Err(Said::new("注册表读不出", "the registry cannot be read")
                .with(format!("open {key_path}: {}", io::Error::from_raw_os_error(code)))),
        }
    }

    fn sub_keys(key: &Key, key_path: &str) -> Result<Vec<String>, Said> {
        let mut names = Vec::new();
        let mut index = 0u32;
        loop {
            let mut buffer = [0u16; 256];
            let mut length = buffer.len() as u32;
            // SAFETY: the buffer and its length are valid; the unused outputs are null.
            let status = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => names.push(String::from_utf16_lossy(&buffer[..length as usize])),
                ERROR_NO_MORE_ITEMS => return Ok(names),
                code => {
                    return Err(Said::new("注册表读不出", "the registry cannot be read")
                        .with(format!("enumerate {key_path}: {}", io::Error::from_raw_os_error(code))))
                }
            }
            index += 1;
        }
    }

    fn string_value(key: &Key, key_path: &str, name: &str) -> Result<Option<String>, Said> {
        let value_name = wide_str(name);
        let mut value_type = 0u32;
        let mut size = 0u32;
        // SAFETY: a null data pointer asks for the size only; the other pointers are valid.
        let status = unsafe {
            RegQueryValueExW(key.0, value_name.as_ptr(), std::ptr::null_mut(), &mut value_type, std::ptr::null_mut(), &mut size)
        };
        let unusable = |detail: String| {
            Said::new("注册表项读不出", "a registry value cannot be read").with(format!("{key_path}\\{name}: {detail}"))
        };
        match status {
            ERROR_SUCCESS => {}
            ERROR_FILE_NOT_FOUND => return Ok(None),
            code => return Err(unusable(io::Error::from_raw_os_error(code).to_string())),
        }
        if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
            return Ok(None);
        }
        if size > MAX_VALUE_BYTES {
            return Err(unusable(format!("{size} bytes")));
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut written = (buffer.len() * 2) as u32;
        // SAFETY: `buffer` holds `written` bytes.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                std::ptr::null_mut(),
                &mut value_type,
                buffer.as_mut_ptr().cast::<u8>(),
                &mut written,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(unusable(io::Error::from_raw_os_error(status).to_string()));
        }
        buffer.truncate((written as usize / 2).min(buffer.len()));
        Ok(Some(text_of(&buffer)))
    }

    /// The uninstall entries whose key name starts with `prefix` (ASCII case folded), read from
    /// the three standard uninstall keys (Runtime `crates/device/src/mumu_manager.rs`, module
    /// `registry`); only `InstallLocation`, `DisplayIcon` and `DisplayVersion` are read.
    pub fn uninstall_entries(prefix: &str) -> Result<Vec<UninstallEntry>, Said> {
        const KEYS: [(&str, &str); 3] = [
            ("HKLM", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
            ("HKLM", r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
            ("HKCU", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        ];
        let prefix = prefix.to_ascii_lowercase();
        let mut entries = Vec::new();
        for (label, uninstall) in KEYS {
            let uninstall_path = format!(r"{label}\{uninstall}");
            let Some(parent) = open(hive(label), uninstall, &uninstall_path)? else {
                continue;
            };
            for name in sub_keys(&parent, &uninstall_path)? {
                if !name.to_ascii_lowercase().starts_with(&prefix) {
                    continue;
                }
                let key_path = format!(r"{uninstall_path}\{name}");
                let Some(entry) = open(parent.0, &name, &key_path)? else {
                    continue;
                };
                entries.push(UninstallEntry {
                    install_location: string_value(&entry, &key_path, "InstallLocation")?,
                    display_icon: string_value(&entry, &key_path, "DisplayIcon")?,
                    display_version: string_value(&entry, &key_path, "DisplayVersion")?,
                    key_path,
                });
            }
        }
        Ok(entries)
    }

    /// A known folder's path as the shell has it, whether or not it exists yet.
    fn known_folder(id: &windows::core::GUID, name: &Said) -> Result<PathBuf, Said> {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY};
        // SAFETY: `id` is a valid KNOWNFOLDERID; the returned buffer is the caller's, read once
        // and freed with CoTaskMemFree as documented.
        let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) }.map_err(|error| {
            Said::new(format!("找不到{}文件夹", name.zh), format!("cannot find the {} folder", name.en)).with(error)
        })?;
        // SAFETY: `path` is the NUL-terminated string SHGetKnownFolderPath returned.
        let text = unsafe { path.to_string() };
        // SAFETY: frees the buffer SHGetKnownFolderPath allocated, once.
        unsafe { CoTaskMemFree(Some(path.0 as *const c_void)) };
        text.map(PathBuf::from).map_err(|error| {
            Said::new(format!("{}文件夹路径读不出", name.zh), format!("the {} folder's path is unreadable", name.en))
                .with(error)
        })
    }

    pub fn desktop_dir() -> Result<PathBuf, Said> {
        known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop, &Said::new("桌面", "desktop"))
    }

    pub fn start_menu_programs_dir() -> Result<PathBuf, Said> {
        known_folder(
            &windows::Win32::UI::Shell::FOLDERID_Programs,
            &Said::new("开始菜单", "Start menu"),
        )
    }

    /// A shortcut at `lnk` to `target`, started in `workdir`, written through the shell's own
    /// IShellLink in a single-threaded apartment entered and left here.
    pub fn create_shortcut(lnk: &Path, target: &Path, workdir: &Path, description: &str) -> Result<(), Said> {
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
            Said::new("快捷方式建不了", "the shortcut cannot be created").with(format!("{}: {error}", lnk.display()))
        };
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(&error))?;
        }
        // SAFETY: plain COM initialisation of the calling thread.
        let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        entered.ok().map_err(|error| failed(&error))?;
        let _apartment = Apartment;
        // SAFETY: every call goes through the interfaces the shell returned, with strings that
        // outlive the calls.
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|error| failed(&error))?;
            link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|error| failed(&error))?;
            link.SetWorkingDirectory(&HSTRING::from(workdir.as_os_str()))
                .map_err(|error| failed(&error))?;
            link.SetDescription(&HSTRING::from(description)).map_err(|error| failed(&error))?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)
                .map_err(|error| failed(&error))?;
            link.cast::<IPersistFile>()
                .and_then(|file| file.Save(&HSTRING::from(lnk.as_os_str()), true))
                .map_err(|error| failed(&error))?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{windows_only, ProcessInfo, UninstallEntry};
    use crate::text::Said;
    use std::path::{Path, PathBuf};

    pub fn free_space(_path: &Path) -> Result<u64, Said> {
        Err(windows_only())
    }

    pub fn processes() -> Result<Vec<ProcessInfo>, Said> {
        Err(windows_only())
    }

    pub fn uninstall_entries(_prefix: &str) -> Result<Vec<UninstallEntry>, Said> {
        Err(windows_only())
    }

    pub fn desktop_dir() -> Result<PathBuf, Said> {
        Err(windows_only())
    }

    pub fn start_menu_programs_dir() -> Result<PathBuf, Said> {
        Err(windows_only())
    }

    pub fn create_shortcut(_lnk: &Path, _target: &Path, _workdir: &Path, _description: &str) -> Result<(), Said> {
        Err(windows_only())
    }
}

pub use imp::{create_shortcut, desktop_dir, free_space, processes, start_menu_programs_dir, uninstall_entries};

/// The processes whose image file name is `name` (ASCII case folded).
pub fn processes_named(name: &str) -> Result<Vec<ProcessInfo>, Said> {
    Ok(processes()?
        .into_iter()
        .filter(|process| process.name.eq_ignore_ascii_case(name))
        .collect())
}
