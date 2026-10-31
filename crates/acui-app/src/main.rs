// SPDX-License-Identifier: GPL-3.0-only
//! ActingCommand 监控台 / ActingCommand Console: a read-only window over one
//! Runtime state root, opened through the formal ledger read face — offline
//! over the files, or online through the typed client of the running Runtime —
//! with a launcher block that starts the daemon detached and asks it, through
//! the same typed client, to shut down. A start that fails is shown in a window
//! of its own (`report`); what the settings file holds that the console cannot
//! take is shown at the top of the window. Every word is put together when it
//! is painted, in the language shown then (`ui`, `phrase`), so a language
//! picked in the box takes effect at once. The ledger window is painted from
//! what the reads here leave in its state (`ledger`), never from the read face
//! itself.

mod install;
mod launcher;
mod ledger;
mod phrase;
mod report;
mod settings;
mod strings;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use acui_model::{
    tab_from_name, DisplayRow, QueryError, ViewModel, ALL_TABS, FILL_ROWS, FILL_WINDOWS, WINDOW,
};
use acui_rows::{
    code, page_source_complete, EventQuery, EventSeverity, LedgerView, RuntimeEventQueryPage,
    MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use acui_source::{console_code, MaterialReader, ReadSource, Session, SourceMode};
use anyhow::{bail, Result};
use install::InstallPaths;
use ledger::paint::{self, port_options, shared};
use ledger::state::{FrameEventsSlot, FrameLookupError, LedgerState, Phase, PortMap, SourceFacts};
use ledger::LedgerHandle;
use report::Fatal;
use settings::{SettingsIssue, TextSize};
use slint::{Timer, TimerMode};
use strings::{fill, Labels, Language};
use ui::Ui;

slint::include_modules!();

#[derive(Default)]
struct Args {
    state_root: Option<PathBuf>,
    tab: Option<LedgerView>,
    /// `--lang` overrides the settings file for this run only.
    language: Option<Language>,
    /// `--source`: `auto` unless told otherwise.
    source: SourceMode,
    help: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--state-root" => match argv.next() {
                Some(path) => args.state_root = Some(PathBuf::from(path)),
                None => bail!("--state-root 缺少路径 / --state-root needs a path"),
            },
            "--tab" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--tab 缺少取值 / --tab needs a value"),
                };
                args.tab = match tab_from_name(&name) {
                    Some(view) => Some(view),
                    None => bail!("未知的 --tab 取值 / unknown --tab value: {name}"),
                };
            }
            "--lang" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--lang 缺少取值 / --lang needs a value"),
                };
                args.language = match Language::from_wire(&name) {
                    Some(language) => Some(language),
                    None => bail!("未知的 --lang 取值 / unknown --lang value: {name}"),
                };
            }
            "--source" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--source 缺少取值 / --source needs a value"),
                };
                args.source = match SourceMode::from_wire(&name) {
                    Some(mode) => mode,
                    None => bail!("未知的 --source 取值 / unknown --source value: {name}"),
                };
            }
            "--help" | "-h" => args.help = true,
            other => bail!("未知参数 / unknown argument: {other}"),
        }
    }
    Ok(args)
}

/// One state root (its read face open or unopened), what the ledger window
/// shows of it, what the windows share, and the window's two timers.
struct App {
    /// The installation inputs of this process, retained across all UI actions.
    installation: Option<acui_installation::Snapshot>,
    /// Read here, on the event loop; the window is never painted from it, only
    /// from what the reads leave in `ledger` (see `prepare`).
    source: Session,
    /// The ledger window's state, frame reads and module list.
    ledger: LedgerHandle,
    /// The language and text size shown, the settings file and its notices,
    /// and the launcher's view.
    ui: Rc<Ui>,
    /// The state root and the two launcher paths, as this run resolved them.
    launcher: launcher::Launcher,
    /// Runs the follow ticks while "follow latest" is on.
    follow: Timer,
    /// Reloads once the time cursor's handle rests.
    cursor_rest: Timer,
}

impl App {
    /// The table of the language shown now.
    fn labels(&self) -> &'static Labels {
        self.ui.labels()
    }

    /// Whether the read face is the running Runtime's.
    fn online(&self) -> bool {
        matches!(self.source, Session::Open(ReadSource::Online(_)))
    }
}

/// Every way out of a start goes through `report`: the usage, a command line
/// it cannot read, and whatever stops `run`, each with its exit code.
fn main() -> ExitCode {
    report::begin();
    let loaded = settings::load();
    let parsed = parse_args();
    // A failure is said in the language this run would have shown: `--lang`
    // when the command line got that far, else the file's.
    let language = parsed
        .as_ref()
        .ok()
        .and_then(|args| args.language)
        .unwrap_or(loaded.settings.language);
    let labels = language.labels();
    let factor = loaded.settings.text_size.factor();
    for issue in &loaded.issues {
        report::log("settings", &issue.text(labels));
    }
    let args = match parsed {
        Ok(args) => args,
        Err(error) => {
            let text = format!("{error}\n\n{}", labels.usage);
            return report::fatal(&Fatal::arguments(text), labels, factor);
        }
    };
    if args.help {
        return report::help(labels, factor);
    }
    match run(args, loaded, language) {
        Ok(()) => ExitCode::SUCCESS,
        Err(fatal) => report::fatal(&fatal, labels, factor),
    }
}

fn run(args: Args, loaded: settings::Loaded, language: Language) -> Result<(), Fatal> {
    let labels = language.labels();
    let stored = loaded.settings;
    let InstallPaths { installation, state_root, actingd_config, actingd_exe } =
        install::resolve(&args, &stored).map_err(|issue| Fatal::failed(issue.text(labels)))?;

    let source = Session::open(&state_root, args.source).map_err(|error| {
        Fatal::failed(fill(labels.ledger_open_failed, &[&format!("{error:#}")]))
    })?;
    let state = match &source {
        Session::Open(source) => {
            let port_map = match source.instance_bindings() {
                Ok(Some(bindings)) => PortMap::Read(bindings),
                Ok(None) => PortMap::Online,
                Err(error) => PortMap::Failed(error),
            };
            let mut model = ViewModel::new(
                source.open_report(),
                source.snapshot_position(),
                state_root.display().to_string(),
                None,
            );
            advance_span(source, &mut model, None);
            model.tab = args.tab.unwrap_or(LedgerView::Events);
            LedgerState::open(SourceFacts::of(source), model, port_map)
        }
        // Nothing is asked of a face that is not open.
        Session::Unopened { root, reason, failure } => {
            LedgerState::unopened(root.clone(), reason.clone(), failure.clone())
        }
    };
    let launcher_view = launcher::LauncherView::new(source.offline_reason().is_some());
    let ui = Ui::new(language, &stored, loaded.file, loaded.issues, launcher_view);
    let app = Rc::new(App {
        installation,
        source,
        ledger: LedgerHandle::new(state),
        ui,
        launcher: launcher::Launcher::new(state_root, actingd_config, actingd_exe),
        follow: Timer::default(),
        cursor_rest: Timer::default(),
    });
    reload(&app, false);

    let window_failed =
        |error: slint::PlatformError| Fatal::failed(fill(labels.window_failed, &[&error.to_string()]));
    let window = AppWindow::new().map_err(window_failed)?;
    fill_strings(window.global::<Strings>(), labels, app.online());
    let text_size = app.ui.text_size.get();
    window.global::<Scale>().set_factor(text_size.factor());
    window.set_text_size_index(text_size.index());
    window.set_language_index(language_index(language));
    install_callbacks(&window, &app);
    launcher::install(&window, &app);
    paint_notices(&window, &app);
    refresh(&window, &app);
    launcher::refresh_status(&window, &app);
    window.run().map_err(window_failed)?;
    Ok(())
}

/// The top of the window: the settings file's problems, errors first, and
/// where the technical log is. A save's failure stays until a save succeeds;
/// one that only repeats a problem found at the start is not said twice.
fn paint_notices(window: &AppWindow, app: &App) {
    let labels = app.labels();
    let notices = app.ui.notices.borrow();
    let fresh = notices.save.as_ref().filter(|issue| !notices.settings.contains(*issue));
    let (mut errors, mut warnings) = (Vec::new(), Vec::new());
    for issue in notices.settings.iter().chain(fresh) {
        let text = issue.text(labels);
        if issue.is_error() {
            errors.push(text);
        } else {
            warnings.push(text);
        }
    }
    window.set_settings_errors(errors.join("\n").into());
    window.set_settings_warnings(warnings.join("\n").into());
    let (note, failed) = report::log_note(labels);
    window.set_log_note(note.into());
    window.set_log_failed(failed);
}

/// A box's save: its failure goes to the log and stays on screen; a save that
/// worked clears the last failure.
fn settings_saved(window: &AppWindow, app: &App, saved: Result<(), SettingsIssue>) {
    let failure = saved.err();
    if let Some(issue) = &failure {
        report::log("settings", &issue.text(app.labels()));
    }
    app.ui.notices.borrow_mut().save = failure;
    paint_notices(window, app);
}

/// A language picked in the box: saved, then shown at once — the labels the
/// window draws, the lines it fills (the launcher's, the frame pane's and the
/// read errors' included) and the notices, the save's failure among them.
/// Setting the box's index from here does not call `selected` again.
fn set_language(window: &AppWindow, app: &Rc<App>, language: Language) {
    let saved = app.ui.settings.save_language(language);
    app.ui.language.set(language);
    fill_strings(window.global::<Strings>(), app.labels(), app.online());
    window.set_language_index(language_index(language));
    settings_saved(window, app, saved);
    refresh(window, app);
    launcher::paint(window, &app.ui);
}

fn language_index(language: Language) -> i32 {
    match language {
        Language::Zh => 0,
        Language::En => 1,
    }
}

/// Fills the `Strings` global from one language's table: at startup, and
/// again at every language switch. A box keeps its picked index when its
/// option list is replaced.
fn fill_strings(global: Strings<'_>, labels: &Labels, online: bool) {
    global.set_window_title(labels.window_title.into());
    global.set_data_source(
        if online { labels.data_source_online } else { labels.data_source }.into(),
    );
    global.set_ledger_dir(labels.ledger_dir.into());
    global.set_storage_format(labels.storage_format.into());
    global.set_show_up_to(labels.show_up_to.into());
    global.set_text_size(labels.text_size.into());
    global.set_language(labels.language.into());
    global.set_text_size_options(shared(&labels.text_sizes));
    global.set_language_options(shared(&labels.languages));
    global.set_severity_min_options(shared(&labels.severity_min));
    global.set_severity_max_options(shared(&labels.severity_max));
    global.set_id_placeholder(labels.id_placeholder.into());
    global.set_continue_reading(labels.continue_reading.into());
    global.set_show_performance(labels.show_performance.into());
    global.set_jump_latest(labels.jump_latest.into());
    global.set_follow_latest(labels.follow_latest.into());
    global.set_card_title(labels.card_title.into());
    global.set_card_note(labels.card_note.into());
    global.set_detail_title(labels.detail_title.into());
    global.set_artifacts_title(labels.artifacts_title.into());
    global.set_overlays_title(labels.overlays_title.into());
    global.set_raw_data_title(labels.raw_data_title.into());
    global.set_col_seq(labels.columns[0].into());
    global.set_col_time(labels.columns[1].into());
    global.set_col_level(labels.columns[2].into());
    global.set_col_module(labels.columns[3].into());
    global.set_col_event(labels.columns[4].into());
    global.set_col_link(labels.columns[5].into());
    global.set_col_port(labels.columns[6].into());
    global.set_launcher_title(labels.launcher_title.into());
    global.set_unlock_owner(labels.unlock_owner.into());
    global.set_unlock_statement(labels.unlock_statement.into());
    global.set_unlock_confirm(labels.unlock_confirm.into());
    global.set_start(labels.start.into());
    global.set_request_shutdown(labels.request_shutdown.into());
}

/// The time of the first event at or after `from` in the snapshot; `None`
/// when there is none. A page the read budget stopped may hold no row and
/// still go on (#411 A51), so the read follows the cursor until a row or its end.
fn event_time(source: &ReadSource, from: Option<u64>) -> Result<Option<u64>> {
    let query = EventQuery {
        view: Some(LedgerView::Events),
        from_sequence: from,
        ..EventQuery::default()
    };
    let mut cursor = None;
    loop {
        let page = source.query(&query, 1, cursor)?;
        if let Some(event) = page.events().first() {
            return Ok(Some(event.timestamp_unix_ms));
        }
        cursor = page.next_cursor().cloned();
        if cursor.is_none() {
            return Ok(None);
        }
    }
}

/// The committed time span, read as the first and the last event of the snapshot.
fn time_span(source: &ReadSource) -> Result<Option<(u64, u64)>> {
    let first = event_time(source, None)?;
    let last = event_time(source, Some(source.snapshot_position()))?;
    Ok(first.zip(last))
}

/// How long one fill may keep reading windows after the first.
const FILL_BUDGET: Duration = Duration::from_secs(1);
/// Windows of slack above a time bound's estimated position, and the most
/// probes that check it.
const TIME_SLACK_WINDOWS: u64 = 2;
const TIME_PROBES: usize = 3;
/// How long the time cursor's handle must rest before the view reloads.
const CURSOR_REST: Duration = Duration::from_millis(400);

/// Fills the timeline from the latest end at the pinned snapshot: backward
/// windows, each read whole, until `FILL_ROWS` more rows show, position 1 is
/// read, `FILL_WINDOWS` windows were read, or `FILL_BUDGET` has passed — at
/// least one window each time. Every page query is served on the Runtime's
/// ledger writer (online), so what the budget leaves is for "read earlier";
/// the top bar states what was read.
/// `earlier` continues below what is loaded; otherwise the view starts over
/// from the pin, or from where a time bound is estimated to fall.
fn reload(app: &App, earlier: bool) {
    let Session::Open(source) = &app.source else {
        return;
    };
    let mut state = app.ledger.state.borrow_mut();
    let state = &mut *state;
    let Some(model) = state.model.as_mut() else {
        return;
    };
    if !earlier {
        // This reload applies the bound the time cursor holds now.
        app.cursor_rest.stop();
        // And it retries a frame read that failed.
        if state.frame_events.failed() {
            state.frame_events = FrameEventsSlot::Empty;
        }
        let snapshot = source.snapshot_position();
        let upper = match model.filters.to_timestamp_unix_ms {
            None => Ok(snapshot),
            Some(bound) => {
                let estimate = upper_for_time(model.span(), snapshot, bound);
                checked_upper(source, model, estimate, snapshot)
            }
        };
        app.ledger.frame.forget();
        match upper {
            Ok(upper) => model.begin(upper),
            Err(error) => {
                model.begin(0);
                model.query_error = Some(error);
                return;
            }
        }
    }
    let (target, started) = (model.row_count() + FILL_ROWS, Instant::now());
    model.query_error = None;
    for read in 0..FILL_WINDOWS {
        if read > 0 && (model.row_count() >= target || started.elapsed() >= FILL_BUDGET) {
            break;
        }
        let Some(window) = model.next_window() else {
            break;
        };
        let pages = model
            .window_query(window)
            .and_then(|query| read_window(source, &query).map_err(|error| read_failed(&error)));
        match pages {
            Ok(pages) => model.apply_window(window, &pages),
            Err(error) => {
                model.query_error = Some(error);
                break;
            }
        }
    }
}

/// One window read whole: its pages, following the cursor when the reply's
/// byte limit splits it.
fn read_window(source: &ReadSource, query: &EventQuery) -> Result<Vec<RuntimeEventQueryPage>> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = source.query(query, MAX_RUNTIME_EVENT_QUERY_EVENTS, cursor)?;
        cursor = page.next_cursor().cloned();
        pages.push(page);
        if cursor.is_none() {
            return Ok(pages);
        }
    }
}

/// Where a time bound is estimated to fall, from the committed span without any
/// read: the position it would take if events were even in time, plus
/// `TIME_SLACK_WINDOWS` windows, capped at the pin; 0 for a bound before the
/// first event. `checked_upper` makes the estimate safe.
fn upper_for_time(span: Option<(u64, u64)>, snapshot: u64, bound: u64) -> u64 {
    let Some((first, last)) = span else {
        return snapshot;
    };
    if bound >= last {
        return snapshot;
    }
    if bound < first {
        return 0;
    }
    let ratio = (bound - first) as f64 / (last - first) as f64;
    let estimate = (ratio * snapshot as f64) as u64;
    (estimate + TIME_SLACK_WINDOWS * WINDOW).min(snapshot)
}

/// Makes an estimated start safe, since reading only goes down from it: one
/// probe with the view's own query — its time bound included — asks for the
/// first matching event above the start. None: nothing above could show, and
/// reading starts there. One found: the start moves above it by a step that
/// doubles each time, for at most `TIME_PROBES` probes and within the fill
/// budget; past either, the pin, which is always safe. A page the read budget
/// stopped may hold no row and still go on (#411 A51): it decides nothing
/// above the start, so that is the pin too.
fn checked_upper(
    source: &ReadSource,
    model: &ViewModel,
    estimate: u64,
    snapshot: u64,
) -> Result<u64, QueryError> {
    let (mut upper, mut step, started) = (estimate, TIME_SLACK_WINDOWS * WINDOW, Instant::now());
    for probe in 0..TIME_PROBES {
        if upper >= snapshot || (probe > 0 && started.elapsed() >= FILL_BUDGET) {
            return Ok(snapshot);
        }
        let mut query = model.query()?;
        query.from_sequence = Some(upper + 1);
        query
            .validate()
            .map_err(|error| QueryError::Rejected(error.code().to_string()))?;
        let page = source.query(&query, 1, None).map_err(|error| read_failed(&error))?;
        let Some(found) = page.events().first() else {
            return Ok(if page.next_cursor().is_some() { snapshot } else { upper });
        };
        upper = found.sequence + step;
        step *= 2;
    }
    Ok(snapshot)
}

fn install_callbacks(window: &AppWindow, app: &Rc<App>) {
    macro_rules! on {
        ($setter:ident, |$app:ident, $value:ident| $body:expr) => {{
            let weak = window.as_weak();
            let $app = Rc::clone(app);
            window.$setter(move |$value| {
                // Unopened there is no model, and every control that calls these is off.
                if $app.ledger.state.borrow().model.is_none() {
                    return;
                }
                $body;
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &$app);
                }
            });
        }};
    }

    on!(on_tab_changed, |app, index| {
        edit(&app, |model| {
            model.tab = ALL_TABS[index.clamp(0, ALL_TABS.len() as i32 - 1) as usize]
        });
        reload(&app, false)
    });
    on!(on_minimum_severity_changed, |app, index| {
        edit(&app, |model| {
            model.filters.minimum_severity = match index {
                1 => Some(EventSeverity::Warning),
                2 => Some(EventSeverity::Error),
                3 => Some(EventSeverity::Fatal),
                _ => None,
            }
        });
        reload(&app, false)
    });
    on!(on_maximum_severity_changed, |app, index| {
        edit(&app, |model| {
            model.filters.maximum_severity = match index {
                1 => Some(EventSeverity::Info),
                2 => Some(EventSeverity::Warning),
                3 => Some(EventSeverity::Error),
                _ => None,
            }
        });
        reload(&app, false)
    });
    // By name against the list as it stands now, never by a position taken from
    // a list that the next page may have rebuilt.
    on!(on_module_changed, |app, name| {
        let module = app
            .ledger
            .module_options
            .borrow()
            .iter()
            .position(|option| *option == name)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| app.ledger.module_choices.borrow().get(index).copied());
        edit(&app, |model| model.filters.origin_module = module);
        reload(&app, false)
    });
    // A port is one instance: its whole set of ids goes into the query, or
    // nothing does. The first item, and any name not on the list, clears both.
    on!(on_port_changed, |app, name| {
        let entry = {
            let state = app.ledger.state.borrow();
            // The list the box shows: made in the language it was painted in,
            // which is the one shown now.
            let (options, choices) = port_options(app.labels(), &state.port_map);
            let entry = options
                .iter()
                .position(|option| *option == name)
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| choices.get(index))
                .and_then(|port| state.port_entry(*port))
                .map(|entry| (entry.port, entry.members.clone()));
            entry
        };
        edit(&app, |model| {
            model.filters.port = entry.as_ref().map(|(port, _)| *port);
            model.filters.instance_ids = entry.map(|(_, members)| members).unwrap_or_default();
        });
        reload(&app, false)
    });
    // Shown or hidden again from the pin: the performance monitor's routine
    // events are left out by the query or dropped as each window is read, never
    // from rows already held.
    on!(on_show_performance_changed, |app, shown| {
        edit(&app, |model| model.filters.show_performance = shown);
        reload(&app, false)
    });
    on!(on_id_changed, |app, text| {
        edit(&app, |model| model.filters.id_text = text.to_string());
        reload(&app, false)
    });
    // The bound and its label follow the handle at once; the reload waits
    // until the handle rests, so a drag is one reload, not one per step.
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_cursor_changed(move |value| {
            let bound = {
                let state = app.ledger.state.borrow();
                let Some(model) = &state.model else {
                    return;
                };
                let bound = model.span().and_then(|(from, to)| {
                    let ratio = (value as f64 / 1000.0).clamp(0.0, 1.0);
                    (ratio < 1.0).then(|| from + ((to - from) as f64 * ratio) as u64)
                });
                bound
            };
            edit(&app, |model| model.filters.to_timestamp_unix_ms = bound);
            // A time bound reads down from the past; following stops at once.
            if let (Some(_), Some(window)) = (bound, weak.upgrade()) {
                stop_following(&window, &app);
            }
            let (rested, held) = (weak.clone(), Rc::downgrade(&app));
            app.cursor_rest.start(TimerMode::SingleShot, CURSOR_REST, move || {
                if let (Some(window), Some(app)) = (rested.upgrade(), held.upgrade()) {
                    reload(&app, false);
                    refresh(&window, &app);
                }
            });
            if let Some(window) = weak.upgrade() {
                refresh(&window, &app);
            }
        });
    }
    on!(on_row_clicked, |app, index| {
        edit(&app, |model| {
            let selected = match model.display_rows().get(index.max(0) as usize) {
                Some(DisplayRow::Event { event, .. }) => Some(event.sequence),
                _ => None,
            };
            if selected.is_some() {
                model.selected_sequence = selected;
            }
        })
    });
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_jump_latest(move || {
            if let Some(window) = weak.upgrade() {
                jump_latest(&window, &app);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_follow_changed(move |on| {
            if let Some(window) = weak.upgrade() {
                set_following(&window, &app, on);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_load_more(move || {
            // A bound the time cursor set but has not applied yet goes first:
            // no window of it is appended below rows read under the old one.
            let pending = app.cursor_rest.running();
            reload(&app, !pending);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &app);
            }
        });
    }
    {
        // Text size applies now, through the one scale property. Only the text
        // size is written: the language is the file's own, or a `--lang`
        // override that this run must not save over it. A save that fails
        // still leaves the size applied, and says so at the top.
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_text_size_changed(move |index| {
            let size = TextSize::from_index(index);
            let saved = app.ui.settings.save_text_size(size);
            app.ui.text_size.set(size);
            if let Some(window) = weak.upgrade() {
                window.global::<Scale>().set_factor(size.factor());
                settings_saved(&window, &app, saved);
            }
        });
    }
    // Language is written now and shown at once.
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_language_changed(move |index| {
            let language = if index == 1 { Language::En } else { Language::Zh };
            if let Some(window) = weak.upgrade() {
                set_language(&window, &app, language);
            }
        });
    }
}

/// How often "follow latest" asks where the ledger is: one fact snapshot,
/// which reads no ledger and writes nothing. A page query, served on the
/// Runtime's ledger writer, follows only when the ledger moved.
const FOLLOW_INTERVAL: Duration = Duration::from_secs(5);

/// A person's jump to the latest: the pin moves to the Runtime's latest
/// position, status and facts are read again — neither read appends a ledger
/// record — and the view starts over from the new pin, with no time bound.
fn jump_latest(window: &AppWindow, app: &App) {
    let Session::Open(source) = &app.source else {
        return;
    };
    if app.ledger.state.borrow().model.is_none() {
        return;
    }
    match source.repin() {
        Err(error) => {
            edit(app, |model| model.query_error = Some(read_failed(&error)));
        }
        Ok(_) => {
            source.refresh_instances();
            start_at_pin(window, app, source);
        }
    }
    refresh(window, app);
}

/// The view starts over from a pin just moved: no time bound, the time cursor
/// back at the top, the span's end read at the new pin.
fn start_at_pin(window: &AppWindow, app: &App, source: &ReadSource) {
    edit(app, |model| {
        model.set_pin(source.open_report(), source.snapshot_position());
        model.filters.to_timestamp_unix_ms = None;
        advance_span(source, model, None);
    });
    window.set_cursor_value(1000.0);
    reload(app, false);
}

/// Moves the span's end to the event at the pin: `time` when a read already
/// holds that event, one read of it otherwise. A span never read is read
/// whole. A failed read leaves the span as it was and stays in `span_error`
/// until one succeeds; the paint says it is the span's.
fn advance_span(source: &ReadSource, model: &mut ViewModel, time: Option<u64>) {
    let span = match (model.span(), time) {
        (Some((first, _)), Some(last)) => Ok(Some((first, last))),
        (Some((first, last)), None) => event_time(source, Some(source.snapshot_position()))
            .map(|time| Some((first, time.unwrap_or(last)))),
        (None, _) => time_span(source),
    };
    match span {
        Ok(span) => {
            model.set_span(span);
            model.span_error = None;
        }
        Err(error) => model.span_error = Some(read_failed(&error)),
    }
}

/// Turning following on catches up like a jump to the latest, without the
/// status read: the pin moves to the latest position, any time bound is
/// dropped and the view starts over from the new pin, however far the ledger
/// has moved since. Then a tick runs every `FOLLOW_INTERVAL`. Turning it off
/// stops the ticks and leaves the view as it is.
fn set_following(window: &AppWindow, app: &Rc<App>, on: bool) {
    let Session::Open(source) = &app.source else {
        return;
    };
    if app.ledger.state.borrow().model.is_none() {
        return;
    }
    if !on {
        stop_following(window, app);
        refresh(window, app);
        return;
    }
    // What an earlier following failed at goes once it is turned on again.
    edit(app, |model| model.follow_error = None);
    if let Err(error) = source.repin() {
        edit(app, |model| model.query_error = Some(read_failed(&error)));
        window.set_following(false);
        refresh(window, app);
        return;
    }
    start_at_pin(window, app, source);
    refresh(window, app);
    let (weak, held) = (window.as_weak(), Rc::downgrade(app));
    app.follow.start(TimerMode::Repeated, FOLLOW_INTERVAL, move || {
        if let (Some(window), Some(app)) = (weak.upgrade(), held.upgrade()) {
            follow_tick(&window, &app);
        }
    });
}

/// Stops following: no more ticks, the box unticked, and what a tick failed
/// at goes with it; the view keeps what it read.
fn stop_following(window: &AppWindow, app: &App) {
    app.follow.stop();
    window.set_following(false);
    edit(app, |model| model.follow_error = None);
}

/// One follow tick. One fact snapshot says where the ledger is and brings the
/// task facts; only when the pin moved, or an earlier tick left windows unread,
/// are the newer windows read onto the top of the view, and the span's end
/// follows the pin. No status is read and nothing is written to the ledger. A
/// failed poll stays in `follow_error` until a tick gets past it; a failed page
/// read stays there too and stops the following; a time bound set meanwhile
/// stops it as well.
fn follow_tick(window: &AppWindow, app: &App) {
    let Session::Open(source) = &app.source else {
        return;
    };
    let bounded = match &app.ledger.state.borrow().model {
        Some(model) => model.filters.to_timestamp_unix_ms.is_some(),
        None => return,
    };
    if bounded {
        stop_following(window, app);
        refresh(window, app);
        return;
    }
    let moved = match source.poll() {
        Ok(moved) => moved,
        Err(error) => {
            edit(app, |model| model.follow_error = Some(read_failed(&error)));
            refresh(window, app);
            return;
        }
    };
    {
        let mut state = app.ledger.state.borrow_mut();
        let Some(model) = state.model.as_mut() else {
            return;
        };
        let cleared = model.follow_error.take().is_some();
        if moved {
            model.set_pin(source.open_report(), source.snapshot_position());
        }
        if !moved && model.next_newer_window().is_none() {
            drop(state);
            if cleared {
                refresh(window, app);
            }
            return;
        }
        let (pin, started) = (source.snapshot_position(), Instant::now());
        let mut pin_time = None;
        // Filters that do not form a query already show why; following reads
        // nothing under them and does not state it a second time.
        let reads = if model.query().is_ok() { FILL_WINDOWS } else { 0 };
        for read in 0..reads {
            if read > 0 && started.elapsed() >= FILL_BUDGET {
                break;
            }
            let Some(window) = model.next_newer_window() else {
                break;
            };
            let pages = model
                .window_query(window)
                .and_then(|query| read_window(source, &query).map_err(|error| read_failed(&error)));
            match pages {
                Ok(pages) => {
                    pin_time = pin_time.or_else(|| {
                        let mut events = pages.iter().flat_map(|page| page.events());
                        let at_pin = events.find(|event| event.sequence == pin);
                        at_pin.map(|event| event.timestamp_unix_ms)
                    });
                    model.apply_newer_window(window, &pages);
                }
                Err(error) => {
                    model.follow_error = Some(error);
                    break;
                }
            }
        }
        if model.follow_error.is_some() {
            // A read that failed stops the following, its reason kept on
            // screen: a Runtime refusing each query is not asked again every
            // tick. Turning following on again retries.
            app.follow.stop();
            window.set_following(false);
        } else if moved {
            advance_span(source, model, pin_time);
        }
    }
    refresh(window, app);
}

/// Paints the ledger window: first `prepare`, then the paint, which reads
/// nothing (`ledger::paint`). The paint asks for the material reader only when
/// it starts a new frame read: once a frame read latched the material
/// connection, taking a reader connects anew, and no other paint waits on that.
fn refresh(window: &AppWindow, app: &App) {
    prepare(app);
    let labels = app.labels();
    let reader = || match &app.source {
        Session::Open(source) => source.material_reader(),
        // Never asked: an unopened face is painted with no frame pane to read for.
        Session::Unopened { .. } => MaterialReader::Unavailable(labels.ledger_unopened.to_owned()),
    };
    paint::refresh(window, &app.ledger, labels, &reader);
}

/// What the paint would otherwise have to read, read here, before it: the read
/// face's facts copied into the window's state — the instances as last read —
/// and, when the loaded rows hold no capture of the selected event's frame,
/// that frame's own events.
fn prepare(app: &App) {
    let Session::Open(source) = &app.source else {
        return;
    };
    let mut state = app.ledger.state.borrow_mut();
    let state = &mut *state;
    state.phase = Phase::Open(SourceFacts::of(source));
    if let Some(model) = &state.model {
        look_up_frame_events(source, model, &mut state.frame_events);
    }
}

/// The selected event's frame, when the loaded rows hold no capture of it:
/// its own events, read once — one query by frame id, kept in `slot` while the
/// pane stays on that frame — so an event still lands on its frame when the
/// capture is filtered out or not loaded.
fn look_up_frame_events(source: &ReadSource, model: &ViewModel, slot: &mut FrameEventsSlot) {
    let Some((basis, _)) = model.selected_basis() else {
        return;
    };
    let Some(frame_id) = basis.links.frame_id().copied() else {
        return;
    };
    let frame = code(&frame_id);
    if slot.of(&frame).is_some() || model.frame_group(&frame, &[]).target.is_some() {
        return;
    }
    let query = EventQuery {
        view: Some(LedgerView::Events),
        frame_id: Some(frame_id),
        ..EventQuery::default()
    };
    // A read the page says is incomplete cannot say the capture is absent.
    let events = read_window(source, &query)
        .map_err(|error| FrameLookupError::Read(error.to_string()))
        .and_then(|pages| match pages.iter().all(|page| page_source_complete(page)) {
            true => Ok(pages.iter().flat_map(|page| page.events().iter().cloned()).collect()),
            false => Err(FrameLookupError::Incomplete),
        });
    *slot = FrameEventsSlot::Read { frame, events };
}

/// Changes the window's view model, when there is one.
fn edit(app: &App, change: impl FnOnce(&mut ViewModel)) {
    if let Some(model) = app.ledger.state.borrow_mut().model.as_mut() {
        change(model);
    }
}

/// A read's error as the view keeps it: the read face's text, and its console
/// code when it carries one; both are put into words when painted.
fn read_failed(error: &anyhow::Error) -> QueryError {
    QueryError::ReadFailed { text: error.to_string(), code: console_code(error) }
}
