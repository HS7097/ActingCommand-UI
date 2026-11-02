// SPDX-License-Identifier: GPL-3.0-only
//! What the workbench's top bar writes: "pause all" and "put all on duty".
//!
//! A press takes a ticket for its slot, one out per slot, and the buttons are off
//! meanwhile. It goes out once, on a thread of its own, over a fresh person's
//! connection. Whatever comes back, nothing is sent again by itself: the event loop
//! takes the answer only with the ticket still out, counts a write for the reads
//! (`super::wrote`, which is what lets go of a read begun before it), and shows the
//! newest reads. A restart lets go of every ticket: what was out then may or may not
//! have happened, and the next `Status` says which. Every answer goes in the log,
//! from the thread, so one that lands after the event loop is not lost.
//!
//! Putting all on duty lifts exactly the pause the newest `Status` showed, by its
//! owner epoch and revision (`resume_paused`), and only one a person may lift here
//! (`lifter`): the rest stay off and say who lifts them. A pause another tool set
//! takes two presses, and the first only asks; one pressed here takes one.
//!
//! `view` is a pure function of what the reads and the presses left, without words;
//! `paint` makes the words.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use acui_source::workbench::{pause_global, InstanceLive, PauseOutcome, CONSOLE_PAUSE_ALL};
use acui_source::{resume_paused, ClientFailure, PauseFact, PauseTarget, ResumeOutcome};

use super::model::Link;
use super::read::panic_message;
use super::{failure_line, paint};
use crate::launcher::spawn_worker;
use crate::ledger::resume::{lifter, Lifter};
use crate::report;
use crate::ui::{self, Ui};

/// A pause already holds: it keeps its reason, and this press set nothing.
const ALREADY_PAUSED: &str = "scheduling_already_paused";
/// The pause a press named is no longer the one that holds.
const PAUSE_MOVED: [&str; 3] = [
    "scheduling_pause_revision_mismatch",
    "scheduling_pause_owner_epoch_mismatch",
    "scheduling_not_paused",
];
/// The client's own: a Runtime that does not know the condition lifted nothing.
const RUNTIME_OPERATION_UNSUPPORTED: &str = "runtime_operation_unsupported";

/// What a write applies to. UI-3d and 3e1 add an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Global,
}

/// What kind of write it is. Pausing and resuming are one kind: an answer still out
/// leaves it open whether the other makes sense yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Scheduling,
}

const GLOBAL_SCHEDULING: (Scope, Kind) = (Scope::Global, Kind::Scheduling);

/// The name of one press's request: an answer counts only under the ticket still out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

/// Which button was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    PauseAll,
    ResumeAll,
}

impl Write {
    /// Its thread's name, also what its lines in the technical log are under.
    fn thread(self) -> &'static str {
        match self {
            Self::PauseAll => "acui-act-pause",
            Self::ResumeAll => "acui-act-resume",
        }
    }
}

/// What the last press came to, before its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    Sending(Write),
    PauseApplied,
    AlreadyPaused,
    Resumed,
    /// The pause was changed or lifted elsewhere: nothing was lifted here.
    PauseMoved,
    /// `code` is the Runtime's own, for its words.
    Refused { write: Write, code: String },
    /// A Runtime that cannot lift a pause on condition.
    Unsupported,
    Unconfirmed,
    NotSent,
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// A write was out across a restart.
    Restarted,
}

/// The workbench's writes, on the event loop only.
#[derive(Default)]
pub struct Actions {
    issued: u64,
    out: BTreeMap<(Scope, Kind), Ticket>,
    /// The pause a first press asks to confirm.
    confirming: Option<PauseTarget>,
    said: Option<Said>,
}

impl Actions {
    /// A new ticket, unless one of that slot is out. A confirmation ends here.
    fn begin(&mut self, slot: (Scope, Kind), write: Write) -> Option<Ticket> {
        if self.out.contains_key(&slot) {
            return None;
        }
        self.issued += 1;
        let ticket = Ticket(self.issued);
        self.out.insert(slot, ticket);
        self.confirming = None;
        self.said = Some(Said::Sending(write));
        Some(ticket)
    }

    /// An answer: whether `ticket` was the one out, which is all that makes it count.
    fn settle(&mut self, slot: (Scope, Kind), ticket: Ticket, said: Said) -> bool {
        if self.out.get(&slot) != Some(&ticket) {
            return false;
        }
        self.out.remove(&slot);
        self.said = Some(said);
        true
    }

    /// Another owner epoch: no ticket holds across it, and no line says anything of
    /// the old one but that a write was out.
    pub fn new_epoch(&mut self) {
        let was_out = !self.out.is_empty();
        self.out.clear();
        self.confirming = None;
        self.said = was_out.then_some(Said::Restarted);
    }

    /// A `Status` read that counts. A confirmation is of one pause, and goes with it.
    /// A line saying a press worked goes once `Status` says otherwise: the reads begun
    /// before that press were let go, so this says someone changed it since.
    pub fn status_read(&mut self, global: Option<&PauseFact>) {
        if self.confirming.as_ref() != global.map(|pause| &pause.target) {
            self.confirming = None;
        }
        let overtaken = match self.said {
            Some(Said::PauseApplied) => global.is_none(),
            Some(Said::Resumed) => global.is_some(),
            _ => false,
        };
        if overtaken {
            self.said = None;
        }
    }

    /// No `Status` answers: nothing to confirm against.
    pub fn status_lost(&mut self) {
        self.confirming = None;
    }
}

/// "Put all on duty" now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeEntry {
    /// Nothing to lift here now: no pause, a write out, no Runtime, or one only
    /// someone else lifts (`ActionsView::holding`).
    Off,
    /// A pause pressed here: one press lifts it.
    OnePress(PauseTarget),
    /// A pause another tool set: the first press asks.
    TwoSteps(PauseTarget),
    /// The first press asked, about this pause and its reason as written.
    Confirming { target: PauseTarget, reason: String },
}

/// The top bar's writes as they stand.
#[derive(Debug, Clone)]
pub struct ActionsView {
    pub can_pause: bool,
    pub resume: ResumeEntry,
    /// Who lifts the global pause, while one holds.
    pub holding: Option<Lifter>,
    pub said: Option<Said>,
    /// Accounts under a pause of their own, which lifting the global one leaves, and
    /// whether Alice lifts any of those.
    pub still_paused: usize,
    pub alice_holds: bool,
}

pub fn view(
    link: &Link,
    global: Option<&PauseFact>,
    instances: &[InstanceLive],
    actions: &Actions,
) -> ActionsView {
    let open = matches!(link, Link::Online { .. }) && !actions.out.contains_key(&GLOBAL_SCHEDULING);
    let resume = match global {
        Some(pause) if open && lifter(&pause.reason) == Lifter::Window => {
            if actions.confirming.as_ref() == Some(&pause.target) {
                ResumeEntry::Confirming {
                    target: pause.target.clone(),
                    reason: pause.reason.clone(),
                }
            } else if pause.reason == CONSOLE_PAUSE_ALL {
                ResumeEntry::OnePress(pause.target.clone())
            } else {
                ResumeEntry::TwoSteps(pause.target.clone())
            }
        }
        _ => ResumeEntry::Off,
    };
    let paused = || instances.iter().filter_map(|instance| instance.pause.as_ref());
    ActionsView {
        can_pause: open && global.is_none(),
        resume,
        holding: global.map(|pause| lifter(&pause.reason)),
        said: actions.said.clone(),
        still_paused: paused().count(),
        alice_holds: paused().any(|pause| lifter(&pause.reason) == Lifter::Alice),
    }
}

/// What a write's thread hands back.
enum Written {
    Pause(PauseOutcome),
    Resume(ResumeOutcome),
    /// The write panicked: whether it got out is not known.
    Panicked(String),
    NoThread(String),
}

impl Written {
    fn said(self, write: Write) -> Said {
        match self {
            Self::Pause(PauseOutcome::Applied { .. }) => Said::PauseApplied,
            Self::Pause(PauseOutcome::Refused(failure)) => {
                let code = refusal(&failure);
                if code == ALREADY_PAUSED {
                    Said::AlreadyPaused
                } else {
                    Said::Refused { write, code: code.to_owned() }
                }
            }
            Self::Resume(ResumeOutcome::Resumed { .. }) => Said::Resumed,
            Self::Resume(ResumeOutcome::Refused(failure)) => {
                let code = refusal(&failure);
                if failure.code == RUNTIME_OPERATION_UNSUPPORTED {
                    Said::Unsupported
                } else if PAUSE_MOVED.contains(&code) {
                    Said::PauseMoved
                } else {
                    Said::Refused { write, code: code.to_owned() }
                }
            }
            Self::Pause(PauseOutcome::NotSent(_)) | Self::Resume(ResumeOutcome::NotSent(_)) => {
                Said::NotSent
            }
            Self::Pause(PauseOutcome::Unconfirmed(_))
            | Self::Resume(ResumeOutcome::Unconfirmed(_))
            | Self::Panicked(_) => Said::Unconfirmed,
            Self::NoThread(error) => Said::NoThread { error },
        }
    }

    /// Into the technical log: what went as it should, and the refusals that only
    /// say the state moved on, without lighting up the log's place on the windows;
    /// the rest as the problems they are.
    fn log(&self, write: Write, ticket: Ticket) {
        let prefix = format!("ticket {}", ticket.0);
        match self {
            Self::Pause(PauseOutcome::Applied { revision }) => report::log_quiet(
                write.thread(),
                &format!("{prefix}: paused at revision {revision}"),
            ),
            Self::Resume(ResumeOutcome::Resumed { revision, .. }) => report::log_quiet(
                write.thread(),
                &format!("{prefix}: resumed at revision {revision}"),
            ),
            Self::Pause(PauseOutcome::Refused(failure))
            | Self::Resume(ResumeOutcome::Refused(failure)) => {
                let code = refusal(failure);
                let line = format!("{prefix}: refused: {}", failure_line(failure));
                if code == ALREADY_PAUSED || PAUSE_MOVED.contains(&code) {
                    report::log_quiet(write.thread(), &line);
                } else {
                    report::log(write.thread(), &line);
                }
            }
            Self::Pause(PauseOutcome::NotSent(failure))
            | Self::Resume(ResumeOutcome::NotSent(failure)) => report::log(
                write.thread(),
                &format!("{prefix}: not sent: {}", failure_line(failure)),
            ),
            Self::Pause(PauseOutcome::Unconfirmed(failure))
            | Self::Resume(ResumeOutcome::Unconfirmed(failure)) => report::log(
                write.thread(),
                &format!("{prefix}: unconfirmed, not sent again: {}", failure_line(failure)),
            ),
            Self::Panicked(message) => report::log(
                write.thread(),
                &format!("{prefix}: panicked, unconfirmed, not sent again: {message}"),
            ),
            Self::NoThread(error) => {
                report::log(write.thread(), &format!("{prefix}: no thread, not sent: {error}"));
            }
        }
    }
}

/// The code a refusal is known by: the host's own when the receipt names it, which
/// is the one that says why; else the class, else the client's.
fn refusal(failure: &ClientFailure) -> &str {
    failure
        .host
        .as_ref()
        .map(|(code, _)| code.as_str())
        .or(failure.runtime_code.as_deref())
        .unwrap_or(failure.code)
}

/// The "pause all" button. The press may land after the read that turned it off.
pub fn pause_pressed(ui: &Ui) {
    let can_pause = ui.model.borrow().actions_view().can_pause;
    if can_pause {
        send(ui, Write::PauseAll, |root| Written::Pause(pause_global(root)));
    }
}

/// The "put all on duty" button: at once, or the first of two presses.
pub fn resume_pressed(ui: &Ui) {
    let entry = ui.model.borrow().actions_view().resume;
    match entry {
        ResumeEntry::OnePress(target) => send_resume(ui, target),
        ResumeEntry::TwoSteps(target) => {
            ui.model.borrow_mut().actions_mut().confirming = Some(target);
            paint::actions(ui);
        }
        ResumeEntry::Confirming { .. } | ResumeEntry::Off => {}
    }
}

/// The confirming press: of the pause the first press asked about, if it still is the
/// one that holds, since a `Status` that says otherwise ended the confirmation.
pub fn resume_confirmed(ui: &Ui) {
    let entry = ui.model.borrow().actions_view().resume;
    if let ResumeEntry::Confirming { target, .. } = entry {
        send_resume(ui, target);
    }
}

fn send_resume(ui: &Ui, target: PauseTarget) {
    send(ui, Write::ResumeAll, move |root| Written::Resume(resume_paused(root, &target)));
}

/// One write, once, on a thread of its own. A thread the system refuses is that
/// press's answer at once, in red. A panic in the write is an answer as well: with no
/// answer at all the buttons would stay off for good.
fn send(ui: &Ui, write: Write, work: impl FnOnce(&Path) -> Written + Send + 'static) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let Some(ticket) = ui.model.borrow_mut().actions_mut().begin(GLOBAL_SCHEDULING, write) else {
        return;
    };
    let root = launcher.state_root.clone();
    let spawned = spawn_worker(write.thread(), move || {
        let written = match catch_unwind(AssertUnwindSafe(|| work(&root))) {
            Ok(written) => written,
            Err(panic) => Written::Panicked(panic_message(&*panic)),
        };
        written.log(write, ticket);
        report::post(write.thread(), move || {
            if let Some(ui) = ui::current() {
                landed(&ui, write, ticket, written);
            }
        });
    });
    match spawned {
        Ok(()) => paint::actions(ui),
        Err(error) => {
            let written = Written::NoThread(error.to_string());
            written.log(write, ticket);
            landed(ui, write, ticket, written);
        }
    }
}

/// An answer, on the event loop. One whose ticket a restart let go changes nothing
/// shown: only the log had it.
fn landed(ui: &Ui, write: Write, ticket: Ticket, written: Written) {
    let said = written.said(write);
    let counted = ui.model.borrow_mut().actions_mut().settle(GLOBAL_SCHEDULING, ticket, said);
    if counted {
        super::wrote(ui);
        paint::actions(ui);
    } else {
        report::log_quiet(
            write.thread(),
            &format!("ticket {}: its answer came after a restart, and counts for nothing", ticket.0),
        );
    }
    ui.paint_log_notes();
}
