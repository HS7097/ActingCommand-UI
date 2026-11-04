// SPDX-License-Identifier: GPL-3.0-only
//! An account's automatic rules (UI-3e1-b): what the main page's rules panel reads of one
//! instance's quiet times, idle action and emulator start, and the changes of settings it sends of
//! them. Nothing out of here names a key, a wire word or a wire type, but for the log.
//!
//! What it reads is all in the slow lane's one `RuntimeFactSnapshot`
//! ([`super::ReadLane::read_config`]), the rows the settings page reads too
//! ([`SettingsRead`]), no read of its own:
//!
//! - an instance's own blackout list (`instance.<instance id>.scheduling.blackout`) and the host's
//!   (`scheduling.blackout.host`), whose entries are read by the Runtime's own parser, the one its
//!   configuration check runs (BX-1, Runtime 0.12.39: `BlackoutEntry::parse_list`). That parser
//!   takes a list whole or refuses it whole: a Runtime that runs holds no list it refuses itself,
//!   so one that does not read here is of a newer Runtime, and is shown, never changed. With no
//!   row (the file names none and no online value is set) the instance has none of its own;
//! - its idle action (`instance.<instance id>.idle.action`), which every instance has a row of;
//! - its emulator start (`instance.<instance id>.start_emulator`), reported only of an instance
//!   that names it or that an online value sets: with no row it is off. Settable since 0.12.39
//!   (Workflow #429 R-3), and `true` only on an instance bound by discovery
//!   (`discovery_binding_required`); the daemon's start, the emulator watch and the idle wake read
//!   it in effect at each of their decisions.
//!
//! What it sends is one key of one instance in one `ConfigureSettings`, based on the snapshot's
//! version, as every change of settings here is ([`super::configure`]). A list goes whole: its
//! entries as [`BlackoutEntry::list_json`] writes them, so an entry this console did not make
//! goes back as the Runtime's parser read it, in the shape that parser reads (a `weekly` entry
//! written without `kind` comes back with it, which reads as the same entry), and nothing of the
//! entry's shape is spelled here. Before it goes, the list is read back by the same parser, and
//! its size is held against the contract's bound on one value, which the client would otherwise
//! refuse to send at all. The idle action is set only to nothing or to closing the game
//! ([`IdleChoice`]): stopping the emulator cannot be spelled here (Alice's ruling 157), and an
//! instance set to it in the file is shown so until a person picks one of the two. Each goes once,
//! whatever comes back.

use actingcommand_contract::{ConfigSettingChange, FactScalar, MAX_CONFIG_SETTING_VALUE_BYTES};
use actingcommand_runtime_config::scheduling::{BLACKOUT_HOST, INSTANCE_BLACKOUT};
use actingcommand_runtime_config::{
    idle, instance_key, scalar_json, BlackoutEntry, BlackoutListKey, BlackoutWindow,
    ConfigFailure, IdleAction, KeyName, ValueType, START_EMULATOR,
};

use super::{
    ConfigEdit, SettingSource, SettingsRead, ShownParameter, ShownValue, IDLE_CLOSE_APP,
    IDLE_NONE, IDLE_STOP_EMULATOR,
};

/// What [`SettingsRead`] keeps of a blackout list's row: its entries as the Runtime's parser reads
/// them, in their order, or why they do not read, for the log.
pub(super) type ListRead = Result<Vec<BlackoutEntry>, String>;

/// The entries of the row of `name`, of the instance `instance` when it is an instance's own,
/// whose value `value` reads as the key's type: `None` for a row of any other key.
pub(super) fn list_read(
    name: KeyName,
    instance: Option<&str>,
    value: &FactScalar,
) -> Option<ListRead> {
    let list = match (name, instance) {
        (KeyName::Exact(BLACKOUT_HOST), None) => BlackoutListKey::Host,
        (KeyName::PerInstance(INSTANCE_BLACKOUT), Some(instance)) => {
            BlackoutListKey::Instance(instance)
        }
        _ => return None,
    };
    let Some(json) = scalar_json(ValueType::List, value) else {
        return Some(Err(format!("{} is no list: {value:?}", list.config_key())));
    };
    Some(BlackoutEntry::parse_list(list, &json).map_err(|failure| failure_line(&failure)))
}

/// What the Runtime's parser refused of a blackout list: the key, the code, the problem, the
/// member it points to and the entry's id. For the log only: the Runtime keeps the pointer and
/// the id to its own configuration check, and its refusal of a change names neither.
fn failure_line(failure: &ConfigFailure) -> String {
    let mut line = format!(
        "{} does not pass the Runtime's blackout parser: {}",
        failure.config_key(),
        failure.failure_code().as_str(),
    );
    if let Some(problem) = failure.problem() {
        line.push_str(&format!(" ({})", problem.as_str()));
    }
    if let Some(field) = failure.field() {
        line.push_str(&format!(" at {field:?}"));
    }
    if let Some(id) = failure.blackout_id() {
        line.push_str(&format!(", entry {id:?}"));
    }
    line
}

/// An instance's own blackout list, its entries in the order read. The entries are the Runtime's
/// own type, which goes no further than this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietList {
    instance: String,
    entries: Vec<BlackoutEntry>,
}

/// One quiet time, as the panel shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietShown {
    /// Every week, on the days it starts on (`days[0]` Monday, through `days[6]` Sunday), from
    /// `start_minute` to `end_minute` of the day, both of a clock `utc_offset_minutes` east of UTC:
    /// an end at or before its start ends the next day, and one equal to its start makes a whole
    /// day.
    Weekly { days: [bool; 7], start_minute: u16, end_minute: u16, utc_offset_minutes: i16 },
    /// Once, from `from_ms` until `until_ms`, in unix ms.
    Once { from_ms: u64, until_ms: u64 },
}

/// A weekly quiet time a person adds: the days it starts on (`days[0]` Monday, through `days[6]`
/// Sunday) and its start and end minute of the day, of this computer's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewWeekly {
    pub days: [bool; 7],
    pub start_minute: u16,
    pub end_minute: u16,
}

/// Why a list was not sent: nothing went to the Runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuietNotSent {
    /// Written out, it is longer than the contract lets one value be.
    TooBig,
    /// The Runtime's parser does not take it, or does not read it back as it was made: most
    /// likely more entries than a list holds. Why, for the log only.
    NotTaken(String),
}

impl QuietList {
    /// The instance whose list it is.
    pub fn instance_id(&self) -> &str {
        &self.instance
    }

    /// Its entries as the panel shows them, in its order.
    pub fn entries(&self) -> impl Iterator<Item = QuietShown> + '_ {
        self.entries.iter().map(shown)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The list without its entry at `index`, every other one as it is; `None` when it has no
    /// such entry.
    pub fn without(&self, index: usize) -> Option<Self> {
        (index < self.entries.len()).then(|| {
            let mut entries = self.entries.clone();
            entries.remove(index);
            Self { instance: self.instance.clone(), entries }
        })
    }

    /// The list with `new` added at its end, of a clock `utc_offset_minutes` east of UTC, every
    /// other entry as it is; `None` when `new` names no day. Its id is `ui-` and the time it is
    /// made, `now_ms`, in base 36, with `-2`, `-3` and so on after it when the list has that id
    /// already. Whether the Runtime takes the list is [`ConfigEdit::quiet_times`]'s to find out.
    pub fn with_weekly(
        &self,
        new: &NewWeekly,
        utc_offset_minutes: i16,
        now_ms: u64,
    ) -> Option<Self> {
        let weekdays: Vec<u8> =
            (1u8..=7).zip(new.days).filter_map(|(day, on)| on.then_some(day)).collect();
        if weekdays.is_empty() {
            return None;
        }
        let base = format!("ui-{}", base36(now_ms));
        let mut id = base.clone();
        let mut next = 1u32;
        while self.entries.iter().any(|entry| entry.id == id) {
            next += 1;
            id = format!("{base}-{next}");
        }
        let mut entries = self.entries.clone();
        entries.push(BlackoutEntry {
            id,
            window: BlackoutWindow::Weekly {
                weekdays,
                utc_offset_minutes,
                start_minute_of_day: new.start_minute,
                end_minute_of_day: new.end_minute,
            },
        });
        Some(Self { instance: self.instance.clone(), entries })
    }
}

fn shown(entry: &BlackoutEntry) -> QuietShown {
    match &entry.window {
        BlackoutWindow::Weekly {
            weekdays,
            utc_offset_minutes,
            start_minute_of_day,
            end_minute_of_day,
        } => {
            // The parser takes days 1 to 7 only, Monday first.
            let mut days = [false; 7];
            for day in weekdays {
                if let Some(on) = usize::from(*day).checked_sub(1).and_then(|at| days.get_mut(at)) {
                    *on = true;
                }
            }
            QuietShown::Weekly {
                days,
                start_minute: *start_minute_of_day,
                end_minute: *end_minute_of_day,
                utc_offset_minutes: *utc_offset_minutes,
            }
        }
        BlackoutWindow::Once { from_unix_ms, until_unix_ms } => {
            QuietShown::Once { from_ms: *from_unix_ms, until_ms: *until_unix_ms }
        }
    }
}

/// `number` in base 36, lower case.
fn base36(mut number: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut digits = Vec::new();
    loop {
        digits.push(char::from(DIGITS[(number % 36) as usize]));
        number /= 36;
        if number == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

/// What an instance does when it has no task soon, as a person may set it here: never stopping
/// its emulator, which this console does not offer (Alice's ruling 157).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleChoice {
    /// `none`.
    Nothing,
    /// `close_app`: the game's application is stopped inside the emulator.
    CloseGame,
}

impl IdleChoice {
    fn action(self) -> IdleAction {
        match self {
            Self::Nothing => IdleAction::None,
            Self::CloseGame => IdleAction::CloseApp,
        }
    }
}

/// An instance's idle action in effect, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleNow {
    Nothing,
    CloseGame,
    /// `stop_emulator`: only shown. Either [`IdleChoice`] replaces it.
    StopEmulator,
    /// A word of the key's range this file has no name for, as written.
    Other(String),
}

impl IdleNow {
    fn of(word: &str) -> Self {
        match word {
            IDLE_NONE => Self::Nothing,
            IDLE_CLOSE_APP => Self::CloseGame,
            IDLE_STOP_EMULATOR => Self::StopEmulator,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// One of an instance's rules in effect: its value, where that comes from, and whether the
/// Runtime takes a change of it while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSetting<T> {
    pub value: T,
    pub source: SettingSource,
    pub settable: bool,
}

/// What a snapshot says of an instance's own quiet times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuietRead {
    /// Its list in effect: with no row an empty one, from its default, settable as the key
    /// table says.
    Read { list: QuietList, source: SettingSource, settable: bool },
    /// It does not read, so it is shown, never changed: its row does not (the settings' unread
    /// rows count it, `SettingsRead::unread`), a row or a section naming no key may be it
    /// (`None`), or the Runtime's parser does not take its entries (why, for the log).
    Unread(Option<String>),
}

impl SettingsRead {
    /// The quiet times of the instance of `instance_id`, its own list.
    pub fn quiet(&self, instance_id: &str) -> QuietRead {
        let key = BlackoutListKey::Instance(instance_id).config_key();
        if self.may_be_unread(&key) {
            return QuietRead::Unread(None);
        }
        let Some(shown) = self.parameters.get(&key) else {
            return QuietRead::Read {
                list: QuietList { instance: instance_id.to_owned(), entries: Vec::new() },
                source: SettingSource::Default,
                settable: self.unreported_settable(&key, KeyName::PerInstance(INSTANCE_BLACKOUT)),
            };
        };
        match self.quiet.get(&key) {
            Some(Ok(entries)) => QuietRead::Read {
                list: QuietList { instance: instance_id.to_owned(), entries: entries.clone() },
                source: shown.source,
                settable: shown.settable,
            },
            Some(Err(line)) => QuietRead::Unread(Some(line.clone())),
            // `take` reads a list's entries wherever it keeps its row.
            None => QuietRead::Unread(Some(format!("{key} has a row whose entries were not read"))),
        }
    }

    /// How many quiet times every instance has, the host's list: `Err` when the Runtime's parser
    /// does not take its entries, why for the log; `None` when its row does not read, or a row or
    /// a section naming no key may be it, or there is none.
    pub fn host_quiet(&self) -> Option<Result<usize, String>> {
        let key = BlackoutListKey::Host.config_key();
        if self.may_be_unread(&key) || !self.parameters.contains_key(&key) {
            return None;
        }
        self.quiet.get(&key).map(|read| read.as_ref().map(Vec::len).map_err(|line| line.clone()))
    }

    /// The idle action in effect of the instance of `instance_id`: every instance has a row of it.
    /// `None` when that does not read: its row does not, or a row or a section naming no key may
    /// be it, or there is none.
    pub fn idle(&self, instance_id: &str) -> Option<RuleSetting<IdleNow>> {
        let key = instance_key(instance_id, idle::ACTION);
        if self.may_be_unread(&key) {
            return None;
        }
        let shown = self.parameters.get(&key)?;
        let ShownValue::Word(word) = &shown.value else {
            return None;
        };
        Some(RuleSetting { value: IdleNow::of(word), source: shown.source, settable: shown.settable })
    }

    /// Whether the instance of `instance_id` has `start_emulator` on in effect, from where, and
    /// whether the Runtime takes a change of it. With no row it is off, from its default, and
    /// settable as `unreported_settable` reckons. `None` when that does not read, as
    /// [`SettingsRead::start_emulator`] has it.
    pub fn start_row(&self, instance_id: &str) -> Option<RuleSetting<bool>> {
        let key = instance_key(instance_id, START_EMULATOR);
        if self.may_be_unread(&key) {
            return None;
        }
        match self.parameters.get(&key) {
            Some(ShownParameter { value: ShownValue::Flag(on), source, settable }) => {
                Some(RuleSetting { value: *on, source: *source, settable: *settable })
            }
            Some(_) => None,
            None => Some(RuleSetting {
                value: false,
                source: SettingSource::Default,
                settable: self.unreported_settable(&key, KeyName::PerInstance(START_EMULATOR)),
            }),
        }
    }

    /// Whether the row of `key` may be among those that do not read: one of that key is, or one
    /// that names no key, or a section that is no list of rows.
    fn may_be_unread(&self, key: &str) -> bool {
        self.unread.iter().any(|row| row.key().map_or(true, |unread| unread == key))
    }
}

impl ConfigEdit {
    /// The instance's own blackout list as `list` has it, as an online value: its entries as the
    /// Runtime's parser reads them, each written as that parser reads it back. Not when the parser
    /// does not take it as made, or it is longer than one value may be: nothing is sent then.
    pub fn quiet_times(list: &QuietList) -> Result<Self, QuietNotSent> {
        let key = BlackoutListKey::Instance(&list.instance);
        let value = BlackoutEntry::list_json(&list.entries);
        match BlackoutEntry::parse_list(key, &value) {
            Ok(read) if read == list.entries => {}
            Ok(_) => {
                return Err(QuietNotSent::NotTaken(format!(
                    "{} reads back as other entries than were made",
                    key.config_key(),
                )));
            }
            Err(failure) => return Err(QuietNotSent::NotTaken(failure_line(&failure))),
        }
        // The contract's bound is on the value written out compactly, as `Display` writes it.
        if value.to_string().len() > MAX_CONFIG_SETTING_VALUE_BYTES {
            return Err(QuietNotSent::TooBig);
        }
        Ok(Self(vec![ConfigSettingChange { key: key.config_key(), value: Some(value) }]))
    }

    /// The instance's own blackout list set online to none at all: an empty list, whatever the
    /// file names.
    pub fn quiet_times_none(instance_id: &str) -> Self {
        let key = BlackoutListKey::Instance(instance_id).config_key();
        Self(vec![ConfigSettingChange { key, value: Some(BlackoutEntry::list_json(&[])) }])
    }

    /// The instance's own blackout list back to the configuration file's: the online one is
    /// dropped, which leaves none of its own when the file names none.
    pub fn quiet_times_back_to_file(instance_id: &str) -> Self {
        let key = BlackoutListKey::Instance(instance_id).config_key();
        Self(vec![ConfigSettingChange { key, value: None }])
    }

    /// The instance's idle action, as an online value.
    pub fn idle_action(instance_id: &str, choice: IdleChoice) -> Self {
        Self(vec![ConfigSettingChange {
            key: instance_key(instance_id, idle::ACTION),
            value: Some(choice.action().as_str().into()),
        }])
    }

    /// The instance's idle action back to the configuration file's, or to the section's when the
    /// file names none of its own.
    pub fn idle_action_back_to_file(instance_id: &str) -> Self {
        Self(vec![ConfigSettingChange { key: instance_key(instance_id, idle::ACTION), value: None }])
    }

    /// Whether the Runtime starts and watches the instance's emulator, as an online value. On
    /// only for an instance bound by discovery: the Runtime refuses it of any other
    /// (`discovery_binding_required`).
    pub fn start_emulator(instance_id: &str, on: bool) -> Self {
        Self(vec![ConfigSettingChange {
            key: instance_key(instance_id, START_EMULATOR),
            value: Some(on.into()),
        }])
    }

    /// The same back to the configuration file's value, or to off when the file names none.
    pub fn start_emulator_back_to_file(instance_id: &str) -> Self {
        Self(vec![ConfigSettingChange {
            key: instance_key(instance_id, START_EMULATOR),
            value: None,
        }])
    }
}
