// SPDX-License-Identifier: GPL-3.0-only
//! A person's stop of one run of an account (UI-3e2): the rescue bar's "stop this run", which asks
//! the Runtime to stop the run the newest `Status` names, at its next checkpoint, and to reset the
//! device itself after it (`stop_run`, Workflow #429 R-6's first slice, Runtime 0.12.53). Only a
//! run another tool started (`direct_run`) is sent: a run the scheduler dispatched, or one the
//! Runtime started for itself, would only be refused, so its button stays off and says why
//! (`Stops::row`). One press sends it: nothing asks first.
//!
//! Each account has a slot of its own (`Stops`, in the `Model` beside `Rescue`), and its tickets
//! come from the count the device's operations take theirs from (`Rescue::ticket`), so the log
//! never has two of one number for an account. A stop goes on a thread of its own (`THREAD`), over
//! a fresh person's connection, and an answer that counts is a write for the reads
//! (`super::wrote`), as an operation's is: it changes what they show of the account. It is no work
//! of the device's (`Model::device_busy`): no operation, refresh or switch of backends holds it
//! back, nor does it hold them; nor does a pause, a quiet time or a fence.
//!
//! A press goes out once. One request alone of all the console's writes goes by itself, and only
//! once, as the follow-up of a first that answered the run still stops (`settled: false`), or whose
//! answer is not known: when a `Status` read online of the same owner epoch, begun after that
//! answer landed, lists the account without that run, so that the reset's result is read (the
//! Runtime answers it from its memory of the run). No timer sends anything, nor is a refusal or one
//! not sent sent again. A `Status` of another owner epoch, or one that no longer lists the account,
//! says what came of it is not known and sends nothing; a read that does not come online says
//! nothing, and the stop waits on, for as long as the run goes on, which its deadline ends. The
//! reading thread gone ends the wait, not known either. A restart lets go of a request out: its
//! answer, when it lands, is the log's only.
//!
//! The button stays off while a stop of the account is out, or waits for a run that still stops,
//! and for `RESCUE_COOLDOWN` after an answer lands or a restart let one go; after one not known,
//! no longer than that: while the run is there, a press is a new stop. A Runtime that answered it
//! has no stop (`not_implemented`, one before 0.12.53) keeps it off until another owner epoch.
//! What the last stop came to is said under the button, with when it landed, until the account's
//! `Status` shows another run; the card and the picture say what holds now.
//!
//! Every answer goes in the log, from the thread, with the stopped run's request id, which the
//! window never shows: quietly when the run stopped, or had ended, as the problem it is otherwise.
//!
//! `Stops` and the views it makes hold no words, as the `Model` they are part of; `paint` makes
//! them.

use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use acui_source::workbench::stop::{stop_run, Origin, Refusal, Reset, StopOutcome};
use acui_source::workbench::{ActivityKind, InstanceLive, RunKey};
use slint::Timer;

use super::failure_line;
use super::model::Link;
use super::paint;
use super::read::panic_message;
use super::rescue::{Rescue, RescueTicket, RESCUE_COOLDOWN};
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// The stop's thread, also what its lines in the technical log are under.
const THREAD: &str = "acui-act-stop";

/// Each account's stop of its run this run, and the owner epoch whose Runtime said it has no stop;
/// on the event loop only.
#[derive(Default)]
pub struct Stops {
    /// By alias.
    slots: BTreeMap<String, StopSlot>,
    /// The owner epoch whose Runtime answered `not_implemented`.
    unsupported: Option<String>,
}

/// One account's: what is out or awaited; what the last stop came to, and the runs that is of; and
/// the ticket whose cooldown runs, which its timer ends (`Stops::cooled`).
#[derive(Default)]
struct StopSlot {
    stage: Stage,
    said: Option<StopSaid>,
    /// The run stopped, and the one the account's newest `Status` named as the answer landed: once
    /// it names a run of neither, the line goes.
    runs: Vec<RunKey>,
    cooling: Option<RescueTicket>,
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

/// One request of a stop: its ticket; the run it stops and that run's package, for R-8's close of
/// the task and the log; the owner epoch online when it was pressed; and, of a follow-up, the
/// first's ticket.
#[derive(Debug, Clone)]
struct Request {
    ticket: RescueTicket,
    run: RunKey,
    package: Option<String>,
    epoch: String,
    follows: Option<RescueTicket>,
}

/// What a press of an account's "stop this run" would stop now (`Model::stop_target`).
pub struct StopTarget {
    pub run: RunKey,
    pub package: Option<String>,
    pub epoch: String,
}

/// "Stop this run" on an account's bar, while it does something: whether it can be pressed, and
/// why not, or what a press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseRow {
    pub offered: bool,
    pub why: CloseWhy,
}

/// What the row says beside the button, by the first of these that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseWhy {
    /// A stop of the account is out, or waits for a run that still stops: the line under the row
    /// says how it goes, and nothing beside the button.
    Stopping,
    /// Its last stop just landed, or a restart let one go.
    CoolingDown,
    /// This owner epoch's Runtime answered that it has no stop.
    Unsupported,
    /// A run another tool started, which a press asks the Runtime to stop: the one that can be
    /// pressed.
    DirectRun,
    /// A direct run whose status names no request: nothing to stop it by. The log hears of it.
    Unidentified,
    /// A run the scheduler dispatched, one finishing under the global pause too: the account's own
    /// pause on its card stops it.
    Scheduled,
    /// A step the Runtime started itself: a host's run, or its recovery ladder.
    RuntimeOwn,
    /// A reconnect, a self-check, an emulator's control: no task runs.
    NoTask,
}

/// What the last stop of an account's run came to, before its words; each that landed with when,
/// in unix ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopSaid {
    /// The first request is out.
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
}

impl Stops {
    /// The row of `instance`, which the newest `Status` read at the owner epoch `epoch` lists, while
    /// it does something: the first of these that holds. A stop of it is out, or waits for a run
    /// that still stops; its last landed a moment ago; this epoch's Runtime has no stop; and then
    /// what it does, of which only a direct run that names its request can be stopped. Whether its
    /// picture is withdrawn is the bar's to say (`Rescue::view`).
    pub fn row(&self, instance: &InstanceLive, epoch: &str) -> Option<CloseRow> {
        let activity = instance.activity.as_ref()?;
        let slot = self.slots.get(&instance.alias);
        let why = if slot.is_some_and(StopSlot::stopping) {
            CloseWhy::Stopping
        } else if slot.is_some_and(|slot| slot.cooling.is_some()) {
            CloseWhy::CoolingDown
        } else if self.unsupported.as_deref() == Some(epoch) {
            CloseWhy::Unsupported
        } else {
            match activity.kind {
                ActivityKind::DirectRun if activity.run.is_some() => CloseWhy::DirectRun,
                ActivityKind::DirectRun => CloseWhy::Unidentified,
                ActivityKind::PolicyRun => CloseWhy::Scheduled,
                ActivityKind::HostRun | ActivityKind::Ladder => CloseWhy::RuntimeOwn,
                ActivityKind::Reconnect
                | ActivityKind::Selfcheck
                | ActivityKind::EmulatorControl => CloseWhy::NoTask,
            }
        };
        Some(CloseRow { offered: why == CloseWhy::DirectRun, why })
    }

    /// What the last stop of `alias` came to, while it is said.
    pub fn said(&self, alias: &str) -> Option<&StopSaid> {
        self.slots.get(alias)?.said.as_ref()
    }

    /// The first request of a stop of `alias` pressed: whether it goes, which it does not while one
    /// of the account is out, waits for a run that still stops, or cools down. One whose answer was
    /// not known waits no more: this press is a new stop. What it came to is, for now, that it is
    /// being sent.
    fn begin(&mut self, alias: &str, request: &Request) -> bool {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if slot.stopping() || slot.cooling.is_some() {
            return false;
        }
        *slot = StopSlot {
            stage: Stage::Out(request.clone()),
            said: Some(StopSaid::Sending),
            runs: vec![request.run],
            cooling: None,
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
        let (said, waits) = landing.said(at_ms, request.follows.is_some());
        if matches!(said, StopSaid::Refused { why: Refusal::NotImplemented, .. }) {
            self.unsupported = Some(request.epoch.clone());
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
    ///   follow-up go, under a ticket of `rescue`'s count, once: nothing sends it again;
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
    /// awaited, so each ends, not known. A request out lands as it would. The lines for the log.
    fn reading_lost(&mut self, at_ms: u64) -> Vec<String> {
        let mut lines = Vec::new();
        for (alias, slot) in &mut self.slots {
            slot.stage = match mem::take(&mut slot.stage) {
                Stage::Waiting { request, .. } => {
                    slot.said = Some(StopSaid::NoStatus { at_ms });
                    lines.push(format!(
                        "{}: no Status is read any more; what came of it is not known, not sent \
                         again",
                        prefix(alias, &request)
                    ));
                    Stage::Idle
                }
                stage => stage,
            };
        }
        lines
    }
}

/// What one request came to, as its thread hands it back.
enum Landing {
    Answer(StopOutcome),
    /// The thread panicked: whether the request got out is not known.
    Panicked(String),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

impl Landing {
    /// What it came to, before its words, as it lands at `at_ms`; and, of the first request
    /// (`follow_up` false), whether its follow-up is to wait for the run to go (`Some`): one that
    /// still stops (`false`), or one whose answer is not known (`true`).
    fn said(self, at_ms: u64, follow_up: bool) -> (StopSaid, Option<bool>) {
        match self {
            Self::Answer(StopOutcome::Stopped { reset: Some(reset), .. }) => {
                (StopSaid::Stopped { at_ms, reset }, None)
            }
            Self::Answer(StopOutcome::Stopped { reset: None, .. }) if follow_up => {
                (StopSaid::Unsettled { at_ms }, None)
            }
            Self::Answer(StopOutcome::Stopped { reset: None, .. }) => {
                (StopSaid::Asked { at_ms }, Some(false))
            }
            Self::Answer(StopOutcome::Refused { failure, why, origin, denied }) => {
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
            Self::Answer(StopOutcome::NotSent(_)) => (StopSaid::NotSent { at_ms }, None),
            Self::Answer(StopOutcome::Unconfirmed(_)) | Self::Panicked(_) => {
                (StopSaid::Unconfirmed { at_ms }, (!follow_up).then_some(true))
            }
            Self::NoThread(error) => (StopSaid::NoThread { at_ms, error }, None),
        }
    }

    /// Into the technical log, with the stopped run's request id, its codes and, of a failed
    /// reset, its whole chain of causes: quietly when the run stopped, ended before the stop, or
    /// still stops after the first's wait; the rest as the problems they are.
    fn log(&self, alias: &str, request: &Request) {
        let prefix = prefix(alias, request);
        let first = request.follows.is_none();
        let line = match self {
            Self::Answer(StopOutcome::Stopped { reset: Some(Reset::Done), .. }) => {
                let line = format!("{prefix}: stopped; the Runtime reset the device");
                report::log_quiet(THREAD, &line);
                return;
            }
            Self::Answer(StopOutcome::Stopped { reset: Some(Reset::NotNeeded), .. }) => {
                let line = format!(
                    "{prefix}: the run ended for another reason before this stop; the Runtime \
                     reset nothing for it (not_needed)"
                );
                report::log_quiet(THREAD, &line);
                return;
            }
            Self::Answer(StopOutcome::Stopped {
                reset: Some(Reset::Failed { cause, chain }), ..
            }) => {
                format!("{prefix}: stopped, its reset failed, fenced; cause {cause:?}: {chain}")
            }
            Self::Answer(StopOutcome::Stopped { reset: None, .. }) if first => {
                let line = format!(
                    "{prefix}: still stopping after the wait; asked once more when a Status shows \
                     the run gone"
                );
                report::log_quiet(THREAD, &line);
                return;
            }
            Self::Answer(StopOutcome::Stopped { reset: None, .. }) => {
                format!(
                    "{prefix}: still stopping after its wait too; its result not read, not sent \
                     again"
                )
            }
            Self::Answer(StopOutcome::Refused { failure, why: Refusal::RunUnknown, .. })
                if first =>
            {
                let line = format!(
                    "{prefix}: the run had ended before the stop; nothing was done: {}",
                    failure_line(failure)
                );
                report::log_quiet(THREAD, &line);
                return;
            }
            Self::Answer(StopOutcome::Refused { failure, why, origin, denied }) => {
                let how = if *denied { "denied" } else { "failed" };
                format!(
                    "{prefix}: refused, {how}, {why:?}, run origin {origin:?}: {}",
                    failure_line(failure)
                )
            }
            Self::Answer(StopOutcome::NotSent(failure)) => {
                format!("{prefix}: not sent: {}", failure_line(failure))
            }
            Self::Answer(StopOutcome::Unconfirmed(failure)) => {
                format!("{prefix}: unconfirmed{}: {}", then(first), failure_line(failure))
            }
            Self::Panicked(message) => {
                format!("{prefix}: panicked, unconfirmed{}: {message}", then(first))
            }
            Self::NoThread(error) => format!("{prefix}: no thread, not sent: {error}"),
        };
        report::log(THREAD, &line);
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

/// The log's words for one request: its ticket, the account, the run it stops and its package,
/// and of a follow-up the first's ticket.
fn prefix(alias: &str, request: &Request) -> String {
    let package = request.package.as_deref().unwrap_or("(none named)");
    let mut prefix = format!(
        "ticket {} of {alias}, stop run {} of package {package}",
        request.ticket.number(),
        request.run.line()
    );
    if let Some(first) = request.follows {
        prefix.push_str(&format!(", the follow-up of ticket {}", first.number()));
    }
    prefix
}

/// "Stop this run" on the bar of the account whose picture shows.
pub fn pressed(ui: &Ui) {
    let alias = ui.model.borrow().picture_account().map(str::to_owned);
    if let Some(alias) = alias {
        press(ui, &alias);
    }
}

/// "Stop this run" of `alias` pressed: nothing when its button cannot be pressed now, which the
/// newest `Status` and the account's own stops decide (`Model::stop_target`); else the run that
/// `Status` names is stopped, once, at once. The device's question asked goes, as any other
/// press of the bar's makes it go.
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
            epoch: target.epoch,
            follows: None,
        };
        model.stops_mut().begin(alias, &request).then_some(request)
    };
    if let Some(request) = request {
        send(ui, launcher.state_root.clone(), alias, request);
    }
}

/// `request` of `alias`, once, on a thread of its own. A thread the system refuses is its answer
/// at once, in red; a panic on it is an answer as well: with none at all the button would stay off
/// for good.
fn send(ui: &Ui, root: PathBuf, alias: &str, request: Request) {
    let (worker_alias, worker_request) = (alias.to_owned(), request.clone());
    let spawned = spawn_worker(THREAD, move || {
        let (alias, request) = (worker_alias, worker_request);
        let landing = match catch_unwind(AssertUnwindSafe(|| stop_run(&root, &alias, request.run))) {
            Ok(outcome) => Landing::Answer(outcome),
            Err(panic) => Landing::Panicked(panic_message(&*panic)),
        };
        landing.log(&alias, &request);
        report::post(THREAD, move || {
            if let Some(ui) = ui::current() {
                landed(&ui, &alias, request.ticket, landing);
            }
        });
    });
    match spawned {
        Ok(()) => repaint(ui),
        Err(error) => {
            let landing = Landing::NoThread(error.to_string());
            landing.log(alias, &request);
            landed(ui, alias, request.ticket, landing);
        }
    }
}

/// An answer, on the event loop. One whose ticket is no longer the one out, which a restart let go,
/// changes nothing shown: only the log had it. One that counts is a write: the reads begun before
/// it are let go, and read again, so that only a `Status` begun after it decides its follow-up.
/// With the reading thread gone, a stop that would wait for one ends at once.
fn landed(ui: &Ui, alias: &str, ticket: RescueTicket, landing: Landing) {
    let at_ms = paint::now_ms();
    let (counted, reading_gone) = {
        let mut model = ui.model.borrow_mut();
        let seen = model.account_run(alias);
        let reading_gone = matches!(model.link(), Link::ThreadFailed);
        (model.stops_mut().settle(alias, ticket, landing, at_ms, seen), reading_gone)
    };
    let Some(cools) = counted else {
        let line = format!(
            "ticket {} of {alias}, stop run: its answer came after a restart let it go, and counts \
             for nothing",
            ticket.number()
        );
        report::log_quiet(THREAD, &line);
        ui.paint_log_notes();
        return;
    };
    if reading_gone {
        lane_lost(ui);
    }
    if cools {
        cool_down(alias.to_owned(), ticket);
    }
    super::wrote(ui);
    repaint(ui);
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
            repaint(&ui);
        }
    });
}

/// What a press, an answer or a cooldown may have changed in the window: the rescue bar, which
/// `paint::refresh` paints with the picture's refresh.
fn repaint(ui: &Ui) {
    paint::refresh(ui);
}
