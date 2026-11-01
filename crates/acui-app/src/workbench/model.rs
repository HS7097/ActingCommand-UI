// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's one state (see `workbench`): what the reads left, and the rule
//! for which of them count. A read is taken only for the owner epoch `Link` names,
//! and only a `Status` read changes that epoch: a slice read before a restart that
//! lands after it says nothing, and a slice read of a new Runtime that lands before
//! `Status` saw it is read again, since `Status` seeing it pokes the slow thread.

use std::mem;
use std::time::Instant;

use acui_source::workbench::{ExplainRead, InstanceLive, LiveRead, LiveSnapshot};
use acui_source::{ClientFailure, PauseFact};

use super::read::Lane;
use crate::strings::{fill, Labels};

/// The Runtime as the newest `Status` read found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// No read has come back yet.
    Unknown,
    Online { pid: u32, epoch: String },
    /// No Runtime to reach: not running.
    Absent(ClientFailure),
    /// A Runtime is there and did not answer: stopping, or busy for now.
    NoAnswer(ClientFailure),
    /// Something is wrong rather than merely down.
    Trouble(ClientFailure),
    /// The thread that reads `Status` did not start, or is gone.
    ThreadFailed,
}

/// What the slow thread read of one kind: the last value taken, when it was, and
/// the failure of the reads since, if they failed.
pub struct Slice<T> {
    pub value: Option<T>,
    pub read_at: Option<Instant>,
    pub error: Option<ClientFailure>,
}

impl<T> Slice<T> {
    fn empty() -> Self {
        Self { value: None, read_at: None, error: None }
    }
}

/// A reading thread that does not do its work; said in red until it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Its last read panicked: the message, for the log. The next read tries again.
    Panicked(String),
    /// It did not start, or is gone: why.
    NoThread(String),
}

impl Fault {
    pub fn text(&self, labels: &Labels) -> String {
        match self {
            Self::Panicked(_) => labels.read_panicked.to_owned(),
            Self::NoThread(error) => fill(labels.thread_failed, &[error]),
        }
    }
}

pub struct Model {
    link: Link,
    /// As the newest `Status` listed them; none while `link` is not `Online`:
    /// nothing of an earlier answer poses as the present.
    instances: Vec<InstanceLive>,
    global_pause: Option<PauseFact>,
    /// Every alias this run has seen, first seen first. Kept across restarts: an
    /// account seen stays on the workbench, in grey, while no Runtime answers.
    seen_aliases: Vec<String>,
    explain: Slice<ExplainRead>,
    /// The owner epoch last online. A Runtime that comes back with the same one
    /// has not restarted, so what was read of it stands.
    epoch: Option<String>,
    live_fault: Option<Fault>,
    slow_fault: Option<Fault>,
}

/// What a `Status` read changed, for what the event loop does next.
pub struct LiveLanded {
    pub previous: Link,
    /// `link` became `Online`.
    pub connected: bool,
    /// `link` names another owner epoch than the last one online, or the first.
    pub epoch_changed: bool,
    /// The thread's fault is over.
    pub fault_cleared: bool,
}

/// What a schedule explanation read changed, for what the event loop does next.
pub struct ExplainLanded {
    pub news: ExplainNews,
    /// The thread's fault is over.
    pub fault_cleared: bool,
}

/// What the log gets about a schedule explanation read: only a change, so a
/// failure that repeats is one line, and its end another.
pub enum ExplainNews {
    Nothing,
    Failed(ClientFailure),
    Recovered,
}

impl Model {
    pub fn new() -> Self {
        Self {
            link: Link::Unknown,
            instances: Vec::new(),
            global_pause: None,
            seen_aliases: Vec::new(),
            explain: Slice::empty(),
            epoch: None,
            live_fault: None,
            slow_fault: None,
        }
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    pub fn faults(&self) -> impl Iterator<Item = &Fault> {
        self.live_fault.iter().chain(self.slow_fault.iter())
    }

    /// One `Status` read: a new owner epoch lets go of everything read of the old
    /// one, but not of the aliases seen.
    pub fn apply_live(&mut self, read: LiveRead) -> LiveLanded {
        let fault_cleared = self.live_fault.take().is_some();
        let down = match read {
            LiveRead::Read(snapshot) => return self.online(*snapshot, fault_cleared),
            LiveRead::NotConnected(failure) => Link::Absent(failure),
            LiveRead::NoAnswer(failure) => Link::NoAnswer(failure),
            LiveRead::Trouble(failure) => Link::Trouble(failure),
        };
        self.leave_online();
        let previous = mem::replace(&mut self.link, down);
        LiveLanded { previous, connected: false, epoch_changed: false, fault_cleared }
    }

    fn online(&mut self, snapshot: LiveSnapshot, fault_cleared: bool) -> LiveLanded {
        let LiveSnapshot { pid, epoch, global_pause, instances } = snapshot;
        let epoch_changed = self.epoch.as_deref() != Some(epoch.as_str());
        if epoch_changed {
            self.explain = Slice::empty();
            self.epoch = Some(epoch.clone());
        }
        for instance in &instances {
            if !self.seen_aliases.contains(&instance.alias) {
                self.seen_aliases.push(instance.alias.clone());
            }
        }
        self.instances = instances;
        self.global_pause = global_pause;
        let previous = mem::replace(&mut self.link, Link::Online { pid, epoch });
        let connected = !matches!(previous, Link::Online { .. });
        LiveLanded { previous, connected, epoch_changed, fault_cleared }
    }

    /// One schedule explanation read. With no epoch to hold it to, a read says
    /// nothing, and a failure neither: `Status` already said why. A failure names
    /// no epoch: one from before a restart is taken for now, and the read that
    /// the restart pokes settles it next.
    pub fn apply_explain(
        &mut self,
        read: Result<ExplainRead, ClientFailure>,
        now: Instant,
    ) -> ExplainLanded {
        let fault_cleared = self.slow_fault.take().is_some();
        let Link::Online { epoch, .. } = &self.link else {
            return ExplainLanded { news: ExplainNews::Nothing, fault_cleared };
        };
        let news = match read {
            Ok(explain) if explain.epoch != *epoch => ExplainNews::Nothing,
            Ok(explain) => {
                let recovered = self.explain.error.take().is_some();
                self.explain.value = Some(explain);
                self.explain.read_at = Some(now);
                if recovered {
                    ExplainNews::Recovered
                } else {
                    ExplainNews::Nothing
                }
            }
            Err(failure) if self.explain.error.as_ref() == Some(&failure) => ExplainNews::Nothing,
            Err(failure) => {
                self.explain.error = Some(failure.clone());
                ExplainNews::Failed(failure)
            }
        };
        ExplainLanded { news, fault_cleared }
    }

    /// `lane`'s read panicked; whether that is news for the log and the window.
    pub fn read_panicked(&mut self, lane: Lane, message: String) -> bool {
        let fault = Some(Fault::Panicked(message));
        let slot = self.fault_slot(lane);
        let news = *slot != fault;
        *slot = fault;
        news
    }

    /// `lane`'s thread did not start, or is gone.
    pub fn lane_lost(&mut self, lane: Lane, error: String) {
        *self.fault_slot(lane) = Some(Fault::NoThread(error));
        if lane == Lane::Live {
            self.leave_online();
            self.link = Link::ThreadFailed;
        }
    }

    fn fault_slot(&mut self, lane: Lane) -> &mut Option<Fault> {
        match lane {
            Lane::Live => &mut self.live_fault,
            Lane::Slow => &mut self.slow_fault,
        }
    }

    /// What is shown of a Runtime that no longer answers. The last value read
    /// stays, for the same epoch coming back.
    fn leave_online(&mut self) {
        self.instances.clear();
        self.global_pause = None;
        self.explain.error = None;
    }
}
