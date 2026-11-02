// SPDX-License-Identifier: GPL-3.0-only
//! What the workbench's top bar writes: "pause all", "put all on duty" and the
//! performance preset.
//!
//! A press takes a ticket for its slot, one out per slot, and the buttons are off
//! meanwhile. It goes out once, on a thread of its own, over a fresh person's
//! connection. Whatever comes back, nothing is sent again by itself: the event loop
//! takes the answer only with the ticket still out, counts a write for the reads
//! (`super::wrote`, which is what lets go of a read begun before it), and shows the
//! newest reads. A restart lets go of every ticket: what was out then may or may not
//! have happened, and the next `Status` says which. Every answer goes in the log,
//! from the thread, so one that lands after the event loop is not lost.
//!
//! Putting all on duty lifts exactly the pause the newest `Status` showed, by its
//! owner epoch and revision (`resume_paused`), and only one a person may lift here
//! (`lifter`): the rest stay off and say who lifts them. A pause another tool set
//! takes two presses, and the first only asks; one pressed here takes one.
//!
//! Picking a preset, or going back to the file's setting, is one change of settings
//! (`ConfigFlow`). It names the version of the newest snapshot, and only a snapshot or an
//! answer that it took moves that version. An answer that says nothing, or that another
//! end got there first, leaves the writes off until a snapshot read after it has come,
//! and only then says what became of it, so a person decides on what they see. Leaving a
//! learning mode the file holds takes two presses, as lifting another tool's pause does.
//!
//! `view` and `preset_view` are pure functions of what the reads and the presses left,
//! without words; `paint` makes the words.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use acui_source::workbench::{
    configure, pause_global, ActivityKind, ConfigEdit, ConfigOutcome, ConfigRead, InstanceLive,
    PauseOutcome, PresetMember, RefusalWhy, SettingSource, CONSOLE_PAUSE_ALL,
};
use acui_source::{resume_paused, ClientFailure, PauseFact, PauseTarget, ResumeOutcome};

use super::model::Link;
use super::read::panic_message;
use super::{failure_line, paint};
use crate::launcher::spawn_worker;
use crate::ledger::resume::{lifter, Lifter};
use crate::report;
use crate::ui::{self, Ui};

/// The presets of the Runtime's defaults file, which have words here, in the order
/// they are shown: from least to most held back, not the table's order by name. Any
/// other preset a configuration file names follows, by its own name.
pub const PRESET_STANDARD: &str = "standard";
pub const PRESET_ADAPTIVE: &str = "adaptive";
pub const PRESET_LIGHT: &str = "light";
const PRESET_ORDER: [&str; 3] = [PRESET_STANDARD, PRESET_ADAPTIVE, PRESET_LIGHT];

/// A pause already holds: it keeps its reason, and this press set nothing.
const ALREADY_PAUSED: &str = "scheduling_already_paused";
/// The pause a press named is no longer the one that holds.
const PAUSE_MOVED: [&str; 3] = [
    "scheduling_pause_revision_mismatch",
    "scheduling_pause_owner_epoch_mismatch",
    "scheduling_not_paused",
];
/// The client's own: a Runtime that does not know the condition lifted nothing.
const RUNTIME_OPERATION_UNSUPPORTED: &str = "runtime_operation_unsupported";

/// What a write applies to. UI-3d and 3e1 add an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Global,
}

/// What kind of write it is. Pausing and resuming are one kind: an answer still out
/// leaves it open whether the other makes sense yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Scheduling,
    Settings,
}

const GLOBAL_SCHEDULING: (Scope, Kind) = (Scope::Global, Kind::Scheduling);
const GLOBAL_SETTINGS: (Scope, Kind) = (Scope::Global, Kind::Settings);

/// The name of one press's request: an answer counts only under the ticket still out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

/// Which button was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    PauseAll,
    ResumeAll,
    /// A change of settings, based on this version of them when there is one.
    Configure { based_on: Option<u64> },
}

impl Write {
    /// Its thread's name, also what its lines in the technical log are under.
    fn thread(self) -> &'static str {
        match self {
            Self::PauseAll => "acui-act-pause",
            Self::ResumeAll => "acui-act-resume",
            Self::Configure { .. } => CONFIG_THREAD,
        }
    }

    fn slot(self) -> (Scope, Kind) {
        match self {
            Self::PauseAll | Self::ResumeAll => GLOBAL_SCHEDULING,
            Self::Configure { .. } => GLOBAL_SETTINGS,
        }
    }
}

const CONFIG_THREAD: &str = "acui-act-config";

/// Where a change of settings stands, on top of the ticket that says one is out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ConfigFlow {
    #[default]
    Idle,
    /// The first press on a preset that leaves the learning mode the file holds asked
    /// about this preset.
    Confirming { name: String },
    InFlight,
    /// The writes stay off until a snapshot read after the answer has come.
    Rereading(Reread),
}

/// Why a snapshot is awaited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reread {
    /// The settings had moved on, and nothing changed.
    Conflict,
    /// Nothing says what became of the change based on this version.
    Unconfirmed { based_on: Option<u64> },
}

/// What the last press came to, before its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    Sending(Write),
    PauseApplied,
    AlreadyPaused,
    Resumed,
    /// The pause was changed or lifted elsewhere: nothing was lifted here.
    PauseMoved,
    /// `code` is the Runtime's own, for its words.
    Refused { write: Write, code: String },
    /// A Runtime that cannot lift a pause on condition.
    Unsupported,
    Unconfirmed,
    NotSent,
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// A write was out across a restart.
    Restarted,
    /// The settings changed as asked. What is shown of them is still a read's.
    ConfigApplied,
    /// Nothing changed, for this reason; any other refusal is `Refused`.
    ConfigRefused(RefusalWhy),
    /// The table read no longer has the preset pressed: nothing was sent.
    PresetGone,
    /// A snapshot is awaited (`ConfigFlow::Rereading`).
    Rereading(Reread),
    /// A snapshot came: the settings had moved on, and it shows how.
    ConflictRefreshed,
    /// A snapshot came, at the version the change was based on: it did not happen.
    NotApplied,
    /// A snapshot came at another version: the settings changed, perhaps by this press.
    UnconfirmedChanged,
    /// A snapshot came without a version to judge by.
    UnconfirmedSeeNow,
}

/// The workbench's writes, on the event loop only.
#[derive(Default)]
pub struct Actions {
    issued: u64,
    out: BTreeMap<(Scope, Kind), Ticket>,
    /// The pause a first press asks to confirm.
    confirming: Option<PauseTarget>,
    flow: ConfigFlow,
    said: Option<Said>,
}

impl Actions {
    /// A new ticket, unless one of its slot is out. Its slot's confirmation ends here.
    fn begin(&mut self, write: Write) -> Option<Ticket> {
        let slot = write.slot();
        if self.out.contains_key(&slot) {
            return None;
        }
        self.issued += 1;
        let ticket = Ticket(self.issued);
        self.out.insert(slot, ticket);
        match write {
            Write::PauseAll | Write::ResumeAll => self.confirming = None,
            Write::Configure { .. } => self.flow = ConfigFlow::InFlight,
        }
        self.said = Some(Said::Sending(write));
        Some(ticket)
    }

    /// An answer: whether `ticket` was the one out, which is all that makes it count.
    /// A change of settings that needs a read before anything more goes out says so.
    fn settle(&mut self, slot: (Scope, Kind), ticket: Ticket, said: Said) -> bool {
        if self.out.get(&slot) != Some(&ticket) {
            return false;
        }
        self.out.remove(&slot);
        if slot == GLOBAL_SETTINGS {
            self.flow = match said {
                Said::Rereading(reread) => ConfigFlow::Rereading(reread),
                _ => ConfigFlow::Idle,
            };
        }
        self.said = Some(said);
        true
    }

    /// Another owner epoch: no ticket holds across it, and no line says anything of
    /// the old one but that a write was out or had not been settled.
    pub fn new_epoch(&mut self) {
        let was_open = !self.out.is_empty() || matches!(self.flow, ConfigFlow::Rereading(_));
        self.out.clear();
        self.confirming = None;
        self.flow = ConfigFlow::Idle;
        self.said = was_open.then_some(Said::Restarted);
    }

    /// A fact snapshot that counts: of this epoch, and read after any write that landed.
    /// That is what a change of settings left waiting was waiting for. A confirmation
    /// goes when what it asks about is no longer so.
    pub fn snapshot_read(&mut self, read: &ConfigRead) {
        match &self.flow {
            ConfigFlow::Rereading(reread) => {
                self.said = Some(match *reread {
                    Reread::Conflict => Said::ConflictRefreshed,
                    Reread::Unconfirmed { based_on } => match (read.version, based_on) {
                        (None, _) => Said::UnconfirmedSeeNow,
                        (Some(now), Some(based_on)) if now == based_on => Said::NotApplied,
                        (Some(_), _) => Said::UnconfirmedChanged,
                    },
                });
                self.flow = ConfigFlow::Idle;
            }
            ConfigFlow::Confirming { name } => {
                let still_asked = read.preset.as_ref().is_ok_and(|preset| {
                    preset.file_holds_full() && preset.names().any(|known| known == name.as_str())
                });
                if !still_asked {
                    self.flow = ConfigFlow::Idle;
                }
            }
            ConfigFlow::Idle | ConfigFlow::InFlight => {}
        }
    }

    /// A `Status` read that counts. A confirmation is of one pause, and goes with it.
    /// A line saying a press worked goes once `Status` says otherwise: the reads begun
    /// before that press were let go, so this says someone changed it since.
    pub fn status_read(&mut self, global: Option<&PauseFact>) {
        if self.confirming.as_ref() != global.map(|pause| &pause.target) {
            self.confirming = None;
        }
        let overtaken = match self.said {
            Some(Said::PauseApplied) => global.is_none(),
            Some(Said::Resumed) => global.is_some(),
            _ => false,
        };
        if overtaken {
            self.said = None;
        }
    }

    /// No `Status` answers: nothing to confirm against.
    pub fn status_lost(&mut self) {
        self.confirming = None;
        if matches!(self.flow, ConfigFlow::Confirming { .. }) {
            self.flow = ConfigFlow::Idle;
        }
    }

    /// A press found nothing to send.
    fn preset_gone(&mut self) {
        self.flow = ConfigFlow::Idle;
        self.said = Some(Said::PresetGone);
    }
}

/// "Put all on duty" now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeEntry {
    /// Nothing to lift here now: no pause, a write out, no Runtime, or one only
    /// someone else lifts (`ActionsView::holding`).
    Off,
    /// A pause pressed here: one press lifts it.
    OnePress(PauseTarget),
    /// A pause another tool set: the first press asks.
    TwoSteps(PauseTarget),
    /// The first press asked, about this pause and its reason as written.
    Confirming { target: PauseTarget, reason: String },
}

/// The top bar's writes as they stand.
#[derive(Debug, Clone)]
pub struct ActionsView {
    pub can_pause: bool,
    pub resume: ResumeEntry,
    /// Who lifts the global pause, while one holds.
    pub holding: Option<Lifter>,
    pub said: Option<Said>,
    /// Accounts under a pause of their own, which lifting the global one leaves, and
    /// whether Alice lifts any of those.
    pub still_paused: usize,
    pub alice_holds: bool,
    /// How many accounts still finish a scheduled run. `None` when that cannot be told:
    /// no answer of `Status` carries a version to judge it by, but the activity that is
    /// counted and the settings' version both came in 0.12.36 (R-0b, R-4), so a fact
    /// snapshot of this Runtime with a version says its `Status` names activities, and
    /// a count of none means none.
    pub finishing: Option<usize>,
    /// A snapshot is awaited before anything more is saved, and none can come: the reads
    /// fail, or no Runtime answers (the same epoch answering again reads one).
    pub reread_failed: bool,
}

/// `config_failed`: the fact snapshot reads since the last one that came did not succeed.
/// Not online, nothing records a failed read, so the link says it.
pub fn view(
    link: &Link,
    global: Option<&PauseFact>,
    instances: &[InstanceLive],
    actions: &Actions,
    config: Option<&ConfigRead>,
    config_failed: bool,
) -> ActionsView {
    let online = matches!(link, Link::Online { .. });
    let open = online && !actions.out.contains_key(&GLOBAL_SCHEDULING);
    let resume = match global {
        Some(pause) if open && lifter(&pause.reason) == Lifter::Window => {
            if actions.confirming.as_ref() == Some(&pause.target) {
                ResumeEntry::Confirming {
                    target: pause.target.clone(),
                    reason: pause.reason.clone(),
                }
            } else if pause.reason == CONSOLE_PAUSE_ALL {
                ResumeEntry::OnePress(pause.target.clone())
            } else {
                ResumeEntry::TwoSteps(pause.target.clone())
            }
        }
        _ => ResumeEntry::Off,
    };
    let paused = || instances.iter().filter_map(|instance| instance.pause.as_ref());
    ActionsView {
        can_pause: open && global.is_none(),
        resume,
        holding: global.map(|pause| lifter(&pause.reason)),
        said: actions.said.clone(),
        still_paused: paused().count(),
        alice_holds: paused().any(|pause| lifter(&pause.reason) == Lifter::Alice),
        finishing: config.is_some_and(|read| read.version.is_some()).then(|| {
            instances
                .iter()
                .filter(|instance| {
                    instance
                        .activity
                        .as_ref()
                        .is_some_and(|activity| activity.kind == ActivityKind::PolicyRun)
                })
                .count()
        }),
        reread_failed: matches!(actions.flow, ConfigFlow::Rereading(_))
            && (config_failed || !online),
    }
}

/// The top bar's performance preset now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetView {
    /// No Runtime answers, or no fact snapshot of this one has come yet.
    NotRead,
    /// The read failed, or what it read tells no preset. There is no second table here
    /// to show instead.
    Unreadable,
    Read {
        options: Vec<PresetOption>,
        state: PresetState,
        /// Whether a change of settings can go out now.
        can_write: bool,
        /// Whether a setting of a preset holds an online value, which the file's replaces.
        back_to_file: bool,
        /// The preset a first press asks to confirm.
        confirming: Option<String>,
    },
}

/// One preset of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetOption {
    pub name: String,
    pub current: bool,
}

/// What is in effect, the first that applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetState {
    Named(String),
    /// No preset, and the configuration file holds the learning mode.
    CustomFileFull,
    /// No preset, and the configuration file names how many tasks run at once.
    CustomFileConcurrency,
    /// No preset otherwise: every setting as it is in effect.
    Custom(Vec<PresetMember>),
}

/// `read` of the Runtime `link` names, unless the reads since failed (`failed`): an
/// older preset would pose as the present.
pub fn preset_view(
    link: &Link,
    read: Option<&ConfigRead>,
    failed: bool,
    actions: &Actions,
) -> PresetView {
    if !matches!(link, Link::Online { .. }) {
        return PresetView::NotRead;
    }
    if failed {
        return PresetView::Unreadable;
    }
    let Some(read) = read else {
        return PresetView::NotRead;
    };
    let Ok(preset) = &read.preset else {
        return PresetView::Unreadable;
    };
    let mut options: Vec<PresetOption> = preset
        .names()
        .map(|name| PresetOption {
            name: name.to_owned(),
            current: preset.current.as_deref() == Some(name),
        })
        .collect();
    options.sort_by_key(|option| {
        PRESET_ORDER
            .iter()
            .position(|name| *name == option.name)
            .unwrap_or(PRESET_ORDER.len())
    });
    let state = match &preset.current {
        Some(name) => PresetState::Named(name.clone()),
        None if preset.file_holds_full() => PresetState::CustomFileFull,
        None if preset.file_fixes_concurrency() => PresetState::CustomFileConcurrency,
        None => PresetState::Custom(preset.members.clone()),
    };
    let (can_write, confirming) = match &actions.flow {
        ConfigFlow::Idle => (true, None),
        ConfigFlow::Confirming { name } => (true, Some(name.clone())),
        ConfigFlow::InFlight | ConfigFlow::Rereading(_) => (false, None),
    };
    let back_to_file = preset.members.iter().any(|member| member.source == SettingSource::Online);
    PresetView::Read { options, state, can_write, back_to_file, confirming }
}

/// What a write's thread hands back.
enum Written {
    Pause(PauseOutcome),
    Resume(ResumeOutcome),
    Config(ConfigOutcome),
    /// The write panicked: whether it got out is not known.
    Panicked(String),
    NoThread(String),
}

impl Written {
    fn said(self, write: Write) -> Said {
        match self {
            Self::Pause(PauseOutcome::Applied { .. }) => Said::PauseApplied,
            Self::Pause(PauseOutcome::Refused(failure)) => {
                let code = refusal(&failure);
                if code == ALREADY_PAUSED {
                    Said::AlreadyPaused
                } else {
                    Said::Refused { write, code: code.to_owned() }
                }
            }
            Self::Resume(ResumeOutcome::Resumed { .. }) => Said::Resumed,
            Self::Resume(ResumeOutcome::Refused(failure)) => {
                let code = refusal(&failure);
                if failure.code == RUNTIME_OPERATION_UNSUPPORTED {
                    Said::Unsupported
                } else if PAUSE_MOVED.contains(&code) {
                    Said::PauseMoved
                } else {
                    Said::Refused { write, code: code.to_owned() }
                }
            }
            Self::Pause(PauseOutcome::NotSent(_)) | Self::Resume(ResumeOutcome::NotSent(_)) => {
                Said::NotSent
            }
            Self::Pause(PauseOutcome::Unconfirmed(_))
            | Self::Resume(ResumeOutcome::Unconfirmed(_)) => Said::Unconfirmed,
            Self::Config(ConfigOutcome::Applied { .. }) => Said::ConfigApplied,
            Self::Config(ConfigOutcome::Conflict { .. }) => Said::Rereading(Reread::Conflict),
            Self::Config(ConfigOutcome::Refused(refused)) => match refused.why {
                RefusalWhy::Other => {
                    Said::Refused { write, code: refusal(&refused.failure).to_owned() }
                }
                why => Said::ConfigRefused(why),
            },
            Self::Config(ConfigOutcome::NotSent(_)) => Said::NotSent,
            Self::Config(ConfigOutcome::Unconfirmed(_)) | Self::Panicked(_) => match write {
                Write::Configure { based_on } => {
                    Said::Rereading(Reread::Unconfirmed { based_on })
                }
                Write::PauseAll | Write::ResumeAll => Said::Unconfirmed,
            },
            Self::NoThread(error) => Said::NoThread { error },
        }
    }

    /// The settings' version a change of them answered with.
    fn applied_version(&self) -> Option<u64> {
        match self {
            Self::Config(ConfigOutcome::Applied { version }) => *version,
            _ => None,
        }
    }

    /// Into the technical log: what went as it should, and the refusals that only
    /// say the state moved on, without lighting up the log's place on the windows;
    /// the rest as the problems they are.
    fn log(&self, write: Write, ticket: Ticket) {
        let prefix = format!("ticket {}", ticket.0);
        match self {
            Self::Pause(PauseOutcome::Applied { revision }) => report::log_quiet(
                write.thread(),
                &format!("{prefix}: paused at revision {revision}"),
            ),
            Self::Resume(ResumeOutcome::Resumed { revision, .. }) => report::log_quiet(
                write.thread(),
                &format!("{prefix}: resumed at revision {revision}"),
            ),
            Self::Pause(PauseOutcome::Refused(failure))
            | Self::Resume(ResumeOutcome::Refused(failure)) => {
                let code = refusal(failure);
                let line = format!("{prefix}: refused: {}", failure_line(failure));
                if code == ALREADY_PAUSED || PAUSE_MOVED.contains(&code) {
                    report::log_quiet(write.thread(), &line);
                } else {
                    report::log(write.thread(), &line);
                }
            }
            Self::Pause(PauseOutcome::NotSent(failure))
            | Self::Resume(ResumeOutcome::NotSent(failure))
            | Self::Config(ConfigOutcome::NotSent(failure)) => report::log(
                write.thread(),
                &format!("{prefix}: not sent: {}", failure_line(failure)),
            ),
            Self::Config(ConfigOutcome::Applied { version }) => report::log_quiet(
                write.thread(),
                &format!("{prefix}: settings changed, version {version:?}"),
            ),
            Self::Config(ConfigOutcome::Conflict { current }) => report::log_quiet(
                write.thread(),
                &format!("{prefix}: settings moved on, version now {current:?}; nothing changed"),
            ),
            Self::Config(ConfigOutcome::Refused(refused)) => report::log(
                write.thread(),
                &format!(
                    "{prefix}: refused: {}, key {:?}, {:?}",
                    failure_line(&refused.failure),
                    refused.key,
                    refused.why,
                ),
            ),
            Self::Pause(PauseOutcome::Unconfirmed(failure))
            | Self::Resume(ResumeOutcome::Unconfirmed(failure))
            | Self::Config(ConfigOutcome::Unconfirmed(failure)) => report::log(
                write.thread(),
                &format!("{prefix}: unconfirmed, not sent again: {}", failure_line(failure)),
            ),
            Self::Panicked(message) => report::log(
                write.thread(),
                &format!("{prefix}: panicked, unconfirmed, not sent again: {message}"),
            ),
            Self::NoThread(error) => {
                report::log(write.thread(), &format!("{prefix}: no thread, not sent: {error}"));
            }
        }
    }
}

/// The code a refusal is known by: the host's own when the receipt names it, which
/// is the one that says why; else the class, else the client's.
fn refusal(failure: &ClientFailure) -> &str {
    failure
        .host
        .as_ref()
        .map(|(code, _)| code.as_str())
        .or(failure.runtime_code.as_deref())
        .unwrap_or(failure.code)
}

/// The "pause all" button. The press may land after the read that turned it off.
pub fn pause_pressed(ui: &Ui) {
    let can_pause = ui.model.borrow().actions_view().can_pause;
    if can_pause {
        send(ui, Write::PauseAll, |root| Written::Pause(pause_global(root)));
    }
}

/// The "put all on duty" button: at once, or the first of two presses.
pub fn resume_pressed(ui: &Ui) {
    let entry = ui.model.borrow().actions_view().resume;
    match entry {
        ResumeEntry::OnePress(target) => send_resume(ui, target),
        ResumeEntry::TwoSteps(target) => {
            ui.model.borrow_mut().actions_mut().confirming = Some(target);
            paint::actions(ui);
        }
        ResumeEntry::Confirming { .. } | ResumeEntry::Off => {}
    }
}

/// The confirming press: of the pause the first press asked about, if it still is the
/// one that holds, since a `Status` that says otherwise ended the confirmation.
pub fn resume_confirmed(ui: &Ui) {
    let entry = ui.model.borrow().actions_view().resume;
    if let ResumeEntry::Confirming { target, .. } = entry {
        send_resume(ui, target);
    }
}

fn send_resume(ui: &Ui, target: PauseTarget) {
    send(ui, Write::ResumeAll, move |root| Written::Resume(resume_paused(root, &target)));
}

/// A preset's segment: at once, or, leaving the learning mode the file holds, the first
/// of two presses. The press may land after the read that made it the one in effect.
pub fn preset_pressed(ui: &Ui, name: &str) {
    let view = ui.model.borrow().preset_view();
    let PresetView::Read { options, state, can_write: true, .. } = view else {
        return;
    };
    if options.iter().any(|option| option.current && option.name == name) {
        return;
    }
    if matches!(state, PresetState::CustomFileFull) {
        ui.model.borrow_mut().actions_mut().flow = ConfigFlow::Confirming { name: name.to_owned() };
        paint::preset(ui);
    } else {
        send_preset(ui, name);
    }
}

/// The confirming press: of the preset the first press asked about, if a snapshot since
/// has not ended the confirmation.
pub fn preset_confirmed(ui: &Ui) {
    let view = ui.model.borrow().preset_view();
    if let PresetView::Read { can_write: true, confirming: Some(name), .. } = view {
        send_preset(ui, &name);
    }
}

/// The "back to the file's setting" button.
pub fn back_to_file_pressed(ui: &Ui) {
    let view = ui.model.borrow().preset_view();
    if matches!(view, PresetView::Read { can_write: true, back_to_file: true, .. }) {
        send_config(ui, ConfigEdit::preset_back_to_file());
    }
}

/// The table is read once at the Runtime's start, so a preset shown is in it: a preset
/// that is not says this program lost track, and nothing goes out.
fn send_preset(ui: &Ui, name: &str) {
    let edit =
        ui.model.borrow().config_read().and_then(|read| ConfigEdit::apply_preset(read, name));
    match edit {
        Some(edit) => send_config(ui, edit),
        None => {
            report::log(CONFIG_THREAD, &format!("preset {name} is not in the table read"));
            ui.model.borrow_mut().actions_mut().preset_gone();
            repaint(ui);
            ui.paint_log_notes();
        }
    }
}

fn send_config(ui: &Ui, edit: ConfigEdit) {
    let based_on = ui.model.borrow().config_version();
    send(ui, Write::Configure { based_on }, move |root| {
        Written::Config(configure(root, &edit, based_on))
    });
}

/// What a press or an answer may have changed: both rows of the top bar's writes.
fn repaint(ui: &Ui) {
    paint::actions(ui);
    paint::preset(ui);
}

/// One write, once, on a thread of its own. A thread the system refuses is that
/// press's answer at once, in red. A panic in the write is an answer as well: with no
/// answer at all the buttons would stay off for good.
fn send(ui: &Ui, write: Write, work: impl FnOnce(&Path) -> Written + Send + 'static) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let Some(ticket) = ui.model.borrow_mut().actions_mut().begin(write) else {
        return;
    };
    let root = launcher.state_root.clone();
    let spawned = spawn_worker(write.thread(), move || {
        let written = match catch_unwind(AssertUnwindSafe(|| work(&root))) {
            Ok(written) => written,
            Err(panic) => Written::Panicked(panic_message(&*panic)),
        };
        written.log(write, ticket);
        report::post(write.thread(), move || {
            if let Some(ui) = ui::current() {
                landed(&ui, write, ticket, written);
            }
        });
    });
    match spawned {
        Ok(()) => repaint(ui),
        Err(error) => {
            let written = Written::NoThread(error.to_string());
            written.log(write, ticket);
            landed(ui, write, ticket, written);
        }
    }
}

/// An answer, on the event loop. One whose ticket a restart let go changes nothing
/// shown: only the log had it.
fn landed(ui: &Ui, write: Write, ticket: Ticket, written: Written) {
    let version = written.applied_version();
    let said = written.said(write);
    let counted = {
        let mut model = ui.model.borrow_mut();
        let counted = model.actions_mut().settle(write.slot(), ticket, said);
        if let (true, Some(version)) = (counted, version) {
            model.config_applied(version);
        }
        counted
    };
    if counted {
        super::wrote(ui);
        repaint(ui);
    } else {
        report::log_quiet(
            write.thread(),
            &format!("ticket {}: its answer came after a restart, and counts for nothing", ticket.0),
        );
    }
    ui.paint_log_notes();
}
