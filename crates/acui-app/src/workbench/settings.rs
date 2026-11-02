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
//! Nothing here is written. Every row says how its value is changed: in the configuration
//! file with a restart, by the installer, or not on this page for now.

use acui_source::workbench::{
    ConfigRead, InstanceLive, SettingName, SettingSource, SettingTier, ShownValue, UnreadRow,
    IDLE_CLOSE_APP, IDLE_NONE, IDLE_STOP_EMULATOR, PRIORITY_LOW, PRIORITY_NORMAL,
};

use super::model::Link;
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
        advanced: Vec<SettingLine>,
        /// Rows of the snapshot that may be of a setting shown here and do not read: the
        /// window says how many, the log which and why.
        unread: Vec<UnreadRow>,
    },
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
/// instances' own settings by the accounts `instances` lists, in that order.
pub fn view(
    link: &Link,
    read: Option<&ConfigRead>,
    failed: bool,
    instances: &[InstanceLive],
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
    let (mut daily, mut advanced) = (Vec::new(), Vec::new());
    for row in settings.rows(&ids) {
        let account = row.instance.map(|id| {
            instances
                .iter()
                .find(|instance| instance.instance_id == id)
                .map_or(id, |instance| instance.alias.clone())
        });
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
            SettingTier::Advanced => advanced.push(line),
        }
    }
    SettingsView::Read { daily, advanced, unread: settings.unread().to_vec() }
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
        SettingName::Unnamed(_) => labels.setting_unnamed,
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
        SettingName::StartEmulator
        | SettingName::MumuRoot
        | SettingName::MemoryPriority
        | SettingName::IoPriority
        | SettingName::LabAuthoring
        | SettingName::LabDebugging => Some(labels.setting_effect_start),
        SettingName::Unnamed(_) => None,
    }
}
