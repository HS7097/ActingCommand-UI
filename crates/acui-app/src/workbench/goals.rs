// SPDX-License-Identifier: GPL-3.0-only
//! What an account's Goals tab says (Workflow #455, the goal page; #463 V3), as the reads leave it,
//! and a person's writes on it (G2: the simple controls). No words and no Slint here, `paint` makes
//! both of a view. Its lines are `acui_source::workbench::goals::page`'s, a pure function of the
//! account's `GoalView`, the language shown and the packages' task names, and the one place that
//! decides them; this says only why there are none.
//!
//! A press writes what `acui_source::workbench::goals::press` says, the one place that decides it,
//! of the newest page read and only while that shows and takes writes (not of a record in the old
//! form, `GoalsPage::writable`): a resource entry's amount or an `integer` variable's number
//! (typed, then "set"), a `choice`'s option, a `switch`, an `ordered` variable's list ("save
//! order"), or a clear. One that loses what the page cannot set back asks first, on its line, with
//! what it loses; only the confirmation sends it, and a press anywhere else, another account or a
//! restart lets go of the question. The confirmation reckons the press again on the newest page
//! read, of the same line: one that no longer writes the same, or no longer asks the same, is not
//! sent, and its line says so; one that does goes naming that page's version. A variable whose
//! declaration this console does not read is only shown, never set: only a value of it that no
//! bound package offers is cleared, which writes no value.
//!
//! An `ordered` variable's list is edited as a draft first (G3), kept here per line, never written
//! by an edit: added to, moved, removed from, as `goals::order_edit` allows, from the keys the next
//! run reads when there is no draft yet, and let go as it comes back to them, as it is discarded,
//! once its save took, or as the tab shows another account or the Runtime starts again. A draft
//! begun on keys the page no longer reads (changed elsewhere since) is not edited further or saved,
//! nor sent by confirming a save asked before, only discarded; nor is the line's draft edited or
//! discarded while its line asks to confirm a press (a save, whose question holds the draft it
//! asked about, or a back to the default). What is typed in a list's search field is kept the same
//! way and writes nothing. A refusal by a variable's rule (`WriteProblem::of_the_rule`) means the
//! page's rule is older than the Runtime's, so the page is read again at once; the item a refusal
//! points at is kept by its key, as sent.
//!
//! The write goes out on a thread of its own (`THREAD`), over a fresh person's connection, once
//! (`goals::edit`, which sends a `fact_observation_not_newer` once more itself), naming the version
//! of the page it was pressed on. One write of an account is out at a time: the record has one
//! version, so a second out at once would only meet the first's. Every control of the page stays
//! off while one is out, and after an answer that may have changed the page, that says another
//! tool did, or that refuses a variable's value by its rule, until a read of the page begun after
//! it has come: until then it shows what was read before. Such an answer counts as a write for the
//! reads (`super::wrote`), which are let go and read again at once. A conflict of versions is not
//! sent again: what to press depends on what the other tool wrote. Any other refusal of the value
//! is said on its line and nothing is read again. A Runtime that has no such write is kept for its
//! owner epoch: the page is only shown in it, which it says once.
//!
//! What a press came to is said on its line, with when it landed (not that it is out, nor of a
//! conflict, a thread refused, a restart or a confirmation the page read again no longer bears
//! out), until that line is pressed again, the tab shows
//! another account than it last did, or the Runtime starts again. One that took says nothing: the
//! line read again says what holds now. Every answer goes in the log, from the thread, with the
//! version it named and its codes.
//!
//! `Writes` and what it says hold no words, as the `Model` they are part of.
//!
//! Workflow #455 RR4b, the records entries (whom to raise, to which grade): their lines are the
//! page's, placed by `acui_source::workbench::goals::page_with`, and a press on them writes what
//! `acui_source::workbench::goal_records::records_press` says, the entry's whole array, naming the
//! records read's version; none asks first. "Remove" is pressed on a record's line. "Add a
//! character" on an entry's line, and "change" on a record's, open its picker under that line
//! (`Picker`, kept per account as what is typed is): the table's options are read then, and the
//! positions of each ladder of the character chosen then, each on a thread of its own
//! (`PICK_THREAD`) over a person's connection, one at a time, never on a clock; what is read is
//! kept by instance, entry and the tables' sha256, and by ladder and sha256 (`Picks`), and let go
//! of under a sha256 the declaration moved from. Options whose lists have expired are read again
//! as the picker opens. Its "save" presses the record its choices make (`picker_view`), on the
//! line it shows under, whose words say what it came to; one that took closes it.

use std::collections::BTreeMap;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use acui_rows::outcome::Tongue;
use acui_source::workbench::goal_records::{
    default_target, pointed_record, read_options, read_positions, to_of, Came, KeyLaneOption,
    Ladder, LaneTarget, OptionsLoaded, PartAnswer, PositionOption, PositionsLoaded, RecordsLine,
    Requirement, Room, TableOptions, RECORDS_KEY, RECORD_KEY,
};
use acui_source::workbench::goals::{
    edit, pointed_key, GoalAct, GoalChange, GoalWrite, GoalsPage, Loses, OrderEdit, Press,
    WriteAnswer, WriteInvalid, WriteOutcome, WriteProblem, WriteRefusal, VARIABLE_KEY,
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
/// The picker's reads' thread (Workflow #455 RR4b), likewise.
const PICK_THREAD: &str = "acui-goal-pick";

/// What the Goals tab shows now: why no line does, or its lines with what the writes leave of them,
/// and (Workflow #455 RR4b) the failure of the read of its records, when that failed: the page is
/// then shown without them, and says so with the codes. A failure is said with its codes, never as
/// an empty page.
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
    Read(GoalsPage, WritesNow, Option<ClientFailure>),
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
    /// The amount or number typed on each resource or `integer` line, by its key.
    pub amounts: BTreeMap<String, String>,
    /// The draft of each `ordered` line, by its key (G3).
    pub orders: BTreeMap<String, OrderDraft>,
    /// What is typed in each `ordered` line's search field, by its key.
    pub searches: BTreeMap<String, String>,
}

/// A draft of an `ordered` variable's list (G3): the keys the next run read when it was begun
/// (`from`), by which a draft begun before the value changed elsewhere is told, and its keys now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderDraft {
    pub from: Vec<String>,
    pub keys: Vec<String>,
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
    /// The Runtime did not take the value, and why; of a list, the key of the item its pointer
    /// names in the list sent, so that it names the same item however the draft moves after.
    Invalid { at_ms: u64, invalid: WriteInvalid, item: Option<String> },
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
/// asks to be confirmed, the amounts typed, the lists' drafts and what their search fields hold,
/// and its records entries' picker while it is open (Workflow #455 RR4b).
#[derive(Default)]
struct Slot {
    stage: Stage,
    said: BTreeMap<String, GoalSaid>,
    asking: Option<Question>,
    amounts: BTreeMap<String, String>,
    orders: BTreeMap<String, OrderDraft>,
    searches: BTreeMap<String, String>,
    picker: Option<Picker>,
}

/// The picker of a records entry (Workflow #455 RR4b), open under one line of an account's page:
/// that line's key (an entry's for "add a character", a record's for "change"); the entry it
/// writes and its table's name, which what is kept of the ladders goes by; the tables' sha256 it
/// reads under, the records read's until its options say theirs; what is typed in its search field
/// and whether it lists every character or only those held; the character chosen, if any; and the
/// choice of each of that character's lanes by its place: absent, its default (`default_target`);
/// none, no target; else the place of a position on the lane's ladder. It writes nothing until
/// "save".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub at: String,
    pub entry: String,
    pub table: String,
    pub sha: Option<String>,
    pub search: String,
    pub all: bool,
    pub key: Option<String>,
    pub choices: BTreeMap<usize, Option<usize>>,
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
            orders: slot.map(|slot| slot.orders.clone()).unwrap_or_default(),
            searches: slot.map(|slot| slot.searches.clone()).unwrap_or_default(),
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

    /// The draft of the `ordered` line `key` of `alias`, while there is one.
    pub fn order(&self, alias: &str, key: &str) -> Option<OrderDraft> {
        self.slots.get(alias).and_then(|slot| slot.orders.get(key)).cloned()
    }

    /// The draft of the `ordered` line `key` of `alias` kept as `draft`; none lets it go.
    pub fn set_order(&mut self, alias: &str, key: &str, draft: Option<OrderDraft>) {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        match draft {
            Some(draft) => {
                slot.orders.insert(key.to_owned(), draft);
            }
            None => {
                slot.orders.remove(key);
            }
        }
    }

    /// The search field of the `ordered` line `key` of `alias` typed as `text`.
    pub fn searched(&mut self, alias: &str, key: &str, text: &str) {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        slot.searches.insert(key.to_owned(), text.to_owned());
    }

    /// The picker of `alias`, while it is open (Workflow #455 RR4b).
    pub fn picker(&self, alias: &str) -> Option<&Picker> {
        self.slots.get(alias)?.picker.as_ref()
    }

    /// The picker of `alias` to change, while it is open.
    pub fn picker_mut(&mut self, alias: &str) -> Option<&mut Picker> {
        self.slots.get_mut(alias)?.picker.as_mut()
    }

    /// The picker of `alias` opened as `picker`, or closed: an opened one takes the place of any
    /// other, and what the line it opens under last came to goes, as a press's does.
    pub fn set_picker(&mut self, alias: &str, picker: Option<Picker>) {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if let Some(picker) = &picker {
            slot.said.remove(&picker.at);
        }
        slot.picker = picker;
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
    /// not show or take writes now, the line is gone, the press writes nothing now, or a list's
    /// draft was begun on keys the page no longer reads), which the line then says, and the
    /// person looks again.
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
    /// line, its list's draft and what its search field holds, which the page read again shows, and
    /// closes the picker open under its line (Workflow #455 RR4b).
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
        let (said, after) = landing.said(at_ms, &request.target);
        let key = request.target.key;
        if edited {
            slot.amounts.remove(&key);
            slot.orders.remove(&key);
            slot.searches.remove(&key);
            if slot.picker.as_ref().is_some_and(|picker| picker.at == key) {
                slot.picker = None;
            }
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
    /// before, since the page shown anew is read anew. What each press came to, the amounts typed,
    /// the lists' drafts and their search fields stay through all of that, and go only as the line
    /// is pressed again, the Runtime starts again (`restarted`), or here, as the tab shows another
    /// account than it last did: then those of the account it showed go, but for a press out, and
    /// so does its picker. A question goes either way.
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
                slot.orders.clear();
                slot.searches.clear();
                slot.picker = None;
            }
        }
    }

    /// The Runtime online is of the owner epoch `epoch`, another than before: a write out of
    /// another epoch is let go, said to be out across a restart; every other line goes, as do a
    /// question, a wait for a read, the amounts typed, the lists' drafts and search fields, the
    /// pickers and what another epoch's Runtime said of its writes. The lines for the log.
    fn restarted(&mut self, epoch: &str) -> Vec<String> {
        if self.unsupported.as_deref().is_some_and(|said| said != epoch) {
            self.unsupported = None;
        }
        let mut lines = Vec::new();
        for (alias, slot) in &mut self.slots {
            slot.said.clear();
            slot.asking = None;
            slot.amounts.clear();
            slot.orders.clear();
            slot.searches.clear();
            slot.picker = None;
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
    /// line), and what it calls for; `target` what it wrote. A variable's value refused by its
    /// rule (G3) says the page's rule is older than the Runtime's, as the page checks what it sends
    /// by the same rules: the page is read again at once. The item a refusal points at in a list
    /// sent is kept by its key.
    fn said(self, at_ms: u64, target: &WriteTarget) -> (Option<GoalSaid>, After) {
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
                    // A records entry's array names its part at fault `/i/...`, a list's `/i`.
                    let records = target.key.starts_with(RECORDS_KEY)
                        || target.key.starts_with(RECORD_KEY);
                    let item = match (&invalid.pointer, &target.write.change) {
                        (Some(pointer), GoalChange::Set(sent)) if records => {
                            pointed_record(sent, pointer)
                        }
                        (Some(pointer), GoalChange::Set(sent)) => pointed_key(sent, pointer),
                        (Some(_) | None, _) => None,
                    };
                    let of_the_rule =
                        target.key.starts_with(VARIABLE_KEY) && invalid.problem.of_the_rule();
                    let after = if of_the_rule { reread } else { settled };
                    (Some(GoalSaid::Invalid { at_ms, invalid, item }), after)
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

/// The amount on the resource line `key`, or the number on the `integer` line `key`, typed as
/// `text`: kept, so that a paint does not take it back, and the line's "set" follows what it is.
pub fn amount_edited(ui: &Ui, key: &str, text: &str) {
    ui.model.borrow_mut().goal_amount_edited(key, text);
    paint::tasks(ui);
}

/// "Set" on the resource or `integer` line `key`, of the amount or number typed.
pub fn set_pressed(ui: &Ui, key: &str) {
    let text = ui.model.borrow().goal_amount(key);
    pressed(ui, key, GoalAct::Amount(text));
}

/// The option at `index` picked on the `choice` line `key`, or on the `switch` line `key` drawn as
/// its two segments while each rule's own value decides (on, off).
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

/// The option of `value` added at the end of the draft of the `ordered` line `key` (G3), as far as
/// it can be now (`Model::goal_order_edit`): nothing is written.
pub fn order_added(ui: &Ui, key: &str, value: &str) {
    order_edited(ui, key, &OrderEdit::Add(value.to_owned()));
}

/// The key at the place `from` of the draft of the `ordered` line `key` moved to the place `to`:
/// whether it moved, which the list's mark follows. A place the window gives that is no place
/// moves nothing.
pub fn order_moved(ui: &Ui, key: &str, from: i32, to: i32) -> bool {
    match (usize::try_from(from), usize::try_from(to)) {
        (Ok(from), Ok(to)) => order_edited(ui, key, &OrderEdit::Move { from, to }),
        _ => {
            paint::tasks(ui);
            false
        }
    }
}

/// The key at the place `place` of the draft of the `ordered` line `key` removed.
pub fn order_removed(ui: &Ui, key: &str, place: i32) {
    match usize::try_from(place) {
        Ok(place) => {
            order_edited(ui, key, &OrderEdit::Remove(place));
        }
        Err(_) => paint::tasks(ui),
    }
}

/// One edit of the draft of the `ordered` line `key`, then the page painted: whether it was made.
fn order_edited(ui: &Ui, key: &str, edit: &OrderEdit) -> bool {
    let language = ui.labels().tongue;
    let made = ui.model.borrow_mut().goal_order_edit(key, edit, language);
    paint::tasks(ui);
    made
}

/// "Discard changes" on the `ordered` line `key`: its draft let go, the keys the next run reads
/// shown again.
pub fn order_discarded(ui: &Ui, key: &str) {
    let language = ui.labels().tongue;
    ui.model.borrow_mut().goal_order_discard(key, language);
    paint::tasks(ui);
}

/// The search field of the `ordered` line `key` typed as `text`: kept, so that a paint does not
/// take it back; it writes nothing.
pub fn order_searched(ui: &Ui, key: &str, text: &str) {
    ui.model.borrow_mut().goal_searched(key, text);
    paint::tasks(ui);
}

/// "Save order" on the `ordered` line `key`: its draft pressed, while there is one that is not
/// stale and is not the keys the next run reads already (`Model::goal_order_draft`); else only
/// painted.
pub fn order_saved(ui: &Ui, key: &str) {
    let language = ui.labels().tongue;
    let keys = ui.model.borrow().goal_order_draft(key, language);
    match keys {
        Some(keys) => pressed(ui, key, GoalAct::Order(keys)),
        None => paint::tasks(ui),
    }
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
/// changes nothing shown: only the log had it. One that may have changed the page, says another
/// tool did, or refuses a variable's value by its rule, is a write: the reads begun before it are
/// let go, and read again at once.
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
/// before is let go (`Writes::restarted`), and so is a picker's read out (`Picks::restarted`), what
/// they kept of the tables staying under their sha256. Painting is the caller's.
pub fn restarted(ui: &Ui) {
    let lines = {
        let mut model = ui.model.borrow_mut();
        let Some(epoch) = model.online_epoch().map(str::to_owned) else {
            return;
        };
        model.goal_picks_mut().restarted();
        model.goal_writes_mut().restarted(&epoch)
    };
    for line in &lines {
        report::log_quiet(THREAD, line);
    }
}

/// What the pickers read (Workflow #455 RR4b), on the event loop only: a table's options by
/// instance, entry and tables' sha256, and a ladder's positions by its table, the ladder and the
/// sha256, kept for the run (`contracts/runtime-interface.md` 892-895 at aa6765dd: a ladder by
/// ladder and sha256, the table's name beside it as two tables of one file may name a class or a
/// key alike); the one read out, under its ticket; and, by account, what the newest read of its
/// picker came to when it holds nothing to show, which holds until the picker is opened anew or
/// another character is chosen, so that nothing is read again in a loop.
#[derive(Default)]
pub struct Picks {
    options: BTreeMap<(String, String, Option<String>), OptionsLoaded>,
    positions: BTreeMap<(Ladder, Option<String>), PositionsLoaded>,
    out: Option<PickRequest>,
    issued: u64,
    said: BTreeMap<String, PickSaid>,
}

/// One read of a picker out: its ticket, the account, the records entry, and what it reads.
#[derive(Debug, Clone)]
pub struct PickRequest {
    ticket: u64,
    pub alias: String,
    pub entry: String,
    pub job: PickJob,
}

/// What a picker's read reads: the entry's table's options, or the positions of these ladders.
#[derive(Debug, Clone)]
pub enum PickJob {
    Options,
    Positions(Vec<Ladder>),
}

/// What a picker's read came to, as its thread hands it back.
pub enum PickAnswer {
    Options(PartAnswer<OptionsLoaded>),
    Positions(Vec<(Ladder, PartAnswer<PositionsLoaded>)>),
    /// The read panicked: its message, for the log.
    Panicked(String),
    /// The system refused the thread. The error is the system's.
    NoThread(String),
}

/// What a picker's read came to that holds nothing to show, before its words: the page not in
/// memory; the table changed while it was read; the part refused, by the edit's word; a Runtime
/// without the read (its code); another failure (its codes); no thread (the system's words); a
/// panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickSaid {
    NotInMemory,
    Moved,
    Refused(WriteProblem),
    Unsupported(&'static str),
    Failed(ClientFailure),
    NoThread(String),
    Panicked,
}

/// The log's line of where a part refused `goal_page_view_refused` names what the root does not
/// have, which the picker's reads keep, being asked with the full receipt
/// (`contracts/runtime-interface.md` 895-899 at aa6765dd); none of another answer.
fn refused_at<T>(came: &Came<T>) -> Option<String> {
    match came {
        Came::Refused { problem, pointer: Some(pointer) } => {
            Some(format!("refused {problem:?} at {pointer}"))
        }
        _ => None,
    }
}

impl PickSaid {
    /// What `came` says when it holds nothing to show; none of a read that answered.
    fn of<T>(came: Came<T>) -> Option<Self> {
        Some(match came {
            Came::Read(_) => return None,
            Came::NotInMemory => Self::NotInMemory,
            Came::Moved => Self::Moved,
            Came::Refused { problem, .. } => Self::Refused(problem),
            Came::Unsupported(code) => Self::Unsupported(code),
            Came::Failed(failure) => Self::Failed(failure),
        })
    }
}

impl Picks {
    /// The options of `entry` of `alias` kept under `sha`.
    pub fn options(
        &self,
        alias: &str,
        entry: &str,
        sha: &Option<String>,
    ) -> Option<&OptionsLoaded> {
        self.options.get(&(alias.to_owned(), entry.to_owned(), sha.clone()))
    }

    /// The positions of `ladder` kept under `sha`.
    pub fn positions(&self, ladder: &Ladder, sha: &Option<String>) -> Option<&PositionsLoaded> {
        self.positions.get(&(ladder.clone(), sha.clone()))
    }

    /// Whether a read of `alias`'s picker is out.
    pub fn reading(&self, alias: &str) -> bool {
        self.out.as_ref().is_some_and(|out| out.alias == alias)
    }

    /// What the newest read of `alias`'s picker came to, when it holds nothing to show.
    pub fn said(&self, alias: &str) -> Option<&PickSaid> {
        self.said.get(alias)
    }

    /// `alias`'s picker opened anew, or another character chosen in it: what its last read came to
    /// goes, and is read again as it is needed.
    pub fn forget_said(&mut self, alias: &str) {
        self.said.remove(alias);
    }

    /// The options of `entry` of `alias` under `sha` let go of when the lists they read have
    /// expired at `now_ms`: the picker opened anew reads them again.
    pub fn forget_stale(&mut self, alias: &str, entry: &str, sha: &Option<String>, now_ms: u64) {
        let key = (alias.to_owned(), entry.to_owned(), sha.clone());
        if self.options.get(&key).is_some_and(|options| !options.fresh_at(now_ms)) {
            self.options.remove(&key);
        }
    }

    /// Everything kept under `sha`, the tables' sha256 the declaration moved from, let go of.
    pub(super) fn drop_moved(&mut self, sha: &str) {
        self.options.retain(|(_, _, kept), _| kept.as_deref() != Some(sha));
        self.positions.retain(|(_, kept), _| kept.as_deref() != Some(sha));
    }

    /// A read of `job` of the picker of `entry` of `alias`, now out.
    fn issue(&mut self, alias: &str, entry: &str, job: PickJob) -> PickRequest {
        self.issued += 1;
        let (alias, entry) = (alias.to_owned(), entry.to_owned());
        let request = PickRequest { ticket: self.issued, alias, entry, job };
        self.out = Some(request.clone());
        request
    }

    /// The account of the read out under `ticket`, when it is the one out.
    pub(super) fn out_alias(&self, ticket: u64) -> Option<String> {
        self.out.as_ref().filter(|out| out.ticket == ticket).map(|out| out.alias.clone())
    }

    /// The read under `ticket` landed: it, when it is the one out.
    fn landed(&mut self, ticket: u64) -> Option<PickRequest> {
        match &self.out {
            Some(out) if out.ticket == ticket => self.out.take(),
            Some(_) | None => None,
        }
    }

    /// Every read out let go of: the Runtime started again, and what comes of it counts for
    /// nothing.
    pub fn restarted(&mut self) {
        self.out = None;
        self.said.clear();
    }

    /// The read `picker` of `alias` needs now, made out, while none is out and its last read did
    /// not come to nothing: the table's options, while none is kept under its sha256; else, of
    /// the character chosen, the positions of each ladder it has that none is kept of.
    pub fn next(&mut self, alias: &str, picker: &Picker) -> Option<PickRequest> {
        if self.out.is_some() || self.said.contains_key(alias) {
            return None;
        }
        let Some(options) = self.options(alias, &picker.entry, &picker.sha) else {
            return Some(self.issue(alias, &picker.entry, PickJob::Options));
        };
        let key = picker.key.as_deref()?;
        // The words do not matter here: only the ladders.
        let table = options.table(&picker.table, Tongue::En);
        let mut missing: Vec<Ladder> = Vec::new();
        for lane in &table.key(key)?.lanes {
            let kept = self.positions(&lane.ladder, &picker.sha).is_some();
            if !kept && !missing.contains(&lane.ladder) {
                missing.push(lane.ladder.clone());
            }
        }
        if missing.is_empty() {
            return None;
        }
        Some(self.issue(alias, &picker.entry, PickJob::Positions(missing)))
    }

    /// What the read under `ticket` came to: kept when it answered, each under the sha256 its
    /// tables were read under (the options' becoming the picker's), and everything kept under a
    /// sha256 the declaration moved from let go of; else said for its account's picker. The
    /// request it answers, none when it is not the one out (a restart let it go), and the log's
    /// lines of it. `picker` is the picker of the request's account, while it is open.
    pub(super) fn take(
        &mut self,
        ticket: u64,
        answer: PickAnswer,
        picker: Option<&mut Picker>,
    ) -> Option<(PickRequest, Vec<String>)> {
        let request = self.landed(ticket)?;
        let mut lines = Vec::new();
        let mut said: Option<PickSaid> = None;
        let mut dropped: Vec<String> = Vec::new();
        match answer {
            PickAnswer::Options(answer) => {
                dropped.extend(answer.dropped);
                match answer.came {
                    Came::Read(loaded) => {
                        let sha = loaded.tables_sha256().map(str::to_owned);
                        let picker = picker.filter(|picker| picker.entry == request.entry);
                        if let Some(picker) = picker {
                            picker.sha = sha.clone();
                        }
                        let key = (request.alias.clone(), request.entry.clone(), sha);
                        self.options.insert(key, loaded);
                    }
                    came => {
                        lines.extend(refused_at(&came));
                        said = PickSaid::of(came);
                    }
                }
            }
            PickAnswer::Positions(answers) => {
                for (ladder, answer) in answers {
                    dropped.extend(answer.dropped);
                    match answer.came {
                        Came::Read(loaded) => {
                            let sha = loaded.tables_sha256().map(str::to_owned);
                            self.positions.insert((ladder, sha), loaded);
                        }
                        came => {
                            lines.extend(refused_at(&came));
                            if said.is_none() {
                                said = PickSaid::of(came);
                            }
                        }
                    }
                }
            }
            PickAnswer::Panicked(message) => {
                lines.push(format!("panicked: {message}"));
                said = Some(PickSaid::Panicked);
            }
            PickAnswer::NoThread(error) => said = Some(PickSaid::NoThread(error)),
        }
        for sha in &dropped {
            self.drop_moved(sha);
            lines.push(format!(
                "the goal tables moved from {sha}: what the pickers kept under it is let go"
            ));
        }
        if let Some(said) = said {
            lines.push(format!("{said:?}"));
            self.said.insert(request.alias.clone(), said);
        }
        Some((request, lines))
    }
}

/// The picker of an account as the Goals tab shows it (Workflow #455 RR4b), before its words: the
/// line it shows under; the records entry's title; whether its "save" can be pressed now (the page
/// takes presses); whether a read of it is out, and what its last read came to when it holds
/// nothing; what is typed in its search field and whether it lists every character; the table, once
/// read; the keys recorded in the entry; the character chosen, if any, and each of its lanes; the
/// press its "save" makes, while every lane's ladder is read and one lane at least has a target;
/// whether that is the record stored already; and whether a character can be added to the entry.
#[derive(Debug, Clone)]
pub struct PickerView {
    pub at: String,
    pub title: Option<String>,
    pub enabled: bool,
    pub reading: bool,
    pub said: Option<PickSaid>,
    pub search: String,
    pub all: bool,
    pub table: Option<TableOptions>,
    pub recorded: Vec<String>,
    pub key: Option<String>,
    pub lanes: Vec<LanePick>,
    pub put: Option<GoalAct>,
    pub unchanged: bool,
    pub room: Room,
}

/// One lane of the character chosen: its ladder as the options say it, its positions once read,
/// the place of the one chosen (none: no target), and what its column requires.
#[derive(Debug, Clone)]
pub struct LanePick {
    pub lane: KeyLaneOption,
    pub positions: Option<Vec<PositionOption>>,
    pub chosen: Option<usize>,
    pub requires: Vec<Requirement>,
}

/// `picker` of `alias` on the line of its records entry `block`, with what `picks` keeps, in
/// `tongue`; `enabled` whether the page takes presses now. Each lane stands at its choice, else at
/// its default ([`default_target`]: its record's target, else its ladder's last position); its
/// "save" makes the record of the character chosen of each lane's position ([`to_of`]).
pub fn picker_view(
    picker: &Picker,
    block: &RecordsLine,
    picks: &Picks,
    alias: &str,
    enabled: bool,
    tongue: Tongue,
) -> PickerView {
    let table = picks
        .options(alias, &picker.entry, &picker.sha)
        .map(|options| options.table(&picker.table, tongue));
    let stored = picker.key.as_deref().and_then(|key| block.recorded(key)).map(|record| &record.to);
    let mut lanes: Vec<LanePick> = Vec::new();
    let option = match (&table, &picker.key) {
        (Some(table), Some(key)) => table.key(key),
        _ => None,
    };
    if let (Some(table), Some(option)) = (&table, option) {
        for (place, lane) in option.lanes.iter().enumerate() {
            let positions =
                picks.positions(&lane.ladder, &picker.sha).map(|positions| positions.shown(tongue));
            let chosen = match picker.choices.get(&place) {
                Some(choice) => *choice,
                None => positions.as_ref().and_then(|positions| {
                    let numbers: Vec<Vec<i64>> =
                        positions.iter().map(|position| position.position.clone()).collect();
                    default_target(&numbers, stored, &lane.column, lane.sub.as_deref())
                }),
            };
            let count = positions.as_ref().map_or(0, Vec::len);
            let requires = table.column(&lane.column).map(|column| column.requires.clone());
            lanes.push(LanePick {
                lane: lane.clone(),
                positions,
                chosen: chosen.filter(|chosen| *chosen < count),
                requires: requires.unwrap_or_default(),
            });
        }
    }
    let loaded = !lanes.is_empty() && lanes.iter().all(|lane| lane.positions.is_some());
    let put = match &picker.key {
        Some(key) if loaded => {
            let targets: Vec<LaneTarget> = lanes
                .iter()
                .map(|lane| LaneTarget {
                    column: lane.lane.column.clone(),
                    sub: lane.lane.sub.clone(),
                    position: lane
                        .chosen
                        .and_then(|chosen| lane.positions.as_ref()?.get(chosen))
                        .map(|position| position.position.clone()),
                })
                .collect();
            let to = to_of(&targets);
            (!to.is_empty()).then(|| GoalAct::Put { key: key.clone(), to })
        }
        Some(_) | None => None,
    };
    let unchanged = match (&put, stored) {
        (Some(GoalAct::Put { to, .. }), Some(stored)) => to == stored,
        _ => false,
    };
    PickerView {
        at: picker.at.clone(),
        title: block.title.clone(),
        enabled,
        reading: picks.reading(alias),
        said: picks.said(alias).cloned(),
        search: picker.search.clone(),
        all: picker.all,
        table,
        recorded: block.stored.iter().map(|record| record.key.clone()).collect(),
        key: picker.key.clone(),
        lanes,
        put,
        unchanged,
        room: block.room,
    }
}

/// "Add a character" on the records entry's line `key`, or "change" on the record's line `key`:
/// its picker opens under that line, and reads what it needs (`Model::goal_picker_open`).
pub fn picker_opened(ui: &Ui, key: &str) {
    let language = ui.labels().tongue;
    let request = ui.model.borrow_mut().goal_picker_open(key, language, paint::now_ms());
    match request {
        Some(request) => pick_send(ui, request),
        None => paint::tasks(ui),
    }
}

/// The picker's search field typed as `text`: kept, so that a paint does not take it back.
pub fn picker_searched(ui: &Ui, text: &str) {
    ui.model.borrow_mut().goal_picker_edit(|picker| picker.search = text.to_owned());
    paint::tasks(ui);
}

/// The picker's switch of every character, against only those held, turned to `all`.
pub fn picker_all(ui: &Ui, all: bool) {
    ui.model.borrow_mut().goal_picker_edit(|picker| picker.all = all);
    paint::tasks(ui);
}

/// The character at `index` among those the picker lists chosen: its ladders' positions are read
/// as far as none is kept.
pub fn picker_key(ui: &Ui, index: i32) {
    let language = ui.labels().tongue;
    let request = match usize::try_from(index) {
        Ok(index) => ui.model.borrow_mut().goal_picker_key(index, language),
        Err(_) => None,
    };
    match request {
        Some(request) => pick_send(ui, request),
        None => paint::tasks(ui),
    }
}

/// On the lane at `lane` of the character chosen, the row `row` picked: the first is no target,
/// each other the position before it.
pub fn picker_lane(ui: &Ui, lane: i32, row: i32) {
    if let (Ok(lane), Ok(row)) = (usize::try_from(lane), usize::try_from(row)) {
        let choice = row.checked_sub(1);
        ui.model.borrow_mut().goal_picker_edit(|picker| {
            picker.choices.insert(lane, choice);
        });
    }
    paint::tasks(ui);
}

/// "Another character": the picker lists the characters again, its choices let go.
pub fn picker_back(ui: &Ui) {
    ui.model.borrow_mut().goal_picker_edit(|picker| {
        picker.key = None;
        picker.choices.clear();
    });
    paint::tasks(ui);
}

/// The picker closed: nothing is written.
pub fn picker_cancelled(ui: &Ui) {
    ui.model.borrow_mut().goal_picker_close();
    paint::tasks(ui);
}

/// "Save" in the picker: the record its choices make, pressed on the line it shows under, as the
/// newest page read makes the press (`Model::goal_picker_put`); else only painted.
pub fn picker_saved(ui: &Ui) {
    let language = ui.labels().tongue;
    let press = ui.model.borrow().goal_picker_put(language);
    match press {
        Some((key, act)) => pressed(ui, &key, act),
        None => paint::tasks(ui),
    }
}

/// "Remove" on the record's line `key`: the entry's array without it, at once.
pub fn record_removed(ui: &Ui, key: &str) {
    pressed(ui, key, GoalAct::Remove);
}

/// `request`, once, on a thread of its own. A thread the system refuses is its answer at once; a
/// panic on it is an answer as well: with none, the picker would wait for good.
fn pick_send(ui: &Ui, request: PickRequest) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let root = launcher.state_root.clone();
    let worker = request.clone();
    let spawned = spawn_worker(PICK_THREAD, move || {
        let (alias, entry) = (worker.alias.as_str(), worker.entry.as_str());
        let read = || match &worker.job {
            PickJob::Options => PickAnswer::Options(read_options(&root, alias, entry)),
            PickJob::Positions(ladders) => {
                PickAnswer::Positions(read_positions(&root, alias, entry, ladders))
            }
        };
        let answer = match catch_unwind(AssertUnwindSafe(read)) {
            Ok(answer) => answer,
            Err(panic) => PickAnswer::Panicked(panic_message(&*panic)),
        };
        let ticket = worker.ticket;
        report::post(PICK_THREAD, move || {
            if let Some(ui) = ui::current() {
                pick_landed(&ui, ticket, answer);
            }
        });
    });
    match spawned {
        Ok(()) => paint::tasks(ui),
        Err(error) => pick_landed(ui, request.ticket, PickAnswer::NoThread(error.to_string())),
    }
}

/// A picker's read, on the event loop: kept, or said on the picker; then what the picker shown
/// needs next is read. One whose ticket is no longer the one out counts for nothing.
fn pick_landed(ui: &Ui, ticket: u64, answer: PickAnswer) {
    let landed = ui.model.borrow_mut().goal_pick_landed(ticket, answer);
    let Some((request, lines, next)) = landed else {
        let line = format!("goal pick ticket {ticket}: its answer came after it was let go");
        report::log_quiet(PICK_THREAD, &line);
        paint::tasks(ui);
        return;
    };
    for line in &lines {
        let line = format!(
            "goal pick ticket {} of {}, entry {}, {:?}: {line}",
            request.ticket, request.alias, request.entry, request.job
        );
        report::log(PICK_THREAD, &line);
    }
    match next {
        Some(next) => pick_send(ui, next),
        None => paint::tasks(ui),
    }
}
