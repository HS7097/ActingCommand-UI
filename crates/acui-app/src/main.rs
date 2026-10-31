// SPDX-License-Identifier: GPL-3.0-only
//! ActingCommand 监控台 / ActingCommand Console: a read-only window over one
//! Runtime state root, opened through the formal ledger read face — offline
//! over the files, or online through the typed client of the running Runtime —
//! with a launcher block that starts the daemon detached and asks it, through
//! the same typed client, to shut down. A start that fails is shown in a window
//! of its own (`report`); what the settings file holds that the console cannot
//! take is shown at the top of the window. Every word is put together when it
//! is painted, in the language shown then (`ui`, `phrase`), so a language
//! picked in the box takes effect at once. The window comes up first; the
//! ledger window's own reader thread then opens the read face and does every
//! read of it, and the window is painted from what the reads leave in its state
//! (`ledger`), never from the read face itself.

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

use acui_model::{tab_from_name, DisplayRow, ALL_TABS};
use acui_rows::{EventSeverity, LedgerView};
use acui_source::SourceMode;
use anyhow::{bail, Result};
use install::InstallPaths;
use ledger::paint::{port_options, shared};
use ledger::{LedgerHandle, OpenArgs};
use report::Fatal;
use settings::{SettingsIssue, TextSize};
use strings::{fill, Labels, Language};
use ui::Ui;

slint::include_modules!();

/// The one ledger window of this process.
const LEDGER_WINDOW: u64 = 1;

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

/// One state root's ledger window, what the windows share, and the launcher.
struct App {
    /// The installation inputs of this process, retained across all UI actions.
    installation: Option<acui_installation::Snapshot>,
    /// The ledger window's side: its state, its reader, its frame reads and
    /// its timers.
    ledger: Rc<LedgerHandle>,
    /// The language and text size shown, the settings file and its notices,
    /// and the launcher's view.
    ui: Rc<Ui>,
    /// The state root and the two launcher paths, as this run resolved them.
    launcher: launcher::Launcher,
}

impl App {
    /// The table of the language shown now.
    fn labels(&self) -> &'static Labels {
        self.ui.labels()
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

/// The window comes up before anything is read: the ledger window's reader
/// starts once it is shown, and a read face that does not open is said in the
/// window, which stays.
fn run(args: Args, loaded: settings::Loaded, language: Language) -> Result<(), Fatal> {
    let labels = language.labels();
    let stored = loaded.settings;
    let InstallPaths { installation, state_root, actingd_config, actingd_exe } =
        install::resolve(&args, &stored).map_err(|issue| Fatal::failed(issue.text(labels)))?;

    // What the launcher's line says of an offline window follows the window's
    // read face once it opens.
    let launcher_view = launcher::LauncherView::new(false);
    let ui = Ui::new(language, &stored, loaded.file, loaded.issues, launcher_view);
    let window_failed =
        |error: slint::PlatformError| Fatal::failed(fill(labels.window_failed, &[&error.to_string()]));
    let window = AppWindow::new().map_err(window_failed)?;
    let open = OpenArgs { state_root: state_root.clone(), mode: args.source, tab: args.tab };
    let app = Rc::new(App {
        installation,
        ledger: LedgerHandle::new(LEDGER_WINDOW, &window, open),
        ui,
        launcher: launcher::Launcher::new(state_root, actingd_config, actingd_exe),
    });
    fill_strings(window.global::<Strings>(), labels);
    let text_size = app.ui.text_size.get();
    window.global::<Scale>().set_factor(text_size.factor());
    window.set_text_size_index(text_size.index());
    window.set_language_index(language_index(language));
    install_callbacks(&window, &app);
    launcher::install(&window, &app);
    paint_notices(&window, &app);
    app.ledger.refresh(&window, &app.ui);
    window.show().map_err(window_failed)?;
    app.ledger.start(None);
    launcher::refresh_status(&window, &app);
    slint::run_event_loop().map_err(window_failed)?;
    window.hide().map_err(window_failed)?;
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
    report::paint_log_note(window, labels);
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
/// window draws, the lines it fills (the launcher's, the frame pane's, the busy
/// line and the read errors' included) and the notices, the save's failure
/// among them. The repaint holds the ledger state's lock for the paint only,
/// so it does not wait on a read in flight. Setting the box's index from here
/// does not call `selected` again.
fn set_language(window: &AppWindow, app: &Rc<App>, language: Language) {
    let saved = app.ui.settings.save_language(language);
    app.ui.language.set(language);
    fill_strings(window.global::<Strings>(), app.labels());
    window.set_language_index(language_index(language));
    settings_saved(window, app, saved);
    app.ledger.refresh(window, &app.ui);
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
/// option list is replaced. The data source line follows the read face, so
/// the ledger window's paint sets it.
fn fill_strings(global: Strings<'_>, labels: &Labels) {
    global.set_window_title(labels.window_title.into());
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
    global.set_reopen_ledger(labels.reopen_ledger.into());
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

/// The ledger window's controls only change its state and hand its reader a
/// request; each paints the window again at once, and the reader paints it
/// again as its reads land. While the read face is not open every control
/// that would query it is off, and a change finds no view model to change.
fn install_callbacks(window: &AppWindow, app: &Rc<App>) {
    macro_rules! on {
        ($setter:ident, |$app:ident, $value:ident| $body:expr) => {{
            let weak = window.as_weak();
            let $app = Rc::clone(app);
            window.$setter(move |$value| {
                $body;
                if let Some(window) = weak.upgrade() {
                    $app.ledger.refresh(&window, &$app.ui);
                }
            });
        }};
    }

    on!(on_tab_changed, |app, index| {
        app.ledger.restart(|model, _| {
            model.tab = ALL_TABS[index.clamp(0, ALL_TABS.len() as i32 - 1) as usize]
        })
    });
    on!(on_minimum_severity_changed, |app, index| {
        app.ledger.restart(|model, _| {
            model.filters.minimum_severity = match index {
                1 => Some(EventSeverity::Warning),
                2 => Some(EventSeverity::Error),
                3 => Some(EventSeverity::Fatal),
                _ => None,
            }
        })
    });
    on!(on_maximum_severity_changed, |app, index| {
        app.ledger.restart(|model, _| {
            model.filters.maximum_severity = match index {
                1 => Some(EventSeverity::Info),
                2 => Some(EventSeverity::Warning),
                3 => Some(EventSeverity::Error),
                _ => None,
            }
        })
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
        app.ledger.restart(|model, _| model.filters.origin_module = module)
    });
    // A port is one instance: its whole set of ids goes into the query, or
    // nothing does. The first item, and any name not on the list, clears both.
    on!(on_port_changed, |app, name| {
        let labels = app.labels();
        app.ledger.restart(|model, port_map| {
            // The list the box shows: made in the language it was painted in,
            // which is the one shown now.
            let (options, choices) = port_options(labels, port_map);
            let entry = options
                .iter()
                .position(|option| *option == name)
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| choices.get(index))
                .and_then(|port| port_map.entry(*port));
            model.filters.port = entry.map(|entry| entry.port);
            model.filters.instance_ids =
                entry.map(|entry| entry.members.clone()).unwrap_or_default();
        })
    });
    // Shown or hidden again from the pin: the performance monitor's routine
    // events are left out by the query or dropped as each window is read, never
    // from rows already held.
    on!(on_show_performance_changed, |app, shown| {
        app.ledger.restart(|model, _| model.filters.show_performance = shown)
    });
    on!(on_id_changed, |app, text| {
        app.ledger.restart(|model, _| model.filters.id_text = text.to_string())
    });
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_cursor_changed(move |value| {
            if let Some(window) = weak.upgrade() {
                app.ledger.cursor_moved(&window, &app.ui, value);
            }
        });
    }
    on!(on_row_clicked, |app, index| {
        app.ledger.edit(|model| {
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
                app.ledger.jump_latest(&window, &app.ui);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_follow_changed(move |on| {
            if let Some(window) = weak.upgrade() {
                app.ledger.set_following(&window, &app.ui, on);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_load_more(move || {
            app.ledger.load_more();
            if let Some(window) = weak.upgrade() {
                app.ledger.refresh(&window, &app.ui);
            }
        });
    }
    // A new session of the window's read face: offered when it did not open,
    // when its reader stopped, or when a read reached another Runtime.
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_reopen_ledger(move || {
            if let Some(window) = weak.upgrade() {
                app.ledger.reopen(&window, &app.ui);
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
