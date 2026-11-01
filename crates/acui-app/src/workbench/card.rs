// SPDX-License-Identifier: GPL-3.0-only
//! What one account's card says, as a pure function of what the reads left: no
//! words and no Slint here, `paint` makes both of a view.
//!
//! The first case that applies is the card's main line. A fence is never lost: when
//! it is not the main line, it is in the small print. A card never says nothing:
//! the last cases cover every account the reads name.
//!
//! `fault` and `held` are not colours of a card yet: nothing the reads carry says a
//! self-check failed or an emulator closed unexpectedly, and a player's hold is
//! UI-3d's. So a card is green, amber or grey.

use std::collections::BTreeMap;

use acui_source::workbench::{
    ActivityFact, ActivityKind, FenceFact, InstanceExplain, InstanceLive,
};
use acui_source::{PauseFact, PauseStage};

use super::model::Link;
use crate::ledger::resume::{lifter, Lifter};

pub struct CardView {
    pub alias: String,
    /// Absent when the host runs without policy inputs, and for an account no
    /// Runtime answers about.
    pub game: Option<String>,
    pub band: Band,
    pub main: Line,
    pub notes: Vec<Line>,
}

/// The colour of a card's band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Ok,
    Attention,
    Offline,
}

/// Why no `Status` answers now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Down {
    NotYet,
    NotRunning,
    NoAnswer,
    Trouble,
    NoThread,
}

impl Down {
    /// `None` while the Runtime answers.
    pub fn of(link: &Link) -> Option<Self> {
        match link {
            Link::Online { .. } => None,
            Link::Unknown => Some(Self::NotYet),
            Link::Absent(_) => Some(Self::NotRunning),
            Link::NoAnswer(_) => Some(Self::NoAnswer),
            Link::Trouble(_) => Some(Self::Trouble),
            Link::ThreadFailed => Some(Self::NoThread),
        }
    }
}

/// What the schedule explanation reads leave for the cards now.
#[derive(Clone, Copy)]
pub enum Schedule<'a> {
    /// The read failed, no thread reads it, or it was not read again for too long.
    Unreadable,
    /// None read of this Runtime yet, or it evaluated nothing since it started.
    NotEvaluated,
    /// The rows of its latest evaluation, by instance alias.
    Evaluated(&'a BTreeMap<String, InstanceExplain>),
}

impl<'a> Schedule<'a> {
    fn row(self, alias: &str) -> Option<&'a InstanceExplain> {
        match self {
            Self::Evaluated(rows) => rows.get(alias),
            Self::Unreadable | Self::NotEvaluated => None,
        }
    }
}

/// Which sentence a not-running row's reason code gets. Several codes share one;
/// the code itself is never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonClass {
    /// A pause or a fence. `Status` says those: a card that gets as far as this
    /// row has neither, so the explanation is older than the `Status` read.
    ControlGate,
    Held,
    Unavailable,
    DeviceBusy,
    Quiet,
    QuietSoon,
    UpdateWait,
    PackProblem,
    Closed,
    Paced,
    Queue,
    RemedyWait,
    NeedsChoice,
    Suspended,
    BenefitUnknown,
    BenefitNonpositive,
    NoProgress,
    NoTask,
    /// No sentence of its own: the code's category speaks, and the log hears of it.
    Unlisted,
}

impl ReasonClass {
    fn of(why: &str) -> Self {
        match why {
            "dispatch_paused_instance" | "dispatch_paused_global" | "dispatch_resource_fenced" => {
                Self::ControlGate
            }
            "dispatch_player_held" => Self::Held,
            "instance_unavailable" => Self::Unavailable,
            "dispatch_instance_held" | "dispatch_instance_cooldown" => Self::DeviceBusy,
            "blackout_active" => Self::Quiet,
            "blackout_approaching" | "policy_day_boundary_approaching" => Self::QuietSoon,
            "application_update_pending" => Self::UpdateWait,
            "pack_registry_not_loaded" => Self::PackProblem,
            "pack_schedule_closed" => Self::Closed,
            "policy_rerun_waiting"
            | "policy_cost_commit_waiting"
            | "dispatch_random_delay_waiting" => Self::Paced,
            "policy_concurrent_runs_deferred"
            | "heavy_scene_budget_deferred"
            | "heavy_scene_budget_exhausted"
            | "instance_already_selected"
            | "policy_chain_handover_waiting" => Self::Queue,
            "policy_remedy_waiting" => Self::RemedyWait,
            "policy_remedy_unchosen" | "policy_remedy_unavailable" | "policy_remedy_exhausted" => {
                Self::NeedsChoice
            }
            "pack_admission_suspended" => Self::Suspended,
            "benefit_unknown" => Self::BenefitUnknown,
            "policy_benefit_nonpositive" => Self::BenefitNonpositive,
            "policy_no_progress" => Self::NoProgress,
            "due_task_missing" => Self::NoTask,
            other if other.starts_with("perf_") => Self::Paced,
            _ => Self::Unlisted,
        }
    }

    /// A person is the one to act, or is the reason.
    fn band(self) -> Band {
        match self {
            Self::Held | Self::NeedsChoice | Self::Suspended => Band::Attention,
            _ => Band::Ok,
        }
    }
}

/// One line of a card, before its words. `package` is an activity's package id; a
/// missing one has words of its own.
pub enum Line {
    Down(Down),
    /// An instance pause, its runs still draining.
    Draining { package: Option<String> },
    /// An instance pause, its device being handed back.
    HandingBack,
    /// A pause in full effect: an instance's handed back, or the global one
    /// (`everyone`).
    Paused { everyone: bool, by: Lifter, reason: String },
    Running { package: Option<String> },
    /// A dispatched run that goes on under a global pause, which only lets it end.
    Finishing { package: Option<String> },
    HostRun,
    Reconnecting,
    SelfChecking,
    ControllingEmulator,
    Climbing,
    /// A fence that waits for a person; `cause` is the failure's code.
    AwaitingPerson { cause: String },
    /// A fence the recovery ladder still works on.
    Recovering,
    /// The recovery ladder holds instead of climbing.
    LadderHold,
    /// Why it does not run cannot be told now: the one line said as a failure.
    ScheduleUnreadable,
    /// A task is selected and its dispatch has not ended.
    Preparing,
    /// A not-running row: `why` is its code, for the log of an `Unlisted` one, and
    /// `wake` that row's own time.
    Reason { class: ReasonClass, why: String, wake: Option<u64> },
    NotEvaluated,
    /// The latest evaluation decided nothing for this account.
    NotPlanned,
}

/// An account no Runtime answers about now: grey, with nothing an earlier answer
/// said.
pub fn down_view(alias: &str, down: Down) -> CardView {
    CardView {
        alias: alias.to_owned(),
        game: None,
        band: Band::Offline,
        main: Line::Down(down),
        notes: Vec::new(),
    }
}

/// An account of the newest `Status`, that read's global pause, and what the
/// schedule explanation reads leave.
pub fn card_view(
    instance: &InstanceLive,
    global: Option<&PauseFact>,
    schedule: Schedule<'_>,
) -> CardView {
    let (band, main, notes) = said(instance, global, schedule);
    CardView { alias: instance.alias.clone(), game: instance.game_id.clone(), band, main, notes }
}

/// The band, the main line and the small print: the first case that applies.
fn said(
    instance: &InstanceLive,
    global: Option<&PauseFact>,
    schedule: Schedule<'_>,
) -> (Band, Line, Vec<Line>) {
    let row = schedule.row(&instance.alias);
    let fence = instance.fence.as_ref().map(fence_line);
    let doing = instance.activity.as_ref().map(|activity| doing_line(activity, global.is_some()));
    if let Some(pause) = &instance.pause {
        match pause.stage {
            Some(PauseStage::Draining) => {
                let package =
                    instance.activity.as_ref().and_then(|activity| activity.package_id.clone());
                return (Band::Attention, Line::Draining { package }, fence.into_iter().collect());
            }
            Some(PauseStage::Paused) => {
                return (Band::Attention, Line::HandingBack, fence.into_iter().collect());
            }
            Some(PauseStage::Released) | None => {}
        }
    }
    // The candidate stopped at placement for the fence: lifting a pause would not
    // run it either, so the fence is what the person has to know first.
    let fence_blocks = row.is_some_and(|row| {
        matches!(row.why.as_str(), "instance_unavailable" | "dispatch_resource_fenced")
    });
    if fence_blocks && doing.is_none() {
        if let Some(fence) = fence {
            let paused = instance
                .pause
                .as_ref()
                .map(|pause| paused_line(pause, false))
                .or_else(|| global.map(|pause| paused_line(pause, true)));
            return (Band::Attention, fence, paused.into_iter().collect());
        }
    }
    if let Some(pause) = &instance.pause {
        let notes = doing.into_iter().chain(fence).collect();
        return (Band::Attention, paused_line(pause, false), notes);
    }
    if let Some(doing) = doing {
        let band = if global.is_some() || fence.is_some() { Band::Attention } else { Band::Ok };
        let global = global.map(|pause| paused_line(pause, true));
        return (band, doing, global.into_iter().chain(fence).collect());
    }
    if let Some(pause) = global {
        return (Band::Attention, paused_line(pause, true), fence.into_iter().collect());
    }
    // With no pause a fence always shows, whatever the explanation says or fails to.
    if let Some(fence) = fence {
        let why = row.filter(|row| ReasonClass::of(&row.why) != ReasonClass::ControlGate);
        return (Band::Attention, fence, why.map(reason_line).into_iter().collect());
    }
    if instance.ladder_busy {
        return (Band::Ok, Line::LadderHold, Vec::new());
    }
    let (band, main) = match schedule {
        Schedule::Unreadable => (Band::Ok, Line::ScheduleUnreadable),
        Schedule::NotEvaluated => (Band::Ok, Line::NotEvaluated),
        Schedule::Evaluated(_) => match row {
            Some(row) if row.preparing => (Band::Ok, Line::Preparing),
            Some(row) => (ReasonClass::of(&row.why).band(), reason_line(row)),
            None => (Band::Ok, Line::NotPlanned),
        },
    };
    (band, main, Vec::new())
}

/// A row's reason. A control gate this far down is an old row (`ControlGate`), and
/// its wake with it.
fn reason_line(row: &InstanceExplain) -> Line {
    let class = ReasonClass::of(&row.why);
    let wake = if class == ReasonClass::ControlGate { None } else { row.next_wake };
    Line::Reason { class, why: row.why.clone(), wake }
}

fn fence_line(fence: &FenceFact) -> Line {
    if fence.awaiting_person {
        Line::AwaitingPerson { cause: fence.cause.clone() }
    } else {
        Line::Recovering
    }
}

fn paused_line(pause: &PauseFact, everyone: bool) -> Line {
    Line::Paused { everyone, by: lifter(&pause.reason), reason: pause.reason.clone() }
}

/// The one activity the Runtime names; which of several an instance shows is the
/// Runtime's to choose.
fn doing_line(activity: &ActivityFact, global_pause: bool) -> Line {
    let package = activity.package_id.clone();
    match activity.kind {
        ActivityKind::PolicyRun if global_pause => Line::Finishing { package },
        ActivityKind::PolicyRun | ActivityKind::DirectRun => Line::Running { package },
        ActivityKind::HostRun => Line::HostRun,
        ActivityKind::Reconnect => Line::Reconnecting,
        ActivityKind::Selfcheck => Line::SelfChecking,
        ActivityKind::EmulatorControl => Line::ControllingEmulator,
        ActivityKind::Ladder => Line::Climbing,
    }
}
