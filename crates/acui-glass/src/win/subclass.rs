// SPDX-License-Identifier: GPL-3.0-only
//! The subclass on a window, and what the native layer holds for it: its glass
//! among the rest, which comes and goes here and follows the window from here.
//!
//! The state is in a table by window handle, on the window's own thread; the
//! subclass carries no pointer to it. Only `SetWindowSubclass` is used, never
//! `SetWindowLongPtrW`: accesskit subclassed the window first and, when it goes,
//! writes back the procedure it found, so a chain changed by hand would break.
//! That same write cuts comctl32's chain if accesskit goes first, and then
//! `WM_NCDESTROY` never reaches this code: whoever closes a window detaches its
//! frame first, and `WM_NCDESTROY` is only the fallback.
//!
//! The procedure takes the table for a moment, clones what it needs and lets
//! go before it calls anything, so a message arriving inside a Win32 call finds
//! the table free. It never touches a UI: what it has to say goes to the sink.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use raw_window_handle::HasWindowHandle;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsZoomed, SIZE_MAXIMIZED, SIZE_RESTORED, SWP_NOSIZE, WINDOWPOS, WM_NCDESTROY, WM_NCHITTEST,
    WM_SIZE, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING,
};

use super::underlay::{self, Layer, Placed};
use super::{dwm, hittest, hwnd};
use crate::{Fade, FrameEvent, Geometry, GlassError, Step};

/// This code's subclass among others on the same window: "ACUI".
const ID_ACUI: usize = 0x4143_5549;

struct Entry {
    /// The hit test and the guard on the maximized size on: the window draws
    /// its own frame.
    chrome: Cell<bool>,
    geometry: Cell<Geometry>,
    maximized: Cell<bool>,
    /// The blur of the glass, in logical px.
    sigma: f32,
    /// The glass, while it stands. Borrowed mutably only to put it in or take it
    /// out, with no call made meanwhile, so a message arriving inside a call that
    /// reads it still finds it.
    layer: RefCell<Option<Layer>>,
    sink: Rc<dyn Fn(FrameEvent)>,
    /// Each said once for the window: a DWM attribute is missing, the hit test
    /// could not be worked out, the maximized size could not be checked.
    corners_noted: Cell<bool>,
    border_noted: Cell<bool>,
    hit_test_failed: Cell<bool>,
    size_guard_failed: Cell<bool>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<isize, Rc<Entry>>> = RefCell::new(HashMap::new());
}

fn entry(key: isize) -> Option<Rc<Entry>> {
    WINDOWS
        .try_with(|windows| {
            let windows = windows.try_borrow().ok()?;
            windows.get(&key).cloned()
        })
        .ok()
        .flatten()
}

fn take_entry(key: isize) -> Option<Rc<Entry>> {
    WINDOWS
        .try_with(|windows| {
            let mut windows = windows.try_borrow_mut().ok()?;
            windows.remove(&key)
        })
        .ok()
        .flatten()
}

fn subclass_failed(detail: impl Into<String>) -> GlassError {
    GlassError { step: Step::Subclass, detail: detail.into() }
}

pub fn attach(
    window: &impl HasWindowHandle,
    geometry: Geometry,
    sigma: f32,
    sink: Rc<dyn Fn(FrameEvent)>,
) -> Result<isize, GlassError> {
    let hwnd = hwnd::of_window(window)?;
    let key = hwnd::key(hwnd);
    if entry(key).is_some() {
        return Err(subclass_failed("this window has a frame already"));
    }
    // SAFETY: `hwnd` was just checked to be a window; the call only reads its state.
    let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
    let state = Rc::new(Entry {
        chrome: Cell::new(false),
        geometry: Cell::new(geometry),
        maximized: Cell::new(zoomed),
        sigma,
        layer: RefCell::new(None),
        sink,
        corners_noted: Cell::new(false),
        border_noted: Cell::new(false),
        hit_test_failed: Cell::new(false),
        size_guard_failed: Cell::new(false),
    });
    // In the table first: the procedure may run as soon as it is set.
    WINDOWS.with(|windows| windows.borrow_mut().insert(key, state));
    // SAFETY: `hwnd` is a live window; `subclass_proc` is a static function
    // that lives as long as the process; the reference data is unused.
    let set = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), ID_ACUI, 0) };
    if !set.as_bool() {
        take_entry(key);
        return Err(subclass_failed("SetWindowSubclass failed"));
    }
    Ok(key)
}

pub fn same_window(key: isize, window: &impl HasWindowHandle) -> bool {
    entry(key).is_some() && hwnd::of_window(window).is_ok_and(|hwnd| hwnd::key(hwnd) == key)
}

pub fn set_geometry(key: isize, geometry: Geometry) {
    if let Some(entry) = entry(key) {
        entry.geometry.set(geometry);
        follow(&entry);
    }
}

pub fn set_chrome(key: isize, on: bool) {
    let Some(entry) = entry(key) else {
        return;
    };
    entry.chrome.set(on);
    if let Err(detail) = dwm::set_corners(hwnd::from_key(key), on) {
        if !entry.corners_noted.replace(true) {
            (entry.sink)(FrameEvent::Info(format!("the DWM corner attribute was not set: {detail}")));
        }
    }
}

fn remove_subclass(hwnd: HWND) -> Result<(), GlassError> {
    // SAFETY: the same procedure and id `attach` set, on this thread (the table
    // and `Frame` stay on it); a handle gone stale only makes the call fail.
    let removed = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), ID_ACUI) };
    if removed.as_bool() {
        Ok(())
    } else {
        Err(subclass_failed("RemoveWindowSubclass failed"))
    }
}

/// Takes the glass and the subclass off a window out of the table, trying both.
fn take_down(hwnd: HWND, state: &Entry) -> Result<(), GlassError> {
    match (destroy_layer(state), remove_subclass(hwnd)) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(layer), Err(subclass)) => {
            Err(GlassError { step: Step::Destroy, detail: format!("{}; {subclass}", layer.detail) })
        }
    }
}

/// Takes everything off. A frame whose window has gone already, or that was
/// detached before, has nothing left to do.
pub fn detach(key: isize) -> Result<(), GlassError> {
    match take_entry(key) {
        Some(state) => take_down(hwnd::from_key(key), &state),
        None => Ok(()),
    }
}

/// `detach` with nobody to answer to: what went wrong goes to the sink.
pub fn detach_to_sink(key: isize) {
    if let Some(state) = take_entry(key) {
        if let Err(error) = take_down(hwnd::from_key(key), &state) {
            (state.sink)(FrameEvent::Failed(error));
        }
    }
}

fn step_failed(step: Step) -> impl Fn(String) -> GlassError {
    move |detail| GlassError { step, detail }
}

/// A new layer's first showing.
fn first_show(layer: &Layer, fade: Option<Fade>, corner: f32) -> Result<(), GlassError> {
    if let Some(fade) = fade {
        layer.fade_in(fade).map_err(step_failed(Step::Animate))?;
    }
    layer.place(corner).map_err(step_failed(Step::Place))?;
    Ok(())
}

/// Builds the glass if the window has none, and shows it: with `fade` when it is
/// new, at once when it was standing, however it was left. A layer that cannot
/// be shown does not stay.
pub fn show_glass(key: isize, fade: Option<Fade>) -> Result<(), GlassError> {
    let Some(state) = entry(key) else {
        return Err(GlassError { step: Step::UnderlayWindow, detail: "the frame's window has gone".to_owned() });
    };
    let corner = state.geometry.get().corner;
    let busy = || "the window's glass is busy".to_owned();
    let standing = state.layer.try_borrow().map_err(|_| step_failed(Step::UnderlayWindow)(busy()))?;
    if let Some(layer) = standing.as_ref() {
        layer.show_at_once().map_err(step_failed(Step::Animate))?;
        layer.place(corner).map_err(step_failed(Step::Place))?;
        return Ok(());
    }
    drop(standing);
    let layer = Layer::build(hwnd::from_key(key), state.sigma)?;
    if let Err(detail) = dwm::set_no_border(layer.window()) {
        if !state.border_noted.replace(true) {
            (state.sink)(FrameEvent::Info(format!("the DWM border attribute was not set: {detail}")));
        }
    }
    if let Err(error) = first_show(&layer, fade, corner) {
        return Err(underlay::with_cleanup(error, layer.destroy()));
    }
    let line = underlay::note_built(&layer);
    *state.layer.try_borrow_mut().map_err(|_| step_failed(Step::UnderlayWindow)(busy()))? = Some(layer);
    (state.sink)(FrameEvent::Info(line));
    Ok(())
}

/// The glass fades out with the closing window; a window without glass has
/// nothing to fade.
pub fn fade_out(key: isize, fade: Fade) -> Result<(), GlassError> {
    let Some(state) = entry(key) else {
        return Ok(());
    };
    let failed = step_failed(Step::Animate);
    let standing = state.layer.try_borrow().map_err(|_| failed("the window's glass is busy".to_owned()))?;
    match standing.as_ref() {
        Some(layer) => layer.fade_out(fade).map_err(failed),
        None => Ok(()),
    }
}

pub fn hide_glass(key: isize) -> Result<(), GlassError> {
    match entry(key) {
        Some(state) => destroy_layer(&state),
        None => Ok(()),
    }
}

/// The glass gone, if it stood, with the log's line on that.
fn destroy_layer(state: &Entry) -> Result<(), GlassError> {
    let failed = step_failed(Step::Destroy);
    let taken = state.layer.try_borrow_mut().map_err(|_| failed("the window's glass is busy".to_owned()))?.take();
    let Some(layer) = taken else {
        return Ok(());
    };
    let summary = layer.summary();
    layer.destroy().map_err(failed)?;
    (state.sink)(FrameEvent::Info(underlay::note_destroyed(&summary)));
    Ok(())
}

/// The glass follows its window. A failure is said once, and counted for the
/// line on its going; a new DPI is said for the log to show the new rectangles.
fn follow(state: &Entry) {
    let Ok(held) = state.layer.try_borrow() else {
        // Nothing holds it mutably across a call, so this does not happen.
        (state.sink)(FrameEvent::Failed(step_failed(Step::Place)("the window's glass is busy".to_owned())));
        return;
    };
    let Some(layer) = held.as_ref() else {
        return;
    };
    let dpi_before = layer.dpi();
    match layer.place(state.geometry.get().corner) {
        Ok(placed) => {
            if let Placed::At { dpi, .. } = placed {
                if dpi != dpi_before {
                    (state.sink)(FrameEvent::Info(format!("垫底窗到了新 DPI：{placed}")));
                }
            }
        }
        Err(detail) => {
            if layer.place_failed() == 1 {
                (state.sink)(FrameEvent::Failed(step_failed(Step::Place)(detail)));
            }
        }
    }
}

/// Passes a message down the chain.
fn forward(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: only called while `subclass_proc` handles this same message, with
    // its own values, which is the one place DefSubclassProc may be called from.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// The hit test of the screen point in `lparam`, or why it could not be worked out.
fn hit_test(hwnd: HWND, lparam: LPARAM, entry: &Entry) -> Result<u32, String> {
    // Two signed words: a monitor left of or above the main one has negative coordinates.
    let mut point = POINT {
        x: (lparam.0 & 0xffff) as u16 as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    let mut client = RECT::default();
    // SAFETY: `hwnd` is the window this message is for, alive through it, and
    // the two out-pointers are to locals.
    let (mapped, sized, dpi, zoomed) = unsafe {
        (
            ScreenToClient(hwnd, &mut point),
            GetClientRect(hwnd, &mut client),
            GetDpiForWindow(hwnd),
            IsZoomed(hwnd),
        )
    };
    if !mapped.as_bool() {
        return Err("ScreenToClient failed".to_owned());
    }
    sized.map_err(|error| format!("GetClientRect: {error}"))?;
    if dpi == 0 {
        return Err("GetDpiForWindow answered 0".to_owned());
    }
    let scale = dpi as f32 / 96.0;
    let geometry = entry.geometry.get();
    Ok(hittest::hit((point.x, point.y), (client.right, client.bottom), &geometry, scale, zoomed.as_bool()))
}

/// Keeps a maximized window's outer rectangle when it is asked to take the
/// client's size.
///
/// winit answers any change of the size limits by asking for the size the
/// client has already, and without a system frame that comes out as an outer
/// size equal to the client's. A maximized window's outer rectangle is a border
/// wider all round than its client, and the window stays maximized, so that
/// would cut the border's width off the client at the right and the bottom. The
/// limits change with the frame, the text size and the theme. The system's own
/// maximizing asks for the wider rectangle and passes through.
fn keep_maximized_size(hwnd: HWND, lparam: LPARAM) -> Result<(), String> {
    // SAFETY: `hwnd` is the window this message is for, alive through it; the
    // call only reads its state.
    if !unsafe { IsZoomed(hwnd) }.as_bool() {
        return Ok(());
    }
    // SAFETY: the `lparam` of WM_WINDOWPOSCHANGING points to the sender's
    // WINDOWPOS, valid and writable through the message and meant to be changed
    // by whoever handles it. The reference ends with this function, before the
    // message goes down the chain, and nothing called while it lives sends one.
    let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
    if position.flags.contains(SWP_NOSIZE) {
        return Ok(());
    }
    let mut client = RECT::default();
    // SAFETY: `hwnd` as above; the out-pointer is to a local.
    unsafe { GetClientRect(hwnd, &mut client) }.map_err(|error| format!("GetClientRect: {error}"))?;
    if (position.cx, position.cy) == (client.right, client.bottom) {
        position.flags |= SWP_NOSIZE;
    }
    Ok(())
}

fn handle_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let key = hwnd::key(hwnd);
    if message == WM_NCDESTROY {
        // Nobody detached this window before it went.
        let state = take_entry(key);
        let taken_down = match &state {
            Some(state) => take_down(hwnd, state),
            None => remove_subclass(hwnd),
        };
        if let (Err(error), Some(state)) = (taken_down, &state) {
            (state.sink)(FrameEvent::Failed(error));
        }
        return forward(hwnd, message, wparam, lparam);
    }
    let Some(state) = entry(key) else {
        return forward(hwnd, message, wparam, lparam);
    };
    match message {
        WM_NCHITTEST if state.chrome.get() => match hit_test(hwnd, lparam, &state) {
            Ok(code) => LRESULT(code as isize),
            Err(detail) => {
                if !state.hit_test_failed.replace(true) {
                    (state.sink)(FrameEvent::Failed(subclass_failed(detail)));
                }
                forward(hwnd, message, wparam, lparam)
            }
        },
        WM_WINDOWPOSCHANGING if state.chrome.get() => {
            if let Err(detail) = keep_maximized_size(hwnd, lparam) {
                if !state.size_guard_failed.replace(true) {
                    (state.sink)(FrameEvent::Failed(subclass_failed(detail)));
                }
            }
            forward(hwnd, message, wparam, lparam)
        }
        WM_WINDOWPOSCHANGED => {
            let result = forward(hwnd, message, wparam, lparam);
            // Moved, resized, raised, shown or hidden.
            follow(&state);
            result
        }
        WM_SIZE => {
            let result = forward(hwnd, message, wparam, lparam);
            let maximized = match wparam.0 as u32 {
                SIZE_MAXIMIZED => Some(true),
                SIZE_RESTORED => Some(false),
                _ => None,
            };
            if let Some(maximized) = maximized {
                if state.maximized.replace(maximized) != maximized {
                    (state.sink)(FrameEvent::Maximized(maximized));
                }
            }
            result
        }
        _ => forward(hwnd, message, wparam, lparam),
    }
}

/// A panic has to stop here: it cannot cross into the window procedures below.
/// It goes to the sink, and the message goes on as if this code were not there.
/// A sink that panicked as well would end the process, as a panic across this
/// boundary does.
fn report_panic(hwnd: HWND, panic: &(dyn Any + Send)) {
    let detail = panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "without a message".to_owned());
    if let Some(state) = entry(hwnd::key(hwnd)) {
        (state.sink)(FrameEvent::Failed(subclass_failed(format!("the window procedure panicked: {detail}"))));
    }
}

// SAFETY: comctl32 calls this with the arguments of a message for a window it
// was set on, on that window's thread. It reads no pointer it is given.
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| handle_message(hwnd, message, wparam, lparam))) {
        Ok(result) => result,
        Err(panic) => {
            report_panic(hwnd, &*panic);
            forward(hwnd, message, wparam, lparam)
        }
    }
}
