// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's one state (see `workbench`): what the reads left, and the rule
//! for which of them count. A read is taken only for the owner epoch `Link` names,
//! and only a `Status` read changes that epoch: a slice read before a restart that
//! lands after it says nothing, and a slice read of a new Runtime that lands before
//! `Status` saw it is read again, since `Status` seeing it pokes the slow thread.
//! A read begun before a write landed is let go before it gets here (`super::landed`),
//! and a write's answer counts only under its ticket (`actions`).

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::time::Instant;

use acui_source::workbench::device::DeviceOp;
use acui_source::workbench::{
    ActivityKind, ConfigRead, ExplainRead, FenceFact, InstanceLive, LiveRead, LiveSnapshot,
    PresetProblem, RunKey, StartRead, TasksRead,
};
use acui_source::{ClientFailure, PauseFact};

use super::actions::{self, AccountEntry, Actions, ActionsView, PresetView, RulesView};
use super::card::{self, CardView, Down, Schedule};
use super::picture::{
    Doing, LinkNow, Offline, PictureView, Pictures, PreviewGate, RefreshHold, RefreshView,
};
use super::preview::{Order, Target};
use super::read::{Lane, Read, EXPLAIN_TOO_OLD, START_TRIES};
use super::rescue::{Rescue, RescueHold, RescueView, Watch};
use super::settings::{self, AutostartView, SettingsView, StartView};
use super::stop::{Review, StopTarget, Stops};
use super::tasks::{self, Listing, TasksNow, TasksView};
use super::tools::{Displays, NameTable, NamesRead, PackTable, WatchdogRead};
use crate::ledger::resume::ResumeElsewhere;
use crate::strings::{fill, Labels, Language};

/// The Runtime as the newest `Status` read found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// No read has come back yet.
    Unknown,
    Online { pid: u32, epoch: String },
    /// No Runtime to reach: not running.
    Absent(ClientFailure),
    /// A Runtime is there and did not answer: stopping, or busy for now.
    NoAnswer(ClientFailure),
    /// Something is wrong rather than merely down.
    Trouble(ClientFailure),
    /// The thread that reads `Status` did not start, or is gone.
    ThreadFailed,
}

/// What the slow thread read of one kind: the last value taken, when it was, and
/// the failure of the reads since, if they failed.
pub struct Slice<T> {
    pub value: Option<T>,
    pub read_at: Option<Instant>,
    pub error: Option<ClientFailure>,
}

impl<T> Slice<T> {
    fn empty() -> Self {
        Self { value: None, read_at: None, error: None }
    }

    /// One read, held to the owner epoch online: one of another epoch says nothing. A
    /// failure names no epoch: one of a read that was out across a restart is taken for
    /// now, and the read that the restart poked, queued behind it, settles it next. That
    /// read does not itself fail for finding its connection over, since the lane asks
    /// again on a new one.
    fn take(
        &mut self,
        read: Result<T, ClientFailure>,
        epoch_of: impl Fn(&T) -> &str,
        online: &str,
        now: Instant,
    ) -> News<ClientFailure> {
        match read {
            Ok(value) if epoch_of(&value) != online => News::Nothing,
            Ok(value) => {
                let recovered = self.error.take().is_some();
                self.value = Some(value);
                self.read_at = Some(now);
                if recovered {
                    News::Recovered
                } else {
                    News::Nothing
                }
            }
            Err(failure) if self.error.as_ref() == Some(&failure) => News::Nothing,
            Err(failure) => {
                self.error = Some(failure.clone());
                News::Failed(failure)
            }
        }
    }
}

/// A reading thread that does not do its work; said in red until it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Its last read panicked: the message, for the log. The next read tries again.
    Panicked(String),
    /// It did not start, or is gone: why.
    NoThread(String),
}

impl Fault {
    pub fn text(&self, labels: &Labels) -> String {
        match self {
            Self::Panicked(_) => labels.read_panicked.to_owned(),
            Self::NoThread(error) => fill(labels.thread_failed, &[error]),
        }
    }
}

/// Which of the workbench's two pages shows: the accounts, or the settings. Only which:
/// both are always there, so what a box on the hidden one holds stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
}

impl Page {
    /// As `--page` spells it.
    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "main" => Some(Self::Main),
            "settings" => Some(Self::Settings),
            _ => None,
        }
    }
}

/// Which of the main page's right column's two tabs shows (UI-3f1): the live view, or the
/// account's daily task list. For this run only, the live view first; it stays across accounts
/// and pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Picture,
    Tasks,
}

/// The daily task list's read (UI-3f1), of the one account `of` names while its list shows:
/// the last answer of this owner epoch, when it was, the failure of the reads since, and when
/// the slow thread was last told to read it now. Another account, or none, starts it anew.
#[derive(Default)]
struct TasksSlice {
    of: Option<String>,
    value: Option<TasksRead>,
    read_at: Option<Instant>,
    error: Option<ClientFailure>,
    asked_at: Option<Instant>,
}

/// Why the left column has no account to show, when no read has left one there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountsNote {
    Nothing,
    /// The Runtime answers, and its configuration has none.
    NoneConfigured,
    /// No Runtime runs, and none was seen this run.
    NotRunning,
    /// The Runtime cannot be read, and none was seen this run.
    Unreachable,
}

/// What this owner epoch's start report came to. It is asked once, and again after a
/// failure while tries are left (`START_TRIES`).
#[derive(Default)]
struct StartSlice {
    read: Option<StartRead>,
    /// One is out.
    asked: bool,
    /// How many of this epoch's failed.
    failures: u8,
    /// A person said they read the notice of the settings it reset.
    acknowledged: bool,
}

/// The task names, as the newest run of `actingctl packs` left them, and what the daily task
/// list shows of the packages beside them. Why one gave none is in the log.
enum Names {
    NotAsked,
    /// It answered, without warnings.
    Ready { names: NameTable, packs: PackTable },
    /// It did not answer, it answered with warnings, or this run cannot tell how to
    /// run it. No task has a name, which is what the line on the workbench says.
    Unreadable,
}

/// What the last run of `watchdog status` left for the settings page: whether the Runtime
/// is started again when it is gone. It runs as the page is gone to, and at no other time.
enum Autostart {
    NotAsked,
    /// A run is out, under the number of its ask: an answer to another is let go of.
    Asked(u64),
    /// The Runtime started again since: what was read is of before, and is read again when
    /// the page is next gone to.
    LetGo,
    Read(WatchdogRead),
}

pub struct Model {
    link: Link,
    /// As the newest `Status` listed them; none while `link` is not `Online`:
    /// nothing of an earlier answer poses as the present.
    instances: Vec<InstanceLive>,
    global_pause: Option<PauseFact>,
    /// Every alias this run has seen, first seen first. Kept across restarts: an
    /// account seen stays on the workbench, in grey, while no Runtime answers.
    seen_aliases: Vec<String>,
    explain: Slice<ExplainRead>,
    /// When the slow thread was last told to read now: no answer can be asked of
    /// it sooner, so a long gap before that says nothing against the explanation.
    explain_asked_at: Option<Instant>,
    config: Slice<ConfigRead>,
    /// The server a fact snapshot of this owner epoch last named for each instance, by
    /// instance id, an empty one not taken. `task.server` is a task's fact, there and gone
    /// again as the Lab works: a card whose newest snapshot names none shows this one, said
    /// to be last seen. Let go of with the epoch.
    servers_seen: BTreeMap<String, String>,
    start: StartSlice,
    names: Names,
    /// The picture proportions each game declares, from the same run of `packs` as the
    /// names; none while it did not answer.
    displays: Option<Displays>,
    /// Whether the workbench is not minimized. The slow thread does not read while
    /// it is, so no read is late then.
    shown: bool,
    /// The owner epoch last online. A Runtime that comes back with the same one
    /// has not restarted, so what was read of it stands.
    epoch: Option<String>,
    live_fault: Option<Fault>,
    /// The schedule explanation read's, and the thread's.
    slow_fault: Option<Fault>,
    /// The fact snapshot read's. It shares a thread with the explanation and fails on
    /// its own: neither read's success says the other's panic is over.
    config_fault: Option<Fault>,
    /// The daily task list's read's, likewise (UI-3f1).
    tasks_fault: Option<Fault>,
    /// The right column's tab, and its daily task list's read and whether its line of tasks
    /// without a package is open (UI-3f1).
    right_tab: RightTab,
    tasks: TasksSlice,
    tasks_fold_open: bool,
    /// The lines the log got about codes the cards have no words for: once each.
    told_codes: BTreeSet<String>,
    actions: Actions,
    /// Picked on the top bar, or by `--page` at the start.
    page: Page,
    autostart: Autostart,
    /// How many runs of `watchdog status` were asked for.
    autostart_asks: u64,
    /// Which account's picture the right column shows, and what the picture thread found.
    pictures: Pictures,
    /// Each account's operations of its device, and the confirmation asked (UI-3e1-a).
    rescue: Rescue,
    /// Each account's stop of its run, beside them (UI-3e2): kept while no Runtime answers, and
    /// judged again by the next `Status` read online (`stops_reviewed`).
    stops: Stops,
    /// The ledger's committed head as the newest `Status` stated it, while online: where the
    /// picture's lookup starts down from.
    ledger_head: Option<u64>,
    /// Whether the rules panel shows in the picture's place (UI-3e1-b), of whichever account's
    /// picture shows: it stays so across accounts and pages, and shows only while that
    /// account's rescue bar does (`rules_view`).
    rules_open: bool,
    /// The pauses the top bar and the cards offer no resume for, by the settings file: fixed
    /// for the run.
    resume_elsewhere: ResumeElsewhere,
}

/// What of an account's device is out from here (`Model::device_busy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceBusy {
    /// An operation of it, from its rescue bar (`rescue`).
    Rescue(DeviceOp),
    /// A switch of its backends from the settings page, out or with its answer lost while no
    /// `Status` has shown it over (`Actions::switching`).
    BackendSwitch,
    /// A refresh of its picture (`refresh`).
    Refresh,
}

/// What a `Status` read changed, for what the event loop does next.
pub struct LiveLanded {
    pub previous: Link,
    /// `link` became `Online`.
    pub connected: bool,
    /// `link` names another owner epoch than the last one online, or the first.
    pub epoch_changed: bool,
    /// The task names were let go with that epoch, and are to be read again.
    pub names_stale: bool,
    /// The thread's fault is over.
    pub fault_cleared: bool,
    /// The alias `--account` named, which no account this read lists has: the first
    /// one's picture shows instead.
    pub unknown_account: Option<String>,
    /// It showed an unconfirmed switch of a device's backends over: the writes are counted
    /// once more, so that the snapshot that judges it is read after this.
    pub switch_over: bool,
}

/// What a schedule explanation read changed, for what the event loop does next.
pub struct ExplainLanded {
    pub news: News<ClientFailure>,
    /// The read's fault is over.
    pub fault_cleared: bool,
}

/// What a task list's read changed, for what the event loop does next.
pub struct TasksLanded {
    pub news: News<ClientFailure>,
    /// The read's fault is over.
    pub fault_cleared: bool,
}

/// What a fact snapshot read changed, for what the event loop does next.
pub struct ConfigLanded {
    pub news: News<ClientFailure>,
    /// Of the performance presets in what it read.
    pub preset: News<PresetProblem>,
    /// The read's fault is over.
    pub fault_cleared: bool,
}

/// What the log gets about a slow read: only a change, so a failure that repeats is
/// one line, and its end another.
pub enum News<F> {
    Nothing,
    Failed(F),
    Recovered,
}

impl Model {
    pub fn new(resume_elsewhere: ResumeElsewhere) -> Self {
        Self {
            link: Link::Unknown,
            instances: Vec::new(),
            global_pause: None,
            seen_aliases: Vec::new(),
            explain: Slice::empty(),
            explain_asked_at: None,
            config: Slice::empty(),
            servers_seen: BTreeMap::new(),
            start: StartSlice::default(),
            names: Names::NotAsked,
            displays: None,
            shown: true,
            epoch: None,
            live_fault: None,
            slow_fault: None,
            config_fault: None,
            tasks_fault: None,
            right_tab: RightTab::Picture,
            tasks: TasksSlice::default(),
            tasks_fold_open: false,
            told_codes: BTreeSet::new(),
            actions: Actions::default(),
            page: Page::Main,
            autostart: Autostart::NotAsked,
            autostart_asks: 0,
            pictures: Pictures::default(),
            rescue: Rescue::default(),
            stops: Stops::default(),
            ledger_head: None,
            rules_open: false,
            resume_elsewhere,
        }
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The owner epoch online, as the wire spells it.
    pub fn online_epoch(&self) -> Option<&str> {
        match &self.link {
            Link::Online { epoch, .. } => Some(epoch.as_str()),
            Link::Unknown
            | Link::Absent(_)
            | Link::NoAnswer(_)
            | Link::Trouble(_)
            | Link::ThreadFailed => None,
        }
    }

    /// The page shown, or to be shown once the window is up.
    pub fn page(&self) -> Page {
        self.page
    }

    /// A confirmation asked on the page left goes with it (`rescue`), and so do the rules panel's
    /// form and confirmation (`Actions::rules_reset`).
    pub fn set_page(&mut self, page: Page) {
        if self.page != page {
            self.rescue.unconfirm();
            self.actions.rules_reset();
        }
        self.page = page;
    }

    /// The top bar's writes as they stand.
    pub fn actions_view(&self) -> ActionsView {
        actions::view(
            &self.link,
            self.global_pause.as_ref(),
            &self.instances,
            &self.actions,
            self.config.value.as_ref(),
            self.config_failed(),
            &self.resume_elsewhere,
        )
    }

    /// The top bar's performance preset.
    pub fn preset_view(&self) -> PresetView {
        actions::preset_view(
            &self.link,
            self.config.value.as_ref(),
            self.config_failed(),
            &self.actions,
        )
    }

    /// The settings page's rows of the Runtime's settings, and its field of the one it changes;
    /// each device's group of its backends by what else of the device is out (`device_busy`).
    pub fn settings_view(&self) -> SettingsView {
        let read = self.config.value.as_ref();
        let busy = |alias: &str| self.device_busy(alias);
        let failed = self.config_failed();
        settings::view(&self.link, read, failed, &self.instances, &self.actions, &busy)
    }

    /// What this owner epoch's start report says of the settings.
    pub fn start_view(&self) -> StartView {
        let start = &self.start;
        let gave_up = start.failures >= START_TRIES;
        let read = start.read.as_ref();
        settings::start_view(&self.link, read, gave_up, start.acknowledged, &self.instances)
    }

    /// Whether the start report is to be asked for now: online, and none of this epoch read
    /// or out, with tries left.
    pub fn start_due(&self) -> bool {
        matches!(self.link, Link::Online { .. })
            && self.start.read.is_none()
            && !self.start.asked
            && self.start.failures < START_TRIES
    }

    pub fn start_asked(&mut self) {
        self.start.asked = true;
    }

    /// A start report read: whether it counts, which one of another epoch does not (the epoch
    /// it is of asks its own when `Status` sees it).
    pub fn apply_start(&mut self, read: StartRead) -> bool {
        if self.epoch.as_deref() != Some(read.epoch.as_str()) {
            return false;
        }
        self.start.asked = false;
        self.start.read = Some(read);
        true
    }

    /// The start report asked for failed: how many of this epoch's tries are gone. A failure
    /// names no epoch, so one of a read out across a restart counts for the new epoch.
    pub fn start_failed(&mut self) -> u8 {
        self.start.asked = false;
        self.start.failures = self.start.failures.saturating_add(1);
        self.start.failures
    }

    /// The notice of the settings this start reset was read: it goes, for this epoch.
    pub fn start_acknowledged(&mut self) {
        self.start.acknowledged = true;
    }

    /// What the settings page says of the Runtime being started again when it is gone.
    pub fn autostart_view(&self) -> AutostartView {
        match &self.autostart {
            Autostart::NotAsked | Autostart::Asked(_) => AutostartView::Reading,
            Autostart::LetGo => AutostartView::LetGo,
            Autostart::Read(read) => settings::autostart_of(read),
        }
    }

    /// Whether a run of `watchdog status` may be asked for, the settings page being gone to:
    /// not while one is out, whose answer is on its way.
    pub fn autostart_due(&self) -> bool {
        !matches!(self.autostart, Autostart::Asked(_))
    }

    /// A run of it was asked for: the number of the ask.
    pub fn autostart_asked(&mut self) -> u64 {
        self.autostart_asks += 1;
        self.autostart = Autostart::Asked(self.autostart_asks);
        self.autostart_asks
    }

    /// The run asked for under `ask` is over: whether it counts, which it does only while it is
    /// the one out.
    pub fn apply_autostart(&mut self, ask: u64, read: WatchdogRead) -> bool {
        if !matches!(self.autostart, Autostart::Asked(out) if out == ask) {
            return false;
        }
        self.autostart = Autostart::Read(read);
        true
    }

    /// Whether the fact snapshot reads since the last that came failed. A thread that does
    /// not read counts as a read that failed, as it does for the schedule (`schedule`).
    fn config_failed(&self) -> bool {
        self.config.error.is_some()
            || self.config_fault.is_some()
            || matches!(self.slow_fault, Some(Fault::NoThread(_)))
    }

    /// The newest fact snapshot of this owner epoch.
    pub fn config_read(&self) -> Option<&ConfigRead> {
        self.config.value.as_ref()
    }

    /// The settings' version a write is based on: of the newest snapshot, or of a write's
    /// answer since. Never a refusal's, which comes without the values.
    pub fn config_version(&self) -> Option<u64> {
        self.config.value.as_ref().and_then(|read| read.version)
    }

    /// A change of settings answered with this version.
    pub fn config_applied(&mut self, version: u64) {
        if let Some(read) = &mut self.config.value {
            read.version = Some(version);
        }
    }

    pub fn actions_mut(&mut self) -> &mut Actions {
        &mut self.actions
    }

    pub fn faults(&self) -> impl Iterator<Item = &Fault> {
        self.live_fault
            .iter()
            .chain(self.slow_fault.iter())
            .chain(self.config_fault.iter())
            .chain(self.tasks_fault.iter())
    }

    pub fn accounts_note(&self) -> AccountsNote {
        let unseen = self.seen_aliases.is_empty();
        match &self.link {
            Link::Unknown => AccountsNote::Nothing,
            Link::Online { .. } if self.instances.is_empty() => AccountsNote::NoneConfigured,
            Link::Online { .. } => AccountsNote::Nothing,
            Link::Absent(_) if unseen => AccountsNote::NotRunning,
            Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed if unseen => {
                AccountsNote::Unreachable
            }
            Link::Absent(_) | Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed => {
                AccountsNote::Nothing
            }
        }
    }

    /// The accounts of the left column: as `Status` listed them while the Runtime
    /// answers; otherwise every alias seen this run, with nothing an earlier answer
    /// said of it. The one whose picture shows is marked.
    pub fn cards(&self, now: Instant) -> Vec<CardView> {
        let picked = self.picture_account();
        match Down::of(&self.link) {
            None => {
                let schedule = self.schedule(now);
                let servers = self.config.value.as_ref().map(|read| &read.servers);
                let global = self.global_pause.as_ref();
                self.instances
                    .iter()
                    .map(|instance| {
                        // The newest snapshot's server, else the one last seen this epoch.
                        let named = servers
                            .and_then(|servers| servers.get(&instance.instance_id))
                            .filter(|server| !server.is_empty());
                        let (server, last_seen) = match named {
                            Some(server) => (Some(server.as_str()), false),
                            None => match self.servers_seen.get(&instance.instance_id) {
                                Some(server) => (Some(server.as_str()), true),
                                None => (None, false),
                            },
                        };
                        let selected = picked == Some(instance.alias.as_str());
                        let elsewhere = &self.resume_elsewhere;
                        let account = actions::account_view(
                            instance,
                            &self.actions,
                            elsewhere,
                            global.is_some(),
                        );
                        card::card_view(
                            instance, server, last_seen, global, schedule, selected, elsewhere,
                            account,
                        )
                    })
                    .collect()
            }
            Some(down) => self
                .seen_aliases
                .iter()
                .map(|alias| {
                    let account = actions::account_down_view(alias, &self.actions);
                    card::down_view(alias, down, picked == Some(alias.as_str()), account)
                })
                .collect(),
        }
    }

    /// The own button of the card of `alias` now: none while no `Status` answers, or while the
    /// newest does not list it.
    pub fn account_entry(&self, alias: &str) -> Option<AccountEntry> {
        if !matches!(self.link, Link::Online { .. }) {
            return None;
        }
        let instance = self.instances.iter().find(|instance| instance.alias == alias)?;
        let global = self.global_pause.is_some();
        Some(actions::account_view(instance, &self.actions, &self.resume_elsewhere, global).entry)
    }

    /// The account whose picture the right column shows, of the cards `cards` lists:
    /// the one picked while it is among them, else the first; none while there is none.
    pub fn picture_account(&self) -> Option<&str> {
        let aliases: Vec<&str> = match Down::of(&self.link) {
            None => self.instances.iter().map(|instance| instance.alias.as_str()).collect(),
            Some(_) => self.seen_aliases.iter().map(String::as_str).collect(),
        };
        self.pictures.account(&aliases)
    }

    /// `--account`, before the window is up: that account's picture shows once a `Status`
    /// lists it.
    pub fn name_account(&mut self, alias: String) {
        self.pictures.name(alias);
    }

    /// An account picked on its card: whether that changed whose picture shows. A confirmation
    /// asked goes when it did (`rescue`), and so do the rules panel's form and confirmation; the
    /// panel itself stays open or closed.
    pub fn pick_account(&mut self, alias: &str) -> bool {
        let before = self.picture_account().map(str::to_owned);
        self.pictures.pick(alias);
        let changed = self.picture_account() != before.as_deref();
        if changed {
            self.rescue.unconfirm();
            self.actions.rules_reset();
        }
        changed
    }

    /// The right column's tab (UI-3f1).
    pub fn right_tab(&self) -> RightTab {
        self.right_tab
    }

    /// A tab picked: whether that changed which shows. A confirmation asked under the live view
    /// goes with it, as with the page (`set_page`), and so do the rules panel's form and
    /// confirmation; the panel itself stays open or closed.
    pub fn pick_tab(&mut self, tab: RightTab) -> bool {
        if self.right_tab == tab {
            return false;
        }
        self.right_tab = tab;
        self.rescue.unconfirm();
        self.actions.rules_reset();
        true
    }

    /// The main page shows its live view, in a window not minimized.
    fn picture_tab_shows(&self) -> bool {
        self.page == Page::Main && self.shown && self.right_tab == RightTab::Picture
    }

    /// The account whose daily task list the slow thread is to read now (UI-3f1), if any: the one
    /// whose picture the right column names, on the main page of a window not minimized under the
    /// list's tab, while the Runtime answers and lists it.
    pub fn tasks_wanted(&self) -> Option<String> {
        if self.page != Page::Main || !self.shown || self.right_tab != RightTab::Tasks {
            return None;
        }
        self.online_epoch()?;
        let alias = self.picture_account()?;
        self.instances.iter().any(|instance| instance.alias == alias).then(|| alias.to_owned())
    }

    /// The slow thread is told to read the list of `wanted` from now on, or none: whether that
    /// changed. A change lets go of all that was read, so the list says it is reading until an
    /// answer of that account comes, and closes its line of tasks without a package.
    pub fn tasks_follow(&mut self, wanted: Option<&str>) -> bool {
        if self.tasks.of.as_deref() == wanted {
            return false;
        }
        self.tasks = TasksSlice { of: wanted.map(str::to_owned), ..TasksSlice::default() };
        self.tasks_fold_open = false;
        true
    }

    /// The slow thread was told to read the list now.
    pub fn tasks_asked(&mut self, now: Instant) {
        self.tasks.asked_at = Some(now);
    }

    /// The list's line of tasks without a package opened, or closed.
    pub fn tasks_fold_toggled(&mut self) {
        self.tasks_fold_open = !self.tasks_fold_open;
    }

    /// One read of the list of `alias`, held to the account it follows and the owner epoch
    /// online, as `Slice::take` holds a read to the epoch: one of another account, or of none,
    /// says nothing, and one of another epoch neither.
    pub fn apply_tasks(
        &mut self,
        alias: &str,
        read: Result<TasksRead, ClientFailure>,
        now: Instant,
    ) -> TasksLanded {
        let fault_cleared = self.tasks_fault.take().is_some();
        let Link::Online { epoch, .. } = &self.link else {
            return TasksLanded { news: News::Nothing, fault_cleared };
        };
        let slice = &mut self.tasks;
        if slice.of.as_deref() != Some(alias) {
            return TasksLanded { news: News::Nothing, fault_cleared };
        }
        let news = match read {
            Ok(read) if read.epoch != *epoch || read.alias != alias => News::Nothing,
            Ok(read) => {
                let recovered = slice.error.take().is_some();
                slice.value = Some(read);
                slice.read_at = Some(now);
                if recovered {
                    News::Recovered
                } else {
                    News::Nothing
                }
            }
            Err(failure) if slice.error.as_ref() == Some(&failure) => News::Nothing,
            Err(failure) => {
                slice.error = Some(failure.clone());
                News::Failed(failure)
            }
        };
        TasksLanded { news, fault_cleared }
    }

    /// Nothing new of the list for too long, while the window shows: what was read is not what
    /// the schedule says now.
    fn tasks_late(&self, now: Instant) -> bool {
        let since = self.tasks.read_at.max(self.tasks.asked_at);
        self.shown
            && since.is_some_and(|since| now.saturating_duration_since(since) > EXPLAIN_TOO_OLD)
    }

    /// The daily task list of the account whose picture the right column names (UI-3f1), as the
    /// reads leave it at `now`: no row of an answer that is not of this account and epoch, or not
    /// new, or after a failure. Above its rows the account's pause or fence as its card says it.
    /// Rows no `ui.json` orders go by their names in `language`.
    pub fn tasks_view(&self, now: Instant, language: Language) -> TasksView {
        if let Some(down) = Down::of(&self.link) {
            return TasksView::Down(down);
        }
        let Some(instance) = self
            .picture_account()
            .and_then(|alias| self.instances.iter().find(|instance| instance.alias == alias))
        else {
            return TasksView::NoAccount;
        };
        let ours = self.tasks.of.as_deref() == Some(instance.alias.as_str());
        let thread_gone = matches!(self.slow_fault, Some(Fault::NoThread(_)));
        let read = if self.tasks_fault.is_some() || thread_gone || self.tasks_late(now) {
            TasksNow::Failed(None)
        } else {
            match (&self.tasks.error, &self.tasks.value) {
                (Some(failure), _) if ours => TasksNow::Failed(Some(failure)),
                (None, Some(read)) if ours => TasksNow::Read(read),
                _ => TasksNow::Reading,
            }
        };
        let global = self.global_pause.as_ref();
        let schedule = self.schedule(now);
        let (_, main, notes) = card::said(instance, global, schedule, &self.resume_elsewhere);
        let (names, packs) = match &self.names {
            Names::Ready { names, packs } => (Some(names), Some(packs)),
            Names::NotAsked | Names::Unreadable => (None, None),
        };
        tasks::view(Listing {
            instance,
            paused: instance.pause.is_some() || global.is_some(),
            held: tasks::held(main, notes),
            read,
            names,
            packs,
            names_unreadable: self.names_unreadable(),
            fold_open: self.tasks_fold_open,
            language,
        })
    }

    pub fn pictures(&self) -> &Pictures {
        &self.pictures
    }

    pub fn pictures_mut(&mut self) -> &mut Pictures {
        &mut self.pictures
    }

    pub fn rescue_mut(&mut self) -> &mut Rescue {
        &mut self.rescue
    }

    pub fn stops_mut(&mut self) -> &mut Stops {
        &mut self.stops
    }

    /// What a press of "stop this run" on `alias`'s bar would stop now (UI-3e2): while its rescue
    /// bar offers that button, the run the newest `Status` names for the account, its package,
    /// and the owner epoch online; none while it cannot be pressed.
    pub fn stop_target(&self, alias: &str) -> Option<StopTarget> {
        let Some(RescueView::Online(rows)) = self.rescue_view(alias) else {
            return None;
        };
        if !rows.close.is_some_and(|row| row.offered) {
            return None;
        }
        let epoch = self.online_epoch()?.to_owned();
        let instance = self.instances.iter().find(|instance| instance.alias == alias)?;
        let activity = instance.activity.as_ref()?;
        Some(StopTarget { run: activity.run?, package: activity.package_id.clone(), epoch })
    }

    /// The run the newest `Status` names for `alias`, if any; none while no Runtime answers.
    pub fn account_run(&self, alias: &str) -> Option<RunKey> {
        let instance = self.instances.iter().find(|instance| instance.alias == alias)?;
        instance.activity.as_ref()?.run
    }

    /// What each account's stop waits for, judged by the newest `Status` when it is a read online
    /// at `at_ms`, and only then: one that does not come online changes nothing (`stop`). A
    /// follow-up's ticket is of the same count as the device's operations'.
    pub fn stops_reviewed(&mut self, at_ms: u64) -> Review {
        let Link::Online { epoch, .. } = &self.link else {
            return Review::default();
        };
        self.stops.reviewed(epoch, &self.instances, &mut self.rescue, at_ms)
    }

    /// Which of its device's works out from here holds `alias`'s device, if any: an operation of
    /// it, a switch of its backends, a refresh of its picture, said in that order. The one place
    /// where each of them keeps the others from going out for an account: the Runtime names none
    /// of them while it waits its turn in the instance's queue, nor a game's operation at all,
    /// and a `Status` comes every few seconds. One account's work holds no other's.
    pub fn device_busy(&self, alias: &str) -> Option<DeviceBusy> {
        if let Some(op) = self.rescue.out(alias) {
            return Some(DeviceBusy::Rescue(op));
        }
        // The switch names its device by instance id: this `Status` gives the account's.
        let id = self
            .instances
            .iter()
            .find(|instance| instance.alias == alias)
            .map(|instance| instance.instance_id.as_str());
        if id.is_some() && self.actions.switching() == id {
            return Some(DeviceBusy::BackendSwitch);
        }
        if self.pictures.refresh_out(alias) {
            return Some(DeviceBusy::Refresh);
        }
        None
    }

    /// Whether the picture may read `alias` now. UI-3d writes this from the account's player
    /// hold and its own ask for the device; nothing that asks it changes then.
    pub fn preview_gate(&self, alias: &str) -> PreviewGate {
        if self.pictures.halted(alias) {
            PreviewGate::Freeze { since_ms: None }
        } else {
            PreviewGate::Open
        }
    }

    /// The account the picture thread is to follow now, if any: the one whose picture shows,
    /// on the main page of a window not minimized under the live view's tab, while the Runtime
    /// answers and lists it, and its gate is open. Fast while it runs a task or its recovery
    /// ladder climbs.
    pub fn preview_target(&self) -> Option<Target> {
        if !self.picture_tab_shows() {
            return None;
        }
        let Link::Online { epoch, .. } = &self.link else {
            return None;
        };
        let alias = self.picture_account()?;
        let instance = self.instances.iter().find(|instance| instance.alias == alias)?;
        if self.preview_gate(alias) != PreviewGate::Open {
            return None;
        }
        let fast = instance.activity.as_ref().is_some_and(|activity| {
            matches!(
                activity.kind,
                ActivityKind::PolicyRun
                    | ActivityKind::DirectRun
                    | ActivityKind::HostRun
                    | ActivityKind::Ladder
            )
        });
        Some(Target {
            alias: alias.to_owned(),
            key: instance.key,
            instance_id: instance.instance_id.clone(),
            epoch: epoch.clone(),
            head: self.ledger_head,
            fast,
        })
    }

    /// What the picture thread is to do now (`preview::sync`).
    pub fn preview_sync(&mut self, now: Instant) -> Option<Order> {
        let target = self.preview_target();
        let account = self.picture_account().map(str::to_owned);
        let withdrawn = account
            .as_deref()
            .is_some_and(|alias| self.preview_gate(alias) == PreviewGate::Withdrawn);
        self.pictures.sync(account.as_deref(), target, withdrawn, now)
    }

    /// What the picture area shows at `now_ms`.
    pub fn picture_view(&self, now_ms: u64) -> PictureView {
        let account = self.picture_account();
        let link = match &self.link {
            Link::Unknown => LinkNow::Pending,
            Link::Online { epoch, .. } => LinkNow::Online(epoch.as_str()),
            Link::Absent(_) => LinkNow::Offline(Offline::NotRunning),
            Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed => {
                LinkNow::Offline(Offline::Unreachable)
            }
        };
        let gate = account.map_or(PreviewGate::Open, |alias| self.preview_gate(alias));
        // The proportions the account's game declares, while `Status` names its game.
        let ratios = account
            .and_then(|alias| self.instances.iter().find(|instance| instance.alias == alias))
            .and_then(|instance| instance.game_id.as_deref())
            .and_then(|game| self.displays.as_ref()?.ratios(game));
        self.pictures.view(account, link, gate, ratios, now_ms)
    }

    /// What the refresh under the picture offers now (UI-3c-2): none while no account's picture
    /// shows, when there is no button.
    pub fn refresh_view(&self) -> Option<RefreshView> {
        let alias = self.picture_account()?;
        Some(self.pictures.refresh_view(alias, self.refresh_hold(alias)))
    }

    /// Why `alias`, whose picture shows, cannot be refreshed now, as the reads say: the first of
    /// these that does not hold. Its picture is followed (`preview_target`): the Runtime answers
    /// and lists it, and its gate is open. No operation of its device, nor a switch of its
    /// backends, is out from here (`device_busy`). It does nothing the Runtime names, whatever
    /// that is. Its emulator was not last read as off: a reading of `unknown` lets it, and one of
    /// `off` may be stale. It is not fenced, and its ladder does not hold. A pause holds nothing
    /// back: a refresh under one takes the device a moment, which the cost under the button says.
    fn refresh_hold(&self, alias: &str) -> Option<RefreshHold> {
        if self.preview_target().is_none() {
            let hold = if !self.picture_tab_shows() {
                RefreshHold::Hidden
            } else if self.online_epoch().is_none() {
                RefreshHold::NotOnline
            } else {
                // Online, the picture is of an account `Status` lists: its gate is shut.
                RefreshHold::GateShut
            };
            return Some(hold);
        }
        let Some(instance) = self.instances.iter().find(|instance| instance.alias == alias) else {
            return Some(RefreshHold::NotOnline);
        };
        if matches!(
            self.device_busy(alias),
            Some(DeviceBusy::Rescue(_) | DeviceBusy::BackendSwitch)
        ) {
            return Some(RefreshHold::Busy(Doing::OperatingDevice));
        }
        if let Some(activity) = &instance.activity {
            let hold = match activity.kind {
                ActivityKind::PolicyRun | ActivityKind::DirectRun | ActivityKind::HostRun => {
                    RefreshHold::Running
                }
                ActivityKind::Reconnect => RefreshHold::Busy(Doing::Reconnecting),
                ActivityKind::Selfcheck => RefreshHold::Busy(Doing::SelfChecking),
                ActivityKind::EmulatorControl => RefreshHold::Busy(Doing::ControllingEmulator),
                ActivityKind::Ladder => RefreshHold::Busy(Doing::Climbing),
            };
            return Some(hold);
        }
        if let Some(at_ms) = instance.emulator_off {
            return Some(RefreshHold::EmulatorOff { at_ms });
        }
        if instance.fence.is_some() || instance.ladder_busy {
            return Some(RefreshHold::Fault);
        }
        None
    }

    /// What `alias`'s rescue bar offers now (UI-3e1-a), as the reads and the account's own
    /// presses leave it: none while the main page does not show, or its live view's tab, the
    /// window is minimized, no `Status` has answered yet, or the one online does not list the
    /// account; while no Runtime answers, only that no device can be operated. Its "stop this
    /// run" and what the account's last stop came to are its stops' (`stop`), which a Runtime
    /// that does not answer leaves as they were, to be judged and drawn once one does; only a
    /// stop out or awaited as the reading thread ended is drawn over that line, how it goes and
    /// what it came to, since then no read online comes again (`Stops::unread`).
    pub fn rescue_view(&self, alias: &str) -> Option<RescueView> {
        if !self.picture_tab_shows() {
            return None;
        }
        let epoch = match &self.link {
            Link::Online { epoch, .. } => epoch.as_str(),
            Link::Unknown => return None,
            Link::Absent(_) | Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed => {
                let stop = self.stops.unread(alias).cloned();
                return Some(RescueView::Offline { stop });
            }
        };
        let instance = self.instances.iter().find(|instance| instance.alias == alias)?;
        let gate = self.preview_gate(alias);
        let hold = self.rescue_hold(instance, gate);
        let watch = self.watch(instance);
        let file_value = self.file_value_lifts(instance);
        let withdrawn = gate == PreviewGate::Withdrawn;
        let stops = &self.stops;
        let rows = self.rescue.view(instance, withdrawn, hold, watch, file_value, stops, epoch);
        Some(RescueView::Online(rows))
    }

    /// Whether "Use the file's value" of `instance`'s device backends group would lift its fence
    /// (#402 HS-5c, Runtime 0.12.40): the fence's cause is the instance's own backend open that
    /// failed, which a switch goes through, and in the newest snapshot of this epoch the group
    /// offers to clear an online value back to a file's value that switches that very backend
    /// (`actions::file_value_switches`). Not while the rows are behind a change of settings taken
    /// since, which may have just cleared it.
    fn file_value_lifts(&self, instance: &InstanceLive) -> bool {
        let Some(failed) = instance.fence.as_ref().and_then(FenceFact::failed_open) else {
            return false;
        };
        if self.actions.rows_behind() {
            return false;
        }
        self.config
            .value
            .as_ref()
            .and_then(|read| read.backends.get(&instance.instance_id))
            .is_some_and(|backends| actions::file_value_switches(backends, &failed))
    }

    /// Whether the emulator's watch would start `instance`'s emulator again once it is stopped,
    /// as the newest snapshot of this epoch has `start_emulator` in effect: the instance's own
    /// pause holds the watch whatever that is; no snapshot, or no settings in it, does not read,
    /// nor do rows behind a change of settings taken since, which may have just turned it on or
    /// off from the rules panel (D-20).
    fn watch(&self, instance: &InstanceLive) -> Watch {
        if instance.pause.is_some() {
            return Watch::Stays;
        }
        if self.actions.rows_behind() {
            return Watch::Unknown;
        }
        let on = self
            .config
            .value
            .as_ref()
            .and_then(|read| read.settings.as_ref())
            .and_then(|settings| settings.start_emulator(&instance.instance_id));
        match on {
            Some(true) => Watch::Restarts,
            Some(false) => Watch::Stays,
            None => Watch::Unknown,
        }
    }

    /// Why every device button of `instance`, which the newest `Status` lists, is off now, under
    /// its gate `gate`: the first of these that holds. Its gate is shut. Its device's work is out
    /// from here (`device_busy`): an operation, a refresh of its picture, a switch of its
    /// backends. Its last operation cools down. It runs a task. Its recovery ladder climbs or
    /// holds. It does something else the Runtime names: claiming the device now would only wait
    /// in its queue and be refused. A pause holds nothing back, whoever's, nor a quiet time, nor
    /// a change of settings out.
    fn rescue_hold(&self, instance: &InstanceLive, gate: PreviewGate) -> Option<RescueHold> {
        if matches!(gate, PreviewGate::Freeze { .. }) {
            return Some(RescueHold::GateShut);
        }
        match self.device_busy(&instance.alias) {
            Some(DeviceBusy::Rescue(op)) => return Some(RescueHold::Out(op)),
            Some(DeviceBusy::Refresh) => return Some(RescueHold::Refreshing),
            Some(DeviceBusy::BackendSwitch) => return Some(RescueHold::Switching),
            None => {}
        }
        if self.rescue.cooling(&instance.alias) {
            return Some(RescueHold::CoolingDown);
        }
        let kind = instance.activity.as_ref().map(|activity| activity.kind);
        let hold = match kind {
            Some(ActivityKind::PolicyRun | ActivityKind::DirectRun | ActivityKind::HostRun) => {
                RescueHold::Running
            }
            Some(ActivityKind::Ladder) => RescueHold::Recovering,
            _ if instance.ladder_busy => RescueHold::Recovering,
            Some(ActivityKind::Reconnect) => RescueHold::Busy(Doing::Reconnecting),
            Some(ActivityKind::Selfcheck) => RescueHold::Busy(Doing::SelfChecking),
            Some(ActivityKind::EmulatorControl) => RescueHold::Busy(Doing::ControllingEmulator),
            None => return None,
        };
        Some(hold)
    }

    /// The confirmation asked goes for good once its button cannot be pressed, as a `Status` read
    /// leaves it, the window minimized included: one that came back later would ask about what
    /// may no longer hold.
    fn rescue_reviewed(&mut self) {
        let Some((alias, op)) = self.rescue.asked() else {
            return;
        };
        let offered = self.rescue_view(alias).is_some_and(|view| view.offers(op));
        if !offered {
            self.rescue.unconfirm();
        }
    }

    /// Whether the rules panel is open, which it shows only while the rescue bar does.
    pub fn rules_open(&self) -> bool {
        self.rules_open
    }

    /// The rescue bar's "auto rules": the panel opens, or closes, and then its form and its
    /// confirmation asked go.
    pub fn rules_toggled(&mut self) {
        self.rules_open = !self.rules_open;
        if !self.rules_open {
            self.actions.rules_reset();
        }
    }

    /// What `alias`'s rules panel shows now (UI-3e1-b), as the reads and the presses leave it: none
    /// while its rescue bar has no rows (`rescue_view`), which is also while no Runtime answers.
    /// Nothing of the account's device holds its rules back, nor its gate, its doing or a pause:
    /// only another change of settings, and rows behind one taken (`actions::rules_view`).
    pub fn rules_view(&self, alias: &str) -> Option<RulesView> {
        if !self.picture_tab_shows() || self.online_epoch().is_none() {
            return None;
        }
        let instance = self.instances.iter().find(|instance| instance.alias == alias)?;
        let paused = instance.pause.is_some() || self.global_pause.is_some();
        Some(actions::rules_view(
            self.config.value.as_ref(),
            self.config_failed(),
            instance,
            &self.instances,
            paused,
            &self.actions,
        ))
    }

    /// The rules panel's form and confirmation are of the account whose picture shows, which a
    /// `Status` may have changed: those of another go. The confirmation goes for good, too, once
    /// "delete all" cannot be pressed, as a read leaves it: one that came back later would ask
    /// about what may no longer hold. So does that of every account's quiet times on the settings
    /// page (UI-3e1-c), once its own "delete all" cannot be pressed.
    fn rules_reviewed(&mut self) {
        let shown = self
            .picture_account()
            .and_then(|alias| self.instances.iter().find(|instance| instance.alias == alias))
            .map(|instance| (instance.alias.clone(), instance.instance_id.clone()));
        self.actions.rules_follow(shown.as_ref().map(|(_, id)| id.as_str()));
        if self.actions.host_confirm() {
            let offered = matches!(
                self.settings_view(),
                SettingsView::Read { host_quiet, .. } if host_quiet.quiet.confirming
            );
            if !offered {
                self.actions.host_unconfirm();
            }
        }
        if self.actions.rules_confirm().is_none() {
            return;
        }
        let offered = shown.is_some_and(|(alias, _)| match self.rules_view(&alias) {
            Some(RulesView::Read(rules)) => rules.quiet.confirming,
            Some(RulesView::Reading | RulesView::ReadFailed { .. } | RulesView::Unreported)
            | None => false,
        });
        if !offered {
            self.actions.rules_unconfirm();
        }
    }

    /// What the schedule explanation reads leave now. A thread that does not read
    /// counts as a read that failed: with no value and no failure the card would
    /// otherwise say there is no explanation yet.
    fn schedule(&self, now: Instant) -> Schedule<'_> {
        if self.explain.error.is_some() || self.slow_fault.is_some() || self.explain_late(now) {
            return Schedule::Unreadable;
        }
        match &self.explain.value {
            Some(read) if read.evaluated => Schedule::Evaluated(&read.rows),
            Some(_) | None => Schedule::NotEvaluated,
        }
    }

    fn explain_late(&self, now: Instant) -> bool {
        let since = self.explain.read_at.max(self.explain_asked_at);
        self.shown
            && since.is_some_and(|since| now.saturating_duration_since(since) > EXPLAIN_TOO_OLD)
    }

    /// The slow thread was told to read now.
    pub fn explain_asked(&mut self, now: Instant) {
        self.explain_asked_at = Some(now);
    }

    /// Whether the workbench is not minimized, as last looked at: whether that changed.
    pub fn set_shown(&mut self, shown: bool) -> bool {
        mem::replace(&mut self.shown, shown) != shown
    }

    /// Whether the log has yet to hear `line`; it has from now on.
    pub fn first_told(&mut self, line: &str) -> bool {
        self.told_codes.insert(line.to_owned())
    }

    /// `package`'s task name in `language`, when the names have one.
    pub fn task_title(&self, package: &str, language: Language) -> Option<&str> {
        match &self.names {
            Names::Ready { names, .. } => names.title(package, language),
            Names::NotAsked | Names::Unreadable => None,
        }
    }

    /// `package`'s summary in `language`, when its game's `ui.json` gives one (UI-3f1).
    pub fn pack_summary(&self, package: &str, language: Language) -> Option<&str> {
        let Names::Ready { packs, .. } = &self.names else {
            return None;
        };
        packs.pack(package)?.ui.as_ref()?.summary(language)
    }

    /// The title in `language` of `game`'s group `group` of its `ui.json` (UI-3f1).
    pub fn group_title(&self, game: &str, group: &str, language: Language) -> Option<&str> {
        let Names::Ready { packs, .. } = &self.names else {
            return None;
        };
        packs.group_title(game, group, language)
    }

    /// Whether names are missing for a reason other than the file not having them.
    pub fn names_unreadable(&self) -> bool {
        matches!(self.names, Names::Unreadable)
    }

    /// One run of `actingctl packs`: the names, with the packages' table of the daily task list,
    /// and the picture proportions, which its warnings do not take away (`NamesRead::Table`).
    pub fn apply_names(&mut self, read: NamesRead) {
        let (names, displays) = match read {
            NamesRead::Table { table, packs, displays, warnings } if warnings.is_empty() => {
                (Names::Ready { names: table, packs }, Some(displays))
            }
            NamesRead::Table { displays, .. } => (Names::Unreadable, Some(displays)),
            NamesRead::NoTool(_) | NamesRead::Failed(_) => (Names::Unreadable, None),
        };
        self.names = names;
        self.displays = displays;
    }

    /// One `Status` read: a new owner epoch lets go of everything read of the old
    /// one, but not of the aliases seen.
    pub fn apply_live(&mut self, read: LiveRead) -> LiveLanded {
        let fault_cleared = self.live_fault.take().is_some();
        let down = match read {
            LiveRead::Read(snapshot) => return self.online(*snapshot, fault_cleared),
            LiveRead::NotConnected(failure) => Link::Absent(failure),
            LiveRead::NoAnswer(failure) => Link::NoAnswer(failure),
            LiveRead::Trouble(failure) => Link::Trouble(failure),
        };
        self.leave_online();
        let previous = mem::replace(&mut self.link, down);
        LiveLanded {
            previous,
            connected: false,
            epoch_changed: false,
            names_stale: false,
            fault_cleared,
            unknown_account: None,
            switch_over: false,
        }
    }

    fn online(&mut self, snapshot: LiveSnapshot, fault_cleared: bool) -> LiveLanded {
        let LiveSnapshot { pid, epoch, global_pause, instances, ledger_head } = snapshot;
        self.ledger_head = ledger_head;
        let epoch_changed = self.epoch.as_deref() != Some(epoch.as_str());
        if epoch_changed {
            // Started again, not for the first time this run: what the watchdog said is of
            // before, and a run out now answers for before too. Nothing reads it again until
            // the settings page is next gone to.
            if self.epoch.is_some() && !matches!(self.autostart, Autostart::NotAsked) {
                self.autostart = Autostart::LetGo;
            }
            self.explain = Slice::empty();
            self.config = Slice::empty();
            // Still of the same account: the poke this epoch's first read gives reads it anew.
            self.tasks = TasksSlice { of: self.tasks.of.take(), ..TasksSlice::default() };
            self.servers_seen.clear();
            self.start = StartSlice::default();
            self.epoch = Some(epoch.clone());
            self.actions.new_epoch();
        }
        for instance in &instances {
            if !self.seen_aliases.contains(&instance.alias) {
                self.seen_aliases.push(instance.alias.clone());
            }
        }
        self.instances = instances;
        let aliases: Vec<&str> =
            self.instances.iter().map(|instance| instance.alias.as_str()).collect();
        let unknown_account = self.pictures.settle(&aliases);
        self.global_pause = global_pause;
        let switch_over =
            self.actions.status_read(self.global_pause.as_ref(), &self.instances, Instant::now());
        let previous = mem::replace(&mut self.link, Link::Online { pid, epoch });
        let connected = !matches!(previous, Link::Online { .. });
        // The run's first answer comes as its first run of `packs` is out or just in:
        // nothing has changed since then.
        let names_stale = epoch_changed && previous != Link::Unknown;
        if names_stale {
            self.names = Names::NotAsked;
            self.displays = None;
        }
        self.rescue_reviewed();
        self.rules_reviewed();
        LiveLanded {
            previous,
            connected,
            epoch_changed,
            names_stale,
            fault_cleared,
            unknown_account,
            switch_over,
        }
    }

    /// One schedule explanation read. With no epoch to hold it to, a read says
    /// nothing, and a failure neither: `Status` already said why.
    pub fn apply_explain(
        &mut self,
        read: Result<ExplainRead, ClientFailure>,
        now: Instant,
    ) -> ExplainLanded {
        let fault_cleared = self.slow_fault.take().is_some();
        let Link::Online { epoch, .. } = &self.link else {
            return ExplainLanded { news: News::Nothing, fault_cleared };
        };
        let news = self.explain.take(read, |read| read.epoch.as_str(), epoch, now);
        ExplainLanded { news, fault_cleared }
    }

    /// One fact snapshot read, on the same terms.
    pub fn apply_config(
        &mut self,
        read: Result<ConfigRead, ClientFailure>,
        now: Instant,
    ) -> ConfigLanded {
        let fault_cleared = self.config_fault.take().is_some();
        let Link::Online { epoch, .. } = &self.link else {
            return ConfigLanded { news: News::Nothing, preset: News::Nothing, fault_cleared };
        };
        let before = preset_problem(self.config.value.as_ref());
        let counts = matches!(&read, Ok(read) if read.epoch == *epoch);
        let news = self.config.take(read, |read| read.epoch.as_str(), epoch, now);
        if let (true, Some(read)) = (counts, &self.config.value) {
            self.actions.snapshot_read(read);
            // What it names now stands for the snapshots after it that name none.
            for (instance_id, server) in &read.servers {
                if !server.is_empty() {
                    self.servers_seen.insert(instance_id.clone(), server.clone());
                }
            }
        }
        let after = preset_problem(self.config.value.as_ref());
        let preset = match after {
            problem if problem == before => News::Nothing,
            Some(problem) => News::Failed(problem),
            None => News::Recovered,
        };
        // A read that failed holds the rules as well as one that came changes them.
        self.rules_reviewed();
        ConfigLanded { news, preset, fault_cleared }
    }

    /// `read` panicked; whether that is news for the log and the window.
    pub fn read_panicked(&mut self, read: Read, message: String) -> bool {
        let fault = Some(Fault::Panicked(message));
        let slot = match read {
            Read::Status => &mut self.live_fault,
            Read::Explain => &mut self.slow_fault,
            Read::Config => &mut self.config_fault,
            Read::Tasks => &mut self.tasks_fault,
            // A start report's panic is one of its failures (`start_failed`), not a fault
            // of a read that a later one of its kind clears: there is no later one.
            Read::Start => return false,
        };
        let news = *slot != fault;
        *slot = fault;
        news
    }

    /// `lane`'s thread did not start, or is gone.
    pub fn lane_lost(&mut self, lane: Lane, error: String) {
        *self.fault_slot(lane) = Some(Fault::NoThread(error));
        if lane == Lane::Live {
            self.leave_online();
            self.link = Link::ThreadFailed;
        }
    }

    fn fault_slot(&mut self, lane: Lane) -> &mut Option<Fault> {
        match lane {
            Lane::Live => &mut self.live_fault,
            Lane::Slow => &mut self.slow_fault,
        }
    }

    /// What is shown of a Runtime that no longer answers. The last value read
    /// stays, for the same epoch coming back. No device can be operated then, so the
    /// confirmation asked goes; each account's stop of its run stays, for the next read online
    /// to judge (`stops_reviewed`).
    fn leave_online(&mut self) {
        self.instances.clear();
        self.global_pause = None;
        self.ledger_head = None;
        self.explain.error = None;
        self.config.error = None;
        self.actions.status_lost();
        self.rescue.unconfirm();
    }
}

fn preset_problem(read: Option<&ConfigRead>) -> Option<PresetProblem> {
    read.and_then(|read| read.preset.as_ref().err().cloned())
}
