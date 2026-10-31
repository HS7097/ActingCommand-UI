// SPDX-License-Identifier: GPL-3.0-only
//! A ledger window's own side, apart from the read face: what it shows
//! (`state`), how that is painted, with every word of it (`paint`), its frame
//! reads (`frame`), a person's conditional resume of a scheduling pause
//! (`resume`), and its reader (`worker`), the thread that opens the read face
//! after the window is up and does every read of it.
//!
//! On the event loop a window only changes its state and hands its reader a
//! request (`Command`); a change that supersedes the reads in flight takes a
//! new generation under the same lock. The reader leaves what it read in the
//! state and asks for a repaint by the window's id, the one thing its job
//! carries; the window is found again in the shared state's registry (`find`).
//! Its three timers run on the event loop: the follow ticks, the time cursor's
//! rest, and while a read is in flight the busy line's clock.
//!
//! The registry's entry holds the window side, and the window side holds the
//! window; the window's callbacks and timers hold neither, so letting the
//! entry go lets the window go, and with it the reader's request channel.

pub mod frame;
pub mod paint;
pub mod resume;
pub mod state;
pub mod worker;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use acui_model::{DisplayRow, Filters, ViewModel, ALL_TABS};
use acui_rows::{code, EventQuery, EventSeverity, LedgerView, OriginModule};
use acui_source::{PauseTarget, SourceMode};
use slint::{SharedString, Timer, TimerMode};

use crate::settings::{TextSize, Theme};
use crate::ui::{self, language_from_index, Ui};
use crate::{launcher, report, LedgerWindow};
use frame::FrameSlot;
use paint::port_options;
use resume::Resume;
use state::{
    Effects, FrameEventsSlot, LedgerFailure, LedgerShared, LedgerState, Phase, PortMap, RequestTag,
};
use worker::{Command, Opening};

/// How often "follow latest" asks where the ledger is: one fact snapshot,
/// which reads no ledger and writes nothing. A page query, served on the
/// Runtime's ledger writer, follows only when the ledger moved.
const FOLLOW_INTERVAL: Duration = Duration::from_secs(5);
/// How long the time cursor's handle must rest before the view reloads.
const CURSOR_REST: Duration = Duration::from_millis(400);
/// How often the busy line's clock moves while a read is in flight.
const BUSY_TICK: Duration = Duration::from_secs(1);

/// What a ledger window opens: the state root, the face it was opened for,
/// and its first tab.
#[derive(Clone)]
pub struct OpenArgs {
    pub state_root: PathBuf,
    pub mode: SourceMode,
    pub tab: Option<LedgerView>,
}

/// One ledger window: the window itself; the state it is painted from, shared
/// with its reader; where its requests go; its frame reads; the module option
/// list as the box currently shows it, with what each entry after the first
/// one means, rebuilt from every page; and its three timers.
pub struct LedgerHandle {
    pub id: u64,
    /// Kept here, and so by the registry's entry: hidden when closed, then let
    /// go with the entry.
    pub window: LedgerWindow,
    open: OpenArgs,
    pub shared: Arc<LedgerShared>,
    /// The reader of the current session; `None` when the system refused it.
    commands: RefCell<Option<Sender<Command>>>,
    pub frame: FrameSlot,
    pub module_options: RefCell<Vec<SharedString>>,
    pub module_choices: RefCell<Vec<OriginModule>>,
    /// The scheduling pause each pause line's resume entry names, in the order
    /// painted; `None` for a line that offers none (`resume::paint`).
    pauses: RefCell<Vec<Option<PauseTarget>>>,
    /// A person's conditional resume in this window (`resume`).
    resume: RefCell<Resume>,
    /// Runs the follow ticks while "follow latest" is on.
    follow: Timer,
    /// Reloads once the time cursor's handle rests.
    cursor_rest: Timer,
    /// Moves the busy line's clock while a read is in flight.
    busy_tick: Timer,
    /// Closed by a person: nothing is painted or sent any more while the
    /// window waits to leave the registry.
    closed: Cell<bool>,
}

/// The ledger window `id`, while it is open.
pub fn find(id: u64) -> Option<Rc<LedgerHandle>> {
    ui::current()?.ledger(id)
}

/// Hands a repaint of window `id` to the event loop, unless one is handed and
/// has not run yet: that one paints what is there by then.
pub fn post_repaint(id: u64, shared: &LedgerShared) {
    if !shared.repaint_pending.swap(true, Ordering::SeqCst) {
        report::post("acui-ledger", move || repaint(id));
    }
}

/// A repaint a reader asked for, on the event loop.
pub fn repaint(id: u64) {
    let Some(handle) = find(id) else {
        return;
    };
    // Cleared first: what changes while this paint runs asks for another.
    handle.shared.repaint_pending.store(false, Ordering::SeqCst);
    let Some(ui) = ui::current() else {
        return;
    };
    handle.refresh(&ui);
}

/// The busy line of window `id` painted again: by its clock, or because the
/// reader started or ended a read.
pub fn busy_changed(id: u64) {
    let (Some(handle), Some(ui)) = (find(id), ui::current()) else {
        return;
    };
    handle.paint_busy(&ui);
}

/// A frame read of window `id`'s `session` found the material reader latched,
/// or its rebuild failed: the reader makes a new one, while that session is the
/// window's and its face is open.
pub fn refresh_reader(id: u64, session: u64) {
    let Some(handle) = find(id) else {
        return;
    };
    let current = {
        let state = handle.lock();
        state.tag.session == session && matches!(state.phase, Phase::Open(_))
    };
    if current {
        handle.send(Command::RefreshReader);
    }
}

impl LedgerHandle {
    /// Window `id`'s ledger side, before its reader starts: the state says the
    /// read face is opening.
    pub fn new(id: u64, window: LedgerWindow, open: OpenArgs) -> Rc<Self> {
        let tag = RequestTag { window: id, session: 1, generation: 1 };
        let state = LedgerState::opening(open.state_root.clone(), tag);
        Rc::new(Self {
            id,
            window,
            open,
            shared: Arc::new(LedgerShared {
                state: Mutex::new(state),
                tick_in_flight: AtomicBool::new(false),
                repaint_pending: AtomicBool::new(false),
            }),
            commands: RefCell::new(None),
            frame: FrameSlot::default(),
            module_options: RefCell::new(Vec::new()),
            module_choices: RefCell::new(Vec::new()),
            pauses: RefCell::new(Vec::new()),
            resume: RefCell::new(Resume::default()),
            follow: Timer::default(),
            cursor_rest: Timer::default(),
            busy_tick: Timer::default(),
            closed: Cell::new(false),
        })
    }

    /// The source this window was opened for: at most one window per source.
    pub fn mode(&self) -> SourceMode {
        self.open.mode
    }

    /// Closed by a person, and waiting to leave the registry.
    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// The window's state. A reader that panicked while it held the lock left
    /// it poisoned: the state is taken as it is and shown as failed, whatever
    /// it says, until the reader's own handler states the panic or a reopen
    /// makes it anew.
    pub fn lock(&self) -> MutexGuard<'_, LedgerState> {
        match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                if !matches!(state.phase, Phase::Failed { .. }) {
                    state.fail(LedgerFailure::Panicked(None));
                }
                state
            }
        }
    }

    /// Starts the reader of the session the state names now, with the view to
    /// start from: a new thread, which opens the read face first. A thread the
    /// system refuses fails the session, in the log and in the window, which is
    /// painted again for it whoever called: at start nothing else would.
    pub fn start(&self, carry: Option<(LedgerView, Filters)>) {
        let session = self.lock().tag.session;
        let opening = Opening {
            state_root: self.open.state_root.clone(),
            mode: self.open.mode,
            tab: self.open.tab,
            carry,
        };
        match worker::spawn(self.id, session, Arc::clone(&self.shared), opening) {
            Ok(sender) => *self.commands.borrow_mut() = Some(sender),
            Err(error) => {
                *self.commands.borrow_mut() = None;
                report::log("acui-ledger", &format!("acui-ledger-{}: {error}", self.id));
                self.lock().fail(LedgerFailure::NoThread(error.to_string()));
                post_repaint(self.id, &self.shared);
            }
        }
    }

    /// A new session on the same state root and face: the reader of the old
    /// one stops at its next page, and the new one opens the read face again,
    /// from the tab and filters the boxes show, without a time bound. The
    /// last resume's line goes with the old session (`Resume::reopened`).
    pub fn reopen(&self, ui: &Ui) {
        self.follow.stop();
        self.cursor_rest.stop();
        self.window.set_following(false);
        self.window.set_cursor_value(1000.0);
        self.frame.clear();
        self.resume.borrow_mut().reopened();
        let carry = {
            let mut state = self.lock();
            let carry = state.model.as_ref().map(|model| (model.tab, model.filters.clone()));
            let tag = RequestTag {
                window: self.id,
                session: state.tag.session + 1,
                generation: state.tag.generation + 1,
            };
            *state = LedgerState::opening(self.open.state_root.clone(), tag);
            carry
        };
        // The state was just made anew; a panic of the old reader poisoned
        // only the one it replaced.
        self.shared.state.clear_poison();
        self.shared.tick_in_flight.store(false, Ordering::SeqCst);
        self.start(carry);
        self.refresh(ui);
    }

    /// Closed by a person: the timers stop, the frame read in flight is no
    /// longer wanted, and the session is let go at once, so its reader leaves
    /// nothing more and stops at its next check — after the page in hand, or
    /// once an opening in flight returns. Nothing is painted or sent any more;
    /// the registry lets the window go after the close has been handled.
    pub fn close(&self) {
        self.closed.set(true);
        self.follow.stop();
        self.cursor_rest.stop();
        self.busy_tick.stop();
        self.frame.clear();
        let mut state = self.lock();
        state.tag.session += 1;
        state.tag.generation += 1;
    }

    /// Hands the reader a request. One that cannot be handed — the reader is
    /// gone — fails the session, loudly. A closed window sends nothing.
    fn send(&self, command: Command) {
        if self.closed.get() {
            return;
        }
        let refused = match self.commands.borrow().as_ref() {
            Some(sender) => sender.send(command).is_err(),
            None => true,
        };
        if !refused {
            return;
        }
        report::log(
            "acui-ledger",
            &format!("acui-ledger-{}: no reader thread took the window's request", self.id),
        );
        {
            let mut state = self.lock();
            if matches!(state.phase, Phase::Opening { .. } | Phase::Open(_)) {
                state.fail(LedgerFailure::Gone);
            }
        }
        post_repaint(self.id, &self.shared);
    }

    /// Changes the live view model, if there is one; no read is superseded.
    pub fn edit(&self, change: impl FnOnce(&mut ViewModel)) {
        if let Some(model) = self.lock().live_mut() {
            change(model);
        }
    }

    /// Changes the live view model and takes a new generation under one lock,
    /// so no read under the old view lands after the change. `None` when there
    /// is no live view model.
    fn supersede(&self, change: impl FnOnce(&mut ViewModel, &PortMap)) -> Option<RequestTag> {
        let mut guard = self.lock();
        let state = &mut *guard;
        if !matches!(state.phase, Phase::Open(_)) {
            return None;
        }
        let model = state.model.as_mut()?;
        change(model, &state.port_map);
        state.tag.generation += 1;
        Some(state.tag)
    }

    /// Changes the live view model and starts the view over from the pin, or
    /// from where its time bound is estimated to fall. That applies the bound
    /// the time cursor holds now.
    pub fn restart(&self, change: impl FnOnce(&mut ViewModel, &PortMap)) {
        let Some(tag) = self.supersede(change) else {
            return;
        };
        self.cursor_rest.stop();
        self.send(Command::Reload { tag, earlier: false });
    }

    /// "Read earlier". A bound the time cursor set but has not applied yet goes
    /// first: no window of it is appended below rows read under the old one.
    pub fn load_more(&self) {
        if self.cursor_rest.running() {
            self.restart(|_, _| {});
            return;
        }
        let tag = {
            let state = self.lock();
            if state.live().is_none() {
                return;
            }
            state.tag
        };
        self.send(Command::Reload { tag, earlier: true });
    }

    /// The bound and its label follow the handle at once; the reload waits
    /// until the handle rests, so a drag is one reload, not one per step. A
    /// time bound reads down from the past: following stops at once.
    pub fn cursor_moved(&self, ui: &Ui, value: f32) {
        let bound = {
            let mut state = self.lock();
            let Some(model) = state.live_mut() else {
                return;
            };
            let bound = model.span().and_then(|(from, to)| {
                let ratio = (value as f64 / 1000.0).clamp(0.0, 1.0);
                (ratio < 1.0).then(|| from + ((to - from) as f64 * ratio) as u64)
            });
            model.filters.to_timestamp_unix_ms = bound;
            bound
        };
        if bound.is_some() {
            self.stop_following();
        }
        let id = self.id;
        self.cursor_rest.start(TimerMode::SingleShot, CURSOR_REST, move || {
            if let Some(handle) = find(id) {
                handle.cursor_rested();
            }
        });
        self.refresh(ui);
    }

    fn cursor_rested(&self) {
        let Some(ui) = ui::current() else {
            return;
        };
        self.restart(|_, _| {});
        self.refresh(&ui);
    }

    /// A person's jump to the latest, on the reader.
    pub fn jump_latest(&self, ui: &Ui) {
        let Some(tag) = self.supersede(|_, _| {}) else {
            return;
        };
        self.cursor_rest.stop();
        self.send(Command::JumpLatest { tag });
        self.refresh(ui);
    }

    /// Turning following on catches up on the reader and starts the ticks, one
    /// every `FOLLOW_INTERVAL`; what an earlier following failed at goes.
    /// Turning it off stops the ticks and leaves the view as it is.
    pub fn set_following(&self, ui: &Ui, on: bool) {
        if !on {
            self.stop_following();
            self.refresh(ui);
            return;
        }
        let Some(tag) = self.supersede(|model, _| model.follow_error = None) else {
            return;
        };
        self.cursor_rest.stop();
        let id = self.id;
        self.follow.start(TimerMode::Repeated, FOLLOW_INTERVAL, move || {
            if let Some(handle) = find(id) {
                handle.follow_tick();
            }
        });
        self.send(Command::FollowOn { tag });
        self.refresh(ui);
    }

    /// Stops following: no more ticks, the box unticked, a follow read in
    /// flight superseded, and what a tick failed at goes with it; the view
    /// keeps what it read.
    pub fn stop_following(&self) {
        self.follow.stop();
        self.window.set_following(false);
        let mut state = self.lock();
        let live = match state.live_mut() {
            Some(model) => {
                model.follow_error = None;
                true
            }
            None => false,
        };
        if live {
            state.tag.generation += 1;
        }
    }

    /// One tick of the follow timer: a time bound set meanwhile stops the
    /// following; otherwise one follow tick is handed to the reader, unless
    /// the last one has not finished.
    fn follow_tick(&self) {
        let Some(ui) = ui::current() else {
            return;
        };
        let tag = {
            let state = self.lock();
            let bounded = match state.live() {
                Some(model) => model.filters.to_timestamp_unix_ms.is_some(),
                None => return,
            };
            let tag = state.tag;
            (!bounded).then_some(tag)
        };
        let Some(tag) = tag else {
            self.stop_following();
            self.refresh(&ui);
            return;
        };
        if self.shared.tick_in_flight.swap(true, Ordering::SeqCst) {
            return;
        }
        self.send(Command::FollowTick { tag });
    }

    /// Paints the window from its state alone (`paint`), after doing what the
    /// reader left for the event loop. The selected event's frame, when the
    /// loaded rows hold no capture of it, is asked of the reader here; the
    /// pane says so until the answer comes. A closed window is not painted.
    pub fn refresh(&self, ui: &Ui) {
        if self.closed.get() {
            return;
        }
        let labels = ui.labels();
        let runtime_ready = ui.launcher_view.borrow().runtime_ready();
        let (lookup, busy) = {
            let mut guard = self.lock();
            let state = &mut *guard;
            let effects = std::mem::take(&mut state.effects);
            self.apply(effects);
            let lookup = wanted_frame_events(state);
            paint::refresh(&self.window, self, state, labels, runtime_ready);
            (lookup, state.busy.is_some())
        };
        // A failure the window shows may have made the technical log; the top
        // of every window says where it is.
        ui.paint_log_notes();
        self.tick_busy(busy);
        if let Some((frame, query)) = lookup {
            self.send(Command::FrameEvents { frame, query });
        }
    }

    /// Only the reopen offer, with the launcher's lines: a Runtime the
    /// launcher finds up now is offered to a window that reads offline for
    /// want of one.
    pub fn paint_reopen(&self, ui: &Ui) {
        if self.closed.get() {
            return;
        }
        let runtime_ready = ui.launcher_view.borrow().runtime_ready();
        let state = self.lock();
        paint::paint_reopen(&self.window, &state, ui.labels(), runtime_ready);
    }

    /// What the reader left for the event loop.
    fn apply(&self, effects: Effects) {
        if effects.cursor_top {
            self.window.set_cursor_value(1000.0);
        }
        if effects.forget_frame {
            self.frame.forget();
        }
        if effects.follow_halted {
            self.follow.stop();
            self.window.set_following(false);
        }
        if let Some(found) = effects.status_reread {
            self.resume.borrow_mut().reread(found);
        }
    }

    /// Only the busy line: by its clock, or as a read starts or ends.
    fn paint_busy(&self, ui: &Ui) {
        if self.closed.get() {
            return;
        }
        let busy = {
            let state = self.lock();
            self.window.set_busy_text(paint::busy_text(ui.labels(), &state).into());
            state.busy.is_some()
        };
        self.tick_busy(busy);
    }

    /// The busy line's clock runs only while a read is in flight.
    fn tick_busy(&self, busy: bool) {
        if busy == self.busy_tick.running() {
            return;
        }
        if !busy {
            self.busy_tick.stop();
            return;
        }
        let id = self.id;
        self.busy_tick.start(TimerMode::Repeated, BUSY_TICK, move || busy_changed(id));
    }
}

/// When the selected event's frame has no capture among the loaded rows and
/// the slot holds nothing for it: the slot is marked pending, and the frame's
/// id and the query by it go to the reader.
fn wanted_frame_events(state: &mut LedgerState) -> Option<(String, EventQuery)> {
    if !matches!(state.phase, Phase::Open(_)) {
        return None;
    }
    let model = state.model.as_ref()?;
    let (basis, _) = model.selected_basis()?;
    let frame_id = basis.links.frame_id().copied()?;
    let frame = code(&frame_id);
    if state.frame_events.holds(&frame) || model.frame_group(&frame, &[]).target.is_some() {
        return None;
    }
    let query = EventQuery {
        view: Some(LedgerView::Events),
        frame_id: Some(frame_id),
        ..EventQuery::default()
    };
    state.frame_events = FrameEventsSlot::Pending { frame: frame.clone() };
    Some((frame, query))
}

/// The ledger window's controls. Each only changes the window's state and
/// hands its reader a request, then paints the window again at once; the
/// reader paints it again as its reads land. While the read face is not open
/// every control that would query it is off, and a change finds no view model
/// to change. A callback holds the window's side weakly and finds the shared
/// state through its slot: the registry's entry is what keeps either.
pub fn install_callbacks(handle: &Rc<LedgerHandle>) {
    macro_rules! on {
        ($setter:ident, |$ledger:ident, $ui:ident, $value:ident| $body:expr) => {{
            let weak = Rc::downgrade(handle);
            handle.window.$setter(move |$value| {
                let (Some($ledger), Some($ui)) = (weak.upgrade(), ui::current()) else {
                    return;
                };
                $body;
                $ledger.refresh(&$ui);
            });
        }};
    }

    on!(on_tab_changed, |ledger, ui, index| {
        ledger.restart(|model, _| {
            model.tab = ALL_TABS[index.clamp(0, ALL_TABS.len() as i32 - 1) as usize]
        })
    });
    on!(on_minimum_severity_changed, |ledger, ui, index| {
        ledger.restart(|model, _| {
            model.filters.minimum_severity = match index {
                1 => Some(EventSeverity::Warning),
                2 => Some(EventSeverity::Error),
                3 => Some(EventSeverity::Fatal),
                _ => None,
            }
        })
    });
    on!(on_maximum_severity_changed, |ledger, ui, index| {
        ledger.restart(|model, _| {
            model.filters.maximum_severity = match index {
                1 => Some(EventSeverity::Info),
                2 => Some(EventSeverity::Warning),
                3 => Some(EventSeverity::Error),
                _ => None,
            }
        })
    });
    // By name against the list as it stands now, never by a position taken from
    // a list that the next page may have rebuilt.
    on!(on_module_changed, |ledger, ui, name| {
        let module = ledger
            .module_options
            .borrow()
            .iter()
            .position(|option| *option == name)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| ledger.module_choices.borrow().get(index).copied());
        ledger.restart(|model, _| model.filters.origin_module = module)
    });
    // A port is one instance: its whole set of ids goes into the query, or
    // nothing does. The first item, and any name not on the list, clears both.
    on!(on_port_changed, |ledger, ui, name| {
        let labels = ui.labels();
        ledger.restart(|model, port_map| {
            // The list the box shows: made in the language it was painted in,
            // which is the one shown now.
            let (options, choices) = port_options(labels, port_map);
            let entry = options
                .iter()
                .position(|option| *option == name)
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| choices.get(index))
                .and_then(|port| port_map.entry(*port));
            model.filters.port = entry.map(|entry| entry.port);
            model.filters.instance_ids =
                entry.map(|entry| entry.members.clone()).unwrap_or_default();
        })
    });
    // Shown or hidden again from the pin: the performance monitor's routine
    // events are left out by the query or dropped as each window is read, never
    // from rows already held.
    on!(on_show_performance_changed, |ledger, ui, shown| {
        ledger.restart(|model, _| model.filters.show_performance = shown)
    });
    on!(on_id_changed, |ledger, ui, text| {
        ledger.restart(|model, _| model.filters.id_text = text.to_string())
    });
    on!(on_row_clicked, |ledger, ui, index| {
        ledger.edit(|model| {
            let selected = match model.display_rows().get(index.max(0) as usize) {
                Some(DisplayRow::Event { event, .. }) => Some(event.sequence),
                _ => None,
            };
            if selected.is_some() {
                model.selected_sequence = selected;
            }
        })
    });
    {
        let weak = Rc::downgrade(handle);
        handle.window.on_cursor_changed(move |value| {
            let (Some(ledger), Some(ui)) = (weak.upgrade(), ui::current()) else {
                return;
            };
            ledger.cursor_moved(&ui, value);
        });
    }
    {
        let weak = Rc::downgrade(handle);
        handle.window.on_jump_latest(move || {
            let (Some(ledger), Some(ui)) = (weak.upgrade(), ui::current()) else {
                return;
            };
            ledger.jump_latest(&ui);
        });
    }
    {
        let weak = Rc::downgrade(handle);
        handle.window.on_follow_changed(move |on| {
            let (Some(ledger), Some(ui)) = (weak.upgrade(), ui::current()) else {
                return;
            };
            ledger.set_following(&ui, on);
        });
    }
    {
        let weak = Rc::downgrade(handle);
        handle.window.on_load_more(move || {
            let (Some(ledger), Some(ui)) = (weak.upgrade(), ui::current()) else {
                return;
            };
            ledger.load_more();
            ledger.refresh(&ui);
        });
    }
    // A new session of the window's read face: offered when it did not open,
    // when its reader stopped, when a read reached another Runtime, or when a
    // Runtime is up while the window reads offline for want of one.
    {
        let weak = Rc::downgrade(handle);
        handle.window.on_reopen_ledger(move || {
            let (Some(ledger), Some(ui)) = (weak.upgrade(), ui::current()) else {
                return;
            };
            ledger.reopen(&ui);
        });
    }
    // A pause line's resume entry, by the line's place as painted: the first
    // press only asks for the confirmation; the confirming press resumes.
    on!(on_resume_asked, |ledger, ui, index| resume::asked(&ledger, index));
    on!(on_resume_confirmed, |ledger, ui, index| resume::confirmed(&ledger, index));
    // The launcher's unlock entry: its confirming press.
    handle.window.on_unlock_owner(|| {
        if let Some(ui) = ui::current() {
            launcher::unlock(&ui);
        }
    });
    // The three display boxes are the shared state's: what is picked here is
    // applied to every window, and saved.
    handle.window.on_text_size_changed(|index| {
        if let Some(ui) = ui::current() {
            ui.set_text_size(TextSize::from_index(index));
        }
    });
    handle.window.on_language_changed(|index| {
        if let Some(ui) = ui::current() {
            ui.set_language(language_from_index(index));
        }
    });
    handle.window.on_theme_changed(|index| {
        if let Some(ui) = ui::current() {
            ui.set_theme(Theme::from_index(index));
        }
    });
}
