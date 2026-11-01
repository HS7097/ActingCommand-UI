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
//! `Status`, and `ScheduleExplanation` without its candidate rows. How often is
//! the caller's, too: a lane never retries and keeps no clock.
//!
//! Left out on purpose: `player_hold` and `control` (the hold's read face is
//! UI-3d's); `emulator_state` (read only at discovery and control, and `off` may
//! be the idle action's doing, so showing it would mislead); `request_id` (nothing
//! here stops a run); a fence's recheck counts.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use acui_rows::{code, EventActor, EventSource};
use actingcommand_contract::{
    InstanceActivity, InstanceActivityKind, OwnerEpoch, RuntimeControlPlaneStatus,
    RuntimeErrorCode, RuntimeInstanceStatus, RuntimeScheduleExplanation,
    ScheduleDispatchState, ScheduleExplanationInstance,
};
use actingcommand_runtime_client::{
    RuntimeChannel, RuntimeClient, RuntimeClientConfig, RuntimeClientError,
};

use super::{global_pause_fact, instance_pause_fact, ClientFailure, PauseFact};

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
        let client = self.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.schedule_explanation(None, false) {
            Ok(explanation) => Ok(explain_read(epoch, &explanation)),
            Err(error) => Err(self.settled(error).into()),
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

/// Of the Runtime's own refusals only `runtime_unavailable` passes, and only when
/// the host has not halted: a halted host keeps the code of what halted it, which
/// may be this one, and says so by the refusal being fatal. `ClientFailure` does
/// not carry that, so this looks at the client's error.
fn live_failure(error: RuntimeClientError) -> LiveRead {
    let unavailable = error.projection().is_some_and(|refusal| {
        refusal.code == RuntimeErrorCode::RuntimeUnavailable && !refusal.fatal
    });
    let failure = ClientFailure::from(error);
    if unavailable || RUNTIME_SILENT_CODES.contains(&failure.code) {
        LiveRead::NoAnswer(failure)
    } else if RUNTIME_ABSENT_CODES.contains(&failure.code) {
        LiveRead::NotConnected(failure)
    } else {
        LiveRead::Trouble(failure)
    }
}

/// One answered `Status`. `epoch` and `pid` are of the connection's
/// `runtime-info.json`, `epoch` as the wire spells it.
#[derive(Debug, Clone)]
pub struct LiveSnapshot {
    pub pid: u32,
    pub epoch: String,
    pub global_pause: Option<PauseFact>,
    pub instances: Vec<InstanceLive>,
}

/// One registered instance as that `Status` reported it.
#[derive(Debug, Clone)]
pub struct InstanceLive {
    pub alias: String,
    pub instance_id: String,
    /// Absent when the host runs without policy inputs.
    pub game_id: Option<String>,
    pub pause: Option<PauseFact>,
    pub fence: Option<FenceFact>,
    /// The instance's recovery ladder holds instead of climbing.
    pub ladder_busy: bool,
    /// Absent while it does nothing the vocabulary names, and always from a
    /// Runtime before 0.12.36: not an error.
    pub activity: Option<ActivityFact>,
}

/// An instance's resource fence: the code of the failure behind it, and whether it
/// now waits for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceFact {
    pub cause: String,
    pub awaiting_person: bool,
}

/// What an instance does now. `policy_run` and `direct_run` always name a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityFact {
    pub kind: ActivityKind,
    pub package_id: Option<String>,
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
/// shown are chosen by, never shown itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceExplain {
    pub why: String,
    /// That row's own wake (`not_running.next_wake_unix_ms`), in unix ms.
    pub next_wake: Option<u64>,
    /// A task is selected and its dispatch has not ended.
    pub preparing: bool,
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
    }
}

fn instance_live(instance: &RuntimeInstanceStatus, owner_epoch: OwnerEpoch) -> InstanceLive {
    let alias = instance.instance_alias();
    InstanceLive {
        alias: alias.to_string(),
        instance_id: code(&instance.instance_id()),
        game_id: instance.game_id().map(str::to_owned),
        pause: instance.pause().map(|pause| instance_pause_fact(alias, pause, owner_epoch)),
        fence: instance.resource_fence().map(|fence| FenceFact {
            cause: fence.cause.as_str().to_owned(),
            awaiting_person: fence.awaiting_person,
        }),
        ladder_busy: instance.ladder_busy_hold().is_some(),
        activity: instance.activity().map(activity_fact),
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
    InstanceExplain {
        why: entry.not_running.why.code().as_str().to_owned(),
        next_wake: entry.not_running.next_wake_unix_ms,
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
