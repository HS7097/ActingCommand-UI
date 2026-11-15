// SPDX-License-Identifier: GPL-3.0-only
//! An instance's goals (Workflow #455, the goal page; Workflow #463 V3, read only): what the Runtime
//! says of one application's goal page, read with `GoalView` ([`super::ReadLane::read_goals`],
//! [`GoalsRead`]), and the lines the console's Goals tab shows of it, a pure function of that page,
//! of the language shown and of the packages' display names ([`page`]). Nothing here writes, and
//! nothing out of here names a wire type: an answer is kept whole ([`GoalsLoaded`]) until it is
//! shown, because its words depend on the language and on the names `actingctl packs` gives, which
//! come and go on their own.
//!
//! What the page says, all at the pinned Runtime (5c629e3b; `contracts/runtime-interface.md`,
//! "Goal request kinds"; `crates/actingcommand-contract/src/runtime/goals.rs`):
//!
//! - `declaration.state`, one of eight words. Three of them list nothing at all, whatever is
//!   stored (`crates/runtime-host/src/host/goals.rs:140-177`): `not_checked_yet` (not in the
//!   Runtime's memory yet; it passes by itself), `root_failing` (the game root cannot be read,
//!   with the file it failed on in `path`) and `no_schedule` (the game has no standard pack root
//!   with `schedule.json`). The other five list the page: `accepted`; `absent` (the game root has
//!   no `goals.json`); `refused` and `unreadable` (the file is not applied: none of its entries
//!   shows, the bound packages' variables still do); `not_read` (no workbench reads game roots,
//!   `host/goals.rs:579-588`).
//! - `entries`, in the Runtime's order: those of `goals.json` in file order, then each variable no
//!   entry places, in the order the bound packages declare them, a shared one once
//!   (`crates/runtime-host/src/goals.rs:644-758`). A `heading` and a `note` with their texts; a
//!   `resource` entry with its pool, the target on that pool (its `{at_least}` as the value, who
//!   set it when the record says, `:768-811`: a target a v2 document wrote names no setter) and the
//!   pool's reading; a `tier` with its default, `not_built` in this build; a variable with its
//!   declaration as its package writes it, its default (`default_json`), the value the next run
//!   reads (`effective_json`), who set the stored value (`set`), whether it is in effect (`state`,
//!   with the `variable_problem` word when it is not) and the stored keys of an `ordered` value the
//!   run leaves out (`removed_keys`, with `keys_left_out` while `in_effect`), and the effective
//!   value in the declaration's own texts (`display`: an option's title, a number with its unit's
//!   text, `:1017-1038`). A placement no bound package declares is `not_offered`.
//! - `orphans`, each stored value no entry shows any more; `other_targets`, the ids of the stored
//!   targets on no declared resource entry's pool, whose targets are read from `targets.active`;
//!   and `targets`, the whole `ResourceTargetView`, whose `not_targetable` pools close the page.
//!
//! Texts are written per locale; the contract gives no fallback, so the console's own is taken
//! ([`in_language`], the one `ui.json`'s titles go by): Chinese tries `zh-Hans`, `zh` and `zh-CN`,
//! then `en`; English tries `en`; either then takes the first text written. Variables no entry
//! places go in groups by the display name of their first package; every one whose package has
//! none goes in one last group ([`GoalLine::Package`]). No entry's name, no declared pool's id and
//! no package id is shown.
//!
//! A pool "without a step" is one a goal names only with a step or an importance of its own, which
//! the pack does not give: `scale_required || importance_required`, exactly what `EditGoal` refuses
//! `pool_unvalued` (`crates/runtime-host/src/goals.rs:540-549`) and what
//! `ResourceTargetView::validate` ties to a missing `default_scale` or `default_importance_milli`.
//! An `integer` variable whose `unit` is a time word holds milliseconds; it is shown in the largest
//! unit at most the declared one in which it is a whole number, down to seconds, and in
//! milliseconds when it is no whole number of seconds: never a fraction ([`whole_time`]).
//!
//! The stored record's form is `targets.active.schema_version`: `v2`, one validity for the whole
//! document; `v3`, the record `EditGoal` writes, each target with a lifetime of its own or none and
//! the document with none (`contracts/resource-targets.md:1054-1059`); `v1`, an agent's old form,
//! read only; any other, a form newer than this console, whose targets it does not interpret. A
//! target lapsed (the document expired, or its own lifetime is before the view's time) counts no
//! more, so it is never shown as met.
//!
//! The page takes the policy outcome gate, the policy and the fact write gate in turn while it is
//! computed (`crates/runtime-host/src/host/goals.rs:11-18`): the caller reads it seldom, as it
//! reads the task switches. A Runtime that does not know the request drops the connection, and the
//! client then says `runtime_operation_unsupported` (`crates/runtime-client/src/client.rs:1762-1787`);
//! that, or a refusal `not_implemented`, is an answer here, that this Runtime has no goals to read
//! ([`GoalsAnswer::Unsupported`]).

use std::collections::{BTreeMap, BTreeSet};

use acui_rows::outcome::Tongue;
use actingcommand_contract::{
    ActiveResourceTargets, GoalDeclarationRead, GoalEntryState, GoalEntryType, GoalOrphan,
    GoalSetBy, GoalValueSource, GoalView, GoalViewEntry, Initiator, ResourceTargetCondition,
    ResourceTargetConditionState, ResourceTargetObservation, ResourceTargetPendingReason,
    ResourceTargetView, ResourceTargetsRejectionReason, RuntimeChannel,
};
use serde_json::Value;

/// The record versions the view serves (`contracts/resource-targets.md:62-64`, `:1054-1059`).
const SCHEMA_V1: &str = "actingcommand.resource-targets.v1";
const SCHEMA_V2: &str = "actingcommand.resource-targets.v2";
const SCHEMA_V3: &str = "actingcommand.resource-targets.v3";
/// The one condition a target states (`TargetCondition::AtLeast`, tagged by `kind`).
const AT_LEAST: &str = "at_least";
/// The `variable_problem` word of an `ordered` value applied in part, while `in_effect`.
const KEYS_LEFT_OUT: &str = "keys_left_out";

/// One answered `GoalView` of one instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalsRead {
    pub epoch: String,
    /// The alias asked for: the client refuses an answer that names another.
    pub alias: String,
    pub answer: GoalsAnswer,
}

/// What the view said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalsAnswer {
    /// This Runtime has no such read: it refused it `not_implemented`, or does not know the request
    /// (the client's `runtime_operation_unsupported`). `code` is which of the two.
    Unsupported { code: &'static str },
    Read(GoalsLoaded),
}

/// One goal page as the Runtime answered it, kept whole until it is shown ([`Self::page`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalsLoaded(Box<GoalView>);

impl GoalsLoaded {
    /// The page's lines in `tongue`, the groups of variables no entry places titled by
    /// `pack_title`, the display name of a package by its id ([`page`]).
    pub fn page(&self, tongue: Tongue, pack_title: &dyn Fn(&str) -> Option<String>) -> GoalsPage {
        page(&self.0, tongue, pack_title)
    }
}

/// The Goals tab of one instance, as the page leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalsPage {
    /// When the Runtime computed the resource target view the page holds, in unix ms.
    pub evaluated_at_ms: u64,
    /// The goal record's version (`GoalView.version`, `0` without a record), an opaque token: the
    /// `expected_version` of a later write, and for the log.
    pub version: u64,
    pub state: PageState,
    /// Top to bottom; none unless the page is listed.
    pub lines: Vec<GoalLine>,
    /// The stored targets' document, whose form and validity go under the lines.
    pub stored: Stored,
    /// No catalog is active: no pool is targetable or not.
    pub no_catalog: bool,
    /// The Runtime reads the instance's stored record as no document (`stale_schema`).
    pub stale_schema: bool,
}

/// Whether the page lists its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageState {
    /// It does, and how the game's `goals.json` stands. `nothing`: it lists no entry, no value
    /// no longer offered and no other target, so this instance has nothing to set now.
    Listed { declaration: Declaration, nothing: bool },
    /// `not_checked_yet`: not in the Runtime's memory yet; it passes by itself.
    NotCheckedYet,
    /// `root_failing`: the game root cannot be read and must be repaired; the file it failed on,
    /// when the Runtime names one.
    RootFailing { path: Option<String> },
    /// `no_schedule`: the game has no standard pack with a schedule, so nothing to set.
    NoSchedule,
}

/// How the game's `goals.json` stands, on a page that lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declaration {
    /// `accepted`: its entries are the page's.
    Accepted,
    /// `absent`: there is none; the page is the packages' variables.
    Absent,
    /// `refused` or `unreadable`: there is one, and it is not applied; the page is the packages'
    /// variables.
    NotTaken,
    /// `not_read`: this Runtime reads no game root's declaration; the page is the packages'
    /// variables.
    NotRead,
}

/// One line of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalLine {
    /// A `heading` of `goals.json`: a group's title, in the language shown.
    Heading(Option<String>),
    /// A `note`: its text and its help, in grey.
    Note { text: Option<String>, help: Option<String> },
    Resource(ResourceLine),
    Tier(TierLine),
    Variable(VariableLine),
    /// The title of a group of variables no entry places: the display name of their first
    /// package, or none for the one group of every variable whose package has none.
    Package(Option<String>),
    /// The title of one of the three groups that close the page, before its lines.
    Group(Group),
    Orphan(OrphanLine),
    OtherTarget(OtherTarget),
    /// A pool no goal can name, and why: neither a declared entry's nor an other target's.
    Closed { pool: String, why: Closed },
}

/// The groups that close the page, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Stored values no entry offers any more (`orphans`).
    NoLongerOffered,
    /// Stored targets on no declared resource entry's pool (`other_targets`).
    OtherTargets,
    /// Pools no goal can name (`targets.not_targetable`).
    NotTargetable,
}

/// A `resource` entry: its title, whether its pool takes a goal here, the pool's reading and the
/// goal stored on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLine {
    pub title: Option<String>,
    pub pool: PoolState,
    /// None while the pool is not targetable.
    pub reading: Option<Reading>,
    /// None: no goal, the default.
    pub goal: Option<StoredGoal>,
    /// Who set it; none when the record names nobody (a target a v2 document wrote).
    pub set: Option<SetBy>,
}

/// What the resource target view says of a pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolState {
    /// Targetable, and the pack gives it both a step and an importance.
    Open,
    /// Targetable only with a step or an importance of its own, which the pack does not give.
    NoStep,
    /// Not targetable now, and why.
    Closed(Closed),
    /// Listed neither way: no catalog is active, or the active one has no such pool.
    Unlisted,
}

/// A goal stored on a pool: its amount, how far it is and whether it still counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGoal {
    /// `at_least`; none when the value does not read as one.
    pub amount: Option<u64>,
    pub progress: Progress,
    /// The target's own lifetime, in unix ms: a v3 target's that has one.
    pub until_ms: Option<u64>,
    /// It counts no more: the document expired, or its own lifetime has passed.
    pub lapsed: bool,
}

/// A pool's current amount as the Runtime reads it for the instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// The amount, and when it was observed, in unix ms.
    Read { current: i64, at_ms: u64 },
    /// No usable reading, and why; the view always says why (`ResourceTargetView::validate`), and
    /// one that does not is shown as no usable reading.
    Pending(Option<Pending>),
}

/// Why a reading is not usable, the contract's closed set (`ResourceTargetPendingReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Missing,
    Expired,
    LowConfidence,
    InvalidValue,
}

/// How far a stored goal is, by the view's condition of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// `computed`: the amount read, when, and how much is still missing; met at 0.
    Computed { current: i64, at_ms: u64, gap: u64 },
    /// `awaiting_observation`: no usable reading yet, and why.
    Awaiting(Pending),
    /// The view gives it no condition: its pool no longer resolves for the instance.
    NoCondition,
}

impl Progress {
    /// Met: a usable reading at or above the amount.
    pub fn met(self) -> bool {
        matches!(self, Self::Computed { gap: 0, .. })
    }
}

/// Why no goal can name a pool, in the console's words where it has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    /// `unmapped_task`: no task of the instance produces it.
    UnmappedTask,
    /// `resource_not_observable`: no `resource.` or `inventory.` fact reads it.
    NotObservable,
    /// `resource_out_of_scope`: the pool is not in the instance's scope.
    OutOfScope,
    /// Any other reason of the closed set, by its code.
    Other(String),
}

/// A `tier` entry, read only: no reader of this build reads a tier's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierLine {
    pub title: Option<String>,
    /// The default option's title (its value when it has none).
    pub default: Option<String>,
}

/// A package variable, placed by an entry or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableLine {
    /// The entry's title, or else the variable's.
    pub title: Option<String>,
    /// The value the next run reads, in words; for a variable not offered, the value stored under
    /// its name, which nothing reads; none when there is neither.
    pub value: Option<Shown>,
    /// The pack's default, while a value set is in effect instead.
    pub default: Option<Shown>,
    /// Who set the value stored under its name, when one is.
    pub set: Option<SetBy>,
    pub state: VariableState,
    /// The stored keys of an `ordered` value the run leaves out (`keys_left_out`), as stored.
    pub left_out: Vec<String>,
}

/// Whether a variable's value is in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableState {
    /// The next run reads [`VariableLine::value`].
    InEffect,
    /// The next run reads the default instead: of a value stored that it does not take
    /// (`stored`), or of a variable that does not apply in the game root; why, by the
    /// `variable_problem` word.
    NotApplied { stored: bool, why: Option<Unapplied> },
    /// No package bound on the instance declares it.
    NotOffered,
}

/// Why a run does not read a variable's value: the `variable_problem` vocabulary (Runtime
/// `contracts/outcome-codes/catalog.json:9733-9792`), one word each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unapplied {
    NotAnOption,
    OutOfRange,
    OffStep,
    TypeChanged,
    KeysRemoved,
    CountOutOfRange,
    KeyRepeated,
    KeysLeftOut,
    EntryUnreadable,
    IrreversibleByRoot,
    PackageUnreadable,
    GoalsUnreadable,
    DeclarationConflict,
    GoalNameConflict,
    NameOutOfLimits,
    CapRange,
    RootUnchecked,
    /// A word of a newer Runtime, by itself.
    Other(String),
}

impl Unapplied {
    fn of(word: &str) -> Self {
        match word {
            "not_an_option" => Self::NotAnOption,
            "out_of_range" => Self::OutOfRange,
            "off_step" => Self::OffStep,
            "type_changed" => Self::TypeChanged,
            "keys_removed" => Self::KeysRemoved,
            "count_out_of_range" => Self::CountOutOfRange,
            "key_repeated" => Self::KeyRepeated,
            KEYS_LEFT_OUT => Self::KeysLeftOut,
            "entry_unreadable" => Self::EntryUnreadable,
            "irreversible_by_root" => Self::IrreversibleByRoot,
            "package_unreadable" => Self::PackageUnreadable,
            "goals_unreadable" => Self::GoalsUnreadable,
            "declaration_conflict" => Self::DeclarationConflict,
            "goal_name_conflict" => Self::GoalNameConflict,
            "name_out_of_limits" => Self::NameOutOfLimits,
            "cap_range" => Self::CapRange,
            "root_unchecked" => Self::RootUnchecked,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// A value in words, as far as this file can make them without the console's tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// The declaration's own words: an option's title, a number with its unit's text; or a text
    /// value of no declaration, as it is.
    Words(String),
    /// A whole number without a unit.
    Number(i64),
    /// A time, `amount` of `unit` ([`whole_time`]).
    Time { amount: i64, unit: TimeUnit },
    /// On or off.
    Switch(bool),
    /// An `ordered` value: each key's title (the key where the declaration gives none), in its
    /// order; empty, none chosen.
    List(Vec<String>),
    /// A value that does not read as its declaration's type, or no JSON at all: its text.
    Raw(String),
}

/// The units a time is shown in: the declaration's four words, and milliseconds below them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
}

impl TimeUnit {
    /// From the largest down.
    const LADDER: [Self; 5] = [Self::Day, Self::Hour, Self::Minute, Self::Second, Self::Millisecond];

    const fn milliseconds(self) -> i64 {
        match self {
            Self::Millisecond => 1,
            Self::Second => 1_000,
            Self::Minute => 60_000,
            Self::Hour => 3_600_000,
            Self::Day => 86_400_000,
        }
    }

    /// The declaration's `unit` word (`GoalTimeUnit`).
    fn of(word: &str) -> Option<Self> {
        match word {
            "second" => Some(Self::Second),
            "minute" => Some(Self::Minute),
            "hour" => Some(Self::Hour),
            "day" => Some(Self::Day),
            _ => None,
        }
    }
}

/// `milliseconds` in the largest unit at most `unit` of which it is a whole number, down to
/// seconds; in milliseconds when it is no whole number of seconds. Never a fraction: 90 minutes
/// declared in hours is 90 minutes.
pub fn whole_time(milliseconds: i64, unit: TimeUnit) -> (i64, TimeUnit) {
    TimeUnit::LADDER
        .iter()
        .skip_while(|step| **step != unit)
        .find(|step| milliseconds % step.milliseconds() == 0)
        .map_or((milliseconds, TimeUnit::Millisecond), |step| {
            (milliseconds / step.milliseconds(), *step)
        })
}

/// Who set a stored value, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetBy {
    /// The Runtime's clock at the request, in unix ms.
    pub at_ms: u64,
    pub by: Setter,
}

/// The channel of the connection that set it (`set.channel`), and on the Lab's the entry it
/// declared (`set.initiator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setter {
    /// `ui`: this console (which declares no initiator).
    Console,
    /// `cli`: `actingctl`, and the installer and scripts that run it.
    CommandLine,
    /// `mcp`: `actingctl mcp-serve`.
    Mcp,
    /// `lab`, and which of its entries; none when it declared none.
    Lab(Option<LabEntry>),
}

/// The Lab's entries an initiator names (`Initiator`, but `UI`, which the Lab never declares).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabEntry {
    /// `Lab-CLI`.
    CommandLine,
    /// `Lab-MCP`.
    Mcp,
    /// `Lab-制包`.
    Authoring,
    /// `Lab-救急`.
    Rescue,
    /// `Lab-看门狗`.
    Watchdog,
}

/// A stored value no entry offers any more: no run reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanLine {
    pub value: Shown,
    pub set: Option<SetBy>,
}

/// A stored target on no declared resource entry's pool, by its id: as the stored document holds
/// it, or none when it does not read here (no such target, one that is no `at_least` target with
/// a resource and a whole amount, or a document of a newer form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherTarget {
    pub id: String,
    pub target: Option<OtherGoal>,
}

/// An other target that reads: its pool (the only name it has), what the view says of that pool,
/// the pool's reading, and the goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherGoal {
    pub pool: String,
    pub state: PoolState,
    pub reading: Option<Reading>,
    pub goal: StoredGoal,
}

/// The instance's stored targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// None is stored, the last one withdrew every target, or the record holds entries only.
    None,
    Document(Document),
}

/// A stored document: its form, its validity and how many targets it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub form: Form,
    /// The document's own validity: a v1 or v2 document's; a v3 one has none.
    pub valid_until_ms: Option<u64>,
    /// v1 or v2: past its validity; v3: every target's own lifetime has passed.
    pub expired: bool,
    pub targets: usize,
}

/// Which form a stored document is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// v1: written by an agent in the old form, read only.
    Old,
    /// v2: one validity for the whole document.
    Whole,
    /// v3: the record goals are set by name in, each target with a lifetime of its own or none.
    PerTarget,
    /// Any other `schema_version`, kept for the log: a newer form, whose targets are not read.
    Newer(String),
}

/// The page of `alias` as the console keeps it.
pub(super) fn goals_read(epoch: String, alias: &str, view: GoalView) -> GoalsRead {
    let answer = GoalsAnswer::Read(GoalsLoaded(Box::new(view)));
    GoalsRead { epoch, alias: alias.to_owned(), answer }
}

/// A Runtime's answer that it has no such read, by the code that says so
/// (`super::pack::view_unsupported`).
pub(super) fn goals_unsupported(epoch: String, alias: &str, code: &'static str) -> GoalsRead {
    GoalsRead { epoch, alias: alias.to_owned(), answer: GoalsAnswer::Unsupported { code } }
}

/// The text of `texts`, each per locale in the order written, in `tongue`: Chinese tries `zh-Hans`,
/// `zh` and `zh-CN`, then `en`; English tries `en`; either then takes the first written. The one
/// order of the console, `ui.json`'s titles' too.
pub fn in_language(texts: &[(String, String)], tongue: Tongue) -> Option<&str> {
    let first: &[&str] = match tongue {
        Tongue::ZhHans => &["zh-Hans", "zh", "zh-CN"],
        Tongue::En => &[],
    };
    first
        .iter()
        .find_map(|locale| text_in(texts, locale))
        .or_else(|| text_in(texts, "en"))
        .or_else(|| texts.first().map(|(_, text)| text.as_str()))
}

fn text_in<'a>(texts: &'a [(String, String)], locale: &str) -> Option<&'a str> {
    texts.iter().find(|(name, _)| name == locale).map(|(_, text)| text.as_str())
}

/// The page's lines of `view` in `tongue`, each group of variables no entry places titled by
/// `pack_title` (see the module's words).
pub fn page(view: &GoalView, tongue: Tongue, pack_title: &dyn Fn(&str) -> Option<String>) -> GoalsPage {
    let targets = view.targets.as_ref();
    let active = targets.active.as_ref();
    let form = active.map(|active| form_of(&active.schema_version));
    let state = match view.declaration.state {
        GoalDeclarationRead::NotCheckedYet => PageState::NotCheckedYet,
        GoalDeclarationRead::RootFailing => {
            PageState::RootFailing { path: view.declaration.path.clone() }
        }
        GoalDeclarationRead::NoSchedule => PageState::NoSchedule,
        GoalDeclarationRead::Accepted => listed(view, Declaration::Accepted),
        GoalDeclarationRead::Absent => listed(view, Declaration::Absent),
        GoalDeclarationRead::Refused | GoalDeclarationRead::Unreadable => {
            listed(view, Declaration::NotTaken)
        }
        GoalDeclarationRead::NotRead => listed(view, Declaration::NotRead),
    };
    let lines = match state {
        PageState::Listed { .. } => {
            let read = Targets { view: targets, active, form: form.as_ref() };
            lines(view, &read, tongue, pack_title)
        }
        PageState::NotCheckedYet | PageState::RootFailing { .. } | PageState::NoSchedule => {
            Vec::new()
        }
    };
    let stored = match (active, form) {
        (Some(active), Some(form)) => Stored::Document(Document {
            valid_until_ms: active.valid_until_unix_ms,
            expired: active.expired,
            targets: active.targets.len(),
            form,
        }),
        (None, _) | (Some(_), None) => Stored::None,
    };
    GoalsPage {
        evaluated_at_ms: targets.evaluated_at_unix_ms,
        version: view.version,
        state,
        lines,
        stored,
        no_catalog: targets.catalog_hash.is_none(),
        stale_schema: targets.stale_schema == Some(true),
    }
}

fn listed(view: &GoalView, declaration: Declaration) -> PageState {
    let nothing =
        view.entries.is_empty() && view.orphans.is_empty() && view.other_targets.is_empty();
    PageState::Listed { declaration, nothing }
}

/// What the lines read of the resource target view.
struct Targets<'a> {
    view: &'a ResourceTargetView,
    active: Option<&'a ActiveResourceTargets>,
    form: Option<&'a Form>,
}

impl Targets<'_> {
    /// What the view says of `pool`.
    fn pool(&self, pool: &str) -> PoolState {
        if let Some(open) = self.view.targetable.iter().find(|open| open.resource == pool) {
            return if open.scale_required || open.importance_required {
                PoolState::NoStep
            } else {
                PoolState::Open
            };
        }
        match self.view.not_targetable.iter().find(|closed| closed.resource == pool) {
            Some(closed) => PoolState::Closed(closed_of(closed.reason)),
            None => PoolState::Unlisted,
        }
    }

    /// The reading of `pool`, while it is targetable.
    fn reading(&self, pool: &str) -> Option<Reading> {
        self.view
            .targetable
            .iter()
            .find(|open| open.resource == pool)
            .map(|open| reading(&open.observation))
    }

    /// The stored target of `id` in document form, while the document is of a form read here.
    fn target(&self, id: &str) -> Option<&Value> {
        if matches!(self.form, Some(Form::Newer(_))) {
            return None;
        }
        self.active?
            .targets
            .iter()
            .find(|target| target.get("id").and_then(Value::as_str) == Some(id))
    }

    /// The goal of the target `id` of `amount`: how far it is, its own lifetime and whether it
    /// still counts.
    fn goal(&self, id: Option<&str>, amount: Option<u64>) -> StoredGoal {
        let conditions: &[ResourceTargetCondition] = match self.active {
            Some(active) => &active.conditions,
            None => &[],
        };
        let progress = id.map_or(Progress::NoCondition, |id| progress(conditions, id));
        let until_ms = match self.form {
            Some(Form::PerTarget) => id
                .and_then(|id| self.target(id))
                .and_then(|target| target.get("valid_until_unix_ms"))
                .and_then(Value::as_u64),
            Some(_) | None => None,
        };
        let expired = self.active.is_some_and(|active| active.expired);
        let lapsed =
            expired || until_ms.is_some_and(|until| until < self.view.evaluated_at_unix_ms);
        StoredGoal { amount, progress, until_ms, lapsed }
    }
}

/// The lines of a page that lists: its entries in their order, each group of the variables no
/// entry places, then the three groups that close it, each under its title while it has a line.
fn lines(
    view: &GoalView,
    targets: &Targets<'_>,
    tongue: Tongue,
    pack_title: &dyn Fn(&str) -> Option<String>,
) -> Vec<GoalLine> {
    let mut lines = Vec::new();
    // By the first package's id, in the order the page first names it, with its display name.
    let mut groups: Vec<(&str, String, Vec<&GoalViewEntry>)> = Vec::new();
    let mut unnamed: Vec<&GoalViewEntry> = Vec::new();
    for entry in &view.entries {
        if entry.placed {
            lines.push(entry_line(entry, targets, tongue));
            continue;
        }
        let first = entry.packages.first().map(String::as_str);
        match first.and_then(|package| pack_title(package).map(|title| (package, title))) {
            Some((package, title)) => {
                match groups.iter().position(|(id, _, _)| *id == package) {
                    Some(index) => groups[index].2.push(entry),
                    None => groups.push((package, title, vec![entry])),
                }
            }
            None => unnamed.push(entry),
        }
    }
    for (_, title, entries) in groups {
        lines.push(GoalLine::Package(Some(title)));
        lines.extend(entries.into_iter().map(|entry| entry_line(entry, targets, tongue)));
    }
    if !unnamed.is_empty() {
        lines.push(GoalLine::Package(None));
        lines.extend(unnamed.into_iter().map(|entry| entry_line(entry, targets, tongue)));
    }
    if !view.orphans.is_empty() {
        lines.push(GoalLine::Group(Group::NoLongerOffered));
        lines.extend(view.orphans.iter().map(orphan_line));
    }
    let others: Vec<OtherTarget> =
        view.other_targets.iter().map(|id| other_target(id, targets)).collect();
    // A pool a declared entry or an other target names says why on that line, not on one of its
    // own.
    let mut named: BTreeSet<&str> = view
        .entries
        .iter()
        .filter_map(|entry| entry.resource.as_ref())
        .map(|resource| resource.pool.as_str())
        .collect();
    named.extend(
        others.iter().filter_map(|other| other.target.as_ref()).map(|goal| goal.pool.as_str()),
    );
    let closed: Vec<GoalLine> = targets
        .view
        .not_targetable
        .iter()
        .filter(|closed| !named.contains(closed.resource.as_str()))
        .map(|closed| GoalLine::Closed {
            pool: closed.resource.clone(),
            why: closed_of(closed.reason),
        })
        .collect();
    if !others.is_empty() {
        lines.push(GoalLine::Group(Group::OtherTargets));
        lines.extend(others.into_iter().map(GoalLine::OtherTarget));
    }
    if !closed.is_empty() {
        lines.push(GoalLine::Group(Group::NotTargetable));
        lines.extend(closed);
    }
    lines
}

/// One entry of the page, by its type.
fn entry_line(entry: &GoalViewEntry, targets: &Targets<'_>, tongue: Tongue) -> GoalLine {
    let declared = entry.entry.as_ref();
    match entry.entry_type {
        GoalEntryType::Heading => GoalLine::Heading(text_of(declared, "title", tongue)),
        GoalEntryType::Note => GoalLine::Note {
            text: text_of(declared, "title", tongue),
            help: text_of(declared, "help", tongue),
        },
        GoalEntryType::Resource => GoalLine::Resource(resource_line(entry, targets, tongue)),
        GoalEntryType::Tier => GoalLine::Tier(TierLine {
            title: text_of(declared, "title", tongue),
            default: tier_default(declared, tongue),
        }),
        GoalEntryType::Variable => GoalLine::Variable(variable_line(entry, tongue)),
    }
}

/// A `resource` entry: its pool as the view says it, and the target on it.
fn resource_line(entry: &GoalViewEntry, targets: &Targets<'_>, tongue: Tongue) -> ResourceLine {
    let pool = entry.resource.as_ref().map(|resource| resource.pool.as_str());
    let goal = entry.value_json.as_deref().map(|json| {
        let id = entry.resource.as_ref().and_then(|resource| resource.target_id.as_deref());
        targets.goal(id, at_least(json))
    });
    ResourceLine {
        title: text_of(entry.entry.as_ref(), "title", tongue),
        pool: pool.map_or(PoolState::Unlisted, |pool| targets.pool(pool)),
        reading: pool.and_then(|pool| targets.reading(pool)),
        goal,
        set: entry.set.as_ref().map(set_by),
    }
}

/// The `at_least` of a resource entry's value `{"at_least": n}`.
fn at_least(json: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(json).ok()?;
    value.get(AT_LEAST)?.as_u64()
}

/// A `tier` entry's default option's title, or its value.
fn tier_default(declared: Option<&Value>, tongue: Tongue) -> Option<String> {
    let declared = declared?;
    let default = declared.get("default")?.as_str()?;
    let option = declared
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|option| option.get("value").and_then(Value::as_str) == Some(default));
    Some(text_of(option, "title", tongue).unwrap_or_else(|| default.to_owned()))
}

/// A variable's line.
fn variable_line(entry: &GoalViewEntry, tongue: Tongue) -> VariableLine {
    let declaration = Declared::of(entry.variable.as_ref());
    let title = text_of(entry.entry.as_ref(), "title", tongue)
        .or_else(|| text_of(entry.variable.as_ref(), "title", tongue));
    let state = match entry.state {
        Some(GoalEntryState::NotOffered) => VariableState::NotOffered,
        Some(GoalEntryState::NotApplied) => VariableState::NotApplied {
            stored: entry.value_json.is_some(),
            why: entry.variable_problem.as_deref().map(Unapplied::of),
        },
        Some(GoalEntryState::InEffect) | Some(GoalEntryState::NotBuilt) | None => {
            VariableState::InEffect
        }
    };
    let display = entry.display.as_ref().map(display_texts);
    let display = display.as_deref().and_then(|display| in_language(display, tongue));
    let value = match (&entry.effective_json, &state) {
        (Some(json), _) => Some(declaration.shown(json, display, tongue)),
        (None, VariableState::NotOffered) => {
            entry.value_json.as_deref().map(|json| declaration.shown(json, None, tongue))
        }
        (None, VariableState::InEffect | VariableState::NotApplied { .. }) => None,
    };
    let default = match entry.source {
        Some(GoalValueSource::Set) => {
            entry.default_json.as_deref().map(|json| declaration.shown(json, None, tongue))
        }
        Some(GoalValueSource::Default) | None => None,
    };
    VariableLine {
        title,
        value,
        default,
        set: entry.set.as_ref().map(set_by),
        state,
        left_out: entry.removed_keys.clone(),
    }
}

/// A declaration's options: each value with its title per locale.
type Options = Vec<(String, Vec<(String, String)>)>;

/// What a variable's declaration says of how its values read, as leniently as it can be read: a
/// declaration of a newer form, or one with a block this console does not know, still shows its
/// values by its type's base control, which on a read-only page is the value alone.
enum Declared {
    Choice(Options),
    Integer(Option<Unit>),
    Switch,
    Ordered(Options),
    /// No declaration, or one of a type this console does not know.
    Unknown,
}

/// An `integer` variable's unit.
enum Unit {
    Time(TimeUnit),
    Text(Vec<(String, String)>),
}

impl Declared {
    fn of(variable: Option<&Value>) -> Self {
        let Some(variable) = variable else {
            return Self::Unknown;
        };
        let options = || -> Options {
            variable
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    let value = option.get("value")?.as_str()?.to_owned();
                    Some((value, texts(option.get("title"))))
                })
                .collect()
        };
        match variable.get("type").and_then(Value::as_str) {
            Some("choice") => Self::Choice(options()),
            Some("integer") => Self::Integer(match variable.get("unit") {
                Some(Value::String(word)) => TimeUnit::of(word).map(Unit::Time),
                Some(text @ Value::Object(_)) => Some(Unit::Text(texts(Some(text)))),
                Some(_) | None => None,
            }),
            Some("switch") => Self::Switch,
            Some("ordered") => Self::Ordered(options()),
            Some(_) | None => Self::Unknown,
        }
    }

    /// The value of `json` in words; `display` the Runtime's words of it in `tongue`, when it
    /// gives some.
    fn shown(&self, json: &str, display: Option<&str>, tongue: Tongue) -> Shown {
        let Ok(value) = serde_json::from_str::<Value>(json) else {
            return Shown::Raw(json.to_owned());
        };
        let raw = || Shown::Raw(value.to_string());
        match self {
            Self::Choice(options) => match value.as_str() {
                Some(chosen) => Shown::Words(match display {
                    Some(display) => display.to_owned(),
                    None => option_title(options, chosen, tongue),
                }),
                None => raw(),
            },
            Self::Integer(unit) => match (value.as_i64(), unit) {
                (Some(number), Some(Unit::Time(unit))) => {
                    let (amount, unit) = whole_time(number, *unit);
                    Shown::Time { amount, unit }
                }
                (Some(number), Some(Unit::Text(text))) => match display {
                    Some(display) => Shown::Words(display.to_owned()),
                    None => match in_language(text, tongue) {
                        Some(unit) => Shown::Words(format!("{number} {unit}")),
                        None => Shown::Number(number),
                    },
                },
                (Some(number), None) => Shown::Number(number),
                (None, _) => raw(),
            },
            Self::Switch => value.as_bool().map_or_else(raw, Shown::Switch),
            Self::Ordered(options) => match keys(&value) {
                Some(keys) => Shown::List(
                    keys.into_iter().map(|key| option_title(options, key, tongue)).collect(),
                ),
                None => raw(),
            },
            Self::Unknown => plain(&value),
        }
    }
}

/// A list of strings' items.
fn keys(value: &Value) -> Option<Vec<&str>> {
    value.as_array()?.iter().map(Value::as_str).collect()
}

/// The title of the option `value` in `tongue`; the value itself when it has none.
fn option_title(options: &Options, value: &str, tongue: Tongue) -> String {
    options
        .iter()
        .find(|(option, _)| option == value)
        .and_then(|(_, title)| in_language(title, tongue))
        .unwrap_or(value)
        .to_owned()
}

/// A value of no declaration, by its JSON type.
fn plain(value: &Value) -> Shown {
    match value {
        Value::Bool(on) => Shown::Switch(*on),
        Value::String(text) => Shown::Words(text.clone()),
        Value::Number(number) => {
            number.as_i64().map_or_else(|| Shown::Raw(value.to_string()), Shown::Number)
        }
        Value::Array(_) => match keys(value) {
            Some(keys) => Shown::List(keys.into_iter().map(str::to_owned).collect()),
            None => Shown::Raw(value.to_string()),
        },
        Value::Null | Value::Object(_) => Shown::Raw(value.to_string()),
    }
}

/// A stored value no entry offers any more.
fn orphan_line(orphan: &GoalOrphan) -> GoalLine {
    GoalLine::Orphan(OrphanLine {
        value: Declared::Unknown.shown(&orphan.value_json, None, Tongue::En),
        set: orphan.set.as_ref().map(set_by),
    })
}

/// An other target by its id, as the stored document holds it.
fn other_target(id: &str, targets: &Targets<'_>) -> OtherTarget {
    let target = targets.target(id).and_then(|target| {
        let pool = target.get("resource")?.as_str()?;
        let condition = target.get("condition")?;
        if condition.get("kind")?.as_str()? != AT_LEAST {
            return None;
        }
        let amount = condition.get("amount")?.as_u64()?;
        Some(OtherGoal {
            pool: pool.to_owned(),
            state: targets.pool(pool),
            reading: targets.reading(pool),
            goal: targets.goal(Some(id), Some(amount)),
        })
    });
    OtherTarget { id: id.to_owned(), target }
}

/// Who set a stored row.
fn set_by(set: &GoalSetBy) -> SetBy {
    let by = match set.channel {
        RuntimeChannel::Ui => Setter::Console,
        RuntimeChannel::Cli => Setter::CommandLine,
        RuntimeChannel::Mcp => Setter::Mcp,
        RuntimeChannel::Lab => Setter::Lab(set.initiator.and_then(lab_entry)),
    };
    SetBy { at_ms: set.unix_ms, by }
}

fn lab_entry(initiator: Initiator) -> Option<LabEntry> {
    match initiator {
        Initiator::LabCli => Some(LabEntry::CommandLine),
        Initiator::LabMcp => Some(LabEntry::Mcp),
        Initiator::LabAuthoring => Some(LabEntry::Authoring),
        Initiator::LabRescue => Some(LabEntry::Rescue),
        Initiator::LabWatchdog => Some(LabEntry::Watchdog),
        Initiator::Ui => None,
    }
}

/// A text of `member` of `value` per locale, in `tongue`.
fn text_of(value: Option<&Value>, member: &str, tongue: Tongue) -> Option<String> {
    let texts = texts(value?.get(member));
    in_language(&texts, tongue).map(str::to_owned)
}

/// A text per locale, in the order written (as the JSON map keeps it); none of what is no text.
fn texts(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(locale, text)| Some((locale.clone(), text.as_str()?.to_owned())))
        .collect()
}

/// How far the target `id` is, by its condition.
fn progress(conditions: &[ResourceTargetCondition], id: &str) -> Progress {
    match conditions.iter().find(|condition| condition.target_id == id) {
        Some(condition) => match &condition.state {
            ResourceTargetConditionState::Computed { current, observed_at_unix_ms, gap } => {
                Progress::Computed { current: *current, at_ms: *observed_at_unix_ms, gap: *gap }
            }
            ResourceTargetConditionState::AwaitingObservation { reason } => {
                Progress::Awaiting(pending(*reason))
            }
        },
        None => Progress::NoCondition,
    }
}

fn form_of(schema_version: &str) -> Form {
    match schema_version {
        SCHEMA_V3 => Form::PerTarget,
        SCHEMA_V2 => Form::Whole,
        SCHEMA_V1 => Form::Old,
        newer => Form::Newer(newer.to_owned()),
    }
}

fn reading(observation: &ResourceTargetObservation) -> Reading {
    match (observation.current, observation.observed_at_unix_ms) {
        (Some(current), Some(at_ms)) => Reading::Read { current, at_ms },
        _ => Reading::Pending(observation.pending.map(pending)),
    }
}

fn pending(reason: ResourceTargetPendingReason) -> Pending {
    match reason {
        ResourceTargetPendingReason::Missing => Pending::Missing,
        ResourceTargetPendingReason::Expired => Pending::Expired,
        ResourceTargetPendingReason::LowConfidence => Pending::LowConfidence,
        ResourceTargetPendingReason::InvalidValue => Pending::InvalidValue,
    }
}

fn closed_of(reason: ResourceTargetsRejectionReason) -> Closed {
    match reason {
        ResourceTargetsRejectionReason::UnmappedTask => Closed::UnmappedTask,
        ResourceTargetsRejectionReason::ResourceNotObservable => Closed::NotObservable,
        ResourceTargetsRejectionReason::ResourceOutOfScope => Closed::OutOfScope,
        other => Closed::Other(spelled(other)),
    }
}

/// The client's code for a receipt it cannot decode (Runtime `crates/runtime-client/src/codes.rs`,
/// `runtime_receipt_decode_failed`). The page is decoded with the contract's strict types inside
/// the receipt (the page and its entries `deny_unknown_fields`:
/// `crates/actingcommand-contract/src/runtime/goals.rs:155`, `:284`), so a page of a shape newer
/// than this console's client fails there, before any of it reaches [`page`].
pub const RECEIPT_DECODE_FAILED: &str = "runtime_receipt_decode_failed";

/// Whether a failed `GoalView` read, with the client's own `code`, is most likely an answer newer
/// than this console: the client could not decode the receipt ([`RECEIPT_DECODE_FAILED`]). The
/// Goals tab then says so, with the codes, instead of a bare read failure; a damaged answer reads
/// the same, which is why it says "most likely".
pub fn newer_than_this_console(code: &str) -> bool {
    code == RECEIPT_DECODE_FAILED
}

/// A reason as the wire spells it (snake case); its variant's name should that ever fail.
fn spelled(reason: ResourceTargetsRejectionReason) -> String {
    match serde_json::to_value(reason) {
        Ok(Value::String(code)) => code,
        Ok(_) | Err(_) => format!("{reason:?}"),
    }
}

/// The texts of a `display` map, for [`in_language`]: in its keys' order, which is no order the
/// declaration wrote, so its first text is the first by locale name.
fn display_texts(display: &BTreeMap<String, String>) -> Vec<(String, String)> {
    display.iter().map(|(locale, text)| (locale.clone(), text.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        in_language, newer_than_this_console, page, whole_time, Closed, Declaration, Document,
        Form, GoalLine, GoalsPage, Group, LabEntry, OrphanLine, OtherGoal, OtherTarget, PageState,
        Pending, PoolState, Progress, Reading, ResourceLine, SetBy, Setter, Shown, Stored,
        StoredGoal, TierLine, TimeUnit, Unapplied, VariableLine, VariableState,
        RECEIPT_DECODE_FAILED,
    };
    use acui_rows::outcome::Tongue;
    use actingcommand_contract::GoalView;
    use serde_json::{json, Value};

    const EVALUATED: u64 = 1_794_532_592_000;
    const OBSERVED: u64 = 1_794_532_500_000;
    const SET_AT: u64 = 1_794_500_000_000;
    const VALID_UNTIL: u64 = 1_826_068_592_000;
    const V1: &str = "actingcommand.resource-targets.v1";
    const V2: &str = "actingcommand.resource-targets.v2";
    const V3: &str = "actingcommand.resource-targets.v3";

    #[test]
    fn only_an_undecodable_receipt_reads_as_newer_than_this_console() {
        assert_eq!(RECEIPT_DECODE_FAILED, "runtime_receipt_decode_failed");
        assert!(newer_than_this_console("runtime_receipt_decode_failed"));
        for other in [
            "runtime_receipt_invalid",
            "runtime_operation_unsupported",
            "runtime_unavailable",
            "runtime_info_decode_failed",
            "",
        ] {
            assert!(!newer_than_this_console(other), "{other}");
        }
    }

    /// A goal page as the wire carries it, read through the contract's own type, so that every
    /// member named here is one the pinned contract has: its declaration, its entries, the other
    /// members by name (`orphans`, `other_targets`), and its resource target view.
    fn view(declaration: Value, entries: Value, rest: Value, targets: Value) -> GoalView {
        let mut view = json!({
            "instance_alias": "node.a",
            "game_id": "game.a",
            "version": 7,
            "declaration": declaration,
            "entries": entries,
            "targets": targets,
        });
        if let Value::Object(rest) = rest {
            for (member, value) in rest {
                view[member.as_str()] = value;
            }
        }
        serde_json::from_value(view).expect("a page the pinned contract reads")
    }

    fn state(word: &str) -> Value {
        json!({"state": word})
    }

    /// The page in English, no package with a display name.
    fn read(view: &GoalView) -> GoalsPage {
        page(view, Tongue::En, &|_| None)
    }

    /// A resource target view as the wire carries it.
    fn targets(targetable: Value, not_targetable: Value, active: Option<Value>) -> Value {
        let mut view = json!({
            "instance_alias": "node.a",
            "policy_instance": "node.a",
            "evaluated_at_unix_ms": EVALUATED,
            "as_of_ledger_position": 12,
            "catalog_hash": format!("sha256:{}", "a".repeat(64)),
            "targetable": targetable,
            "not_targetable": not_targetable,
        });
        if let Some(active) = active {
            view["active"] = active;
        }
        view
    }

    fn no_targets() -> Value {
        targets(json!([]), json!([]), None)
    }

    /// A targetable pool; `scale` and `importance` whether the pack gives it each default; with a
    /// reading, its amount.
    fn pool(id: &str, scale: bool, importance: bool, current: Option<i64>) -> Value {
        let observation = match current {
            Some(current) => json!({"current": current, "observed_at_unix_ms": OBSERVED}),
            None => json!({"pending": "missing"}),
        };
        let mut pool = json!({
            "resource": id,
            "fact_key": format!("inventory.{id}"),
            "producing_tasks": ["task.daily"],
            "scale_required": !scale,
            "importance_required": !importance,
            "observation": observation,
        });
        if scale {
            pool["default_scale"] = json!(100);
        }
        if importance {
            pool["default_importance_milli"] = json!(500);
        }
        pool
    }

    fn closed_pool(id: &str, reason: &str) -> Value {
        json!({"resource": id, "reason": reason})
    }

    fn active(
        schema: &str,
        targets: Value,
        conditions: Value,
        expired: bool,
        until: Option<u64>,
    ) -> Value {
        let mut active = json!({
            "policy_sha256": format!("sha256:{}", "b".repeat(64)),
            "schema_version": schema,
            "version": 41,
            "event_id": format!("evt_{}", "1".repeat(32)),
            "expired": expired,
            "targets": targets,
            "conditions": conditions,
        });
        if let Some(until) = until {
            active["valid_until_unix_ms"] = json!(until);
        }
        active
    }

    fn target(id: &str, pool: &str, amount: Value) -> Value {
        json!({
            "id": id,
            "resource": pool,
            "condition": {"kind": "at_least", "amount": amount},
            "apply": {"mode": "adjust", "weight": "score_stage"},
        })
    }

    fn computed(id: &str, pool: &str, current: i64, gap: u64) -> Value {
        json!({
            "target_id": id,
            "resource": pool,
            "fact_key": format!("inventory.{pool}"),
            "state": {"kind": "computed", "current": current, "observed_at_unix_ms": OBSERVED, "gap": gap},
        })
    }

    fn awaiting(id: &str, pool: &str, reason: &str) -> Value {
        json!({
            "target_id": id,
            "resource": pool,
            "fact_key": format!("inventory.{pool}"),
            "state": {"kind": "awaiting_observation", "reason": reason},
        })
    }

    fn set(channel: &str, initiator: Option<&str>) -> Value {
        let mut set = json!({"unix_ms": SET_AT, "channel": channel});
        if let Some(initiator) = initiator {
            set["initiator"] = json!(initiator);
        }
        set
    }

    fn heading(name: &str, title: Value) -> Value {
        json!({
            "name": name,
            "entry_type": "heading",
            "placed": true,
            "entry": {"name": name, "type": "heading", "title": title},
        })
    }

    fn note(name: &str, title: Value, help: Value) -> Value {
        json!({
            "name": name,
            "entry_type": "note",
            "placed": true,
            "entry": {"name": name, "type": "note", "title": title, "help": help},
        })
    }

    /// A resource entry on `pool`, with the target `(id, amount)` on it or none.
    fn resource(name: &str, pool: &str, target: Option<(&str, u64)>, set: Option<Value>) -> Value {
        let mut entry = json!({
            "name": name,
            "entry_type": "resource",
            "placed": true,
            "entry": {"name": name, "type": "resource", "title": {"en": name}, "governs": {"pool": pool}},
            "state": "in_effect",
            "resource": {"pool": pool},
        });
        match target {
            Some((id, amount)) => {
                let value = format!("{{\"at_least\":{amount}}}");
                entry["value_json"] = json!(value);
                entry["effective_json"] = json!(value);
                entry["source"] = json!("set");
                entry["resource"]["target_id"] = json!(id);
            }
            None => entry["source"] = json!("default"),
        }
        if let Some(set) = set {
            entry["set"] = set;
        }
        entry
    }

    /// A variable's entry: placed by an entry of its name or not, its declaration, the packages
    /// that declare it, and the members of its values by name.
    fn variable(
        name: &str,
        placed: bool,
        declaration: Value,
        packages: &[&str],
        values: Value,
    ) -> Value {
        let mut entry = json!({
            "name": name,
            "entry_type": "variable",
            "placed": placed,
            "variable": declaration,
            "packages": packages,
        });
        if placed {
            entry["entry"] = json!({"name": name, "type": "variable"});
        }
        if let Value::Object(values) = values {
            for (member, value) in values {
                entry[member.as_str()] = value;
            }
        }
        entry
    }

    fn choice(name: &str) -> Value {
        json!({
            "name": name,
            "type": "choice",
            "title": {"en": "Item", "zh-Hans": "物品"},
            "options": [
                {"value": "item_a", "title": {"en": "Item A", "zh-Hans": "甲"}},
                {"value": "item_b", "title": {"en": "Item B", "zh-Hans": "乙"}},
            ],
            "default": "item_b",
        })
    }

    fn integer(name: &str, unit: Option<Value>) -> Value {
        let mut declaration = json!({
            "name": name,
            "type": "integer",
            "title": {"en": "Rounds"},
            "min": 0,
            "max": 864_000_000,
            "default": 0,
        });
        if let Some(unit) = unit {
            declaration["unit"] = unit;
        }
        declaration
    }

    fn switch(name: &str, title: &str) -> Value {
        json!({"name": name, "type": "switch", "title": {"en": title}, "default": false})
    }

    fn ordered(name: &str) -> Value {
        json!({
            "name": name,
            "type": "ordered",
            "title": {"en": "Order"},
            "options": [
                {"value": "item_a", "title": {"en": "Item A"}},
                {"value": "item_b", "title": {"en": "Item B"}},
            ],
            "min_count": 0,
            "max_count": 2,
            "default": ["item_b", "item_a"],
        })
    }

    /// A switch in effect at its default, in a group of variables no entry places.
    fn unplaced_switch(name: &str, title: &str, packages: &[&str]) -> Value {
        variable(
            name,
            false,
            switch(name, title),
            packages,
            json!({
                "default_json": "false",
                "effective_json": "false",
                "source": "default",
                "state": "in_effect",
            }),
        )
    }

    fn switch_line(title: &str) -> GoalLine {
        GoalLine::Variable(VariableLine {
            title: Some(title.to_owned()),
            value: Some(Shown::Switch(false)),
            default: None,
            set: None,
            state: VariableState::InEffect,
            left_out: Vec::new(),
        })
    }

    fn goal(amount: u64, progress: Progress) -> StoredGoal {
        StoredGoal { amount: Some(amount), progress, until_ms: None, lapsed: false }
    }

    #[test]
    fn the_five_kinds_of_entry_go_in_the_runtimes_order() {
        let view = view(
            state("accepted"),
            json!([
                heading("daily", json!({"en": "Daily", "zh-Hans": "日常"})),
                note("about", json!({"en": "Read me"}), json!({"en": "More words"})),
                resource("gold", "pool.gold", Some(("gold", 5000)), Some(set("lab", Some("Lab-CLI")))),
                {
                    "name": "pace",
                    "entry_type": "tier",
                    "placed": true,
                    "entry": {
                        "name": "pace",
                        "type": "tier",
                        "title": {"en": "Pace"},
                        "governs": {"rules": ["rule.a"]},
                        "options": [
                            {"value": "slow", "title": {"en": "Slow"}, "scale_milli": 500},
                            {"value": "fast", "title": {"en": "Fast"}, "scale_milli": 2000},
                        ],
                        "default": "slow",
                    },
                    "default_json": "\"slow\"",
                    "source": "default",
                    "state": "not_built",
                },
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_a\"",
                    "effective_json": "\"item_a\"",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                    "display": {"en": "Item A", "zh-Hans": "甲"},
                })),
            ]),
            json!({}),
            targets(
                json!([pool("pool.gold", true, true, Some(1200))]),
                json!([]),
                Some(active(
                    V3,
                    json!([target("gold", "pool.gold", json!(5000))]),
                    json!([computed("gold", "pool.gold", 1200, 3800)]),
                    false,
                    None,
                )),
            ),
        );
        let english = read(&view);
        assert_eq!(
            english.state,
            PageState::Listed { declaration: Declaration::Accepted, nothing: false }
        );
        assert_eq!(
            english.lines,
            vec![
                GoalLine::Heading(Some("Daily".to_owned())),
                GoalLine::Note {
                    text: Some("Read me".to_owned()),
                    help: Some("More words".to_owned()),
                },
                GoalLine::Resource(ResourceLine {
                    title: Some("gold".to_owned()),
                    pool: PoolState::Open,
                    reading: Some(Reading::Read { current: 1200, at_ms: OBSERVED }),
                    goal: Some(goal(
                        5000,
                        Progress::Computed { current: 1200, at_ms: OBSERVED, gap: 3800 },
                    )),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::CommandLine)) }),
                }),
                GoalLine::Tier(TierLine {
                    title: Some("Pace".to_owned()),
                    default: Some("Slow".to_owned()),
                }),
                GoalLine::Variable(VariableLine {
                    title: Some("Item".to_owned()),
                    value: Some(Shown::Words("Item A".to_owned())),
                    default: Some(Shown::Words("Item B".to_owned())),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Console }),
                    state: VariableState::InEffect,
                    left_out: Vec::new(),
                }),
            ]
        );
        assert_eq!(
            english.stored,
            Stored::Document(Document {
                form: Form::PerTarget,
                valid_until_ms: None,
                expired: false,
                targets: 1,
            })
        );
        assert_eq!(english.version, 7);
        assert_eq!(english.evaluated_at_ms, EVALUATED);
        assert!(!english.no_catalog);
        assert!(!english.stale_schema);

        // The same page in Chinese: each text in its own locale, or English's.
        let chinese = page(&view, Tongue::ZhHans, &|_| None);
        assert_eq!(chinese.lines[0], GoalLine::Heading(Some("日常".to_owned())));
        assert_eq!(
            chinese.lines[1],
            GoalLine::Note { text: Some("Read me".to_owned()), help: Some("More words".to_owned()) }
        );
        match &chinese.lines[4] {
            GoalLine::Variable(line) => {
                assert_eq!(line.title.as_deref(), Some("物品"));
                assert_eq!(line.value, Some(Shown::Words("甲".to_owned())));
                assert_eq!(line.default, Some(Shown::Words("乙".to_owned())));
            }
            line => panic!("{line:?}"),
        }
    }

    #[test]
    fn three_states_list_nothing_whatever_the_page_holds() {
        let entries = json!([heading("daily", json!({"en": "Daily"}))]);
        for (declaration, expected) in [
            (state("not_checked_yet"), PageState::NotCheckedYet),
            (state("no_schedule"), PageState::NoSchedule),
            (state("root_failing"), PageState::RootFailing { path: None }),
            (
                json!({
                    "state": "root_failing",
                    "path": "game.a/schedule.json",
                    "io_kind": "not_found",
                }),
                PageState::RootFailing { path: Some("game.a/schedule.json".to_owned()) },
            ),
        ] {
            let page = read(&view(declaration, entries.clone(), json!({}), no_targets()));
            assert_eq!(page.state, expected);
            assert!(page.lines.is_empty(), "{:?}", page.lines);
        }
    }

    #[test]
    fn a_declaration_not_taken_or_not_there_still_lists_the_variables() {
        let entries = json!([unplaced_switch("extra", "Also the extra", &["pack.a"])]);
        for (declaration, expected) in [
            (state("absent"), Declaration::Absent),
            (
                json!({
                    "state": "refused",
                    "sha256": format!("sha256:{}", "c".repeat(64)),
                    "standard_pack_problem": "goal_type_unknown",
                    "json_pointer": "/entries/0/type",
                }),
                Declaration::NotTaken,
            ),
            (state("unreadable"), Declaration::NotTaken),
            (state("not_read"), Declaration::NotRead),
            (state("accepted"), Declaration::Accepted),
        ] {
            let page = read(&view(declaration, entries.clone(), json!({}), no_targets()));
            assert_eq!(page.state, PageState::Listed { declaration: expected, nothing: false });
            assert_eq!(page.lines, vec![GoalLine::Package(None), switch_line("Also the extra")]);
        }
    }

    #[test]
    fn a_page_with_nothing_to_set_says_so() {
        let page = read(&view(state("accepted"), json!([]), json!({}), no_targets()));
        assert_eq!(
            page.state,
            PageState::Listed { declaration: Declaration::Accepted, nothing: true }
        );
        assert!(page.lines.is_empty(), "{:?}", page.lines);
        assert_eq!(page.stored, Stored::None);
        // Pools no goal can name still close it: nothing is there to set all the same.
        let closed = targets(json!([]), json!([closed_pool("pool.gems", "unmapped_task")]), None);
        let page = read(&view(state("absent"), json!([]), json!({}), closed));
        assert_eq!(page.state, PageState::Listed { declaration: Declaration::Absent, nothing: true });
        assert_eq!(
            page.lines,
            vec![
                GoalLine::Group(Group::NotTargetable),
                GoalLine::Closed { pool: "pool.gems".to_owned(), why: Closed::UnmappedTask },
            ]
        );
    }

    #[test]
    fn a_value_not_applied_not_offered_or_applied_in_part_says_so() {
        let view = view(
            state("accepted"),
            json!([
                variable("rounds", true, integer("rounds", None), &["pack.a"], json!({
                    "default_json": "0",
                    "value_json": "900000000",
                    "effective_json": "0",
                    "source": "default",
                    "set": set("mcp", None),
                    "state": "not_applied",
                    "variable_problem": "out_of_range",
                })),
                variable("extra", true, switch("extra", "Also the extra"), &["pack.a"], json!({
                    "default_json": "false",
                    "effective_json": "false",
                    "source": "default",
                    "state": "not_applied",
                    "variable_problem": "irreversible_by_root",
                    "variable_cause": {
                        "cause": "irreversible_capabilities",
                        "packages": {"pack.a": ["cap.buy"]},
                    },
                })),
                variable("later", true, switch("later", "Later"), &["pack.a"], json!({
                    "default_json": "false",
                    "value_json": "true",
                    "effective_json": "false",
                    "source": "default",
                    "set": set("ui", None),
                    "state": "not_applied",
                    "variable_problem": "a_newer_word",
                })),
                {
                    "name": "route",
                    "entry_type": "variable",
                    "placed": true,
                    "entry": {"name": "route", "type": "variable"},
                    "variable": choice("route"),
                    "unbound_packages": ["pack.b"],
                    "value_json": "\"item_a\"",
                    "set": set("cli", None),
                    "state": "not_offered",
                },
                variable("order", true, ordered("order"), &["pack.a"], json!({
                    "default_json": "[\"item_b\",\"item_a\"]",
                    "value_json": "[\"item_c\",\"item_a\"]",
                    "effective_json": "[\"item_a\"]",
                    "removed_keys": ["item_c"],
                    "source": "set",
                    "set": set("lab", Some("Lab-MCP")),
                    "state": "in_effect",
                    "variable_problem": "keys_left_out",
                })),
            ]),
            json!({}),
            no_targets(),
        );
        assert_eq!(
            read(&view).lines,
            vec![
                GoalLine::Variable(VariableLine {
                    title: Some("Rounds".to_owned()),
                    value: Some(Shown::Number(0)),
                    default: None,
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Mcp }),
                    state: VariableState::NotApplied {
                        stored: true,
                        why: Some(Unapplied::OutOfRange),
                    },
                    left_out: Vec::new(),
                }),
                GoalLine::Variable(VariableLine {
                    title: Some("Also the extra".to_owned()),
                    value: Some(Shown::Switch(false)),
                    default: None,
                    set: None,
                    state: VariableState::NotApplied {
                        stored: false,
                        why: Some(Unapplied::IrreversibleByRoot),
                    },
                    left_out: Vec::new(),
                }),
                GoalLine::Variable(VariableLine {
                    title: Some("Later".to_owned()),
                    value: Some(Shown::Switch(false)),
                    default: None,
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Console }),
                    state: VariableState::NotApplied {
                        stored: true,
                        why: Some(Unapplied::Other("a_newer_word".to_owned())),
                    },
                    left_out: Vec::new(),
                }),
                GoalLine::Variable(VariableLine {
                    title: Some("Item".to_owned()),
                    value: Some(Shown::Words("Item A".to_owned())),
                    default: None,
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::CommandLine }),
                    state: VariableState::NotOffered,
                    left_out: Vec::new(),
                }),
                GoalLine::Variable(VariableLine {
                    title: Some("Order".to_owned()),
                    value: Some(Shown::List(vec!["Item A".to_owned()])),
                    default: Some(Shown::List(vec!["Item B".to_owned(), "Item A".to_owned()])),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Mcp)) }),
                    state: VariableState::InEffect,
                    left_out: vec!["item_c".to_owned()],
                }),
            ]
        );
    }

    #[test]
    fn every_word_of_variable_problem_is_read() {
        for (word, expected) in [
            ("not_an_option", Unapplied::NotAnOption),
            ("out_of_range", Unapplied::OutOfRange),
            ("off_step", Unapplied::OffStep),
            ("type_changed", Unapplied::TypeChanged),
            ("keys_removed", Unapplied::KeysRemoved),
            ("count_out_of_range", Unapplied::CountOutOfRange),
            ("key_repeated", Unapplied::KeyRepeated),
            ("keys_left_out", Unapplied::KeysLeftOut),
            ("entry_unreadable", Unapplied::EntryUnreadable),
            ("irreversible_by_root", Unapplied::IrreversibleByRoot),
            ("package_unreadable", Unapplied::PackageUnreadable),
            ("goals_unreadable", Unapplied::GoalsUnreadable),
            ("declaration_conflict", Unapplied::DeclarationConflict),
            ("goal_name_conflict", Unapplied::GoalNameConflict),
            ("name_out_of_limits", Unapplied::NameOutOfLimits),
            ("cap_range", Unapplied::CapRange),
            ("root_unchecked", Unapplied::RootUnchecked),
        ] {
            assert_eq!(Unapplied::of(word), expected, "{word}");
        }
    }

    #[test]
    fn values_no_longer_offered_other_targets_and_closed_pools_close_the_page() {
        let view = view(
            state("accepted"),
            json!([resource("gold", "pool.gold", Some(("gold", 50)), None)]),
            json!({
                "orphans": [
                    {"name": "old_route", "value_json": "\"a\"", "set": set("lab", None)},
                    {"name": "old_rounds", "value_json": "3"},
                    {"name": "gold", "value_json": "{\"at_least\":5}"},
                ],
                "other_targets": ["wood", "odd"],
            }),
            targets(
                json!([pool("pool.wood", true, true, Some(10))]),
                json!([
                    closed_pool("pool.gold", "unmapped_task"),
                    closed_pool("pool.gems", "resource_not_observable"),
                    closed_pool("pool.keys", "resource_out_of_scope"),
                    closed_pool("pool.relics", "unknown_resource"),
                ]),
                Some(active(
                    V2,
                    json!([
                        target("gold", "pool.gold", json!(50)),
                        target("wood", "pool.wood", json!(80)),
                        {"id": "odd", "resource": "pool.keys", "condition": {"kind": "at_most", "amount": 5}},
                    ]),
                    json!([computed("wood", "pool.wood", 10, 70)]),
                    false,
                    Some(VALID_UNTIL),
                )),
            ),
        );
        let page = read(&view);
        assert_eq!(
            page.lines,
            vec![
                // Its pool says why on its line, so no closed line of it below.
                GoalLine::Resource(ResourceLine {
                    title: Some("gold".to_owned()),
                    pool: PoolState::Closed(Closed::UnmappedTask),
                    reading: None,
                    goal: Some(goal(50, Progress::NoCondition)),
                    set: None,
                }),
                GoalLine::Group(Group::NoLongerOffered),
                GoalLine::Orphan(OrphanLine {
                    value: Shown::Words("a".to_owned()),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(None) }),
                }),
                GoalLine::Orphan(OrphanLine { value: Shown::Number(3), set: None }),
                GoalLine::Orphan(OrphanLine {
                    value: Shown::Raw("{\"at_least\":5}".to_owned()),
                    set: None,
                }),
                GoalLine::Group(Group::OtherTargets),
                GoalLine::OtherTarget(OtherTarget {
                    id: "wood".to_owned(),
                    target: Some(OtherGoal {
                        pool: "pool.wood".to_owned(),
                        state: PoolState::Open,
                        reading: Some(Reading::Read { current: 10, at_ms: OBSERVED }),
                        goal: goal(80, Progress::Computed { current: 10, at_ms: OBSERVED, gap: 70 }),
                    }),
                }),
                // No `at_least` target: not read here, so its pool is said below.
                GoalLine::OtherTarget(OtherTarget { id: "odd".to_owned(), target: None }),
                GoalLine::Group(Group::NotTargetable),
                GoalLine::Closed { pool: "pool.gems".to_owned(), why: Closed::NotObservable },
                GoalLine::Closed { pool: "pool.keys".to_owned(), why: Closed::OutOfScope },
                GoalLine::Closed {
                    pool: "pool.relics".to_owned(),
                    why: Closed::Other("unknown_resource".to_owned()),
                },
            ]
        );
        assert_eq!(
            page.stored,
            Stored::Document(Document {
                form: Form::Whole,
                valid_until_ms: Some(VALID_UNTIL),
                expired: false,
                targets: 3,
            })
        );
    }

    /// #168's review: rows no longer offered and rows that do not read, in turns, keep the
    /// Runtime's order; an amount that is no whole number does not read.
    #[test]
    fn other_targets_that_read_and_do_not_keep_their_order() {
        let view = view(
            state("absent"),
            json!([]),
            json!({"other_targets": ["gone", "missing", "wood", "half"]}),
            targets(
                json!([pool("pool.wood", true, true, None)]),
                json!([closed_pool("pool.gems", "unmapped_task")]),
                Some(active(
                    V2,
                    json!([
                        target("gone", "pool.gems", json!(300)),
                        target("wood", "pool.wood", json!(80)),
                        target("half", "pool.wood", json!(1.5)),
                    ]),
                    json!([awaiting("wood", "pool.wood", "low_confidence")]),
                    false,
                    Some(VALID_UNTIL),
                )),
            ),
        );
        assert_eq!(
            read(&view).lines,
            vec![
                GoalLine::Group(Group::OtherTargets),
                GoalLine::OtherTarget(OtherTarget {
                    id: "gone".to_owned(),
                    target: Some(OtherGoal {
                        pool: "pool.gems".to_owned(),
                        state: PoolState::Closed(Closed::UnmappedTask),
                        reading: None,
                        goal: goal(300, Progress::NoCondition),
                    }),
                }),
                GoalLine::OtherTarget(OtherTarget { id: "missing".to_owned(), target: None }),
                GoalLine::OtherTarget(OtherTarget {
                    id: "wood".to_owned(),
                    target: Some(OtherGoal {
                        pool: "pool.wood".to_owned(),
                        state: PoolState::Open,
                        reading: Some(Reading::Pending(Some(Pending::Missing))),
                        goal: goal(80, Progress::Awaiting(Pending::LowConfidence)),
                    }),
                }),
                GoalLine::OtherTarget(OtherTarget { id: "half".to_owned(), target: None }),
            ]
        );
    }

    /// #168's review: a pool the pack gives one default of has no step either way (`||`, not
    /// `&&`), and so it is under a v3 record too.
    #[test]
    fn a_pool_with_one_default_only_has_no_step() {
        let view = view(
            state("accepted"),
            json!([
                resource("both", "pool.both", None, None),
                resource("scale", "pool.scale", Some(("scale", 40)), Some(set("lab", Some("Lab-制包")))),
                resource("importance", "pool.importance", None, None),
                resource("neither", "pool.neither", None, None),
            ]),
            json!({}),
            targets(
                json!([
                    pool("pool.both", true, true, Some(1)),
                    pool("pool.scale", true, false, Some(2)),
                    pool("pool.importance", false, true, Some(3)),
                    pool("pool.neither", false, false, Some(4)),
                ]),
                json!([]),
                Some(active(
                    V3,
                    json!([target("scale", "pool.scale", json!(40))]),
                    json!([computed("scale", "pool.scale", 2, 38)]),
                    false,
                    None,
                )),
            ),
        );
        let lines = read(&view).lines;
        let pools: Vec<&PoolState> = lines
            .iter()
            .map(|line| match line {
                GoalLine::Resource(line) => &line.pool,
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            pools,
            vec![&PoolState::Open, &PoolState::NoStep, &PoolState::NoStep, &PoolState::NoStep]
        );
        assert_eq!(
            lines[1],
            GoalLine::Resource(ResourceLine {
                title: Some("scale".to_owned()),
                pool: PoolState::NoStep,
                reading: Some(Reading::Read { current: 2, at_ms: OBSERVED }),
                goal: Some(goal(40, Progress::Computed { current: 2, at_ms: OBSERVED, gap: 38 })),
                set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Authoring)) }),
            })
        );
    }

    #[test]
    fn variables_no_entry_places_go_by_their_first_packages_name() {
        let view = view(
            state("absent"),
            json!([
                unplaced_switch("one", "One", &["pack.a"]),
                unplaced_switch("two", "Two", &["pack.b"]),
                unplaced_switch("three", "Three", &["pack.a", "pack.c"]),
                unplaced_switch("four", "Four", &["pack.c"]),
                unplaced_switch("five", "Five", &[]),
            ]),
            json!({}),
            no_targets(),
        );
        let titles = |package: &str| match package {
            "pack.a" => Some("Pack A".to_owned()),
            "pack.c" => Some("Pack C".to_owned()),
            _ => None,
        };
        assert_eq!(
            page(&view, Tongue::En, &titles).lines,
            vec![
                GoalLine::Package(Some("Pack A".to_owned())),
                switch_line("One"),
                switch_line("Three"),
                GoalLine::Package(Some("Pack C".to_owned())),
                switch_line("Four"),
                GoalLine::Package(None),
                switch_line("Two"),
                switch_line("Five"),
            ]
        );
        // Without any name, all of them are the one group of other settings.
        assert_eq!(read(&view).lines[0], GoalLine::Package(None));
        assert_eq!(read(&view).lines.len(), 6);
    }

    #[test]
    fn a_time_is_a_whole_number_of_the_largest_unit_it_fits() {
        assert_eq!(whole_time(5_400_000, TimeUnit::Hour), (90, TimeUnit::Minute));
        assert_eq!(whole_time(7_200_000, TimeUnit::Hour), (2, TimeUnit::Hour));
        assert_eq!(whole_time(86_400_000, TimeUnit::Minute), (1440, TimeUnit::Minute));
        assert_eq!(whole_time(90_000, TimeUnit::Day), (90, TimeUnit::Second));
        assert_eq!(whole_time(1_500, TimeUnit::Second), (1500, TimeUnit::Millisecond));
        assert_eq!(whole_time(0, TimeUnit::Day), (0, TimeUnit::Day));
        assert_eq!(whole_time(-5_400_000, TimeUnit::Hour), (-90, TimeUnit::Minute));
        let view = view(
            state("accepted"),
            json!([
                variable("wait", true, integer("wait", Some(json!("hour"))), &["pack.a"], json!({
                    "default_json": "1500",
                    "value_json": "5400000",
                    "effective_json": "5400000",
                    "source": "set",
                    "set": set("lab", Some("Lab-救急")),
                    "state": "in_effect",
                })),
                variable("count", true, integer("count", Some(json!({"en": "rounds", "zh-Hans": "轮"}))), &["pack.a"], json!({
                    "default_json": "0",
                    "value_json": "3",
                    "effective_json": "3",
                    "source": "set",
                    "set": set("lab", Some("Lab-看门狗")),
                    "state": "in_effect",
                    "display": {"en": "3 rounds", "zh-Hans": "3 轮"},
                })),
            ]),
            json!({}),
            no_targets(),
        );
        assert_eq!(
            read(&view).lines,
            vec![
                GoalLine::Variable(VariableLine {
                    title: Some("Rounds".to_owned()),
                    value: Some(Shown::Time { amount: 90, unit: TimeUnit::Minute }),
                    default: Some(Shown::Time { amount: 1500, unit: TimeUnit::Millisecond }),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Rescue)) }),
                    state: VariableState::InEffect,
                    left_out: Vec::new(),
                }),
                GoalLine::Variable(VariableLine {
                    title: Some("Rounds".to_owned()),
                    value: Some(Shown::Words("3 rounds".to_owned())),
                    default: Some(Shown::Words("0 rounds".to_owned())),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Watchdog)) }),
                    state: VariableState::InEffect,
                    left_out: Vec::new(),
                }),
            ]
        );
        match &page(&view, Tongue::ZhHans, &|_| None).lines[1] {
            GoalLine::Variable(line) => {
                assert_eq!(line.value, Some(Shown::Words("3 轮".to_owned())));
                assert_eq!(line.default, Some(Shown::Words("0 轮".to_owned())));
            }
            line => panic!("{line:?}"),
        }
    }

    #[test]
    fn texts_fall_back_as_ui_jsons_do() {
        fn texts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
            pairs.iter().map(|(locale, text)| ((*locale).to_owned(), (*text).to_owned())).collect()
        }
        let both = texts(&[("en", "Gold"), ("zh-Hans", "金币")]);
        assert_eq!(in_language(&both, Tongue::ZhHans), Some("金币"));
        assert_eq!(in_language(&both, Tongue::En), Some("Gold"));
        let order = texts(&[("zh-CN", "a"), ("zh", "b"), ("zh-Hans", "c"), ("en", "d")]);
        assert_eq!(in_language(&order, Tongue::ZhHans), Some("c"));
        assert_eq!(in_language(&order, Tongue::En), Some("d"));
        let no_hans = texts(&[("zh-CN", "a"), ("zh", "b")]);
        assert_eq!(in_language(&no_hans, Tongue::ZhHans), Some("b"));
        assert_eq!(in_language(&texts(&[("zh-CN", "a")]), Tongue::ZhHans), Some("a"));
        // English before the first written, and the first written last.
        let other = texts(&[("ja", "ゴールド"), ("en", "Gold")]);
        assert_eq!(in_language(&other, Tongue::ZhHans), Some("Gold"));
        let only = texts(&[("ja", "ゴールド")]);
        assert_eq!(in_language(&only, Tongue::ZhHans), Some("ゴールド"));
        assert_eq!(in_language(&only, Tongue::En), Some("ゴールド"));
        let chinese = texts(&[("zh-Hans", "金币"), ("ja", "ゴールド")]);
        assert_eq!(in_language(&chinese, Tongue::En), Some("金币"));
        assert_eq!(in_language(&[], Tongue::En), None);
    }

    #[test]
    fn a_lapsed_goal_counts_no_more_and_each_form_is_told() {
        // v3: each target's own lifetime, one ahead and one past at the view's time.
        let view_v3 = view(
            state("accepted"),
            json!([
                resource("gold", "pool.gold", Some(("gold", 5)), None),
                resource("wood", "pool.wood", Some(("wood", 5)), None),
            ]),
            json!({}),
            targets(
                json!([pool("pool.gold", true, true, Some(9)), pool("pool.wood", true, true, Some(9))]),
                json!([]),
                Some(active(
                    V3,
                    json!([
                        {
                            "id": "gold",
                            "resource": "pool.gold",
                            "condition": {"kind": "at_least", "amount": 5},
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "valid_until_unix_ms": EVALUATED + 1_000,
                        },
                        {
                            "id": "wood",
                            "resource": "pool.wood",
                            "condition": {"kind": "at_least", "amount": 5},
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "valid_until_unix_ms": EVALUATED - 1_000,
                        },
                    ]),
                    json!([computed("gold", "pool.gold", 9, 0), computed("wood", "pool.wood", 9, 0)]),
                    false,
                    None,
                )),
            ),
        );
        let met = Progress::Computed { current: 9, at_ms: OBSERVED, gap: 0 };
        let goals: Vec<Option<StoredGoal>> = read(&view_v3)
            .lines
            .into_iter()
            .map(|line| match line {
                GoalLine::Resource(line) => line.goal,
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            goals,
            vec![
                Some(StoredGoal {
                    amount: Some(5),
                    progress: met,
                    until_ms: Some(EVALUATED + 1_000),
                    lapsed: false,
                }),
                Some(StoredGoal {
                    amount: Some(5),
                    progress: met,
                    until_ms: Some(EVALUATED - 1_000),
                    lapsed: true,
                }),
            ]
        );
        // v2: the document's validity, expired for every goal of it.
        let view_v2 = |schema: &str, expired: bool| {
            view(
                state("accepted"),
                json!([resource("gold", "pool.gold", Some(("gold", 5)), None)]),
                json!({}),
                targets(
                    json!([pool("pool.gold", true, true, Some(9))]),
                    json!([]),
                    Some(active(
                        schema,
                        json!([target("gold", "pool.gold", json!(5))]),
                        json!([computed("gold", "pool.gold", 9, 0)]),
                        expired,
                        Some(VALID_UNTIL),
                    )),
                ),
            )
        };
        let expired = read(&view_v2(V2, true));
        match &expired.lines[0] {
            GoalLine::Resource(line) => assert_eq!(
                line.goal,
                Some(StoredGoal { amount: Some(5), progress: met, until_ms: None, lapsed: true })
            ),
            line => panic!("{line:?}"),
        }
        assert_eq!(
            expired.stored,
            Stored::Document(Document {
                form: Form::Whole,
                valid_until_ms: Some(VALID_UNTIL),
                expired: true,
                targets: 1,
            })
        );
        let form = |schema: &str| match read(&view_v2(schema, false)).stored {
            Stored::Document(document) => document.form,
            Stored::None => panic!("a stored document"),
        };
        assert_eq!(form(V1), Form::Old);
        assert_eq!(form(V3), Form::PerTarget);
        let newer = "actingcommand.resource-targets.v4";
        assert_eq!(form(newer), Form::Newer(newer.to_owned()));
    }

    #[test]
    fn a_newer_form_reads_no_other_target() {
        let view = view(
            state("absent"),
            json!([]),
            json!({"other_targets": ["wood"]}),
            targets(
                json!([pool("pool.wood", true, true, Some(1))]),
                json!([]),
                Some(active(
                    "actingcommand.resource-targets.v4",
                    json!([target("wood", "pool.wood", json!(80))]),
                    json!([]),
                    false,
                    None,
                )),
            ),
        );
        assert_eq!(
            read(&view).lines,
            vec![
                GoalLine::Group(Group::OtherTargets),
                GoalLine::OtherTarget(OtherTarget { id: "wood".to_owned(), target: None }),
            ]
        );
    }

    #[test]
    fn who_set_a_value_is_its_channel_and_on_the_labs_its_entry() {
        let orphan = |set: Value| json!({"name": "old", "value_json": "true", "set": set});
        let view = view(
            state("absent"),
            json!([]),
            json!({"orphans": [
                orphan(set("ui", None)),
                orphan(set("cli", None)),
                orphan(set("mcp", None)),
                orphan(set("lab", None)),
                orphan(set("lab", Some("Lab-CLI"))),
                orphan(set("lab", Some("Lab-MCP"))),
                orphan(set("lab", Some("Lab-制包"))),
                orphan(set("lab", Some("Lab-救急"))),
                orphan(set("lab", Some("Lab-看门狗"))),
            ]}),
            no_targets(),
        );
        let setters: Vec<Setter> = read(&view)
            .lines
            .into_iter()
            .filter_map(|line| match line {
                GoalLine::Orphan(OrphanLine { value: Shown::Switch(true), set: Some(set) }) => {
                    assert_eq!(set.at_ms, SET_AT);
                    Some(set.by)
                }
                GoalLine::Group(Group::NoLongerOffered) => None,
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            setters,
            vec![
                Setter::Console,
                Setter::CommandLine,
                Setter::Mcp,
                Setter::Lab(None),
                Setter::Lab(Some(LabEntry::CommandLine)),
                Setter::Lab(Some(LabEntry::Mcp)),
                Setter::Lab(Some(LabEntry::Authoring)),
                Setter::Lab(Some(LabEntry::Rescue)),
                Setter::Lab(Some(LabEntry::Watchdog)),
            ]
        );
    }

    #[test]
    fn a_value_of_another_type_than_declared_is_shown_as_written() {
        let view = view(
            state("accepted"),
            json!([
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "effective_json": "7",
                    "source": "default",
                    "state": "in_effect",
                })),
                variable("odd", true, json!({"name": "odd", "type": "slider", "title": {"en": "Odd"}}), &["pack.a"], json!({
                    "effective_json": "[\"x\",\"y\"]",
                    "source": "default",
                    "state": "in_effect",
                })),
                variable("broken", true, switch("broken", "Broken"), &["pack.a"], json!({
                    "effective_json": "not json",
                    "source": "default",
                    "state": "in_effect",
                })),
            ]),
            json!({}),
            no_targets(),
        );
        let values: Vec<Option<Shown>> = read(&view)
            .lines
            .into_iter()
            .map(|line| match line {
                GoalLine::Variable(line) => line.value,
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            values,
            vec![
                Some(Shown::Raw("7".to_owned())),
                Some(Shown::List(vec!["x".to_owned(), "y".to_owned()])),
                Some(Shown::Raw("not json".to_owned())),
            ]
        );
    }

    #[test]
    fn without_a_catalog_or_with_a_stale_mark_it_says_so() {
        let mut targets = targets(json!([]), json!([]), None);
        targets.as_object_mut().expect("an object").remove("catalog_hash");
        targets["stale_schema"] = json!(true);
        let page = read(&view(state("absent"), json!([]), json!({}), targets));
        assert!(page.no_catalog);
        assert!(page.stale_schema);
        assert_eq!(page.stored, Stored::None);
    }
}
