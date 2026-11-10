// SPDX-License-Identifier: GPL-3.0-only
//! What the ledger window shows, held apart from the read face it was read
//! through, and shared between the window's reader thread (`worker`) and the
//! event loop. The reader opens the read face and keeps it; it leaves the view
//! model, the port map, what it says about the read face itself
//! (`SourceFacts`), the material reader and the selected frame's own events
//! here, under the lock, and never reads while it holds it. The paint
//! (`paint`) reads this and nothing else, holding the lock only while it
//! paints, so it never waits on a read.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Instant;

use acui_model::{Filters, ViewModel};
use acui_rows::{InstanceId, PortBindings, PortEntry, ProjectedEvent};
use acui_source::{
    ClientFailure, InstanceFacts, LedgerOpenFailure, MaterialReader, OfflineFacts, OfflineReason,
    ReadSource, RuntimeInstance, RuntimeInstances,
};

/// One ledger window's state and the two flags its reader and its timers
/// share with the event loop.
pub struct LedgerShared {
    pub state: Mutex<LedgerState>,
    /// A follow tick is queued or running on the reader: the follow timer
    /// sends no other until it has finished, so ticks never pile up behind a
    /// slow page.
    pub tick_in_flight: AtomicBool,
    /// A repaint is handed to the event loop and has not run yet: no second
    /// one is handed over meanwhile.
    pub repaint_pending: AtomicBool,
}

/// Which request of which window a read belongs to. `session` counts the read
/// faces this window opened ("reopen" starts a new one); `generation` counts
/// the changes that supersede every read in flight — a tab, a filter, the time
/// cursor at rest, a jump to the latest, following turned on or off, a reopen.
/// A read checks its tag against the state's before it starts, before each page
/// and before it leaves anything here; a read whose tag is no longer the
/// state's leaves nothing and stops at the next page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTag {
    pub window: u64,
    pub session: u64,
    pub generation: u64,
}

/// One ledger window's view: whether its read face is open and what it says,
/// the view model when it is, the port map, the selected frame's own events
/// when they had to be read, what the reader is doing, and what it left for
/// the event loop to do.
pub struct LedgerState {
    /// The request every read in flight must still match.
    pub tag: RequestTag,
    pub phase: Phase,
    /// `None` until the read face opened: there is no page to model. A failure
    /// after it opened keeps it, so a reopen starts from the same tab and
    /// filters the boxes still show.
    pub model: Option<ViewModel>,
    /// What the port box picks from: offline, instance identity by ADB port,
    /// read once at the pinned snapshot; online, the instances the last
    /// status read reports. The box's items come from it at every paint
    /// (`paint::port_options`), in the language shown then.
    pub port_map: PortMap,
    pub frame_events: FrameEventsSlot,
    /// What the reader is reading now, and since when; `None` while it waits
    /// for a request.
    pub busy: Option<Busy>,
    /// The material reader the session gives out, published by the reader once
    /// the face opened and again whenever a frame read found it latched; a new
    /// frame read takes a clone of it.
    pub reader: Option<MaterialReader>,
    /// What the reader asks the event loop to do at the next paint.
    pub effects: Effects,
}

/// Whether the window's read face is open.
pub enum Phase {
    /// The window is up and the reader is opening the read face.
    Opening { root: PathBuf },
    /// Open, with what the paint takes from it, as the reader last copied it.
    Open(SourceFacts),
    /// The offline face the ledger refused to open, with its own error.
    /// Nothing was read: no page, no port map, no material.
    Unopened { root: PathBuf, reason: OfflineReason, failure: LedgerOpenFailure },
    /// `auto` could not read online, for `reason`, and opened no face: the
    /// window says why and offers the offline face, which only a person's
    /// press opens (a session of `SourceMode::Offline`). Nothing was read: no
    /// page, no port map, no material, no fact replay.
    OfflineOffered { root: PathBuf, reason: OfflineReason },
    /// The read face could not be opened, or the reader stopped: nothing more
    /// is read in this session.
    Failed { root: PathBuf, failure: LedgerFailure },
}

/// Why a session reads nothing more: put into words when painted.
pub enum LedgerFailure {
    /// The read face refused to open: its own words, and the console's own code
    /// beneath them when it carries one.
    Open { text: String, code: Option<&'static str> },
    /// The reader thread panicked: its message, when it gave one as text.
    Panicked(Option<String>),
    /// The system refused the reader thread: its error.
    NoThread(String),
    /// The reader thread is gone, and a request could not be handed to it.
    Gone,
}

/// What the paint takes from an open read face: where it reads, which face
/// answered and why, whether it can follow, and the instances as last read.
/// Copied by the reader, outside the lock, after every read that changes them.
pub struct SourceFacts {
    pub state_root: PathBuf,
    pub offline_reason: Option<OfflineReason>,
    pub follows: bool,
    pub instances: InstancesSnapshot,
}

/// The instances as the read face last read them: online, status and facts
/// (see `acui_source::RuntimeInstances`); offline, the facts at the pin.
pub enum InstancesSnapshot {
    Online(Result<RuntimeInstances, ClientFailure>),
    Offline(OfflineFacts),
}

impl SourceFacts {
    /// The facts as `source` holds them now. Copying them never connects.
    pub fn of(source: &ReadSource) -> Self {
        let instances = match source.instance_facts() {
            InstanceFacts::Online(read) => InstancesSnapshot::Online((*read).clone()),
            InstanceFacts::Offline(facts) => InstancesSnapshot::Offline(facts.clone()),
        };
        Self {
            state_root: source.state_root().to_path_buf(),
            offline_reason: source.offline_reason().cloned(),
            follows: source.follows(),
            instances,
        }
    }
}

/// The port map this session filters by: the offline read face derives it
/// once from the binding facts through the pinned snapshot; online it is the
/// instances the last status read reports, made again at every status read.
pub enum PortMap {
    /// The read face is still opening.
    Opening,
    /// Online: the Runtime derives no binding through its page, so the box
    /// lists the instances the session's last status read reports — at open,
    /// at a jump to the latest, after a person's resume — in its order (by
    /// alias), or why that read failed; and the instance picked before it,
    /// when that read took the pick away (`check_pick`). Only the ids the
    /// status reports now: the ids a port was bound to before belong to the
    /// offline face.
    Online { instances: Result<Vec<OnlineInstance>, ClientFailure>, dropped: Option<DroppedPick> },
    /// The read face is unopened, offered and not opened, or failed: nothing
    /// was read.
    Unopened,
    /// The read face refused, with its typed code as the outermost context.
    Failed(anyhow::Error),
    Read(PortBindings),
}

/// One instance a status read reports, as the box lists it online: its id,
/// its alias, and the ADB port the status states for it — none for a
/// serial-configured instance or one without an ADB target, and the box shows
/// no port the status did not state.
#[derive(Debug, Clone)]
pub struct OnlineInstance {
    pub instance_id: InstanceId,
    pub alias: String,
    pub adb_port: Option<u16>,
}

/// An instance picked online that the status read after the pick did not
/// report, or could not confirm since it failed: the pick went back to every
/// instance, and the line under the boxes says which went — by its alias when
/// the list it was picked from named it.
#[derive(Debug, Clone)]
pub struct DroppedPick {
    pub instance_id: InstanceId,
    pub alias: Option<String>,
}

/// What an item of the box after the first one picks: offline a port of the
/// session's map, queried as its whole set of ids; online one instance the
/// status read reports, by its id.
#[derive(Debug, Clone, PartialEq)]
pub enum PortChoice {
    Port(u16),
    Instance(InstanceId),
}

/// What the view's filters picked in the box: a port, or, with no port and a
/// single id, one instance; `None` while every instance is shown.
pub fn box_pick(filters: &Filters) -> Option<PortChoice> {
    match (filters.port, filters.instance_ids.as_slice()) {
        (Some(port), _) => Some(PortChoice::Port(port)),
        (None, [id]) => Some(PortChoice::Instance(*id)),
        (None, _) => None,
    }
}

impl PortMap {
    /// The online map of the status read `instances` holds: the instances it
    /// reports — a row the fact snapshot alone holds is not one — or its
    /// failure; no pick dropped yet. Offline there is no status read, and no
    /// instance it reports.
    pub fn online(instances: &InstancesSnapshot) -> Self {
        let instances = match instances {
            InstancesSnapshot::Online(Ok(read)) => {
                Ok(read.instances.iter().filter_map(OnlineInstance::reported).collect())
            }
            InstancesSnapshot::Online(Err(failure)) => Err(failure.clone()),
            InstancesSnapshot::Offline(_) => Ok(Vec::new()),
        };
        Self::Online { instances, dropped: None }
    }

    /// Checks the view's instance pick (`filters`) against this map, the one
    /// a status read just made: a pick it does not report — or cannot
    /// confirm, since the read failed — goes, so the view reads every
    /// instance again, and the map keeps which went, with its alias when
    /// `before`, the map it was picked from, names it. Only the online map
    /// holds a pick; offline no status read gives one, and a pick there goes
    /// too. Whether a pick went.
    pub fn check_pick(&mut self, filters: &mut Filters, before: &PortMap) -> bool {
        let Some(PortChoice::Instance(id)) = box_pick(filters) else {
            return false;
        };
        let Self::Online { instances, dropped } = self else {
            filters.instance_ids.clear();
            return true;
        };
        let reported = match instances {
            Ok(list) => list.iter().any(|instance| instance.instance_id == id),
            Err(_) => false,
        };
        if reported {
            return false;
        }
        filters.instance_ids.clear();
        *dropped = Some(DroppedPick { instance_id: id, alias: before.alias_of(&id) });
        true
    }

    /// A person picked in the box: what a status read took away before is
    /// said no more.
    pub fn forget_dropped(&mut self) {
        if let Self::Online { dropped, .. } = self {
            *dropped = None;
        }
    }

    /// The alias the online list gives `id`, when it lists it.
    fn alias_of(&self, id: &InstanceId) -> Option<String> {
        let Self::Online { instances: Ok(list), .. } = self else {
            return None;
        };
        let instance = list.iter().find(|instance| instance.instance_id == *id)?;
        Some(instance.alias.clone())
    }

    pub fn bindings(&self) -> Option<&PortBindings> {
        match self {
            Self::Read(bindings) => Some(bindings),
            Self::Opening | Self::Online { .. } | Self::Unopened | Self::Failed(_) => None,
        }
    }

    pub fn entry(&self, port: u16) -> Option<&PortEntry> {
        self.bindings()?.ports.iter().find(|entry| entry.port == port)
    }
}

impl OnlineInstance {
    /// An instance the status read registers; `None` for a row the fact
    /// snapshot alone holds.
    fn reported(instance: &RuntimeInstance) -> Option<Self> {
        let live = instance.status.as_ref()?;
        Some(Self { instance_id: live.id, alias: live.alias.clone(), adb_port: live.adb_port })
    }
}

/// The events read for the last frame the pane had to look up, by the frame's
/// id as `code` writes it: asked once, and only when the loaded rows hold no
/// capture of it. A failed read stays until another frame is looked up, or a
/// reload clears it to ask again: it is never asked again by itself.
pub enum FrameEventsSlot {
    Empty,
    /// Asked of the reader, not answered yet.
    Pending { frame: String },
    /// Answered: the positions the lookup read, `from` to `to` (online; offline
    /// it reads by frame id alone, `None`), and what it found there.
    Read {
        frame: String,
        searched: Option<(u64, u64)>,
        events: Result<Vec<ProjectedEvent>, FrameLookupError>,
    },
}

impl FrameEventsSlot {
    /// The read this slot holds for `frame`, if it holds that frame's.
    pub fn of(&self, frame: &str) -> Option<&Result<Vec<ProjectedEvent>, FrameLookupError>> {
        match self {
            Self::Read { frame: held, events, .. } if held.as_str() == frame => Some(events),
            _ => None,
        }
    }

    /// The positions the read this slot holds for `frame` looked in, when it
    /// looked in a window of them.
    pub fn searched(&self, frame: &str) -> Option<(u64, u64)> {
        match self {
            Self::Read { frame: held, searched, .. } if held.as_str() == frame => *searched,
            _ => None,
        }
    }

    /// Whether `frame`'s events are asked of the reader and not answered yet.
    pub fn pending(&self, frame: &str) -> bool {
        matches!(self, Self::Pending { frame: held } if held.as_str() == frame)
    }

    /// Whether this slot is asked or answered for `frame`.
    pub fn holds(&self, frame: &str) -> bool {
        self.pending(frame) || self.of(frame).is_some()
    }

    /// Whether the read this slot holds failed.
    pub fn failed(&self) -> bool {
        matches!(self, Self::Read { events: Err(_), .. })
    }
}

/// Why a frame's own events could not be read: put into words when painted
/// (`paint`).
#[derive(Clone)]
pub enum FrameLookupError {
    /// A page of the read says its source is incomplete, so it cannot say the
    /// capture is absent.
    Incomplete,
    /// The read face's own words, and the outcome code beneath them when the
    /// error carries one: the console's own (a window past its page limit), or
    /// the Runtime's code of a page it did not answer.
    Read { text: String, code: Option<&'static str> },
}

/// What the reader is reading, since when, and about how long a read of that
/// kind may take at most: the busy line says how long it has waited, so a slow
/// page does not look like a window that hangs.
pub struct Busy {
    pub kind: BusyKind,
    /// When this read started.
    pub since: Instant,
    /// When the request it belongs to started: the busy line shows only once
    /// that has run a while, so a quick request does not flash it.
    pub started: Instant,
    /// Seconds; per page when `per_page`.
    pub bound: u64,
    pub per_page: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum BusyKind {
    Opening,
    /// The offline port map, read once at the pin.
    Bindings,
    Span,
    /// One point read of the time cursor's locate, of at most `of`.
    Locating { read: usize, of: usize },
    /// One window of a fill, of at most `of`.
    Reading { read: usize, of: usize },
    Repinning,
    Polling,
    Instances,
    FrameEvents,
    /// The material connection rebuilt after a frame read latched it.
    Reconnecting,
}

impl Busy {
    pub fn new(kind: BusyKind, bound: u64, per_page: bool) -> Self {
        let now = Instant::now();
        Self { kind, since: now, started: now, bound, per_page }
    }
}

/// What a read leaves for the event loop, done at the next paint: what it
/// changed belongs to the window, not to the reader.
#[derive(Default)]
pub struct Effects {
    /// The view starts over from a pin just moved: the time cursor's handle
    /// goes back to the top.
    pub cursor_top: bool,
    /// The view started over: the frame pane's request is dropped, so the next
    /// paint of a frame asks again.
    pub forget_frame: bool,
    /// A follow read failed, or a fill's read did (a page the Runtime refused
    /// included): the ticks stop and the box is unticked, its reason kept on
    /// screen, so no tick asks a page again by itself.
    pub follow_halted: bool,
    /// The status was read again after a person's resume (`resume`), and
    /// what that read found: the resume line says it, never a read that
    /// failed as one that was made.
    pub status_reread: Option<StatusReread>,
}

/// What the status read after a person's resume found.
pub enum StatusReread {
    /// It was read: the pause lines are this read's.
    Read,
    /// It was not read, for the client's failure: whether scheduling is
    /// paused is unknown.
    Failed(ClientFailure),
    /// The session reads offline: there is no status to read.
    Offline,
}

impl LedgerState {
    /// A window whose reader is opening the read face of `root` for `tag`'s
    /// session.
    pub fn opening(root: PathBuf, tag: RequestTag) -> Self {
        Self {
            tag,
            phase: Phase::Opening { root },
            model: None,
            port_map: PortMap::Opening,
            frame_events: FrameEventsSlot::Empty,
            busy: None,
            reader: None,
            effects: Effects::default(),
        }
    }

    /// The session reads nothing more, for `failure`. The view model stays, so
    /// a reopen starts from the same view.
    pub fn fail(&mut self, failure: LedgerFailure) {
        let root = self.state_root().to_path_buf();
        self.phase = Phase::Failed { root, failure };
        self.port_map = PortMap::Unopened;
        self.busy = None;
        self.reader = None;
    }

    /// The view model, while the read face is open.
    pub fn live(&self) -> Option<&ViewModel> {
        if matches!(self.phase, Phase::Open(_)) {
            self.model.as_ref()
        } else {
            None
        }
    }

    pub fn live_mut(&mut self) -> Option<&mut ViewModel> {
        if matches!(self.phase, Phase::Open(_)) {
            self.model.as_mut()
        } else {
            None
        }
    }

    /// The reader starts `busy`: within one request, which keeps the time it
    /// started; the reader clears it when the request is over.
    pub fn set_busy(&mut self, busy: Busy) {
        let started = self.busy.as_ref().map_or(busy.started, |held| held.started);
        self.busy = Some(Busy { started, ..busy });
    }

    pub fn state_root(&self) -> &Path {
        match &self.phase {
            Phase::Open(facts) => &facts.state_root,
            Phase::Opening { root }
            | Phase::Unopened { root, .. }
            | Phase::OfflineOffered { root, .. }
            | Phase::Failed { root, .. } => root,
        }
    }

    /// Why the window reads offline, if it does: an unopened face is the
    /// offline one; or why it reads nothing, when `auto` could not read online.
    pub fn offline_reason(&self) -> Option<&OfflineReason> {
        match &self.phase {
            Phase::Open(facts) => facts.offline_reason.as_ref(),
            Phase::Unopened { reason, .. } | Phase::OfflineOffered { reason, .. } => Some(reason),
            Phase::Opening { .. } | Phase::Failed { .. } => None,
        }
    }

    pub fn port_bindings(&self) -> Option<&PortBindings> {
        self.port_map.bindings()
    }
}
