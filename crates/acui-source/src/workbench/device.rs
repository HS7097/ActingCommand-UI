// SPDX-License-Identifier: GPL-3.0-only
//! A person's operation of an instance's device (UI-3e1-a): the rescue bar's reconnect and
//! self-check, its emulator's start, stop and restart, and its game's launch and restart
//! ([`DeviceOp`]). Each goes once on a fresh person's connection, which is let go after it, and
//! never again by itself, whatever comes back ([`operate`]); what to show after it is the next
//! `Status`'s. Nothing out of here names a wire type: what came back is a [`DeviceOutcome`], and
//! what held an instance is a [`backends::Holder`], the words the switch of backends reads too.
//!
//! The connection is that of a person's change of settings ([`super::configure`]), asking for
//! the full receipt, whose refusal of a request that waited its turn in vain names what held the
//! instance, but at [`DEVICE_IO`]. Each request is the client's own method, with the client's own
//! receipt wait, on the caller's thread, so that is never the event loop's:
//!
//! - a self-check (`SelfCheckInstance`) waits `CONNECTION_PREPARATION_WAIT_MS`, its claim's wait
//!   in the instance's queue and the io timeout;
//! - an emulator's control (`ControlEmulatorInstance`) a fixed wait of its action's, which the
//!   io timeout plays no part in: a restart's is the longest, over twelve minutes;
//! - a game's (`ApplicationLifecycle`) the io timeout and its claim's wait, and then the client
//!   reads the request's records back by its correlation, a bounded read of the ledger, as the
//!   picture's refresh does ([`super::preview::observe_frame`]).
//!
//! A request meets the instance's one queue: what holds the instance, or a claim that waits for
//! it first, keeps it waiting a short while (an emulator's control 5 s, the others 3 s), and
//! then the Runtime refuses it (`lease_queue_expired`) and nothing was done. An operator's claim
//! goes through a closed gate, so no pause holds these back.

use std::path::Path;
use std::time::Duration;

use actingcommand_contract::{
    ApplicationLifecycleAction, EmulatorInstanceAction, RuntimeReceiptState, RuntimeResult,
    SchedulingResumeSelfCheck, StartupPackageDisposition, StartupPackageWithheld,
};
use actingcommand_runtime_client::{RuntimeClient, RuntimeClientError, RuntimeClientErrorClass};

use super::backends::{self, Holder};
use super::configure_config;
use crate::{person_connection, ClientFailure, RUNTIME_RESULT_UNEXPECTED};

/// The io timeout of a device's connection, within the client's bound of 60 s. At the client's
/// default of 5 s a game's receipt would be waited for 8 s, where its restart closes the
/// device's resources and runs two adb commands of up to 12 s each; at 30 s it is 33 s, and a
/// self-check's 63 s. A receipt not back by then is unconfirmed, never a success, and a Runtime
/// that hangs is told 30 s later, on the caller's thread.
pub const DEVICE_IO: Duration = Duration::from_secs(30);

/// The client's and the contract's own spellings of the three operations
/// (`ClientLocation::SelfCheckInstance`, `ContractLocation::ControlEmulatorInstance`,
/// `ClientLocation::ApplicationLifecycle`), for an answer of another shape.
const SELF_CHECK_INSTANCE: &str = "self_check_instance";
const CONTROL_EMULATOR_INSTANCE: &str = "control_emulator_instance";
const APPLICATION_LIFECYCLE: &str = "application_lifecycle";

/// What a refusal of a request that waited its turn in vain (`lease_queue_expired`) says of the
/// instance's holder, when it has one: vocabulary `claim_kind`.
const HOLDER_KIND: &str = "holder_kind";

/// One of the rescue bar's operations of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceOp {
    /// Close the device's session, connect it anew and check it (`SelfCheckInstance`): of any
    /// instance that drives a device of its own, an emulator's or a handset's
    /// ([`super::InstanceLive::physical`]). A passing check lifts the instance's fence.
    SelfCheck,
    /// Start, stop or restart its emulator (`ControlEmulatorInstance`): only of an instance whose
    /// capability profile has the control available, a discovery-bound MuMu instance's
    /// ([`super::InstanceLive::emulator_controls`]). Stopping it closes its game too.
    EmulatorStart,
    EmulatorStop,
    EmulatorRestart,
    /// Launch its game, or force-stop and launch it again (`ApplicationLifecycle`).
    GameLaunch,
    GameRestart,
}

/// What became of one [`operate`].
#[derive(Debug, Clone)]
pub enum DeviceOutcome {
    /// The Runtime did it, and says what came of it.
    Done(DeviceDone),
    /// A game's launch or restart the Runtime did, as its committed receipt says, after which
    /// the client's read-back of the request's records failed
    /// (`runtime_projection_failed_after_terminal`): said as done, the failure for the log.
    DoneUnread(ClientFailure),
    /// The Runtime answered that it did not do it.
    Refused(DeviceRefusal),
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it. It is not sent again.
    Unconfirmed(ClientFailure),
}

/// What a done operation came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceDone {
    /// The emulator's control was carried out and its readiness wait met: whether the emulator
    /// runs now, and what became of the instance's startup package.
    Emulator { running: bool, startup: Startup },
    /// The game's command completed. Whether the game came to its main screen it does not say:
    /// the picture shows it.
    Game,
    /// The device was connected anew and checked. `passed`: the check names no failure and both
    /// its capture and its touch verified (`SchedulingResumeSelfCheck`). `failure`: the code of a
    /// failed connection or of a claim it could not take, for its words and the log, never
    /// shown. `size`: the capture's frame, width by height, when it opened.
    SelfCheck { passed: bool, failure: Option<String>, size: Option<(u32, u32)> },
}

/// What an emulator's start or restart did with the instance's startup package, which then runs
/// as a task of its own (`StartupPackageDisposition`, `StartupPackageWithheld`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    /// Queued for the Runtime's own scheduling point: it brings the game to its main screen.
    Scheduled,
    /// Queued, and the instance's own pause holds it until that resumes.
    HeldByOwnPause,
    /// Queued, and the global pause holds it until that is lifted.
    HeldByAllPause,
    /// None: the instance has no startup package, or the action was a stop.
    None,
}

/// A refusal of a device's operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRefusal {
    /// The Runtime's code in `runtime_code`, the host's behind it in `host`.
    pub failure: ClientFailure,
    /// What held the instance, as the refusal names it (`holder_kind`): a request that waited
    /// its turn in vain names the instance's holder, when it has one.
    pub holder: Option<Holder>,
    /// The Runtime denied it (its receipt's state), so nothing was done; else the operation
    /// failed on the way, and the device is as the card says.
    pub denied: bool,
}

/// One person's operation `op` of the device of the instance `alias` names: a fresh person's
/// connection, one request, and the connection let go. Sent once, whatever comes back. The
/// client checks that an answer is of the request: an emulator's control by its instance and
/// action, a self-check by its instance, a game's by its action (its answer names no instance).
pub fn operate(root: &Path, alias: &str, op: DeviceOp) -> DeviceOutcome {
    let connected = person_connection(configure_config(root).with_io_timeout(DEVICE_IO));
    let client = match connected {
        Ok(client) => client,
        Err(error) => return DeviceOutcome::NotSent(ClientFailure::from(error)),
    };
    match op {
        DeviceOp::SelfCheck => self_check(&client, alias),
        DeviceOp::EmulatorStart => control_emulator(&client, alias, EmulatorInstanceAction::Start),
        DeviceOp::EmulatorStop => control_emulator(&client, alias, EmulatorInstanceAction::Stop),
        DeviceOp::EmulatorRestart => {
            control_emulator(&client, alias, EmulatorInstanceAction::Restart)
        }
        DeviceOp::GameLaunch => control_game(&client, alias, ApplicationLifecycleAction::Launch),
        DeviceOp::GameRestart => control_game(&client, alias, ApplicationLifecycleAction::Restart),
    }
}

fn self_check(client: &RuntimeClient, alias: &str) -> DeviceOutcome {
    match client.self_check_instance(alias) {
        Ok(RuntimeResult::InstanceSelfChecked { selfcheck, .. }) => {
            DeviceOutcome::Done(self_checked(selfcheck))
        }
        // The client checks the result itself and gives back no other.
        Ok(_) => unexpected(SELF_CHECK_INSTANCE),
        Err(error) => failed(error, None),
    }
}

fn control_emulator(
    client: &RuntimeClient,
    alias: &str,
    action: EmulatorInstanceAction,
) -> DeviceOutcome {
    match client.control_emulator_instance(alias, action) {
        Ok(RuntimeResult::EmulatorInstanceControlled {
            running,
            startup_package,
            startup_package_withheld,
            ..
        }) => DeviceOutcome::Done(DeviceDone::Emulator {
            running,
            startup: startup(startup_package, startup_package_withheld),
        }),
        // Likewise.
        Ok(_) => unexpected(CONTROL_EMULATOR_INSTANCE),
        Err(error) => failed(error, None),
    }
}

/// The records the client reads back with a game's answer are not used: the picture's lane
/// finds what the game shows.
fn control_game(
    client: &RuntimeClient,
    alias: &str,
    action: ApplicationLifecycleAction,
) -> DeviceOutcome {
    match client.control_application(alias, action) {
        Ok(_) => DeviceOutcome::Done(DeviceDone::Game),
        Err(error) => failed(error, Some(action)),
    }
}

/// A failed request after the connection, told apart in this order:
///
/// 1. a game's `action` whose receipt the client committed as completed, and whose read-back of
///    the request's records failed after it (`after_commit`): done, unread. It carries no
///    refusal, so the next step would call it unconfirmed;
/// 2. a refusal the Runtime answered and went on after, of a request nothing leaves uncertain:
///    refused;
/// 3. anything else, a refusal that halted the host included: unconfirmed, as for the global
///    pause (`pause_global`), since a failed write may have got the request out.
fn failed(error: RuntimeClientError, game: Option<ApplicationLifecycleAction>) -> DeviceOutcome {
    let completed = game.is_some_and(|action| {
        error.committed_receipt().is_some_and(|receipt| {
            matches!(
                receipt.result(),
                Some(RuntimeResult::ApplicationLifecycleCompleted { action: done, .. })
                    if *done == action
            )
        })
    });
    if completed {
        return DeviceOutcome::DoneUnread(ClientFailure::from(error));
    }
    let answered = error.projection().is_some_and(|refusal| !refusal.fatal)
        && !matches!(error.disposition(), RuntimeClientErrorClass::Uncertain);
    if !answered {
        return DeviceOutcome::Unconfirmed(ClientFailure::from(error));
    }
    let holder = error
        .receipt_outcome()
        .and_then(|outcome| outcome.value(HOLDER_KIND))
        .and_then(|value| value.as_str())
        .map(backends::holder);
    let denied = error
        .received_receipt()
        .is_some_and(|receipt| matches!(receipt.state(), RuntimeReceiptState::Denied));
    DeviceOutcome::Refused(DeviceRefusal { failure: ClientFailure::from(error), holder, denied })
}

/// A side of a self-check is `ok` only when this reconnect's open verified it, so a check that
/// names no failure passes only with both.
fn self_checked(check: SchedulingResumeSelfCheck) -> DeviceDone {
    DeviceDone::SelfCheck {
        passed: check.failure_code.is_none() && check.capture.ok && check.touch.ok,
        size: check.capture.width.zip(check.capture.height),
        failure: check.failure_code,
    }
}

/// A pause holds only a package that was queued: the answer names none beside `none`.
fn startup(
    package: StartupPackageDisposition,
    withheld: Option<StartupPackageWithheld>,
) -> Startup {
    match (package, withheld) {
        (StartupPackageDisposition::None, _) => Startup::None,
        (StartupPackageDisposition::Scheduled, None) => Startup::Scheduled,
        (StartupPackageDisposition::Scheduled, Some(StartupPackageWithheld::InstancePaused)) => {
            Startup::HeldByOwnPause
        }
        (StartupPackageDisposition::Scheduled, Some(StartupPackageWithheld::SchedulingPaused)) => {
            Startup::HeldByAllPause
        }
    }
}

fn unexpected(operation: &'static str) -> DeviceOutcome {
    DeviceOutcome::Unconfirmed(ClientFailure {
        code: RUNTIME_RESULT_UNEXPECTED,
        operation,
        runtime_code: None,
        host: None,
    })
}
