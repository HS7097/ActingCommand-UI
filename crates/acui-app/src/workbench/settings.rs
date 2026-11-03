// SPDX-License-Identifier: GPL-3.0-only
//! The settings page's rows of the Runtime's settings: which show, in which group, and how
//! each is changed, as a pure function of what the reads left; and which words of the table
//! each part takes. `paint` puts them together and into the window.
//!
//! Which settings show, and in which group, is the Runtime's own (its tiers of the keys,
//! read in `acui-source`): this page keeps no list of keys. A key the words here have no
//! name for still shows, as an unnamed setting, and its name goes to the log. A key the
//! Runtime has no value of, which a key without a default has while the file does not name
//! it, shows as unset rather than not at all.
//!
//! Only an answer of the Runtime online counts: while none comes, none of its settings
//! shows, since an older read would pose as the present.
//!
//! One row's setting is changed here: how many days an evidence frame nobody confirmed reading
//! is kept, the one the Runtime tiers advanced and takes while it runs. Its row is a field and
//! "save" (`actions::evidence_view`), in its place among the others. Every other row says how
//! its value is changed: in the configuration file with a restart, by the installer, or not on
//! this page for now. Each device's backends are changed in its group, below the rows.
//!
//! And what the start report of this owner epoch says of the settings changed while a Runtime
//! ran: which this start did not restore, and which it kept over a default that changed.
//!
//! And, under the Runtime, whether it is started again when it is gone without a formal close,
//! as the last run of `watchdog status` has it (`tools`): only shown, since registering the
//! task is an installation's step and no setting of the Runtime.
//!
//! And after the advanced rows, a group for each device the newest `Status` lists whose
//! backends the snapshot names (HS-U): its touch and capture backends, each in a box of the
//! backends that kind of device has, with where its value comes from, and the device's newest
//! self-check by the entries its touch backend opens. A fixture names no backend, nor does a
//! Runtime before 0.12.39: no group then, and that is no error. How a group is changed is
//! `actions::backend_edit`'s.

use acui_source::workbench::backends::{
    CaptureBackend, CheckStatus, EntryCheck, InEffect, InstanceBackends, Through, TouchBackend,
};
use acui_source::workbench::{
    ConfigRead, InstanceLive, ReportedName, ReportedSetting, SettingName, SettingSource,
    SettingTier, ShownValue, StartRead, UnreadRow, IDLE_CLOSE_APP, IDLE_NONE, IDLE_STOP_EMULATOR,
    PRIORITY_LOW, PRIORITY_NORMAL,
};

use super::actions::{self, Actions, BackendEdit, EvidenceView};
use super::model::Link;
use super::tools::{TaskState, WatchdogRead};
use crate::install::ToolIssue;
use crate::strings::{fill, Labels};

/// The settings page's rows of the Runtime's settings now, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsView {
    /// No Runtime runs.
    NotRunning,
    /// None can be read now: it does not answer, no thread reads it, or this run has no
    /// state root (`paint`'s to tell).
    Unreachable,
    /// Nothing has come back yet, or the Runtime answers and no fact snapshot of it has.
    Reading,
    /// The fact snapshot reads since the last that came failed.
    Failed,
    /// The snapshot that came reports no configuration at all.
    Unreported,
    Read {
        daily: Vec<SettingLine>,
        /// The advanced rows before the field of the evidence days, when it shows, and
        /// those after it; all of them otherwise.
        advanced: Vec<SettingLine>,
        evidence: Option<EvidenceView>,
        advanced_after: Vec<SettingLine>,
        /// Each device's backends, in the order of the newest `Status`.
        backends: Vec<BackendGroup>,
        /// The instance ids of devices whose backends the snapshot names and the newest
        /// `Status` does not list: not shown, and said in the log.
        unlisted: Vec<String>,
        /// Rows of the snapshot that may be of a setting shown here and do not read: the
        /// window says how many, the log which and why.
        unread: Vec<UnreadRow>,
    },
}

/// One device's group of its backends: its account's alias, what the snapshot says of them,
/// and how it can be changed now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendGroup {
    pub account: String,
    pub backends: InstanceBackends,
    pub edit: BackendEdit,
}

/// Which of a device's self-check entries a group says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckEntry {
    Touch,
    Capture,
    /// MuMu's own pair, which opens both at once.
    Nemu,
}

/// A device's newest self-check, of the entries its touch backend in effect opens: MuMu's own
/// pair under MuMu's own touch, else the touch and the capture backends' own. An entry another
/// configuration left is not said. When the newest of them was checked, if any says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfCheckView {
    pub entries: Vec<(CheckEntry, CheckStatus)>,
    pub at: Option<u64>,
}

pub fn selfcheck_of(backends: &InstanceBackends) -> SelfCheckView {
    let check = &backends.selfcheck;
    let nemu = backends
        .touch
        .as_ref()
        .is_some_and(|row| row.value == InEffect::Named(TouchBackend::Nemu));
    let entries: Vec<(CheckEntry, &EntryCheck)> = if nemu {
        check.nemu.iter().map(|entry| (CheckEntry::Nemu, entry)).collect()
    } else {
        let touch = check.input.iter().map(|entry| (CheckEntry::Touch, entry));
        touch.chain(check.capture.iter().map(|entry| (CheckEntry::Capture, entry))).collect()
    };
    SelfCheckView {
        at: entries.iter().filter_map(|(_, entry)| entry.checked_at).max(),
        entries: entries.into_iter().map(|(which, entry)| (which, entry.status.clone())).collect(),
    }
}

/// One row of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingLine {
    pub name: SettingName,
    /// The account of an instance's own setting: its alias, or its instance id when the
    /// newest `Status` does not list it.
    pub account: Option<String>,
    pub value: ShownValue,
    pub source: SettingSource,
    pub change: Change,
}

/// How a value is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The Runtime takes a change while it runs, and this page offers none for now.
    ViewOnly,
    /// Only the configuration file changes it, read when the Runtime starts.
    FileAndRestart,
    /// The installer sets it.
    Installer,
}

/// `read` of the Runtime `link` names, unless the reads since failed (`failed`); its
/// instances' own settings by the accounts `instances` lists, in that order. The evidence
/// days are a field while the Runtime takes a change of them and tiers them advanced, with
/// what `actions` left of it; otherwise a row as any other.
pub fn view(
    link: &Link,
    read: Option<&ConfigRead>,
    failed: bool,
    instances: &[InstanceLive],
    actions: &Actions,
) -> SettingsView {
    match link {
        Link::Online { .. } => {}
        Link::Unknown => return SettingsView::Reading,
        Link::Absent(_) => return SettingsView::NotRunning,
        Link::NoAnswer(_) | Link::Trouble(_) | Link::ThreadFailed => {
            return SettingsView::Unreachable
        }
    }
    if failed {
        return SettingsView::Failed;
    }
    let Some(read) = read else {
        return SettingsView::Reading;
    };
    let Some(settings) = &read.settings else {
        return SettingsView::Unreported;
    };
    let ids: Vec<&str> = instances.iter().map(|instance| instance.instance_id.as_str()).collect();
    let (mut daily, mut advanced, mut advanced_after) = (Vec::new(), Vec::new(), Vec::new());
    let mut evidence = None;
    for row in settings.rows(&ids) {
        let days = match row.value {
            ShownValue::Number(days) => Some(Some(days)),
            ShownValue::Unset => Some(None),
            ShownValue::Set | ShownValue::Flag(_) | ShownValue::Word(_) => None,
        };
        let editable = row.name == SettingName::EvidenceMaxAgeDays
            && row.tier == SettingTier::Advanced
            && row.settable
            && evidence.is_none();
        if let (true, Some(days)) = (editable, days) {
            evidence = Some(actions::evidence_view(days, row.source, actions));
            continue;
        }
        let account = row.instance.map(|id| account_of(instances, id));
        let change = change_of(&row.name, row.settable);
        let line = SettingLine {
            name: row.name,
            account,
            value: row.value,
            source: row.source,
            change,
        };
        match row.tier {
            SettingTier::Daily => daily.push(line),
            SettingTier::Advanced if evidence.is_some() => advanced_after.push(line),
            SettingTier::Advanced => advanced.push(line),
        }
    }
    let backends = instances
        .iter()
        .filter_map(|instance| {
            let backends = read.backends.get(&instance.instance_id)?;
            Some(BackendGroup {
                account: instance.alias.clone(),
                backends: backends.clone(),
                edit: actions::backend_edit(backends, actions),
            })
        })
        .collect();
    let unlisted = read.backends.keys().filter(|id| !ids.contains(&id.as_str())).cloned().collect();
    let unread = settings.unread().to_vec();
    SettingsView::Read { daily, advanced, evidence, advanced_after, backends, unlisted, unread }
}

/// The account of an instance's own setting: its alias, or its instance id when the newest
/// `Status` does not list it.
fn account_of(instances: &[InstanceLive], id: String) -> String {
    instances
        .iter()
        .find(|instance| instance.instance_id == id)
        .map_or(id, |instance| instance.alias.clone())
}

/// What this owner epoch's start report says of the settings changed while a Runtime ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartView {
    /// Nothing to say: no Runtime answers, or none has come yet.
    Nothing,
    /// Every try of this epoch failed: whether any setting was reset is not known.
    Unreadable,
    Read {
        /// Not restored: an update or a rollback reset them.
        dropped: Vec<ReportedLine>,
        /// Kept over a default that changed.
        base_changed: Vec<ReportedLine>,
        /// The notice of the ones reset was read; the settings page still says them.
        acknowledged: bool,
    },
}

/// One setting a start report names, with the account of an instance's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedLine {
    pub name: ReportedName,
    pub account: Option<ReportedAccount>,
}

/// The account of an instance's own setting a start report names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportedAccount {
    Alias(String),
    /// The newest `Status` does not list its instance: most likely one the configuration no
    /// longer has, which is one reason a setting is not restored. Its id is in the log.
    NotListed,
}

/// `read`, the start report of the Runtime `link` names, while it answers; `gave_up` when every
/// try of this epoch failed.
pub fn start_view(
    link: &Link,
    read: Option<&StartRead>,
    gave_up: bool,
    acknowledged: bool,
    instances: &[InstanceLive],
) -> StartView {
    if !matches!(link, Link::Online { .. }) {
        return StartView::Nothing;
    }
    let account = |id: &str| {
        instances
            .iter()
            .find(|instance| instance.instance_id == id)
            .map_or(ReportedAccount::NotListed, |instance| {
                ReportedAccount::Alias(instance.alias.clone())
            })
    };
    let lines = |settings: &[ReportedSetting]| -> Vec<ReportedLine> {
        settings
            .iter()
            .map(|setting| ReportedLine {
                name: setting.name.clone(),
                account: setting.instance.as_deref().map(account),
            })
            .collect()
    };
    match read {
        Some(read) => StartView::Read {
            dropped: lines(read.dropped.as_slice()),
            base_changed: lines(read.base_changed.as_slice()),
            acknowledged,
        },
        None if gave_up => StartView::Unreadable,
        None => StartView::Nothing,
    }
}

/// Whether the Runtime is started again when it is gone, as the settings page says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartView {
    /// This run has no state root, so nothing runs `actingctl` (`paint`'s to tell).
    Unreachable,
    /// A run is out, or about to be.
    Reading,
    /// The Runtime started again since the last answer, which is read again when the page is
    /// next gone to.
    LetGo,
    /// The task as the report has it; `warned` when the report says something else is wrong
    /// (a fatal hold, a start that failed, …), which the page only points at in the log.
    On { warned: bool },
    Off { warned: bool },
    /// The task is there, and turned off or not as it was set up: it may not start anything.
    Differs,
    /// It cannot be told: the run failed, or the watchdog could not read the task or the root.
    Unreadable,
    /// This run cannot tell where `actingctl` is: the settings file names no Runtime, or
    /// names one not laid out as an installation is.
    NoPaths,
    NotInstalledLayout,
}

/// What a run of `watchdog status` came to, as the settings page says it.
pub fn autostart_of(read: &WatchdogRead) -> AutostartView {
    match read {
        WatchdogRead::Report(report) => match report.task {
            TaskState::On => AutostartView::On { warned: report.says_wrong() },
            TaskState::Missing => AutostartView::Off { warned: report.says_wrong() },
            TaskState::Differs => AutostartView::Differs,
            TaskState::Unread => AutostartView::Unreadable,
        },
        WatchdogRead::NoTool(ToolIssue::NoActingdExe | ToolIssue::NoActingdConfig) => {
            AutostartView::NoPaths
        }
        WatchdogRead::NoTool(ToolIssue::NotUnderRuntime(_) | ToolIssue::OtherConfig { .. }) => {
            AutostartView::NotInstalledLayout
        }
        WatchdogRead::Failed(_) => AutostartView::Unreadable,
    }
}

/// The Lab's options are the installer's, whatever the Runtime takes; any other setting the
/// Runtime does not take while it runs is the file's.
fn change_of(name: &SettingName, settable: bool) -> Change {
    match name {
        SettingName::LabAuthoring | SettingName::LabDebugging => Change::Installer,
        _ if !settable => Change::FileAndRestart,
        _ => Change::ViewOnly,
    }
}

/// A setting's name; one without a name here says so.
pub fn name_words(labels: &Labels, name: &SettingName) -> &'static str {
    match name {
        SettingName::QuietTimesHost => labels.setting_quiet_host,
        SettingName::QuietTimes => labels.setting_quiet,
        SettingName::StartEmulator => labels.setting_start_emulator,
        SettingName::IdleAction => labels.setting_idle_action,
        SettingName::MumuRoot => labels.setting_mumu_root,
        SettingName::EvidenceMaxAgeDays => labels.setting_evidence_days,
        SettingName::MemoryPriority => labels.setting_memory_priority,
        SettingName::IoPriority => labels.setting_io_priority,
        SettingName::LabAuthoring => labels.setting_lab_authoring,
        SettingName::LabDebugging => labels.setting_lab_debugging,
        SettingName::TouchBackend => labels.backend_touch,
        SettingName::CaptureBackend => labels.backend_capture,
        SettingName::Unnamed(_) => labels.setting_unnamed,
    }
}

/// A touch backend's name, as the contract's table has it (HS-M 6).
pub fn touch_words(labels: &Labels, backend: TouchBackend) -> &'static str {
    match backend {
        TouchBackend::AdbInput => labels.backend_touch_adb,
        TouchBackend::MaaTouch => labels.backend_touch_maatouch,
        TouchBackend::Minitouch => labels.backend_touch_minitouch,
        TouchBackend::Nemu => labels.backend_nemu,
    }
}

/// A capture backend's name, likewise.
pub fn capture_words(labels: &Labels, backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::Adb => labels.backend_capture_adb,
        CaptureBackend::DroidCast => labels.backend_capture_droidcast,
        CaptureBackend::Nemu => labels.backend_nemu,
        CaptureBackend::Scrcpy => labels.backend_capture_scrcpy,
    }
}

/// A backend in effect in words: a word without a name here as it is written.
pub fn in_effect_words<T: Copy>(
    value: &InEffect<T>,
    words: impl Fn(T) -> &'static str,
) -> String {
    match value {
        InEffect::Named(backend) => words(*backend).to_owned(),
        InEffect::Word(word) => word.clone(),
    }
}

/// The way an online value came in.
pub fn through_words(labels: &Labels, through: Through) -> &'static str {
    match through {
        Through::CommandLine => labels.backend_through_cli,
        Through::Mcp => labels.backend_through_mcp,
        Through::Console => labels.backend_through_console,
        Through::Lab => labels.backend_through_lab,
        Through::Other => labels.backend_through_other,
    }
}

/// A value in words: a list by whether it has entries, a key's own words by what they mean,
/// and any other word as written.
pub fn value_words(labels: &Labels, name: &SettingName, value: &ShownValue) -> String {
    match value {
        ShownValue::Unset => labels.setting_unset.to_owned(),
        ShownValue::Set => labels.setting_listed.to_owned(),
        ShownValue::Flag(true) => labels.setting_on.to_owned(),
        ShownValue::Flag(false) => labels.setting_off.to_owned(),
        ShownValue::Number(days) if *name == SettingName::EvidenceMaxAgeDays => {
            fill(labels.setting_days, &[&days.to_string()])
        }
        ShownValue::Number(number) => number.to_string(),
        ShownValue::Word(word) => match (name, word.as_str()) {
            (SettingName::IdleAction, IDLE_NONE) => labels.setting_idle_none.to_owned(),
            (SettingName::IdleAction, IDLE_CLOSE_APP) => labels.setting_idle_close_app.to_owned(),
            (SettingName::IdleAction, IDLE_STOP_EMULATOR) => {
                labels.setting_idle_stop_emulator.to_owned()
            }
            (SettingName::MemoryPriority | SettingName::IoPriority, PRIORITY_NORMAL) => {
                labels.setting_priority_normal.to_owned()
            }
            (SettingName::MemoryPriority | SettingName::IoPriority, PRIORITY_LOW) => {
                labels.setting_priority_low.to_owned()
            }
            (_, other) => other.to_owned(),
        },
    }
}

pub fn change_words(labels: &Labels, change: Change) -> &'static str {
    match change {
        Change::ViewOnly => labels.setting_view_only,
        Change::FileAndRestart => labels.setting_file_restart,
        Change::Installer => labels.setting_by_installer,
    }
}

/// When a change of it takes effect, as the Runtime's key table says (`runtime-config.md`);
/// nothing for a setting without a name here.
pub fn effect_words(labels: &Labels, name: &SettingName) -> Option<&'static str> {
    match name {
        SettingName::QuietTimesHost | SettingName::QuietTimes => Some(labels.setting_effect_round),
        SettingName::IdleAction => Some(labels.setting_effect_idle),
        SettingName::EvidenceMaxAgeDays => Some(labels.setting_effect_frames),
        SettingName::TouchBackend | SettingName::CaptureBackend => Some(labels.backend_effect),
        SettingName::StartEmulator
        | SettingName::MumuRoot
        | SettingName::MemoryPriority
        | SettingName::IoPriority
        | SettingName::LabAuthoring
        | SettingName::LabDebugging => Some(labels.setting_effect_start),
        SettingName::Unnamed(_) => None,
    }
}
