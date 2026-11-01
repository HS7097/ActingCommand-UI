// SPDX-License-Identifier: GPL-3.0-only
//! The DWM attributes the frame would like. Each is a nicety on a Windows that
//! has it: not having it stops nothing, so a failure is only text for the log.

use std::ffi::c_void;
use std::mem::size_of_val;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT,
    DWMWCP_ROUNDSMALL,
};

/// The window's corners: the small rounding that sits under the frame's own,
/// or the system's default. Only Windows 11 has this attribute.
pub fn set_corners(hwnd: HWND, own_frame: bool) -> Result<(), String> {
    let corner = if own_frame { DWMWCP_ROUNDSMALL } else { DWMWCP_DEFAULT };
    // SAFETY: the pointer and the size describe `corner`, which outlives the
    // call; a stale handle only makes the call fail.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const c_void,
            size_of_val(&corner) as u32,
        )
    }
    .map_err(|error| error.to_string())
}

/// No accent-coloured border round the glass's window: the frame draws the
/// edge. Only Windows 11 has this attribute.
pub fn set_no_border(hwnd: HWND) -> Result<(), String> {
    let color = DWMWA_COLOR_NONE;
    // SAFETY: the pointer and the size describe `color`, which outlives the
    // call; a stale handle only makes the call fail.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &color as *const _ as *const c_void,
            size_of_val(&color) as u32,
        )
    }
    .map_err(|error| error.to_string())
}
