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
//! A third thread runs the Runtime's `actingctl` for the task names (`tools`): that
//! is no read of the running Runtime, and it runs only when asked.
//!
//! What a person writes from the top bar is in `actions`: each write on a thread and
//! a connection of its own, sent once. Once one lands, a read begun before that no
//! longer counts: it is let go, and read again.
//!
//! A read that fails is no event to log each time: the log gets a change, as it
//! happens, so a failure that repeats every few seconds is one line, and its end
//! another.

pub mod actions;
pub mod card;
pub mod model;
pub mod paint;
pub mod read;
pub mod tools;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acui_source::workbench::{ExplainRead, LiveRead};
use acui_source::{ClientFailure, RuntimeFacts};
use slint::{ComponentHandle, Timer, TimerMode};

use self::model::{ExplainNews, Link};
use self::read::{Lane, Landing, Poke, Tally};
use self::tools::NamesRead;
use crate::launcher;
use crate::report;
use crate::ui::Ui;

/// How often the event loop looks whether the workbench is minimized.
const SHOWN_CHECK: Duration = Duration::from_secs(1);

/// The reading threads of this run. Letting this go stops them.
pub struct Reading {
    live: Option<Sender<Poke>>,
    slow: Option<Sender<Poke>>,
    /// The thread that runs `actingctl`, or why there is none.
    tools: Result<Sender<Poke>, String>,
    /// A run of it was asked for while minimized: it runs when the window is back.
    names_owed: bool,
    /// Whether the workbench is not minimized; `SHOWN_CHECK` keeps it.
    shown: Arc<AtomicBool>,
    tally: Arc<Tally>,
    /// How many writes have landed. A read states what it was when it began.
    write_gen: Arc<AtomicU64>,
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

    /// A run of `actingctl packs`, now or once the window is back; the error says
    /// why no thread can do it.
    fn ask_names(&mut self) -> Result<(), String> {
        if !self.shown.load(Ordering::SeqCst) {
            self.names_owed = true;
            return Ok(());
        }
        self.names_owed = false;
        let sender = self.tools.as_ref().map_err(String::clone)?;
        if sender.send(Poke).is_err() {
            let why = format!("{} ended", tools::THREAD);
            self.tools = Err(why.clone());
            return Err(why);
        }
        Ok(())
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
    let write_gen = Arc::new(AtomicU64::new(0));
    let spawned = |lane: Lane| {
        let root = launcher.state_root.clone();
        let (shown, tally, write_gen) =
            (Arc::clone(&shown), Arc::clone(&tally), Arc::clone(&write_gen));
        match read::spawn(lane, root, shown, tally, write_gen) {
            Ok(sender) => Some(sender),
            Err(error) => {
                lane_failed(ui, lane, error.to_string());
                None
            }
        }
    };
    let (live, slow) = (spawned(Lane::Live), spawned(Lane::Slow));
    let tools = tools::spawn(launcher.tool_paths()).map_err(|error| error.to_string());
    let shown_check = Timer::default();
    shown_check.start(TimerMode::Repeated, SHOWN_CHECK, check_shown);
    *ui.reading.borrow_mut() = Some(Reading {
        live,
        slow,
        tools,
        names_owed: false,
        shown,
        tally,
        write_gen,
        _shown_check: shown_check,
    });
    ask_names(ui);
}

/// After the event loop: the threads are told to end, and the log gets how many
/// reads this run made. A read still out ends on its own; what it hands over then
/// goes nowhere, and the log says that too (`report::post`).
pub fn stop(ui: &Ui) {
    let reading = ui.reading.borrow_mut().take();
    if let Some(reading) = reading {
        report::log_quiet("acui-reads", &reading.tally.line());
    }
}

/// A `Status` read now: a request of the launcher's is over.
pub fn poke_live(ui: &Ui) {
    poke(ui, Lane::Live);
}

/// A write's answer landed. A read begun before this no longer counts, whatever it
/// found: it may say what the write changed. Counted here and not at the press, since
/// a read begun after the press and before the Runtime acted would otherwise count
/// and lay the old state over the new. The reads poked now are all of after it.
pub fn wrote(ui: &Ui) {
    if let Some(reading) = ui.reading.borrow().as_ref() {
        reading.write_gen.fetch_add(1, Ordering::SeqCst);
    }
    poke(ui, Lane::Live);
    poke(ui, Lane::Slow);
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

/// A run of `actingctl packs`; with no thread to do it, that is its answer.
fn ask_names(ui: &Ui) {
    let asked = ui.reading.borrow_mut().as_mut().map_or(Ok(()), |reading| reading.ask_names());
    if let Err(why) = asked {
        let failure = tools::ToolFailure::NoThread(why);
        names_landed(ui, NamesRead::Failed(failure), Duration::ZERO);
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
    paint::actions(ui);
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
        let owed = ui.reading.borrow().as_ref().is_some_and(|reading| reading.names_owed);
        if owed {
            ask_names(&ui);
        }
    }
}

/// A reading thread's result, on the event loop. `generation` is what `wrote` had
/// counted when the read began. A panic is no finding about the Runtime, so no write
/// makes it old.
fn landed(landing: Landing, generation: u64) {
    let Some(ui) = crate::ui::current() else {
        return;
    };
    let writes = ui
        .reading
        .borrow()
        .as_ref()
        .map_or(0, |reading| reading.write_gen.load(Ordering::SeqCst));
    if generation < writes {
        let lane = match &landing {
            Landing::Live(_) => Some(Lane::Live),
            Landing::Explain(_) => Some(Lane::Slow),
            Landing::Panicked { .. } => None,
        };
        if let Some(lane) = lane {
            poke(&ui, lane);
            return;
        }
    }
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
    if landed.names_stale {
        ask_names(ui);
        ui.paint_names_note();
    }
    // Before the cards: the explanation's clock starts here, and they go by it.
    if landed.connected || landed.epoch_changed {
        poke(ui, Lane::Slow);
    }
    paint::cards(ui);
    paint::actions(ui);
    if landed.fault_cleared {
        ui.paint_notices();
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

/// A run of `actingctl packs` is over, on the event loop.
fn names_read(read: NamesRead, took: Duration) {
    if let Some(ui) = crate::ui::current() {
        names_landed(&ui, read, took);
    }
}

/// Every run goes in the log: they are few, and the limit on them is checked there.
fn names_landed(ui: &Ui, read: NamesRead, took: Duration) {
    report::log_quiet(tools::THREAD, &format!("{} ({} ms)", read.log_line(), took.as_millis()));
    ui.model.borrow_mut().apply_names(read);
    ui.paint_names_note();
    paint::cards(ui);
    ui.paint_log_notes();
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
        (_, Link::Online { pid, epoch }) => {
            report::log_quiet(Lane::Live.name(), &format!("online: pid {pid}, owner epoch {epoch}"));
            return;
        }
        (_, Link::Absent(failure)) => {
            report::log_quiet(Lane::Live.name(), &format!("no Runtime: {}", failure_line(failure)));
            return;
        }
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
