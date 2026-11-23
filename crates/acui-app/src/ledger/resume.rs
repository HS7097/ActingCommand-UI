// SPDX-License-Identifier: GPL-3.0-only
//! A person's conditional resume of a scheduling pause, in the ledger window
//! (B.4, V11; design-final §10). The window lists the pauses of its last status
//! read, the global one first, then each instance's, each by its own scope. A
//! pause another tool set offers "resume scheduling" in two steps, as the
//! unlock entry does: the first press only asks for the confirmation, and only
//! the confirming press sends anything. That press goes out once, on a thread
//! of its own, over a fresh person's connection: a resume of exactly the pause
//! the line showed, by the owner epoch and the revision of the status read it
//! came from, so the Runtime refuses it once that pause has changed. Whatever
//! comes back — lifted, refused, or not confirmed — nothing is sent again: the
//! window's reader reads the status again on a connection of its own, and the
//! window is painted from that. The line says what that read found: a read
//! that failed, or one never made, leaves whether scheduling is paused
//! unknown, and the line says so in red, never that the status was read.
//! The line belongs to the session of the window it came back in: a reopen
//! starts the new session without it, which reads the status itself.
//!
//! Three kinds of pause offer no resume, only why: one whose reason the settings
//! file lists (`resume_elsewhere_reasons`), which only whoever set it lifts; a
//! deployment's, which the installer lifts; and one the Lab's rescue module set
//! for an operation, which the operation lifts as it finishes, but for a stop's
//! (`down`), which stays until it is lifted from the Lab. The
//! contract keeps no actor with a pause and no prefix of its reason, so all are
//! known by the reason alone. While that line cannot be taken, every pause but
//! a deployment's, a Lab rescue's and this console's own two, "pause all", a global
//! one, and "pause this account", an instance's, is of the first kind
//! (`ResumeElsewhere`).
//!
//! Of the pauses set for a person to lift after looking, the one that holds a
//! newly added instance (`instance.added`, the contract's
//! `INSTANCE_ADDED_PAUSE_REASON`, Runtime 945727d4
//! `crates/actingcommand-contract/src/runtime.rs:2477`), which no operation of
//! the Lab's rescue module lifts as it does its `lab-<word>-<ms>` ones, and the
//! hold the Runtime's start sets on an instance whose settlement facts it could
//! not record (`policy_settlement_facts_skipped`, Runtime ISO-START-1) are
//! neither a deployment's nor a Lab rescue's: this window offers their resume
//! (`Lifter::Window`), unless the settings file lists their reason or that line
//! cannot be taken, when they are of the first kind (`Lifter::Setter`). The
//! pinned Runtime sets both: `instance.added` at the first start that
//! sees an `instances[]` entry declaring `start_paused`
//! (`crates/runtime-host/src/host/start_pause.rs:3-9, 110-119`); on such an
//! entry that start found held by a pause of another reason, when that pause is
//! released while the Runtime runs, a person's resume from this window included
//! (`start_pause.rs:15-20`), so it is not always the instance's first pause;
//! and from the Lab's `instance add` on the instance it has just added
//! (`crates/lab-rescue/src/rescue.rs:1739-1823`); the settlement hold at a
//! start (`crates/runtime-host/src/host/runtime_facts.rs:1162-1178`). A newly
//! added instance's line has words of its own (`instance_added`); who lifts it,
//! and whether it is offered, are as for another tool's.
//!
//! This console's own two are said as its own, not another tool's, and are
//! resumed here as those are. Whether a pause still on its way is offered is
//! the Runtime's rule, by the mode its status answers since 0.12.75 (#480
//! PG-2e): a person's pause, this console's own or another party's, in any
//! stage, and a tool's hold only once released; from a Runtime before 0.12.75,
//! which answers no mode, this console's own of an instance in any stage, and
//! another's still on its way gets no resume (`pause_text`). Who lifts a pause
//! is still its reason's to say, as below: the mode never is.
//!
//! The hold of a Lab recording (Runtime 0.12.64), the pause the Lab's `record
//! start` sets on the instance it records (`lab-record-<ms>`,
//! `lab_record_started`), is not a Lab rescue's: the recording's end lifts
//! it, but a pause has no expiry, and a recording nobody ends holds its instance
//! until a resume lifts it. So this window still offers its resume
//! (`Lifter::LabRecord`), an instance's only, as the Lab sets it on no other
//! scope, and unless the settings file lists its reason or that line cannot be
//! taken. Its line says how long ago the hold began, by its reason, and its
//! entry is worded apart from another tool's; the first press adds to the line
//! what a resume does to a recording that has not ended: each press it makes
//! after warns that the instance is not held, and what it records may mix in a
//! scheduled task's screens.

use acui_rows::format_full;
use acui_source::workbench::{
    lab_record_started, CONSOLE_PAUSE_ACCOUNT, CONSOLE_PAUSE_ALL, INSTANCE_ADDED_PAUSE_REASON,
};
use acui_source::{
    resume_paused, resume_taken, resumed_device, ClientFailure, PauseFact, PauseStage,
    PauseTarget, ResumeCause, ResumeOutcome, ResumedDevice,
};

use crate::ledger::paint::{age_text, code_short, models};
use crate::ledger::state::{InstancesSnapshot, LedgerState, Phase, StatusReread};
use crate::ledger::worker::Command;
use crate::ledger::{find, LedgerHandle};
use crate::strings::{fill, outcome_informs, outcome_short, Labels};
use crate::workbench::card::shown_in_the_way;
use crate::workbench::paint::in_the_way_text;
use crate::{launcher, report, ui, LedgerWindow, PauseItem};

/// The prefix of the reason of a deployment's pause, which the installer lifts:
/// the operator's own deployments.
const DEPLOY_PREFIX: &str = "deploy-";

/// The words of the Lab rescue module's own pause reasons, `lab-<word>-<unix ms>`,
/// one per operation that pauses (since 0.12.48): Runtime
/// `crates/lab-rescue/src/rescue.rs`, `hold_reason` and its only callers, `down`,
/// `deploy` and `rollback`, `config replace` and `config restore`, `place`, and
/// (0.12.52 on, INST-5) `instance add`, whose word is `instance`
/// (`contracts/install-cli.md`, "Instances, the first slice"); it hands back only
/// its own (`contracts/install-cli.md`, "The Lab's rescue module"). The instance
/// a person adds is then paused with `instance.added`, which is not of this shape
/// and keeps its resume.
const LAB_RESCUE_WORDS: [&str; 7] =
    ["config", "deploy", "down", "instance", "place", "restore", "rollback"];

/// Whether `reason` is exactly of that shape: `lab-`, one of those words, `-`, digits.
fn lab_rescue(reason: &str) -> bool {
    let Some((word, ms)) = reason.strip_prefix("lab-").and_then(|r| r.split_once('-')) else {
        return false;
    };
    LAB_RESCUE_WORDS.contains(&word) && !ms.is_empty() && ms.bytes().all(|b| b.is_ascii_digit())
}

/// The pauses only whoever set them lifts, by their reasons: the settings
/// file's `resume_elsewhere_reasons` (`settings`), none by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeElsewhere {
    /// The reasons the line lists, each one the Runtime takes; none without it.
    Listed(Vec<String>),
    /// The line, or the file, could not be read, so which reasons it lists is
    /// not known: until it is fixed, every pause but this console's own is one
    /// (`console_set`).
    Unusable,
}

impl Default for ResumeElsewhere {
    fn default() -> Self {
        Self::Listed(Vec::new())
    }
}

impl ResumeElsewhere {
    fn names(&self, pause: &PauseFact) -> bool {
        match self {
            Self::Listed(reasons) => reasons.iter().any(|listed| *listed == pause.reason),
            Self::Unusable => !console_set(pause),
        }
    }
}

/// Whether this console set `pause` itself, by its reason and its scope together: its
/// "pause all" is global only (`pause_global`), its "pause this account" an instance's only
/// (`pause_account`). Either reason on the other scope is another tool's.
fn console_set(pause: &PauseFact) -> bool {
    match pause.target.instance_alias() {
        None => pause.reason == CONSOLE_PAUSE_ALL,
        Some(_) => pause.reason == CONSOLE_PAUSE_ACCOUNT,
    }
}

/// Whether `pause` is a newly added instance's: an instance's, with the reason
/// `instance.added` (`INSTANCE_ADDED_PAUSE_REASON`). Only its words are its own.
fn instance_added(pause: &PauseFact) -> bool {
    pause.target.instance_alias().is_some() && pause.reason == INSTANCE_ADDED_PAUSE_REASON
}

/// Who lifts a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifter {
    /// Another tool set it: this window may resume it.
    Window,
    /// Whoever set it lifts it, and this console does not (`ResumeElsewhere`).
    Setter,
    Installer,
    /// The Lab's rescue module set it for an operation, which lifts it as it
    /// finishes; a stop's (`down`, Runtime `rescue.rs`, which only holds) and
    /// one left behind are lifted from the Lab (`lab_rescue`).
    LabRescue,
    /// A Lab recording holds the instance it records, by an instance's pause
    /// only, and its end lifts the pause; this window may still resume it.
    /// `started_unix_ms`: when the recording took it, by its reason
    /// (`lab_record_started`).
    LabRecord { started_unix_ms: u64 },
}

pub(crate) fn lifter(pause: &PauseFact, elsewhere: &ResumeElsewhere) -> Lifter {
    let reason = pause.reason.as_str();
    if reason.starts_with(DEPLOY_PREFIX) {
        Lifter::Installer
    } else if lab_rescue(reason) {
        Lifter::LabRescue
    } else if elsewhere.names(pause) {
        Lifter::Setter
    } else if let (Some(started_unix_ms), Some(_)) =
        (lab_record_started(reason), pause.target.instance_alias())
    {
        Lifter::LabRecord { started_unix_ms }
    } else {
        Lifter::Window
    }
}

/// One window's resume: the pause a first press asks to confirm, and the
/// press in flight or the last one's outcome. On the event loop only.
#[derive(Default)]
pub struct Resume {
    confirming: Option<PauseTarget>,
    flight: Option<Flight>,
}

struct Flight {
    target: PauseTarget,
    stage: Stage,
}

enum Stage {
    /// Out, on its own thread.
    Sending,
    /// The system refused the thread: nothing was sent.
    NoThread(String),
    /// It came back, and the status read after it.
    Settled { outcome: ResumeOutcome, reread: Reread },
}

/// The status read after an answer.
enum Reread {
    /// Asked of the window's reader, on a connection of its own; not landed.
    Asked,
    /// The reader's read landed, with what it found.
    Landed(StatusReread),
    /// Not asked: nothing was sent, or the face was not open when the answer
    /// came. A session whose face is not open reads nothing more; a reopen
    /// starts a new one, without this line.
    NotAsked,
}

impl Resume {
    /// Whether a press is out, or its status read has not landed while the
    /// window's face is open: no pause offers a resume meanwhile.
    fn busy(&self, open: bool) -> bool {
        match self.flight.as_ref().map(|flight| &flight.stage) {
            Some(Stage::Sending) => true,
            Some(Stage::Settled { reread: Reread::Asked, .. }) => open,
            Some(
                Stage::Settled { reread: Reread::Landed(_) | Reread::NotAsked, .. }
                | Stage::NoThread(_),
            )
            | None => false,
        }
    }

    /// The status read asked for after the last press landed, with what it
    /// found.
    pub fn reread(&mut self, found: StatusReread) {
        if let Some(Flight { stage: Stage::Settled { reread, .. }, .. }) = &mut self.flight {
            if matches!(reread, Reread::Asked) {
                *reread = Reread::Landed(found);
            }
        }
    }

    /// A new session of the window reads the status itself as it opens, and
    /// starts without the old one's confirmation or the line of a press that
    /// came back, or never went out, in it: that line was the old session's.
    /// A press still out stays, so the window stays busy and nothing is sent
    /// twice; its answer is shown as it lands, with the status read after it
    /// in the session open then.
    pub fn reopened(&mut self) {
        self.confirming = None;
        if !matches!(self.flight, Some(Flight { stage: Stage::Sending, .. })) {
            self.flight = None;
        }
    }
}

/// The pause lines of the window's last status read, the resume entry on each
/// one a person may lift here, and the last press's line; painted with the
/// rest of the window, under its state's lock. The pause each line's entry
/// names is kept in the order painted, for the presses; a confirmation whose
/// pause is no longer offered goes. A status read that failed leaves one red
/// line in their place: whether scheduling is paused is unknown.
pub fn paint(window: &LedgerWindow, ledger: &LedgerHandle, state: &LedgerState, labels: &Labels) {
    let open = matches!(state.phase, Phase::Opening { .. } | Phase::Open(_));
    let mut resume = ledger.resume.borrow_mut();
    let busy = resume.busy(open);
    let mut items = Vec::new();
    let mut offered = Vec::new();
    match pauses(state) {
        Ok(facts) => {
            for fact in facts {
                let by = lifter(fact, &ledger.open.resume_elsewhere);
                let (mut text, resumable) = pause_text(labels, fact, by);
                let target = (resumable && !busy).then(|| fact.target.clone());
                let confirming =
                    target.is_some() && resume.confirming.as_ref() == target.as_ref();
                let recording = matches!(by, Lifter::LabRecord { .. });
                // Once asked, the line says what the resume does to the recording.
                if confirming && recording {
                    text.push_str(labels.pause_lab_record_confirming);
                }
                items.push(PauseItem {
                    text: text.into(),
                    offered: target.is_some(),
                    confirming,
                    recording,
                    failed: false,
                });
                offered.push(target);
            }
        }
        Err(failure) => {
            let (code, operation) = unread(labels, failure);
            items.push(PauseItem {
                text: fill(labels.pauses_unread, &[&code, operation]).into(),
                offered: false,
                confirming: false,
                recording: false,
                failed: true,
            });
            offered.push(None);
        }
    }
    let stale = resume
        .confirming
        .as_ref()
        .is_some_and(|confirming| !offered.iter().flatten().any(|target| target == confirming));
    if stale {
        resume.confirming = None;
    }
    let (said, failed) = line(labels, &resume, state);
    drop(resume);
    *ledger.pauses.borrow_mut() = offered;
    window.set_pauses(models(items));
    window.set_resume_line(said.into());
    window.set_resume_failed(failed);
}

/// The pauses of the last status read, global first: online only, and only
/// while the face is open. A status read that failed has no pauses to show,
/// only its failure.
fn pauses(state: &LedgerState) -> Result<Vec<&PauseFact>, &ClientFailure> {
    let Phase::Open(facts) = &state.phase else {
        return Ok(Vec::new());
    };
    let read = match &facts.instances {
        InstancesSnapshot::Online(Ok(read)) => read,
        InstancesSnapshot::Online(Err(failure)) => return Err(failure),
        InstancesSnapshot::Offline(_) => return Ok(Vec::new()),
    };
    let instances = read
        .instances
        .iter()
        .filter_map(|instance| instance.status.as_ref()?.pause.as_ref());
    Ok(read.global_pause.iter().chain(instances).collect())
}

/// A status read that failed, for a line: the Runtime's refusal code when it
/// answered with one, else the client's, with its short words; and the
/// client's operation.
fn unread(labels: &Labels, failure: &ClientFailure) -> (String, &'static str) {
    let code = failure.runtime_code.as_deref().unwrap_or(failure.code);
    (code_short(labels, code), failure.operation)
}

/// A pause's line, `by` who lifts it, and whether a person may resume it here. `by` decides who
/// is to lift it and the words, and nothing else: one only whoever set it lifts, a deployment's
/// and a Lab rescue's are never offered. Any other, one another tool, this console or a Lab
/// recording set, is offered when the Runtime takes its resume now, by the stage and the mode its
/// `Status` answers ([`resume_taken`]; runtime-host `host/policy_dispatch.rs` 1242-1280,
/// `check_resumable`, at 945727d4): global, or an instance's handed back, in either mode; a
/// person's pause (`stand`) in every stage, another party's too (`actingctl`'s, MCP's, the Lab's
/// `pause` row's), which may stand on its way with no bound, an unconfirmed close's until a
/// person's resume (`contracts/scheduling-pause.md` 227-249); a tool's hold (`quiet_or_lift`, a
/// Lab recording's among them) only once released, as the Runtime refuses it before
/// (`policy_dispatch.rs` 1270-1277). The mode says only whether the Runtime takes it, never whose
/// the pause is: every pause restored at a start answers `stand`, a recording's hold too, which
/// its reason still tells (`scheduling-pause.md` 849-852). A Runtime before 0.12.75 answers no
/// mode (803-806): there this console's own, a
/// person's, is offered in any stage and any other only released, as before. An instance's line
/// still on its way says what it is doing, and what its latest check read in its way as its card
/// says it (`card::shown_in_the_way`). The ledger's records of a pause's life (Runtime 0.12.74) do
/// not move this: they say what happened, not what holds now.
fn pause_text(labels: &Labels, fact: &PauseFact, by: Lifter) -> (String, bool) {
    let scope = scope_text(labels, &fact.target);
    let (revision, since) = (fact.revision.to_string(), format_full(fact.since_unix_ms));
    let values: [&str; 4] = [&scope, &fact.reason, &revision, &since];
    let mut text = match by {
        Lifter::Setter => return (fill(labels.pause_setter, &values), false),
        Lifter::Installer => return (fill(labels.pause_deploy, &values), false),
        Lifter::LabRescue => return (fill(labels.pause_lab_rescue, &values), false),
        Lifter::LabRecord { started_unix_ms } => {
            let began = age_text(labels, started_unix_ms);
            fill(labels.pause_lab_record, &[&scope, &fact.reason, &revision, &since, &began])
        }
        // This console's own, by reason and scope (`console_set`), is not another tool's.
        Lifter::Window if console_set(fact) => fill(labels.pause_console, &values),
        // A newly added instance's pause, an instance's: said as what it is, resumed as another
        // tool's.
        Lifter::Window if instance_added(fact) => fill(labels.pause_instance_added, &values),
        Lifter::Window => fill(labels.pause_other_tool, &values),
    };
    // This console's own, by reason and scope, as its card's button has it: an unconfirmed
    // close's words name that button only then.
    let own = matches!(by, Lifter::Window) && console_set(fact);
    // Without a mode answered, this console's own, a person's pause, in any stage, and any other
    // only once released.
    let resumable = resume_taken(fact.stage, fact.mode).unwrap_or(own);
    let doing = match fact.stage {
        None | Some(PauseStage::Released) => None,
        Some(PauseStage::Draining) => Some(labels.pause_draining),
        Some(PauseStage::Paused) => Some(labels.pause_releasing),
    };
    if let Some(doing) = doing {
        text.push_str(" · ");
        text.push_str(doing);
        if let Some(words) = in_the_way_text(labels, &shown_in_the_way(fact), own) {
            text.push_str(&fill(labels.pause_in_the_way, &[&words]));
        }
    }
    (text, resumable)
}

fn scope_text(labels: &Labels, target: &PauseTarget) -> String {
    match target.instance_alias() {
        None => labels.pause_global.to_owned(),
        Some(alias) => fill(labels.pause_instance, &[alias]),
    }
}

/// The last press's line, and whether it says a failure: in flight; without a
/// thread; or its outcome, then what became of the status read after it.
/// Nothing before the first press.
fn line(labels: &Labels, resume: &Resume, state: &LedgerState) -> (String, bool) {
    let Some(flight) = &resume.flight else {
        return (String::new(), false);
    };
    let scope = scope_text(labels, &flight.target);
    match &flight.stage {
        Stage::Sending => {
            let (epoch, revision) =
                (flight.target.owner_epoch(), flight.target.revision().to_string());
            (fill(labels.resume_sending, &[&scope, &epoch, &revision]), false)
        }
        Stage::NoThread(error) => {
            let why = fill(labels.thread_failed, &[error]);
            (fill(labels.resume_no_thread, &[&scope, &why]), true)
        }
        Stage::Settled { outcome, reread } => {
            let (mut text, failed) = outcome_text(labels, &scope, outcome);
            // Nothing sent, nothing to read again.
            if matches!(outcome, ResumeOutcome::NotSent(_)) {
                return (text, failed);
            }
            let (after, unknown) = reread_text(labels, reread, state);
            text.push_str(&after);
            (text, failed || unknown)
        }
    }
}

/// What became of the status read after an answer, and whether it leaves
/// whether scheduling is paused unknown. Only a read that was made says the
/// pause lines are current: one that failed says its code; a face that does
/// not read, or reads offline, says nothing was read.
fn reread_text(labels: &Labels, reread: &Reread, state: &LedgerState) -> (String, bool) {
    if !matches!(state.phase, Phase::Opening { .. } | Phase::Open(_)) {
        return (labels.resume_not_reread.to_owned(), true);
    }
    match reread {
        Reread::Asked => (labels.resume_rereading.to_owned(), false),
        Reread::Landed(StatusReread::Read) => (labels.resume_reread.to_owned(), false),
        Reread::Landed(StatusReread::Failed(failure)) => {
            let (code, operation) = unread(labels, failure);
            (fill(labels.resume_reread_failed, &[&code, operation]), true)
        }
        Reread::Landed(StatusReread::Offline) => (labels.resume_reread_offline.to_owned(), true),
        // Asked of no reader: whether scheduling is paused is unknown.
        Reread::NotAsked => (labels.resume_not_reread.to_owned(), true),
    }
}

/// What a press came back with, and whether it says a failure: a lift whose
/// device did not connect says so too; one that found the instance's first-start
/// pause not applied yet, which holds it again, or its device held by a player,
/// says that, a failure only by its code's category; and one whose receipt says
/// why it connected nothing, its pause not quiet yet or an iOS handset this
/// Runtime has no backend for, says that, no failure (`resumed_device`).
fn outcome_text(labels: &Labels, scope: &str, outcome: &ResumeOutcome) -> (String, bool) {
    match outcome {
        ResumeOutcome::Resumed { revision, cause, device_failure } => {
            let mut text = fill(labels.resume_done, &[scope, &revision.to_string()]);
            let device = resumed_device(cause.as_ref(), device_failure.as_deref());
            // The receipt's cause decides, whatever the self-check's shape (#480 PG-2d).
            let why = match device {
                ResumedDevice::WithoutReconnect => Some(labels.resume_without_reconnect),
                ResumedDevice::WithoutBackend => Some(labels.resume_without_backend),
                // The emulator was off: nothing to connect yet, no failure.
                ResumedDevice::EmulatorNotRunning => Some(labels.resume_emulator_off),
                _ => None,
            };
            if let Some(why) = why {
                text.push_str(why);
                return (text, false);
            }
            let Some(code) = device_failure else {
                return (text, false);
            };
            let words = match outcome_short(labels, code) {
                Some(short) => format!("{code} · {short}"),
                None => code.clone(),
            };
            let (template, failed) = match device {
                ResumedDevice::HeldAgain => (labels.resume_held_again, !outcome_informs(code)),
                ResumedDevice::PlayerHolds => (labels.resume_player_hold, !outcome_informs(code)),
                ResumedDevice::Fine
                | ResumedDevice::WithoutReconnect
                | ResumedDevice::WithoutBackend
                | ResumedDevice::EmulatorNotRunning
                | ResumedDevice::ReconnectFailed(_) => (labels.resume_device_failed, true),
            };
            text.push_str(&fill(template, &[&words]));
            (text, failed)
        }
        ResumeOutcome::Refused(failure) => {
            (failure_text(labels, scope, labels.resume_failed, failure), true)
        }
        ResumeOutcome::NotSent(failure) => {
            (failure_text(labels, scope, labels.resume_not_sent, failure), true)
        }
        ResumeOutcome::Unconfirmed(failure) => {
            (failure_text(labels, scope, labels.resume_unconfirmed, failure), true)
        }
    }
}

/// The Runtime's refusal code verbatim when it answered with one, else
/// `template`; then the client's own code and operation, the host failure
/// behind a refusal when the receipt names one, and last the short words of
/// the code shown — the code table's, or "not in this console's table".
fn failure_text(labels: &Labels, scope: &str, template: &str, failure: &ClientFailure) -> String {
    let mut text = match &failure.runtime_code {
        Some(runtime_code) => fill(
            labels.resume_refused,
            &[scope, runtime_code, failure.code, failure.operation],
        ),
        None => fill(template, &[scope, failure.code, failure.operation]),
    };
    if let Some((host_code, operation)) = &failure.host {
        text.push_str(&fill(labels.host_failure, &[host_code, operation]));
    }
    let code = failure.runtime_code.as_deref().unwrap_or(failure.code);
    if let Some(short) = outcome_short(labels, code) {
        text.push_str(" · ");
        text.push_str(&short);
    }
    text
}

/// The first press on a pause's entry: only the confirmation is asked for.
/// `index` is the line's place among the pauses as painted.
pub fn asked(ledger: &LedgerHandle, index: i32) {
    let target = picked(ledger, index);
    ledger.resume.borrow_mut().confirming = target;
}

/// The confirming press: the one resume of the pause the line showed, on
/// `acui-resume`, if it is the pause the first press asked about and no other
/// press is out. The line says it is resuming until the answer comes. A
/// closed window sends none: the press can still land during its fade.
pub fn confirmed(ledger: &LedgerHandle, index: i32) {
    if ledger.is_closed() {
        return;
    }
    let target = picked(ledger, index);
    let open = matches!(ledger.lock().phase, Phase::Opening { .. } | Phase::Open(_));
    let mut resume = ledger.resume.borrow_mut();
    let asked_for = resume.confirming.take();
    let Some(target) = target.filter(|target| asked_for.as_ref() == Some(target)) else {
        return;
    };
    if resume.busy(open) {
        return;
    }
    let (root, id, job) = (ledger.open.state_root.clone(), ledger.id, target.clone());
    let sent = launcher::spawn_worker("acui-resume", move || {
        let outcome = resume_paused(&root, &job);
        report::post("acui-resume", move || settled(id, job, outcome));
    });
    let stage = match sent {
        Ok(()) => Stage::Sending,
        Err(error) => Stage::NoThread(error.to_string()),
    };
    resume.flight = Some(Flight { target, stage });
}

/// The pause the entry of the line at `index` names, if that line offers one.
fn picked(ledger: &LedgerHandle, index: i32) -> Option<PauseTarget> {
    let index = usize::try_from(index).ok()?;
    ledger.pauses.borrow().get(index).cloned().flatten()
}

/// The answer, on the event loop. The window keeps it, and unless nothing was
/// sent its reader reads the status again; a face that is not open reads
/// nothing, and the line says so, until a reopen starts a new session
/// without the line. A window closed meanwhile shows nothing more, so the
/// technical log keeps the answer.
fn settled(id: u64, target: PauseTarget, outcome: ResumeOutcome) {
    let (Some(ledger), Some(ui)) = (find(id), ui::current()) else {
        report::log("acui-resume", &log_text(id, &target, &outcome));
        return;
    };
    if ledger.is_closed() {
        report::log("acui-resume", &log_text(id, &target, &outcome));
        return;
    }
    // A cause this console does not read shows on no line: the log keeps it, quietly.
    if let ResumeOutcome::Resumed { cause: Some(ResumeCause::Other(_)), .. } = &outcome {
        report::log_quiet("acui-resume", &log_text(id, &target, &outcome));
    }
    let open = matches!(ledger.lock().phase, Phase::Opening { .. } | Phase::Open(_));
    let ask = open && !matches!(outcome, ResumeOutcome::NotSent(_));
    let reread = if ask { Reread::Asked } else { Reread::NotAsked };
    let stage = Stage::Settled { outcome, reread };
    ledger.resume.borrow_mut().flight = Some(Flight { target, stage });
    if ask {
        ledger.send(Command::RereadInstances);
    }
    ledger.refresh(&ui);
}

/// An answer no window shows any more, for the technical log.
fn log_text(id: u64, target: &PauseTarget, outcome: &ResumeOutcome) -> String {
    let scope = match target.instance_alias() {
        Some(alias) => format!("instance {alias}"),
        None => "global scheduling".to_owned(),
    };
    let said = match outcome {
        ResumeOutcome::Resumed { revision, cause, device_failure } => {
            let mut device = match resumed_device(cause.as_ref(), device_failure.as_deref()) {
                ResumedDevice::Fine => "device failure none".to_owned(),
                ResumedDevice::WithoutReconnect => {
                    "nothing reconnected, its pause not quiet yet, \
                     scheduling_resume_without_reconnect"
                        .to_owned()
                }
                ResumedDevice::WithoutBackend => {
                    "nothing connected, an iOS handset this Runtime has no backend for, \
                     scheduling_resume_without_backend"
                        .to_owned()
                }
                ResumedDevice::HeldAgain => {
                    "held again by its first-start pause, dispatch_paused_instance".to_owned()
                }
                ResumedDevice::PlayerHolds => {
                    "a player holds the device, so nothing reconnected, player_hold_active"
                        .to_owned()
                }
                ResumedDevice::EmulatorNotRunning => {
                    "nothing connected, its emulator not running, instance_not_running".to_owned()
                }
                ResumedDevice::ReconnectFailed(code) => format!("device failure {code}"),
            };
            if let Some(ResumeCause::Other(code)) = cause {
                device.push_str(&format!(", with a cause this console does not read: {code}"));
            }
            format!("resumed at revision {revision}, {device}")
        }
        ResumeOutcome::Refused(failure) => format!(
            "refused {} ({} {})",
            failure.runtime_code.as_deref().unwrap_or("-"),
            failure.code,
            failure.operation
        ),
        ResumeOutcome::NotSent(failure) => {
            format!("not sent: {} ({})", failure.code, failure.operation)
        }
        ResumeOutcome::Unconfirmed(failure) => {
            format!("unconfirmed: {} ({})", failure.code, failure.operation)
        }
    };
    format!(
        "acui-ledger-{id} closed before the resume of {scope} at revision {} came back: {said}",
        target.revision()
    )
}
