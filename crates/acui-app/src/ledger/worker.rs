// SPDX-License-Identifier: GPL-3.0-only
//! The ledger window's reader: one thread per session of the window,
//! `acui-ledger-<id>`, which opens the read face itself, keeps it for the whole
//! session, and runs the window's requests one after another. The read face
//! never leaves this thread.
//!
//! The lock on the window's state is held only to check a request's tag, to
//! take what a read needs and to leave what it found; never while reading. A
//! request whose tag is no longer the state's is superseded: it leaves nothing,
//! and a read that follows a cursor stops at its next page. A read whose page
//! is slow therefore holds up only this window's own later requests, never the
//! event loop. Each request runs inside `catch_unwind`: a panic fails the
//! session loudly, in the window and in the technical log, and ends the thread.
//!
//! Nothing here holds a language table: what the window says is put into words
//! when it paints (`paint`).

use std::any::Any;
use std::cell::Cell;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use acui_model::{Filters, QueryError, ViewModel, FILL_ROWS, FILL_WINDOWS, WINDOW};
use acui_rows::outcome::CONSOLE_TIME_SPAN_PAGE_LIMIT;
use acui_rows::{
    page_source_complete, EventQuery, LedgerView, ProjectedEvent, RuntimeEventQueryPage,
    MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use acui_source::{console_code, ReadSource, Session, SourceMode};

use crate::ledger::state::{
    Busy, BusyKind, FrameEventsSlot, FrameLookupError, InstancesSnapshot, LedgerFailure,
    LedgerShared, LedgerState, Phase, PortMap, RequestTag, SourceFacts, StatusReread,
};
use crate::{launcher, report};

/// How long one fill may keep reading windows after the first.
const FILL_BUDGET: Duration = Duration::from_secs(1);
/// Windows of slack above a time bound's estimated position, and the most
/// probes that check it.
const TIME_SLACK_WINDOWS: u64 = 2;
const TIME_PROBES: usize = 3;
/// The most pages a span read follows the cursor for without finding an
/// event. A page the read budget stopped may hold no row and still go on
/// (#411 A51); a run of them that never ends would leave the window reading
/// for ever, so past this the span read fails, loudly
/// (`CONSOLE_TIME_SPAN_PAGE_LIMIT`).
const MAX_SPAN_PAGES: usize = 64;

/// About how long a read of each kind may take at most, in seconds, as the
/// busy line states it; from acui-source's own bounds. One online page: the
/// page connection's 20 s io timeout, eight times over, plus the 1 s backoff
/// before its one retry. One offline page: the 30 s each later read of the
/// snapshot sets itself.
const ONLINE_PAGE_SECONDS: u64 = 161;
const OFFLINE_PAGE_SECONDS: u64 = 30;
/// Opening offline: the ledger's own 60 s opening, then the 30 s fact replay.
/// Opening online (and `auto`, which tries online first): the three
/// connections, each its connect, `Health` and `BindChannel` (5 s, 20 s and
/// 5 s each), the first page on a fresh page connection (101 s at worst) and
/// the status and fact reads (5 s each).
const OFFLINE_OPEN_SECONDS: u64 = 90;
const ONLINE_OPEN_SECONDS: u64 = 200;
/// The offline port map's binding read.
const BINDINGS_SECONDS: u64 = 30;
/// One exchange on the main connection (5 s), after its rebuild at worst
/// (three times 5 s).
const POLL_SECONDS: u64 = 20;
/// The status and the fact read on the main connection, after its rebuild at
/// worst.
const INSTANCES_SECONDS: u64 = 25;
/// The material connection's rebuild: its connect, `Health` and `BindChannel`.
const REBUILD_SECONDS: u64 = 15;

/// What a window asks of its reader.
pub enum Command {
    /// The view starts over from the pin, or from where a time bound is
    /// estimated to fall; with `earlier`, it reads on below what is loaded.
    Reload { tag: RequestTag, earlier: bool },
    /// A person's jump to the latest: repin, status and facts read again, and
    /// the view starts over from the new pin.
    JumpLatest { tag: RequestTag },
    /// Following turned on: repin, and the view starts over from the new pin.
    FollowOn { tag: RequestTag },
    /// One follow tick.
    FollowTick { tag: RequestTag },
    /// The selected frame's own events, by frame id: one query, read once
    /// while the slot is still pending for `frame`. Not superseded by a new
    /// generation: the answer depends on the frame alone.
    FrameEvents { frame: String, query: EventQuery },
    /// A frame read found the material reader latched, or its rebuild failed:
    /// a new one is made and published.
    RefreshReader,
    /// A person's resume came back (`resume`): status and facts are read again
    /// on a connection of their own. Not superseded by a new generation: the
    /// status belongs to the session, not to the view.
    RereadInstances,
}

/// What a session opens: the state root and the face `--source` asks for, and
/// the view to start with — the window's first one (`tab`), or the one the
/// last session left (`carry`), its time bound dropped.
pub struct Opening {
    pub state_root: PathBuf,
    pub mode: SourceMode,
    pub tab: Option<LedgerView>,
    pub carry: Option<(LedgerView, Filters)>,
}

/// A newer request took over, or another session took the window: the read
/// leaves nothing.
struct Superseded;

/// A read's outcome: `Err(Superseded)` once a newer request took over, else
/// the read's own result.
type Read<T, E> = Result<Result<T, E>, Superseded>;

/// Starts the reader of `session` of window `id` on `acui-ledger-<id>`, and
/// gives back where its requests go. The thread opens the read face first.
pub fn spawn(
    id: u64,
    session: u64,
    shared: Arc<LedgerShared>,
    opening: Opening,
) -> io::Result<Sender<Command>> {
    let (sender, commands) = mpsc::channel();
    let worker = Worker { id, session, shared, halted: Cell::new(None) };
    launcher::spawn_worker(&format!("acui-ledger-{id}"), move || worker.run(opening, commands))?;
    Ok(sender)
}

struct Worker {
    id: u64,
    session: u64,
    shared: Arc<LedgerShared>,
    /// The request whose follow read failed and stopped the following: a tick
    /// of it still queued behind is not run.
    halted: Cell<Option<RequestTag>>,
}

impl Worker {
    fn run(self, opening: Opening, commands: Receiver<Command>) {
        let source = match catch_unwind(AssertUnwindSafe(|| self.start(&opening))) {
            Ok(Some(source)) => source,
            // Unopened, failed, or another session took the window: there is
            // nothing to read through, and no request is sent to it.
            Ok(None) => return,
            Err(panic) => return self.panicked(panic),
        };
        // Ends once the window lets the session go: its sender is dropped.
        while let Ok(command) = commands.recv() {
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| self.execute(&source, command))) {
                return self.panicked(panic);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, LedgerState> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `change` under the lock, while this session is still the window's.
    fn in_session<R>(
        &self,
        change: impl FnOnce(&mut LedgerState) -> R,
    ) -> Result<R, Superseded> {
        let mut state = self.lock();
        if state.tag.window != self.id || state.tag.session != self.session {
            return Err(Superseded);
        }
        Ok(change(&mut *state))
    }

    /// `change` under the lock, while `tag` is still the window's request.
    fn current<R>(
        &self,
        tag: RequestTag,
        change: impl FnOnce(&mut LedgerState) -> R,
    ) -> Result<R, Superseded> {
        let mut state = self.lock();
        if state.tag != tag {
            return Err(Superseded);
        }
        Ok(change(&mut *state))
    }

    /// Before each page of a read for `tag`: whether it is still wanted.
    fn page(&self, tag: RequestTag) -> bool {
        self.current(tag, |_| ()).is_ok()
    }

    /// The busy line, for this session.
    fn busy(&self, busy: Busy) -> Result<(), Superseded> {
        self.in_session(|state| state.set_busy(busy))?;
        self.busy_changed();
        Ok(())
    }

    /// The busy line, for `tag`'s request: the check before it starts.
    fn busy_for(&self, tag: RequestTag, busy: Busy) -> Result<(), Superseded> {
        self.current(tag, |state| state.set_busy(busy))?;
        self.busy_changed();
        Ok(())
    }

    /// The request is over: no busy line.
    fn idle(&self) {
        if self.in_session(|state| state.busy = None).is_ok() {
            self.busy_changed();
        }
    }

    /// The window is painted again from its state, once the event loop gets to
    /// it; a repaint already handed over and not run yet covers this one too.
    fn repaint(&self) {
        super::post_repaint(self.id, &self.shared);
    }

    /// Only the busy line is painted again.
    fn busy_changed(&self) {
        let id = self.id;
        report::post("acui-ledger", move || super::busy_changed(id));
    }

    /// The session fails for the panic: the window says so, the log has its
    /// message, and the thread ends, since the state it left may not hold
    /// together.
    fn panicked(&self, panic: Box<dyn Any + Send>) {
        let message = panic
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned());
        let shown = message.as_deref().unwrap_or("-");
        report::log("acui-ledger", &format!("acui-ledger-{} panicked: {shown}", self.id));
        if self.in_session(|state| state.fail(LedgerFailure::Panicked(message))).is_ok() {
            self.repaint();
        }
    }

    /// Opens the session, then reads its first view; the source to read
    /// through, when it opened.
    fn start(&self, opening: &Opening) -> Option<ReadSource> {
        let opened = self.open(opening);
        let source = match opened {
            Ok(Some((source, tag))) => {
                let first = self.reload(&source, tag, false);
                self.settle(first);
                Some(source)
            }
            Ok(None) => None,
            Err(Superseded) => None,
        };
        self.idle();
        source
    }

    /// The read face, opened here; then, offline, its port map, and on either
    /// face the committed span, all read before anything is left in the state.
    /// `None` when it did not open: the state says why.
    fn open(&self, opening: &Opening) -> Result<Option<(ReadSource, RequestTag)>, Superseded> {
        let bound = match opening.mode {
            SourceMode::Offline => OFFLINE_OPEN_SECONDS,
            SourceMode::Auto | SourceMode::Online => ONLINE_OPEN_SECONDS,
        };
        self.busy(Busy::new(BusyKind::Opening, bound, false))?;
        let session = match Session::open(&opening.state_root, opening.mode) {
            Ok(session) => session,
            Err(error) => {
                let failure =
                    LedgerFailure::Open { text: format!("{error:#}"), code: console_code(&error) };
                self.in_session(|state| state.fail(failure))?;
                self.repaint();
                return Ok(None);
            }
        };
        let source = match session {
            Session::Open(source) => source,
            Session::Unopened { root, reason, failure } => {
                self.in_session(|state| {
                    state.phase = Phase::Unopened { root, reason, failure };
                    state.port_map = PortMap::Unopened;
                })?;
                self.repaint();
                return Ok(None);
            }
        };
        let online = source.follows();
        let wanted = || self.in_session(|_| ()).is_ok();
        if !online {
            self.busy(Busy::new(BusyKind::Bindings, BINDINGS_SECONDS, false))?;
        }
        let port_map = match source.instance_bindings() {
            Ok(Some(bindings)) => PortMap::Read(bindings),
            Ok(None) => PortMap::Online,
            Err(error) => PortMap::Failed(error),
        };
        let mut model = ViewModel::new(
            source.open_report(),
            source.snapshot_position(),
            opening.state_root.display().to_string(),
            None,
        );
        self.busy(Busy::new(BusyKind::Span, page_seconds(online), true))?;
        set_span(&mut model, time_span(&source, &wanted)?);
        match &opening.carry {
            None => model.tab = opening.tab.unwrap_or(LedgerView::Events),
            Some((tab, filters)) => {
                model.tab = *tab;
                model.filters = filters.clone();
                model.filters.to_timestamp_unix_ms = None;
                // A port is one instance by the map this session read: its
                // whole set again, or no port when the map has none.
                if let Some(port) = model.filters.port {
                    match port_map.entry(port) {
                        Some(entry) => model.filters.instance_ids = entry.members.clone(),
                        None => {
                            model.filters.port = None;
                            model.filters.instance_ids.clear();
                        }
                    }
                }
            }
        }
        let facts = SourceFacts::of(&source);
        let reader = source.material_reader();
        let tag = self.in_session(|state| {
            state.phase = Phase::Open(facts);
            state.model = Some(model);
            state.port_map = port_map;
            state.reader = Some(reader);
            state.frame_events = FrameEventsSlot::Empty;
            state.tag
        })?;
        self.repaint();
        Ok(Some((source, tag)))
    }

    fn execute(&self, source: &ReadSource, command: Command) {
        let finished = match command {
            Command::Reload { tag, earlier } => self.reload(source, tag, earlier),
            Command::JumpLatest { tag } => self.jump_latest(source, tag),
            Command::FollowOn { tag } => self.follow_on(source, tag),
            Command::FollowTick { tag } => {
                let ticked = self.follow_tick(source, tag);
                self.shared.tick_in_flight.store(false, std::sync::atomic::Ordering::SeqCst);
                ticked
            }
            Command::FrameEvents { frame, query } => self.frame_events(source, frame, &query),
            Command::RefreshReader => self.refresh_reader(source),
            Command::RereadInstances => self.reread_instances(source),
        };
        self.settle(finished);
        self.idle();
    }

    /// A request that ran to its end, or one a newer request took over: the
    /// newer one is in the queue, and nothing is left to do for this one.
    fn settle(&self, finished: Result<(), Superseded>) {
        match finished {
            Ok(()) | Err(Superseded) => {}
        }
    }

    /// Fills the timeline from the latest end at the pinned snapshot: backward
    /// windows, each read whole, until `FILL_ROWS` more rows show, position 1
    /// is read, `FILL_WINDOWS` windows were read, or `FILL_BUDGET` has passed —
    /// at least one window each time. Every page query is served on the
    /// Runtime's ledger writer (online), so what the budget leaves is for "read
    /// earlier"; the top bar states what was read. `earlier` continues below
    /// what is loaded; otherwise the view starts over from the pin, or from
    /// where a time bound is estimated to fall. Each window read is painted.
    fn reload(&self, source: &ReadSource, tag: RequestTag, earlier: bool) -> Result<(), Superseded> {
        let online = source.follows();
        let wanted = || self.page(tag);
        if !earlier {
            // And it retries a frame read that failed.
            let plan = self.current(tag, |state| {
                if state.frame_events.failed() {
                    state.frame_events = FrameEventsSlot::Empty;
                }
                state.model.as_ref().map(|model| {
                    (model.filters.to_timestamp_unix_ms, model.span(), model.query())
                })
            })?;
            let Some((bound, span, query)) = plan else {
                return Ok(());
            };
            let snapshot = source.snapshot_position();
            let upper = match bound {
                None => Ok(snapshot),
                Some(bound) => {
                    self.busy_for(tag, Busy::new(BusyKind::Probing, page_seconds(online), true))?;
                    let estimate = upper_for_time(span, snapshot, bound);
                    checked_upper(source, query, estimate, snapshot, &wanted)?
                }
            };
            let go_on = self.current(tag, |state| {
                state.effects.forget_frame = true;
                let Some(model) = state.model.as_mut() else {
                    return false;
                };
                match upper {
                    Ok(upper) => {
                        model.begin(upper);
                        true
                    }
                    Err(error) => {
                        model.begin(0);
                        model.query_error = Some(error);
                        false
                    }
                }
            })?;
            self.repaint();
            if !go_on {
                return Ok(());
            }
        }
        let started = Instant::now();
        let target = self.current(tag, |state| {
            state.model.as_mut().map(|model| {
                model.query_error = None;
                model.row_count() + FILL_ROWS
            })
        })?;
        let Some(target) = target else {
            return Ok(());
        };
        for read in 0..FILL_WINDOWS {
            let next = self.current(tag, |state| {
                let Some(model) = state.model.as_ref() else {
                    return None;
                };
                if read > 0 && (model.row_count() >= target || started.elapsed() >= FILL_BUDGET) {
                    return None;
                }
                let Some(window) = model.next_window() else {
                    return None;
                };
                let query = model.window_query(window);
                let kind = BusyKind::Reading { read: read + 1, of: FILL_WINDOWS };
                state.set_busy(Busy::new(kind, page_seconds(online), true));
                Some((window, query))
            })?;
            let Some((window, query)) = next else {
                break;
            };
            self.busy_changed();
            let pages = match query {
                Ok(query) => read_window(source, &query, &wanted)?.map_err(|error| read_failed(&error)),
                Err(error) => Err(error),
            };
            let applied = self.current(tag, |state| {
                let Some(model) = state.model.as_mut() else {
                    return false;
                };
                match pages {
                    Ok(pages) => {
                        model.apply_window(window, &pages);
                        true
                    }
                    Err(error) => {
                        model.query_error = Some(error);
                        false
                    }
                }
            })?;
            self.repaint();
            if !applied {
                break;
            }
        }
        Ok(())
    }

    /// A person's jump to the latest: the pin moves to the Runtime's latest
    /// position, status and facts are read again — neither read appends a
    /// ledger record — and the view starts over from the new pin, with no time
    /// bound.
    fn jump_latest(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        self.busy_for(tag, Busy::new(BusyKind::Repinning, ONLINE_PAGE_SECONDS, true))?;
        if let Err(error) = source.repin() {
            self.current(tag, |state| {
                if let Some(model) = state.model.as_mut() {
                    model.query_error = Some(read_failed(&error));
                }
            })?;
            self.repaint();
            return Ok(());
        }
        self.repinned(source, true)?;
        self.busy_for(tag, Busy::new(BusyKind::Instances, INSTANCES_SECONDS, false))?;
        source.refresh_instances();
        let facts = SourceFacts::of(source);
        self.in_session(|state| state.phase = Phase::Open(facts))?;
        self.start_at_pin(source, tag)
    }

    /// Turning following on catches up like a jump to the latest, without the
    /// status read: the pin moves to the latest position, any time bound is
    /// dropped and the view starts over from the new pin, however far the
    /// ledger has moved since. A repin that fails stops the following.
    fn follow_on(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        self.busy_for(tag, Busy::new(BusyKind::Repinning, ONLINE_PAGE_SECONDS, true))?;
        if let Err(error) = source.repin() {
            self.current(tag, |state| {
                if let Some(model) = state.model.as_mut() {
                    model.query_error = Some(read_failed(&error));
                }
                state.effects.follow_halted = true;
            })?;
            self.halted.set(Some(tag));
            self.repaint();
            return Ok(());
        }
        self.repinned(source, true)?;
        self.start_at_pin(source, tag)
    }

    /// The read face's pin and facts after it moved or polled, into the view
    /// model and the state for the whole session, whatever request is current:
    /// every pin a page or a frame read uses is the model's, so it never lags
    /// the face's own.
    fn repinned(&self, source: &ReadSource, moved: bool) -> Result<(), Superseded> {
        let (open, pin, facts) =
            (source.open_report(), source.snapshot_position(), SourceFacts::of(source));
        self.in_session(|state| {
            if moved {
                if let Some(model) = state.model.as_mut() {
                    model.set_pin(open, pin);
                }
            }
            state.phase = Phase::Open(facts);
        })
    }

    /// The view starts over from a pin just moved: no time bound, the time
    /// cursor back at the top, the span's end read at the new pin.
    fn start_at_pin(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        let span = self.current(tag, |state| {
            state.effects.cursor_top = true;
            state.model.as_mut().map(|model| {
                model.filters.to_timestamp_unix_ms = None;
                model.span()
            })
        })?;
        let Some(span) = span else {
            return Ok(());
        };
        self.busy_for(tag, Busy::new(BusyKind::Span, page_seconds(source.follows()), true))?;
        let wanted = || self.page(tag);
        let read = read_span(source, span, None, &wanted)?;
        self.current(tag, |state| {
            if let Some(model) = state.model.as_mut() {
                set_span(model, read);
            }
        })?;
        self.reload(source, tag, false)
    }

    /// One follow tick. One fact snapshot says where the ledger is and brings
    /// the task facts; only when the pin moved, or an earlier tick left windows
    /// unread, are the newer windows read onto the top of the view, and the
    /// span's end follows the pin. No status is read and nothing is written to
    /// the ledger. A failed poll stays in `follow_error` until a tick gets past
    /// it; a failed page read stays there too and stops the following.
    fn follow_tick(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        if self.halted.get() == Some(tag) {
            return Ok(());
        }
        self.busy_for(tag, Busy::new(BusyKind::Polling, POLL_SECONDS, false))?;
        let moved = match source.poll() {
            Ok(moved) => moved,
            Err(error) => {
                self.current(tag, |state| {
                    if let Some(model) = state.model.as_mut() {
                        model.follow_error = Some(read_failed(&error));
                    }
                })?;
                self.repaint();
                return Ok(());
            }
        };
        self.repinned(source, moved)?;
        let plan = self.current(tag, |state| {
            let Some(model) = state.model.as_mut() else {
                return (false, None);
            };
            let cleared = model.follow_error.take().is_some();
            if !moved && model.next_newer_window().is_none() {
                return (cleared, None);
            }
            // Filters that do not form a query already show why; following
            // reads nothing under them and does not state it a second time.
            let reads = if model.query().is_ok() { FILL_WINDOWS } else { 0 };
            (true, Some(reads))
        })?;
        let (painted, reads) = plan;
        if painted {
            self.repaint();
        }
        let Some(reads) = reads else {
            return Ok(());
        };
        let (pin, started) = (source.snapshot_position(), Instant::now());
        let wanted = || self.page(tag);
        let mut pin_time = None;
        let mut failed = false;
        for read in 0..reads {
            if read > 0 && started.elapsed() >= FILL_BUDGET {
                break;
            }
            let next = self.current(tag, |state| {
                let Some(model) = state.model.as_ref() else {
                    return None;
                };
                let Some(window) = model.next_newer_window() else {
                    return None;
                };
                let query = model.window_query(window);
                let kind = BusyKind::Reading { read: read + 1, of: reads };
                state.set_busy(Busy::new(kind, ONLINE_PAGE_SECONDS, true));
                Some((window, query))
            })?;
            let Some((window, query)) = next else {
                break;
            };
            self.busy_changed();
            let pages = match query {
                Ok(query) => read_window(source, &query, &wanted)?.map_err(|error| read_failed(&error)),
                Err(error) => Err(error),
            };
            if let Ok(pages) = &pages {
                pin_time = pin_time.or_else(|| {
                    let mut events = pages.iter().flat_map(|page| page.events());
                    let at_pin = events.find(|event| event.sequence == pin);
                    at_pin.map(|event| event.timestamp_unix_ms)
                });
            }
            let applied = self.current(tag, |state| {
                let Some(model) = state.model.as_mut() else {
                    return false;
                };
                match pages {
                    Ok(pages) => {
                        model.apply_newer_window(window, &pages);
                        true
                    }
                    Err(error) => {
                        // A read that failed stops the following, its reason
                        // kept on screen: a Runtime refusing each query is not
                        // asked again every tick. Turning following on again
                        // retries.
                        model.follow_error = Some(error);
                        state.effects.follow_halted = true;
                        false
                    }
                }
            })?;
            self.repaint();
            if !applied {
                self.halted.set(Some(tag));
                failed = true;
                break;
            }
        }
        if failed || !moved {
            return Ok(());
        }
        let span = self.current(tag, |state| state.model.as_ref().map(ViewModel::span))?;
        let Some(span) = span else {
            return Ok(());
        };
        if pin_time.is_none() {
            self.busy_for(tag, Busy::new(BusyKind::Span, ONLINE_PAGE_SECONDS, true))?;
        }
        let read = read_span(source, span, pin_time, &wanted)?;
        self.current(tag, |state| {
            if let Some(model) = state.model.as_mut() {
                set_span(model, read);
            }
        })?;
        self.repaint();
        Ok(())
    }

    /// The selected event's frame, when the loaded rows hold no capture of it:
    /// its own events, read once — one query by frame id — so an event still
    /// lands on its frame when the capture is filtered out or not loaded. A
    /// read the page says is incomplete cannot say the capture is absent.
    fn frame_events(
        &self,
        source: &ReadSource,
        frame: String,
        query: &EventQuery,
    ) -> Result<(), Superseded> {
        // Superseded when another session took the window, or the pane moved
        // to another frame: either way not wanted any more.
        let pending =
            || self.in_session(|state| state.frame_events.pending(&frame)).unwrap_or(false);
        if !pending() {
            return Ok(());
        }
        self.busy(Busy::new(BusyKind::FrameEvents, page_seconds(source.follows()), true))?;
        let events: Result<Vec<ProjectedEvent>, FrameLookupError> = match read_window(
            source,
            query,
            &pending,
        )? {
            Ok(pages) => match pages.iter().all(|page| page_source_complete(page)) {
                true => Ok(pages.iter().flat_map(|page| page.events().iter().cloned()).collect()),
                false => Err(FrameLookupError::Incomplete),
            },
            Err(error) => Err(FrameLookupError::Read(error.to_string())),
        };
        self.in_session(|state| {
            if state.frame_events.pending(&frame) {
                state.frame_events = FrameEventsSlot::Read { frame, events };
            }
        })?;
        self.repaint();
        Ok(())
    }

    /// A new material reader, published for the next frame read: on a latched
    /// material connection, taking one connects anew, here and not on the
    /// event loop.
    fn refresh_reader(&self, source: &ReadSource) -> Result<(), Superseded> {
        self.busy(Busy::new(BusyKind::Reconnecting, REBUILD_SECONDS, false))?;
        let reader = source.material_reader();
        self.in_session(|state| state.reader = Some(reader))
    }

    /// After a person's resume, whatever became of it: status and facts read
    /// again on a connection made for this read alone — never on the one the
    /// resume went out on — and left for the whole session, whatever request
    /// is current. Neither read appends a ledger record. Offline there is no
    /// status, and nothing is read. What the read found goes with it: one
    /// that failed is never said as one that was made.
    fn reread_instances(&self, source: &ReadSource) -> Result<(), Superseded> {
        self.busy(Busy::new(BusyKind::Instances, INSTANCES_SECONDS, false))?;
        source.reread_instances();
        let facts = SourceFacts::of(source);
        let found = match &facts.instances {
            InstancesSnapshot::Online(Ok(_)) => StatusReread::Read,
            InstancesSnapshot::Online(Err(failure)) => StatusReread::Failed(failure.clone()),
            InstancesSnapshot::Offline(_) => StatusReread::Offline,
        };
        self.in_session(|state| {
            state.phase = Phase::Open(facts);
            state.effects.status_reread = Some(found);
        })?;
        self.repaint();
        Ok(())
    }
}

/// About how long one page may take, on the face the session reads.
const fn page_seconds(online: bool) -> u64 {
    if online {
        ONLINE_PAGE_SECONDS
    } else {
        OFFLINE_PAGE_SECONDS
    }
}

/// A read's error as the view keeps it: the read face's text, and its console
/// code when it carries one; both are put into words when painted.
fn read_failed(error: &anyhow::Error) -> QueryError {
    QueryError::ReadFailed { text: error.to_string(), code: console_code(error) }
}

/// One window read whole: its pages, following the cursor when the reply's
/// byte limit splits it. `wanted` is asked before each page.
fn read_window(
    source: &ReadSource,
    query: &EventQuery,
    wanted: &dyn Fn() -> bool,
) -> Read<Vec<RuntimeEventQueryPage>, anyhow::Error> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        if !wanted() {
            return Err(Superseded);
        }
        let page = match source.query(query, MAX_RUNTIME_EVENT_QUERY_EVENTS, cursor) {
            Ok(page) => page,
            Err(error) => return Ok(Err(error)),
        };
        cursor = page.next_cursor().cloned();
        pages.push(page);
        if cursor.is_none() {
            return Ok(Ok(pages));
        }
    }
}

/// The time of the first event at or after `from` in the snapshot; `None`
/// when there is none. A page the read budget stopped may hold no row and
/// still go on (#411 A51), so the read follows the cursor until a row or its
/// end, for at most `MAX_SPAN_PAGES` pages. `wanted` is asked before each page.
fn event_time(
    source: &ReadSource,
    from: Option<u64>,
    wanted: &dyn Fn() -> bool,
) -> Read<Option<u64>, QueryError> {
    let query = EventQuery {
        view: Some(LedgerView::Events),
        from_sequence: from,
        ..EventQuery::default()
    };
    let mut cursor = None;
    for _ in 0..MAX_SPAN_PAGES {
        if !wanted() {
            return Err(Superseded);
        }
        let page = match source.query(&query, 1, cursor) {
            Ok(page) => page,
            Err(error) => return Ok(Err(read_failed(&error))),
        };
        if let Some(event) = page.events().first() {
            return Ok(Ok(Some(event.timestamp_unix_ms)));
        }
        cursor = page.next_cursor().cloned();
        if cursor.is_none() {
            return Ok(Ok(None));
        }
    }
    Ok(Err(QueryError::ReadFailed {
        text: format!("{CONSOLE_TIME_SPAN_PAGE_LIMIT}: {MAX_SPAN_PAGES}"),
        code: Some(CONSOLE_TIME_SPAN_PAGE_LIMIT),
    }))
}

/// The committed time span, read as the first and the last event of the snapshot.
fn time_span(source: &ReadSource, wanted: &dyn Fn() -> bool) -> Read<Option<(u64, u64)>, QueryError> {
    let first = match event_time(source, None, wanted)? {
        Ok(first) => first,
        Err(error) => return Ok(Err(error)),
    };
    let last = match event_time(source, Some(source.snapshot_position()), wanted)? {
        Ok(last) => last,
        Err(error) => return Ok(Err(error)),
    };
    Ok(Ok(first.zip(last)))
}

/// The span with its end moved to the event at the pin: `time` when a read
/// already holds that event, one read of it otherwise. A span never read is
/// read whole.
fn read_span(
    source: &ReadSource,
    span: Option<(u64, u64)>,
    time: Option<u64>,
    wanted: &dyn Fn() -> bool,
) -> Read<Option<(u64, u64)>, QueryError> {
    match (span, time) {
        (Some((first, _)), Some(last)) => Ok(Ok(Some((first, last)))),
        (Some((first, last)), None) => {
            let end = event_time(source, Some(source.snapshot_position()), wanted)?;
            Ok(end.map(|time| Some((first, time.unwrap_or(last)))))
        }
        (None, _) => time_span(source, wanted),
    }
}

/// A span read's outcome into the view: a failed read leaves the span as it
/// was and stays in `span_error` until one succeeds; the paint says it is the
/// span's.
fn set_span(model: &mut ViewModel, read: Result<Option<(u64, u64)>, QueryError>) {
    match read {
        Ok(span) => {
            model.set_span(span);
            model.span_error = None;
        }
        Err(error) => model.span_error = Some(error),
    }
}

/// Where a time bound is estimated to fall, from the committed span without any
/// read: the position it would take if events were even in time, plus
/// `TIME_SLACK_WINDOWS` windows, capped at the pin; 0 for a bound before the
/// first event. `checked_upper` makes the estimate safe.
fn upper_for_time(span: Option<(u64, u64)>, snapshot: u64, bound: u64) -> u64 {
    let Some((first, last)) = span else {
        return snapshot;
    };
    if bound >= last {
        return snapshot;
    }
    if bound < first {
        return 0;
    }
    let ratio = (bound - first) as f64 / (last - first) as f64;
    let estimate = (ratio * snapshot as f64) as u64;
    (estimate + TIME_SLACK_WINDOWS * WINDOW).min(snapshot)
}

/// Makes an estimated start safe, since reading only goes down from it: one
/// probe with the view's own query — its time bound included — asks for the
/// first matching event above the start. None: nothing above could show, and
/// reading starts there. One found: the start moves above it by a step that
/// doubles each time, for at most `TIME_PROBES` probes and within the fill
/// budget; past either, the pin, which is always safe. A page the read budget
/// stopped may hold no row and still go on (#411 A51): it decides nothing
/// above the start, so that is the pin too. `wanted` is asked before each probe.
fn checked_upper(
    source: &ReadSource,
    query: Result<EventQuery, QueryError>,
    estimate: u64,
    snapshot: u64,
    wanted: &dyn Fn() -> bool,
) -> Read<u64, QueryError> {
    let (mut upper, mut step, started) = (estimate, TIME_SLACK_WINDOWS * WINDOW, Instant::now());
    for probe in 0..TIME_PROBES {
        if upper >= snapshot || (probe > 0 && started.elapsed() >= FILL_BUDGET) {
            return Ok(Ok(snapshot));
        }
        let mut probed = match &query {
            Ok(query) => query.clone(),
            Err(error) => return Ok(Err(error.clone())),
        };
        probed.from_sequence = Some(upper + 1);
        if let Err(error) = probed.validate() {
            return Ok(Err(QueryError::Rejected(error.code().to_string())));
        }
        if !wanted() {
            return Err(Superseded);
        }
        let page = match source.query(&probed, 1, None) {
            Ok(page) => page,
            Err(error) => return Ok(Err(read_failed(&error))),
        };
        let Some(found) = page.events().first() else {
            return Ok(Ok(if page.next_cursor().is_some() { snapshot } else { upper }));
        };
        upper = found.sequence + step;
        step *= 2;
    }
    Ok(Ok(snapshot))
}
