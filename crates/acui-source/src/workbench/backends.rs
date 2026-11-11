// SPDX-License-Identifier: GPL-3.0-only
//! An instance's device backends (HS-U, Runtime #402 HS-2): what the settings page's group
//! "device backends" reads of each device instance, the one change of settings it sends, and
//! what a refusal of that change says. Nothing out of here names a key, a wire word or a wire
//! type, but for the log: its touch and capture backends are [`TouchBackend`] and
//! [`CaptureBackend`], which only this file spells.
//!
//! What it reads is all in the slow lane's one `RuntimeFactSnapshot`
//! ([`super::ReadLane::read_config`]), no read of its own:
//!
//! - each instance's `touch_backend` and `capture_backend` rows of `config.parameters.instance`:
//!   the value, its source and whether the Runtime takes a change of it. A device instance's file
//!   names both and a fixture's neither, and a Runtime before 0.12.39 has neither key, so an
//!   instance no such row names has no group, and that is no error;
//! - its hidden `device_kind` row, none for an emulator;
//! - the `host.config_overrides` row of an online value: when it was set, through which way, and
//!   the configuration file's value under it;
//! - its newest self-check, the instance's `backend.selfcheck.<entry>.*` facts.
//!
//! The backends each kind of device has are the contract's table (HS-M 2.1, 6), taken in the
//! order of the key table's ranges (`TOUCH_BACKEND_TOKENS`, `CAPTURE_BACKEND_TOKENS`). The device
//! layer's own rule is in a crate the console does not link, and the Runtime is the judge
//! (`backend_kind_unsupported`).
//!
//! What it sends ([`ConfigEdit::backends`]) names one instance and only its two backend keys, one
//! or both, in one request based on the snapshot's version. The Runtime switches that device
//! before it answers, all of it or nothing, and refuses a request that mixes in anything else
//! (`backend_switch_not_alone`). The client waits for that answer itself: a `ConfigureSettings`
//! that names a backend key waits the contract's `BACKEND_SWITCH_WAIT_MS` and its io timeout for
//! the receipt ([`switch_wait`]), on the thread that sends it. It goes once, whatever comes back.
//!
//! An answer lost on the way is no answer of any other change ([`UnconfirmedSwitch`]): the
//! Runtime may still be switching the device, and the settings' version stays as it was until
//! the switch commits at its end, so only a snapshot read once the switch is over tells.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use acui_rows::code;
use actingcommand_contract::codes::{DeviceSelfCheckFailure, SharedCode};
use actingcommand_contract::outcome::{CauseRelation, Outcome};
use actingcommand_contract::{
    is_instance_backend_key, ConfigSettingChange, DeviceSelfCheckEntry, DeviceSelfCheckStatus,
    FactScalar, FactValue, RuntimeFactRecord, RuntimeFactScope, BACKEND_SWITCH_WAIT_MS,
    CONFIG_OVERRIDES_FACT_KEY, CONFIG_PARAMETERS_FACT_PREFIX, CONNECTION_PREPARATION_CLAIM_WAIT_MS,
};
use actingcommand_runtime_client::RuntimeChannel;
use actingcommand_runtime_config::performance::presets;
use actingcommand_runtime_config::{
    canonical_json, find_row, instance_key, scalar_json, ConfigValueProblem, KeyName, ValueType,
    CAPTURE_BACKEND, CAPTURE_BACKEND_TOKENS, DEVICE_KIND, OVERRIDE_BASE_JSON, OVERRIDE_CHANNEL,
    OVERRIDE_CONFIG_KEY, OVERRIDE_SET_UNIX_MS, TOUCH_BACKEND, TOUCH_BACKEND_TOKENS,
};

use super::{
    setting_source, ActivityKind, ConfigEdit, ConfigRead, FenceFact, InstanceLive, SettingSource,
    ROW_KEY, VALUE_INVALID, VALUE_PROBLEM,
};

/// The two words of `DEVICE_KIND_TOKENS`.
const EMULATOR: &str = "emulator";
const HANDSET: &str = "handset";

/// MuMu's own backend, a word of both ranges.
const NEMU_IPC: &str = "nemu_ipc";

/// The self-check facts of an instance (runtime-host `host/runtime_facts.rs`, Workflow #317 sc1):
/// `backend.selfcheck.<entry>.<suffix>`, the entry `input`, `capture` or `nemu`. Spelled by a
/// crate the console does not link.
const SELFCHECK_PREFIX: &str = "backend.selfcheck.";
const SELFCHECK_STATUS: &str = "status";
const SELFCHECK_CHECKED_AT: &str = "checked_at_unix_ms";
const SELFCHECK_SELECTED: &str = "selected";

/// What a refusal of a change of an instance's backends says. The value problems are the key
/// table's; the host's code of a switch it refused and its problems are runtime-host's
/// (`backend_switch_refused`, vocabulary `backend_switch_problem`), as are the code of a missing
/// MuMu app index with why it is missing (vocabulary `nemu_app_index_problem`, #402 HS-3) and the
/// causes of a failed capture open (vocabulary `capture_open_cause`): all of crates the console
/// does not link, spelled as the outcome catalog has them. Two of those words stay in the
/// vocabulary and come only from a Runtime 0.12.39 or earlier, after a rollback: since 0.12.40
/// (#402 HS-5a, HS-5b) a game that does not run fails no open, so no paired open says
/// `display_missing` and no app index read says `application_not_running`.
const KIND_UNSUPPORTED: &str = ConfigValueProblem::BackendKindUnsupported.as_str();
const PAIR_REQUIRED: &str = ConfigValueProblem::BackendPairRequired.as_str();
const NOT_ALONE: &str = ConfigValueProblem::BackendSwitchNotAlone.as_str();
const SWITCH_REFUSED: &str = "backend_switch_refused";
const SWITCH_PROBLEM: &str = "backend_switch_problem";
const INSTANCE_BUSY: &str = "instance_busy";
const PLAYER_HELD: &str = "player_held";
const INSTANCE_FENCED: &str = "instance_fenced";
const ENDPOINT_UNAVAILABLE: &str = "endpoint_unavailable";
const OPEN_FAILED: &str = "open_failed";
const RESTORE_FAILED: &str = "restore_failed";
const LEASE_QUEUE_EXPIRED: &str = SharedCode::LeaseQueueExpired.as_str();
const HOLDER_KIND: &str = "holder_kind";
const TIMEOUT_MS: &str = "timeout_ms";
const TOUCH_OPEN: &str = DeviceSelfCheckFailure::InputBackendOpenFailed.as_str();
const CAPTURE_OPEN: &str = DeviceSelfCheckFailure::CaptureBackendOpenFailed.as_str();
const PAIR_OPEN: &str = DeviceSelfCheckFailure::PairedBackendOpenFailed.as_str();
const TOUCH_CHECK: &str = DeviceSelfCheckFailure::InputCheckFailed.as_str();
const CAPTURE_CHECK: &str = DeviceSelfCheckFailure::CaptureCheckFailed.as_str();
const CAPTURE_OPEN_CAUSE: &str = "capture_open_cause";
const DISPLAY_MISSING: &str = "display_missing";
const CONNECTION_FAILED: &str = "connection_failed";
const NEMU_APP_INDEX_MISSING: &str = "nemu_app_index_missing";
const NEMU_APP_INDEX_PROBLEM: &str = "nemu_app_index_problem";
const PACKAGE_NOT_INSTALLED: &str = "package_not_installed";
const APPLICATION_NOT_RUNNING: &str = "application_not_running";
const INFO_UNAVAILABLE: &str = "info_unavailable";
const INFO_MALFORMED: &str = "info_malformed";
const CAUSED_BY: &str = CauseRelation::CausedBy.as_str();
const NOTE: &str = CauseRelation::Note.as_str();

/// The kind of device an instance drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// An emulator: an instance whose `device_kind` is `emulator`, or that has none.
    Emulator,
    Handset,
}

impl DeviceKind {
    /// The touch backends this kind of device has, of those this file has a name for, in the
    /// key table's order: what the group offers.
    pub fn touch_backends(self) -> Vec<TouchBackend> {
        offered::<TouchBackend>(|backend| self.has_touch(backend))
    }

    /// The capture backends this kind of device has, likewise.
    pub fn capture_backends(self) -> Vec<CaptureBackend> {
        offered::<CaptureBackend>(|backend| self.has_capture(backend))
    }

    /// The contract's table (HS-M 2.1): every touch backend on an emulator, all but MuMu's own
    /// on a handset.
    fn has_touch(self, backend: TouchBackend) -> bool {
        !(self == Self::Handset && backend == TouchBackend::Nemu)
    }

    /// ADB on both; DroidCast and MuMu's own on an emulator; scrcpy on a handset, and on an
    /// emulator only from #402's second step on.
    fn has_capture(self, backend: CaptureBackend) -> bool {
        match backend {
            CaptureBackend::Adb => true,
            CaptureBackend::DroidCast | CaptureBackend::Nemu => self == Self::Emulator,
            CaptureBackend::Scrcpy => self == Self::Handset,
        }
    }
}

/// A touch backend this file has a name for: a word of `TOUCH_BACKEND_TOKENS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TouchBackend {
    /// `adb_shell_input`
    AdbInput,
    /// `maatouch`
    MaaTouch,
    /// `minitouch`
    Minitouch,
    /// `nemu_ipc`: MuMu's own, an emulator's only, and only beside MuMu's own capture.
    Nemu,
}

impl TouchBackend {
    /// The one capture backend this touch backend goes with, when it goes with only one: MuMu's
    /// own touch only beside MuMu's own capture (`backend_pair_required`). Any capture backend,
    /// MuMu's own included, goes with each of the others.
    pub fn paired_capture(self) -> Option<CaptureBackend> {
        (self == Self::Nemu).then_some(CaptureBackend::Nemu)
    }
}

/// A capture backend this file has a name for: a word of `CAPTURE_BACKEND_TOKENS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CaptureBackend {
    /// `adb`
    Adb,
    /// `droidcast_raw`: an emulator's only.
    DroidCast,
    /// `nemu_ipc`: MuMu's own, an emulator's only.
    Nemu,
    /// `scrcpy`: a handset's only, until #402's second step brings it to emulators.
    Scrcpy,
}

/// One of the two backend keys' backends, by the word of the key's range.
trait BackendWord: Copy + 'static {
    /// The key's member: `instance.<instance id>.<member>`.
    const MEMBER: &'static str;
    /// The key's range, in the key table's order.
    const TOKENS: &'static [&'static str];
    /// Every backend of the key this file has a name for.
    const ALL: &'static [Self];

    fn word(self) -> &'static str;
}

impl BackendWord for TouchBackend {
    const MEMBER: &'static str = TOUCH_BACKEND;
    const TOKENS: &'static [&'static str] = TOUCH_BACKEND_TOKENS;
    const ALL: &'static [Self] = &[Self::AdbInput, Self::MaaTouch, Self::Minitouch, Self::Nemu];

    fn word(self) -> &'static str {
        match self {
            Self::AdbInput => "adb_shell_input",
            Self::MaaTouch => "maatouch",
            Self::Minitouch => "minitouch",
            Self::Nemu => NEMU_IPC,
        }
    }
}

impl BackendWord for CaptureBackend {
    const MEMBER: &'static str = CAPTURE_BACKEND;
    const TOKENS: &'static [&'static str] = CAPTURE_BACKEND_TOKENS;
    const ALL: &'static [Self] = &[Self::Adb, Self::DroidCast, Self::Nemu, Self::Scrcpy];

    fn word(self) -> &'static str {
        match self {
            Self::Adb => "adb",
            Self::DroidCast => "droidcast_raw",
            Self::Nemu => NEMU_IPC,
            Self::Scrcpy => "scrcpy",
        }
    }
}

/// The backend `word` names, if this file has a name for it.
fn named<T: BackendWord>(word: &str) -> Option<T> {
    T::ALL.iter().copied().find(|backend| backend.word() == word)
}

/// The backends of the key's range `has` admits, in the range's order. A word of the range
/// without a name here is never offered ([`unnamed_backend_words`]), nor is a name whose word the
/// range no longer has.
fn offered<T: BackendWord>(has: impl Fn(T) -> bool) -> Vec<T> {
    T::TOKENS
        .iter()
        .filter_map(|word| named::<T>(word))
        .filter(|backend| has(*backend))
        .collect()
}

/// The words of the two keys' ranges this file has no name for, so offers nowhere, each after
/// its key's member: for the log, once.
pub fn unnamed_backend_words() -> Vec<String> {
    let mut words = unnamed::<TouchBackend>();
    words.extend(unnamed::<CaptureBackend>());
    words
}

fn unnamed<T: BackendWord>() -> Vec<String> {
    T::TOKENS
        .iter()
        .filter(|word| named::<T>(word).is_none())
        .map(|word| format!("{} {word}", T::MEMBER))
        .collect()
}

/// A backend in effect: one this file has a name for, or a word of the key's range it has none
/// for, shown as it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InEffect<T> {
    Named(T),
    Word(String),
}

fn in_effect<T: BackendWord>(word: &str) -> InEffect<T> {
    named::<T>(word).map_or_else(|| InEffect::Word(word.to_owned()), InEffect::Named)
}

/// One of an instance's two backend rows, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRow<T> {
    pub value: InEffect<T>,
    /// `File` or `Online` (`explicit`, `instruction`): the key has no default, and nothing
    /// discovers it.
    pub source: SettingSource,
    /// Whether the Runtime takes a change of it while it runs.
    pub settable: bool,
    /// What `host.config_overrides` says of an online value. `None` for a file value, and for an
    /// online one the snapshot holds no such row of, which is the caller's to log.
    pub online: Option<OnlineChange<T>>,
}

/// An online value's record (`host.config_overrides`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlineChange<T> {
    /// When it was set, in unix ms.
    pub set_unix_ms: Option<u64>,
    /// The way it came in; `None` for a record from before the Runtime kept that (U-2).
    pub through: Option<Through>,
    /// The configuration file's value under it, which clearing it goes back to. `None` when the
    /// file names none, or its text is no word of the key's range.
    pub file_value: Option<InEffect<T>>,
}

/// The way an online value came in: the channel of the connection that set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Through {
    CommandLine,
    Mcp,
    /// This console, or another on the console's channel.
    Console,
    Lab,
    /// A channel this file does not know.
    Other,
}

/// An instance's newest self-check, by the entry of its opens. Which entries count is the touch
/// backend's: MuMu's own pair (`nemu`) when its touch is MuMu's own, else the touch and capture
/// backends' own opens (`input`, `capture`); an entry a previous configuration left is still
/// here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelfCheck {
    pub input: Option<EntryCheck>,
    pub capture: Option<EntryCheck>,
    pub nemu: Option<EntryCheck>,
}

/// One entry's newest self-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryCheck {
    pub status: CheckStatus,
    /// When, in unix ms.
    pub checked_at: Option<u64>,
    /// The backend the open selected, as the fact spells it (`-` for none). Its spelling is the
    /// device layer's, not the key's range, so it is not compared: for the log only.
    selected: Option<String>,
}

impl EntryCheck {
    /// The backend the open selected, as the fact spells it: for the log only.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }
}

/// What one self-check came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Passed,
    Failed,
    /// `unknown`: the open did not tell.
    Unsure,
    /// A word or a value this file does not read, as the wire states it: for the log.
    Unread(String),
}

/// What a snapshot says of one device instance's backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceBackends {
    instance: String,
    /// `Err`: the `device_kind` row's value as the wire states it, of a kind this file does not
    /// know, or a row that does not read. The group can then only be shown.
    pub kind: Result<DeviceKind, String>,
    /// `None`: no such row, or one that does not read, which the settings' unread rows count
    /// (`SettingsRead::unread`).
    pub touch: Option<BackendRow<TouchBackend>>,
    pub capture: Option<BackendRow<CaptureBackend>>,
    pub selfcheck: SelfCheck,
}

impl InstanceBackends {
    /// The instance's id.
    pub fn instance_id(&self) -> &str {
        &self.instance
    }

    /// What a change of its backends is sent to. Only when its kind and both its rows read: a
    /// change is judged by what is in effect, and the pair by both.
    pub fn target(&self) -> Option<BackendTarget> {
        (self.kind.is_ok() && self.touch.is_some() && self.capture.is_some())
            .then(|| BackendTarget(self.instance.clone()))
    }

    /// Everything read, the self-check's selected backends included: for the log only.
    pub fn line(&self) -> String {
        format!(
            "instance {} backends: kind {:?}, touch {:?}, capture {:?}, self-check {:?}",
            self.instance, self.kind, self.touch, self.capture, self.selfcheck,
        )
    }
}

/// The one instance a change of backends is sent to: only a read gives one
/// ([`InstanceBackends::target`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendTarget(String);

impl BackendTarget {
    pub fn instance_id(&self) -> &str {
        &self.0
    }
}

/// What one change of backends does with one of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChange<T> {
    /// Not sent: it stays as it is.
    Keep,
    To(T),
    /// Back to the configuration file's value: the online one is dropped, which switches the
    /// device as well unless the file's value is the one in effect (the Runtime then takes the
    /// change without a switch).
    BackToFile,
}

impl ConfigEdit {
    /// One switch of `target`'s backends: its touch and capture backend keys, the ones that do
    /// not `Keep`, in one request and nothing else (HS-M 2.1, one instance at a time). `None`
    /// when both keep. Whether the pair goes together is the caller's
    /// (`TouchBackend::paired_capture`), and the Runtime's to judge (`backend_pair_required`).
    pub fn backends(
        target: &BackendTarget,
        touch: BackendChange<TouchBackend>,
        capture: BackendChange<CaptureBackend>,
    ) -> Option<Self> {
        let changes: Vec<ConfigSettingChange> =
            [backend_change(target, touch), backend_change(target, capture)]
                .into_iter()
                .flatten()
                .collect();
        (!changes.is_empty()).then_some(Self(changes))
    }
}

fn backend_change<T: BackendWord>(
    target: &BackendTarget,
    change: BackendChange<T>,
) -> Option<ConfigSettingChange> {
    let word = match change {
        BackendChange::Keep => return None,
        BackendChange::To(backend) => Some(backend.word()),
        BackendChange::BackToFile => None,
    };
    Some(ConfigSettingChange {
        key: instance_key(&target.0, T::MEMBER),
        value: word.map(Into::into),
    })
}

/// The longest a switch's claim waits for its turn in the instance's queue: the contract's
/// `CONNECTION_PREPARATION_CLAIM_WAIT_MS`, the first term of `BACKEND_SWITCH_WAIT_MS`. A device
/// free within it switches; one still busy then is refused (`backend_busy`). The settings page
/// says it in whole seconds.
pub const CLAIM_WAIT: Duration = Duration::from_millis(CONNECTION_PREPARATION_CLAIM_WAIT_MS);

/// How long the client waits for the answer to a switch of an instance's backends, as
/// runtime-client reckons it: the receipt wait of a `ConfigureSettings` that names a backend key
/// is `BACKEND_SWITCH_WAIT_MS` plus the connection's io timeout (`receipt_response_timeout`,
/// crates/runtime-client/src/client.rs:6766-6779 at v0.12.39). [`super::configure`] leaves that
/// io timeout at the client's default (`DEFAULT_RUNTIME_IO_TIMEOUT`, client.rs:86), which is read
/// back from the same configuration (`configure_io_timeout`), not spelled here.
///
/// The contract's constant is the switch's worst case (#402 HS-2b, register §1.274: the claim,
/// the two session closes, the two preparations and the commit's ledger write). A close slower
/// than its allowance, or a ledger writer configured above its default, can outlast it, so this
/// bounds no switch: it is the earliest an unconfirmed one may be over. The settings page says it
/// in whole seconds.
pub fn switch_wait() -> Duration {
    Duration::from_millis(BACKEND_SWITCH_WAIT_MS).saturating_add(super::configure_io_timeout())
}

/// A switch of an instance's backends whose answer was lost: the
/// [`super::ConfigOutcome::Unconfirmed`] of a change that `switches_backends` (the client's wait
/// ran out, or its connection failed), or the switch's write thread panicked.
///
/// The Runtime may then still be switching the device, and go on to commit the change, to put
/// the previous backends back, or to fail at that too and leave the device unavailable. It
/// changes no setting before it commits, at the switch's very end (the device's steps run
/// outside the settings' gate), so a snapshot at the version the switch was based on says
/// nothing of it while it may go on: neither that it did not happen nor that it may be asked
/// again. It is over ([`UnconfirmedSwitch::seen`]) once [`switch_wait`] has passed since it was
/// sent and a `Status` shows the instance doing anything but a self-check: the switch shows as
/// one from its claim's grant until it answers, after its commit, and its claim waits at most
/// [`CLAIM_WAIT`] of that wait, so one not shown by then never began. Only a snapshot read after
/// that `Status` tells what came of it ([`UnconfirmedSwitch::verdict`]). Nothing here sends it
/// again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnconfirmedSwitch {
    instance: String,
    based_on: Option<u64>,
    sent: Instant,
}

impl UnconfirmedSwitch {
    /// The switch of `target`'s backends asked for at `sent`, based on the settings' version
    /// `based_on`. `sent` is the press's, a moment before its thread sends it: the claim's
    /// seconds inside the wait leave that moment no weight.
    pub fn new(target: &BackendTarget, based_on: Option<u64>, sent: Instant) -> Self {
        Self { instance: target.0.clone(), based_on, sent }
    }

    /// The instance it switches.
    pub fn instance_id(&self) -> &str {
        &self.instance
    }

    /// What a `Status` that landed at `now` says of it, `instances` being that status's.
    pub fn seen(&self, now: Instant, instances: &[InstanceLive]) -> SwitchSeen {
        if now.saturating_duration_since(self.sent) < switch_wait() {
            return SwitchSeen::MayGoOn;
        }
        let Some(instance) = instances.iter().find(|instance| instance.instance_id == self.instance)
        else {
            return SwitchSeen::Unlisted;
        };
        let checking = instance
            .activity
            .as_ref()
            .is_some_and(|activity| activity.kind == ActivityKind::Selfcheck);
        if checking {
            SwitchSeen::MayGoOn
        } else {
            SwitchSeen::Over
        }
    }

    /// What a snapshot read after a `Status` that said it was over says of it: by the settings'
    /// version, with the instance's backends and newest self-check as that read has them.
    pub fn verdict(&self, read: &ConfigRead) -> SwitchVerdict {
        let version = match (read.version, self.based_on) {
            (Some(now), Some(based_on)) if now == based_on => SwitchVersion::Unchanged,
            (Some(_), Some(_)) => SwitchVersion::Changed,
            (None, _) | (_, None) => SwitchVersion::Unknown,
        };
        SwitchVerdict { version, now: read.backends.get(&self.instance).cloned() }
    }
}

/// What a `Status` says of an unconfirmed switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchSeen {
    /// It may still go on: not [`switch_wait`] since it was sent, or the instance shows a
    /// self-check, the switch's or another's. Nothing is to be said of it but that, and nothing
    /// of a version.
    MayGoOn,
    /// Over: past the wait, and the instance does something else, or nothing.
    Over,
    /// Past the wait, and the `Status` lists no such instance, so nothing switches it: over as
    /// well, but in a state the console knows nothing of, which is the caller's to log.
    Unlisted,
}

/// What a snapshot read once an unconfirmed switch was over says of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchVerdict {
    pub version: SwitchVersion,
    /// The instance's backends and newest self-check as the same read has them, which the
    /// verdict is said with: on the backends asked for, back on the previous ones, or on none
    /// that passed. `None` when the read names no backend of it.
    pub now: Option<InstanceBackends>,
}

/// The settings' version an unconfirmed switch's verdict is read at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchVersion {
    /// Another than the one it was based on: the settings changed, perhaps by this switch. What
    /// shows is what holds.
    Changed,
    /// The one it was based on: the Runtime committed none of it, so it did not take effect.
    /// The device is on its previous backends, or unavailable when they did not come back: its
    /// self-check says which, and is said with it.
    Unchanged,
    /// The read, or the switch, has no version to judge by.
    Unknown,
}

/// Whether `name` is one of an instance's two backend keys, which the settings page shows as
/// the instance's device backends, not as rows of its tiers.
pub(super) fn is_backend_name(name: KeyName) -> bool {
    matches!(name, KeyName::PerInstance(TOUCH_BACKEND | CAPTURE_BACKEND))
}

/// Every instance a backend row of the snapshot names, by instance id, with its kind, both rows
/// as far as they read, their online records and its self-check.
pub(super) fn read(records: &[RuntimeFactRecord]) -> BTreeMap<String, InstanceBackends> {
    let stamps = stamps(records);
    let mut kinds = BTreeMap::new();
    let mut groups: BTreeMap<String, InstanceBackends> = BTreeMap::new();
    let rows = records
        .iter()
        .filter(|record| {
            record.scope == RuntimeFactScope::Runtime
                && record.key.starts_with(CONFIG_PARAMETERS_FACT_PREFIX)
        })
        .filter_map(|record| match &record.value {
            FactValue::RecordList(rows) => Some(rows),
            _ => None,
        })
        .flatten();
    for row in rows {
        let Some(FactScalar::String(key)) = row.get(ROW_KEY) else {
            continue;
        };
        let Some((key_row, Some(instance))) = find_row(key) else {
            continue;
        };
        match key_row.name {
            KeyName::PerInstance(DEVICE_KIND) => {
                kinds.insert(instance.to_owned(), device_kind(row));
            }
            KeyName::PerInstance(TOUCH_BACKEND) => {
                group(&mut groups, instance).touch = backend_row(row, stamps.get(key.as_str()));
            }
            KeyName::PerInstance(CAPTURE_BACKEND) => {
                group(&mut groups, instance).capture = backend_row(row, stamps.get(key.as_str()));
            }
            _ => {}
        }
    }
    let mut checks = selfchecks(records);
    for (instance, backends) in &mut groups {
        backends.kind = kinds.remove(instance).unwrap_or(Ok(DeviceKind::Emulator));
        backends.selfcheck = checks.remove(instance).unwrap_or_default();
    }
    groups
}

fn group<'a>(
    groups: &'a mut BTreeMap<String, InstanceBackends>,
    instance: &str,
) -> &'a mut InstanceBackends {
    groups.entry(instance.to_owned()).or_insert_with(|| InstanceBackends {
        instance: instance.to_owned(),
        kind: Ok(DeviceKind::Emulator),
        touch: None,
        capture: None,
        selfcheck: SelfCheck::default(),
    })
}

fn device_kind(row: &BTreeMap<String, FactScalar>) -> Result<DeviceKind, String> {
    match presets::parameter_of_row(row).map(|parameter| parameter.value) {
        Some(FactScalar::String(word)) if word == EMULATOR => Ok(DeviceKind::Emulator),
        Some(FactScalar::String(word)) if word == HANDSET => Ok(DeviceKind::Handset),
        Some(FactScalar::String(word)) => Err(word),
        Some(other) => Err(format!("{other:?}")),
        None => Err(format!("{row:?}")),
    }
}

/// A backend row, if it reads: a parameter whose value is a word, as the settings' reading
/// takes a row of a string key (`SettingsRead::take`).
fn backend_row<T: BackendWord>(
    row: &BTreeMap<String, FactScalar>,
    stamp: Option<&Stamp>,
) -> Option<BackendRow<T>> {
    let parameter = presets::parameter_of_row(row)?;
    let FactScalar::String(word) = &parameter.value else {
        return None;
    };
    let source = setting_source(parameter.source);
    let online = match (source, stamp) {
        (SettingSource::Online, Some(stamp)) => Some(stamp.change()),
        _ => None,
    };
    Some(BackendRow { value: in_effect(word), source, settable: parameter.settable, online })
}

/// What a `host.config_overrides` row says of a backend key's online value.
struct Stamp {
    set_unix_ms: Option<u64>,
    through: Option<Through>,
    base_json: Option<String>,
}

impl Stamp {
    fn change<T: BackendWord>(&self) -> OnlineChange<T> {
        OnlineChange {
            set_unix_ms: self.set_unix_ms,
            through: self.through,
            file_value: self.base_json.as_deref().and_then(file_value::<T>),
        }
    }
}

/// The rows of `host.config_overrides` of backend keys, by key.
fn stamps(records: &[RuntimeFactRecord]) -> BTreeMap<String, Stamp> {
    let overrides = records
        .iter()
        .find(|record| {
            record.scope == RuntimeFactScope::Runtime && record.key == CONFIG_OVERRIDES_FACT_KEY
        })
        .map(|record| &record.value);
    let Some(FactValue::RecordList(rows)) = overrides else {
        return BTreeMap::new();
    };
    rows.iter()
        .filter_map(|row| {
            let Some(FactScalar::String(key)) = row.get(OVERRIDE_CONFIG_KEY) else {
                return None;
            };
            if !is_instance_backend_key(key) {
                return None;
            }
            let set_unix_ms = match row.get(OVERRIDE_SET_UNIX_MS) {
                Some(FactScalar::TimestampMs(ms)) => Some(*ms),
                Some(FactScalar::Integer(ms)) => u64::try_from(*ms).ok(),
                _ => None,
            };
            let through = match row.get(OVERRIDE_CHANNEL) {
                Some(FactScalar::String(channel)) => Some(through_of(channel)),
                _ => None,
            };
            let base_json = match row.get(OVERRIDE_BASE_JSON) {
                Some(FactScalar::String(text)) => Some(text.clone()),
                _ => None,
            };
            Some((key.clone(), Stamp { set_unix_ms, through, base_json }))
        })
        .collect()
}

/// The file's value an online one overrode, by its canonical JSON text, which the key table
/// writes of a word of the key's range as it does here; its text of no value (`null`) is none.
fn file_value<T: BackendWord>(base_json: &str) -> Option<InEffect<T>> {
    T::TOKENS
        .iter()
        .copied()
        .find(|word| {
            scalar_json(ValueType::String, &FactScalar::String((*word).to_owned()))
                .is_some_and(|json| canonical_json(&json) == base_json)
        })
        .map(|word| in_effect(word))
}

const CHANNELS: [RuntimeChannel; 4] =
    [RuntimeChannel::Cli, RuntimeChannel::Mcp, RuntimeChannel::Ui, RuntimeChannel::Lab];

fn through_of(channel: &str) -> Through {
    CHANNELS
        .into_iter()
        .find(|known| known.as_str() == channel)
        .map_or(Through::Other, |known| match known {
            RuntimeChannel::Cli => Through::CommandLine,
            RuntimeChannel::Mcp => Through::Mcp,
            RuntimeChannel::Ui => Through::Console,
            RuntimeChannel::Lab => Through::Lab,
        })
}

/// What one entry's facts say, as they come.
#[derive(Default)]
struct EntryFacts {
    status: Option<CheckStatus>,
    checked_at: Option<u64>,
    selected: Option<String>,
}

/// Every instance's self-check facts, by instance id. An entry without its status is none.
fn selfchecks(records: &[RuntimeFactRecord]) -> BTreeMap<String, SelfCheck> {
    let mut facts: BTreeMap<(String, DeviceSelfCheckEntry), EntryFacts> = BTreeMap::new();
    for record in records {
        let RuntimeFactScope::Instance { instance_id } = &record.scope else {
            continue;
        };
        let Some((entry, suffix)) = record
            .key
            .strip_prefix(SELFCHECK_PREFIX)
            .and_then(|rest| rest.split_once('.'))
        else {
            continue;
        };
        let Some(entry) =
            DeviceSelfCheckEntry::ALL.iter().copied().find(|known| known.as_str() == entry)
        else {
            continue;
        };
        let part = facts.entry((code(instance_id), entry)).or_default();
        match suffix {
            SELFCHECK_STATUS => part.status = Some(check_status(&record.value)),
            SELFCHECK_CHECKED_AT => {
                part.checked_at = match &record.value {
                    FactValue::Integer(ms) => u64::try_from(*ms).ok(),
                    FactValue::TimestampMs(ms) => Some(*ms),
                    _ => None,
                };
            }
            SELFCHECK_SELECTED => {
                part.selected = Some(match &record.value {
                    FactValue::String(text) => text.clone(),
                    other => code(other),
                });
            }
            _ => {}
        }
    }
    let mut checks: BTreeMap<String, SelfCheck> = BTreeMap::new();
    for ((instance, entry), part) in facts {
        let Some(status) = part.status else {
            continue;
        };
        let check = EntryCheck { status, checked_at: part.checked_at, selected: part.selected };
        let selfcheck = checks.entry(instance).or_default();
        match entry {
            DeviceSelfCheckEntry::Input => selfcheck.input = Some(check),
            DeviceSelfCheckEntry::Capture => selfcheck.capture = Some(check),
            DeviceSelfCheckEntry::Nemu => selfcheck.nemu = Some(check),
        }
    }
    checks
}

fn check_status(value: &FactValue) -> CheckStatus {
    let FactValue::String(word) = value else {
        return CheckStatus::Unread(code(value));
    };
    match DeviceSelfCheckStatus::ALL.iter().copied().find(|known| known.as_str() == word.as_str())
    {
        Some(DeviceSelfCheckStatus::Passed) => CheckStatus::Passed,
        Some(DeviceSelfCheckStatus::Failed) => CheckStatus::Failed,
        Some(DeviceSelfCheckStatus::Unknown) => CheckStatus::Unsure,
        None => CheckStatus::Unread(word.clone()),
    }
}

/// What a refusal of a change of an instance's backends says beyond its `RefusalWhy`, which
/// keeps its class: a value problem stays `Value` (its problem `Other`), the host's refusal of a
/// switch `Other`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRefusal {
    /// `None`: a refusal of another kind, which its `RefusalWhy` says.
    pub why: Option<BackendWhy>,
    /// The new backends opened and passed, the Runtime then took none of the change (its record
    /// failed, or the change no longer applied), and the previous backends did not come back:
    /// the refusal's last `note`. The device is unavailable until a self-check of it passes.
    pub not_restored: Option<OpenFailure>,
    /// The receipt's whole outcome, codes and values: for the log only.
    chain: String,
}

impl BackendRefusal {
    /// The receipt's whole outcome: for the log only.
    pub fn line(&self) -> &str {
        &self.chain
    }
}

/// What a refusal says of the backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendWhy {
    /// `config_value_invalid` (`backend_kind_unsupported`): this kind of device has no such
    /// backend. Nothing changed.
    KindUnsupported,
    /// `config_value_invalid` (`backend_pair_required`): MuMu's own touch beside another capture.
    PairRequired,
    /// `config_value_invalid` (`backend_switch_not_alone`): the request mixed in another key, or
    /// another instance's. This program's fault, never the person's.
    NotAlone,
    /// `backend_switch_refused`: the device was not switched, and nothing changed.
    Switch(SwitchRefusal),
}

/// Why the Runtime did not switch the device (`backend_switch_problem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchRefusal {
    /// `instance_busy`: not free within the switch claim's wait. What held it (its
    /// `lease_queue_expired`'s `holder_kind`), and how long it was waited for.
    Busy { holder: Option<Holder>, waited_ms: Option<u64> },
    /// `player_held`: a player has the device.
    PlayerHeld,
    /// `instance_fenced`: its resources were not confirmed closed; a passing self-check lifts it.
    Fenced,
    /// `endpoint_unavailable`: the emulator is stopped, or the handset is not bound.
    EndpointUnavailable,
    /// `open_failed`: the new backends did not open or pass their self-check; the previous
    /// ones are back. The failure, when it names one.
    OpenFailed(Option<OpenFailure>),
    /// `restore_failed`: and the previous ones did not come back either, so the device is
    /// unavailable. The new backends' failure and the restore's, of the two causes in that
    /// order; a single cause is the restore's.
    RestoreFailed { new: Option<OpenFailure>, restore: Option<OpenFailure> },
    /// A problem this file does not know, as the wire spells it, empty when there is none.
    Other(String),
}

/// What held an instance a switch waited for (`holder_kind`, vocabulary `claim_kind`, the
/// scheduler's `ClaimKind`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// `policy_dispatch`
    PolicyDispatch,
    /// `ladder`
    Ladder,
    /// `ladder_continuation`
    LadderContinuation,
    /// `startup_package`
    StartupPackage,
    /// `autostart`
    Autostart,
    /// `direct_task_run`
    DirectTaskRun,
    /// `emulator_control`
    EmulatorControl,
    /// `input_lifecycle`
    InputLifecycle,
    /// `resume_reconnect`
    ResumeReconnect,
    /// `self_check`
    SelfCheck,
    /// `backend_switch`
    BackendSwitch,
    /// `daemon_start_preparation`
    DaemonStartPreparation,
    /// `lab_operation`
    LabOperation,
    /// `client_lease`
    ClientLease,
    /// `resource_close`
    ResourceClose,
    /// A word this file does not know, as the wire spells it.
    Other(String),
}

/// Why backends did not open, as far as a failure's causes tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFailure {
    pub step: FailedStep,
    /// Why a capture open, or MuMu's own pair's, failed (`capture_open_cause`).
    pub capture: Option<CaptureCause>,
    /// `nemu_app_index_missing` anywhere under it: MuMu's own pair found no MuMu app index to
    /// open with, and why, as its `nemu_app_index_problem` says.
    pub app_index: Option<AppIndexProblem>,
}

/// Why MuMu's own pair found no MuMu app index (#402 HS-3, vocabulary `nemu_app_index_problem`):
/// the configuration names none, and the Runtime read the state of the device's game from
/// MuMuManager before it opened the pair. A switch it refuses for that leaves the previous
/// backends back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppIndexProblem {
    /// `package_not_installed`: the game package configured for the device is not installed on it.
    PackageNotInstalled,
    /// `application_not_running`: the game is installed and not running. Only a Runtime 0.12.39
    /// or earlier, after a rollback, refuses so: since 0.12.40 (#402 HS-5b) a stopped game
    /// resolves the index as a running one does.
    ApplicationNotRunning,
    /// `info_unavailable`: MuMuManager gave no answer.
    InfoUnavailable,
    /// `info_malformed`: MuMuManager answered something that does not read.
    InfoMalformed,
    /// A problem this file does not know, as the wire spells it, empty when there is none.
    Other(String),
}

/// The step of a device's preparation that failed: the self-check's own failures, the first
/// found down the failure's causes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedStep {
    /// `input_backend_open_failed`
    TouchOpen,
    /// `capture_backend_open_failed`
    CaptureOpen,
    /// `paired_backend_open_failed`: MuMu's own connection.
    NemuOpen,
    /// `input_check_failed`
    TouchCheck,
    /// `capture_check_failed`
    CaptureCheck,
    /// None of them: the failure's own code, for its words by its kind and the log.
    Other(String),
}

/// Why a capture open failed (`capture_open_cause`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCause {
    /// `display_missing`: the game is not running. Only a Runtime 0.12.39 or earlier, after a
    /// rollback, says it: since 0.12.40 (#402 HS-5a) a paired session without the game's display
    /// takes the main display.
    AppNotRunning,
    /// `connection_failed`: the emulator could not be reached.
    Unreachable,
    /// `other`, or a word this file does not know.
    Other,
}

/// The backends' part of a refusal of a change of them, the receipt's top code being `top`.
pub(super) fn refusal(top: &str, answer: Option<&Outcome>) -> Option<BackendRefusal> {
    let answer = answer?;
    let why = match top {
        VALUE_INVALID => match answer.value(VALUE_PROBLEM).and_then(|value| value.as_str()) {
            Some(KIND_UNSUPPORTED) => Some(BackendWhy::KindUnsupported),
            Some(PAIR_REQUIRED) => Some(BackendWhy::PairRequired),
            Some(NOT_ALONE) => Some(BackendWhy::NotAlone),
            Some(_) | None => None,
        },
        SWITCH_REFUSED => Some(BackendWhy::Switch(switch_refusal(answer))),
        _ => None,
    };
    // Only a switch refused after the device's steps notes the restore's failure: a refusal of
    // the switch itself names both failures as its causes.
    let not_restored = match top {
        SWITCH_REFUSED => None,
        _ => links(answer, NOTE).last().map(open_failure),
    };
    (why.is_some() || not_restored.is_some()).then(|| BackendRefusal {
        why,
        not_restored,
        chain: answer.to_string(),
    })
}

fn switch_refusal(answer: &Outcome) -> SwitchRefusal {
    let problem = answer.value(SWITCH_PROBLEM).and_then(|value| value.as_str()).unwrap_or_default();
    match problem {
        INSTANCE_BUSY => {
            let expired = links(answer, CAUSED_BY).find_map(|cause| {
                found(cause, &|outcome: &Outcome| outcome.code().as_str() == LEASE_QUEUE_EXPIRED)
            });
            SwitchRefusal::Busy {
                holder: expired
                    .and_then(|outcome| outcome.value(HOLDER_KIND))
                    .and_then(|value| value.as_str())
                    .map(holder),
                waited_ms: expired
                    .and_then(|outcome| outcome.value(TIMEOUT_MS))
                    .and_then(|value| value.as_u64()),
            }
        }
        PLAYER_HELD => SwitchRefusal::PlayerHeld,
        INSTANCE_FENCED => SwitchRefusal::Fenced,
        ENDPOINT_UNAVAILABLE => SwitchRefusal::EndpointUnavailable,
        OPEN_FAILED => SwitchRefusal::OpenFailed(links(answer, CAUSED_BY).next().map(open_failure)),
        RESTORE_FAILED => match links(answer, CAUSED_BY).collect::<Vec<_>>().as_slice() {
            [] => SwitchRefusal::RestoreFailed { new: None, restore: None },
            [restore] => {
                SwitchRefusal::RestoreFailed { new: None, restore: Some(open_failure(restore)) }
            }
            [new, restore, ..] => SwitchRefusal::RestoreFailed {
                new: Some(open_failure(new)),
                restore: Some(open_failure(restore)),
            },
        },
        other => SwitchRefusal::Other(other.to_owned()),
    }
}

pub(super) fn holder(word: &str) -> Holder {
    match word {
        "policy_dispatch" => Holder::PolicyDispatch,
        "ladder" => Holder::Ladder,
        "ladder_continuation" => Holder::LadderContinuation,
        "startup_package" => Holder::StartupPackage,
        "autostart" => Holder::Autostart,
        "direct_task_run" => Holder::DirectTaskRun,
        "emulator_control" => Holder::EmulatorControl,
        "input_lifecycle" => Holder::InputLifecycle,
        "resume_reconnect" => Holder::ResumeReconnect,
        "self_check" => Holder::SelfCheck,
        "backend_switch" => Holder::BackendSwitch,
        "daemon_start_preparation" => Holder::DaemonStartPreparation,
        "lab_operation" => Holder::LabOperation,
        "client_lease" => Holder::ClientLease,
        "resource_close" => Holder::ResourceClose,
        other => Holder::Other(other.to_owned()),
    }
}

/// The outcomes `outcome` links by `relation`, in order. A stop's failed reset is read so too
/// (`stop`).
pub(super) fn links<'a>(
    outcome: &'a Outcome,
    relation: &'static str,
) -> impl Iterator<Item = &'a Outcome> {
    outcome
        .causes()
        .iter()
        .filter(move |link| link.relation().as_str() == relation)
        .map(|link| link.outcome())
}

/// The first of `outcome` and its causes, depth first down `caused_by`, that `wanted` takes.
fn found<'a>(outcome: &'a Outcome, wanted: &dyn Fn(&Outcome) -> bool) -> Option<&'a Outcome> {
    if wanted(outcome) {
        return Some(outcome);
    }
    links(outcome, CAUSED_BY).find_map(|cause| found(cause, wanted))
}

/// The first of `outcome` and the outcomes it links, by any relation, depth first, whose code is
/// `code`.
fn anywhere<'a>(outcome: &'a Outcome, code: &str) -> Option<&'a Outcome> {
    if outcome.code().as_str() == code {
        return Some(outcome);
    }
    outcome.causes().iter().find_map(|link| anywhere(link.outcome(), code))
}

/// One failure of backends to open: its failing step, the first of the self-check's own found
/// down its causes, else its own code; why the capture did not open, of that step or else the
/// first cause that says; and why a missing MuMu app index is missing, when one is anywhere under
/// it.
fn open_failure(failure: &Outcome) -> OpenFailure {
    let step_of =
        found(failure, &|outcome: &Outcome| failed_step(outcome.code().as_str()).is_some());
    let step = step_of
        .and_then(|outcome| failed_step(outcome.code().as_str()))
        .unwrap_or_else(|| FailedStep::Other(failure.code().as_str().to_owned()));
    let capture = step_of.and_then(capture_cause).or_else(|| {
        found(failure, &|outcome: &Outcome| outcome.value(CAPTURE_OPEN_CAUSE).is_some())
            .and_then(capture_cause)
    });
    let app_index = anywhere(failure, NEMU_APP_INDEX_MISSING).map(app_index_problem);
    OpenFailure { step, capture, app_index }
}

fn app_index_problem(missing: &Outcome) -> AppIndexProblem {
    let word = missing
        .value(NEMU_APP_INDEX_PROBLEM)
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    match word {
        PACKAGE_NOT_INSTALLED => AppIndexProblem::PackageNotInstalled,
        APPLICATION_NOT_RUNNING => AppIndexProblem::ApplicationNotRunning,
        INFO_UNAVAILABLE => AppIndexProblem::InfoUnavailable,
        INFO_MALFORMED => AppIndexProblem::InfoMalformed,
        other => AppIndexProblem::Other(other.to_owned()),
    }
}

impl FenceFact {
    /// The instance's own backend open whose failure is the fence's cause: its touch's
    /// (`TouchOpen`), its capture's (`CaptureOpen`) or MuMu's own pair's (`NemuOpen`), as the
    /// self-check's failures spell them (#402 HS-5c, Runtime 0.12.40); `None` for any other
    /// cause. Other backends remedy such a fence: a switch of the instance's backends goes
    /// through it, and the new backends' passing self-check lifts it. A clear back to the file's
    /// values switches only when it changes a backend in effect: one back to the same values
    /// commits at once, with no switch, and the fence stays. A fence of any other cause still
    /// refuses a switch (`instance_fenced`), and a Runtime 0.12.39 or earlier, after a rollback,
    /// refuses one of every fenced instance.
    pub fn failed_open(&self) -> Option<FailedStep> {
        match self.cause.as_str() {
            TOUCH_OPEN => Some(FailedStep::TouchOpen),
            CAPTURE_OPEN => Some(FailedStep::CaptureOpen),
            PAIR_OPEN => Some(FailedStep::NemuOpen),
            _ => None,
        }
    }
}

fn failed_step(code: &str) -> Option<FailedStep> {
    match code {
        TOUCH_OPEN => Some(FailedStep::TouchOpen),
        CAPTURE_OPEN => Some(FailedStep::CaptureOpen),
        PAIR_OPEN => Some(FailedStep::NemuOpen),
        TOUCH_CHECK => Some(FailedStep::TouchCheck),
        CAPTURE_CHECK => Some(FailedStep::CaptureCheck),
        _ => None,
    }
}

fn capture_cause(outcome: &Outcome) -> Option<CaptureCause> {
    let word = outcome.value(CAPTURE_OPEN_CAUSE)?.as_str()?;
    Some(match word {
        DISPLAY_MISSING => CaptureCause::AppNotRunning,
        CONNECTION_FAILED => CaptureCause::Unreachable,
        _ => CaptureCause::Other,
    })
}
