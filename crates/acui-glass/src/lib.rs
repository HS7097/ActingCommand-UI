// SPDX-License-Identifier: GPL-3.0-only
//! The native layer under the Aero theme's window frame: subclassing a window
//! for hit testing, and the Composition underlay window behind its glass.
//!
//! This is the only crate of the console that holds Win32 and WinRT code, and
//! this file is its whole face: safe, and the same on every platform. All the
//! unsafe code is in `win`. Where the frame cannot be drawn, `capability` says
//! so and `Frame::attach` fails. It knows nothing of Slint: a window comes in
//! through `HasWindowHandle`, and what happens later goes out through a sink.
//!
//! Everything here stays on the window's own thread (`Frame` is not `Send`);
//! only `FrameEvent` may cross threads, so a sink can post it instead of
//! touching a UI from inside a window procedure. The glass is built only when
//! `show_glass` asks, and says so to the sink when it comes and when it goes.

#![deny(unsafe_code)]

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use raw_window_handle::HasWindowHandle;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use unsupported as backend;

#[cfg(windows)]
#[allow(unsafe_code)]
mod win;
#[cfg(windows)]
use win as backend;

/// What this platform can draw of the Aero frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Its own frame over real glass.
    Glass,
    /// The system's frame around a solid window: `Frame::attach` always fails.
    SolidOnly,
}

/// What this platform can draw. Ask once: the answer does not change in a run.
pub fn capability() -> Capability {
    backend::capability()
}

/// The frame's measurements, in logical px. The native layer turns them into
/// physical px at each use, by the window's DPI at that moment.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub caption: f32,
    pub frame: f32,
    pub buttons_width: f32,
    pub buttons_height: f32,
    pub buttons_inset: f32,
    pub corner: f32,
}

#[derive(Debug, Clone, Copy)]
pub enum Easing {
    EaseOut,
    EaseIn,
}

/// The glass's part of a window opening or closing.
#[derive(Debug, Clone, Copy)]
pub struct Fade {
    pub millis: u32,
    pub from_scale: f32,
    pub easing: Easing,
}

/// What a frame reports after `attach` returned: maybe from inside the window
/// procedure, so a sink must not touch a UI, only post the event onward.
#[derive(Debug)]
pub enum FrameEvent {
    Maximized(bool),
    /// For the technical log, and no failure: something optional this Windows
    /// does not offer, or the glass built, gone or at a new DPI. Written as it
    /// comes, these lines reach the log even as the event loop ends.
    Info(String),
    Failed(GlassError),
}

/// The step that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Unsupported,
    WindowHandle,
    Subclass,
    Composition,
    UnderlayWindow,
    Target,
    Brush,
    Place,
    Animate,
    Destroy,
}

/// A failure of this layer. Its text is for the technical log, not for a player.
#[derive(Debug)]
pub struct GlassError {
    pub step: Step,
    pub detail: String,
}

impl fmt::Display for GlassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.step, self.detail)
    }
}

impl std::error::Error for GlassError {}

/// The native frame on one window. Dropping it detaches it too; a failure of
/// that goes to the sink, as `drop` has nowhere else to put it.
pub struct Frame {
    key: isize,
    /// Native state belongs to the window's thread.
    not_send: PhantomData<Rc<()>>,
}

impl Frame {
    /// Subclass `window`, with the hit test off and no glass yet.
    pub fn attach(
        window: &impl HasWindowHandle,
        geometry: Geometry,
        sigma: f32,
        sink: Rc<dyn Fn(FrameEvent)>,
    ) -> Result<Frame, GlassError> {
        let key = backend::attach(window, geometry, sigma, sink)?;
        Ok(Frame { key, not_send: PhantomData })
    }

    /// Whether this frame is on `window`'s native window as it is now.
    pub fn same_window(&self, window: &impl HasWindowHandle) -> bool {
        backend::same_window(self.key, window)
    }

    pub fn set_geometry(&self, geometry: Geometry) {
        backend::set_geometry(self.key, geometry);
    }

    /// The hit test on or off, with the DWM corner preference on a best-effort
    /// basis: a Windows without that attribute is not a failure.
    pub fn set_chrome(&self, on: bool) {
        backend::set_chrome(self.key, on);
    }

    /// Build the glass and show it, at once or with `fade`.
    pub fn show_glass(&self, fade: Option<Fade>) -> Result<(), GlassError> {
        backend::show_glass(self.key, fade)
    }

    pub fn fade_out(&self, fade: Fade) -> Result<(), GlassError> {
        backend::fade_out(self.key, fade)
    }

    /// Take the glass down. The subclass stays, passing messages on.
    pub fn hide_glass(&self) -> Result<(), GlassError> {
        backend::hide_glass(self.key)
    }

    /// Take everything off the window, and say whether that worked.
    pub fn detach(self) -> Result<(), GlassError> {
        let key = self.key;
        // The work is done here, with its result returned: `drop` must not repeat it.
        std::mem::forget(self);
        backend::detach(key)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        backend::detach_to_sink(self.key);
    }
}
