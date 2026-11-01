// SPDX-License-Identifier: GPL-3.0-only
//! From a window as the outside holds it to its Win32 handle, and from that
//! handle to the key the native state is held by.

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

use crate::{GlassError, Step};

/// The live Win32 window behind `window`.
pub fn of_window(window: &impl HasWindowHandle) -> Result<HWND, GlassError> {
    let failed = |detail: String| GlassError { step: Step::WindowHandle, detail };
    let handle = window.window_handle().map_err(|error| failed(error.to_string()))?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(failed("not a Win32 window".to_owned()));
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    // SAFETY: IsWindow takes any value as a handle and only answers whether it
    // names a window.
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Ok(hwnd)
    } else {
        Err(failed("the handle names no window".to_owned()))
    }
}

pub fn key(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

pub fn from_key(key: isize) -> HWND {
    HWND(key as *mut c_void)
}
