// SPDX-License-Identifier: GPL-3.0-only
//! A person's player hold of one instance (Workflow #429 UI-3d, "Take Over"; the Runtime's R-7a
//! shapes, register §1.233): what of it `Status` says, the two requests that set and end it, the
//! console's own record of a request it sent, and the settlement of such a request whose receipt
//! never came. Nothing out of here names a wire type: the read face is a [`HoldFact`], what came
//! back is a [`HoldOutcome`], a settlement is a [`HoldSettled`], the hold a release names is an
//! opaque [`HoldTarget`] and a request an opaque [`HoldRequestId`].
//!
//! **The read face.** Each instance's `player_hold` and `player_hold_operation`, read through the
//! contract's own pure function, `settle_player_hold` without an attempt, which describes the
//! instance now (Runtime f6e2cd26 `crates/actingcommand-contract/src/runtime/player_hold.rs`
//! 1052-1059), never `control`, which a reader is told not to read (780-784). The record of an
//! acquire is entered before its hold, the hold's row written and the gate closed after it
//! (Runtime f6e2cd26 `contracts/player-hold.md` 90), so a `Status` may show a running record and
//! no hold for a moment; a running record
//! always names its phase, and the two deadlines stand only in the acquire's three phases
//! (`PlayerHoldOperation::validate` 574-625, `PlayerHoldState::validate` 403-433). Every time is
//! the one `Status` stored, never computed again here: `since_unix_ms` is when the acquire was
//! taken (387), up to two minutes before the device was handed over, which is
//! `reached_held_unix_ms` (396), and a hold restored from a stop during its acquire has none. The
//! console's gate over the picture and the device, open, frozen or withdrawn, is [`gate`], from the
//! fact and the console's own ask in flight.
//!
//! **The two writes.** [`send`] makes a fresh person's connection, prepares one request, which
//! writes nothing (runtime-client `crates/runtime-client/src/client/player_hold.rs` 141-156,
//! 159-176), has the caller store its attempt (62-95) and only then submits it, once, on the handle
//! that prepared it (182-196, 200-216). Nothing is sent when the attempt cannot be stored. The
//! client waits for the receipt on the caller's thread, an acquire's drain, the checkpoint grace,
//! the close allowance and the io margin (contract `player_hold.rs` 40-74: with
//! [`ACQUIRE_DRAIN_MS`] about two minutes and the io), a release's 33 s and the io, so this is never
//! the event loop's. The client settles a lost receipt itself only when the connection ended
//! before the receipt's header (`dropped`) or the receipt's deadline passed (`timed_out`), and then
//! returns the settlement (client `player_hold.rs` 298-321, `crates/runtime-client/src/error.rs`
//! 376-383); any other failure, and every real receipt, comes back as it is. Nothing here sends a
//! request again, nor another one in its place: a release refused because the hold changed
//! (`player_hold_expectation_mismatch`) is read again and asked of the person once more.
//!
//! On v0.12.76 (f6e2cd26) both requests are answered at the handler, after the ui channel's list
//! took them (contract `crates/actingcommand-contract/src/runtime/interface.rs` 598-602): Denied
//! `not_implemented`, value `operation` `acquire_player_hold` or `release_player_hold`
//! (`crates/runtime-host/src/host/requests.rs` 818-827, 846-862), not fatal
//! (`crates/runtime-host/src/error.rs` 718-727), and nothing is recorded: the ledger's head and the
//! status are unchanged and the status carries none of the hold's members
//! (`crates/runtime-host/src/tests/gate_429_r7a.rs` 133-175). That is [`HoldOutcome::NotImplemented`].
//! A receipt of `player_hold_interrupted`, a Runtime shutting down that keeps the hold, is a real
//! receipt the client does not settle (runtime-client
//! `crates/runtime-client/src/tests/gate_429_r7a.rs` 680-732): the stored attempt settles it after
//! the restart.
//!
//! **The stored attempt.** The console keeps every request it sent in one record file of its own,
//! outside any state root, as a [`HoldFile`] of [`StoredAttempt`]s, each with the state root it
//! was sent to and its instance; the attempt itself is the contract's own `PlayerHoldAttempt`,
//! stored with the most cautious trigger, `reopened` (contract `player_hold.rs` 924-938). Reading
//! and writing the file is the caller's; its text is this module's ([`HoldFile::parse`],
//! [`HoldFile::render`]). A state root reached by another spelling of its path, through a junction
//! or a substituted drive, is not told to be the same ([`same_root`]): its attempts stay in the
//! file, neither settled nor dropped.
//!
//! **The settlement.** [`settle_stored`] calls the contract's `settle_player_hold` with a stored
//! attempt and a `Status` of the same state root (contract `player_hold.rs` 1024-1214), whose
//! decision names whether it is final (`is_final`, on the wire `final`, 966-994; only
//! `in_progress`, `in_progress_other` and `status_unreadable` are not, 298-306). It may be called
//! with an attempt only once the submit that sent it returned, or the process that sent it is gone
//! (1020-1023): the caller holds a stored attempt back while its own request of that instance is
//! out. An attempt is let go once its settlement is settled or unsupported, or once it was settled
//! in another owner epoch than the one it was sent in; one settled in its own epoch as anything
//! else is kept, to be settled again by a later `Status` (Runtime f6e2cd26
//! `contracts/player-hold.md` 450-454: a final A9, A10 or R6 may still be settled again).

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use acui_rows::code;
use actingcommand_contract::codes::SharedCode;
use actingcommand_contract::{
    settle_player_hold, ChannelVia, OwnerEpoch, PlayerHoldAttempt, PlayerHoldDecision,
    PlayerHoldExpectation, PlayerHoldNext, PlayerHoldOperation, PlayerHoldOperationKind,
    PlayerHoldOperationState, PlayerHoldPhase, PlayerHoldRestoredFrom, PlayerHoldSettlement,
    PlayerHoldSettlementOutcome, PlayerHoldSettlementTrigger, PlayerHoldSettlementVerdict,
    PlayerHoldSource, PlayerHoldStartupChoice, PlayerHoldStartupPackage, PlayerHoldState,
    RequestId, RuntimeChannel, RuntimeControlPlaneStatus, RuntimeReceiptState,
    MAX_PLAYER_HOLD_DRAIN_TIMEOUT_MS, MIN_PLAYER_HOLD_DRAIN_TIMEOUT_MS,
};
use actingcommand_runtime_client::{
    PreparedPlayerHoldAcquire, PreparedPlayerHoldRelease, RuntimeClient, RuntimeClientError,
    RuntimeClientErrorClass,
};
use serde_json::{Map, Value};

use super::configure_config;
use super::stop::chain;
use crate::{runtime_connection, ClientFailure, NoConnection};

/// The drain an acquire asks for: a run in flight may go on for a minute, and then has the
/// checkpoint grace to stop at a checkpoint (Workflow #429 B.2, the console's review). Within the
/// contract's closed bounds, those of an instance pause's drain (Runtime f6e2cd26 contract
/// `player_hold.rs` 42-44), which the assertion below holds it to.
pub const ACQUIRE_DRAIN_MS: u64 = 60_000;
const _: () = assert!(
    ACQUIRE_DRAIN_MS >= MIN_PLAYER_HOLD_DRAIN_TIMEOUT_MS
        && ACQUIRE_DRAIN_MS <= MAX_PLAYER_HOLD_DRAIN_TIMEOUT_MS
);

/// The host's own spellings of the player hold's codes this console tells apart, of a crate this
/// one does not use (Runtime f6e2cd26 `crates/runtime-host/src/codes.rs` 1007-1022): only looked
/// up, never shown.
pub const PLAYER_HOLD_ACTIVE: &str = "player_hold_active";
pub const PLAYER_HOLD_ALREADY_HELD: &str = "player_hold_already_held";
pub const PLAYER_HOLD_ACQUIRING: &str = "player_hold_acquiring";
pub const PLAYER_HOLD_RELEASING: &str = "player_hold_releasing";
pub const PLAYER_HOLD_NOT_HELD: &str = "player_hold_not_held";
pub const PLAYER_HOLD_EXPECTATION_MISMATCH: &str = "player_hold_expectation_mismatch";
pub const PLAYER_HOLD_DRAIN_INCOMPLETE: &str = "player_hold_drain_incomplete";
pub const PLAYER_HOLD_HANDBACK_BUSY: &str = "player_hold_handback_busy";
pub const PLAYER_HOLD_INTERRUPTED: &str = "player_hold_interrupted";
pub const PLAYER_HOLD_PERSIST_FAILED: &str = "player_hold_persist_failed";
pub const PLAYER_HOLD_NOT_DURABLE: &str = "player_hold_not_durable";
/// A release's reconnect claim not had within its wait: the hold stays (Runtime f6e2cd26
/// `contracts/player-hold.md` 121-122).
const LEASE_QUEUE_EXPIRED: &str = SharedCode::LeaseQueueExpired.as_str();
const NOT_IMPLEMENTED: &str = SharedCode::NotImplemented.as_str();

// ---- The read face ----

/// What one instance's `Status` says of its player hold: the hold now, if any; the last accepted
/// operation of this owner epoch, which stays after the hold is gone; and whether the hold ended
/// under a pause that is still the same one (contract
/// `RuntimeControlPlaneStatus::hold_ended_with_pause`, Runtime f6e2cd26 `player_hold.rs` 888-921).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldFact {
    pub hold: Option<HoldNow>,
    pub record: Option<HoldRecord>,
    pub ended_with_pause: bool,
}

/// The hold now: its phase; when the acquire was taken (`since_ms`) and when the device was
/// handed over (`reached_held_ms`, none while acquiring and on a hold restored from a stop during
/// its acquire); who began it; the two deadlines, only in the acquire's three phases; when the
/// device session's close began, only while handing back; whether a start restored it; and the
/// hold a release of it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldNow {
    pub phase: HoldPhase,
    pub since_ms: u64,
    pub reached_held_ms: Option<u64>,
    pub source: HoldSource,
    pub drain_deadline_ms: Option<u64>,
    pub grace_deadline_ms: Option<u64>,
    pub close_started_ms: Option<u64>,
    pub restored: Option<Restored>,
    pub target: HoldTarget,
}

/// Vocabulary `player_hold_phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldPhase {
    Requesting,
    Draining,
    HandingBack,
    Held,
    Releasing,
}

impl HoldPhase {
    /// One of the acquire's three phases.
    pub fn acquiring(self) -> bool {
        matches!(self, Self::Requesting | Self::Draining | Self::HandingBack)
    }
}

/// Whether a restored hold had been handed over before the Runtime stopped (vocabulary
/// `player_hold_restored_from`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    Acquiring,
    Held,
}

/// Who began or ended a hold operation, by its channel only, and the channel a Lab request came
/// through (`via`); its actor goes to the log only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldSource {
    pub channel: HoldChannel,
    pub via: Option<HoldChannel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldChannel {
    Ui,
    Cli,
    Mcp,
    Lab,
}

/// The last accepted hold operation of the instance in this owner epoch. `of_hold`: its `hold_id`
/// is the hold's now. `code` and `failed_phase` of a failed one; `cancelled`, how many runs its
/// drain asked to stop, of an acquire that drained; `next`, `selfcheck_failure` and `startup` of
/// a completed release; `request`, its request, opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldRecord {
    pub kind: HoldKind,
    pub state: RecordState,
    pub phase: Option<HoldPhase>,
    pub of_hold: bool,
    pub source: HoldSource,
    pub ended_ms: Option<u64>,
    pub code: Option<String>,
    pub failed_phase: Option<HoldPhase>,
    pub cancelled: Option<u32>,
    pub next: Option<HoldNext>,
    pub selfcheck_failure: Option<String>,
    pub startup: Option<StartupDone>,
    pub request: HoldRequestId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldKind {
    Acquire,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordState {
    Running,
    Completed,
    Failed,
}

/// What a release did next (vocabulary `player_hold_next`): reconnected the device, or touched
/// nothing under the instance's pause or the global one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldNext {
    Reconnected,
    InstancePaused,
    SchedulingPaused,
}

/// What a release did with the instance's startup package: queued one, behind the pause when
/// `withheld`; cancelled one queued; nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupDone {
    Scheduled { withheld: bool },
    Cancelled,
    Nothing,
}

/// The hold a release names: the owner epoch of the status read, the hold's id and its slot
/// revision, which the Runtime compares under its hold table's lock (Runtime f6e2cd26
/// `contracts/player-hold.md` 101-110). Made only
/// from a status read; [`HoldTarget::line`] is for the log only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldTarget(PlayerHoldExpectation);

impl HoldTarget {
    /// Whether this is the hold the acquire `request` set: a hold's id is its acquire's request.
    pub fn is_of(&self, request: &HoldRequestId) -> bool {
        self.0.hold_id == request.0
    }

    /// For the technical log only, never shown.
    pub fn line(&self) -> String {
        format!(
            "hold {} revision {} in owner epoch {}",
            code(&self.0.hold_id),
            self.0.revision,
            code(&self.0.owner_epoch)
        )
    }
}

/// One hold request's id, opaque outside this crate, as [`super::RunKey`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldRequestId(RequestId);

impl HoldRequestId {
    /// For the technical log only, never shown.
    pub fn line(&self) -> String {
        code(&self.0)
    }
}

/// One `Status` as it was read, kept whole for the settlement of a stored attempt alone
/// ([`settle_stored`]); nothing else outside this crate reads it. Its `Debug` prints only its owner
/// epoch.
#[derive(Clone)]
pub struct StatusCopy(Arc<RuntimeControlPlaneStatus>);

impl StatusCopy {
    pub(crate) fn new(status: RuntimeControlPlaneStatus) -> Self {
        Self(Arc::new(status))
    }
}

impl fmt::Debug for StatusCopy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("StatusCopy").field(&code(&self.0.owner_epoch())).finish()
    }
}

/// The hold fact of the instance `alias` of `status`, read by the contract's own description of
/// it, `settle_player_hold` without an attempt (Runtime f6e2cd26 contract `player_hold.rs`
/// 1052-1059); `None` with neither a hold nor a record. A status that does not list the instance
/// is not asked about: the caller asks of the instances it lists, and such an answer is `None`
/// too. Every time is as the status stored it, never computed again (Runtime f6e2cd26
/// `contracts/player-hold.md` 170-173).
pub fn hold_fact(status: &RuntimeControlPlaneStatus, alias: &str) -> Option<HoldFact> {
    let PlayerHoldSettlement::Current(view) = settle_player_hold(None, alias, Some(status)) else {
        return None;
    };
    let epoch = status.owner_epoch();
    let hold = view.hold.as_ref().map(|hold| hold_now(hold, epoch));
    let record = view.operation.as_ref().map(|record| hold_record(record, view.hold.as_ref()));
    if hold.is_none() && record.is_none() {
        return None;
    }
    Some(HoldFact { hold, record, ended_with_pause: view.hold_ended_with_pause })
}

fn hold_now(hold: &PlayerHoldState, owner_epoch: OwnerEpoch) -> HoldNow {
    HoldNow {
        phase: phase_of(hold.phase),
        since_ms: hold.since_unix_ms,
        reached_held_ms: hold.reached_held_unix_ms,
        source: source_of(hold.source),
        drain_deadline_ms: hold.drain_deadline_unix_ms,
        grace_deadline_ms: hold.grace_deadline_unix_ms,
        close_started_ms: hold.close_started_unix_ms,
        restored: hold.restored_from_phase.map(|from| match from {
            PlayerHoldRestoredFrom::Acquiring => Restored::Acquiring,
            PlayerHoldRestoredFrom::Held => Restored::Held,
        }),
        target: HoldTarget(PlayerHoldExpectation {
            owner_epoch,
            hold_id: hold.hold_id,
            revision: hold.revision,
        }),
    }
}

fn hold_record(record: &PlayerHoldOperation, hold: Option<&PlayerHoldState>) -> HoldRecord {
    let result = record.result.as_ref();
    HoldRecord {
        kind: kind_of(record.kind),
        state: match record.state {
            PlayerHoldOperationState::Running => RecordState::Running,
            PlayerHoldOperationState::Completed => RecordState::Completed,
            PlayerHoldOperationState::Failed => RecordState::Failed,
        },
        phase: record.phase.map(phase_of),
        of_hold: hold.is_some_and(|hold| hold.hold_id == record.hold_id),
        source: source_of(record.source),
        ended_ms: record.ended_unix_ms,
        code: record.code.as_ref().map(|code| code.as_str().to_owned()),
        failed_phase: record.failed_phase.map(phase_of),
        cancelled: result.and_then(|result| result.drained.as_ref()).map(|drained| drained.cancelled),
        next: result.and_then(|result| result.next).map(next_of),
        selfcheck_failure: result
            .and_then(|result| result.selfcheck.as_ref())
            .and_then(|selfcheck| selfcheck.failure_code.clone()),
        startup: result.and_then(|result| {
            let withheld = result.startup_package_withheld.is_some();
            result.startup_package.map(|package| startup_of(package, withheld))
        }),
        request: HoldRequestId(record.request_id),
    }
}

fn phase_of(phase: PlayerHoldPhase) -> HoldPhase {
    match phase {
        PlayerHoldPhase::Requesting => HoldPhase::Requesting,
        PlayerHoldPhase::Draining => HoldPhase::Draining,
        PlayerHoldPhase::HandingBack => HoldPhase::HandingBack,
        PlayerHoldPhase::Held => HoldPhase::Held,
        PlayerHoldPhase::Releasing => HoldPhase::Releasing,
    }
}

fn kind_of(kind: PlayerHoldOperationKind) -> HoldKind {
    match kind {
        PlayerHoldOperationKind::Acquire => HoldKind::Acquire,
        PlayerHoldOperationKind::Release => HoldKind::Release,
    }
}

fn next_of(next: PlayerHoldNext) -> HoldNext {
    match next {
        PlayerHoldNext::Reconnected => HoldNext::Reconnected,
        PlayerHoldNext::InstancePaused => HoldNext::InstancePaused,
        PlayerHoldNext::SchedulingPaused => HoldNext::SchedulingPaused,
    }
}

fn startup_of(package: PlayerHoldStartupPackage, withheld: bool) -> StartupDone {
    match package {
        PlayerHoldStartupPackage::Scheduled => StartupDone::Scheduled { withheld },
        PlayerHoldStartupPackage::Cancelled => StartupDone::Cancelled,
        PlayerHoldStartupPackage::None => StartupDone::Nothing,
    }
}

/// By its channel only; its actor is not kept (the Lab's own work on the source's shape may
/// change it).
fn source_of(source: PlayerHoldSource) -> HoldSource {
    let channel = match source.channel {
        RuntimeChannel::Ui => HoldChannel::Ui,
        RuntimeChannel::Cli => HoldChannel::Cli,
        RuntimeChannel::Mcp => HoldChannel::Mcp,
        RuntimeChannel::Lab => HoldChannel::Lab,
    };
    let via = source.via.map(|via| match via {
        ChannelVia::Mcp => HoldChannel::Mcp,
        ChannelVia::Ui => HoldChannel::Ui,
    });
    HoldSource { channel, via }
}

/// What the console may do with an instance's picture and device now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldGate {
    Open,
    /// Not read: the picture stays on its last frame, and no device button is pressed.
    Freeze,
    /// The device is a person's: nothing of it shows, and no device button is drawn.
    Withdrawn,
}

/// The gate of an instance with the hold fact `fact`, `asking` while the console's own acquire of
/// it is out or not yet followed by a `Status` (the first that holds): a hold held is withdrawn;
/// an ask of this console's, a hold in any other phase, and a running record without a hold (an
/// acquire entered before its hold, or a release whose hold is already lifted while it reconnects)
/// freeze; anything else is open. The Runtime's own device gate is the hold alone (Runtime
/// f6e2cd26 contract `player_hold.rs` 714-718): a pause the hold ended under opens it again.
pub fn gate(fact: Option<&HoldFact>, asking: bool) -> HoldGate {
    let hold = fact.and_then(|fact| fact.hold.as_ref());
    if hold.is_some_and(|hold| hold.phase == HoldPhase::Held) {
        return HoldGate::Withdrawn;
    }
    if asking || hold.is_some() {
        return HoldGate::Freeze;
    }
    let running = fact
        .and_then(|fact| fact.record.as_ref())
        .is_some_and(|record| record.state == RecordState::Running);
    if running {
        HoldGate::Freeze
    } else {
        HoldGate::Open
    }
}

// ---- The two writes ----

/// What a release asks for the instance's startup package: queue one (`run`) or cancel the one
/// queued (`skip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupChoice {
    Run,
    Skip,
}

/// One request: an acquire, or a release of the hold `target` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldAsk {
    Acquire,
    Release { target: HoldTarget, startup: StartupChoice },
}

impl HoldAsk {
    pub fn kind(&self) -> HoldKind {
        match self {
            Self::Acquire => HoldKind::Acquire,
            Self::Release { .. } => HoldKind::Release,
        }
    }
}

/// What became of one [`send`]. `chain`, in each failure, has every code of it, for the log only.
#[derive(Debug, Clone)]
pub enum HoldOutcome {
    /// The hold is held: the device is the person's. `cancelled`: runs its drain asked to stop.
    Acquired { cancelled: u32 },
    /// The hold is lifted: what came next, the self-check's failure of a reconnect, and the
    /// startup package.
    Released { next: HoldNext, selfcheck_failure: Option<String>, startup: StartupDone },
    /// This Runtime has no player hold (`not_implemented`, v0.12.76 and before), and recorded
    /// nothing.
    NotImplemented { failure: ClientFailure, chain: String },
    /// The Runtime answered and did not do it: why, its code (for the log and the words), and
    /// whether it denied the request at its entry, which records nothing, or failed after taking
    /// it (an acquire that did not drain or hand over in time, a hold not recorded).
    Refused { why: HoldRefusal, code: String, denied: bool, failure: ClientFailure, chain: String },
    /// `player_hold_interrupted`: the Runtime is shutting down and keeps the hold; the stored
    /// attempt settles once it started again.
    Interrupted { failure: ClientFailure, chain: String },
    /// The receipt never came, and the client settled the request from a `Status` it read: what
    /// that came to; the table's row that decided (`settlement_rule`) and every code of the
    /// client's failure, the one that lost the receipt among them, for the log only.
    Settled { said: HoldSettled, rule: &'static str, chain: String },
    /// No connection, or the request was not prepared: nothing was sent.
    NotSent { failure: ClientFailure, chain: String },
    /// The attempt could not be stored: nothing was sent.
    NotStored { error: String },
    /// Sent, or perhaps sent, and nothing says what became of it: the stored attempt settles it
    /// from the next `Status`.
    Unconfirmed { failure: ClientFailure, chain: String },
}

/// Why the Runtime did not do a request, by its receipt's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldRefusal {
    /// The instance is held already, or being taken over or handed back.
    AlreadyBusy,
    /// A release of an instance with no hold.
    NotHeld,
    /// A release of a hold that changed: another epoch, another hold or another revision.
    Moved,
    /// A release whose reconnect claim was not had in time: the hold stays.
    DeviceBusy,
    /// Any other: its code says.
    Other,
}

/// The kind of refusal `code` names (Runtime f6e2cd26 `crates/runtime-host/src/codes.rs`
/// 1007-1022; `lease_queue_expired`, `contracts/player-hold.md` 121-122).
pub fn refusal_of(code: &str) -> HoldRefusal {
    match code {
        PLAYER_HOLD_ALREADY_HELD | PLAYER_HOLD_ACQUIRING | PLAYER_HOLD_RELEASING => {
            HoldRefusal::AlreadyBusy
        }
        PLAYER_HOLD_NOT_HELD => HoldRefusal::NotHeld,
        PLAYER_HOLD_EXPECTATION_MISMATCH => HoldRefusal::Moved,
        LEASE_QUEUE_EXPIRED => HoldRefusal::DeviceBusy,
        _ => HoldRefusal::Other,
    }
}

/// What a failed submit says, before it is told apart ([`classify`]): whether the client settled
/// it; the receipt's code, when a receipt came; and whether the Runtime answered it as a refusal it
/// went on after, of a request nothing leaves uncertain (as a stop's, `stop.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureFacts<'a> {
    pub settled: bool,
    pub code: Option<&'a str>,
    pub answered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Settled,
    Interrupted,
    NotImplemented,
    Refused,
    Unconfirmed,
}

/// A failed submit told apart, the first that holds: settled by the client; a receipt of
/// `player_hold_interrupted`; an answered `not_implemented`; any other answered refusal; anything
/// else is unconfirmed, the handle latched and the stored attempt kept for the next `Status`.
pub fn classify(facts: &FailureFacts<'_>) -> FailureClass {
    if facts.settled {
        return FailureClass::Settled;
    }
    if facts.code == Some(PLAYER_HOLD_INTERRUPTED) {
        return FailureClass::Interrupted;
    }
    if !facts.answered {
        return FailureClass::Unconfirmed;
    }
    if facts.code == Some(NOT_IMPLEMENTED) {
        FailureClass::NotImplemented
    } else {
        FailureClass::Refused
    }
}

/// One prepared request, of either kind.
enum Prepared {
    Acquire(PreparedPlayerHoldAcquire),
    Release(PreparedPlayerHoldRelease),
}

impl Prepared {
    fn attempt(&self, trigger: PlayerHoldSettlementTrigger) -> PlayerHoldAttempt {
        match self {
            Self::Acquire(prepared) => prepared.attempt(trigger),
            Self::Release(prepared) => prepared.attempt(trigger),
        }
    }
}

/// One request of the instance `alias` of the state root `root`: a fresh person's connection, the
/// request prepared, its attempt handed to `store`, and only once that took, the request submitted
/// once. Whatever comes back, nothing is sent again. The attempt stored, when it was, comes back
/// beside the outcome.
pub fn send(
    root: &Path,
    alias: &str,
    ask: &HoldAsk,
    store: &mut dyn FnMut(&StoredAttempt) -> Result<(), String>,
) -> (HoldOutcome, Option<StoredAttempt>) {
    let client = match runtime_connection(configure_config(root)) {
        Ok(client) => client,
        Err(NoConnection::Failed(error)) => {
            let chain = chain(&error.outcome());
            return (HoldOutcome::NotSent { failure: ClientFailure::from(error), chain }, None);
        }
        Err(refused) => {
            let failure = ClientFailure::from(refused);
            return (HoldOutcome::NotSent { chain: failure.code.to_owned(), failure }, None);
        }
    };
    let prepared = match prepare(&client, alias, ask) {
        Ok(prepared) => prepared,
        Err(error) => {
            let chain = chain(&error.outcome());
            return (HoldOutcome::NotSent { failure: ClientFailure::from(error), chain }, None);
        }
    };
    let stored =
        StoredAttempt::new(root, alias, prepared.attempt(PlayerHoldSettlementTrigger::Reopened));
    if let Err(error) = store(&stored) {
        return (HoldOutcome::NotStored { error }, None);
    }
    let outcome = match prepared {
        Prepared::Acquire(prepared) => match client.submit_acquire_player_hold(prepared) {
            Ok(acquired) => HoldOutcome::Acquired { cancelled: acquired.drained.cancelled },
            Err(error) => failed(error),
        },
        Prepared::Release(prepared) => match client.submit_release_player_hold(prepared) {
            Ok(released) => HoldOutcome::Released {
                next: next_of(released.next),
                selfcheck_failure: released.selfcheck.and_then(|selfcheck| selfcheck.failure_code),
                startup: startup_of(
                    released.startup_package,
                    released.startup_package_withheld.is_some(),
                ),
            },
            Err(error) => failed(error),
        },
    };
    (outcome, Some(stored))
}

/// The request of `ask`, prepared on `client`, which writes nothing.
fn prepare(
    client: &RuntimeClient,
    alias: &str,
    ask: &HoldAsk,
) -> Result<Prepared, RuntimeClientError> {
    match ask {
        HoldAsk::Acquire => {
            client.prepare_acquire_player_hold(alias, ACQUIRE_DRAIN_MS).map(Prepared::Acquire)
        }
        HoldAsk::Release { target, startup } => {
            let startup = match startup {
                StartupChoice::Run => PlayerHoldStartupChoice::Run,
                StartupChoice::Skip => PlayerHoldStartupChoice::Skip,
            };
            client
                .prepare_release_player_hold(alias, target.0, Some(startup))
                .map(Prepared::Release)
        }
    }
}

/// A failed submit, its facts read off the client's failure and told apart by [`classify`].
fn failed(error: RuntimeClientError) -> HoldOutcome {
    let chain = chain(&error.outcome());
    let settlement = error.player_hold_settlement();
    let rule = settlement
        .and_then(PlayerHoldSettlement::decision)
        .map_or("none", |decision| decision.rule.as_str());
    let settled = settlement.map(settled_of);
    // The receipt's own code, not the client's (`runtime_request_rejected`) nor its operation.
    let code = error.receipt_outcome().map(|outcome| outcome.code().as_str().to_owned());
    let answered = error.projection().is_some_and(|refusal| !refusal.fatal)
        && !matches!(error.disposition(), RuntimeClientErrorClass::Uncertain);
    let denied = error
        .received_receipt()
        .is_some_and(|receipt| matches!(receipt.state(), RuntimeReceiptState::Denied));
    let class =
        classify(&FailureFacts { settled: settled.is_some(), code: code.as_deref(), answered });
    let failure = ClientFailure::from(error);
    match (class, settled) {
        (FailureClass::Settled, Some(said)) => HoldOutcome::Settled { said, rule, chain },
        (FailureClass::Interrupted, _) => HoldOutcome::Interrupted { failure, chain },
        (FailureClass::NotImplemented, _) => HoldOutcome::NotImplemented { failure, chain },
        (FailureClass::Refused, _) => {
            let code = code.unwrap_or_default();
            HoldOutcome::Refused { why: refusal_of(&code), code, denied, failure, chain }
        }
        (FailureClass::Settled, None) | (FailureClass::Unconfirmed, _) => {
            HoldOutcome::Unconfirmed { failure, chain }
        }
    }
}

// ---- The settlement ----

/// What a settlement came to, before its words. Of the table's rows (Runtime f6e2cd26
/// `contracts/player-hold.md` 457-504):
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldSettled {
    /// A2, A4, A6, A11, A12, R2: it was done. `restored`: a start restored the hold since;
    /// `ended_by`: who ended the hold since, or is ending it; `record`: its own record, when the
    /// instance's record now is its own.
    Done {
        kind: HoldKind,
        restored: bool,
        ended_by: Option<HoldSource>,
        record: Option<HoldRecord>,
    },
    /// A3, R3: it failed after it was taken; `code`, its own record's.
    Failed { kind: HoldKind, code: Option<String> },
    /// A8, R5, R10: it was not done, and will not be.
    NotPerformed { kind: HoldKind },
    /// R7: another party ended the hold.
    EndedByOther { source: HoldSource },
    /// R9, R11: the hold has ended; who ended it is not known.
    Ended,
    /// A7, R8: a Runtime without the hold dropped the request.
    Unsupported,
    /// An unconfirmed settlement that a later read may still settle (not final).
    Checking,
    /// An unconfirmed settlement that is final.
    Unconfirmed,
}

/// The settlement `settlement` said in [`HoldSettled`]: final or not by `is_final` (on the wire
/// `final`, Runtime f6e2cd26 contract `player_hold.rs` 971-972); its own record only when the
/// instance's record now is the attempt's own request; who ended the hold by `ended_by`'s source.
fn settled_of(settlement: &PlayerHoldSettlement) -> HoldSettled {
    match settlement.decision() {
        Some(decision) => decided(decision),
        // Only without an attempt, which the client never settles: not confirmed.
        None => HoldSettled::Unconfirmed,
    }
}

fn decided(decision: &PlayerHoldDecision) -> HoldSettled {
    let unconfirmed =
        if decision.is_final { HoldSettled::Unconfirmed } else { HoldSettled::Checking };
    match decision.verdict {
        PlayerHoldSettlementVerdict::Unsupported => return HoldSettled::Unsupported,
        PlayerHoldSettlementVerdict::Unconfirmed => return unconfirmed,
        PlayerHoldSettlementVerdict::Settled => {}
    }
    let Some(attempt) = decision.attempt.as_ref() else {
        return unconfirmed;
    };
    let kind = kind_of(attempt.kind);
    let own = decision
        .operation
        .as_ref()
        .filter(|record| record.request_id == attempt.request_id)
        .map(|record| hold_record(record, decision.hold.as_ref()));
    let ended_by = decision.ended_by.map(|ended| source_of(ended.source));
    match decision.settlement_outcome {
        Some(PlayerHoldSettlementOutcome::Completed) => {
            HoldSettled::Done { kind, restored: decision.restored, ended_by, record: own }
        }
        Some(PlayerHoldSettlementOutcome::Failed) => {
            HoldSettled::Failed { kind, code: own.and_then(|record| record.code) }
        }
        Some(PlayerHoldSettlementOutcome::NotPerformed) => HoldSettled::NotPerformed { kind },
        Some(PlayerHoldSettlementOutcome::EndedByOther) => match ended_by {
            Some(source) => HoldSettled::EndedByOther { source },
            None => HoldSettled::Ended,
        },
        Some(PlayerHoldSettlementOutcome::HoldEnded) => HoldSettled::Ended,
        None => unconfirmed,
    }
}

/// What a stored attempt's settlement came to: its words' value; whether the attempt is kept, to
/// be settled again; and the table's row that decided, for the log only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSettled {
    pub said: HoldSettled,
    pub keep: bool,
    pub rule: &'static str,
}

/// `stored` settled by `status`, a `Status` of its state root (contract `settle_player_hold`,
/// Runtime f6e2cd26 `player_hold.rs` 1024-1214). It is let go once settled or unsupported, or once
/// `status` is of another owner epoch than the one it was sent in; any other settlement keeps it,
/// final or not (A9, A10, R6, P2 and A5 of its own epoch among them), to be settled again by a
/// later `Status` (`contracts/player-hold.md` 450-454). Call it only while no request of the
/// console's own of that
/// instance is out (1020-1023).
pub fn settle_stored(stored: &StoredAttempt, status: &StatusCopy) -> StoredSettled {
    let settlement = settle_player_hold(Some(&stored.attempt), &stored.alias, Some(&*status.0));
    let decision = settlement.decision();
    let rule = decision.map_or("none", |decision| decision.rule.as_str());
    let known = decision.is_some_and(|decision| {
        matches!(
            decision.verdict,
            PlayerHoldSettlementVerdict::Settled | PlayerHoldSettlementVerdict::Unsupported
        )
    });
    let same_epoch = status.0.owner_epoch() == stored.attempt.sent_in_owner_epoch;
    StoredSettled { said: settled_of(&settlement), keep: !known && same_epoch, rule }
}

/// Whether the attempt a request stored is kept after its outcome landed: while what it did may
/// still be learnt from a later `Status` (an interrupted acquire, one unconfirmed, a settlement not
/// final or final and unconfirmed). Every real receipt's effect is known, and an entry refusal
/// records nothing (Runtime f6e2cd26 `contracts/player-hold.md` 92, an acquire's; 97-108, a
/// release's, whose record only its step 9 writes).
pub fn keeps_attempt(outcome: &HoldOutcome) -> bool {
    matches!(
        outcome,
        HoldOutcome::Interrupted { .. }
            | HoldOutcome::Unconfirmed { .. }
            | HoldOutcome::Settled {
                said: HoldSettled::Checking | HoldSettled::Unconfirmed,
                ..
            }
    )
}

// ---- The stored attempts ----

/// The record file's version, and its members' names.
const FILE_VERSION: u64 = 1;
const VERSION: &str = "version";
const ATTEMPTS: &str = "attempts";
const STATE_ROOT: &str = "state_root";
const ALIAS: &str = "alias";
const ATTEMPT: &str = "attempt";

/// One request this console sent, as it keeps it: the state root it was sent to, as the console's
/// launcher names it, the instance, and the contract's attempt of it, its trigger `reopened`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAttempt {
    root: String,
    alias: String,
    attempt: PlayerHoldAttempt,
}

impl StoredAttempt {
    pub fn new(root: &Path, alias: &str, attempt: PlayerHoldAttempt) -> Self {
        Self { root: root.to_string_lossy().into_owned(), alias: alias.to_owned(), attempt }
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// Whether it was sent to the state root `root` ([`same_root`]).
    pub fn of_root(&self, root: &Path) -> bool {
        same_root(&self.root, root)
    }

    pub fn kind(&self) -> HoldKind {
        kind_of(self.attempt.kind)
    }

    pub fn request(&self) -> HoldRequestId {
        HoldRequestId(self.attempt.request_id)
    }

    /// Its kind, its request and the owner epoch it was sent in: for the technical log only.
    pub fn line(&self) -> String {
        let kind = match self.attempt.kind {
            PlayerHoldOperationKind::Acquire => "acquire",
            PlayerHoldOperationKind::Release => "release",
        };
        format!(
            "{kind} {} of {} sent in owner epoch {}",
            code(&self.attempt.request_id),
            self.alias,
            code(&self.attempt.sent_in_owner_epoch)
        )
    }

    /// This item of the record file, as the file writes it: for the technical log, once as it is
    /// stored.
    pub fn json(&self) -> String {
        // An attempt is plain members; one that did not serialize would be written as its error,
        // which no read of the file takes, so the caller's check of what it renders refuses it.
        let attempt = serde_json::to_value(&self.attempt)
            .unwrap_or_else(|error| Value::String(format!("unserializable: {error}")));
        format!(
            "{{\"{STATE_ROOT}\":{},\"{ALIAS}\":{},\"{ATTEMPT}\":{attempt}}}",
            Value::String(self.root.clone()),
            Value::String(self.alias.clone()),
        )
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let item = exact_object(value, &[STATE_ROOT, ALIAS, ATTEMPT])?;
        let text = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} is not a string"))
        };
        let root = text(STATE_ROOT)?;
        let alias = text(ALIAS)?;
        let attempt = item.get(ATTEMPT).cloned().unwrap_or_default();
        let attempt = serde_json::from_value::<PlayerHoldAttempt>(attempt)
            .map_err(|error| format!("{ATTEMPT} does not read: {error}"))?;
        Ok(Self { root, alias, attempt })
    }

    fn matches(&self, other: &StoredAttempt) -> bool {
        same_root_text(&self.root, &other.root)
            && self.alias == other.alias
            && self.attempt.request_id == other.attempt.request_id
    }
}

/// The console's record file of the requests it sent, in the order they were put in, the newest
/// last: `{"version":1,"attempts":[{"state_root":…,"alias":…,"attempt":{…}}]}`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HoldFile {
    attempts: Vec<StoredAttempt>,
}

impl HoldFile {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The file's text read: no member more or less, at the top and in each item, than the file
    /// has; version 1; each attempt the contract's, which takes no member it does not know either.
    /// A request put in twice is kept once.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
        let top = exact_object(&value, &[VERSION, ATTEMPTS])?;
        if top.get(VERSION).and_then(Value::as_u64) != Some(FILE_VERSION) {
            return Err(format!("{VERSION} is not {FILE_VERSION}"));
        }
        let items =
            top.get(ATTEMPTS).and_then(Value::as_array).ok_or(format!("{ATTEMPTS} is not a list"))?;
        let mut file = Self::empty();
        for (index, item) in items.iter().enumerate() {
            let stored = StoredAttempt::from_value(item)
                .map_err(|error| format!("{ATTEMPTS}[{index}]: {error}"))?;
            file.put(stored);
        }
        Ok(file)
    }

    /// The file's text, its items in their order.
    pub fn render(&self) -> String {
        let items: Vec<String> = self.attempts.iter().map(StoredAttempt::json).collect();
        format!("{{\"{VERSION}\":{FILE_VERSION},\"{ATTEMPTS}\":[{}]}}", items.join(","))
    }

    /// `stored`, last, unless its request is in already.
    pub fn put(&mut self, stored: StoredAttempt) {
        let request = stored.attempt.request_id;
        if self.attempts.iter().any(|kept| kept.attempt.request_id == request) {
            return;
        }
        self.attempts.push(stored);
    }

    /// `stored` taken out, by its state root ([`same_root`]), its instance and its request:
    /// whether it was in.
    pub fn take(&mut self, stored: &StoredAttempt) -> bool {
        let before = self.attempts.len();
        self.attempts.retain(|kept| !kept.matches(stored));
        self.attempts.len() != before
    }

    /// The attempts sent to the state root `root`, in their order.
    pub fn of_root<'a>(&'a self, root: &'a Path) -> impl Iterator<Item = &'a StoredAttempt> {
        self.attempts.iter().filter(move |stored| stored.of_root(root))
    }
}

/// `value` as an object of exactly the members `keys`.
fn exact_object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let object = value.as_object().ok_or("not an object")?;
    if let Some(unknown) = object.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(format!("the member {unknown} is not known"));
    }
    if let Some(missing) = keys.iter().find(|key| !object.contains_key(**key)) {
        return Err(format!("the member {missing} is missing"));
    }
    Ok(object)
}

/// Whether the state root `a`, as stored, is `b`: by their text, `/` as `\`, without a separator
/// at the end, ASCII letters of either case alike. Another spelling of the same folder, through a
/// junction or a substituted drive, is not told to be it.
pub fn same_root(a: &str, b: &Path) -> bool {
    same_root_text(a, &b.to_string_lossy())
}

fn same_root_text(a: &str, b: &str) -> bool {
    root_key(a) == root_key(b)
}

fn root_key(root: &str) -> String {
    let mut key: String = root
        .chars()
        .map(|letter| if letter == '/' { '\\' } else { letter.to_ascii_lowercase() })
        .collect();
    while key.ends_with('\\') {
        key.pop();
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use actingcommand_contract::outcome::CodeStr;
    use actingcommand_contract::{
        IdentifierIssuer, InstancePauseStage, InstancePauseState, PlayerHoldOperationResult,
        PlayerHoldPausesAtEnd, RuntimeInstanceStatus, SchedulingDrainSummary,
        SchedulingPauseReason, SchedulingResumeSelfCheck,
    };
    use serde_json::json;
    use std::path::PathBuf;

    const A: &str = "node.a";
    const B: &str = "node.b";
    const ROOT: &str = r"D:\AC\state";

    struct Ids {
        issuer: IdentifierIssuer,
        e0: OwnerEpoch,
        e1: OwnerEpoch,
        r: RequestId,
        h: RequestId,
        other: RequestId,
    }

    fn ids() -> Ids {
        let issuer = IdentifierIssuer::new().expect("identifier issuer");
        let epoch = |issuer: &IdentifierIssuer| {
            *issuer.mint_owner_epoch().expect("owner epoch").transport()
        };
        let request = |issuer: &IdentifierIssuer| {
            *issuer.mint_request_id().expect("request id").transport()
        };
        Ids {
            e0: epoch(&issuer),
            e1: epoch(&issuer),
            r: request(&issuer),
            h: request(&issuer),
            other: request(&issuer),
            issuer,
        }
    }

    fn source(channel: RuntimeChannel) -> PlayerHoldSource {
        let actor = match channel {
            RuntimeChannel::Ui => actingcommand_contract::PlayerHoldSourceActor::User,
            RuntimeChannel::Cli | RuntimeChannel::Mcp => {
                actingcommand_contract::PlayerHoldSourceActor::Cli
            }
            RuntimeChannel::Lab => actingcommand_contract::PlayerHoldSourceActor::Lab,
        };
        PlayerHoldSource { actor, channel, via: None }
    }

    /// A hold held, handed over at 2 000, taken at 1 000, from the command line.
    fn held(hold_id: RequestId) -> PlayerHoldState {
        PlayerHoldState {
            hold_id,
            revision: 1,
            phase: PlayerHoldPhase::Held,
            since_unix_ms: 1_000,
            source: source(RuntimeChannel::Cli),
            drain_deadline_unix_ms: None,
            grace_deadline_unix_ms: None,
            close_started_unix_ms: None,
            reached_held_unix_ms: Some(2_000),
            restored_from_owner_epoch: None,
            restored_from_phase: None,
        }
    }

    /// A hold in one of the acquire's three phases, with its two deadlines.
    fn acquiring(hold_id: RequestId, phase: PlayerHoldPhase) -> PlayerHoldState {
        PlayerHoldState {
            phase,
            drain_deadline_unix_ms: Some(61_000),
            grace_deadline_unix_ms: Some(91_000),
            reached_held_unix_ms: None,
            ..held(hold_id)
        }
    }

    fn code_str(spelling: &str) -> CodeStr {
        serde_json::from_value(json!(spelling)).expect("code")
    }

    fn selfcheck(failure_code: Option<&str>) -> SchedulingResumeSelfCheck {
        serde_json::from_value(json!({
            "capture": {"ok": true, "width": 1280, "height": 720, "capture_backend": "fake"},
            "touch": {"ok": true, "backend": "fake", "max_x": 1280, "max_y": 720, "invasive": false},
            "failure_code": failure_code
        }))
        .expect("self-check")
    }

    /// A completed release's result of `next`, as the contract's shape check takes it.
    fn release_result(next: PlayerHoldNext) -> PlayerHoldOperationResult {
        PlayerHoldOperationResult {
            revision: Some(2),
            next: Some(next),
            selfcheck: (next == PlayerHoldNext::Reconnected).then(|| selfcheck(None)),
            startup_package: Some(PlayerHoldStartupPackage::None),
            pauses_at_end: match next {
                PlayerHoldNext::Reconnected => None,
                PlayerHoldNext::InstancePaused => Some(PlayerHoldPausesAtEnd {
                    instance_revision: Some(3),
                    global_revision: None,
                }),
                PlayerHoldNext::SchedulingPaused => Some(PlayerHoldPausesAtEnd {
                    instance_revision: None,
                    global_revision: Some(3),
                }),
            },
            ..PlayerHoldOperationResult::default()
        }
    }

    /// A record of `kind` in `state`, with the members its state asks for.
    fn record(
        kind: PlayerHoldOperationKind,
        state: PlayerHoldOperationState,
        request_id: RequestId,
        hold_id: RequestId,
    ) -> PlayerHoldOperation {
        let mut record = PlayerHoldOperation {
            request_id,
            kind,
            state,
            phase: None,
            hold_id,
            source: source(RuntimeChannel::Cli),
            since_unix_ms: 1_000,
            ended_unix_ms: None,
            code: None,
            failed_phase: None,
            result: None,
        };
        match state {
            PlayerHoldOperationState::Running => {
                record.phase = Some(match kind {
                    PlayerHoldOperationKind::Acquire => PlayerHoldPhase::Draining,
                    PlayerHoldOperationKind::Release => PlayerHoldPhase::Releasing,
                });
            }
            PlayerHoldOperationState::Completed => {
                record.ended_unix_ms = Some(3_000);
                record.result = Some(match kind {
                    PlayerHoldOperationKind::Acquire => PlayerHoldOperationResult {
                        revision: Some(1),
                        drained: Some(SchedulingDrainSummary { finished: 0, cancelled: 1 }),
                        ..PlayerHoldOperationResult::default()
                    },
                    PlayerHoldOperationKind::Release => {
                        release_result(PlayerHoldNext::Reconnected)
                    }
                });
            }
            PlayerHoldOperationState::Failed => {
                record.ended_unix_ms = Some(3_000);
                record.code = Some(code_str(PLAYER_HOLD_DRAIN_INCOMPLETE));
                record.failed_phase = Some(match kind {
                    PlayerHoldOperationKind::Acquire => PlayerHoldPhase::Draining,
                    PlayerHoldOperationKind::Release => PlayerHoldPhase::Releasing,
                });
            }
        }
        record
    }

    fn pause(revision: u64) -> InstancePauseState {
        InstancePauseState {
            revision,
            pause_reason: SchedulingPauseReason::new("operator.stop").expect("reason"),
            since_unix_ms: 10,
            stage: InstancePauseStage::Released,
            pause_mode: None,
            in_the_way: Vec::new(),
        }
    }

    fn instance(
        ids: &Ids,
        alias: &str,
        hold: Option<PlayerHoldState>,
        operation: Option<PlayerHoldOperation>,
        paused: Option<u64>,
    ) -> RuntimeInstanceStatus {
        RuntimeInstanceStatus::new(
            alias,
            *ids.issuer.mint_instance_id().expect("instance id").transport(),
            false,
            0,
            false,
            false,
            false,
        )
        .expect("instance")
        .with_pause(paused.map(pause))
        .expect("pause")
        .with_player_hold(hold)
        .expect("hold")
        .with_player_hold_operation(operation)
        .expect("record")
    }

    /// A status of `epoch` with the instance `A` as named, and `B` bare.
    fn status(
        ids: &Ids,
        epoch: OwnerEpoch,
        hold: Option<PlayerHoldState>,
        operation: Option<PlayerHoldOperation>,
    ) -> RuntimeControlPlaneStatus {
        status_paused(ids, epoch, hold, operation, None)
    }

    fn status_paused(
        ids: &Ids,
        epoch: OwnerEpoch,
        hold: Option<PlayerHoldState>,
        operation: Option<PlayerHoldOperation>,
        paused: Option<u64>,
    ) -> RuntimeControlPlaneStatus {
        RuntimeControlPlaneStatus::new(
            epoch,
            vec![instance(ids, A, hold, operation, paused), instance(ids, B, None, None, None)],
        )
        .expect("status")
    }

    fn fact(status: &RuntimeControlPlaneStatus) -> HoldFact {
        hold_fact(status, A).expect("a hold fact")
    }

    fn attempt(
        ids: &Ids,
        kind: PlayerHoldOperationKind,
        trigger: PlayerHoldSettlementTrigger,
    ) -> PlayerHoldAttempt {
        PlayerHoldAttempt {
            kind,
            request_id: ids.r,
            sent_in_owner_epoch: ids.e0,
            expected_hold_id: match kind {
                PlayerHoldOperationKind::Acquire => ids.r,
                PlayerHoldOperationKind::Release => ids.h,
            },
            trigger,
        }
    }

    fn stored(ids: &Ids, kind: PlayerHoldOperationKind) -> StoredAttempt {
        StoredAttempt::new(
            Path::new(ROOT),
            A,
            attempt(ids, kind, PlayerHoldSettlementTrigger::Reopened),
        )
    }

    fn settle_with(
        ids: &Ids,
        kind: PlayerHoldOperationKind,
        trigger: PlayerHoldSettlementTrigger,
        status: Option<&RuntimeControlPlaneStatus>,
    ) -> HoldSettled {
        settled_of(&settle_player_hold(Some(&attempt(ids, kind, trigger)), A, status))
    }

    fn failure() -> ClientFailure {
        ClientFailure {
            code: "runtime_request_rejected",
            operation: "acquire_player_hold",
            runtime_code: None,
            host: None,
        }
    }

    use PlayerHoldOperationKind::{Acquire, Release};
    use PlayerHoldOperationState::{Completed, Failed, Running};
    use PlayerHoldSettlementTrigger::{Dropped, Reopened, TimedOut};

    // ---- hold_fact ----

    #[test]
    fn hold_fact_is_none_without_a_hold_or_a_record() {
        let ids = ids();
        let status = status(&ids, ids.e0, None, None);
        assert_eq!(hold_fact(&status, A), None);
    }

    #[test]
    fn hold_fact_reads_a_held_hold() {
        let ids = ids();
        let mut hold = held(ids.h);
        hold.revision = 4;
        hold.source = source(RuntimeChannel::Mcp);
        let status = status(&ids, ids.e0, Some(hold), None);
        let now = fact(&status).hold.expect("a hold");
        assert_eq!(now.phase, HoldPhase::Held);
        assert_eq!(now.since_ms, 1_000);
        assert_eq!(now.reached_held_ms, Some(2_000));
        assert_eq!(now.source, HoldSource { channel: HoldChannel::Mcp, via: None });
        assert_eq!(now.target.0.revision, 4);
        assert_eq!(now.target.0.owner_epoch, ids.e0);
        assert!(now.target.is_of(&HoldRequestId(ids.h)));
        assert!(!now.target.is_of(&HoldRequestId(ids.r)));
    }

    #[test]
    fn hold_fact_keeps_both_deadlines_while_draining() {
        let ids = ids();
        let hold = acquiring(ids.r, PlayerHoldPhase::Draining);
        let running = record(Acquire, Running, ids.r, ids.r);
        let status = status(&ids, ids.e0, Some(hold), Some(running));
        let now = fact(&status).hold.expect("a hold");
        assert_eq!(now.phase, HoldPhase::Draining);
        assert_eq!(now.drain_deadline_ms, Some(61_000));
        assert_eq!(now.grace_deadline_ms, Some(91_000));
        assert_eq!(now.reached_held_ms, None);
    }

    #[test]
    fn hold_fact_reads_when_the_close_began_while_handing_back() {
        let ids = ids();
        let mut hold = acquiring(ids.r, PlayerHoldPhase::HandingBack);
        hold.close_started_unix_ms = Some(70_000);
        let status = status(&ids, ids.e0, Some(hold), None);
        let now = fact(&status).hold.expect("a hold");
        assert_eq!(now.phase, HoldPhase::HandingBack);
        assert_eq!(now.close_started_ms, Some(70_000));
    }

    #[test]
    fn hold_fact_reads_a_hold_restored_from_its_acquire() {
        let ids = ids();
        let hold = PlayerHoldState {
            reached_held_unix_ms: None,
            restored_from_owner_epoch: Some(ids.e0),
            restored_from_phase: Some(PlayerHoldRestoredFrom::Acquiring),
            ..held(ids.r)
        };
        let status = status(&ids, ids.e1, Some(hold), None);
        let now = fact(&status).hold.expect("a hold");
        assert_eq!(now.restored, Some(Restored::Acquiring));
        assert_eq!(now.reached_held_ms, None);
    }

    #[test]
    fn hold_fact_reads_a_running_release_whose_hold_is_lifted() {
        let ids = ids();
        let status = status(&ids, ids.e0, None, Some(record(Release, Running, ids.r, ids.h)));
        let fact = fact(&status);
        assert_eq!(fact.hold, None);
        let record = fact.record.expect("a record");
        assert_eq!(record.kind, HoldKind::Release);
        assert_eq!(record.state, RecordState::Running);
        assert_eq!(record.phase, Some(HoldPhase::Releasing));
    }

    #[test]
    fn hold_fact_reads_an_acquire_entered_before_its_hold() {
        let ids = ids();
        let mut requesting = record(Acquire, Running, ids.r, ids.r);
        requesting.phase = Some(PlayerHoldPhase::Requesting);
        let status = status(&ids, ids.e0, None, Some(requesting));
        let fact = fact(&status);
        assert_eq!(fact.hold, None);
        let record = fact.record.expect("a record");
        assert_eq!(record.state, RecordState::Running);
        assert_eq!(record.phase, Some(HoldPhase::Requesting));
    }

    #[test]
    fn hold_fact_ended_with_pause_holds_while_the_pause_is_the_same() {
        let ids = ids();
        let mut ended = record(Release, Completed, ids.r, ids.h);
        ended.result = Some(release_result(PlayerHoldNext::InstancePaused));
        let same = status_paused(&ids, ids.e0, None, Some(ended.clone()), Some(3));
        assert!(fact(&same).ended_with_pause);
        assert_eq!(fact(&same).record.and_then(|record| record.next), Some(HoldNext::InstancePaused));
        let another = status_paused(&ids, ids.e0, None, Some(ended), Some(5));
        assert!(!fact(&another).ended_with_pause);
    }

    #[test]
    fn hold_fact_reads_a_failed_acquire() {
        let ids = ids();
        let status = status(&ids, ids.e0, None, Some(record(Acquire, Failed, ids.r, ids.r)));
        let record = fact(&status).record.expect("a record");
        assert_eq!(record.state, RecordState::Failed);
        assert_eq!(record.code.as_deref(), Some(PLAYER_HOLD_DRAIN_INCOMPLETE));
        assert_eq!(record.failed_phase, Some(HoldPhase::Draining));
        assert_eq!(record.ended_ms, Some(3_000));
        assert!(!record.of_hold);
    }

    #[test]
    fn hold_fact_reads_the_runs_its_own_acquire_stopped() {
        let ids = ids();
        let status =
            status(&ids, ids.e0, Some(held(ids.r)), Some(record(Acquire, Completed, ids.r, ids.r)));
        let record = fact(&status).record.expect("a record");
        assert_eq!(record.cancelled, Some(1));
        assert!(record.of_hold);
        assert_eq!(record.request, HoldRequestId(ids.r));
    }

    #[test]
    fn hold_fact_reads_a_reconnect_that_failed() {
        let ids = ids();
        let mut released = record(Release, Completed, ids.r, ids.h);
        let mut result = release_result(PlayerHoldNext::Reconnected);
        result.selfcheck = Some(selfcheck(Some("capture_backend_open_failed")));
        released.result = Some(result);
        let status = status(&ids, ids.e0, None, Some(released));
        let record = fact(&status).record.expect("a record");
        assert_eq!(record.next, Some(HoldNext::Reconnected));
        assert_eq!(record.selfcheck_failure.as_deref(), Some("capture_backend_open_failed"));
        assert_eq!(record.startup, Some(StartupDone::Nothing));
    }

    #[test]
    fn hold_fact_reads_a_failed_release_of_the_hold_now() {
        let ids = ids();
        let status =
            status(&ids, ids.e0, Some(held(ids.h)), Some(record(Release, Failed, ids.r, ids.h)));
        let fact = fact(&status);
        assert_eq!(fact.hold.map(|hold| hold.phase), Some(HoldPhase::Held));
        let record = fact.record.expect("a record");
        assert_eq!(record.state, RecordState::Failed);
        assert!(record.of_hold);
    }

    // ---- gate ----

    #[test]
    fn gate_without_a_fact_follows_the_console_s_own_ask() {
        assert_eq!(gate(None, false), HoldGate::Open);
        assert_eq!(gate(None, true), HoldGate::Freeze);
    }

    #[test]
    fn gate_withdraws_a_held_hold_whatever_is_asked() {
        let ids = ids();
        let status = status(&ids, ids.e0, Some(held(ids.h)), None);
        let fact = fact(&status);
        assert_eq!(gate(Some(&fact), false), HoldGate::Withdrawn);
        assert_eq!(gate(Some(&fact), true), HoldGate::Withdrawn);
    }

    #[test]
    fn gate_freezes_a_hold_draining_or_releasing() {
        let ids = ids();
        let draining = status(&ids, ids.e0, Some(acquiring(ids.r, PlayerHoldPhase::Draining)), None);
        assert_eq!(gate(Some(&fact(&draining)), false), HoldGate::Freeze);
        let releasing_hold = PlayerHoldState { phase: PlayerHoldPhase::Releasing, ..held(ids.h) };
        let releasing = status(&ids, ids.e0, Some(releasing_hold), None);
        assert_eq!(gate(Some(&fact(&releasing)), false), HoldGate::Freeze);
    }

    #[test]
    fn gate_freezes_a_running_record_without_a_hold() {
        let ids = ids();
        let release = status(&ids, ids.e0, None, Some(record(Release, Running, ids.r, ids.h)));
        assert_eq!(gate(Some(&fact(&release)), false), HoldGate::Freeze);
        let acquire = status(&ids, ids.e0, None, Some(record(Acquire, Running, ids.r, ids.r)));
        assert_eq!(gate(Some(&fact(&acquire)), false), HoldGate::Freeze);
    }

    #[test]
    fn gate_opens_after_a_completed_acquire_whose_hold_is_gone() {
        let ids = ids();
        let status = status(&ids, ids.e0, None, Some(record(Acquire, Completed, ids.r, ids.r)));
        assert_eq!(gate(Some(&fact(&status)), false), HoldGate::Open);
    }

    // ---- settle_stored ----

    #[test]
    fn settle_stored_decides_each_row_and_whether_the_attempt_is_kept() {
        let ids = ids();
        let only_b = |epoch| {
            RuntimeControlPlaneStatus::new(epoch, vec![instance(&ids, B, None, None, None)])
                .expect("status")
        };
        let rows: Vec<(&str, PlayerHoldOperationKind, RuntimeControlPlaneStatus, bool)> = vec![
            // The shape f6e2cd26 leaves: nothing recorded.
            ("A9", Acquire, status(&ids, ids.e0, None, None), true),
            ("A13", Acquire, status(&ids, ids.e1, None, None), false),
            ("A11", Acquire, status(&ids, ids.e1, Some(held(ids.r)), None), false),
            (
                "A1",
                Acquire,
                status(
                    &ids,
                    ids.e0,
                    Some(acquiring(ids.r, PlayerHoldPhase::Draining)),
                    Some(record(Acquire, Running, ids.r, ids.r)),
                ),
                true,
            ),
            (
                "A2",
                Acquire,
                status(&ids, ids.e0, Some(held(ids.r)), Some(record(Acquire, Completed, ids.r, ids.r))),
                false,
            ),
            (
                "R7",
                Release,
                status(&ids, ids.e0, None, Some(record(Release, Completed, ids.other, ids.h))),
                false,
            ),
            ("R10", Release, status(&ids, ids.e1, Some(held(ids.h)), None), false),
            ("P2", Acquire, only_b(ids.e0), true),
            ("P2", Acquire, only_b(ids.e1), false),
        ];
        for (rule, kind, status, keep) in rows {
            let settled = settle_stored(&stored(&ids, kind), &StatusCopy::new(status));
            assert_eq!(settled.rule, rule, "{rule}");
            assert_eq!(settled.keep, keep, "{rule}");
            match rule {
                "A9" | "A13" => assert_eq!(settled.said, HoldSettled::Unconfirmed, "{rule}"),
                "A11" => assert!(
                    matches!(settled.said, HoldSettled::Done { restored: true, .. }),
                    "{rule}: {:?}",
                    settled.said
                ),
                "A1" => assert_eq!(settled.said, HoldSettled::Checking, "{rule}"),
                "A2" => assert!(
                    matches!(
                        &settled.said,
                        HoldSettled::Done { kind: HoldKind::Acquire, record: Some(record), .. }
                            if record.cancelled == Some(1)
                    ),
                    "{rule}: {:?}",
                    settled.said
                ),
                "R7" => assert!(
                    matches!(settled.said, HoldSettled::EndedByOther { .. }),
                    "{rule}: {:?}",
                    settled.said
                ),
                "R10" => assert_eq!(
                    settled.said,
                    HoldSettled::NotPerformed { kind: HoldKind::Release },
                    "{rule}"
                ),
                _ => {}
            }
        }
    }

    // ---- settled_of ----

    #[test]
    fn settled_of_says_a_failure_with_its_own_record_s_code() {
        let ids = ids();
        let a3 = status(&ids, ids.e0, None, Some(record(Acquire, Failed, ids.r, ids.r)));
        assert_eq!(
            settle_with(&ids, Acquire, Reopened, Some(&a3)),
            HoldSettled::Failed {
                kind: HoldKind::Acquire,
                code: Some(PLAYER_HOLD_DRAIN_INCOMPLETE.to_owned())
            }
        );
        let mut failed = record(Release, Failed, ids.r, ids.h);
        failed.code = Some(code_str(LEASE_QUEUE_EXPIRED));
        let r3 = status(&ids, ids.e0, Some(held(ids.h)), Some(failed));
        assert_eq!(
            settle_with(&ids, Release, TimedOut, Some(&r3)),
            HoldSettled::Failed {
                kind: HoldKind::Release,
                code: Some(LEASE_QUEUE_EXPIRED.to_owned())
            }
        );
    }

    #[test]
    fn settled_of_says_a_hold_that_ended() {
        let ids = ids();
        let r9 = status(&ids, ids.e0, None, None);
        assert_eq!(settle_with(&ids, Release, TimedOut, Some(&r9)), HoldSettled::Ended);
        let r11 = status(&ids, ids.e1, None, None);
        assert_eq!(settle_with(&ids, Release, Reopened, Some(&r11)), HoldSettled::Ended);
    }

    #[test]
    fn settled_of_says_who_ended_a_hold_it_set() {
        let ids = ids();
        let mut ended = record(Release, Completed, ids.other, ids.r);
        ended.source = source(RuntimeChannel::Lab);
        let lab = HoldSource { channel: HoldChannel::Lab, via: None };
        let a6 = status(&ids, ids.e0, None, Some(ended.clone()));
        assert_eq!(
            settle_with(&ids, Acquire, TimedOut, Some(&a6)),
            HoldSettled::Done {
                kind: HoldKind::Acquire,
                restored: false,
                ended_by: Some(lab),
                record: None
            }
        );
        let a12 = status(&ids, ids.e1, None, Some(ended));
        assert_eq!(
            settle_with(&ids, Acquire, Reopened, Some(&a12)),
            HoldSettled::Done {
                kind: HoldKind::Acquire,
                restored: true,
                ended_by: Some(lab),
                record: None
            }
        );
    }

    #[test]
    fn settled_of_says_what_its_own_release_did() {
        let ids = ids();
        let mut released = record(Release, Completed, ids.r, ids.h);
        let mut result = release_result(PlayerHoldNext::Reconnected);
        result.selfcheck = Some(selfcheck(Some("capture_backend_open_failed")));
        result.startup_package = Some(PlayerHoldStartupPackage::Scheduled);
        released.result = Some(result);
        let r2 = status(&ids, ids.e0, None, Some(released));
        let HoldSettled::Done { kind, record: Some(record), .. } =
            settle_with(&ids, Release, Dropped, Some(&r2))
        else {
            panic!("done with its own record");
        };
        assert_eq!(kind, HoldKind::Release);
        assert_eq!(record.next, Some(HoldNext::Reconnected));
        assert_eq!(record.selfcheck_failure.as_deref(), Some("capture_backend_open_failed"));
        assert_eq!(record.startup, Some(StartupDone::Scheduled { withheld: false }));
    }

    #[test]
    fn settled_of_says_checking_while_a_later_read_may_settle() {
        let ids = ids();
        let r1 = status(&ids, ids.e0, Some(held(ids.h)), Some(record(Release, Running, ids.r, ids.h)));
        assert_eq!(settle_with(&ids, Release, TimedOut, Some(&r1)), HoldSettled::Checking);
        let releasing = PlayerHoldState { phase: PlayerHoldPhase::Releasing, ..held(ids.h) };
        let r4 = status(
            &ids,
            ids.e0,
            Some(releasing),
            Some(record(Release, Running, ids.other, ids.h)),
        );
        assert_eq!(settle_with(&ids, Release, TimedOut, Some(&r4)), HoldSettled::Checking);
        // P1: no status read.
        assert_eq!(settle_with(&ids, Acquire, TimedOut, None), HoldSettled::Checking);
    }

    #[test]
    fn settled_of_says_unsupported_of_a_dropped_request_without_the_hold() {
        let ids = ids();
        let bare = status(&ids, ids.e0, None, None);
        assert_eq!(settle_with(&ids, Acquire, Dropped, Some(&bare)), HoldSettled::Unsupported);
        assert_eq!(settle_with(&ids, Release, Dropped, Some(&bare)), HoldSettled::Unsupported);
    }

    // ---- classify ----

    #[test]
    fn classify_takes_a_settlement_first() {
        let facts =
            FailureFacts { settled: true, code: Some(PLAYER_HOLD_INTERRUPTED), answered: true };
        assert_eq!(classify(&facts), FailureClass::Settled);
    }

    #[test]
    fn classify_tells_an_interrupted_acquire() {
        let facts =
            FailureFacts { settled: false, code: Some(PLAYER_HOLD_INTERRUPTED), answered: true };
        assert_eq!(classify(&facts), FailureClass::Interrupted);
    }

    #[test]
    fn classify_tells_a_runtime_without_the_hold() {
        let facts = FailureFacts { settled: false, code: Some(NOT_IMPLEMENTED), answered: true };
        assert_eq!(classify(&facts), FailureClass::NotImplemented);
    }

    #[test]
    fn classify_tells_any_other_answered_refusal() {
        let facts =
            FailureFacts { settled: false, code: Some(PLAYER_HOLD_ALREADY_HELD), answered: true };
        assert_eq!(classify(&facts), FailureClass::Refused);
    }

    #[test]
    fn classify_leaves_anything_unanswered_unconfirmed() {
        let none = FailureFacts { settled: false, code: None, answered: false };
        assert_eq!(classify(&none), FailureClass::Unconfirmed);
        let unanswered = FailureFacts { settled: false, code: Some(NOT_IMPLEMENTED), answered: false };
        assert_eq!(classify(&unanswered), FailureClass::Unconfirmed);
    }

    // ---- refusal_of ----

    #[test]
    fn refusal_of_names_each_code() {
        assert_eq!(refusal_of(PLAYER_HOLD_ALREADY_HELD), HoldRefusal::AlreadyBusy);
        assert_eq!(refusal_of(PLAYER_HOLD_ACQUIRING), HoldRefusal::AlreadyBusy);
        assert_eq!(refusal_of(PLAYER_HOLD_RELEASING), HoldRefusal::AlreadyBusy);
        assert_eq!(refusal_of(PLAYER_HOLD_NOT_HELD), HoldRefusal::NotHeld);
        assert_eq!(refusal_of(PLAYER_HOLD_EXPECTATION_MISMATCH), HoldRefusal::Moved);
        assert_eq!(refusal_of("lease_queue_expired"), HoldRefusal::DeviceBusy);
        assert_eq!(refusal_of(PLAYER_HOLD_DRAIN_INCOMPLETE), HoldRefusal::Other);
        assert_eq!(refusal_of("a_code_this_console_does_not_know"), HoldRefusal::Other);
    }

    // ---- keeps_attempt ----

    #[test]
    fn keeps_attempt_only_while_a_later_status_may_still_tell() {
        let refused = |denied| HoldOutcome::Refused {
            why: HoldRefusal::Other,
            code: PLAYER_HOLD_DRAIN_INCOMPLETE.to_owned(),
            denied,
            failure: failure(),
            chain: String::new(),
        };
        let done = HoldSettled::Done {
            kind: HoldKind::Acquire,
            restored: false,
            ended_by: None,
            record: None,
        };
        let settled = |said: HoldSettled, rule: &'static str| HoldOutcome::Settled {
            said,
            rule,
            chain: String::new(),
        };
        for outcome in [
            HoldOutcome::Acquired { cancelled: 0 },
            HoldOutcome::NotImplemented { failure: failure(), chain: String::new() },
            refused(true),
            refused(false),
            settled(done, "A2"),
            settled(HoldSettled::Unsupported, "A7"),
        ] {
            assert!(!keeps_attempt(&outcome), "{outcome:?}");
        }
        for outcome in [
            HoldOutcome::Interrupted { failure: failure(), chain: String::new() },
            HoldOutcome::Unconfirmed { failure: failure(), chain: String::new() },
            settled(HoldSettled::Checking, "A1"),
            settled(HoldSettled::Unconfirmed, "A9"),
        ] {
            assert!(keeps_attempt(&outcome), "{outcome:?}");
        }
    }

    // ---- HoldFile ----

    fn attempt_json(ids: &Ids) -> Value {
        serde_json::to_value(attempt(ids, Acquire, Reopened)).expect("attempt")
    }

    #[test]
    fn hold_file_reads_an_empty_list() {
        let file = HoldFile::parse(r#"{"version":1,"attempts":[]}"#).expect("an empty list");
        assert_eq!(file, HoldFile::empty());
        assert_eq!(HoldFile::empty().render(), r#"{"version":1,"attempts":[]}"#);
    }

    #[test]
    fn hold_file_reads_back_what_it_wrote_in_its_order() {
        let ids = ids();
        let mut file = HoldFile::empty();
        let first = stored(&ids, Acquire);
        let second = StoredAttempt::new(
            Path::new(r"E:\other\state"),
            B,
            PlayerHoldAttempt { request_id: ids.other, ..attempt(&ids, Release, Reopened) },
        );
        file.put(first.clone());
        file.put(second.clone());
        let back = HoldFile::parse(&file.render()).expect("read back");
        assert_eq!(back, file);
        assert_eq!(back.attempts, vec![first, second]);
    }

    #[test]
    fn hold_file_refuses_what_it_does_not_know_and_what_it_lacks() {
        let ids = ids();
        let item = json!({"state_root": ROOT, "alias": A, "attempt": attempt_json(&ids)});
        let good = json!({"version": 1, "attempts": [item.clone()]});
        HoldFile::parse(&good.to_string()).expect("the good one reads");
        let mut top_extra = good.clone();
        top_extra["more"] = json!(1);
        let mut item_extra = item.clone();
        item_extra["more"] = json!(1);
        let mut attempt_extra = item.clone();
        attempt_extra["attempt"]["more"] = json!(1);
        let mut no_alias = item.clone();
        no_alias.as_object_mut().expect("an item").remove("alias");
        for bad in [
            top_extra,
            json!({"version": 1, "attempts": [item_extra]}),
            json!({"version": 1, "attempts": [attempt_extra]}),
            json!({"version": 2, "attempts": [item]}),
            json!({"version": 1, "attempts": [no_alias]}),
        ] {
            assert!(HoldFile::parse(&bad.to_string()).is_err(), "{bad}");
        }
    }

    #[test]
    fn hold_file_keeps_one_request_once() {
        let ids = ids();
        let mut file = HoldFile::empty();
        file.put(stored(&ids, Acquire));
        file.put(stored(&ids, Acquire));
        assert_eq!(file.attempts.len(), 1);
    }

    #[test]
    fn hold_file_takes_by_root_alias_and_request() {
        let ids = ids();
        let here = stored(&ids, Acquire);
        let elsewhere =
            StoredAttempt::new(Path::new(r"E:\other\state"), A, attempt(&ids, Acquire, Reopened));
        let other_alias = StoredAttempt::new(
            Path::new(ROOT),
            B,
            PlayerHoldAttempt { request_id: ids.h, ..attempt(&ids, Acquire, Reopened) },
        );
        let mut file = HoldFile::empty();
        file.put(here.clone());
        file.put(other_alias.clone());
        // Another root's attempt of the same request is not this one.
        assert!(!file.take(&elsewhere));
        assert!(!file.take(&StoredAttempt { alias: B.to_owned(), ..here.clone() }));
        let spelt_otherwise = StoredAttempt { root: "d:/ac/STATE/".to_owned(), ..here.clone() };
        assert!(file.take(&spelt_otherwise));
        assert_eq!(file.attempts, vec![other_alias]);
    }

    #[test]
    fn hold_file_lists_only_its_root_s_attempts() {
        let ids = ids();
        let here = stored(&ids, Acquire);
        let elsewhere = StoredAttempt::new(
            Path::new(r"E:\other\state"),
            A,
            PlayerHoldAttempt { request_id: ids.h, ..attempt(&ids, Acquire, Reopened) },
        );
        let mut file = HoldFile::empty();
        file.put(elsewhere);
        file.put(here.clone());
        let root = PathBuf::from(ROOT);
        let listed: Vec<&StoredAttempt> = file.of_root(&root).collect();
        assert_eq!(listed, vec![&here]);
    }

    // ---- same_root ----

    #[test]
    fn same_root_takes_another_spelling_of_the_same_text() {
        assert!(same_root(r"D:\AC\state", Path::new("D:/AC/state")));
        assert!(same_root(r"D:\AC\state", Path::new(r"d:\ac\STATE")));
        assert!(same_root(r"D:\AC\state\", Path::new(r"D:\AC\state")));
        assert!(same_root(r"D:\AC\state", Path::new(r"D:\AC\state\")));
    }

    #[test]
    fn same_root_tells_another_folder_apart() {
        assert!(!same_root(r"D:\AC\state", Path::new(r"D:\AC\state2")));
        assert!(!same_root(r"D:\AC\state", Path::new(r"E:\AC\state")));
    }
}
