// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's reads of the running Runtime. Two threads read for the cards and the top
//! bar (`read`), a third for the picture (`preview`); their results land on the event loop,
//! and what counts of them goes into the `Model`.
//!
//! 1. `Model` is the workbench's only state, and changes only on the event loop.
//! 2. Neither `Model` nor any view of it holds words: a language switch only paints
//!    again.
//! 3. `acui-app` uses no Runtime crate: wire types end in `acui-source`.
//! 4. Nothing here calls `Session::open`, `OnlineSource::pin`, `source.query` or
//!    `monitor_status`. The two reading threads read `Status`, `ScheduleExplanation`
//!    without candidates and, while an account's daily task list shows, of that one account
//!    with them (UI-3f1) and its `PackScheduleView` (UI-3f2), and `RuntimeFactSnapshot`, none of
//!    which walks the ledger or takes a ledger record. The picture's thread is the one
//!    exception: it reads ledger pages, which take no record either, and frames' material,
//!    whose read takes none when it succeeds or finds the file gone and one for any other
//!    failure; so a frame it failed to read is not read again, and it reads only while the
//!    picture shows (`preview`).
//! 5. How many writes have landed (`wrote`) is no concern of the picture: a frame is
//!    history, which no write changes, so no picture read is let go for a write, and
//!    nothing the picture does counts as one, not even its refresh (`refresh`): the frame
//!    that captures is one more record for the picture to read.
//! 6. A person's operation of a device (`rescue`) is a write: its answer counts one (`wrote`),
//!    since it changes what the reads show of the account, its activity, its emulator's
//!    reading and its fence. Rule 5 stands: a refresh is still none. Neither goes out while
//!    the other, or a switch of that device's backends, is out for the same account
//!    (`Model::device_busy`, the one place that says so). A person's stop of an account's run
//!    (`stop`, UI-3e2) is a write too, its answer counting one, but no work of the device's: it
//!    is not in `device_busy`, and neither holds back the other. So is a person's switch of one
//!    task package of an account on its daily task list (`switch`, UI-3f2), whose answer counts
//!    one when it may have changed the account's switches or says another tool did; one of them
//!    is out for an account at a time, and holds nothing else back.
//!
//! Another thread runs the Runtime's `actingctl` (`tools`), only when asked: `packs` for the
//! task names and the picture proportions each game's standard pack declares, and `watchdog
//! status` as the settings page is gone to, for whether the Runtime is started again when it
//! is gone. Neither is a read of the running Runtime's state.
//!
//! What a person writes from the top bar is in `actions`, and so is the pause of one account
//! and its lift from that account's card: each write on a thread and a connection of its own,
//! sent once. Once one lands, a read begun before that no
//! longer counts: it is let go, and read again. So it is once a `Status` shows over a
//! switch of a device's backends whose answer was lost, which only a snapshot read after
//! that may judge. A person's refresh of the picture is sent
//! the same way, on a ticket of the account's own (`refresh`), and lets no read go.
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
//! the first. Whether the picture thread follows it, and which, is told the thread in one
//! place (`preview::sync`), each time that may change: a `Status` read, a page gone to, the
//! window minimized or back, a card picked, the reading thread gone. Over that picture, the same
//! account's rescue bar (`rescue`), painted with the picture's refresh whenever that is, and once
//! more as a fact snapshot lands, which its question before an emulator's stop reads.
//!
//! The right column has two tabs (UI-3f1), the `Model`'s too (`tab_picked`): the live view and
//! that same account's daily task list (`tasks`), with a switch for each of its task packages
//! (UI-3f2). The picture reads only under its own tab, and the list and its switches only under
//! its: which account's list the slow thread reads, if any, and whether its switches too, is told
//! it in one place (`tasks_sync`), each time that may change, as for the picture. A list shown
//! anew, or of another account, lets go of what was read and is read at once, its switches too.
//!
//! A read that fails is no event to log each time: the log gets a change, as it
//! happens, so a failure that repeats every few seconds is one line, and its end
//! another.

pub mod actions;
pub mod card;
pub mod model;
pub mod paint;
pub mod picture;
pub mod preview;
pub mod read;
pub mod refresh;
pub mod rescue;
pub mod settings;
pub mod stop;
pub mod switch;
pub mod tasks;
pub mod tools;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acui_source::workbench::pack::PacksRead;
use acui_source::workbench::{ConfigRead, ExplainRead, LiveRead, StartRead, TasksRead};
use acui_source::{ClientFailure, RuntimeFacts};
use slint::{ComponentHandle, Timer, TimerMode};

use self::model::{Link, News, Page, RightTab};
use self::preview::{Order, PreviewTally};
use self::read::{Lane, Landing, Listed, Poke, Read, Tally, TasksFor, START_RETRY, START_TRIES};
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
    /// The picture's thread, or why there is none; and what it read.
    preview: Result<Sender<Order>, String>,
    preview_tally: Arc<PreviewTally>,
    /// A run of `packs` was asked for while minimized: it runs when the window is back.
    names_owed: bool,
    /// So was one of `watchdog status`: it runs then if the settings page still shows.
    autostart_owed: bool,
    /// Whether the workbench is not minimized; `SHOWN_CHECK` keeps it.
    shown: Arc<AtomicBool>,
    /// The account whose daily task list the slow thread reads, while it shows (`tasks_sync`).
    tasks_for: TasksFor,
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

    /// `order` to the picture's thread. The error says that thread is gone, once: one that
    /// never started was said when it did not, and nothing is sent to it.
    fn send_preview(&mut self, order: Order) -> Result<(), String> {
        let Ok(sender) = &self.preview else {
            return Ok(());
        };
        if sender.send(order).is_err() {
            let why = format!("{} ended", preview::THREAD);
            self.preview = Err(why.clone());
            return Err(why);
        }
        Ok(())
    }
}

/// The reading threads, once the workbench is up, the picture's reading a frame at most every
/// `frame_gap`. With no state root there is nothing to read: the workbench says so on its own.
pub fn start(ui: &Ui, frame_gap: Duration) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let shown = Arc::new(AtomicBool::new(true));
    let tasks_for: TasksFor = Arc::new(Mutex::new(None));
    let tally = Arc::new(Tally::default());
    let write_gen = Arc::new(AtomicU64::new(0));
    let spawned = |lane: Lane| {
        let root = launcher.state_root.clone();
        let (shown, tasks_for, tally, write_gen) = (
            Arc::clone(&shown),
            Arc::clone(&tasks_for),
            Arc::clone(&tally),
            Arc::clone(&write_gen),
        );
        match read::spawn(lane, root, shown, tasks_for, tally, write_gen) {
            Ok(sender) => Some(sender),
            Err(error) => {
                lane_failed(ui, lane, error.to_string());
                None
            }
        }
    };
    let (live, slow) = (spawned(Lane::Live), spawned(Lane::Slow));
    let tools = tools::spawn(launcher.tool_paths()).map_err(|error| error.to_string());
    let preview_tally = Arc::new(PreviewTally::default());
    let generation = ui.model.borrow().pictures().generation();
    let root = launcher.state_root.clone();
    let preview = preview::spawn(root, generation, Arc::clone(&preview_tally), frame_gap)
        .map_err(|error| error.to_string());
    if let Err(why) = &preview {
        preview::thread_failed(ui, why.clone());
    }
    let shown_check = Timer::default();
    shown_check.start(TimerMode::Repeated, SHOWN_CHECK, check_shown);
    *ui.reading.borrow_mut() = Some(Reading {
        live,
        slow,
        tools,
        preview,
        preview_tally,
        names_owed: false,
        autostart_owed: false,
        shown,
        tasks_for,
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
        report::log_quiet(preview::THREAD, &reading.preview_tally.line());
    }
}

/// A page picked on the top bar, or by `--page` before the window is up: then the
/// window's first paint shows it, and the threads' first round reads for it. Going to
/// the settings reads the fact snapshot now, since the page shows what it says, and only
/// that: the schedule explanation, which waits on the policy lock, is no read of a page.
/// It also runs `watchdog status` once, the only time that runs. The picture reads only on the
/// main page.
pub fn show_page(ui: &Ui, page: Page) {
    if ui.model.borrow().page() == page {
        return;
    }
    ui.model.borrow_mut().set_page(page);
    paint::page(ui);
    preview::sync(ui);
    tasks_sync(ui);
    paint::picture(ui);
    paint::tasks(ui);
    if page == Page::Settings {
        // A device's operation or refresh just pressed on the main page holds its group's switch
        // now, not only from the next `Status`'s paint on.
        paint::settings(ui);
        ask(ui, Read::Config.lane(), Poke::Only(Read::Config));
        ask_autostart(ui);
    }
}

/// A card picked, by a click or from the keyboard: its picture is the one the right column
/// shows from now on, and the one the picture thread follows; so is its daily task list, under
/// that tab, whichever tab shows.
pub fn account_picked(ui: &Ui, alias: &str) {
    let changed = ui.model.borrow_mut().pick_account(alias);
    if changed {
        preview::sync(ui);
        tasks_sync(ui);
        paint::cards(ui);
        paint::picture(ui);
        paint::tasks(ui);
    }
}

/// A tab of the right column picked (UI-3f1), by a click or from the keyboard: the live view,
/// or the account's daily task list. Only the one shown reads.
pub fn tab_picked(ui: &Ui, tab: RightTab) {
    let changed = ui.model.borrow_mut().pick_tab(tab);
    if changed {
        preview::sync(ui);
        tasks_sync(ui);
        paint::picture(ui);
        paint::tasks(ui);
    }
}

/// The daily task list's line of the account's own tasks, those no package was found for,
/// opened or closed.
pub fn tasks_fold_toggled(ui: &Ui) {
    ui.model.borrow_mut().tasks_fold_toggled();
    paint::tasks(ui);
}

/// Which account's daily task list the slow thread is to read now, if any (`Model::tasks_wanted`),
/// and whether its switches too (`Model::packs_wanted`), into its slot (`TasksFor`), the one place
/// that says so: a list shown anew, or of another account, starts afresh and is read at once, its
/// switches with it. Each time this may change: a `Status` read, a page or a tab gone to, the
/// window minimized or back, a card picked, a reading thread gone, a Runtime that said it has no
/// switches.
fn tasks_sync(ui: &Ui) {
    if tasks_follow(ui) {
        ask(ui, Read::Tasks.lane(), Poke::List);
    }
}

/// `tasks_sync` but for its poke: whether the list is to be read at once. A caller that pokes the
/// slow thread for all its reads right after asks for it there, so that it is read once.
fn tasks_follow(ui: &Ui) -> bool {
    let (listed, changed) = {
        let mut model = ui.model.borrow_mut();
        let wanted = model.tasks_wanted();
        let changed = model.tasks_follow(wanted.as_deref());
        let packs = model.packs_wanted();
        (wanted.map(|alias| Listed { alias, packs }), changed)
    };
    if let Some(reading) = ui.reading.borrow().as_ref() {
        let mut slot = reading.tasks_for.lock().unwrap_or_else(|held| held.into_inner());
        if *slot != listed {
            *slot = listed.clone();
        }
    }
    changed && listed.is_some()
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
    if lane == Read::Tasks.lane() && poke.asks(Read::Tasks) {
        ui.model.borrow_mut().tasks_asked(Instant::now());
    }
    if lane == Read::Packs.lane() && poke.asks(Read::Packs) {
        ui.model.borrow_mut().packs_asked(Instant::now());
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
        // No read will say what came of a stop that waits for one.
        stop::lane_lost(ui);
        ui.launcher_view.borrow_mut().reading_failed(error);
        // No `Status` says what is online any more: the picture stops, and the task list.
        preview::sync(ui);
        tasks_sync(ui);
    }
    ui.paint_notices();
    ui.paint_accounts_note();
    paint::cards(ui);
    paint::picture(ui);
    paint::tasks(ui);
    paint::actions(ui);
    paint::preset(ui);
    paint::settings(ui);
    paint::start(ui);
    launcher::paint(ui);
}

/// A minimized workbench reads less (`read`), and the picture and the task list nothing; one
/// that comes back reads at once. Each second the picture shows, its corner says how old the
/// frame is.
fn check_shown() {
    let Some(ui) = crate::ui::current() else {
        return;
    };
    let Some(window) = ui.workbench() else {
        return;
    };
    let shown = !window.window().is_minimized();
    let changed = ui.model.borrow_mut().set_shown(shown);
    let mut list_fresh = false;
    if changed {
        preview::sync(&ui);
        list_fresh = tasks_follow(&ui);
        paint::picture(&ui);
        paint::tasks(&ui);
    } else if shown {
        paint::picture_age(&ui);
    }
    let came_back = ui
        .reading
        .borrow()
        .as_ref()
        .is_some_and(|reading| !reading.shown.swap(shown, Ordering::SeqCst) && shown);
    if !came_back && list_fresh {
        ask(&ui, Read::Tasks.lane(), Poke::List);
    }
    if came_back {
        // Every read of the slow thread, the task list's and its switches' among them while it
        // shows.
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
        Landing::Tasks { alias, read } => tasks_landed(&ui, &alias, read),
        Landing::Packs { alias, read } => packs_landed(&ui, &alias, read),
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
    // Before the slow thread is poked: online or not, the account gone or listed.
    let list_fresh = tasks_follow(ui);
    // What was asked for on the command line and cannot be had is said, not passed over.
    if let Some(alias) = &landed.unknown_account {
        report::log(
            "acui-picture",
            &format!("--account {alias}: no account has that alias; the first one's picture shows"),
        );
    }
    // Before the cards: the explanation's clock starts here, and they go by it. Every read of the
    // slow thread, the task list's among them while it shows; and so does a write's (`wrote`).
    if landed.connected || landed.epoch_changed {
        poke(ui, Lane::Slow);
        ask_start(ui);
    } else if list_fresh && !landed.switch_over {
        ask(ui, Read::Tasks.lane(), Poke::List);
    }
    // No refresh of the picture out holds across a restart, nor any operation of a device, nor a
    // switch of a task.
    if landed.epoch_changed {
        refresh::restarted(ui);
        rescue::restarted(ui);
        switch::restarted(ui);
    }
    // A stop of a run is judged by a read online only, of its own epoch or another: its follow-up,
    // or a restart's letting it go (`stop`).
    stop::status_read(ui);
    // An unconfirmed switch shown over: only a snapshot begun after this says what came of it.
    if landed.switch_over {
        wrote(ui);
    }
    // Online or not, another epoch, the account gone, a task begun or over.
    preview::sync(ui);
    paint::cards(ui);
    paint::picture(ui);
    paint::tasks(ui);
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
    // The task list says the account's state above its rows as its card does.
    paint::tasks(ui);
}

/// The task list's read of `alias`, as the cards' explanation is taken: a failure and its end
/// each one line of the log, not every failed read; and each reason code it has no words for,
/// once, whether a row says it or not (`paint::tasks_unlisted`).
fn tasks_landed(ui: &Ui, alias: &str, read: Result<TasksRead, ClientFailure>) {
    if let Ok(read) = &read {
        paint::tasks_unlisted(ui, read);
    }
    let landed = ui.model.borrow_mut().apply_tasks(alias, read, Instant::now());
    let thread = Lane::Slow.name();
    match &landed.news {
        News::Nothing => {}
        News::Failed(failure) => {
            let line = format!("task list of {alias} failed: {}", failure_line(failure));
            report::log(thread, &line);
        }
        News::Recovered => report::log(thread, &format!("task list of {alias} read again")),
    }
    if landed.fault_cleared {
        ui.paint_notices();
    }
    paint::tasks(ui);
}

/// The switches' read of `alias` (UI-3f2), taken as the list's is: a failure and its end each one
/// line of the log. A Runtime that said it has none is said once for its owner epoch, and the slow
/// thread reads them no more in it (`tasks_sync`); a view that names no version of the account's own
/// scope, which the contract never leaves out, is said once too, and has no switch.
fn packs_landed(ui: &Ui, alias: &str, read: Result<PacksRead, ClientFailure>) {
    let landed = ui.model.borrow_mut().apply_packs(alias, read, Instant::now());
    let thread = Lane::Slow.name();
    match &landed.news {
        News::Nothing => {}
        News::Failed(failure) => {
            let line = format!("task switches of {alias} failed: {}", failure_line(failure));
            report::log(thread, &line);
        }
        News::Recovered => report::log(thread, &format!("task switches of {alias} read again")),
    }
    if let Some(epoch) = &landed.unsupported {
        let line = format!(
            "the Runtime of owner epoch {epoch} has no task switches (PackScheduleView \
             not_implemented or unknown): not read again in it"
        );
        report::log_quiet(thread, &line);
        tasks_sync(ui);
    }
    if landed.unversioned {
        let line = format!(
            "the task switches of {alias} name no version of the account's own scope: no switch \
             is offered"
        );
        if ui.model.borrow_mut().first_told(&line) {
            report::log(thread, &line);
        }
    }
    if landed.fault_cleared {
        ui.paint_notices();
    }
    paint::tasks(ui);
}

/// The servers are on the cards, the version says whether the finishing count holds,
/// the preset is on the top bar, and the rest of the settings on the settings page; and
/// whether the emulator's watch starts an emulator again, on the rescue bar's question before
/// a stop. A read that answers is no event for the log, only a change of how it went.
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
    paint::rescue(ui);
}

fn panicked(ui: &Ui, read: Read, message: String) {
    let line = format!("read panicked: {message}");
    if ui.model.borrow_mut().read_panicked(read, message) {
        report::log(read.lane().name(), &line);
        ui.paint_notices();
        paint::cards(ui);
        paint::tasks(ui);
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

/// Every run goes in the log: they are few, and the limit on them is checked there. The
/// picture's hint of its proportions goes by the same run.
fn names_landed(ui: &Ui, read: NamesRead, took: Duration) {
    report::log_quiet(tools::THREAD, &format!("{} ({} ms)", read.log_line(), took.as_millis()));
    ui.model.borrow_mut().apply_names(read);
    ui.paint_names_note();
    paint::cards(ui);
    paint::picture(ui);
    paint::tasks(ui);
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
