// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's one state (see `workbench`): what the reads left, and the rule
//! for which of them count. A read is taken only for the owner epoch `Link` names,
//! and only a `Status` read changes that epoch: a slice read before a restart that
//! lands after it says nothing, and a slice read of a new Runtime that lands before
//! `Status` saw it is read again, since `Status` seeing it pokes the slow thread.

use std::collections::BTreeSet;
use std::mem;
use std::time::Instant;

use acui_source::workbench::{ExplainRead, InstanceLive, LiveRead, LiveSnapshot};
use acui_source::{ClientFailure, PauseFact};

use super::card::{self, CardView, Down, Schedule};
use super::read::{Lane, EXPLAIN_TOO_OLD};
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

/// Why the left column has no account to show, when no read has left one there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountsNote {
    Nothing,
    /// The Runtime answers, and its configuration has none.
    NoneConfigured,
    /// No Runtime runs, and none was seen this run.
    NotRunning,
    /// The Runtime cannot be read, and none was seen this run.
    Unreachable,
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
    /// When the slow thread was last told to read now: no answer can be asked of
    /// it sooner, so a long gap before that says nothing against the explanation.
    explain_asked_at: Option<Instant>,
    /// Whether the workbench is not minimized. The slow thread does not read while
    /// it is, so no read is late then.
    shown: bool,
    /// The owner epoch last online. A Runtime that comes back with the same one
    /// has not restarted, so what was read of it stands.
    epoch: Option<String>,
    live_fault: Option<Fault>,
    slow_fault: Option<Fault>,
    /// The lines the log got about codes the cards have no words for: once each.
    told_codes: BTreeSet<String>,
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
            explain_asked_at: None,
            shown: true,
            epoch: None,
            live_fault: None,
            slow_fault: None,
            told_codes: BTreeSet::new(),
        }
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    pub fn faults(&self) -> impl Iterator<Item = &Fault> {
        self.live_fault.iter().chain(self.slow_fault.iter())
    }

    pub fn accounts_note(&self) -> AccountsNote {
        let unseen = self.seen_aliases.is_empty();
        match &self.link {
            Link::Unknown => AccountsNote::Nothing,
            Link::Online { .. } if self.instances.is_empty() => AccountsNote::NoneConfigured,
            Link::Online { .. } => AccountsNote::Nothing,
            Link::Absent(_) if unseen => AccountsNote::NotRunning,
            Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed if unseen => {
                AccountsNote::Unreachable
            }
            Link::Absent(_) | Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed => {
                AccountsNote::Nothing
            }
        }
    }

    /// The accounts of the left column: as `Status` listed them while the Runtime
    /// answers; otherwise every alias seen this run, with nothing an earlier answer
    /// said of it.
    pub fn cards(&self, now: Instant) -> Vec<CardView> {
        match Down::of(&self.link) {
            None => {
                let schedule = self.schedule(now);
                self.instances
                    .iter()
                    .map(|instance| {
                        card::card_view(instance, self.global_pause.as_ref(), schedule)
                    })
                    .collect()
            }
            Some(down) => {
                self.seen_aliases.iter().map(|alias| card::down_view(alias, down)).collect()
            }
        }
    }

    /// What the schedule explanation reads leave now. A thread that does not read
    /// counts as a read that failed: with no value and no failure the card would
    /// otherwise say there is no explanation yet.
    fn schedule(&self, now: Instant) -> Schedule<'_> {
        if self.explain.error.is_some() || self.slow_fault.is_some() || self.explain_late(now) {
            return Schedule::Unreadable;
        }
        match &self.explain.value {
            Some(read) if read.evaluated => Schedule::Evaluated(&read.rows),
            Some(_) | None => Schedule::NotEvaluated,
        }
    }

    fn explain_late(&self, now: Instant) -> bool {
        let since = self.explain.read_at.max(self.explain_asked_at);
        self.shown
            && since.is_some_and(|since| now.saturating_duration_since(since) > EXPLAIN_TOO_OLD)
    }

    /// The slow thread was told to read now.
    pub fn explain_asked(&mut self, now: Instant) {
        self.explain_asked_at = Some(now);
    }

    /// Whether the workbench is not minimized, as last looked at.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// Whether the log has yet to hear `line`; it has from now on.
    pub fn first_told(&mut self, line: &str) -> bool {
        self.told_codes.insert(line.to_owned())
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
    /// no epoch: one of a read that was out across a restart is taken for now, and
    /// the read that the restart poked, queued behind it, settles it next. That read
    /// does not itself fail for finding its connection over, since the lane asks
    /// again on a new one (`ReadLane::read_explain`).
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
