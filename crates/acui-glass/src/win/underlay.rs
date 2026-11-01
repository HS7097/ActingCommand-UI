// SPDX-License-Identifier: GPL-3.0-only
//! One window's glass. Composition draws above a window's own pixels, so the
//! glass lives on a second window of its own, without input or a surface, kept
//! right under the main window's client area. On it, Composition takes the raw
//! pixels behind (`CreateBackdropBrush`; the host backdrop comes already
//! blurred) through `gaussian`, and the main window's clear glass area shows
//! the result. The window has no owner: an owned window would be raised over
//! its owner.
//!
//! This holds no table and calls nothing back: `subclass` keeps each window's
//! layer, says what became of it, and takes it down. All the Composition calls
//! here are safe ones; only the Win32 ones are in `unsafe`.

use std::cell::Cell;
use std::fmt;

use windows::core::{w, Interface, Result as WinResult, HSTRING, PCWSTR};
use windows::Foundation::TimeSpan;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionEffectBrush, CompositionEffectSourceParameter, CompositionRoundedRectangleGeometry, Compositor,
    SpriteVisual,
};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetWindowRect, IsIconic, IsWindowVisible, SetWindowPos,
    ShowWindow, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_DISABLED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_numerics::{Vector2, Vector3};

use super::{context, gaussian};
use crate::{Easing, Fade, GlassError, Step};

/// The name the blur's source goes by inside its effect.
const SOURCE: &str = "Behind";

thread_local! {
    /// The layers this process has built and taken down, for the log's lines.
    static BUILT: Cell<u32> = const { Cell::new(0) };
    static DESTROYED: Cell<u32> = const { Cell::new(0) };
}

/// Where a layer stands after being placed.
#[derive(Clone, Copy)]
pub enum Placed {
    /// The main window is out of sight, so the layer is hidden.
    Hidden,
    /// Both rectangles on screen, in physical px.
    At { main: RECT, underlay: RECT, dpi: u32 },
}

fn rect_text(rect: &RECT) -> String {
    format!("{}x{} @ ({}, {})", rect.right - rect.left, rect.bottom - rect.top, rect.left, rect.top)
}

impl fmt::Display for Placed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Placed::Hidden => write!(f, "主窗不可见，垫底窗隐藏"),
            Placed::At { main, underlay, dpi } => {
                write!(f, "主窗矩形 {}，垫底窗矩形 {}，DPI {dpi}", rect_text(main), rect_text(underlay))
            }
        }
    }
}

pub struct Layer {
    main: HWND,
    window: HWND,
    clip: CompositionRoundedRectangleGeometry,
    visual: SpriteVisual,
    /// What the running animations are made with, and what keeps the layer drawn.
    compositor: Compositor,
    _brush: CompositionEffectBrush,
    _target: DesktopWindowTarget,
    /// An animation was started, and may still run.
    fading: Cell<bool>,
    last: Cell<Placed>,
    /// The DPI it was last seen on; 0 until then.
    dpi: Cell<u32>,
    place_failures: Cell<u32>,
}

fn failed(step: Step) -> impl Fn(windows::core::Error) -> GlassError {
    move |error| GlassError { step, detail: error.to_string() }
}

/// The window's DPI as it stands at this moment.
fn dpi_of(hwnd: HWND) -> Result<u32, String> {
    // SAFETY: only reads the window's state; a stale handle answers 0.
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => Err("GetDpiForWindow answered 0".to_owned()),
        dpi => Ok(dpi),
    }
}

fn create_window() -> Result<HWND, GlassError> {
    // SAFETY: the class name is a static string of a system class, every other
    // pointer argument is empty, and the handle comes back owned by this thread.
    unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            w!("STATIC"),
            PCWSTR::null(),
            WS_POPUP | WS_DISABLED,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .map_err(failed(Step::UnderlayWindow))
}

fn destroy_window(window: HWND) -> Result<(), String> {
    // SAFETY: `window` is a layer's own, made on this thread, and is gone from
    // every structure that named it; a stale handle only makes the call fail.
    unsafe { DestroyWindow(window) }.map_err(|error| format!("DestroyWindow: {error}"))
}

/// The backdrop through the blur.
fn blur_brush(compositor: &Compositor, sigma: f32) -> WinResult<CompositionEffectBrush> {
    let source = CompositionEffectSourceParameter::Create(&HSTRING::from(SOURCE))?;
    let effect = gaussian::blur(sigma, source.cast()?);
    let brush = compositor.CreateEffectFactory(&effect)?.CreateBrush()?;
    brush.SetSourceParameter(&HSTRING::from(SOURCE), &compositor.CreateBackdropBrush()?)?;
    Ok(brush)
}

/// A visual filling the window with `brush`, inside rounded corners.
fn filled_visual(
    compositor: &Compositor,
    brush: &CompositionEffectBrush,
) -> WinResult<(SpriteVisual, CompositionRoundedRectangleGeometry)> {
    let visual = compositor.CreateSpriteVisual()?;
    visual.SetBrush(brush)?;
    visual.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
    let clip = compositor.CreateRoundedRectangleGeometry()?;
    visual.SetClip(&compositor.CreateGeometricClipWithGeometry(&clip)?)?;
    Ok((visual, clip))
}

/// Everything Composition draws on the window just made.
fn dress(main: HWND, window: HWND, compositor: Compositor, sigma: f32) -> Result<Layer, GlassError> {
    let interop = compositor.cast::<ICompositorDesktopInterop>().map_err(failed(Step::Target))?;
    // SAFETY: `window` is a live window of this thread, made just now, with no
    // other target on it.
    let target = unsafe { interop.CreateDesktopWindowTarget(window, false) }.map_err(failed(Step::Target))?;
    let dpi = dpi_of(main).map_err(|detail| GlassError { step: Step::Brush, detail })?;
    let strength = sigma.max(0.0) * dpi as f32 / 96.0;
    let brush = blur_brush(&compositor, strength).map_err(failed(Step::Brush))?;
    let (visual, clip) = filled_visual(&compositor, &brush).map_err(failed(Step::Brush))?;
    target.SetRoot(&visual).map_err(failed(Step::Target))?;
    Ok(Layer {
        main,
        window,
        clip,
        visual,
        compositor,
        _brush: brush,
        _target: target,
        fading: Cell::new(false),
        last: Cell::new(Placed::Hidden),
        dpi: Cell::new(0),
        place_failures: Cell::new(0),
    })
}

/// `error`, with what became of the cleaning up after it if that failed too.
pub fn with_cleanup(error: GlassError, cleanup: Result<(), String>) -> GlassError {
    match cleanup {
        Ok(()) => error,
        Err(detail) => GlassError { step: error.step, detail: format!("{}; cleaning up: {detail}", error.detail) },
    }
}

impl Layer {
    /// The layer under `main`, hidden and not placed, `sigma` logical px strong
    /// at `main`'s DPI now. A window that moves to another DPI keeps that blur:
    /// the effect is fixed when it is made.
    pub fn build(main: HWND, sigma: f32) -> Result<Layer, GlassError> {
        let compositor = context::compositor()?;
        let window = create_window()?;
        dress(main, window, compositor, sigma).map_err(|error| with_cleanup(error, destroy_window(window)))
    }

    pub fn window(&self) -> HWND {
        self.window
    }

    /// The DPI it was last seen on; 0 if it has not been seen yet.
    pub fn dpi(&self) -> u32 {
        self.dpi.get()
    }

    fn hide(&self) {
        // SAFETY: `window` is this layer's own; a stale handle only does nothing.
        // The answer is only whether it was visible before.
        unsafe { ShowWindow(self.window, SW_HIDE) }.as_bool();
    }

    /// Puts the layer exactly under the main window's client area, right below
    /// it and so above nothing else of the user's, or hides it if the main
    /// window is out of sight. The corners are `corner` logical px at the main
    /// window's DPI now.
    pub fn place(&self, corner: f32) -> Result<Placed, String> {
        // SAFETY: both only read the window's state; a stale handle answers false.
        let out_of_sight = unsafe { IsIconic(self.main).as_bool() || !IsWindowVisible(self.main).as_bool() };
        if out_of_sight {
            self.hide();
            self.last.set(Placed::Hidden);
            return Ok(Placed::Hidden);
        }
        let mut client = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: the two out-pointers are to locals; a stale handle only makes
        // the calls fail.
        let (sized, mapped) =
            unsafe { (GetClientRect(self.main, &mut client), ClientToScreen(self.main, &mut origin)) };
        sized.map_err(|error| format!("GetClientRect: {error}"))?;
        if !mapped.as_bool() {
            return Err("ClientToScreen failed".to_owned());
        }
        let dpi = dpi_of(self.main)?;
        let (width, height) = (client.right - client.left, client.bottom - client.top);
        let size = Vector2 { X: width as f32, Y: height as f32 };
        let radius = corner * dpi as f32 / 96.0;
        self.clip.SetSize(size).map_err(|error| format!("clip size: {error}"))?;
        self.clip.SetCornerRadius(Vector2 { X: radius, Y: radius }).map_err(|error| format!("clip corners: {error}"))?;
        // The fade's zoom is about the middle, as the window's own.
        self.visual
            .SetCenterPoint(Vector3 { X: size.X / 2.0, Y: size.Y / 2.0, Z: 0.0 })
            .map_err(|error| format!("centre: {error}"))?;
        let mut underlay = RECT::default();
        // SAFETY: both windows are named by handles, a stale one only makes the
        // call fail, and the out-pointer is to a local.
        unsafe {
            SetWindowPos(
                self.window,
                Some(self.main),
                origin.x,
                origin.y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
            .map_err(|error| format!("SetWindowPos: {error}"))?;
            GetWindowRect(self.window, &mut underlay).map_err(|error| format!("GetWindowRect: {error}"))?;
        }
        let main = RECT { left: origin.x, top: origin.y, right: origin.x + width, bottom: origin.y + height };
        let placed = Placed::At { main, underlay, dpi };
        self.last.set(placed);
        self.dpi.set(dpi);
        Ok(placed)
    }

    /// Counts a failure to place; answers how many there have been.
    pub fn place_failed(&self) -> u32 {
        let failures = self.place_failures.get() + 1;
        self.place_failures.set(failures);
        failures
    }

    /// Fades and zooms from `from` to `to` (opacity, scale), with the curve of
    /// the same name in Slint.
    fn animate(&self, from: (f32, f32), to: (f32, f32), fade: Fade) -> WinResult<()> {
        let (start, end) = match fade.easing {
            Easing::EaseOut => (Vector2 { X: 0.0, Y: 0.0 }, Vector2 { X: 0.58, Y: 1.0 }),
            Easing::EaseIn => (Vector2 { X: 0.42, Y: 0.0 }, Vector2 { X: 1.0, Y: 1.0 }),
        };
        let compositor = &self.compositor;
        let curve = compositor.CreateCubicBezierEasingFunction(start, end)?;
        let duration = TimeSpan { Duration: i64::from(fade.millis.max(1)) * 10_000 };
        let opacity = compositor.CreateScalarKeyFrameAnimation()?;
        opacity.InsertKeyFrame(0.0, from.0)?;
        opacity.InsertKeyFrameWithEasingFunction(1.0, to.0, &curve)?;
        opacity.SetDuration(duration)?;
        let zoom = compositor.CreateVector3KeyFrameAnimation()?;
        zoom.InsertKeyFrame(0.0, Vector3 { X: from.1, Y: from.1, Z: 1.0 })?;
        zoom.InsertKeyFrameWithEasingFunction(1.0, Vector3 { X: to.1, Y: to.1, Z: 1.0 }, &curve)?;
        zoom.SetDuration(duration)?;
        self.fading.set(true);
        self.visual.StartAnimation(&HSTRING::from("Opacity"), &opacity)?;
        self.visual.StartAnimation(&HSTRING::from("Scale"), &zoom)
    }

    pub fn fade_in(&self, fade: Fade) -> Result<(), String> {
        self.animate((0.0, fade.from_scale), (1.0, 1.0), fade).map_err(|error| format!("fade-in: {error}"))
    }

    /// Fades and shrinks with the closing window; it stays placed, clear.
    pub fn fade_out(&self, fade: Fade) -> Result<(), String> {
        self.animate((1.0, 1.0), (0.0, fade.from_scale), fade).map_err(|error| format!("fade-out: {error}"))
    }

    fn to_full_view(&self) -> WinResult<()> {
        if self.fading.replace(false) {
            self.visual.StopAnimation(&HSTRING::from("Opacity"))?;
            self.visual.StopAnimation(&HSTRING::from("Scale"))?;
        }
        self.visual.SetOpacity(1.0)?;
        self.visual.SetScale(Vector3 { X: 1.0, Y: 1.0, Z: 1.0 })
    }

    /// In full view now, whatever an earlier fade was doing or left behind.
    pub fn show_at_once(&self) -> Result<(), String> {
        self.to_full_view().map_err(|error| format!("full view: {error}"))
    }

    /// Off the screen and gone. Its Composition objects go first, then the window
    /// they draw on. The compositor they were made with stays (`context`).
    pub fn destroy(self) -> Result<(), String> {
        let window = self.window;
        self.hide();
        drop(self);
        destroy_window(window)
    }

    /// How it stood, for the log's line on its going.
    pub fn summary(&self) -> String {
        format!("{}，跟随定位失败 {} 次", self.last.get(), self.place_failures.get())
    }
}

/// The log's line on a layer built and placed, with the running totals: an equal
/// count of the two lines at the end of a run means none was left behind.
pub fn note_built(layer: &Layer) -> String {
    BUILT.set(BUILT.get() + 1);
    format!("垫底窗已建立：{}；累计建立 {}，销毁 {}", layer.last.get(), BUILT.get(), DESTROYED.get())
}

/// The log's line on a layer gone, from what `Layer::summary` said of it.
pub fn note_destroyed(summary: &str) -> String {
    DESTROYED.set(DESTROYED.get() + 1);
    format!("垫底窗已销毁：{summary}；累计建立 {}，销毁 {}", BUILT.get(), DESTROYED.get())
}
