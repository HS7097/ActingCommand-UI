// SPDX-License-Identifier: GPL-3.0-only
//! Windows. Nothing is built on it yet: it answers like every other platform
//! (`unsupported`), with the same calls the native layer will fill in, so the
//! console keeps the system's frame and there is never a `Frame`.

use std::rc::Rc;

use raw_window_handle::HasWindowHandle;

use crate::{Capability, Fade, FrameEvent, Geometry, GlassError, Step};

fn unsupported() -> GlassError {
    GlassError { step: Step::Unsupported, detail: "the Aero frame is not built yet".to_owned() }
}

pub fn capability() -> Capability {
    Capability::SolidOnly
}

pub fn attach(
    _window: &impl HasWindowHandle,
    _geometry: Geometry,
    _sigma: f32,
    _sink: Rc<dyn Fn(FrameEvent)>,
) -> Result<isize, GlassError> {
    Err(unsupported())
}

pub fn same_window(_key: isize, _window: &impl HasWindowHandle) -> bool {
    false
}

pub fn set_geometry(_key: isize, _geometry: Geometry) {}

pub fn set_chrome(_key: isize, _on: bool) {}

pub fn show_glass(_key: isize, _fade: Option<Fade>) -> Result<(), GlassError> {
    Err(unsupported())
}

pub fn fade_out(_key: isize, _fade: Fade) -> Result<(), GlassError> {
    Err(unsupported())
}

pub fn hide_glass(_key: isize) -> Result<(), GlassError> {
    Err(unsupported())
}

pub fn detach(_key: isize) -> Result<(), GlassError> {
    Err(unsupported())
}

pub fn detach_to_sink(_key: isize) {}
