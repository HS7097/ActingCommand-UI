// SPDX-License-Identifier: GPL-3.0-only
//! A person's switch of one task package of an account (UI-3f2; Workflow #429 R-8, Runtime
//! 0.12.55): the box at the front of a package's row on the daily task list. What it shows, where
//! that comes from and when it next changes are the account's `PackScheduleView` (read beside the
//! list, `read`), and only that: a press moves no box, a read does. Off writes one open row in the
//! account's own scope that the package is not scheduled; on goes back to normal, withdrawing the
//! account's own rows when the layer below schedules it and else writing an open row that it is
//! (`pack::switch`, the one place that decides). Neither stops a run of it already going.
//!
//! A press goes out on a thread of its own (`THREAD`), over a fresh person's connection, once
//! (`pack::edit`, which sends a `fact_observation_not_newer` once more itself), and names the
//! version of the account's own scope the view it was pressed on gave. One switch of an account is
//! out at a time, of all its packages: the scope has one version, so a second out at once would
//! only meet the first's. Every switch of the account stays off while one is out, and after an
//! answer that may have changed them, or that says another tool did, until a read of them begun
//! after it has come: until then they show what was read before. Such an answer counts as a write
//! for the reads (`super::wrote`): every read begun before it is let go, and read again at once, so
//! that the switches come back within a moment. A conflict of versions is not sent again: what to
//! press depends on what the other tool wrote, so the person looks and presses again. A Runtime
//! that has no such write (`not_implemented`) is kept for its owner epoch, as one whose view said
//! so is, and has no switch in it.
//!
//! What a press came to is said under its row, with when it landed (not on the line that it is
//! out, nor of a conflict, a thread refused or a restart), until that row is pressed again, the
//! list shows another account than it last did, or the Runtime starts again: the list hidden and
//! shown again (another tab or page, the window minimized, a Runtime that stops answering a while)
//! keeps it. One that took says nothing, its row's source line says what holds now. A restart lets
//! go of a press out: its answer, when it lands, is the log's only. Every answer goes in the log,
//! from the thread, with the version it named and its codes: quietly when it took or met another
//! tool's change; as the problem it is otherwise.
//!
//! `Switches` and what it says hold no words, as the `Model` they are part of; `paint` makes them.

use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use acui_source::workbench::pack::{edit, EditAnswer, PackChange, PackOutcome, PackRefusal};

use super::failure_line;
use super::paint;
use super::read::panic_message;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// The switch's thread, also what its lines in the technical log are under.
const THREAD: &str = "acui-act-pack";

/// Each account's switches this run, and the owner epoch whose Runtime said it has none; on the
/// event loop only.
#[derive(Default)]
pub struct Switches {
    /// By alias.
    slots: BTreeMap<String, Slot>,
    /// The account whose list showed last, while it shows and after it is hidden: what its presses
    /// came to stays until the list shows another account (`follow`).
    listed: Option<String>,
    /// The owner epoch whose Runtime answered the view or a write `not_implemented`, or did not
    /// know the view.
    unsupported: Option<String>,
    /// How many presses went out this run: each one's ticket.
    issued: u64,
}

/// One account's: where its one press stands, and what each of its packages' last press came to.
#[derive(Default)]
struct Slot {
    stage: Stage,
    /// By package id.
    said: BTreeMap<String, SwitchSaid>,
}

#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    Out(Request),
    /// Its answer may have changed the switches, or says another tool did: they wait for a read of
    /// them begun after it, which is any read of them that counts from now on (`super::wrote`).
    Behind,
}

/// One press: its ticket; the package, what it writes and the version of the account's own scope
/// it names; and the owner epoch online when it was pressed.
#[derive(Debug, Clone)]
pub struct Request {
    ticket: u64,
    package: String,
    change: PackChange,
    expected: u64,
    epoch: String,
}

/// What a press of a package's switch would write now (`Model::switch_target`).
pub struct SwitchTarget {
    pub package: String,
    pub change: PackChange,
    pub expected: u64,
    pub epoch: String,
}

/// What the last press of a package's switch came to, before its words; each that landed and is
/// said with when, in unix ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchSaid {
    /// It is out.
    Sending,
    /// Another tool changed the account's switches since they were read: nothing changed, and they
    /// are read again.
    Conflict,
    /// Another write of the account's switches landed in the same millisecond, the second time too.
    NotNewer { at_ms: u64 },
    /// The account's switches would hold more rows than the Runtime keeps.
    TooMany { at_ms: u64 },
    /// What the Runtime holds of the account's switches does not read.
    StoredUnreadable { at_ms: u64 },
    /// The Runtime did not take the row: this program's fault, which the log has.
    Invalid { at_ms: u64 },
    /// Any other refusal: its codes, the host's before the Runtime's, for its words.
    Refused { at_ms: u64, codes: Vec<String> },
    /// No connection: nothing was sent.
    NotSent { at_ms: u64 },
    /// Nothing says what became of it: it may have happened.
    Unconfirmed { at_ms: u64 },
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// It was out across a restart: what came of it is not known.
    Restarted,
}

/// What an answer calls for once it landed: whether it counts as a write, so that the reads begun
/// before it are let go and the switches wait for a read after it; whether the Runtime has no such
/// write.
#[derive(Debug, Clone, Copy, Default)]
struct After {
    reread: bool,
    unsupported: bool,
}

impl Switches {
    /// Whether the Runtime of owner epoch `epoch` said it has no switches.
    pub fn unsupported(&self, epoch: &str) -> bool {
        self.unsupported.as_deref() == Some(epoch)
    }

    /// The Runtime of owner epoch `epoch` said its view has no switches: whether that is news.
    pub fn view_unsupported(&mut self, epoch: &str) -> bool {
        if self.unsupported(epoch) {
            return false;
        }
        self.unsupported = Some(epoch.to_owned());
        true
    }

    /// Whether a press of `alias` is out, or its answer waits for a read of the switches.
    pub fn busy(&self, alias: &str) -> bool {
        self.slots.get(alias).is_some_and(|slot| !matches!(slot.stage, Stage::Idle))
    }

    /// What each of `alias`'s packages' last press came to, by package id.
    pub fn said(&self, alias: &str) -> Option<&BTreeMap<String, SwitchSaid>> {
        self.slots.get(alias).map(|slot| &slot.said)
    }

    /// A read of `alias`'s switches that counts came: the one an answer waited for.
    pub fn read(&mut self, alias: &str) {
        if let Some(slot) = self.slots.get_mut(alias) {
            if matches!(slot.stage, Stage::Behind) {
                slot.stage = Stage::Idle;
            }
        }
    }

    /// The list shows `wanted`'s from now on, or nothing (another tab or page, the window minimized,
    /// the Runtime not answering): none waits any more for a read of before, since none counts
    /// after an answer that counted as a write. What each press came to stays through all of that,
    /// and goes only as its row is pressed again (`press`), the Runtime starts again (`restarted`),
    /// or here, as the list shows another account than it last did: then what the presses of the
    /// account it showed came to goes, but for one out.
    pub fn follow(&mut self, wanted: Option<&str>) {
        for slot in self.slots.values_mut() {
            if matches!(slot.stage, Stage::Behind) {
                slot.stage = Stage::Idle;
            }
        }
        let Some(wanted) = wanted else {
            return;
        };
        let left = self.listed.replace(wanted.to_owned());
        if let Some(left) = left.filter(|left| left.as_str() != wanted) {
            if let Some(slot) = self.slots.get_mut(&left) {
                slot.said.retain(|_, said| *said == SwitchSaid::Sending);
            }
        }
    }

    /// A press of `alias` that writes what `target` says: the request to send, none while one of
    /// the account is out or waits for a read. What it came to is, for now, that it is out.
    pub fn press(&mut self, alias: &str, target: SwitchTarget) -> Option<Request> {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if !matches!(slot.stage, Stage::Idle) {
            return None;
        }
        self.issued += 1;
        let request = Request {
            ticket: self.issued,
            package: target.package,
            change: target.change,
            expected: target.expected,
            epoch: target.epoch,
        };
        slot.said.insert(request.package.clone(), SwitchSaid::Sending);
        slot.stage = Stage::Out(request.clone());
        Some(request)
    }

    /// What the press of `alias` under `ticket` came to, landing at `at_ms`: none when that ticket
    /// is not the one out, which a restart let go.
    fn settle(&mut self, alias: &str, ticket: u64, landing: Landing, at_ms: u64) -> Option<After> {
        let slot = self.slots.get_mut(alias)?;
        let request = match mem::take(&mut slot.stage) {
            Stage::Out(request) if request.ticket == ticket => request,
            other => {
                slot.stage = other;
                return None;
            }
        };
        let (said, after) = landing.said(at_ms);
        match said {
            Some(said) => slot.said.insert(request.package.clone(), said),
            None => slot.said.remove(&request.package),
        };
        slot.stage = if after.reread { Stage::Behind } else { Stage::Idle };
        if after.unsupported {
            self.unsupported = Some(request.epoch);
        }
        Some(after)
    }

    /// The Runtime online is of the owner epoch `epoch`, another than before: a press out of
    /// another epoch is let go, said to be out across a restart; every other line goes, as does a
    /// wait for a read, and what another epoch's Runtime said of its switches. The lines for the
    /// log.
    fn restarted(&mut self, epoch: &str) -> Vec<String> {
        if self.unsupported.as_deref().is_some_and(|said| said != epoch) {
            self.unsupported = None;
        }
        let mut lines = Vec::new();
        for (alias, slot) in &mut self.slots {
            slot.said.clear();
            slot.stage = match mem::take(&mut slot.stage) {
                Stage::Out(request) if request.epoch != epoch => {
                    lines.push(format!(
                        "{}: let go, the Runtime started again; what came of it is not known",
                        prefix(alias, &request)
                    ));
                    slot.said.insert(request.package, SwitchSaid::Restarted);
                    Stage::Idle
                }
                // Not reached: no press is of an epoch no `Status` has shown yet.
                Stage::Out(request) => {
                    slot.said.insert(request.package.clone(), SwitchSaid::Sending);
                    Stage::Out(request)
                }
                Stage::Behind | Stage::Idle => Stage::Idle,
            };
        }
        lines
    }
}

/// What one press came to, as its thread hands it back.
enum Landing {
    Answer(EditAnswer),
    /// The thread panicked: whether the write got out is not known.
    Panicked(String),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

impl Landing {
    /// What it came to, before its words, as it lands at `at_ms` (none: nothing to say under the
    /// row), and what it calls for.
    fn said(self, at_ms: u64) -> (Option<SwitchSaid>, After) {
        let reread = After { reread: true, unsupported: false };
        let settled = After::default();
        match self {
            Self::Answer(EditAnswer { outcome, .. }) => match outcome {
                PackOutcome::Edited { .. } => (None, reread),
                PackOutcome::Conflict { .. } => (Some(SwitchSaid::Conflict), reread),
                PackOutcome::Refused { why, failure, .. } => {
                    let said = match why {
                        PackRefusal::NotNewer => {
                            return (Some(SwitchSaid::NotNewer { at_ms }), reread);
                        }
                        PackRefusal::TooMany => SwitchSaid::TooMany { at_ms },
                        PackRefusal::StoredUnreadable => SwitchSaid::StoredUnreadable { at_ms },
                        PackRefusal::Invalid => SwitchSaid::Invalid { at_ms },
                        PackRefusal::Other => {
                            let host = failure.host.map(|(code, _)| code);
                            let codes = host.into_iter().chain(failure.runtime_code).collect();
                            SwitchSaid::Refused { at_ms, codes }
                        }
                    };
                    (Some(said), settled)
                }
                PackOutcome::NotImplemented(_) => {
                    (None, After { reread: false, unsupported: true })
                }
                PackOutcome::NotSent(_) => (Some(SwitchSaid::NotSent { at_ms }), settled),
                PackOutcome::Unconfirmed(_) => (Some(SwitchSaid::Unconfirmed { at_ms }), reread),
            },
            Self::Panicked(_) => (Some(SwitchSaid::Unconfirmed { at_ms }), reread),
            Self::NoThread(error) => (Some(SwitchSaid::NoThread { error }), settled),
        }
    }

    /// Into the technical log, with the version it named and its codes: quietly when it took or
    /// met another tool's change; the rest as the problems they are.
    fn log(&self, alias: &str, request: &Request) {
        let prefix = prefix(alias, request);
        let answer = match self {
            Self::Answer(answer) => answer,
            Self::Panicked(message) => {
                let line = format!("{prefix}: panicked, unconfirmed, read again: {message}");
                report::log(THREAD, &line);
                return;
            }
            Self::NoThread(error) => {
                report::log(THREAD, &format!("{prefix}: no thread, not sent: {error}"));
                return;
            }
        };
        let resent = if answer.resent {
            ", sent once more after fact_observation_not_newer"
        } else {
            ""
        };
        match &answer.outcome {
            PackOutcome::Edited { version } => {
                let line = format!("{prefix}: done{resent}, the list now at version {version}");
                report::log_quiet(THREAD, &line);
            }
            PackOutcome::Conflict { current } => {
                let line = format!(
                    "{prefix}: the list is at version {current:?}, another tool changed it; \
                     nothing changed{resent}, read again, not sent again"
                );
                report::log_quiet(THREAD, &line);
            }
            PackOutcome::Refused { why, failure, field } => {
                let line = format!(
                    "{prefix}: refused{resent}, {why:?}, field {field:?}: {}",
                    failure_line(failure)
                );
                report::log(THREAD, &line);
            }
            PackOutcome::NotImplemented(failure) => {
                let line = format!(
                    "{prefix}: the Runtime of owner epoch {} has no task switches \
                     (EditPackSchedule not_implemented), none offered in that epoch: {}",
                    request.epoch,
                    failure_line(failure)
                );
                report::log(THREAD, &line);
            }
            PackOutcome::NotSent(failure) => {
                report::log(THREAD, &format!("{prefix}: not sent: {}", failure_line(failure)));
            }
            PackOutcome::Unconfirmed(failure) => {
                let line = format!(
                    "{prefix}: unconfirmed{resent}, read again, not sent again: {}",
                    failure_line(failure)
                );
                report::log(THREAD, &line);
            }
        }
    }
}

/// The log's words for one press: its ticket, the account, the package, what it writes and the
/// version it names.
fn prefix(alias: &str, request: &Request) -> String {
    format!(
        "pack ticket {} of {alias}, {} → {}, based on version {}",
        request.ticket,
        request.package,
        request.change.line(),
        request.expected
    )
}

/// The switch of `package` on the daily task list of the account shown, pressed to `on`: nothing
/// when it cannot be pressed now, which the newest read of the switches and the account's own
/// presses decide (`Model::switch_target`); else what it writes is sent, once, at once. The box
/// shows what the reads say either way, which this paints.
pub fn toggled(ui: &Ui, package: &str, on: bool) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let pressed = ui.model.borrow_mut().switch_pressed(package, on);
    match pressed {
        Some((alias, request)) => send(ui, launcher.state_root.clone(), &alias, request),
        None => paint::tasks(ui),
    }
}

/// `request` of `alias`, once, on a thread of its own. A thread the system refuses is its answer
/// at once, in red; a panic on it is an answer as well: with none at all the account's switches
/// would stay off for good.
fn send(ui: &Ui, root: PathBuf, alias: &str, request: Request) {
    let (worker_alias, worker_request) = (alias.to_owned(), request.clone());
    let spawned = spawn_worker(THREAD, move || {
        let (alias, request) = (worker_alias, worker_request);
        let write = || edit(&root, &alias, &request.package, request.change, request.expected);
        let landing = match catch_unwind(AssertUnwindSafe(write)) {
            Ok(answer) => Landing::Answer(answer),
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
        Ok(()) => paint::tasks(ui),
        Err(error) => {
            let landing = Landing::NoThread(error.to_string());
            landing.log(alias, &request);
            landed(ui, alias, request.ticket, landing);
        }
    }
}

/// An answer, on the event loop. One whose ticket is no longer the one out, which a restart let go,
/// changes nothing shown: only the log had it. One that may have changed the switches, or says
/// another tool did, is a write: the reads begun before it are let go, and read again at once. A
/// Runtime that has no such write reads them no more in its owner epoch (`super::tasks_sync`).
fn landed(ui: &Ui, alias: &str, ticket: u64, landing: Landing) {
    let at_ms = paint::now_ms();
    let after = ui.model.borrow_mut().switches_mut().settle(alias, ticket, landing, at_ms);
    let Some(after) = after else {
        let line = format!(
            "pack ticket {ticket} of {alias}: its answer came after a restart let it go, and \
             counts for nothing"
        );
        report::log_quiet(THREAD, &line);
        ui.paint_log_notes();
        return;
    };
    if after.reread {
        super::wrote(ui);
    }
    if after.unsupported {
        super::tasks_sync(ui);
    }
    paint::tasks(ui);
    ui.paint_log_notes();
}

/// A `Status` read online found another owner epoch (`super::live_landed`): every press out of
/// before is let go (`Switches::restarted`). Painting is the caller's.
pub fn restarted(ui: &Ui) {
    let lines = {
        let mut model = ui.model.borrow_mut();
        let Some(epoch) = model.online_epoch().map(str::to_owned) else {
            return;
        };
        model.switches_mut().restarted(&epoch)
    };
    for line in &lines {
        report::log_quiet(THREAD, line);
    }
}
