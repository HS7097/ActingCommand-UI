// SPDX-License-Identifier: GPL-3.0-only
//! What the workbench reads from the running Runtime (UI-3b): a [`ReadLane`] per
//! reading thread, and read results that carry no wire type, so `acui-app` needs
//! no Runtime crate.
//!
//! A lane follows the Runtime across restarts: it connects when it has no
//! connection, lets the connection go after a fatal failure and connects anew on
//! its next read, whatever owner epoch it then reaches. This is why it does not
//! reuse `Rebuilding`, which refuses another epoch since a session's pins were
//! stated by the first one. Every read states its epoch; dropping a read of an
//! epoch that is gone is the caller's.
//!
//! Lanes ask only what is answered from memory or takes no ledger record:
//! `Status`, `ScheduleExplanation` of every instance without its candidate rows and, for an
//! account's task list (UI-3f1), of that one instance with them ([`ReadLane::read_tasks`]),
//! `RuntimeFactSnapshot` and `StartReport`. The picture's lane ([`preview::PreviewLane`],
//! UI-3c) is the one exception: it reads ledger pages, which take no record either but are
//! read from the ledger, and frames' material, whose read takes no record when it succeeds or
//! finds the file gone, and one for any other failure. How often is the caller's, too: a lane keeps
//! no clock, and asks again by itself in one case only, a read on a connection that turns
//! out to have ended (`ReadLane::asked_once_more`; the picture lane's pages likewise).
//!
//! The configuration's key table, the console's tiers of its keys and the performance
//! presets are the Runtime's, read through its crate and only here: which preset is in
//! effect and which settings the console shows are its functions, and what comes out of
//! this file names neither a key nor a wire type, but for the key a refusal named, that
//! of a setting this file has no name for and those a start report names, all for the
//! log.
//!
//! Left out on purpose: `player_hold` and `control` (the hold's read face is
//! UI-3d's); a fence's recheck counts. An activity's `request_id` is kept, as the run's
//! identity that a stop of it names and nothing outside this crate reads
//! ([`ActivityFact::run`], [`stop::stop_run`], UI-3e2).
//! `emulator_state` is a condition of the picture's refresh and of the device's buttons, whose
//! reason says when it was seen, and never shown as what holds now
//! ([`InstanceLive::emulator_off`], [`InstanceLive::emulator_on`]; UI-3c-2, UI-3e1-a). The
//! Runtime reads it at discovery, at an emulator's control and, while its emulator watch is on,
//! at the watch's poll of a discovery-bound instance, a minute apart by default (EMU-W R2), and
//! `off` may be the idle action's doing: a reading may be a minute old, or older. What the
//! device's buttons are drawn for is the status's too: whether an instance drives a device of
//! its own, and which of its emulator's controls its capability profile has available
//! ([`InstanceLive::physical`], [`InstanceLive::emulator_controls`]).
//!
//! The writes here are a person's global pause ([`pause_global`]), a person's pause of one
//! account ([`pause_account`]) and a person's change
//! of settings ([`configure`]), in the picture's file a person's refresh of the picture
//! ([`preview::observe_frame`]), and in the device's a person's operation of a device: its
//! reconnect and self-check, its emulator's control, its game's launch and restart
//! ([`device::operate`], UI-3e1-a), and in the stop's a person's stop of one run of an account
//! ([`stop::stop_run`], UI-3e2). Like the conditional resume beside them (`resume_paused`)
//! each goes out once on a fresh person's connection, and never again by itself, whatever
//! comes back, with one exception: a stop that answered that the run still stops, or whose
//! answer is not known, is asked once more, on another fresh connection, once a `Status` shows
//! the run gone, so that its reset's result is read (the console's `stop`, not this crate,
//! decides when).
//!
//! An instance's touch and capture backends (HS-U, Runtime 0.12.39's #402 HS-2) are settings
//! too, but not rows of the page's tiers: [`backends`] reads them as each device instance's
//! group, out of the same snapshot, and spells the one change of them, which switches that
//! device before the Runtime answers.
//!
//! An account's automatic rules (UI-3e1-b), its quiet times, its idle action and its emulator's
//! start, are rows of the page's tiers that the main page changes: [`rules`] reads them out of the
//! same snapshot, a blackout list's entries through the Runtime's own parser (BX-1, 0.12.39), and
//! spells their changes, a list's in the shape that parser reads.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Spelling;
use acui_rows::{code, EventActor, EventSource};
use actingcommand_contract::outcome::{Detail, Outcome};
use actingcommand_contract::{
    config_parameters_fact_key, is_instance_backend_key, ConfigParameter, ConfigParameterSource,
    ConfigSettingChange, EmulatorCapability, EmulatorRunningState, ExecutionBackendProvenance,
    FactScalar, FactValue, InstanceActivity, InstanceActivityKind, InstanceId, OwnerEpoch,
    RequestId, RuntimeControlPlaneStatus,
    RuntimeErrorCode, RuntimeFactRecord, RuntimeFactScope, RuntimeFactSnapshot,
    RuntimeInstanceStatus, RuntimeResult, RuntimeScheduleExplanation, RuntimeStartReport,
    ScheduleCandidateEligibility, ScheduleDispatchState,
    ScheduleExplanationInstance, SchedulingPauseReason, SchedulingPauseScope,
    CONFIG_PARAMETERS_FACT_PREFIX, MIN_SCHEDULING_PAUSE_DRAIN_TIMEOUT_MS,
};
use actingcommand_runtime_client::{
    RuntimeChannel, RuntimeClient, RuntimeClientConfig, RuntimeClientError,
};
use actingcommand_runtime_config::frame_retention::EVIDENCE_MAX_AGE_DAYS;
use actingcommand_runtime_config::host::{PROCESS_IO_PRIORITY, PROCESS_MEMORY_PRIORITY};
use actingcommand_runtime_config::lab_module::{AUTHORING, DEBUGGING};
use actingcommand_runtime_config::performance::presets::{self, CurrentMember, PresetTable};
use actingcommand_runtime_config::performance::{self, pacing};
use actingcommand_runtime_config::{
    find_row, idle, instance_key, scalar_json, scheduling, ui_tier, AutoGaps, ConfigCode,
    ConfigValueProblem, IdleAction, KeyName, ProcessPriorityLevel, UiTier, ValueType,
    CAPTURE_BACKEND, START_EMULATOR, TOUCH_BACKEND, UI_TIERS,
};

use super::{
    global_pause_fact, instance_pause_fact, ClientFailure, OnlineSource, PauseFact,
    RUNTIME_RESULT_UNEXPECTED,
};

pub mod backends;
pub mod device;
pub mod preview;
pub mod rules;
pub mod stop;

/// The reason this console's "pause all" gives, a free name of the pause, no outcome
/// code. A pause of this reason is one a person pressed here, so lifting it asks for
/// no second press.
pub const CONSOLE_PAUSE_ALL: &str = "console_pause_all";

/// The reason this console's "pause this account" gives, a pause of one instance's scope
/// ([`pause_account`]), likewise a free name and no outcome code, and likewise lifted on the
/// account's card with one press. As a name the Runtime takes it is 21 bytes of `[a-z_]`.
pub const CONSOLE_PAUSE_ACCOUNT: &str = "console_pause_account";

/// Whether `reason` is a name the Runtime takes as a pause's reason, by the contract's own
/// check (`SchedulingPauseReason::new`: 1 to 64 bytes of `[a-z0-9_.-]`). The console's
/// settings name reasons it offers no resume for, and one no pause can carry names none.
pub fn pause_reason_valid(reason: &str) -> bool {
    SchedulingPauseReason::new(reason).is_ok()
}

/// The client's own spelling of the pause's operation
/// (`ContractLocation::PauseScheduling`).
const PAUSE_SCHEDULING: &str = "pause_scheduling";

/// The pacing mode's words, for what they are said in (`SettingValue::Word`).
pub const MODE_OBSERVE: &str = pacing::OBSERVE;
pub const MODE_PACE: &str = pacing::PACE;
pub const MODE_FULL: &str = pacing::FULL;

/// The instance fact of the server the last admitted package declared.
const TASK_SERVER: &str = "task.server";

/// The words of an instance's idle action (`idle.action`), for what they are said in
/// (`ShownValue::Word`).
pub const IDLE_NONE: &str = IdleAction::None.as_str();
pub const IDLE_CLOSE_APP: &str = IdleAction::CloseApp.as_str();
pub const IDLE_STOP_EMULATOR: &str = IdleAction::StopEmulator.as_str();
/// IDLE-2's `auto`: only its word, which the thresholds it is built with here do not change.
pub const IDLE_AUTO: &str =
    IdleAction::Auto(AutoGaps { return_home_min_gap_ms: 0, stop_emulator_min_gap_ms: 0 }).as_str();

/// The words of the Runtime process's two priorities (`host.*`), likewise.
pub const PRIORITY_NORMAL: &str = ProcessPriorityLevel::Normal.token();
pub const PRIORITY_LOW: &str = ProcessPriorityLevel::Low.token();

/// The MuMu root actingd discovers the emulator from: a key of the console's tiers, whose
/// spelling the crate keeps to itself (a key of the start-up assembly).
const MUMU_ROOT: &str = "mumu_root";

/// A row of `config.parameters` names its key here.
const ROW_KEY: &str = "key";

/// The two values of a schedule explanation's reason that choose its words too: why a
/// `benefit_unknown` candidate's benefit is unknown, and why a `pool_line_held` one is held.
const BENEFIT_GAP: &str = "benefit_gap";
const POOL_LINE_HOLD: &str = "pool_line_hold";

/// One reading thread's connection: actor `ui`, source `ui`, channel `ui`, at its
/// own io timeout. The client holds its lock for a whole exchange and latches the
/// connection on a timeout, so a read that must not wait for a slow one needs a
/// lane of its own.
pub struct ReadLane {
    root: PathBuf,
    io: Duration,
    client: Option<RuntimeClient>,
}

impl ReadLane {
    /// No connection yet: the first read makes it.
    pub fn new(root: impl Into<PathBuf>, io: Duration) -> Self {
        Self { root: root.into(), io, client: None }
    }

    /// One `Status`. See [`LiveRead`] for how a failure is told apart.
    pub fn read_live(&mut self) -> LiveRead {
        let client = match self.connection() {
            Ok(client) => client,
            Err(error) => return live_failure(error),
        };
        let info = client.runtime_info();
        let (pid, epoch) = (info.pid(), code(&info.owner_epoch()));
        match client.status() {
            Ok(status) => LiveRead::Read(Box::new(live_snapshot(pid, epoch, &status))),
            Err(error) => live_failure(self.settled(error)),
        }
    }

    /// One `ScheduleExplanation` of every instance, without candidate rows.
    pub fn read_explain(&mut self) -> Result<ExplainRead, ClientFailure> {
        self.asked_once_more(Self::ask_explain)
    }

    /// One `ScheduleExplanation` of the one instance `alias`, with its candidate rows: that
    /// account's task list (UI-3f1). The cards' read stays the one above, of every instance
    /// without them, so neither answer grows with the other, and one too large for a frame costs
    /// only the list.
    pub fn read_tasks(&mut self, alias: &str) -> Result<TasksRead, ClientFailure> {
        self.asked_once_more(|lane| lane.ask_tasks(alias))
    }

    /// One `RuntimeFactSnapshot`, and the settings' version read under the same gate,
    /// so what it says of the configuration is of that version.
    pub fn read_config(&mut self) -> Result<ConfigRead, ClientFailure> {
        self.asked_once_more(Self::ask_config)
    }

    /// One `StartReport`: which settings changed online this start did not restore, and
    /// which it restored over a default that changed.
    pub fn read_start(&mut self) -> Result<StartRead, ClientFailure> {
        self.asked_once_more(Self::ask_start)
    }

    /// `ask`, on the connection this lane kept or a new one.
    ///
    /// This lane reads seldom, and not at all while the workbench is minimized, so
    /// the connection it kept may have ended with the Runtime it was made to, and
    /// the read a restart pokes is the first to find out. Such a request reached
    /// nothing, so it goes once more on a new connection, and only that answer
    /// counts: the restart costs no failure and no wait for the next read. A
    /// connection made for this read is not asked again, and neither is anything
    /// that may be a Runtime busy with the first request (`connection_ended`). Only
    /// reads go through here.
    fn asked_once_more<T>(
        &mut self,
        mut ask: impl FnMut(&mut Self) -> Result<T, RuntimeClientError>,
    ) -> Result<T, ClientFailure> {
        let kept = self.client.is_some();
        let asked = match ask(self) {
            Err(error) if kept && connection_ended(&error) => ask(self),
            asked => asked,
        };
        asked.map_err(ClientFailure::from)
    }

    fn ask_explain(&mut self) -> Result<ExplainRead, RuntimeClientError> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.schedule_explanation(None, false) {
            Ok(explanation) => Ok(explain_read(epoch, &explanation)),
            Err(error) => Err(self.settled(error)),
        }
    }

    fn ask_tasks(&mut self, alias: &str) -> Result<TasksRead, RuntimeClientError> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.schedule_explanation(Some(alias), true) {
            Ok(explanation) => Ok(tasks_read(epoch, alias, &explanation)),
            Err(error) => Err(self.settled(error)),
        }
    }

    fn ask_config(&mut self) -> Result<ConfigRead, RuntimeClientError> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.runtime_fact_snapshot_versioned() {
            Ok((snapshot, version)) => Ok(config_read(epoch, &snapshot, version)),
            Err(error) => Err(self.settled(error)),
        }
    }

    fn ask_start(&mut self) -> Result<StartRead, RuntimeClientError> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.start_report() {
            Ok(report) => Ok(start_read(epoch, &report)),
            Err(error) => Err(self.settled(error)),
        }
    }

    /// The connection to ask on, made first when there is none.
    fn connection(&mut self) -> Result<RuntimeClient, RuntimeClientError> {
        if let Some(client) = &self.client {
            return Ok(client.clone());
        }
        let client = RuntimeClient::connect(
            RuntimeClientConfig::new(
                self.root.as_path(),
                RuntimeChannel::Ui,
                EventActor::Ui,
                EventSource::Ui,
            )
            .with_io_timeout(self.io),
        )?;
        self.client = Some(client.clone());
        Ok(client)
    }

    /// `error` as a read gives it back. After a fatal one the client has latched
    /// the connection, so the lane lets it go; a passing refusal leaves it in place.
    fn settled(&mut self, error: RuntimeClientError) -> RuntimeClientError {
        if error.is_fatal() {
            self.client = None;
        }
        error
    }
}

/// What one `Status` came to.
#[derive(Debug, Clone)]
pub enum LiveRead {
    /// No Runtime to reach: no `runtime-info.json`, or nothing listens at the
    /// address it names.
    NotConnected(ClientFailure),
    /// A Runtime is there and did not answer this read: it refused it as
    /// unavailable and goes on (it does so while it stops, or when the ledger's
    /// head was not answered in time), the read timed out, or the connection
    /// ended. The next read settles which.
    NoAnswer(ClientFailure),
    /// Something is wrong rather than merely down: `runtime-info.json` does not
    /// read, what came back does not decode or match the request, or the Runtime
    /// refused for any other reason. A halted host is one: it answers every read
    /// by what halted it until a restart. This does not pass by itself, so it is
    /// shown as an error.
    Trouble(ClientFailure),
    Read(Box<LiveSnapshot>),
}

/// The client's own spellings (`ClientCode`) of what [`LiveRead::NotConnected`]
/// and [`LiveRead::NoAnswer`] stand for, beside the one refusal [`live_failure`]
/// lets pass. Everything else the client can say, and every other refusal, a code
/// of a later Runtime included, is [`LiveRead::Trouble`]: it stays loud.
const RUNTIME_ABSENT_CODES: &[&str] = &["runtime_info_unavailable", "runtime_connect_failed"];
const RUNTIME_SILENT_CODES: &[&str] = &[
    "runtime_request_write_failed",
    "runtime_request_flush_failed",
    "runtime_receipt_header_failed",
    "runtime_receipt_read_failed",
    "runtime_receipt_timeout",
    // The next read connects to the new Runtime.
    "runtime_owner_epoch_changed",
];

fn live_failure(error: RuntimeClientError) -> LiveRead {
    match Unreached::of(error) {
        Unreached::NotConnected(failure) => LiveRead::NotConnected(failure),
        Unreached::NoAnswer(failure) => LiveRead::NoAnswer(failure),
        Unreached::Trouble(failure) => LiveRead::Trouble(failure),
    }
}

/// A read that did not come back, told apart as a failed `Status` is: see
/// [`LiveRead`] for each. The picture's reads ([`preview::PreviewLane`]) fail so.
#[derive(Debug, Clone)]
pub enum Unreached {
    NotConnected(ClientFailure),
    NoAnswer(ClientFailure),
    Trouble(ClientFailure),
}

impl Unreached {
    /// Of the Runtime's own refusals only `runtime_unavailable` passes, and only when
    /// the host has not halted: a halted host keeps the code of what halted it, which
    /// may be this one, and says so by the refusal being fatal. `ClientFailure` does
    /// not carry that, so this looks at the client's error.
    fn of(error: RuntimeClientError) -> Self {
        let unavailable = error.projection().is_some_and(|refusal| {
            refusal.code == RuntimeErrorCode::RuntimeUnavailable && !refusal.fatal
        });
        let failure = ClientFailure::from(error);
        if unavailable || RUNTIME_SILENT_CODES.contains(&failure.code) {
            Self::NoAnswer(failure)
        } else if RUNTIME_ABSENT_CODES.contains(&failure.code) {
            Self::NotConnected(failure)
        } else {
            Self::Trouble(failure)
        }
    }

    /// The client's failure, whichever it is: for the log.
    pub fn failure(&self) -> &ClientFailure {
        match self {
            Self::NotConnected(failure) | Self::NoAnswer(failure) | Self::Trouble(failure) => {
                failure
            }
        }
    }
}

/// Whether a request found its connection over: the write failed, or the connection
/// ended before the receipt's header. A read that timed out is not this, though the
/// client gives it the same code when the request has no deadline of its own: the
/// Runtime may be busy, and a second request would wait behind the first.
fn connection_ended(error: &RuntimeClientError) -> bool {
    matches!(
        error.code().spelling(),
        "runtime_request_write_failed" | "runtime_request_flush_failed"
    ) || error.receipt_header_io().is_some_and(|io| {
        matches!(
            io.kind(),
            ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
        )
    })
}

/// What became of one person's global pause ([`pause_global`]), or of one person's pause of
/// one account ([`pause_account`]).
#[derive(Debug, Clone)]
pub enum PauseOutcome {
    /// The pause holds, at this revision of its scope.
    Applied { revision: u64 },
    /// The Runtime answered and set nothing. Its code is the host's, in `host`:
    /// `runtime_code` is only the receipt's class. One pause already holding is
    /// `scheduling_already_paused`, and keeps its own reason.
    Refused(ClientFailure),
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it. It is not sent again.
    Unconfirmed(ClientFailure),
}

/// One person's global pause: a fresh person's connection, one `PauseScheduling`,
/// and the connection let go. The global pause drains nothing and answers at once,
/// and the Runtime does not use its drain timeout, which it still validates: the
/// lowest it takes keeps the wait for a receipt that does not come the shortest
/// (that timeout, the checkpoint grace and the io timeout). Sent once, whatever comes
/// back; what to show after it is a read on another connection.
///
/// Every failure after the connection that is no refusal counts as unconfirmed, not
/// only those the client calls uncertain: a failed write may have got the frame out.
pub fn pause_global(root: &Path) -> PauseOutcome {
    pause(root, SchedulingPauseScope::Global, CONSOLE_PAUSE_ALL)
}

/// One person's pause of the one account `instance_alias`: as [`pause_global`], a fresh
/// person's connection, one `PauseScheduling` of that instance's scope with this console's
/// own reason ([`CONSOLE_PAUSE_ACCOUNT`]) and the same drain timeout, and the connection let
/// go; sent once, whatever comes back. An instance pause does use that timeout: a run of the
/// account still in flight once it has passed is asked to stop at its next checkpoint, and the
/// Runtime answers once the device is handed back, so the receipt may take that timeout, the
/// checkpoint grace and the io timeout. A stage that fails lifts the pause again, and is a
/// refusal; so is a fixture instance's (`fixture_execution_scope_forbidden`), and an alias
/// the Runtime does not know.
pub fn pause_account(root: &Path, instance_alias: &str) -> PauseOutcome {
    let scope = SchedulingPauseScope::Instance { instance_alias: instance_alias.to_owned() };
    pause(root, scope, CONSOLE_PAUSE_ACCOUNT)
}

/// One `PauseScheduling` of `scope` with `reason`, for both of the above.
fn pause(root: &Path, scope: SchedulingPauseScope, reason: &str) -> PauseOutcome {
    let reason = match SchedulingPauseReason::new(reason) {
        Ok(reason) => reason,
        Err(error) => {
            return PauseOutcome::NotSent(ClientFailure {
                code: error.code().spelling(),
                operation: PAUSE_SCHEDULING,
                runtime_code: None,
                host: None,
            })
        }
    };
    let client = match OnlineSource::connect_as_person(root) {
        Ok(client) => client,
        Err(error) => return PauseOutcome::NotSent(ClientFailure::from(error)),
    };
    let answer = client.pause_scheduling(scope, reason, MIN_SCHEDULING_PAUSE_DRAIN_TIMEOUT_MS);
    match answer {
        Ok(RuntimeResult::SchedulingPaused { revision, .. }) => PauseOutcome::Applied { revision },
        // The client checks the result itself and gives back no other.
        Ok(_) => PauseOutcome::Unconfirmed(ClientFailure {
            code: RUNTIME_RESULT_UNEXPECTED,
            operation: PAUSE_SCHEDULING,
            runtime_code: None,
            host: None,
        }),
        Err(error) => {
            let failure = ClientFailure::from(error);
            if failure.runtime_code.is_some() {
                PauseOutcome::Refused(failure)
            } else {
                PauseOutcome::Unconfirmed(failure)
            }
        }
    }
}

/// The settings to change in one `ConfigureSettings`, all of them or none. Only this file
/// can spell one, so nothing outside it holds a key or the wire's shape.
#[derive(Debug, Clone)]
pub struct ConfigEdit(Vec<ConfigSettingChange>);

impl ConfigEdit {
    /// Every setting of the preset `name` of the table `read` holds; `None` when it holds
    /// none by that name. A setting the preset leaves unwritten is a change without a
    /// value, which the crate's `changes` decides.
    pub fn apply_preset(read: &ConfigRead, name: &str) -> Option<Self> {
        let preset = read.preset.as_ref().ok()?;
        Some(Self(preset.table.get(name)?.changes()))
    }

    /// Every setting a preset sets, back to its value in the configuration file: a change
    /// without a value drops the online one.
    pub fn preset_back_to_file() -> Self {
        Self(
            presets::MEMBERS
                .iter()
                .map(|member| ConfigSettingChange { key: member.key.to_owned(), value: None })
                .collect(),
        )
    }

    /// How many tasks run at once, as an online value. The Runtime alone knows the range,
    /// and says it when it refuses one.
    pub fn concurrent_runs(runs: i64) -> Self {
        Self(vec![ConfigSettingChange {
            key: pacing::MAX_CONCURRENT_RUNS.to_owned(),
            value: Some(runs.into()),
        }])
    }

    /// How many tasks run at once, back to the configuration file's value, or to no limit
    /// when the file names none: the key has no default.
    pub fn concurrent_runs_back_to_file() -> Self {
        Self(vec![ConfigSettingChange {
            key: pacing::MAX_CONCURRENT_RUNS.to_owned(),
            value: None,
        }])
    }

    /// How many days an evidence frame nobody confirmed reading is kept, as an online value.
    /// The Runtime alone knows the range, and says it when it refuses one.
    pub fn evidence_days(days: i64) -> Self {
        Self(vec![ConfigSettingChange {
            key: EVIDENCE_MAX_AGE_DAYS.to_owned(),
            value: Some(days.into()),
        }])
    }

    /// The same, back to the configuration file's value, or to the default when the file
    /// names none.
    pub fn evidence_days_back_to_file() -> Self {
        Self(vec![ConfigSettingChange { key: EVIDENCE_MAX_AGE_DAYS.to_owned(), value: None }])
    }

    /// Whether it changes an instance's backends ([`ConfigEdit::backends`]): the Runtime switches
    /// that device before it answers, so the answer takes as long as the switch, which the
    /// client waits `BACKEND_SWITCH_WAIT_MS` and its io timeout for ([`backends::switch_wait`]),
    /// and an answer lost is an [`backends::UnconfirmedSwitch`]'s, not another change's.
    pub fn switches_backends(&self) -> bool {
        self.0.iter().any(|change| is_instance_backend_key(&change.key))
    }

    /// What it changes, keys and values as the wire has them: for the log only.
    pub fn line(&self) -> String {
        self.0
            .iter()
            .map(|change| match &change.value {
                Some(value) => format!("{} = {value}", change.key),
                None => format!("{} back to the file", change.key),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What became of one [`configure`].
#[derive(Debug, Clone)]
pub enum ConfigOutcome {
    /// The changes hold. `version` is the settings' version after them, absent from a
    /// Runtime before 0.12.36. The values to show are a read's, not this answer's: a
    /// cleared key without a file value comes back as the text "null".
    Applied { version: Option<u64> },
    /// The settings are at another version than the one named, and nothing changed.
    /// `current` is only for the log: it comes without the values, so what is sent next
    /// is based on a snapshot read, which has both.
    Conflict { current: Option<u64> },
    /// The Runtime answered and changed nothing.
    Refused(ConfigRefusal),
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it. It is not sent again.
    ///
    /// Of a change that [`ConfigEdit::switches_backends`] the Runtime may still be switching
    /// the device, and moves the settings' version only once it commits, at the switch's end:
    /// a snapshot at the version it was based on then says nothing yet. That is a
    /// [`backends::UnconfirmedSwitch`], judged as its own says, never as any other change.
    Unconfirmed(ClientFailure),
}

/// One refusal: why, for its words, and for the log only what the Runtime said and the key
/// it named.
#[derive(Debug, Clone)]
pub struct ConfigRefusal {
    pub why: RefusalWhy,
    pub failure: ClientFailure,
    pub key: Option<String>,
    /// HS-U: of a change of an instance's backends, what the refusal says of them beyond `why`.
    /// `None` for any other change, and for a refusal that says nothing of them.
    pub backend: Option<backends::BackendRefusal>,
}

/// Why the Runtime refused a change of settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusalWhy {
    /// The value does not do: how, and the range of a bounded key.
    Value { problem: ValueProblem, range: Option<(i64, i64)> },
    /// The key cannot change while the Runtime runs.
    NotSettable,
    /// This Runtime has no such key.
    UnknownKey,
    /// The request's shape is wrong: this program's fault.
    BadRequest,
    /// The Runtime could not save the change.
    NotSaved,
    /// This Runtime does not take a change made this way.
    Unsupported,
    /// Any other refusal: its code is in the failure.
    Other,
}

/// How a value does not do; the rest of the Runtime's list concerns sections nothing here
/// writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueProblem {
    WrongType,
    OutOfRange,
    SectionCheck,
    RepeatedKey,
    /// `start_emulator` on for an instance whose emulator the Runtime cannot start: one not bound
    /// by discovery (Workflow #429 R-3, 0.12.39).
    DiscoveryBindingRequired,
    Other,
}

const VALUE_INVALID: &str = ConfigCode::ConfigValueInvalid.as_str();
const KEY_NOT_SETTABLE: &str = ConfigCode::ConfigKeyNotSettable.as_str();
const KEY_UNKNOWN: &str = ConfigCode::ConfigKeyUnknown.as_str();
const WRONG_TYPE: &str = ConfigValueProblem::WrongType.as_str();
const OUT_OF_RANGE: &str = ConfigValueProblem::OutOfRange.as_str();
const SECTION_CHECK: &str = ConfigValueProblem::SectionCheck.as_str();
const REPEATED_KEY: &str = ConfigValueProblem::RepeatedKey.as_str();
const DISCOVERY_BINDING_REQUIRED: &str = ConfigValueProblem::DiscoveryBindingRequired.as_str();
/// The contract's and the host's own spellings, of crates this one does not use.
const INVALID_CONFIGURE_SETTINGS: &str = "invalid_configure_settings";
const OVERRIDES_PERSIST_FAILED: &str = "host_config_overrides_persist_failed";
const NOT_IMPLEMENTED: &str = "not_implemented";
const VERSION_CONFLICT: &str = "fact_observation_version_conflict";
/// What a refusal's outcome says beyond its code.
const CURRENT_VERSION: &str = "current_version";
const CONFIG_KEY: &str = "config_key";
const VALUE_PROBLEM: &str = "config_value_problem";
const RANGE_MIN: &str = "range_min";
const RANGE_MAX: &str = "range_max";

/// One person's change of settings: a fresh person's connection, one `ConfigureSettings`
/// based on the settings' version `expected` when there is one, and the connection let
/// go. Sent once, whatever comes back.
///
/// Only this connection asks for the full receipt: the console's default keeps no key, no
/// problem and no range, and no cause of a refused switch of backends.
///
/// A change of an instance's backends switches that device before the Runtime answers: the
/// client then waits for the receipt the contract's `BACKEND_SWITCH_WAIT_MS` and the io timeout
/// ([`backends::switch_wait`]), where any other change answers at once. It waits on the caller's
/// thread, so that is never the event loop's. That wait is the Runtime's worst case of the whole
/// switch, but a session close has no source bound and a ledger writer may be configured above
/// its default: the client may give up, or its connection fail, while the switch goes on, and its
/// unconfirmed answer is then a [`backends::UnconfirmedSwitch`]'s.
pub fn configure(root: &Path, edit: &ConfigEdit, expected: Option<u64>) -> ConfigOutcome {
    let connected = RuntimeClient::connect(configure_config(root));
    let client = match connected {
        Ok(client) => client,
        Err(error) => return ConfigOutcome::NotSent(ClientFailure::from(error)),
    };
    match client.configure_settings(edit.0.clone(), expected) {
        Ok(configured) => ConfigOutcome::Applied { version: configured.config_version },
        Err(error) => configure_failed(error, edit.switches_backends()),
    }
}

/// The client configuration of [`configure`]'s connection: a person's, asking for the full
/// receipt, at the client's default io timeout. A device's operation takes the same, at its own
/// io timeout ([`device::DEVICE_IO`]).
fn configure_config(root: &Path) -> RuntimeClientConfig {
    RuntimeClientConfig::new(root, RuntimeChannel::Ui, EventActor::User, EventSource::Ui)
        .with_detail(Detail::Full)
}

/// The io timeout of [`configure`]'s connection, which the client adds to the receipt wait of a
/// switch of backends ([`backends::switch_wait`]): read from the same configuration, whose state
/// root plays no part in it, so that it is the client's default and not a copy of it.
fn configure_io_timeout() -> Duration {
    configure_config(Path::new("")).io_timeout()
}

/// A failure after the connection that is no refusal counts as unconfirmed, as for
/// [`pause_global`]: a failed write may have got the frame out. So does a refusal that
/// halted the host: a change the ledger took and the section record it then refused stops
/// the Runtime, and the next start restores it. A refusal of a change that `switches_backends`
/// says what it says of them too. Its unconfirmed answer is the caller's to judge as a switch
/// ([`backends::UnconfirmedSwitch`]): the caller knows what it asked for, and has to judge so
/// as well a switch whose write's thread panicked, which brings no answer here at all.
fn configure_failed(error: RuntimeClientError, switches_backends: bool) -> ConfigOutcome {
    if !error.projection().is_some_and(|refusal| !refusal.fatal) {
        return ConfigOutcome::Unconfirmed(ClientFailure::from(error));
    }
    let answer = error.receipt_outcome();
    let top = answer.map_or("", |outcome| outcome.code().as_str());
    if top == VERSION_CONFLICT {
        let current = answer
            .and_then(|outcome| outcome.value(CURRENT_VERSION))
            .and_then(|value| value.as_u64());
        return ConfigOutcome::Conflict { current };
    }
    let why = match top {
        VALUE_INVALID => RefusalWhy::Value {
            problem: value_problem(answer),
            range: receipt_integer(answer, RANGE_MIN).zip(receipt_integer(answer, RANGE_MAX)),
        },
        KEY_NOT_SETTABLE => RefusalWhy::NotSettable,
        KEY_UNKNOWN => RefusalWhy::UnknownKey,
        INVALID_CONFIGURE_SETTINGS => RefusalWhy::BadRequest,
        OVERRIDES_PERSIST_FAILED => RefusalWhy::NotSaved,
        NOT_IMPLEMENTED => RefusalWhy::Unsupported,
        _ => RefusalWhy::Other,
    };
    let key = answer
        .and_then(|outcome| outcome.value(CONFIG_KEY))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let backend = if switches_backends { backends::refusal(top, answer) } else { None };
    ConfigOutcome::Refused(ConfigRefusal { why, failure: ClientFailure::from(error), key, backend })
}

fn value_problem(answer: Option<&Outcome>) -> ValueProblem {
    let problem = answer
        .and_then(|outcome| outcome.value(VALUE_PROBLEM))
        .and_then(|value| value.as_str());
    match problem {
        Some(WRONG_TYPE) => ValueProblem::WrongType,
        Some(OUT_OF_RANGE) => ValueProblem::OutOfRange,
        Some(SECTION_CHECK) => ValueProblem::SectionCheck,
        Some(REPEATED_KEY) => ValueProblem::RepeatedKey,
        Some(DISCOVERY_BINDING_REQUIRED) => ValueProblem::DiscoveryBindingRequired,
        Some(_) | None => ValueProblem::Other,
    }
}

fn receipt_integer(answer: Option<&Outcome>, key: &str) -> Option<i64> {
    answer?.value(key)?.as_i64()
}

/// One answered `Status`. `epoch` and `pid` are of the connection's
/// `runtime-info.json`, `epoch` as the wire spells it.
#[derive(Debug, Clone)]
pub struct LiveSnapshot {
    pub pid: u32,
    pub epoch: String,
    pub global_pause: Option<PauseFact>,
    pub instances: Vec<InstanceLive>,
    /// The ledger's committed head when the Runtime sampled the status
    /// (`ledger_committed_head`, 0 for an empty ledger): where the picture's lookup
    /// starts down from. Absent when the status states no source; the picture then
    /// asks nothing until a status states one.
    pub ledger_head: Option<u64>,
}

/// One registered instance as that `Status` reported it.
#[derive(Debug, Clone)]
pub struct InstanceLive {
    pub alias: String,
    pub instance_id: String,
    /// The same id in the wire's own type, which nothing outside this crate reads:
    /// what the picture's pages ask by ([`preview::PreviewLane`]). The contract gives
    /// the id no reading from text, so it is kept as the status gave it.
    pub key: InstanceKey,
    /// Absent when the host runs without policy inputs.
    pub game_id: Option<String>,
    pub pause: Option<PauseFact>,
    pub fence: Option<FenceFact>,
    /// The instance's recovery ladder holds instead of climbing.
    pub ladder_busy: bool,
    /// Absent while it does nothing the vocabulary names, and always from a
    /// Runtime before 0.12.36: not an error.
    pub activity: Option<ActivityFact>,
    /// When the Runtime last read the instance's emulator as not running, in unix ms;
    /// absent when it last read it running or holds no reading (`unknown`). A reading of
    /// discovery, of an emulator's control or of the emulator watch's poll, so it may be
    /// stale: a condition of the picture's refresh and of the device's buttons, whose reason
    /// says when it was seen, never shown as what holds now.
    pub emulator_off: Option<u64>,
    /// When the Runtime last read the instance's emulator as running, in unix ms, read as
    /// `emulator_off` is: absent when it last read it not running or holds no reading.
    pub emulator_on: Option<u64>,
    /// It drives a device of its own (`backend_provenance` `physical_device`), an emulator or a
    /// handset, where a fixture's simulation does not: what the self-check and the game's
    /// buttons are drawn for. A status that states no provenance counts as none (`unstated`).
    pub physical: bool,
    /// Which of its emulator's controls the Runtime offers: those its capability profile has
    /// available. Every instance that drives a device of its own has a profile, but only a
    /// discovery-bound MuMu instance's has `instance.start`, `instance.stop` and
    /// `instance.restart` available; a handset's and an emulator's named by its address have
    /// them unavailable, and a fixture has no profile: each is offered none. A status that
    /// states no profile counts as offering none (`unstated`).
    pub emulator_controls: EmulatorControls,
    /// What the status left out of what `physical` and `emulator_controls` are read from.
    pub unstated: Unstated,
}

/// Which of an instance's emulator's controls the Runtime offers (`instance.start`,
/// `instance.stop`, `instance.restart` of its capability profile): each of the emulator's
/// buttons by its own ([`device::DeviceOp`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmulatorControls {
    pub start: bool,
    pub stop: bool,
    pub restart: bool,
}

/// What a status left out of what [`InstanceLive::physical`] and
/// [`InstanceLive::emulator_controls`] are read from, so that it counts as no device, or no
/// control: for the log only. A Runtime states every instance's provenance, and a capability
/// profile of every instance that drives a device of its own (only a fixture has none), so
/// either left out of such an instance is out of the ordinary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unstated {
    /// No `backend_provenance`.
    pub provenance: bool,
    /// No `capabilities`.
    pub capabilities: bool,
}

/// An instance's id as the wire types it, opaque outside this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceKey(InstanceId);

/// An instance's resource fence: the code of the failure behind it, and whether it
/// now waits for a person. The backend open whose failure caused it, which a switch of the
/// instance's backends remedies, is [`FenceFact::failed_open`] (`backends`, which spells the
/// self-check's failures).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceFact {
    pub cause: String,
    pub awaiting_person: bool,
}

/// What an instance does now. `policy_run` and `direct_run` always name a package. `run`: the
/// request of what it does (`request_id`), the run's identity a stop of it names (UI-3e2, UI-3e3),
/// which a `direct_run` and a `policy_run` always state; never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityFact {
    pub kind: ActivityKind,
    pub package_id: Option<String>,
    pub run: Option<RunKey>,
}

/// A run's identity as the wire types it, its request's id, opaque outside this crate as
/// [`InstanceKey`] is: what a stop of the run names ([`stop::stop_run`]), compared and copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunKey(RequestId);

impl RunKey {
    /// The request's id as the wire spells it: for the technical log only, never shown.
    pub fn line(&self) -> String {
        code(&self.0)
    }
}

/// `InstanceActivityKind`, a closed vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    PolicyRun,
    DirectRun,
    HostRun,
    Reconnect,
    Selfcheck,
    EmulatorControl,
    Ladder,
}

/// One answered `ScheduleExplanation`. `evaluated` is false before the first
/// evaluated cycle after a start, when `rows` is empty; an instance the latest
/// evaluation decided nothing for has no row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainRead {
    pub epoch: String,
    pub evaluated: bool,
    /// By instance alias.
    pub rows: BTreeMap<String, InstanceExplain>,
}

/// An instance's one not-running row. `why` is its reason code, which the words
/// shown are chosen by, never shown itself; so are `gap` and `hold`, the reason's
/// `benefit_gap` and `pool_line_hold` when it has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceExplain {
    pub why: String,
    pub gap: Option<String>,
    pub hold: Option<String>,
    /// That row's own wake in unix ms; with no due row (`due_task_missing`) the
    /// instance's earliest wake, which is the only one there is then.
    pub next_wake: Option<u64>,
    /// A task is selected and its dispatch has not ended.
    pub preparing: bool,
}

/// One answered `ScheduleExplanation` of one instance, with its candidate rows (UI-3f1): that
/// account's task list. `evaluated_at` is absent before the first evaluated cycle after a start;
/// `entry` when the latest evaluation decided nothing for the instance. Its catalog hash stays
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasksRead {
    pub epoch: String,
    /// The alias asked for: the client refuses an answer that names another.
    pub alias: String,
    pub evaluated_at: Option<u64>,
    pub entry: Option<TasksEntry>,
}

/// The instance's entry: its earliest wake, the task selected for it, and its candidate rows in
/// the Runtime's own order, due rows first. `cut_short`: it counted more rows than it answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasksEntry {
    pub next_wake: Option<u64>,
    pub selected: Option<SelectedTask>,
    pub rows: Vec<TaskRow>,
    pub cut_short: bool,
}

/// The task selected for the instance, and where its dispatch stands. Its decision id stays
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedTask {
    pub task_id: String,
    pub stage: DispatchStage,
}

/// `ScheduleDispatchState`, a closed vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchStage {
    Pending,
    Intent,
    Admitted,
    Rejected,
    Completed,
}

/// One candidate row, a decision that was not selected. `why` is the code of its last reason,
/// which the words shown are chosen by, never shown itself; so are `gap` and `hold`, as of an
/// instance's row ([`InstanceExplain`]). `wake` is the row's own. Its other reason codes, its
/// since and its expected runtime stay here: the list shows no count and no number of the
/// scheduler's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub task_id: String,
    /// Its eligibility is true.
    pub due: bool,
    pub why: String,
    pub gap: Option<String>,
    pub hold: Option<String>,
    pub wake: Option<u64>,
}

/// What one answered `StartReport` says of the settings changed while a Runtime ran, as far as
/// the console reads it. Each is said once, by key, in the Runtime's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRead {
    pub epoch: String,
    /// The ones this start did not restore: an update or a rollback dropped them, or the
    /// configuration file now names them itself.
    pub dropped: Vec<ReportedSetting>,
    /// The ones it restored although the default they change has itself changed.
    pub base_changed: Vec<ReportedSetting>,
}

impl StartRead {
    /// Both lists by key: for the log only.
    pub fn line(&self) -> String {
        format!(
            "dropped [{}], base changed [{}]",
            reported_keys(&self.dropped),
            reported_keys(&self.base_changed),
        )
    }
}

fn reported_keys(settings: &[ReportedSetting]) -> String {
    settings.iter().map(|setting| setting.key.as_str()).collect::<Vec<_>>().join(", ")
}

/// One setting a start report names: which it is, for its words, and the instance of an
/// instance's own. Its key stays here, for the log (`StartRead::line`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedSetting {
    pub name: ReportedName,
    /// The instance id of an instance's own setting.
    pub instance: Option<String>,
    key: String,
}

/// Which setting a start report names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportedName {
    /// One the performance preset sets.
    Preset(PresetSetting),
    /// Any other, by the settings page's name: `SettingName::Unnamed` for one without a name
    /// here, a key this Runtime's key table does not have included (its key, then).
    Setting(SettingName),
}

fn start_read(epoch: String, report: &RuntimeStartReport) -> StartRead {
    StartRead {
        epoch,
        dropped: reported_settings(&report.instructions_dropped),
        base_changed: reported_settings(&report.instructions_base_changed),
    }
}

fn reported_settings(keys: &[String]) -> Vec<ReportedSetting> {
    keys.iter().map(|key| reported_setting(key)).collect()
}

fn reported_setting(key: &str) -> ReportedSetting {
    let (name, instance) = match (preset_setting(key), find_row(key)) {
        (Some(setting), _) => (ReportedName::Preset(setting), None),
        (None, Some((row, instance))) => {
            (ReportedName::Setting(setting_name(row.name)), instance.map(str::to_owned))
        }
        (None, None) => (ReportedName::Setting(SettingName::Unnamed(key.to_owned())), None),
    };
    ReportedSetting { name, instance, key: key.to_owned() }
}

/// One answered `RuntimeFactSnapshot`, as far as the workbench reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRead {
    pub epoch: String,
    /// The settings' version read with it. Absent from a Runtime before 0.12.36: not
    /// an error, and also how to tell that its `Status` names no activity.
    pub version: Option<u64>,
    pub preset: Result<PresetRead, PresetProblem>,
    /// The server the last admitted package declared, as it wrote it, by instance id.
    /// Only a package's admission records it, and an emulator's stop withdraws it, so
    /// most of the time an instance has none.
    pub servers: BTreeMap<String, String>,
    /// The settings the console shows. `None` when the snapshot has no `config.parameters`
    /// record at all: a Runtime that reports no configuration, whose settings are then not
    /// known, rather than unset.
    pub settings: Option<SettingsRead>,
    /// HS-U: each device instance's touch and capture backends, by instance id: every instance a
    /// backend row names. Empty from a Runtime before 0.12.39, which has no such keys, and of a
    /// fixture, whose file names none: neither is an error.
    pub backends: BTreeMap<String, backends::InstanceBackends>,
}

/// What a snapshot says of the settings the console shows, by the Runtime's own tiers of
/// them (`ui_tier`): every `config.parameters` row of a tier other than hidden, but for the
/// four settings of the performance preset, which are shown through the preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRead {
    /// By key, as `config.parameters` names it.
    parameters: BTreeMap<String, ShownParameter>,
    /// The rows that do not read and may be of such a setting: of a tier other than hidden,
    /// or naming no key; a section that is no list of rows counts as one. Said, not dropped:
    /// a key among them is not shown, rather than as unset.
    unread: Vec<UnreadRow>,
    /// UI-3e1-b: the entries of each blackout list's row `parameters` has, the host's and each
    /// instance's own, by key, as the Runtime's parser reads them ([`rules`]).
    quiet: BTreeMap<String, rules::ListRead>,
    /// The snapshot has a row of an instance's touch or capture backend: its Runtime is 0.12.39
    /// or later, which has those keys (HS-1), and takes a change of `start_emulator` (R-3).
    backend_rows: bool,
}

/// A `config.parameters` row of a setting the console shows, or naming no key, that does
/// not read; or a whole section that is no list of rows. Only the log says which and why
/// (`line`): the window counts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadRow {
    /// The section's record key.
    section: String,
    why: Unread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Unread {
    /// The section is no list of rows.
    Section,
    /// A row naming no key.
    Keyless,
    /// A row of this key whose members are not those of a parameter.
    Shape(String),
    /// A row of this key whose value is not of the key's type: a row the Runtime would
    /// not report.
    Value { key: String, value_type: ValueType, value: FactScalar },
}

impl UnreadRow {
    /// The key of the setting it is the row of, if it names one.
    fn key(&self) -> Option<&str> {
        match &self.why {
            Unread::Shape(key) | Unread::Value { key, .. } => Some(key.as_str()),
            Unread::Section | Unread::Keyless => None,
        }
    }

    /// Which row it is and why it does not read, keys and all: for the log only.
    pub fn line(&self) -> String {
        let section = &self.section;
        match &self.why {
            Unread::Section => format!("{section} is no list of rows"),
            Unread::Keyless => format!("a row of {section} names no key"),
            Unread::Shape(key) => {
                format!("the row of {key} in {section} lacks a value, a known source or a settable flag")
            }
            Unread::Value { key, value_type, value } => {
                format!("the value of {key} in {section} is no {value_type:?}: {value:?}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShownParameter {
    value: ShownValue,
    source: SettingSource,
    settable: bool,
}

impl SettingsRead {
    /// Every setting to show, in the order of the Runtime's tiers: an exact key once, a
    /// per-instance key for each of `instances` (instance ids, in their order). A key with no
    /// row, which a key without a default has while the file does not name it, is unset at
    /// its default, and settable as its key-table row says (but `unreported_settable`), so it
    /// is not left off; a key whose row does not read is (`unread` has it). Then every row of a
    /// shown tier none of those named, of an instance `instances` does not list. An instance's
    /// two backends are none of them: they are its device backends group's
    /// (`ConfigRead::backends`), and a key without a row there is no unset setting but a
    /// fixture's, or a Runtime's without them.
    pub fn rows(&self, instances: &[&str]) -> Vec<SettingRow> {
        let mut rows = Vec::new();
        let mut named = BTreeSet::new();
        for &(name, _) in UI_TIERS {
            if backends::is_backend_name(name) {
                continue;
            }
            let keys: Vec<(String, Option<&str>)> = match name {
                KeyName::Exact(key) => vec![(key.to_owned(), None)],
                KeyName::PerInstance(member) => instances
                    .iter()
                    .map(|instance| (instance_key(instance, member), Some(*instance)))
                    .collect(),
            };
            for (key, instance) in keys {
                let Some(tier) = shown_tier(&key) else {
                    continue;
                };
                if self.unread.iter().any(|row| row.key() == Some(key.as_str())) {
                    continue;
                }
                rows.push(self.row(&key, name, tier, instance));
                named.insert(key);
            }
        }
        for key in self.parameters.keys().filter(|key| !named.contains(*key)) {
            let (Some(tier), Some((row, instance))) = (shown_tier(key), find_row(key)) else {
                continue;
            };
            rows.push(self.row(key, row.name, tier, instance));
        }
        rows
    }

    fn row(
        &self,
        key: &str,
        name: KeyName,
        tier: SettingTier,
        instance: Option<&str>,
    ) -> SettingRow {
        let (value, source, settable) = match self.parameters.get(key) {
            Some(shown) => (shown.value.clone(), shown.source, shown.settable),
            None => (
                ShownValue::Unset,
                SettingSource::Default,
                self.unreported_settable(key, name),
            ),
        };
        SettingRow {
            name: setting_name(name),
            tier,
            instance: instance.map(str::to_owned),
            value,
            source,
            settable,
        }
    }

    /// Whether the Runtime takes a change while it runs of `key`, which has no row: as its
    /// key-table row says, but for an instance's `start_emulator`. That one is settable only
    /// since 0.12.39 (Workflow #429 R-3), and the Runtime that answers may be an older one than
    /// the key table here: another instance's row of it says which. With none, a row of any
    /// instance's touch or capture backend does: those keys are 0.12.39's too (HS-1), and a device
    /// instance's file names both. With neither it is not, as before 0.12.39 (a change in the file
    /// with a restart is taken on 0.12.39 too).
    fn unreported_settable(&self, key: &str, name: KeyName) -> bool {
        if name != KeyName::PerInstance(START_EMULATOR) {
            return find_row(key).is_some_and(|(row, _)| row.settable.is_settable());
        }
        self.parameters
            .iter()
            .find_map(|(other, shown)| {
                find_row(other)
                    .is_some_and(|(row, _)| row.name == name)
                    .then_some(shown.settable)
            })
            .unwrap_or(self.backend_rows)
    }

    /// The rows that do not read, in the snapshot's order: none of them is shown.
    pub fn unread(&self) -> &[UnreadRow] {
        &self.unread
    }

    /// Whether the instance of `instance_id` has `start_emulator` on in effect, the file's value
    /// or an instruction's, as the emulator watch reads it at every poll: it starts a stopped
    /// emulator of an instance that has it on, unless that instance's own pause holds it (EMU-W
    /// R3, Runtime 0.12.39). With no row it is off: the Runtime reports the key only of an
    /// instance that names it or that an instruction sets it for, and it has no default. `None`
    /// when that does not read: its row does not, or a row or section naming no key may be it.
    pub fn start_emulator(&self, instance_id: &str) -> Option<bool> {
        let key = instance_key(instance_id, START_EMULATOR);
        if self.unread.iter().any(|row| row.key().map_or(true, |unread| unread == key)) {
            return None;
        }
        match self.parameters.get(&key) {
            Some(ShownParameter { value: ShownValue::Flag(on), .. }) => Some(*on),
            Some(_) => None,
            None => Some(false),
        }
    }

    /// A row of `section`. One whose value is not of its key's type does not read, as one of
    /// another shape does not: shown as some other value, it would say what the Runtime
    /// does not.
    fn take(&mut self, section: &str, parameter: ConfigParameter) {
        // A key the key table does not have is hidden (`ui_tier`), as is one of no shown tier.
        let (Some((row, instance)), Some(_)) =
            (find_row(&parameter.key), shown_tier(&parameter.key))
        else {
            return;
        };
        let Some(value) = shown_value(row.value_type, &parameter.value) else {
            self.unread.push(UnreadRow {
                section: section.to_owned(),
                why: Unread::Value {
                    key: parameter.key,
                    value_type: row.value_type,
                    value: parameter.value,
                },
            });
            return;
        };
        // UI-3e1-b/-c: a blackout list's entries are read as well, by the Runtime's own parser: an
        // instance's own for its account's rules, its row showing only as set or not, and the
        // host's for the editor of every account's quiet times, which the settings page shows in
        // its row's place.
        if let Some(entries) = rules::list_read(row.name, instance, &parameter.value) {
            self.quiet.insert(parameter.key.clone(), entries);
        }
        // HS-U: an instance's backends are read as its group (`backends::read`); one whose row
        // does not read is counted above as any other.
        if backends::is_backend_name(row.name) {
            self.backend_rows = true;
            return;
        }
        let shown = ShownParameter {
            value,
            source: setting_source(parameter.source),
            settable: parameter.settable,
        };
        self.parameters.insert(parameter.key, shown);
    }
}

/// One setting the console shows, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    pub name: SettingName,
    pub tier: SettingTier,
    /// The instance id of an instance's own setting.
    pub instance: Option<String>,
    pub value: ShownValue,
    pub source: SettingSource,
    /// Whether the Runtime takes a change of it while it runs.
    pub settable: bool,
}

/// Which setting it is, for its words. Of the console's tiers' keys, without the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingName {
    /// The quiet times of every instance.
    QuietTimesHost,
    /// An instance's own quiet times.
    QuietTimes,
    /// Whether the Runtime starts an instance's emulator at its start.
    StartEmulator,
    /// What the Runtime does with an instance that has no task soon.
    IdleAction,
    /// Where the MuMu emulator is.
    MumuRoot,
    /// How many days an evidence frame nobody confirmed reading is kept.
    EvidenceMaxAgeDays,
    /// The Runtime process's memory priority.
    MemoryPriority,
    /// The Runtime process's I/O priority.
    IoPriority,
    /// The Lab's two options, authoring and debugging.
    LabAuthoring,
    LabDebugging,
    /// An instance's touch and capture backends (HS-U). The settings page shows them as the
    /// instance's device backends group, never as rows (`backends`): a start report names them.
    TouchBackend,
    CaptureBackend,
    /// A key of the tiers this file has no name for: its key-table name (a per-instance
    /// one's member), for the log only. A start report's key the key table does not have is
    /// one too, by the key itself.
    Unnamed(String),
}

/// The console's two tiers that show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingTier {
    Daily,
    Advanced,
}

/// A setting's value as the settings page has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShownValue {
    /// None in effect: there is no row, or a list or object has no entry.
    Unset,
    /// A list or object with entries: which they are is not said.
    Set,
    Flag(bool),
    Number(i64),
    /// A word of the key's range, a path, or a number of another type, as written.
    Word(String),
}

/// The page's tier of `key`: the Runtime's own, but none for the settings of the
/// performance preset, which are all exact keys.
fn shown_tier(key: &str) -> Option<SettingTier> {
    if presets::MEMBERS.iter().any(|member| member.key == key) {
        return None;
    }
    match ui_tier(key) {
        UiTier::Daily => Some(SettingTier::Daily),
        UiTier::Advanced => Some(SettingTier::Advanced),
        UiTier::Hidden => None,
    }
}

fn setting_name(name: KeyName) -> SettingName {
    match name {
        KeyName::Exact(scheduling::BLACKOUT_HOST) => SettingName::QuietTimesHost,
        KeyName::PerInstance(scheduling::INSTANCE_BLACKOUT) => SettingName::QuietTimes,
        KeyName::PerInstance(START_EMULATOR) => SettingName::StartEmulator,
        KeyName::PerInstance(idle::ACTION) => SettingName::IdleAction,
        KeyName::Exact(MUMU_ROOT) => SettingName::MumuRoot,
        KeyName::Exact(EVIDENCE_MAX_AGE_DAYS) => SettingName::EvidenceMaxAgeDays,
        KeyName::Exact(PROCESS_MEMORY_PRIORITY) => SettingName::MemoryPriority,
        KeyName::Exact(PROCESS_IO_PRIORITY) => SettingName::IoPriority,
        KeyName::Exact(AUTHORING) => SettingName::LabAuthoring,
        KeyName::Exact(DEBUGGING) => SettingName::LabDebugging,
        KeyName::PerInstance(TOUCH_BACKEND) => SettingName::TouchBackend,
        KeyName::PerInstance(CAPTURE_BACKEND) => SettingName::CaptureBackend,
        KeyName::Exact(other) | KeyName::PerInstance(other) => {
            SettingName::Unnamed(other.to_owned())
        }
    }
}

/// A value of type `value_type` as the page has it: a list or object, reported as JSON text,
/// set or not by its entries; the rest the scalar it is. `None` when it is not of that type
/// (`scalar_json`), JSON text of a list, object or decimal included.
fn shown_value(value_type: ValueType, value: &FactScalar) -> Option<ShownValue> {
    let json = scalar_json(value_type, value)?;
    let entries = match value_type {
        ValueType::List => Some(json.as_array()?.len()),
        ValueType::Object => Some(json.as_object()?.len()),
        ValueType::Decimal if !json.is_number() => return None,
        ValueType::Decimal
        | ValueType::Boolean
        | ValueType::Integer
        | ValueType::DurationMs
        | ValueType::String => None,
    };
    let shown = match (entries, value) {
        (Some(0), _) => ShownValue::Unset,
        (Some(_), _) => ShownValue::Set,
        (None, FactScalar::Boolean(flag)) => ShownValue::Flag(*flag),
        (None, FactScalar::Integer(number)) => ShownValue::Number(*number),
        (None, FactScalar::String(word)) => ShownValue::Word(word.clone()),
        (None, FactScalar::TimestampMs(ms) | FactScalar::DurationMs(ms)) => {
            ShownValue::Word(ms.to_string())
        }
    };
    Some(shown)
}

fn setting_source(source: ConfigParameterSource) -> SettingSource {
    match source {
        ConfigParameterSource::Explicit => SettingSource::File,
        ConfigParameterSource::Default => SettingSource::Default,
        ConfigParameterSource::Discovered => SettingSource::Discovered,
        ConfigParameterSource::Instruction => SettingSource::Online,
    }
}

/// Every `config.parameters` section of the snapshot.
fn settings_read(records: &[RuntimeFactRecord]) -> Option<SettingsRead> {
    let mut sections = records
        .iter()
        .filter(|record| {
            record.scope == RuntimeFactScope::Runtime
                && record.key.starts_with(CONFIG_PARAMETERS_FACT_PREFIX)
        })
        .peekable();
    sections.peek()?;
    let mut read = SettingsRead {
        parameters: BTreeMap::new(),
        unread: Vec::new(),
        quiet: BTreeMap::new(),
        backend_rows: false,
    };
    for record in sections {
        let section = record.key.as_str();
        let unread = |why| UnreadRow { section: section.to_owned(), why };
        let FactValue::RecordList(rows) = &record.value else {
            read.unread.push(unread(Unread::Section));
            continue;
        };
        for row in rows {
            match presets::parameter_of_row(row) {
                Some(parameter) => read.take(section, parameter),
                None => match row.get(ROW_KEY) {
                    Some(FactScalar::String(key)) if shown_tier(key).is_none() => {}
                    Some(FactScalar::String(key)) => {
                        read.unread.push(unread(Unread::Shape(key.clone())));
                    }
                    Some(_) | None => read.unread.push(unread(Unread::Keyless)),
                },
            }
        }
    }
    Some(read)
}

/// Which performance preset is in effect, of the table the Runtime holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetRead {
    table: PresetTable,
    /// `None`: no preset's every member equals what is in effect (`custom`).
    pub current: Option<String>,
    /// Every setting a preset sets, as it is in effect.
    pub members: Vec<PresetMember>,
}

impl PresetRead {
    /// The presets' names, in the table's order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.table.presets().iter().map(|preset| preset.name())
    }

    /// The configuration file holds the learning mode, which no preset can be.
    pub fn file_holds_full(&self) -> bool {
        self.members.iter().any(|member| {
            member.setting == PresetSetting::Mode
                && member.source == SettingSource::File
                && matches!(&member.value, Some(SettingValue::Word(word)) if word == MODE_FULL)
        })
    }

    /// The configuration file names how many tasks run at once.
    pub fn file_fixes_concurrency(&self) -> bool {
        self.members.iter().any(|member| {
            member.setting == PresetSetting::MaxConcurrentRuns
                && member.source == SettingSource::File
        })
    }
}

/// One of the settings a performance preset sets: its value in effect (`None`: it
/// has none) and where that comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetMember {
    pub setting: PresetSetting,
    pub value: Option<SettingValue>,
    pub source: SettingSource,
}

/// `performance::presets::MEMBERS`, without their keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetSetting {
    Mode,
    MaxConcurrentRuns,
    HeavyDispatchLimit,
    ParallelEnabled,
}

/// A setting's value, without the wire's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Flag(bool),
    Number(i64),
    Word(String),
}

/// Where a setting's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingSource {
    /// The configuration file.
    File,
    Default,
    /// Learned at the Runtime's start.
    Discovered,
    /// Changed while the Runtime runs.
    Online,
}

/// Why a snapshot that came back tells no performance preset. It is said once as one
/// sentence on the workbench; these are for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetProblem {
    /// It has no record of that section's settings.
    SectionMissing(&'static str),
    /// That section's record is not a list of settings it can read.
    SectionUnreadable(&'static str),
    /// Those sections have no table of presets.
    TableMissing,
    /// The table does not pass the crate's check: how it says so.
    TableRefused(String),
    /// The crate judged the preset by a setting this file does not know.
    MemberUnknown(&'static str),
}

impl PresetProblem {
    /// For the technical log.
    pub fn line(&self) -> String {
        match self {
            Self::SectionMissing(section) => format!("no record of section {section}"),
            Self::SectionUnreadable(section) => format!("section {section} does not read back"),
            Self::TableMissing => format!("no {} row", presets::PRESETS),
            Self::TableRefused(failure) => format!("{} refused: {failure}", presets::PRESETS),
            Self::MemberUnknown(key) => format!("preset member {key} is not known here"),
        }
    }
}

fn config_read(epoch: String, snapshot: &RuntimeFactSnapshot, version: Option<u64>) -> ConfigRead {
    ConfigRead {
        epoch,
        version,
        preset: preset_read(&snapshot.records),
        servers: servers(&snapshot.records),
        settings: settings_read(&snapshot.records),
        backends: backends::read(&snapshot.records),
    }
}

/// The rows of the two sections the presets' keys are in, the table's row among them,
/// and the crate's own judgement of them, the same `actingctl perf-preset get` makes.
fn preset_read(records: &[RuntimeFactRecord]) -> Result<PresetRead, PresetProblem> {
    let mut parameters = Vec::new();
    for section in [performance::SECTION, scheduling::SECTION] {
        section_parameters(records, section, &mut parameters)?;
    }
    let table = match presets::table_of(&parameters) {
        None => return Err(PresetProblem::TableMissing),
        Some(Err(failure)) => return Err(PresetProblem::TableRefused(format!("{failure:?}"))),
        Some(Ok(table)) => table,
    };
    let judged = presets::current_preset(&table, &parameters);
    let members = judged.members.iter().map(preset_member).collect::<Result<Vec<_>, _>>()?;
    let current = (judged.preset != presets::CUSTOM).then_some(judged.preset);
    Ok(PresetRead { table, current, members })
}

fn section_parameters(
    records: &[RuntimeFactRecord],
    section: &'static str,
    parameters: &mut Vec<ConfigParameter>,
) -> Result<(), PresetProblem> {
    let key = config_parameters_fact_key(section);
    let record = records
        .iter()
        .find(|record| record.scope == RuntimeFactScope::Runtime && record.key == key)
        .ok_or(PresetProblem::SectionMissing(section))?;
    let FactValue::RecordList(rows) = &record.value else {
        return Err(PresetProblem::SectionUnreadable(section));
    };
    for row in rows {
        let parameter = presets::parameter_of_row(row)
            .ok_or(PresetProblem::SectionUnreadable(section))?;
        parameters.push(parameter);
    }
    Ok(())
}

/// The setting of a preset `key` is, if it is one this file knows.
fn preset_setting(key: &str) -> Option<PresetSetting> {
    match key {
        pacing::MODE => Some(PresetSetting::Mode),
        pacing::MAX_CONCURRENT_RUNS => Some(PresetSetting::MaxConcurrentRuns),
        pacing::HEAVY_DISPATCH_LIMIT_NORMAL => Some(PresetSetting::HeavyDispatchLimit),
        scheduling::PARALLEL_ENABLED => Some(PresetSetting::ParallelEnabled),
        _ => None,
    }
}

fn preset_member(member: &CurrentMember) -> Result<PresetMember, PresetProblem> {
    let setting = preset_setting(member.key).ok_or(PresetProblem::MemberUnknown(member.key))?;
    Ok(PresetMember {
        setting,
        value: member.value.as_ref().map(setting_value),
        source: setting_source(member.source),
    })
}

fn setting_value(value: &FactScalar) -> SettingValue {
    match value {
        FactScalar::Boolean(flag) => SettingValue::Flag(*flag),
        FactScalar::Integer(number) => SettingValue::Number(*number),
        FactScalar::String(word) => SettingValue::Word(word.clone()),
        FactScalar::TimestampMs(ms) | FactScalar::DurationMs(ms) => {
            SettingValue::Word(ms.to_string())
        }
    }
}

/// The `task.server` records. A string by contract; any other value is kept as the
/// wire states it, as `fold_task_facts` keeps it, not dropped.
fn servers(records: &[RuntimeFactRecord]) -> BTreeMap<String, String> {
    records
        .iter()
        .filter(|record| record.key == TASK_SERVER)
        .filter_map(|record| {
            let RuntimeFactScope::Instance { instance_id } = &record.scope else {
                return None;
            };
            let server = match &record.value {
                FactValue::String(text) => text.clone(),
                other => code(other),
            };
            Some((code(instance_id), server))
        })
        .collect()
}

fn live_snapshot(pid: u32, epoch: String, status: &RuntimeControlPlaneStatus) -> LiveSnapshot {
    let owner_epoch = status.owner_epoch();
    LiveSnapshot {
        pid,
        epoch,
        global_pause: status.scheduling_pause().map(|pause| global_pause_fact(pause, owner_epoch)),
        instances: status
            .instances()
            .iter()
            .map(|instance| instance_live(instance, owner_epoch))
            .collect(),
        ledger_head: status.source().map(|source| source.ledger_committed_head),
    }
}

fn instance_live(instance: &RuntimeInstanceStatus, owner_epoch: OwnerEpoch) -> InstanceLive {
    let alias = instance.instance_alias();
    let (emulator_on, emulator_off) = match instance.emulator_state() {
        EmulatorRunningState::On { observed_at_unix_ms } => (Some(observed_at_unix_ms), None),
        EmulatorRunningState::Off { observed_at_unix_ms } => (None, Some(observed_at_unix_ms)),
        EmulatorRunningState::Unknown => (None, None),
    };
    let provenance = instance.backend_provenance();
    InstanceLive {
        alias: alias.to_string(),
        instance_id: code(&instance.instance_id()),
        key: InstanceKey(instance.instance_id()),
        game_id: instance.game_id().map(str::to_owned),
        pause: instance.pause().map(|pause| instance_pause_fact(alias, pause, owner_epoch)),
        fence: instance.resource_fence().map(|fence| FenceFact {
            cause: fence.cause.as_str().to_owned(),
            awaiting_person: fence.awaiting_person,
        }),
        ladder_busy: instance.ladder_busy_hold().is_some(),
        activity: instance.activity().map(activity_fact),
        emulator_off,
        emulator_on,
        physical: provenance == Some(ExecutionBackendProvenance::PhysicalDevice),
        emulator_controls: emulator_controls(instance),
        unstated: Unstated {
            provenance: provenance.is_none(),
            capabilities: instance.capabilities().is_none(),
        },
    }
}

/// The emulator's controls the instance's capability profile has available; none without one.
fn emulator_controls(instance: &RuntimeInstanceStatus) -> EmulatorControls {
    let Some(profile) = instance.capabilities() else {
        return EmulatorControls::default();
    };
    let offered = |capability: EmulatorCapability| profile.capability(capability).is_ok();
    EmulatorControls {
        start: offered(EmulatorCapability::InstanceStart),
        stop: offered(EmulatorCapability::InstanceStop),
        restart: offered(EmulatorCapability::InstanceRestart),
    }
}

fn activity_fact(activity: &InstanceActivity) -> ActivityFact {
    ActivityFact {
        kind: match activity.kind {
            InstanceActivityKind::PolicyRun => ActivityKind::PolicyRun,
            InstanceActivityKind::DirectRun => ActivityKind::DirectRun,
            InstanceActivityKind::HostRun => ActivityKind::HostRun,
            InstanceActivityKind::Reconnect => ActivityKind::Reconnect,
            InstanceActivityKind::Selfcheck => ActivityKind::Selfcheck,
            InstanceActivityKind::EmulatorControl => ActivityKind::EmulatorControl,
            InstanceActivityKind::Ladder => ActivityKind::Ladder,
        },
        package_id: activity.package_id.clone(),
        run: activity.request_id.map(RunKey),
    }
}

fn explain_read(epoch: String, explanation: &RuntimeScheduleExplanation) -> ExplainRead {
    ExplainRead {
        epoch,
        evaluated: explanation.evaluated_at_unix_ms.is_some(),
        rows: explanation
            .instances
            .iter()
            .map(|entry| (entry.instance_alias.clone(), instance_explain(entry)))
            .collect(),
    }
}

fn instance_explain(entry: &ScheduleExplanationInstance) -> InstanceExplain {
    let why = &entry.not_running.why;
    InstanceExplain {
        why: why.code().as_str().to_owned(),
        gap: reason_value(why, BENEFIT_GAP),
        hold: reason_value(why, POOL_LINE_HOLD),
        next_wake: entry.not_running.next_wake_unix_ms.or(entry.next_wake_unix_ms),
        preparing: entry.selected.as_ref().is_some_and(|selected| {
            match selected.dispatch_state {
                ScheduleDispatchState::Pending
                | ScheduleDispatchState::Intent
                | ScheduleDispatchState::Admitted => true,
                ScheduleDispatchState::Rejected | ScheduleDispatchState::Completed => false,
            }
        }),
    }
}

/// A reason's value `key` when it is text: only the two that choose words are read.
fn reason_value(why: &Outcome, key: &str) -> Option<String> {
    why.value(key).and_then(|value| value.as_str()).map(str::to_owned)
}

/// The entry of `alias` in an answer asked of that one instance: the client has refused one that
/// names another, so the entry is there exactly when the evaluation decided for it.
fn tasks_read(epoch: String, alias: &str, explanation: &RuntimeScheduleExplanation) -> TasksRead {
    TasksRead {
        epoch,
        alias: alias.to_owned(),
        evaluated_at: explanation.evaluated_at_unix_ms,
        entry: explanation
            .instances
            .iter()
            .find(|entry| entry.instance_alias == alias)
            .map(tasks_entry),
    }
}

fn tasks_entry(entry: &ScheduleExplanationInstance) -> TasksEntry {
    let rows: Vec<TaskRow> = entry
        .candidates
        .iter()
        .map(|row| TaskRow {
            task_id: row.task_id.clone(),
            due: row.eligibility == ScheduleCandidateEligibility::True,
            why: row.why.code().as_str().to_owned(),
            gap: reason_value(&row.why, BENEFIT_GAP),
            hold: reason_value(&row.why, POOL_LINE_HOLD),
            wake: row.next_wake_unix_ms,
        })
        .collect();
    let cut_short =
        u32::try_from(rows.len()).map_or(true, |answered| answered < entry.candidates_total);
    TasksEntry {
        next_wake: entry.next_wake_unix_ms,
        selected: entry.selected.as_ref().map(|selected| SelectedTask {
            task_id: selected.task_id.clone(),
            stage: dispatch_stage(selected.dispatch_state),
        }),
        rows,
        cut_short,
    }
}

fn dispatch_stage(state: ScheduleDispatchState) -> DispatchStage {
    match state {
        ScheduleDispatchState::Pending => DispatchStage::Pending,
        ScheduleDispatchState::Intent => DispatchStage::Intent,
        ScheduleDispatchState::Admitted => DispatchStage::Admitted,
        ScheduleDispatchState::Rejected => DispatchStage::Rejected,
        ScheduleDispatchState::Completed => DispatchStage::Completed,
    }
}
