// SPDX-License-Identifier: GPL-3.0-only
//! A person's operation of an account's device (UI-3e1-a): the rescue bar's reconnect and
//! self-check, its emulator's start, stop and restart, and its game's launch and restart
//! (`DeviceOp`). Each is sent as the picture's refresh is (`refresh`): every account has a ticket
//! of its own, one out at a time, and a press goes out once, on a thread of its own, over a fresh
//! person's connection (`device::operate`); whatever comes back, nothing is sent again by itself.
//! Unlike a refresh's, an operation's answer counts a write for the reads (`super::wrote`): it
//! changes what they show of the account, its activity, its emulator's reading and its fence.
//!
//! A button can be pressed only while the reads and the account's own presses let it
//! (`Model::rescue_view`): never while another work of the same device is out from here, an
//! operation, a refresh of its picture or a switch of its backends (`Model::device_busy`), and
//! not for `RESCUE_COOLDOWN` after an operation lands, whatever it came to, or after a restart
//! let it go; the window never counts that down. So a press that lands after its button went
//! off sends nothing. Stopping or restarting the emulator and restarting the game close the
//! game, and take two presses: the first only asks, and the question goes once anything else of
//! the device is pressed, it is cancelled, another account or page is picked, the Runtime starts
//! again, or a `Status` read leaves the button off. A press of the same button again asks again:
//! only the confirmation sends it.
//!
//! The only wait is the client's own for the receipt, on the operation's thread. A restart of
//! the Runtime lets every ticket go: what was out may or may not have happened, and the next
//! `Status` says which. Every answer goes in the log, from the thread, so one that lands after
//! the event loop is not lost: quietly when the Runtime did it, as the problem it is otherwise.
//!
//! `Rescue` and the views it makes hold no words, as the `Model` they are part of; `paint` makes
//! them.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use acui_source::workbench::backends::Holder;
use acui_source::workbench::device::{operate, DeviceDone, DeviceOp, DeviceOutcome};
use acui_source::workbench::{ActivityKind, InstanceLive, Unstated};
use slint::Timer;

use super::failure_line;
use super::paint;
use super::picture::Doing;
use super::read::panic_message;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// How long an account's device buttons stay off after an operation of it lands, or after a
/// restart let one go.
pub const RESCUE_COOLDOWN: Duration = Duration::from_secs(5);

/// The operations' threads, also what their lines in the technical log are under.
const SELF_CHECK_THREAD: &str = "acui-act-selfcheck";
const EMULATOR_THREAD: &str = "acui-act-emulator";
const GAME_THREAD: &str = "acui-act-game";

fn thread(op: DeviceOp) -> &'static str {
    match op {
        DeviceOp::SelfCheck => SELF_CHECK_THREAD,
        DeviceOp::EmulatorStart | DeviceOp::EmulatorStop | DeviceOp::EmulatorRestart => {
            EMULATOR_THREAD
        }
        DeviceOp::GameLaunch | DeviceOp::GameRestart => GAME_THREAD,
    }
}

/// For the log.
fn name(op: DeviceOp) -> &'static str {
    match op {
        DeviceOp::SelfCheck => "reconnect and self-check",
        DeviceOp::EmulatorStart => "emulator start",
        DeviceOp::EmulatorStop => "emulator stop",
        DeviceOp::EmulatorRestart => "emulator restart",
        DeviceOp::GameLaunch => "game launch",
        DeviceOp::GameRestart => "game restart",
    }
}

/// Whether `op` takes two presses: it closes the game, with the emulator or on its own.
fn asks_first(op: DeviceOp) -> bool {
    matches!(op, DeviceOp::EmulatorStop | DeviceOp::EmulatorRestart | DeviceOp::GameRestart)
}

/// Each account's operations of its device this run, and the confirmation asked; on the event
/// loop only.
#[derive(Default)]
pub struct Rescue {
    /// How many operations were pressed this run: the last one's ticket.
    issued: u64,
    /// By alias.
    slots: BTreeMap<String, RescueSlot>,
    /// The operation of an account whose first press asked for the confirmation.
    confirming: Option<(String, DeviceOp)>,
}

/// One account's: the operation out, if any; what the last came to; and the one whose cooldown
/// runs, which its timer ends (`Rescue::cooled`).
#[derive(Default)]
struct RescueSlot {
    out: Option<(RescueTicket, DeviceOp)>,
    said: Option<Rescued>,
    cooling: Option<RescueTicket>,
}

/// The name of one operation of an account's device: its answer counts only under the ticket
/// still out for that account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescueTicket(u64);

impl RescueTicket {
    /// For the log.
    pub fn number(self) -> u64 {
        self.0
    }
}

/// What the last operation of an account came to, before its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rescued {
    /// It is out.
    Sending(DeviceOp),
    /// The Runtime did it, as the answer came at `at_ms` (unix ms), and says what came of it.
    /// `unread`: a game's, whose records the client did not read back after it.
    Done { op: DeviceOp, at_ms: u64, done: DeviceDone, unread: bool },
    /// The Runtime did not do it: the host's code and the Runtime's, for their words; what held
    /// the instance, when the refusal names it; and whether the Runtime denied it, so nothing was
    /// done, or it failed on the way, which leaves the device as the card says.
    Refused {
        op: DeviceOp,
        host: Option<String>,
        class: Option<String>,
        holder: Option<Holder>,
        denied: bool,
    },
    /// Nothing says what became of it: it may have happened. It is not sent again.
    Unconfirmed(DeviceOp),
    /// No connection: nothing was sent.
    NotSent(DeviceOp),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// One was out across a restart.
    Restarted,
}

/// Why every device button of an account is off now: the first of these that holds, in this
/// order (`Model::rescue_view`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescueHold {
    /// Its gate is shut: its device is being handed over (UI-3d).
    GateShut,
    /// An operation of it is out: the line of what it came to says so.
    Out(DeviceOp),
    /// A refresh of its picture is out.
    Refreshing,
    /// A switch of its device's backends is out, or its answer was lost and no `Status` has
    /// shown it over.
    Switching,
    /// An operation of it just landed, or a restart let one go.
    CoolingDown,
    /// It runs a task: a policy's, a direct run's or the host's own.
    Running,
    /// Its recovery ladder climbs or holds.
    Recovering,
    /// It reconnects, checks itself or has its emulator controlled, as the card says.
    Busy(Doing),
}

/// What an account's rescue bar offers now, before its words (`Model::rescue_view`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RescueView {
    /// No Runtime answers: the bar is one line, that no device can be operated. Nothing an
    /// earlier `Status` said decides which rows there would be.
    Offline,
    Online(RescueRows),
}

impl RescueView {
    /// Whether `op`'s button is drawn and can be pressed now.
    pub fn offers(&self, op: DeviceOp) -> bool {
        match self {
            Self::Offline => false,
            Self::Online(rows) => rows.offers(op),
        }
    }
}

/// The bar's rows of an account the newest `Status` lists. A row or a button that is `None` is
/// not drawn, which is not the same as drawn and off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescueRows {
    /// "Reconnect and check": of an instance that drives a device of its own.
    pub check: Option<Button>,
    /// The check is the thing to do first: the instance is fenced, and it lifts the fence.
    pub check_urgent: bool,
    /// The emulator's: of such an instance whose capability profile has any of its controls
    /// available, a discovery-bound MuMu instance's, each button by its own.
    pub emulator: Option<EmulatorRow>,
    /// The game's: of an instance that drives a device of its own.
    pub game: Option<GameRow>,
    /// "Close this task", always off, while the instance does something: why.
    pub close: Option<CloseWhy>,
    /// Why every device button is off, if anything is.
    pub hold: Option<RescueHold>,
    /// The operation whose confirmation is asked, while its button can be pressed.
    pub confirming: Option<DeviceOp>,
    /// What the account's last operation came to, if one was pressed this run.
    pub said: Option<Rescued>,
    /// Whether the emulator's watch would start the emulator again after a stop: what the
    /// stop's confirmation says.
    pub watch: Watch,
    /// What the status left out of what the rows are drawn by: for the log.
    pub unstated: Unstated,
}

/// Whether the Runtime's emulator watch starts an account's emulator again once it is stopped
/// (EMU-W R3, Runtime 0.12.39): it reads `start_emulator` in effect, the file's value or an
/// instruction's, at every poll, and only the account's own pause holds it; a global pause, the
/// 116 stop included, does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
    /// It is on, and the account's own pause does not hold it.
    Restarts,
    /// Whether it is on does not read: no snapshot of this epoch, or not its row.
    Unknown,
    /// It is off, or the account's own pause holds it.
    Stays,
}

impl RescueRows {
    /// Whether `op`'s button is drawn and can be pressed now.
    pub fn offers(&self, op: DeviceOp) -> bool {
        let button = match op {
            DeviceOp::SelfCheck => self.check,
            DeviceOp::EmulatorStart => self.emulator.and_then(|row| row.start),
            DeviceOp::EmulatorStop => self.emulator.and_then(|row| row.stop),
            DeviceOp::EmulatorRestart => self.emulator.and_then(|row| row.restart),
            DeviceOp::GameLaunch => self.game.map(|row| row.launch),
            DeviceOp::GameRestart => self.game.map(|row| row.restart),
        };
        button.is_some_and(|button| button.offered)
    }
}

/// The emulator's row: each of its buttons drawn when its control is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmulatorRow {
    pub start: Option<Button>,
    pub stop: Option<Button>,
    pub restart: Option<Button>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameRow {
    pub launch: Button,
    pub restart: Button,
}

/// One drawn button: whether it can be pressed now, and, while nothing holds every button
/// (`RescueRows::hold`), its own reason when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button {
    pub offered: bool,
    pub why_not: Option<ButtonWhy>,
}

/// Why one button cannot be pressed, of the account's own readings. A reading of the emulator
/// may be stale, so its time is said with it, and one of `unknown` holds nothing back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonWhy {
    /// The emulator was last read not running, at this time (unix ms): no stop, no restart (a
    /// start is the way), and of a discovery-bound instance no check and no game.
    EmulatorOff { at_ms: u64 },
    /// It was last read running: no start.
    EmulatorOn { at_ms: u64 },
    /// It is fenced, or its ladder holds: no game, a check first.
    Fault,
}

/// Why "close this task" cannot be pressed: this Runtime stops no run from here
/// (`StopInstanceRun` is not implemented), by what the instance does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseWhy {
    /// A policy's task or a direct run's: not from here, with this Runtime.
    NotHere,
    /// The host's own startup run, or its recovery ladder: not to be closed.
    SystemFlow,
    /// A reconnect, a self-check, an emulator's control: no task runs.
    NoTask,
}

impl Rescue {
    /// The operation of `alias` out, if any.
    pub fn out(&self, alias: &str) -> Option<DeviceOp> {
        self.slots.get(alias)?.out.map(|(_, op)| op)
    }

    /// Whether the last operation of `alias` cools down.
    pub fn cooling(&self, alias: &str) -> bool {
        self.slots.get(alias).is_some_and(|slot| slot.cooling.is_some())
    }

    /// What the last operation of `alias` came to, if one was pressed this run.
    pub fn said(&self, alias: &str) -> Option<&Rescued> {
        self.slots.get(alias)?.said.as_ref()
    }

    /// The operation of `alias` whose confirmation is asked, if any.
    pub fn confirming(&self, alias: &str) -> Option<DeviceOp> {
        self.confirming.as_ref().filter(|(asked, _)| asked == alias).map(|(_, op)| *op)
    }

    /// The confirmation asked, of whichever account.
    pub fn asked(&self) -> Option<(&str, DeviceOp)> {
        self.confirming.as_ref().map(|(alias, op)| (alias.as_str(), *op))
    }

    /// The confirmation asked goes, whatever it was of.
    pub fn unconfirm(&mut self) {
        self.confirming = None;
    }

    fn ask(&mut self, alias: &str, op: DeviceOp) {
        self.confirming = Some((alias.to_owned(), op));
    }

    fn take_asked(&mut self) -> Option<(String, DeviceOp)> {
        self.confirming.take()
    }

    /// `op` of `alias` pressed: its ticket, unless an operation of it is out or cools down. What
    /// it came to is, for now, that it is being sent; the confirmation asked goes.
    fn begin(&mut self, alias: &str, op: DeviceOp) -> Option<RescueTicket> {
        let slot = self.slots.entry(alias.to_owned()).or_default();
        if slot.out.is_some() || slot.cooling.is_some() {
            return None;
        }
        self.issued += 1;
        let ticket = RescueTicket(self.issued);
        slot.out = Some((ticket, op));
        slot.said = Some(Rescued::Sending(op));
        self.confirming = None;
        Some(ticket)
    }

    /// What the operation of `alias` under `ticket` came to: whether that ticket is still the one
    /// out, which is all that makes it count. Its cooldown runs from now, whatever it came to.
    fn settle(&mut self, alias: &str, ticket: RescueTicket, said: Rescued) -> bool {
        let Some(slot) = self.slots.get_mut(alias) else {
            return false;
        };
        if slot.out.map(|(out, _)| out) != Some(ticket) {
            return false;
        }
        *slot = RescueSlot { out: None, said: Some(said), cooling: Some(ticket) };
        true
    }

    /// The cooldown of `alias`'s operation under `ticket` ran out: whether it was the one that
    /// ran.
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

    /// The Runtime started again: no operation out holds across that, nor the confirmation
    /// asked. Each is let go, says it was out across a restart, and cools down from now. The ones
    /// let go, by alias, for the log and their cooldowns.
    fn restarted(&mut self) -> Vec<(String, RescueTicket, DeviceOp)> {
        self.confirming = None;
        let mut released = Vec::new();
        for (alias, slot) in &mut self.slots {
            if let Some((ticket, op)) = slot.out.take() {
                slot.said = Some(Rescued::Restarted);
                slot.cooling = Some(ticket);
                released.push((alias.clone(), ticket, op));
            }
        }
        released
    }

    /// The rows of `instance`'s bar, `withdrawn` when its gate took the picture down, and `hold`
    /// why every device button is off, if anything is (`Model::rescue_view`); each button by its
    /// own readings only while nothing holds them all. A pause holds none back, whoever's, nor
    /// does a quiet time: an operator's claim goes through a closed gate, and these buttons are a
    /// person's maintenance. A fence holds back only the game's: the check lifts it, and so does
    /// a start of the emulator that succeeds. `watch` is what a stop of its emulator would meet.
    pub fn view(
        &self,
        instance: &InstanceLive,
        withdrawn: bool,
        hold: Option<RescueHold>,
        watch: Watch,
    ) -> RescueRows {
        let alias = instance.alias.as_str();
        let drawn = instance.physical && !withdrawn;
        let controls = instance.emulator_controls;
        let discovered = controls.start || controls.stop || controls.restart;
        let button = |why_not: Option<ButtonWhy>| Button {
            offered: hold.is_none() && why_not.is_none(),
            why_not: why_not.filter(|_| hold.is_none()),
        };
        let off = |applies: bool| {
            instance.emulator_off.filter(|_| applies).map(|at_ms| ButtonWhy::EmulatorOff { at_ms })
        };
        let on = instance.emulator_on.map(|at_ms| ButtonWhy::EmulatorOn { at_ms });
        let faulty = instance.fence.is_some() || instance.ladder_busy;
        let game_why = off(discovered).or_else(|| faulty.then_some(ButtonWhy::Fault));
        let close = instance.activity.as_ref().filter(|_| !withdrawn).map(|activity| {
            match activity.kind {
                ActivityKind::PolicyRun | ActivityKind::DirectRun => CloseWhy::NotHere,
                ActivityKind::HostRun | ActivityKind::Ladder => CloseWhy::SystemFlow,
                ActivityKind::Reconnect
                | ActivityKind::Selfcheck
                | ActivityKind::EmulatorControl => CloseWhy::NoTask,
            }
        });
        let rows = RescueRows {
            check: drawn.then(|| button(off(discovered))),
            check_urgent: instance.fence.is_some(),
            emulator: (drawn && discovered).then(|| EmulatorRow {
                start: controls.start.then(|| button(on)),
                stop: controls.stop.then(|| button(off(true))),
                restart: controls.restart.then(|| button(off(true))),
            }),
            game: drawn.then(|| GameRow { launch: button(game_why), restart: button(game_why) }),
            close,
            hold,
            confirming: None,
            said: self.said(alias).cloned(),
            watch,
            unstated: instance.unstated,
        };
        let confirming = self.confirming(alias).filter(|op| rows.offers(*op));
        RescueRows { confirming, ..rows }
    }
}

/// What one operation came to, as its thread hands it back.
enum Landing {
    Answer(DeviceOutcome),
    /// The thread panicked: whether the request got out is not known.
    Panicked(String),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

impl Landing {
    /// What `op` came to, before its words, as it lands at `now_ms`.
    fn said(self, op: DeviceOp, now_ms: u64) -> Rescued {
        match self {
            Self::Answer(DeviceOutcome::Done(done)) => {
                Rescued::Done { op, at_ms: now_ms, done, unread: false }
            }
            Self::Answer(DeviceOutcome::DoneUnread(_)) => {
                Rescued::Done { op, at_ms: now_ms, done: DeviceDone::Game, unread: true }
            }
            Self::Answer(DeviceOutcome::Refused(refusal)) => Rescued::Refused {
                op,
                host: refusal.failure.host.map(|(code, _)| code),
                class: refusal.failure.runtime_code,
                holder: refusal.holder,
                denied: refusal.denied,
            },
            Self::Answer(DeviceOutcome::NotSent(_)) => Rescued::NotSent(op),
            Self::Answer(DeviceOutcome::Unconfirmed(_)) | Self::Panicked(_) => {
                Rescued::Unconfirmed(op)
            }
            Self::NoThread(error) => Rescued::NoThread { error },
        }
    }

    /// Into the technical log, codes and all: what the Runtime did without lighting up the log's
    /// place on the windows; the rest as the problems they are, a game's whose records were not
    /// read back included.
    fn log(&self, alias: &str, op: DeviceOp, ticket: RescueTicket) {
        let thread = thread(op);
        let prefix = format!("ticket {} of {alias}, {}", ticket.number(), name(op));
        let line = match self {
            Self::Answer(DeviceOutcome::Done(done)) => {
                report::log_quiet(thread, &format!("{prefix}: done: {done:?}"));
                return;
            }
            Self::Answer(DeviceOutcome::DoneUnread(failure)) => {
                format!("{prefix}: done, its records not read back: {}", failure_line(failure))
            }
            Self::Answer(DeviceOutcome::Refused(refusal)) => {
                let how = if refusal.denied { "denied" } else { "failed" };
                format!(
                    "{prefix}: refused, {how}, holder {:?}: {}",
                    refusal.holder,
                    failure_line(&refusal.failure)
                )
            }
            Self::Answer(DeviceOutcome::NotSent(failure)) => {
                format!("{prefix}: not sent: {}", failure_line(failure))
            }
            Self::Answer(DeviceOutcome::Unconfirmed(failure)) => {
                format!("{prefix}: unconfirmed, not sent again: {}", failure_line(failure))
            }
            Self::Panicked(message) => {
                format!("{prefix}: panicked, unconfirmed, not sent again: {message}")
            }
            Self::NoThread(error) => format!("{prefix}: no thread, not sent: {error}"),
        };
        report::log(thread, &line);
    }
}

/// A button of the rescue bar, of the account whose picture shows.
pub fn pressed(ui: &Ui, op: DeviceOp) {
    let alias = ui.model.borrow().picture_account().map(str::to_owned);
    if let Some(alias) = alias {
        press(ui, &alias, op);
    }
}

/// `op` of `alias`'s device pressed, from the rescue bar or from another place of the window
/// (UI-3d, 3g1), which goes by the same view: nothing when its button cannot be pressed now. One
/// that takes two presses asks first, whatever was asked before; another goes out once.
pub fn press(ui: &Ui, alias: &str, op: DeviceOp) {
    let offered = ui.model.borrow().rescue_view(alias).is_some_and(|view| view.offers(op));
    if !offered {
        return;
    }
    if asks_first(op) {
        ui.model.borrow_mut().rescue_mut().ask(alias, op);
        repaint(ui);
        return;
    }
    send(ui, alias, op);
}

/// The confirmation's button: the operation it asked about goes out once, when its button can
/// still be pressed; the question goes either way.
pub fn confirmed(ui: &Ui) {
    let asked = ui.model.borrow_mut().rescue_mut().take_asked();
    let Some((alias, op)) = asked else {
        return;
    };
    let offered = ui.model.borrow().rescue_view(&alias).is_some_and(|view| view.offers(op));
    if offered {
        send(ui, &alias, op);
    } else {
        repaint(ui);
    }
}

/// The confirmation's "cancel": the question goes, and nothing is sent.
pub fn cancelled(ui: &Ui) {
    ui.model.borrow_mut().rescue_mut().unconfirm();
    repaint(ui);
}

/// `op` of `alias`, once, on a thread of its own, under a ticket of the account's. A thread the
/// system refuses is the press's answer at once, in red; a panic on it is an answer as well:
/// with none at all the buttons would stay off for good.
fn send(ui: &Ui, alias: &str, op: DeviceOp) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let Some(ticket) = ui.model.borrow_mut().rescue_mut().begin(alias, op) else {
        return;
    };
    let root = launcher.state_root.clone();
    let worker_alias = alias.to_owned();
    let spawned = spawn_worker(thread(op), move || {
        let alias = worker_alias;
        let landing = match catch_unwind(AssertUnwindSafe(|| operate(&root, &alias, op))) {
            Ok(outcome) => Landing::Answer(outcome),
            Err(panic) => Landing::Panicked(panic_message(&*panic)),
        };
        landing.log(&alias, op, ticket);
        report::post(thread(op), move || {
            if let Some(ui) = ui::current() {
                landed(&ui, &alias, op, ticket, landing);
            }
        });
    });
    match spawned {
        Ok(()) => repaint(ui),
        Err(error) => {
            let landing = Landing::NoThread(error.to_string());
            landing.log(alias, op, ticket);
            landed(ui, alias, op, ticket, landing);
        }
    }
}

/// An answer, on the event loop. One whose ticket is no longer the one out, which a restart let
/// go, changes nothing shown: only the log had it. One that counts is a write: the reads begun
/// before it are let go, and read again.
fn landed(ui: &Ui, alias: &str, op: DeviceOp, ticket: RescueTicket, landing: Landing) {
    let said = landing.said(op, paint::now_ms());
    let counted = ui.model.borrow_mut().rescue_mut().settle(alias, ticket, said);
    if !counted {
        let line = format!(
            "ticket {} of {alias}, {}: its answer came after a restart let it go, and counts for \
             nothing",
            ticket.number(),
            name(op)
        );
        report::log_quiet(thread(op), &line);
        ui.paint_log_notes();
        return;
    }
    cool_down(alias.to_owned(), ticket);
    super::wrote(ui);
    repaint(ui);
    ui.paint_log_notes();
}

/// The Runtime started again: every operation out is let go, says so, and cools down from now;
/// the confirmation asked goes. Painting is the caller's.
pub fn restarted(ui: &Ui) {
    let released = ui.model.borrow_mut().rescue_mut().restarted();
    for (alias, ticket, op) in released {
        let line = format!(
            "ticket {} of {alias}, {}: let go, the Runtime started again; what came of it is not \
             known",
            ticket.number(),
            name(op)
        );
        report::log_quiet(thread(op), &line);
        cool_down(alias, ticket);
    }
}

/// `alias`'s operation under `ticket` cools down: its buttons come back once this runs out.
fn cool_down(alias: String, ticket: RescueTicket) {
    Timer::single_shot(RESCUE_COOLDOWN, move || {
        let Some(ui) = ui::current() else {
            return;
        };
        let cooled = ui.model.borrow_mut().rescue_mut().cooled(&alias, ticket);
        if cooled {
            repaint(&ui);
        }
    });
}

/// What a press, an answer or a cooldown may have changed in the window: the rescue bar, and the
/// picture's refresh, which an operation out holds back; `paint::refresh` paints both.
fn repaint(ui: &Ui) {
    paint::refresh(ui);
}
