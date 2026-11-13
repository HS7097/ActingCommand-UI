// SPDX-License-Identifier: GPL-3.0-only
//! A person's stop of one run of an account, from the rescue bar (UI-3e2, UI-3e3, UI-3e2b): "stop
//! this run" of a run another tool started (`direct_run`), and "close this task" of one the
//! scheduler dispatched (`policy_run`). The first asks the Runtime to stop the run the newest
//! `Status` names, at its next checkpoint, and to reset the device itself after it (`stop_run`,
//! Workflow #429 R-6's first slice, Runtime 0.12.53). The second closes the run's task first, and
//! then stops the run so (`close_then_stop`; R-8 and R-6b, Runtime 0.12.55): on one fresh person's
//! connection it reads the account's `PackScheduleView`, and when that lists the run's package it
//! writes the open row that the package is not scheduled, the very row the daily task list's switch
//! turned off writes (`switch`), and stops the run only once that took. A package the view does not
//! list (a reading's maintenance runs it, or the catalog changed since the dispatch) gets no row,
//! which the list could neither show nor withdraw, and its run is only stopped. A closed task is
//! dispatched no more until its switch on the daily task list turns it on again, the one way back;
//! a run only stopped would be dispatched again after its cooldown while its trigger holds, so a
//! scheduled run has no button that only stops it. A step the Runtime started for itself would only
//! be refused, so its button stays off and says why (`Stops::row`). One press sends either: nothing
//! asks first.
//!
//! Each account has a slot of its own (`Stops`, in the `Model` beside `Rescue`), and its tickets
//! come from the count the device's operations take theirs from (`Rescue::ticket`), so the log
//! never has two of one number for an account. A stop goes on a thread of its own (`THREAD`), over
//! a fresh person's connection, and an answer that counts is a write for the reads
//! (`super::wrote`), as an operation's is: it changes what they show of the account. It is no work
//! of the device's (`Model::device_busy`): no operation, refresh or switch of backends holds it
//! back, nor does it hold them; nor does a pause, a quiet time or a fence. A close holds the
//! account's slot of the daily task list's switches too while it is out (`Switches::close_begin`),
//! so that it and a switch never both have a write of the account out: the button stays off while a
//! switch is out (`Switches::writing`), and every switch while a close is, and after one that may
//! have written until the switches are read again.
//!
//! A press goes out once. One request alone of all the console's writes goes by itself, and only
//! once, as the follow-up of a first stop that answered the run still stops (`settled: false`), or
//! whose answer is not known: when a `Status` read online of the same owner epoch, begun after that
//! answer landed, lists the account without that run, so that the reset's result is read (the
//! Runtime answers it from its memory of the run). A follow-up only stops: it writes no close row
//! again. No timer sends anything, nor is a refusal or one not sent sent again, nor a close that did
//! not take. A `Status` of another owner epoch, or one that no longer lists the account, says what
//! came of it is not known and sends nothing; a read that does not come online says nothing, and
//! the stop waits on, for as long as the run goes on, which its deadline ends. The reading thread
//! gone ends the wait, not known either; a request out then lands as it would, and what either
//! comes to is drawn over the bar's one line that no Runtime answers, since no read online comes
//! again. A restart lets go of a request out: its answer, when it lands, is the log's only.
//!
//! The button stays off while a stop of the account is out, or waits for a run that still stops,
//! and for `RESCUE_COOLDOWN` after an answer lands or a restart let one go; after one not known,
//! no longer than that: while the run is there, a press is a new stop. A Runtime that answered it
//! has no stop (`not_implemented`, one before 0.12.53) keeps it off until another owner epoch; one
//! that has no task switches (a close's view or close row `not_implemented`, before 0.12.55; or the
//! daily task list's view said so) keeps "close this task" off until another owner epoch. A close's
//! view whose connection ended before its answer, which the client reports as a view it does not
//! know, is a view not read, and keeps nothing. Nothing else tells the console which Runtime it
//! reads: no read it makes names a revision. What the last stop came to is said under the button,
//! with when it landed, until the account's `Status` shows another run; of a close, that the task
//! was closed first, and when; the card and the picture say what holds now.
//!
//! Every answer goes in the log, from the thread, with the stopped run's request id, which the
//! window never shows, of a close its package and both steps' codes, and of a failure its whole
//! chain of causes: quietly when the run stopped, or had ended before the stop, or still stops
//! after the first's wait, a close that took before it included; as the problem it is otherwise.
//!
//! `Stops` and the views it makes hold no words, as the `Model` they are part of; `paint` makes
//! them.

use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::Instant;

use acui_source::workbench::pack::{
    close_then_stop, CloseAnswer, EditAnswer, PackOutcome, PackRefusal,
};
use acui_source::workbench::stop::{stop_run, Origin, Refusal, Reset, StopOutcome};
use acui_source::workbench::{ActivityKind, InstanceLive, RunKey};
use acui_source::ClientFailure;
use slint::Timer;

use super::failure_line;
use super::model::Link;
use super::paint;
use super::read::panic_message;
use super::rescue::{Rescue, RescueTicket, RESCUE_COOLDOWN};
use super::switch::Switches;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// The stop's thread, also what its lines in the technical log are under.
const THREAD: &str = "acui-act-stop";

/// Each account's stop of its run this run, and the owner epoch whose Runtime said it has no stop,
/// which is of the whole Runtime, not of an account; on the event loop only. A Runtime that said,
/// as a close went out, that it has no task switches is kept where the daily task list keeps its
/// own view's saying so (`Switches::view_unsupported`).
#[derive(Default)]
pub struct Stops {
    /// By alias.
    slots: BTreeMap<String, StopSlot>,
    /// The owner epoch whose Runtime answered a stop `not_implemented`.
    unsupported: Option<String>,
}

/// One account's: what is out or awaited; what the last stop came to, of a close what became of
/// the task, and the runs that is of; the ticket whose cooldown runs, which its timer ends
/// (`Stops::cooled`); and whether the thread that reads `Status` went while it was out or awaited.
#[derive(Default)]
struct StopSlot {
    stage: Stage,
    said: Option<StopSaid>,
    /// Of "close this task": what became of the task, which its stop's follow-up keeps.
    task: Option<TaskClose>,
    /// The run stopped, and the one the account's newest `Status` named as the answer landed: once
    /// it names a run of neither, the line goes.
    runs: Vec<RunKey>,
    cooling: Option<RescueTicket>,
    /// The thread that reads `Status` went while this stop was out or awaited (`reading_lost`):
    /// what it says from then on is drawn over the line that no Runtime answers (`Stops::unread`),
    /// since no read online ever comes again to draw it with the rows.
    unread: bool,
}

#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    /// A request is out: the first, or the follow-up (`Request::follows`).
    Out(Request),
    /// The first answered that the run still stops (`unconfirmed` false), or nothing says what
    /// became of it (`unconfirmed`): its follow-up goes once a `Status` online of its owner epoch
    /// lists the account without the run.
    Waiting { request: Request, unconfirmed: bool },
}

/// One request of a stop: its ticket; the run it stops and that run's package, which a close of the
/// task names, and the log; whether the scheduler dispatched that run, as the `Status` it was
/// pressed on said, for the log; whether it closes the run's task first ("close this task", never a
/// follow-up); the owner epoch online when it was pressed; and, of a follow-up, the first's ticket.
#[derive(Debug, Clone)]
struct Request {
    ticket: RescueTicket,
    run: RunKey,
    package: Option<String>,
    scheduled: bool,
    close: bool,
    epoch: String,
    follows: Option<RescueTicket>,
}

/// What a press of an account's "stop this run" or "close this task" would do now
/// (`Model::stop_target`): the run, its package, whether the scheduler dispatched it
/// (`policy_run`), whose task a press closes first, and the owner epoch online.
pub struct StopTarget {
    pub run: RunKey,
    pub package: Option<String>,
    pub scheduled: bool,
    pub epoch: String,
}

/// The rescue bar's row of "stop this run" or "close this task", while the account does something:
/// whether it can be pressed, and why not, or what a press does; and which of the two it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseRow {
    pub offered: bool,
    pub why: CloseWhy,
    /// "Close this task": the account runs a run the scheduler dispatched. Else "stop this run".
    pub task: bool,
}

/// What the row says beside the button, by the first of these that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseWhy {
    /// A stop or a close of the account is out, or waits for a run that still stops: the line under
    /// the row says how it goes, and nothing beside the button.
    Stopping,
    /// Its last stop just landed, or a restart let one go.
    CoolingDown,
    /// A switch of the account's daily task list is out (`Switches::writing`): one write of an
    /// account at a time.
    SwitchWriting,
    /// This owner epoch's Runtime answered that it has no stop.
    Unsupported,
    /// A run the scheduler dispatched, while this owner epoch's Runtime said it has no task
    /// switches: one before 0.12.55.
    CloseUnsupported,
    /// A run the scheduler dispatched, one finishing under the global pause too, which a press
    /// closes the task of and then stops: one of the two that can be pressed.
    CloseTask,
    /// A run the scheduler dispatched whose status names no request or no package: nothing to close
    /// it by. The log hears of it.
    CloseUnidentified,
    /// A run another tool started, which a press asks the Runtime to stop: the other that can be
    /// pressed.
    DirectRun,
    /// A run another tool started whose status names no request: nothing to stop it by. The log
    /// hears of it.
    Unidentified,
    /// A step the Runtime started itself: a host's run, or its recovery ladder.
    RuntimeOwn,
    /// A reconnect, a self-check, an emulator's control: no task runs.
    NoTask,
}

/// What the last stop of an account's run came to, and of one pressed as "close this task", what
/// became of the task, before their words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopLine {
    pub said: StopSaid,
    /// None: "stop this run", which closes nothing.
    pub task: Option<TaskClose>,
}

/// What a press of "close this task" did to the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskClose {
    /// Not known to be closed: the close is out, was not written, or what came of it is not known;
    /// nothing after it was stopped.
    Asked,
    /// Its close row was written, as the answer landed at this time (unix ms): the task is no
    /// longer scheduled, and the run was asked to stop after it.
    Closed { at_ms: u64 },
    /// The account's view does not list the run's package: no row was written, and the run was
    /// only asked to stop.
    NotListed,
}

/// What the last stop of an account's run came to, before its words; each that landed with when,
/// in unix ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopSaid {
    /// The first request is out: of "close this task", its close and the stop after it.
    Sending,
    /// The run ended, and the Runtime's own reset after it went as `reset` says.
    Stopped { at_ms: u64, reset: Reset },
    /// The first answered that the run still stops: its follow-up waits for it to go.
    Asked { at_ms: u64 },
    /// The follow-up is out, the run gone from `Status`.
    Reading,
    /// The follow-up answered that the run still stops, too: not sent again.
    Unsettled { at_ms: u64 },
    /// The Runtime refused the first or the follow-up (`follow_up`): why, who started the run when
    /// that is why, the host's code and the Runtime's for the words of any other reason, and whether
    /// it denied it, so that nothing was done, or failed on the way.
    Refused {
        at_ms: u64,
        why: Refusal,
        origin: Option<Origin>,
        host: Option<String>,
        class: Option<String>,
        denied: bool,
        follow_up: bool,
    },
    /// Nothing says what became of it: it may have happened.
    Unconfirmed { at_ms: u64 },
    /// The close of "close this task" was not written, or its view not read, and the run was not
    /// stopped: why.
    CloseRefused { at_ms: u64, why: CloseRefusal },
    /// Nothing says whether the close of "close this task" was written; the run was not stopped.
    CloseUnconfirmed { at_ms: u64 },
    /// The thread of "close this task" panicked: whether its close was written, and whether the
    /// run was stopped after it, are both not known. Nothing is sent again.
    ClosePanicked { at_ms: u64 },
    /// No connection: nothing was sent.
    NotSent { at_ms: u64 },
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { at_ms: u64, error: String },
    /// One was out, or awaited, across a restart: what came of it is not known.
    Restarted { at_ms: u64 },
    /// The account left the Runtime's list while awaited: not known either.
    Unlisted { at_ms: u64 },
    /// The thread that reads `Status` is gone while one was awaited: not known either.
    NoStatus { at_ms: u64 },
}

/// Why the close of "close this task" was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseRefusal {
    /// This Runtime has no task switches: nothing was done.
    Unsupported,
    /// Another write of the account's switches landed in the same millisecond, the second time too.
    NotNewer,
    /// The account's switches would hold more rows than the Runtime keeps.
    TooMany,
    /// What the Runtime holds of the account's switches does not read.
    StoredUnreadable,
    /// The view's connection ended before its answer (`CloseAnswer::ViewDropped`): it was not read.
    ViewDropped,
    /// Any other refusal, or the view not read: its codes, the host's before the Runtime's and then
    /// the client's own, for its words.
    Other { codes: Vec<String> },
}

/// What a `Status` read online calls for (`Stops::reviewed`), for the event loop to do: each
/// follow-up to send, each cooldown to start, each line for the log.
#[derive(Default)]
pub struct Review {
    follow_ups: Vec<(String, Request)>,
    cooling: Vec<(String, RescueTicket)>,
    lines: Vec<String>,
}

impl StopSlot {
    /// A stop of it is out, or waits for a run that still stops.
    fn stopping(&self) -> bool {
        matches!(self.stage, Stage::Out(_) | Stage::Waiting { unconfirmed: false, .. })
    }

    /// What its last stop came to, while it is said.
    fn line(&self) -> Option<StopLine> {
        let said = self.said.clone()?;
        Some(StopLine { said, task: self.task })
    }
}

impl Stops {
    /// The row of `instance`, which the newest `Status` read at the owner epoch `epoch` lists, while
    /// it does something: the first of these that holds. A stop or a close of it is out, or waits
    /// for a run that still stops; its last landed a moment ago; a switch of its daily task list is
    /// out (`switches`); this epoch's Runtime has no stop; and then what it does: a scheduled run
    /// that names its request and its package can be closed and stopped, but while this epoch's
    /// Runtime has no task switches, and a direct run that names its request stopped. The Runtime's
    /// own steps are never sent: a ladder's request is its claim's, not a run's. Whether its picture
    /// is withdrawn is the bar's to say (`Rescue::view`).
    pub fn row(
        &self,
        instance: &InstanceLive,
        epoch: &str,
        switches: &Switches,
    ) -> Option<CloseRow> {
        let activity = instance.activity.as_ref()?;
        let slot = self.slots.get(&instance.alias);
        let why = if slot.is_some_and(StopSlot::stopping) {
            CloseWhy::Stopping
        } else if slot.is_some_and(|slot| slot.cooling.is_some()) {
            CloseWhy::CoolingDown
        } else if switches.writing(&instance.alias) {
            CloseWhy::SwitchWriting
        } else if self.unsupported.as_deref() == Some(epoch) {
            CloseWhy::Unsupported
        } else {
            let unnamed = activity.run.is_none() || activity.package_id.is_none();
            match activity.kind {
                ActivityKind::PolicyRun if switches.unsupported(epoch) => {
                    CloseWhy::CloseUnsupported
                }
                ActivityKind::PolicyRun if unnamed => CloseWhy::CloseUnidentified,
                ActivityKind::PolicyRun => CloseWhy::CloseTask,
                ActivityKind::DirectRun if activity.run.is_none() => CloseWhy::Unidentified,
                ActivityKind::DirectRun => CloseWhy::DirectRun,
                ActivityKind::HostRun | ActivityKind::Ladder => CloseWhy::RuntimeOwn,
                ActivityKind::Reconnect
                | ActivityKind::Selfcheck
                | ActivityKind::EmulatorControl => CloseWhy::NoTask,
            }
        };
        let offered = matches!(why, CloseWhy::DirectRun | CloseWhy::CloseTask);
        let task = activity.kind == ActivityKind::PolicyRun;
        Some(CloseRow { offered, why, task })
    }

    /// What the last stop of `alias` came to, while it is said.
    pub fn said(&self, alias: &str) -> Option<StopLine> {
        self.slots.get(alias)?.line()
    }

    /// What the last stop of `alias` came to, when the thread that reads `Status` went while it
    /// was out or awaited: the one line of it a bar with no rows draws, since no read online ever
    /// comes again to draw it with them. Of one awaited, that what came of it is not known
    /// (`StopSaid::NoStatus`); of one out, how it goes, then its answer as it lands.
    pub fn unread(&self, alias: &str) -> Option<StopLine> {
        self.slots.get(alias).filter(|slot| slot.unread)?.line()
    }

    /// The first request of a stop of `alias` pressed: whether it goes, which it does not while one
    /// of the account is out, waits for a run that still stops, or cools down. One whose answer was
    /// not known waits no more: this press is a new stop. What it came to is, for now, that it is
    /// being sent; of a close, that the task is not known to be closed yet.
    fn begin(&mut self, alias: &str, request: &Request) -> bool {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if slot.stopping() || slot.cooling.is_some() {
            return false;
        }
        *slot = StopSlot {
            stage: Stage::Out(request.clone()),
            said: Some(StopSaid::Sending),
            task: request.close.then_some(TaskClose::Asked),
            runs: vec![request.run],
            cooling: None,
            unread: false,
        };
        true
    }

    /// What the stop of `alias` under `ticket` came to, landing at `at_ms` while the account's
    /// newest `Status` names the run `seen`: none when that ticket is not the one out, which a
    /// restart let go; else whether its cooldown runs from now, which it does but while the first
    /// waits for a run that still stops. A Runtime that has no stop is kept for its owner epoch.
    fn settle(
        &mut self,
        alias: &str,
        ticket: RescueTicket,
        landing: Landing,
        at_ms: u64,
        seen: Option<RunKey>,
    ) -> Option<bool> {
        let slot = self.slots.get_mut(alias)?;
        let request = match mem::take(&mut slot.stage) {
            Stage::Out(request) if request.ticket == ticket => request,
            other => {
                slot.stage = other;
                return None;
            }
        };
        if let Some(run) = seen.filter(|run| !slot.runs.contains(run)) {
            slot.runs.push(run);
        }
        let Settled { said, waits, task } = landing.settled(at_ms, &request);
        if let StopSaid::Refused { why: Refusal::NotImplemented, .. } = &said {
            self.unsupported = Some(request.epoch.clone());
        }
        if task.is_some() {
            slot.task = task;
        }
        slot.said = Some(said);
        let cools = waits != Some(false);
        if cools {
            slot.cooling = Some(ticket);
        }
        slot.stage = match waits {
            Some(unconfirmed) => Stage::Waiting { request, unconfirmed },
            None => Stage::Idle,
        };
        Some(cools)
    }

    /// The cooldown of `alias`'s stop under `ticket` ran out: whether it was the one that ran.
    fn cooled(&mut self, alias: &str, ticket: RescueTicket) -> bool {
        let Some(slot) = self.slots.get_mut(alias) else {
            return false;
        };
        if slot.cooling != Some(ticket) {
            return false;
        }
        slot.cooling = None;
        true
    }

    /// A `Status` read online at the owner epoch `epoch`, listing `instances`, at `at_ms`, begun
    /// after every stop's answer that counted (`super::wrote`). Of each account's stop:
    ///
    /// - one out of another epoch is let go, said to be out across a restart, and cools down from
    ///   now; its answer, when it lands, counts for nothing;
    /// - one awaited of another epoch is not known, and nothing is sent: the Runtime's memory of the
    ///   run went with it; nor when the account is not listed;
    /// - one awaited whose account is listed without its run, idle or doing another, has its
    ///   follow-up go, under a ticket of `rescue`'s count, once, a stop alone even after a close:
    ///   nothing sends it again;
    /// - the line of one that landed goes once the account runs another run.
    ///
    /// Tickets come from `rescue`'s count; what is to be done outside the model is the `Review`'s.
    pub fn reviewed(
        &mut self,
        epoch: &str,
        instances: &[InstanceLive],
        rescue: &mut Rescue,
        at_ms: u64,
    ) -> Review {
        if self.unsupported.as_deref().is_some_and(|said| said != epoch) {
            self.unsupported = None;
        }
        let mut review = Review::default();
        for (alias, slot) in &mut self.slots {
            // `None`: not listed; else the run it names now, if any.
            let listed = instances
                .iter()
                .find(|instance| instance.alias == *alias)
                .map(|instance| instance.activity.as_ref().and_then(|activity| activity.run));
            let was_idle = matches!(slot.stage, Stage::Idle);
            slot.stage = match mem::take(&mut slot.stage) {
                Stage::Idle => {
                    let another = listed.flatten().is_some_and(|run| !slot.runs.contains(&run));
                    if another {
                        slot.said = None;
                        slot.task = None;
                    }
                    Stage::Idle
                }
                Stage::Out(request) if request.epoch != epoch => {
                    slot.said = Some(StopSaid::Restarted { at_ms });
                    slot.cooling = Some(request.ticket);
                    review.cooling.push((alias.clone(), request.ticket));
                    review.lines.push(format!(
                        "{}: let go, the Runtime started again; what came of it is not known",
                        prefix(alias, &request)
                    ));
                    Stage::Idle
                }
                Stage::Waiting { request, .. } if request.epoch != epoch => {
                    slot.said = Some(StopSaid::Restarted { at_ms });
                    review.lines.push(format!(
                        "{}: awaited across a restart of the Runtime; what came of it is not \
                         known, not sent again",
                        prefix(alias, &request)
                    ));
                    Stage::Idle
                }
                Stage::Waiting { request, .. } if listed.is_none() => {
                    slot.said = Some(StopSaid::Unlisted { at_ms });
                    review.lines.push(format!(
                        "{}: the Runtime no longer lists the account; what came of it is not \
                         known, not sent again",
                        prefix(alias, &request)
                    ));
                    Stage::Idle
                }
                Stage::Waiting { request, unconfirmed } if listed == Some(Some(request.run)) => {
                    Stage::Waiting { request, unconfirmed }
                }
                Stage::Waiting { request, .. } => {
                    let follow_up = Request {
                        ticket: rescue.ticket(),
                        close: false,
                        follows: Some(request.ticket),
                        ..request
                    };
                    slot.said = Some(StopSaid::Reading);
                    review.follow_ups.push((alias.clone(), follow_up.clone()));
                    Stage::Out(follow_up)
                }
                stage @ Stage::Out(_) => stage,
            };
            // Said just now: the run this `Status` names is one its line is of, as at a landing.
            let said_now = !was_idle && matches!(slot.stage, Stage::Idle);
            if let Some(run) = listed.flatten().filter(|run| said_now && !slot.runs.contains(run)) {
                slot.runs.push(run);
            }
        }
        review
    }

    /// The thread that reads `Status` is gone, at `at_ms`: no read will say what came of a stop
    /// awaited, so each ends, not known. A request out lands as it would. What either says from
    /// now on is drawn without the rows (`unread`). The lines for the log.
    fn reading_lost(&mut self, at_ms: u64) -> Vec<String> {
        let mut lines = Vec::new();
        for (alias, slot) in &mut self.slots {
            slot.stage = match mem::take(&mut slot.stage) {
                stage @ Stage::Out(_) => {
                    slot.unread = true;
                    stage
                }
                Stage::Waiting { request, .. } => {
                    slot.unread = true;
                    slot.said = Some(StopSaid::NoStatus { at_ms });
                    lines.push(format!(
                        "{}: no Status is read any more; what came of it is not known, not sent \
                         again",
                        prefix(alias, &request)
                    ));
                    Stage::Idle
                }
                Stage::Idle => Stage::Idle,
            };
        }
        lines
    }
}

/// What one request came to, as its thread hands it back.
enum Landing {
    /// A stop alone: of a direct run, or a follow-up.
    Answer(StopOutcome),
    /// "Close this task": its view, its close and the stop after it (`close_then_stop`).
    Close(CloseAnswer),
    /// The thread panicked: whether the request got out is not known.
    Panicked(String),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

/// What one landing comes to, before its words: what it says; of a first stop, a close's included,
/// whether its follow-up is to wait for the run to go (`Some`): one that still stops (`false`), or
/// one whose answer is not known (`true`); and of a close, what became of the task, when that is
/// known (none leaves it as it was).
struct Settled {
    said: StopSaid,
    waits: Option<bool>,
    task: Option<TaskClose>,
}

impl Landing {
    /// What it came to as it lands at `at_ms`, of `request`.
    fn settled(self, at_ms: u64, request: &Request) -> Settled {
        let follow_up = request.follows.is_some();
        let (said, waits) = match self {
            Self::Answer(outcome) => stop_said(outcome, at_ms, follow_up),
            Self::Close(CloseAnswer::Closed { stop, .. }) => {
                let (said, waits) = stop_said(stop, at_ms, false);
                return Settled { said, waits, task: Some(TaskClose::Closed { at_ms }) };
            }
            Self::Close(CloseAnswer::NotListed(stop)) => {
                let (said, waits) = stop_said(stop, at_ms, false);
                return Settled { said, waits, task: Some(TaskClose::NotListed) };
            }
            Self::Close(CloseAnswer::NotSent(_)) => (StopSaid::NotSent { at_ms }, None),
            Self::Close(CloseAnswer::Unsupported { .. }) => {
                (StopSaid::CloseRefused { at_ms, why: CloseRefusal::Unsupported }, None)
            }
            Self::Close(CloseAnswer::ViewFailed(failure)) => {
                let why = CloseRefusal::Other { codes: failure_codes(&failure) };
                (StopSaid::CloseRefused { at_ms, why }, None)
            }
            Self::Close(CloseAnswer::ViewDropped(_)) => {
                (StopSaid::CloseRefused { at_ms, why: CloseRefusal::ViewDropped }, None)
            }
            Self::Close(CloseAnswer::NotClosed(EditAnswer { outcome, .. })) => {
                (not_closed(outcome, at_ms), None)
            }
            // It may have panicked after its stop went: whether the close was written, and whether
            // the run stopped, are not known. Nothing is sent again.
            Self::Panicked(_) if request.close => (StopSaid::ClosePanicked { at_ms }, None),
            Self::Panicked(_) => (StopSaid::Unconfirmed { at_ms }, (!follow_up).then_some(true)),
            Self::NoThread(error) => (StopSaid::NoThread { at_ms, error }, None),
        };
        Settled { said, waits, task: None }
    }

    /// Of "close this task", whether its Runtime said it has no task switches: its view or its
    /// close row `not_implemented`. A view whose connection ended before its answer is no such
    /// word (`CloseAnswer::ViewDropped`).
    fn close_unsupported(&self) -> bool {
        matches!(
            self,
            Self::Close(
                CloseAnswer::Unsupported { .. }
                    | CloseAnswer::NotClosed(EditAnswer {
                        outcome: PackOutcome::NotImplemented(_),
                        ..
                    })
            )
        )
    }

    /// Of "close this task", whether its close row may have been written: a write of the account's
    /// switches, which wait for a read of them after it (`Switches::close_landed`).
    fn close_wrote(&self) -> bool {
        match self {
            Self::Close(CloseAnswer::Closed { .. }) | Self::Panicked(_) => true,
            Self::Close(CloseAnswer::NotClosed(EditAnswer { outcome, .. })) => {
                matches!(outcome, PackOutcome::Unconfirmed(_) | PackOutcome::Edited { .. })
            }
            Self::Close(
                CloseAnswer::NotSent(_)
                | CloseAnswer::Unsupported { .. }
                | CloseAnswer::ViewFailed(_)
                | CloseAnswer::ViewDropped(_)
                | CloseAnswer::NotListed(_),
            )
            | Self::Answer(_)
            | Self::NoThread(_) => false,
        }
    }

    /// Into the technical log, with the stopped run's request id, of a close its package and its
    /// write's version or codes, its codes and, of a failed reset or a request that failed, its
    /// whole chain of causes: quietly when the run stopped, ended before the stop, or still stops
    /// after the first's wait, a close that took before it included; the rest as the problems they
    /// are.
    fn log(&self, alias: &str, request: &Request) {
        let first = request.follows.is_none();
        let (line, quiet) = match self {
            Self::Answer(outcome) => stop_line(outcome, first),
            Self::Close(answer) => close_line(answer),
            Self::Panicked(message) if request.close => (
                format!(
                    "panicked; whether the close row was written and whether the run was stopped \
                     are both not known, not sent again: {message}"
                ),
                false,
            ),
            Self::Panicked(message) => {
                (format!("panicked, unconfirmed{}: {message}", then(first)), false)
            }
            Self::NoThread(error) => (format!("no thread, not sent: {error}"), false),
        };
        let line = format!("{}: {line}", prefix(alias, request));
        if quiet {
            report::log_quiet(THREAD, &line);
        } else {
            report::log(THREAD, &line);
        }
    }
}

/// What a stop's answer came to, before its words, as it lands at `at_ms`; and, of the first
/// request (`follow_up` false), whether its follow-up is to wait for the run to go (`Some`): one
/// that still stops (`false`), or one whose answer is not known (`true`).
fn stop_said(outcome: StopOutcome, at_ms: u64, follow_up: bool) -> (StopSaid, Option<bool>) {
    match outcome {
        StopOutcome::Stopped { reset: Some(reset), .. } => {
            (StopSaid::Stopped { at_ms, reset }, None)
        }
        StopOutcome::Stopped { reset: None, .. } if follow_up => {
            (StopSaid::Unsettled { at_ms }, None)
        }
        StopOutcome::Stopped { reset: None, .. } => (StopSaid::Asked { at_ms }, Some(false)),
        StopOutcome::Refused { failure, why, origin, denied, .. } => {
            let said = StopSaid::Refused {
                at_ms,
                why,
                origin,
                host: failure.host.map(|(code, _)| code),
                class: failure.runtime_code,
                denied,
                follow_up,
            };
            (said, None)
        }
        StopOutcome::NotSent { .. } => (StopSaid::NotSent { at_ms }, None),
        StopOutcome::Unconfirmed { .. } => {
            (StopSaid::Unconfirmed { at_ms }, (!follow_up).then_some(true))
        }
    }
}

/// A close row's write that did not take, before its words: why, or that it may have.
fn not_closed(outcome: PackOutcome, at_ms: u64) -> StopSaid {
    let why = match outcome {
        // A write that names no version meets no other's: not reached. Said as another write at
        // the same moment, which it would be.
        PackOutcome::Conflict { .. } | PackOutcome::Refused { why: PackRefusal::NotNewer, .. } => {
            CloseRefusal::NotNewer
        }
        PackOutcome::Refused { why: PackRefusal::TooMany, .. } => CloseRefusal::TooMany,
        PackOutcome::Refused { why: PackRefusal::StoredUnreadable, .. } => {
            CloseRefusal::StoredUnreadable
        }
        PackOutcome::Refused { why: PackRefusal::Invalid | PackRefusal::Other, failure, .. } => {
            CloseRefusal::Other { codes: failure_codes(&failure) }
        }
        PackOutcome::NotImplemented(_) => CloseRefusal::Unsupported,
        // Not reached: the write goes on the connection the view was read on.
        PackOutcome::NotSent(_) => return StopSaid::NotSent { at_ms },
        // `Edited` is not reached: a close row that took is followed by the stop.
        PackOutcome::Unconfirmed(_) | PackOutcome::Edited { .. } => {
            return StopSaid::CloseUnconfirmed { at_ms };
        }
    };
    StopSaid::CloseRefused { at_ms, why }
}

/// A refusal's codes for its words: the host's before the Runtime's, then the client's own, which
/// a read that failed may be all there is of.
fn failure_codes(failure: &ClientFailure) -> Vec<String> {
    let host = failure.host.as_ref().map(|(code, _)| code.clone());
    host.into_iter()
        .chain(failure.runtime_code.clone())
        .chain(Some(failure.code.to_owned()))
        .collect()
}

/// A stop's answer in the log's words, of the first request or the follow-up, and whether it goes
/// quietly: when the run stopped, ended before the stop, or still stops after the first's wait.
fn stop_line(outcome: &StopOutcome, first: bool) -> (String, bool) {
    match outcome {
        StopOutcome::Stopped { reset: Some(Reset::Done), .. } => {
            ("stopped; the Runtime reset the device".to_owned(), true)
        }
        StopOutcome::Stopped { reset: Some(Reset::NotNeeded), .. } => (
            "the run ended for another reason before this stop; the Runtime reset nothing for it \
             (not_needed)"
                .to_owned(),
            true,
        ),
        StopOutcome::Stopped { reset: Some(Reset::Failed { cause, chain }), .. } => {
            (format!("stopped, its reset failed, fenced; cause {cause:?}: {chain}"), false)
        }
        StopOutcome::Stopped { reset: None, .. } if first => (
            "still stopping after the wait; asked once more when a Status shows the run gone"
                .to_owned(),
            true,
        ),
        StopOutcome::Stopped { reset: None, .. } => (
            "still stopping after its wait too; its result not read, not sent again".to_owned(),
            false,
        ),
        StopOutcome::Refused { failure, chain, why: Refusal::RunUnknown, .. } if first => (
            format!(
                "the run had ended before the stop; nothing was done: {}; chain: {chain}",
                failure_line(failure)
            ),
            true,
        ),
        StopOutcome::Refused { failure, chain, why, origin, denied } => {
            let how = if *denied { "denied" } else { "failed" };
            let line = format!(
                "refused, {how}, {why:?}, run origin {origin:?}: {}; chain: {chain}",
                failure_line(failure)
            );
            (line, false)
        }
        StopOutcome::NotSent { failure, chain } => {
            (format!("not sent: {}; chain: {chain}", failure_line(failure)), false)
        }
        StopOutcome::Unconfirmed { failure, chain } => {
            let line =
                format!("unconfirmed{}: {}; chain: {chain}", then(first), failure_line(failure));
            (line, false)
        }
    }
}

/// "Close this task" in the log's words, both its steps, and whether it goes quietly: when its
/// close took, or its package is not listed, and its stop's would.
fn close_line(answer: &CloseAnswer) -> (String, bool) {
    match answer {
        CloseAnswer::Closed { version, resent, stop } => {
            let (stop, quiet) = stop_line(stop, true);
            let line = format!(
                "closed{}, the list now at version {version}; then {stop}",
                resent_words(*resent)
            );
            (line, quiet)
        }
        CloseAnswer::NotListed(stop) => {
            let (stop, quiet) = stop_line(stop, true);
            let line = format!(
                "the account's PackScheduleView lists no such package, so no close row was \
                 written; then {stop}"
            );
            (line, quiet)
        }
        CloseAnswer::NotSent(failure) => (format!("not sent: {}", failure_line(failure)), false),
        CloseAnswer::Unsupported { code, failure } => {
            let line = format!(
                "this Runtime has no task switches (PackScheduleView {code}); nothing was written \
                 and the run was not stopped: {}",
                failure_line(failure)
            );
            (line, false)
        }
        CloseAnswer::ViewFailed(failure) => {
            let line = format!(
                "the account's PackScheduleView was not read; nothing was written and the run was \
                 not stopped: {}",
                failure_line(failure)
            );
            (line, false)
        }
        CloseAnswer::ViewDropped(failure) => {
            let line = format!(
                "the account's PackScheduleView was not read, its connection having ended before \
                 its answer (made again in the same owner epoch), not taken as no task switches; \
                 nothing was written and the run was not stopped: {}",
                failure_line(failure)
            );
            (line, false)
        }
        CloseAnswer::NotClosed(EditAnswer { outcome, resent }) => {
            let line = format!(
                "the close row's write{}: {}; the run was not stopped, not sent again",
                resent_words(*resent),
                edit_line(outcome)
            );
            (line, false)
        }
    }
}

/// What a close row's write that did not take came to, for the log.
fn edit_line(outcome: &PackOutcome) -> String {
    match outcome {
        // Not reached: a close row that took is followed by the stop.
        PackOutcome::Edited { version } => format!("done, the list now at version {version}"),
        PackOutcome::Conflict { current } => {
            format!("in conflict, the list at version {current:?}, nothing changed")
        }
        PackOutcome::Refused { why, failure, field } => {
            format!("refused, {why:?}, field {field:?}: {}", failure_line(failure))
        }
        PackOutcome::NotImplemented(failure) => format!(
            "EditPackSchedule not_implemented, no task switches in this Runtime: {}",
            failure_line(failure)
        ),
        PackOutcome::NotSent(failure) => format!("not sent: {}", failure_line(failure)),
        PackOutcome::Unconfirmed(failure) => {
            format!("unconfirmed, it may have been written: {}", failure_line(failure))
        }
    }
}

/// That a close row went a second time after `fact_observation_not_newer`, for the log.
fn resent_words(resent: bool) -> &'static str {
    if resent {
        ", sent once more after fact_observation_not_newer"
    } else {
        ""
    }
}

/// What follows an answer not known, for the log: of the first request, a follow-up once a `Status`
/// shows the run gone; of the follow-up, nothing.
fn then(first: bool) -> &'static str {
    if first {
        ", asked once more if a Status shows the run gone"
    } else {
        ", not sent again"
    }
}

/// The log's words for one request: its ticket, the account, of a close the package it closes and
/// the run it stops after, else the run it stops, which kind of run that is and its package, and of
/// a follow-up the first's ticket.
fn prefix(alias: &str, request: &Request) -> String {
    let package = request.package.as_deref().unwrap_or("(none named)");
    let ticket = request.ticket.number();
    let run = request.run.line();
    let mut prefix = if request.close {
        format!("ticket {ticket} of {alias}, close package {package} then stop run {run}")
    } else {
        let kind = if request.scheduled { "policy run" } else { "direct run" };
        format!("ticket {ticket} of {alias}, stop run {run} ({kind}) of package {package}")
    };
    if let Some(first) = request.follows {
        prefix.push_str(&format!(", the follow-up of ticket {}", first.number()));
    }
    prefix
}

/// "Stop this run", or "close this task", on the bar of the account whose picture shows.
pub fn pressed(ui: &Ui) {
    let alias = ui.model.borrow().picture_account().map(str::to_owned);
    if let Some(alias) = alias {
        press(ui, &alias);
    }
}

/// The button of `alias`'s bar pressed: nothing when it cannot be pressed now, which the newest
/// `Status`, the account's own stops and its switches decide (`Model::stop_target`); else the run
/// that `Status` names is stopped, of a scheduled run after its task is closed, once, at once. A
/// close holds the account's switches' slot until it lands. The device's question asked goes, as
/// any other press of the bar's makes it go.
fn press(ui: &Ui, alias: &str) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let request = {
        let mut model = ui.model.borrow_mut();
        let Some(target) = model.stop_target(alias) else {
            return;
        };
        let rescue = model.rescue_mut();
        rescue.unconfirm();
        let request = Request {
            ticket: rescue.ticket(),
            run: target.run,
            package: target.package,
            scheduled: target.scheduled,
            close: target.scheduled,
            epoch: target.epoch,
            follows: None,
        };
        if request.close && !model.switches_mut().close_begin(alias, request.ticket) {
            return;
        }
        if !model.stops_mut().begin(alias, &request) {
            if request.close {
                model.switches_mut().close_landed(alias, request.ticket, false);
            }
            return;
        }
        request
    };
    send(ui, launcher.state_root.clone(), alias, request);
}

/// `request` of `alias`, once, on a thread of its own. A thread the system refuses is its answer
/// at once, in red; a panic on it is an answer as well: with none at all the button would stay off
/// for good, and of a close the account's switches too.
fn send(ui: &Ui, root: PathBuf, alias: &str, request: Request) {
    let (worker_alias, worker_request) = (alias.to_owned(), request.clone());
    let spawned = spawn_worker(THREAD, move || {
        let (alias, request) = (worker_alias, worker_request);
        let landing = match catch_unwind(AssertUnwindSafe(|| sent(&root, &alias, &request))) {
            Ok(landing) => landing,
            Err(panic) => Landing::Panicked(panic_message(&*panic)),
        };
        landing.log(&alias, &request);
        report::post(THREAD, move || {
            if let Some(ui) = ui::current() {
                landed(&ui, &alias, &request, landing);
            }
        });
    });
    match spawned {
        Ok(()) => repaint(ui, request.close),
        Err(error) => {
            let landing = Landing::NoThread(error.to_string());
            landing.log(alias, &request);
            landed(ui, alias, &request, landing);
        }
    }
}

/// `request` sent, on its thread: of a close, its view, its close and the stop after it; else the
/// stop alone. A close always names its package (`Model::stop_target`).
fn sent(root: &Path, alias: &str, request: &Request) -> Landing {
    match request.package.as_deref().filter(|_| request.close) {
        Some(package) => Landing::Close(close_then_stop(root, alias, package, request.run)),
        None => Landing::Answer(stop_run(root, alias, request.run)),
    }
}

/// An answer, on the event loop. A close gives back the account's switches' slot first, whatever
/// else becomes of it. One whose ticket is no longer the one out, which a restart let go, changes
/// nothing shown: only the log had it. One that counts is a write: the reads begun before it are let
/// go, and read again, so that only a `Status` begun after it decides its follow-up, and the
/// switches show what a close wrote. A close whose Runtime said it has no task switches keeps that
/// for its owner epoch where the daily task list keeps its own view's saying so, which then reads
/// them no more either (`super::tasks_sync`). With the reading thread gone, a stop that would wait
/// for one ends at once.
fn landed(ui: &Ui, alias: &str, request: &Request, landing: Landing) {
    let at_ms = paint::now_ms();
    let ticket = request.ticket;
    let wrote = request.close.then(|| landing.close_wrote());
    let unsupported = request.close && landing.close_unsupported();
    let (counted, reading_gone, no_switches) = {
        let mut model = ui.model.borrow_mut();
        if let Some(wrote) = wrote {
            model.switches_mut().close_landed(alias, ticket, wrote);
        }
        let seen = model.account_run(alias);
        let reading_gone = matches!(model.link(), Link::ThreadFailed);
        let counted = model.stops_mut().settle(alias, ticket, landing, at_ms, seen);
        let no_switches = unsupported
            && counted.is_some()
            && model.switches_mut().view_unsupported(&request.epoch, Instant::now()).is_some();
        (counted, reading_gone, no_switches)
    };
    let Some(cools) = counted else {
        let line = format!(
            "ticket {} of {alias}, stop run: its answer came after a restart let it go, and counts \
             for nothing",
            ticket.number()
        );
        report::log_quiet(THREAD, &line);
        repaint(ui, request.close);
        ui.paint_log_notes();
        return;
    };
    if reading_gone {
        lane_lost(ui);
    }
    if no_switches {
        super::tasks_sync(ui);
    }
    if cools {
        cool_down(alias.to_owned(), ticket);
    }
    super::wrote(ui);
    repaint(ui, request.close);
    ui.paint_log_notes();
}

/// A `Status` read landed (`super::live_landed`), after a restart let the device's operations go:
/// what each account's stop waits for, judged by it when it is a read online, and only then
/// (`Stops::reviewed`). The follow-ups it calls for go out, each once. Painting is the caller's.
pub fn status_read(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let review = ui.model.borrow_mut().stops_reviewed(paint::now_ms());
    let Review { follow_ups, cooling, lines } = review;
    for line in &lines {
        report::log(THREAD, line);
    }
    for (alias, ticket) in cooling {
        cool_down(alias, ticket);
    }
    for (alias, request) in follow_ups {
        send(ui, launcher.state_root.clone(), &alias, request);
    }
}

/// The thread that reads `Status` is gone (`super::lane_failed`): every stop awaited ends, not
/// known. Painting is the caller's.
pub fn lane_lost(ui: &Ui) {
    let lines = ui.model.borrow_mut().stops_mut().reading_lost(paint::now_ms());
    for line in &lines {
        report::log(THREAD, line);
    }
}

/// `alias`'s stop under `ticket` cools down: its button comes back once this runs out.
fn cool_down(alias: String, ticket: RescueTicket) {
    Timer::single_shot(RESCUE_COOLDOWN, move || {
        let Some(ui) = ui::current() else {
            return;
        };
        let cooled = ui.model.borrow_mut().stops_mut().cooled(&alias, ticket);
        if cooled {
            repaint(&ui, false);
        }
    });
}

/// What a press, an answer or a cooldown may have changed in the window: the rescue bar, which
/// `paint::refresh` paints with the picture's refresh; and of a close, the daily task list, whose
/// switches it holds while it is out.
fn repaint(ui: &Ui, close: bool) {
    paint::refresh(ui);
    if close {
        paint::tasks(ui);
    }
}
