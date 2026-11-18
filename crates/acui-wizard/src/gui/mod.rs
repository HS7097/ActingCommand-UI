// SPDX-License-Identifier: GPL-3.0-only
//! The wizard's window (Workflow #429 UI-W2b): twelve pages over the back end the command line
//! uses too. The pages only gather choices and run read-only checks (the install set, the
//! folder, MuMu, the other ActingCommand and the phones); nothing is written before "Install" on
//! page 10 but the device bridge the phone page lays into the install folder to find the phones
//! with (`acui_wizard::pre_lay_tools`, said on that page and kept by the install), and then
//! `acui_wizard::install` does it all on a worker thread, step by step. What the usage page
//! ticks decides the pages after the games: without MuMu, the emulator page is passed by and
//! MuMu is not looked for; without Android phones, the phone page; with neither, the instance
//! page too.
//!
//! Everything the window shows is painted from `State` in the language picked: a switch of
//! language paints it all again. Work that waits (hashing the set, a folder check, the emulator
//! search, a folder picker, the install) runs on a thread of its own and hands its result back
//! to the window's thread; a result for a question asked again since is dropped.

mod labels;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use acui_wizard::alias::{alias_problem, default_alias, default_phone_alias, MAX_ALIAS_BYTES};
use acui_wizard::apps::Game;
use acui_wizard::config::{
    backend_problem, handset_backend_problem, CAPTURE_BACKENDS, DEFAULT_BACKEND, HANDSET_ALIAS_LIMIT,
    HANDSET_CAPTURE_BACKENDS, HANDSET_CAPTURE_DEFAULT, HANDSET_TOUCH_BACKENDS, HANDSET_TOUCH_DEFAULT, TOUCH_BACKENDS,
};
use acui_wizard::devices::{self, row_use, Asked, DeviceRow, RowUse};
use acui_wizard::key_salt::KeySalt;
use acui_wizard::log::LOG_FILE;
use acui_wizard::manifest::Program;
use acui_wizard::mumu::{self, Discovery, MumuInstance, MINIMUM_VERSION};
use acui_wizard::occupancy::{self, OtherRoot};
use acui_wizard::plan::{Plan, PlanHandset, PlanInstance, PlanOptions, PLAN_SCHEMA};
use acui_wizard::platform;
use acui_wizard::root::{check_root, check_root_beside_tools, is_installation_root, plain_size, text_issues, RootIssue};
use acui_wizard::runtime::Answer;
use acui_wizard::set::{self, InstallSet, SetError, SetFault, VerifiedSet, SET_FILE};
use acui_wizard::winpath::strip_verbatim;
use acui_wizard::{
    cleanup, install, pre_lay_tools, Done, Event, Failure, Lang, Left, PreLaid, PreLayFailure, Request, Said, Step,
};

use crate::desktop;
use labels::{fill, Labels, EN, ZH};

slint::include_modules!();

// The pages in their order: the one table of their numbers. The step bar's names and the
// titles (`labels.rs`) are in this order and as many, and the window takes each number from
// here (`number_pages`), so no number of a page stands anywhere else.
const WELCOME: usize = 0;
const USAGE: usize = 1;
const FILES: usize = 2;
const LOCATION: usize = 3;
const GAMES: usize = 4;
const EMULATORS: usize = 5;
const PHONES: usize = 6;
const ACCOUNTS: usize = 7;
const OPTIONS: usize = 8;
const CONFIRM: usize = 9;
const INSTALL: usize = 10;
const FINISH: usize = 11;
/// How many pages there are.
const PAGE_COUNT: usize = FINISH + 1;

/// How long typing in the folder box must pause before the folder is checked.
const ROOT_PAUSE: Duration = Duration::from_millis(300);
/// The space the first page states before the set is checked: what a set with one game
/// takes, rounded up.
const SPACE_GUESS_GB: &str = "2";

/// The set file as read, unchecked, for the first page.
struct Preview {
    set: InstallSet,
    /// Each game's display name (else its folder name), in the set's order.
    games: Vec<String>,
}

/// The set file in `dir` and its games' names, before the check: the names come from each
/// standard package zip's `applications.json` alone, so the first page names the games as
/// people know them.
fn read_preview(dir: &Path) -> Option<Preview> {
    let set = set::read_set_file(dir).ok()?;
    let games = set
        .bundles
        .iter()
        .map(|bundle| set::unchecked_label(dir, bundle).unwrap_or_else(|| bundle.game.clone()))
        .collect();
    Some(Preview { set, games })
}

enum SetState {
    /// No folder holding `install-set.json` is known; `wrong_pick` when the last folder picked
    /// held none.
    Missing { wrong_pick: bool },
    /// Known, not checked yet.
    Waiting,
    Checking { done: u64, total: u64, line: Option<Said> },
    Ready(Box<VerifiedSet>),
    Failed(SetError),
}

/// One line under the folder box.
enum RootLine {
    Fresh,
    Empty,
    /// It holds only the device bridge the phone page laid into it.
    PreLaid,
    Space { free: u64, need: u64 },
    Issue(RootIssue),
    Contains(String),
    Worker(String),
}

struct RootReport {
    lines: Vec<RootLine>,
    usable: bool,
}

enum EmuState {
    Idle,
    Searching,
    Done { discovery: Discovery, others: Vec<OtherRoot> },
}

struct EmuRow {
    instance: MumuInstance,
    /// The other installation that uses it: its root, or its process.
    taken: Option<String>,
    picked: bool,
    /// The person said yes to taking it although another installation uses it.
    confirmed: bool,
}

/// The phone page's work ([`App::phone_work`]): the device bridge laid into the install folder,
/// the Lab asked what it can do, the phones listed.
enum PhoneState {
    /// Not begun, or let go of (the Android box unticked).
    Idle,
    /// Laying the device bridge: the line said last, the bytes laid.
    Laying { line: Option<Said>, done: u64, total: u64 },
    Asking,
    Listing,
    /// The list is shown (`State::phone_rows`); `keys_unmade` when the answer says the rows carry
    /// no key, and why.
    Listed { keys_unmade: Option<String> },
    /// The run's key salt could not be minted: no phone can be told apart.
    NoSalt(Said),
    /// The device bridge could not be laid; what it wrote is in `State::stray`.
    LayFailed(PreLayFailure),
    /// The set's Lab lacks these parameters to bind a phone before the install.
    Unable(Vec<String>),
    /// Asking or listing failed.
    Failed(Said),
}

/// One device the phone page lists, and whether it is ticked (only a row a phone's instance may
/// take, `devices::row_use`).
struct DeviceEntry {
    row: DeviceRow,
    picked: bool,
}

/// One instance of the instance page: an emulator's, or a phone's (`device_key`).
#[derive(Clone)]
struct Account {
    /// The MuMu index; for a phone, its number among the phones ticked (1 on), for its default
    /// name alone.
    index: u16,
    /// The emulator's name; for a phone, its model as the list gave it (empty when none).
    name: String,
    alias: String,
    /// The person typed the name; it no longer follows the game.
    edited: bool,
    game: Option<String>,
    server: Option<String>,
    touch: String,
    capture: String,
    start_emulator: bool,
    confirmed_in_use: bool,
    advanced: bool,
    /// A phone's instance: the phone's device key, as the phone page listed it under the run's
    /// key salt; `None` for an emulator's.
    device_key: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StepMark {
    Waiting,
    Running,
    Finished,
    Failed,
    Skipped,
}

enum InstallState {
    NotStarted,
    Running,
    Done(Box<Done>),
    Failed(Box<Failure>),
}

/// The question over the window.
#[derive(Clone, Copy)]
enum Ask {
    Quit,
    TakeInUse(usize),
    NoEmulator,
    Busy,
}

struct State {
    lang: Lang,
    page: usize,
    ask: Option<Ask>,
    picking: bool,
    /// Why the last folder picker did not open.
    picker_error: Option<String>,
    /// The usage page's MuMu box. Off, the emulator page is passed by, MuMu is not looked for and
    /// there is no emulator's instance.
    use_mumu: bool,
    /// The usage page's Android box. Off, the phone page is passed by and there is no phone's
    /// instance; with neither box, the instance page too.
    use_android: bool,
    /// The run's key salt (`acui_wizard::key_salt`), minted when the window opens, which the
    /// phone page lists with and the install binds with; why not, when the system's random
    /// source failed.
    key_salt: Result<KeySalt, Said>,
    /// The device bridge the phone page laid, one for each install folder it laid one into; the
    /// install takes the one of its folder ([`State::pre_laid_here`]).
    pre_laid: Vec<PreLaid>,
    /// The `tools` folders a lay that failed wrote into, which stay as they are.
    stray: Vec<PathBuf>,
    phone: PhoneState,
    phone_generation: u64,
    /// The install folder the phone page's work was for.
    phone_root: Option<PathBuf>,
    phone_rows: Vec<DeviceEntry>,
    set_dir: Option<PathBuf>,
    preview: Option<Preview>,
    set: SetState,
    set_generation: u64,
    root_text: String,
    root_generation: u64,
    root_report: Option<(u64, RootReport)>,
    games: Vec<(Game, bool)>,
    emu: EmuState,
    emu_generation: u64,
    emu_rows: Vec<EmuRow>,
    accounts: Vec<Account>,
    options: PlanOptions,
    install: InstallState,
    marks: Vec<StepMark>,
    current: Option<usize>,
    detail: Option<Said>,
    bytes: Option<(u64, u64)>,
    /// What went wrong opening the console, the folder or the log.
    action_error: Option<String>,
}

impl State {
    fn labels(&self) -> &'static Labels {
        Labels::of(self.lang)
    }

    fn verified(&self) -> Option<&VerifiedSet> {
        match &self.set {
            SetState::Ready(set) => Some(&**set),
            _ => None,
        }
    }

    fn chosen_games(&self) -> Vec<Game> {
        self.games.iter().filter(|(_, picked)| *picked).map(|(game, _)| game.clone()).collect()
    }

    fn game(&self, folder: &str) -> Option<&Game> {
        self.games.iter().map(|(game, _)| game).find(|game| game.game == folder)
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(self.root_text.trim())
    }

    /// The bytes the games ticked need.
    fn space_needed(&self) -> u64 {
        let games: Vec<String> = self.chosen_games().into_iter().map(|game| game.game).collect();
        self.verified().map_or(0, |set| set.space_needed(&games))
    }

    fn root_usable(&self) -> bool {
        !self.root_text.trim().is_empty()
            && matches!(&self.root_report, Some((generation, report)) if *generation == self.root_generation && report.usable)
    }

    fn mumu_root(&self) -> Option<String> {
        match &self.emu {
            EmuState::Done { discovery, .. } => discovery
                .found
                .as_ref()
                .map(|found| strip_verbatim(&found.root.to_string_lossy())),
            _ => None,
        }
    }

    /// The device bridge the phone page laid into the folder in the box, if it laid one there.
    fn pre_laid_here(&self) -> Option<&PreLaid> {
        let root = self.root();
        self.pre_laid.iter().find(|laid| laid.root == root)
    }

    /// Every `tools` folder the phone page wrote into, laid whole or in part, each once.
    fn laid_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.pre_laid.iter().map(|laid| laid.dir.clone()).collect();
        for dir in &self.stray {
            if !dirs.contains(dir) {
                dirs.push(dir.clone());
            }
        }
        dirs
    }

    /// The phone page is writing: it holds the window until it is done.
    fn laying(&self) -> bool {
        matches!(self.phone, PhoneState::Laying { .. })
    }
}

struct App {
    window: WizardWindow,
    state: RefCell<State>,
    root_timer: Timer,
    accounts: Rc<VecModel<AccountRow>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Runs `work` on the window, when there is one.
fn with_app(work: impl FnOnce(&App)) {
    let app = APP.with(|cell| cell.borrow().clone());
    if let Some(app) = app {
        work(&app);
    }
}

/// Hands `work` to the window's thread from a worker. The loop refuses only once the window
/// has closed for good, when there is no one left to tell; the install keeps its own log.
fn post(work: impl FnOnce(&App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(work));
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn shared(items: impl IntoIterator<Item = impl Into<SharedString>>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(Into::into).collect::<Vec<SharedString>>()))
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// A sentence of the back end in `lang`, without its closing stop (the window's own sentence
/// around it has one).
fn said(said: &Said, lang: Lang) -> String {
    said.in_lang(lang).trim_end_matches(['。', '.', ' ']).to_owned()
}

/// Each page's number for the window, which shows a page by it: every one is set, since a
/// number left unset would be 0, the welcome page's.
fn number_pages(window: &WizardWindow) {
    let pages = window.global::<Pages>();
    pages.set_welcome(WELCOME as i32);
    pages.set_usage(USAGE as i32);
    pages.set_files(FILES as i32);
    pages.set_location(LOCATION as i32);
    pages.set_games(GAMES as i32);
    pages.set_emulators(EMULATORS as i32);
    pages.set_phones(PHONES as i32);
    pages.set_accounts(ACCOUNTS as i32);
    pages.set_options(OPTIONS as i32);
    pages.set_confirm(CONFIRM as i32);
    pages.set_install(INSTALL as i32);
    pages.set_finish(FINISH as i32);
}

/// The window, until it is closed; the exit code says whether an install failed.
pub fn run() -> ExitCode {
    // The language Windows speaks to this user; the switch at the top right changes it.
    let lang = desktop::system_lang();
    let labels = Labels::of(lang);
    let window = match WizardWindow::new() {
        Ok(window) => window,
        Err(error) => {
            desktop::alert(labels.window_title, &fill(labels.window_failed, &[&error.to_string()]));
            return ExitCode::FAILURE;
        }
    };
    window.global::<Look>().set_skin(Skin::Flat);
    number_pages(&window);
    let set_dir = set::beside_program();
    let preview = set_dir.as_deref().and_then(read_preview);
    let set = if set_dir.is_some() { SetState::Waiting } else { SetState::Missing { wrong_pick: false } };
    let state = State {
        lang,
        page: WELCOME,
        ask: None,
        picking: false,
        picker_error: None,
        // As the wizard was before the usage page: for MuMu.
        use_mumu: true,
        use_android: false,
        // Once for the whole run: the phone page's list and the install's bind need the same.
        key_salt: KeySalt::mint(),
        pre_laid: Vec::new(),
        stray: Vec::new(),
        phone: PhoneState::Idle,
        phone_generation: 0,
        phone_root: None,
        phone_rows: Vec::new(),
        set_dir,
        preview,
        set,
        set_generation: 0,
        root_text: String::new(),
        root_generation: 0,
        root_report: None,
        games: Vec::new(),
        emu: EmuState::Idle,
        emu_generation: 0,
        emu_rows: Vec::new(),
        accounts: Vec::new(),
        options: PlanOptions::default(),
        install: InstallState::NotStarted,
        marks: vec![StepMark::Waiting; Step::ALL.len()],
        current: None,
        detail: None,
        bytes: None,
        action_error: None,
    };
    let accounts = Rc::new(VecModel::default());
    window.global::<Wiz>().set_accounts(ModelRc::from(accounts.clone()));
    let app = Rc::new(App {
        window,
        state: RefCell::new(state),
        root_timer: Timer::default(),
        accounts,
    });
    APP.with(|cell| *cell.borrow_mut() = Some(app.clone()));
    app.wire();
    app.fill_strings();
    app.paint();
    let ran = app.window.run();
    let failed = matches!(app.state.borrow().install, InstallState::Failed(_));
    APP.with(|cell| *cell.borrow_mut() = None);
    if let Err(error) = ran {
        desktop::alert(labels.window_title, &fill(labels.window_failed, &[&error.to_string()]));
        return ExitCode::FAILURE;
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

impl App {
    fn wire(&self) {
        let wiz = self.window.global::<Wiz>();
        wiz.on_next(|| with_app(App::next));
        wiz.on_back(|| with_app(App::back));
        wiz.on_cancel(|| with_app(App::cancel));
        wiz.on_language(|english| with_app(|app| app.language(if english { Lang::En } else { Lang::Zh })));
        wiz.on_dialog_answer(|yes| with_app(|app| app.answer(yes)));
        wiz.on_choose_set_folder(|| with_app(|app| app.pick(true)));
        wiz.on_browse_root(|| with_app(|app| app.pick(false)));
        wiz.on_usage_toggled(|which| with_app(|app| app.usage_toggled(which)));
        wiz.on_root_edited(|text| with_app(|app| app.root_edited(text.to_string())));
        wiz.on_game_toggled(|row| with_app(|app| app.game_toggled(row)));
        wiz.on_search_again(|| with_app(App::search));
        wiz.on_skip_emulators(|| with_app(App::skip_emulators));
        wiz.on_emulator_toggled(|row| with_app(|app| app.emulator_toggled(row)));
        wiz.on_phone_toggled(|row| with_app(|app| app.phone_toggled(row)));
        wiz.on_refresh_phones(|| with_app(App::refresh_phones));
        wiz.on_alias_edited(|row, text| with_app(|app| app.alias_edited(row, text.to_string())));
        wiz.on_game_picked(|row, pick| with_app(|app| app.game_picked(row, pick)));
        wiz.on_server_picked(|row, pick| with_app(|app| app.server_picked(row, pick)));
        // A phone's lists are a phone's own (`account_row`).
        wiz.on_touch_picked(|row, pick| {
            with_app(|app| app.account_changed(row, |account| {
                let backends: &[&str] = if account.device_key.is_some() { &HANDSET_TOUCH_BACKENDS } else { &TOUCH_BACKENDS };
                if let Some(backend) = usize::try_from(pick).ok().and_then(|pick| backends.get(pick)) {
                    account.touch = (*backend).to_owned();
                }
            }))
        });
        wiz.on_capture_picked(|row, pick| {
            with_app(|app| app.account_changed(row, |account| {
                let backends: &[&str] = if account.device_key.is_some() { &HANDSET_CAPTURE_BACKENDS } else { &CAPTURE_BACKENDS };
                if let Some(backend) = usize::try_from(pick).ok().and_then(|pick| backends.get(pick)) {
                    account.capture = (*backend).to_owned();
                }
            }))
        });
        wiz.on_start_toggled(|row| {
            with_app(|app| app.account_changed(row, |account| account.start_emulator = !account.start_emulator))
        });
        wiz.on_advanced_toggled(|row| {
            with_app(|app| app.account_changed(row, |account| account.advanced = !account.advanced))
        });
        wiz.on_option_toggled(|which| with_app(|app| app.option_toggled(which)));
        wiz.on_open_console(|| with_app(App::open_console));
        wiz.on_open_folder(|| with_app(App::open_folder));
        wiz.on_open_log(|| with_app(App::open_log));
        self.window.window().on_close_requested(|| {
            let mut response = CloseRequestResponse::HideWindow;
            with_app(|app| response = app.close_requested());
            response
        });
    }

    fn fill_strings(&self) {
        let l = self.state.borrow().labels();
        let strings = self.window.global::<Strings>();
        strings.set_window_title(l.window_title.into());
        strings.set_side_name(l.side_name.into());
        strings.set_side_title(l.side_title.into());
        strings.set_picking_note(l.picking_note.into());
        strings.set_back(l.back.into());
        strings.set_cancel(l.cancel.into());
        strings.set_browse(l.browse.into());
        strings.set_choose_folder(l.choose_folder.into());
        strings.set_search_again(l.search_again.into());
        strings.set_skip_step(l.skip_step.into());
        strings.set_welcome_body(l.welcome_body.into());
        strings.set_welcome_what_head(l.welcome_what_head.into());
        strings.set_welcome_before_head(l.welcome_before_head.into());
        strings.set_req_windows(l.req_windows.into());
        strings.set_welcome_untouched(l.welcome_untouched.into());
        strings.set_usage_body(l.usage_body.into());
        strings.set_usage_mumu(l.usage_mumu.into());
        strings.set_usage_mumu_note(fill(l.usage_mumu_note, &[&mumu::version_text(MINIMUM_VERSION)]).into());
        strings.set_usage_android(l.usage_android.into());
        strings.set_usage_android_note(l.usage_android_note.into());
        strings.set_usage_ios(l.usage_ios.into());
        strings.set_usage_ios_note(l.usage_ios_note.into());
        strings.set_root_body(l.root_body.into());
        strings.set_root_folders(l.root_folders.into());
        strings.set_root_label(l.root_label.into());
        strings.set_games_body(l.games_body.into());
        strings.set_accounts_body(l.accounts_body.into());
        strings.set_col_emulator(l.col_emulator.into());
        strings.set_col_name(l.col_name.into());
        strings.set_col_game(l.col_game.into());
        strings.set_col_server(l.col_server.into());
        strings.set_name_hint(l.name_hint.into());
        strings.set_advanced(l.advanced.into());
        strings.set_touch_label(l.touch_label.into());
        strings.set_capture_label(l.capture_label.into());
        strings.set_start_emulator(l.start_emulator.into());
        strings.set_advanced_note(l.advanced_note.into());
        strings.set_opt_desktop(l.opt_desktop.into());
        strings.set_opt_start_menu(l.opt_start_menu.into());
        strings.set_opt_watchdog(l.opt_watchdog.into());
        strings.set_opt_watchdog_note(l.opt_watchdog_note.into());
        strings.set_opt_start(l.opt_start.into());
        strings.set_opt_pause(l.opt_pause.into());
        strings.set_opt_pause_note(l.opt_pause_note.into());
        strings.set_opt_pause_off_note(l.opt_pause_off_note.into());
        strings.set_open_console(l.open_console.into());
        strings.set_open_folder(l.open_folder.into());
        strings.set_copy_error(l.copy_error.into());
        strings.set_open_log(l.open_log.into());
        strings.set_refresh(l.refresh.into());
        strings.set_touch_options(shared(TOUCH_BACKENDS));
        strings.set_capture_options(shared(CAPTURE_BACKENDS));
        strings.set_phone_touch_options(shared(HANDSET_TOUCH_BACKENDS));
        strings.set_phone_capture_options(shared(HANDSET_CAPTURE_BACKENDS));
    }

    // ---- Painting -------------------------------------------------------------------------

    /// The frame (steps, title, buttons, question) and the page shown.
    fn paint(&self) {
        let state = self.state.borrow();
        let l = state.labels();
        let wiz = self.window.global::<Wiz>();
        let page = state.page;
        let failed = matches!(state.install, InstallState::Failed(_));
        wiz.set_page(page as i32);
        wiz.set_english(state.lang == Lang::En);
        wiz.set_page_title(if page == FINISH && failed { l.finish_failed_title } else { l.titles[page] }.into());
        let steps: Vec<StepItem> = (0..l.steps.len())
            .map(|index| StepItem {
                label: l.steps[index].into(),
                state: if index == page {
                    1
                } else if index == INSTALL && failed {
                    4
                } else if (index == EMULATORS && !state.use_mumu)
                    || (index == PHONES && !state.use_android)
                    || (index == ACCOUNTS && !state.use_mumu && !state.use_android)
                {
                    // Passed by without MuMu, without phones, without either (`App::next`).
                    3
                } else if index == ACCOUNTS && page > ACCOUNTS && state.accounts.is_empty() {
                    3
                } else if index < page {
                    2
                } else {
                    0
                },
            })
            .collect();
        wiz.set_steps(model(steps));
        // While the phone page lays the device bridge, it holds the window (`App::back`).
        wiz.set_back_shown((USAGE..=CONFIRM).contains(&page) && !(page == PHONES && state.laying()));
        wiz.set_next_shown(page <= CONFIRM);
        wiz.set_next_text(if page == CONFIRM { l.install } else { l.next }.into());
        wiz.set_next_enabled(can_next(&state));
        wiz.set_cancel_text(if page == FINISH { l.close } else { l.cancel }.into());
        wiz.set_cancel_enabled(page != INSTALL);
        wiz.set_log_line(log_path(&state).map_or_else(String::new, |log| fill(l.log_line, &[&log.display().to_string()])).into());
        wiz.set_picking(state.picking);
        // Once the phone page laid the device bridge, quitting leaves it: the question says where.
        let laid: Vec<String> = state.laid_dirs().iter().map(|dir| strip_verbatim(&dir.to_string_lossy())).collect();
        let quit = if laid.is_empty() { l.quit_ask.to_owned() } else { fill(l.quit_ask_laid, &[&laid.join(l.list_separator)]) };
        let question = state.ask.map(|ask| match ask {
            Ask::Quit => (quit.clone(), l.quit_yes, l.quit_no),
            Ask::TakeInUse(_) => (l.emu_take_ask.to_owned(), l.emu_take_yes, l.emu_take_no),
            Ask::NoEmulator => (l.emu_none_ask.to_owned(), l.emu_none_yes, l.emu_none_no),
            Ask::Busy => (l.busy_note.to_owned(), l.ok, ""),
        });
        wiz.set_dialog_shown(question.is_some());
        if let Some((text, yes, no)) = question {
            wiz.set_dialog_text(text.into());
            wiz.set_dialog_yes(yes.into());
            wiz.set_dialog_no(no.into());
        }
        match page {
            WELCOME => paint_welcome(&state, &wiz),
            USAGE => paint_usage(&state, &wiz),
            FILES => paint_files(&state, &wiz),
            LOCATION => paint_location(&state, &wiz),
            GAMES => paint_games(&state, &wiz),
            EMULATORS => paint_emulators(&state, &wiz),
            PHONES => paint_phones(&state, &wiz),
            ACCOUNTS => {
                wiz.set_accounts_note(accounts_note(&state).into());
                wiz.set_account_problems(shared(account_problems(&state)));
            }
            OPTIONS => paint_options(&state, &wiz),
            CONFIRM => paint_confirm(&state, &wiz),
            INSTALL => paint_install(&state, &wiz),
            _ => paint_finish(&state, &wiz),
        }
    }

    /// Every account row afresh (on entering the page and on a language switch).
    fn paint_accounts(&self) {
        let state = self.state.borrow();
        let chosen = state.chosen_games();
        let rows: Vec<AccountRow> = state
            .accounts
            .iter()
            .map(|account| account_row(account, &chosen, state.labels()))
            .collect();
        self.accounts.set_vec(rows);
    }

    fn paint_account(&self, row: usize) {
        let state = self.state.borrow();
        let chosen = state.chosen_games();
        if let Some(account) = state.accounts.get(row) {
            if row < self.accounts.row_count() {
                self.accounts.set_row_data(row, account_row(account, &chosen, state.labels()));
            }
        }
    }

    // ---- Moving between pages -------------------------------------------------------------

    fn next(&self) {
        let page = {
            let state = self.state.borrow();
            if !can_next(&state) || state.ask.is_some() {
                return;
            }
            state.page
        };
        match page {
            WELCOME | USAGE | FILES | LOCATION | ACCOUNTS | OPTIONS => self.go(page + 1),
            // Without MuMu the emulator page is passed by, without phones the phone page, and
            // without either the instance page: no search, no instance (the usage page let go of
            // them when its box was unticked).
            GAMES => {
                let (mumu, android) = {
                    let state = self.state.borrow();
                    (state.use_mumu, state.use_android)
                };
                self.go(if mumu {
                    EMULATORS
                } else if android {
                    PHONES
                } else {
                    OPTIONS
                });
            }
            EMULATORS => {
                let (picked, android) = {
                    let state = self.state.borrow();
                    (state.emu_rows.iter().any(|row| row.picked), state.use_android)
                };
                if picked {
                    sync_accounts(&mut self.state.borrow_mut());
                    self.go(if android { PHONES } else { ACCOUNTS });
                } else if android {
                    // The phones may still bring instances: whether there are none is the phone
                    // page's to say.
                    drop_emulators(&mut self.state.borrow_mut());
                    self.go(PHONES);
                } else {
                    self.ask(Ask::NoEmulator);
                }
            }
            // No phone ticked is a choice too (`can_next`): the instance page comes only with
            // some instance to set up.
            PHONES => {
                let any = {
                    let mut state = self.state.borrow_mut();
                    sync_accounts(&mut state);
                    !state.accounts.is_empty()
                };
                self.go(if any { ACCOUNTS } else { OPTIONS });
            }
            CONFIRM => self.start_install(),
            _ => {}
        }
    }

    fn back(&self) {
        let target = {
            let state = self.state.borrow();
            match state.page {
                // The device bridge is being laid: the page holds until it is done.
                PHONES if state.laying() => return,
                PHONES if !state.use_mumu => GAMES,
                PHONES => EMULATORS,
                ACCOUNTS if state.use_android => PHONES,
                ACCOUNTS => EMULATORS,
                OPTIONS if !state.accounts.is_empty() => ACCOUNTS,
                OPTIONS if state.use_android => PHONES,
                OPTIONS if state.use_mumu => EMULATORS,
                OPTIONS => GAMES,
                page @ USAGE..=CONFIRM => page - 1,
                _ => return,
            }
        };
        self.go(target);
    }

    fn go(&self, page: usize) {
        self.state.borrow_mut().page = page;
        match page {
            FILES => {
                if matches!(self.state.borrow().set, SetState::Waiting) {
                    self.start_check();
                }
            }
            LOCATION => {
                let default = self.state.borrow().root_text.trim().is_empty();
                if default {
                    let text = strip_verbatim(&desktop::default_root().to_string_lossy());
                    self.window.global::<Wiz>().set_root_text(text.as_str().into());
                    self.state.borrow_mut().root_text = text;
                }
                self.state.borrow_mut().root_generation += 1;
                self.check_root();
            }
            EMULATORS => {
                if matches!(self.state.borrow().emu, EmuState::Idle) {
                    self.search();
                }
            }
            // Begun afresh when it has not begun, was for another install folder, or failed (the
            // device bridge not laid, the Lab not asked or the phones not listed), so that a page
            // come back to tries again what it said to try; else what it found stays, and
            // "Refresh" lists again.
            PHONES => {
                let fresh = {
                    let state = self.state.borrow();
                    matches!(state.phone, PhoneState::Idle | PhoneState::LayFailed(_) | PhoneState::Failed(_))
                        || state.phone_root.as_ref() != Some(&state.root())
                };
                if fresh {
                    self.phone_work();
                }
            }
            ACCOUNTS => self.paint_accounts(),
            _ => {}
        }
        self.paint();
    }

    fn cancel(&self) {
        let (page, laying) = {
            let state = self.state.borrow();
            (state.page, state.laying())
        };
        match page {
            INSTALL => {}
            FINISH => self.close(),
            _ if laying => self.ask(Ask::Busy),
            _ => self.ask(Ask::Quit),
        }
    }

    fn close(&self) {
        // Hiding the last window ends the loop; should the platform refuse, the loop is asked
        // to end directly.
        if self.window.hide().is_err() {
            let _ = slint::quit_event_loop();
        }
    }

    fn close_requested(&self) -> CloseRequestResponse {
        // The phone page laying the device bridge writes as the install does.
        let (page, running, asking, picking) = {
            let state = self.state.borrow();
            (
                state.page,
                matches!(state.install, InstallState::Running) || state.laying(),
                state.ask.is_some(),
                state.picking,
            )
        };
        if page == FINISH {
            return CloseRequestResponse::HideWindow;
        }
        // A folder picker is open over the window: it is answered first.
        if picking {
            return CloseRequestResponse::KeepWindowShown;
        }
        if running {
            self.ask(Ask::Busy);
        } else if !asking {
            self.ask(Ask::Quit);
        }
        CloseRequestResponse::KeepWindowShown
    }

    fn ask(&self, ask: Ask) {
        self.state.borrow_mut().ask = Some(ask);
        self.paint();
    }

    fn answer(&self, yes: bool) {
        let ask = self.state.borrow_mut().ask.take();
        match ask {
            Some(Ask::Quit) if yes => {
                self.paint();
                self.close();
                return;
            }
            Some(Ask::TakeInUse(row)) if yes => {
                if let Some(row) = self.state.borrow_mut().emu_rows.get_mut(row) {
                    row.picked = true;
                    row.confirmed = true;
                }
            }
            Some(Ask::NoEmulator) if yes => {
                drop_emulators(&mut self.state.borrow_mut());
                self.go(OPTIONS);
                return;
            }
            _ => {}
        }
        self.paint();
    }

    fn language(&self, lang: Lang) {
        self.state.borrow_mut().lang = lang;
        self.fill_strings();
        self.paint_accounts();
        self.paint();
    }

    // ---- 2 Usage ---------------------------------------------------------------------------

    fn usage_toggled(&self, which: i32) {
        {
            let mut state = self.state.borrow_mut();
            match which {
                // MuMu unticked lets go of the emulators and instances chosen, as "Skip this
                // step" does, and of what the search found or is still finding: ticked again,
                // the emulator page looks for MuMu afresh.
                0 => {
                    state.use_mumu = !state.use_mumu;
                    if !state.use_mumu {
                        drop_emulators(&mut state);
                        state.emu = EmuState::Idle;
                        state.emu_generation += 1;
                    }
                }
                // Android unticked lets go of the phones chosen and of what the phone page found
                // or is still finding: ticked again, it lists afresh. The device bridge it laid
                // stays where it is (the install takes it, `State::pre_laid_here`).
                1 => {
                    state.use_android = !state.use_android;
                    if !state.use_android {
                        drop_phones(&mut state);
                    }
                }
                // The iOS box is greyed and never asks.
                _ => {}
            }
        }
        self.paint();
    }

    // ---- 3 Files ---------------------------------------------------------------------------

    fn start_check(&self) {
        let (dir, generation) = {
            let mut state = self.state.borrow_mut();
            let Some(dir) = state.set_dir.clone() else {
                state.set = SetState::Missing { wrong_pick: false };
                return;
            };
            state.set_generation += 1;
            state.set = SetState::Checking { done: 0, total: 0, line: None };
            (dir, state.set_generation)
        };
        let spawned = spawn("acsetup-set-check", move || {
            let mut shown = u64::MAX;
            let result = set::verify_set(&dir, &mut |event: Event| match event {
                Event::Bytes { done, total } => {
                    let permille = if total == 0 { 1000 } else { done.saturating_mul(1000) / total };
                    if permille != shown {
                        shown = permille;
                        post(move |app| app.set_progress(generation, done, total, None));
                    }
                }
                // The check's words for a person; its full lines (paths, repositories and
                // commits) go into the setup log when the install checks the set again.
                Event::Plain(line) => post(move |app| {
                    let (done, total) = match &app.state.borrow().set {
                        SetState::Checking { done, total, .. } => (*done, *total),
                        _ => (0, 0),
                    };
                    app.set_progress(generation, done, total, Some(line));
                }),
                _ => {}
            });
            post(move |app| app.set_checked(generation, result));
        });
        if let Err(error) = spawned {
            self.state.borrow_mut().set = SetState::Failed(SetError {
                fault: SetFault::Unusable,
                said: Said::new(fill(ZH.worker_failed, &[&error]), fill(EN.worker_failed, &[&error])),
            });
        }
        self.paint();
    }

    fn set_progress(&self, generation: u64, done: u64, total: u64, line: Option<Said>) {
        {
            let mut state = self.state.borrow_mut();
            if generation != state.set_generation {
                return;
            }
            if let SetState::Checking { done: was_done, total: was_total, line: was_line } = &mut state.set {
                *was_done = done;
                *was_total = total;
                if line.is_some() {
                    *was_line = line;
                }
            }
        }
        self.paint();
    }

    fn set_checked(&self, generation: u64, result: Result<VerifiedSet, SetError>) {
        {
            let mut state = self.state.borrow_mut();
            if generation != state.set_generation {
                return;
            }
            match result {
                Ok(set) => {
                    let games = set.games();
                    let previous: Vec<(String, bool)> =
                        state.games.iter().map(|(game, picked)| (game.game.clone(), *picked)).collect();
                    state.games = games
                        .into_iter()
                        .map(|game| {
                            let picked = previous
                                .iter()
                                .find(|(folder, _)| *folder == game.game)
                                .map_or(true, |(_, picked)| *picked);
                            (game, picked)
                        })
                        .collect();
                    state.set = SetState::Ready(Box::new(set));
                }
                Err(error) => state.set = SetState::Failed(error),
            }
        }
        self.paint();
    }

    /// The folder picker, for the install set (`for_set`) or for the install location.
    fn pick(&self, for_set: bool) {
        let title = {
            let mut state = self.state.borrow_mut();
            if state.picking {
                return;
            }
            state.picking = true;
            state.picker_error = None;
            let l = state.labels();
            let title = if for_set { l.set_dialog_title } else { l.root_dialog_title };
            title.to_owned()
        };
        // The picker is owned by this window, so it opens above it and the window takes no
        // input until it closes; the window also draws a veil of its own meanwhile (`picking`).
        let owner = self.handle();
        let spawned = spawn("acsetup-folder-picker", move || {
            let picked = desktop::pick_folder(&title, owner);
            post(move |app| app.picked(for_set, picked));
        });
        if let Err(error) = spawned {
            let mut state = self.state.borrow_mut();
            state.picking = false;
            state.picker_error = Some(error);
        }
        self.paint();
    }

    /// This window's handle, for a dialog it owns: Slint's own, else the active window of this
    /// thread (the window's thread, whose only window this is); `None` when neither is there.
    fn handle(&self) -> Option<isize> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = self.window.window().window_handle();
        let from_slint = match HasWindowHandle::window_handle(&handle).map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(win32)) => Some(win32.hwnd.get()),
            _ => None,
        };
        from_slint.or_else(desktop::active_window)
    }

    fn picked(&self, for_set: bool, picked: Result<Option<PathBuf>, String>) {
        let chosen = {
            let mut state = self.state.borrow_mut();
            state.picking = false;
            match picked {
                Ok(chosen) => chosen,
                Err(error) => {
                    state.picker_error = Some(error);
                    None
                }
            }
        };
        let Some(dir) = chosen else {
            self.paint();
            return;
        };
        if for_set {
            let holds_set = dir.join(SET_FILE).is_file();
            {
                let mut state = self.state.borrow_mut();
                if holds_set {
                    state.preview = read_preview(&dir);
                    state.set_dir = Some(dir);
                    state.set = SetState::Waiting;
                } else {
                    state.set = SetState::Missing { wrong_pick: true };
                }
            }
            if holds_set {
                self.start_check();
                return;
            }
            self.paint();
        } else {
            let text = strip_verbatim(&dir.to_string_lossy());
            self.window.global::<Wiz>().set_root_text(text.as_str().into());
            {
                let mut state = self.state.borrow_mut();
                state.root_text = text;
                state.root_generation += 1;
            }
            self.check_root();
            self.paint();
        }
    }

    // ---- 4 Location ------------------------------------------------------------------------

    fn root_edited(&self, text: String) {
        {
            let mut state = self.state.borrow_mut();
            state.root_text = text;
            state.root_generation += 1;
        }
        self.root_timer
            .start(TimerMode::SingleShot, ROOT_PAUSE, || with_app(App::check_root));
        self.paint();
    }

    /// Checks the folder in the box as it is now, on a worker. A folder the phone page laid the
    /// device bridge into is checked as the install will take it: that `tools` beside nothing,
    /// one copy of the Tools less to find room for.
    fn check_root(&self) {
        let (generation, text, needed, laid) = {
            let state = self.state.borrow();
            let laid = state.pre_laid_here().is_some();
            let tools = state.verified().map_or(0, |set| set.program(Program::Tools).bytes);
            let needed = if laid { state.space_needed().saturating_sub(tools) } else { state.space_needed() };
            (state.root_generation, state.root_text.trim().to_owned(), needed, laid)
        };
        let spawned = spawn("acsetup-root-check", move || {
            let report = root_report(&text, needed, laid);
            post(move |app| app.root_checked(generation, report));
        });
        if let Err(error) = spawned {
            self.root_checked(generation, RootReport { lines: vec![RootLine::Worker(error)], usable: false });
        }
    }

    fn root_checked(&self, generation: u64, report: RootReport) {
        {
            let mut state = self.state.borrow_mut();
            if generation != state.root_generation {
                return;
            }
            state.root_report = Some((generation, report));
        }
        self.paint();
    }

    // ---- 5 Games ---------------------------------------------------------------------------

    fn game_toggled(&self, row: i32) {
        if let Ok(row) = usize::try_from(row) {
            if let Some(entry) = self.state.borrow_mut().games.get_mut(row) {
                entry.1 = !entry.1;
            }
        }
        self.paint();
    }

    // ---- 6 Emulators -----------------------------------------------------------------------

    fn search(&self) {
        let generation = {
            let mut state = self.state.borrow_mut();
            if matches!(state.emu, EmuState::Searching) {
                return;
            }
            state.emu = EmuState::Searching;
            state.emu_generation += 1;
            state.emu_generation
        };
        let spawned = spawn("acsetup-discover", move || {
            // What the search notes on the way is not shown here: the install's own checks run
            // the same search again and write every note into the setup log.
            let mut noted = |_: Said| {};
            let discovery = mumu::discover(None, &mut noted);
            let others = occupancy::scan(&mut noted);
            post(move |app| app.discovered(generation, discovery, others));
        });
        if let Err(error) = spawned {
            let discovery = Discovery {
                absent: false,
                found: None,
                version: None,
                instances: Vec::new(),
                problem: Some(Said::new(fill(ZH.worker_failed, &[&error]), fill(EN.worker_failed, &[&error]))),
            };
            self.discovered(generation, discovery, Vec::new());
            return;
        }
        self.paint();
    }

    fn discovered(&self, generation: u64, discovery: Discovery, others: Vec<OtherRoot>) {
        {
            let mut state = self.state.borrow_mut();
            if generation != state.emu_generation {
                return;
            }
            let previous: Vec<(u16, bool, bool)> =
                state.emu_rows.iter().map(|row| (row.instance.index, row.picked, row.confirmed)).collect();
            state.emu_rows = discovery
                .instances
                .iter()
                .map(|instance| {
                    let taken = occupancy::used_by(&others, instance).map(|(other, _)| other.name());
                    // Nothing is ticked for the person: they tick the emulators they hand over.
                    // A search again keeps what they ticked.
                    let before = previous.iter().find(|(index, _, _)| *index == instance.index);
                    let (picked, confirmed) = match before {
                        Some((_, _, confirmed)) if taken.is_some() && !confirmed => (false, false),
                        Some((_, picked, confirmed)) => (*picked, *confirmed),
                        None => (false, false),
                    };
                    EmuRow { instance: instance.clone(), taken, picked, confirmed }
                })
                .collect();
            state.emu = EmuState::Done { discovery, others };
        }
        self.paint();
    }

    fn emulator_toggled(&self, row: i32) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        let ask = {
            let mut state = self.state.borrow_mut();
            let Some(entry) = state.emu_rows.get_mut(row) else {
                return;
            };
            if entry.picked {
                entry.picked = false;
                entry.confirmed = false;
                false
            } else if entry.taken.is_some() && !entry.confirmed {
                true
            } else {
                entry.picked = true;
                false
            }
        };
        if ask {
            self.ask(Ask::TakeInUse(row));
        } else {
            self.paint();
        }
    }

    fn skip_emulators(&self) {
        let android = {
            let mut state = self.state.borrow_mut();
            drop_emulators(&mut state);
            state.use_android
        };
        self.go(if android { PHONES } else { OPTIONS });
    }

    // ---- 7 Phones --------------------------------------------------------------------------

    /// The phone page's work, on a worker ([`phones_worker`]): the device bridge laid into the
    /// install folder unless the page laid it there already, the Lab asked what it can do, the
    /// phones listed, each part heard as it ends. Each run is a question of its own
    /// (`State::phone_generation`): a result of one asked before is not shown, but what a lay
    /// wrote is kept whatever: it is on the disk. After a lay that failed, the work is the whole
    /// of it again; a folder that still holds what that lay left is refused by `pre_lay_tools` as
    /// any folder that is not new or empty is, and nothing of it is deleted.
    fn phone_work(&self) {
        let job = {
            let mut state = self.state.borrow_mut();
            let root = state.root();
            let salt = state.key_salt.clone();
            let set = state.verified().cloned();
            let laid = state.pre_laid_here().is_some();
            state.phone_generation += 1;
            state.phone_root = Some(root.clone());
            match (salt, set) {
                (Err(problem), _) => {
                    state.phone = PhoneState::NoSalt(problem);
                    None
                }
                // The files page lets no one past a set that is not checked.
                (Ok(_), None) => {
                    state.phone = PhoneState::Idle;
                    None
                }
                (Ok(salt), Some(set)) => {
                    state.phone = if laid { PhoneState::Asking } else { PhoneState::Laying { line: None, done: 0, total: 0 } };
                    Some((set, root, salt, laid, state.phone_generation))
                }
            }
        };
        if let Some((set, root, salt, laid, generation)) = job {
            let spawned = spawn("acsetup-phones", move || phones_worker(set, root, salt, laid, generation));
            if let Err(error) = spawned {
                let mut state = self.state.borrow_mut();
                if state.phone_generation == generation {
                    state.phone = PhoneState::Failed(Said::new(fill(ZH.worker_failed, &[&error]), fill(EN.worker_failed, &[&error])));
                }
            }
        }
        self.paint();
    }

    /// "Refresh": the work again unless it is under way, which after a lay that failed lays,
    /// asks and lists ([`App::phone_work`]).
    fn refresh_phones(&self) {
        let busy = matches!(self.state.borrow().phone, PhoneState::Laying { .. } | PhoneState::Asking | PhoneState::Listing);
        if !busy {
            self.phone_work();
        }
    }

    fn phones_progress(&self, generation: u64, done: u64, total: u64, line: Option<Said>) {
        {
            let mut state = self.state.borrow_mut();
            if generation != state.phone_generation {
                return;
            }
            if let PhoneState::Laying { line: was_line, done: was_done, total: was_total } = &mut state.phone {
                *was_done = done;
                *was_total = total;
                if line.is_some() {
                    *was_line = line;
                }
            }
        }
        self.paint();
    }

    /// The lay's end: the device bridge laid is kept for its folder, a lay that wrote and failed
    /// is kept among what stays (`State::stray`), whatever was asked since.
    fn phones_laid(&self, generation: u64, result: Result<PreLaid, PreLayFailure>) {
        {
            let mut state = self.state.borrow_mut();
            let shown = match result {
                Ok(laid) => {
                    state.pre_laid.retain(|other| other.root != laid.root);
                    state.pre_laid.push(laid);
                    PhoneState::Asking
                }
                Err(failure) => {
                    if failure.wrote && !state.stray.contains(&failure.dir) {
                        state.stray.push(failure.dir.clone());
                    }
                    PhoneState::LayFailed(failure)
                }
            };
            if generation != state.phone_generation {
                return;
            }
            state.phone = shown;
        }
        self.paint();
    }

    /// What the Lab answered goes into the install log with the device bridge laid into `root`
    /// ([`PreLaid::note`]), as the install logs an answer: its reading, then its whole stdout on
    /// a line of its own. The command line names the salt's variable, never the salt.
    fn note_answer(state: &mut State, root: &Path, said: Said, answer: &Answer) {
        if let Some(laid) = state.pre_laid.iter_mut().find(|laid| laid.root.as_path() == root) {
            laid.note(&said.with(format!("{}: {}", answer.command, answer.reason())).to_string());
            if let Some(stdout) = answer.log_stdout() {
                laid.note(&format!("  {} stdout: {stdout}", answer.command));
            }
        }
    }

    fn phones_asked(&self, generation: u64, root: PathBuf, asked: Result<Asked, Said>) {
        {
            let mut state = self.state.borrow_mut();
            if let Ok(asked) = &asked {
                for answer in &asked.answers {
                    Self::note_answer(&mut state, &root, Said::new("help 答复", "help answered"), answer);
                }
            }
            if generation != state.phone_generation {
                return;
            }
            state.phone = match asked {
                Ok(asked) if asked.missing.is_empty() => PhoneState::Listing,
                Ok(asked) => PhoneState::Unable(asked.missing),
                Err(problem) => PhoneState::Failed(problem),
            };
        }
        self.paint();
    }

    /// The list: a row ticked before stays ticked by its key while a phone's instance may still
    /// take it.
    fn phones_listed(&self, generation: u64, root: PathBuf, listed: Result<Answer, Said>) {
        {
            let mut state = self.state.borrow_mut();
            let listing = listed.and_then(|answer| {
                Self::note_answer(&mut state, &root, Said::new("devices 答复", "devices answered"), &answer);
                devices::read_listing(answer.code, answer.line.as_ref())
            });
            if generation != state.phone_generation {
                return;
            }
            match listing {
                Ok(listing) => {
                    let ticked: Vec<String> = state
                        .phone_rows
                        .iter()
                        .filter(|entry| entry.picked)
                        .filter_map(|entry| entry.row.device_key.clone())
                        .collect();
                    state.phone_rows = listing
                        .rows
                        .into_iter()
                        .map(|row| {
                            let picked = row_use(&row) == RowUse::Usable
                                && row.device_key.as_ref().is_some_and(|key| ticked.contains(key));
                            DeviceEntry { row, picked }
                        })
                        .collect();
                    state.phone = PhoneState::Listed { keys_unmade: listing.keys_unmade };
                }
                Err(problem) => state.phone = PhoneState::Failed(problem),
            }
        }
        self.paint();
    }

    fn phone_toggled(&self, row: i32) {
        if let Ok(row) = usize::try_from(row) {
            if let Some(entry) = self.state.borrow_mut().phone_rows.get_mut(row) {
                if row_use(&entry.row) == RowUse::Usable {
                    entry.picked = !entry.picked;
                }
            }
        }
        self.paint();
    }

    // ---- 8 Accounts ------------------------------------------------------------------------

    fn alias_edited(&self, row: i32, text: String) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let Some(account) = state.accounts.get_mut(row) else {
                return;
            };
            account.alias = text;
            account.edited = true;
        }
        // The row is not set again: its box holds what was typed.
        self.paint();
    }

    fn game_picked(&self, row: i32, pick: i32) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let chosen = state.chosen_games();
            let game = usize::try_from(pick)
                .ok()
                .and_then(|pick| pick.checked_sub(1))
                .and_then(|pick| chosen.get(pick))
                .cloned();
            let Some(account) = state.accounts.get_mut(row) else {
                return;
            };
            if account.game.as_deref() != game.as_ref().map(|game| game.game.as_str()) {
                account.game = game.as_ref().map(|game| game.game.clone());
                account.server = game.as_ref().and_then(only_server);
                if !account.edited {
                    account.alias = account_default_alias(account, game.as_ref());
                }
            }
        }
        self.paint_account(row);
        self.paint();
    }

    fn server_picked(&self, row: i32, pick: i32) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let server = state
                .accounts
                .get(row)
                .and_then(|account| account.game.clone())
                .and_then(|folder| state.game(&folder).cloned())
                .and_then(|game| {
                    usize::try_from(pick)
                        .ok()
                        .and_then(|pick| pick.checked_sub(1))
                        .and_then(|pick| game.servers.get(pick))
                        .map(|server| server.server.clone())
                });
            if let Some(account) = state.accounts.get_mut(row) {
                account.server = server;
            }
        }
        self.paint_account(row);
        self.paint();
    }

    fn account_changed(&self, row: i32, change: impl FnOnce(&mut Account)) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let Some(account) = state.accounts.get_mut(row) else {
                return;
            };
            change(account);
        }
        self.paint_account(row);
        self.paint();
    }

    // ---- 9 Options -------------------------------------------------------------------------

    fn option_toggled(&self, which: i32) {
        {
            let mut state = self.state.borrow_mut();
            let options = &mut state.options;
            match which {
                0 => options.desktop_shortcut = !options.desktop_shortcut,
                1 => options.start_menu_shortcut = !options.start_menu_shortcut,
                2 => options.watchdog = !options.watchdog,
                // Only a Runtime the wizard started takes the first-start pause, so the pause goes
                // with the start (`plan_problems`): off with it, and back on, as advised, with it.
                3 => {
                    options.start_after_install = !options.start_after_install;
                    options.pause_first_start = options.start_after_install;
                }
                4 if options.start_after_install => options.pause_first_start = !options.pause_first_start,
                _ => {}
            }
        }
        self.paint();
    }

    // ---- 10, 11 The install ----------------------------------------------------------------

    fn start_install(&self) {
        let request = {
            let mut state = self.state.borrow_mut();
            let Some(set_dir) = state.set_dir.clone() else {
                return;
            };
            // The device bridge the phone page laid into this folder, which the install takes as
            // its own whether or not phones are still chosen; the run's key salt, which the
            // phones were listed with.
            let request = Request {
                set_dir,
                root: state.root(),
                plan: plan_of(&state),
                pre_laid: state.pre_laid_here().cloned(),
                key_salt: state.key_salt.as_ref().ok().cloned(),
            };
            state.install = InstallState::Running;
            state.marks = vec![StepMark::Waiting; Step::ALL.len()];
            state.current = None;
            state.detail = None;
            state.bytes = None;
            state.page = INSTALL;
            request
        };
        let spawned = spawn("acsetup-install", move || {
            let mut shown = u64::MAX;
            let result = install(&request, &mut |event: Event| match event {
                Event::Bytes { done, total } => {
                    let permille = if total == 0 { 1000 } else { done.saturating_mul(1000) / total };
                    if permille != shown {
                        shown = permille;
                        post(move |app| app.install_event(Event::Bytes { done, total }));
                    }
                }
                other => post(move |app| app.install_event(other)),
            });
            post(move |app| app.install_finished(result));
        });
        if let Err(error) = spawned {
            let failure = Failure {
                step: None,
                reason: Said::new(fill(ZH.worker_failed, &[&error]), fill(EN.worker_failed, &[&error])),
                log: None,
                wrote: false,
                left: Left::default(),
            };
            self.install_finished(Err(failure));
            return;
        }
        self.paint();
    }

    fn install_event(&self, event: Event) {
        {
            let mut state = self.state.borrow_mut();
            let at = |step: Step| step.number().saturating_sub(1);
            match event {
                Event::Started(step) => {
                    if let Some(mark) = state.marks.get_mut(at(step)) {
                        *mark = StepMark::Running;
                    }
                    state.current = Some(at(step));
                    state.detail = None;
                    state.bytes = None;
                }
                Event::Finished(step, _) => {
                    if let Some(mark) = state.marks.get_mut(at(step)) {
                        *mark = StepMark::Finished;
                    }
                    state.current = None;
                }
                Event::Skipped(step, _) => {
                    if let Some(mark) = state.marks.get_mut(at(step)) {
                        *mark = StepMark::Skipped;
                    }
                    state.current = None;
                }
                Event::Failed(step, reason) => {
                    if let Some(mark) = state.marks.get_mut(at(step)) {
                        *mark = StepMark::Failed;
                    }
                    state.current = Some(at(step));
                    state.detail = Some(reason);
                }
                // The checks before the first step say what they do in `Plain` lines; their
                // full lines (paths, repositories, commits) are the log's.
                Event::Line(line) => {
                    if state.marks.iter().any(|mark| *mark != StepMark::Waiting) {
                        state.detail = Some(line);
                    }
                }
                Event::Plain(line) => state.detail = Some(line),
                Event::Bytes { done, total } => state.bytes = Some((done, total)),
            }
        }
        self.paint();
    }

    fn install_finished(&self, result: Result<Done, Failure>) {
        {
            let mut state = self.state.borrow_mut();
            state.install = match result {
                Ok(done) => InstallState::Done(Box::new(done)),
                Err(failure) => InstallState::Failed(Box::new(failure)),
            };
            state.page = FINISH;
            state.ask = None;
        }
        self.paint();
    }

    // ---- 12 Finish -------------------------------------------------------------------------

    fn act(&self, outcome: std::io::Result<()>) {
        {
            let mut state = self.state.borrow_mut();
            state.action_error = outcome.err().map(|error| error.to_string());
        }
        self.paint();
    }

    fn open_console(&self) {
        let root = self.state.borrow().root();
        self.act(desktop::start_console(&root));
    }

    fn open_folder(&self) {
        let root = self.state.borrow().root();
        self.act(desktop::open_with_explorer(&root));
    }

    fn open_log(&self) {
        let log = log_path(&self.state.borrow());
        match log {
            Some(log) => self.act(desktop::open_with_explorer(&log)),
            None => self.act(Err(std::io::Error::new(std::io::ErrorKind::NotFound, LOG_FILE))),
        }
    }
}

// ---- What each page shows ---------------------------------------------------------------------

fn can_next(state: &State) -> bool {
    match state.page {
        WELCOME => true,
        // Nothing ticked is a choice too: only the core, no instance.
        USAGE => true,
        FILES => state.verified().is_some(),
        LOCATION => state.root_usable(),
        GAMES => state.games.iter().any(|(_, picked)| *picked),
        EMULATORS => matches!(state.emu, EmuState::Done { .. }),
        // Once listed, with any phones ticked or none; never while it works or after it failed.
        PHONES => matches!(state.phone, PhoneState::Listed { .. }),
        ACCOUNTS => account_problems(state).is_empty(),
        // The options have no rule among themselves (`acui_wizard::plan::plan_problems`).
        OPTIONS => true,
        CONFIRM => matches!(state.install, InstallState::NotStarted),
        _ => false,
    }
}

/// The setup log: once the install has begun, where it is or will be.
fn log_path(state: &State) -> Option<PathBuf> {
    match &state.install {
        InstallState::NotStarted => None,
        InstallState::Running => Some(state.root().join(LOG_FILE)),
        InstallState::Done(done) => Some(done.log.clone()),
        InstallState::Failed(failure) => failure.log.clone(),
    }
}

fn game_labels(games: &[Game], separator: &str) -> String {
    games.iter().map(|game| game.label.as_str()).collect::<Vec<_>>().join(separator)
}

/// `bytes` in GB, rounded up to a half.
fn half_gb(bytes: u64) -> String {
    let halves = (bytes as f64 / f64::from(1u32 << 30) * 2.0).ceil() / 2.0;
    format!("{}", halves.max(0.5))
}

fn paint_welcome(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let what = match (state.verified(), &state.preview) {
        (Some(set), _) => fill(
            l.welcome_what,
            &[&set.set.runtime.tag, &set.set.console.tag, &game_labels(&set.games(), l.list_separator)],
        ),
        (None, Some(preview)) => fill(
            l.welcome_what,
            &[&preview.set.runtime.tag, &preview.set.console.tag, &preview.games.join(l.list_separator)],
        ),
        (None, None) => l.welcome_what_unknown.to_owned(),
    };
    wiz.set_welcome_what(what.into());
    let gb = match state.verified() {
        Some(_) => half_gb(state.space_needed()),
        None => SPACE_GUESS_GB.to_owned(),
    };
    wiz.set_welcome_space(fill(l.req_space, &[&gb]).into());
}

fn paint_usage(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    wiz.set_usage_mumu(state.use_mumu);
    wiz.set_usage_android(state.use_android);
    // With nothing ticked, what the install will be and how instances come later.
    wiz.set_usage_note(if state.use_mumu || state.use_android { "" } else { l.usage_none }.into());
}

fn paint_files(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let lang = state.lang;
    let place = state
        .set_dir
        .as_ref()
        .filter(|_| !matches!(state.set, SetState::Missing { .. }))
        .map_or_else(String::new, |dir| fill(l.set_where, &[&strip_verbatim(&dir.to_string_lossy())]));
    wiz.set_set_where(place.into());
    let mut status = String::new();
    let mut busy = false;
    let mut progress = 0.0f32;
    let mut line = String::new();
    let mut error = String::new();
    let mut detail = String::new();
    let mut summary = Vec::new();
    let mut choosable = false;
    match &state.set {
        SetState::Missing { wrong_pick } => {
            status = if *wrong_pick { l.set_not_here } else { l.set_missing }.to_owned();
            choosable = true;
        }
        SetState::Waiting => {
            status = l.set_checking.to_owned();
            busy = true;
        }
        SetState::Checking { done, total, line: said } => {
            status = l.set_checking.to_owned();
            busy = true;
            progress = if *total == 0 { 0.0 } else { (*done as f64 / *total as f64) as f32 };
            line = said.as_ref().map_or_else(String::new, |said| said.in_lang(lang).to_owned());
        }
        SetState::Ready(set) => {
            let files = set.set.files();
            let bytes: u64 = files.iter().map(|(_, file)| file.size).sum();
            status = fill(l.set_checked, &[&files.len().to_string(), &plain_size(bytes)]);
            summary.push(KeyValue {
                key: l.key_version.into(),
                value: fill(l.version_value, &[&set.set.runtime.tag, &set.set.console.tag]).into(),
            });
            summary.push(KeyValue {
                key: l.key_games.into(),
                value: game_labels(&set.games(), l.list_separator).into(),
            });
        }
        SetState::Failed(failure) => {
            error = match &failure.fault {
                SetFault::Damaged { file } => fill(l.set_damaged, &[file]),
                SetFault::Mismatch { game } => fill(l.set_mismatch, &[game]),
                SetFault::ConsoleMismatch => l.set_console_mismatch.to_owned(),
                SetFault::Unusable => l.set_unusable.to_owned(),
            };
            detail = fill(l.details, &[failure.said.in_lang(lang)]);
            choosable = true;
        }
    }
    if let Some(picker) = &state.picker_error {
        error = fill(l.dialog_failed, &[picker]);
    }
    wiz.set_set_status(status.into());
    wiz.set_set_busy(busy);
    wiz.set_set_progress(progress);
    wiz.set_set_line(line.into());
    wiz.set_set_error(error.into());
    wiz.set_set_error_detail(detail.into());
    wiz.set_set_summary(model(summary));
    wiz.set_set_choosable(choosable);
}

fn root_line(line: &RootLine, l: &Labels) -> (i32, String) {
    match line {
        RootLine::Fresh => (1, l.root_fresh.to_owned()),
        RootLine::Empty => (1, l.root_empty.to_owned()),
        RootLine::PreLaid => (1, l.root_pre_laid.to_owned()),
        RootLine::Space { free, need } => (1, fill(l.root_space, &[&plain_size(*free), &plain_size(*need)])),
        RootLine::Contains(root) => (2, fill(l.root_contains, &[root])),
        RootLine::Worker(error) => (2, fill(l.worker_failed, &[error])),
        RootLine::Issue(issue) => (
            2,
            match issue {
                RootIssue::Empty | RootIssue::NotAbsolute => l.root_full_path.to_owned(),
                RootIssue::NetworkPath => l.root_network.to_owned(),
                RootIssue::DriveRoot => l.root_drive_root.to_owned(),
                RootIssue::SlotName => l.root_ab.to_owned(),
                RootIssue::Quote | RootIssue::Percent => l.root_symbols.to_owned(),
                RootIssue::NotAFolder => l.root_not_folder.to_owned(),
                RootIssue::NotEmpty => l.root_not_empty.to_owned(),
                RootIssue::Unreadable(_) => l.root_unreadable.to_owned(),
                RootIssue::InsideRoot(other) => fill(l.root_inside, &[other]),
                RootIssue::PerUserFolder(folder) => fill(l.root_per_user, &[folder]),
                RootIssue::NoSpace { needed, free } => fill(l.root_no_space, &[&plain_size(*needed), &plain_size(*free)]),
                RootIssue::SpaceUnknown(_) => l.root_space_unknown.to_owned(),
            },
        ),
    }
}

fn paint_location(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let mut lines = Vec::new();
    let mut status = String::new();
    match &state.root_report {
        Some((generation, report)) if *generation == state.root_generation => {
            let mut seen = BTreeSet::new();
            for line in &report.lines {
                let (mark, text) = root_line(line, l);
                if seen.insert(text.clone()) {
                    lines.push(MarkLine { mark, text: text.into(), detail: SharedString::new() });
                }
            }
        }
        _ => status = l.root_checking.to_owned(),
    }
    if let Some(picker) = &state.picker_error {
        status = fill(l.dialog_failed, &[picker]);
    }
    wiz.set_root_lines(model(lines));
    wiz.set_root_status(status.into());
}

fn paint_games(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let rows: Vec<GameRow> = state
        .games
        .iter()
        .map(|(game, picked)| GameRow {
            text: fill(
                if game.servers.len() == 1 { l.games_row_one } else { l.games_row },
                &[&game.label, &game.servers.len().to_string()],
            )
            .into(),
            picked: *picked,
        })
        .collect();
    wiz.set_games(model(rows));
    let none = !state.games.iter().any(|(_, picked)| *picked);
    wiz.set_games_problem(if none { l.games_none } else { "" }.into());
}

/// Whether the version MuMu reported is older than the Runtime takes.
fn too_old(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return false;
    };
    let parts: Vec<u32> = version.split('.').filter_map(|part| part.parse().ok()).collect();
    parts.len() == 4 && [parts[0], parts[1], parts[2], parts[3]] < MINIMUM_VERSION
}

fn paint_emulators(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let mut text = l.emu_searching.to_owned();
    let mut busy = true;
    let mut caution = String::new();
    let mut skippable = false;
    if let EmuState::Done { discovery, others } = &state.emu {
        busy = false;
        text = match &discovery.problem {
            Some(_) if discovery.absent => l.emu_absent.to_owned(),
            Some(_) if too_old(discovery.version.as_deref()) => {
                fill(l.emu_old, &[discovery.version.as_deref().unwrap_or("?")])
            }
            Some(problem) => fill(l.emu_problem, &[&said(problem, state.lang)]),
            None if state.emu_rows.is_empty() => l.emu_empty.to_owned(),
            None => l.emu_body.to_owned(),
        };
        skippable = discovery.problem.is_some() || state.emu_rows.is_empty();
        let unknown: Vec<String> = others
            .iter()
            .filter(|other| other.instances.is_none())
            .map(|other| other.name())
            .collect();
        if !unknown.is_empty() {
            caution = fill(l.emu_unknown, &[&unknown.join(l.list_separator)]);
        }
    }
    let rows: Vec<EmulatorRow> = state
        .emu_rows
        .iter()
        .map(|row| EmulatorRow {
            text: format!(
                "#{} {} · {}",
                row.instance.index,
                row.instance.name,
                if row.instance.running { l.emu_running } else { l.emu_stopped }
            )
            .into(),
            taken: row.taken.as_ref().map_or_else(String::new, |root| fill(l.emu_taken, &[root])).into(),
            picked: row.picked,
        })
        .collect();
    wiz.set_emu_text(text.into());
    wiz.set_emu_busy(busy);
    wiz.set_emu_caution(caution.into());
    wiz.set_emulators(model(rows));
    wiz.set_emu_skippable(skippable);
}

fn paint_phones(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let lang = state.lang;
    // The word "instance" is explained here when no page before did (without MuMu).
    wiz.set_phones_body(fill(l.phones_body, &[if state.use_mumu { "" } else { l.instance_explained }]).into());
    // The one write before the install, said while it is laid and once it is there.
    let tools = strip_verbatim(&state.root().join(Program::Tools.dir()).to_string_lossy());
    let noted = state.laying() || state.pre_laid_here().is_some();
    wiz.set_phones_note(if noted { fill(l.phones_write_note, &[&tools]) } else { String::new() }.into());
    let mut text = String::new();
    let mut busy = false;
    let mut progress = 0.0f32;
    let mut indeterminate = true;
    let mut line = String::new();
    let mut problem = String::new();
    let mut caution = String::new();
    let mut refreshable = false;
    match &state.phone {
        PhoneState::Idle => {}
        PhoneState::Laying { line: last, done, total } => {
            text = l.phones_laying.to_owned();
            busy = true;
            if *total > 0 {
                progress = (*done as f64 / *total as f64) as f32;
                indeterminate = false;
            }
            line = last.as_ref().map_or_else(String::new, |last| last.in_lang(lang).to_owned());
        }
        PhoneState::Asking => {
            text = l.phones_asking.to_owned();
            busy = true;
        }
        PhoneState::Listing => {
            text = l.phones_listing.to_owned();
            busy = true;
        }
        PhoneState::Listed { keys_unmade } => {
            text = if state.phone_rows.is_empty() { l.phones_none } else { l.phones_found }.to_owned();
            refreshable = true;
            if let Some(why) = keys_unmade {
                caution = fill(l.phones_keys_unmade, &[why]);
            }
        }
        PhoneState::NoSalt(said_problem) => problem = fill(l.phones_salt_failed, &[&said(said_problem, lang)]),
        PhoneState::LayFailed(failure) => {
            problem = fill(l.phones_lay_failed, &[&said(&failure.reason, lang)]);
            problem.push('\n');
            problem.push_str(&if failure.wrote {
                fill(l.phones_lay_left, &[&strip_verbatim(&failure.dir.to_string_lossy())])
            } else {
                l.phones_lay_nothing.to_owned()
            });
            // It lays again, then asks and lists ([`App::refresh_phones`]).
            refreshable = true;
        }
        PhoneState::Unable(missing) => {
            problem = l.phones_unable.to_owned();
            problem.push('\n');
            problem.push_str(&fill(l.phones_lacks, &[&missing.join(l.list_separator)]));
        }
        PhoneState::Failed(said_problem) => {
            problem = fill(l.phones_list_failed, &[&said(said_problem, lang)]);
            refreshable = true;
        }
    }
    // The rows of the last list, while it is the one shown.
    let rows: Vec<PhoneRow> = if matches!(state.phone, PhoneState::Listed { .. }) {
        state.phone_rows.iter().map(|entry| phone_row(entry, l)).collect()
    } else {
        Vec::new()
    };
    wiz.set_phones_text(text.into());
    wiz.set_phones_busy(busy);
    wiz.set_phones_progress(progress);
    wiz.set_phones_indeterminate(indeterminate);
    wiz.set_phones_line(line.into());
    wiz.set_phones_problem(problem.into());
    wiz.set_phones_caution(caution.into());
    wiz.set_phones(model(rows));
    wiz.set_phones_refreshable(refreshable);
}

/// One row of the phone page: its model (else "unknown model"), its connection and its state in
/// a person's words (else as adb says it), and what to do about a row that cannot be ticked.
fn phone_row(entry: &DeviceEntry, l: &Labels) -> PhoneRow {
    let row = &entry.row;
    let transport = match row.transport.as_str() {
        devices::USB => l.transport_usb,
        "tcp" => l.transport_tcp,
        "" => "?",
        other => other,
    };
    let state = match row.state.as_str() {
        devices::DEVICE => l.state_device,
        "unauthorized" => l.state_unauthorized,
        "offline" => l.state_offline,
        "no" => l.state_no,
        "" => "?",
        other => other,
    };
    let usable = row_use(row) == RowUse::Usable;
    let hint = match row_use(row) {
        RowUse::Usable => String::new(),
        RowUse::Unauthorized => l.hint_unauthorized.to_owned(),
        RowUse::NoDriver => l.hint_driver.to_owned(),
        RowUse::NotHandset => l.hint_tcp.to_owned(),
        RowUse::NoKey => l.hint_no_key.to_owned(),
        RowUse::Other => fill(l.hint_other, &[&row.state]),
    };
    PhoneRow {
        text: fill(l.phone_row, &[row.model.as_deref().unwrap_or(l.unknown_model), transport, state]).into(),
        hint: hint.into(),
        picked: entry.picked && usable,
        usable,
    }
}

fn only_server(game: &Game) -> Option<String> {
    match game.servers.as_slice() {
        [server] => Some(server.server.clone()),
        _ => None,
    }
}

/// The name the instance page offers: an emulator's `<game>-<index>` ("MuMu" before a game is
/// chosen), a phone's `<game>-phone<number>` (its model before a game is chosen), each within its
/// game's alias limit and a phone's within the Runtime's limit for a phone too.
fn account_default_alias(account: &Account, game: Option<&Game>) -> String {
    match (&account.device_key, game) {
        (None, Some(game)) => default_alias(&game.label, &game.game, account.index, game.alias_limit),
        (None, None) => default_alias("MuMu", "", account.index, MAX_ALIAS_BYTES),
        (Some(_), Some(game)) => default_phone_alias(
            &game.label,
            &game.game,
            usize::from(account.index),
            game.alias_limit.min(HANDSET_ALIAS_LIMIT),
        ),
        (Some(_), None) => default_phone_alias(&account.name, "", usize::from(account.index), HANDSET_ALIAS_LIMIT),
    }
}

/// No emulator handed over and no emulator's instance, as "Skip this step" leaves them; what the
/// search found stays listed, and the phones' instances stay.
fn drop_emulators(state: &mut State) {
    for row in &mut state.emu_rows {
        row.picked = false;
        row.confirmed = false;
    }
    state.accounts.retain(|account| account.device_key.is_some());
}

/// No phone ticked and no phone's instance, and the phone page let go of: ticked again, it lists
/// afresh. The device bridge it laid stays on the disk and in `State::pre_laid`.
fn drop_phones(state: &mut State) {
    state.phone_rows.clear();
    state.accounts.retain(|account| account.device_key.is_none());
    state.phone = PhoneState::Idle;
    state.phone_root = None;
    state.phone_generation += 1;
}

/// The accounts for the emulators ticked, then for the phones ticked, keeping what was set for
/// each already (an emulator's by its index, a phone's by its device key): a game no longer
/// ticked is let go, a lone game or server is taken, an untouched name follows its game.
fn sync_accounts(state: &mut State) {
    let chosen = state.chosen_games();
    let mut accounts = Vec::new();
    for row in state.emu_rows.iter().filter(|row| row.picked) {
        let mut account = state
            .accounts
            .iter()
            .find(|account| account.device_key.is_none() && account.index == row.instance.index)
            .cloned()
            .unwrap_or_else(|| Account {
                index: row.instance.index,
                name: row.instance.name.clone(),
                alias: String::new(),
                edited: false,
                game: None,
                server: None,
                touch: DEFAULT_BACKEND.to_owned(),
                capture: DEFAULT_BACKEND.to_owned(),
                start_emulator: false,
                confirmed_in_use: false,
                advanced: false,
                device_key: None,
            });
        account.name = row.instance.name.clone();
        account.confirmed_in_use = row.confirmed;
        accounts.push(account);
    }
    // Only a row with its key is ever ticked (`App::phone_toggled`).
    let phones = state
        .phone_rows
        .iter()
        .filter(|entry| entry.picked)
        .filter_map(|entry| Some((entry, entry.row.device_key.as_ref()?)));
    for (at, (entry, key)) in phones.enumerate() {
        let mut account = state
            .accounts
            .iter()
            .find(|account| account.device_key.as_ref() == Some(key))
            .cloned()
            .unwrap_or_else(|| Account {
                index: 0,
                name: String::new(),
                alias: String::new(),
                edited: false,
                game: None,
                server: None,
                touch: HANDSET_TOUCH_DEFAULT.to_owned(),
                capture: HANDSET_CAPTURE_DEFAULT.to_owned(),
                start_emulator: false,
                confirmed_in_use: false,
                advanced: false,
                device_key: Some(key.clone()),
            });
        account.index = u16::try_from(at + 1).unwrap_or(u16::MAX);
        account.name = entry.row.model.clone().unwrap_or_default();
        accounts.push(account);
    }
    for account in &mut accounts {
        if let Some(folder) = &account.game {
            if !chosen.iter().any(|game| game.game == *folder) {
                account.game = None;
                account.server = None;
            }
        }
        if account.game.is_none() {
            if let [game] = chosen.as_slice() {
                account.game = Some(game.game.clone());
                account.server = only_server(game);
            }
        }
        let game = account.game.as_ref().and_then(|folder| chosen.iter().find(|game| game.game == *folder));
        if !account.edited {
            account.alias = account_default_alias(account, game);
        }
    }
    state.accounts = accounts;
}

/// The head of an account's lines, as the instance and confirm pages name it: the emulator's
/// index and name, or the phone and its model.
fn account_label(account: &Account, l: &Labels) -> String {
    match &account.device_key {
        None => format!("#{} {}", account.index, account.name),
        Some(_) => fill(l.account_phone, &[if account.name.is_empty() { l.unknown_model } else { account.name.as_str() }]),
    }
}

/// The page's note for each kind of instance it holds, each on its own line.
fn accounts_note(state: &State) -> String {
    let l = state.labels();
    let mut notes = Vec::new();
    if state.accounts.iter().any(|account| account.device_key.is_none()) {
        notes.push(l.accounts_emulators_note);
    }
    if state.accounts.iter().any(|account| account.device_key.is_some()) {
        notes.push(l.accounts_phones_note);
    }
    notes.join("\n")
}

fn account_row(account: &Account, chosen: &[Game], l: &Labels) -> AccountRow {
    let game = account.game.as_ref().and_then(|folder| chosen.iter().find(|game| game.game == *folder));
    let game_index = account
        .game
        .as_ref()
        .and_then(|folder| chosen.iter().position(|game| game.game == *folder))
        .map_or(0, |at| at + 1);
    let servers: Vec<&str> = std::iter::once(l.pick)
        .chain(game.into_iter().flat_map(|game| game.servers.iter().map(|server| server.label.as_str())))
        .collect();
    let server_index = match (game, &account.server) {
        (Some(game), Some(server)) => game.servers.iter().position(|entry| entry.server == *server).map_or(0, |at| at + 1),
        _ => 0,
    };
    // A phone's backends are a phone's own lists (`Strings.phone-touch-options`).
    let phone = account.device_key.is_some();
    let touches: &[&str] = if phone { &HANDSET_TOUCH_BACKENDS } else { &TOUCH_BACKENDS };
    let captures: &[&str] = if phone { &HANDSET_CAPTURE_BACKENDS } else { &CAPTURE_BACKENDS };
    AccountRow {
        emulator: account_label(account, l).into(),
        alias: account.alias.as_str().into(),
        games: shared(std::iter::once(l.pick).chain(chosen.iter().map(|game| game.label.as_str()))),
        game_index: game_index as i32,
        servers: shared(servers),
        server_index: server_index as i32,
        touch_index: touches.iter().position(|backend| *backend == account.touch).unwrap_or(0) as i32,
        capture_index: captures.iter().position(|backend| *backend == account.capture).unwrap_or(0) as i32,
        start_emulator: account.start_emulator,
        advanced: account.advanced,
        phone,
    }
}

/// What stops the accounts page, each once.
fn account_problems(state: &State) -> Vec<String> {
    let l = state.labels();
    let lang = state.lang;
    let mut problems: Vec<String> = Vec::new();
    let mut push = |text: String| {
        if !problems.contains(&text) {
            problems.push(text);
        }
    };
    if state.accounts.iter().any(|account| account.game.is_none() || account.server.is_none()) {
        push(l.accounts_choose.to_owned());
    }
    let mut names = BTreeSet::new();
    if state.accounts.iter().any(|account| !names.insert(account.alias.as_str())) {
        push(l.accounts_duplicate.to_owned());
    }
    for account in &state.accounts {
        let phone = account.device_key.is_some();
        let limit = account
            .game
            .as_deref()
            .and_then(|folder| state.game(folder))
            .map_or(MAX_ALIAS_BYTES, |game| game.alias_limit);
        // A phone's alias within the Runtime's limit for a phone too (`plan_problems`).
        let limit = if phone { limit.min(HANDSET_ALIAS_LIMIT) } else { limit };
        if let Some(problem) = alias_problem(&account.alias, limit) {
            push(format!("{}{}", said(&problem, lang), if lang == Lang::Zh { "。" } else { "." }));
        }
        let backends = if phone {
            handset_backend_problem(&account.touch, &account.capture)
        } else {
            backend_problem(&account.touch, &account.capture)
        };
        if let Some(problem) = backends {
            push(fill(l.accounts_backend, &[&account_label(account, l), &said(&problem, lang)]));
        }
    }
    problems
}

fn paint_options(state: &State, wiz: &Wiz<'_>) {
    let options = &state.options;
    wiz.set_opt_desktop(options.desktop_shortcut);
    wiz.set_opt_start_menu(options.start_menu_shortcut);
    wiz.set_opt_watchdog(options.watchdog);
    wiz.set_opt_start(options.start_after_install);
    wiz.set_opt_pause(options.pause_first_start);
}

fn plan_of(state: &State) -> Plan {
    let instances: Vec<PlanInstance> = state
        .accounts
        .iter()
        .filter(|account| account.device_key.is_none())
        .map(|account| PlanInstance {
            index: account.index,
            alias: account.alias.clone(),
            game: account.game.clone().unwrap_or_default(),
            server: account.server.clone().unwrap_or_default(),
            touch_backend: account.touch.clone(),
            capture_backend: account.capture.clone(),
            start_emulator: account.start_emulator,
            confirmed_in_use: account.confirmed_in_use,
        })
        .collect();
    // Each phone by the key the phone page listed it with, which the install's bind finds it by.
    let handsets: Vec<PlanHandset> = state
        .accounts
        .iter()
        .filter_map(|account| {
            Some(PlanHandset {
                alias: account.alias.clone(),
                game: account.game.clone().unwrap_or_default(),
                server: account.server.clone().unwrap_or_default(),
                touch_backend: account.touch.clone(),
                capture_backend: account.capture.clone(),
                device_key: account.device_key.clone()?,
            })
        })
        .collect();
    Plan {
        schema_version: PLAN_SCHEMA.to_owned(),
        language: state.lang.wire().to_owned(),
        games: state.chosen_games().into_iter().map(|game| game.game).collect(),
        mumu_root: if instances.is_empty() { None } else { state.mumu_root() },
        instances,
        handsets,
        options: state.options.clone(),
    }
}

fn paint_confirm(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let root = state.root_text.trim().to_owned();
    let mut summary = vec![KeyValue { key: l.key_root.into(), value: root.as_str().into() }];
    if let Some(set) = state.verified() {
        summary.push(KeyValue {
            key: l.key_version.into(),
            value: fill(l.version_value, &[&set.set.runtime.tag, &set.set.console.tag]).into(),
        });
    }
    summary.push(KeyValue {
        key: l.key_games.into(),
        value: game_labels(&state.chosen_games(), l.list_separator).into(),
    });
    let usages: Vec<&str> = [(state.use_mumu, l.usage_mumu), (state.use_android, l.usage_android)]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect();
    summary.push(KeyValue {
        key: l.key_usage.into(),
        value: if usages.is_empty() { l.usage_none_short.to_owned() } else { usages.join(l.list_separator) }.into(),
    });
    // How many phones, and their instances' names.
    let phones: Vec<&str> = state
        .accounts
        .iter()
        .filter(|account| account.device_key.is_some())
        .map(|account| account.alias.as_str())
        .collect();
    if state.use_android {
        summary.push(KeyValue {
            key: l.key_phones.into(),
            value: if phones.is_empty() {
                l.confirm_phones_none.to_owned()
            } else {
                fill(l.phones_value, &[&phones.len().to_string(), &phones.join(l.list_separator)])
            }
            .into(),
        });
    }
    let accounts: Vec<String> = state
        .accounts
        .iter()
        .map(|account| {
            let game = account.game.as_deref().and_then(|folder| state.game(folder));
            let server = game
                .zip(account.server.as_deref())
                .and_then(|(game, server)| game.server(server))
                .map_or("?", |server| server.label.as_str());
            let mut line = fill(
                l.account_value,
                &[
                    &account_label(account, l),
                    &account.alias,
                    game.map_or("?", |game| game.label.as_str()),
                    server,
                ],
            );
            // What the advanced fold changed from its defaults, so the list is the whole plan.
            let (touch, capture) = match account.device_key {
                None => (DEFAULT_BACKEND, DEFAULT_BACKEND),
                Some(_) => (HANDSET_TOUCH_DEFAULT, HANDSET_CAPTURE_DEFAULT),
            };
            if account.touch != touch || account.capture != capture {
                line.push_str(&format!(
                    " · {} {} / {} {}",
                    l.touch_label, account.touch, l.capture_label, account.capture
                ));
            }
            if account.start_emulator {
                line.push_str(" · ");
                line.push_str(l.start_emulator_short);
            }
            line
        })
        .collect();
    summary.push(KeyValue {
        key: l.key_accounts.into(),
        value: if accounts.is_empty() { l.accounts_none.to_owned() } else { accounts.join("\n") }.into(),
    });
    let options = &state.options;
    let chosen: Vec<&str> = [
        options.desktop_shortcut,
        options.start_menu_shortcut,
        options.watchdog,
        options.start_after_install,
        options.pause_first_start && options.start_after_install,
    ]
    .iter()
    .zip(l.options_short)
    .filter(|(on, _)| **on)
    .map(|(_, name)| name)
    .collect();
    summary.push(KeyValue {
        key: l.key_options.into(),
        value: if chosen.is_empty() { l.options_none.to_owned() } else { chosen.join(l.list_separator) }.into(),
    });
    // Said where it is decided: instances that go on duty at the first start.
    if !state.accounts.is_empty() && !(options.start_after_install && options.pause_first_start) {
        summary.push(KeyValue { key: l.key_attention.into(), value: l.attention_unpaused.into() });
    }
    // What the install will make of these choices: a licence folder for a game that brings
    // licence files or for the text-recognition models' licences, and the full configuration kept
    // while the first start runs without it.
    let licences = state.verified().is_some_and(|set| {
        !set.set.vision.licenses.is_empty()
            || state
                .chosen_games()
                .iter()
                .any(|game| set.bundle(&game.game).is_some_and(|bundle| !bundle.entry.files.is_empty()))
    });
    let mut folders = l.folders_value.to_owned();
    if licences {
        folders.push_str(l.list_separator);
        folders.push_str(l.folder_licenses);
    }
    let mut files = l.files_value.to_owned();
    if !state.accounts.is_empty() && options.start_after_install && options.pause_first_start {
        files.push_str(l.list_separator);
        files.push_str(l.file_full_config);
    }
    if !phones.is_empty() {
        files.push_str(l.list_separator);
        files.push_str(l.file_bindings);
    }
    summary.push(KeyValue { key: l.key_folders.into(), value: folders.into() });
    summary.push(KeyValue { key: l.key_files.into(), value: files.into() });
    wiz.set_summary(model(summary));
    wiz.set_confirm_note(fill(l.confirm_note, &[&root]).into());
}

fn paint_install(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let begun = state.marks.iter().any(|mark| *mark != StepMark::Waiting);
    wiz.set_install_pre(if begun { "" } else { l.install_pre }.into());
    let lines: Vec<MarkLine> = state
        .marks
        .iter()
        .enumerate()
        .map(|(index, mark)| MarkLine {
            mark: match mark {
                StepMark::Waiting => 3,
                StepMark::Running => 4,
                StepMark::Finished => 1,
                StepMark::Failed => 2,
                StepMark::Skipped => 5,
            },
            text: l.install_steps.get(index).copied().unwrap_or("").into(),
            detail: if *mark == StepMark::Skipped { l.install_skipped } else { "" }.into(),
        })
        .collect();
    wiz.set_install_steps(model(lines));
    wiz.set_install_current(state.current.map_or(-1, |current| current as i32));
    wiz.set_install_detail(state.detail.as_ref().map_or_else(String::new, |detail| detail.in_lang(state.lang).to_owned()).into());
    let (progress, indeterminate) = match state.bytes {
        Some((done, total)) if total > 0 => ((done as f64 / total as f64) as f32, false),
        _ => (0.0, true),
    };
    wiz.set_install_progress(progress);
    wiz.set_install_indeterminate(indeterminate);
}

fn paint_finish(state: &State, wiz: &Wiz<'_>) {
    let l = state.labels();
    let lang = state.lang;
    let root = strip_verbatim(&state.root().to_string_lossy());
    wiz.set_finish_action(state.action_error.as_ref().map_or_else(String::new, |error| fill(l.open_failed, &[error])).into());
    match &state.install {
        InstallState::Done(done) => {
            // Each instance is named only when there are some: without MuMu, or with no emulator
            // handed over, there is none to be paused or to look at, and how to add them later is
            // said below either way.
            let instances = done.instances > 0;
            let body = if done.started && done.paused && instances {
                l.finish_paused
            } else if done.started {
                l.finish_running
            } else if instances {
                l.finish_installed_unpaused
            } else {
                l.finish_installed
            };
            let mut next = if !instances {
                ""
            } else if done.paused {
                l.finish_next
            } else {
                l.finish_next_plain
            }
            .to_owned();
            // The phones' instances bound, and what they do today.
            if !done.phones.is_empty() {
                if !next.is_empty() {
                    next.push('\n');
                }
                next.push_str(&fill(l.finish_phones, &[&done.phones.join(l.list_separator)]));
            }
            wiz.set_finished_ok(true);
            wiz.set_finish_body(fill(body, &[&root]).into());
            wiz.set_finish_next(next.into());
            wiz.set_finish_more(l.finish_more.into());
            let mut note = String::new();
            if !done.shortcuts.is_empty() {
                let made: Vec<String> = done.shortcuts.iter().map(|path| path.display().to_string()).collect();
                note.push_str(&fill(l.finish_shortcuts, &[&made.join(l.list_separator)]));
                note.push('\n');
            }
            note.push_str(&if done.watchdog {
                fill(l.uninstall_note_watchdog, &[&root, &root, &root])
            } else {
                fill(l.uninstall_note, &[&root])
            });
            // The device bridge laid into the installed folder is its own `tools`; one laid
            // anywhere else stays there.
            let tools = done.root.join(Program::Tools.dir());
            if let Some(elsewhere) = laid_elsewhere(state, Some(tools.as_path())) {
                note.push('\n');
                note.push_str(&elsewhere);
            }
            wiz.set_finish_note(note.into());
            wiz.set_finish_transcript(SharedString::new());
            wiz.set_finish_has_log(true);
        }
        InstallState::Failed(failure) => {
            let step = failure
                .step
                .map_or(l.failed_checks_step, |step| l.install_steps.get(step.number().saturating_sub(1)).copied().unwrap_or(""));
            wiz.set_finished_ok(false);
            wiz.set_finish_body(fill(l.failed_at, &[step, &said(&failure.reason, lang)]).into());
            // What the install left, in the order a person undoes it, then the folder: only
            // what it did (`acui_wizard::cleanup`).
            let next = if failure.wrote {
                let mut text = fill(l.failed_kept, &[&root]);
                for (at, line) in cleanup(&state.root(), &failure.left).iter().enumerate() {
                    text.push_str(&format!("\n{}. {}", at + 1, line.in_lang(lang)));
                }
                text
            } else {
                l.failed_nothing.to_owned()
            };
            wiz.set_finish_next(next.into());
            wiz.set_finish_more(SharedString::new());
            // The device bridge the install took is among what it left (`cleanup`); one laid
            // into another folder stays there too.
            let elsewhere = laid_elsewhere(state, failure.left.pre_laid_tools.as_deref());
            wiz.set_finish_note(elsewhere.unwrap_or_default().into());
            wiz.set_finish_transcript(transcript(state, failure).into());
            wiz.set_finish_has_log(failure.log.is_some());
        }
        _ => {}
    }
}

/// The sentence for the `tools` folders the phone page wrote into but `taken`, the one the
/// install took as its own, or `None` when there are none.
fn laid_elsewhere(state: &State, taken: Option<&Path>) -> Option<String> {
    let l = state.labels();
    let dirs: Vec<String> = state
        .laid_dirs()
        .iter()
        .filter(|dir| Some(dir.as_path()) != taken)
        .map(|dir| strip_verbatim(&dir.to_string_lossy()))
        .collect();
    (!dirs.is_empty()).then(|| fill(l.laid_elsewhere, &[&dirs.join(l.list_separator)]))
}

/// What "Copy error" copies: the failure in both languages, the folder, the log and the set.
fn transcript(state: &State, failure: &Failure) -> String {
    let log = failure.log.as_ref().map_or_else(|| "-".to_owned(), |log| log.display().to_string());
    let set = state.set_dir.as_ref().map_or_else(|| "-".to_owned(), |dir| dir.display().to_string());
    format!(
        "{failure}\n安装位置 / Install folder: {}\n安装记录 / Setup log: {log}\n安装文件 / Setup files: {set}",
        state.root().display()
    )
}

// ---- The phone page's work, on a worker -------------------------------------------------------

/// [`App::phone_work`]'s parts, in order, each handed to the window's thread as it ends: unless
/// `laid`, the device bridge laid into `root` (`acui_wizard::pre_lay_tools`, its bytes and its
/// words for a person heard as they come), then the root's Lab asked whether it can bind a phone
/// before the install (`devices::ask`), then, if it can, the phones listed with the run's `salt`
/// (`devices::list`, the salt in the Lab's environment alone). A part that fails ends the work.
fn phones_worker(set: VerifiedSet, root: PathBuf, salt: KeySalt, laid: bool, generation: u64) {
    if !laid {
        let mut shown = u64::MAX;
        let result = pre_lay_tools(&set, &root, &mut |event: Event| match event {
            Event::Bytes { done, total } => {
                let permille = if total == 0 { 1000 } else { done.saturating_mul(1000) / total };
                if permille != shown {
                    shown = permille;
                    post(move |app| app.phones_progress(generation, done, total, None));
                }
            }
            // Its words for a person; its full lines go into the install log with what it laid.
            Event::Plain(line) => post(move |app| {
                let (done, total) = match &app.state.borrow().phone {
                    PhoneState::Laying { done, total, .. } => (*done, *total),
                    _ => (0, 0),
                };
                app.phones_progress(generation, done, total, Some(line));
            }),
            _ => {}
        });
        let failed = result.is_err();
        post(move |app| app.phones_laid(generation, result));
        if failed {
            return;
        }
    }
    let asked = devices::ask(&root);
    let able = matches!(&asked, Ok(asked) if asked.missing.is_empty());
    let asked_root = root.clone();
    post(move |app| app.phones_asked(generation, asked_root, asked));
    if !able {
        return;
    }
    let listed = devices::list(&root, &salt);
    post(move |app| app.phones_listed(generation, root, listed));
}

// ---- The folder check, on a worker ------------------------------------------------------------

/// The folder `text` checked, with `needed` bytes of room; `laid`: the phone page laid the device
/// bridge into it, so it may hold that `tools` alone (`check_root_beside_tools`; what `tools`
/// holds is the install's to check, file by file).
fn root_report(text: &str, needed: u64, laid: bool) -> RootReport {
    let rules = text_issues(text);
    if !rules.is_empty() {
        return RootReport { lines: rules.into_iter().map(RootLine::Issue).collect(), usable: false };
    }
    let path = Path::new(text.trim());
    let issues = if laid { check_root_beside_tools(path, needed) } else { check_root(path, needed) };
    let mut usable = issues.is_empty();
    let mut lines = Vec::new();
    let place: Vec<&RootIssue> = issues
        .iter()
        .filter(|issue| matches!(issue, RootIssue::NotAFolder | RootIssue::NotEmpty | RootIssue::Unreadable(_)))
        .collect();
    if place.is_empty() {
        lines.push(if laid && path.exists() {
            RootLine::PreLaid
        } else if path.exists() {
            RootLine::Empty
        } else {
            RootLine::Fresh
        });
    }
    for issue in place {
        if *issue == RootIssue::NotEmpty && is_installation_root(path) {
            lines.push(RootLine::Contains(strip_verbatim(&path.to_string_lossy())));
        } else {
            lines.push(RootLine::Issue(issue.clone()));
        }
    }
    for issue in &issues {
        if matches!(issue, RootIssue::InsideRoot(_) | RootIssue::PerUserFolder(_)) {
            lines.push(RootLine::Issue(issue.clone()));
        }
    }
    let space: Vec<&RootIssue> = issues
        .iter()
        .filter(|issue| matches!(issue, RootIssue::NoSpace { .. } | RootIssue::SpaceUnknown(_)))
        .collect();
    if space.is_empty() {
        match platform::free_space(path) {
            Ok(free) => lines.push(RootLine::Space { free, need: needed }),
            Err(reason) => {
                usable = false;
                lines.push(RootLine::Issue(RootIssue::SpaceUnknown(reason.to_string())));
            }
        }
    } else {
        lines.extend(space.into_iter().cloned().map(RootLine::Issue));
    }
    RootReport { lines, usable }
}
