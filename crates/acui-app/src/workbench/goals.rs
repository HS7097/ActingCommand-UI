// SPDX-License-Identifier: GPL-3.0-only
//! What an account's Goals tab says (Workflow #455, the goal page; #463 V3), as the reads leave it,
//! and a person's writes on it (G2: the simple controls). No words and no Slint here, `paint` makes
//! both of a view. Its lines are `acui_source::workbench::goals::page`'s, a pure function of the
//! account's `GoalView`, the language shown and the packages' task names, and the one place that
//! decides them; this says only why there are none.
//!
//! A press writes what `acui_source::workbench::goals::press` says, the one place that decides it,
//! of the newest page read and only while that shows and takes writes (not of a record in the old
//! form, `GoalsPage::writable`): a resource entry's amount (typed, then "set"), a `choice`'s
//! option, a `switch`, or a clear. One that loses what the page cannot set back asks first, on its
//! line, with what it loses; only the confirmation sends it, and a press anywhere else, another
//! account or a restart lets go of the question. The confirmation reckons the press again on the
//! newest page read, of the same line: one that no longer writes the same, or no longer asks the
//! same, is not sent, and its line says so; one that does goes naming that page's version.
//! `integer` and `ordered` variables are only shown in this version (G3's).
//!
//! The write goes out on a thread of its own (`THREAD`), over a fresh person's connection, once
//! (`goals::edit`, which sends a `fact_observation_not_newer` once more itself), naming the version
//! of the page it was pressed on. One write of an account is out at a time: the record has one
//! version, so a second out at once would only meet the first's. Every control of the page stays
//! off while one is out, and after an answer that may have changed the page, or that says another
//! tool did, until a read of the page begun after it has come: until then it shows what was read
//! before. Such an answer counts as a write for the reads (`super::wrote`), which are let go and
//! read again at once. A conflict of versions is not sent again: what to press depends on what the
//! other tool wrote. A refusal of the value is said on its line and nothing is read again. A
//! Runtime that has no such write is kept for its owner epoch: the page is only shown in it, which
//! it says once.
//!
//! What a press came to is said on its line, with when it landed (not that it is out, nor of a
//! conflict, a thread refused, a restart or a confirmation the page read again no longer bears
//! out), until that line is pressed again, the tab shows
//! another account than it last did, or the Runtime starts again. One that took says nothing: the
//! line read again says what holds now. Every answer goes in the log, from the thread, with the
//! version it named and its codes.
//!
//! `Writes` and what it says hold no words, as the `Model` they are part of.

use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use acui_source::workbench::goals::{
    edit, GoalAct, GoalWrite, GoalsPage, Loses, Press, WriteAnswer, WriteInvalid, WriteOutcome,
    WriteRefusal,
};
use acui_source::ClientFailure;

use super::card::Down;
use super::failure_line;
use super::model::Fault;
use super::paint;
use super::read::panic_message;
use crate::identity;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// The write's thread, also what its lines in the technical log are under.
const THREAD: &str = "acui-act-goal";

/// What the Goals tab shows now: why no line does, or its lines with what the writes leave of them.
/// A failure is said with its codes, never as an empty page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalsView {
    /// No Runtime answers now.
    Down(Down),
    NoAccount,
    /// The tab shows anew, of another account or another owner epoch: no answer yet.
    Reading,
    /// The read failed: its failure, whose codes the tab says.
    Failed(ClientFailure),
    /// The read panicked, or the reading thread is gone.
    Fault(Fault),
    /// Nothing new came for too long while the window shows.
    Late,
    /// This Runtime has no such read: the code that says so.
    Unsupported(&'static str),
    Read(GoalsPage, WritesNow),
}

/// What the writes leave of the page shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritesNow {
    /// Its controls can be pressed: the page lists, the Runtime takes writes of goals, and no write
    /// of the account is out or waits for a read.
    pub enabled: bool,
    /// The Runtime of this owner epoch said it takes no writes of goals.
    pub unsupported: bool,
    /// What each line's last press came to, by its key.
    pub said: BTreeMap<String, GoalSaid>,
    /// The line that asks to confirm a press, by its key, and what the press loses.
    pub asking: Option<(String, Loses)>,
    /// The amount typed on each resource line, by its key.
    pub amounts: BTreeMap<String, String>,
}

/// What a line's last press came to, before its words; each that landed and is said with when, in
/// unix ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalSaid {
    /// It is out.
    Sending,
    /// Another tool changed the page since it was read: nothing changed, and it is read again.
    Conflict,
    /// Another write of the account's goals landed in the same millisecond, the second time too.
    NotNewer { at_ms: u64 },
    /// The Runtime did not take the value, and why.
    Invalid { at_ms: u64, invalid: WriteInvalid },
    /// Any other refusal: its codes, the host's before the Runtime's.
    Refused { at_ms: u64, codes: Vec<String> },
    /// No connection: nothing was sent. In root mode, because the Runtime up is not the
    /// installation folder's (`not_this_root`, `identity::not_this_root`).
    NotSent { at_ms: u64, not_this_root: bool },
    /// Nothing says what became of it: it may have happened. The page is read again.
    Unconfirmed { at_ms: u64 },
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// It was out across a restart: what came of it is not known.
    Restarted,
    /// Confirmed after the page was read again, which no longer makes the press write or ask what
    /// it asked about: nothing was sent.
    Stale,
}

/// Each account's writes of its goals this run, and the owner epoch whose Runtime said it takes
/// none; on the event loop only.
#[derive(Default)]
pub struct Writes {
    /// By alias.
    slots: BTreeMap<String, Slot>,
    /// The account whose page showed last, while it shows and after it is hidden: what its presses
    /// came to stays until the tab shows another account (`follow`).
    listed: Option<String>,
    /// The owner epoch whose Runtime said it takes no writes of goals.
    unsupported: Option<String>,
    /// How many presses went out this run: each one's ticket.
    issued: u64,
}

/// One account's: where its one write stands, what each line's last press came to, the press that
/// asks to be confirmed, and the amounts typed.
#[derive(Default)]
struct Slot {
    stage: Stage,
    said: BTreeMap<String, GoalSaid>,
    asking: Option<Question>,
    amounts: BTreeMap<String, String>,
}

/// A press that asks to be confirmed: what it writes, the press itself, reckoned again as it is
/// confirmed, and what it loses.
struct Question {
    target: WriteTarget,
    act: GoalAct,
    loses: Loses,
}

#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    Out(GoalRequest),
    /// Its answer may have changed the page, or says another tool did: it waits for a read of the
    /// page begun after it, which is any read of it that counts from now on (`super::wrote`).
    Behind,
}

/// What a press writes: the line's key, the write, the page's version it names, and the owner
/// epoch online when it was pressed.
#[derive(Debug, Clone)]
pub struct WriteTarget {
    pub key: String,
    pub write: GoalWrite,
    pub expected: u64,
    pub epoch: String,
}

/// One press out, under its ticket.
#[derive(Debug, Clone)]
pub struct GoalRequest {
    ticket: u64,
    target: WriteTarget,
}

/// What a press came to at once (`Model::goal_pressed`).
pub enum GoalPressed {
    /// Nothing: it cannot be pressed now, or writes nothing.
    Nothing,
    /// It asks to be confirmed first.
    Asked,
    /// It goes: the account and the request.
    Send(String, GoalRequest),
}

/// What an answer calls for once it landed: whether it counts as a write, so that the reads begun
/// before it are let go and the page waits for a read after it; whether the Runtime has no such
/// write.
#[derive(Debug, Clone, Copy, Default)]
struct After {
    reread: bool,
    unsupported: bool,
}

impl Writes {
    /// Whether the Runtime of owner epoch `epoch` said it takes no writes of goals.
    pub fn unsupported(&self, epoch: &str) -> bool {
        self.unsupported.as_deref() == Some(epoch)
    }

    /// Whether a write of `alias` is out, or its answer waits for a read of the page.
    pub fn busy(&self, alias: &str) -> bool {
        self.slots.get(alias).is_some_and(|slot| !matches!(slot.stage, Stage::Idle))
    }

    /// What the writes leave of the page of `alias` in owner epoch `epoch`, a page that `listed`.
    pub fn now(&self, alias: &str, epoch: &str, listed: bool) -> WritesNow {
        let unsupported = self.unsupported(epoch);
        let slot = self.slots.get(alias);
        WritesNow {
            enabled: listed && !unsupported && !self.busy(alias),
            unsupported,
            said: slot.map(|slot| slot.said.clone()).unwrap_or_default(),
            asking: slot
                .and_then(|slot| slot.asking.as_ref())
                .map(|question| (question.target.key.clone(), question.loses.clone())),
            amounts: slot.map(|slot| slot.amounts.clone()).unwrap_or_default(),
        }
    }

    /// The amount typed on the line `key` of `alias`.
    pub fn amount(&self, alias: &str, key: &str) -> String {
        self.slots
            .get(alias)
            .and_then(|slot| slot.amounts.get(key))
            .cloned()
            .unwrap_or_default()
    }

    /// The amount on the line `key` of `alias` typed as `text`.
    pub fn amount_edited(&mut self, alias: &str, key: &str, text: &str) {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        slot.amounts.insert(key.to_owned(), text.to_owned());
    }

    /// A press `act` of `alias` that asks to be confirmed before it writes `target`, losing
    /// `loses`: it takes the place of any other question, and what the line's last press came to
    /// goes.
    pub fn ask(&mut self, alias: &str, target: WriteTarget, act: GoalAct, loses: Loses) {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        slot.said.remove(&target.key);
        slot.asking = Some(Question { target, act, loses });
    }

    /// The question of `alias`: the key of its line and the press, to be reckoned again on the
    /// newest page read as it is confirmed.
    pub fn question(&self, alias: &str) -> Option<(&str, &GoalAct)> {
        let question = self.slots.get(alias)?.asking.as_ref()?;
        Some((question.target.key.as_str(), &question.act))
    }

    /// The question of `alias` cancelled: whether there was one.
    pub fn cancel(&mut self, alias: &str) -> bool {
        self.slots.get_mut(alias).is_some_and(|slot| slot.asking.take().is_some())
    }

    /// The question of `alias` confirmed in owner epoch `epoch`, its press reckoned again as
    /// `again` on the newest page read, of that page's version: the request to send, naming that
    /// version; none when there was no question, it was asked in another epoch, the Runtime takes
    /// no writes, or a write of the account is out or waits for a read; and none when `again` does
    /// not write what was asked about or does not ask the same (or there is none: the page does
    /// not show or take writes now, the line is gone, or the press writes nothing now), which the
    /// line then says, and the person looks again.
    pub fn confirm(
        &mut self,
        alias: &str,
        epoch: &str,
        again: Option<(Press, u64)>,
    ) -> Option<GoalRequest> {
        let unsupported = self.unsupported(epoch);
        let slot = self.slots.get_mut(alias)?;
        let Question { target, loses, .. } = slot.asking.take()?;
        if unsupported || target.epoch != epoch {
            return None;
        }
        let expected = match again {
            Some((press, version))
                if press.write == target.write && press.asks.as_ref() == Some(&loses) =>
            {
                version
            }
            Some(_) | None => {
                slot.said.insert(target.key, GoalSaid::Stale);
                return None;
            }
        };
        self.press(alias, WriteTarget { expected, ..target })
    }

    /// A press of `alias` that writes `target`: the request to send, none while one of the account
    /// is out or waits for a read. Any question goes; what the line's press came to is, for now,
    /// that it is out.
    pub fn press(&mut self, alias: &str, target: WriteTarget) -> Option<GoalRequest> {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if !matches!(slot.stage, Stage::Idle) {
            return None;
        }
        slot.asking = None;
        self.issued += 1;
        let request = GoalRequest { ticket: self.issued, target };
        slot.said.insert(request.target.key.clone(), GoalSaid::Sending);
        slot.stage = Stage::Out(request.clone());
        Some(request)
    }

    /// What the write of `alias` under `ticket` came to, landing at `at_ms`: none when that ticket
    /// is not the one out, which a restart let go. One that took lets go of the amount typed on its
    /// line, which the page read again shows.
    fn settle(
        &mut self,
        alias: &str,
        ticket: u64,
        landing: Landing,
        at_ms: u64,
    ) -> Option<After> {
        let slot = self.slots.get_mut(alias)?;
        let request = match mem::take(&mut slot.stage) {
            Stage::Out(request) if request.ticket == ticket => request,
            other => {
                slot.stage = other;
                return None;
            }
        };
        let edited = landing.edited();
        let (said, after) = landing.said(at_ms);
        let key = request.target.key;
        if edited {
            slot.amounts.remove(&key);
        }
        match said {
            Some(said) => slot.said.insert(key, said),
            None => slot.said.remove(&key),
        };
        slot.stage = if after.reread { Stage::Behind } else { Stage::Idle };
        if after.unsupported {
            self.unsupported = Some(request.target.epoch);
        }
        Some(after)
    }

    /// A read of the page of `alias` that counts came: the one an answer waited for.
    pub fn read(&mut self, alias: &str) {
        if let Some(slot) = self.slots.get_mut(alias) {
            if matches!(slot.stage, Stage::Behind) {
                slot.stage = Stage::Idle;
            }
        }
    }

    /// The tab shows `wanted`'s page from now on, or nothing: none waits any more for a read of
    /// before, since the page shown anew is read anew. What each press came to, and the amounts
    /// typed, stay through all of that, and go only as the line is pressed again, the Runtime
    /// starts again (`restarted`), or here, as the tab shows another account than it last did:
    /// then those of the account it showed go, but for a press out. A question goes either way.
    pub fn follow(&mut self, wanted: Option<&str>) {
        for slot in self.slots.values_mut() {
            if matches!(slot.stage, Stage::Behind) {
                slot.stage = Stage::Idle;
            }
            slot.asking = None;
        }
        let Some(wanted) = wanted else {
            return;
        };
        let left = self.listed.replace(wanted.to_owned());
        if let Some(left) = left.filter(|left| left.as_str() != wanted) {
            if let Some(slot) = self.slots.get_mut(&left) {
                slot.said.retain(|_, said| *said == GoalSaid::Sending);
                slot.amounts.clear();
            }
        }
    }

    /// The Runtime online is of the owner epoch `epoch`, another than before: a write out of
    /// another epoch is let go, said to be out across a restart; every other line goes, as do a
    /// question, a wait for a read, the amounts typed and what another epoch's Runtime said of its
    /// writes. The lines for the log.
    fn restarted(&mut self, epoch: &str) -> Vec<String> {
        if self.unsupported.as_deref().is_some_and(|said| said != epoch) {
            self.unsupported = None;
        }
        let mut lines = Vec::new();
        for (alias, slot) in &mut self.slots {
            slot.said.clear();
            slot.asking = None;
            slot.amounts.clear();
            slot.stage = match mem::take(&mut slot.stage) {
                Stage::Out(request) if request.target.epoch != epoch => {
                    lines.push(format!(
                        "{}: let go, the Runtime started again; what came of it is not known",
                        prefix(alias, &request)
                    ));
                    slot.said.insert(request.target.key, GoalSaid::Restarted);
                    Stage::Idle
                }
                // Not reached: no press is of an epoch no `Status` has shown yet.
                Stage::Out(request) => {
                    slot.said.insert(request.target.key.clone(), GoalSaid::Sending);
                    Stage::Out(request)
                }
                Stage::Behind | Stage::Idle => Stage::Idle,
            };
        }
        lines
    }
}

/// What one write came to, as its thread hands it back.
enum Landing {
    Answer(WriteAnswer),
    /// The thread panicked: whether the write got out is not known.
    Panicked(String),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

impl Landing {
    /// Whether the Runtime took it.
    fn edited(&self) -> bool {
        matches!(self, Self::Answer(WriteAnswer { outcome: WriteOutcome::Edited { .. }, .. }))
    }

    /// What it came to, before its words, as it lands at `at_ms` (none: nothing to say on the
    /// line), and what it calls for.
    fn said(self, at_ms: u64) -> (Option<GoalSaid>, After) {
        let reread = After { reread: true, unsupported: false };
        let settled = After::default();
        match self {
            Self::Answer(WriteAnswer { outcome, .. }) => match outcome {
                WriteOutcome::Edited { .. } => (None, reread),
                WriteOutcome::Conflict { .. } => (Some(GoalSaid::Conflict), reread),
                WriteOutcome::Refused { why: WriteRefusal::NotNewer, .. } => {
                    (Some(GoalSaid::NotNewer { at_ms }), reread)
                }
                WriteOutcome::Refused { why: WriteRefusal::Invalid(invalid), .. } => {
                    (Some(GoalSaid::Invalid { at_ms, invalid }), settled)
                }
                WriteOutcome::Refused { why: WriteRefusal::Other, failure } => {
                    let host = failure.host.map(|(code, _)| code);
                    let codes = host.into_iter().chain(failure.runtime_code).collect();
                    (Some(GoalSaid::Refused { at_ms, codes }), settled)
                }
                WriteOutcome::Unsupported { .. } => {
                    (None, After { reread: false, unsupported: true })
                }
                WriteOutcome::NotSent(failure) => {
                    let not_this_root = identity::not_this_root(&failure);
                    (Some(GoalSaid::NotSent { at_ms, not_this_root }), settled)
                }
                WriteOutcome::Unconfirmed(_) => (Some(GoalSaid::Unconfirmed { at_ms }), reread),
            },
            Self::Panicked(_) => (Some(GoalSaid::Unconfirmed { at_ms }), reread),
            Self::NoThread(error) => (Some(GoalSaid::NoThread { error }), settled),
        }
    }

    /// Into the technical log, with the version it named and its codes: quietly when it took or
    /// met another tool's change; the rest as the problems they are.
    fn log(&self, alias: &str, request: &GoalRequest) {
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
            WriteOutcome::Edited { version, replaced } => {
                let replaced = match replaced {
                    Some(id) => format!(", replacing the target {id} on its pool"),
                    None => String::new(),
                };
                let line = format!(
                    "{prefix}: done{resent}{replaced}, the record now at version {version}"
                );
                report::log_quiet(THREAD, &line);
            }
            WriteOutcome::Conflict { current } => {
                let line = format!(
                    "{prefix}: the record is at version {current:?}, another tool changed it; \
                     nothing changed{resent}, read again, not sent again"
                );
                report::log_quiet(THREAD, &line);
            }
            WriteOutcome::Refused { why, failure } => {
                let line =
                    format!("{prefix}: refused{resent}, {why:?}: {}", failure_line(failure));
                report::log(THREAD, &line);
            }
            WriteOutcome::Unsupported { code, failure } => {
                let line = format!(
                    "{prefix}: the Runtime of owner epoch {} takes no writes of goals ({code}): the \
                     Goals tab only shows them in that epoch: {}",
                    request.target.epoch,
                    failure_line(failure)
                );
                report::log(THREAD, &line);
            }
            WriteOutcome::NotSent(failure) => {
                report::log(THREAD, &format!("{prefix}: not sent: {}", failure_line(failure)));
            }
            WriteOutcome::Unconfirmed(failure) => {
                let line = format!(
                    "{prefix}: unconfirmed{resent}, read again, not sent again: {}",
                    failure_line(failure)
                );
                report::log(THREAD, &line);
            }
        }
    }
}

/// The log's words for one write: its ticket, the account, what it writes and the version it
/// names.
fn prefix(alias: &str, request: &GoalRequest) -> String {
    format!(
        "goal ticket {} of {alias}, {}, based on version {}",
        request.ticket,
        request.target.write.line(),
        request.target.expected
    )
}

/// The amount on the resource line `key` typed as `text`: kept, so that a paint does not take it
/// back, and the line's "set" follows what it is.
pub fn amount_edited(ui: &Ui, key: &str, text: &str) {
    ui.model.borrow_mut().goal_amount_edited(key, text);
    paint::tasks(ui);
}

/// "Set" on the resource line `key`, of the amount typed.
pub fn set_pressed(ui: &Ui, key: &str) {
    let text = ui.model.borrow().goal_amount(key);
    pressed(ui, key, GoalAct::Amount(text));
}

/// The option at `index` picked on the `choice` line `key`.
pub fn picked(ui: &Ui, key: &str, index: i32) {
    match usize::try_from(index) {
        Ok(index) => pressed(ui, key, GoalAct::Pick(index)),
        Err(_) => paint::tasks(ui),
    }
}

/// The `switch` line `key` turned to `on`.
pub fn toggled(ui: &Ui, key: &str, on: bool) {
    pressed(ui, key, GoalAct::Switch(on));
}

/// "Clear" or "back to the default" on the line `key`.
pub fn clear_pressed(ui: &Ui, key: &str) {
    pressed(ui, key, GoalAct::Clear);
}

/// The question on the page confirmed: what it asked about is sent, once, at once, while the newest
/// page read, in the language shown, still makes the press write and ask the same
/// (`Model::goal_confirmed`); else nothing is sent, and its line says so.
pub fn confirmed(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let language = ui.labels().tongue;
    let request = ui.model.borrow_mut().goal_confirmed(language);
    match request {
        Some((alias, request)) => send(ui, launcher.state_root.clone(), &alias, request),
        None => paint::tasks(ui),
    }
}

/// The question on the page cancelled: nothing is sent.
pub fn cancelled(ui: &Ui) {
    ui.model.borrow_mut().goal_cancelled();
    paint::tasks(ui);
}

/// A press `act` on the line `key` of the page shown: nothing when it cannot be pressed now, which
/// the newest read of the page and the account's own writes decide (`Model::goal_pressed`, of the
/// page in the language shown, whose words the question says); a question on the line when it asks
/// first; else what it writes is sent, once, at once. The controls show what the reads say either
/// way, which this paints.
fn pressed(ui: &Ui, key: &str, act: GoalAct) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let language = ui.labels().tongue;
    let pressed = ui.model.borrow_mut().goal_pressed(key, &act, language);
    match pressed {
        GoalPressed::Send(alias, request) => {
            send(ui, launcher.state_root.clone(), &alias, request);
        }
        GoalPressed::Asked | GoalPressed::Nothing => paint::tasks(ui),
    }
}

/// `request` of `alias`, once, on a thread of its own. A thread the system refuses is its answer
/// at once, in red; a panic on it is an answer as well: with none at all the account's page would
/// stay off for good.
fn send(ui: &Ui, root: PathBuf, alias: &str, request: GoalRequest) {
    let (worker_alias, worker_request) = (alias.to_owned(), request.clone());
    let spawned = spawn_worker(THREAD, move || {
        let (alias, request) = (worker_alias, worker_request);
        let target = &request.target;
        let write = || edit(&root, &alias, &target.write, target.expected);
        let landing = match catch_unwind(AssertUnwindSafe(write)) {
            Ok(answer) => Landing::Answer(answer),
            Err(panic) => Landing::Panicked(panic_message(&*panic)),
        };
        landing.log(&alias, &request);
        let ticket = request.ticket;
        report::post(THREAD, move || {
            if let Some(ui) = ui::current() {
                landed(&ui, &alias, ticket, landing);
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
/// changes nothing shown: only the log had it. One that may have changed the page, or says another
/// tool did, is a write: the reads begun before it are let go, and read again at once.
fn landed(ui: &Ui, alias: &str, ticket: u64, landing: Landing) {
    let at_ms = paint::now_ms();
    let after = ui.model.borrow_mut().goal_writes_mut().settle(alias, ticket, landing, at_ms);
    let Some(after) = after else {
        let line = format!(
            "goal ticket {ticket} of {alias}: its answer came after a restart let it go, and \
             counts for nothing"
        );
        report::log_quiet(THREAD, &line);
        ui.paint_log_notes();
        return;
    };
    if after.reread {
        super::wrote(ui);
    }
    paint::tasks(ui);
    ui.paint_log_notes();
}

/// A `Status` read online found another owner epoch (`super::live_landed`): every write out of
/// before is let go (`Writes::restarted`). Painting is the caller's.
pub fn restarted(ui: &Ui) {
    let lines = {
        let mut model = ui.model.borrow_mut();
        let Some(epoch) = model.online_epoch().map(str::to_owned) else {
            return;
        };
        model.goal_writes_mut().restarted(&epoch)
    };
    for line in &lines {
        report::log_quiet(THREAD, line);
    }
}
