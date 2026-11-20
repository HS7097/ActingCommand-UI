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
//! epoch that is gone is the caller's. Every connection here, a lane's as a person's,
//! is made through `runtime_connection`: in root mode a Runtime that is not the
//! installation folder's is not connected to at all ([`LiveRead::NotThisRoot`]).
//!
//! Lanes ask only what is answered from memory or takes no ledger record:
//! `Status`, `ScheduleExplanation` of every instance without its candidate rows and, for an
//! account's task list (UI-3f1), of that one instance with them ([`ReadLane::read_tasks`]),
//! `RuntimeFactSnapshot` and `StartReport`; and for that list's switches (UI-3f2), that one
//! instance's `PackScheduleView` ([`ReadLane::read_packs`]), answered from memory once the
//! Runtime has brought its fact store up to the ledger under the fact write gate, a read of the
//! ledger's newest records on its writer's thread that is nearly always empty; and for that
//! account's goals (Workflow #455, #463 V3), its `GoalView` ([`ReadLane::read_goals`]), which
//! records nothing and is computed under the policy's locks and the same gate. The picture's lane
//! ([`preview::PreviewLane`], UI-3c) is the one exception: it reads ledger pages, which take no
//! record either but are read from the ledger, and frames' material, whose read takes no record
//! when it succeeds or
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
//! ([`device::operate`], UI-3e1-a), in the stop's a person's stop of one run of an account
//! ([`stop::stop_run`], UI-3e2), and in the pack's a person's switch of one task package of an
//! account, on or off ([`pack::edit`], UI-3f2), and a person's "close this task" of a run the
//! scheduler dispatched, which reads the account's view, writes the switch's off of the run's
//! package and then stops the run, all on one connection ([`pack::close_then_stop`], UI-3e2b),
//! and in the goals' a person's set or clear of one value of an account's goal page by its name
//! ([`goals::edit`], Workflow #455 G2).
//! Like the conditional resume beside them (`resume_paused`) each goes out once on a fresh
//! person's connection, and never again by itself, whatever comes back, with two exceptions: a
//! stop that answered that the run still stops, or whose answer is not known, is asked once more,
//! on another fresh connection, once a `Status` shows the run gone, so that its reset's result is
//! read (the console's `stop`, not this crate, decides when); and a switch, a close or a goal's
//! write the Runtime refused `fact_observation_not_newer`, which its contract says to send again,
//! is sent once more at once on the same connection.
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
//!
//! What a ledger record of a pause's life says (Runtime 0.12.74, #480 PG-2d) is read here too,
//! as what a pause's receipt says is ([`pause_moment`], beside `pause_answer`), for the ledger
//! window, which finds such records in its own pages: no lane reads them, and nothing on the
//! workbench shows them.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Spelling;
use acui_rows::{code, EventActor, EventSource};
use actingcommand_contract::outcome::{Detail, Outcome};
use actingcommand_contract::{
    config_parameters_fact_key, is_instance_backend_key, ConfigParameter, ConfigParameterSource,
    ConfigSettingChange, EmulatorCapability, EmulatorRunningState, EventType,
    ExecutionBackendProvenance, FactScalar, FactValue, InstanceActivity, InstanceActivityKind,
    InstanceId, InstancePauseStage, OwnerEpoch, PauseObstacle, ProjectedEvent, ProjectionPayload,
    PublicEventPayload, RequestId, RuntimeControlPlaneStatus,
    RuntimeErrorCode, RuntimeFactRecord, RuntimeFactScope, RuntimeFactSnapshot,
    RuntimeInstanceStatus, RuntimeReceipt, RuntimeReceiptState, RuntimeResult,
    RuntimeScheduleExplanation, RuntimeStartReport, ScheduleCandidateEligibility,
    ScheduleDispatchState, ScheduleExplanationInstance, SchedulingPauseMode,
    SchedulingPauseReason, SchedulingPauseScope, CONFIG_PARAMETERS_FACT_PREFIX,
    LEDGER_WRITER_REPLY_WAIT_MS, MIN_SCHEDULING_PAUSE_DRAIN_TIMEOUT_MS,
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
    global_pause_fact, instance_pause_fact, pause_stage, runtime_connection, ClientFailure,
    NoConnection, OnlineSource, PauseFact, PauseStage, RESUME_WITHOUT_BACKEND,
    RESUME_WITHOUT_RECONNECT, RUNTIME_RESULT_UNEXPECTED,
};

pub mod backends;
pub mod device;
pub mod goals;
pub mod pack;
pub mod preview;
pub mod rules;
pub mod stop;

/// The reason this console's "pause all" gives, a free name of the pause, no outcome
/// code. A pause of this reason is one a person pressed here, so lifting it asks for
/// no second press.
pub const CONSOLE_PAUSE_ALL: &str = "console_pause_all";

/// The reason the setup wizard (acsetup) gives the global pause it sets right after a new
/// installation's first start, so that a person looks over every account before anything is
/// played. Another tool's pause like any other, lifted with "put all on duty" and its
/// confirmation; only the words for it are its own.
pub const SETUP_FIRST_START: &str = "first-start";

/// The reason this console's "pause this account" gives, a pause of one instance's scope
/// ([`pause_account`]), likewise a free name and no outcome code, and likewise lifted on the
/// account's card with one press. As a name the Runtime takes it is 21 bytes of `[a-z_]`.
pub const CONSOLE_PAUSE_ACCOUNT: &str = "console_pause_account";

/// The reason of the pause that holds a newly added instance until a person has looked it over
/// and resumes it, the contract's own (Runtime 945727d4
/// `crates/actingcommand-contract/src/runtime.rs:2477`): the Runtime's first start that sees an
/// `instances[]` entry declaring `start_paused` holds that instance with it
/// (`crates/runtime-host/src/host/start_pause.rs:3-9, 110-119`), and the Lab's `instance add`
/// pauses the instance it has just added with it (`crates/lab-rescue/src/rescue.rs:1739-1823`).
/// Another tool's pause like any other in who lifts it and where; only the words for it are its
/// own.
pub use actingcommand_contract::INSTANCE_ADDED_PAUSE_REASON;

/// Whether `reason` is a name the Runtime takes as a pause's reason, by the contract's own
/// check (`SchedulingPauseReason::new`: 1 to 64 bytes of `[a-z0-9_.-]`). The console's
/// settings name reasons it offers no resume for, and one no pause can carry names none.
pub fn pause_reason_valid(reason: &str) -> bool {
    SchedulingPauseReason::new(reason).is_ok()
}

/// The prefix of the reason the Lab's `record start` gives the pause it sets on the instance it
/// records (Runtime 0.12.64, #463 S1-7; Runtime `crates/lab/src/recording/hold.rs`, `reason_for`),
/// followed by the start of the recording that set it, in Unix milliseconds, in decimal. The
/// recording's end (`record stop`, `record archive`) lifts the pause of exactly that reason.
const LAB_RECORD_PREFIX: &str = "lab-record-";

/// The milliseconds of a reason of exactly that shape, the prefix and then digits only, that fit an
/// `i64`, as the clock an age is told by does (`acui_rows::seconds_since`): when the recording
/// took the pause. Any other reason, another `lab-` one among them, has none.
pub fn lab_record_started(reason: &str) -> Option<u64> {
    let ms = reason.strip_prefix(LAB_RECORD_PREFIX)?;
    // `parse` alone would take a leading `+`.
    if !ms.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    ms.parse::<u64>().ok().filter(|ms| i64::try_from(*ms).is_ok())
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
/// lane of its own. Made through `runtime_connection`, as every connection is: in
/// root mode a Runtime not taken for the installation folder's is not connected to,
/// and nothing is read.
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
            Err(not) => return live_failure(not),
        };
        let info = client.runtime_info();
        let (pid, epoch) = (info.pid(), code(&info.owner_epoch()));
        match client.status() {
            Ok(status) => LiveRead::Read(Box::new(live_snapshot(pid, epoch, &status))),
            Err(error) => live_failure(NoConnection::Failed(self.settled(error))),
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

    /// One `PackScheduleView` of the one instance `alias`: that account's task switches (UI-3f2).
    /// A Runtime that has none says so as its answer ([`pack::PacksAnswer::Unsupported`]), not
    /// as a failure: one before 0.12.30, which drops the connection on it, costs this lane its
    /// connection, as any fatal failure does.
    pub fn read_packs(&mut self, alias: &str) -> Result<pack::PacksRead, ClientFailure> {
        self.asked_once_more(|lane| lane.ask_packs(alias))
    }

    /// One `GoalView` of the one instance `alias`: that account's goal page (Workflow #455, Workflow
    /// #463 V3), kept whole until it is shown as its lines ([`goals::page`]). A Runtime
    /// that has no such read says so as its answer ([`goals::GoalsAnswer::Unsupported`]), as for the
    /// switches, and one that does not know the request costs this lane its connection, as any
    /// fatal failure does.
    pub fn read_goals(&mut self, alias: &str) -> Result<goals::GoalsRead, ClientFailure> {
        self.asked_once_more(|lane| lane.ask_goals(alias))
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
        mut ask: impl FnMut(&mut Self) -> Result<T, NoConnection>,
    ) -> Result<T, ClientFailure> {
        let kept = self.client.is_some();
        let asked = match ask(self) {
            Err(NoConnection::Failed(error)) if kept && connection_ended(&error) => ask(self),
            asked => asked,
        };
        asked.map_err(ClientFailure::from)
    }

    fn ask_explain(&mut self) -> Result<ExplainRead, NoConnection> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.schedule_explanation(None, false) {
            Ok(explanation) => Ok(explain_read(epoch, &explanation)),
            Err(error) => Err(NoConnection::Failed(self.settled(error))),
        }
    }

    fn ask_tasks(&mut self, alias: &str) -> Result<TasksRead, NoConnection> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.schedule_explanation(Some(alias), true) {
            Ok(explanation) => Ok(tasks_read(epoch, alias, &explanation)),
            Err(error) => Err(NoConnection::Failed(self.settled(error))),
        }
    }

    fn ask_packs(&mut self, alias: &str) -> Result<pack::PacksRead, NoConnection> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.pack_schedule_view(alias) {
            Ok(view) => Ok(pack::packs_read(epoch, alias, &view)),
            Err(error) => match pack::view_unsupported(&error) {
                Some(code) => {
                    self.settled(error);
                    Ok(pack::packs_unsupported(epoch, alias, code))
                }
                None => Err(NoConnection::Failed(self.settled(error))),
            },
        }
    }

    fn ask_goals(&mut self, alias: &str) -> Result<goals::GoalsRead, NoConnection> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.goal_view(alias) {
            Ok(view) => Ok(goals::goals_read(epoch, alias, view)),
            Err(error) => match pack::view_unsupported(&error) {
                Some(code) => {
                    self.settled(error);
                    Ok(goals::goals_unsupported(epoch, alias, code))
                }
                None => Err(NoConnection::Failed(self.settled(error))),
            },
        }
    }

    fn ask_config(&mut self) -> Result<ConfigRead, NoConnection> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.runtime_fact_snapshot_versioned() {
            Ok((snapshot, version)) => Ok(config_read(epoch, &snapshot, version)),
            Err(error) => Err(NoConnection::Failed(self.settled(error))),
        }
    }

    fn ask_start(&mut self) -> Result<StartRead, NoConnection> {
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.start_report() {
            Ok(report) => Ok(start_read(epoch, &report)),
            Err(error) => Err(NoConnection::Failed(self.settled(error))),
        }
    }

    /// The connection to ask on, made first when there is none (`runtime_connection`).
    fn connection(&mut self) -> Result<RuntimeClient, NoConnection> {
        if let Some(client) = &self.client {
            return Ok(client.clone());
        }
        let client = runtime_connection(
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
    /// Root mode only: the Runtime the state root names is not the installation
    /// folder's, or cannot be told to be, so nothing was connected to it and nothing
    /// read (`CONSOLE_RUNTIME_NOT_THIS_ROOT`). One whose process is gone is
    /// `NotConnected`, with the console's own `CONSOLE_RUNTIME_GONE`.
    NotThisRoot(ClientFailure),
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

fn live_failure(not: NoConnection) -> LiveRead {
    if let NoConnection::NotThisRoot = not {
        return LiveRead::NotThisRoot(ClientFailure::from(not));
    }
    match Unreached::of(not) {
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
    /// not carry that, so this looks at the client's error. In root mode a Runtime the
    /// console did not connect to (`runtime_connection`) is `NotConnected`, with the
    /// console's own code: nothing of it is read.
    fn of(not: NoConnection) -> Self {
        let error = match not {
            NoConnection::Failed(error) => error,
            not => return Self::NotConnected(ClientFailure::from(not)),
        };
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
/// one account ([`pause_account`]). Both are a person's pause, `stand` (#480 PG-2c, Runtime
/// 0.12.73; `contracts/scheduling-pause.md` 197-281 at 1067dfaf): once its row is recorded the
/// Runtime never lifts it by itself, not for something in the way, a shutdown or a failure of its
/// own; only a person's resume does. What came of an instance pause's stages is the cause its
/// completed receipt carries under its success code ([`pause_answer`]); a global pause has no
/// stages, and its receipt carries none (runtime-host `host/policy_dispatch.rs` 3774-3808).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseOutcome {
    /// The pause holds, at this revision of its scope: an instance's with its device handed back
    /// within its caller's time, or the global one. `cause`: the code of a cause under the success
    /// code that this console does not read, for the log only; 0.12.74 gives none such.
    Applied { revision: u64, cause: Option<String> },
    /// An instance pause holds at this revision, its gate closed, and was still on its way at the
    /// end of its caller's time (`scheduling_pause_standing`): the Runtime carries its stages on by
    /// itself, until its device is handed back or a person resumes it (`scheduling-pause.md`
    /// 213-238). No failure. `stage`: where it stood then, draining or handing its device back;
    /// `in_the_way`: what was in the way at that read, empty when nothing was and the hand-back had
    /// not finished (222-225). Either is `None` when the cause carried none this build reads.
    Standing { revision: u64, stage: Option<PauseStage>, in_the_way: Option<Vec<Obstacle>> },
    /// A person's resume lifted the pause while its own stages still ran
    /// (`scheduling_pause_lifted_by_resume`, 270-274): it set the pause at this revision, and no
    /// pause of it holds now. No failure. `stage`: where it stood then.
    LiftedByResume { revision: u64, stage: Option<PauseStage> },
    /// The Runtime shut down before the device was handed back
    /// (`scheduling_pause_kept_at_shutdown`, 264-269): the pause holds at this revision until the
    /// Runtime stops, and the next start restores it. No failure. `stage`: where it stood then.
    KeptAtShutdown { revision: u64, stage: Option<PauseStage> },
    /// The Runtime denied it (`Denied`), or answered that the record of its pauses failed
    /// (`scheduling_pause_persist_failed`), and set nothing: what can refuse a pause for its own
    /// input is checked before its row is recorded (`scheduling-pause.md` 260-263, 702-709), and a
    /// refused record leaves the gate as it was (runtime-host `host/scheduling_pause.rs` 102-119,
    /// 164-173). Its code is the host's, in `host`: `runtime_code` is only the receipt's class. One
    /// pause already holding is `scheduling_already_paused`, and keeps its own reason.
    Refused(ClientFailure),
    /// The Runtime answered that it failed (`Failed`): a fatal failure, or one that fenced the
    /// instance, in its stages, which lifts nothing of a person's pause: its gate stays closed and
    /// its row names it, so the next start restores it (`scheduling-pause.md` 275-281;
    /// `policy_dispatch.rs` 3911-3946). A fatal failure before the row was recorded leaves none,
    /// and the Runtime latched (3767-3852): what holds is the status's to say. Its code, as
    /// `Refused`'s.
    Failed(ClientFailure),
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it. It is not sent again.
    Unconfirmed(ClientFailure),
}

/// What was in the way of an instance pause's stages when its caller's time ended, as its
/// `scheduling_pause_standing` named it, and, since Runtime 0.12.75 (#480 PG-2e), what the latest
/// check of anyone's instance pause read in its way, as the status answers it
/// ([`PauseFact::in_the_way`]): the contract's closed list `pause_obstacle`, ten words
/// (`PauseObstacle`; contract `runtime.rs` 2323-2376 at 945727d4), each in the list once, in that
/// order (runtime-host `host/policy_dispatch.rs` 1429-1452 at 945727d4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obstacle {
    /// A run of the instance still in flight; past the drain it was asked to stop at its next
    /// checkpoint.
    ContainedRun,
    /// A policy run the instance's worker still holds, until its settlement.
    PolicyRun,
    /// Another party's lease holds the instance: a client's, a direct run, an operator's control, a
    /// Lab operation, a self-check.
    Lease,
    /// The Runtime's own automatic emulator start.
    AutomaticStart,
    /// The Runtime's own stuck-recovery ladder.
    RecoveryLadder,
    /// An entry queued for the instance that the pause does not hold back.
    QueuedEntry,
    /// A client owes the instance its reset after a run of its ended cancelled.
    ClientReset,
    /// The lease of the device session's close was refused; the close is tried again.
    CloseRefused,
    /// The session's close could not be confirmed and fenced the instance; it is not tried again,
    /// and only a person's resume connects the instance again. Only a person's pause (`stand`)
    /// stands on it; a tool's hold fails there (`policy_dispatch.rs` 1335-1339, 4899-4921).
    CloseUnconfirmed,
    /// The last check of the stages failed, not fatally; it is tried again.
    CheckFailed,
}

impl Obstacle {
    /// The contract's word as this crate's: a receipt's, a record's and the status's alike.
    pub(crate) const fn of(word: PauseObstacle) -> Self {
        match word {
            PauseObstacle::ContainedRun => Self::ContainedRun,
            PauseObstacle::PolicyRun => Self::PolicyRun,
            PauseObstacle::Lease => Self::Lease,
            PauseObstacle::AutomaticStart => Self::AutomaticStart,
            PauseObstacle::RecoveryLadder => Self::RecoveryLadder,
            PauseObstacle::QueuedEntry => Self::QueuedEntry,
            PauseObstacle::ClientReset => Self::ClientReset,
            PauseObstacle::CloseRefused => Self::CloseRefused,
            PauseObstacle::CloseUnconfirmed => Self::CloseUnconfirmed,
            PauseObstacle::CheckFailed => Self::CheckFailed,
        }
    }

    const fn word(self) -> PauseObstacle {
        match self {
            Self::ContainedRun => PauseObstacle::ContainedRun,
            Self::PolicyRun => PauseObstacle::PolicyRun,
            Self::Lease => PauseObstacle::Lease,
            Self::AutomaticStart => PauseObstacle::AutomaticStart,
            Self::RecoveryLadder => PauseObstacle::RecoveryLadder,
            Self::QueuedEntry => PauseObstacle::QueuedEntry,
            Self::ClientReset => PauseObstacle::ClientReset,
            Self::CloseRefused => PauseObstacle::CloseRefused,
            Self::CloseUnconfirmed => PauseObstacle::CloseUnconfirmed,
            Self::CheckFailed => PauseObstacle::CheckFailed,
        }
    }

    /// Its word, as the wire spells it: for the log.
    pub const fn as_str(self) -> &'static str {
        self.word().as_str()
    }
}

/// The causes a person's pause's completed receipt carries under its success code
/// (`caused_by`), each informing, the host's own spellings, of a crate this one does not use
/// (runtime-host `codes.rs` 1705-1715 at 1067dfaf), and the keys of their values (contract
/// `outcome/keys.rs` 420-421). The record of a pause's life carries two of them on top as well
/// ([`PauseMoment`]).
const PAUSE_STANDING: &str = "scheduling_pause_standing";
const PAUSE_LIFTED_BY_RESUME: &str = "scheduling_pause_lifted_by_resume";
const PAUSE_KEPT_AT_SHUTDOWN: &str = "scheduling_pause_kept_at_shutdown";
const PAUSE_STAGE: &str = "pause_stage";
const IN_THE_WAY: &str = "in_the_way";
/// The failed record of a pause's rows, the host's own spelling (runtime-host `codes.rs` 1393):
/// the gate is left as it was.
const PAUSE_PERSIST_FAILED: &str = "scheduling_pause_persist_failed";

/// What a person's pause answered `Completed` with says of it: `revision`, its result's, and
/// `outcome`, its receipt's, whose first cause under the success code tells, read as the Lab reads
/// it (Runtime `crates/lab-ops/src/scheduling.rs` 186-205, `hold_not_quiet`, at 1067dfaf): its code
/// by its spelling, `pause_stage` as an `InstancePauseStage` and `in_the_way` as a list of
/// `PauseObstacle`, both as the receipt carries them (`RuntimeReceipt::scheduling_paused`, contract
/// `runtime.rs` 6141-6165; runtime-host `host/policy_dispatch.rs` 1429-1456, `stages_cause`). No
/// cause: it holds. A cause of another code: it holds as well, the code kept for the log. An
/// absent `in_the_way` is an empty one (`contracts/scheduling-pause.md` 222-225).
fn pause_answer(revision: u64, outcome: &Outcome) -> PauseOutcome {
    let Some(cause) = outcome.causes().first().map(|link| link.outcome()) else {
        return PauseOutcome::Applied { revision, cause: None };
    };
    match cause.code().as_str() {
        PAUSE_STANDING => PauseOutcome::Standing {
            revision,
            stage: stage_value(cause),
            in_the_way: in_the_way_value(cause),
        },
        PAUSE_LIFTED_BY_RESUME => {
            PauseOutcome::LiftedByResume { revision, stage: stage_value(cause) }
        }
        PAUSE_KEPT_AT_SHUTDOWN => {
            PauseOutcome::KeptAtShutdown { revision, stage: stage_value(cause) }
        }
        other => PauseOutcome::Applied { revision, cause: Some(other.to_owned()) },
    }
}

/// The `pause_stage` an outcome of a pause carries, as an `InstancePauseStage`; `None` without
/// one, or with a word this build does not read.
fn stage_value(outcome: &Outcome) -> Option<PauseStage> {
    outcome
        .value(PAUSE_STAGE)
        .and_then(|stage| serde_json::from_value::<InstancePauseStage>(stage.clone()).ok())
        .map(pause_stage)
}

/// The `in_the_way` an outcome of a pause carries, as a list of `PauseObstacle`: an absent one is
/// an empty one (`contracts/scheduling-pause.md` 222-225); `None` with a word this build does not
/// read.
fn in_the_way_value(outcome: &Outcome) -> Option<Vec<Obstacle>> {
    match outcome.value(IN_THE_WAY) {
        None => Some(Vec::new()),
        Some(words) => serde_json::from_value::<Vec<PauseObstacle>>(words.clone())
            .ok()
            .map(|words| words.into_iter().map(Obstacle::of).collect()),
    }
}

/// One moment of an instance pause's life that the Runtime records in its ledger, so that a second
/// console, the Lab or this one after a restart can tell what happened to a pause (#480 PG-2d,
/// Runtime 0.12.74; `contracts/scheduling-pause.md` 711-760 at 1067dfaf): a
/// `runtime.lifecycle_observed` record of the phase `device_state_observed`, under the instance and
/// the links of the pause's own request, whose outcome's code says which and whose severity is that
/// code's category (runtime-host `host/policy_dispatch.rs` 4002-4031, `record_pause_moment`). The
/// ledger window reads it, online and offline, in the public projection (`ProjectionProfile::Ui`),
/// which keeps of that outcome every node's code, category and relation, and of its values only
/// the registered names, `pause_stage` and `in_the_way` but not `pause_reason` (contract
/// `outcome.rs` 1278-1308, `outcome/keys.rs` 218 and 420-421, `event/payload.rs` 14127-14130).
/// Nothing on the workbench reads it: what a pause is doing now is the status's `stage`
/// (`contracts/scheduling-pause.md` 797-804).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseMoment {
    /// `scheduling_pause_standing` (info): a person's pause answered at the end of its caller's
    /// time with something still in the way, the record of its receipt's own cause
    /// (`policy_dispatch.rs` 1429-1456, 3954-3972): where it stood, what was in the way, and, for
    /// `check_failed`, the code of the failed check, its cause. The Runtime carries its stages on,
    /// unless only an unconfirmed close stood in the way: that fences the instance and is not
    /// tried again, and only a person's resume reconnects it (1309-1311, 3973-3989, 4833-4848).
    /// `stage` or `in_the_way` is `None` when the record carries a word this build does not read.
    Standing { stage: Option<PauseStage>, in_the_way: Option<Vec<Obstacle>>, cause: Option<String> },
    /// `scheduling_pause_settled` (info, no member): that pause later became quiet by itself, its
    /// tasks stopped and its device handed back; the pause still holds (4326-4340).
    Settled,
    /// `scheduling_resume_without_reconnect` (info, no member): a person's resume lifted the pause
    /// while it still drained or handed its device back, and nothing reconnected. Recorded under
    /// the pause request's links, not the resume's (5281-5282, 5316-5326;
    /// `contracts/scheduling-pause.md` 718-720, 727, 762-764).
    ResumedWithoutReconnect,
    /// `scheduling_pause_kept_at_shutdown` (info): the Runtime stopped before the pause became
    /// quiet, and keeps it until its next start, which restores it: where it stood (1434-1435,
    /// 3963-3971, 4335).
    KeptAtShutdown { stage: Option<PauseStage> },
    /// `scheduling_pause_lifted_not_quiet` (warning): a tool's hold (`quiet_or_lift`) was lifted
    /// without its instance having become quiet, so it held nothing: what was in the way (perhaps
    /// nothing read, when another party held the instance's admission guard), and the code of the
    /// failure that ended it, its cause (3889-3910). Which tool's hold it was, its `pause_reason`,
    /// the projection leaves out.
    LiftedNotQuiet { in_the_way: Option<Vec<Obstacle>>, cause: Option<String> },
    /// `scheduling_resume_without_backend` (info, no member, Workflow #479 F-2): the pause of an
    /// iOS handset was lifted, by a person's resume or a tool's, and nothing connected, this
    /// Runtime having no backend for its device system (5407-5420;
    /// `contracts/device-binding.md` 535-543).
    ResumedWithoutBackend,
}

impl PauseMoment {
    /// The record's code, as the wire spells it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Standing { .. } => PAUSE_STANDING,
            Self::Settled => PAUSE_SETTLED,
            Self::ResumedWithoutReconnect => RESUME_WITHOUT_RECONNECT,
            Self::KeptAtShutdown { .. } => PAUSE_KEPT_AT_SHUTDOWN,
            Self::LiftedNotQuiet { .. } => PAUSE_LIFTED_NOT_QUIET,
            Self::ResumedWithoutBackend => RESUME_WITHOUT_BACKEND,
        }
    }
}

/// The two codes of a pause's life only its records carry, never a receipt, the host's own
/// spellings (runtime-host `codes.rs` 1728 and 1735 at 1067dfaf).
const PAUSE_SETTLED: &str = "scheduling_pause_settled";
const PAUSE_LIFTED_NOT_QUIET: &str = "scheduling_pause_lifted_not_quiet";

/// The moment of a pause's life `event` records ([`PauseMoment`]), read as `stop_marked` reads a
/// record's outcome: a `runtime.lifecycle_observed` record of the Runtime's family whose public
/// projection's outcome has one of the six codes on top. Any other record is none, another code of
/// `device_state_observed` among them (the emulator watch's `emulator_autostart_skipped`, the idle
/// check's `instance_idle_action`), and shows as before.
pub fn pause_moment(event: &ProjectedEvent) -> Option<PauseMoment> {
    if event.event_type != EventType::RuntimeLifecycleObserved {
        return None;
    }
    let ProjectionPayload::Public(payload) = &event.payload else {
        return None;
    };
    let PublicEventPayload::Runtime(payload) = payload.as_ref() else {
        return None;
    };
    moment_of(payload.outcome()?)
}

/// What a record's `outcome` says of a pause's life, by its top code's spelling, its values read as
/// a pause's receipt's cause is (`pause_answer`): `None` for any other code.
fn moment_of(outcome: &Outcome) -> Option<PauseMoment> {
    let cause = || outcome.causes().first().map(|link| link.outcome().code().as_str().to_owned());
    Some(match outcome.code().as_str() {
        PAUSE_STANDING => PauseMoment::Standing {
            stage: stage_value(outcome),
            in_the_way: in_the_way_value(outcome),
            cause: cause(),
        },
        PAUSE_SETTLED => PauseMoment::Settled,
        RESUME_WITHOUT_RECONNECT => PauseMoment::ResumedWithoutReconnect,
        PAUSE_KEPT_AT_SHUTDOWN => PauseMoment::KeptAtShutdown { stage: stage_value(outcome) },
        PAUSE_LIFTED_NOT_QUIET => {
            PauseMoment::LiftedNotQuiet { in_the_way: in_the_way_value(outcome), cause: cause() }
        }
        RESUME_WITHOUT_BACKEND => PauseMoment::ResumedWithoutBackend,
        _ => return None,
    })
}

/// What a pause's failure says of it, by the Runtime's own receipt of it the client kept
/// ([`RuntimeClientError::received_receipt`]), as the Lab tells its own apart (Runtime
/// `crates/lab-ops/src/scheduling.rs` 207-234, `failed_pause`, at 1067dfaf).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailedPause {
    /// Nothing was set ([`PauseOutcome::Refused`]).
    Refused,
    /// A person's pause failed in its stages, which lifts nothing ([`PauseOutcome::Failed`]).
    Failed,
    /// No receipt of the Runtime's: whether it was made is not known.
    Unanswered,
}

/// The receipt's `state` and the code `code` its outcome has on top: `Denied` set nothing, and so
/// did a `Failed` whose code is the refused record of the pauses; any other `Failed` lifted
/// nothing; no receipt says nothing.
fn failed_pause(state: Option<RuntimeReceiptState>, code: Option<&str>) -> FailedPause {
    match state {
        Some(RuntimeReceiptState::Denied) => FailedPause::Refused,
        Some(_) if code == Some(PAUSE_PERSIST_FAILED) => FailedPause::Refused,
        Some(_) => FailedPause::Failed,
        None => FailedPause::Unanswered,
    }
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
/// Runtime answers once the device is handed back or, at the latest, at the end of its caller's
/// time, that timeout and the checkpoint grace (1 s and 30 s; runtime-host
/// `host/contained_task.rs` 1383-1393 at 1067dfaf), with what was still in the way
/// ([`PauseOutcome::Standing`]); the client waits for the receipt that long and its io timeout
/// besides (runtime-client `client.rs` 7041-7057). A fixture instance's is a refusal
/// (`fixture_execution_scope_forbidden`), and so is an alias the Runtime does not know.
pub fn pause_account(root: &Path, instance_alias: &str) -> PauseOutcome {
    let scope = SchedulingPauseScope::Instance { instance_alias: instance_alias.to_owned() };
    pause(root, scope, CONSOLE_PAUSE_ACCOUNT)
}

/// One `PauseScheduling` of `scope` with `reason`, for both of the above: a person's pause,
/// `stand`, sent with the receipt's outcome kept (`pause_scheduling_as`, runtime-client
/// `client.rs` 1844-1884 at 1067dfaf; `pause_scheduling` sends the same mode and lets the outcome
/// go). A Runtime before 0.12.73 (`runtime-client` 11 or lower) cannot decode the request and
/// ends the connection without a receipt (1850-1851): unconfirmed.
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
    let answer = client.pause_scheduling_as(
        scope,
        reason,
        MIN_SCHEDULING_PAUSE_DRAIN_TIMEOUT_MS,
        SchedulingPauseMode::Stand,
    );
    match answer {
        Ok(answer) => match answer.result {
            RuntimeResult::SchedulingPaused { revision, .. } => {
                pause_answer(revision, &answer.outcome)
            }
            // The client checks the result itself and gives back no other.
            _ => PauseOutcome::Unconfirmed(ClientFailure {
                code: RUNTIME_RESULT_UNEXPECTED,
                operation: PAUSE_SCHEDULING,
                runtime_code: None,
                host: None,
            }),
        },
        Err(error) => {
            let failed = failed_pause(
                error.received_receipt().map(RuntimeReceipt::state),
                error.receipt_outcome().map(|outcome| outcome.code().as_str()),
            );
            let failure = ClientFailure::from(error);
            match failed {
                FailedPause::Refused => PauseOutcome::Refused(failure),
                FailedPause::Failed => PauseOutcome::Failed(failure),
                FailedPause::Unanswered => PauseOutcome::Unconfirmed(failure),
            }
        }
    }
}

/// The longest the Runtime takes, once it has one person's pause of one account
/// ([`pause_account`]), to show it in `Status` (runtime-host `host/policy_dispatch.rs` 3809-3862,
/// at 1067dfaf, the Runtime this console pins). Its deadlines are taken first (3813); then it
/// records the pauses, which the ledger's writer answers within `LEDGER_WRITER_REPLY_WAIT_MS`
/// (10 s; contract `runtime.rs` 154-159; the record at 3844), and sets the pause, stage
/// `draining`, in the table `Status` reads right after (3848-3854), before any of its stages runs.
/// From then on a person's pause is never lifted by the Runtime, a failed stage's included
/// (3911-3946; `contracts/scheduling-pause.md` 197-209): only a person's resume lifts it. So this
/// is all there is to wait for: a pause a read showed and that is gone again was lifted by a
/// resume ([`PauseVerdict::LiftedLater`]), and one no read showed did not hold, its request lost
/// (a Runtime before 0.12.73 drops it undecoded, runtime-client `client.rs` 1850-1851). The
/// locks it waits for on the way have no bound in the source, so this bounds no pause: only one
/// no read showed is taken as not held, and not before this long after its press
/// ([`pause_verdict`]).
pub const ACCOUNT_PAUSE_SETTLES: Duration = Duration::from_millis(LEDGER_WRITER_REPLY_WAIT_MS);

/// The same of one person's global pause ([`pause_global`]): it drains nothing, and holds once
/// the pauses are recorded (`policy_dispatch.rs` 3774-3808), within `LEDGER_WRITER_REPLY_WAIT_MS`
/// (10 s). Nothing lifts it again but a resume.
pub const GLOBAL_PAUSE_SETTLES: Duration = Duration::from_millis(LEDGER_WRITER_REPLY_WAIT_MS);

/// How many `Status` reads landed after the answer that said nothing are watched, at least, before
/// a pause none of them showed is taken as not held ([`pause_verdict`]). Counted from that answer,
/// not from the press: the client waits for a pause's receipt up to the drain timeout, the
/// checkpoint grace and its io timeout (1 s, 30 s and the default 5 s; runtime-client `client.rs`
/// 1859-1872, 7041-7057, 90 and 185, at 1067dfaf), longer than a pause takes to show, so an answer
/// that said nothing may land past [`GLOBAL_PAUSE_SETTLES`] and [`ACCOUNT_PAUSE_SETTLES`], with the
/// pause not yet recorded. Each read counted was begun after that answer (the workbench lets go of
/// a read begun before a write's answer), so three of them are three looks at the Runtime after
/// it, past either time.
pub const PAUSE_WATCH_READS: u32 = 3;

/// The longest a pause whose answer said nothing is watched while the reads show it on its way,
/// draining or handing its device back ([`PauseSeen::Staging`]): ten minutes from the press. A
/// person's pause that a read shows on its way is moved on by the Runtime only to held, its device
/// handed back, or not at all until a person resumes it: past its caller's time the host carries
/// its stages on with no bound (`policy_dispatch.rs` 1266-1296; `contracts/scheduling-pause.md`
/// 225-238), and a close it could not confirm leaves it handing its device back, not tried again
/// (1309-1320). One has been seen lifted 107 s after its press (UI-PF1), on a Runtime before
/// 0.12.73, which still lifted a pause whose stage failed. A pause still on its way at ten minutes
/// is past anything the console has seen, and it is no longer said to be watched, nor confirmed,
/// but left to what the card shows.
pub const PAUSE_WATCH_LIMIT: Duration = Duration::from_secs(10 * 60);

/// What one `Status` shows of the scope of a pause whose answer said nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseSeen {
    /// No pause holds there.
    Absent,
    /// An instance pause still draining its runs or handing its device back
    /// ([`PauseStage::Draining`], [`PauseStage::Paused`]): the Runtime carries it on to held by
    /// itself, and nothing but a person's resume lifts it.
    Staging,
    /// A pause that holds: an instance pause with its device handed back
    /// ([`PauseStage::Released`]), or the global pause, which has no stage.
    Holds,
    /// A pause another party set there, with a reason other than the one pressed: never this
    /// press's, whatever its stage.
    Other,
}

impl PauseSeen {
    /// What `pause`, the pause of the scope as a `Status` reported it, shows of the pause pressed
    /// with `reason` ([`CONSOLE_PAUSE_ALL`], [`CONSOLE_PAUSE_ACCOUNT`]).
    pub fn of(pause: Option<&PauseFact>, reason: &str) -> Self {
        match pause {
            None => Self::Absent,
            Some(pause) if pause.reason != reason => Self::Other,
            Some(pause) => match pause.stage {
                Some(PauseStage::Draining | PauseStage::Paused) => Self::Staging,
                Some(PauseStage::Released) | None => Self::Holds,
            },
        }
    }
}

/// What the `Status` reads after a pause whose answer said nothing come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseVerdict {
    /// Nothing yet: the next read is to be watched.
    Watching,
    /// The pause holds: confirmed late.
    Held,
    /// It does not hold: no read showed it, so it was never recorded.
    NotHeld,
    /// It held, a read having shown it, and is gone: a person's resume lifted it since, as nothing
    /// else lifts a person's pause (Runtime 0.12.73). No failure.
    LiftedLater,
    /// Still on its way at [`PAUSE_WATCH_LIMIT`]: neither held nor not, and left to the card.
    StillStaging,
    /// Another party's pause holds there: the press is not its, and the card says whose it is.
    Overtaken,
}

/// The verdict of the reads after a pause whose answer said nothing, the newest showing `seen` of
/// its scope: `shown` is whether an earlier one showed a pause there, `reads` how many reads have
/// landed since that answer, the newest included, `watched` how long it is since the press, and
/// `settles` how long the Runtime takes to show such a pause ([`ACCOUNT_PAUSE_SETTLES`],
/// [`GLOBAL_PAUSE_SETTLES`]). The stage says it, not a time:
///
/// - a pause that holds (handed back, or the global one) holds, confirmed late;
/// - one on its way (draining or handing its device back) gets no verdict, the Runtime moving it
///   on by itself, until [`PAUSE_WATCH_LIMIT`] from the press, when it is still on its way;
/// - none, after a read showed one, held and was lifted later by a person's resume;
/// - none, and none shown yet, did not hold only once both [`PAUSE_WATCH_READS`] reads have
///   landed since the answer and `settles` has passed since the press, the later of the two, or
///   at [`PAUSE_WATCH_LIMIT`] from the press however few have;
/// - another party's pause there overtakes the press: it is left to the card, which names it.
///
/// It is the verdict of every read, not of the first only: a later read that shows otherwise
/// gives another, which the caller says instead.
pub fn pause_verdict(
    seen: PauseSeen,
    shown: bool,
    reads: u32,
    watched: Duration,
    settles: Duration,
) -> PauseVerdict {
    match seen {
        PauseSeen::Other => PauseVerdict::Overtaken,
        PauseSeen::Holds => PauseVerdict::Held,
        PauseSeen::Staging if watched >= PAUSE_WATCH_LIMIT => PauseVerdict::StillStaging,
        PauseSeen::Staging => PauseVerdict::Watching,
        PauseSeen::Absent if shown => PauseVerdict::LiftedLater,
        // At the end of the watch, however few reads have landed: the watch ends on a verdict.
        PauseSeen::Absent if watched >= PAUSE_WATCH_LIMIT => PauseVerdict::NotHeld,
        PauseSeen::Absent if reads >= PAUSE_WATCH_READS && watched >= settles => {
            PauseVerdict::NotHeld
        }
        PauseSeen::Absent => PauseVerdict::Watching,
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
    let connected = runtime_connection(configure_config(root));
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actingcommand_contract::codes::ReceiptCode;
    use actingcommand_contract::outcome::{Code, IoOp, Outcome};
    use actingcommand_contract::{InstancePauseStage, PauseObstacle, RuntimeReceiptState};

    use super::{
        failed_pause, lab_record_started, moment_of, pause_answer, pause_verdict, FailedPause,
        Obstacle, PauseMoment, PauseOutcome, PauseSeen, PauseStage, PauseVerdict,
        ACCOUNT_PAUSE_SETTLES, GLOBAL_PAUSE_SETTLES, PAUSE_WATCH_LIMIT, PAUSE_WATCH_READS,
    };

    /// How long the Runtime takes to show a pause, here: an account's.
    const SETTLES: Duration = ACCOUNT_PAUSE_SETTLES;
    const SECOND: Duration = Duration::from_secs(1);
    const REVISION: u64 = 7;

    /// A code of the host's, which this crate does not use, by its spelling in the catalog the
    /// pinned contract crate embeds.
    fn host(spelling: &'static str) -> Code {
        Code::relayed(spelling).unwrap_or_else(|| panic!("the pinned catalog has no {spelling}"))
    }

    /// A completed pause's receipt outcome as the Runtime gives it
    /// (`RuntimeReceipt::scheduling_paused`): the success code, caused by `cause` when there is one.
    fn completed(cause: Option<Outcome>) -> Outcome {
        let success = Outcome::new(ReceiptCode::PauseSchedulingCompleted);
        match cause {
            Some(cause) => success.caused_by(cause),
            None => success,
        }
    }

    /// `scheduling_pause_standing` as the host builds it (`stages_cause`).
    fn standing(stage: InstancePauseStage, in_the_way: &[PauseObstacle]) -> Outcome {
        Outcome::new(host("scheduling_pause_standing"))
            .pause_stage(stage)
            .in_the_way(in_the_way.iter().copied())
    }

    #[test]
    fn a_pause_without_a_cause_holds() {
        assert_eq!(
            pause_answer(REVISION, &completed(None)),
            PauseOutcome::Applied { revision: REVISION, cause: None }
        );
    }

    #[test]
    fn a_standing_pause_says_its_stage_and_what_is_in_the_way() {
        let cause = standing(
            InstancePauseStage::Draining,
            &[PauseObstacle::ContainedRun, PauseObstacle::Lease],
        );
        assert_eq!(
            pause_answer(REVISION, &completed(Some(cause))),
            PauseOutcome::Standing {
                revision: REVISION,
                stage: Some(PauseStage::Draining),
                in_the_way: Some(vec![Obstacle::ContainedRun, Obstacle::Lease]),
            }
        );
        // Nothing in the way at that read, the hand-back not finished.
        let cause = standing(InstancePauseStage::Paused, &[]);
        assert_eq!(
            pause_answer(REVISION, &completed(Some(cause))),
            PauseOutcome::Standing {
                revision: REVISION,
                stage: Some(PauseStage::Paused),
                in_the_way: Some(Vec::new()),
            }
        );
    }

    #[test]
    fn a_standing_pause_whose_check_failed_is_read_from_the_first_cause() {
        let failure =
            Outcome::from_io_error(&std::io::Error::from(std::io::ErrorKind::TimedOut), IoOp::Read);
        let cause =
            standing(InstancePauseStage::Paused, &[PauseObstacle::CheckFailed]).caused_by(failure);
        assert_eq!(
            pause_answer(REVISION, &completed(Some(cause))),
            PauseOutcome::Standing {
                revision: REVISION,
                stage: Some(PauseStage::Paused),
                in_the_way: Some(vec![Obstacle::CheckFailed]),
            }
        );
    }

    #[test]
    fn a_resume_or_a_shutdown_during_the_stages_says_so() {
        let lifted = Outcome::new(host("scheduling_pause_lifted_by_resume"))
            .pause_stage(InstancePauseStage::Draining);
        assert_eq!(
            pause_answer(REVISION, &completed(Some(lifted))),
            PauseOutcome::LiftedByResume { revision: REVISION, stage: Some(PauseStage::Draining) }
        );
        let kept = Outcome::new(host("scheduling_pause_kept_at_shutdown"))
            .pause_stage(InstancePauseStage::Paused);
        assert_eq!(
            pause_answer(REVISION, &completed(Some(kept))),
            PauseOutcome::KeptAtShutdown { revision: REVISION, stage: Some(PauseStage::Paused) }
        );
    }

    #[test]
    fn a_cause_this_console_does_not_read_holds_and_is_kept() {
        let other = Outcome::new(host("scheduling_resume_without_reconnect"));
        assert_eq!(
            pause_answer(REVISION, &completed(Some(other))),
            PauseOutcome::Applied {
                revision: REVISION,
                cause: Some("scheduling_resume_without_reconnect".to_owned()),
            }
        );
    }

    #[test]
    fn a_received_cause_with_words_this_build_does_not_read_keeps_none_of_them() {
        // As a newer Runtime might answer it: a stage and an obstacle the pinned contract lacks.
        let outcome: Outcome = serde_json::from_value(serde_json::json!({
            "code": "request_pause_scheduling_completed",
            "category": "success",
            "values": {},
            "causes": [{
                "code": "scheduling_pause_standing",
                "category": "info",
                "relation": "caused_by",
                "values": {"pause_stage": "parked", "in_the_way": ["contained_run", "new_word"]},
            }],
        }))
        .expect("a receipt outcome's form");
        assert_eq!(
            pause_answer(REVISION, &outcome),
            PauseOutcome::Standing { revision: REVISION, stage: None, in_the_way: None }
        );
        // No list at all is an empty one.
        let outcome: Outcome = serde_json::from_value(serde_json::json!({
            "code": "request_pause_scheduling_completed",
            "category": "success",
            "values": {},
            "causes": [{
                "code": "scheduling_pause_standing",
                "category": "info",
                "relation": "caused_by",
                "values": {"pause_stage": "paused"},
            }],
        }))
        .expect("a receipt outcome's form");
        assert_eq!(
            pause_answer(REVISION, &outcome),
            PauseOutcome::Standing {
                revision: REVISION,
                stage: Some(PauseStage::Paused),
                in_the_way: Some(Vec::new()),
            }
        );
    }

    #[test]
    fn a_standing_record_says_its_stage_what_was_in_the_way_and_a_failed_check() {
        // As the ledger window reads it: the public projection, codes only.
        let record =
            standing(InstancePauseStage::Draining, &[PauseObstacle::ContainedRun]).codes_only();
        assert_eq!(
            moment_of(&record),
            Some(PauseMoment::Standing {
                stage: Some(PauseStage::Draining),
                in_the_way: Some(vec![Obstacle::ContainedRun]),
                cause: None,
            })
        );
        let failure =
            Outcome::from_io_error(&std::io::Error::from(std::io::ErrorKind::TimedOut), IoOp::Read);
        let record = standing(InstancePauseStage::Paused, &[PauseObstacle::CheckFailed])
            .caused_by(failure)
            .codes_only();
        assert_eq!(
            moment_of(&record),
            Some(PauseMoment::Standing {
                stage: Some(PauseStage::Paused),
                in_the_way: Some(vec![Obstacle::CheckFailed]),
                cause: Some("foreign_os_error".to_owned()),
            })
        );
    }

    #[test]
    fn the_records_of_no_member_say_which_moment_alone() {
        for (code, moment) in [
            ("scheduling_pause_settled", PauseMoment::Settled),
            ("scheduling_resume_without_reconnect", PauseMoment::ResumedWithoutReconnect),
            ("scheduling_resume_without_backend", PauseMoment::ResumedWithoutBackend),
        ] {
            let record = Outcome::new(host(code)).codes_only();
            assert_eq!(moment.code(), code);
            assert_eq!(moment_of(&record), Some(moment), "{code}");
        }
    }

    #[test]
    fn a_pause_kept_at_a_shutdown_says_where_it_stood() {
        let record = Outcome::new(host("scheduling_pause_kept_at_shutdown"))
            .pause_stage(InstancePauseStage::Paused)
            .codes_only();
        assert_eq!(
            moment_of(&record),
            Some(PauseMoment::KeptAtShutdown { stage: Some(PauseStage::Paused) })
        );
    }

    #[test]
    fn a_hold_lifted_not_quiet_keeps_what_was_in_the_way_and_its_failure_but_not_whose() {
        let record = Outcome::new(host("scheduling_pause_lifted_not_quiet"))
            .pause_reason("lab-record-1")
            .in_the_way([PauseObstacle::QueuedEntry])
            .caused_by(Outcome::new(host("scheduling_pause_release_busy")))
            .codes_only();
        // The projection drops the hold's reason, a name.
        assert!(record.value("pause_reason").is_none());
        assert_eq!(
            moment_of(&record),
            Some(PauseMoment::LiftedNotQuiet {
                in_the_way: Some(vec![Obstacle::QueuedEntry]),
                cause: Some("scheduling_pause_release_busy".to_owned()),
            })
        );
        // Another party held the admission guard: nothing read in the way.
        let record = Outcome::new(host("scheduling_pause_lifted_not_quiet"))
            .in_the_way(std::iter::empty::<PauseObstacle>())
            .caused_by(Outcome::new(host("scheduling_pause_drain_incomplete")))
            .codes_only();
        assert_eq!(
            moment_of(&record),
            Some(PauseMoment::LiftedNotQuiet {
                in_the_way: Some(Vec::new()),
                cause: Some("scheduling_pause_drain_incomplete".to_owned()),
            })
        );
    }

    #[test]
    fn another_record_of_a_device_state_is_no_moment_of_a_pause() {
        // The emulator watch's and the idle check's, written as the same phase.
        for code in ["emulator_autostart_skipped", "instance_idle_action"] {
            let record = Outcome::new(host(code)).codes_only();
            assert_eq!(moment_of(&record), None, "{code}");
        }
        // A pause's receipt's cause, which no record carries on top.
        let lifted = Outcome::new(host("scheduling_pause_lifted_by_resume"))
            .pause_stage(InstancePauseStage::Draining)
            .codes_only();
        assert_eq!(moment_of(&lifted), None);
    }

    #[test]
    fn a_record_with_words_this_build_does_not_read_keeps_none_of_them() {
        // As a newer Runtime might write it: a stage and an obstacle the pinned contract lacks.
        let record: Outcome = serde_json::from_value(serde_json::json!({
            "code": "scheduling_pause_standing",
            "category": "info",
            "values": {"pause_stage": "parked", "in_the_way": ["lease", "new_word"]},
        }))
        .expect("a record outcome's form");
        assert_eq!(
            moment_of(&record),
            Some(PauseMoment::Standing { stage: None, in_the_way: None, cause: None })
        );
    }

    #[test]
    fn a_denied_pause_or_a_refused_record_set_nothing() {
        assert_eq!(
            failed_pause(Some(RuntimeReceiptState::Denied), Some("scheduling_already_paused")),
            FailedPause::Refused
        );
        assert_eq!(
            failed_pause(Some(RuntimeReceiptState::Failed), Some("scheduling_pause_persist_failed")),
            FailedPause::Refused
        );
    }

    #[test]
    fn a_failed_pause_lifted_nothing_and_no_receipt_says_nothing() {
        assert_eq!(
            failed_pause(Some(RuntimeReceiptState::Failed), Some("instance_resources_fenced")),
            FailedPause::Failed
        );
        assert_eq!(failed_pause(Some(RuntimeReceiptState::Failed), None), FailedPause::Failed);
        assert_eq!(failed_pause(None, None), FailedPause::Unanswered);
    }

    #[test]
    fn an_account_pause_shows_as_soon_as_a_global_one() {
        assert_eq!(ACCOUNT_PAUSE_SETTLES, GLOBAL_PAUSE_SETTLES);
    }

    #[test]
    fn a_pause_that_holds_holds_at_once() {
        for shown in [false, true] {
            for reads in [1, PAUSE_WATCH_READS, PAUSE_WATCH_READS * 10] {
                for watched in [Duration::ZERO, SETTLES, PAUSE_WATCH_LIMIT * 2] {
                    let verdict = pause_verdict(PauseSeen::Holds, shown, reads, watched, SETTLES);
                    assert_eq!(verdict, PauseVerdict::Held, "{shown} {reads} {watched:?}");
                }
            }
        }
    }

    #[test]
    fn a_pause_still_staging_gets_no_verdict_past_the_settling() {
        for shown in [false, true] {
            for reads in [1, PAUSE_WATCH_READS, PAUSE_WATCH_READS * 10] {
                for watched in [Duration::ZERO, SETTLES, SETTLES * 2, PAUSE_WATCH_LIMIT - SECOND] {
                    let verdict =
                        pause_verdict(PauseSeen::Staging, shown, reads, watched, SETTLES);
                    assert_eq!(verdict, PauseVerdict::Watching, "{shown} {reads} {watched:?}");
                }
            }
        }
    }

    #[test]
    fn a_pause_shown_and_gone_held_and_was_lifted_later() {
        for reads in [1, PAUSE_WATCH_READS, PAUSE_WATCH_READS * 10] {
            for watched in [SECOND, SETTLES, PAUSE_WATCH_LIMIT, PAUSE_WATCH_LIMIT * 2] {
                let verdict = pause_verdict(PauseSeen::Absent, true, reads, watched, SETTLES);
                assert_eq!(verdict, PauseVerdict::LiftedLater, "{reads} {watched:?}");
            }
        }
    }

    #[test]
    fn no_pause_never_shown_does_not_hold_after_the_reads_and_the_time_or_at_the_end() {
        // Never shown: only with both the reads and the settling time behind.
        let reads = PAUSE_WATCH_READS;
        let verdict = pause_verdict(PauseSeen::Absent, false, reads, SETTLES - SECOND, SETTLES);
        assert_eq!(verdict, PauseVerdict::Watching);
        let verdict = pause_verdict(PauseSeen::Absent, false, reads, SETTLES, SETTLES);
        assert_eq!(verdict, PauseVerdict::NotHeld);
        // At the end of the watch, however few reads landed: the watch ends on a verdict.
        let verdict = pause_verdict(PauseSeen::Absent, false, 1, PAUSE_WATCH_LIMIT, SETTLES);
        assert_eq!(verdict, PauseVerdict::NotHeld);
    }

    #[test]
    fn a_pause_staging_until_the_limit_is_still_staging_and_not_confirmed() {
        for shown in [false, true] {
            for watched in [PAUSE_WATCH_LIMIT, PAUSE_WATCH_LIMIT * 2] {
                let verdict = pause_verdict(PauseSeen::Staging, shown, 200, watched, SETTLES);
                assert_eq!(verdict, PauseVerdict::StillStaging, "{shown} {watched:?}");
            }
        }
        // A read at the limit that shows it held, or gone, a resume having lifted it, says that
        // instead.
        let verdict = pause_verdict(PauseSeen::Holds, true, 200, PAUSE_WATCH_LIMIT, SETTLES);
        assert_eq!(verdict, PauseVerdict::Held);
        let verdict = pause_verdict(PauseSeen::Absent, true, 200, PAUSE_WATCH_LIMIT, SETTLES);
        assert_eq!(verdict, PauseVerdict::LiftedLater);
    }

    #[test]
    fn another_partys_pause_is_never_this_presss() {
        for shown in [false, true] {
            for reads in [1, PAUSE_WATCH_READS, PAUSE_WATCH_READS * 10] {
                for watched in [Duration::ZERO, SETTLES, PAUSE_WATCH_LIMIT * 2] {
                    let verdict = pause_verdict(PauseSeen::Other, shown, reads, watched, SETTLES);
                    assert_eq!(verdict, PauseVerdict::Overtaken, "{shown} {reads} {watched:?}");
                }
            }
        }
    }

    #[test]
    fn the_end_for_no_pause_counts_from_the_answer() {
        // A pause, global or an account's, shows within 10 s, and its answer said nothing 36 s
        // after the press, the client's whole wait: the reads after that answer are watched all
        // the same.
        let settles = Duration::from_secs(10);
        for (reads, watched) in [(1, 39), (2, 42)] {
            let watched = Duration::from_secs(watched);
            let verdict = pause_verdict(PauseSeen::Absent, false, reads, watched, settles);
            assert_eq!(verdict, PauseVerdict::Watching, "{reads}");
        }
        let watched = Duration::from_secs(45);
        let verdict = pause_verdict(PauseSeen::Absent, false, PAUSE_WATCH_READS, watched, settles);
        assert_eq!(verdict, PauseVerdict::NotHeld);
        // Nor earlier than the settling time after the press, however many reads have landed.
        let verdict =
            pause_verdict(PauseSeen::Absent, false, PAUSE_WATCH_READS * 10, SECOND * 9, settles);
        assert_eq!(verdict, PauseVerdict::Watching);
    }

    #[test]
    fn a_recording_hold_gives_its_milliseconds() {
        assert_eq!(lab_record_started("lab-record-1763270000000"), Some(1_763_270_000_000));
        assert_eq!(lab_record_started("lab-record-0"), Some(0));
        assert_eq!(
            lab_record_started("lab-record-9223372036854775807"),
            Some(9_223_372_036_854_775_807)
        );
    }

    #[test]
    fn any_other_reason_gives_none() {
        for reason in [
            "lab-record-",
            "lab-record-12a",
            "lab-record-+1",
            "lab-record--1",
            "lab-record-1-2",
            "lab-record-9223372036854775808",
            "lab-record-18446744073709551615",
            "lab-record-99999999999999999999",
            "lab-down-1763270000000",
            "xlab-record-1",
            "console_pause_account",
        ] {
            assert_eq!(lab_record_started(reason), None, "{reason}");
        }
    }
}
