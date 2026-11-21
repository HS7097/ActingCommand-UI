// SPDX-License-Identifier: GPL-3.0-only
//! A person's take-over of an account's device, "Take Over", and its hand-back, "Resume
//! Auto-Play" (Workflow #429 UI-3d): the rescue bar's first row has the one or the other before
//! "Auto Rules" at its end (`Holds::row`), of an instance that drives a device of its own only, as
//! the Runtime takes a player hold of a registered physical instance only (Runtime f6e2cd26
//! `contracts/player-hold.md` 75-76). Under a pause too: a hold may be acquired whatever pause
//! holds the instance (92-93).
//!
//! What the card, the picture and the device's buttons say comes from the reads alone: the
//! instance's hold and its last hold operation, as `Status` says them
//! (`acui_source::workbench::hold`). The console keeps no state of a hold of its own, but for the
//! requests it sent, which it
//! records before it sends them (`hold_file`) so that one whose answer never came is settled from
//! a `Status` after a restart of the console, of its connection or of the Runtime (`status_read`).
//!
//! A press of "Take Over" asks first, always: it stops what runs, and once sent it cannot be
//! withdrawn (ruling 134). A press of "Resume Auto-Play" goes out at once when this console set the
//! hold this run; of any other hold, another console's included, it asks first, since a person may
//! be playing on the device; the question keeps the hold it was asked of, and goes once a `Status`
//! shows another hold or none (`Model::rescue_reviewed`). Any channel may end a hold (ruling 135):
//! who began it is said by its channel. Each request goes out once, on a thread of its own, over a
//! fresh person's connection, under a ticket of the count the device's operations take theirs from
//! (`Rescue::ticket`), and nothing is sent again by itself, whatever comes back: a release refused
//! because the hold changed is read again and asked of the person once more. The thread waits for
//! the receipt as long as the client does, about two minutes for an acquire (Runtime f6e2cd26
//! `crates/actingcommand-contract/src/runtime/player_hold.rs` 40-74); a restart of the Runtime does
//! not let it go: its connection ends, and the client settles it then
//! (`crates/runtime-client/src/client/player_hold.rs` 298-321).
//!
//! An acquire freezes the account's picture as it is pressed (`preview::halt`), and the next
//! `Status` read after its answer decides the gate from then on; one that did nothing lets the
//! picture go at once (`preview::release`). A Runtime that answers `not_implemented`, v0.12.76
//! among them (Runtime f6e2cd26 `crates/runtime-host/src/host/requests.rs` 818-827), records
//! nothing, and is kept for its owner epoch: every account's row says the Runtime cannot take over
//! yet, and offers nothing, until another epoch.
//!
//! `Holds` and the views it makes hold no words, as the `Model` they are part of; `paint` makes
//! them.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

use acui_source::workbench::hold::{
    keeps_attempt, send as send_hold, settle_stored, HoldAsk, HoldFile, HoldKind, HoldOutcome,
    HoldPhase, HoldRequestId, HoldSettled, HoldSource, HoldTarget, RecordState, Restored,
    StartupChoice, StoredAttempt, ACQUIRE_DRAIN_MS,
};
use acui_source::workbench::InstanceLive;
use slint::Timer;

use super::failure_line;
use super::paint;
use super::preview;
use super::read::panic_message;
use super::rescue::{Asked, RescueTicket, RescueView, RESCUE_COOLDOWN};
use crate::hold_file;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// The two requests' threads, also what their lines in the technical log are under.
const ACQUIRE_THREAD: &str = "acui-act-hold";
const RELEASE_THREAD: &str = "acui-act-hand-back";
/// The settlements of what the record file keeps, and the file's own reads, in the log.
const HOLD_LOG: &str = "acui-hold";

fn thread(kind: HoldKind) -> &'static str {
    match kind {
        HoldKind::Acquire => ACQUIRE_THREAD,
        HoldKind::Release => RELEASE_THREAD,
    }
}

/// For the log.
fn name(kind: HoldKind) -> &'static str {
    match kind {
        HoldKind::Acquire => "take over (AcquirePlayerHold)",
        HoldKind::Release => "hand back (ReleasePlayerHold)",
    }
}

/// Each account's take-over and hand-back this run, and what of them is the console's own; on the
/// event loop only.
#[derive(Default)]
pub struct Holds {
    /// By alias.
    slots: BTreeMap<String, HoldSlot>,
    /// The owner epoch whose Runtime answered a request `not_implemented`, or whose dropped
    /// request the client settled as unsupported: of the whole Runtime, not of an account.
    unsupported: Option<String>,
    /// The record file as last read.
    stored: StoredNow,
    /// The accounts whose "bring the game to its main screen first" was unticked; ticked by
    /// default.
    skip_startup: BTreeSet<String>,
    /// Of each stored attempt, by its log line, the rule of the table its last settlement came to:
    /// a settlement is logged, and said, only when that changes.
    told: BTreeMap<String, &'static str>,
    /// The acquires this console stored and sent this run: a hold one of them set is handed back
    /// without a question.
    mine: Vec<HoldRequestId>,
    /// By their log lines, the stored attempts let go whose taking out of the record file failed:
    /// taken out again at every `Status` read before the file is read again, and never settled or
    /// said again meanwhile.
    to_forget: BTreeMap<String, StoredAttempt>,
}

/// The record file as last read: not yet, its record, or why it could not be read.
#[derive(Default)]
enum StoredNow {
    #[default]
    NotRead,
    Read(HoldFile),
    Failed(String),
}

/// One account's: the request out, if any; what the last came to; the ticket whose cooldown runs;
/// and whether its picture, frozen at the press, waits for the next `Status` to be let go.
#[derive(Default)]
struct HoldSlot {
    out: Option<HoldOut>,
    said: Option<HoldSaid>,
    cooling: Option<RescueTicket>,
    awaiting_read: bool,
}

/// One request out: its ticket, what it asks, and the owner epoch online when it was pressed.
struct HoldOut {
    ticket: RescueTicket,
    ask: HoldAsk,
    epoch: String,
}

/// "Take Over" or "Resume Auto-Play" on the bar's first row, before "Auto Rules" at its end
/// (`back` the latter): whether it can be pressed, why or why not, and whether "bring the game to
/// its main screen first" shows under that row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldRow {
    pub back: bool,
    pub offered: bool,
    pub why: HoldWhy,
    pub startup_shown: bool,
}

/// What the row says, by the first of these that holds (`Holds::row`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldWhy {
    /// Nothing holds the device: "Take Over" can be pressed.
    TakeOver,
    /// The device is a person's: "Resume Auto-Play" can be pressed. Who began the hold; whether a
    /// press asks first (a hold this console did not set this run); whether a pause holds the
    /// instance, under which a hand-back reconnects nothing.
    HandBack { source: HoldSource, asks: bool, paused: bool },
    /// This console's own request is out: the line under the row says how it goes.
    Asking,
    /// A take-over is under way, and cannot be withdrawn.
    Acquiring,
    /// A hand-back is under way.
    Releasing,
    /// A request of it just landed.
    CoolingDown,
    /// Another work of its device is out from here: an operation, a refresh, a switch.
    DeviceBusy,
    /// This owner epoch's Runtime has no player hold.
    Unsupported,
    /// The record file cannot be read or written: nothing is sent. Why.
    FileFailed(String),
}

/// What the last request of an account came to, before its words; `about`, its request, when it
/// was stored.
#[derive(Debug, Clone)]
pub enum HoldSaid {
    /// It is out.
    Sending { kind: HoldKind, about: Option<HoldRequestId> },
    /// Its answer landed at `at_ms`.
    Landed { at_ms: u64, kind: HoldKind, outcome: HoldOutcome, about: Option<HoldRequestId> },
    /// A stored attempt was settled from a `Status`, at `at_ms`; the settlement names the kind
    /// where its words need it.
    Stored { at_ms: u64, settled: HoldSettled, about: Option<HoldRequestId> },
    /// The system refused its thread: nothing was sent. The error is the system's.
    NoThread { at_ms: u64, error: String, about: Option<HoldRequestId> },
    /// Its thread panicked. With `about`, the attempt it stored: whether it was sent is not known,
    /// and the attempt is settled from the next `Status`. Without, it panicked before its attempt
    /// was stored, and a request is submitted only after that: nothing was sent.
    Panicked { at_ms: u64, about: Option<HoldRequestId> },
}

impl HoldSaid {
    fn about(&self) -> Option<&HoldRequestId> {
        match self {
            Self::Sending { about, .. }
            | Self::Landed { about, .. }
            | Self::Stored { about, .. }
            | Self::NoThread { about, .. }
            | Self::Panicked { about, .. } => about.as_ref(),
        }
    }

    /// Whether it says that the Runtime has no player hold: of its owner epoch only.
    fn unsupported(&self) -> bool {
        matches!(
            self,
            Self::Landed {
                outcome: HoldOutcome::NotImplemented { .. }
                    | HoldOutcome::Settled { said: HoldSettled::Unsupported, .. },
                ..
            } | Self::Stored { settled: HoldSettled::Unsupported, .. }
        )
    }
}

/// What the picture's place says while it is withdrawn, the device a person's: who began the
/// hold, since when (when the device was handed over, else when the acquire was taken), whether a
/// start restored it, and how many runs its own acquire stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldPanel {
    pub source: HoldSource,
    pub since_ms: u64,
    pub restored: Option<Restored>,
    pub cancelled: Option<u32>,
}

/// The panel of `instance`, while its hold is held.
pub fn held_panel(instance: &InstanceLive) -> Option<HeldPanel> {
    let fact = instance.hold.as_ref()?;
    let hold = fact.hold.as_ref().filter(|hold| hold.phase == HoldPhase::Held)?;
    let own = fact
        .record
        .as_ref()
        .filter(|record| record.of_hold && record.kind == HoldKind::Acquire);
    Some(HeldPanel {
        source: hold.source,
        since_ms: hold.reached_held_ms.unwrap_or(hold.since_ms),
        restored: hold.restored,
        cancelled: own.and_then(|record| record.cancelled).filter(|cancelled| *cancelled > 0),
    })
}

impl Holds {
    /// The row of `instance`, which the newest `Status` read at the owner epoch `epoch` lists,
    /// `global_pause` whether that read had the global pause, `busy` whether another work of its
    /// device is out from here: none of an instance that drives no device of its own. The first
    /// of these that holds: this console's own request is out; the hold is held, and "Resume
    /// Auto-Play" can be pressed but while it cools down, the record file fails or this Runtime
    /// has no hold; a hand-back is under way; a take-over is; and, with no hold, "Take Over" can be
    /// pressed but while it cools down, the record file fails, this Runtime has no hold or the
    /// device is busy. A pause holds nothing back.
    pub fn row(
        &self,
        instance: &InstanceLive,
        global_pause: bool,
        epoch: &str,
        busy: bool,
    ) -> Option<HoldRow> {
        if !instance.physical {
            return None;
        }
        let slot = self.slots.get(&instance.alias);
        let fact = instance.hold.as_ref();
        let hold = fact.and_then(|fact| fact.hold.as_ref());
        let running = |kind: HoldKind| {
            fact.and_then(|fact| fact.record.as_ref())
                .is_some_and(|record| record.state == RecordState::Running && record.kind == kind)
        };
        let releasing = hold.is_some_and(|hold| hold.phase == HoldPhase::Releasing)
            || (hold.is_none() && running(HoldKind::Release));
        let held = hold.filter(|hold| hold.phase == HoldPhase::Held);
        let cooling = slot.is_some_and(|slot| slot.cooling.is_some());
        let unsupported = self.unsupported.as_deref() == Some(epoch);
        let file_failed = match &self.stored {
            StoredNow::Failed(error) => Some(error.clone()),
            StoredNow::NotRead | StoredNow::Read(_) => None,
        };
        let held_back = |why: HoldWhy| {
            if cooling {
                HoldWhy::CoolingDown
            } else if let Some(error) = file_failed.clone() {
                HoldWhy::FileFailed(error)
            } else if unsupported {
                HoldWhy::Unsupported
            } else {
                why
            }
        };
        if slot.is_some_and(|slot| slot.out.is_some()) {
            let back = held.is_some() || releasing;
            return Some(HoldRow { back, offered: false, why: HoldWhy::Asking, startup_shown: false });
        }
        if let Some(hold) = held {
            let asks = !self.mine.iter().any(|request| hold.target.is_of(request));
            let paused = instance.pause.is_some() || global_pause;
            let why = held_back(HoldWhy::HandBack { source: hold.source, asks, paused });
            let offered = matches!(why, HoldWhy::HandBack { .. });
            return Some(HoldRow { back: true, offered, why, startup_shown: true });
        }
        if releasing {
            let why = HoldWhy::Releasing;
            return Some(HoldRow { back: true, offered: false, why, startup_shown: false });
        }
        if hold.is_some() || running(HoldKind::Acquire) {
            let why = HoldWhy::Acquiring;
            return Some(HoldRow { back: false, offered: false, why, startup_shown: false });
        }
        let why = held_back(if busy { HoldWhy::DeviceBusy } else { HoldWhy::TakeOver });
        let offered = why == HoldWhy::TakeOver;
        Some(HoldRow { back: false, offered, why, startup_shown: false })
    }

    /// What the last request of `alias` came to, if one was pressed, or a stored one settled.
    pub fn said(&self, alias: &str) -> Option<&HoldSaid> {
        self.slots.get(alias)?.said.as_ref()
    }

    /// Whether a request of `alias` is out.
    pub fn out(&self, alias: &str) -> bool {
        self.slots.get(alias).is_some_and(|slot| slot.out.is_some())
    }

    /// Whether a hand-back of `alias` asks for its startup package: unless its box was unticked.
    pub fn starts(&self, alias: &str) -> bool {
        !self.skip_startup.contains(alias)
    }

    fn set_starts(&mut self, alias: &str, run: bool) {
        if run {
            self.skip_startup.remove(alias);
        } else {
            self.skip_startup.insert(alias.to_owned());
        }
    }

    /// `out` of `alias` begins: whether it goes, which it does not while one is out or cools down.
    /// What it came to is, for now, that it is being sent.
    fn begin(&mut self, alias: &str, out: HoldOut) -> bool {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if slot.out.is_some() || slot.cooling.is_some() {
            return false;
        }
        slot.said = Some(HoldSaid::Sending { kind: out.ask.kind(), about: None });
        slot.out = Some(out);
        true
    }

    /// The request of `alias` under `ticket`, as its answer lands: none when that ticket is not
    /// the one out.
    fn take_out(&mut self, alias: &str, ticket: RescueTicket) -> Option<HoldOut> {
        let slot = self.slots.get_mut(alias)?;
        if slot.out.as_ref().map(|out| out.ticket) != Some(ticket) {
            return None;
        }
        slot.out.take()
    }

    /// What the request of `alias` under `ticket` came to: said, cooling down from now, and its
    /// picture let go at the next `Status` (`awaiting_read`) unless now.
    fn landed(&mut self, alias: &str, ticket: RescueTicket, said: HoldSaid, awaiting_read: bool) {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        slot.said = Some(said);
        slot.cooling = Some(ticket);
        slot.awaiting_read = awaiting_read;
    }

    /// The cooldown of `alias`'s request under `ticket` ran out: whether it was the one that ran.
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

    /// A `Status` read online at the owner epoch `epoch`: another epoch's word that the Runtime
    /// has no hold goes, and every account's line that said so with it; the accounts whose picture
    /// waited for this read, with no request out, are let go.
    fn read_online(&mut self, epoch: &str) -> Vec<String> {
        if self.unsupported.as_deref().is_some_and(|said| said != epoch) {
            self.unsupported = None;
            for slot in self.slots.values_mut() {
                if slot.said.as_ref().is_some_and(HoldSaid::unsupported) {
                    slot.said = None;
                }
            }
        }
        let mut released = Vec::new();
        for (alias, slot) in &mut self.slots {
            if slot.awaiting_read && slot.out.is_none() {
                slot.awaiting_read = false;
                released.push(alias.clone());
            }
        }
        released
    }

    /// Whether the record file is to be read: not read yet, or failed, which every `Status` read
    /// online tries again.
    fn needs_load(&self) -> bool {
        matches!(self.stored, StoredNow::NotRead | StoredNow::Failed(_))
    }

    /// `stored` was taken out of the record file: out of what was read of it, of what was told, and
    /// of what is still to be taken out.
    fn forgotten(&mut self, stored: &StoredAttempt) {
        if let StoredNow::Read(file) = &mut self.stored {
            file.take(stored);
        }
        let line = stored.line();
        self.told.remove(&line);
        self.to_forget.remove(&line);
    }

    /// A stored attempt of `stored.alias()` settled at `at_ms`: said, unless what is said is what
    /// this console's own request of another request came to, which only its own settlement
    /// says over.
    fn say_stored(&mut self, stored: &StoredAttempt, settled: HoldSettled, at_ms: u64) {
        let slot = self.slots.entry(stored.alias().to_owned()).or_default();
        let request = stored.request();
        let may = match &slot.said {
            None | Some(HoldSaid::Stored { .. }) => true,
            Some(HoldSaid::Sending { .. }) => false,
            Some(said) => said.about() == Some(&request),
        };
        if may {
            slot.said = Some(HoldSaid::Stored { at_ms, settled, about: Some(request) });
        }
    }
}

/// What one request came to, as its thread hands it back.
enum Landing {
    /// Its outcome, and the attempt it stored, when it stored one.
    Answer { outcome: HoldOutcome, stored: Option<StoredAttempt> },
    /// The thread panicked. The attempt it stored, if it got so far: then whether the request got
    /// out is not known; else nothing was sent, as the request goes only once its attempt is
    /// stored (`acui_source::workbench::hold::send`).
    Panicked { message: String, stored: Option<StoredAttempt> },
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

impl Landing {
    fn stored(&self) -> Option<&StoredAttempt> {
        match self {
            Self::Answer { stored, .. } | Self::Panicked { stored, .. } => stored.as_ref(),
            Self::NoThread(_) => None,
        }
    }

    /// Whether its stored attempt, if any, stays in the record file: a panic's does, to be settled
    /// from the next `Status`.
    fn keeps(&self) -> bool {
        match self {
            Self::Answer { outcome, .. } => keeps_attempt(outcome),
            Self::Panicked { .. } => true,
            Self::NoThread(_) => false,
        }
    }

    /// Whether nothing was done for sure: then the frozen picture goes at once.
    fn did_nothing(&self) -> bool {
        match self {
            Self::Answer { outcome, .. } => matches!(
                outcome,
                HoldOutcome::NotSent { .. }
                    | HoldOutcome::NotStored { .. }
                    | HoldOutcome::NotImplemented { .. }
                    | HoldOutcome::Refused { denied: true, .. }
                    | HoldOutcome::Settled {
                        said: HoldSettled::Unsupported | HoldSettled::NotPerformed { .. },
                        ..
                    }
            ),
            Self::Panicked { stored, .. } => stored.is_none(),
            Self::NoThread(_) => true,
        }
    }

    /// Whether it says that the Runtime has no player hold.
    fn unsupported(&self) -> bool {
        matches!(
            self,
            Self::Answer {
                outcome: HoldOutcome::NotImplemented { .. }
                    | HoldOutcome::Settled { said: HoldSettled::Unsupported, .. },
                ..
            }
        )
    }

    /// What it came to, before its words, of a request of `kind` landing at `at_ms`.
    fn said(&self, kind: HoldKind, at_ms: u64) -> HoldSaid {
        let about = self.stored().map(StoredAttempt::request);
        match self {
            Self::Answer { outcome, .. } => {
                HoldSaid::Landed { at_ms, kind, outcome: outcome.clone(), about }
            }
            Self::Panicked { .. } => HoldSaid::Panicked { at_ms, about },
            Self::NoThread(error) => HoldSaid::NoThread { at_ms, error: error.clone(), about },
        }
    }

    /// Into the technical log, with what it asked (`asked`, `asked_line`), its request, its owner
    /// epoch and its codes: quietly when the Runtime did it, as the problem it is otherwise,
    /// `not_implemented` included.
    fn log(&self, alias: &str, kind: HoldKind, ticket: RescueTicket, asked: &str) {
        let prefix = format!("ticket {} of {alias}, {} ({asked})", ticket.number(), name(kind));
        let about = self.stored().map_or_else(|| "nothing stored".to_owned(), StoredAttempt::line);
        let (line, quiet) = match self {
            Self::Answer { outcome, .. } => outcome_line(outcome),
            Self::Panicked { message, stored: Some(_) } => (
                format!("panicked; whether it was sent is not known, not sent again: {message}"),
                false,
            ),
            Self::Panicked { message, stored: None } => (
                format!("panicked before its attempt was stored: not sent: {message}"),
                false,
            ),
            Self::NoThread(error) => (format!("no thread, not sent: {error}"), false),
        };
        let line = format!("{prefix} ({about}): {line}");
        if quiet {
            report::log_quiet(thread(kind), &line);
        } else {
            report::log(thread(kind), &line);
        }
    }
}

/// What a request asks, for the log: an acquire's drain, a release's hold and its startup package.
fn asked_line(ask: &HoldAsk) -> String {
    match ask {
        HoldAsk::Acquire => format!("drain {ACQUIRE_DRAIN_MS} ms"),
        HoldAsk::Release { target, startup } => {
            format!("{}, startup package {startup:?}", target.line())
        }
    }
}

/// An outcome in the log's words, and whether it goes quietly.
fn outcome_line(outcome: &HoldOutcome) -> (String, bool) {
    match outcome {
        HoldOutcome::Acquired { cancelled } => (
            format!("taken over, the device is the person's; runs its drain stopped early: {cancelled}"),
            true,
        ),
        HoldOutcome::Released { next, selfcheck_failure, startup } => (
            format!(
                "handed back; next {next:?}, self-check failure {selfcheck_failure:?}, startup \
                 package {startup:?}"
            ),
            selfcheck_failure.is_none(),
        ),
        HoldOutcome::NotImplemented { failure, chain } => (
            format!(
                "refused, denied, not_implemented: this Runtime has no player hold and recorded \
                 nothing, not sent again: {}; chain: {chain}",
                failure_line(failure)
            ),
            false,
        ),
        HoldOutcome::Refused { why, code, denied, failure, chain } => {
            let how = if *denied { "denied" } else { "failed" };
            let line = format!(
                "refused, {how}, {why:?} ({code}), not sent again: {}; chain: {chain}",
                failure_line(failure)
            );
            (line, false)
        }
        HoldOutcome::Interrupted { failure, chain } => (
            format!(
                "player_hold_interrupted: the Runtime is shutting down and keeps the hold; the \
                 stored attempt is settled once it started again: {}; chain: {chain}",
                failure_line(failure)
            ),
            false,
        ),
        HoldOutcome::Settled { said, rule, chain } => (
            format!(
                "its receipt never came; the client settled it by rule {rule}: {said:?}; chain: \
                 {chain}"
            ),
            matches!(said, HoldSettled::Done { .. }),
        ),
        HoldOutcome::NotSent { failure, chain } => {
            (format!("not sent: {}; chain: {chain}", failure_line(failure)), false)
        }
        HoldOutcome::NotStored { error } => {
            (format!("not sent: the record file did not take it: {error}"), false)
        }
        HoldOutcome::Unconfirmed { failure, chain } => (
            format!(
                "unconfirmed, not sent again; the stored attempt is settled from the next Status: \
                 {}; chain: {chain}",
                failure_line(failure)
            ),
            false,
        ),
    }
}

/// "Take Over" or "Resume Auto-Play" of the account whose picture shows.
pub fn pressed(ui: &Ui) {
    let alias = ui.model.borrow().picture_account().map(str::to_owned);
    if let Some(alias) = alias {
        press(ui, &alias);
    }
}

/// The row's button of `alias` pressed: nothing when it cannot be pressed now. "Take Over" asks
/// first; "Resume Auto-Play" of a hold this console set this run goes out at once, of any other it
/// asks first, keeping the hold the person saw.
fn press(ui: &Ui, alias: &str) {
    let (row, target) = {
        let model = ui.model.borrow();
        let row = match model.rescue_view(alias) {
            Some(RescueView::Online(rows)) => rows.take_over,
            Some(RescueView::Offline { .. }) | None => None,
        };
        (row, model.hold_target(alias))
    };
    let Some(row) = row.filter(|row| row.offered) else {
        return;
    };
    match (row.why, target) {
        (HoldWhy::TakeOver, _) => ask(ui, alias, Asked::TakeOver),
        (HoldWhy::HandBack { asks: true, .. }, Some(target)) => {
            ask(ui, alias, Asked::HandBack { target });
        }
        (HoldWhy::HandBack { asks: false, .. }, Some(target)) => {
            let ask = release(ui, alias, target);
            send(ui, alias, ask);
        }
        _ => {}
    }
}

/// `asked` of `alias` is the bar's one question now.
fn ask(ui: &Ui, alias: &str, asked: Asked) {
    ui.model.borrow_mut().rescue_mut().ask(alias, asked);
    paint::refresh(ui);
}

/// A release of the hold `target`, with what the account's box asks for its startup package.
fn release(ui: &Ui, alias: &str, target: HoldTarget) -> HoldAsk {
    let startup =
        if ui.model.borrow().holds().starts(alias) { StartupChoice::Run } else { StartupChoice::Skip };
    HoldAsk::Release { target, startup }
}

/// The confirmation's button of a take-over or a hand-back of `alias` (`rescue::confirmed`, which
/// took the question): it goes out once when its button can still be pressed, a hand-back of the
/// very hold it asked about; else the bar is painted again.
pub fn confirmed(ui: &Ui, alias: &str, asked: Asked) {
    let offered = ui.model.borrow().rescue_view(alias).is_some_and(|view| view.offers_asked(&asked));
    let ask = match asked {
        Asked::TakeOver if offered => Some(HoldAsk::Acquire),
        Asked::HandBack { target } if offered => Some(release(ui, alias, target)),
        Asked::TakeOver | Asked::HandBack { .. } | Asked::Device(_) => None,
    };
    match ask {
        Some(ask) => send(ui, alias, ask),
        None => paint::refresh(ui),
    }
}

/// "Bring the game to its main screen first" ticked or not, on the bar of the account whose
/// picture shows: what its next hand-back asks for.
pub fn startup_toggled(ui: &Ui, run: bool) {
    let alias = ui.model.borrow().picture_account().map(str::to_owned);
    if let Some(alias) = alias {
        ui.model.borrow_mut().holds_mut().set_starts(&alias, run);
        paint::refresh(ui);
    }
}

/// `ask` of `alias`, once, on a thread of its own, under a ticket of the rescue bar's count. Only
/// with a state root: with none there is no Runtime to ask, and nothing is frozen. An acquire
/// freezes the account's picture first. A thread the system refuses is the answer at once, in
/// red; a panic on it is an answer as well: with none at all the picture would stay frozen.
fn send(ui: &Ui, alias: &str, ask: HoldAsk) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let root = launcher.state_root.clone();
    let kind = ask.kind();
    let ticket = {
        let mut model = ui.model.borrow_mut();
        let Some(epoch) = model.online_epoch().map(str::to_owned) else {
            return;
        };
        let rescue = model.rescue_mut();
        rescue.unconfirm();
        let ticket = rescue.ticket();
        let out = HoldOut { ticket, ask: ask.clone(), epoch };
        if !model.holds_mut().begin(alias, out) {
            return;
        }
        ticket
    };
    if kind == HoldKind::Acquire {
        preview::halt(ui, alias);
    }
    let asked = asked_line(&ask);
    let worker_asked = asked.clone();
    let worker_alias = alias.to_owned();
    let spawned = spawn_worker(thread(kind), move || {
        let alias = worker_alias;
        let thread = thread(kind);
        let mut kept: Option<StoredAttempt> = None;
        let sent = catch_unwind(AssertUnwindSafe(|| {
            send_hold(&root, &alias, &ask, &mut |stored: &StoredAttempt| -> Result<(), String> {
                hold_file::store(stored)?;
                let line = format!(
                    "ticket {} of {alias}, {}: stored {}: {}",
                    ticket.number(),
                    name(kind),
                    stored.line(),
                    stored.json()
                );
                report::log_quiet(thread, &line);
                kept = Some(stored.clone());
                Ok(())
            })
        }));
        let landing = match sent {
            Ok((outcome, stored)) => Landing::Answer { outcome, stored },
            Err(panic) => Landing::Panicked { message: panic_message(&*panic), stored: kept },
        };
        landing.log(&alias, kind, ticket, &worker_asked);
        report::post(thread, move || {
            if let Some(ui) = ui::current() {
                landed(&ui, &alias, ticket, landing);
            }
        });
    });
    match spawned {
        Ok(()) => paint::refresh(ui),
        Err(error) => {
            let landing = Landing::NoThread(error.to_string());
            landing.log(alias, kind, ticket, &asked);
            landed(ui, alias, ticket, landing);
        }
    }
}

/// An answer, on the event loop. One whose ticket is no longer the one out changes nothing shown:
/// only the log had it. One that counts: an acquire stored is the console's own from now on; its
/// stored attempt is taken out of the record file unless a later `Status` may still tell what it
/// did; a Runtime with no hold is kept for its owner epoch; a request that did nothing lets the
/// frozen picture go now, any other at the next `Status`; it cools down, and is a write, so the
/// reads begun before it are let go and read again.
fn landed(ui: &Ui, alias: &str, ticket: RescueTicket, landing: Landing) {
    let at_ms = paint::now_ms();
    let taken = ui.model.borrow_mut().holds_mut().take_out(alias, ticket);
    let Some(out) = taken else {
        let line = format!(
            "ticket {} of {alias}: its answer came when it was no longer out, and counts for \
             nothing",
            ticket.number()
        );
        report::log_quiet(HOLD_LOG, &line);
        ui.paint_log_notes();
        return;
    };
    let kind = out.ask.kind();
    let stored = landing.stored().cloned();
    let release_now = landing.did_nothing();
    {
        let mut model = ui.model.borrow_mut();
        let holds = model.holds_mut();
        if kind == HoldKind::Acquire {
            if let Some(stored) = &stored {
                holds.mine.push(stored.request());
            }
        }
        if landing.unsupported() {
            holds.unsupported = Some(out.epoch.clone());
        }
        holds.landed(alias, ticket, landing.said(kind, at_ms), !release_now);
    }
    if let Some(stored) = stored.filter(|_| !landing.keeps()) {
        forget(ui, &stored, thread(kind));
    }
    if release_now {
        preview::release(ui, alias);
    }
    cool_down(alias.to_owned(), ticket);
    super::wrote(ui);
    // The record as the file holds it now, with what this request stored: unless taking it out
    // just failed, which the next `Status` read tries again.
    if !matches!(ui.model.borrow().holds().stored, StoredNow::Failed(_)) {
        reload(ui);
    }
    paint::refresh(ui);
    paint::cards(ui);
    ui.paint_log_notes();
}

/// `stored` taken out of the record file, and out of what was read of it. A failure is said in red
/// on every account's row, and kept to be taken out again at every `Status` read
/// (`retry_forgets`), the attempt neither settled nor said again meanwhile; into the log as the
/// problem it is, once for each attempt and again only when the error's text changes.
fn forget(ui: &Ui, stored: &StoredAttempt, log: &str) {
    let forgotten = hold_file::forget(stored);
    let mut model = ui.model.borrow_mut();
    let holds = model.holds_mut();
    match forgotten {
        Ok(_) => holds.forgotten(stored),
        Err(error) => {
            let first = holds.to_forget.insert(stored.line(), stored.clone()).is_none();
            let changed = !matches!(&holds.stored, StoredNow::Failed(said) if *said == error);
            if first || changed {
                let line = format!(
                    "{}: not taken out of the record file; tried again at every Status: {error}",
                    stored.line()
                );
                report::log(log, &line);
            }
            holds.stored = StoredNow::Failed(error);
        }
    }
}

/// The attempts whose taking out failed, taken out again: whether none is left. Until then the file
/// is not read again, so that none of them is settled or said once more.
fn retry_forgets(ui: &Ui) -> bool {
    let pending: Vec<StoredAttempt> =
        ui.model.borrow().holds().to_forget.values().cloned().collect();
    for stored in &pending {
        forget(ui, stored, HOLD_LOG);
    }
    ui.model.borrow().holds().to_forget.is_empty()
}

/// The record file read again. One that does not read is said once for each new error text, and
/// on every account's row while it does not.
fn reload(ui: &Ui) {
    let loaded = hold_file::load();
    let mut model = ui.model.borrow_mut();
    let holds = model.holds_mut();
    match loaded {
        Ok(file) => {
            if matches!(holds.stored, StoredNow::Failed(_)) {
                report::log(HOLD_LOG, "the record file reads again");
            }
            holds.stored = StoredNow::Read(file);
        }
        Err(error) => {
            let news = !matches!(&holds.stored, StoredNow::Failed(said) if *said == error);
            if news {
                let line = format!(
                    "the record file cannot be read: no take-over or hand-back is sent until it \
                     reads; it is read again at every Status: {error}"
                );
                report::log(HOLD_LOG, &line);
            }
            holds.stored = StoredNow::Failed(error);
        }
    }
}

/// A `Status` read landed (`super::live_landed`), online only: another owner epoch's word that the
/// Runtime has no hold goes; the pictures that waited for this read are let go; the attempts whose
/// taking out failed are taken out again, and only once none is left the record file is read when
/// it was not, or failed; and every attempt it keeps of this state root is settled by this
/// `Status`, but of an account with a request of this console's out, which is drawn from the read
/// alone until it lands, and but one already let go. A settlement is logged and said only when its rule changes, and said
/// only of the account's newest attempt; one let go is taken out of the record file. Painting is
/// the caller's.
pub fn status_read(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let root = launcher.state_root.clone();
    let (epoch, status) = {
        let model = ui.model.borrow();
        let Some(epoch) = model.online_epoch() else {
            return;
        };
        (epoch.to_owned(), model.status_copy().cloned())
    };
    let released = ui.model.borrow_mut().holds_mut().read_online(&epoch);
    for alias in &released {
        preview::release(ui, alias);
    }
    if ui.model.borrow().holds().needs_load() && retry_forgets(ui) {
        reload(ui);
    }
    let Some(status) = status else {
        return;
    };
    let stored: Vec<StoredAttempt> = match &ui.model.borrow().holds().stored {
        StoredNow::Read(file) => file.of_root(&root).cloned().collect(),
        StoredNow::NotRead | StoredNow::Failed(_) => Vec::new(),
    };
    let mut newest: BTreeMap<String, usize> = BTreeMap::new();
    for (index, stored) in stored.iter().enumerate() {
        newest.insert(stored.alias().to_owned(), index);
    }
    let at_ms = paint::now_ms();
    for (index, stored) in stored.iter().enumerate() {
        let line = stored.line();
        let skipped = {
            let model = ui.model.borrow();
            let holds = model.holds();
            holds.out(stored.alias()) || holds.to_forget.contains_key(&line)
        };
        if skipped {
            continue;
        }
        let settled = settle_stored(stored, &status);
        let news = ui.model.borrow().holds().told.get(&line) != Some(&settled.rule);
        if news {
            let then = if settled.keep { "kept" } else { "taken out of the record file" };
            let text =
                format!("{line}: settled by rule {}: {:?}; {then}", settled.rule, settled.said);
            report::log_quiet(HOLD_LOG, &text);
            let mut model = ui.model.borrow_mut();
            let holds = model.holds_mut();
            if newest.get(stored.alias()) == Some(&index) {
                holds.say_stored(stored, settled.said.clone(), at_ms);
            }
            if settled.keep {
                holds.told.insert(line, settled.rule);
            }
        }
        if !settled.keep {
            forget(ui, stored, HOLD_LOG);
        }
    }
}

/// `alias`'s request under `ticket` cools down: its row comes back once this runs out.
fn cool_down(alias: String, ticket: RescueTicket) {
    Timer::single_shot(RESCUE_COOLDOWN, move || {
        let Some(ui) = ui::current() else {
            return;
        };
        let cooled = ui.model.borrow_mut().holds_mut().cooled(&alias, ticket);
        if cooled {
            paint::refresh(&ui);
        }
    });
}
