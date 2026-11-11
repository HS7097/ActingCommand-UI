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
//! The two slots may both be out at once, so each keeps what its last press came to,
//! and the window says both: an answer of one never hides what the other said.
//!
//! Putting all on duty lifts exactly the pause the newest `Status` showed, by its
//! owner epoch and revision (`resume_paused`), and only one a person may lift here
//! (`lifter`): the rest stay off and say who lifts them. A pause another tool set
//! takes two presses, and the first only asks; one pressed here takes one.
//!
//! Each account's card has a button of its own (`account_view`), on the same terms as the top
//! bar's pause and resume but under a ticket of that account's, one out per account: with no
//! pause of its own, "pause this account" sends that instance's pause with this console's own
//! reason (`pause_account`); under the one this console set there, handed back, "resume this
//! account" lifts exactly the pause the newest `Status` showed, by its owner epoch and revision
//! (`resume_paused`), at one press. A pause of the account another tool set gets no button on
//! the card, which says who lifts it; the ledger window resumes those it may.
//!
//! Picking a preset, or going back to the file's setting, is one change of settings
//! (`ConfigFlow`). It names the version of the newest snapshot, and only a snapshot or an
//! answer that it took moves that version. An answer that says nothing, or that another
//! end got there first, leaves the writes off until a snapshot read after it has come,
//! and only then says what became of it, so a person decides on what they see. Leaving a
//! learning mode the file holds takes two presses, as lifting another tool's pause does.
//!
//! How many tasks run at once has a stepper under the presets. Typing and its two buttons
//! change only a draft, and only "apply" sends it, as one more change of settings: an empty
//! draft goes back to the file's setting, a whole number is sent as it is, with no range
//! here (the Runtime's refusal says it), and anything else is not sent. The draft stays
//! while nothing changed, and goes once what it asked for may have happened: with the
//! snapshot read after an answer that says it did, or after one that says nothing, when that
//! snapshot shows another version.
//!
//! The settings page's field of how many days an evidence frame nobody confirmed reading is
//! kept is one more of the same: a draft of its own, and "save" sends it as "apply" does the
//! stepper's. It is a change of settings like the top bar's, so one of them is out at a time
//! and each waits for the snapshot the others wait for; but what it came to is said on a line
//! of its own (`Field`), so neither answer hides the other's.
//!
//! The settings page's group of each device's touch and capture backends (HS-U) is one more:
//! its boxes change a draft of that device's own, and "apply" sends the device's two backend
//! keys that change, one or both, as one change of settings (`Field::Backends`), which
//! switches that device before the Runtime answers. MuMu's own touch goes only with MuMu's own
//! capture: picking it brings that capture along, and picking another capture under it asks
//! which touch goes along before both are sent. "Use the file's value" sends a clear, which is
//! a switch as well. The answer takes as long as the switch, at most the client's whole wait
//! (the contract's `BACKEND_SWITCH_WAIT_MS` and the io timeout, `backends::switch_wait`), and its
//! group says it is switching meanwhile; what it came to is said on a line of its own, naming
//! the account. An answer lost is no other change's (`Reread::Switch`): the device may still be
//! switching, and the settings' version moves only at the switch's end, so nothing is judged
//! until a `Status` shows the switch over, and then a snapshot read after that `Status` says
//! what came of it, with the device's self-check as it then stands. Nothing is sent again by
//! itself, whatever comes back.
//!
//! An account's rules (UI-3e1-b), on the main page's panel in its picture's place, are more of
//! the same: its own quiet times, its idle action and, of a discovery-bound MuMu instance, whether
//! the Runtime starts and watches its emulator (`rules_view`). Each press sends one change of one
//! key of that account's at once, from no draft, as one more change of settings (`Field::Rules`):
//! one of them is out at a time with every other, and while any change of settings is out or
//! awaits its snapshot, a switch of backends included, the panel changes nothing. Its lines speak
//! of the account last written from it (`Actions::rules_target`), as the backends' line does of
//! its device. A quiet time is added or deleted by sending the account's whole list again, derived
//! from the newest snapshot's rows and based on its version; so once any change of settings is
//! taken, nothing is derived from the rows until a snapshot read after it has come
//! (`Actions::rows_behind`): the version its answer names stands in for the snapshot's meanwhile,
//! and the rows are still of before it, so a list derived from them would undo that change and
//! the Runtime would take it. Deleting every quiet time takes two presses. A time typed into the
//! form to add that does not say a day and two times of day, or a list too long for one value or
//! that the Runtime's parser does not take, is said there and not sent; a delete that leaves a
//! list still too long for one value is said on the quiet times' line, and not sent. Stopping the
//! emulator when idle cannot be picked here (Alice's ruling 157), and the emulator's start is not
//! turned off, nor gone back to the file's value, while the idle action in effect stops the
//! emulator, which the Runtime takes only with it on: the person changes the idle action first.
//!
//! `view`, `preset_view`, `evidence_view`, `backend_edit` and `rules_view` are pure functions of
//! what the reads and the presses left, without words; `paint` makes the words.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::time::Instant;

use acui_rows::local_utc_offset_minutes;
use acui_source::workbench::backends::{
    BackendChange, BackendRefusal, BackendRow, BackendTarget, BackendWhy, CaptureBackend,
    DeviceKind, FailedStep, InEffect, InstanceBackends, SwitchSeen, SwitchVersion, TouchBackend,
    UnconfirmedSwitch,
};
use acui_source::workbench::rules::{
    IdleChoice, IdleNow, NewWeekly, QuietNotSent, QuietRead, RuleSetting,
};
use acui_source::workbench::{
    configure, pause_account, pause_global, ActivityKind, ConfigEdit, ConfigOutcome, ConfigRead,
    ConfigRefusal, InstanceLive, PauseOutcome, PresetMember, PresetSetting, RefusalWhy,
    SettingSource, SettingValue, CONSOLE_PAUSE_ACCOUNT, CONSOLE_PAUSE_ALL,
};
use acui_source::{
    resume_paused, ClientFailure, PauseFact, PauseStage, PauseTarget, ResumeOutcome,
};

use super::model::{DeviceBusy, Link};
use super::read::panic_message;
use super::settings::{BackendGroup, SettingsView};
use super::{failure_line, paint};
use crate::launcher::spawn_worker;
use crate::ledger::resume::{lifter, Lifter, ResumeElsewhere};
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

/// Where a change of settings was asked for, which is where what it came to is said: the
/// top bar's presets and stepper, the settings page's field of the evidence days, its
/// group of one device's backends (`Actions::backend_said_of` names which), or one of an
/// account's rules on the main page's rules panel (UI-3e1-b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    TopBar,
    EvidenceDays,
    Backends,
    Rules(RuleItem),
}

/// Which of an account's rules a change from its rules panel is of. The account is not part
/// of it: the line of each speaks of the account last written from the panel, as the
/// backends' line does of its device, so that `Field` stays a plain value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuleItem {
    /// Its own quiet times.
    Quiet,
    /// What is done with it when it has no task soon.
    Idle,
    /// Whether the Runtime starts and watches its emulator.
    Start,
}

/// The line what a press came to is said on: one for pausing and resuming, and one for each
/// field a change of settings is asked from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SaidAt {
    Scheduling,
    Settings(Field),
}

/// The name of one press's request: an answer counts only under the ticket still out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

/// Which button was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    PauseAll,
    ResumeAll,
    /// A change of settings asked from `field`, based on this version of them when there
    /// is one.
    Configure { based_on: Option<u64>, field: Field },
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

    fn said_at(self) -> SaidAt {
        match self {
            Self::PauseAll | Self::ResumeAll => SaidAt::Scheduling,
            Self::Configure { field, .. } => SaidAt::Settings(field),
        }
    }

    /// Whether it is a switch of a device's backends.
    fn switches_backends(self) -> bool {
        matches!(self, Self::Configure { field: Field::Backends, .. })
    }
}

/// Which of an account card's own buttons was pressed (`account_view`). The account is not
/// part of it: each account's presses are kept by its alias (`Actions::accounts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountWrite {
    /// "Pause this account".
    Pause,
    /// "Resume this account".
    Resume,
}

impl AccountWrite {
    /// Its thread's name, also what its lines in the technical log are under.
    fn thread(self) -> &'static str {
        match self {
            Self::Pause => "acui-act-pause-account",
            Self::Resume => "acui-act-resume-account",
        }
    }
}

/// What the last press of an account card's own button came to, before its words: the top
/// bar's pause and resume come to the same (`Said`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountSaid {
    Sending(AccountWrite),
    Paused,
    /// A pause of the account already holds: it keeps its reason, and this press set nothing.
    AlreadyPaused,
    /// The pause is lifted; `device_failure` is the device code of the reconnect that followed
    /// it and failed (`selfcheck.failure_code`), which never rolls the resume back.
    Resumed { device_failure: Option<String> },
    /// The pause was changed or lifted elsewhere: nothing was lifted here.
    PauseMoved,
    /// `code` is the Runtime's own, shown as it gave it, with its words.
    Refused { write: AccountWrite, code: String },
    /// A Runtime that cannot lift a pause on condition.
    Unsupported,
    Unconfirmed,
    NotSent,
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// A press was out across a restart.
    Restarted,
}

/// One account's: the press of its card's own button out, if any, and what the last came to.
#[derive(Debug, Clone, Default)]
struct AccountSlot {
    out: Option<(Ticket, AccountWrite)>,
    said: Option<AccountSaid>,
}

const CONFIG_THREAD: &str = "acui-act-config";
/// What the technical log's lines about the settings page's boxes are under, as `paint`'s.
const SETTINGS_LOG: &str = "acui-settings";
/// And those about the rules panel's.
const RULES_LOG: &str = "acui-rules";

/// Where a change of settings stands, on top of the ticket that says one is out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ConfigFlow {
    #[default]
    Idle,
    /// The first press on a preset that leaves the learning mode the file holds asked
    /// about this preset.
    Confirming { name: String },
    /// One asked from `field` is out.
    InFlight(Field),
    /// The writes stay off until a snapshot read after the answer to the one asked from
    /// `field` has come.
    Rereading(Reread, Field),
}

impl ConfigFlow {
    /// The field of the change out or awaiting its snapshot.
    fn field(&self) -> Option<Field> {
        match self {
            Self::InFlight(field) | Self::Rereading(_, field) => Some(*field),
            Self::Idle | Self::Confirming { .. } => None,
        }
    }

    /// Whether a change of settings can go out now: none is out or awaits its snapshot.
    fn open(&self) -> bool {
        matches!(self, Self::Idle | Self::Confirming { .. })
    }
}

/// Why a snapshot is awaited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reread {
    /// The settings had moved on, and nothing changed.
    Conflict,
    /// Nothing says what became of the change based on this version.
    Unconfirmed { based_on: Option<u64> },
    /// Nothing says what became of a switch of a device's backends, which may still go on
    /// (`Actions::switch` holds it): a snapshot judges it only once a `Status` shows it over.
    Switch,
}

/// What the last press of a slot came to, before its words.
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
    /// "Apply" found no whole number in the stepper's field: nothing was sent.
    NotWhole,
    /// A quiet time's "delete" left a list still longer than one value of a setting may be:
    /// nothing was sent. "Add" says the same in its form (`FormProblem::TooBig`).
    QuietTooBig,
    /// The Runtime refused a change of a device's backends, and said more of it than its class.
    BackendRefused(BackendRefusal),
    /// A device's "use the file's value" changed the settings as asked: the file's value is in
    /// effect. Its receipt is the same whether the device switched or not: the Runtime switches
    /// nothing when the file's value is the one already in effect.
    BackToFile,
    /// A snapshot came after a `Status` showed an unconfirmed switch over: what its version
    /// says of it, and the device's backends and self-check as that snapshot has them.
    SwitchOver { version: SwitchVersion, now: Option<InstanceBackends> },
}

/// One device's draft of its backends: what its boxes were set to, which is not in effect.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BackendDraft {
    touch: Option<TouchBackend>,
    /// The capture picked. MuMu's own touch picked brings MuMu's own capture along whatever
    /// this says, and taking another touch again shows this once more.
    capture: Option<CaptureBackend>,
    /// The touch picked in the question a capture under MuMu's own touch asks.
    asked: Option<TouchBackend>,
    /// A switch of this device was taken since: the draft shows until the snapshot read after
    /// that answer, and is not sent meanwhile.
    applied: bool,
}

impl BackendDraft {
    fn is_empty(&self) -> bool {
        self.touch.is_none() && self.capture.is_none()
    }
}

/// What the form's two fields hold as it opens: a night, as text in them, not a hint over an
/// empty field, so that ticking a day and pressing "add" adds what they show.
const QUIET_FORM_START: &str = "23:00";
const QUIET_FORM_END: &str = "07:00";

/// The rules panel's form of a quiet time to add, of one account at a time: what its boxes and
/// its two fields hold, which is not in effect. Closed while it has no owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QuietForm {
    /// The instance id of the account it is open for.
    owner: Option<String>,
    /// The days the time starts on, `days[0]` Monday through `days[6]` Sunday.
    days: [bool; 7],
    /// Its start and end, hours and minutes: `QUIET_FORM_START` and `QUIET_FORM_END` as it
    /// opens, then as typed.
    start: String,
    end: String,
    /// What the last "add" found in it, which nothing was sent for; it goes once the form is
    /// changed.
    problem: Option<FormProblem>,
    /// What it says was sent, and nothing yet says it was not taken: taken, or perhaps taken
    /// (`Said::UnconfirmedChanged`), it goes, so that it is not added twice.
    adding: bool,
}

impl QuietForm {
    /// The form as the panel of the account `instance` shows it: none while it is not open for
    /// that account.
    fn view(&self, instance: &str) -> Option<QuietFormView> {
        (self.owner.as_deref() == Some(instance)).then(|| QuietFormView {
            days: self.days,
            start: self.start.clone(),
            end: self.end.clone(),
            problem: self.problem,
        })
    }
}

/// A switch whose answer was lost, and whether a `Status` showed it over since.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSwitch {
    switch: UnconfirmedSwitch,
    over: bool,
}

/// What a field holds that is not yet in effect: as typed, or as a step left it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    text: String,
    /// A change of settings asked from this field was taken since, this draft or another (a
    /// preset's, for the stepper): it shows until the snapshot read after that answer, which
    /// shows what holds instead, and is not sent meanwhile.
    applied: bool,
}

/// What a draft asks for.
enum Typed {
    /// The configuration file's value.
    Empty,
    Whole(i64),
    /// Not sent.
    NotWhole,
}

impl Typed {
    fn of(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() {
            Self::Empty
        } else {
            text.parse::<i64>().map_or(Self::NotWhole, Self::Whole)
        }
    }
}

/// The workbench's writes, on the event loop only.
#[derive(Default)]
pub struct Actions {
    issued: u64,
    out: BTreeMap<(Scope, Kind), Ticket>,
    /// The pause a first press asks to confirm.
    confirming: Option<PauseTarget>,
    flow: ConfigFlow,
    /// The stepper's, of how many tasks run at once.
    draft: Option<Draft>,
    /// The settings page's, of the evidence days.
    evidence_draft: Option<Draft>,
    /// The settings page's, of each device's backends, by instance id.
    backend_drafts: BTreeMap<String, BackendDraft>,
    /// The switch of backends out: what it was sent to, and when it was pressed, which an
    /// answer that says nothing makes an `UnconfirmedSwitch` of; and whether it was a "use the
    /// file's value", which a change taken is said as.
    backend_out: Option<(BackendTarget, Instant, bool)>,
    /// The instance id of the device the backends' line speaks of: the last one switched.
    backend_said_of: Option<String>,
    /// The switch whose answer was lost, while `ConfigFlow::Rereading(Reread::Switch, _)`.
    switch: Option<PendingSwitch>,
    /// What the last press said on each line came to: only its own press, answer or read
    /// changes it.
    said: BTreeMap<SaidAt, Said>,
    /// The instance id of the account the rules' lines speak of: the last one a rule was
    /// written for from its panel, or whose "delete" was stopped here (`Said::QuietTooBig`).
    /// Another account's lets go of every line of the one before (`Actions::rules_speak_of`).
    rules_target: Option<String>,
    /// The rules panel's form of a quiet time to add.
    quiet_form: QuietForm,
    /// The instance id of the account whose first press of "delete all" its quiet times asked.
    rules_confirm: Option<String>,
    /// A change of settings was taken, from wherever, and no snapshot read after it has come:
    /// the rows shown are of before it, whatever version its answer named, and nothing derived
    /// from them goes out meanwhile. Only the rules' changes are derived from them: the others
    /// name their keys outright.
    rows_behind: bool,
    /// Each account card's own button, by alias: one press out per account, its tickets of the
    /// same count as the top bar's.
    accounts: BTreeMap<String, AccountSlot>,
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
            Write::Configure { field, .. } => {
                self.flow = ConfigFlow::InFlight(field);
                // Whatever change goes out, "delete all" asked about quiet times as they were.
                self.rules_confirm = None;
            }
        }
        self.said.insert(write.said_at(), Said::Sending(write));
        Some(ticket)
    }

    /// An answer: whether `ticket` was the one out, which is all that makes it count.
    /// A change of settings that needs a read before anything more goes out says so.
    fn settle(&mut self, write: Write, ticket: Ticket, said: Said) -> bool {
        let slot = write.slot();
        if self.out.get(&slot) != Some(&ticket) {
            return false;
        }
        self.out.remove(&slot);
        let mut said = said;
        if let Write::Configure { based_on, field } = write {
            if field == Field::Backends {
                said = self.backends_settled(based_on, said);
            }
            self.flow = match said {
                Said::Rereading(reread) => ConfigFlow::Rereading(reread, field),
                _ => ConfigFlow::Idle,
            };
            // What changed may have changed what the field's draft asked for: from the top
            // bar, a preset included.
            if said == Said::ConfigApplied {
                if let Some(draft) = self.draft_mut(field) {
                    draft.applied = true;
                }
            }
            // Taken, the rows read before it are behind, whatever its field, and whether or not
            // its answer named a version (a Runtime before 0.12.36 names none, and checks none).
            if matches!(said, Said::ConfigApplied | Said::BackToFile) {
                self.rows_behind = true;
            }
            if field == Field::Rules(RuleItem::Quiet) {
                self.quiet_settled(&said);
            }
        }
        self.said.insert(write.said_at(), said);
        true
    }

    /// `write` of the card of the account `alias` pressed: a new ticket, unless one of that
    /// account's is out. What it came to is, for now, that it is being sent.
    fn account_begin(&mut self, alias: &str, write: AccountWrite) -> Option<Ticket> {
        let slot = self.accounts.entry(alias.to_owned()).or_default();
        if slot.out.is_some() {
            return None;
        }
        self.issued += 1;
        let ticket = Ticket(self.issued);
        slot.out = Some((ticket, write));
        slot.said = Some(AccountSaid::Sending(write));
        Some(ticket)
    }

    /// An answer of the account `alias`'s press: whether `ticket` was the one out for it,
    /// which is all that makes it count.
    fn account_settle(&mut self, alias: &str, ticket: Ticket, said: AccountSaid) -> bool {
        let Some(slot) = self.accounts.get_mut(alias) else {
            return false;
        };
        if slot.out.map(|(out, _)| out) != Some(ticket) {
            return false;
        }
        *slot = AccountSlot { out: None, said: Some(said) };
        true
    }

    /// Whether a press of the card of the account `alias` is out.
    fn account_out(&self, alias: &str) -> bool {
        self.accounts.get(alias).is_some_and(|slot| slot.out.is_some())
    }

    /// What the last press of the card of the account `alias` came to, if one was pressed this
    /// epoch.
    pub fn account_said(&self, alias: &str) -> Option<&AccountSaid> {
        self.accounts.get(alias)?.said.as_ref()
    }

    /// A change of the quiet times answered: a time added that was taken leaves the form, closed,
    /// so that it is not added twice; one a snapshot is to judge waits for it; any other answer
    /// says it was not taken, and the form keeps what it holds.
    fn quiet_settled(&mut self, said: &Said) {
        if !self.quiet_form.adding {
            return;
        }
        match said {
            Said::ConfigApplied | Said::BackToFile => self.quiet_form = QuietForm::default(),
            Said::Rereading(_) => {}
            _ => self.quiet_form.adding = false,
        }
    }

    /// A switch of backends answered: the device's draft was taken when the switch was; an
    /// answer that says nothing is held as the switch it may still be. With no switch to hold
    /// (this program lost track of what it sent), it is judged as any other change's, said in
    /// the log as the fault it is.
    fn backends_settled(&mut self, based_on: Option<u64>, said: Said) -> Said {
        let out = self.backend_out.take();
        match (said, out) {
            (Said::ConfigApplied, Some((target, _, to_file))) => {
                if let Some(draft) = self.backend_drafts.get_mut(target.instance_id()) {
                    draft.applied = true;
                }
                if to_file {
                    Said::BackToFile
                } else {
                    Said::ConfigApplied
                }
            }
            (Said::Rereading(Reread::Switch), Some((target, sent, _))) => {
                let switch = UnconfirmedSwitch::new(&target, based_on, sent);
                self.switch = Some(PendingSwitch { switch, over: false });
                Said::Rereading(Reread::Switch)
            }
            (Said::Rereading(Reread::Switch), None) => {
                report::log(
                    CONFIG_THREAD,
                    "an unconfirmed switch of backends whose target this console no longer \
                     holds: judged by the settings' version, as any other change",
                );
                Said::Rereading(Reread::Unconfirmed { based_on })
            }
            (said, _) => said,
        }
    }

    fn draft_mut(&mut self, field: Field) -> Option<&mut Draft> {
        match field {
            Field::TopBar => self.draft.as_mut(),
            Field::EvidenceDays => self.evidence_draft.as_mut(),
            // Each device's own, by its instance (`backends_settled`).
            Field::Backends => None,
            // A rule's change is sent as it is pressed, from no draft.
            Field::Rules(_) => None,
        }
    }

    /// Another owner epoch: no ticket holds across it, and no line says anything of
    /// the old one but, on its own, that a write was out or had not been settled. The
    /// drafts go too: what holds is to be seen afresh.
    pub fn new_epoch(&mut self) {
        let mut said = BTreeMap::new();
        if self.out.contains_key(&GLOBAL_SCHEDULING) {
            said.insert(SaidAt::Scheduling, Said::Restarted);
        }
        if let Some(field) = self.flow.field() {
            said.insert(SaidAt::Settings(field), Said::Restarted);
        }
        // The backends' line names its device while it says the restart.
        if !said.contains_key(&SaidAt::Settings(Field::Backends)) {
            self.backend_said_of = None;
        }
        // So do the rules' lines their account.
        if !said.keys().any(|at| matches!(at, SaidAt::Settings(Field::Rules(_)))) {
            self.rules_target = None;
        }
        self.out.clear();
        self.confirming = None;
        self.flow = ConfigFlow::Idle;
        self.draft = None;
        self.evidence_draft = None;
        self.backend_drafts.clear();
        self.backend_out = None;
        self.switch = None;
        self.said = said;
        self.quiet_form = QuietForm::default();
        self.rules_confirm = None;
        // The snapshots of this epoch are all to come.
        self.rows_behind = false;
        // An account's card says only that its press was out; whatever else it said goes.
        self.accounts.retain(|_, slot| slot.out.is_some());
        for slot in self.accounts.values_mut() {
            *slot = AccountSlot { out: None, said: Some(AccountSaid::Restarted) };
        }
    }

    /// Another account's picture shown, another page gone to, or the rules panel closed: its form
    /// of a quiet time to add goes, whatever it held, and so does the confirmation "delete all"
    /// asked.
    pub fn rules_reset(&mut self) {
        self.quiet_form = QuietForm::default();
        self.rules_confirm = None;
    }

    /// The rules panel shows the account of `instance`, if any, as a `Status` read leaves it: a
    /// form or a confirmation of another account goes, as when another is picked.
    pub fn rules_follow(&mut self, instance: Option<&str>) {
        if self.quiet_form.owner.is_some() && self.quiet_form.owner.as_deref() != instance {
            self.quiet_form = QuietForm::default();
        }
        if self.rules_confirm.is_some() && self.rules_confirm.as_deref() != instance {
            self.rules_confirm = None;
        }
    }

    /// What the last change of `item` of the account of `instance` from its rules panel came to,
    /// if one was pressed this epoch: only while that account is the one the rules' lines speak
    /// of. Once another account's rule is written, the list this one's panel shows is what came
    /// of its own (D-7).
    pub fn rules_said(&self, instance: &str, item: RuleItem) -> Option<&Said> {
        if self.rules_target.as_deref() != Some(instance) {
            return None;
        }
        self.said.get(&SaidAt::Settings(Field::Rules(item)))
    }

    /// The rules' lines speak of the account of `instance` from now on: what they said of
    /// another goes.
    fn rules_speak_of(&mut self, instance: &str) {
        if self.rules_target.as_deref() != Some(instance) {
            self.said.retain(|at, _| !matches!(at, SaidAt::Settings(Field::Rules(_))));
            self.rules_target = Some(instance.to_owned());
        }
    }

    /// The instance id of the account whose "delete all" quiet times asked to confirm.
    pub fn rules_confirm(&self) -> Option<&str> {
        self.rules_confirm.as_deref()
    }

    /// The confirmation "delete all" asked goes: it can no longer be pressed.
    pub fn rules_unconfirm(&mut self) {
        self.rules_confirm = None;
    }

    /// Whether a change of settings was taken and the rows shown are of before it
    /// (`Actions::rows_behind`).
    pub fn rows_behind(&self) -> bool {
        self.rows_behind
    }

    /// The device a switch of backends sent from here may be acting on now, by instance id: the
    /// one out, or the one whose answer was lost while no `Status` has shown it over. Not one
    /// shown over that awaits its snapshot, nor a change that met the settings moved on, which
    /// switched nothing (`Model::device_busy`).
    pub fn switching(&self) -> Option<&str> {
        match &self.flow {
            ConfigFlow::InFlight(Field::Backends) => {
                self.backend_out.as_ref().map(|(target, _, _)| target.instance_id())
            }
            ConfigFlow::Rereading(Reread::Switch, _) => self
                .switch
                .as_ref()
                .filter(|pending| !pending.over)
                .map(|pending| pending.switch.instance_id()),
            _ => None,
        }
    }

    /// A fact snapshot that counts: of this epoch, and read after any write that landed.
    /// That is what a change of settings left waiting was waiting for, and what a draft
    /// that was applied waited for. A confirmation goes when what it asks about is no
    /// longer so. The rows are no longer behind, and a rule's line that its change was taken
    /// goes: the panel shows what holds now.
    pub fn snapshot_read(&mut self, read: &ConfigRead) {
        self.rows_behind = false;
        self.said.retain(|at, said| {
            !(matches!(at, SaidAt::Settings(Field::Rules(_)))
                && matches!(said, Said::ConfigApplied | Said::BackToFile))
        });
        for draft in [&mut self.draft, &mut self.evidence_draft] {
            if draft.as_ref().is_some_and(|draft| draft.applied) {
                *draft = None;
            }
        }
        // A device's draft goes with the switch taken, and with the device's group.
        self.backend_drafts
            .retain(|instance, draft| !draft.applied && read.backends.contains_key(instance));
        match &self.flow {
            ConfigFlow::Rereading(reread, field) => {
                let (reread, field) = (*reread, *field);
                let said = match reread {
                    Reread::Conflict => Said::ConflictRefreshed,
                    Reread::Unconfirmed { based_on } => match (read.version, based_on) {
                        (None, _) => Said::UnconfirmedSeeNow,
                        (Some(now), Some(based_on)) if now == based_on => Said::NotApplied,
                        (Some(_), _) => Said::UnconfirmedChanged,
                    },
                    Reread::Switch => {
                        self.switch_judged(read, field);
                        return;
                    }
                };
                // The settings changed, perhaps by this press: what shows is what holds.
                if said == Said::UnconfirmedChanged {
                    match field {
                        Field::TopBar => self.draft = None,
                        Field::EvidenceDays => self.evidence_draft = None,
                        // Only when the switch's target was lost (`backends_settled`).
                        Field::Backends => {
                            if let Some(instance) = &self.backend_said_of {
                                self.backend_drafts.remove(instance);
                            }
                        }
                        // A time added that may have been taken is not added twice.
                        Field::Rules(RuleItem::Quiet) if self.quiet_form.adding => {
                            self.quiet_form = QuietForm::default();
                        }
                        Field::Rules(_) => {}
                    }
                }
                // Whatever came of it, no time added is out any more.
                if field == Field::Rules(RuleItem::Quiet) {
                    self.quiet_form.adding = false;
                }
                let from = match field {
                    Field::Rules(_) => format!("{field:?} of instance {:?}", self.rules_target),
                    Field::TopBar | Field::EvidenceDays | Field::Backends => format!("{field:?}"),
                };
                log_verdict(reread, &from, read.version, &said);
                self.said.insert(SaidAt::Settings(field), said);
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
            ConfigFlow::Idle | ConfigFlow::InFlight(_) => {}
        }
    }

    /// A snapshot that counts while an unconfirmed switch is awaited: nothing yet while no
    /// `Status` has shown it over, since the settings' version moves only at its end; once one
    /// has, this snapshot was read after it, and what it says of the switch is said, with the
    /// device's self-check as it has it. A switch nobody holds is this program's fault: said in
    /// the log, and the line says to go by what shows.
    fn switch_judged(&mut self, read: &ConfigRead, field: Field) {
        let Some(pending) = &self.switch else {
            report::log(
                CONFIG_THREAD,
                "a switch's verdict was awaited with no switch held: nothing more is awaited",
            );
            self.said.insert(SaidAt::Settings(field), Said::UnconfirmedSeeNow);
            self.flow = ConfigFlow::Idle;
            return;
        };
        if !pending.over {
            return;
        }
        let verdict = pending.switch.verdict(read);
        let instance = pending.switch.instance_id().to_owned();
        // The settings changed, perhaps by this switch: what shows is what holds.
        if verdict.version == SwitchVersion::Changed {
            self.backend_drafts.remove(&instance);
        }
        let line = format!(
            "snapshot read after the unconfirmed switch of instance {instance} was over, \
             version now {:?}: {verdict:?}",
            read.version,
        );
        report::log(CONFIG_THREAD, &line);
        let said = Said::SwitchOver { version: verdict.version, now: verdict.now };
        self.said.insert(SaidAt::Settings(field), said);
        self.switch = None;
        self.flow = ConfigFlow::Idle;
    }

    /// A `Status` read that counts, landed at `now` with these `instances`. A confirmation is
    /// of one pause, and goes with it. A line saying a press worked goes once `Status` says
    /// otherwise: the reads begun before that press were let go, so this says someone changed
    /// it since. Whether it shows an unconfirmed switch over: then the writes are to be counted
    /// once more (`super::wrote`), so that the snapshot that judges it is read after this.
    pub fn status_read(
        &mut self,
        global: Option<&PauseFact>,
        instances: &[InstanceLive],
        now: Instant,
    ) -> bool {
        if self.confirming.as_ref() != global.map(|pause| &pause.target) {
            self.confirming = None;
        }
        let overtaken = match self.said.get(&SaidAt::Scheduling) {
            Some(Said::PauseApplied) => global.is_none(),
            Some(Said::Resumed) => global.is_some(),
            _ => false,
        };
        if overtaken {
            self.said.remove(&SaidAt::Scheduling);
        }
        // So does an account card's, by the account's own pause.
        for (alias, slot) in &mut self.accounts {
            let paused = instances
                .iter()
                .any(|instance| instance.alias == *alias && instance.pause.is_some());
            let overtaken = match slot.said {
                Some(AccountSaid::Paused) => !paused,
                Some(AccountSaid::Resumed { .. }) => paused,
                _ => false,
            };
            if overtaken {
                slot.said = None;
            }
        }
        self.switch_seen(instances, now)
    }

    /// What a `Status` says of the unconfirmed switch awaited, if one is and it may still go
    /// on: whether it shows it over now.
    fn switch_seen(&mut self, instances: &[InstanceLive], now: Instant) -> bool {
        if !matches!(self.flow, ConfigFlow::Rereading(Reread::Switch, _)) {
            return false;
        }
        let Some(pending) = self.switch.as_mut().filter(|pending| !pending.over) else {
            return false;
        };
        let instance = pending.switch.instance_id().to_owned();
        match pending.switch.seen(now, instances) {
            SwitchSeen::MayGoOn => false,
            SwitchSeen::Over => {
                pending.over = true;
                report::log_quiet(
                    CONFIG_THREAD,
                    &format!(
                        "the unconfirmed switch of instance {instance} is over, as a Status \
                         shows: what came of it is read next"
                    ),
                );
                true
            }
            SwitchSeen::Unlisted => {
                pending.over = true;
                report::log(
                    CONFIG_THREAD,
                    &format!(
                        "the Status lists no instance {instance}, whose switch is unconfirmed: \
                         taken as over, in a state this console does not know"
                    ),
                );
                true
            }
        }
    }

    /// No `Status` answers: nothing to confirm against, and no rules panel to press.
    pub fn status_lost(&mut self) {
        self.confirming = None;
        self.rules_confirm = None;
        if matches!(self.flow, ConfigFlow::Confirming { .. }) {
            self.flow = ConfigFlow::Idle;
        }
    }

    /// A press found nothing to send.
    fn preset_gone(&mut self) {
        self.flow = ConfigFlow::Idle;
        self.said.insert(SaidAt::Settings(Field::TopBar), Said::PresetGone);
    }
}

/// What the snapshot awaited after an answer came to, into the log as well: the window's
/// line goes with the next press of its field, the log's stays. Only the settings having
/// moved on, which changed nothing, is no problem. `from` names the field, and a rule's
/// account.
fn log_verdict(reread: Reread, from: &str, version: Option<u64>, said: &Said) {
    let line =
        format!("snapshot read after {reread:?} from {from}, version now {version:?}: {said:?}");
    match said {
        Said::ConflictRefreshed => report::log_quiet(CONFIG_THREAD, &line),
        _ => report::log(CONFIG_THREAD, &line),
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
    /// What the last press came to: of "pause all" or "put all on duty", of a change of
    /// settings from the top bar, and of one from the evidence days' field, each on its own
    /// line.
    pub scheduling_said: Option<Said>,
    pub settings_said: Option<Said>,
    pub evidence_said: Option<Said>,
    /// And of the last switch of a device's backends, on a line of its own that names the
    /// device's account: its alias, `None` when the newest `Status` does not list it.
    pub backends_said: Option<Said>,
    pub backends_account: Option<String>,
    /// Accounts under a pause of their own, which lifting the global one leaves, and
    /// whether any of those is one only whoever set it lifts (`Lifter::Setter`).
    pub still_paused: usize,
    pub setter_holds: bool,
    /// How many accounts still finish a scheduled run. `None` when that cannot be told:
    /// no answer of `Status` carries a version to judge it by, but the activity that is
    /// counted and the settings' version both came in 0.12.36 (R-0b, R-4), so a fact
    /// snapshot of this Runtime with a version says its `Status` names activities, and
    /// a count of none means none.
    pub finishing: Option<usize>,
    /// A snapshot is awaited before anything more is saved, and none can come: the reads
    /// fail, or no Runtime answers (the same epoch answering again reads one). On the line
    /// of the field whose change awaits it. Not while an unconfirmed switch may still go on,
    /// which no snapshot judges yet: its line keeps saying that.
    pub reread_failed: Option<Field>,
}

/// `config_failed`: the fact snapshot reads since the last one that came did not succeed.
/// Not online, nothing records a failed read, so the link says it. `elsewhere`: the pauses
/// the settings file leaves to whoever set them.
pub fn view(
    link: &Link,
    global: Option<&PauseFact>,
    instances: &[InstanceLive],
    actions: &Actions,
    config: Option<&ConfigRead>,
    config_failed: bool,
    elsewhere: &ResumeElsewhere,
) -> ActionsView {
    let online = matches!(link, Link::Online { .. });
    let open = online && !actions.out.contains_key(&GLOBAL_SCHEDULING);
    let resume = match global {
        Some(pause) if open && lifter(pause, elsewhere) == Lifter::Window => {
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
        holding: global.map(|pause| lifter(pause, elsewhere)),
        scheduling_said: actions.said.get(&SaidAt::Scheduling).cloned(),
        settings_said: actions.said.get(&SaidAt::Settings(Field::TopBar)).cloned(),
        evidence_said: actions.said.get(&SaidAt::Settings(Field::EvidenceDays)).cloned(),
        backends_said: actions.said.get(&SaidAt::Settings(Field::Backends)).cloned(),
        backends_account: actions.backend_said_of.as_ref().and_then(|id| {
            instances
                .iter()
                .find(|instance| instance.instance_id == *id)
                .map(|instance| instance.alias.clone())
        }),
        still_paused: paused().count(),
        setter_holds: paused().any(|pause| lifter(pause, elsewhere) == Lifter::Setter),
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
        reread_failed: match actions.flow {
            ConfigFlow::Rereading(Reread::Switch, _)
                if actions.switch.as_ref().is_some_and(|pending| !pending.over) =>
            {
                None
            }
            ConfigFlow::Rereading(_, field) if config_failed || !online => Some(field),
            _ => None,
        },
    }
}

/// An account card's own button now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountEntry {
    /// None: no Runtime answers; the account's own pause is another's, which the card says who
    /// lifts; or it is this console's and still drains or hands the device back, which the
    /// Runtime refuses to lift.
    Off,
    /// "Pause this account", and whether it can be pressed now.
    Pause { offered: bool },
    /// "Resume this account": of the pause this console set there, as the newest `Status`
    /// showed it, and whether it can be pressed now.
    Resume { target: PauseTarget, offered: bool },
}

/// An account card's own button, and what its last press came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountView {
    pub entry: AccountEntry,
    pub said: Option<AccountSaid>,
    /// The newest `Status` shows a global pause, which a lift of the account's own leaves: a
    /// resume said done then says that the account is still not scheduled, as the top bar's
    /// says the pauses that still hold (`resume_all_still_paused`).
    pub global_holds: bool,
}

/// The card's own button of `instance`, an account of the newest `Status` while it answers:
/// with no pause of its own, "pause this account"; under this console's own (its reason, and
/// one only whoever set it lifts by the settings file is not this console's to lift), handed
/// back, "resume this account"; under any other, none. Either can be pressed while no press of
/// that account is out. A fixture is offered the pause too: the Runtime's refusal says why not.
/// `global_holds`: that `Status` shows a global pause.
pub fn account_view(
    instance: &InstanceLive,
    actions: &Actions,
    elsewhere: &ResumeElsewhere,
    global_holds: bool,
) -> AccountView {
    let offered = !actions.account_out(&instance.alias);
    let entry = match &instance.pause {
        None => AccountEntry::Pause { offered },
        Some(pause)
            if pause.reason == CONSOLE_PAUSE_ACCOUNT
                && lifter(pause, elsewhere) == Lifter::Window
                && pause.stage == Some(PauseStage::Released) =>
        {
            AccountEntry::Resume { target: pause.target.clone(), offered }
        }
        Some(_) => AccountEntry::Off,
    };
    AccountView { entry, said: actions.account_said(&instance.alias).cloned(), global_holds }
}

/// An account no `Status` answers about now: no button, and what its last press came to; no
/// global pause known.
pub fn account_down_view(alias: &str, actions: &Actions) -> AccountView {
    AccountView {
        entry: AccountEntry::Off,
        said: actions.account_said(alias).cloned(),
        global_holds: false,
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
        /// How many tasks run at once; `None` when the snapshot does not name it.
        stepper: Option<StepperView>,
    },
}

/// The stepper of how many tasks run at once. It is pressed when a change of settings can
/// go out (`PresetView::Read::can_write`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepperView {
    /// The value in effect, `None` when it has none (no limit), and where it comes from.
    pub value: Option<SettingValue>,
    pub source: SettingSource,
    /// What the field holds instead of the value, as typed or stepped to.
    pub draft: Option<String>,
    /// Whether "apply" would change anything: an empty draft only over an online value,
    /// which it drops, a whole number only when it is not the value in effect. Text that is
    /// no whole number may be pressed, to be told so; a draft already applied may not.
    pub can_apply: bool,
    /// Whether the value is an online one, which going back to the file drops.
    pub back_to_file: bool,
}

fn stepper_view(members: &[PresetMember], draft: Option<&Draft>) -> Option<StepperView> {
    let member =
        members.iter().find(|member| member.setting == PresetSetting::MaxConcurrentRuns)?;
    let online = member.source == SettingSource::Online;
    let value = match member.value {
        Some(SettingValue::Number(runs)) => Some(runs),
        Some(SettingValue::Flag(_) | SettingValue::Word(_)) | None => None,
    };
    Some(StepperView {
        value: member.value.clone(),
        source: member.source,
        draft: draft.map(|draft| draft.text.clone()),
        can_apply: changes_anything(draft, value, online),
        back_to_file: online,
    })
}

/// Whether sending `draft` would change a value `value` from `online` or not: an empty draft
/// only over an online value, which it drops; a whole number only when it is not the value
/// in effect. Text that is no whole number may be pressed, to be told so; a draft already
/// applied may not.
fn changes_anything(draft: Option<&Draft>, value: Option<i64>, online: bool) -> bool {
    draft.is_some_and(|draft| {
        !draft.applied
            && match Typed::of(&draft.text) {
                Typed::Empty => online,
                Typed::Whole(number) => value != Some(number),
                Typed::NotWhole => true,
            }
    })
}

/// The settings page's field of how many days an evidence frame nobody confirmed reading is
/// kept. "Save" goes by the same rules as the stepper's "apply"; both can be pressed when a
/// change of settings can go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceView {
    /// The value in effect, `None` when it has none, and where it comes from.
    pub days: Option<i64>,
    pub source: SettingSource,
    /// What the field holds instead of the value, as typed.
    pub draft: Option<String>,
    /// Whether a change of settings can go out now.
    pub can_write: bool,
    /// Whether "save" would change anything (`changes_anything`).
    pub can_save: bool,
}

/// The evidence days in effect, `days` from `source`, as the field shows them with what the
/// presses left.
pub fn evidence_view(days: Option<i64>, source: SettingSource, actions: &Actions) -> EvidenceView {
    let draft = actions.evidence_draft.as_ref();
    EvidenceView {
        days,
        source,
        draft: draft.map(|draft| draft.text.clone()),
        can_write: actions.flow.open(),
        can_save: changes_anything(draft, days, source == SettingSource::Online),
    }
}

/// What the settings page's group of one device's backends offers now: each box's choices and
/// the one it shows, whether it can be changed, "use the file's value", the pair, the question,
/// "apply", and where the group stands. Its words are `paint`'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendEdit {
    /// `None`: that row does not read, and the group does not show it.
    pub touch: Option<RowEdit<TouchBackend>>,
    pub capture: Option<RowEdit<CaptureBackend>>,
    /// MuMu's own touch is picked over another capture in effect: both change together.
    pub paired: bool,
    /// A capture picked under MuMu's own touch in effect would leave that touch without its
    /// pair: which touch goes with it is asked before both are sent, and "apply" is off.
    pub question: Option<Question>,
    /// What "apply" sends, while it would change anything and can be pressed.
    pub apply: Option<(BackendChange<TouchBackend>, BackendChange<CaptureBackend>)>,
    pub state: GroupState,
}

/// One of a device's two boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowEdit<T> {
    pub choices: Vec<Choice<T>>,
    /// The choice shown: the draft's, else the one in effect.
    pub shown: usize,
    /// The choice of what is in effect, when the box has it.
    in_effect: Option<usize>,
    /// The box shows another than what is in effect.
    pub drafted: bool,
    /// The box can be changed now.
    pub enabled: bool,
    /// "Use the file's value", while the value in effect is an online one.
    pub file_value: Option<UseFileValue>,
}

impl<T: Copy> RowEdit<T> {
    /// What picking the choice at `index` leaves the box's draft: `Some(None)` for what is in
    /// effect, `None` for a choice the box does not have.
    fn pick(&self, index: i32) -> Option<Option<T>> {
        let index = usize::try_from(index).ok()?;
        let choice = self.choices.get(index)?;
        if Some(index) == self.in_effect {
            return Some(None);
        }
        Some(match choice {
            Choice::Backend(backend) => Some(*backend),
            Choice::Word(_) => None,
        })
    }
}

/// One choice of a box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice<T> {
    Backend(T),
    /// What is in effect, a word of the key's range without a name here, as it is written:
    /// picking it keeps it.
    Word(String),
}

impl<T: PartialEq> Choice<T> {
    fn is(&self, now: &InEffect<T>) -> bool {
        match (self, now) {
            (Self::Backend(choice), InEffect::Named(now)) => choice == now,
            (Self::Word(choice), InEffect::Word(now)) => choice == now,
            (Self::Backend(_), InEffect::Word(_)) | (Self::Word(_), InEffect::Named(_)) => false,
        }
    }
}

/// "Use the file's value" of one row: the clear it sends, of that row's key alone, or of both
/// (`both`) when clearing it alone might leave MuMu's own touch without MuMu's own capture, or
/// the file's value under the other online one does not read. Whether it can be pressed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseFileValue {
    pub both: bool,
    pub touch: BackendChange<TouchBackend>,
    pub capture: BackendChange<CaptureBackend>,
    pub enabled: bool,
}

/// The question a capture picked under MuMu's own touch asks: which touch goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Every touch backend of the device but MuMu's own.
    pub choices: Vec<TouchBackend>,
    pub picked: Option<usize>,
    /// What "apply both" sends, once a touch is picked and it can be pressed.
    pub apply: Option<(BackendChange<TouchBackend>, BackendChange<CaptureBackend>)>,
}

/// Where a device's group stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// Its boxes can be changed.
    Open,
    /// Its kind of device does not read: shown only.
    KindUnread,
    /// One of its rows does not read: shown only.
    RowUnread,
    /// Its switch is out.
    Switching,
    /// Its switch's answer was lost: it may still go on, or (`over`) a `Status` showed it over
    /// and the snapshot that tells what came of it is awaited.
    Unconfirmed { over: bool },
    /// The settings had moved on when it was sent: a snapshot is awaited.
    Rereading,
    /// An operation of the device is out from the main page (UI-3e1-a): no switch meanwhile.
    Rescuing,
    /// A refresh of its account's picture is out: no switch meanwhile.
    Refreshing,
    /// Another change of settings is out or awaits its snapshot.
    OtherWrite,
}

/// The group of `backends` as the presses left it. A box offers the backends its kind of
/// device has, and what is in effect first when that is none of them; a box set to what is in
/// effect asks for nothing. MuMu's own touch picked leaves its capture only MuMu's own. "Apply"
/// sends what the boxes ask for, one key or both, unless a capture picked under MuMu's own touch
/// in effect waits for the question's touch. A draft whose switch was taken is not sent again.
/// Nothing is sent while another work of the same device is out from here (`device`,
/// `Model::device_busy`).
pub fn backend_edit(
    backends: &InstanceBackends,
    actions: &Actions,
    device: Option<DeviceBusy>,
) -> BackendEdit {
    let state = group_state(backends, actions, device);
    let open = state == GroupState::Open;
    let draft = actions.backend_drafts.get(backends.instance_id());
    let sendable = open && !draft.is_some_and(|draft| draft.applied);
    let kind = backends.kind.as_ref().ok().copied();
    let touch_offered = kind.map_or_else(Vec::new, DeviceKind::touch_backends);
    let (touch_choices, touch_wanted) = match &backends.touch {
        Some(row) => {
            let choices = choices(touch_offered.clone(), &row.value);
            let wanted = draft
                .and_then(|draft| draft.touch)
                .filter(|touch| changes(&choices, &row.value, *touch));
            (Some(choices), wanted)
        }
        None => (None, None),
    };
    let nemu_picked = touch_wanted == Some(TouchBackend::Nemu);
    let (capture_choices, capture_wanted) = match &backends.capture {
        Some(row) if nemu_picked => {
            let choices = vec![Choice::Backend(CaptureBackend::Nemu)];
            let wanted = Some(CaptureBackend::Nemu)
                .filter(|capture| changes(&choices, &row.value, *capture));
            (Some(choices), wanted)
        }
        Some(row) => {
            let offered = kind.map_or_else(Vec::new, DeviceKind::capture_backends);
            let choices = choices(offered, &row.value);
            let wanted = draft
                .and_then(|draft| draft.capture)
                .filter(|capture| changes(&choices, &row.value, *capture));
            (Some(choices), wanted)
        }
        None => (None, None),
    };
    let touch_change = touch_wanted.map_or(BackendChange::Keep, BackendChange::To);
    let capture_change = capture_wanted.map_or(BackendChange::Keep, BackendChange::To);
    let nemu_touch_now = backends
        .touch
        .as_ref()
        .is_some_and(|row| row.value == InEffect::Named(TouchBackend::Nemu));
    let unpaired = touch_wanted.is_none()
        && nemu_touch_now
        && capture_wanted.is_some_and(|capture| capture != CaptureBackend::Nemu);
    let (touch_file, capture_file) = use_file_value(backends, open);
    let touch = backends.touch.as_ref().zip(touch_choices).map(|(row, choices)| {
        row_edit(choices, &row.value, touch_wanted, open && row.settable, touch_file)
    });
    let capture = backends.capture.as_ref().zip(capture_choices).map(|(row, choices)| {
        row_edit(choices, &row.value, capture_wanted, open && row.settable, capture_file)
    });
    let question = unpaired.then(|| {
        let choices: Vec<TouchBackend> = touch_offered
            .iter()
            .copied()
            .filter(|touch| *touch != TouchBackend::Nemu)
            .collect();
        let asked = draft.and_then(|draft| draft.asked).filter(|asked| choices.contains(asked));
        Question {
            picked: asked.and_then(|asked| choices.iter().position(|touch| *touch == asked)),
            apply: asked
                .filter(|_| sendable)
                .map(|asked| (BackendChange::To(asked), capture_change)),
            choices,
        }
    });
    let anything = touch_wanted.is_some() || capture_wanted.is_some();
    BackendEdit {
        touch,
        capture,
        paired: nemu_picked && capture_wanted == Some(CaptureBackend::Nemu),
        question,
        apply: (sendable && anything && !unpaired).then_some((touch_change, capture_change)),
        state,
    }
}

/// The choices of a box: the backends `offered`, after what is in effect when that is none of
/// them, a word without a name here included.
fn choices<T: Copy + PartialEq>(offered: Vec<T>, now: &InEffect<T>) -> Vec<Choice<T>> {
    let mut choices = Vec::new();
    match now {
        InEffect::Named(backend) if !offered.contains(backend) => {
            choices.push(Choice::Backend(*backend));
        }
        InEffect::Word(word) => choices.push(Choice::Word(word.clone())),
        InEffect::Named(_) => {}
    }
    choices.extend(offered.into_iter().map(Choice::Backend));
    choices
}

/// Whether a box asking for `wanted` changes anything: a backend it offers, not the one in
/// effect.
fn changes<T: Copy + PartialEq>(choices: &[Choice<T>], now: &InEffect<T>, wanted: T) -> bool {
    choices.contains(&Choice::Backend(wanted)) && *now != InEffect::Named(wanted)
}

fn row_edit<T: Copy + PartialEq>(
    choices: Vec<Choice<T>>,
    now: &InEffect<T>,
    wanted: Option<T>,
    enabled: bool,
    file_value: Option<UseFileValue>,
) -> RowEdit<T> {
    let in_effect = choices.iter().position(|choice| choice.is(now));
    let drafted = wanted.and_then(|wanted| {
        choices.iter().position(|choice| *choice == Choice::Backend(wanted))
    });
    RowEdit {
        shown: drafted.or(in_effect).unwrap_or(0),
        drafted: drafted.is_some(),
        choices,
        in_effect,
        enabled,
        file_value,
    }
}

/// Whether "Use the file's value" on a row of the group of `backends` switches the backend whose
/// open `failed`: what the rescue bar asks before it points to it for a fence a switch lifts
/// (#402 HS-5c, `Model::rescue_view`). The touch for the touch's open, the capture for the
/// capture's, either for MuMu's own pair, which is both. Each row's offer is the group's own
/// (`use_file_value`): it clears that row, and the other too when it clears both. A cleared row
/// switches when the file's value under it is not the one in effect, or does not read; a clear
/// back to the same values the Runtime commits at once, with no switch, and the fence stays.
pub fn file_value_switches(backends: &InstanceBackends, failed: &FailedStep) -> bool {
    let (Some(touch), Some(capture)) = (&backends.touch, &backends.capture) else {
        return false;
    };
    let (touch_offer, capture_offer) = use_file_value(backends, true);
    [touch_offer, capture_offer].into_iter().flatten().any(|offer| {
        let touch_switched =
            matches!(offer.touch, BackendChange::BackToFile) && file_value_differs(touch);
        let capture_switched =
            matches!(offer.capture, BackendChange::BackToFile) && file_value_differs(capture);
        match failed {
            FailedStep::TouchOpen => touch_switched,
            FailedStep::CaptureOpen => capture_switched,
            FailedStep::NemuOpen => touch_switched || capture_switched,
            FailedStep::TouchCheck | FailedStep::CaptureCheck | FailedStep::Other(_) => false,
        }
    })
}

/// Whether clearing `row`'s online value brings back another backend than the one in effect:
/// the file's value under it is not that one, or does not read.
fn file_value_differs<T: PartialEq>(row: &BackendRow<T>) -> bool {
    row.online.as_ref().and_then(|online| online.file_value.as_ref()) != Some(&row.value)
}

/// "Use the file's value" of each row whose value is an online one. Clearing only the one
/// online row brings back the file's pair, which actingd checked. With both online, clearing
/// one leaves the other's online value beside the first's file value: when that could leave
/// MuMu's own touch without MuMu's own capture, or the file's value under it does not read,
/// both are cleared.
fn use_file_value(
    backends: &InstanceBackends,
    open: bool,
) -> (Option<UseFileValue>, Option<UseFileValue>) {
    let (Some(touch), Some(capture)) = (&backends.touch, &backends.capture) else {
        return (None, None);
    };
    let touch_online = touch.source == SettingSource::Online;
    let capture_online = capture.source == SettingSource::Online;
    let nemu_touch = InEffect::Named(TouchBackend::Nemu);
    let nemu_capture = InEffect::Named(CaptureBackend::Nemu);
    let touch_file = touch.online.as_ref().and_then(|online| online.file_value.as_ref());
    let capture_file = capture.online.as_ref().and_then(|online| online.file_value.as_ref());
    let touch_alone = !capture_online
        || touch_file.is_some_and(|file| *file != nemu_touch || capture.value == nemu_capture);
    let capture_alone = !touch_online
        || capture_file.is_some_and(|file| touch.value != nemu_touch || *file == nemu_capture);
    let touch_offer = touch_online.then(|| UseFileValue {
        both: !touch_alone,
        touch: BackendChange::BackToFile,
        capture: if touch_alone { BackendChange::Keep } else { BackendChange::BackToFile },
        enabled: open && touch.settable && (touch_alone || capture.settable),
    });
    let capture_offer = capture_online.then(|| UseFileValue {
        both: !capture_alone,
        touch: if capture_alone { BackendChange::Keep } else { BackendChange::BackToFile },
        capture: BackendChange::BackToFile,
        enabled: open && capture.settable && (capture_alone || touch.settable),
    });
    (touch_offer, capture_offer)
}

/// Where the group of `backends` stands: its own switch out or awaited first, then what keeps
/// it shown only, then the device's operation or its picture's refresh out from here
/// (`device`), then another change of settings out. Of a switch, `device` names only this
/// group's own, which the first already said.
fn group_state(
    backends: &InstanceBackends,
    actions: &Actions,
    device: Option<DeviceBusy>,
) -> GroupState {
    let ours = actions.flow.field() == Some(Field::Backends)
        && actions.backend_said_of.as_deref() == Some(backends.instance_id());
    if ours {
        match actions.flow {
            ConfigFlow::InFlight(_) => return GroupState::Switching,
            ConfigFlow::Rereading(Reread::Switch, _) => {
                let over = actions.switch.as_ref().is_some_and(|pending| pending.over);
                return GroupState::Unconfirmed { over };
            }
            ConfigFlow::Rereading(..) => return GroupState::Rereading,
            ConfigFlow::Idle | ConfigFlow::Confirming { .. } => {}
        }
    }
    if backends.kind.is_err() {
        GroupState::KindUnread
    } else if backends.target().is_none() {
        GroupState::RowUnread
    } else if let Some(DeviceBusy::Rescue(_)) = device {
        GroupState::Rescuing
    } else if device == Some(DeviceBusy::Refresh) {
        GroupState::Refreshing
    } else if !actions.flow.open() {
        GroupState::OtherWrite
    } else {
        GroupState::Open
    }
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
        ConfigFlow::InFlight(_) | ConfigFlow::Rereading(..) => (false, None),
    };
    let back_to_file = preset.members.iter().any(|member| member.source == SettingSource::Online);
    let stepper = stepper_view(&preset.members, actions.draft.as_ref());
    PresetView::Read { options, state, can_write, back_to_file, confirming, stepper }
}

/// An account's rules panel now, before its words (`Model::rules_view`). Nothing an older
/// snapshot said of the rules shows while the reads fail, as on the settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesView {
    /// No fact snapshot of this owner epoch has come yet.
    Reading,
    /// The snapshot reads since the last one that came failed. `rereading`: a change of this
    /// account's rules awaits a snapshot that does not come, so what became of it is not known.
    ReadFailed { rereading: bool },
    /// The snapshot that came reports no configuration at all.
    Unreported,
    Read(AccountRules),
}

/// The rules of one account the newest `Status` lists, as the newest snapshot has them and the
/// presses left them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRules {
    /// The account's instance, which every change of its rules names.
    pub instance_id: String,
    /// Why none of them can be changed now, if anything holds them all.
    pub hold: Option<RulesHold>,
    pub quiet: QuietRule,
    /// How many quiet times every account has, of the host's own list, which is only shown
    /// here (Q12): `Err` when the Runtime's parser does not take its entries, why for the log;
    /// `None` when its row does not read, or there is none.
    pub host_quiet: Option<Result<usize, String>>,
    pub idle: IdleRule,
    /// Of an account whose emulator the Runtime starts, which only a discovery-bound MuMu
    /// instance's is (`InstanceLive::emulator_controls`): none of any other, which cannot have it
    /// on.
    pub start: Option<StartRule>,
}

/// Why no rule of an account can be changed now, the first that holds in this order; then each
/// rule's own row says whether the Runtime takes a change of it while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesHold {
    /// A switch of a device's backends is out or awaits its snapshot, which every other change
    /// of settings waits for (HS-U, D-19): its device's account, `None` when the newest
    /// `Status` does not list it.
    Switching { account: Option<String> },
    /// A change of one of this account's own rules is out or awaits its snapshot: its line says
    /// so.
    OwnWrite,
    /// Another change of settings is out or awaits its snapshot: another account's rule, the
    /// top bar's, or the evidence days'.
    OtherWrite,
    /// A change of settings was taken and no snapshot read after it has come (`rows_behind`).
    RowsBehind,
}

/// An account's own quiet times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietRule {
    /// Its list in effect, where it comes from and whether the Runtime takes a change of it; or
    /// that it does not read, which is shown so and never changed (why, for the log once).
    pub read: QuietRead,
    /// A change can be sent now: nothing holds the rules, and the list reads and is settable.
    pub can_write: bool,
    /// "Back to the file's setting", while the list in effect is an online one.
    pub back_to_file: bool,
    /// "Delete all" can be pressed: a change can be sent, and there is a time to delete.
    pub can_clear: bool,
    /// Its first press asked, and it can still be pressed.
    pub confirming: bool,
    /// The form of a time to add, while it is open for this account.
    pub form: Option<QuietFormView>,
    /// What the last change of it from here came to, while this account was the last one
    /// written; one taken only until a snapshot read after it shows the list.
    pub said: Option<Said>,
}

/// The form of a quiet time to add, as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietFormView {
    /// The days it starts on, `days[0]` Monday through `days[6]` Sunday.
    pub days: [bool; 7],
    pub start: String,
    pub end: String,
    /// Why the last "add" sent nothing, until the form is changed.
    pub problem: Option<FormProblem>,
}

/// Why "add" sent nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormProblem {
    /// No day is ticked.
    NoDay,
    /// A time is not hours and minutes of a day.
    NotTime,
    /// The list with it is longer than one value of a setting may be.
    TooBig,
    /// The Runtime's own parser does not take the list with it: most likely more times than a
    /// list holds (the log says why).
    NotTaken,
}

/// An account's idle action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleRule {
    /// The action in effect, where it comes from and whether the Runtime takes a change of it;
    /// `None` when it does not read: shown so, never changed.
    pub now: Option<RuleSetting<IdleNow>>,
    /// A change can be sent now: nothing holds the rules, and it reads and is settable. Either
    /// choice replaces stopping the emulator, which is not one here.
    pub can_write: bool,
    /// "Back to the file's setting", while the action in effect is an online one.
    pub back_to_file: bool,
    /// The account drives no device of its own: the Runtime's idle check passes it over, so an
    /// action does nothing for it (D-12).
    pub no_device: bool,
    /// A pause holds it, its own or every account's: no idle action is taken meanwhile.
    pub paused: bool,
    /// What the last change of it from here came to, as of `QuietRule::said`.
    pub said: Option<Said>,
}

/// Whether the Runtime starts and watches an account's emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRule {
    /// Whether it is on in effect, from where and whether the Runtime takes a change of it;
    /// `None` when it does not read: shown so, never changed.
    pub now: Option<RuleSetting<bool>>,
    /// A change can be sent now: nothing holds the rules, and it reads and is settable.
    pub can_write: bool,
    /// The idle action in effect stops the emulator, which the Runtime takes only with this on:
    /// "off" and "back to the file's setting" are not offered, and the idle action is the
    /// person's to change first (D-11).
    pub held_on: bool,
    /// "Back to the file's setting", while the value in effect is an online one.
    pub back_to_file: bool,
    /// What the last change of it from here came to, as of `QuietRule::said`.
    pub said: Option<Said>,
}

/// The rules of `instance`, an account of `instances`, the newest `Status`'s, as `read` has
/// them unless the reads since failed (`failed`), and as the presses left them; `paused` when a
/// pause holds it, its own or every account's. Nothing holds them but another change of settings
/// out or awaited and rows behind a change taken: not the account's device, its gate, its doing
/// or a pause (v11 B.2 H3).
pub fn rules_view(
    read: Option<&ConfigRead>,
    failed: bool,
    instance: &InstanceLive,
    instances: &[InstanceLive],
    paused: bool,
    actions: &Actions,
) -> RulesView {
    let id = instance.instance_id.as_str();
    let ours = actions.rules_target.as_deref() == Some(id);
    if failed {
        let rereading = ours && matches!(actions.flow, ConfigFlow::Rereading(_, Field::Rules(_)));
        return RulesView::ReadFailed { rereading };
    }
    let Some(read) = read else {
        return RulesView::Reading;
    };
    let Some(settings) = &read.settings else {
        return RulesView::Unreported;
    };
    let hold = rules_hold(ours, instances, actions);
    let open = hold.is_none();
    let said = |item: RuleItem| actions.rules_said(id, item).cloned();
    let quiet_read = settings.quiet(id);
    let (can_write, back_to_file, any) = match &quiet_read {
        QuietRead::Read { list, source, settable } => {
            (open && *settable, *source == SettingSource::Online, !list.is_empty())
        }
        QuietRead::Unread(_) => (false, false, false),
    };
    let can_clear = can_write && any;
    let quiet = QuietRule {
        read: quiet_read,
        can_write,
        back_to_file,
        can_clear,
        confirming: can_clear && actions.rules_confirm.as_deref() == Some(id),
        form: actions.quiet_form.view(id),
        said: said(RuleItem::Quiet),
    };
    let idle_now = settings.idle(id);
    let stops_emulator =
        idle_now.as_ref().is_some_and(|now| now.value == IdleNow::StopEmulator);
    let idle = IdleRule {
        can_write: open && idle_now.as_ref().is_some_and(|now| now.settable),
        back_to_file: idle_now.as_ref().is_some_and(|now| now.source == SettingSource::Online),
        no_device: !instance.physical,
        paused,
        said: said(RuleItem::Idle),
        now: idle_now,
    };
    let start = instance.emulator_controls.start.then(|| {
        let now = settings.start_row(id);
        StartRule {
            can_write: open && now.as_ref().is_some_and(|now| now.settable),
            held_on: stops_emulator,
            back_to_file: now.as_ref().is_some_and(|now| now.source == SettingSource::Online),
            said: said(RuleItem::Start),
            now,
        }
    });
    RulesView::Read(AccountRules {
        instance_id: id.to_owned(),
        hold,
        quiet,
        host_quiet: settings.host_quiet(),
        idle,
        start,
    })
}

/// Why no rule can be changed now (`RulesHold`), `ours` when the rules' lines speak of the
/// account the panel shows. The global ticket of settings is out exactly while the flow is not
/// open, and its field says whose it is.
fn rules_hold(ours: bool, instances: &[InstanceLive], actions: &Actions) -> Option<RulesHold> {
    let hold = match actions.flow.field() {
        Some(Field::Backends) => {
            let account = actions.backend_said_of.as_ref().and_then(|of| {
                instances
                    .iter()
                    .find(|instance| instance.instance_id == *of)
                    .map(|instance| instance.alias.clone())
            });
            RulesHold::Switching { account }
        }
        Some(Field::Rules(_)) if ours => RulesHold::OwnWrite,
        Some(Field::Rules(_) | Field::TopBar | Field::EvidenceDays) => RulesHold::OtherWrite,
        None if actions.rows_behind => RulesHold::RowsBehind,
        None => return None,
    };
    Some(hold)
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
            // What a refusal of a switch says of the backends comes before its class.
            Self::Config(ConfigOutcome::Refused(ConfigRefusal { backend: Some(backend), .. }))
                if write.switches_backends() =>
            {
                Said::BackendRefused(backend)
            }
            Self::Config(ConfigOutcome::Refused(refused)) => match refused.why {
                RefusalWhy::Other => {
                    Said::Refused { write, code: refusal(&refused.failure).to_owned() }
                }
                why => Said::ConfigRefused(why),
            },
            Self::Config(ConfigOutcome::NotSent(_)) => Said::NotSent,
            // A switch may still go on: only a `Status` that shows it over lets a snapshot
            // judge it. The thread's panic too, which brings no answer at all.
            Self::Config(ConfigOutcome::Unconfirmed(_)) | Self::Panicked(_) => match write {
                Write::Configure { field: Field::Backends, .. } => Said::Rereading(Reread::Switch),
                Write::Configure { based_on, .. } => {
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
            Self::Config(ConfigOutcome::Refused(refused)) => {
                let mut line = format!(
                    "{prefix}: refused: {}, key {:?}, {:?}",
                    failure_line(&refused.failure),
                    refused.key,
                    refused.why,
                );
                // The backends' part, the receipt's whole outcome with it; a request that mixed
                // in anything else is this console's own fault.
                if let Some(backend) = &refused.backend {
                    line.push_str(&format!(
                        "; backends: {:?}, not restored {:?}; outcome: {}",
                        backend.why,
                        backend.not_restored,
                        backend.line(),
                    ));
                    if backend.why == Some(BackendWhy::NotAlone) {
                        line.push_str(
                            "; this console's own fault: its switch mixed in another setting or \
                             another instance's",
                        );
                    }
                }
                report::log(write.thread(), &line);
            }
            Self::Config(ConfigOutcome::Unconfirmed(failure)) if write.switches_backends() => {
                report::log(
                    write.thread(),
                    &format!(
                        "{prefix}: switch unconfirmed, not sent again; the device may still be \
                         switching, so a Status is to show it over first: {}",
                        failure_line(failure)
                    ),
                );
            }
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

/// A card's "pause this account": the pause of that account alone, as "pause all" is the
/// global one. The press may land after the read that turned it off.
pub fn account_pause_pressed(ui: &Ui, alias: &str) {
    let entry = ui.model.borrow().account_entry(alias);
    if matches!(entry, Some(AccountEntry::Pause { offered: true })) {
        let instance_alias = alias.to_owned();
        send_account(ui, alias, AccountWrite::Pause, move |root| {
            AccountWritten::Pause(pause_account(root, &instance_alias))
        });
    }
}

/// A card's "resume this account": the conditional lift of exactly the pause the newest
/// `Status` showed there, as "put all on duty" lifts the global one. At one press, with no
/// confirmation: this console set that pause itself (`account_view`), so there is no other
/// tool's pause to ask about, and the condition keeps it from lifting any other.
pub fn account_resume_pressed(ui: &Ui, alias: &str) {
    let entry = ui.model.borrow().account_entry(alias);
    if let Some(AccountEntry::Resume { target, offered: true }) = entry {
        send_account(ui, alias, AccountWrite::Resume, move |root| {
            AccountWritten::Resume(resume_paused(root, &target))
        });
    }
}

/// What the thread of an account card's press hands back.
enum AccountWritten {
    Pause(PauseOutcome),
    Resume(ResumeOutcome),
    /// The write panicked: whether it got out is not known.
    Panicked(String),
    NoThread(String),
}

impl AccountWritten {
    /// As `Written::said` has it of the top bar's pause and resume.
    fn said(self, write: AccountWrite) -> AccountSaid {
        match self {
            Self::Pause(PauseOutcome::Applied { .. }) => AccountSaid::Paused,
            Self::Pause(PauseOutcome::Refused(failure)) => {
                let code = refusal(&failure);
                if code == ALREADY_PAUSED {
                    AccountSaid::AlreadyPaused
                } else {
                    AccountSaid::Refused { write, code: code.to_owned() }
                }
            }
            Self::Resume(ResumeOutcome::Resumed { device_failure, .. }) => {
                AccountSaid::Resumed { device_failure }
            }
            Self::Resume(ResumeOutcome::Refused(failure)) => {
                let code = refusal(&failure);
                if failure.code == RUNTIME_OPERATION_UNSUPPORTED {
                    AccountSaid::Unsupported
                } else if PAUSE_MOVED.contains(&code) {
                    AccountSaid::PauseMoved
                } else {
                    AccountSaid::Refused { write, code: code.to_owned() }
                }
            }
            Self::Pause(PauseOutcome::NotSent(_)) | Self::Resume(ResumeOutcome::NotSent(_)) => {
                AccountSaid::NotSent
            }
            Self::Pause(PauseOutcome::Unconfirmed(_))
            | Self::Resume(ResumeOutcome::Unconfirmed(_))
            | Self::Panicked(_) => AccountSaid::Unconfirmed,
            Self::NoThread(error) => AccountSaid::NoThread { error },
        }
    }

    /// Into the technical log, naming the account, as `Written::log` does the top bar's: what
    /// went as it should and the refusals that only say the state moved on quietly, the rest,
    /// a reconnect that failed after a lift included, as the problems they are.
    fn log(&self, alias: &str, write: AccountWrite, ticket: Ticket) {
        let thread = write.thread();
        let prefix = format!("ticket {} of {alias}", ticket.0);
        match self {
            Self::Pause(PauseOutcome::Applied { revision }) => {
                report::log_quiet(thread, &format!("{prefix}: paused at revision {revision}"));
            }
            Self::Resume(ResumeOutcome::Resumed { revision, device_failure: None }) => {
                report::log_quiet(thread, &format!("{prefix}: resumed at revision {revision}"));
            }
            Self::Resume(ResumeOutcome::Resumed { revision, device_failure: Some(code) }) => {
                report::log(
                    thread,
                    &format!(
                        "{prefix}: resumed at revision {revision}; the device's reconnect \
                         failed: {code}"
                    ),
                );
            }
            Self::Pause(PauseOutcome::Refused(failure))
            | Self::Resume(ResumeOutcome::Refused(failure)) => {
                let code = refusal(failure);
                let line = format!("{prefix}: refused: {}", failure_line(failure));
                if code == ALREADY_PAUSED || PAUSE_MOVED.contains(&code) {
                    report::log_quiet(thread, &line);
                } else {
                    report::log(thread, &line);
                }
            }
            Self::Pause(PauseOutcome::NotSent(failure))
            | Self::Resume(ResumeOutcome::NotSent(failure)) => {
                report::log(thread, &format!("{prefix}: not sent: {}", failure_line(failure)));
            }
            Self::Pause(PauseOutcome::Unconfirmed(failure))
            | Self::Resume(ResumeOutcome::Unconfirmed(failure)) => report::log(
                thread,
                &format!("{prefix}: unconfirmed, not sent again: {}", failure_line(failure)),
            ),
            Self::Panicked(message) => report::log(
                thread,
                &format!("{prefix}: panicked, unconfirmed, not sent again: {message}"),
            ),
            Self::NoThread(error) => {
                report::log(thread, &format!("{prefix}: no thread, not sent: {error}"));
            }
        }
    }
}

/// One press of an account card's own button, once, on a thread of its own, under a ticket of
/// that account's, as `send` sends the top bar's: a thread the system refuses is the press's
/// answer at once, in red, and a panic in the write is an answer as well.
fn send_account(
    ui: &Ui,
    alias: &str,
    write: AccountWrite,
    work: impl FnOnce(&Path) -> AccountWritten + Send + 'static,
) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let Some(ticket) = ui.model.borrow_mut().actions_mut().account_begin(alias, write) else {
        return;
    };
    let root = launcher.state_root.clone();
    let worker_alias = alias.to_owned();
    let spawned = spawn_worker(write.thread(), move || {
        let alias = worker_alias;
        let written = match catch_unwind(AssertUnwindSafe(|| work(&root))) {
            Ok(written) => written,
            Err(panic) => AccountWritten::Panicked(panic_message(&*panic)),
        };
        written.log(&alias, write, ticket);
        report::post(write.thread(), move || {
            if let Some(ui) = ui::current() {
                account_landed(&ui, &alias, write, ticket, written);
            }
        });
    });
    match spawned {
        Ok(()) => paint::cards(ui),
        Err(error) => {
            let written = AccountWritten::NoThread(error.to_string());
            written.log(alias, write, ticket);
            account_landed(ui, alias, write, ticket, written);
        }
    }
}

/// An account's answer, on the event loop, as `landed` takes the top bar's: one whose ticket a
/// restart let go changes nothing shown, and only the log had it.
fn account_landed(
    ui: &Ui,
    alias: &str,
    write: AccountWrite,
    ticket: Ticket,
    written: AccountWritten,
) {
    let said = written.said(write);
    let counted = ui.model.borrow_mut().actions_mut().account_settle(alias, ticket, said);
    if counted {
        super::wrote(ui);
        paint::cards(ui);
    } else {
        report::log_quiet(
            write.thread(),
            &format!(
                "ticket {} of {alias}: its answer came after a restart, and counts for nothing",
                ticket.0
            ),
        );
    }
    ui.paint_log_notes();
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
        send_config(ui, ConfigEdit::preset_back_to_file(), Field::TopBar);
    }
}

/// The stepper's field as the person typed it: the draft, whatever it says.
pub fn concurrency_edited(ui: &Ui, text: &str) {
    ui.model.borrow_mut().actions_mut().draft =
        Some(Draft { text: text.to_owned(), applied: false });
    paint::preset(ui);
}

/// One of the stepper's buttons: one off or one on, from the whole number the field holds,
/// else from the value in effect, else from none. Only the draft changes: there is no range
/// here to stop at.
pub fn concurrency_stepped(ui: &Ui, step: i32) {
    let view = ui.model.borrow().preset_view();
    let PresetView::Read { can_write: true, stepper: Some(stepper), .. } = view else {
        return;
    };
    let typed = stepper.draft.as_deref().map(Typed::of);
    let from = match (typed, stepper.value) {
        (Some(Typed::Whole(runs)), _) | (_, Some(SettingValue::Number(runs))) => runs,
        _ => 0,
    };
    let text = from.saturating_add(i64::from(step)).to_string();
    ui.model.borrow_mut().actions_mut().draft = Some(Draft { text, applied: false });
    paint::preset(ui);
}

/// The stepper's "apply": what the draft asks for, once, or, when it is no whole number,
/// nothing but saying so.
pub fn concurrency_apply(ui: &Ui) {
    let view = ui.model.borrow().preset_view();
    let PresetView::Read { can_write: true, stepper: Some(stepper), .. } = view else {
        return;
    };
    if !stepper.can_apply {
        return;
    }
    let Some(draft) = stepper.draft else {
        return;
    };
    match Typed::of(&draft) {
        Typed::Empty => send_config(ui, ConfigEdit::concurrent_runs_back_to_file(), Field::TopBar),
        Typed::Whole(runs) => send_config(ui, ConfigEdit::concurrent_runs(runs), Field::TopBar),
        Typed::NotWhole => not_whole(ui, Field::TopBar),
    }
}

/// A field's text that is no whole number: nothing is sent, and its line says so.
fn not_whole(ui: &Ui, field: Field) {
    ui.model.borrow_mut().actions_mut().said.insert(SaidAt::Settings(field), Said::NotWhole);
    repaint(ui);
}

/// The stepper's "back to the file's setting": its one setting only.
pub fn concurrency_back_to_file(ui: &Ui) {
    let view = ui.model.borrow().preset_view();
    let offered = matches!(
        view,
        PresetView::Read {
            can_write: true,
            stepper: Some(StepperView { back_to_file: true, .. }),
            ..
        }
    );
    if offered {
        send_config(ui, ConfigEdit::concurrent_runs_back_to_file(), Field::TopBar);
    }
}

/// The evidence days' field as the person typed it: the draft, whatever it says.
pub fn evidence_edited(ui: &Ui, text: &str) {
    ui.model.borrow_mut().actions_mut().evidence_draft =
        Some(Draft { text: text.to_owned(), applied: false });
    paint::settings(ui);
}

/// The evidence days' "save": what the draft asks for, once, as the stepper's "apply" sends
/// its own; an empty draft goes back to the file's value, or the default.
pub fn evidence_save(ui: &Ui) {
    let view = ui.model.borrow().settings_view();
    let SettingsView::Read { evidence: Some(evidence), .. } = view else {
        return;
    };
    if !(evidence.can_write && evidence.can_save) {
        return;
    }
    let Some(draft) = evidence.draft else {
        return;
    };
    match Typed::of(&draft) {
        Typed::Empty => {
            send_config(ui, ConfigEdit::evidence_days_back_to_file(), Field::EvidenceDays);
        }
        Typed::Whole(days) => send_config(ui, ConfigEdit::evidence_days(days), Field::EvidenceDays),
        Typed::NotWhole => not_whole(ui, Field::EvidenceDays),
    }
}

/// The settings page's group of one device's backends as it stands, by its instance id.
fn backend_group(ui: &Ui, instance: &str) -> Option<BackendGroup> {
    let view = ui.model.borrow().settings_view();
    let SettingsView::Read { backends, .. } = view else {
        return None;
    };
    backends.into_iter().find(|group| group.backends.instance_id() == instance)
}

/// A device's touch box set to the choice at `index`: its draft, whatever it was. A question a
/// capture asked goes with it: the touch picked here answers it. A pick on a box that is not
/// open changes nothing, and the box goes back to what it shows; one of a choice the box does
/// not have is this program's fault as well, in the log.
pub fn backend_touch_picked(ui: &Ui, instance: &str, index: i32) {
    let picked = backend_group(ui, instance)
        .and_then(|group| group.edit.touch)
        .filter(|row| row.enabled)
        .map(|row| row.pick(index));
    match picked {
        Some(Some(touch)) => backend_draft(ui, instance, |draft| {
            draft.touch = touch;
            draft.asked = None;
        }),
        Some(None) => unoffered(instance, "touch", index),
        None => {}
    }
    paint::settings(ui);
}

/// The same of a device's capture box. A question it asked before goes with it.
pub fn backend_capture_picked(ui: &Ui, instance: &str, index: i32) {
    let picked = backend_group(ui, instance)
        .and_then(|group| group.edit.capture)
        .filter(|row| row.enabled)
        .map(|row| row.pick(index));
    match picked {
        Some(Some(capture)) => backend_draft(ui, instance, |draft| {
            draft.capture = capture;
            draft.asked = None;
        }),
        Some(None) => unoffered(instance, "capture", index),
        None => {}
    }
    paint::settings(ui);
}

/// The question's box set to the choice at `index`, the first being "pick one".
pub fn backend_asked(ui: &Ui, instance: &str, index: i32) {
    let picked = backend_group(ui, instance)
        .filter(|group| group.edit.state == GroupState::Open)
        .and_then(|group| group.edit.question)
        .map(|question| match usize::try_from(index) {
            Ok(0) => Some(None),
            Ok(at) => question.choices.get(at - 1).copied().map(Some),
            Err(_) => None,
        });
    match picked {
        Some(Some(asked)) => backend_draft(ui, instance, |draft| draft.asked = asked),
        Some(None) => unoffered(instance, "question's touch", index),
        None => {}
    }
    paint::settings(ui);
}

/// "Apply": what the boxes ask for, once.
pub fn backend_apply(ui: &Ui, instance: &str) {
    let Some(group) = backend_group(ui, instance) else {
        return;
    };
    if let Some((touch, capture)) = group.edit.apply {
        send_backends(ui, &group.backends, touch, capture);
    }
}

/// "Apply both": the question's touch with the capture that asked it, once.
pub fn backend_apply_both(ui: &Ui, instance: &str) {
    let Some(group) = backend_group(ui, instance) else {
        return;
    };
    if let Some((touch, capture)) = group.edit.question.and_then(|question| question.apply) {
        send_backends(ui, &group.backends, touch, capture);
    }
}

/// "Cancel" on the question: the device's draft goes, and its boxes show what is in effect.
pub fn backend_cancel(ui: &Ui, instance: &str) {
    ui.model.borrow_mut().actions_mut().backend_drafts.remove(instance);
    paint::settings(ui);
}

/// "Use the file's value" of the touch row, or of the capture row: its clear, once, and the
/// other's with it when the offer says both.
pub fn backend_file_value(ui: &Ui, instance: &str, touch_row: bool) {
    let Some(group) = backend_group(ui, instance) else {
        return;
    };
    let offer = if touch_row {
        group.edit.touch.and_then(|row| row.file_value)
    } else {
        group.edit.capture.and_then(|row| row.file_value)
    };
    if let Some(offer) = offer.filter(|offer| offer.enabled) {
        send_backends(ui, &group.backends, offer.touch, offer.capture);
    }
}

/// A device's draft, changed; one that asks for nothing goes. Whatever it asks for now was not
/// taken yet.
fn backend_draft(ui: &Ui, instance: &str, change: impl FnOnce(&mut BackendDraft)) {
    let mut model = ui.model.borrow_mut();
    let drafts = &mut model.actions_mut().backend_drafts;
    let draft = drafts.entry(instance.to_owned()).or_default();
    change(draft);
    draft.applied = false;
    let empty = draft.is_empty();
    if empty {
        drafts.remove(instance);
    }
}

/// A box's pick of a choice it does not have: the box went out of step with what it shows.
fn unoffered(instance: &str, which: &str, index: i32) {
    let line =
        format!("instance {instance}: the {which} box picked choice {index}, which it has not");
    report::log(SETTINGS_LOG, &line);
}

/// One switch of the device `backends` reads: what is sent goes in the log first, keys and
/// all, as any change of settings (`send_config`). The device and when it was pressed are kept
/// for the answer: an answer that says nothing is judged as that switch's. So is whether it only
/// goes back to the file's values, which a change taken is said as.
fn send_backends(
    ui: &Ui,
    backends: &InstanceBackends,
    touch: BackendChange<TouchBackend>,
    capture: BackendChange<CaptureBackend>,
) {
    let instance = backends.instance_id();
    let Some(target) = backends.target() else {
        let line = format!("instance {instance}: no switch of its backends can be sent");
        report::log(CONFIG_THREAD, &line);
        return;
    };
    let Some(edit) = ConfigEdit::backends(&target, touch, capture) else {
        let line = format!("instance {instance}: a switch that changes nothing; not sent");
        report::log(CONFIG_THREAD, &line);
        return;
    };
    let to_file =
        !matches!(touch, BackendChange::To(_)) && !matches!(capture, BackendChange::To(_));
    {
        let mut model = ui.model.borrow_mut();
        let actions = model.actions_mut();
        actions.backend_said_of = Some(instance.to_owned());
        actions.backend_out = Some((target, Instant::now(), to_file));
    }
    send_config(ui, edit, Field::Backends);
}

/// The rescue bar's "auto rules": the rules panel opens in the picture's place, or closes, which
/// lets go of its form and its confirmation asked. Open, it stays so across accounts and pages.
pub fn rules_toggled(ui: &Ui) {
    ui.model.borrow_mut().rules_toggled();
    paint::rescue(ui);
}

/// The rules of the account whose picture shows, as its open panel offers them now: none while
/// the panel is closed or has no rules to change. Every press goes by this, recomputed, so one
/// that lands after its button went off sends nothing.
fn rules_shown(ui: &Ui) -> Option<AccountRules> {
    let model = ui.model.borrow();
    if !model.rules_open() {
        return None;
    }
    let alias = model.picture_account()?;
    match model.rules_view(alias)? {
        RulesView::Read(rules) => Some(rules),
        RulesView::Reading | RulesView::ReadFailed { .. } | RulesView::Unreported => None,
    }
}

/// A press on the rules panel other than the confirmation's: that confirmation goes first,
/// whatever this press comes to. Then the rules as `rules_shown` has them.
fn rules_pressed(ui: &Ui) -> Option<AccountRules> {
    ui.model.borrow_mut().actions_mut().rules_confirm = None;
    rules_shown(ui)
}

/// "Add a time…": the form of a quiet time to add opens for the account shown, while its list
/// reads and the Runtime takes a change of it: no day ticked, and its fields holding
/// `QUIET_FORM_START` and `QUIET_FORM_END`. One open for it already stays as it is; one closed by
/// a time added opens so again.
pub fn quiet_form_opened(ui: &Ui) {
    let rules = rules_pressed(ui);
    if let Some(rules) = rules.filter(|rules| {
        matches!(rules.quiet.read, QuietRead::Read { settable: true, .. })
    }) {
        let mut model = ui.model.borrow_mut();
        let form = &mut model.actions_mut().quiet_form;
        if form.owner.as_deref() != Some(rules.instance_id.as_str()) {
            *form = QuietForm {
                owner: Some(rules.instance_id),
                start: QUIET_FORM_START.to_owned(),
                end: QUIET_FORM_END.to_owned(),
                ..QuietForm::default()
            };
        }
    }
    paint::rescue(ui);
}

/// The form's box of the day at `day`, `0` Monday through `6` Sunday, ticked or not. One of a day
/// the form has not is this program's fault, in the log.
pub fn quiet_day_toggled(ui: &Ui, day: i32, on: bool) {
    let Some(day) = usize::try_from(day).ok().filter(|day| *day < 7) else {
        let line = format!("the quiet time form's box of day {day} was ticked, which it has not");
        report::log(RULES_LOG, &line);
        paint::rescue(ui);
        ui.paint_log_notes();
        return;
    };
    quiet_form_changed(ui, |form| form.days[day] = on);
}

/// The form's start as the person typed it, whatever it says.
pub fn quiet_start_edited(ui: &Ui, text: &str) {
    quiet_form_changed(ui, |form| form.start = text.to_owned());
}

/// And its end.
pub fn quiet_end_edited(ui: &Ui, text: &str) {
    quiet_form_changed(ui, |form| form.end = text.to_owned());
}

/// The form open for the account shown, changed: what it says is not checked yet, so the problem
/// the last "add" found goes. A change of a form not open for that account changes nothing, and
/// the panel goes back to what it shows.
fn quiet_form_changed(ui: &Ui, change: impl FnOnce(&mut QuietForm)) {
    if let Some(rules) = rules_pressed(ui) {
        let mut model = ui.model.borrow_mut();
        let form = &mut model.actions_mut().quiet_form;
        if form.owner.as_deref() == Some(rules.instance_id.as_str()) {
            change(&mut *form);
            form.problem = None;
        }
    }
    paint::rescue(ui);
}

/// The form's "add": the account's list with a weekly time of the days ticked, between the two
/// times typed of this computer's clock, sent once, as a whole list (`ConfigEdit::quiet_times`).
/// No day ticked, a time that is not hours and minutes, a list too long, or one the Runtime's
/// parser does not take, sends nothing, and the form says why.
pub fn quiet_add(ui: &Ui) {
    let Some(rules) = rules_pressed(ui) else {
        paint::rescue(ui);
        return;
    };
    let id = rules.instance_id.as_str();
    let (QuietRead::Read { list, .. }, true, Some(form)) =
        (&rules.quiet.read, rules.quiet.can_write, rules.quiet.form.as_ref())
    else {
        paint::rescue(ui);
        return;
    };
    let new = match (form.days.contains(&true), minute_of(&form.start), minute_of(&form.end)) {
        (false, _, _) => return quiet_problem(ui, id, FormProblem::NoDay),
        (true, Some(start_minute), Some(end_minute)) => {
            NewWeekly { days: form.days, start_minute, end_minute }
        }
        (true, _, _) => return quiet_problem(ui, id, FormProblem::NotTime),
    };
    let Some(list) = list.with_weekly(&new, local_utc_offset_minutes(), paint::now_ms()) else {
        return quiet_problem(ui, id, FormProblem::NoDay);
    };
    match ConfigEdit::quiet_times(&list) {
        Ok(edit) => {
            {
                let mut model = ui.model.borrow_mut();
                let form = &mut model.actions_mut().quiet_form;
                form.problem = None;
                form.adding = true;
            }
            send_rules(ui, id, RuleItem::Quiet, edit);
        }
        Err(QuietNotSent::TooBig) => quiet_problem(ui, id, FormProblem::TooBig),
        Err(QuietNotSent::NotTaken(why)) => {
            let line =
                format!("instance {id}: the quiet times with one added were not sent: {why}");
            report::log(CONFIG_THREAD, &line);
            quiet_problem(ui, id, FormProblem::NotTaken);
            ui.paint_log_notes();
        }
    }
}

/// What "add" found in the form open for `instance`: nothing was sent.
fn quiet_problem(ui: &Ui, instance: &str, problem: FormProblem) {
    {
        let mut model = ui.model.borrow_mut();
        let form = &mut model.actions_mut().quiet_form;
        if form.owner.as_deref() == Some(instance) {
            form.problem = Some(problem);
        }
    }
    paint::rescue(ui);
}

/// A quiet time's "delete": the account's list without the one at `index`, every other as it
/// is, sent once as a whole list. A list still too long for one value is said on the quiet
/// times' line, and not sent. One of an entry the list has not is this program's fault, in the
/// log.
pub fn quiet_delete(ui: &Ui, index: i32) {
    let Some(rules) = rules_pressed(ui) else {
        paint::rescue(ui);
        return;
    };
    let id = rules.instance_id.as_str();
    let (QuietRead::Read { list, .. }, true) = (&rules.quiet.read, rules.quiet.can_write) else {
        paint::rescue(ui);
        return;
    };
    let Some(list) = usize::try_from(index).ok().and_then(|index| list.without(index)) else {
        let line =
            format!("instance {id}: delete pressed on quiet time {index}, which its list has not");
        report::log(RULES_LOG, &line);
        paint::rescue(ui);
        ui.paint_log_notes();
        return;
    };
    match ConfigEdit::quiet_times(&list) {
        Ok(edit) => send_rules(ui, id, RuleItem::Quiet, edit),
        // The list read may be the file's, which the bound on one online value does not hold,
        // and each entry goes back as the parser writes it, which may be longer than as it was
        // read: one entry fewer may still be too long. That is said, as "add"'s form says it,
        // and nothing goes out.
        Err(QuietNotSent::TooBig) => {
            {
                let mut model = ui.model.borrow_mut();
                let actions = model.actions_mut();
                actions.rules_speak_of(id);
                let at = SaidAt::Settings(Field::Rules(RuleItem::Quiet));
                actions.said.insert(at, Said::QuietTooBig);
            }
            paint::rescue(ui);
        }
        // One entry fewer of a list the Runtime's parser took: that it does not take it is this
        // console's fault, in the log, and nothing goes out.
        Err(QuietNotSent::NotTaken(why)) => {
            let line =
                format!("instance {id}: the quiet times with one deleted were not sent: {why}");
            report::log(CONFIG_THREAD, &line);
            paint::rescue(ui);
            ui.paint_log_notes();
        }
    }
}

/// "Delete all": the first of two presses only asks.
pub fn quiet_clear(ui: &Ui) {
    if let Some(rules) = rules_pressed(ui).filter(|rules| rules.quiet.can_clear) {
        ui.model.borrow_mut().actions_mut().rules_confirm = Some(rules.instance_id);
    }
    paint::rescue(ui);
}

/// The confirming press: the account's list set online to none at all, an empty list whatever
/// the file names, sent once, if it can still be pressed; the question goes either way.
pub fn quiet_clear_confirmed(ui: &Ui) {
    let rules = rules_shown(ui);
    ui.model.borrow_mut().actions_mut().rules_confirm = None;
    match rules.filter(|rules| rules.quiet.confirming) {
        Some(rules) => {
            let id = rules.instance_id.as_str();
            send_rules(ui, id, RuleItem::Quiet, ConfigEdit::quiet_times_none(id));
        }
        None => paint::rescue(ui),
    }
}

/// The confirmation's "cancel": the question goes, and nothing is sent.
pub fn quiet_clear_cancelled(ui: &Ui) {
    ui.model.borrow_mut().actions_mut().rules_confirm = None;
    paint::rescue(ui);
}

/// The quiet times' "back to the file's setting": the online list is dropped, once, which leaves
/// none of the account's own when the file names none.
pub fn quiet_back_to_file(ui: &Ui) {
    match rules_pressed(ui).filter(|rules| rules.quiet.can_write && rules.quiet.back_to_file) {
        Some(rules) => {
            let id = rules.instance_id.as_str();
            send_rules(ui, id, RuleItem::Quiet, ConfigEdit::quiet_times_back_to_file(id));
        }
        None => paint::rescue(ui),
    }
}

/// One of the idle action's two choices, once, unless it is the one in effect. Stopping the
/// emulator in effect is replaced by either.
pub fn idle_picked(ui: &Ui, choice: IdleChoice) {
    let picked = rules_pressed(ui).filter(|rules| {
        let idle = &rules.idle;
        idle.can_write && !idle.now.as_ref().is_some_and(|now| is_choice(&now.value, choice))
    });
    match picked {
        Some(rules) => {
            let id = rules.instance_id.as_str();
            send_rules(ui, id, RuleItem::Idle, ConfigEdit::idle_action(id, choice));
        }
        None => paint::rescue(ui),
    }
}

/// Whether the idle action in effect is `choice`.
fn is_choice(now: &IdleNow, choice: IdleChoice) -> bool {
    matches!(
        (now, choice),
        (IdleNow::Nothing, IdleChoice::Nothing) | (IdleNow::CloseGame, IdleChoice::CloseGame)
    )
}

/// The idle action's "back to the file's setting", once: the file's, or the section's when the
/// file names none of the account's own, whatever that is.
pub fn idle_back_to_file(ui: &Ui) {
    match rules_pressed(ui).filter(|rules| rules.idle.can_write && rules.idle.back_to_file) {
        Some(rules) => {
            let id = rules.instance_id.as_str();
            send_rules(ui, id, RuleItem::Idle, ConfigEdit::idle_action_back_to_file(id));
        }
        None => paint::rescue(ui),
    }
}

/// The emulator's start and watch turned on or off, once, unless it is so in effect; never off
/// while the idle action in effect stops the emulator (D-11).
pub fn start_picked(ui: &Ui, on: bool) {
    let picked = rules_pressed(ui).filter(|rules| {
        rules.start.as_ref().is_some_and(|start| {
            start.can_write
                && start.now.as_ref().map(|now| now.value) != Some(on)
                && (on || !start.held_on)
        })
    });
    match picked {
        Some(rules) => {
            let id = rules.instance_id.as_str();
            send_rules(ui, id, RuleItem::Start, ConfigEdit::start_emulator(id, on));
        }
        None => paint::rescue(ui),
    }
}

/// Its "back to the file's setting", once: the file's value, or off when the file names none;
/// not while the idle action in effect stops the emulator, which the file's value might not
/// leave on (D-11).
pub fn start_back_to_file(ui: &Ui) {
    let picked = rules_pressed(ui).filter(|rules| {
        rules
            .start
            .as_ref()
            .is_some_and(|start| start.can_write && start.back_to_file && !start.held_on)
    });
    match picked {
        Some(rules) => {
            let id = rules.instance_id.as_str();
            send_rules(ui, id, RuleItem::Start, ConfigEdit::start_emulator_back_to_file(id));
        }
        None => paint::rescue(ui),
    }
}

/// One change of a rule of the account `instance`, as any change of settings (`send_config`):
/// based on the version of the snapshot its press read the rows of, the same one, since no
/// snapshot lands between that press and this on the event loop. The rules' lines speak of this
/// account from now on: what they said of another goes.
fn send_rules(ui: &Ui, instance: &str, item: RuleItem, edit: ConfigEdit) {
    ui.model.borrow_mut().actions_mut().rules_speak_of(instance);
    send_config(ui, edit, Field::Rules(item));
}

/// A time of day as a person types it, hours and minutes, `23:00` or `7:05` (a full-width colon
/// as well), as its minute of the day; `None` for anything else.
fn minute_of(text: &str) -> Option<u16> {
    let text = text.trim();
    let (hours, minutes) = text.split_once(':').or_else(|| text.split_once('：'))?;
    let hours = clock_digits(hours.trim(), 1)?;
    let minutes = clock_digits(minutes.trim(), 2)?;
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// `part` as a number of one or two ASCII digits, at least `least` of them.
fn clock_digits(part: &str, least: usize) -> Option<u16> {
    let width = part.len() >= least && part.len() <= 2;
    if width && part.bytes().all(|byte| byte.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

/// The table is read once at the Runtime's start, so a preset shown is in it: a preset
/// that is not says this program lost track, and nothing goes out.
fn send_preset(ui: &Ui, name: &str) {
    let edit =
        ui.model.borrow().config_read().and_then(|read| ConfigEdit::apply_preset(read, name));
    match edit {
        Some(edit) => send_config(ui, edit, Field::TopBar),
        None => {
            report::log(CONFIG_THREAD, &format!("preset {name} is not in the table read"));
            ui.model.borrow_mut().actions_mut().preset_gone();
            repaint(ui);
            ui.paint_log_notes();
        }
    }
}

/// What it sends goes in the log first, keys and all: the answer's line does not say.
fn send_config(ui: &Ui, edit: ConfigEdit, field: Field) {
    let based_on = ui.model.borrow().config_version();
    send(ui, Write::Configure { based_on, field }, move |root| {
        report::log_quiet(
            CONFIG_THREAD,
            &format!("sending, based on version {based_on:?}: {}", edit.line()),
        );
        Written::Config(configure(root, &edit, based_on))
    });
}

/// What a press or an answer may have changed: both rows of the top bar's writes, what came
/// of them, the settings page's field, which a change of settings out turns off, and the rescue
/// bar with its rules panel, which any change of settings out holds as well.
fn repaint(ui: &Ui) {
    paint::actions(ui);
    paint::preset(ui);
    paint::settings(ui);
    paint::rescue(ui);
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
        let counted = model.actions_mut().settle(write, ticket, said);
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
