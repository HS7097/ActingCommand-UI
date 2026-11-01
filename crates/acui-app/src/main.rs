// SPDX-License-Identifier: GPL-3.0-only
//! ActingCommand 监控台 / ActingCommand Console. Its main window is the
//! workbench (`ui`): the Runtime in plain words with the launcher's Start and
//! Request Shutdown (`launcher`), the language, the text size and the theme
//! (`skin`), and the way into the advanced troubleshooting and ledger window
//! (`ledger`) — a read-only window over this run's state root, opened through
//! the formal ledger read face, offline over the files or online through the
//! typed client of the running Runtime, with the launcher's technical lines and
//! its unlock entry. That window opens only when asked: from the workbench, or
//! at the start with `--tab` or `--source`; at most one per source.
//!
//! While it is up two threads read the Runtime for the workbench (`workbench`).
//!
//! A start that fails is shown in a window of its own (`report`); a state root
//! this run cannot find is said in red on the workbench, which stays. Every
//! word is put together when it is painted, in the language shown then (`ui`,
//! `phrase`), so a language, a text size or a theme picked in any window takes
//! effect at once in every window. A ledger window comes up first; its own reader
//! thread then opens the read face and does every read of it, and the window
//! is painted from what the reads leave in its state, never from the read face
//! itself. Started directly, the program opens no console window of its own;
//! what it has to print goes to the console it was started from, if any.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod install;
mod launcher;
mod ledger;
mod phrase;
mod report;
mod settings;
mod skin;
mod strings;
mod ui;
mod workbench;

use std::path::PathBuf;
use std::process::ExitCode;

use acui_model::tab_from_name;
use acui_rows::LedgerView;
use acui_source::SourceMode;
use anyhow::{bail, Result};
use launcher::Launcher;
use report::Fatal;
use strings::{fill, Language};
use ui::Ui;

slint::include_modules!();

#[derive(Default)]
struct Args {
    state_root: Option<PathBuf>,
    tab: Option<LedgerView>,
    /// `--lang` overrides the settings file for this run only.
    language: Option<Language>,
    /// `--source`, when given; a ledger window of no source asked for reads
    /// the default one, `auto`.
    source: Option<SourceMode>,
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
                    Some(mode) => Some(mode),
                    None => bail!("未知的 --source 取值 / unknown --source value: {name}"),
                };
            }
            "--help" | "-h" => args.help = true,
            other => bail!("未知参数 / unknown argument: {other}"),
        }
    }
    Ok(args)
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

/// The workbench comes up whatever this run's paths are: a state root it
/// cannot find is said on it in red, and only Start, Request Shutdown and the
/// ledger window are off. Then the reading threads, whose first `Status` read
/// is the first word on the Runtime. A ledger window opens at the
/// start only for `--tab` or `--source`: the fixed entry passes `--state-root`
/// on every start, and that alone opens none. Once the event loop is over the
/// reading threads are told to end, the windows are hidden and let go, and then
/// the shared state.
fn run(args: Args, loaded: settings::Loaded, language: Language) -> Result<(), Fatal> {
    let labels = language.labels();
    let stored = loaded.settings;
    let launcher = install::resolve(&args, &stored).map(Launcher::new);
    if let Err(issue) = &launcher {
        report::log("startup", &issue.text(labels));
    }
    let ui = Ui::new(language, &stored, loaded.file, loaded.issues, launcher);
    let window_failed =
        |error: slint::PlatformError| Fatal::failed(fill(labels.window_failed, &[&error.to_string()]));
    let workbench = WorkbenchWindow::new().map_err(window_failed)?;
    ui.show_workbench(workbench).map_err(window_failed)?;
    workbench::start(&ui);
    if args.tab.is_some() || args.source.is_some() {
        ui.open_ledger(args.source.unwrap_or_default(), args.tab);
    }
    let ran = slint::run_event_loop().map_err(window_failed);
    ui.close_all();
    drop(ui);
    ran
}
