// SPDX-License-Identifier: GPL-3.0-only
//! Windows: the frame's native half, a subclass on each window that gives it
//! its own hit test (`subclass`). The glass behind the frame is not built yet:
//! a window's glass area is simply clear, and its calls answer as unsupported.

mod dwm;
mod hittest;
mod hwnd;
mod subclass;

pub use subclass::{attach, detach, detach_to_sink, same_window, set_chrome, set_geometry};

use crate::{Capability, Fade, GlassError, Step};

fn unsupported() -> GlassError {
    GlassError { step: Step::Unsupported, detail: "the Aero glass is not built yet".to_owned() }
}

pub fn capability() -> Capability {
    Capability::Glass
}

pub fn show_glass(_key: isize, _fade: Option<Fade>) -> Result<(), GlassError> {
    Err(unsupported())
}

pub fn fade_out(_key: isize, _fade: Fade) -> Result<(), GlassError> {
    Err(unsupported())
}

pub fn hide_glass(_key: isize) -> Result<(), GlassError> {
    Err(unsupported())
}
