// SPDX-License-Identifier: GPL-3.0-only
//! What the ledger window shows, held apart from the read face it was read
//! through. The paint (`paint`) reads this and nothing else, so it never waits
//! on a read: the reads leave the view model, the port map and the selected
//! frame's own events here, and copy in what they say about the read face
//! itself (`SourceFacts`) before every paint.

use std::path::{Path, PathBuf};

use acui_model::ViewModel;
use acui_rows::{PortBindings, PortEntry, ProjectedEvent};
use acui_source::{
    ClientFailure, InstanceFacts, LedgerOpenFailure, OfflineFacts, OfflineReason, ReadSource,
    RuntimeInstances,
};

use crate::strings::Labels;

/// One ledger window's view: whether its read face is open and what it says,
/// the view model when it is, the port map, and the selected frame's own
/// events when they had to be read.
pub struct LedgerState {
    pub phase: Phase,
    /// `None` while the read face is unopened: there is no page to model.
    pub model: Option<ViewModel>,
    /// Instance identity by ADB port, read once at the pinned snapshot. The
    /// port box's items come from it at every paint (`paint::port_options`),
    /// in the language shown then.
    pub port_map: PortMap,
    pub frame_events: FrameEventsSlot,
}

/// Whether the window's read face is open.
pub enum Phase {
    /// Open, with what the paint takes from it, as last copied out.
    Open(SourceFacts),
    /// The offline face the ledger refused to open, with its own error.
    /// Nothing was read: no page, no port map, no material.
    Unopened { root: PathBuf, reason: OfflineReason, failure: LedgerOpenFailure },
}

/// What the paint takes from an open read face: where it reads, which face
/// answered and why, whether it can follow, and the instances as last read.
/// The material reader is not among them: the paint asks the caller for one
/// only when it starts a new frame read (`paint::refresh`).
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
    /// Online: the Runtime does not derive bindings through its page.
    Online,
    /// The read face is unopened: nothing was read.
    Unopened,
    /// The read face refused, with its typed code as the outermost context.
    Failed(anyhow::Error),
    Read(PortBindings),
}

/// The events read for the last frame the pane had to look up, by the frame's
/// id as `code` writes it: read once, and only when the loaded rows hold no
/// capture of it. A failed read stays until another frame is looked up, or a
/// reload clears it to ask again.
pub enum FrameEventsSlot {
    Empty,
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

impl LedgerState {
    /// An open read face, its first view model and its port map.
    pub fn open(facts: SourceFacts, model: ViewModel, port_map: PortMap) -> Self {
        Self {
            phase: Phase::Open(facts),
            model: Some(model),
            port_map,
            frame_events: FrameEventsSlot::Empty,
        }
    }

    /// An offline face the ledger refused to open.
    pub fn unopened(root: PathBuf, reason: OfflineReason, failure: LedgerOpenFailure) -> Self {
        Self {
            phase: Phase::Unopened { root, reason, failure },
            model: None,
            port_map: PortMap::Unopened,
            frame_events: FrameEventsSlot::Empty,
        }
    }

    pub fn state_root(&self) -> &Path {
        match &self.phase {
            Phase::Open(facts) => &facts.state_root,
            Phase::Unopened { root, .. } => root,
        }
    }

    /// Why the window reads offline, if it does: an unopened face is the
    /// offline one.
    pub fn offline_reason(&self) -> Option<&OfflineReason> {
        match &self.phase {
            Phase::Open(facts) => facts.offline_reason.as_ref(),
            Phase::Unopened { reason, .. } => Some(reason),
        }
    }

    pub fn port_bindings(&self) -> Option<&PortBindings> {
        match &self.port_map {
            PortMap::Read(bindings) => Some(bindings),
            PortMap::Online | PortMap::Unopened | PortMap::Failed(_) => None,
        }
    }

    pub fn port_entry(&self, port: u16) -> Option<&PortEntry> {
        self.port_bindings()?.ports.iter().find(|entry| entry.port == port)
    }
}
