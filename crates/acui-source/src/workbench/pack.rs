// SPDX-License-Identifier: GPL-3.0-only
//! An account's task switches (UI-3f2; Workflow #429 R-8, Runtime 0.12.55): what the Runtime says
//! of each task package of one instance, read with `PackScheduleView` ([`super::ReadLane::read_packs`],
//! [`PacksRead`]); what a press of one package's switch writes ([`switch`]); and that write, one
//! `EditPackSchedule` of the instance's own scope ([`edit`]). And the rescue bar's "close this task"
//! of a run the scheduler dispatched (UI-3e2b), which writes the switch's off of the run's package
//! and then stops the run ([`close_then_stop`]). Nothing out of here names a wire type.
//!
//! The view lists, once each and by package id, every package a task of the instance's active
//! catalog runs; of each, whether it is schedulable now, which layer decided (the instance's own
//! rows, its server's, its game's, the pack's default, or none: no row covers now, so it is), the
//! row that decided, when that next changes, and what the layer below would decide (`below`). It
//! is read from memory and records nothing, but the Runtime brings its fact store up to the ledger
//! under the fact write gate first, a moment's hold of a lock every fact write takes: the caller
//! reads it seldom. A Runtime before 0.12.55 refuses it `not_implemented`, and one before 0.12.30
//! does not know it and drops the connection: either is an answer here, that this Runtime has no
//! switches ([`PacksAnswer::Unsupported`]), and no failure.
//!
//! The console writes only the instance's own scope, never its server's or its game's. Off is one
//! open row that the package is not schedulable, whatever decided before: a person's plain order.
//! On goes back to normal: the instance's own rows withdrawn when the layer below schedules it,
//! else one open row that it is ([`switch`]). Either replaces every row of that package in that
//! scope, rows that only begin later included, and no other package's. The write names the view's
//! version of the scope, so it is refused when anything else wrote that scope since
//! ([`PackOutcome::Conflict`]); the caller reads the view again and the person presses again. One
//! refusal alone is sent again here, once, at once, on the same connection, with the same version
//! and the time taken anew: `fact_observation_not_newer`, which the Runtime answers when another
//! list of that scope landed in the same millisecond, and whose contract says to send again.
//!
//! "Close this task" goes on one fresh person's connection too, in this order: the instance's view,
//! read to see whether the run's package is one the instance's active catalog runs; when it is, the
//! same open row that it is not schedulable, from now on, naming no version, since what it writes
//! depends on nothing read, so that no other package's change since meets it as a conflict, with
//! `fact_observation_not_newer` sent again once, as a switch's is; and once that took, and only
//! then, the run's stop ([`super::stop::stop_run`]'s, on the same connection, so that it cannot
//! fail to connect). A package the view does not list gets no row: the account's task list could
//! neither show it nor withdraw it, and the Runtime would name it every round. Its run is only
//! stopped. Nothing of it is sent again here.
//!
//! The connection is that of a person's change of settings ([`super::configure`]): a fresh
//! person's one, asking for the full receipt, let go after the write. Its answer is waited for on
//! the caller's thread, so that is never the event loop's.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use actingcommand_contract::codes::SharedCode;
use actingcommand_contract::{
    FactScope, PackScheduleEditRow, PackSchedulePackage, PackScheduleSource, PackScheduleView,
};
use actingcommand_runtime_client::{RuntimeClient, RuntimeClientError, RuntimeClientErrorClass};

use super::stop::{stop_on, StopOutcome};
use super::{configure_config, RunKey};
use crate::{ClientFailure, Spelling};

/// The codes a refusal of the view or of a write is told apart by: a Runtime without them; another
/// writer's change of the scope since its version was read; another list of the scope landed in
/// the same millisecond; rows the Runtime does not take. The host's own spellings, of a crate this
/// one does not use, but `not_implemented`, which is shared, and the client's own code of a request
/// the Runtime does not know.
const NOT_IMPLEMENTED: &str = SharedCode::NotImplemented.as_str();
const OPERATION_UNSUPPORTED: &str = "runtime_operation_unsupported";
const VERSION_CONFLICT: &str = "fact_observation_version_conflict";
const NOT_NEWER: &str = "fact_observation_not_newer";
const SCHEDULE_INVALID: &str = "session_pack_schedule_invalid";
/// What those refusals say beyond their code: the scope's version now, and where a list broke a
/// rule, a JSON pointer: into the request, `/package_id` or `/rows…`, when its rows are what the
/// Runtime does not take; into the scope's list stored, the empty one or a row's `/<n>…`, when that
/// is what does not read ([`stored_pointer`]).
const CURRENT_VERSION: &str = "current_version";
const FIELD: &str = "field";
/// The pointer of an edit's rows: more rows than one scope holds.
const ROWS_POINTER: &str = "/rows";

/// One answered `PackScheduleView` of one instance, as far as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacksRead {
    pub epoch: String,
    /// The alias asked for: the client refuses an answer that names another.
    pub alias: String,
    pub answer: PacksAnswer,
}

/// What the view said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacksAnswer {
    /// This Runtime has no task switches: it refused the view `not_implemented` (before 0.12.55),
    /// or does not know the request (before 0.12.30, the client's `runtime_operation_unsupported`).
    /// Nothing was recorded. `code` is which of the two, for the log.
    Unsupported { code: &'static str },
    Read(PackStates),
}

/// Each package's state, and the version a write of the instance's own scope names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackStates {
    /// The version of the instance's own scope's list (`0` while it has none), an opaque token
    /// compared only for equality: the next write's `expected_version`. `None` when the answer
    /// names no such scope, which the contract never leaves out: then nothing is written.
    pub version: Option<u64>,
    /// In the Runtime's order, by package id.
    pub packages: Vec<PackState>,
}

/// One package as the view said it. The version of the instance's own scope is kept beside the
/// packages ([`PackStates::version`]); the view's server's and game's scopes and their versions are
/// not kept at all, the console writing neither. Nothing of this is shown but `on`, `source` and
/// the times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackState {
    pub package_id: String,
    /// Schedulable now.
    pub on: bool,
    pub source: PackSource,
    /// The row that decided: none when `source` is `None`.
    pub row: Option<Span>,
    /// When the decision next changes, in unix ms: the deciding row's end, or the next row of its
    /// layer that begins; none when nothing bounds it.
    pub next_change_ms: Option<u64>,
    /// What the layer below would decide were the deciding layer's rows of it gone: none when
    /// `source` is `None` or `PackDefault`, never `Instance`.
    pub below: Option<Below>,
}

/// A row's period, in unix ms: from its start, until its end when it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub from_ms: u64,
    pub until_ms: Option<u64>,
}

/// The layer below's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Below {
    pub on: bool,
    pub source: PackSource,
}

/// Which layer decided a package (`PackScheduleSource`, a closed vocabulary). `PackDefault` comes
/// with R-9's pack defaults: no Runtime gives it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackSource {
    Instance,
    Server,
    Game,
    PackDefault,
    None,
}

/// What a press writes in the instance's own scope for one package: its rows withdrawn, or one
/// open row from the moment it is sent, on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackChange {
    Withdraw,
    Set(bool),
}

impl PackChange {
    /// For the log only.
    pub fn line(self) -> &'static str {
        match self {
            Self::Withdraw => "withdraw",
            Self::Set(true) => "on",
            Self::Set(false) => "off",
        }
    }

    /// The rows it writes, an open one from `now_ms`.
    fn rows(self, now_ms: u64) -> Vec<PackScheduleEditRow> {
        match self {
            Self::Withdraw => Vec::new(),
            Self::Set(schedulable) => {
                vec![PackScheduleEditRow { from_unix_ms: now_ms, until_unix_ms: None, schedulable }]
            }
        }
    }
}

/// What a press of `state`'s switch to `on` writes, the one place that decides it; none when it
/// writes nothing. Off is an open row that it is not schedulable, whatever decided before. On, of
/// a package the instance's own rows decide, withdraws them when the layer below schedules it, and
/// else writes an open row that it is; of one a lower layer decides, it writes that open row. A
/// package already as pressed writes nothing; nor does one the instance's own rows decide without
/// the view naming what is below them, which the contract never leaves out: neither way is known
/// to be right, so it has no switch at all ([`offered`]).
pub fn switch(state: &PackState, on: bool) -> Option<PackChange> {
    if state.source == PackSource::Instance && state.below.is_none() {
        return None;
    }
    if state.on == on {
        return None;
    }
    if !on {
        return Some(PackChange::Set(false));
    }
    let change = match state.source {
        PackSource::Instance if state.below.is_some_and(|below| below.on) => PackChange::Withdraw,
        PackSource::Instance
        | PackSource::Server
        | PackSource::Game
        | PackSource::PackDefault
        | PackSource::None => PackChange::Set(true),
    };
    Some(change)
}

/// Whether `state` has a switch: whether a press of it would write something ([`switch`]).
pub fn offered(state: &PackState) -> bool {
    switch(state, !state.on).is_some()
}

/// What became of one [`edit`], and whether it was sent a second time after
/// `fact_observation_not_newer`, which only the log says.
#[derive(Debug, Clone)]
pub struct EditAnswer {
    pub outcome: PackOutcome,
    pub resent: bool,
}

/// What became of one write of a package's rows.
#[derive(Debug, Clone)]
pub enum PackOutcome {
    /// The Runtime took it: the scope's version after it, for the log (the next write names the
    /// version of a view read after this one).
    Edited { version: u64 },
    /// The scope's list is at another version than the one named: another writer changed it since
    /// the view was read, and nothing changed. `current` is only for the log.
    Conflict { current: Option<u64> },
    /// The Runtime refused it and changed nothing: why, the Runtime's codes, and the pointer it
    /// named, for the log.
    Refused { why: PackRefusal, failure: ClientFailure, field: Option<String> },
    /// This Runtime has no such write (`not_implemented`, before 0.12.55), and recorded nothing.
    NotImplemented(ClientFailure),
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it: a receipt that did not come in
    /// time, a failed write, a refusal that halted the host. It is not sent again.
    Unconfirmed(ClientFailure),
}

/// Why the Runtime refused a write of a package's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackRefusal {
    /// Another list of the scope landed in the same millisecond, the second time too.
    NotNewer,
    /// The scope would hold more rows than it may, of every package together (`/rows`).
    TooMany,
    /// The scope's list stored does not read: a pointer into it, not into the request, the empty
    /// one or a row's `/<n>…` ([`stored_pointer`]).
    StoredUnreadable,
    /// Any other rule the rows broke: this program's fault.
    Invalid,
    /// Any other refusal: its codes are in the failure.
    Other,
}

/// One person's switch of the package `package` of the instance `alias`: a fresh person's
/// connection, one `EditPackSchedule` of the instance's own scope that writes what `change` says,
/// only while the scope's list is at `expected`, the version of the view the press was on, and the
/// connection let go. Sent once, whatever comes back, but for one `fact_observation_not_newer`,
/// sent again at once on the same connection, its row's time taken anew.
pub fn edit(
    root: &Path,
    alias: &str,
    package: &str,
    change: PackChange,
    expected: u64,
) -> EditAnswer {
    let client = match RuntimeClient::connect(configure_config(root)) {
        Ok(client) => client,
        Err(error) => {
            let outcome = PackOutcome::NotSent(ClientFailure::from(error));
            return EditAnswer { outcome, resent: false };
        }
    };
    edit_on(&client, alias, package, change, Some(expected))
}

/// The write of [`edit`] on a connection already made, with a version or without one (which
/// compares nothing): `fact_observation_not_newer` sent once more on it, the time taken anew. The
/// close row of [`close_then_stop`] is this write of an off, without one.
fn edit_on(
    client: &RuntimeClient,
    alias: &str,
    package: &str,
    change: PackChange,
    expected: Option<u64>,
) -> EditAnswer {
    let send = || {
        let scope = FactScope::Instance { instance_id: alias.to_owned() };
        client.edit_pack_schedule(scope, package, change.rows(now_ms()), expected)
    };
    let (answer, resent) = match send() {
        Err(error) if refused_not_newer(&error) => (send(), true),
        answer => (answer, false),
    };
    let outcome = match answer {
        Ok(edited) => PackOutcome::Edited { version: edited.version },
        Err(error) => failed(error),
    };
    EditAnswer { outcome, resent }
}

/// What became of one [`close_then_stop`], step by step as they went out: the run's stop follows
/// only a close row the Runtime took, or a package the view does not list, which gets no row.
#[derive(Debug, Clone)]
pub enum CloseAnswer {
    /// No connection: nothing was sent.
    NotSent(ClientFailure),
    /// This Runtime has no task switches: it answered the view `not_implemented` (before 0.12.55)
    /// or did not know it (before 0.12.30), `code` saying which, for the log. Nothing was written,
    /// and the run was not stopped.
    Unsupported { code: &'static str, failure: ClientFailure },
    /// The view was not read: nothing was written, and the run was not stopped. A read changes
    /// nothing, so whatever failed leaves nothing uncertain.
    ViewFailed(ClientFailure),
    /// The close row was not written, or may have been: what its write came to, which is never
    /// `Edited`. The run was not stopped.
    NotClosed(EditAnswer),
    /// The close row was written: the scope's version after it, and whether it went a second time
    /// after `fact_observation_not_newer`, both for the log. Then the run's stop.
    Closed { version: u64, resent: bool, stop: StopOutcome },
    /// The view lists no such package: no row was written, and the run's stop went alone.
    NotListed(StopOutcome),
}

/// One person's "close this task" of the run `run` of the instance `alias`, which runs the package
/// `package` (UI-3e2b): a fresh person's connection; the instance's `PackScheduleView`; when it
/// lists `package`, one `EditPackSchedule` of the instance's own scope that writes an open row from
/// now on that `package` is not schedulable, naming no version, `fact_observation_not_newer` sent
/// once more at once; once that took, and only then, the run's stop ([`super::stop::stop_run`]'s);
/// when the view does not list `package`, the stop alone; and the connection let go. A view or a
/// close that fails stops nothing, and nothing is sent again.
pub fn close_then_stop(root: &Path, alias: &str, package: &str, run: RunKey) -> CloseAnswer {
    let client = match RuntimeClient::connect(configure_config(root)) {
        Ok(client) => client,
        Err(error) => return CloseAnswer::NotSent(ClientFailure::from(error)),
    };
    let view = match client.pack_schedule_view(alias) {
        Ok(view) => view,
        Err(error) => {
            return match view_unsupported(&error) {
                Some(code) => {
                    CloseAnswer::Unsupported { code, failure: ClientFailure::from(error) }
                }
                None => CloseAnswer::ViewFailed(ClientFailure::from(error)),
            };
        }
    };
    if !view.packages.iter().any(|listed| listed.package_id == package) {
        return CloseAnswer::NotListed(stop_on(&client, alias, run));
    }
    match edit_on(&client, alias, package, PackChange::Set(false), None) {
        EditAnswer { outcome: PackOutcome::Edited { version }, resent } => {
            CloseAnswer::Closed { version, resent, stop: stop_on(&client, alias, run) }
        }
        answer => CloseAnswer::NotClosed(answer),
    }
}

/// Whether the Runtime answered a write `fact_observation_not_newer` and went on: the one refusal
/// sent again.
fn refused_not_newer(error: &RuntimeClientError) -> bool {
    answered(error)
        && error.receipt_outcome().is_some_and(|outcome| outcome.code().as_str() == NOT_NEWER)
}

/// Whether a failed request was answered with a refusal the Runtime went on after, and nothing
/// leaves it uncertain.
fn answered(error: &RuntimeClientError) -> bool {
    error.projection().is_some_and(|refusal| !refusal.fatal)
        && !matches!(error.disposition(), RuntimeClientErrorClass::Uncertain)
}

/// A failed write after the connection, told apart as a person's change of settings is
/// (`configure_failed`), in this order:
///
/// 1. anything but a refusal the Runtime answered and went on after, of a request nothing leaves
///    uncertain, a refusal that halted the host included: unconfirmed, since a failed write may
///    have got the request out;
/// 2. a conflict of versions, with the version now;
/// 3. a Runtime without the write;
/// 4. any other refusal, why by its receipt's code and, of a list it does not take, by its pointer:
///    the request's `/rows` alone is the scope's bound, any other of the request's is this
///    program's fault, and one into the list stored is that list ([`stored_pointer`]).
fn failed(error: RuntimeClientError) -> PackOutcome {
    if !answered(&error) {
        return PackOutcome::Unconfirmed(ClientFailure::from(error));
    }
    let answer = error.receipt_outcome();
    let top = answer.map_or("", |outcome| outcome.code().as_str());
    if top == VERSION_CONFLICT {
        let current = answer
            .and_then(|outcome| outcome.value(CURRENT_VERSION))
            .and_then(|value| value.as_u64());
        return PackOutcome::Conflict { current };
    }
    if top == NOT_IMPLEMENTED {
        return PackOutcome::NotImplemented(ClientFailure::from(error));
    }
    let field = answer
        .and_then(|outcome| outcome.value(FIELD))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let why = match (top, field.as_deref()) {
        (NOT_NEWER, _) => PackRefusal::NotNewer,
        (SCHEDULE_INVALID, Some(ROWS_POINTER)) => PackRefusal::TooMany,
        (SCHEDULE_INVALID, Some(pointer)) if stored_pointer(pointer) => {
            PackRefusal::StoredUnreadable
        }
        (SCHEDULE_INVALID, _) => PackRefusal::Invalid,
        _ => PackRefusal::Other,
    };
    PackOutcome::Refused { why, failure: ClientFailure::from(error), field }
}

/// Whether a refusal's pointer is into the scope's list stored, not into the request: the empty
/// one, the list as a whole (not a list, or more rows than the bound), or one of its rows, `/<n>`
/// or `/<n>/<member>`, as the Runtime reads the list it holds before it replaces the package's rows
/// in it (Runtime 0.12.55, `pack_schedule::read_rows`). The request's own all begin with a name,
/// `/package_id` or `/rows`.
fn stored_pointer(pointer: &str) -> bool {
    let row = |rest: &str| rest.starts_with(|c: char| c.is_ascii_digit());
    pointer.is_empty() || pointer.strip_prefix('/').is_some_and(row)
}

/// Whether a failed view says the Runtime has none, and the code that says so: the client's own
/// word that the Runtime does not know the request, or a refusal `not_implemented` it went on
/// after; none for any other failure.
pub(super) fn view_unsupported(error: &RuntimeClientError) -> Option<&'static str> {
    if error.code().spelling() == OPERATION_UNSUPPORTED {
        return Some(OPERATION_UNSUPPORTED);
    }
    let refused = answered(error)
        && error
            .receipt_outcome()
            .is_some_and(|outcome| outcome.code().as_str() == NOT_IMPLEMENTED);
    refused.then_some(NOT_IMPLEMENTED)
}

/// The view of `alias` as the console reads it: the version of the instance's own scope, and each
/// package's state.
pub(super) fn packs_read(epoch: String, alias: &str, view: &PackScheduleView) -> PacksRead {
    let own = FactScope::Instance { instance_id: alias.to_owned() };
    let version = view.scopes.iter().find(|scope| scope.scope == own).map(|scope| scope.version);
    PacksRead {
        epoch,
        alias: alias.to_owned(),
        answer: PacksAnswer::Read(PackStates {
            version,
            packages: view.packages.iter().map(pack_state).collect(),
        }),
    }
}

/// A Runtime's answer that it has no view, by the code that says so ([`view_unsupported`]).
pub(super) fn packs_unsupported(epoch: String, alias: &str, code: &'static str) -> PacksRead {
    PacksRead { epoch, alias: alias.to_owned(), answer: PacksAnswer::Unsupported { code } }
}

fn pack_state(package: &PackSchedulePackage) -> PackState {
    PackState {
        package_id: package.package_id.clone(),
        on: package.schedulable,
        source: pack_source(package.source),
        row: package
            .row
            .map(|row| Span { from_ms: row.from_unix_ms, until_ms: row.until_unix_ms }),
        next_change_ms: package.next_change_unix_ms,
        below: package
            .below
            .map(|below| Below { on: below.schedulable, source: pack_source(below.source) }),
    }
}

fn pack_source(source: PackScheduleSource) -> PackSource {
    match source {
        PackScheduleSource::Instance => PackSource::Instance,
        PackScheduleSource::Server => PackSource::Server,
        PackScheduleSource::Game => PackSource::Game,
        PackScheduleSource::PackDefault => PackSource::PackDefault,
        PackScheduleSource::None => PackSource::None,
    }
}

/// This computer's clock in unix ms, taken just before a row is sent: the Runtime is on the same
/// one, by loopback.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::{switch, Below, PackChange, PackSource, PackState};

    fn state(on: bool, source: PackSource, below: Option<(bool, PackSource)>) -> PackState {
        PackState {
            package_id: "daily".to_owned(),
            on,
            source,
            row: None,
            next_change_ms: None,
            below: below.map(|(on, source)| Below { on, source }),
        }
    }

    #[test]
    fn off_writes_an_open_off_row_whatever_decided() {
        let decided = [
            state(true, PackSource::None, None),
            state(true, PackSource::Instance, Some((true, PackSource::None))),
            state(true, PackSource::Instance, Some((false, PackSource::Server))),
            state(true, PackSource::Server, Some((false, PackSource::Game))),
            state(true, PackSource::Game, Some((true, PackSource::None))),
            state(true, PackSource::PackDefault, None),
        ];
        for state in &decided {
            assert_eq!(switch(state, false), Some(PackChange::Set(false)), "{state:?}");
        }
    }

    #[test]
    fn on_over_its_own_row_withdraws_it_when_below_schedules_it() {
        let own = state(false, PackSource::Instance, Some((true, PackSource::None)));
        assert_eq!(switch(&own, true), Some(PackChange::Withdraw));
        let own = state(false, PackSource::Instance, Some((true, PackSource::Game)));
        assert_eq!(switch(&own, true), Some(PackChange::Withdraw));
    }

    #[test]
    fn on_over_its_own_row_writes_an_open_on_row_when_below_does_not() {
        let own = state(false, PackSource::Instance, Some((false, PackSource::Server)));
        assert_eq!(switch(&own, true), Some(PackChange::Set(true)));
    }

    #[test]
    fn on_over_a_lower_layer_writes_an_open_on_row() {
        for source in [PackSource::Server, PackSource::Game, PackSource::PackDefault] {
            let lower = state(false, source, Some((true, PackSource::None)));
            assert_eq!(switch(&lower, true), Some(PackChange::Set(true)), "{source:?}");
        }
        let default = state(false, PackSource::PackDefault, None);
        assert_eq!(switch(&default, true), Some(PackChange::Set(true)));
    }

    #[test]
    fn its_own_row_without_below_has_no_switch() {
        for on in [true, false] {
            let own = state(on, PackSource::Instance, None);
            assert_eq!(switch(&own, true), None);
            assert_eq!(switch(&own, false), None);
        }
    }

    #[test]
    fn as_pressed_writes_nothing() {
        assert_eq!(switch(&state(true, PackSource::None, None), true), None);
        let own = state(false, PackSource::Instance, Some((true, PackSource::None)));
        assert_eq!(switch(&own, false), None);
        let lower = state(true, PackSource::Server, Some((false, PackSource::Game)));
        assert_eq!(switch(&lower, true), None);
    }
}
