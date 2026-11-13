// SPDX-License-Identifier: GPL-3.0-only
//! A person's stop of one run of an account (UI-3e2; Workflow #429 R-6's first slice, Runtime
//! 0.12.53): one `StopInstanceRun` of the run the newest `Status` names
//! ([`super::ActivityFact::run`]), on a fresh person's connection, which is let go after it
//! ([`stop_run`]). The Runtime asks the run to stop at its next checkpoint, resets the device
//! itself with the run's own lease, and answers once the run ended or once the wait is over; what
//! to show after it is the next `Status`'s. Nothing out of here names a wire type: what came back
//! is a [`StopOutcome`].
//!
//! A person's "close this task" of a run the scheduler dispatched (UI-3e2b) closes the task
//! first, and only once its close row took sends this same stop, on the connection its close went
//! out on; of a package the account's view does not list, it writes no row and sends the stop
//! alone ([`stop_on`], `super::pack::close_then_stop`).
//!
//! A run a client started (`direct_run`) is stopped, and since 0.12.55 one the scheduler
//! dispatched (`policy_run`) too: only a step the Runtime started for itself is refused
//! (`run_origin` `host`), and nothing is stopped. 0.12.53 and 0.12.54 refuse a scheduled run as
//! well, with `run_origin` `policy`, which only they answer ([`Origin::Policy`]). A Runtime before
//! 0.12.53 has no stop at all: it refuses `not_implemented` and records nothing
//! ([`Refusal::NotImplemented`]); nothing else tells the console so, since it reads no version of
//! the Runtime.
//!
//! The connection is that of a person's change of settings ([`super::configure`]), asking for
//! the full receipt, at the client's default io timeout: never the reading lane's, whose one
//! connection is held for a whole exchange, so that a stop on it would hold `Status` back as long.
//! The client waits for the receipt [`STOP_WAIT`], the reset's own wait and the io timeout, on the
//! caller's thread, so that is never the event loop's. A receipt that does not come in time
//! latches this connection and is unconfirmed: the client's own word is to read `Status` and stop
//! again on a new connection, which is the caller's to decide.
//!
//! When an answer that the run stopped or ended is said to have come, [`stop_time`] decides: the
//! first request's answer comes within its wait, as the run ends, and is said at its time; a
//! follow-up's comes once a `Status` showed the run gone, after the Runtime ended it, and is said
//! as the time the console learned it ([`StopTime`]). The run's own end is the ledger's; nothing
//! here reads it.

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
    /// host's behind it in `host`; `chain`, as in each failure here, has every code of it.
    Refused {
        failure: ClientFailure,
        chain: String,
        why: Refusal,
        origin: Option<Origin>,
        denied: bool,
    },
    /// No connection, so nothing was sent.
    NotSent { failure: ClientFailure, chain: String },
    /// Sent, or perhaps sent, and nothing says what became of it: a receipt that did not come in
    /// time, a failed write, a refusal that halted the host, an answer of another run or one whose
    /// `settled` and `reset` disagree (`runtime_result_unexpected`). Nothing here sends it again.
    Unconfirmed { failure: ClientFailure, chain: String },
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
    /// Runtime's own, which `origin` names; since 0.12.55 only that, and on 0.12.53 and 0.12.54 the
    /// scheduler's too.
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
    /// The scheduler: a policy run. Only 0.12.53 and 0.12.54 answer it; 0.12.55 stops a policy run,
    /// and refuses only the Runtime's own steps (`Host`). The token stays registered.
    Policy,
    /// The Runtime itself: a startup package, a recovery ladder's rung, or another step of its own.
    Host,
}

/// When an answer that a run stopped or ended landed (unix ms), as a person is told it: as its
/// time, or as the time the console learned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopTime {
    /// The first request's answer, which came within its wait ([`STOP_WAIT`]) as the run ended.
    At(u64),
    /// A follow-up's answer, sent once a `Status` showed the run gone: when the console learned
    /// that the run had ended, not when the Runtime ended it, which was before.
    LearnedAt(u64),
}

/// How the time of an answer of a stop landing at `at_ms` is said ([`StopTime`]): of a follow-up
/// (`follow_up`), as the time the console learned what it says; else as its time.
pub fn stop_time(follow_up: bool, at_ms: u64) -> StopTime {
    if follow_up {
        StopTime::LearnedAt(at_ms)
    } else {
        StopTime::At(at_ms)
    }
}

/// One person's stop of the run `run` of the instance `alias` names: a fresh person's connection,
/// one `StopInstanceRun` waiting [`STOP_WAIT`] for the run's end, and the connection let go. Sent
/// once, whatever comes back. The client checks that an answer is of `run`, and that it states a
/// reset exactly when the run ended.
pub fn stop_run(root: &Path, alias: &str, run: RunKey) -> StopOutcome {
    let client = match RuntimeClient::connect(configure_config(root)) {
        Ok(client) => client,
        Err(error) => {
            let chain = chain(&error.outcome());
            return StopOutcome::NotSent { failure: ClientFailure::from(error), chain };
        }
    };
    stop_on(&client, alias, run)
}

/// The stop of [`stop_run`] on a connection already made, one `StopInstanceRun` waiting
/// [`STOP_WAIT`]: a person's "close this task" sends it on the connection its view and its close
/// row went out on, once the close row took, or when the view does not list the run's package
/// (`super::pack::close_then_stop`). Sent once, whatever comes back.
pub(super) fn stop_on(client: &RuntimeClient, alias: &str, run: RunKey) -> StopOutcome {
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
    // The failure as one outcome, the receipt's or else the client's own, with all it links.
    let causes = chain(&error.outcome());
    let answered = error.projection().is_some_and(|refusal| !refusal.fatal)
        && !matches!(error.disposition(), RuntimeClientErrorClass::Uncertain);
    if !answered {
        return StopOutcome::Unconfirmed { failure: ClientFailure::from(error), chain: causes };
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
    StopOutcome::Refused { failure: ClientFailure::from(error), chain: causes, why, origin, denied }
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

#[cfg(test)]
mod tests {
    use super::{stop_time, StopTime};

    const LANDED_MS: u64 = 1_794_549_812_000;

    #[test]
    fn an_answer_within_the_first_wait_is_said_at_its_time() {
        assert_eq!(stop_time(false, LANDED_MS), StopTime::At(LANDED_MS));
    }

    #[test]
    fn a_follow_up_answer_is_said_as_the_time_it_was_learned() {
        assert_eq!(stop_time(true, LANDED_MS), StopTime::LearnedAt(LANDED_MS));
    }
}
