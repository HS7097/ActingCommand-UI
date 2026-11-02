// SPDX-License-Identifier: GPL-3.0-only
//! What the windows of this process share, on the event loop only: the
//! language shown, the text size, the theme, what the settings file holds that
//! this run could not take, the launcher and its view, and the windows
//! themselves — the workbench, which is the main window, and the ledger windows
//! opened from it, at most one per source. A window takes its words from here
//! at the moment it paints, and a language, text size or theme picked in any
//! window is applied to every window at once.
//!
//! `main` holds the one strong reference for the length of the event loop and
//! lets it go once the loop is over, after every window is hidden; the
//! thread-local slot holds a weak one. That is how a worker's result, handed to
//! the event loop, and every window's callback find the shared state without
//! holding it: nothing a window holds keeps it alive.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use acui_rows::LedgerView;
use acui_source::SourceMode;
use slint::{CloseRequestResponse, ComponentHandle, PlatformError, Timer};

use crate::install::InstallIssue;
use crate::launcher::{self, Launcher, LauncherView};
use crate::ledger::paint::shared;
use crate::ledger::{self, LedgerHandle, OpenArgs};
use crate::report;
use crate::settings::{Settings, SettingsFile, SettingsIssue, TextSize, Theme};
use crate::skin::{self, Closing, Skins, WindowKey};
use crate::strings::{fill, Labels, Language};
use crate::{LedgerWindow, Look, Scale, Strings, WorkbenchWindow};

/// The shared look on one window, of either kind: the words of the language
/// shown, the text size's factor, the theme's skin, and the three boxes on what
/// is picked. A new window gets it before it shows.
macro_rules! apply_appearance {
    ($window:expr, $ui:expr, $key:expr) => {{
        let (window, ui) = ($window, $ui);
        fill_strings(window.global::<Strings>(), ui.labels());
        let size = ui.text_size.get();
        window.global::<Scale>().set_factor(size.factor());
        window.set_text_size_index(size.index());
        window.set_language_index(language_index(ui.language.get()));
        window.set_theme_index(ui.theme.get().index());
        skin::apply_before_show(window.global::<Look>(), ui, $key);
    }};
}

pub struct Ui {
    pub language: Cell<Language>,
    pub text_size: Cell<TextSize>,
    pub theme: Cell<Theme>,
    /// Each window's look, by the theme.
    pub skins: Skins,
    /// The settings file the boxes save to.
    pub settings: SettingsFile,
    pub notices: RefCell<Notices>,
    /// The launcher of this run's state root, or why this run has none: then
    /// the workbench says so in red, and Start and the ledger window are off.
    pub launcher: Result<Launcher, InstallIssue>,
    pub launcher_view: RefCell<LauncherView>,
    /// The main window, from just before it shows until the event loop is over.
    workbench: RefCell<Option<WorkbenchWindow>>,
    /// The ledger windows open now, by id.
    ledgers: RefCell<BTreeMap<u64, Rc<LedgerHandle>>>,
    /// The id the next ledger window gets.
    next_id: Cell<u64>,
    /// The workbench was asked to close while an unlock ran: it stayed, and
    /// says why until the unlock is over.
    pub close_refused: Cell<bool>,
}

/// The settings file's problems: what it held at the start that this run could
/// not take, and the last save's failure, shown until a save succeeds. Then
/// why the last ledger window asked for could not be made or shown, until one
/// is.
pub struct Notices {
    pub settings: Vec<SettingsIssue>,
    pub save: Option<SettingsIssue>,
    pub window: Option<String>,
}

thread_local! {
    static CURRENT: RefCell<Weak<Ui>> = RefCell::new(Weak::new());
}

impl Ui {
    /// The shared state, with the thread-local slot pointed at it.
    pub fn new(
        language: Language,
        stored: &Settings,
        settings: SettingsFile,
        issues: Vec<SettingsIssue>,
        launcher: Result<Launcher, InstallIssue>,
    ) -> Rc<Self> {
        let launcher_view = LauncherView::new(launcher.is_ok());
        let ui = Rc::new(Self {
            language: Cell::new(language),
            text_size: Cell::new(stored.text_size),
            theme: Cell::new(stored.theme),
            skins: Skins::detect(),
            settings,
            notices: RefCell::new(Notices { settings: issues, save: None, window: None }),
            launcher,
            launcher_view: RefCell::new(launcher_view),
            workbench: RefCell::new(None),
            ledgers: RefCell::new(BTreeMap::new()),
            next_id: Cell::new(1),
            close_refused: Cell::new(false),
        });
        CURRENT.with(|current| *current.borrow_mut() = Rc::downgrade(&ui));
        ui
    }

    /// The table of the language shown now; taken again at every paint.
    pub fn labels(&self) -> &'static Labels {
        self.language.get().labels()
    }

    /// Whether unlock-owner runs now: the workbench does not close meanwhile.
    pub fn unlocking(&self) -> bool {
        self.launcher.as_ref().is_ok_and(|launcher| launcher.unlocking())
    }

    pub fn workbench(&self) -> Option<WorkbenchWindow> {
        self.workbench.borrow().as_ref().map(|window| window.clone_strong())
    }

    /// The ledger window `id`, while it is open.
    pub fn ledger(&self, id: u64) -> Option<Rc<LedgerHandle>> {
        self.ledgers.borrow().get(&id).cloned()
    }

    /// The ledger windows open now, taken out of the registry: nothing holds
    /// the registry while they paint.
    pub fn ledgers(&self) -> Vec<Rc<LedgerHandle>> {
        self.ledgers.borrow().values().cloned().collect()
    }

    /// The main window: its callbacks, the shared look and every line it
    /// shows, then shown.
    pub fn show_workbench(&self, window: WorkbenchWindow) -> Result<(), PlatformError> {
        install_workbench(&window);
        apply_appearance!(&window, self, WindowKey::Workbench);
        *self.workbench.borrow_mut() = Some(window.clone_strong());
        self.paint_workbench();
        self.paint_notices();
        launcher::paint(self);
        window.show()?;
        self.skins.after_show(WindowKey::Workbench);
        Ok(())
    }

    /// The ledger window of `mode`: the one open is shown again; else a new
    /// one, with the shared look and every line painted before it shows, and
    /// its reader started after, so nothing is read before the window is up.
    /// `tab` is a new window's first tab. A window that cannot be made or
    /// shown is said on the workbench, and in the log. With no state root
    /// there is nothing to open: the entry is off.
    pub fn open_ledger(&self, mode: SourceMode, tab: Option<LedgerView>) {
        let Ok(launcher) = &self.launcher else {
            return;
        };
        // A window closed a moment ago and not out of the registry yet is not
        // shown again: it reads nothing more.
        let open = self
            .ledgers()
            .into_iter()
            .find(|handle| handle.mode() == mode && !handle.is_closed());
        if let Some(open) = open {
            // Slint 1.17 has no call that brings a window to the front.
            open.window.window().set_minimized(false);
            self.window_shown(open.window.show());
            return;
        }
        let window = match LedgerWindow::new() {
            Ok(window) => window,
            Err(error) => {
                self.window_shown(Err(error));
                return;
            }
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let open = OpenArgs { state_root: launcher.state_root.clone(), mode, tab };
        let handle = LedgerHandle::new(id, window, open);
        apply_appearance!(&handle.window, self, WindowKey::Ledger(id));
        ledger::install_callbacks(&handle);
        handle.window.window().on_close_requested(move || match current() {
            Some(ui) => ui.close_ledger(id),
            None => CloseRequestResponse::HideWindow,
        });
        self.ledgers.borrow_mut().insert(id, Rc::clone(&handle));
        self.paint_notices();
        launcher::paint(self);
        handle.refresh(self);
        if let Err(error) = handle.window.show() {
            // Nothing was read: the window goes as it came.
            self.skins.window_closing(WindowKey::Ledger(id));
            let removed = self.ledgers.borrow_mut().remove(&id);
            drop(removed);
            self.window_shown(Err(error));
            return;
        }
        self.skins.after_show(WindowKey::Ledger(id));
        self.window_shown(Ok(()));
        handle.start(None);
    }

    /// What became of a ledger window asked for: a failure stays on the
    /// workbench until one shows, and goes to the log.
    fn window_shown(&self, shown: Result<(), PlatformError>) {
        let failure = shown.err().map(|error| error.to_string());
        if let Some(error) = &failure {
            report::log("acui-window", &fill(self.labels().window_failed, &[error]));
        }
        self.notices.borrow_mut().window = failure;
        self.paint_notices();
    }

    /// A ledger window a person closed: its timers stop and its reads in flight
    /// are let go at once (`LedgerHandle::close`), whether or not it fades. A
    /// window with nothing to fade loses its native frame now and hides on this
    /// answer; it leaves the registry once the close has been handled, so the
    /// window is never let go inside its own callback. One with a frame of its
    /// own stays through its fade (`Ui::finish_ledger_close`).
    fn close_ledger(&self, id: u64) -> CloseRequestResponse {
        let key = WindowKey::Ledger(id);
        let closing = self.skins.begin_close(self, key);
        if matches!(closing, Closing::Underway) {
            return CloseRequestResponse::KeepWindowShown;
        }
        if let Some(handle) = self.ledger(id) {
            handle.close();
        }
        let Closing::FadingFor(fade) = closing else {
            self.skins.window_closing(key);
            Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = current() {
                    let removed = ui.ledgers.borrow_mut().remove(&id);
                    drop(removed);
                }
            });
            return CloseRequestResponse::HideWindow;
        };
        Timer::single_shot(fade, move || {
            if let Some(ui) = current() {
                ui.finish_ledger_close(id);
            }
        });
        CloseRequestResponse::KeepWindowShown
    }

    /// A ledger window's fade has run out: hidden, its native frame off, and let
    /// go. A window that does not hide is said in the log.
    fn finish_ledger_close(&self, id: u64) {
        if let Some(handle) = self.ledger(id) {
            if let Err(error) = handle.window.hide() {
                report::log("acui-window", &format!("acui-ledger-{id}: {error}"));
            }
        }
        self.skins.window_closing(WindowKey::Ledger(id));
        let removed = self.ledgers.borrow_mut().remove(&id);
        drop(removed);
    }

    /// After the event loop: every native frame off, then every window hidden
    /// and let go, before the shared state itself goes. A window that does not
    /// hide is said in the log.
    pub fn close_all(&self) {
        self.skins.detach_all();
        let ledgers = std::mem::take(&mut *self.ledgers.borrow_mut());
        for handle in ledgers.values() {
            handle.close();
            if let Err(error) = handle.window.hide() {
                report::log("acui-window", &format!("acui-ledger-{}: {error}", handle.id));
            }
        }
        drop(ledgers);
        let workbench = self.workbench.borrow_mut().take();
        if let Some(window) = workbench {
            if let Err(error) = window.hide() {
                report::log("acui-window", &format!("workbench: {error}"));
            }
        }
    }

    /// A language picked in either kind of window: saved, then shown at once
    /// in every window — the labels each draws, the lines it fills and the
    /// notices, the save's failure among them. A ledger window's repaint holds
    /// its state's lock for the paint only, so it does not wait on a read in
    /// flight. Setting a box's index from here does not call `selected` again.
    pub fn set_language(&self, language: Language) {
        let saved = self.settings.save_language(language);
        self.language.set(language);
        let labels = self.labels();
        let index = language_index(language);
        if let Some(window) = self.workbench() {
            fill_strings(window.global::<Strings>(), labels);
            window.set_language_index(index);
        }
        for handle in self.ledgers() {
            fill_strings(handle.window.global::<Strings>(), labels);
            handle.window.set_language_index(index);
        }
        self.skins.paint_failures(self);
        self.settings_saved(saved);
        self.paint_workbench();
        launcher::paint(self);
        for handle in self.ledgers() {
            handle.refresh(self);
        }
    }

    /// A text size picked in either kind of window: applied now in every
    /// window, through the one scale property. Only the text size is written:
    /// the language is the file's own, or a `--lang` override that this run
    /// must not save over it. A save that fails still leaves the size applied,
    /// and says so at the top. The caption band grows with the text, so a
    /// window's own frame is told its new measures.
    pub fn set_text_size(&self, size: TextSize) {
        let saved = self.settings.save_text_size(size);
        self.text_size.set(size);
        if let Some(window) = self.workbench() {
            window.global::<Scale>().set_factor(size.factor());
            window.set_text_size_index(size.index());
            self.skins.geometry_changed(self, WindowKey::Workbench);
        }
        for handle in self.ledgers() {
            handle.window.global::<Scale>().set_factor(size.factor());
            handle.window.set_text_size_index(size.index());
            self.skins.geometry_changed(self, WindowKey::Ledger(handle.id));
        }
        self.settings_saved(saved);
    }

    /// A theme picked in either kind of window: applied now in every window,
    /// through each window's own look. Only the theme is written, as with the
    /// text size; a save that fails still leaves the theme applied, and says so
    /// at the top.
    pub fn set_theme(&self, theme: Theme) {
        let saved = self.settings.save_theme(theme);
        self.theme.set(theme);
        if let Some(window) = self.workbench() {
            window.set_theme_index(theme.index());
            self.skins.switch(self, WindowKey::Workbench, theme);
        }
        for handle in self.ledgers() {
            handle.window.set_theme_index(theme.index());
            self.skins.switch(self, WindowKey::Ledger(handle.id), theme);
        }
        // The save rewrote the line a bad value stood on, so the warning the
        // start gave about that line ("kept as it is") is no longer true.
        if saved.is_ok() {
            self.notices.borrow_mut().settings.retain(|issue| {
                !matches!(issue, SettingsIssue::UnknownValue { key: "theme", .. })
            });
        }
        self.settings_saved(saved);
    }

    /// A box's save: its failure goes to the log and stays on screen; a save
    /// that worked clears the last failure.
    fn settings_saved(&self, saved: Result<(), SettingsIssue>) {
        let failure = saved.err();
        if let Some(issue) = &failure {
            report::log("settings", &issue.text(self.labels()));
        }
        self.notices.borrow_mut().save = failure;
        self.paint_notices();
    }

    /// The top of every window: the settings file's problems, errors first; on
    /// the workbench, what it could not do itself; and where the technical log
    /// is. A save's failure stays until a save succeeds; one that only repeats
    /// a problem found at the start is not said twice.
    pub fn paint_notices(&self) {
        let labels = self.labels();
        let (errors, warnings, own) = {
            let notices = self.notices.borrow();
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
            let mut own = Vec::new();
            if let Some(error) = &notices.window {
                own.push(fill(labels.window_failed, &[error]));
            }
            if self.close_refused.get() {
                own.push(labels.close_unlock_busy.to_owned());
            }
            (errors.join("\n"), warnings.join("\n"), own.join("\n"))
        };
        if let Some(window) = self.workbench() {
            window.set_settings_errors(errors.as_str().into());
            window.set_settings_warnings(warnings.as_str().into());
            window.set_errors(own.into());
        }
        for handle in self.ledgers() {
            handle.window.set_settings_errors(errors.as_str().into());
            handle.window.set_settings_warnings(warnings.as_str().into());
        }
        self.paint_log_notes();
    }

    /// Where the technical log is, or why it cannot be written, on every
    /// window: by the notices, and by every ledger repaint.
    pub fn paint_log_notes(&self) {
        let (note, failed) = report::log_note(self.labels());
        if let Some(window) = self.workbench() {
            window.set_log_note(note.as_str().into());
            window.set_log_failed(failed);
        }
        for handle in self.ledgers() {
            handle.window.set_log_note(note.as_str().into());
            handle.window.set_log_failed(failed);
        }
    }

    /// The workbench's lines that only the language changes: why this run has
    /// no state root, and the Runtime's configuration file, where an instance
    /// is added by hand for now.
    pub fn paint_workbench(&self) {
        let Some(window) = self.workbench() else {
            return;
        };
        let labels = self.labels();
        let issue = match &self.launcher {
            Ok(_) => String::new(),
            Err(issue) => fill(labels.install_issue, &[&issue.text(labels)]),
        };
        window.set_install_issue(issue.into());
        let config = self.launcher.as_ref().ok().and_then(|launcher| {
            launcher.actingd_config.as_ref()
        });
        let (path, note) = match config {
            Some(path) => (path.display().to_string(), ""),
            None => (String::new(), labels.config_path_missing),
        };
        window.set_config_path(path.into());
        window.set_config_note(note.into());
    }
}

/// The shared state, from a job a worker handed to the event loop or from a
/// window's callback; `None` once the process has let it go.
pub fn current() -> Option<Rc<Ui>> {
    CURRENT.with(|current| current.borrow().upgrade())
}

/// The language a box's index means.
pub fn language_from_index(index: i32) -> Language {
    if index == 1 {
        Language::En
    } else {
        Language::Zh
    }
}

fn language_index(language: Language) -> i32 {
    match language {
        Language::Zh => 0,
        Language::En => 1,
    }
}

/// The workbench's controls. Start and Request Shutdown are the launcher's;
/// "troubleshooting and ledger" opens the window of the default source; the
/// three boxes apply to every window. Closing it ends the program, after its
/// fade if it has a frame of its own, unless an unlock runs: then it stays and
/// says why, without fading.
fn install_workbench(window: &WorkbenchWindow) {
    window.on_start_runtime(|| {
        if let Some(ui) = current() {
            launcher::start(&ui);
        }
    });
    window.on_request_shutdown(|| {
        if let Some(ui) = current() {
            launcher::shutdown(&ui);
        }
    });
    window.on_open_ledger(|| {
        if let Some(ui) = current() {
            ui.open_ledger(SourceMode::Auto, None);
        }
    });
    window.on_text_size_changed(|index| {
        if let Some(ui) = current() {
            ui.set_text_size(TextSize::from_index(index));
        }
    });
    window.on_language_changed(|index| {
        if let Some(ui) = current() {
            ui.set_language(language_from_index(index));
        }
    });
    window.on_theme_changed(|index| {
        if let Some(ui) = current() {
            ui.set_theme(Theme::from_index(index));
        }
    });
    window.window().on_close_requested(|| {
        let Some(ui) = current() else {
            return CloseRequestResponse::HideWindow;
        };
        if ui.unlocking() {
            ui.close_refused.set(true);
            ui.paint_notices();
            return CloseRequestResponse::KeepWindowShown;
        }
        match ui.skins.begin_close(&ui, WindowKey::Workbench) {
            Closing::AtOnce => {
                // Its native frame comes off before it hides.
                ui.skins.window_closing(WindowKey::Workbench);
                quit_event_loop();
                CloseRequestResponse::HideWindow
            }
            Closing::FadingFor(fade) => {
                Timer::single_shot(fade, || {
                    if let Some(ui) = current() {
                        finish_workbench_close(&ui);
                    }
                });
                CloseRequestResponse::KeepWindowShown
            }
            Closing::Underway => CloseRequestResponse::KeepWindowShown,
        }
    });
}

/// The workbench's fade has run out: the program ends, and `Ui::close_all` takes
/// the windows down, the ledger windows with it. The controls stayed live while
/// it faded, so an unlock may have begun: then it stays, as if asked during the
/// unlock.
fn finish_workbench_close(ui: &Ui) {
    if ui.unlocking() {
        ui.skins.close_called_off(ui, WindowKey::Workbench);
        ui.close_refused.set(true);
        ui.paint_notices();
        return;
    }
    quit_event_loop();
}

fn quit_event_loop() {
    if let Err(error) = slint::quit_event_loop() {
        report::log("acui-window", &format!("quit_event_loop: {error}"));
    }
}

/// Fills one window's `Strings` global from one language's table: before the
/// window shows, and again at every language switch, in every window. A box
/// keeps its picked index when its option list is replaced. The ledger
/// window's data source line follows its read face, so its paint sets it.
fn fill_strings(global: Strings<'_>, labels: &Labels) {
    global.set_window_title(labels.window_title.into());
    global.set_ledger_window_title(labels.ledger_window_title.into());
    global.set_ledger_dir(labels.ledger_dir.into());
    global.set_storage_format(labels.storage_format.into());
    global.set_show_up_to(labels.show_up_to.into());
    global.set_text_size(labels.text_size.into());
    global.set_language(labels.language.into());
    global.set_theme(labels.theme.into());
    global.set_text_size_options(shared(&labels.text_sizes));
    global.set_language_options(shared(&labels.languages));
    global.set_theme_options(shared(&labels.theme_options));
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
    global.set_resume_scheduling(labels.resume_scheduling.into());
    global.set_resume_statement(labels.resume_statement.into());
    global.set_resume_confirm(labels.resume_confirm.into());
    global.set_start(labels.start.into());
    global.set_request_shutdown(labels.request_shutdown.into());
    global.set_open_ledger(labels.open_ledger.into());
    global.set_add_instance(labels.add_instance.into());
    global.set_add_instance_note(labels.add_instance_note.into());
    global.set_config_file(labels.config_file.into());
    global.set_caption_minimize(labels.caption_minimize.into());
    global.set_caption_maximize(labels.caption_maximize.into());
    global.set_caption_restore(labels.caption_restore.into());
    global.set_caption_close(labels.caption_close.into());
}
