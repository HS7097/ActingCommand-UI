// SPDX-License-Identifier: GPL-3.0-only
//! What the windows of this process share, on the event loop only: the
//! language shown, the text size, the theme the settings file names, what the
//! settings file holds that this run could not take, and the launcher's view.
//! A window takes its words from here at the moment it paints, so a language
//! switch reaches every line at once.
//!
//! The process holds the strong reference; the thread-local slot holds a weak
//! one. That is how a worker's result, handed to the event loop, finds the
//! launcher's view and the language of the moment without the worker ever
//! holding either.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use crate::launcher::LauncherView;
use crate::settings::{Settings, SettingsFile, SettingsIssue, TextSize, Theme};
use crate::strings::{Labels, Language};

pub struct Ui {
    pub language: Cell<Language>,
    pub text_size: Cell<TextSize>,
    /// Read, checked and kept; nothing draws by it yet.
    pub theme: Cell<Theme>,
    /// The settings file the two boxes save to.
    pub settings: SettingsFile,
    pub notices: RefCell<Notices>,
    pub launcher_view: RefCell<LauncherView>,
}

/// The settings file's problems: what it held at the start that this run could
/// not take, and the last save's failure, shown until a save succeeds.
pub struct Notices {
    pub settings: Vec<SettingsIssue>,
    pub save: Option<SettingsIssue>,
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
        launcher_view: LauncherView,
    ) -> Rc<Self> {
        let ui = Rc::new(Self {
            language: Cell::new(language),
            text_size: Cell::new(stored.text_size),
            theme: Cell::new(stored.theme),
            settings,
            notices: RefCell::new(Notices { settings: issues, save: None }),
            launcher_view: RefCell::new(launcher_view),
        });
        CURRENT.with(|current| *current.borrow_mut() = Rc::downgrade(&ui));
        ui
    }

    /// The table of the language shown now; taken again at every paint.
    pub fn labels(&self) -> &'static Labels {
        self.language.get().labels()
    }
}

/// The shared state, from a job a worker handed to the event loop; `None` once
/// the process has let it go.
pub fn current() -> Option<Rc<Ui>> {
    CURRENT.with(|current| current.borrow().upgrade())
}
