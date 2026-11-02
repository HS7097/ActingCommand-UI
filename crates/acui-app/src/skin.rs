// SPDX-License-Identifier: GPL-3.0-only
//! The theme on screen: the skin each window's own `Look` is drawn in, set
//! before the window shows and again in every window whenever a theme is
//! picked, and, in the Aero skin on Windows, the native frame under it. The
//! theme itself is the shared state's (`Ui::theme`); what a skin looks like is
//! the kit's (`ui/kit/tokens.slint`); what this platform can draw of the Aero
//! frame, and the frame's native half, are the native layer's (`acui_glass`).
//!
//! A window falls one level at a time when its native half fails, says so in
//! red and in the log, once per step, and tries again only when a theme is
//! picked. It is never moved to the flat skin behind a person's back.
//!
//! The native layer reports from inside a window procedure, so its events are
//! posted to the event loop before anything here looks at them.
//!
//! A window with a frame of its own has nothing of the system's to animate its
//! opening and closing, so it fades itself: it starts out of sight, comes in
//! once its frame is on, and goes out before it closes. Whatever becomes of the
//! frame, the window comes into sight. A window in the system's frame is left
//! to the system. A switch of theme does not fade.
//!
//! The glass is the native layer's second window under a window's frame, built
//! once the frame is on: `Look.glass` says it stands, and the frame's glass area
//! is clear only then. It fades with the window, in the same turn of the loop.
//! Its coming and going are in the log, so a run can be checked from there. A
//! window whose glass fails is drawn solid, and never quietly goes without.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use acui_glass::{Capability, Easing, Fade, Frame, FrameEvent, Geometry, GlassError, Step};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, SharedString, Timer};

use crate::report;
use crate::settings::Theme;
use crate::strings::Labels;
use crate::ui::{current, Ui};
use crate::{AeroConst, Look, Skin};

/// A new window has no native handle until the event loop has made it: how
/// often to look again, and how long apart.
const ATTACH_RETRIES: u32 = 10;
const ATTACH_RETRY_EVERY: Duration = Duration::from_millis(20);

/// How long a closing window stays after its fade has run out, for the last
/// frame of the fade to reach the screen.
const CLOSE_AFTER_FADE: Duration = Duration::from_millis(40);

const LOG_KIND: &str = "acui-glass";

/// Which window of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WindowKey {
    Workbench,
    Ledger(u64),
}

impl WindowKey {
    /// Runs `f` on this window, while it is open.
    fn with<R>(self, ui: &Ui, f: impl FnOnce(&slint::Window, Look<'_>, AeroConst<'_>) -> R) -> Option<R> {
        match self {
            WindowKey::Workbench => {
                let window = ui.workbench()?;
                Some(f(window.window(), window.global::<Look>(), window.global::<AeroConst>()))
            }
            WindowKey::Ledger(id) => {
                let handle = ui.ledger(id)?;
                let window = &handle.window;
                Some(f(window.window(), window.global::<Look>(), window.global::<AeroConst>()))
            }
        }
    }
}

/// How much of its theme a window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    /// Its own frame over see-through glass.
    AeroGlass,
    /// Its own frame over solid glass.
    AeroChromeSolid,
    /// The Aero page inside the system's frame. Where the platform has no
    /// frame of ours this is simply the Aero theme, and no failure.
    AeroSystemFrame,
    Flat,
}

impl Level {
    /// Whether the window draws its own frame at this level.
    fn has_own_frame(self) -> bool {
        matches!(self, Level::AeroGlass | Level::AeroChromeSolid)
    }
}

/// What a window says in red about its look.
#[derive(Debug, Clone, Copy)]
enum Failure {
    Chrome,
    Glass,
    Close,
}

impl Failure {
    fn text(self, labels: &Labels) -> &'static str {
        match self {
            Failure::Chrome => labels.chrome_failed,
            Failure::Glass => labels.glass_failed,
            Failure::Close => labels.close_failed,
        }
    }
}

/// How a window's glass comes up once its frame is on.
#[derive(Debug, Clone, Copy)]
enum Showing {
    /// With the window, which is fading in.
    Opening,
    /// A switch of theme, which does not fade.
    AtOnce,
}

/// How a window's close goes on, as `Skins::begin_close` found it.
#[derive(Debug, Clone, Copy)]
pub enum Closing {
    /// With nothing of its own to fade: the window can go now.
    AtOnce,
    /// It has begun to fade out: it goes when this time has passed.
    FadingFor(Duration),
    /// It is fading out from an earlier request, which stands.
    Underway,
}

/// The native side of one window's look.
struct WindowSkin {
    frame: Option<Frame>,
    level: Level,
    /// It is fading out: nothing but its close happens to it any more.
    closing: bool,
    /// What is said on the window now.
    failure: Option<Failure>,
    /// The steps already reported for it: a step fails loudly once.
    reported: Vec<Step>,
}

/// The windows' looks: what this platform can draw, and each window's native
/// side. A window's own skin is held by that window's `Look`.
pub struct Skins {
    capability: Capability,
    windows: RefCell<BTreeMap<WindowKey, WindowSkin>>,
}

impl Skins {
    /// Asks the native layer what this platform can draw, once for the run.
    pub fn detect() -> Self {
        Self { capability: acui_glass::capability(), windows: RefCell::new(BTreeMap::new()) }
    }

    /// Whether a window of `theme` draws its own frame here.
    fn frame_wanted(&self, theme: Theme) -> bool {
        theme == Theme::Aero && self.capability == Capability::Glass
    }

    /// Whether this window is at a level that has its own frame, and staying.
    fn has_chrome_level(&self, key: WindowKey) -> bool {
        self.windows.borrow().get(&key).is_some_and(|window| !window.closing && window.level.has_own_frame())
    }

    /// A theme picked while the window is open: its look changes on the spot,
    /// and whatever failed for it gets a new try. The hit test comes on after
    /// the window's own frame has been asked for, and goes off before the frame
    /// is given up, so the page's top is never taken for a title bar. The
    /// subclass stays on a window going flat, passing messages on, so a person
    /// switching back and forth does not put it on and off.
    pub fn switch(&self, ui: &Ui, key: WindowKey, theme: Theme) {
        let wanted = self.frame_wanted(theme);
        let hidden = {
            let mut windows = self.windows.borrow_mut();
            // A window on its way out takes no more of it.
            let Some(window) = windows.get_mut(&key).filter(|window| !window.closing) else {
                return;
            };
            window.level = level_of(theme, wanted);
            window.failure = None;
            window.reported.clear();
            match window.frame.as_ref().filter(|_| !wanted) {
                Some(frame) => {
                    let hidden = frame.hide_glass();
                    frame.set_chrome(false);
                    hidden
                }
                None => Ok(()),
            }
        };
        if let Err(error) = hidden {
            self.failed(ui, key, error);
        }
        key.with(ui, |_, look, _| {
            look.set_failure(SharedString::default());
            look.set_skin(skin_of(theme));
            look.set_glass(false);
            look.set_chrome(wanted);
            // One still waiting for its frame need not wait any more.
            look.set_appear(true);
        });
        if wanted {
            self.try_attach(ui, key, ATTACH_RETRIES, Showing::AtOnce);
        }
    }

    /// A window just shown: its frame goes on as soon as it has a native handle,
    /// and it fades in then.
    pub fn after_show(&self, key: WindowKey) {
        if !self.has_chrome_level(key) {
            return;
        }
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = current() {
                ui.skins.try_attach(&ui, key, ATTACH_RETRIES, Showing::Opening);
            }
        });
    }

    /// Puts the frame on and its hit test on, then the glass behind it, or looks
    /// again shortly for a handle that is not there yet. Out of tries, the window
    /// falls to the system's frame. Either way that is the end of the wait: the
    /// window comes into sight, in the same turn as its glass starts to fade in.
    fn try_attach(&self, ui: &Ui, key: WindowKey, retries_left: u32, showing: Showing) {
        // The theme changed, or the window is closing or went, since this was
        // asked for. A switch of theme brought it into sight itself.
        if !self.has_chrome_level(key) {
            return;
        }
        let attached = key.with(ui, |window, look, aero| self.put_frame(key, window, &look, &aero));
        match attached {
            None => {}
            Some(Ok(())) => {
                self.put_glass(ui, key, showing);
                key.with(ui, |_, look, _| look.set_appear(true));
            }
            Some(Err(error)) if error.step == Step::WindowHandle && retries_left > 0 => {
                Timer::single_shot(ATTACH_RETRY_EVERY, move || {
                    if let Some(ui) = current() {
                        ui.skins.try_attach(&ui, key, retries_left - 1, showing);
                    }
                });
            }
            Some(Err(error)) => {
                self.failed(ui, key, error);
                key.with(ui, |_, look, _| look.set_appear(true));
            }
        }
    }

    fn put_frame(
        &self,
        key: WindowKey,
        window: &slint::Window,
        look: &Look<'_>,
        aero: &AeroConst<'_>,
    ) -> Result<(), GlassError> {
        let handle = window.window_handle();
        look.set_maximized(window.is_maximized());
        let measures = geometry(look, aero);
        let held = self.windows.borrow_mut().get_mut(&key).and_then(|window| window.frame.take());
        let frame = match held {
            Some(frame) if frame.same_window(&handle) => {
                frame.set_geometry(measures);
                frame
            }
            stale => {
                if let Some(stale) = stale {
                    detach_loudly(key, stale);
                }
                Frame::attach(&handle, measures, aero.get_sigma(), sink(key))?
            }
        };
        frame.set_chrome(true);
        if let Some(window) = self.windows.borrow_mut().get_mut(&key) {
            window.frame = Some(frame);
        }
        Ok(())
    }

    /// The glass behind a frame just put on, for a window that wants it: one
    /// that has fallen to solid glass stays so until a theme is picked. Only a
    /// glass that stands makes the frame's glass area clear.
    fn put_glass(&self, ui: &Ui, key: WindowKey, showing: Showing) {
        let fade = key.with(ui, |_, _, aero| match showing {
            Showing::Opening => Some(fade_in(&aero)),
            Showing::AtOnce => None,
        });
        let Some(fade) = fade else {
            return;
        };
        let shown = {
            let windows = self.windows.borrow();
            let frame = windows
                .get(&key)
                .filter(|window| window.level == Level::AeroGlass)
                .and_then(|window| window.frame.as_ref());
            match frame {
                Some(frame) => frame.show_glass(fade),
                None => return,
            }
        };
        match shown {
            Ok(()) => {
                key.with(ui, |_, look, _| look.set_glass(true));
            }
            Err(error) => self.failed(ui, key, error),
        }
    }

    /// The frame's measures changed: the text size, or maximizing.
    pub fn geometry_changed(&self, ui: &Ui, key: WindowKey) {
        let windows = self.windows.borrow();
        let Some(frame) = windows.get(&key).and_then(|window| window.frame.as_ref()) else {
            return;
        };
        if let Some(measures) = key.with(ui, |_, look, aero| geometry(&look, &aero)) {
            frame.set_geometry(measures);
        }
    }

    /// A person asked this window to close. With a frame of its own it begins
    /// to fade out, its glass with it, and stays until the time this answers has
    /// passed; asked again meanwhile, it answers that the close is under way. A
    /// glass that does not fade is in the log, and the window closes all the same.
    pub fn begin_close(&self, ui: &Ui, key: WindowKey) -> Closing {
        {
            let mut windows = self.windows.borrow_mut();
            match windows.get_mut(&key) {
                Some(window) if window.closing => return Closing::Underway,
                Some(window) if window.level.has_own_frame() => window.closing = true,
                _ => return Closing::AtOnce,
            };
        }
        let fading = key.with(ui, |_, look, aero| {
            look.set_appear(false);
            (fade_out(&aero), Duration::from_millis(aero.get_vanish_time().max(0) as u64) + CLOSE_AFTER_FADE)
        });
        let Some((fade, stays_for)) = fading else {
            return Closing::AtOnce;
        };
        let faded = match self.windows.borrow().get(&key).and_then(|window| window.frame.as_ref()) {
            Some(frame) => frame.fade_out(fade),
            None => Ok(()),
        };
        if let Err(error) = faded {
            self.failed(ui, key, error);
        }
        Closing::FadingFor(stays_for)
    }

    /// A close begun that must not end after all: the window stays, in the theme
    /// picked while it was fading if one was, and comes back into sight, its glass
    /// at once and whatever its fade had reached.
    pub fn close_called_off(&self, ui: &Ui, key: WindowKey) {
        if let Some(window) = self.windows.borrow_mut().get_mut(&key) {
            window.closing = false;
        }
        self.switch(ui, key, ui.theme.get());
    }

    /// A window about to close: the native frame comes off while the window is
    /// still all there. Nothing of its look is touched after this.
    pub fn window_closing(&self, key: WindowKey) {
        let window = self.windows.borrow_mut().remove(&key);
        if let Some(frame) = window.and_then(|window| window.frame) {
            detach_loudly(key, frame);
        }
    }

    /// Every window's frame off, before the windows are let go.
    pub fn detach_all(&self) {
        let windows = std::mem::take(&mut *self.windows.borrow_mut());
        for (key, window) in windows {
            if let Some(frame) = window.frame {
                detach_loudly(key, frame);
            }
        }
    }

    /// What the windows say about their looks, in the language shown now.
    pub fn paint_failures(&self, ui: &Ui) {
        let failures: Vec<(WindowKey, Failure)> = self
            .windows
            .borrow()
            .iter()
            .filter_map(|(key, window)| window.failure.map(|failure| (*key, failure)))
            .collect();
        for (key, failure) in failures {
            paint_failure(ui, key, failure);
        }
    }

    /// An event of the native layer, back on the event loop.
    fn frame_event(&self, ui: &Ui, key: WindowKey, event: FrameEvent) {
        match event {
            FrameEvent::Maximized(maximized) => {
                key.with(ui, |_, look, _| look.set_maximized(maximized));
                self.geometry_changed(ui, key);
            }
            FrameEvent::Info(text) => log_info(key, &text),
            FrameEvent::Failed(error) => self.failed(ui, key, error),
        }
    }

    /// A step of the native layer failed: in the log, then on the window with
    /// the level it falls to. Once per step until a theme is picked again.
    fn failed(&self, ui: &Ui, key: WindowKey, error: GlassError) {
        let level = {
            let mut windows = self.windows.borrow_mut();
            match windows.get_mut(&key) {
                Some(window) if window.reported.contains(&error.step) => return,
                Some(window) => {
                    window.reported.push(error.step);
                    Some(window.level).filter(|_| !window.closing)
                }
                None => None,
            }
        };
        report::log(LOG_KIND, &format!("{key:?}: {error}"));
        match (error.step, level) {
            // Nothing left to fall from: a window gone, going, or gone flat meanwhile.
            (_, None | Some(Level::Flat)) => {}
            (Step::Unsupported | Step::WindowHandle | Step::Subclass, Some(_)) => {
                self.fall_to_system_frame(ui, key);
            }
            (Step::Destroy, Some(_)) => {}
            (_, Some(Level::AeroGlass)) => self.fall_to_solid_glass(ui, key),
            (_, Some(_)) => {}
        }
    }

    /// The system's frame after all: the glass goes, and the hit test goes off
    /// before the window asks for that frame back.
    fn fall_to_system_frame(&self, ui: &Ui, key: WindowKey) {
        self.fell(key, Level::AeroSystemFrame, Failure::Chrome);
        key.with(ui, |_, look, _| {
            look.set_glass(false);
            look.set_chrome(false);
        });
        paint_failure(ui, key, Failure::Chrome);
    }

    /// The window's own frame stays, over solid glass; what is left of the glass
    /// that failed goes.
    fn fall_to_solid_glass(&self, ui: &Ui, key: WindowKey) {
        self.fell(key, Level::AeroChromeSolid, Failure::Glass);
        key.with(ui, |_, look, _| look.set_glass(false));
        paint_failure(ui, key, Failure::Glass);
    }

    fn fell(&self, key: WindowKey, level: Level, failure: Failure) {
        if let Some(window) = self.windows.borrow_mut().get_mut(&key) {
            window.level = level;
            window.failure = Some(failure);
            if let Some(frame) = &window.frame {
                if let Err(error) = frame.hide_glass() {
                    report::log(LOG_KIND, &format!("{key:?}: {error}"));
                }
                if level == Level::AeroSystemFrame {
                    frame.set_chrome(false);
                }
            }
        }
    }

    /// Something the window's own frame asked for did not happen.
    fn say_failure(&self, ui: &Ui, key: WindowKey, failure: Failure) {
        if let Some(window) = self.windows.borrow_mut().get_mut(&key) {
            window.failure = Some(failure);
        }
        paint_failure(ui, key, failure);
    }
}

/// A new window's look, before it shows: the theme of this run as it stands,
/// and its caption buttons. The native frame goes on once it shows
/// (`Skins::after_show`). Its own frame is asked for from the start, so the
/// window is never seen with the system's first, and out of sight until then.
pub fn apply_before_show(look: Look<'_>, ui: &Ui, key: WindowKey) {
    look.on_minimize(move || with_open_window(key, |window| window.set_minimized(true)));
    look.on_toggle_maximize(move || {
        with_open_window(key, |window| window.set_maximized(!window.is_maximized()));
    });
    look.on_close(move || request_close(key));
    let theme = ui.theme.get();
    let wanted = ui.skins.frame_wanted(theme);
    look.set_skin(skin_of(theme));
    // No glass stands yet: it is built, and this set, once the frame is on.
    look.set_glass(false);
    look.set_chrome(wanted);
    look.set_appear(!wanted);
    let window = WindowSkin {
        frame: None,
        level: level_of(theme, wanted),
        closing: false,
        failure: None,
        reported: Vec::new(),
    };
    ui.skins.windows.borrow_mut().insert(key, window);
}

fn with_open_window(key: WindowKey, act: impl FnOnce(&slint::Window)) {
    if let Some(ui) = current() {
        key.with(&ui, |window, _, _| act(window));
    }
}

/// The frame's close button: the same request as Alt+F4 or the taskbar's. It
/// comes in the middle of handling a click, which must not be re-entered, and
/// the window may well go, so the request waits for the next turn of the loop.
fn request_close(key: WindowKey) {
    Timer::single_shot(Duration::ZERO, move || {
        let Some(ui) = current() else {
            return;
        };
        let dispatched = key.with(&ui, |window, _, _| window.try_dispatch_event(WindowEvent::CloseRequested));
        if let Some(Err(error)) = dispatched {
            report::log("acui-window", &format!("{key:?}: close: {error}"));
            ui.skins.say_failure(&ui, key, Failure::Close);
        }
    });
}

fn log_info(key: WindowKey, text: &str) {
    report::log_quiet(LOG_KIND, &format!("{key:?}: {text}"));
}

/// Where the native layer's reports go: back to the event loop, with nothing of
/// the window touched from inside its window procedure. A line for the log only
/// is written as it comes, touching no window: the glass's last lines come as
/// the loop ends, when a post would be lost and the count of them wrong.
fn sink(key: WindowKey) -> Rc<dyn Fn(FrameEvent)> {
    Rc::new(move |event: FrameEvent| {
        if let FrameEvent::Info(text) = &event {
            log_info(key, text);
            return;
        }
        let posted = slint::invoke_from_event_loop(move || {
            if let Some(ui) = current() {
                ui.skins.frame_event(&ui, key, event);
            }
        });
        if let Err(error) = posted {
            report::log(LOG_KIND, &format!("{key:?}: a report of the frame was lost: {error}"));
        }
    })
}

fn detach_loudly(key: WindowKey, frame: Frame) {
    if let Err(error) = frame.detach() {
        report::log(LOG_KIND, &format!("{key:?}: {error}"));
    }
}

fn paint_failure(ui: &Ui, key: WindowKey, failure: Failure) {
    let text = failure.text(ui.labels());
    key.with(ui, |_, look, _| look.set_failure(text.into()));
}

/// The glass's part of a window opening.
fn fade_in(aero: &AeroConst<'_>) -> Fade {
    Fade {
        millis: aero.get_appear_time().max(0) as u32,
        from_scale: aero.get_zoom_from(),
        easing: Easing::EaseOut,
    }
}

/// The glass's part of a window closing.
fn fade_out(aero: &AeroConst<'_>) -> Fade {
    Fade {
        millis: aero.get_vanish_time().max(0) as u32,
        from_scale: aero.get_zoom_from(),
        easing: Easing::EaseIn,
    }
}

/// The frame's measures as this window's look has them now.
fn geometry(look: &Look<'_>, aero: &AeroConst<'_>) -> Geometry {
    Geometry {
        caption: look.get_caption_height(),
        frame: look.get_frame(),
        buttons_width: aero.get_caption_buttons_width(),
        buttons_height: aero.get_caption_buttons_height(),
        buttons_inset: aero.get_caption_buttons_inset(),
        corner: look.get_corner(),
    }
}

/// The skin a theme is drawn in.
fn skin_of(theme: Theme) -> Skin {
    match theme {
        Theme::Flat => Skin::Flat,
        Theme::Aero => Skin::Aero,
    }
}

/// A window's level when nothing has failed.
fn level_of(theme: Theme, frame_wanted: bool) -> Level {
    match (theme, frame_wanted) {
        (Theme::Flat, _) => Level::Flat,
        (Theme::Aero, true) => Level::AeroGlass,
        (Theme::Aero, false) => Level::AeroSystemFrame,
    }
}
