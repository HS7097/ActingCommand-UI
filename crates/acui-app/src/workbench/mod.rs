// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's reads of the running Runtime. Two threads read (`read`), their
//! results land on the event loop, and what counts of them goes into the `Model`.
//!
//! 1. `Model` is the workbench's only state, and changes only on the event loop.
//! 2. Neither `Model` nor any view of it holds words: a language switch only paints
//!    again.
//! 3. `acui-app` uses no Runtime crate: wire types end in `acui-source`.
//! 4. Nothing here calls `Session::open`, `OnlineSource::pin`, `source.query` or
//!    `monitor_status`: the reads are `Status` and `ScheduleExplanation` without
//!    candidates, neither of which walks the ledger or takes a ledger record.
//!
//! A read that fails is no event to log each time: the log gets a change, as it
//! happens, so a failure that repeats every few seconds is one line, and its end
//! another.

pub mod card;
pub mod model;
pub mod paint;
pub mod read;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acui_source::workbench::{ExplainRead, LiveRead};
use acui_source::{ClientFailure, RuntimeFacts};
use slint::{ComponentHandle, Timer, TimerMode};

use self::model::{ExplainNews, Link};
use self::read::{Lane, Landing, Poke, Tally};
use crate::launcher;
use crate::report;
use crate::ui::Ui;

/// How often the event loop looks whether the workbench is minimized.
const SHOWN_CHECK: Duration = Duration::from_secs(1);

/// The reading threads of this run. Letting this go stops them.
pub struct Reading {
    live: Option<Sender<Poke>>,
    slow: Option<Sender<Poke>>,
    /// Whether the workbench is not minimized; `SHOWN_CHECK` keeps it.
    shown: Arc<AtomicBool>,
    tally: Arc<Tally>,
    _shown_check: Timer,
}

impl Reading {
    /// Whether `lane`'s thread turned out to be gone.
    fn poke(&mut self, lane: Lane) -> bool {
        let slot = match lane {
            Lane::Live => &mut self.live,
            Lane::Slow => &mut self.slow,
        };
        let gone = slot.as_ref().is_some_and(|sender| sender.send(Poke).is_err());
        if gone {
            *slot = None;
        }
        gone
    }
}

/// The reading threads, once the workbench is up. With no state root there is
/// nothing to read: the workbench says so on its own.
pub fn start(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let shown = Arc::new(AtomicBool::new(true));
    let tally = Arc::new(Tally::default());
    let spawned = |lane: Lane| {
        let root = launcher.state_root.clone();
        match read::spawn(lane, root, Arc::clone(&shown), Arc::clone(&tally)) {
            Ok(sender) => Some(sender),
            Err(error) => {
                lane_failed(ui, lane, error.to_string());
                None
            }
        }
    };
    let (live, slow) = (spawned(Lane::Live), spawned(Lane::Slow));
    let shown_check = Timer::default();
    shown_check.start(TimerMode::Repeated, SHOWN_CHECK, check_shown);
    *ui.reading.borrow_mut() = Some(Reading { live, slow, shown, tally, _shown_check: shown_check });
}

/// After the event loop: the threads are told to end, and the log gets how many
/// reads this run made. A read still out ends on its own; what it hands over then
/// goes nowhere, and the log says that too (`report::post`).
pub fn stop(ui: &Ui) {
    let reading = ui.reading.borrow_mut().take();
    if let Some(reading) = reading {
        report::log("acui-reads", &reading.tally.line());
    }
}

/// A `Status` read now: a request of the launcher's is over.
pub fn poke_live(ui: &Ui) {
    poke(ui, Lane::Live);
}

fn poke(ui: &Ui, lane: Lane) {
    if lane == Lane::Slow {
        ui.model.borrow_mut().explain_asked(Instant::now());
    }
    let gone = ui.reading.borrow_mut().as_mut().is_some_and(|reading| reading.poke(lane));
    if gone {
        lane_failed(ui, lane, format!("{} ended", lane.name()));
    }
}

/// A thread that does not read says so in red, and a `Status` thread also on the
/// status line, which would otherwise go on saying what it last found.
fn lane_failed(ui: &Ui, lane: Lane, error: String) {
    report::log(lane.name(), &format!("no reading thread: {error}"));
    ui.model.borrow_mut().lane_lost(lane, error.clone());
    if lane == Lane::Live {
        ui.launcher_view.borrow_mut().reading_failed(error);
    }
    ui.paint_notices();
    ui.paint_accounts_note();
    paint::cards(ui);
    launcher::paint(ui);
}

/// A minimized workbench reads less (`read`); one that comes back reads at once.
fn check_shown() {
    let Some(ui) = crate::ui::current() else {
        return;
    };
    let Some(window) = ui.workbench() else {
        return;
    };
    let shown = !window.window().is_minimized();
    ui.model.borrow_mut().set_shown(shown);
    let came_back = ui
        .reading
        .borrow()
        .as_ref()
        .is_some_and(|reading| !reading.shown.swap(shown, Ordering::SeqCst) && shown);
    if came_back {
        poke(&ui, Lane::Live);
        poke(&ui, Lane::Slow);
    }
}

/// A reading thread's result, on the event loop.
fn landed(landing: Landing) {
    let Some(ui) = crate::ui::current() else {
        return;
    };
    match landing {
        Landing::Live(read) => live_landed(&ui, read),
        Landing::Explain(read) => explain_landed(&ui, read),
        Landing::Panicked { lane, message } => panicked(&ui, lane, message),
    }
    // A thread may have just made the log, with a long read.
    ui.paint_log_notes();
}

fn live_landed(ui: &Ui, read: LiveRead) {
    let probe = probe_of(&read);
    let (landed, link_changed, note_changed) = {
        let mut model = ui.model.borrow_mut();
        let note = model.accounts_note();
        let landed = model.apply_live(read);
        log_link(&landed.previous, model.link());
        let link_changed = landed.previous != *model.link();
        (landed, link_changed, model.accounts_note() != note)
    };
    // Painting the launcher goes through every ledger window: only for news.
    let status_changed = ui.launcher_view.borrow_mut().live(probe);
    if status_changed || link_changed {
        launcher::paint(ui);
    }
    if note_changed {
        ui.paint_accounts_note();
    }
    paint::cards(ui);
    if landed.fault_cleared {
        ui.paint_notices();
    }
    if landed.connected || landed.epoch_changed {
        poke(ui, Lane::Slow);
    }
}

fn explain_landed(ui: &Ui, read: Result<ExplainRead, ClientFailure>) {
    let landed = ui.model.borrow_mut().apply_explain(read, Instant::now());
    match &landed.news {
        ExplainNews::Nothing => {}
        ExplainNews::Failed(failure) => {
            report::log(Lane::Slow.name(), &format!("explanation failed: {}", failure_line(failure)));
        }
        ExplainNews::Recovered => report::log(Lane::Slow.name(), "explanation read again"),
    }
    if landed.fault_cleared {
        ui.paint_notices();
    }
    paint::cards(ui);
}

fn panicked(ui: &Ui, lane: Lane, message: String) {
    let line = format!("read panicked: {message}");
    if ui.model.borrow_mut().read_panicked(lane, message) {
        report::log(lane.name(), &line);
        ui.paint_notices();
        paint::cards(ui);
    }
}

/// What the launcher takes from a `Status` read.
fn probe_of(read: &LiveRead) -> Result<RuntimeFacts, ClientFailure> {
    match read {
        LiveRead::Read(snapshot) => {
            Ok(RuntimeFacts { pid: snapshot.pid, owner_epoch: snapshot.epoch.clone() })
        }
        LiveRead::NotConnected(failure)
        | LiveRead::NoAnswer(failure)
        | LiveRead::Trouble(failure) => Err(failure.clone()),
    }
}

/// A change of the link, for the log. The run's first answer is no news when it is
/// one of the two ordinary ones.
fn log_link(previous: &Link, link: &Link) {
    if previous == link {
        return;
    }
    let line = match (previous, link) {
        (Link::Unknown, Link::Online { .. } | Link::Absent(_)) => return,
        (_, Link::Online { pid, epoch }) => format!("online: pid {pid}, owner epoch {epoch}"),
        (_, Link::Absent(failure)) => format!("no Runtime: {}", failure_line(failure)),
        (_, Link::NoAnswer(failure)) => format!("no answer: {}", failure_line(failure)),
        (_, Link::Trouble(failure)) => format!("trouble: {}", failure_line(failure)),
        (_, Link::Unknown | Link::ThreadFailed) => return,
    };
    report::log(Lane::Live.name(), &line);
}

/// A client failure with its codes, for the log only.
fn failure_line(failure: &ClientFailure) -> String {
    let mut line = format!("{} ({})", failure.code, failure.operation);
    if let Some(code) = &failure.runtime_code {
        line.push_str(&format!(", refused: {code}"));
    }
    if let Some((code, operation)) = &failure.host {
        line.push_str(&format!(", host: {code} ({operation})"));
    }
    line
}
