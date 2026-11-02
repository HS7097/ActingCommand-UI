// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's reads of the running Runtime. Two threads read (`read`), their
//! results land on the event loop, and what counts of them goes into the `Model`.
//!
//! 1. `Model` is the workbench's only state, and changes only on the event loop.
//! 2. Neither `Model` nor any view of it holds words: a language switch only paints
//!    again.
//! 3. `acui-app` uses no Runtime crate: wire types end in `acui-source`.
//! 4. Nothing here calls `Session::open`, `OnlineSource::pin`, `source.query` or
//!    `monitor_status`: the reads are `Status`, `ScheduleExplanation` without
//!    candidates and `RuntimeFactSnapshot`, none of which walks the ledger or takes a
//!    ledger record.
//!
//! A third thread runs the Runtime's `actingctl` (`tools`), only when asked: `packs` for the
//! task names, and `watchdog status` as the settings page is gone to, for whether the Runtime
//! is started again when it is gone. Neither is a read of the running Runtime's state.
//!
//! What a person writes from the top bar is in `actions`: each write on a thread and
//! a connection of its own, sent once. Once one lands, a read begun before that no
//! longer counts: it is let go, and read again.
//!
//! Which of the workbench's two pages shows, the accounts or the settings, is the
//! `Model`'s too (`show_page`). The settings page shows the Runtime's settings as the
//! fact snapshot has them (`settings`), so going there reads it at once; and whether the
//! Runtime is started again when it is gone, which going there asks `watchdog status` once.
//!
//! The start report is read once for each owner epoch (`ask_start`): what it says of the
//! settings an update or a rollback reset is said on the notice, until a person says they
//! read it, and on the settings page.
//!
//! Which account's picture the main page's right column shows is the `Model`'s too
//! (`picture`): the one picked on its card (`account_picked`) or named by `--account`, else
//! the first.
//!
//! A read that fails is no event to log each time: the log gets a change, as it
//! happens, so a failure that repeats every few seconds is one line, and its end
//! another.

pub mod actions;
pub mod card;
pub mod model;
pub mod paint;
pub mod picture;
pub mod read;
pub mod settings;
pub mod tools;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acui_source::workbench::{ConfigRead, ExplainRead, LiveRead, StartRead};
use acui_source::{ClientFailure, RuntimeFacts};
use slint::{ComponentHandle, Timer, TimerMode};

use self::model::{Link, News, Page};
use self::read::{Lane, Landing, Poke, Read, Tally, START_RETRY, START_TRIES};
use self::tools::{Job, NamesRead, ToolFailure, WatchdogRead};
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
    tools: Result<Sender<Job>, String>,
    /// A run of `packs` was asked for while minimized: it runs when the window is back.
    names_owed: bool,
    /// So was one of `watchdog status`: it runs then if the settings page still shows.
    autostart_owed: bool,
    /// Whether the workbench is not minimized; `SHOWN_CHECK` keeps it.
    shown: Arc<AtomicBool>,
    tally: Arc<Tally>,
    /// How many writes have landed. A read states what it was when it began.
    write_gen: Arc<AtomicU64>,
    _shown_check: Timer,
}

impl Reading {
    /// Whether `lane`'s thread turned out to be gone.
    fn poke(&mut self, lane: Lane, poke: Poke) -> bool {
        let slot = match lane {
            Lane::Live => &mut self.live,
            Lane::Slow => &mut self.slow,
        };
        let gone = slot.as_ref().is_some_and(|sender| sender.send(poke).is_err());
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
        self.send_tools(Job::Names)
    }

    /// `job` to the thread that runs `actingctl`; the error says why no thread can run it.
    fn send_tools(&mut self, job: Job) -> Result<(), String> {
        let sender = self.tools.as_ref().map_err(String::clone)?;
        if sender.send(job).is_err() {
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
        autostart_owed: false,
        shown,
        tally,
        write_gen,
        _shown_check: shown_check,
    });
    ask_names(ui);
    // `--page settings`: the page was gone to before there was a thread to ask.
    if ui.model.borrow().page() == Page::Settings {
        ask_autostart(ui);
    }
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

/// A page picked on the top bar, or by `--page` before the window is up: then the
/// window's first paint shows it, and the threads' first round reads for it. Going to
/// the settings reads the fact snapshot now, since the page shows what it says, and only
/// that: the schedule explanation, which waits on the policy lock, is no read of a page.
/// It also runs `watchdog status` once, the only time that runs.
pub fn show_page(ui: &Ui, page: Page) {
    if ui.model.borrow().page() == page {
        return;
    }
    ui.model.borrow_mut().set_page(page);
    paint::page(ui);
    if page == Page::Settings {
        ask(ui, Read::Config.lane(), Poke::Only(Read::Config));
        ask_autostart(ui);
    }
}

/// A card picked, by a click or from the keyboard: its picture is the one the right column
/// shows from now on.
pub fn account_picked(ui: &Ui, alias: &str) {
    let changed = ui.model.borrow_mut().pick_account(alias);
    if changed {
        paint::cards(ui);
        paint::picture(ui);
    }
}

/// A `Status` read now: a request of the launcher's is over.
pub fn poke_live(ui: &Ui) {
    poke(ui, Lane::Live);
}

/// A write's answer landed. A read begun before this no longer counts, whatever it
/// found: it may say what the write changed. Counted here and not at the press, since
/// a read begun after the press and before the Runtime acted would otherwise count
/// and lay the old state over the new. The reads poked now are all of after it, and
/// they are the only reads asked for: `landed` lets an old one go without asking again,
/// which would read the whole lane a second time (a write counts before it pokes, so
/// nothing these pokes start can be old on account of it).
pub fn wrote(ui: &Ui) {
    if let Some(reading) = ui.reading.borrow().as_ref() {
        reading.write_gen.fetch_add(1, Ordering::SeqCst);
    }
    poke(ui, Lane::Live);
    poke(ui, Lane::Slow);
}

/// Every read of `lane`'s thread now.
fn poke(ui: &Ui, lane: Lane) {
    ask(ui, lane, Poke::All);
}

/// What `poke` asks of `lane`'s thread, now.
fn ask(ui: &Ui, lane: Lane, poke: Poke) {
    if lane == Read::Explain.lane() && poke.asks(Read::Explain) {
        ui.model.borrow_mut().explain_asked(Instant::now());
    }
    let gone = ui.reading.borrow_mut().as_mut().is_some_and(|reading| reading.poke(lane, poke));
    if gone {
        lane_failed(ui, lane, format!("{} ended", lane.name()));
    }
}

/// A run of `actingctl packs`; with no thread to do it, that is its answer.
fn ask_names(ui: &Ui) {
    let asked = ui.reading.borrow_mut().as_mut().map_or(Ok(()), |reading| reading.ask_names());
    if let Err(why) = asked {
        let failure = ToolFailure::NoThread(why);
        names_landed(ui, NamesRead::Failed(failure), Duration::ZERO);
    }
}

/// A run of `actingctl watchdog status`, the settings page being gone to: not while one is
/// out, whose answer is on its way; not before there is a thread, when `start` asks; and not
/// while the workbench is minimized, when it is owed until the window is back. With no thread
/// to run it, that is its answer.
fn ask_autostart(ui: &Ui) {
    if !ui.model.borrow().autostart_due() {
        return;
    }
    let sent = {
        let mut reading = ui.reading.borrow_mut();
        let Some(reading) = reading.as_mut() else {
            return;
        };
        if !reading.shown.load(Ordering::SeqCst) {
            reading.autostart_owed = true;
            return;
        }
        reading.autostart_owed = false;
        let ask = ui.model.borrow_mut().autostart_asked();
        reading.send_tools(Job::Watchdog(ask)).map_err(|why| (ask, why))
    };
    match sent {
        Ok(()) => paint::autostart(ui),
        Err((ask, why)) => {
            let read = WatchdogRead::Failed(ToolFailure::NoThread(why));
            autostart_landed(ui, ask, read, Duration::ZERO);
        }
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
    paint::picture(ui);
    paint::actions(ui);
    paint::preset(ui);
    paint::settings(ui);
    paint::start(ui);
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
        let owed = ui.reading.borrow().as_ref().is_some_and(|reading| reading.autostart_owed);
        if owed && ui.model.borrow().page() == Page::Settings {
            ask_autostart(&ui);
        }
    }
}

/// A reading thread's result, on the event loop. `generation` is what `wrote` had
/// counted when the read began. A panic is no finding about the Runtime, so no write
/// makes it old; nor does a write change what the start report says of the start, and
/// that read is not asked again by a write.
fn landed(landing: Landing, generation: u64) {
    let Some(ui) = crate::ui::current() else {
        return;
    };
    let writes = ui
        .reading
        .borrow()
        .as_ref()
        .map_or(0, |reading| reading.write_gen.load(Ordering::SeqCst));
    let ageless = matches!(landing, Landing::Panicked { .. } | Landing::Start(_));
    if generation < writes && !ageless {
        return;
    }
    match landing {
        Landing::Live(read) => live_landed(&ui, read),
        Landing::Explain(read) => explain_landed(&ui, read),
        Landing::Config(read) => config_landed(&ui, read),
        Landing::Start(read) => {
            start_landed(&ui, read.map_err(|failure| failure_line(&failure)));
        }
        Landing::Panicked { read: Read::Start, message } => {
            start_landed(&ui, Err(format!("read panicked: {message}")));
        }
        Landing::Panicked { read, message } => panicked(&ui, read, message),
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
    // What was asked for on the command line and cannot be had is said, not passed over.
    if let Some(alias) = &landed.unknown_account {
        report::log(
            "acui-picture",
            &format!("--account {alias}: no account has that alias; the first one's picture shows"),
        );
    }
    // Before the cards: the explanation's clock starts here, and they go by it.
    if landed.connected || landed.epoch_changed {
        poke(ui, Lane::Slow);
        ask_start(ui);
    }
    paint::cards(ui);
    paint::picture(ui);
    paint::actions(ui);
    paint::preset(ui);
    paint::settings(ui);
    paint::start(ui);
    // A Runtime started again lets go of what the watchdog said.
    if landed.epoch_changed {
        paint::autostart(ui);
    }
    if landed.fault_cleared {
        ui.paint_notices();
    }
}

/// The start report of this owner epoch, when it is due (`Model::start_due`): after a
/// connection or a new epoch, and once more a while after one that failed.
fn ask_start(ui: &Ui) {
    let due = ui.model.borrow().start_due();
    if due {
        ui.model.borrow_mut().start_asked();
        ask(ui, Lane::Slow, Poke::Only(Read::Start));
    }
}

/// The start report a person read the notice of: the notice goes, for this epoch.
pub fn start_acknowledged(ui: &Ui) {
    ui.model.borrow_mut().start_acknowledged();
    paint::start(ui);
}

/// A start report, or why there is none. What it names goes in the log quietly: an update's
/// or a rollback's doing, no fault here. Every failure goes in the log, a few at most for an
/// epoch: one with tries left is asked again after a while, and after the last the settings
/// page says the report cannot be read.
fn start_landed(ui: &Ui, read: Result<StartRead, String>) {
    let thread = Lane::Slow.name();
    match read {
        Ok(read) => {
            let line = format!("start report of owner epoch {}: {}", read.epoch, read.line());
            if ui.model.borrow_mut().apply_start(read) {
                report::log_quiet(thread, &line);
            }
        }
        Err(why) => {
            let tries = ui.model.borrow_mut().start_failed();
            let again = tries < START_TRIES;
            let then = if again { "asked again later" } else { "not asked again this owner epoch" };
            let line = format!("start report failed ({tries} of {START_TRIES}), {then}: {why}");
            report::log(thread, &line);
            if again {
                Timer::single_shot(START_RETRY, || {
                    if let Some(ui) = crate::ui::current() {
                        ask_start(&ui);
                    }
                });
            }
        }
    }
    paint::start(ui);
}

fn explain_landed(ui: &Ui, read: Result<ExplainRead, ClientFailure>) {
    let landed = ui.model.borrow_mut().apply_explain(read, Instant::now());
    match &landed.news {
        News::Nothing => {}
        News::Failed(failure) => {
            report::log(Lane::Slow.name(), &format!("explanation failed: {}", failure_line(failure)));
        }
        News::Recovered => report::log(Lane::Slow.name(), "explanation read again"),
    }
    if landed.fault_cleared {
        ui.paint_notices();
    }
    paint::cards(ui);
}

/// The servers are on the cards, the version says whether the finishing count holds,
/// the preset is on the top bar, and the rest of the settings on the settings page. A
/// read that answers is no event for the log, only a change of how it went.
fn config_landed(ui: &Ui, read: Result<ConfigRead, ClientFailure>) {
    let landed = ui.model.borrow_mut().apply_config(read, Instant::now());
    let thread = Lane::Slow.name();
    match &landed.news {
        News::Nothing => {}
        News::Failed(failure) => {
            report::log(thread, &format!("fact snapshot failed: {}", failure_line(failure)));
        }
        News::Recovered => report::log_quiet(thread, "fact snapshot read again"),
    }
    match &landed.preset {
        News::Nothing => {}
        News::Failed(problem) => {
            report::log(thread, &format!("performance presets unreadable: {}", problem.line()));
        }
        News::Recovered => report::log_quiet(thread, "performance presets read again"),
    }
    if landed.fault_cleared {
        ui.paint_notices();
    }
    paint::cards(ui);
    paint::actions(ui);
    paint::preset(ui);
    paint::settings(ui);
}

fn panicked(ui: &Ui, read: Read, message: String) {
    let line = format!("read panicked: {message}");
    if ui.model.borrow_mut().read_panicked(read, message) {
        report::log(read.lane().name(), &line);
        ui.paint_notices();
        paint::cards(ui);
        paint::actions(ui);
        paint::preset(ui);
        paint::settings(ui);
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

/// A run of `actingctl watchdog status` is over, on the event loop.
fn watchdog_read(ask: u64, read: WatchdogRead, took: Duration) {
    if let Some(ui) = crate::ui::current() {
        autostart_landed(&ui, ask, read, took);
    }
}

/// Every run goes in the log too, as few as they are: quietly when nothing is wrong, and
/// otherwise so that the window shows where the log is, whether the settings page points there
/// or the report says something is wrong that the page does not show (`is_problem`). One asked
/// before the Runtime started again says nothing on the page.
fn autostart_landed(ui: &Ui, ask: u64, read: WatchdogRead, took: Duration) {
    let line = format!("{} ({} ms)", read.log_line(), took.as_millis());
    let problem = read.is_problem();
    if !ui.model.borrow_mut().apply_autostart(ask, read) {
        report::log_quiet(tools::THREAD, &format!("{line}; asked before the Runtime started again"));
        return;
    }
    if problem {
        report::log(tools::THREAD, &line);
    } else {
        report::log_quiet(tools::THREAD, &line);
    }
    paint::autostart(ui);
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
