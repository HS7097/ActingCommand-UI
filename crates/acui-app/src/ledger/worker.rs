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

use acui_model::{Filters, QueryError, ViewModel, FILL_ROWS, FILL_WINDOWS, MAX_WIDTH, WINDOW};
use acui_rows::outcome::{CONSOLE_TIME_SPAN_PAGE_LIMIT, CONSOLE_WINDOW_PAGE_LIMIT};
use acui_rows::{
    page_source_complete, EventQuery, LedgerView, ProjectedEvent, RuntimeEventQueryPage,
    MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use acui_source::{console_code, ReadSource, RunRecovery, Session, SourceMode};

use crate::ledger::state::{
    Busy, BusyKind, FrameEventsSlot, FrameLookupError, InstancesSnapshot, LedgerFailure,
    LedgerShared, LedgerState, Phase, PortMap, RequestTag, SourceFacts, StatusReread,
};
use crate::{launcher, report};

/// How long one fill may keep reading windows after the first.
const FILL_BUDGET: Duration = Duration::from_secs(1);
/// Windows of slack above a time bound's estimated position, and the most
/// probes that check it. Offline only: online the view takes no time bound.
const TIME_SLACK_WINDOWS: u64 = 2;
const TIME_PROBES: usize = 3;
/// The most pages one window of positions is read in, following its cursor: a
/// window of the timeline (at most `MAX_WIDTH` positions, 16 full pages) or
/// of a frame's lookup. The rest of the limit is for pages the reply's byte
/// limit or the Runtime's read budget cut short. A window still going on past
/// it fails, loudly (`CONSOLE_WINDOW_PAGE_LIMIT`), and is not shown as read.
const MAX_WINDOW_PAGES: usize = 64;
/// The lowest position a ledger holds, where the span's first event is read.
/// Runtime v0.12.47 states no other floor: a ledger's records are `1..=head`,
/// contiguous (the writer's open and the offline opening check it,
/// ledger-store.md LN-OFF-1; global-ledger-query.md), and eviction removes a
/// capture's material file under an eviction proof, never a record
/// (ledger-store.md, retention); neither the status (`RuntimeStateSource`),
/// the fact snapshot, a page's read scope nor an eviction proof names a lowest
/// readable position. Should the point read find no event there all the same,
/// one window of at most `MAX_WIDTH` positions above it is read, of one row,
/// and nothing wider: the read fails rather than go on.
const FIRST_POSITION: u64 = 1;

/// About how long a read of each kind may take at most, in seconds, as the
/// busy line states it; from acui-source's own bounds. One online page: the
/// page connection's 20 s io timeout, eight times over, plus the 1 s backoff
/// before its one retry. One offline page: the 30 s each later read of the
/// snapshot sets itself.
const ONLINE_PAGE_SECONDS: u64 = 161;
const OFFLINE_PAGE_SECONDS: u64 = 30;
/// Opening offline: the ledger's own 60 s opening, then the 30 s fact replay.
/// Opening online (and `auto`, which opens no other face): the three
/// connections, each its connect, `Health` and `BindChannel` (5 s, 20 s and
/// 5 s each: 90 s), then the status read and the fact read the pin is taken
/// from (5 s each, the fact read after the main connection's rebuild when the
/// status read latched it: 25 s). No page is asked to open.
const OFFLINE_OPEN_SECONDS: u64 = 90;
const ONLINE_OPEN_SECONDS: u64 = 115;
/// The offline port map's binding read.
const BINDINGS_SECONDS: u64 = 30;
/// One exchange on the main connection (5 s), after its rebuild at worst
/// (three times 5 s).
const POLL_SECONDS: u64 = 20;
/// The status and the fact read on the main connection, each after its
/// rebuild at worst: a jump to the latest's repin, and the read after a
/// person's resume.
const INSTANCES_SECONDS: u64 = 40;
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
    /// The selected frame's own events, by frame id — online within the window
    /// of positions `searched`, offline by frame id alone, as before slice 1
    /// (`searched` is `None`): one query, read once while the slot is still
    /// pending for `frame`. Not superseded by a new generation: the answer
    /// depends on the frame alone.
    FrameEvents { frame: String, query: EventQuery, searched: Option<(u64, u64)> },
    /// A frame read found the material reader latched, or its rebuild failed:
    /// a new one is made and published.
    RefreshReader,
    /// A person's resume came back (`resume`): status and facts are read again
    /// on a connection of their own. Not superseded by a new generation: the
    /// status belongs to the session, not to the view.
    RereadInstances,
}

/// What a session opens: the state root and the face it asks for — the one
/// `--source` asked for, or the offline face a person pressed for in a window
/// of `auto` (`LedgerHandle::open_offline`) — and the view to start with — the
/// window's first one (`tab`), or the one the last session left (`carry`), its
/// time bound dropped.
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

/// How a fill ended, for the span read after it (`span_after`).
#[derive(Default)]
struct Fill {
    /// The time of the event at the pin, when a window the fill read holds it.
    pin_time: Option<u64>,
    /// The fill stopped at a read that failed — a page the Runtime refused, a
    /// window past its page limit, any read the face could not answer — whose
    /// reason is on screen: no more page is asked for this request, and
    /// following stops, until a person reads again.
    stopped: bool,
}

/// Starts the reader of `session` of window `id` on `acui-ledger-<id>`, and
/// gives back where its requests go. The thread opens the read face first.
pub fn spawn(
    id: u64,
    session: u64,
    shared: Arc<LedgerShared>,
    opening: Opening,
) -> io::Result<Sender<Command>> {
    let (sender, commands) = mpsc::channel();
    let worker = Worker {
        id,
        session,
        shared,
        halted: Cell::new(None),
        first_time: Cell::new(None),
        span_pin: Cell::new(None),
    };
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
    /// The time of the ledger's first event, once read: the same at every pin,
    /// so read once per session.
    first_time: Cell<Option<u64>>,
    /// The pin the committed span was last read at, once a read of it
    /// succeeded: a fill at that pin asks no span read again.
    span_pin: Cell<Option<u64>>,
}

impl Worker {
    fn run(self, opening: Opening, commands: Receiver<Command>) {
        let source = match catch_unwind(AssertUnwindSafe(|| self.start(&opening))) {
            Ok(Some(source)) => source,
            // Unopened, offered and not opened, failed, or another session
            // took the window: there is nothing to read through, and no
            // request is sent to it.
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

    /// Opens the session, then reads its first view and its committed span;
    /// the source to read through, when it opened.
    fn start(&self, opening: &Opening) -> Option<ReadSource> {
        let opened = self.open(opening);
        let source = match opened {
            Ok(Some((source, tag))) => {
                let first = self
                    .reload(&source, tag, false)
                    .and_then(|fill| self.span_after(&source, tag, fill));
                self.settle(first);
                self.reported(&source);
                Some(source)
            }
            Ok(None) => None,
            Err(Superseded) => None,
        };
        self.idle();
        source
    }

    /// The read face, opened here; then, offline, its port map, read before
    /// anything is left in the state. The committed span is read after the
    /// first fill (`span_after`), whose first window may already hold the event
    /// at the pin. `None` when it did not open: the state says why.
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
            // `auto` could not read online: no face is opened, and nothing of
            // the ledger is read; the window offers the offline face.
            Session::OfflineOffered { root, reason } => {
                self.in_session(|state| {
                    state.phase = Phase::OfflineOffered { root, reason };
                    state.port_map = PortMap::Unopened;
                })?;
                self.repaint();
                return Ok(None);
            }
        };
        if !source.follows() {
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
            Command::Reload { tag, earlier } => self
                .reload(source, tag, earlier)
                .and_then(|fill| self.span_after(source, tag, fill)),
            Command::JumpLatest { tag } => self.jump_latest(source, tag),
            Command::FollowOn { tag } => self.follow_on(source, tag),
            Command::FollowTick { tag } => {
                let ticked = self.follow_tick(source, tag);
                self.shared.tick_in_flight.store(false, std::sync::atomic::Ordering::SeqCst);
                ticked
            }
            Command::FrameEvents { frame, query, searched } => {
                self.frame_events(source, frame, &query, searched)
            }
            Command::RefreshReader => self.refresh_reader(source),
            Command::RereadInstances => self.reread_instances(source),
        };
        self.settle(finished);
        self.reported(source);
        self.idle();
    }

    /// A request that ran to its end, or one a newer request took over: the
    /// newer one is in the queue, and nothing is left to do for this one.
    fn settle(&self, finished: Result<(), Superseded>) {
        match finished {
            Ok(()) | Err(Superseded) => {}
        }
    }

    /// What the read face states about itself after a request's pages, into
    /// the view, whatever request is current: online, whether every page read
    /// at the pin read its source whole, `None` before the first of them.
    fn reported(&self, source: &ReadSource) {
        let open = source.open_report();
        let set = self.in_session(|state| {
            if let Some(model) = state.model.as_mut() {
                model.set_open(open);
            }
        });
        if set.is_ok() {
            self.repaint();
        }
    }

    /// Fills the timeline from the latest end at the pinned snapshot: backward
    /// windows, each read whole, until `FILL_ROWS` more rows show, position 1
    /// is read, `FILL_WINDOWS` windows were read, or `FILL_BUDGET` has passed —
    /// at least one window each time. Every page query costs the Runtime a
    /// read and a verification (online, on its ledger reader pool), so what the
    /// budget leaves is for "read earlier"; the top bar states what was read. `earlier` continues below
    /// what is loaded; otherwise the view starts over from the pin, or, offline,
    /// from where a time bound is estimated to fall. Each window read is
    /// painted; each is `from_sequence` to `to_sequence`, at most `MAX_WIDTH`
    /// positions, of pages of `MAX_RUNTIME_EVENT_QUERY_EVENTS` rows at the pin,
    /// with run recovery. A window that failed — the Runtime refused a page, or
    /// the window passed its page limit — ends the fill with its code on screen,
    /// stops following, and nothing asks a page again until a person reads
    /// again: neither this request's span read (`Fill::stopped`) nor the next
    /// follow tick. The time of the event at the pin, when a window read holds
    /// it, so the span's end needs no read of its own.
    fn reload(
        &self,
        source: &ReadSource,
        tag: RequestTag,
        earlier: bool,
    ) -> Result<Fill, Superseded> {
        let online = source.follows();
        let wanted = || self.page(tag);
        let pin = source.snapshot_position();
        let mut fill = Fill::default();
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
                return Ok(fill);
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
            let stopped = matches!(&upper, Err(error) if read_stopped(error));
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
                        state.effects.follow_halted |= stopped;
                        false
                    }
                }
            })?;
            self.repaint();
            if !go_on {
                self.halt(tag, stopped);
                fill.stopped = stopped;
                return Ok(fill);
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
            return Ok(fill);
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
                Ok(query) => read_window(source, &query, RunRecovery::Derived, &wanted)?,
                Err(error) => Err(error),
            };
            if let Ok(pages) = &pages {
                fill.pin_time = fill.pin_time.or_else(|| event_time_at(pages, pin));
            }
            let stopped = matches!(&pages, Err(error) if read_stopped(error));
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
                        // Following, if on, would ask this view's next window
                        // by itself at its next tick: it stops, as a failed
                        // follow read stops it.
                        state.effects.follow_halted |= stopped;
                        false
                    }
                }
            })?;
            self.repaint();
            if !applied {
                self.halt(tag, stopped);
                fill.stopped = stopped;
                break;
            }
        }
        Ok(fill)
    }

    /// After a read of `tag` that failed (`stopped`): a follow tick of that
    /// request still queued behind is not run (`follow_tick`); the follow
    /// timer itself stops on the event loop (`Effects::follow_halted`).
    fn halt(&self, tag: RequestTag, stopped: bool) {
        if stopped {
            self.halted.set(Some(tag));
        }
    }

    /// A person's jump to the latest: status and facts are read again and the
    /// pin moves to the head they state (`ReadSource::repin`) — neither read
    /// appends a ledger record, and no page is asked for it — and the view
    /// starts over from the new pin, with no time bound. The instances are the
    /// ones this read found, whatever became of the pin.
    fn jump_latest(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        self.busy_for(tag, Busy::new(BusyKind::Repinning, INSTANCES_SECONDS, false))?;
        if let Err(error) = source.repin() {
            let facts = SourceFacts::of(source);
            self.in_session(|state| state.phase = Phase::Open(facts))?;
            self.current(tag, |state| {
                if let Some(model) = state.model.as_mut() {
                    model.query_error = Some(read_failed(&error));
                }
            })?;
            self.repaint();
            return Ok(());
        }
        self.repinned(source, true)?;
        self.start_at_pin(source, tag)
    }

    /// Turning following on catches up like a jump to the latest, without the
    /// status read: one follow poll (`ReadSource::poll`), whose fact snapshot
    /// moves the pin to the head it states, any time bound is dropped and the
    /// view starts over from the pin, however far the ledger has moved since.
    /// A poll that fails stops the following.
    fn follow_on(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        self.busy_for(tag, Busy::new(BusyKind::Repinning, POLL_SECONDS, false))?;
        if let Err(error) = source.poll() {
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
    /// cursor back at the top, and after the fill the span's end at the new
    /// pin (`span_after`).
    fn start_at_pin(&self, source: &ReadSource, tag: RequestTag) -> Result<(), Superseded> {
        let live = self.current(tag, |state| {
            state.effects.cursor_top = true;
            state.model.as_mut().map(|model| model.filters.to_timestamp_unix_ms = None).is_some()
        })?;
        if !live {
            return Ok(());
        }
        let fill = self.reload(source, tag, false)?;
        self.span_after(source, tag, fill)
    }

    /// After a fill a person asked for — the first one, any reload, "read
    /// earlier", a jump to the latest, following turned on — the committed
    /// span at the pin (`read_span`), unless it was read at this pin already
    /// (`span_pin`). So a span the first fill could not read, because a newer
    /// request took it over, is read by the next fill that runs to its end. A
    /// fill that stopped at a read that failed asks no span page: its reason
    /// is on screen, and no page is asked again until a person reads again;
    /// only a span it can complete without a page moves on.
    fn span_after(
        &self,
        source: &ReadSource,
        tag: RequestTag,
        fill: Fill,
    ) -> Result<(), Superseded> {
        if self.span_pin.get() == Some(source.snapshot_position()) {
            return Ok(());
        }
        let needs_page = self.first_time.get().is_none() || fill.pin_time.is_none();
        if fill.stopped && needs_page {
            return Ok(());
        }
        self.read_span(source, tag, fill.pin_time)
    }

    /// The committed span at the pin into the view: the ledger's first event's
    /// time, read once per session (`first_time`), and the time of the event at
    /// the pin — `pin_time` when a window just read held it, one point read
    /// otherwise. An empty ledger (pin 0) has no span and asks no page. A read
    /// that fails leaves the span as it was and says why (`set_span`); one
    /// that succeeds is kept as read at this pin (`span_pin`).
    fn read_span(
        &self,
        source: &ReadSource,
        tag: RequestTag,
        pin_time: Option<u64>,
    ) -> Result<(), Superseded> {
        let pin = source.snapshot_position();
        let read = if pin == 0 {
            Ok(None)
        } else {
            if self.first_time.get().is_none() || pin_time.is_none() {
                let bound = page_seconds(source.follows());
                self.busy_for(tag, Busy::new(BusyKind::Span, bound, true))?;
            }
            let wanted = || self.page(tag);
            match self.first_time(source, &wanted)? {
                Err(error) => Err(error),
                Ok(first) => match pin_time {
                    Some(last) => Ok(Some((first, last))),
                    // The pin's own event: the ledger holds every position up
                    // to its head, so a point read that finds none fails.
                    None => match event_time(source, (pin, pin), &wanted)? {
                        Ok(Some(last)) => Ok(Some((first, last))),
                        Ok(None) => Err(span_failed((pin, pin), false)),
                        Err(error) => Err(error),
                    },
                },
            }
        };
        let read_whole = read.is_ok();
        self.current(tag, |state| {
            if let Some(model) = state.model.as_mut() {
                set_span(model, read);
            }
        })?;
        if read_whole {
            self.span_pin.set(Some(pin));
        }
        self.repaint();
        Ok(())
    }

    /// The time of the ledger's first event: a point read of `FIRST_POSITION`,
    /// and only when that holds no event, one window of at most `MAX_WIDTH`
    /// positions from it, within the pin, of one row; kept for the session
    /// once read — the first event is the same at every pin. Nothing found in
    /// that window fails: no read goes wider. Asked only at a pin past 0.
    fn first_time(&self, source: &ReadSource, wanted: &dyn Fn() -> bool) -> Read<u64, QueryError> {
        if let Some(first) = self.first_time.get() {
            return Ok(Ok(first));
        }
        let found = match event_time(source, (FIRST_POSITION, FIRST_POSITION), wanted)? {
            Ok(Some(first)) => Ok(first),
            Ok(None) => {
                let last = (FIRST_POSITION + MAX_WIDTH - 1).min(source.snapshot_position());
                let window = (FIRST_POSITION, last);
                if last <= FIRST_POSITION {
                    Err(span_failed(window, false))
                } else {
                    match event_time(source, window, wanted)? {
                        Ok(Some(first)) => Ok(first),
                        Ok(None) => Err(span_failed(window, false)),
                        Err(error) => Err(error),
                    }
                }
            }
            Err(error) => Err(error),
        };
        if let Ok(first) = found {
            self.first_time.set(Some(first));
        }
        Ok(found)
    }

    /// One follow tick. One fact snapshot says where the ledger is and brings
    /// the task facts; only when the pin moved, or an earlier tick left windows
    /// unread, are the newer windows read onto the top of the view, and the
    /// span's end follows the pin (`read_span`; a span read that failed is not
    /// asked again by a tick). No status is read and nothing is written to the
    /// ledger. A failed poll stays in `follow_error` until a tick gets past it;
    /// a failed page read stays there too and stops the following, so a page
    /// the Runtime refused is not asked again every tick.
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
                Ok(query) => read_window(source, &query, RunRecovery::Derived, &wanted)?,
                Err(error) => Err(error),
            };
            if let Ok(pages) = &pages {
                pin_time = pin_time.or_else(|| event_time_at(pages, pin));
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
        // A span read that failed is not asked again by a tick: its reason
        // stays on screen until a person reads again (a jump to the latest,
        // following turned on, a reopen). Only a span this tick can complete
        // without a page moves on.
        let needs_page = self.first_time.get().is_none() || pin_time.is_none();
        let span_stale = self.current(tag, |state| {
            state.model.as_ref().is_some_and(|model| model.span_error.is_some())
        })?;
        if span_stale && needs_page {
            return Ok(());
        }
        self.read_span(source, tag, pin_time)
    }

    /// The selected event's frame, when the loaded rows hold no capture of it:
    /// its own events, read once — one query by frame id, online within the
    /// window of positions `searched` around the event that names it, without
    /// run recovery — so an event still lands on its frame when the capture is
    /// filtered out or not loaded. A read the page says is incomplete cannot
    /// say the capture is absent; one that is complete and holds none says in
    /// which positions it looked, when it looked in a window of them.
    fn frame_events(
        &self,
        source: &ReadSource,
        frame: String,
        query: &EventQuery,
        searched: Option<(u64, u64)>,
    ) -> Result<(), Superseded> {
        // Superseded when another session took the window, or the pane moved
        // to another frame: either way not wanted any more.
        let pending =
            || self.in_session(|state| state.frame_events.pending(&frame)).unwrap_or(false);
        if !pending() {
            return Ok(());
        }
        self.busy(Busy::new(BusyKind::FrameEvents, page_seconds(source.follows()), true))?;
        let events: Result<Vec<ProjectedEvent>, FrameLookupError> =
            match read_window(source, query, RunRecovery::Skipped, &pending)? {
                Ok(pages) => match pages.iter().all(|page| page_source_complete(page)) {
                    true => {
                        Ok(pages.iter().flat_map(|page| page.events().iter().cloned()).collect())
                    }
                    false => Err(FrameLookupError::Incomplete),
                },
                Err(QueryError::ReadFailed { text, code }) => {
                    Err(FrameLookupError::Read { text, code })
                }
                // `read_window` fails with a read's error only; any other is
                // still said, as it is.
                Err(other) => Err(FrameLookupError::Read { text: format!("{other:?}"), code: None }),
            };
        self.in_session(|state| {
            if state.frame_events.pending(&frame) {
                state.frame_events = FrameEventsSlot::Read { frame, searched, events };
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

/// A read's error as the view keeps it: the read face's text, and the outcome
/// code it carries, if any — the console's own, or the Runtime's code of a
/// page it did not answer; both are put into words when painted.
fn read_failed(error: &anyhow::Error) -> QueryError {
    QueryError::ReadFailed { text: error.to_string(), code: console_code(error) }
}

/// Whether an error that ended a fill is a read that failed — the face could
/// not answer, or the console stopped a window at its page limit — rather
/// than filters that form no query: after one, no page is asked again until
/// a person reads again.
fn read_stopped(error: &QueryError) -> bool {
    matches!(error, QueryError::ReadFailed { .. })
}

/// One window of positions read whole: its pages of
/// `MAX_RUNTIME_EVENT_QUERY_EVENTS` rows at the pin, following the cursor when
/// the reply's byte limit or the Runtime's read budget cut a page short (a
/// budget-cut page answers a cursor past its start, so each page goes on), for
/// at most `MAX_WINDOW_PAGES` pages; with run recovery when `recovery` derives
/// it. A window still going on past that fails, loudly
/// (`CONSOLE_WINDOW_PAGE_LIMIT`), and none of its pages is shown. A page that
/// fails fails the window, as the read face said it, and is not asked again
/// here. `wanted` is asked before each page.
fn read_window(
    source: &ReadSource,
    query: &EventQuery,
    recovery: RunRecovery,
    wanted: &dyn Fn() -> bool,
) -> Read<Vec<RuntimeEventQueryPage>, QueryError> {
    let mut pages = Vec::new();
    let mut cursor = None;
    for _ in 0..MAX_WINDOW_PAGES {
        if !wanted() {
            return Err(Superseded);
        }
        let page = match source.query(query, MAX_RUNTIME_EVENT_QUERY_EVENTS, cursor, recovery) {
            Ok(page) => page,
            Err(error) => return Ok(Err(read_failed(&error))),
        };
        cursor = page.next_cursor().cloned();
        pages.push(page);
        if cursor.is_none() {
            return Ok(Ok(pages));
        }
    }
    Ok(Err(QueryError::ReadFailed {
        text: format!("{CONSOLE_WINDOW_PAGE_LIMIT}: {MAX_WINDOW_PAGES}"),
        code: Some(CONSOLE_WINDOW_PAGE_LIMIT),
    }))
}

/// The time of the first event in positions `from..=to` at the pin: one page
/// of limit 1, without run recovery, its cursor never followed — a point read
/// when `from == to`. `None` when the page read the whole range and found no
/// event. A page that holds no event and still goes on — the Runtime's read
/// budget stopped it before the range's end — decides nothing, and the read
/// fails (`span_failed`). `wanted` is asked before the page.
fn event_time(
    source: &ReadSource,
    (from, to): (u64, u64),
    wanted: &dyn Fn() -> bool,
) -> Read<Option<u64>, QueryError> {
    if !wanted() {
        return Err(Superseded);
    }
    let query = EventQuery {
        view: Some(LedgerView::Events),
        from_sequence: Some(from),
        to_sequence: Some(to),
        ..EventQuery::default()
    };
    let page = match source.query(&query, 1, None, RunRecovery::Skipped) {
        Ok(page) => page,
        Err(error) => return Ok(Err(read_failed(&error))),
    };
    if let Some(event) = page.events().first() {
        return Ok(Ok(Some(event.timestamp_unix_ms)));
    }
    if page.next_cursor().is_some() {
        return Ok(Err(span_failed((from, to), true)));
    }
    Ok(Ok(None))
}

/// A span read that came back without the event it must hold, in positions
/// `from` to `to`: cut by the Runtime's read budget, or with none in its whole
/// range (`CONSOLE_TIME_SPAN_PAGE_LIMIT`).
fn span_failed((from, to): (u64, u64), budget_cut: bool) -> QueryError {
    let cut = if budget_cut { " read_budget" } else { "" };
    QueryError::ReadFailed {
        text: format!("{CONSOLE_TIME_SPAN_PAGE_LIMIT}: {from}–{to}{cut}"),
        code: Some(CONSOLE_TIME_SPAN_PAGE_LIMIT),
    }
}

/// The time of the event at `pin` among a window's pages, when they hold it.
fn event_time_at(pages: &[RuntimeEventQueryPage], pin: u64) -> Option<u64> {
    let mut events = pages.iter().flat_map(|page| page.events());
    events.find(|event| event.sequence == pin).map(|event| event.timestamp_unix_ms)
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
/// Offline only: online the view takes no time bound, and the online face
/// sends no page with a time condition (#429).
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
        let page = match source.query(&probed, 1, None, RunRecovery::Skipped) {
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
