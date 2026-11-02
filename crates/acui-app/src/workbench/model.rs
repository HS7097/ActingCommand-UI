// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's one state (see `workbench`): what the reads left, and the rule
//! for which of them count. A read is taken only for the owner epoch `Link` names,
//! and only a `Status` read changes that epoch: a slice read before a restart that
//! lands after it says nothing, and a slice read of a new Runtime that lands before
//! `Status` saw it is read again, since `Status` seeing it pokes the slow thread.
//! A read begun before a write landed is let go before it gets here (`super::landed`),
//! and a write's answer counts only under its ticket (`actions`).

use std::collections::BTreeSet;
use std::mem;
use std::time::Instant;

use acui_source::workbench::{
    ActivityKind, ConfigRead, ExplainRead, InstanceLive, LiveRead, LiveSnapshot, PresetProblem,
    StartRead,
};
use acui_source::{ClientFailure, PauseFact};

use super::actions::{self, Actions, ActionsView, PresetView};
use super::card::{self, CardView, Down, Schedule};
use super::picture::{LinkNow, Offline, PictureView, Pictures, PreviewGate};
use super::preview::{Order, Target};
use super::read::{Lane, Read, EXPLAIN_TOO_OLD, START_TRIES};
use super::settings::{self, AutostartView, SettingsView, StartView};
use super::tools::{NameTable, NamesRead, WatchdogRead};
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

/// The task names, as the newest run of `actingctl packs` left them. Why one gave
/// none is in the log.
enum Names {
    NotAsked,
    /// It answered, without warnings.
    Ready(NameTable),
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
    start: StartSlice,
    names: Names,
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
    /// The ledger's committed head as the newest `Status` stated it, while online: where the
    /// picture's lookup starts down from.
    ledger_head: Option<u64>,
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
}

/// What a schedule explanation read changed, for what the event loop does next.
pub struct ExplainLanded {
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
    pub fn new() -> Self {
        Self {
            link: Link::Unknown,
            instances: Vec::new(),
            global_pause: None,
            seen_aliases: Vec::new(),
            explain: Slice::empty(),
            explain_asked_at: None,
            config: Slice::empty(),
            start: StartSlice::default(),
            names: Names::NotAsked,
            shown: true,
            epoch: None,
            live_fault: None,
            slow_fault: None,
            config_fault: None,
            told_codes: BTreeSet::new(),
            actions: Actions::default(),
            page: Page::Main,
            autostart: Autostart::NotAsked,
            autostart_asks: 0,
            pictures: Pictures::default(),
            ledger_head: None,
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

    pub fn set_page(&mut self, page: Page) {
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

    /// The settings page's rows of the Runtime's settings, and its field of the one it changes.
    pub fn settings_view(&self) -> SettingsView {
        let read = self.config.value.as_ref();
        settings::view(&self.link, read, self.config_failed(), &self.instances, &self.actions)
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
        self.live_fault.iter().chain(self.slow_fault.iter()).chain(self.config_fault.iter())
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
                        let server = servers
                            .and_then(|servers| servers.get(&instance.instance_id))
                            .map(String::as_str);
                        let selected = picked == Some(instance.alias.as_str());
                        card::card_view(instance, server, global, schedule, selected)
                    })
                    .collect()
            }
            Some(down) => self
                .seen_aliases
                .iter()
                .map(|alias| card::down_view(alias, down, picked == Some(alias.as_str())))
                .collect(),
        }
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

    /// An account picked on its card: whether that changed whose picture shows.
    pub fn pick_account(&mut self, alias: &str) -> bool {
        let before = self.picture_account().map(str::to_owned);
        self.pictures.pick(alias);
        self.picture_account() != before.as_deref()
    }

    pub fn pictures(&self) -> &Pictures {
        &self.pictures
    }

    pub fn pictures_mut(&mut self) -> &mut Pictures {
        &mut self.pictures
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
    /// on the main page of a window not minimized, while the Runtime answers and lists it, and
    /// its gate is open. Fast while it runs a task or its recovery ladder climbs.
    pub fn preview_target(&self) -> Option<Target> {
        if self.page != Page::Main || !self.shown {
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
        self.pictures.view(account, link, gate, now_ms)
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
            Names::Ready(table) => table.title(package, language),
            Names::NotAsked | Names::Unreadable => None,
        }
    }

    /// Whether names are missing for a reason other than the file not having them.
    pub fn names_unreadable(&self) -> bool {
        matches!(self.names, Names::Unreadable)
    }

    /// One run of `actingctl packs`.
    pub fn apply_names(&mut self, read: NamesRead) {
        self.names = match read {
            NamesRead::Table { table, warnings } if warnings.is_empty() => Names::Ready(table),
            NamesRead::Table { .. } | NamesRead::NoTool(_) | NamesRead::Failed(_) => {
                Names::Unreadable
            }
        };
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
        self.actions.status_read(self.global_pause.as_ref());
        let previous = mem::replace(&mut self.link, Link::Online { pid, epoch });
        let connected = !matches!(previous, Link::Online { .. });
        // The run's first answer comes as its first run of `packs` is out or just in:
        // nothing has changed since then.
        let names_stale = epoch_changed && previous != Link::Unknown;
        if names_stale {
            self.names = Names::NotAsked;
        }
        LiveLanded {
            previous,
            connected,
            epoch_changed,
            names_stale,
            fault_cleared,
            unknown_account,
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
        }
        let after = preset_problem(self.config.value.as_ref());
        let preset = match after {
            problem if problem == before => News::Nothing,
            Some(problem) => News::Failed(problem),
            None => News::Recovered,
        };
        ConfigLanded { news, preset, fault_cleared }
    }

    /// `read` panicked; whether that is news for the log and the window.
    pub fn read_panicked(&mut self, read: Read, message: String) -> bool {
        let fault = Some(Fault::Panicked(message));
        let slot = match read {
            Read::Status => &mut self.live_fault,
            Read::Explain => &mut self.slow_fault,
            Read::Config => &mut self.config_fault,
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
    /// stays, for the same epoch coming back.
    fn leave_online(&mut self) {
        self.instances.clear();
        self.global_pause = None;
        self.ledger_head = None;
        self.explain.error = None;
        self.config.error = None;
        self.actions.status_lost();
    }
}

fn preset_problem(read: Option<&ConfigRead>) -> Option<PresetProblem> {
    read.and_then(|read| read.preset.as_ref().err().cloned())
}
