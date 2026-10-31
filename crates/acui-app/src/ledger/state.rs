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

use acui_model::ViewModel;
use acui_rows::{PortBindings, PortEntry, ProjectedEvent};
use acui_source::{
    ClientFailure, InstanceFacts, LedgerOpenFailure, MaterialReader, OfflineFacts, OfflineReason,
    ReadSource, RuntimeInstances,
};

use crate::strings::Labels;

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
    /// Instance identity by ADB port, read once at the pinned snapshot. The
    /// port box's items come from it at every paint (`paint::port_options`),
    /// in the language shown then.
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
/// once from the binding facts through the pinned snapshot.
pub enum PortMap {
    /// The read face is still opening.
    Opening,
    /// Online: the Runtime does not derive bindings through its page.
    Online,
    /// The read face is unopened, or failed: nothing was read.
    Unopened,
    /// The read face refused, with its typed code as the outermost context.
    Failed(anyhow::Error),
    Read(PortBindings),
}

impl PortMap {
    pub fn bindings(&self) -> Option<&PortBindings> {
        match self {
            Self::Read(bindings) => Some(bindings),
            Self::Opening | Self::Online | Self::Unopened | Self::Failed(_) => None,
        }
    }

    pub fn entry(&self, port: u16) -> Option<&PortEntry> {
        self.bindings()?.ports.iter().find(|entry| entry.port == port)
    }
}

/// The events read for the last frame the pane had to look up, by the frame's
/// id as `code` writes it: asked once, and only when the loaded rows hold no
/// capture of it. A failed read stays until another frame is looked up, or a
/// reload clears it to ask again.
pub enum FrameEventsSlot {
    Empty,
    /// Asked of the reader, not answered yet.
    Pending { frame: String },
    Read { frame: String, events: Result<Vec<ProjectedEvent>, FrameLookupError> },
}

impl FrameEventsSlot {
    /// The read this slot holds for `frame`, if it holds that frame's.
    pub fn of(&self, frame: &str) -> Option<&Result<Vec<ProjectedEvent>, FrameLookupError>> {
        match self {
            Self::Read { frame: held, events } if held.as_str() == frame => Some(events),
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

/// Why a frame's own events could not be read: put into words when painted.
#[derive(Clone)]
pub enum FrameLookupError {
    /// A page of the read says its source is incomplete, so it cannot say the
    /// capture is absent.
    Incomplete,
    /// The read face's own words.
    Read(String),
}

impl FrameLookupError {
    pub fn text(&self, labels: &Labels) -> String {
        match self {
            Self::Incomplete => labels.source_incomplete.to_owned(),
            Self::Read(text) => text.clone(),
        }
    }
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
    /// The probes that make a time bound's estimated start safe.
    Probing,
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
    /// A follow read failed: the ticks stop and the box is unticked, its
    /// reason kept on screen.
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
            Phase::Opening { root } | Phase::Unopened { root, .. } | Phase::Failed { root, .. } => {
                root
            }
        }
    }

    /// Why the window reads offline, if it does: an unopened face is the
    /// offline one.
    pub fn offline_reason(&self) -> Option<&OfflineReason> {
        match &self.phase {
            Phase::Open(facts) => facts.offline_reason.as_ref(),
            Phase::Unopened { reason, .. } => Some(reason),
            Phase::Opening { .. } | Phase::Failed { .. } => None,
        }
    }

    pub fn port_bindings(&self) -> Option<&PortBindings> {
        self.port_map.bindings()
    }
}
