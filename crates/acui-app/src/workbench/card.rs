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
//!
//! Whether a card is the one picked, whose picture the right column shows, is no read's:
//! the caller says (`picture`); nor is the card's own button, of its account's pause, which the
//! presses leave as well (`actions::account_view`).

use std::collections::BTreeMap;

use acui_source::workbench::{
    ActivityFact, ActivityKind, FenceFact, InstanceExplain, InstanceLive,
};
use acui_source::{PauseFact, PauseStage};

use super::actions::AccountView;
use super::model::Link;
use crate::ledger::resume::{lifter, Lifter, ResumeElsewhere};

pub struct CardView {
    pub alias: String,
    /// Absent when the host runs without policy inputs, and for an account no
    /// Runtime answers about.
    pub game: Option<String>,
    /// The server its last admitted package declared, as it wrote it, when the newest
    /// fact snapshot has one; else the one a snapshot of this owner epoch last had, if any
    /// did (`server_last_seen`).
    pub server: Option<String>,
    /// `server` is not the newest snapshot's but remembered from an earlier one of this
    /// owner epoch: the card says so after it.
    pub server_last_seen: bool,
    pub band: Band,
    pub main: Line,
    pub notes: Vec<Line>,
    /// Its picture is the one the right column shows.
    pub selected: bool,
    /// Its own button, and what its last press came to.
    pub account: AccountView,
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
    /// Root mode: the Runtime up is not the installation folder's, and nothing of it is read.
    NotThisRoot,
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
            Link::NotThisRoot => Some(Self::NotThisRoot),
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

/// Which sentence a not-running row's reason code gets, the card's and the daily task list's
/// alike (UI-3f1): one class, one sentence. Several codes share one; the code itself is never
/// shown. Two values of a reason choose too: a `benefit_unknown`'s `benefit_gap`, and a
/// `pool_line_held`'s `pool_line_hold`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonClass {
    /// A pause or a fence. `Status` says those: a card that gets as far as this
    /// row has neither, so the explanation is older than the `Status` read. The task
    /// list says the pause above its rows.
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
    /// Another task of the account is selected this round.
    OtherFirst,
    /// The step before it in its chain is still out.
    ChainWait,
    RemedyWait,
    NeedsChoice,
    Suspended,
    /// Its benefit is unknown, and the reason names no gap this console has words for.
    BenefitUnknown,
    /// The pack declares no effect of the task; one of its entries is pending; a pool it
    /// touches has no value; how much of a pool there is now is not known, or of a capped
    /// pool, its capacity or a line.
    NoEffects,
    EntryPending,
    Unvalued,
    ReadingUnknown,
    BenefitNonpositive,
    NoProgress,
    NoTask,
    NotMet,
    NotTime,
    Cooling,
    RetryLater,
    Missed,
    /// The time a fact holds was served already.
    Served,
    /// Its feedback stop holds.
    Enough,
    FactsMissing,
    Refreshing,
    RefreshUnclear,
    RefreshStuck,
    /// A fact is missing, and no task reads it.
    RefreshUnbound,
    Locked,
    OutOfHours,
    NotCapable,
    TurnedOff,
    NoProfile,
    HostShort,
    NextRound,
    GateUnmet,
    Reserve,
    /// The pools' lines: a run would cross a hard line; at a pool's cap, a better candidate
    /// goes first; the lines are wrong, or so high that nothing spends the pool.
    HardLine,
    BetterFirst,
    LinesInvalid,
    LinesUnspendable,
    DependencyWait,
    WithinTolerance,
    Budget,
    /// The Runtime gave a decision no reason: its fault, which the log hears of.
    ReasonMissing,
    /// No sentence of its own: a general one says it is not scheduled now, and the log
    /// hears of the code once. So do `timeline_active`, `benefit_fixed_value` and a
    /// `pool_line_held` of a hold this console has no words for.
    Unlisted,
}

impl ReasonClass {
    /// The class of the reason code `why`, its `benefit_gap` `gap` and its `pool_line_hold`
    /// `hold`.
    pub(super) fn of(why: &str, gap: Option<&str>, hold: Option<&str>) -> Self {
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
            | "heavy_scene_budget_exhausted" => Self::Queue,
            "instance_already_selected" => Self::OtherFirst,
            "policy_chain_handover_waiting" => Self::ChainWait,
            "policy_remedy_waiting" => Self::RemedyWait,
            "policy_remedy_unchosen" | "policy_remedy_unavailable" | "policy_remedy_exhausted" => {
                Self::NeedsChoice
            }
            "pack_admission_suspended" | "policy_task_paused" => Self::Suspended,
            "benefit_unknown" => match gap {
                Some("no_effects") => Self::NoEffects,
                Some("entry_pending") => Self::EntryPending,
                Some("valuation_pending" | "valuation_missing") => Self::Unvalued,
                Some("reading_unknown") => Self::ReadingUnknown,
                Some(_) | None => Self::BenefitUnknown,
            },
            "policy_benefit_nonpositive" => Self::BenefitNonpositive,
            "policy_no_progress" => Self::NoProgress,
            "due_task_missing" => Self::NoTask,
            "trigger_false" => Self::NotMet,
            "fact_time_pending" => Self::NotTime,
            "task_cooldown_active" => Self::Cooling,
            "policy_retry_backoff_active" => Self::RetryLater,
            "clock_occurrence_expired" => Self::Missed,
            "fact_time_served" => Self::Served,
            "feedback_stop_true" => Self::Enough,
            "trigger_unknown"
            | "feedback_stop_unknown"
            | "selection_input_unknown"
            | "fact_time_unavailable"
            | "fact_reference_unavailable"
            | "policy_instance_fact_missing" => Self::FactsMissing,
            "fact_refresh_requested" | "fact_refresh_pending" => Self::Refreshing,
            "fact_refresh_unresolved" => Self::RefreshUnclear,
            "fact_refresh_exhausted" => Self::RefreshStuck,
            "fact_refresh_unbound" => Self::RefreshUnbound,
            "feature_locked" => Self::Locked,
            // `timeline_active` is a window that is open: no reason not to run.
            "timeline_unavailable" | "policy_activity_window_closed" => Self::OutOfHours,
            "capability_missing" => Self::NotCapable,
            "instance_override_disabled" => Self::TurnedOff,
            "activity_profile_missing" => Self::NoProfile,
            "host_capacity_insufficient" => Self::HostShort,
            "score_deferred" => Self::NextRound,
            "score_unknown" | "gate_rejected" => Self::GateUnmet,
            "resource_reserve_insufficient" => Self::Reserve,
            "pool_line_held" => match hold {
                Some("would_cross_hard_line" | "capped_would_cross_hard_line") => Self::HardLine,
                Some("capped_cap_unknown") => Self::ReadingUnknown,
                Some("capped_not_head" | "capped_not_dispatchable" | "capped_limited_spent") => {
                    Self::BetterFirst
                }
                Some("lines_invalid") => Self::LinesInvalid,
                Some(_) | None => Self::Unlisted,
            },
            // Runtime 0.12.53 on; a string on the wire, so the pinned client reads it all the same.
            "pool_lines_unspendable" => Self::LinesUnspendable,
            "dependency_already_served" => Self::DependencyWait,
            "benefit_within_tolerance" => Self::WithinTolerance,
            "policy_budget_exhausted" => Self::Budget,
            "decision_reason_missing" => Self::ReasonMissing,
            other if other.starts_with("perf_") => Self::Paced,
            _ => Self::Unlisted,
        }
    }

    /// A person is the one to act, or is the reason: amber, on a card and on the task list.
    pub(super) fn band(self) -> Band {
        match self {
            Self::Held
            | Self::NeedsChoice
            | Self::Suspended
            | Self::RefreshUnbound
            | Self::LinesInvalid
            | Self::LinesUnspendable
            | Self::ReasonMissing => Band::Attention,
            _ => Band::Ok,
        }
    }
}

/// A reason code as the log hears of it: with the value that chose its words, when one did.
pub(super) fn reason_logged(why: &str, gap: Option<&str>, hold: Option<&str>) -> String {
    match (why, gap, hold) {
        ("benefit_unknown", Some(gap), _) => format!("{why} (benefit_gap {gap})"),
        ("pool_line_held", _, Some(hold)) => format!("{why} (pool_line_hold {hold})"),
        _ => why.to_owned(),
    }
}

/// The class of an account's not-running row.
fn row_class(row: &InstanceExplain) -> ReasonClass {
    ReasonClass::of(&row.why, row.gap.as_deref(), row.hold.as_deref())
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
    /// A not-running row: `why` is its code, with the value that chose its words
    /// (`reason_logged`), for the log of an `Unlisted` one, and `wake` that row's own time.
    Reason { class: ReasonClass, why: String, wake: Option<u64> },
    NotEvaluated,
    /// The latest evaluation decided nothing for this account.
    NotPlanned,
}

/// An account no Runtime answers about now: grey, with nothing an earlier answer
/// said, but what its own button's last press came to (`account`).
pub fn down_view(alias: &str, down: Down, selected: bool, account: AccountView) -> CardView {
    CardView {
        alias: alias.to_owned(),
        game: None,
        server: None,
        server_last_seen: false,
        band: Band::Offline,
        main: Line::Down(down),
        notes: Vec::new(),
        selected,
        account,
    }
}

/// An account of the newest `Status`, its server in the newest fact snapshot or, when that
/// names none, the one last seen this owner epoch (`server_last_seen`), that read's global
/// pause, and what the schedule explanation reads leave; who lifts a pause by `elsewhere`; its
/// own button as the reads and the presses leave it (`account`).
#[allow(clippy::too_many_arguments)]
pub fn card_view(
    instance: &InstanceLive,
    server: Option<&str>,
    server_last_seen: bool,
    global: Option<&PauseFact>,
    schedule: Schedule<'_>,
    selected: bool,
    elsewhere: &ResumeElsewhere,
    account: AccountView,
) -> CardView {
    let (band, main, notes) = said(instance, global, schedule, elsewhere);
    CardView {
        alias: instance.alias.clone(),
        game: instance.game_id.clone(),
        server: server.map(str::to_owned),
        server_last_seen,
        band,
        main,
        notes,
        selected,
        account,
    }
}

/// The band, the main line and the small print: the first case that applies. The daily task
/// list says the account's pause or fence above its rows as this says it (`tasks::held`).
pub(super) fn said(
    instance: &InstanceLive,
    global: Option<&PauseFact>,
    schedule: Schedule<'_>,
    elsewhere: &ResumeElsewhere,
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
                .map(|pause| paused_line(pause, false, elsewhere))
                .or_else(|| global.map(|pause| paused_line(pause, true, elsewhere)));
            return (Band::Attention, fence, paused.into_iter().collect());
        }
    }
    if let Some(pause) = &instance.pause {
        let notes = doing.into_iter().chain(fence).collect();
        return (Band::Attention, paused_line(pause, false, elsewhere), notes);
    }
    if let Some(doing) = doing {
        let band = if global.is_some() || fence.is_some() { Band::Attention } else { Band::Ok };
        let global = global.map(|pause| paused_line(pause, true, elsewhere));
        return (band, doing, global.into_iter().chain(fence).collect());
    }
    if let Some(pause) = global {
        let main = paused_line(pause, true, elsewhere);
        return (Band::Attention, main, fence.into_iter().collect());
    }
    // With no pause a fence always shows, whatever the explanation says or fails to.
    if let Some(fence) = fence {
        let why = row.filter(|row| row_class(row) != ReasonClass::ControlGate);
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
            Some(row) => (row_class(row).band(), reason_line(row)),
            None => (Band::Ok, Line::NotPlanned),
        },
    };
    (band, main, Vec::new())
}

/// A row's reason. A control gate this far down is an old row (`ControlGate`), and
/// its wake with it.
fn reason_line(row: &InstanceExplain) -> Line {
    let class = row_class(row);
    let wake = if class == ReasonClass::ControlGate { None } else { row.next_wake };
    let why = reason_logged(&row.why, row.gap.as_deref(), row.hold.as_deref());
    Line::Reason { class, why, wake }
}

fn fence_line(fence: &FenceFact) -> Line {
    if fence.awaiting_person {
        Line::AwaitingPerson { cause: fence.cause.clone() }
    } else {
        Line::Recovering
    }
}

fn paused_line(pause: &PauseFact, everyone: bool, elsewhere: &ResumeElsewhere) -> Line {
    Line::Paused { everyone, by: lifter(pause, elsewhere), reason: pause.reason.clone() }
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
