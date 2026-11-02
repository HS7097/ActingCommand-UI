// SPDX-License-Identifier: GPL-3.0-only
//! Which part of the frame a point of the window is.

use windows::Win32::UI::WindowsAndMessaging::{
    HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT,
};

use crate::Geometry;

/// How far a resize corner reaches along each edge, in logical px, when the
/// frame itself is narrower: an 8px frame alone is a hard target.
const CORNER_REACH: f32 = 16.0;

/// The hit-test code of `point` in a client area of `client` size, both in
/// physical px. `scale` is the window's DPI over 96: the logical `geometry`
/// times that is the size on screen, as Slint draws it (nothing here sets
/// `SLINT_SCALE_FACTOR`, under which the two could differ). A maximized window
/// has no edges to pull. Inside the caption only the buttons are client area,
/// so nothing else there that can be clicked ever gets the pointer.
pub fn hit(point: (i32, i32), client: (i32, i32), geometry: &Geometry, scale: f32, zoomed: bool) -> u32 {
    let (x, y) = (point.0 as f32, point.1 as f32);
    let (width, height) = (client.0 as f32, client.1 as f32);
    if !zoomed {
        let corner = geometry.frame.max(CORNER_REACH) * scale;
        let (left, right) = (x < corner, x >= width - corner);
        let (top, bottom) = (y < corner, y >= height - corner);
        if top && left {
            return HTTOPLEFT;
        }
        if top && right {
            return HTTOPRIGHT;
        }
        if bottom && left {
            return HTBOTTOMLEFT;
        }
        if bottom && right {
            return HTBOTTOMRIGHT;
        }
        let edge = geometry.frame * scale;
        if x < edge {
            return HTLEFT;
        }
        if x >= width - edge {
            return HTRIGHT;
        }
        if y < edge {
            return HTTOP;
        }
        if y >= height - edge {
            return HTBOTTOM;
        }
    }
    let buttons_right = width - geometry.buttons_inset * scale;
    let buttons_left = buttons_right - geometry.buttons_width * scale;
    if x >= buttons_left && x < buttons_right && y < geometry.buttons_height * scale {
        return HTCLIENT;
    }
    if y < geometry.caption * scale {
        return HTCAPTION;
    }
    HTCLIENT
}
