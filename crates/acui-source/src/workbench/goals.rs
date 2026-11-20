// SPDX-License-Identifier: GPL-3.0-only
//! An instance's goals (Workflow #455, the goal page; Workflow #463 V3): what the Runtime says of
//! one application's goal page, read with `GoalView` ([`super::ReadLane::read_goals`],
//! [`GoalsRead`]), and the lines the console's Goals tab shows of it, a pure function of that page,
//! of the language shown and of the packages' display names ([`page`]); what a press on one of its
//! lines writes, and whether it asks first ([`press`]); and that write, one `EditGoal` ([`edit`]).
//! Nothing out of here names a wire type: an answer is kept whole ([`GoalsLoaded`]) until it is
//! shown, because its words depend on the language and on the names `actingctl packs` gives, which
//! come and go on their own.
//!
//! The writes (G2: the simple controls; G3: numbers and orders). A resource entry's amount
//! (`{"at_least": n}`, a whole number of at least 1 checked here, the pool's capacity by the
//! Runtime, whose `out_of_range` names it), a `choice` variable's option (its value as a JSON
//! string), a `switch` variable (`true` or `false`), an `integer` variable's number ([`number`]:
//! typed in its entry unit, checked here against its range, then its step, as the Runtime judges
//! a value, 945727d4 `crates/actingcommand-contract/src/package_variables.rs:336-347`, sent only
//! by "set", and written in milliseconds for a time unit) and an `ordered` variable's list (edited
//! as a draft, [`order_edit`], and saved whole as the compact JSON list of its option values,
//! [`ordered_json`], checked here against its counts and the 1024 bytes a value's text holds,
//! `crates/policy/src/resource_targets/v3.rs:48`, [`order_problem`]) are set; a resource entry's
//! goal, a variable's value ("back to the default"), a value no longer offered, a value of a
//! placement no bound package offers, and an other target whose id has an entry name's form
//! (`goal_name`, `[a-z0-9][a-z0-9_]*` of at most 64 bytes, the only names `EditGoal` takes) are
//! cleared, but for a name two lines share (below). A variable whose declaration this console does
//! not read as the Runtime reads it (a type it does not know, or a declaration the Runtime would
//! refuse, [`ValueControl::Unknown`]) is shown and never written, as the contract's fallback rule
//! has it (945727d4 `contracts/standard-pack-goals.md:139-141`): neither set nor put back to the
//! default; only a value stored under the name of a placement no bound package offers is cleared,
//! which writes no value. A `block` is never read: one this console does not know is shown as its
//! type's base block, the same rule. Tiers (`tier_not_built`) are only shown; the bound packages'
//! remedy parameters (`GoalView.remedy_parameters`) are not read here at all. One press is one
//! `EditGoal`, of one name, naming the page's version (`GoalView.version`) as its
//! `expected_version`. Nothing of a page is pressed while the stored record is of the old form
//! (`v1`), against which the Runtime refuses every write `record_unreadable`
//! ([`GoalsPage::writable`]).
//!
//! A press asks first only where what it loses cannot be set back from here ([`Loses`]): a value no
//! run reads (no longer offered, or of a placement not offered); a value stored that the next run
//! does not take, replaced by another option, switch, number or list or put back to the default (an
//! option no longer offered, a value of another type, a number off its range or step, a list of
//! keys no longer offered: no control here sets it again, and of a variable the game root does not
//! apply no value is set at all, `variable_not_applying`); the keys of an `ordered` value in effect
//! that its declaration no longer offers and the run leaves out (`keys_left_out`, 945727d4
//! `crates/policy/src/variables.rs:639-653`), lost as another list is saved or the value put back
//! to the default; an other target; a resource goal on a pool that takes no goal here now (without
//! a step, not targetable, not listed), cleared, as no amount is set on that pool here again; and a
//! resource goal set elsewhere (no setter, or another channel than this console's) that carries
//! what a goal set by name does not: tasks, a step, an importance, a validity. Setting a pool's
//! amount replaces its goal whole, whatever its id (`crates/runtime-host/src/goals.rs:305-311`;
//! `crates/policy/src/resource_targets/v3.rs:1539-1566`), so those are lost then too; which of them
//! it carries is read from the stored document's target of the same id, and all four are said when
//! that does not read. A goal set elsewhere that carries none of them, on a pool that takes a goal
//! here, loses nothing a goal set here does not hold again, so it is only said beside the line, not
//! asked. Back to the default of a variable whose value is in effect, a number set or a list saved
//! while the value in effect is the one stored (any of them but of an `ordered` value with keys
//! left out, above), and a goal this console set on a pool that takes one here, ask nothing; nor
//! does editing a list's draft, which writes nothing.
//!
//! A write names a name only, which the Runtime resolves by what is stored under it, in its order
//! (`crates/runtime-host/src/goals.rs:354-377`): a `clear` takes a resource entry's goal on its pool
//! first, then a stored value of the name, then a target of that id; a resource entry's `set` also
//! removes the targets of its name on other pools
//! (`crates/policy/src/resource_targets/v3.rs:1555-1556`). So a value no longer offered, an other
//! target or a variable's value, whose name another line of the page has too (an entry of any type,
//! a resource entry and a variable the game root both names among them, another value no longer
//! offered, another other target) is not cleared here, which could clear the other line's instead
//! ([`OrphanLine::shared`], [`OtherTarget::shared`], [`VariableLine::shared`]); a resource entry's
//! own clear is not in doubt, the Runtime taking its pool's goal first. An amount set on a resource
//! entry whose name an other target, or another resource entry's goal, has as its id asks first,
//! saying it removes that goal too ([`ResourceLine::same_name`]).
//!
//! The write goes on a fresh person's connection, the `ui` channel with the full receipt, at the
//! slow reads' 20 seconds ([`EDIT_IO`]), and is sent once, but for `fact_observation_not_newer`,
//! sent again at once on the same connection. What came back is told apart by its code and values
//! alone ([`answered`]): a version conflict (`current_version`), a Runtime without the write, the
//! goal refused with its `goal_problem` word and the values that locate it, or another refusal.
//!
//! What the page says, all at the pinned Runtime (0.12.76, f6e2cd26;
//! `contracts/runtime-interface.md`, "Goal request kinds";
//! `crates/actingcommand-contract/src/runtime/goals.rs`):
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
//!   and `targets`, the whole `ResourceTargetView`, whose `not_targetable` pools close the page
//!   (Workflow #455 G0: not those out of the instance's scope, and none on a page with nothing to
//!   set, [`Group::NotTargetable`]).
//!
//! A page that lists nothing to set (no entry, no value no longer offered, no other target) of a
//! game whose root has no `goals.json` (`absent`), with no pool targetable either
//! (`targets.targetable` empty), says that the game's standard pack offers no goals to set here
//! yet ([`PageState::Listed`]'s `none_offered`), as Workflow #455 G0 rules. A pool targetable may
//! take an other target `{resource, at_least}` (945727d4
//! `crates/runtime-host/src/goals.rs:288-289`, `:334-343`, `:540-546`), but for one without the
//! pack's step or importance, which is refused `pool_unvalued` (`:547-549`). Only a page with
//! something to set says what can be changed here ([`GoalsPage::something_to_set`]).
//!
//! Texts are written per locale; the contract gives no fallback, so the console's own is taken
//! ([`in_language`], the one the display titles of `pack-catalog.json` go by, `ui.json`'s before
//! Runtime 0.12.68): Chinese tries `zh-Hans`, `zh` and `zh-CN`, then `en`; English tries `en`;
//! either then takes the first text written. Variables no entry
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
//! milliseconds when it is no whole number of seconds: never a fraction ([`whole_time`]). It is
//! typed in one unit of its own ([`entry_unit`]): the largest at most the declared one that is a
//! whole number of steps, of which `min` is a whole number, and one of which on from `min` is not
//! above `max`, so that every whole number typed lands on a step and more than `min` can be typed;
//! failing every unit, the largest of which both `min` and the step are whole numbers
//! (milliseconds always are). A value in effect that is no whole number of that unit (set
//! elsewhere, or the default) is still shown, and can only be changed here to a whole number of
//! it; such a value is set with the Lab.
//!
//! The stored record's form is `targets.active.schema_version`: `v2`, one validity for the whole
//! document; `v3`, the record `EditGoal` writes, each target with a lifetime of its own or none and
//! the document with none (`contracts/resource-targets.md:1055-1060`); `v1`, an agent's old form,
//! read only, which no `EditGoal` writes against (`record_unreadable`, "neither a v2 nor a v3
//! record", `contracts/outcome-codes/catalog.json:11273-11275`); any other, a form newer than this
//! console, whose targets it does not interpret. A
//! target lapsed (the document expired, or its own lifetime is before the view's time) counts no
//! more, so it is never shown as met.
//!
//! The page takes the policy outcome gate, the policy and the fact write gate in turn while it is
//! computed (`crates/runtime-host/src/host/goals.rs:11-18`): the caller reads it seldom, as it
//! reads the task switches. A Runtime that does not know the request drops the connection, and the
//! client then says `runtime_operation_unsupported` (`crates/runtime-client/src/client.rs:1723-1787`);
//! that, or a refusal `not_implemented`, is an answer here, that this Runtime has no goals to read
//! ([`GoalsAnswer::Unsupported`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use acui_rows::outcome::Tongue;
use actingcommand_contract::codes::SharedCode;
use actingcommand_contract::{
    goal_name, remedy_token_valid, ActiveResourceTargets, GoalDeclarationRead, GoalEdit,
    GoalEntryState, GoalEntryType, GoalOrphan, GoalProblem, GoalSetBy, GoalValueSource, GoalView,
    GoalViewEntry, Initiator, ResourceTargetCondition, ResourceTargetConditionState,
    ResourceTargetObservation, ResourceTargetPendingReason, ResourceTargetView,
    ResourceTargetsRejectionReason, RuntimeChannel, MAX_GOAL_LIST_VALUES, MAX_GOAL_ORDERED_COUNT,
    MAX_GOAL_SAFE_INTEGER,
};
use actingcommand_runtime_client::RuntimeClientError;
use serde_json::{Map, Value};

use super::{configure_config, pack};
use crate::{runtime_connection, ClientFailure, Spelling};

/// The record versions the view serves (`contracts/resource-targets.md:62-64`, `:1055-1060`).
const SCHEMA_V1: &str = "actingcommand.resource-targets.v1";
const SCHEMA_V2: &str = "actingcommand.resource-targets.v2";
const SCHEMA_V3: &str = "actingcommand.resource-targets.v3";
/// The one condition a target states (`TargetCondition::AtLeast`, tagged by `kind`).
const AT_LEAST: &str = "at_least";
/// The `variable_problem` word of an `ordered` value applied in part, while `in_effect`.
const KEYS_LEFT_OUT: &str = "keys_left_out";

/// A write's connection's io timeout: the slow reads' (the console's `acui-slow` lane), since the
/// Runtime takes the fact write gate for it; the client's default of 5 seconds is a read's.
pub const EDIT_IO: Duration = Duration::from_secs(20);

/// The codes a write's answer is told apart by: the client's own of a request the Runtime does not
/// know; the shared `not_implemented`; and the host's own spellings, of a crate this one does not
/// use: another writer's change since the version named, another write in the same millisecond,
/// and the goal refused.
const OPERATION_UNSUPPORTED: &str = "runtime_operation_unsupported";
const NOT_IMPLEMENTED: &str = SharedCode::NotImplemented.as_str();
const VERSION_CONFLICT: &str = "fact_observation_version_conflict";
const NOT_NEWER: &str = "fact_observation_not_newer";
const GOAL_VALUE_INVALID: &str = "goal_value_invalid";
/// What those refusals say beyond their code (`contracts/outcome-codes/runtime-host.json`,
/// `goal_value_invalid` and `fact_observation_version_conflict`).
const CURRENT_VERSION: &str = "current_version";
const GOAL_PROBLEM: &str = "goal_problem";
const JSON_POINTER: &str = "json_pointer";
const RANGE_MIN: &str = "range_min";
const RANGE_MAX: &str = "range_max";
const POOL_ID: &str = "pool_id";
const VARIABLE_PROBLEM: &str = "variable_problem";
/// The members of a stored target a goal set by name does not carry
/// (`contracts/resource-targets.md`, the v2 and v3 targets).
const TASKS: &str = "tasks";
const SCALE: &str = "scale";
const IMPORTANCE: &str = "importance_milli";
const VALID_UNTIL: &str = "valid_until_unix_ms";

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

impl GoalsPage {
    /// Whether anything of it can be written: only a page that lists, of a record not stored in the
    /// old form. One that does not list (`not_checked_yet`, `root_failing`, `no_schedule`) refuses
    /// every `EditGoal` with its word; against a `v1` record every one is refused
    /// `record_unreadable`, the record being "neither a v2 nor a v3 record the readers read"
    /// (`contracts/outcome-codes/catalog.json:11273-11275`), so that page only shows.
    pub fn writable(&self) -> bool {
        let old = matches!(&self.stored, Stored::Document(Document { form: Form::Old, .. }));
        matches!(self.state, PageState::Listed { .. }) && !old
    }

    /// Whether the page has something to set: it lists, and lists an entry, a value no longer
    /// offered or an other target (not `nothing`). Only then does the tab say what can be changed
    /// here.
    pub fn something_to_set(&self) -> bool {
        matches!(self.state, PageState::Listed { nothing: false, .. })
    }

    /// The line of `key` ([`GoalLine::key`]).
    pub fn line(&self, key: &str) -> Option<&GoalLine> {
        self.lines.iter().find(|line| line.key().as_deref() == Some(key))
    }
}

/// Whether the page lists its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageState {
    /// It does, and how the game's `goals.json` stands. `nothing`: it lists no entry, no value
    /// no longer offered and no other target, so this instance has nothing to set now (and no
    /// line at all). `none_offered`: besides, the game has no `goals.json` (`absent`) and no pool
    /// is targetable, so the game's standard pack offers no goals to set here yet; said instead
    /// of `nothing`'s words.
    Listed { declaration: Declaration, nothing: bool, none_offered: bool },
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
    /// A pool no goal can name, and why: neither a declared entry's nor an other target's, nor one
    /// out of the instance's scope.
    Closed { pool: String, why: Closed },
}

impl GoalLine {
    /// What names the line among the page's lines, never shown: the entry's name for a resource
    /// entry or a variable, the stored name for a value no longer offered, the id for an other
    /// target, each with its kind, as names of different kinds may be the same: a resource entry
    /// and a variable of one name are both listed when the game root names them both
    /// (`goal_name_conflict`). None of a line nothing writes.
    pub fn key(&self) -> Option<String> {
        match self {
            Self::Resource(line) => Some(format!("resource:{}", line.name)),
            Self::Variable(line) => Some(format!("{VARIABLE_KEY}{}", line.name)),
            Self::Orphan(line) => Some(format!("orphan:{}", line.name)),
            Self::OtherTarget(other) => Some(format!("target:{}", other.id)),
            Self::Heading(_)
            | Self::Note { .. }
            | Self::Tier(_)
            | Self::Package(_)
            | Self::Group(_)
            | Self::Closed { .. } => None,
        }
    }
}

/// What the key of a variable's line begins with ([`GoalLine::key`]).
pub const VARIABLE_KEY: &str = "variable:";

/// The groups that close the page, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Stored values no entry offers any more (`orphans`).
    NoLongerOffered,
    /// Stored targets on no declared resource entry's pool (`other_targets`).
    OtherTargets,
    /// Pools no goal can name (`targets.not_targetable`), but those out of the instance's scope;
    /// never on a page with nothing to set.
    NotTargetable,
}

/// A `resource` entry: its name, its title, whether its pool takes a goal here, the pool's reading
/// and the goal stored on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLine {
    /// The entry's name, which a write names; never shown.
    pub name: String,
    pub title: Option<String>,
    pub pool: PoolState,
    /// None while the pool is not targetable, and while the goal waits for a reading, which says
    /// why already ([`Progress::Awaiting`]).
    pub reading: Option<Reading>,
    /// None: no goal, the default.
    pub goal: Option<StoredGoal>,
    /// Who set it; none when the record names nobody (a target a v2 document wrote).
    pub set: Option<SetBy>,
    /// A goal set elsewhere (no setter, or another channel than this console's): what of it a
    /// goal set here loses. None without a goal, and of this console's.
    pub elsewhere: Option<Carried>,
    /// The target of another pool whose id is this entry's name, an other target's or another
    /// resource entry's goal: an amount set here removes it too
    /// (`crates/policy/src/resource_targets/v3.rs:1555-1556`).
    pub same_name: Option<Removed>,
}

/// What an other target holds, as a question says it: its pool and amount, when it reads.
pub type Held = Option<(String, u64)>;

/// The target of a resource entry's name on another pool, which an amount set on the entry removes
/// too, as a question says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removed {
    /// An other target: what it holds, as its own line has it.
    Other(Held),
    /// The goal of another resource entry: that entry's title (no pool's id is shown), its amount
    /// when it reads, and what it carries that a goal set by name does not, read as a goal set
    /// elsewhere's is ([`Carried`]).
    Entry { title: Option<String>, amount: Option<u64>, carried: Carried },
}

impl ResourceLine {
    /// Whether an amount can be set on it here: its pool is targetable with the pack's own step
    /// and importance (else `pool_not_targetable` or `pool_unvalued`).
    pub fn settable(&self) -> bool {
        self.pool == PoolState::Open
    }
}

/// What a stored target carries that a goal set by name does not, and so loses as it is replaced
/// or cleared: its own tasks, step (`scale`), importance (`importance_milli`) and validity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Carried {
    pub tasks: bool,
    pub step: bool,
    pub importance: bool,
    pub validity: bool,
}

impl Carried {
    /// All four: what is said of a target whose stored document does not read here.
    pub const ALL: Self = Self { tasks: true, step: true, importance: true, validity: true };
    /// None of them: what a goal set by name carries.
    pub const NONE: Self = Self { tasks: false, step: false, importance: false, validity: false };

    /// Whether it carries any of them.
    pub fn any(self) -> bool {
        self.tasks || self.step || self.importance || self.validity
    }
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
    /// A value stored under its name, which no run reads in this build: its option's title, its
    /// value when it has none, or its text when it is no option value at all.
    pub stored: Option<String>,
    /// Who set the value stored, when one is.
    pub set: Option<SetBy>,
}

/// A package variable, placed by an entry or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableLine {
    /// The variable's name, which a write names; never shown.
    pub name: String,
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
    /// The keys of an `ordered` value stored and not applied that its declaration no longer
    /// offers, each once, in their stored order: the Runtime names them only of a value in effect
    /// (`removed_keys`, 945727d4 `crates/runtime-host/src/goals.rs:865-879`), so they are found
    /// here, by the declaration's options.
    pub gone: Vec<String>,
    /// What a press changes of it here, at the value the next run reads.
    pub control: ValueControl,
    /// A value is stored under its name (`value_json`).
    pub stored: bool,
    /// Another line of the page has its name (an entry of any type, as a resource entry the game
    /// root names it for too, `goal_name_conflict`; a value no longer offered, an other target): a
    /// clear of the name could clear that line's instead (a resource entry's goal is taken first),
    /// so its value is not cleared here (see the module's words).
    pub shared: bool,
}

impl VariableLine {
    /// Whether a value can be set on it now: one a bound package offers, which applies in the game
    /// root (else `not_offered` or `variable_not_applying`).
    pub fn settable(&self) -> bool {
        match &self.state {
            VariableState::InEffect => true,
            VariableState::NotApplied { why, .. } => !why.as_ref().is_some_and(Unapplied::by_root),
            VariableState::NotOffered => false,
        }
    }

    /// What clears the value stored under its name, when one is: "clear" of a placement no bound
    /// package offers, whose value no run reads, whatever its declaration (one with none reads as
    /// unknown, 945727d4 `crates/runtime-host/src/goals.rs:901-913`), as clearing writes no value
    /// and a value nobody can remove must not exist (`:346-347`); "back to the default" of a
    /// variable this page has a control of. None of a variable whose declaration this console does
    /// not read ([`ValueControl::Unknown`]), which is never written here. Whether it is pressed
    /// here is [`Self::clearable`].
    pub fn clear_offer(&self) -> Option<ClearOffer> {
        if !self.stored {
            return None;
        }
        match (&self.state, &self.control) {
            (VariableState::NotOffered, _) => Some(ClearOffer::Clear),
            (_, ValueControl::Unknown) => None,
            (_, ValueControl::Number { .. } | ValueControl::Ordered { .. }) => {
                Some(ClearOffer::Restore)
            }
            (_, ValueControl::Choice { .. } | ValueControl::Switch { .. }) => {
                Some(ClearOffer::Restore)
            }
        }
    }

    /// The clear pressed here, of [`Self::clear_offer`]: none of a name another line has
    /// ([`Self::shared`]), which says so instead.
    pub fn clearable(&self) -> Option<ClearOffer> {
        self.clear_offer().filter(|_| !self.shared)
    }

    /// The value stored under its name that the next run does not take, while there is one (a
    /// value not applied): none set here could be that value again (an option no longer offered, a
    /// value of another type, a number off its range or step), and of a variable the game root does
    /// not apply none is set at all, so a press that replaces it or puts it back to the default
    /// asks first, saying it.
    pub fn unused(&self) -> Option<&Shown> {
        match &self.state {
            VariableState::NotApplied { stored: Some(stored), .. } => Some(stored),
            VariableState::NotApplied { stored: None, .. }
            | VariableState::InEffect
            | VariableState::NotOffered => None,
        }
    }

    /// The value stored under its name, in words, as a question of its clear says it: of a value
    /// not applied, the one stored; else the one shown, which of a variable not offered is the one
    /// stored.
    fn stored_value(&self) -> Shown {
        match &self.state {
            VariableState::NotApplied { stored: Some(stored), .. } => stored.clone(),
            VariableState::NotApplied { stored: None, .. }
            | VariableState::InEffect
            | VariableState::NotOffered => {
                self.value.clone().unwrap_or_else(|| Shown::Raw(String::new()))
            }
        }
    }

    /// The keys of an `ordered` value in effect that the run leaves out (`keys_left_out`), lost as
    /// another list is saved (`cleared` false) or the value is put back to the default (`cleared`):
    /// what that asks about; none when there are none.
    fn left_out_lost(&self, cleared: bool) -> Option<Loses> {
        let in_effect = matches!(self.state, VariableState::InEffect);
        (in_effect && !self.left_out.is_empty())
            .then(|| Loses::LeftOut { keys: self.left_out.clone(), cleared })
    }
}

/// What a press changes of a variable here: a `choice`'s option, a `switch` (G2), an `integer`'s
/// number, an `ordered` variable's list (G3); nothing of a declaration this console does not read
/// as the Runtime reads it (`Unknown`: no `variable`, a type it does not know, or a declaration
/// the Runtime refuses), which is only shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueControl {
    /// Its options in the declaration's order, and the one the next run reads when it is one.
    Choice { options: Vec<ChoiceOption>, chosen: Option<String> },
    /// Whether the next run reads it on, when that reads as a boolean.
    Switch { on: Option<bool> },
    /// Its rule, and the number the next run reads when it reads as one (`effective_json`).
    Number { rule: NumberRule, now: Option<i64> },
    /// Its rule, and the keys the next run reads in their order (`effective_json`; none when it
    /// does not read as a list of strings): of a value not applied the default, of one applied in
    /// part the value stored without the keys no longer offered.
    Ordered { rule: OrderRule, chosen: Vec<String> },
    /// Shown, never written.
    Unknown,
}

/// An `integer` variable's rule, as the Runtime reads it (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:708-768`): safe integers `min` up to
/// `max`, on a `step` of at least 1 from `min` (1 when the declaration gives none); `max` need not
/// be on a step. A time unit's values are milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberRule {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub unit: NumberUnit,
}

/// An `integer` variable's unit: none; a text, in the language shown; or a time word, the unit
/// declared and the one typed in ([`entry_unit`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberUnit {
    Plain,
    Text(String),
    Time { declared: TimeUnit, entry: TimeUnit },
}

impl NumberRule {
    /// How many milliseconds one whole number typed is: those of the unit typed in, else 1.
    fn entry_milliseconds(&self) -> i64 {
        match self.unit {
            NumberUnit::Time { entry, .. } => entry.milliseconds(),
            NumberUnit::Plain | NumberUnit::Text(_) => 1,
        }
    }

    /// `value` in the unit typed in; none when it is no whole number of it.
    pub fn in_entry(&self, value: i64) -> Option<i64> {
        let unit = self.entry_milliseconds();
        (value % unit == 0).then(|| value / unit)
    }

    /// The step between the values that can be typed here: the unit typed in when it is a whole
    /// number of steps (the larger of the two: [`entry_unit`] takes a unit that the step divides,
    /// or failing that one that divides the step), else the step. `min` is a whole number of the
    /// unit typed in either way, so the values that can be typed here are `min` and each this far
    /// on from it.
    pub fn interval(&self) -> i64 {
        let (unit, step) = (self.entry_milliseconds(), self.step.max(1));
        if unit % step == 0 {
            unit
        } else {
            step
        }
    }
}

/// The unit an `integer` variable declared in the time unit `declared`, with `min`, `max` and
/// `step` (in milliseconds), is typed in, the one place that decides it. From `declared` down to
/// milliseconds ([`TimeUnit::LADDER`]), the first (largest) unit that is a whole number of steps,
/// of which `min` is a whole number, and one of which on from `min` is not above `max`: each whole
/// number `n` typed in it is then `n` such units, on a step from `min` (`min = m·u`, `u = s·step`,
/// so `n·u − min` is a whole number of steps), more than `min` alone can be typed, and a value
/// declared without a step (one millisecond) is typed in the unit declared where its range holds
/// one of it. Failing every unit (a step of 7 milliseconds, say, or a unit declared that is longer
/// than the range, as the Runtime's own test has, 945727d4
/// `crates/actingcommand-contract/src/package_variables_tests.rs:156-178`: minutes from 0 to 200
/// milliseconds), the first of which both `min` and the step are whole numbers, which milliseconds
/// always are. A value that is no whole number of the unit typed in is not set here.
pub fn entry_unit(declared: TimeUnit, min: i64, max: i64, step: i64) -> TimeUnit {
    let step = step.max(1);
    let below = || TimeUnit::LADDER.into_iter().skip_while(move |unit| *unit != declared);
    below()
        .find(|unit| {
            let size = unit.milliseconds();
            size.rem_euclid(step) == 0
                && min.rem_euclid(size) == 0
                && min.checked_add(size).is_some_and(|next| next <= max)
        })
        .or_else(|| {
            below().find(|unit| {
                let size = unit.milliseconds();
                min.rem_euclid(size) == 0 && step.rem_euclid(size) == 0
            })
        })
        .unwrap_or(TimeUnit::Millisecond)
}

/// An `ordered` variable's rule, as the Runtime reads it (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:786-916`): its options in the
/// declaration's order, and how many of them a value names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRule {
    pub options: Vec<OrderOption>,
    pub min_count: u64,
    pub max_count: u64,
}

/// One option of an `ordered` variable: its value, which a write names (never shown); its title in
/// the language shown (its value when it has none); and its titles in every language, in lower
/// case, which a search looks in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderOption {
    pub value: String,
    pub title: String,
    pub words: Vec<String>,
}

impl OrderRule {
    /// The option of `value`, when it is one.
    pub fn option(&self, value: &str) -> Option<&OrderOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// How many keys a list can hold: `max_count`, but never more than there are options (the
    /// Runtime does not refuse a `max_count` above them, 945727d4
    /// `crates/actingcommand-contract/src/package_variables.rs:841-863`).
    pub fn cap(&self) -> u64 {
        self.max_count.min(count_of(self.options.len()))
    }
}

/// `len` as a count.
fn count_of(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// One option of a `choice`: its value, which a write names, and its title in the language shown
/// (its value when it has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: String,
    pub title: String,
}

/// How a variable's stored value is cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearOffer {
    /// "Back to the default": the next run reads the pack's default again.
    Restore,
    /// "Clear": a value no run reads, which this page cannot set back.
    Clear,
}

/// Whether a variable's value is in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableState {
    /// The next run reads [`VariableLine::value`].
    InEffect,
    /// The next run reads the default instead: of a value stored that it does not take (`stored`,
    /// that value in words), or of a variable that does not apply in the game root; why, by the
    /// `variable_problem` word.
    NotApplied { stored: Option<Shown>, why: Option<Unapplied> },
    /// No package bound on the instance declares it.
    NotOffered,
}

/// Why a run does not read a variable's value: the `variable_problem` vocabulary (Runtime
/// `contracts/outcome-codes/catalog.json:10696-10756`), one word each.
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

    /// Whether it is the game root's judgement of the variable, which no value set here changes: a
    /// set of it is refused `variable_not_applying` (`contracts/package-variables.md:295-374`).
    pub fn by_root(&self) -> bool {
        match self {
            Self::IrreversibleByRoot
            | Self::PackageUnreadable
            | Self::GoalsUnreadable
            | Self::DeclarationConflict
            | Self::GoalNameConflict
            | Self::NameOutOfLimits
            | Self::CapRange
            | Self::RootUnchecked => true,
            Self::NotAnOption
            | Self::OutOfRange
            | Self::OffStep
            | Self::TypeChanged
            | Self::KeysRemoved
            | Self::CountOutOfRange
            | Self::KeyRepeated
            | Self::KeysLeftOut
            | Self::EntryUnreadable
            | Self::Other(_) => false,
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
    /// The name it is stored under, which its clear names; never shown.
    pub name: String,
    pub value: Shown,
    pub set: Option<SetBy>,
    /// Another line of the page has its name (an entry of any type, another value no longer
    /// offered, an other target): a clear of the name could clear that line's instead, so it is
    /// not cleared here (see the module's words).
    pub shared: bool,
}

/// A stored target on no declared resource entry's pool, by its id: as the stored document holds
/// it, or none when it does not read here (no such target, one that is no `at_least` target with
/// a resource and a whole amount, or a document of a newer form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherTarget {
    pub id: String,
    pub target: Option<OtherGoal>,
    /// Its id has an entry name's form (`goal_name`), so `EditGoal` can clear it by that name;
    /// else only the Lab's whole document can
    /// (`crates/actingcommand-contract/src/standard_pack_goals.rs:783-791`).
    pub by_name: bool,
    /// Another line of the page has its id as its name (an entry of any type, a value no longer
    /// offered, another other target): a clear of the name could clear that line's instead, so it
    /// is not cleared here (see the module's words).
    pub shared: bool,
}

impl OtherTarget {
    /// What it holds, as a question says it.
    pub fn held(&self) -> Held {
        let target = self.target.as_ref()?;
        Some((target.pool.clone(), target.goal.amount?))
    }

    /// Whether a clear of its id can be pressed here: an entry name's form, which no other line
    /// of the page has.
    pub fn clearable(&self) -> bool {
        self.by_name && !self.shared
    }
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
/// order of the console, the display titles' of `pack-catalog.json` (`ui.json`'s before Runtime
/// 0.12.68) too.
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
    let nothing = nothing_to_set(view);
    // Workflow #455 G0's rule: no `goals.json`, and `targets.targetable` empty. A pool targetable
    // may take an other target `{resource, at_least}` (945727d4
    // `crates/runtime-host/src/goals.rs:288-289`, `:334-343`, `:540-546`), but for one without the
    // pack's step or importance, which is refused `pool_unvalued` (`:547-549`).
    let none_offered = nothing
        && view.targets.targetable.is_empty()
        && matches!(declaration, Declaration::Absent);
    PageState::Listed { declaration, nothing, none_offered }
}

/// Whether `view` lists nothing to set: no entry, no value no longer offered and no other target.
fn nothing_to_set(view: &GoalView) -> bool {
    view.entries.is_empty() && view.orphans.is_empty() && view.other_targets.is_empty()
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

    /// What the stored target `id` carries that a goal set by name does not: by its own members,
    /// its validity a v3 target's own or a v1 or v2 document's; all four when it does not read
    /// here (no id, no such target, a document of a newer form).
    fn carried(&self, id: Option<&str>) -> Carried {
        let Some(target) = id.and_then(|id| self.target(id)) else {
            return Carried::ALL;
        };
        let has = |member: &str| target.get(member).is_some_and(|value| !value.is_null());
        let validity = match self.form {
            Some(Form::PerTarget) => has(VALID_UNTIL),
            Some(Form::Whole | Form::Old) => {
                self.active.is_some_and(|active| active.valid_until_unix_ms.is_some())
            }
            Some(Form::Newer(_)) | None => true,
        };
        Carried {
            tasks: target.get(TASKS).and_then(Value::as_array).is_some_and(|tasks| !tasks.is_empty()),
            step: has(SCALE),
            importance: has(IMPORTANCE),
            validity,
        }
    }
}

/// A pool's reading as a line says it: none while its goal waits for a reading, whose words say
/// why already, so that "no reading yet" is said once.
fn reading_beside(reading: Option<Reading>, goal: Option<&StoredGoal>) -> Option<Reading> {
    match (reading, goal.map(|goal| goal.progress)) {
        (Some(Reading::Pending(_)), Some(Progress::Awaiting(_))) => None,
        (reading, _) => reading,
    }
}

/// The lines of a page that lists: its entries in their order, each group of the variables no
/// entry places, then the three groups that close it, each under its title while it has a line.
/// The pools no goal can name close only a page with something to set, and never with one out of
/// the instance's scope (Workflow #455 G0).
fn lines(
    view: &GoalView,
    targets: &Targets<'_>,
    tongue: Tongue,
    pack_title: &dyn Fn(&str) -> Option<String>,
) -> Vec<GoalLine> {
    let mut lines = Vec::new();
    let shared = shared_names(view);
    let others: Vec<OtherTarget> = view
        .other_targets
        .iter()
        .map(|id| other_target(id, targets, shared.contains(id.as_str())))
        .collect();
    let line_of = |entry: &GoalViewEntry| {
        let shared = shared.contains(entry.name.as_str());
        entry_line(entry, targets, &others, &view.entries, shared, tongue)
    };
    // By the first package's id, in the order the page first names it, with its display name.
    let mut groups: Vec<(&str, String, Vec<&GoalViewEntry>)> = Vec::new();
    let mut unnamed: Vec<&GoalViewEntry> = Vec::new();
    for entry in &view.entries {
        if entry.placed {
            lines.push(line_of(entry));
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
        lines.extend(entries.into_iter().map(line_of));
    }
    if !unnamed.is_empty() {
        lines.push(GoalLine::Package(None));
        lines.extend(unnamed.into_iter().map(line_of));
    }
    if !view.orphans.is_empty() {
        lines.push(GoalLine::Group(Group::NoLongerOffered));
        lines.extend(view.orphans.iter().map(|orphan| {
            orphan_line(orphan, shared.contains(orphan.name.as_str()))
        }));
    }
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
    // Workflow #455 G0. The Runtime judges every pool of the active catalog for the instance, and
    // each one this instance cannot target goes in `not_targetable`, other games' pools too
    // (945727d4 `crates/policy/src/resource_targets.rs:1790-1817`). A pool out of the instance's
    // scope (`:834-835`) is never one this instance sets a goal on (`EditGoal` refuses a pool not
    // targetable, `crates/runtime-host/src/goals.rs:540-546`), so it is not listed. Whether a
    // fact reads a pool's amount is judged before its scope
    // (`crates/policy/src/resource_targets.rs:828-835`), so a pool of another game that nothing
    // reads is `resource_not_observable` and stays listed: a pool no goal can name carries only
    // its id and its reason, not its game
    // (`crates/actingcommand-contract/src/resource_targets.rs:179-182`). A page with nothing to
    // set lists none of them, the game's own included: it would be a table of pool ids alone.
    let closed: Vec<GoalLine> = if nothing_to_set(view) {
        Vec::new()
    } else {
        targets
            .view
            .not_targetable
            .iter()
            .filter(|closed| closed.reason != ResourceTargetsRejectionReason::ResourceOutOfScope)
            .filter(|closed| !named.contains(closed.resource.as_str()))
            .map(|closed| GoalLine::Closed {
                pool: closed.resource.clone(),
                why: closed_of(closed.reason),
            })
            .collect()
    };
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

/// The names more than one line of the page has: its entries' (of any type, placed or not), its
/// values' no longer offered and its other targets' ids, together.
fn shared_names(view: &GoalView) -> BTreeSet<&str> {
    let names = view
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .chain(view.orphans.iter().map(|orphan| orphan.name.as_str()))
        .chain(view.other_targets.iter().map(String::as_str));
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut shared: BTreeSet<&str> = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            shared.insert(name);
        }
    }
    shared
}

/// One entry of the page, by its type; `others` the page's other targets, `entries` all its
/// entries, `shared` whether another line has its name.
fn entry_line(
    entry: &GoalViewEntry,
    targets: &Targets<'_>,
    others: &[OtherTarget],
    entries: &[GoalViewEntry],
    shared: bool,
    tongue: Tongue,
) -> GoalLine {
    let declared = entry.entry.as_ref();
    match entry.entry_type {
        GoalEntryType::Heading => GoalLine::Heading(text_of(declared, "title", tongue)),
        GoalEntryType::Note => GoalLine::Note {
            text: text_of(declared, "title", tongue),
            help: text_of(declared, "help", tongue),
        },
        GoalEntryType::Resource => {
            GoalLine::Resource(resource_line(entry, targets, others, entries, tongue))
        }
        GoalEntryType::Tier => GoalLine::Tier(TierLine {
            title: text_of(declared, "title", tongue),
            default: tier_default(declared, tongue),
            stored: entry.value_json.as_deref().map(|json| tier_value(declared, json, tongue)),
            set: entry.set.as_ref().map(set_by),
        }),
        GoalEntryType::Variable => GoalLine::Variable(variable_line(entry, shared, tongue)),
    }
}

/// A `resource` entry: its pool as the view says it, and the target on it, with what that carries
/// when it was set elsewhere; and the target of its name on another pool, if any: an other target
/// among `others`, else the goal of another resource entry among `entries` whose id is this
/// entry's name. A set on this entry removes every target of its name on another pool
/// (`crates/policy/src/resource_targets/v3.rs:1555-1556` at the pinned Runtime). Its own clear is
/// not in doubt whatever other line has its name: the Runtime takes its pool's goal first.
fn resource_line(
    entry: &GoalViewEntry,
    targets: &Targets<'_>,
    others: &[OtherTarget],
    entries: &[GoalViewEntry],
    tongue: Tongue,
) -> ResourceLine {
    let pool = entry.resource.as_ref().map(|resource| resource.pool.as_str());
    let id = entry.resource.as_ref().and_then(|resource| resource.target_id.as_deref());
    let goal = entry.value_json.as_deref().map(|json| targets.goal(id, at_least(json)));
    let by_console =
        entry.set.as_ref().is_some_and(|set| matches!(set.channel, RuntimeChannel::Ui));
    let elsewhere = match &goal {
        Some(_) if !by_console => Some(targets.carried(id)),
        Some(_) | None => None,
    };
    let reading = reading_beside(pool.and_then(|pool| targets.reading(pool)), goal.as_ref());
    ResourceLine {
        name: entry.name.clone(),
        title: text_of(entry.entry.as_ref(), "title", tongue),
        pool: pool.map_or(PoolState::Unlisted, |pool| targets.pool(pool)),
        reading,
        goal,
        set: entry.set.as_ref().map(set_by),
        elsewhere,
        same_name: others
            .iter()
            .find(|other| other.id == entry.name)
            .map(|other| Removed::Other(other.held()))
            .or_else(|| entry_of_name_elsewhere(entries, &entry.name, pool, targets, tongue)),
    }
}

/// The goal of a resource entry of `entries` on another pool than `pool` whose target id is
/// `name`, as a question says it: that entry's title in `tongue`, its amount, and what the stored
/// target of that id carries ([`Targets::carried`], all four when it does not read).
fn entry_of_name_elsewhere(
    entries: &[GoalViewEntry],
    name: &str,
    pool: Option<&str>,
    targets: &Targets<'_>,
    tongue: Tongue,
) -> Option<Removed> {
    entries
        .iter()
        .filter(|other| matches!(other.entry_type, GoalEntryType::Resource))
        .filter_map(|other| other.resource.as_ref().map(|resource| (other, resource)))
        .find(|(_, resource)| {
            resource.target_id.as_deref() == Some(name) && Some(resource.pool.as_str()) != pool
        })
        .map(|(other, _)| Removed::Entry {
            title: text_of(other.entry.as_ref(), "title", tongue),
            amount: other.value_json.as_deref().and_then(at_least),
            carried: targets.carried(Some(name)),
        })
}

/// The `at_least` of a resource entry's value `{"at_least": n}`.
fn at_least(json: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(json).ok()?;
    value.get(AT_LEAST)?.as_u64()
}

/// A `tier` entry's default option's title, or its value.
fn tier_default(declared: Option<&Value>, tongue: Tongue) -> Option<String> {
    let default = declared?.get("default")?.as_str()?;
    Some(tier_option(declared, default, tongue))
}

/// A value stored under a `tier` entry's name: its option's title, its value when it has none, or
/// its text when it is no string.
fn tier_value(declared: Option<&Value>, json: &str, tongue: Tongue) -> String {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::String(value)) => tier_option(declared, &value, tongue),
        Ok(value) => value.to_string(),
        Err(_) => json.to_owned(),
    }
}

/// The title of a `tier` entry's option `value`, or the value.
fn tier_option(declared: Option<&Value>, value: &str, tongue: Tongue) -> String {
    let option = declared
        .and_then(|declared| declared.get("options"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|option| option.get("value").and_then(Value::as_str) == Some(value));
    text_of(option, "title", tongue).unwrap_or_else(|| value.to_owned())
}

/// A variable's line; `shared` whether another line has its name.
fn variable_line(entry: &GoalViewEntry, shared: bool, tongue: Tongue) -> VariableLine {
    let declaration = Declared::of(entry.variable.as_ref(), tongue);
    let title = text_of(entry.entry.as_ref(), "title", tongue)
        .or_else(|| text_of(entry.variable.as_ref(), "title", tongue));
    let state = match entry.state {
        Some(GoalEntryState::NotOffered) => VariableState::NotOffered,
        Some(GoalEntryState::NotApplied) => VariableState::NotApplied {
            stored: entry.value_json.as_deref().map(|json| declaration.shown(json, None, tongue)),
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
    let gone = match (&declaration, &state, entry.value_json.as_deref()) {
        (Declared::Ordered(rule), VariableState::NotApplied { .. }, Some(json)) => {
            gone_keys(rule, json)
        }
        _ => Vec::new(),
    };
    VariableLine {
        name: entry.name.clone(),
        title,
        value,
        default,
        set: entry.set.as_ref().map(set_by),
        state,
        left_out: entry.removed_keys.clone(),
        gone,
        control: declaration.control(entry.effective_json.as_deref(), tongue),
        stored: entry.value_json.is_some(),
        shared,
    }
}

/// The keys of the `ordered` value stored `json` that `rule` no longer offers, each once, in their
/// stored order; none of a value that is no list.
fn gone_keys(rule: &OrderRule, json: &str) -> Vec<String> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut gone: Vec<String> = Vec::new();
    for key in items.iter().filter_map(Value::as_str) {
        if rule.option(key).is_none() && !gone.iter().any(|seen| seen == key) {
            gone.push(key.to_owned());
        }
    }
    gone
}

/// A declaration's options: each value with its title per locale.
type Options = Vec<(String, Vec<(String, String)>)>;

/// What a variable's declaration says of how its values read. A `choice` and a `switch` are read
/// as leniently as they can be; an `integer` and an `ordered` variable as the Runtime reads them,
/// which writes them, so that what is checked here is what it checks: one it would refuse is
/// `Unknown`, shown and never written. Its `block` is never read: a block this console does not
/// know shows the type's base control (945727d4 `contracts/standard-pack-goals.md:139-141`).
enum Declared {
    Choice(Options),
    Integer(NumberRule),
    Switch,
    Ordered(OrderRule),
    /// No declaration, one of a type this console does not know, or one the Runtime would refuse.
    Unknown,
}

impl Declared {
    fn of(variable: Option<&Value>, tongue: Tongue) -> Self {
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
        let read = match variable.get("type").and_then(Value::as_str) {
            Some("choice") => Some(Self::Choice(options())),
            Some("integer") => number_rule(variable, tongue).map(Self::Integer),
            Some("switch") => Some(Self::Switch),
            Some("ordered") => order_rule(variable, tongue).map(Self::Ordered),
            Some(_) | None => None,
        };
        read.unwrap_or(Self::Unknown)
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
            Self::Integer(rule) => match (value.as_i64(), &rule.unit) {
                (Some(number), NumberUnit::Time { declared, .. }) => {
                    let (amount, unit) = whole_time(number, *declared);
                    Shown::Time { amount, unit }
                }
                (Some(number), NumberUnit::Text(unit)) => Shown::Words(match display {
                    Some(display) => display.to_owned(),
                    None => format!("{number} {unit}"),
                }),
                (Some(number), NumberUnit::Plain) => Shown::Number(number),
                (None, _) => raw(),
            },
            Self::Switch => value.as_bool().map_or_else(raw, Shown::Switch),
            Self::Ordered(rule) => match keys(&value) {
                Some(keys) => Shown::List(
                    keys.into_iter()
                        .map(|key| {
                            rule.option(key).map_or(key, |option| option.title.as_str()).to_owned()
                        })
                        .collect(),
                ),
                None => raw(),
            },
            Self::Unknown => plain(&value),
        }
    }

    /// What a press changes of a variable of this declaration here, at `effective`, the value the
    /// next run reads: a `choice`'s options, titled in `tongue`, with the one chosen when it is
    /// one of them; a `switch`, on when it reads as `true`; an `integer`'s rule with the number
    /// now; an `ordered` variable's rule with its keys now. Nothing of a declaration this console
    /// does not read (`Unknown`).
    fn control(&self, effective: Option<&str>, tongue: Tongue) -> ValueControl {
        let value = effective.and_then(|json| serde_json::from_str::<Value>(json).ok());
        match self {
            Self::Choice(options) => {
                let chosen = value
                    .as_ref()
                    .and_then(Value::as_str)
                    .filter(|chosen| options.iter().any(|(option, _)| option.as_str() == *chosen))
                    .map(str::to_owned);
                let options = options
                    .iter()
                    .map(|(option, titles)| ChoiceOption {
                        value: option.clone(),
                        title: in_language(titles, tongue).unwrap_or(option.as_str()).to_owned(),
                    })
                    .collect();
                ValueControl::Choice { options, chosen }
            }
            Self::Switch => ValueControl::Switch { on: value.as_ref().and_then(Value::as_bool) },
            Self::Integer(rule) => ValueControl::Number {
                rule: rule.clone(),
                now: value.as_ref().and_then(Value::as_i64),
            },
            Self::Ordered(rule) => ValueControl::Ordered {
                rule: rule.clone(),
                chosen: value
                    .as_ref()
                    .and_then(keys)
                    .map(|found| found.into_iter().map(str::to_owned).collect::<Vec<String>>())
                    .unwrap_or_default(),
            },
            Self::Unknown => ValueControl::Unknown,
        }
    }
}

/// An `integer` variable's rule, read as the Runtime reads it (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:708-784`): `min` and `max` safe integers
/// (`crates/actingcommand-contract/src/standard_pack_goals.rs:983-995`), `step` one too, 1 when
/// none is written, at least 1, `min` not above `max`; `unit` none, one of the four time words, or
/// a text, taken in `tongue`. None of anything else: a missing bound, a `unit` of another word or
/// JSON type, a text with no text in it.
fn number_rule(variable: &Value, tongue: Tongue) -> Option<NumberRule> {
    let safe = |member: &str| -> Option<i64> {
        variable
            .get(member)?
            .as_i64()
            .filter(|number| (-MAX_GOAL_SAFE_INTEGER..=MAX_GOAL_SAFE_INTEGER).contains(number))
    };
    let min = safe("min")?;
    let max = safe("max")?;
    let step = match variable.get("step") {
        None => 1,
        Some(_) => safe("step")?,
    };
    if min > max || step < 1 {
        return None;
    }
    let unit = match variable.get("unit") {
        None => NumberUnit::Plain,
        Some(Value::String(word)) => {
            let declared = TimeUnit::of(word)?;
            NumberUnit::Time { declared, entry: entry_unit(declared, min, max, step) }
        }
        Some(text @ Value::Object(_)) => {
            NumberUnit::Text(in_language(&texts(Some(text)), tongue)?.to_owned())
        }
        Some(_) => return None,
    };
    Some(NumberRule { min, max, step, unit })
}

/// An `ordered` variable's rule, read as the Runtime reads it (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:786-877`): 1 to 1024 options
/// (`standard_pack_goals.rs:91`), each a `value` of `^[a-z][a-z0-9_]{0,63}$`
/// (`crates/actingcommand-contract/src/event/payload/remedies.rs:29-36`) written once, titled in
/// `tongue` (its value when it has no title); `max_count` 1 to 32 (`standard_pack_goals.rs:93`),
/// `min_count` neither above it nor above the number of options. None of anything else. Only of
/// option values of that form is the compact JSON of a list its canonical text, by which the
/// Runtime counts a value's bytes ([`order_fits`]).
fn order_rule(variable: &Value, tongue: Tongue) -> Option<OrderRule> {
    let items = variable.get("options")?.as_array()?;
    if !(1..=MAX_GOAL_LIST_VALUES).contains(&items.len()) {
        return None;
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut options = Vec::with_capacity(items.len());
    for item in items {
        let value = item.get("value")?.as_str()?;
        if !remedy_token_valid(value) || !seen.insert(value) {
            return None;
        }
        let titles = texts(item.get("title"));
        let title = in_language(&titles, tongue).unwrap_or(value).to_owned();
        let mut words: Vec<String> = titles.iter().map(|(_, text)| text.to_lowercase()).collect();
        if words.is_empty() {
            words.push(title.to_lowercase());
        }
        options.push(OrderOption { value: value.to_owned(), title, words });
    }
    let min_count = variable.get("min_count")?.as_u64()?;
    let max_count = variable.get("max_count")?.as_u64()?;
    if max_count == 0
        || max_count > MAX_GOAL_ORDERED_COUNT
        || min_count > max_count
        || min_count > count_of(options.len())
    {
        return None;
    }
    Some(OrderRule { options, min_count, max_count })
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

/// A stored value no entry offers any more; `shared` whether another line has its name.
fn orphan_line(orphan: &GoalOrphan, shared: bool) -> GoalLine {
    GoalLine::Orphan(OrphanLine {
        name: orphan.name.clone(),
        value: Declared::Unknown.shown(&orphan.value_json, None, Tongue::En),
        set: orphan.set.as_ref().map(set_by),
        shared,
    })
}

/// An other target by its id, as the stored document holds it; `shared` whether another line has
/// its id as its name.
fn other_target(id: &str, targets: &Targets<'_>, shared: bool) -> OtherTarget {
    let target = targets.target(id).and_then(|target| {
        let pool = target.get("resource")?.as_str()?;
        let condition = target.get("condition")?;
        if condition.get("kind")?.as_str()? != AT_LEAST {
            return None;
        }
        let amount = condition.get("amount")?.as_u64()?;
        let goal = targets.goal(Some(id), Some(amount));
        Some(OtherGoal {
            pool: pool.to_owned(),
            state: targets.pool(pool),
            reading: reading_beside(targets.reading(pool), Some(&goal)),
            goal,
        })
    });
    OtherTarget { id: id.to_owned(), target, by_name: goal_name(id), shared }
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

/// One write of the page, one `EditGoal`: the name it names, and what it does to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalWrite {
    pub name: String,
    pub change: GoalChange,
}

/// Set the value of a name to a JSON text, or clear what is stored under it (the default again; a
/// resource entry's goal removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalChange {
    Set(String),
    Clear,
}

impl GoalWrite {
    fn set(name: &str, value_json: String) -> Self {
        Self { name: name.to_owned(), change: GoalChange::Set(value_json) }
    }

    fn clear(name: &str) -> Self {
        Self { name: name.to_owned(), change: GoalChange::Clear }
    }

    /// As the wire spells it.
    fn edit(&self) -> GoalEdit {
        match &self.change {
            GoalChange::Set(value_json) => {
                GoalEdit::Set { name: self.name.clone(), value_json: value_json.clone() }
            }
            GoalChange::Clear => GoalEdit::Clear { name: self.name.clone() },
        }
    }

    /// For the log only.
    pub fn line(&self) -> String {
        match &self.change {
            GoalChange::Set(value_json) => format!("set {} to {value_json}", self.name),
            GoalChange::Clear => format!("clear {}", self.name),
        }
    }
}

/// A `choice` variable's value: its option's value, a JSON string.
pub fn choice_json(option: &str) -> String {
    Value::String(option.to_owned()).to_string()
}

/// A `switch` variable's value: `true` or `false`.
pub fn switch_json(on: bool) -> String {
    Value::Bool(on).to_string()
}

/// A resource entry's value: `{"at_least": n}`, no other member (a `by_unix_ms` is
/// `deadline_unsupported`, any other `wrong_type`).
pub fn resource_json(at_least: u64) -> String {
    let mut value = Map::new();
    value.insert(AT_LEAST.to_owned(), Value::from(at_least));
    Value::Object(value).to_string()
}

/// Why the amount typed for a resource goal is not sent: only what holds of every pool, a whole
/// number of at least 1. Its pool's capacity is the Runtime's to judge, since the view gives none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmountProblem {
    /// Nothing typed.
    Empty,
    /// Not a whole number in digits.
    NotWhole,
    /// Below 1.
    BelowOne,
    /// More digits than a number here holds.
    TooLarge,
}

/// The amount `text` types for a resource goal, around spaces: a whole number of at least 1, in
/// ASCII digits, a sign of `+` allowed.
pub fn amount(text: &str) -> Result<u64, AmountProblem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AmountProblem::Empty);
    }
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AmountProblem::NotWhole);
    }
    if negative {
        return Err(AmountProblem::BelowOne);
    }
    match digits.parse::<u64>() {
        Ok(0) => Err(AmountProblem::BelowOne),
        Ok(amount) => Ok(amount),
        Err(_) => Err(AmountProblem::TooLarge),
    }
}

/// Why the number typed for an `integer` variable is not sent: what the Runtime would refuse, as it
/// judges a value it is to write, the range before the step (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:336-347`); and the number the next run
/// reads already, which would write nothing new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberProblem {
    /// Nothing typed.
    Empty,
    /// Not a whole number in digits.
    NotWhole,
    /// Outside `min..=max`, or too large to be a number at all.
    OutOfRange,
    /// Not on a step from `min`.
    OffStep,
    /// The number the next run reads already.
    Same,
}

/// The number `text` types for an `integer` variable of `rule` whose number in effect is `now`,
/// around spaces: a whole number of the unit typed in, in ASCII digits after one sign of `+` or `-`
/// at most (a full-width digit, a point, a space inside or a digit group is no whole number, as of
/// an [`amount`]), in milliseconds for a time unit; within `min..=max`, then on a step from `min`,
/// as the Runtime judges it; and not `now`, which of a value stored and not used is the default, so
/// that typing the default writes nothing either (the value stored is replaced by another number,
/// or put back to the default).
pub fn number(text: &str, rule: &NumberRule, now: Option<i64>) -> Result<i64, NumberProblem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(NumberProblem::Empty);
    }
    let digits = text.strip_prefix(|sign: char| sign == '+' || sign == '-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NumberProblem::NotWhole);
    }
    let typed = text.parse::<i64>().map_err(|_| NumberProblem::OutOfRange)?;
    let value =
        typed.checked_mul(rule.entry_milliseconds()).ok_or(NumberProblem::OutOfRange)?;
    if !(rule.min..=rule.max).contains(&value) {
        return Err(NumberProblem::OutOfRange);
    }
    if (i128::from(value) - i128::from(rule.min)) % i128::from(rule.step.max(1)) != 0 {
        return Err(NumberProblem::OffStep);
    }
    if now == Some(value) {
        return Err(NumberProblem::Same);
    }
    Ok(value)
}

/// The largest number that can be set on `rule` here: `max` need be neither on a step (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:727-740`) nor a whole number of the unit
/// typed in, so the last number at most `max` that is `min` and a whole number of
/// [`NumberRule::interval`] on. The reader keeps `min` and `max` safe integers, so it does not
/// overflow.
pub fn largest(rule: &NumberRule) -> i64 {
    let interval = rule.interval();
    rule.min + (rule.max - rule.min) / interval * interval
}

/// An `integer` variable's value: the number, spelled as the Runtime spells one (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:286-297`, `:440-445`).
pub fn integer_json(value: i64) -> String {
    Value::from(value).to_string()
}

/// One edit of an `ordered` variable's draft: an option's value added at the end, the key at a
/// place moved to another, the key at a place removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEdit {
    Add(String),
    Move { from: usize, to: usize },
    Remove(usize),
}

/// The draft `keys` of an `ordered` variable of `rule` after `edit`; none when it cannot be made:
/// an added value that is none of the options, is in the draft already, would make more keys than
/// the list holds ([`OrderRule::cap`]) or would not fit in a value ([`order_fits`]); a move from or
/// to a place outside the draft, or to where it is; a removal outside it. Fewer keys than
/// `min_count` is no reason, which [`order_problem`] says.
pub fn order_edit(rule: &OrderRule, keys: &[String], edit: &OrderEdit) -> Option<Vec<String>> {
    let mut edited = keys.to_vec();
    match edit {
        OrderEdit::Add(value) => {
            let room = count_of(keys.len()) < rule.cap();
            let offered = rule.option(value).is_some() && !keys.contains(value);
            if !(offered && room && order_fits(keys, Some(value.as_str()))) {
                return None;
            }
            edited.push(value.clone());
        }
        OrderEdit::Move { from, to } => {
            if *from >= keys.len() || *to >= keys.len() || from == to {
                return None;
            }
            let key = edited.remove(*from);
            edited.insert(*to, key);
        }
        OrderEdit::Remove(place) => {
            if *place >= keys.len() {
                return None;
            }
            edited.remove(*place);
        }
    }
    Some(edited)
}

/// An `ordered` variable's value: the compact JSON list of `keys`, `["a","b"]`.
pub fn ordered_json(keys: &[String]) -> String {
    Value::Array(keys.iter().cloned().map(Value::String).collect()).to_string()
}

/// The most bytes of a value's canonical text the Runtime stores (945727d4
/// `crates/policy/src/resource_targets/v3.rs:48`, `:486-497`); above it a write is refused
/// `value_too_large` (`crates/runtime-host/src/goals.rs:183-185`).
pub const VALUE_BYTES: usize = 1024;

/// Whether the list `keys`, with `extra` added at its end, fits in a value: its compact JSON, which
/// of option values of `[a-z0-9_]` only (as the reader holds them, [`order_rule`]) is its canonical
/// text (945727d4 `crates/policy/src/canonical.rs:37-96`: no whitespace, and a list of strings has
/// no member to sort), at most [`VALUE_BYTES`] long.
pub fn order_fits(keys: &[String], extra: Option<&str>) -> bool {
    fits_with(ordered_json(keys).len(), keys.is_empty(), extra)
}

/// Whether any option of `rule` that the draft `keys` does not hold would still fit in a value
/// added at their end ([`order_fits`]), however many keys the list holds.
pub fn order_room(rule: &OrderRule, keys: &[String]) -> bool {
    let bytes = ordered_json(keys).len();
    rule.options.iter().any(|option| {
        !keys.contains(&option.value)
            && fits_with(bytes, keys.is_empty(), Some(option.value.as_str()))
    })
}

/// Whether a list whose compact JSON is `bytes` long (`empty`: none in it) fits in a value with
/// `extra` added at its end.
fn fits_with(bytes: usize, empty: bool, extra: Option<&str>) -> bool {
    let added = extra.map_or(0, |key| usize::from(!empty) + Value::from(key).to_string().len());
    bytes + added <= VALUE_BYTES
}

/// Why a draft of an `ordered` variable is not saved, in this order: a key named twice; keys its
/// declaration no longer offers (how many), which a draft should not hold at all; fewer keys than
/// `min_count`, or more than `max_count` (that count); a text longer than a value holds (its
/// bytes). The order is this console's, not the Runtime's, which counts the keys still offered,
/// then refuses the keys it leaves out, then the bytes (945727d4
/// `crates/actingcommand-contract/src/package_variables.rs:358-385`;
/// `crates/runtime-host/src/goals.rs:315-327`, `:396-401`, `:409-423`): the keys no longer offered
/// come first, so that what is counted after them is the keys still offered, as the Runtime counts
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderProblem {
    Repeated,
    Gone(usize),
    TooFew(u64),
    TooMany(u64),
    TooLong(usize),
}

/// What keeps the draft `keys` of an `ordered` variable of `rule` from being saved, when anything
/// does ([`OrderProblem`]).
pub fn order_problem(rule: &OrderRule, keys: &[String]) -> Option<OrderProblem> {
    if keys.iter().enumerate().any(|(at, key)| keys[..at].contains(key)) {
        return Some(OrderProblem::Repeated);
    }
    let gone = keys.iter().filter(|key| rule.option(key).is_none()).count();
    if gone > 0 {
        return Some(OrderProblem::Gone(gone));
    }
    let count = count_of(keys.len());
    if count < rule.min_count {
        return Some(OrderProblem::TooFew(rule.min_count));
    }
    if count > rule.max_count {
        return Some(OrderProblem::TooMany(rule.max_count));
    }
    let bytes = ordered_json(keys).len();
    (bytes > VALUE_BYTES).then_some(OrderProblem::TooLong(bytes))
}

/// How many options a list of an `ordered` variable's options shows at most ([`order_matches`]).
pub const ORDER_SHOWN: usize = 100;
/// The fewest options of an `ordered` variable that has a search field: more than eight.
pub const ORDER_SEARCH_FROM: usize = 9;

/// One option a list of options shows: its value (never shown), its title in the language shown,
/// whether the draft has it, and whether it can be added now (not in the draft, room for another,
/// and it fits in a value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderMatch {
    pub value: String,
    pub title: String,
    pub chosen: bool,
    pub fits: bool,
}

/// The options of `rule` that `query` finds, around spaces and in any case, in the title of any
/// language (never by the option's value, which is not shown), all of them for none; in the
/// declaration's order, the first [`ORDER_SHOWN`] of them, and how many more it finds; each marked
/// against the draft `keys`.
pub fn order_matches(rule: &OrderRule, keys: &[String], query: &str) -> (Vec<OrderMatch>, usize) {
    let query = query.trim().to_lowercase();
    let room = count_of(keys.len()) < rule.cap();
    let bytes = ordered_json(keys).len();
    let mut found: Vec<OrderMatch> = Vec::new();
    let mut more = 0;
    let finds = |option: &&OrderOption| {
        query.is_empty() || option.words.iter().any(|word| word.contains(query.as_str()))
    };
    for option in rule.options.iter().filter(finds) {
        if found.len() == ORDER_SHOWN {
            more += 1;
            continue;
        }
        let chosen = keys.contains(&option.value);
        found.push(OrderMatch {
            value: option.value.clone(),
            title: option.title.clone(),
            chosen,
            fits: !chosen && room && fits_with(bytes, keys.is_empty(), Some(option.value.as_str())),
        });
    }
    (found, more)
}

/// The key that a refusal's JSON pointer `pointer` names in the list `value_json` that was sent: an
/// `ordered` value's item at fault is named `/i` (945727d4
/// `crates/runtime-host/src/goals.rs:469-495`); the string at that place, none of another pointer,
/// place or item.
pub fn pointed_key(value_json: &str, pointer: &str) -> Option<String> {
    let place = pointer.strip_prefix('/')?;
    if place.is_empty() || !place.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let place: usize = place.parse().ok()?;
    let value: Value = serde_json::from_str(value_json).ok()?;
    Some(value.as_array()?.get(place)?.as_str()?.to_owned())
}

/// What a press on a line does: set an amount or a number (as typed), clear, pick a `choice`'s
/// option (by its place among the options), turn a `switch`, save an `ordered` variable's list (its
/// keys in order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalAct {
    Amount(String),
    Clear,
    Pick(usize),
    Switch(bool),
    Order(Vec<String>),
}

/// What one press writes, and what it loses that this page cannot set back, which it asks about
/// first; none: it goes at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Press {
    pub write: GoalWrite,
    pub asks: Option<Loses>,
}

/// What a press loses that this page cannot set back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loses {
    /// A value stored that no run reads, or that no value set here replaces again, cleared: no
    /// longer offered, of a placement no bound package offers, of a variable whose value stored
    /// the next run does not take. The value.
    Value(Shown),
    /// A value stored that the next run does not take, replaced by an option, a switch, a number
    /// or a list set here: the value ([`VariableLine::unused`]).
    Replaced(Shown),
    /// The keys of an `ordered` value in effect that its declaration no longer offers, which the
    /// run leaves out (`keys_left_out`, [`VariableLine::left_out`]), as stored: lost as another
    /// list is saved, or as the value is put back to the default (`cleared`). No list set here
    /// holds them again.
    LeftOut { keys: Vec<String>, cleared: bool },
    /// An other target, with its pool and amount when it reads.
    Target(Held),
    /// A declared resource's goal set elsewhere, cleared (`cleared`) or replaced by a goal set
    /// here: what it carries that one set here does not.
    Elsewhere { cleared: bool, carried: Carried },
    /// A declared resource's goal cleared on a pool that takes no goal here now (without a step,
    /// not targetable, not listed), so that no amount is set on it here again: its amount when it
    /// reads; and what it carries that a goal set by name does not, when it was set elsewhere and
    /// carries any.
    Unsettable { amount: Option<u64>, carried: Option<Carried> },
    /// A declared resource's amount set here, which removes the target of the entry's name on
    /// another pool too ([`ResourceLine::same_name`]): that target, an other target or another
    /// entry's goal; and what the goal on the entry's own pool, set elsewhere, carries that one
    /// set here does not, when it carries any.
    SameName { removed: Removed, carried: Option<Carried> },
}

/// What a press `act` on `line` writes, the one place that decides it: none when it writes nothing
/// (the line takes no such press, the value is as pressed already, the amount or number typed is
/// not one, the list is not one to save). A resource entry's amount only on a pool that takes a
/// goal here, a variable's value only while one can be set on it, and only of a declaration this
/// console reads; a clear wherever something is stored to clear, but of a value no longer offered,
/// an other target or a variable's value whose name another line of the page has, or a value of a
/// declaration this console does not read but where no bound package offers it. See the module's
/// words for what asks first; what of the page that depends on is on the line already (`shared`,
/// `same_name`, `left_out`).
pub fn press(line: &GoalLine, act: &GoalAct) -> Option<Press> {
    match (line, act) {
        (GoalLine::Resource(line), GoalAct::Amount(text)) if line.settable() => {
            let at_least = amount(text).ok()?;
            let carried = line.elsewhere.filter(|carried| carried.any());
            let asks = match (&line.same_name, carried) {
                (Some(removed), carried) => {
                    Some(Loses::SameName { removed: removed.clone(), carried })
                }
                (None, Some(carried)) => Some(Loses::Elsewhere { cleared: false, carried }),
                (None, None) => None,
            };
            Some(Press { write: GoalWrite::set(&line.name, resource_json(at_least)), asks })
        }
        // On a pool that takes no goal here now, no amount is set back here: asked whoever set it.
        (GoalLine::Resource(line), GoalAct::Clear) => {
            let goal = line.goal.as_ref()?;
            let carried = line.elsewhere.filter(|carried| carried.any());
            let asks = if line.settable() {
                carried.map(|carried| Loses::Elsewhere { cleared: true, carried })
            } else {
                Some(Loses::Unsettable { amount: goal.amount, carried })
            };
            Some(Press { write: GoalWrite::clear(&line.name), asks })
        }
        // A value stored that the next run does not take is lost as another is set: asked.
        (GoalLine::Variable(line), GoalAct::Pick(index)) if line.settable() => {
            let ValueControl::Choice { options, chosen } = &line.control else {
                return None;
            };
            let option = options.get(*index)?;
            if chosen.as_deref() == Some(option.value.as_str()) {
                return None;
            }
            let write = GoalWrite::set(&line.name, choice_json(&option.value));
            Some(Press { write, asks: line.unused().cloned().map(Loses::Replaced) })
        }
        (GoalLine::Variable(line), GoalAct::Switch(on)) if line.settable() => {
            let ValueControl::Switch { on: now } = &line.control else {
                return None;
            };
            if *now == Some(*on) {
                return None;
            }
            let write = GoalWrite::set(&line.name, switch_json(*on));
            Some(Press { write, asks: line.unused().cloned().map(Loses::Replaced) })
        }
        // The number typed, checked as the Runtime checks it; the number in effect writes nothing.
        (GoalLine::Variable(line), GoalAct::Amount(text)) if line.settable() => {
            let ValueControl::Number { rule, now } = &line.control else {
                return None;
            };
            let value = number(text, rule, *now).ok()?;
            let write = GoalWrite::set(&line.name, integer_json(value));
            Some(Press { write, asks: line.unused().cloned().map(Loses::Replaced) })
        }
        // A list saved: not the one in effect, and none the Runtime would refuse. A value stored
        // that the next run does not take is lost as another is set, and so are the keys of one in
        // effect that the run leaves out: asked.
        (GoalLine::Variable(line), GoalAct::Order(keys)) if line.settable() => {
            let ValueControl::Ordered { rule, chosen } = &line.control else {
                return None;
            };
            if keys == chosen || order_problem(rule, keys).is_some() {
                return None;
            }
            let write = GoalWrite::set(&line.name, ordered_json(keys));
            let asks = match line.unused() {
                Some(stored) => Some(Loses::Replaced(stored.clone())),
                None => line.left_out_lost(false),
            };
            Some(Press { write, asks })
        }
        // Back to the default asks nothing while the value stored is in effect, but for the keys
        // of an `ordered` one that the run leaves out, which no list set here holds again; one the
        // next run does not take, for any reason (of a variable the game root does not apply too,
        // on which no value is set here, `variable_not_applying`), is lost as a value not offered
        // is. A name another line has is not cleared here.
        (GoalLine::Variable(line), GoalAct::Clear) => {
            let asks = match line.clearable()? {
                ClearOffer::Restore if line.unused().is_none() => line.left_out_lost(true),
                ClearOffer::Restore | ClearOffer::Clear => Some(Loses::Value(line.stored_value())),
            };
            Some(Press { write: GoalWrite::clear(&line.name), asks })
        }
        (GoalLine::Orphan(line), GoalAct::Clear) if !line.shared => Some(Press {
            write: GoalWrite::clear(&line.name),
            asks: Some(Loses::Value(line.value.clone())),
        }),
        (GoalLine::OtherTarget(other), GoalAct::Clear) if other.clearable() => Some(Press {
            write: GoalWrite::clear(&other.id),
            asks: Some(Loses::Target(other.held())),
        }),
        _ => None,
    }
}

/// Why the Runtime refused a write `goal_value_invalid`: the `goal_problem` vocabulary (Runtime
/// `contracts/outcome-codes/catalog.json:11211-11289`), one word each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteProblem {
    NameUnknown,
    HoldsNoValue,
    TierNotBuilt,
    RemedyParameter,
    VariableNotApplying,
    NotOffered,
    ValueNotJson,
    WrongType,
    OutOfRange,
    OffStep,
    NotAnOption,
    KeyRepeated,
    CountOutOfRange,
    ValueTooLarge,
    PoolNotTargetable,
    PoolUnvalued,
    DeadlineUnsupported,
    ValueNotSet,
    RecordUnreadable,
    RecordOutOfLimits,
    NotCheckedYet,
    RootFailing,
    NoSchedule,
    /// A word of a newer Runtime, by itself.
    Other(String),
}

impl WriteProblem {
    /// Whether it is one of the words a variable's value is refused with by its declaration's rule
    /// (945727d4 `crates/runtime-host/src/goals.rs:432-473`): out of range, off its step, not an
    /// option, a count out of range, a key named twice, of the wrong type. What this page sends is
    /// checked here by the same rules, so only a rule the Runtime holds newer than the page's says
    /// one: the page is read again at once.
    pub fn of_the_rule(&self) -> bool {
        matches!(
            self,
            Self::OutOfRange
                | Self::OffStep
                | Self::NotAnOption
                | Self::CountOutOfRange
                | Self::KeyRepeated
                | Self::WrongType
        )
    }

    /// The word `word`, by the pinned contract's own vocabulary.
    pub fn of(word: &str) -> Self {
        match GoalProblem::ALL.iter().find(|problem| problem.as_str() == word) {
            Some(problem) => Self::from_contract(*problem),
            None => Self::Other(word.to_owned()),
        }
    }

    fn from_contract(problem: GoalProblem) -> Self {
        match problem {
            GoalProblem::NameUnknown => Self::NameUnknown,
            GoalProblem::HoldsNoValue => Self::HoldsNoValue,
            GoalProblem::TierNotBuilt => Self::TierNotBuilt,
            GoalProblem::RemedyParameter => Self::RemedyParameter,
            GoalProblem::VariableNotApplying => Self::VariableNotApplying,
            GoalProblem::NotOffered => Self::NotOffered,
            GoalProblem::ValueNotJson => Self::ValueNotJson,
            GoalProblem::WrongType => Self::WrongType,
            GoalProblem::OutOfRange => Self::OutOfRange,
            GoalProblem::OffStep => Self::OffStep,
            GoalProblem::NotAnOption => Self::NotAnOption,
            GoalProblem::KeyRepeated => Self::KeyRepeated,
            GoalProblem::CountOutOfRange => Self::CountOutOfRange,
            GoalProblem::ValueTooLarge => Self::ValueTooLarge,
            GoalProblem::PoolNotTargetable => Self::PoolNotTargetable,
            GoalProblem::PoolUnvalued => Self::PoolUnvalued,
            GoalProblem::DeadlineUnsupported => Self::DeadlineUnsupported,
            GoalProblem::ValueNotSet => Self::ValueNotSet,
            GoalProblem::RecordUnreadable => Self::RecordUnreadable,
            GoalProblem::RecordOutOfLimits => Self::RecordOutOfLimits,
            GoalProblem::NotCheckedYet => Self::NotCheckedYet,
            GoalProblem::RootFailing => Self::RootFailing,
            GoalProblem::NoSchedule => Self::NoSchedule,
        }
    }
}

/// A `goal_value_invalid`: its word, and the values that locate it, each when the Runtime gives it:
/// the range (`range_min`, `range_max`: a number's bounds, a resource's 1 to its pool's capacity, a
/// list's counts, the 1024 bytes a value's text holds, the targets or entries a record holds), the
/// member or item at fault (`json_pointer`), the pool (`pool_id`) and why a variable does not apply
/// (`variable_problem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteInvalid {
    pub problem: WriteProblem,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub pointer: Option<String>,
    pub pool: Option<String>,
    pub variable: Option<Unapplied>,
}

/// A refusal the Runtime answered and went on after, by its receipt's code and values alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answered {
    /// The record is at another version than the one named: another writer changed it since the
    /// page was read, and nothing changed. `current` only for the log.
    Conflict { current: Option<u64> },
    /// This Runtime has no such write.
    NotImplemented,
    /// Another write of the record landed in the same millisecond.
    NotNewer,
    /// The name or the value is refused, and why.
    Invalid(WriteInvalid),
    /// Any other refusal: its codes are in the failure.
    Other,
}

/// What a refusal of `code` with `values` says, the one place that tells them apart.
pub fn answered(code: &str, values: &Map<String, Value>) -> Answered {
    let integer = |key: &str| values.get(key).and_then(Value::as_i64);
    let text = |key: &str| values.get(key).and_then(Value::as_str).map(str::to_owned);
    match code {
        VERSION_CONFLICT => {
            Answered::Conflict { current: values.get(CURRENT_VERSION).and_then(Value::as_u64) }
        }
        NOT_IMPLEMENTED => Answered::NotImplemented,
        NOT_NEWER => Answered::NotNewer,
        GOAL_VALUE_INVALID => Answered::Invalid(WriteInvalid {
            problem: WriteProblem::of(&text(GOAL_PROBLEM).unwrap_or_default()),
            min: integer(RANGE_MIN),
            max: integer(RANGE_MAX),
            pointer: text(JSON_POINTER),
            pool: text(POOL_ID),
            variable: text(VARIABLE_PROBLEM).as_deref().map(Unapplied::of),
        }),
        _ => Answered::Other,
    }
}

/// What became of one [`edit`], and whether it was sent a second time after
/// `fact_observation_not_newer`, which only the log says.
#[derive(Debug, Clone)]
pub struct WriteAnswer {
    pub outcome: WriteOutcome,
    pub resent: bool,
}

/// What became of one write.
#[derive(Debug, Clone)]
pub enum WriteOutcome {
    /// The Runtime took it: the record's version after it, and the id of a target on the same pool
    /// it replaced, both for the log (the next write names the version of a page read after this).
    Edited { version: u64, replaced: Option<String> },
    /// Another writer changed the record since its version was read: nothing changed.
    Conflict { current: Option<u64> },
    /// Refused, nothing changed: why, and the Runtime's codes.
    Refused { why: WriteRefusal, failure: ClientFailure },
    /// This Runtime has no such write: the client's word that it does not know the request, or a
    /// refusal `not_implemented`; `code` says which.
    Unsupported { code: &'static str, failure: ClientFailure },
    /// No connection, so nothing was sent.
    NotSent(ClientFailure),
    /// Sent, or perhaps sent, and nothing says what became of it: a receipt that did not come in
    /// time, a failed write, a refusal that halted the host. It is not sent again: the page is read
    /// again first, and since the write named a version, one sent again after it took would only
    /// meet a version conflict.
    Unconfirmed(ClientFailure),
}

/// Why the Runtime refused a write.
#[derive(Debug, Clone)]
pub enum WriteRefusal {
    /// Another write of the record landed in the same millisecond, the second time too.
    NotNewer,
    Invalid(WriteInvalid),
    Other,
}

/// One person's write `write` of the goals of the instance `alias`: a fresh person's connection at
/// [`EDIT_IO`], one `EditGoal` of `write` only while the record is at `expected`, the version of the
/// page the press was on, and the connection let go. Sent once, whatever comes back, but for one
/// `fact_observation_not_newer`, sent again at once on the same connection.
pub fn edit(root: &Path, alias: &str, write: &GoalWrite, expected: u64) -> WriteAnswer {
    let client = match runtime_connection(configure_config(root).with_io_timeout(EDIT_IO)) {
        Ok(client) => client,
        Err(error) => {
            let outcome = WriteOutcome::NotSent(ClientFailure::from(error));
            return WriteAnswer { outcome, resent: false };
        }
    };
    let send = || client.edit_goal(alias, Some(expected), write.edit());
    let (answer, resent) = match send() {
        Err(error) if pack::refused_not_newer(&error) => (send(), true),
        answer => (answer, false),
    };
    let outcome = match answer {
        Ok(edited) => {
            WriteOutcome::Edited { version: edited.version, replaced: edited.replaced_target_id }
        }
        Err(error) => failed(error),
    };
    WriteAnswer { outcome, resent }
}

/// What a failed write after the connection comes to, before its failure's codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// This Runtime has no such write, by the code that says so.
    Unsupported(&'static str),
    /// Nothing says what became of it.
    Unconfirmed,
    /// A refusal the Runtime answered and went on after, other than `not_implemented`.
    Answered(Answered),
}

/// What a failed write comes to, the one place that tells it, in this order: the client's own code
/// `client` that the Runtime does not know the request; anything but a refusal the Runtime answered
/// and went on after, of a request nothing leaves uncertain (`settled`: a refusal that halted the
/// host is not), unconfirmed, since a failed write may have got the request out; then the refusal
/// by its receipt's `code` and `values` ([`answered`]), `not_implemented` saying the Runtime has no
/// such write.
pub fn failure_of(client: &str, settled: bool, code: &str, values: &Map<String, Value>) -> Failed {
    if client == OPERATION_UNSUPPORTED {
        return Failed::Unsupported(OPERATION_UNSUPPORTED);
    }
    if !settled {
        return Failed::Unconfirmed;
    }
    match answered(code, values) {
        Answered::NotImplemented => Failed::Unsupported(NOT_IMPLEMENTED),
        said => Failed::Answered(said),
    }
}

/// A failed write after the connection ([`failure_of`]), with its codes.
fn failed(error: RuntimeClientError) -> WriteOutcome {
    let class = {
        let answer = error.receipt_outcome();
        let code = answer.map_or("", |outcome| outcome.code().as_str());
        let empty = Map::new();
        let values = answer.map_or(&empty, |outcome| outcome.values());
        failure_of(error.code().spelling(), pack::answered(&error), code, values)
    };
    let failure = ClientFailure::from(error);
    match class {
        Failed::Unsupported(code) => WriteOutcome::Unsupported { code, failure },
        Failed::Unconfirmed => WriteOutcome::Unconfirmed(failure),
        Failed::Answered(Answered::Conflict { current }) => WriteOutcome::Conflict { current },
        Failed::Answered(Answered::NotImplemented) => {
            WriteOutcome::Unsupported { code: NOT_IMPLEMENTED, failure }
        }
        Failed::Answered(Answered::NotNewer) => {
            WriteOutcome::Refused { why: WriteRefusal::NotNewer, failure }
        }
        Failed::Answered(Answered::Invalid(invalid)) => {
            WriteOutcome::Refused { why: WriteRefusal::Invalid(invalid), failure }
        }
        Failed::Answered(Answered::Other) => {
            WriteOutcome::Refused { why: WriteRefusal::Other, failure }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        amount, answered, choice_json, entry_unit, failure_of, in_language, integer_json, largest,
        newer_than_this_console, number, order_edit, order_fits, order_matches, order_problem,
        order_room, ordered_json, page, pointed_key, press, resource_json, switch_json, whole_time,
        AmountProblem, Answered, Carried, ChoiceOption, ClearOffer, Closed, Declaration, Document,
        Failed, Form, GoalAct, GoalChange, GoalLine, GoalWrite, GoalsPage, Group, LabEntry, Loses,
        NumberProblem, NumberRule, NumberUnit, OrderEdit, OrderOption, OrderProblem, OrderRule,
        OrphanLine, OtherGoal, OtherTarget, PageState, Pending, PoolState, Press, Progress, Reading,
        Removed, ResourceLine, SetBy, Setter, Shown, Stored, StoredGoal, TierLine, TimeUnit,
        Unapplied, ValueControl, VariableLine, VariableState, WriteInvalid, WriteProblem,
        ORDER_SHOWN, RECEIPT_DECODE_FAILED, VALUE_BYTES,
    };
    use acui_rows::outcome::Tongue;
    use actingcommand_contract::{GoalProblem, GoalView};
    use serde_json::{json, Map, Value};

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

    fn switch_line(name: &str, title: &str) -> GoalLine {
        GoalLine::Variable(VariableLine {
            name: name.to_owned(),
            title: Some(title.to_owned()),
            value: Some(Shown::Switch(false)),
            default: None,
            set: None,
            state: VariableState::InEffect,
            left_out: Vec::new(),
            gone: Vec::new(),
            control: ValueControl::Switch { on: Some(false) },
            stored: false,
            shared: false,
        })
    }

    /// An option of an `ordered` rule titled in one language.
    fn order_option(value: &str, title: &str) -> OrderOption {
        OrderOption {
            value: value.to_owned(),
            title: title.to_owned(),
            words: vec![title.to_lowercase()],
        }
    }

    /// The `ordered` declaration's rule in English.
    fn item_rule() -> OrderRule {
        OrderRule {
            options: vec![order_option("item_a", "Item A"), order_option("item_b", "Item B")],
            min_count: 0,
            max_count: 2,
        }
    }

    /// The `integer` declaration's rule, of `unit`.
    fn rounds_rule(unit: NumberUnit) -> NumberRule {
        NumberRule { min: 0, max: 864_000_000, step: 1, unit }
    }

    fn strings(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_owned()).collect()
    }

    fn goal(amount: u64, progress: Progress) -> StoredGoal {
        StoredGoal { amount: Some(amount), progress, until_ms: None, lapsed: false }
    }

    /// The `choice` declaration's two options in English.
    fn item_options() -> Vec<ChoiceOption> {
        vec![
            ChoiceOption { value: "item_a".to_owned(), title: "Item A".to_owned() },
            ChoiceOption { value: "item_b".to_owned(), title: "Item B".to_owned() },
        ]
    }

    /// Carried: nothing a goal set by name does not carry.
    const NOTHING: Carried = Carried { tasks: false, step: false, importance: false, validity: false };

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
            PageState::Listed {
                declaration: Declaration::Accepted,
                nothing: false,
                none_offered: false,
            }
        );
        assert!(english.something_to_set());
        assert_eq!(
            english.lines,
            vec![
                GoalLine::Heading(Some("Daily".to_owned())),
                GoalLine::Note {
                    text: Some("Read me".to_owned()),
                    help: Some("More words".to_owned()),
                },
                GoalLine::Resource(ResourceLine {
                    name: "gold".to_owned(),
                    title: Some("gold".to_owned()),
                    pool: PoolState::Open,
                    reading: Some(Reading::Read { current: 1200, at_ms: OBSERVED }),
                    goal: Some(goal(
                        5000,
                        Progress::Computed { current: 1200, at_ms: OBSERVED, gap: 3800 },
                    )),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::CommandLine)) }),
                    // Set by the Lab, as a goal by name: nothing it carries is lost here.
                    elsewhere: Some(NOTHING),
                    same_name: None,
                }),
                GoalLine::Tier(TierLine {
                    title: Some("Pace".to_owned()),
                    default: Some("Slow".to_owned()),
                    stored: None,
                    set: None,
                }),
                GoalLine::Variable(VariableLine {
                    name: "pick".to_owned(),
                    title: Some("Item".to_owned()),
                    value: Some(Shown::Words("Item A".to_owned())),
                    default: Some(Shown::Words("Item B".to_owned())),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Console }),
                    state: VariableState::InEffect,
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    control: ValueControl::Choice {
                        options: item_options(),
                        chosen: Some("item_a".to_owned()),
                    },
                    stored: true,
                    shared: false,
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
                // The options in the language shown, their values as declared.
                assert_eq!(
                    line.control,
                    ValueControl::Choice {
                        options: vec![
                            ChoiceOption { value: "item_a".to_owned(), title: "甲".to_owned() },
                            ChoiceOption { value: "item_b".to_owned(), title: "乙".to_owned() },
                        ],
                        chosen: Some("item_a".to_owned()),
                    }
                );
            }
            line => panic!("{line:?}"),
        }
        // Only a page that lists is written, and each line that writes is found by its key.
        assert!(english.writable());
        let keys: Vec<Option<String>> = english.lines.iter().map(GoalLine::key).collect();
        assert_eq!(
            keys,
            vec![
                None,
                None,
                Some("resource:gold".to_owned()),
                None,
                Some("variable:pick".to_owned()),
            ]
        );
        assert_eq!(english.line("variable:pick"), english.lines.get(4));
        assert_eq!(english.line("resource:pick"), None);
        assert_eq!(english.line("variable:pace"), None);
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
            assert!(!page.writable());
            assert!(!page.something_to_set());
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
            assert_eq!(
                page.state,
                PageState::Listed { declaration: expected, nothing: false, none_offered: false }
            );
            assert_eq!(
                page.lines,
                vec![GoalLine::Package(None), switch_line("extra", "Also the extra")]
            );
            assert!(page.something_to_set());
        }
    }

    #[test]
    fn a_page_with_nothing_to_set_says_so() {
        let page = read(&view(state("accepted"), json!([]), json!({}), no_targets()));
        assert_eq!(
            page.state,
            PageState::Listed {
                declaration: Declaration::Accepted,
                nothing: true,
                none_offered: false,
            }
        );
        assert!(page.lines.is_empty(), "{:?}", page.lines);
        assert_eq!(page.stored, Stored::None);
        assert!(!page.something_to_set());
        // G0: pools no goal can name no longer close it, the game's own (`unmapped_task`) as little
        // as another game's: nothing is there to set all the same, and the page says only that.
        let closed = targets(
            json!([]),
            json!([
                closed_pool("pool.gems", "unmapped_task"),
                closed_pool("pool.keys", "resource_not_observable"),
                closed_pool("pool.relics", "resource_out_of_scope"),
            ]),
            None,
        );
        let page = read(&view(state("absent"), json!([]), json!({}), closed));
        assert_eq!(
            page.state,
            PageState::Listed { declaration: Declaration::Absent, nothing: true, none_offered: true }
        );
        assert!(page.lines.is_empty(), "{:?}", page.lines);
        assert!(!page.something_to_set());
    }

    /// Workflow #455 G0: "this game's standard pack offers no goals to set here yet" only while
    /// all three hold: the page lists nothing to set, no pool is targetable, and the game has no
    /// `goals.json` (`absent`). Without any one of them the page stands as before.
    #[test]
    fn a_pack_offering_no_goals_is_said_only_when_all_three_hold() {
        let closed = json!([closed_pool("pool.gems", "unmapped_task")]);
        let no_pool = || targets(json!([]), closed.clone(), None);
        let stands = |declaration: Declaration, nothing: bool, none_offered: bool| {
            PageState::Listed { declaration, nothing, none_offered }
        };
        let page = read(&view(state("absent"), json!([]), json!({}), no_pool()));
        assert_eq!(page.state, stands(Declaration::Absent, true, true));
        assert!(page.lines.is_empty(), "{:?}", page.lines);
        assert!(!page.something_to_set());

        // Something to set: an entry, a value no longer offered, an other target.
        let entry = json!([unplaced_switch("extra", "Also the extra", &["pack.a"])]);
        let page = read(&view(state("absent"), entry, json!({}), no_pool()));
        assert_eq!(page.state, stands(Declaration::Absent, false, false));
        assert!(page.something_to_set());
        for rest in [
            json!({"orphans": [{"name": "old_route", "value_json": "\"a\""}]}),
            json!({"other_targets": ["wood"]}),
        ] {
            let page = read(&view(state("absent"), json!([]), rest.clone(), no_pool()));
            assert_eq!(page.state, stands(Declaration::Absent, false, false), "{rest}");
            assert!(page.something_to_set(), "{rest}");
        }

        // A targetable pool: still nothing on the page, but an other target could be set on it.
        // Its pools no goal can name are not listed either.
        let open = targets(json!([pool("pool.wood", true, true, None)]), closed.clone(), None);
        let page = read(&view(state("absent"), json!([]), json!({}), open));
        assert_eq!(page.state, stands(Declaration::Absent, true, false));
        assert!(page.lines.is_empty(), "{:?}", page.lines);
        assert!(!page.something_to_set());

        // Another declaration's state: nothing to set, as before.
        for (declaration, expected) in [
            (state("accepted"), Declaration::Accepted),
            (state("refused"), Declaration::NotTaken),
            (state("unreadable"), Declaration::NotTaken),
            (state("not_read"), Declaration::NotRead),
        ] {
            let page = read(&view(declaration, json!([]), json!({}), no_pool()));
            assert_eq!(page.state, stands(expected, true, false));
            assert!(page.lines.is_empty(), "{:?}", page.lines);
        }
    }

    /// Workflow #455 G0: a pool out of the instance's scope (`resource_out_of_scope`) is never
    /// listed among the pools no goal can name, even on a page with something to set; a pool
    /// nothing reads (`resource_not_observable`, another game's or not) and the game's own pool no
    /// task produces still are. A resource entry on a pool out of scope says so on its own line.
    #[test]
    fn pools_out_of_scope_are_never_listed_and_the_others_still_are() {
        let closed = targets(
            json!([pool("pool.wood", true, true, Some(3))]),
            json!([
                closed_pool("pool.ap", "unmapped_task"),
                closed_pool("pool.keys", "resource_out_of_scope"),
                closed_pool("pool.gems", "resource_not_observable"),
                closed_pool("pool.coins", "resource_out_of_scope"),
            ]),
            None,
        );
        let entry = json!([unplaced_switch("extra", "Also the extra", &["pack.a"])]);
        for declaration in [state("absent"), state("accepted")] {
            let page = read(&view(declaration, entry.clone(), json!({}), closed.clone()));
            assert!(page.something_to_set());
            assert_eq!(
                page.lines,
                vec![
                    GoalLine::Package(None),
                    switch_line("extra", "Also the extra"),
                    GoalLine::Group(Group::NotTargetable),
                    GoalLine::Closed { pool: "pool.ap".to_owned(), why: Closed::UnmappedTask },
                    GoalLine::Closed { pool: "pool.gems".to_owned(), why: Closed::NotObservable },
                ]
            );
        }
        // Only pools out of scope: the group has no line, so no title either.
        let out = targets(json!([]), json!([closed_pool("pool.keys", "resource_out_of_scope")]), None);
        let page = read(&view(state("accepted"), entry, json!({}), out));
        assert_eq!(
            page.lines,
            vec![GoalLine::Package(None), switch_line("extra", "Also the extra")]
        );
        // A line that names a pool out of scope still says why on that line, and the pool still
        // has no line of its own.
        let keys = json!([resource("keys", "pool.keys", None, None)]);
        let page = read(&view(state("accepted"), keys, json!({}), closed));
        assert_eq!(
            page.lines,
            vec![
                GoalLine::Resource(ResourceLine {
                    name: "keys".to_owned(),
                    title: Some("keys".to_owned()),
                    pool: PoolState::Closed(Closed::OutOfScope),
                    reading: None,
                    goal: None,
                    set: None,
                    elsewhere: None,
                    same_name: None,
                }),
                GoalLine::Group(Group::NotTargetable),
                GoalLine::Closed { pool: "pool.ap".to_owned(), why: Closed::UnmappedTask },
                GoalLine::Closed { pool: "pool.gems".to_owned(), why: Closed::NotObservable },
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
        let lines = read(&view).lines;
        assert_eq!(
            lines,
            vec![
                // G1's follow-up: the value stored that is not used, in words.
                GoalLine::Variable(VariableLine {
                    name: "rounds".to_owned(),
                    title: Some("Rounds".to_owned()),
                    value: Some(Shown::Number(0)),
                    default: None,
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Mcp }),
                    state: VariableState::NotApplied {
                        stored: Some(Shown::Number(900_000_000)),
                        why: Some(Unapplied::OutOfRange),
                    },
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    control: ValueControl::Number {
                        rule: rounds_rule(NumberUnit::Plain),
                        now: Some(0),
                    },
                    stored: true,
                    shared: false,
                }),
                GoalLine::Variable(VariableLine {
                    name: "extra".to_owned(),
                    title: Some("Also the extra".to_owned()),
                    value: Some(Shown::Switch(false)),
                    default: None,
                    set: None,
                    state: VariableState::NotApplied {
                        stored: None,
                        why: Some(Unapplied::IrreversibleByRoot),
                    },
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    control: ValueControl::Switch { on: Some(false) },
                    stored: false,
                    shared: false,
                }),
                GoalLine::Variable(VariableLine {
                    name: "later".to_owned(),
                    title: Some("Later".to_owned()),
                    value: Some(Shown::Switch(false)),
                    default: None,
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Console }),
                    state: VariableState::NotApplied {
                        stored: Some(Shown::Switch(true)),
                        why: Some(Unapplied::Other("a_newer_word".to_owned())),
                    },
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    control: ValueControl::Switch { on: Some(false) },
                    stored: true,
                    shared: false,
                }),
                GoalLine::Variable(VariableLine {
                    name: "route".to_owned(),
                    title: Some("Item".to_owned()),
                    value: Some(Shown::Words("Item A".to_owned())),
                    default: None,
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::CommandLine }),
                    state: VariableState::NotOffered,
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    control: ValueControl::Choice { options: item_options(), chosen: None },
                    stored: true,
                    shared: false,
                }),
                // G3: applied in part, the keys left out are the Runtime's; the keys the next run
                // reads, without them, are what the list is edited from.
                GoalLine::Variable(VariableLine {
                    name: "order".to_owned(),
                    title: Some("Order".to_owned()),
                    value: Some(Shown::List(vec!["Item A".to_owned()])),
                    default: Some(Shown::List(vec!["Item B".to_owned(), "Item A".to_owned()])),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Mcp)) }),
                    state: VariableState::InEffect,
                    left_out: vec!["item_c".to_owned()],
                    gone: Vec::new(),
                    control: ValueControl::Ordered {
                        rule: item_rule(),
                        chosen: vec!["item_a".to_owned()],
                    },
                    stored: true,
                    shared: false,
                }),
            ]
        );
        // What can be set and cleared of each: not of a variable the root does not apply, nor of
        // one no bound package offers, whose stored value is cleared; G3: a number and a list are
        // put back to the default as a choice or a switch is.
        let variables: Vec<&VariableLine> = lines
            .iter()
            .map(|line| match line {
                GoalLine::Variable(line) => line,
                line => panic!("{line:?}"),
            })
            .collect();
        let settable: Vec<bool> = variables.iter().map(|line| line.settable()).collect();
        assert_eq!(settable, vec![true, false, true, false, true]);
        let offers: Vec<Option<ClearOffer>> =
            variables.iter().map(|line| line.clear_offer()).collect();
        assert_eq!(
            offers,
            vec![
                Some(ClearOffer::Restore),
                None,
                Some(ClearOffer::Restore),
                Some(ClearOffer::Clear),
                Some(ClearOffer::Restore),
            ]
        );
    }

    /// G1's follow-up: a value stored that is not used reads by its declaration, a number with its
    /// unit's text too, in the language shown.
    #[test]
    fn a_value_stored_and_not_used_is_shown_in_its_declarations_words() {
        let view = view(
            state("accepted"),
            json!([
                variable("points", true, integer("points", Some(json!({"en": "points", "zh-Hans": "点"}))), &["pack.a"], json!({
                    "default_json": "0",
                    "value_json": "120",
                    "effective_json": "0",
                    "source": "default",
                    "set": set("ui", None),
                    "state": "not_applied",
                    "variable_problem": "off_step",
                    "display": {"en": "0 points", "zh-Hans": "0 点"},
                })),
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_c\"",
                    "effective_json": "\"item_b\"",
                    "source": "default",
                    "state": "not_applied",
                    "variable_problem": "not_an_option",
                })),
            ]),
            json!({}),
            no_targets(),
        );
        let states = |tongue: Tongue| -> Vec<VariableState> {
            page(&view, tongue, &|_| None)
                .lines
                .into_iter()
                .map(|line| match line {
                    GoalLine::Variable(line) => line.state,
                    line => panic!("{line:?}"),
                })
                .collect()
        };
        assert_eq!(
            states(Tongue::ZhHans),
            vec![
                VariableState::NotApplied {
                    stored: Some(Shown::Words("120 点".to_owned())),
                    why: Some(Unapplied::OffStep),
                },
                // An option no longer offered has no title: its value.
                VariableState::NotApplied {
                    stored: Some(Shown::Words("item_c".to_owned())),
                    why: Some(Unapplied::NotAnOption),
                },
            ]
        );
        match &states(Tongue::En)[0] {
            VariableState::NotApplied { stored, .. } => {
                assert_eq!(stored, &Some(Shown::Words("120 points".to_owned())));
            }
            state => panic!("{state:?}"),
        }
    }

    /// G1's follow-up: a value stored under a tier's name, which this build does not use, and who
    /// set it.
    #[test]
    fn a_tier_shows_a_value_stored_under_its_name() {
        let tier = |name: &str, stored: Option<&str>| {
            let mut entry = json!({
                "name": name,
                "entry_type": "tier",
                "placed": true,
                "entry": {
                    "name": name,
                    "type": "tier",
                    "title": {"en": "Pace"},
                    "governs": {"rules": ["rule.a"]},
                    "options": [
                        {"value": "slow", "title": {"en": "Slow", "zh-Hans": "慢"}, "scale_milli": 500},
                        {"value": "fast", "title": {"en": "Fast", "zh-Hans": "快"}, "scale_milli": 2000},
                    ],
                    "default": "slow",
                },
                "default_json": "\"slow\"",
                "source": "default",
                "state": "not_built",
            });
            if let Some(stored) = stored {
                entry["value_json"] = json!(stored);
                entry["set"] = set("cli", None);
            }
            entry
        };
        let view = view(
            state("accepted"),
            json!([
                tier("pace", Some("\"fast\"")),
                tier("speed", Some("\"odd\"")),
                tier("rate", Some("3")),
                tier("calm", None),
            ]),
            json!({}),
            no_targets(),
        );
        let tiers = |tongue: Tongue| -> Vec<TierLine> {
            page(&view, tongue, &|_| None)
                .lines
                .into_iter()
                .map(|line| match line {
                    GoalLine::Tier(line) => line,
                    line => panic!("{line:?}"),
                })
                .collect()
        };
        let set_by = Some(SetBy { at_ms: SET_AT, by: Setter::CommandLine });
        let line = |stored: Option<&str>, set: Option<SetBy>| TierLine {
            title: Some("Pace".to_owned()),
            default: Some("Slow".to_owned()),
            stored: stored.map(str::to_owned),
            set,
        };
        assert_eq!(
            tiers(Tongue::En),
            vec![
                line(Some("Fast"), set_by),
                line(Some("odd"), set_by),
                line(Some("3"), set_by),
                line(None, None),
            ]
        );
        assert_eq!(tiers(Tongue::ZhHans)[0].stored.as_deref(), Some("快"));
        // A tier is never written here.
        assert_eq!(read(&view).lines.iter().filter_map(GoalLine::key).count(), 0);
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
                        {"id": "odd", "resource": "pool.gems", "condition": {"kind": "at_most", "amount": 5}},
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
                    name: "gold".to_owned(),
                    title: Some("gold".to_owned()),
                    pool: PoolState::Closed(Closed::UnmappedTask),
                    reading: None,
                    goal: Some(goal(50, Progress::NoCondition)),
                    set: None,
                    // A v2 document's target names no setter: set elsewhere, with the document's
                    // validity.
                    elsewhere: Some(Carried { validity: true, ..NOTHING }),
                    same_name: None,
                }),
                GoalLine::Group(Group::NoLongerOffered),
                GoalLine::Orphan(OrphanLine {
                    name: "old_route".to_owned(),
                    value: Shown::Words("a".to_owned()),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(None) }),
                    shared: false,
                }),
                GoalLine::Orphan(OrphanLine {
                    name: "old_rounds".to_owned(),
                    value: Shown::Number(3),
                    set: None,
                    shared: false,
                }),
                // Stored under the resource entry's name: a clear of "gold" takes that entry's
                // goal first, so this one is not cleared here.
                GoalLine::Orphan(OrphanLine {
                    name: "gold".to_owned(),
                    value: Shown::Raw("{\"at_least\":5}".to_owned()),
                    set: None,
                    shared: true,
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
                    by_name: true,
                    shared: false,
                }),
                // No `at_least` target: not read here, so its pool is said below.
                GoalLine::OtherTarget(OtherTarget {
                    id: "odd".to_owned(),
                    target: None,
                    by_name: true,
                    shared: false,
                }),
                // G0: a pool out of the instance's scope (`pool.keys`) is not listed.
                GoalLine::Group(Group::NotTargetable),
                GoalLine::Closed { pool: "pool.gems".to_owned(), why: Closed::NotObservable },
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
                    by_name: true,
                    shared: false,
                }),
                GoalLine::OtherTarget(OtherTarget {
                    id: "missing".to_owned(),
                    target: None,
                    by_name: true,
                    shared: false,
                }),
                // G1's follow-up: a goal waiting for a reading says why, so the pool's own "no
                // reading" is not said a second time.
                GoalLine::OtherTarget(OtherTarget {
                    id: "wood".to_owned(),
                    target: Some(OtherGoal {
                        pool: "pool.wood".to_owned(),
                        state: PoolState::Open,
                        reading: None,
                        goal: goal(80, Progress::Awaiting(Pending::LowConfidence)),
                    }),
                    by_name: true,
                    shared: false,
                }),
                GoalLine::OtherTarget(OtherTarget {
                    id: "half".to_owned(),
                    target: None,
                    by_name: true,
                    shared: false,
                }),
            ]
        );
    }

    /// G1's follow-up: "no reading yet" once on a resource line, by its goal while one waits for
    /// a reading; the pool's reading stays without a goal, and beside a goal that has one.
    #[test]
    fn a_goal_waiting_for_a_reading_says_so_once() {
        let view = view(
            state("accepted"),
            json!([
                resource("gold", "pool.gold", Some(("gold", 5000)), Some(set("ui", None))),
                resource("wood", "pool.wood", None, None),
                resource("gems", "pool.gems", Some(("gems", 10)), Some(set("ui", None))),
            ]),
            json!({}),
            targets(
                json!([
                    pool("pool.gold", true, true, None),
                    pool("pool.wood", true, true, None),
                    pool("pool.gems", true, true, Some(4)),
                ]),
                json!([]),
                Some(active(
                    V3,
                    json!([target("gold", "pool.gold", json!(5000)), target("gems", "pool.gems", json!(10))]),
                    json!([awaiting("gold", "pool.gold", "missing"), computed("gems", "pool.gems", 4, 6)]),
                    false,
                    None,
                )),
            ),
        );
        let readings: Vec<Option<Reading>> = read(&view)
            .lines
            .into_iter()
            .map(|line| match line {
                GoalLine::Resource(line) => {
                    // Set by this console: nothing it carries is lost.
                    assert_eq!(line.elsewhere, None, "{}", line.name);
                    line.reading
                }
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            readings,
            vec![
                None,
                Some(Reading::Pending(Some(Pending::Missing))),
                Some(Reading::Read { current: 4, at_ms: OBSERVED }),
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
                name: "scale".to_owned(),
                title: Some("scale".to_owned()),
                pool: PoolState::NoStep,
                reading: Some(Reading::Read { current: 2, at_ms: OBSERVED }),
                goal: Some(goal(40, Progress::Computed { current: 2, at_ms: OBSERVED, gap: 38 })),
                set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Authoring)) }),
                elsewhere: Some(NOTHING),
                same_name: None,
            })
        );
        // No amount is set on a pool without a step; only the open one takes one.
        let settable: Vec<bool> = lines
            .iter()
            .map(|line| match line {
                GoalLine::Resource(line) => line.settable(),
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(settable, vec![true, false, false, false]);
        assert_eq!(press(&lines[1], &GoalAct::Amount("50".to_owned())), None);
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
                switch_line("one", "One"),
                switch_line("three", "Three"),
                GoalLine::Package(Some("Pack C".to_owned())),
                switch_line("four", "Four"),
                GoalLine::Package(None),
                switch_line("two", "Two"),
                switch_line("five", "Five"),
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
                    name: "wait".to_owned(),
                    title: Some("Rounds".to_owned()),
                    value: Some(Shown::Time { amount: 90, unit: TimeUnit::Minute }),
                    default: Some(Shown::Time { amount: 1500, unit: TimeUnit::Millisecond }),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Rescue)) }),
                    state: VariableState::InEffect,
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    // G3: declared in hours without a step, it is typed in hours.
                    control: ValueControl::Number {
                        rule: rounds_rule(NumberUnit::Time {
                            declared: TimeUnit::Hour,
                            entry: TimeUnit::Hour,
                        }),
                        now: Some(5_400_000),
                    },
                    stored: true,
                    shared: false,
                }),
                GoalLine::Variable(VariableLine {
                    name: "count".to_owned(),
                    title: Some("Rounds".to_owned()),
                    value: Some(Shown::Words("3 rounds".to_owned())),
                    default: Some(Shown::Words("0 rounds".to_owned())),
                    set: Some(SetBy { at_ms: SET_AT, by: Setter::Lab(Some(LabEntry::Watchdog)) }),
                    state: VariableState::InEffect,
                    left_out: Vec::new(),
                    gone: Vec::new(),
                    control: ValueControl::Number {
                        rule: rounds_rule(NumberUnit::Text("rounds".to_owned())),
                        now: Some(3),
                    },
                    stored: true,
                    shared: false,
                }),
            ]
        );
        match &page(&view, Tongue::ZhHans, &|_| None).lines[1] {
            GoalLine::Variable(line) => {
                assert_eq!(line.value, Some(Shown::Words("3 轮".to_owned())));
                assert_eq!(line.default, Some(Shown::Words("0 轮".to_owned())));
                // A text unit in the language shown.
                assert_eq!(
                    line.control,
                    ValueControl::Number {
                        rule: rounds_rule(NumberUnit::Text("轮".to_owned())),
                        now: Some(3),
                    }
                );
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
                GoalLine::OtherTarget(OtherTarget {
                    id: "wood".to_owned(),
                    target: None,
                    by_name: true,
                    shared: false,
                }),
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
                GoalLine::Orphan(OrphanLine { value: Shown::Switch(true), set: Some(set), .. }) => {
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

    #[test]
    fn values_are_written_as_the_runtime_reads_them() {
        assert_eq!(choice_json("item_a"), "\"item_a\"");
        assert_eq!(switch_json(true), "true");
        assert_eq!(switch_json(false), "false");
        assert_eq!(resource_json(5000), "{\"at_least\":5000}");
        let back: Value = serde_json::from_str(&resource_json(5000)).expect("JSON");
        assert_eq!(back, json!({"at_least": 5000}));
        let back: Value = serde_json::from_str(&choice_json("item_a")).expect("JSON");
        assert_eq!(back, json!("item_a"));
        // One EditGoal of one name, as the contract spells it.
        let pick = GoalWrite { name: "pick".to_owned(), change: GoalChange::Set(choice_json("item_b")) };
        assert_eq!(
            serde_json::to_value(pick.edit()).expect("an edit"),
            json!({"kind": "set", "name": "pick", "value_json": "\"item_b\""})
        );
        let gone = GoalWrite { name: "gold".to_owned(), change: GoalChange::Clear };
        assert_eq!(
            serde_json::to_value(gone.edit()).expect("an edit"),
            json!({"kind": "clear", "name": "gold"})
        );
        assert_eq!(pick.line(), "set pick to \"item_b\"");
        assert_eq!(gone.line(), "clear gold");
    }

    /// What a write sets reads back on the next page as what was set.
    #[test]
    fn what_is_written_reads_back_on_the_page() {
        let view = view(
            state("accepted"),
            json!([
                {
                    "name": "gold",
                    "entry_type": "resource",
                    "placed": true,
                    "entry": {"name": "gold", "type": "resource", "title": {"en": "gold"}, "governs": {"pool": "pool.gold"}},
                    "value_json": resource_json(300),
                    "effective_json": resource_json(300),
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                    "resource": {"pool": "pool.gold", "target_id": "gold"},
                },
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": choice_json("item_b"),
                    "value_json": choice_json("item_a"),
                    "effective_json": choice_json("item_a"),
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
                variable("extra", true, switch("extra", "Extra"), &["pack.a"], json!({
                    "default_json": switch_json(false),
                    "value_json": switch_json(true),
                    "effective_json": switch_json(true),
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
            ]),
            json!({}),
            targets(
                json!([pool("pool.gold", true, true, Some(1))]),
                json!([]),
                Some(active(
                    V3,
                    json!([target("gold", "pool.gold", json!(300))]),
                    json!([computed("gold", "pool.gold", 1, 299)]),
                    false,
                    None,
                )),
            ),
        );
        let lines = read(&view).lines;
        match &lines[0] {
            GoalLine::Resource(line) => {
                assert_eq!(line.goal.as_ref().and_then(|goal| goal.amount), Some(300));
                assert_eq!(line.elsewhere, None);
            }
            line => panic!("{line:?}"),
        }
        match &lines[1] {
            GoalLine::Variable(line) => assert_eq!(
                line.control,
                ValueControl::Choice { options: item_options(), chosen: Some("item_a".to_owned()) }
            ),
            line => panic!("{line:?}"),
        }
        match &lines[2] {
            GoalLine::Variable(line) => {
                assert_eq!(line.control, ValueControl::Switch { on: Some(true) });
            }
            line => panic!("{line:?}"),
        }
    }

    #[test]
    fn an_amount_is_a_whole_number_of_at_least_one() {
        for (text, expected) in [
            ("5000", Ok::<u64, AmountProblem>(5000)),
            (" 12 ", Ok(12)),
            ("+7", Ok(7)),
            ("1", Ok(1)),
            ("", Err(AmountProblem::Empty)),
            ("   ", Err(AmountProblem::Empty)),
            ("0", Err(AmountProblem::BelowOne)),
            ("-3", Err(AmountProblem::BelowOne)),
            ("-0", Err(AmountProblem::BelowOne)),
            ("1.5", Err(AmountProblem::NotWhole)),
            ("abc", Err(AmountProblem::NotWhole)),
            ("1e3", Err(AmountProblem::NotWhole)),
            ("１２", Err(AmountProblem::NotWhole)),
            ("1 000", Err(AmountProblem::NotWhole)),
            ("-", Err(AmountProblem::NotWhole)),
            ("+", Err(AmountProblem::NotWhole)),
            ("99999999999999999999999", Err(AmountProblem::TooLarge)),
        ] {
            assert_eq!(amount(text), expected, "{text:?}");
        }
    }

    /// A page to press on: resource entries with a goal this console set, one set elsewhere that
    /// carries tasks and a step, one without a goal, one whose target does not read; a choice set
    /// here, one at its default, a switch, an `integer` set, a placement not offered with a value;
    /// a value no longer offered; other targets by a name, by an id that is no name, not read.
    fn pressed_page() -> GoalsPage {
        let view = view(
            state("accepted"),
            json!([
                resource("gold", "pool.gold", Some(("gold", 50)), Some(set("ui", None))),
                resource("wood", "pool.wood", Some(("agent_wood", 80)), Some(set("mcp", None))),
                resource("gems", "pool.gems", None, None),
                resource("keys", "pool.keys", Some(("lost", 5)), None),
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_a\"",
                    "effective_json": "\"item_a\"",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
                variable("other", true, choice("other"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "effective_json": "\"item_b\"",
                    "source": "default",
                    "state": "in_effect",
                })),
                variable("extra", true, switch("extra", "Extra"), &["pack.a"], json!({
                    "default_json": "false",
                    "effective_json": "false",
                    "source": "default",
                    "state": "in_effect",
                })),
                variable("rounds", true, integer("rounds", None), &["pack.a"], json!({
                    "default_json": "0",
                    "value_json": "3",
                    "effective_json": "3",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
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
            ]),
            json!({
                "orphans": [{"name": "old_route", "value_json": "\"a\""}],
                "other_targets": ["stone", "energy-floor", "lost_target"],
            }),
            targets(
                json!([
                    pool("pool.gold", true, true, Some(1)),
                    pool("pool.wood", true, true, Some(2)),
                    pool("pool.gems", true, true, Some(3)),
                    pool("pool.keys", true, true, Some(4)),
                    pool("pool.stone", true, true, Some(5)),
                ]),
                json!([]),
                Some(active(
                    V3,
                    json!([
                        target("gold", "pool.gold", json!(50)),
                        {
                            "id": "agent_wood",
                            "resource": "pool.wood",
                            "condition": {"kind": "at_least", "amount": 80},
                            "scale": 10,
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "tasks": ["task.daily"],
                        },
                        target("stone", "pool.stone", json!(20)),
                        target("energy-floor", "pool.iron", json!(7)),
                    ]),
                    json!([]),
                    false,
                    None,
                )),
            ),
        );
        read(&view)
    }

    #[test]
    fn a_press_writes_one_name_and_asks_only_for_what_cannot_be_set_back() {
        let page = pressed_page();
        assert!(page.writable());
        let on = |key: &str, act: GoalAct| -> Option<Press> {
            let line = page.line(key).unwrap_or_else(|| panic!("no line {key}"));
            press(line, &act)
        };
        let set_of = |name: &str, json: &str| GoalWrite {
            name: name.to_owned(),
            change: GoalChange::Set(json.to_owned()),
        };
        let clear_of = |name: &str| GoalWrite { name: name.to_owned(), change: GoalChange::Clear };
        let typed = |text: &str| GoalAct::Amount(text.to_owned());
        let goes = |write: GoalWrite| Some(Press { write, asks: None });
        let asks = |write: GoalWrite, loses: Loses| Some(Press { write, asks: Some(loses) });

        // A goal this console set: replaced and cleared without a question.
        assert_eq!(on("resource:gold", typed("300")), goes(set_of("gold", "{\"at_least\":300}")));
        assert_eq!(on("resource:gold", typed(" 7 ")), goes(set_of("gold", "{\"at_least\":7}")));
        assert_eq!(on("resource:gold", typed("0")), None);
        assert_eq!(on("resource:gold", typed("")), None);
        assert_eq!(on("resource:gold", GoalAct::Clear), goes(clear_of("gold")));
        assert_eq!(on("resource:gold", GoalAct::Switch(true)), None);
        // One set elsewhere, carrying tasks and a step: asked, saying so.
        let carried = Carried { tasks: true, step: true, ..NOTHING };
        assert_eq!(
            on("resource:wood", typed("90")),
            asks(set_of("wood", "{\"at_least\":90}"), Loses::Elsewhere { cleared: false, carried })
        );
        assert_eq!(
            on("resource:wood", GoalAct::Clear),
            asks(clear_of("wood"), Loses::Elsewhere { cleared: true, carried })
        );
        // No goal: set without a question, nothing to clear.
        assert_eq!(on("resource:gems", typed("5")), goes(set_of("gems", "{\"at_least\":5}")));
        assert_eq!(on("resource:gems", GoalAct::Clear), None);
        // Set elsewhere, its target not read: all four are said.
        assert_eq!(
            on("resource:keys", typed("5")),
            asks(
                set_of("keys", "{\"at_least\":5}"),
                Loses::Elsewhere { cleared: false, carried: Carried::ALL }
            )
        );
        // A choice: another option is set, the one chosen is nothing; back to the default asks
        // nothing.
        assert_eq!(on("variable:pick", GoalAct::Pick(1)), goes(set_of("pick", "\"item_b\"")));
        assert_eq!(on("variable:pick", GoalAct::Pick(0)), None);
        assert_eq!(on("variable:pick", GoalAct::Pick(2)), None);
        assert_eq!(on("variable:pick", GoalAct::Switch(true)), None);
        assert_eq!(on("variable:pick", GoalAct::Clear), goes(clear_of("pick")));
        // At its default: nothing stored to clear.
        assert_eq!(on("variable:other", GoalAct::Pick(0)), goes(set_of("other", "\"item_a\"")));
        assert_eq!(on("variable:other", GoalAct::Clear), None);
        // A switch.
        assert_eq!(on("variable:extra", GoalAct::Switch(true)), goes(set_of("extra", "true")));
        assert_eq!(on("variable:extra", GoalAct::Switch(false)), None);
        assert_eq!(on("variable:extra", GoalAct::Pick(0)), None);
        // G3: an `integer` in effect: another number set, and back to the default, at once; the
        // number in effect, or none, writes nothing.
        assert_eq!(on("variable:rounds", typed("5")), goes(set_of("rounds", "5")));
        assert_eq!(on("variable:rounds", typed("3")), None);
        assert_eq!(on("variable:rounds", typed("x")), None);
        assert_eq!(on("variable:rounds", GoalAct::Clear), goes(clear_of("rounds")));
        // Not offered: nothing set; its stored value cleared, asked.
        assert_eq!(on("variable:route", GoalAct::Pick(1)), None);
        assert_eq!(
            on("variable:route", GoalAct::Clear),
            asks(clear_of("route"), Loses::Value(Shown::Words("Item A".to_owned())))
        );
        // No longer offered: cleared, asked.
        assert_eq!(
            on("orphan:old_route", GoalAct::Clear),
            asks(clear_of("old_route"), Loses::Value(Shown::Words("a".to_owned())))
        );
        assert_eq!(on("orphan:old_route", typed("5")), None);
        // Other targets: by a name, asked with what it holds; by an id that is no name, only the
        // Lab's; not read, asked without.
        assert_eq!(
            on("target:stone", GoalAct::Clear),
            asks(clear_of("stone"), Loses::Target(Some(("pool.stone".to_owned(), 20))))
        );
        assert_eq!(on("target:energy-floor", GoalAct::Clear), None);
        assert_eq!(
            on("target:lost_target", GoalAct::Clear),
            asks(clear_of("lost_target"), Loses::Target(None))
        );
        let by_name: Vec<bool> = page
            .lines
            .iter()
            .filter_map(|line| match line {
                GoalLine::OtherTarget(other) => Some(other.by_name),
                _ => None,
            })
            .collect();
        assert_eq!(by_name, vec![true, false, true]);
    }

    /// fix1: the Runtime resolves a name by what is stored under it (a clear takes a resource
    /// entry's goal on its pool first, then a stored value, then a target of that id), and a
    /// resource entry's set removes the targets of its name on other pools. So a value no longer
    /// offered, an other target, or (fix4) a variable's value, whose name another line has (an
    /// entry of any type, a value no longer offered, an other target) is not cleared here; an
    /// amount set on an entry whose name an other target, or another entry's goal, has asks first,
    /// with what that goal holds, and with what the entry's own goal set elsewhere carries.
    #[test]
    fn a_name_two_lines_share_is_not_cleared_by_it() {
        let view = view(
            state("accepted"),
            json!([
                note("about", json!({"en": "Read me"}), json!({"en": "More words"})),
                resource("gold", "pool.gold", Some(("gold_main", 50)), Some(set("ui", None))),
                resource("wood", "pool.wood", Some(("agent_wood", 80)), Some(set("mcp", None))),
                resource("gems", "pool.gems", None, None),
                // Another resource entry whose goal's id is the entry name `gems`.
                resource("silver", "pool.silver", Some(("gems", 40)), Some(set("mcp", None))),
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_a\"",
                    "effective_json": "\"item_a\"",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
            ]),
            json!({
                "orphans": [
                    {"name": "gems", "value_json": "{\"at_least\":5}"},
                    {"name": "about", "value_json": "true"},
                    {"name": "stone", "value_json": "3"},
                    {"name": "old_route", "value_json": "\"a\""},
                ],
                "other_targets": ["gold", "wood", "pick", "stone", "iron"],
            }),
            targets(
                json!([
                    pool("pool.gold", true, true, Some(1)),
                    pool("pool.wood", true, true, Some(2)),
                    pool("pool.gems", true, true, Some(3)),
                    pool("pool.old_gold", true, true, Some(4)),
                    pool("pool.silver", true, true, Some(5)),
                ]),
                json!([]),
                Some(active(
                    V3,
                    json!([
                        target("gold_main", "pool.gold", json!(50)),
                        {
                            "id": "agent_wood",
                            "resource": "pool.wood",
                            "condition": {"kind": "at_least", "amount": 80},
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "tasks": ["task.daily"],
                        },
                        target("gold", "pool.old_gold", json!(30)),
                        target("wood", "pool.old_wood", json!(20)),
                        target("pick", "pool.iron", json!(9)),
                        target("stone", "pool.stone", json!(8)),
                        target("iron", "pool.ore", json!(7)),
                        target("gems", "pool.silver", json!(40)),
                    ]),
                    json!([]),
                    false,
                    None,
                )),
            ),
        );
        let page = read(&view);
        let on = |key: &str, act: GoalAct| -> Option<Press> {
            let line = page.line(key).unwrap_or_else(|| panic!("no line {key}"));
            press(line, &act)
        };
        let set_of = |name: &str, json: &str| GoalWrite {
            name: name.to_owned(),
            change: GoalChange::Set(json.to_owned()),
        };
        let clear_of = |name: &str| GoalWrite { name: name.to_owned(), change: GoalChange::Clear };
        let typed = |text: &str| GoalAct::Amount(text.to_owned());
        let goes = |write: GoalWrite| Some(Press { write, asks: None });
        let asks = |write: GoalWrite, loses: Loses| Some(Press { write, asks: Some(loses) });

        // Which lines' names another line has.
        let orphans: Vec<bool> = page
            .lines
            .iter()
            .filter_map(|line| match line {
                GoalLine::Orphan(orphan) => Some(orphan.shared),
                _ => None,
            })
            .collect();
        assert_eq!(orphans, vec![true, true, true, false]);
        let others: Vec<(bool, bool)> = page
            .lines
            .iter()
            .filter_map(|line| match line {
                GoalLine::OtherTarget(other) => Some((other.shared, other.clearable())),
                _ => None,
            })
            .collect();
        assert_eq!(
            others,
            vec![(true, false), (true, false), (true, false), (true, false), (false, true)]
        );

        // 1. A value no longer offered under a resource entry's name, a note's, an other target's:
        // not cleared here. One no other line has is, asked.
        assert_eq!(on("orphan:gems", GoalAct::Clear), None);
        assert_eq!(on("orphan:about", GoalAct::Clear), None);
        assert_eq!(on("orphan:stone", GoalAct::Clear), None);
        assert_eq!(
            on("orphan:old_route", GoalAct::Clear),
            asks(clear_of("old_route"), Loses::Value(Shown::Words("a".to_owned())))
        );
        // 2. An other target of a resource entry's name, of a variable's with a value stored, of a
        // value no longer offered's: not cleared here. One no other line has is, asked.
        assert_eq!(on("target:gold", GoalAct::Clear), None);
        assert_eq!(on("target:wood", GoalAct::Clear), None);
        assert_eq!(on("target:pick", GoalAct::Clear), None);
        assert_eq!(on("target:stone", GoalAct::Clear), None);
        assert_eq!(
            on("target:iron", GoalAct::Clear),
            asks(clear_of("iron"), Loses::Target(Some(("pool.ore".to_owned(), 7))))
        );
        // A resource entry's own clear is not in doubt: the Runtime takes its pool's goal first.
        assert_eq!(on("resource:gold", GoalAct::Clear), goes(clear_of("gold")));
        // fix4: a variable's value whose name another line has (here an other target) is not put
        // back to the default here either.
        match page.line("variable:pick") {
            Some(GoalLine::Variable(line)) => {
                assert!(line.shared);
                assert_eq!(line.clear_offer(), Some(ClearOffer::Restore));
                assert_eq!(line.clearable(), None);
            }
            line => panic!("{line:?}"),
        }
        assert_eq!(on("variable:pick", GoalAct::Clear), None);
        // Its value is still set: a set names the variable before any target.
        assert_eq!(on("variable:pick", GoalAct::Pick(1)), goes(set_of("pick", "\"item_b\"")));

        // 3. An amount set on an entry whose name an other target has removes that target too:
        // asked, with what it holds; with what its own goal set elsewhere carries as well.
        match page.line("resource:gold") {
            Some(GoalLine::Resource(line)) => assert_eq!(
                line.same_name,
                Some(Removed::Other(Some(("pool.old_gold".to_owned(), 30))))
            ),
            line => panic!("{line:?}"),
        }
        assert_eq!(
            on("resource:gold", typed("60")),
            asks(
                set_of("gold", "{\"at_least\":60}"),
                Loses::SameName {
                    removed: Removed::Other(Some(("pool.old_gold".to_owned(), 30))),
                    carried: None,
                }
            )
        );
        let carried = Carried { tasks: true, ..NOTHING };
        assert_eq!(
            on("resource:wood", typed("90")),
            asks(
                set_of("wood", "{\"at_least\":90}"),
                Loses::SameName {
                    removed: Removed::Other(Some(("pool.old_wood".to_owned(), 20))),
                    carried: Some(carried),
                }
            )
        );
        // Its clear takes only its own pool's goal.
        assert_eq!(
            on("resource:wood", GoalAct::Clear),
            asks(clear_of("wood"), Loses::Elsewhere { cleared: true, carried })
        );
        // No other target of its name, but another resource entry's goal has its name as its id:
        // that goal goes too, so asked, naming that entry by its title, with what its goal holds
        // (fix4: a goal by name, it carries none of the four).
        assert_eq!(
            on("resource:gems", typed("5")),
            asks(
                set_of("gems", "{\"at_least\":5}"),
                Loses::SameName {
                    removed: Removed::Entry {
                        title: Some("silver".to_owned()),
                        amount: Some(40),
                        carried: NOTHING,
                    },
                    carried: None,
                }
            )
        );
        // No target of its name anywhere else: set at once.
        assert_eq!(on("resource:silver", typed("45")).map(|press| press.asks.is_some()), Some(false));
    }

    /// fix1: a variable the game root does not apply takes no value set here
    /// (`variable_not_applying`), so a value stored for it is lost for good once put back to the
    /// default: that asks first, with the value stored, in the language the page is shown in. So
    /// does any value stored and not used (fix4); one whose value is in effect asks nothing; one
    /// with no value stored has nothing to clear.
    #[test]
    fn back_to_the_default_asks_first_where_the_root_does_not_apply_the_variable() {
        let view = view(
            state("accepted"),
            json!([
                variable("extra", true, switch("extra", "Extra"), &["pack.a"], json!({
                    "default_json": "false",
                    "value_json": "true",
                    "effective_json": "false",
                    "source": "default",
                    "set": set("ui", None),
                    "state": "not_applied",
                    "variable_problem": "irreversible_by_root",
                })),
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_a\"",
                    "effective_json": "\"item_b\"",
                    "source": "default",
                    "set": set("cli", None),
                    "state": "not_applied",
                    "variable_problem": "package_unreadable",
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
                variable("calm", true, switch("calm", "Calm"), &["pack.a"], json!({
                    "default_json": "false",
                    "effective_json": "false",
                    "source": "default",
                    "state": "not_applied",
                    "variable_problem": "root_unchecked",
                })),
            ]),
            json!({}),
            no_targets(),
        );
        let clear_of = |name: &str| GoalWrite { name: name.to_owned(), change: GoalChange::Clear };
        let asks = |name: &str, value: Shown| {
            Some(Press { write: clear_of(name), asks: Some(Loses::Value(value)) })
        };
        let english = read(&view);
        let on = |page: &GoalsPage, key: &str, act: GoalAct| -> Option<Press> {
            let line = page.line(key).unwrap_or_else(|| panic!("no line {key}"));
            press(line, &act)
        };
        // Offered as "back to the default", as any variable with a control and a value stored.
        let offers: Vec<Option<ClearOffer>> = english
            .lines
            .iter()
            .map(|line| match line {
                GoalLine::Variable(line) => line.clear_offer(),
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            offers,
            vec![Some(ClearOffer::Restore), Some(ClearOffer::Restore), Some(ClearOffer::Restore), None]
        );
        assert_eq!(on(&english, "variable:extra", GoalAct::Clear), asks("extra", Shown::Switch(true)));
        assert_eq!(
            on(&english, "variable:pick", GoalAct::Clear),
            asks("pick", Shown::Words("Item A".to_owned()))
        );
        // Nothing is set on either.
        assert_eq!(on(&english, "variable:extra", GoalAct::Switch(true)), None);
        assert_eq!(on(&english, "variable:pick", GoalAct::Pick(0)), None);
        // A reason word newer than this console: whether a value could be set on it again is not
        // known, so it asks too, with the value stored.
        assert_eq!(on(&english, "variable:later", GoalAct::Clear), asks("later", Shown::Switch(true)));
        // Nothing stored: nothing to clear.
        assert_eq!(on(&english, "variable:calm", GoalAct::Clear), None);
        // The question's value is in the page's language: the page a press reads is made in the
        // language shown.
        let chinese = page(&view, Tongue::ZhHans, &|_| None);
        assert_eq!(
            on(&chinese, "variable:pick", GoalAct::Clear),
            asks("pick", Shown::Words("甲".to_owned()))
        );
    }

    /// The writes and questions a page's presses come to, by line key.
    struct Presses<'a>(&'a GoalsPage);

    impl Presses<'_> {
        fn on(&self, key: &str, act: GoalAct) -> Option<Press> {
            let line = self.0.line(key).unwrap_or_else(|| panic!("no line {key}"));
            press(line, &act)
        }
    }

    fn set_of(name: &str, json: &str) -> GoalWrite {
        GoalWrite { name: name.to_owned(), change: GoalChange::Set(json.to_owned()) }
    }

    fn clear_of(name: &str) -> GoalWrite {
        GoalWrite { name: name.to_owned(), change: GoalChange::Clear }
    }

    fn goes(write: GoalWrite) -> Option<Press> {
        Some(Press { write, asks: None })
    }

    fn asks(write: GoalWrite, loses: Loses) -> Option<Press> {
        Some(Press { write, asks: Some(loses) })
    }

    fn typed(text: &str) -> GoalAct {
        GoalAct::Amount(text.to_owned())
    }

    /// fix4 (1): a value stored that the next run does not take, for any reason, is set back from
    /// here by no control (an option no longer offered, a value of another type, a reason word
    /// this console does not know): another option picked, the switch turned, or back to the
    /// default, each asks first, saying the value stored. A value in effect asks nothing either way.
    #[test]
    fn a_value_the_next_run_does_not_take_is_asked_about_before_it_goes() {
        let view = view(
            state("accepted"),
            json!([
                variable("pick", true, choice("pick"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_c\"",
                    "effective_json": "\"item_b\"",
                    "source": "default",
                    "set": set("cli", None),
                    "state": "not_applied",
                    "variable_problem": "not_an_option",
                })),
                variable("extra", true, switch("extra", "Extra"), &["pack.a"], json!({
                    "default_json": "false",
                    "value_json": "\"yes\"",
                    "effective_json": "false",
                    "source": "default",
                    "set": set("mcp", None),
                    "state": "not_applied",
                    "variable_problem": "type_changed",
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
                variable("route", true, choice("route"), &["pack.a"], json!({
                    "default_json": "\"item_b\"",
                    "value_json": "\"item_a\"",
                    "effective_json": "\"item_a\"",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
            ]),
            json!({}),
            no_targets(),
        );
        let page = read(&view);
        let presses = Presses(&page);
        // An option no longer offered: another option, or the default again, loses it.
        let item_c = Shown::Words("item_c".to_owned());
        assert_eq!(
            presses.on("variable:pick", GoalAct::Pick(0)),
            asks(set_of("pick", "\"item_a\""), Loses::Replaced(item_c.clone()))
        );
        assert_eq!(
            presses.on("variable:pick", GoalAct::Clear),
            asks(clear_of("pick"), Loses::Value(item_c))
        );
        // A value of another type: the switch turned, or the default again.
        let yes = Shown::Raw("\"yes\"".to_owned());
        assert_eq!(
            presses.on("variable:extra", GoalAct::Switch(true)),
            asks(set_of("extra", "true"), Loses::Replaced(yes.clone()))
        );
        assert_eq!(
            presses.on("variable:extra", GoalAct::Clear),
            asks(clear_of("extra"), Loses::Value(yes))
        );
        // A reason word this console does not know: the same.
        assert_eq!(
            presses.on("variable:later", GoalAct::Switch(true)),
            asks(set_of("later", "true"), Loses::Replaced(Shown::Switch(true)))
        );
        // In effect: another option and the default again, at once.
        assert_eq!(presses.on("variable:route", GoalAct::Pick(1)), goes(set_of("route", "\"item_b\"")));
        assert_eq!(presses.on("variable:route", GoalAct::Clear), goes(clear_of("route")));
    }

    /// fix4 (2): on a pool that takes no goal here now (without a step, not targetable, not
    /// listed) "Set" sends nothing, and a goal on it, whoever set it, is not set back from here
    /// once cleared: its clear asks first, with its amount, and with what it carries when it was
    /// set elsewhere and carries any. On a pool that takes a goal, this console's goal goes at once.
    #[test]
    fn a_goal_on_a_pool_that_takes_none_here_asks_before_it_is_cleared() {
        let view = view(
            state("accepted"),
            json!([
                resource("scale", "pool.scale", Some(("scale", 40)), Some(set("ui", None))),
                resource("gems", "pool.gems", Some(("gems", 10)), Some(set("mcp", None))),
                resource("keys", "pool.keys", Some(("keys", 5)), Some(set("lab", Some("Lab-CLI")))),
                resource("gold", "pool.gold", Some(("gold", 50)), Some(set("ui", None))),
            ]),
            json!({}),
            targets(
                json!([pool("pool.scale", true, false, Some(2)), pool("pool.gold", true, true, Some(1))]),
                json!([closed_pool("pool.gems", "unmapped_task")]),
                Some(active(
                    V3,
                    json!([
                        target("scale", "pool.scale", json!(40)),
                        {
                            "id": "gems",
                            "resource": "pool.gems",
                            "condition": {"kind": "at_least", "amount": 10},
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "tasks": ["task.daily"],
                        },
                        target("keys", "pool.keys", json!(5)),
                        target("gold", "pool.gold", json!(50)),
                    ]),
                    json!([]),
                    false,
                    None,
                )),
            ),
        );
        let page = read(&view);
        let pools: Vec<PoolState> = page
            .lines
            .iter()
            .map(|line| match line {
                GoalLine::Resource(line) => line.pool.clone(),
                line => panic!("{line:?}"),
            })
            .collect();
        assert_eq!(
            pools,
            vec![
                PoolState::NoStep,
                PoolState::Closed(Closed::UnmappedTask),
                PoolState::Unlisted,
                PoolState::Open,
            ]
        );
        let presses = Presses(&page);
        for key in ["resource:scale", "resource:gems", "resource:keys"] {
            assert_eq!(presses.on(key, typed("60")), None, "{key}");
        }
        assert_eq!(presses.on("resource:gold", typed("60")), goes(set_of("gold", "{\"at_least\":60}")));
        // This console's goal on a pool without a step: asked, with its amount.
        assert_eq!(
            presses.on("resource:scale", GoalAct::Clear),
            asks(clear_of("scale"), Loses::Unsettable { amount: Some(40), carried: None })
        );
        // Set elsewhere with tasks of its own, on a pool not targetable: with those too.
        assert_eq!(
            presses.on("resource:gems", GoalAct::Clear),
            asks(
                clear_of("gems"),
                Loses::Unsettable {
                    amount: Some(10),
                    carried: Some(Carried { tasks: true, ..NOTHING }),
                }
            )
        );
        // Set elsewhere by name, carrying none of the four, on a pool not listed: asked all the
        // same, which it was not while its pool took a goal.
        assert_eq!(
            presses.on("resource:keys", GoalAct::Clear),
            asks(clear_of("keys"), Loses::Unsettable { amount: Some(5), carried: None })
        );
        assert_eq!(presses.on("resource:gold", GoalAct::Clear), goes(clear_of("gold")));
    }

    /// fix4 (3): a resource entry and a variable of one name, both listed when the game root names
    /// them both (`goal_name_conflict`), are two lines of two keys. The variable's value is not put
    /// back to the default here, since a clear of the name takes the resource's goal first; the
    /// resource's own clear is not in doubt. A variable no other line names is cleared at once.
    #[test]
    fn a_resource_and_a_variable_of_one_name_are_two_lines() {
        let view = view(
            state("accepted"),
            json!([
                resource("gold", "pool.gold", Some(("gold", 50)), Some(set("ui", None))),
                variable("gold", false, switch("gold", "Gold"), &["pack.a"], json!({
                    "default_json": "false",
                    "value_json": "true",
                    "effective_json": "false",
                    "source": "default",
                    "set": set("cli", None),
                    "state": "not_applied",
                    "variable_problem": "goal_name_conflict",
                })),
                variable("extra", false, switch("extra", "Extra"), &["pack.a"], json!({
                    "default_json": "false",
                    "value_json": "true",
                    "effective_json": "true",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
            ]),
            json!({}),
            targets(
                json!([pool("pool.gold", true, true, Some(1))]),
                json!([]),
                Some(active(
                    V3,
                    json!([target("gold", "pool.gold", json!(50))]),
                    json!([]),
                    false,
                    None,
                )),
            ),
        );
        let page = read(&view);
        let keys: Vec<String> = page.lines.iter().filter_map(GoalLine::key).collect();
        assert_eq!(keys, vec!["resource:gold", "variable:gold", "variable:extra"]);
        assert!(matches!(page.line("resource:gold"), Some(GoalLine::Resource(_))));
        match page.line("variable:gold") {
            Some(GoalLine::Variable(line)) => {
                assert!(line.shared);
                assert_eq!(line.clear_offer(), Some(ClearOffer::Restore));
                assert_eq!(line.clearable(), None);
            }
            line => panic!("{line:?}"),
        }
        let presses = Presses(&page);
        assert_eq!(presses.on("variable:gold", GoalAct::Clear), None);
        // Nor is a value set on it: the game root does not apply it.
        assert_eq!(presses.on("variable:gold", GoalAct::Switch(true)), None);
        assert_eq!(presses.on("resource:gold", GoalAct::Clear), goes(clear_of("gold")));
        assert_eq!(presses.on("variable:extra", GoalAct::Clear), goes(clear_of("extra")));
    }

    /// fix4 (4): an amount that removes another resource entry's goal of the entry's name says
    /// that entry by its title, in the language shown, never by its pool, with what its goal
    /// holds: its amount, and what it carries, read from the stored target of that id as a goal
    /// set elsewhere's is. And a goal set elsewhere that carries an importance only is said to
    /// lose only that.
    #[test]
    fn another_entrys_goal_removed_is_said_by_its_title_and_what_it_carries() {
        let mut silver =
            resource("silver", "pool.silver", Some(("gems", 40)), Some(set("mcp", None)));
        silver["entry"]["title"] = json!({"en": "Silver", "zh-Hans": "银币"});
        let view = view(
            state("accepted"),
            json!([
                resource("gems", "pool.gems", None, None),
                silver,
                resource("wood", "pool.wood", Some(("agent_wood", 80)), Some(set("lab", None))),
            ]),
            json!({}),
            targets(
                json!([
                    pool("pool.gems", true, true, Some(1)),
                    pool("pool.silver", true, true, Some(2)),
                    pool("pool.wood", true, true, Some(3)),
                ]),
                json!([]),
                Some(active(
                    V3,
                    json!([
                        {
                            "id": "gems",
                            "resource": "pool.silver",
                            "condition": {"kind": "at_least", "amount": 40},
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "importance_milli": 700,
                            "valid_until_unix_ms": VALID_UNTIL,
                        },
                        {
                            "id": "agent_wood",
                            "resource": "pool.wood",
                            "condition": {"kind": "at_least", "amount": 80},
                            "apply": {"mode": "adjust", "weight": "score_stage"},
                            "importance_milli": 300,
                        },
                    ]),
                    json!([]),
                    false,
                    None,
                )),
            ),
        );
        let removed = |title: &str| Removed::Entry {
            title: Some(title.to_owned()),
            amount: Some(40),
            carried: Carried { importance: true, validity: true, ..NOTHING },
        };
        for (tongue, title) in [(Tongue::En, "Silver"), (Tongue::ZhHans, "银币")] {
            let shown = page(&view, tongue, &|_| None);
            match shown.line("resource:gems") {
                Some(GoalLine::Resource(line)) => assert_eq!(line.same_name, Some(removed(title))),
                line => panic!("{line:?}"),
            }
            assert_eq!(
                Presses(&shown).on("resource:gems", typed("5")),
                asks(
                    set_of("gems", "{\"at_least\":5}"),
                    Loses::SameName { removed: removed(title), carried: None }
                )
            );
        }
        // An importance of its own only: that alone is said, replaced or cleared.
        let page = read(&view);
        let presses = Presses(&page);
        let importance = Carried { importance: true, ..NOTHING };
        assert_eq!(
            presses.on("resource:wood", typed("90")),
            asks(
                set_of("wood", "{\"at_least\":90}"),
                Loses::Elsewhere { cleared: false, carried: importance }
            )
        );
        assert_eq!(
            presses.on("resource:wood", GoalAct::Clear),
            asks(clear_of("wood"), Loses::Elsewhere { cleared: true, carried: importance })
        );
    }

    /// fix4 (5): against a record of the old form every write is refused `record_unreadable`, so a
    /// page over one takes no press at all, though it lists; over the other forms, or no record, it
    /// does.
    #[test]
    fn a_page_over_an_old_record_takes_no_press() {
        let of = |active: Option<Value>| {
            read(&view(
                state("accepted"),
                json!([resource("gold", "pool.gold", Some(("gold", 5)), None)]),
                json!({}),
                targets(json!([pool("pool.gold", true, true, Some(1))]), json!([]), active),
            ))
        };
        let document = |schema: &str| {
            Some(active(
                schema,
                json!([target("gold", "pool.gold", json!(5))]),
                json!([]),
                false,
                Some(VALID_UNTIL),
            ))
        };
        let old = of(document(V1));
        assert_eq!(
            old.state,
            PageState::Listed {
                declaration: Declaration::Accepted,
                nothing: false,
                none_offered: false,
            }
        );
        assert!(matches!(&old.stored, Stored::Document(Document { form: Form::Old, .. })));
        assert!(!old.writable());
        assert!(of(document(V2)).writable());
        assert!(of(document(V3)).writable());
        assert!(of(None).writable());
    }

    fn values(given: Value) -> Map<String, Value> {
        match given {
            Value::Object(values) => values,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_refusal_is_told_apart_by_its_code_and_values() {
        let conflict = values(json!({"fact_key": "session.resource_targets", "current_version": 12}));
        assert_eq!(
            answered("fact_observation_version_conflict", &conflict),
            Answered::Conflict { current: Some(12) }
        );
        assert_eq!(
            answered("fact_observation_version_conflict", &Map::new()),
            Answered::Conflict { current: None }
        );
        assert_eq!(answered("not_implemented", &Map::new()), Answered::NotImplemented);
        assert_eq!(answered("fact_observation_not_newer", &Map::new()), Answered::NotNewer);
        for code in [
            "instance_unknown",
            "channel_binding_missing",
            "resource_target_view_instance_unconfigured",
            "resource_target_view_policy_unreadable",
            "",
        ] {
            assert_eq!(answered(code, &Map::new()), Answered::Other, "{code}");
        }
        let invalid = |given: Value| match answered("goal_value_invalid", &values(given)) {
            Answered::Invalid(invalid) => invalid,
            other => panic!("{other:?}"),
        };
        let bare = |problem: WriteProblem| WriteInvalid {
            problem,
            min: None,
            max: None,
            pointer: None,
            pool: None,
            variable: None,
        };
        // A resource's amount above its pool's capacity: the range, the member and the pool.
        assert_eq!(
            invalid(json!({
                "goal_problem": "out_of_range",
                "goal_name": "gold",
                "json_pointer": "/at_least",
                "range_min": 1,
                "range_max": 9999,
                "pool_id": "pool.gold",
            })),
            WriteInvalid {
                min: Some(1),
                max: Some(9999),
                pointer: Some("/at_least".to_owned()),
                pool: Some("pool.gold".to_owned()),
                ..bare(WriteProblem::OutOfRange)
            }
        );
        assert_eq!(
            invalid(json!({"goal_problem": "value_too_large", "range_min": 0, "range_max": 1024})),
            WriteInvalid { min: Some(0), max: Some(1024), ..bare(WriteProblem::ValueTooLarge) }
        );
        assert_eq!(
            invalid(json!({"goal_problem": "count_out_of_range", "range_min": 1, "range_max": 3})),
            WriteInvalid { min: Some(1), max: Some(3), ..bare(WriteProblem::CountOutOfRange) }
        );
        assert_eq!(
            invalid(json!({"goal_problem": "record_out_of_limits", "range_min": 0, "range_max": 16})),
            WriteInvalid { min: Some(0), max: Some(16), ..bare(WriteProblem::RecordOutOfLimits) }
        );
        assert_eq!(
            invalid(json!({
                "goal_problem": "variable_not_applying",
                "goal_name": "extra",
                "variable_problem": "irreversible_by_root",
            })),
            WriteInvalid {
                variable: Some(Unapplied::IrreversibleByRoot),
                ..bare(WriteProblem::VariableNotApplying)
            }
        );
        assert_eq!(
            invalid(json!({"goal_problem": "wrong_type", "json_pointer": "/by"})),
            WriteInvalid { pointer: Some("/by".to_owned()), ..bare(WriteProblem::WrongType) }
        );
        assert_eq!(
            invalid(json!({"goal_problem": "pool_unvalued", "pool_id": "pool.wood"})),
            WriteInvalid { pool: Some("pool.wood".to_owned()), ..bare(WriteProblem::PoolUnvalued) }
        );
        assert_eq!(
            invalid(json!({"goal_problem": "not_checked_yet"})),
            bare(WriteProblem::NotCheckedYet)
        );
        assert_eq!(
            invalid(json!({"goal_problem": "a_newer_word"})),
            bare(WriteProblem::Other("a_newer_word".to_owned()))
        );
    }

    /// The seven kinds of answer but the one that took: a Runtime without the write (the client's
    /// word, or `not_implemented`), nothing known, a conflict, another write in the same
    /// millisecond, the value refused, another refusal.
    #[test]
    fn a_failed_write_comes_to_one_of_its_kinds() {
        let none = Map::new();
        // The client's own word that the Runtime does not know the request comes first.
        assert_eq!(
            failure_of("runtime_operation_unsupported", false, "", &none),
            Failed::Unsupported("runtime_operation_unsupported")
        );
        assert_eq!(
            failure_of("runtime_request_failed", true, "not_implemented", &none),
            Failed::Unsupported("not_implemented")
        );
        // Nothing settles it: a failed write, a receipt not in time, a refusal that halted the
        // host, whatever its code.
        for code in ["", "goal_value_invalid", "fact_observation_version_conflict"] {
            assert_eq!(failure_of("runtime_io_timeout", false, code, &none), Failed::Unconfirmed);
        }
        let conflict = values(json!({"current_version": 3}));
        assert_eq!(
            failure_of("runtime_request_refused", true, "fact_observation_version_conflict", &conflict),
            Failed::Answered(Answered::Conflict { current: Some(3) })
        );
        assert_eq!(
            failure_of("runtime_request_refused", true, "fact_observation_not_newer", &none),
            Failed::Answered(Answered::NotNewer)
        );
        let invalid = values(json!({"goal_problem": "not_offered", "goal_name": "route"}));
        assert_eq!(
            failure_of("runtime_request_refused", true, "goal_value_invalid", &invalid),
            Failed::Answered(Answered::Invalid(WriteInvalid {
                problem: WriteProblem::NotOffered,
                min: None,
                max: None,
                pointer: None,
                pool: None,
                variable: None,
            }))
        );
        assert_eq!(
            failure_of("runtime_request_refused", true, "channel_binding_missing", &none),
            Failed::Answered(Answered::Other)
        );
    }

    #[test]
    fn every_word_of_goal_problem_is_read() {
        assert_eq!(GoalProblem::ALL.len(), 23);
        let words: Vec<WriteProblem> =
            GoalProblem::ALL.iter().map(|problem| WriteProblem::of(problem.as_str())).collect();
        for (problem, word) in words.iter().zip(GoalProblem::ALL) {
            assert!(!matches!(problem, WriteProblem::Other(_)), "{word}");
        }
        for (at, problem) in words.iter().enumerate() {
            assert!(!words[..at].contains(problem), "{problem:?} twice");
        }
        for (word, expected) in [
            ("name_unknown", WriteProblem::NameUnknown),
            ("holds_no_value", WriteProblem::HoldsNoValue),
            ("tier_not_built", WriteProblem::TierNotBuilt),
            ("remedy_parameter", WriteProblem::RemedyParameter),
            ("variable_not_applying", WriteProblem::VariableNotApplying),
            ("not_offered", WriteProblem::NotOffered),
            ("value_not_json", WriteProblem::ValueNotJson),
            ("wrong_type", WriteProblem::WrongType),
            ("out_of_range", WriteProblem::OutOfRange),
            ("off_step", WriteProblem::OffStep),
            ("not_an_option", WriteProblem::NotAnOption),
            ("key_repeated", WriteProblem::KeyRepeated),
            ("count_out_of_range", WriteProblem::CountOutOfRange),
            ("value_too_large", WriteProblem::ValueTooLarge),
            ("pool_not_targetable", WriteProblem::PoolNotTargetable),
            ("pool_unvalued", WriteProblem::PoolUnvalued),
            ("deadline_unsupported", WriteProblem::DeadlineUnsupported),
            ("value_not_set", WriteProblem::ValueNotSet),
            ("record_unreadable", WriteProblem::RecordUnreadable),
            ("record_out_of_limits", WriteProblem::RecordOutOfLimits),
            ("not_checked_yet", WriteProblem::NotCheckedYet),
            ("root_failing", WriteProblem::RootFailing),
            ("no_schedule", WriteProblem::NoSchedule),
        ] {
            assert_eq!(WriteProblem::of(word), expected, "{word}");
        }
    }

    #[test]
    fn a_variable_the_root_does_not_apply_takes_no_value() {
        for word in [
            "irreversible_by_root",
            "package_unreadable",
            "goals_unreadable",
            "declaration_conflict",
            "goal_name_conflict",
            "name_out_of_limits",
            "cap_range",
            "root_unchecked",
        ] {
            assert!(Unapplied::of(word).by_root(), "{word}");
        }
        for word in [
            "not_an_option",
            "out_of_range",
            "off_step",
            "type_changed",
            "keys_removed",
            "count_out_of_range",
            "key_repeated",
            "keys_left_out",
            "entry_unreadable",
            "a_newer_word",
        ] {
            assert!(!Unapplied::of(word).by_root(), "{word}");
        }
    }

    /// G3: the control a variable of `declaration` comes with, in `tongue`, at the value in effect
    /// `effective`.
    fn control_in(declaration: Value, effective: &str, tongue: Tongue) -> ValueControl {
        let view = view(
            state("accepted"),
            json!([variable("v", true, declaration, &["pack.a"], json!({
                "effective_json": effective,
                "source": "default",
                "state": "in_effect",
            }))]),
            json!({}),
            no_targets(),
        );
        match page(&view, tongue, &|_| None).lines.into_iter().next() {
            Some(GoalLine::Variable(line)) => line.control,
            line => panic!("{line:?}"),
        }
    }

    fn control_of(declaration: Value, effective: &str) -> ValueControl {
        control_in(declaration, effective, Tongue::En)
    }

    /// G3 (the UI channel's ruling of 11-21, option B with A as its fallback): a time is typed in
    /// the largest unit at most the declared one that is a whole number of steps, of which `min`
    /// is a whole number, and one of which on from `min` is not above `max`; failing every unit,
    /// the largest of which both `min` and the step are whole numbers.
    #[test]
    fn a_time_is_typed_in_the_largest_unit_on_a_step_from_min() {
        for (declared, min, max, step, expected) in [
            (TimeUnit::Minute, 0, 7_200_000, 60_000, TimeUnit::Minute),
            // No unit is a whole number of 30-second steps of which 30 seconds is a whole number:
            // the fallback takes seconds.
            (TimeUnit::Minute, 30_000, 3_645_000, 30_000, TimeUnit::Second),
            // The Runtime's own test's (945727d4
            // `crates/actingcommand-contract/src/package_variables_tests.rs:156-178`), whose range
            // holds no minute: milliseconds, so that every value on its step can be typed.
            (TimeUnit::Minute, 0, 200, 10, TimeUnit::Millisecond),
            // The same step on a range that holds minutes: minutes.
            (TimeUnit::Minute, 0, 7_200_000, 10, TimeUnit::Minute),
            (TimeUnit::Minute, 0, 7_200_000, 30_000, TimeUnit::Minute),
            (TimeUnit::Hour, 1_800_000, 86_400_000, 3_600_000, TimeUnit::Minute),
            (TimeUnit::Day, 0, 604_800_000, 3_600_000, TimeUnit::Day),
            // A unit declared longer than the range: down to one the range holds.
            (TimeUnit::Day, 0, 43_200_000, 3_600_000, TimeUnit::Hour),
            (TimeUnit::Hour, 0, 1_800_000, 60_000, TimeUnit::Minute),
            (TimeUnit::Second, 0, 60_000, 1, TimeUnit::Second),
            // No step (1 millisecond): the unit declared.
            (TimeUnit::Hour, 0, 86_400_000, 1, TimeUnit::Hour),
            // A `min` that is no whole number of minutes: down to seconds.
            (TimeUnit::Minute, 30_000, 3_600_000, 1, TimeUnit::Second),
            // A step of 7 milliseconds: no unit is a whole number of it; the fallback,
            // milliseconds.
            (TimeUnit::Minute, 0, 7_200_000, 7, TimeUnit::Millisecond),
            // A negative `min`.
            (TimeUnit::Minute, -60_000, 60_000, 60_000, TimeUnit::Minute),
            (TimeUnit::Minute, -30_000, 30_000, 1, TimeUnit::Second),
            (TimeUnit::Hour, -5_400_000, 5_400_000, 60_000, TimeUnit::Minute),
        ] {
            assert_eq!(
                entry_unit(declared, min, max, step),
                expected,
                "{declared:?} {min} {max} {step}"
            );
        }
    }

    #[test]
    fn a_value_is_in_the_unit_typed_in_only_when_it_is_a_whole_number_of_it() {
        let minutes = NumberRule {
            min: 0,
            max: 7_200_000,
            step: 60_000,
            unit: NumberUnit::Time { declared: TimeUnit::Hour, entry: TimeUnit::Minute },
        };
        assert_eq!(minutes.in_entry(120_000), Some(2));
        assert_eq!(minutes.in_entry(-60_000), Some(-1));
        assert_eq!(minutes.in_entry(0), Some(0));
        assert_eq!(minutes.in_entry(90_000), None);
        let plain = NumberRule { min: -50, max: 50, step: 5, unit: NumberUnit::Plain };
        assert_eq!(plain.in_entry(-45), Some(-45));
    }

    /// G3: an `integer` and an `ordered` variable are read as the Runtime reads them; whatever it
    /// would refuse, or a type this console does not know, is shown only. A block is not read.
    #[test]
    fn numbers_and_lists_are_read_as_the_runtime_reads_them() {
        let integer_of = |members: Value| {
            let mut declaration = json!({
                "name": "v",
                "type": "integer",
                "title": {"en": "Wait"},
                "min": 0,
                "max": 7_200_000,
                "default": 0,
            });
            if let Value::Object(members) = members {
                for (member, value) in members {
                    declaration[member.as_str()] = value;
                }
            }
            declaration
        };
        assert_eq!(
            control_of(integer_of(json!({"step": 60_000, "unit": "minute"})), "1800000"),
            ValueControl::Number {
                rule: NumberRule {
                    min: 0,
                    max: 7_200_000,
                    step: 60_000,
                    unit: NumberUnit::Time { declared: TimeUnit::Minute, entry: TimeUnit::Minute },
                },
                now: Some(1_800_000),
            }
        );
        // No step: 1. Another block than the base one is not read.
        assert_eq!(
            control_of(integer_of(json!({"block": "slider"})), "5"),
            ValueControl::Number {
                rule: NumberRule { min: 0, max: 7_200_000, step: 1, unit: NumberUnit::Plain },
                now: Some(5),
            }
        );
        let max_safe = 9_007_199_254_740_991_i64;
        for broken in [
            json!({"name": "v", "type": "integer", "title": {"en": "Wait"}, "min": 0, "default": 0}),
            integer_of(json!({"step": 0})),
            integer_of(json!({"step": -5})),
            integer_of(json!({"min": 10, "max": 5})),
            integer_of(json!({"unit": "week"})),
            integer_of(json!({"unit": 5})),
            integer_of(json!({"unit": {}})),
            integer_of(json!({"min": -max_safe - 1})),
            integer_of(json!({"max": max_safe + 1})),
            integer_of(json!({"min": 1.5})),
        ] {
            assert_eq!(control_of(broken.clone(), "0"), ValueControl::Unknown, "{broken}");
        }
        // The safe integers themselves read.
        assert!(matches!(
            control_of(integer_of(json!({"min": -max_safe, "max": max_safe})), "0"),
            ValueControl::Number { .. }
        ));

        let ordered_with = |options: Value, min_count: Value, max_count: Value| {
            json!({
                "name": "v",
                "type": "ordered",
                "title": {"en": "Order"},
                "options": options,
                "min_count": min_count,
                "max_count": max_count,
                "default": [],
            })
        };
        let two = json!([
            {"value": "item_a", "title": {"en": "Item A", "zh-Hans": "甲"}},
            {"value": "item_b", "title": {"en": "Item B"}},
        ]);
        // Titled in the language shown, each searched in every language's title.
        assert_eq!(
            control_in(ordered_with(two.clone(), json!(0), json!(2)), "[\"item_b\"]", Tongue::ZhHans),
            ValueControl::Ordered {
                rule: OrderRule {
                    options: vec![
                        OrderOption {
                            value: "item_a".to_owned(),
                            title: "甲".to_owned(),
                            words: strings(&["item a", "甲"]),
                        },
                        OrderOption {
                            value: "item_b".to_owned(),
                            title: "Item B".to_owned(),
                            words: strings(&["item b"]),
                        },
                    ],
                    min_count: 0,
                    max_count: 2,
                },
                chosen: strings(&["item_b"]),
            }
        );
        // A value in effect that is no list of strings: none chosen. `ordered_list` is its block.
        let mut listed = ordered_with(two.clone(), json!(0), json!(2));
        listed["block"] = json!("ordered_list");
        match control_of(listed, "7") {
            ValueControl::Ordered { chosen, .. } => assert!(chosen.is_empty()),
            control => panic!("{control:?}"),
        }
        let many = |count: usize| -> Value {
            (0..count)
                .map(|at| {
                    json!({"value": format!("crew_{at:04}"), "title": {"en": format!("Crew {at}")}})
                })
                .collect()
        };
        let long_value = "a".repeat(65);
        for broken in [
            ordered_with(json!([]), json!(0), json!(1)),
            ordered_with(many(1025), json!(0), json!(1)),
            ordered_with(two.clone(), json!(0), json!(0)),
            ordered_with(two.clone(), json!(0), json!(33)),
            ordered_with(two.clone(), json!(3), json!(3)),
            ordered_with(two.clone(), json!(2), json!(1)),
            ordered_with(two.clone(), json!(-1), json!(1)),
            ordered_with(json!([{"title": {"en": "No value"}}]), json!(0), json!(1)),
            ordered_with(json!([{"value": "Bad-Value", "title": {"en": "Bad"}}]), json!(0), json!(1)),
            ordered_with(json!([{"value": long_value, "title": {"en": "Long"}}]), json!(0), json!(1)),
            ordered_with(
                json!([
                    {"value": "item_a", "title": {"en": "A"}},
                    {"value": "item_a", "title": {"en": "A again"}},
                ]),
                json!(0),
                json!(2),
            ),
            json!({"name": "v", "type": "table", "title": {"en": "Table"}}),
            json!({"name": "v", "title": {"en": "No type"}}),
        ] {
            assert_eq!(control_of(broken.clone(), "[]"), ValueControl::Unknown, "{broken}");
        }
        // 1024 options and a `max_count` above the number of options read.
        assert!(matches!(
            control_of(ordered_with(many(1024), json!(0), json!(32)), "[]"),
            ValueControl::Ordered { .. }
        ));
        match control_of(ordered_with(two, json!(0), json!(32)), "[]") {
            ValueControl::Ordered { rule, .. } => assert_eq!(rule.cap(), 2),
            control => panic!("{control:?}"),
        }
    }

    /// The rules of the walk's `wait` (minutes), `pause` (typed in seconds, `max` off its step),
    /// `offset` (negative) and `keep` (a text unit).
    fn wait_rule() -> NumberRule {
        NumberRule {
            min: 0,
            max: 7_200_000,
            step: 60_000,
            unit: NumberUnit::Time {
                declared: TimeUnit::Minute,
                entry: entry_unit(TimeUnit::Minute, 0, 7_200_000, 60_000),
            },
        }
    }

    fn pause_rule() -> NumberRule {
        NumberRule {
            min: 30_000,
            max: 3_645_000,
            step: 30_000,
            unit: NumberUnit::Time {
                declared: TimeUnit::Minute,
                entry: entry_unit(TimeUnit::Minute, 30_000, 3_645_000, 30_000),
            },
        }
    }

    #[test]
    fn a_number_is_checked_as_the_runtime_checks_it() {
        let wait = wait_rule();
        assert_eq!(wait.unit, NumberUnit::Time { declared: TimeUnit::Minute, entry: TimeUnit::Minute });
        let now = Some(1_800_000);
        for (text, expected) in [
            ("45", Ok::<i64, NumberProblem>(2_700_000)),
            (" +45 ", Ok(2_700_000)),
            ("0", Ok(0)),
            ("120", Ok(7_200_000)),
            ("30", Err(NumberProblem::Same)),
            ("", Err(NumberProblem::Empty)),
            ("  ", Err(NumberProblem::Empty)),
            ("abc", Err(NumberProblem::NotWhole)),
            ("1.5", Err(NumberProblem::NotWhole)),
            ("４５", Err(NumberProblem::NotWhole)),
            ("4 5", Err(NumberProblem::NotWhole)),
            ("1,000", Err(NumberProblem::NotWhole)),
            ("+-5", Err(NumberProblem::NotWhole)),
            ("-", Err(NumberProblem::NotWhole)),
            ("121", Err(NumberProblem::OutOfRange)),
            ("-1", Err(NumberProblem::OutOfRange)),
            // Beyond what parses, and beyond what is a number of milliseconds.
            ("99999999999999999999", Err(NumberProblem::OutOfRange)),
            ("999999999999999999", Err(NumberProblem::OutOfRange)),
        ] {
            assert_eq!(number(text, &wait, now), expected, "{text:?}");
        }
        assert_eq!(largest(&wait), 7_200_000);

        let pause = pause_rule();
        assert_eq!(pause.unit, NumberUnit::Time { declared: TimeUnit::Minute, entry: TimeUnit::Second });
        for (text, expected) in [
            ("45", Err::<i64, NumberProblem>(NumberProblem::OffStep)),
            // Not above `max`, but off its step.
            ("3645", Err(NumberProblem::OffStep)),
            ("3660", Err(NumberProblem::OutOfRange)),
            ("90", Ok(90_000)),
            ("3630", Ok(3_630_000)),
        ] {
            assert_eq!(number(text, &pause, Some(60_000)), expected, "{text:?}");
        }
        assert_eq!(largest(&pause), 3_630_000);
        assert_eq!(pause.interval(), 30_000);

        let offset = NumberRule { min: -50, max: 50, step: 5, unit: NumberUnit::Plain };
        for (text, expected) in [
            ("-45", Ok::<i64, NumberProblem>(-45)),
            ("-47", Err(NumberProblem::OffStep)),
            ("-55", Err(NumberProblem::OutOfRange)),
            ("15", Err(NumberProblem::Same)),
        ] {
            assert_eq!(number(text, &offset, Some(15)), expected, "{text:?}");
        }
        assert_eq!(largest(&offset), 50);

        let keep =
            NumberRule { min: 0, max: 100, step: 10, unit: NumberUnit::Text("points".to_owned()) };
        assert_eq!(number("40", &keep, Some(50)), Ok(40));
        assert_eq!(number("45", &keep, Some(50)), Err(NumberProblem::OffStep));

        // Declared in hours without a step: typed in whole hours, so the largest is the last whole
        // hour at most `max`.
        let hours = NumberRule {
            min: 0,
            max: 5_400_000,
            step: 1,
            unit: NumberUnit::Time {
                declared: TimeUnit::Hour,
                entry: entry_unit(TimeUnit::Hour, 0, 5_400_000, 1),
            },
        };
        assert_eq!(hours.interval(), 3_600_000);
        assert_eq!(largest(&hours), 3_600_000);
        assert_eq!(number("1", &hours, Some(5_400_000)), Ok(3_600_000));
        assert_eq!(number("2", &hours, Some(5_400_000)), Err(NumberProblem::OutOfRange));

        assert_eq!(integer_json(-45), "-45");
        assert_eq!(integer_json(2_700_000), "2700000");
    }

    /// A rule of three options `item_a` to `item_c`.
    fn three_rule(min_count: u64, max_count: u64) -> OrderRule {
        OrderRule {
            options: vec![
                order_option("item_a", "Item A"),
                order_option("item_b", "Item B"),
                order_option("item_c", "Item C"),
            ],
            min_count,
            max_count,
        }
    }

    /// The value of the `at`th option 64 bytes long.
    fn long_key(at: usize) -> String {
        format!("long_{at:02}_{}", "x".repeat(56))
    }

    /// A rule of `count` options of values 64 bytes long, and a short one, `short`.
    fn long_rule(count: usize) -> OrderRule {
        let mut options: Vec<OrderOption> = (0..count)
            .map(|at| order_option(&long_key(at), &format!("Long crew {at:02}")))
            .collect();
        options.push(order_option("short", "Short"));
        OrderRule { options, min_count: 0, max_count: 32 }
    }

    #[test]
    fn a_list_is_edited_within_its_rule() {
        assert_eq!(ordered_json(&[]), "[]");
        assert_eq!(ordered_json(&strings(&["a", "b"])), "[\"a\",\"b\"]");
        let rule = three_rule(0, 2);
        let add = |keys: &[&str], value: &str| {
            order_edit(&rule, &strings(keys), &OrderEdit::Add(value.to_owned()))
        };
        // At the end.
        assert_eq!(add(&["item_b"], "item_a"), Some(strings(&["item_b", "item_a"])));
        assert_eq!(add(&[], "item_c"), Some(strings(&["item_c"])));
        // In it already, none of the options, the list full.
        assert_eq!(add(&["item_b"], "item_b"), None);
        assert_eq!(add(&["item_b"], "item_x"), None);
        assert_eq!(add(&["item_b", "item_a"], "item_c"), None);
        // A `max_count` of 32 over two options: two at most.
        let two = OrderRule {
            options: vec![order_option("item_a", "Item A"), order_option("item_b", "Item B")],
            min_count: 0,
            max_count: 32,
        };
        assert_eq!(two.cap(), 2);
        let both = order_edit(&two, &strings(&["item_a"]), &OrderEdit::Add("item_b".to_owned()));
        assert_eq!(both, Some(strings(&["item_a", "item_b"])));
        let (offered, more) = order_matches(&two, &strings(&["item_a", "item_b"]), "");
        assert_eq!(more, 0);
        assert!(offered.iter().all(|option| option.chosen && !option.fits));

        // Fifteen keys of 64 bytes take 1006 bytes and fit; a sixteenth would make 1073.
        let long = long_rule(16);
        let fifteen: Vec<String> = (0..15).map(long_key).collect();
        assert_eq!(long_key(0).len(), 64);
        assert_eq!(ordered_json(&fifteen).len(), 1006);
        assert!(order_fits(&fifteen, None));
        assert!(!order_fits(&fifteen, Some(long_key(15).as_str())));
        let sixteen: Vec<String> = (0..16).map(long_key).collect();
        assert_eq!(ordered_json(&sixteen).len(), 1073);
        assert!(ordered_json(&sixteen).len() > VALUE_BYTES);
        assert_eq!(order_edit(&long, &fifteen, &OrderEdit::Add(long_key(15))), None);
        // A short one still fits; without it, no option left fits.
        let mut with_short = fifteen.clone();
        with_short.push("short".to_owned());
        assert_eq!(order_edit(&long, &fifteen, &OrderEdit::Add("short".to_owned())), Some(with_short));
        assert!(order_room(&long, &fifteen));
        let only_long = OrderRule { options: long.options[..16].to_vec(), ..long.clone() };
        assert!(!order_room(&only_long, &fifteen));
        assert!(order_room(&only_long, &fifteen[..14]));

        let keys = strings(&["item_a", "item_b", "item_c"]);
        let edit = |change: OrderEdit| order_edit(&three_rule(0, 3), &keys, &change);
        // The first moved up (the window's place -1 is never a place) and the last down: none.
        assert_eq!(edit(OrderEdit::Move { from: 0, to: usize::MAX }), None);
        assert_eq!(edit(OrderEdit::Move { from: 2, to: 3 }), None);
        assert_eq!(edit(OrderEdit::Move { from: 3, to: 2 }), None);
        assert_eq!(edit(OrderEdit::Move { from: 1, to: 1 }), None);
        assert_eq!(
            edit(OrderEdit::Move { from: 0, to: 1 }),
            Some(strings(&["item_b", "item_a", "item_c"]))
        );
        assert_eq!(
            edit(OrderEdit::Move { from: 2, to: 1 }),
            Some(strings(&["item_a", "item_c", "item_b"]))
        );
        assert_eq!(edit(OrderEdit::Remove(3)), None);
        assert_eq!(edit(OrderEdit::Remove(1)), Some(strings(&["item_a", "item_c"])));
        // Below `min_count` is no reason not to remove.
        let one = order_edit(&three_rule(1, 3), &strings(&["item_a"]), &OrderEdit::Remove(0));
        assert_eq!(one, Some(Vec::new()));
    }

    #[test]
    fn what_keeps_a_list_from_being_saved_is_said_in_one_order() {
        let rule = three_rule(1, 2);
        let problem = |keys: &[&str]| order_problem(&rule, &strings(keys));
        assert_eq!(problem(&["item_a", "item_a", "item_x", "item_y"]), Some(OrderProblem::Repeated));
        assert_eq!(problem(&["item_a", "item_x", "item_y"]), Some(OrderProblem::Gone(2)));
        assert_eq!(problem(&[]), Some(OrderProblem::TooFew(1)));
        assert_eq!(problem(&["item_a", "item_b", "item_c"]), Some(OrderProblem::TooMany(2)));
        assert_eq!(problem(&["item_a"]), None);
        assert_eq!(problem(&["item_c", "item_a"]), None);
        let sixteen: Vec<String> = (0..16).map(long_key).collect();
        assert_eq!(order_problem(&long_rule(16), &sixteen), Some(OrderProblem::TooLong(1073)));
        assert_eq!(order_problem(&long_rule(16), &sixteen[..15]), None);
    }

    #[test]
    fn options_are_found_by_their_titles_in_any_language() {
        let crew = |at: usize| {
            json!({"value": format!("crew_{at:03}"), "title": {"en": format!("Crew {at:03}"), "zh-Hans": format!("队员 {at:03}")}})
        };
        let mut options: Vec<Value> = (1..=12).map(crew).collect();
        options.push(json!({"value": "reserve_01", "title": {"en": "Reserve 01"}}));
        let declaration = json!({
            "name": "v",
            "type": "ordered",
            "title": {"en": "Crew"},
            "options": options,
            "min_count": 1,
            "max_count": 32,
            "default": ["crew_001"],
        });
        let rule = match control_in(declaration, "[\"crew_001\"]", Tongue::ZhHans) {
            ValueControl::Ordered { rule, .. } => rule,
            control => panic!("{control:?}"),
        };
        let titles = |query: &str| -> Vec<String> {
            let (found, more) = order_matches(&rule, &strings(&["crew_001"]), query);
            assert_eq!(more, 0, "{query}");
            found.into_iter().map(|option| option.title).collect()
        };
        // An English title on a Chinese page, in any case.
        assert_eq!(titles("Reserve"), strings(&["Reserve 01"]));
        assert_eq!(titles("  RESERVE "), strings(&["Reserve 01"]));
        assert_eq!(titles("crew 007"), strings(&["队员 007"]));
        assert_eq!(titles("队员 01"), strings(&["队员 010", "队员 011", "队员 012"]));
        // Never by the option's value.
        assert!(titles("crew_").is_empty());
        assert!(titles("reserve_01").is_empty());
        assert_eq!(titles("").len(), 13);
        // The one in the draft is marked, and does not fit again.
        let (found, _) = order_matches(&rule, &strings(&["crew_001"]), "队员 001");
        assert_eq!(found.len(), 1);
        assert!(found[0].chosen && !found[0].fits);

        // At most the first hundred, and how many more.
        for (count, more) in [(400, 300), (1024, 924)] {
            let rule = OrderRule {
                options: (0..count).map(|at| order_option(&format!("crew_{at:04}"), &format!("Crew {at:04}"))).collect(),
                min_count: 0,
                max_count: 32,
            };
            let (found, left) = order_matches(&rule, &[], "");
            assert_eq!(found.len(), ORDER_SHOWN);
            assert_eq!(left, more);
            assert_eq!(found[0].value, "crew_0000");
            assert_eq!(found[99].value, "crew_0099");
        }

        // A long value that no longer fits does not; a short one still does.
        let long = long_rule(16);
        let fifteen: Vec<String> = (0..15).map(long_key).collect();
        let (found, _) = order_matches(&long, &fifteen, "");
        let fits: Vec<(String, bool, bool)> =
            found.into_iter().map(|option| (option.value, option.chosen, option.fits)).collect();
        assert_eq!(fits[0], (long_key(0), true, false));
        assert_eq!(fits[15], (long_key(15), false, false));
        assert_eq!(fits[16], ("short".to_owned(), false, true));
        // Full: nothing fits.
        let (found, _) = order_matches(&three_rule(0, 2), &strings(&["item_a", "item_c"]), "");
        assert!(found.iter().all(|option| !option.fits));
        assert_eq!(
            found.iter().map(|option| option.chosen).collect::<Vec<bool>>(),
            vec![true, false, true]
        );
        // A key no longer offered in the draft is no fault here.
        let (found, more) = order_matches(&three_rule(0, 3), &strings(&["item_x"]), "item");
        assert_eq!(more, 0);
        assert!(found.iter().all(|option| !option.chosen && option.fits));
    }

    #[test]
    fn a_refusals_pointer_names_the_key_sent_at_its_place() {
        let sent = "[\"item_a\",\"item_b\",3]";
        assert_eq!(pointed_key(sent, "/1"), Some("item_b".to_owned()));
        assert_eq!(pointed_key(sent, "/0"), Some("item_a".to_owned()));
        assert_eq!(pointed_key(sent, "/2"), None);
        assert_eq!(pointed_key(sent, "/3"), None);
        assert_eq!(pointed_key(sent, "/x"), None);
        assert_eq!(pointed_key(sent, "/-1"), None);
        assert_eq!(pointed_key(sent, "/"), None);
        assert_eq!(pointed_key(sent, ""), None);
        assert_eq!(pointed_key("\"item_a\"", "/0"), None);
        assert_eq!(pointed_key("not json", "/0"), None);
    }

    /// An `ordered` declaration of `item_a` to `item_c`.
    fn ordered_of(name: &str, min_count: u64, max_count: u64, default: &[&str]) -> Value {
        json!({
            "name": name,
            "type": "ordered",
            "title": {"en": "Order"},
            "options": [
                {"value": "item_a", "title": {"en": "Item A"}},
                {"value": "item_b", "title": {"en": "Item B"}},
                {"value": "item_c", "title": {"en": "Item C"}},
            ],
            "min_count": min_count,
            "max_count": max_count,
            "default": default,
        })
    }

    /// G3: a page of numbers and lists to press on: a number stored and not used; a list in
    /// effect, one in effect with a key left out, one not used whose keys are partly gone, one not
    /// used with a key named twice, one not used of another type; a type this console does not
    /// know, in effect and not used; and a
    /// placement not offered, with a value and no declaration.
    fn numbers_and_lists_page() -> GoalsPage {
        let keep = json!({
            "name": "keep",
            "type": "integer",
            "title": {"en": "Keep"},
            "min": 0,
            "max": 100,
            "step": 10,
            "unit": {"en": "points"},
            "default": 50,
        });
        read(&view(
            state("accepted"),
            json!([
                variable("keep", true, keep, &["pack.a"], json!({
                    "default_json": "50",
                    "value_json": "120",
                    "effective_json": "50",
                    "source": "default",
                    "set": set("ui", None),
                    "state": "not_applied",
                    "variable_problem": "out_of_range",
                    "display": {"en": "50 points"},
                })),
                variable("order", true, ordered_of("order", 1, 3, &["item_b"]), &["pack.a"], json!({
                    "default_json": "[\"item_b\"]",
                    "value_json": "[\"item_a\",\"item_b\"]",
                    "effective_json": "[\"item_a\",\"item_b\"]",
                    "source": "set",
                    "set": set("ui", None),
                    "state": "in_effect",
                })),
                variable("kept", true, ordered_of("kept", 1, 3, &["item_b"]), &["pack.a"], json!({
                    "default_json": "[\"item_b\"]",
                    "value_json": "[\"item_x\",\"item_a\"]",
                    "effective_json": "[\"item_a\"]",
                    "removed_keys": ["item_x"],
                    "source": "set",
                    "set": set("lab", Some("Lab-CLI")),
                    "state": "in_effect",
                    "variable_problem": "keys_left_out",
                })),
                variable("squad", true, ordered_of("squad", 2, 3, &["item_b", "item_c"]), &["pack.a"], json!({
                    "default_json": "[\"item_b\",\"item_c\"]",
                    "value_json": "[\"item_a\",\"item_x\",\"item_y\"]",
                    "effective_json": "[\"item_b\",\"item_c\"]",
                    "source": "default",
                    "set": set("lab", Some("Lab-CLI")),
                    "state": "not_applied",
                    "variable_problem": "keys_removed",
                })),
                variable("twice", true, ordered_of("twice", 1, 3, &["item_b"]), &["pack.a"], json!({
                    "default_json": "[\"item_b\"]",
                    "value_json": "[\"item_x\",\"item_a\",\"item_x\"]",
                    "effective_json": "[\"item_b\"]",
                    "source": "default",
                    "set": set("cli", None),
                    "state": "not_applied",
                    "variable_problem": "key_repeated",
                })),
                variable("typed", true, ordered_of("typed", 1, 3, &["item_b"]), &["pack.a"], json!({
                    "default_json": "[\"item_b\"]",
                    "value_json": "\"item_a\"",
                    "effective_json": "[\"item_b\"]",
                    "source": "default",
                    "set": set("cli", None),
                    "state": "not_applied",
                    "variable_problem": "type_changed",
                })),
                variable("odd", true, json!({"name": "odd", "type": "table", "title": {"en": "Odd"}}), &["pack.a"], json!({
                    "value_json": "[1]",
                    "effective_json": "[1]",
                    "source": "set",
                    "set": set("lab", None),
                    "state": "in_effect",
                })),
                variable("weekly", true, json!({"name": "weekly", "type": "integer", "title": {"en": "Weekly"}, "min": 0, "max": 9, "unit": "week", "default": 1}), &["pack.a"], json!({
                    "value_json": "12",
                    "effective_json": "1",
                    "source": "default",
                    "set": set("cli", None),
                    "state": "not_applied",
                    "variable_problem": "out_of_range",
                })),
                {
                    "name": "lost",
                    "entry_type": "variable",
                    "placed": true,
                    "entry": {"name": "lost", "type": "variable"},
                    "value_json": "3",
                    "set": set("cli", None),
                    "state": "not_offered",
                },
            ]),
            json!({}),
            no_targets(),
        ))
    }

    /// G3: what a number or a list set, and back to the default, write, and what they ask: only
    /// what cannot be set back from here, a value stored and not used, or the keys of a list in
    /// effect that the run leaves out. A type this console does not know is never written, but
    /// cleared where no bound package offers it.
    #[test]
    fn numbers_and_lists_ask_only_for_what_cannot_be_set_back() {
        let page = numbers_and_lists_page();
        let presses = Presses(&page);
        let order = |keys: &[&str]| GoalAct::Order(strings(keys));

        // A number stored and not used: another number, or the default again, loses it; the
        // default typed is the number in effect, which writes nothing.
        let stored = Shown::Words("120 points".to_owned());
        assert_eq!(
            presses.on("variable:keep", typed("40")),
            asks(set_of("keep", "40"), Loses::Replaced(stored.clone()))
        );
        assert_eq!(presses.on("variable:keep", typed("50")), None);
        assert_eq!(presses.on("variable:keep", typed("45")), None);
        assert_eq!(presses.on("variable:keep", GoalAct::Clear), asks(clear_of("keep"), Loses::Value(stored)));
        assert_eq!(presses.on("variable:keep", order(&["item_a"])), None);

        // A list in effect with nothing left out: saved and put back at once.
        assert_eq!(
            presses.on("variable:order", order(&["item_b", "item_a"])),
            goes(set_of("order", "[\"item_b\",\"item_a\"]"))
        );
        assert_eq!(presses.on("variable:order", order(&["item_a", "item_b"])), None);
        assert_eq!(presses.on("variable:order", order(&["item_a", "item_x"])), None);
        assert_eq!(presses.on("variable:order", order(&[])), None);
        assert_eq!(presses.on("variable:order", order(&["item_a", "item_a"])), None);
        assert_eq!(presses.on("variable:order", typed("5")), None);
        assert_eq!(presses.on("variable:order", GoalAct::Clear), goes(clear_of("order")));

        // In effect with a key left out: saving another list, or the default again, loses it.
        let left_out = strings(&["item_x"]);
        assert_eq!(
            presses.on("variable:kept", order(&["item_a", "item_b"])),
            asks(
                set_of("kept", "[\"item_a\",\"item_b\"]"),
                Loses::LeftOut { keys: left_out.clone(), cleared: false }
            )
        );
        assert_eq!(presses.on("variable:kept", order(&["item_a"])), None);
        assert_eq!(
            presses.on("variable:kept", GoalAct::Clear),
            asks(clear_of("kept"), Loses::LeftOut { keys: left_out, cleared: true })
        );

        // Not used: the value stored is replaced, asked; the default as it is writes nothing.
        let squad = Shown::List(strings(&["Item A", "item_x", "item_y"]));
        assert_eq!(
            presses.on("variable:squad", order(&["item_a", "item_b"])),
            asks(set_of("squad", "[\"item_a\",\"item_b\"]"), Loses::Replaced(squad.clone()))
        );
        assert_eq!(presses.on("variable:squad", order(&["item_b", "item_c"])), None);
        assert_eq!(presses.on("variable:squad", GoalAct::Clear), asks(clear_of("squad"), Loses::Value(squad)));

        // A type this console does not know, in effect or not used: nothing set, nothing cleared.
        for key in ["variable:odd", "variable:weekly"] {
            assert_eq!(presses.on(key, typed("5")), None, "{key}");
            assert_eq!(presses.on(key, order(&["item_a"])), None, "{key}");
            assert_eq!(presses.on(key, GoalAct::Clear), None, "{key}");
            match page.line(key) {
                Some(GoalLine::Variable(line)) => {
                    assert_eq!(line.control, ValueControl::Unknown, "{key}");
                    assert_eq!(line.clear_offer(), None, "{key}");
                    assert!(line.settable(), "{key}");
                }
                line => panic!("{line:?}"),
            }
        }
        // Not offered, with a value and no declaration: cleared, asked, as G2's.
        assert_eq!(
            presses.on("variable:lost", GoalAct::Clear),
            asks(clear_of("lost"), Loses::Value(Shown::Number(3)))
        );
        match page.line("variable:lost") {
            Some(GoalLine::Variable(line)) => {
                assert_eq!(line.control, ValueControl::Unknown);
                assert_eq!(line.clear_offer(), Some(ClearOffer::Clear));
            }
            line => panic!("{line:?}"),
        }
    }

    /// G3: the keys of a list stored and not used that its declaration no longer offers are found
    /// here, each once, in their order: the Runtime names none of them then (`removed_keys` is
    /// only of a value in effect). None of a value in effect, nor of one that is no list.
    #[test]
    fn the_keys_no_longer_offered_of_a_list_not_used_are_found_here() {
        let page = numbers_and_lists_page();
        let line = |key: &str| match page.line(key) {
            Some(GoalLine::Variable(line)) => line.clone(),
            line => panic!("{line:?}"),
        };
        // `keys_removed`: the Runtime names none of them.
        let squad = line("variable:squad");
        assert!(squad.left_out.is_empty());
        assert_eq!(squad.gone, strings(&["item_x", "item_y"]));
        // Named twice, said once.
        assert_eq!(line("variable:twice").gone, strings(&["item_x"]));
        let kept = line("variable:kept");
        assert_eq!(kept.left_out, strings(&["item_x"]));
        assert!(kept.gone.is_empty());
        assert!(line("variable:typed").gone.is_empty());
        assert!(line("variable:order").gone.is_empty());
        // Of a type this console does not know, none either.
        assert!(line("variable:weekly").gone.is_empty());
    }

    #[test]
    fn only_the_words_of_a_rule_say_the_page_is_older_than_the_runtime() {
        for word in ["out_of_range", "off_step", "not_an_option", "count_out_of_range", "key_repeated", "wrong_type"] {
            assert!(WriteProblem::of(word).of_the_rule(), "{word}");
        }
        for word in ["value_too_large", "variable_not_applying", "not_offered", "name_unknown", "a_newer_word"] {
            assert!(!WriteProblem::of(word).of_the_rule(), "{word}");
        }
    }
}
