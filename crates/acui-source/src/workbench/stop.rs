// SPDX-License-Identifier: GPL-3.0-only
//! A person's stop of one run of an account (UI-3e2; Workflow #429 R-6's first slice, Runtime
//! 0.12.53): one `StopInstanceRun` of the run the newest `Status` names
//! ([`super::ActivityFact::run`]), on a fresh person's connection, which is let go after it
//! ([`stop_run`]). The Runtime asks the run to stop at its next checkpoint, resets the device
//! itself with the run's own lease, and answers once the run ended or once the wait is over; what
//! to show after it is the next `Status`'s. Nothing out of here names a wire type: what came back
//! is a [`StopOutcome`].
//!
//! Only a run a client started (`direct_run`) is stopped: one the scheduler dispatched or the
//! Runtime started for itself is refused, and nothing is stopped. A Runtime before 0.12.53 has no
//! stop at all: it refuses `not_implemented` and records nothing ([`Refusal::NotImplemented`]);
//! nothing else tells the console so, since it reads no version of the Runtime.
//!
//! The connection is that of a person's change of settings ([`super::configure`]), asking for
//! the full receipt, at the client's default io timeout: never the reading lane's, whose one
//! connection is held for a whole exchange, so that a stop on it would hold `Status` back as long.
//! The client waits for the receipt [`STOP_WAIT`], the reset's own wait and the io timeout, on the
//! caller's thread, so that is never the event loop's. A receipt that does not come in time
//! latches this connection and is unconfirmed: the client's own word is to read `Status` and stop
//! again on a new connection, which is the caller's to decide.

use std::path::Path;
use std::time::Duration;

use actingcommand_contract::codes::SharedCode;
use actingcommand_contract::outcome::{CauseRelation, Outcome, RunOrigin};
use actingcommand_contract::{InstanceRunReset, RuntimeReceiptState};
use actingcommand_runtime_client::{
    RuntimeClient, RuntimeClientError, RuntimeClientErrorClass, StoppedRun,
};

use super::backends::links;
use super::{configure_config, RunKey};
use crate::ClientFailure;

/// How long the Runtime is asked to wait for the run's end before it answers (`wait_ms`), the
/// first request's and the follow-up's alike: a run whose next checkpoint comes within it answers
/// at once with its reset, and a follow-up, sent once the run left `Status`, finds it ended.
pub const STOP_WAIT: Duration = Duration::from_secs(5);

/// The codes a refusal of a stop is told apart by: a run no client may stop; a run the instance
/// neither runs nor stopped last (the host's own spelling, of a crate this one does not use); a
/// Runtime that has no stop.
const NOT_CLIENT_STOPPABLE: &str = SharedCode::ScheduledContainedTaskNotClientCancellable.as_str();
const RUN_UNKNOWN: &str = "contained_task_cancellation_identity_missing";
const NOT_IMPLEMENTED: &str = SharedCode::NotImplemented.as_str();
/// What a refusal of a run no client may stop names: who started it (vocabulary `run_origin`).
const RUN_ORIGIN: &str = "run_origin";
const POLICY: &str = RunOrigin::Policy.as_str();
const HOST: &str = RunOrigin::Host.as_str();
/// The fence a failed reset leaves, which the stopped run's receipt gives as its cause, once or
/// more down the chain: the failure under it is the reset's own (the host's spelling).
const INSTANCE_RESOURCES_FENCED: &str = "instance_resources_fenced";
const CAUSED_BY: &str = CauseRelation::CausedBy.as_str();

/// What became of one [`stop_run`].
#[derive(Debug, Clone)]
pub enum StopOutcome {
    /// The Runtime took the stop (`InstanceRunStopped`). `settled`: the run ended within the
    /// wait, and `reset` says how the Runtime's own reset after it went; else it still stops, the
    /// stop staying asked, and there is no `reset` yet.
    Stopped { settled: bool, reset: Option<Reset> },
    /// The Runtime answered and stopped nothing: why, who started the run when the refusal names
    /// it, and whether it denied the request, which leaves everything as it was, or failed on the
    /// way, which leaves the device as the card says. The Runtime's code is in `runtime_code`, the
    /// host's behind it in `host`.
    Refused { failure: ClientFailure, why: Refusal, origin: Option<Origin>, denied: bool },
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it: a receipt that did not come in
    /// time, a failed write, a refusal that halted the host, an answer of another run or one whose
    /// `settled` and `reset` disagree (`runtime_result_unexpected`). Nothing here sends it again.
    Unconfirmed(ClientFailure),
}

/// How the Runtime's own reset of the device after the stopped run went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reset {
    /// The stop ended the run, and the Runtime reset the device with the run's lease.
    Done,
    /// The Runtime did not reset it: the run had ended, or been asked to end, for another reason
    /// first (on its own, at its deadline, by a pause's drain, by its own client), and its reset,
    /// if it owes one, is not the Runtime's; the device is as the card says.
    NotNeeded,
    /// The reset failed, and the instance is fenced. `cause`: the code of why, the first down the
    /// receipt's causes past the fence's own, none when there is none, for its words and the log,
    /// never shown; `chain`: every code of the receipt's outcome, for the log only.
    Failed { cause: Option<String>, chain: String },
}

/// Why the Runtime refused a stop, by its receipt's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No client may stop this run (`scheduled_contained_task_not_client_cancellable`): the
    /// scheduler's or the Runtime's own, which `origin` names.
    NotStoppable,
    /// The instance neither runs this run nor stopped it last
    /// (`contained_task_cancellation_identity_missing`): it ended before the stop came, or a stop
    /// of another run since took the place of the instance's last stopped one.
    RunUnknown,
    /// This Runtime has no stop of a run (`not_implemented`, a Runtime before 0.12.53), and
    /// recorded nothing.
    NotImplemented,
    /// Any other: its codes are in the failure.
    Other,
}

/// Who started a run no client may stop (`run_origin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The scheduler: a policy run.
    Policy,
    /// The Runtime itself: a startup package, a recovery ladder's rung, or another step of its own.
    Host,
}

/// One person's stop of the run `run` of the instance `alias` names: a fresh person's connection,
/// one `StopInstanceRun` waiting [`STOP_WAIT`] for the run's end, and the connection let go. Sent
/// once, whatever comes back. The client checks that an answer is of `run`, and that it states a
/// reset exactly when the run ended.
pub fn stop_run(root: &Path, alias: &str, run: RunKey) -> StopOutcome {
    let client = match RuntimeClient::connect(configure_config(root)) {
        Ok(client) => client,
        Err(error) => return StopOutcome::NotSent(ClientFailure::from(error)),
    };
    match client.stop_instance_run(alias, run.0, STOP_WAIT) {
        Ok(answer) => stopped(answer),
        Err(error) => failed(error),
    }
}

fn stopped(answer: StoppedRun) -> StopOutcome {
    let StoppedRun { stopped, outcome } = answer;
    let reset = stopped.reset.map(|reset| match reset {
        InstanceRunReset::Done => Reset::Done,
        InstanceRunReset::NotNeeded => Reset::NotNeeded,
        InstanceRunReset::Failed => {
            Reset::Failed { cause: reset_cause(&outcome), chain: chain(&outcome) }
        }
    });
    StopOutcome::Stopped { settled: stopped.settled, reset }
}

/// A failed request after the connection, told apart as a device's operation is
/// (`device::failed`), in this order:
///
/// 1. a refusal the Runtime answered and went on after, of a request nothing leaves uncertain:
///    refused, why by its receipt's code, and who started the run by its `run_origin`;
/// 2. anything else, a refusal that halted the host included, which carries a projection that is
///    fatal and no uncertainty: unconfirmed, since a failed write may have got the request out.
fn failed(error: RuntimeClientError) -> StopOutcome {
    let answered = error.projection().is_some_and(|refusal| !refusal.fatal)
        && !matches!(error.disposition(), RuntimeClientErrorClass::Uncertain);
    if !answered {
        return StopOutcome::Unconfirmed(ClientFailure::from(error));
    }
    let answer = error.receipt_outcome();
    // The receipt's own code, not the client's (`runtime_request_rejected`) nor its operation.
    let why = match answer.map_or("", |outcome| outcome.code().as_str()) {
        NOT_CLIENT_STOPPABLE => Refusal::NotStoppable,
        RUN_UNKNOWN => Refusal::RunUnknown,
        NOT_IMPLEMENTED => Refusal::NotImplemented,
        _ => Refusal::Other,
    };
    let origin = answer
        .and_then(|outcome| outcome.value(RUN_ORIGIN))
        .and_then(|value| value.as_str())
        .and_then(|word| match word {
            POLICY => Some(Origin::Policy),
            HOST => Some(Origin::Host),
            _ => None,
        });
    let denied = error
        .received_receipt()
        .is_some_and(|receipt| matches!(receipt.state(), RuntimeReceiptState::Denied));
    StopOutcome::Refused { failure: ClientFailure::from(error), why, origin, denied }
}

/// Why a reset failed: from the receipt's top outcome down the first `caused_by` of each, past
/// every fence one after another (a touch that failed and left resources unconfirmed fences more
/// than once), the first code that is not the fence's; none when the chain ends before one.
fn reset_cause(outcome: &Outcome) -> Option<String> {
    let mut below = links(outcome, CAUSED_BY).next()?;
    while below.code().as_str() == INSTANCE_RESOURCES_FENCED {
        below = links(below, CAUSED_BY).next()?;
    }
    Some(below.code().as_str().to_owned())
}

/// Every code of `outcome` and of the outcomes it links, depth first, each link with its relation:
/// for the log only.
fn chain(outcome: &Outcome) -> String {
    let mut line = outcome.code().as_str().to_owned();
    for link in outcome.causes() {
        line.push_str(&format!(" [{} {}]", link.relation().as_str(), chain(link.outcome())));
    }
    line
}
