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
//! Two kinds of pause offer no resume, only why: the one Alice lifts herself
//! (`alice-116-scheduler-stop`), and a deployment's, which the installer
//! lifts. The contract keeps no actor with a pause and no prefix of its
//! reason, so both are known by the reason alone.

use acui_rows::format_full;
use acui_source::{
    resume_paused, ClientFailure, PauseFact, PauseStage, PauseTarget, ResumeOutcome,
};

use crate::ledger::paint::{code_short, models};
use crate::ledger::state::{InstancesSnapshot, LedgerState, Phase, StatusReread};
use crate::ledger::worker::Command;
use crate::ledger::{find, LedgerHandle};
use crate::strings::{fill, outcome_short, Labels};
use crate::{launcher, report, ui, LedgerWindow, PauseItem};

/// The reason of the pause only Alice lifts (decision 116).
const ALICE_STOP: &str = "alice-116-scheduler-stop";
/// The reasons of a deployment's pause, which the installer lifts: acsetup's
/// update (UI-2c1) and `install restart` without `--hold` (R-15b v4); and the
/// operator's own deployments, by their prefix.
const DEPLOY_REASONS: [&str; 2] = ["acsetup_update", "install_restart"];
const DEPLOY_PREFIX: &str = "deploy-";

/// Who lifts a pause.
enum Lifter {
    /// Another tool set it: this window may resume it.
    Window,
    Alice,
    Installer,
}

fn lifter(reason: &str) -> Lifter {
    if reason == ALICE_STOP {
        Lifter::Alice
    } else if DEPLOY_REASONS.contains(&reason) || reason.starts_with(DEPLOY_PREFIX) {
        Lifter::Installer
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
                let (text, resumable) = pause_text(labels, fact);
                let target = (resumable && !busy).then(|| fact.target.clone());
                let confirming =
                    target.is_some() && resume.confirming.as_ref() == target.as_ref();
                items.push(PauseItem {
                    text: text.into(),
                    offered: target.is_some(),
                    confirming,
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

/// A pause's line, and whether a person may resume it here: one another tool
/// set, global, or an instance's whose device is handed back — the Runtime
/// refuses the resume of one still draining or handing it back.
fn pause_text(labels: &Labels, fact: &PauseFact) -> (String, bool) {
    let scope = scope_text(labels, &fact.target);
    let (revision, since) = (fact.revision.to_string(), format_full(fact.since_unix_ms));
    let values: [&str; 4] = [&scope, &fact.reason, &revision, &since];
    match lifter(&fact.reason) {
        Lifter::Alice => (fill(labels.pause_alice, &values), false),
        Lifter::Installer => (fill(labels.pause_deploy, &values), false),
        Lifter::Window => {
            let mut text = fill(labels.pause_other_tool, &values);
            let resumable = match fact.stage {
                None | Some(PauseStage::Released) => true,
                Some(PauseStage::Draining) => {
                    text.push_str(labels.pause_draining);
                    false
                }
                Some(PauseStage::Paused) => {
                    text.push_str(labels.pause_releasing);
                    false
                }
            };
            (text, resumable)
        }
    }
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
/// device did not connect says so too.
fn outcome_text(labels: &Labels, scope: &str, outcome: &ResumeOutcome) -> (String, bool) {
    match outcome {
        ResumeOutcome::Resumed { revision, device_failure } => {
            let mut text = fill(labels.resume_done, &[scope, &revision.to_string()]);
            if let Some(code) = device_failure {
                let words = match outcome_short(labels, code) {
                    Some(short) => format!("{code} · {short}"),
                    None => code.clone(),
                };
                text.push_str(&fill(labels.resume_device_failed, &[&words]));
            }
            (text, device_failure.is_some())
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
        ResumeOutcome::Resumed { revision, device_failure } => format!(
            "resumed at revision {revision}, device failure {}",
            device_failure.as_deref().unwrap_or("none")
        ),
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
