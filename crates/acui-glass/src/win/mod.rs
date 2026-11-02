// SPDX-License-Identifier: GPL-3.0-only
//! Windows: the frame's native half. A subclass on each window gives it its own
//! hit test and holds its state (`subclass`); a second window under it carries
//! its glass (`underlay`), through Composition (`context`) and a blur (`gaussian`).

mod context;
mod dwm;
mod gaussian;
mod hittest;
mod hwnd;
mod subclass;
mod underlay;

pub use subclass::{
    attach, detach, detach_to_sink, fade_out, hide_glass, same_window, set_chrome, set_geometry, show_glass,
};

use crate::Capability;

pub fn capability() -> Capability {
    Capability::Glass
}
