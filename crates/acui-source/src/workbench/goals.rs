// SPDX-License-Identifier: GPL-3.0-only
//! An account's goals (Workflow #455 T1-C, step 1, the read half): what the Runtime says of the
//! resource targets of one instance, read with `ResourceTargetView` ([`super::ReadLane::read_goals`],
//! [`GoalsRead`]), and the rows the console's Goals tab shows of it, a pure function of that view
//! ([`page`]). Nothing here writes, and nothing out of here names a wire type.
//!
//! What the view says, all at the pinned Runtime (803fdfa3): it answers `targetable` and
//! `not_targetable`, every pool of the active catalog judged for the instance with the
//! predicates of a v2 target that names no tasks, and `active`, the stored document as the
//! evaluator reads it (`contracts/resource-targets.md:676-710`;
//! `crates/policy/src/resource_targets.rs:1683-1754`, `:1787-1903`). A targetable pool's
//! `default_scale` and `default_importance_milli` are its valuation's `scale` and
//! `gap.weight_milli`, `scale_required` and `importance_required` say that a goal on it would have
//! to state its own (`:1721-1749`). A not targetable pool's reason is the first of
//! `resource_not_observable` and `resource_out_of_scope` its pool meets, or `unmapped_task` when no
//! task of the instance produces it (`:761-784`, `:1694-1719`); any other reason of the closed set
//! is shown as its code. `active` holds the targets in document form (`:1800`, the v1 and v2
//! target shapes at `:75-88` and `:145-183`, each with `id`, `resource` and `condition {kind:
//! at_least, amount}`) and one condition per target whose pool still resolves for the instance
//! (`:1866-1869`): `computed` with the gap `max(T - c, 0)` (`:1878`), or `awaiting_observation`
//! with why. `expired` is the Runtime clock past `valid_until_unix_ms` (`:1898`), and an expired
//! document weighs nothing (`contracts/resource-targets.md:335`, `:362`). This Runtime always
//! states the validity (`crates/runtime-host/src/host/resource_targets.rs:270`); one that does
//! not is said so, not guessed. The document's `schema_version` is v1's or v2's
//! (`crates/policy/src/resource_targets.rs:43-45`, served at `:1830`, `:1853`): v2 is what an agent
//! submits today; a v1 document was written by an agent in the old form; any other is of a form
//! newer than this console, whose targets it does not interpret. A v1 document may hold more than
//! one target of a pool (its parse makes ids and tasks unique, not pools: `:326-340`), a v2 one at
//! most one (`:460`). `stale_schema` is true when the Runtime reads a stored v1 or v2 record as no
//! document at all (`crates/actingcommand-contract/src/resource_targets.rs:135-140`); this Runtime
//! never sets it (`crates/runtime-host/src/host/resource_targets.rs:289`).
//!
//! The rows, top to bottom, each kind in the Runtime's own order: a goal row of each targetable
//! pool with both defaults; a row of each targetable pool without them, which the console could
//! not give a goal (`NoStep`); a greyed row of each not targetable pool, with why; a row of each
//! stored goal whose pool is not targetable now, "no longer offered", with why when the view
//! lists that pool as not targetable (and no greyed row of that pool beside it); a row of each
//! stored target this console cannot read. The document's validity goes under them ([`Stored`]).
//! The pool's id is the only name step 1 has, so it is a row's name; no title is made up for it.
//!
//! The view takes the policy outcome gate, the policy and the fact write gate in turn while it is
//! computed (`crates/runtime-host/src/host/resource_targets.rs:186-207`): the caller reads it
//! seldom, as it reads the task switches. A Runtime that does not know the request drops the
//! connection, and the client then says `runtime_operation_unsupported`
//! (`crates/runtime-client/src/client.rs:1935-1969`); that, or a refusal `not_implemented`, is an
//! answer here, that this Runtime has no goals to read ([`GoalsAnswer::Unsupported`]).

use actingcommand_contract::{
    ActiveResourceTargets, ResourceTargetCondition, ResourceTargetConditionState,
    ResourceTargetObservation, ResourceTargetPendingReason, ResourceTargetView,
    ResourceTargetsRejectionReason, TargetableResource,
};
use serde_json::Value;

/// The document versions the view serves (`crates/policy/src/resource_targets.rs:43-45`).
const SCHEMA_V1: &str = "actingcommand.resource-targets.v1";
const SCHEMA_V2: &str = "actingcommand.resource-targets.v2";
/// The one condition a target states (`TargetCondition::AtLeast`, tagged by `kind`).
const AT_LEAST: &str = "at_least";

/// One answered `ResourceTargetView` of one instance, as far as the console reads it.
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
    Read(GoalsPage),
}

/// The Goals tab of one instance, as the view leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalsPage {
    /// When the Runtime computed the view, in unix ms.
    pub evaluated_at_ms: u64,
    /// No catalog is active: the view lists no pool at all.
    pub no_catalog: bool,
    /// The Runtime reads the instance's stored record as no document (`stale_schema`).
    pub stale_schema: bool,
    /// Top to bottom.
    pub rows: Vec<GoalRow>,
    /// The stored document, whose validity goes under the rows.
    pub stored: Stored,
}

/// One row of the tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRow {
    /// A pool a goal can be set on: the pack gives it both a step and an importance.
    Goal(PoolRow),
    /// A pool a goal can name, but only with a step or an importance of its own, which the pack
    /// does not give: the console cannot choose those numbers.
    NoStep(PoolRow),
    /// A pool no goal can name, greyed, and why.
    Closed { pool: String, why: Closed },
    /// A goal stored on a pool that is not targetable now: why, when the view lists that pool as
    /// not targetable; none when it lists the pool nowhere (no catalog is active, or the active
    /// one has no such pool).
    NoLongerOffered { goal: StoredGoal, why: Option<Closed> },
    /// A target of a document this console reads (v1 or v2) that does not read here: its place in
    /// the document, from 0.
    Unread { position: usize },
}

/// A targetable pool: its id, its reading, and the goals stored on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolRow {
    pub pool: String,
    pub reading: Reading,
    pub goals: Goals,
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

/// What the stored document says of one targetable pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Goals {
    /// The document is of a newer form than this console reads: whether it holds a goal of this
    /// pool is not known here.
    Unknown,
    /// Its goals of this pool, in document order: none, one, or more of a v1 document.
    Set(Vec<StoredGoal>),
}

/// One stored target as read here. `id` is the document's, for the log only: what the tab shows
/// is the pool's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGoal {
    pub id: String,
    pub pool: String,
    /// `at_least`.
    pub amount: u64,
    pub progress: Progress,
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

/// The instance's stored document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// None is stored, or the last one withdrew every goal (`contracts/resource-targets.md:707`).
    None,
    Document(Document),
}

/// A stored document: its form, its version, its validity and how many goals it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub form: Form,
    /// The `fact.published` event's sequence that holds it, an opaque token: for the log, and the
    /// write half's `expected_version`.
    pub version: u64,
    pub valid_until_ms: Option<u64>,
    pub expired: bool,
    pub goals: usize,
}

/// Which form a stored document is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// v2, what an agent submits today.
    Current,
    /// v1: written by an agent in the old form. Its targets are shown, read only.
    Old,
    /// Any other `schema_version`, kept for the log: a newer form, which needs a newer console.
    Newer(String),
}

/// The view of `alias` as the console reads it.
pub(super) fn goals_read(epoch: String, alias: &str, view: &ResourceTargetView) -> GoalsRead {
    GoalsRead { epoch, alias: alias.to_owned(), answer: GoalsAnswer::Read(page(view)) }
}

/// A Runtime's answer that it has no such read, by the code that says so
/// (`super::pack::view_unsupported`).
pub(super) fn goals_unsupported(epoch: String, alias: &str, code: &'static str) -> GoalsRead {
    GoalsRead { epoch, alias: alias.to_owned(), answer: GoalsAnswer::Unsupported { code } }
}

/// The tab's rows of `view`, top to bottom, and its document (see the module's words).
pub fn page(view: &ResourceTargetView) -> GoalsPage {
    let active = view.active.as_ref();
    let form = active.map(|active| form_of(&active.schema_version));
    // None while the document's targets are not read here: of a newer form.
    let entries: Option<Vec<Entry>> = match (active, &form) {
        (Some(_), Some(Form::Newer(_))) => None,
        (Some(active), _) => Some(read_entries(active)),
        (None, _) => Some(Vec::new()),
    };
    let on_pool = |pool: &str| -> Goals {
        match &entries {
            None => Goals::Unknown,
            Some(entries) => Goals::Set(
                entries
                    .iter()
                    .filter_map(|entry| match entry {
                        Entry::Goal(goal) if goal.pool == pool => Some(goal.clone()),
                        Entry::Goal(_) | Entry::Unread { .. } => None,
                    })
                    .collect(),
            ),
        }
    };
    let mut goal_rows = Vec::new();
    let mut no_step_rows = Vec::new();
    for resource in &view.targetable {
        let row = PoolRow {
            pool: resource.resource.clone(),
            reading: reading(&resource.observation),
            goals: on_pool(resource.resource.as_str()),
        };
        if stepped(resource) {
            goal_rows.push(GoalRow::Goal(row));
        } else {
            no_step_rows.push(GoalRow::NoStep(row));
        }
    }
    let mut rows = goal_rows;
    rows.append(&mut no_step_rows);
    // A pool with a goal on it is that goal's row below, with its reason, not a row of its own.
    for resource in &view.not_targetable {
        let held = match on_pool(resource.resource.as_str()) {
            Goals::Set(goals) => !goals.is_empty(),
            Goals::Unknown => false,
        };
        if !held {
            let why = closed(resource.reason);
            rows.push(GoalRow::Closed { pool: resource.resource.clone(), why });
        }
    }
    for entry in entries.iter().flatten() {
        match entry {
            Entry::Goal(goal) => {
                let offered = view.targetable.iter().any(|resource| resource.resource == goal.pool);
                if !offered {
                    let why = view
                        .not_targetable
                        .iter()
                        .find(|resource| resource.resource == goal.pool)
                        .map(|resource| closed(resource.reason));
                    rows.push(GoalRow::NoLongerOffered { goal: goal.clone(), why });
                }
            }
            Entry::Unread { position } => rows.push(GoalRow::Unread { position: *position }),
        }
    }
    let stored = match (active, form) {
        (Some(active), Some(form)) => Stored::Document(Document {
            form,
            version: active.version,
            valid_until_ms: active.valid_until_unix_ms,
            expired: active.expired,
            goals: active.targets.len(),
        }),
        (None, _) | (Some(_), None) => Stored::None,
    };
    GoalsPage {
        evaluated_at_ms: view.evaluated_at_unix_ms,
        no_catalog: view.catalog_hash.is_none(),
        stale_schema: view.stale_schema == Some(true),
        rows,
        stored,
    }
}

/// One target of the stored document, as read here.
enum Entry {
    Goal(StoredGoal),
    Unread { position: usize },
}

/// The targets of a document of a form this console reads, in document order.
fn read_entries(active: &ActiveResourceTargets) -> Vec<Entry> {
    active
        .targets
        .iter()
        .enumerate()
        .map(|(position, target)| match stored_goal(target, &active.conditions) {
            Some(goal) => Entry::Goal(goal),
            None => Entry::Unread { position },
        })
        .collect()
}

/// One target in document form: its id, its pool and its `at_least` amount, and its condition by
/// its id; none when it does not read so.
fn stored_goal(target: &Value, conditions: &[ResourceTargetCondition]) -> Option<StoredGoal> {
    let id = target.get("id")?.as_str()?;
    let pool = target.get("resource")?.as_str()?;
    let condition = target.get("condition")?;
    if condition.get("kind")?.as_str()? != AT_LEAST {
        return None;
    }
    let amount = condition.get("amount")?.as_u64()?;
    let progress = match conditions.iter().find(|condition| condition.target_id == id) {
        Some(condition) => match &condition.state {
            ResourceTargetConditionState::Computed { current, observed_at_unix_ms, gap } => {
                Progress::Computed { current: *current, at_ms: *observed_at_unix_ms, gap: *gap }
            }
            ResourceTargetConditionState::AwaitingObservation { reason } => {
                Progress::Awaiting(pending(*reason))
            }
        },
        None => Progress::NoCondition,
    };
    Some(StoredGoal { id: id.to_owned(), pool: pool.to_owned(), amount, progress })
}

/// Whether the pack gives a pool both a step and an importance, so that a goal on it needs no
/// number of its own.
fn stepped(resource: &TargetableResource) -> bool {
    resource.default_scale.is_some()
        && resource.default_importance_milli.is_some()
        && !resource.scale_required
        && !resource.importance_required
}

fn form_of(schema_version: &str) -> Form {
    match schema_version {
        SCHEMA_V2 => Form::Current,
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

fn closed(reason: ResourceTargetsRejectionReason) -> Closed {
    match reason {
        ResourceTargetsRejectionReason::UnmappedTask => Closed::UnmappedTask,
        ResourceTargetsRejectionReason::ResourceNotObservable => Closed::NotObservable,
        ResourceTargetsRejectionReason::ResourceOutOfScope => Closed::OutOfScope,
        other => Closed::Other(spelled(other)),
    }
}

/// The client's code for a receipt it cannot decode (Runtime `crates/runtime-client/src/codes.rs:19`
/// at 803fdfa3, `runtime_receipt_decode_failed`). The view is decoded with the contract's strict
/// types inside the receipt (`crates/runtime-client/src/client.rs:1941-1958`; the view, a
/// condition, its state and a row all `deny_unknown_fields`:
/// `crates/actingcommand-contract/src/resource_targets.rs:48`, `:57`, `:120`, `:213`), so a view
/// of a shape newer than this console's client fails there, before any of it reaches [`page`].
pub const RECEIPT_DECODE_FAILED: &str = "runtime_receipt_decode_failed";

/// Whether a failed `ResourceTargetView` read, with the client's own `code`, is most likely an
/// answer newer than this console: the client could not decode the receipt
/// ([`RECEIPT_DECODE_FAILED`]). The Goals tab then says so, with the codes, instead of a bare
/// read failure; a damaged answer reads the same, which is why it says "most likely".
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

#[cfg(test)]
mod tests {
    use super::{
        newer_than_this_console, page, Closed, Document, Form, GoalRow, Goals, Pending, PoolRow,
        Progress, Reading, Stored, StoredGoal, RECEIPT_DECODE_FAILED,
    };

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
    use actingcommand_contract::ResourceTargetView;
    use serde_json::{json, Value};

    const EVALUATED: u64 = 1_794_532_592_000;
    const OBSERVED: u64 = 1_794_532_500_000;
    const VALID_UNTIL: u64 = 1_826_068_592_000;
    const V1: &str = "actingcommand.resource-targets.v1";
    const V2: &str = "actingcommand.resource-targets.v2";

    /// A view as the wire carries it, read through the contract's own type, so that every member
    /// named here is one the pinned contract has.
    fn view(targetable: Value, not_targetable: Value, active: Option<Value>) -> ResourceTargetView {
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
        serde_json::from_value(view).expect("a view the pinned contract reads")
    }

    /// A targetable pool; with a step, the pack gives it both defaults; with a reading, its amount.
    fn pool(id: &str, step: bool, current: Option<i64>) -> Value {
        let observation = match current {
            Some(current) => json!({"current": current, "observed_at_unix_ms": OBSERVED}),
            None => json!({"pending": "missing"}),
        };
        let mut pool = json!({
            "resource": id,
            "fact_key": format!("inventory.{id}"),
            "producing_tasks": ["task.daily"],
            "scale_required": !step,
            "importance_required": !step,
            "observation": observation,
        });
        if step {
            pool["default_scale"] = json!(100);
            pool["default_importance_milli"] = json!(500);
        }
        pool
    }

    fn closed_pool(id: &str, reason: &str) -> Value {
        json!({"resource": id, "reason": reason})
    }

    fn active(schema: &str, targets: Value, conditions: Value, expired: bool) -> Value {
        json!({
            "policy_sha256": format!("sha256:{}", "b".repeat(64)),
            "schema_version": schema,
            "version": 41,
            "event_id": format!("evt_{}", "1".repeat(32)),
            "valid_until_unix_ms": VALID_UNTIL,
            "expired": expired,
            "targets": targets,
            "conditions": conditions,
        })
    }

    fn target(id: &str, pool: &str, amount: u64) -> Value {
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

    fn goal(id: &str, pool: &str, amount: u64, progress: Progress) -> StoredGoal {
        StoredGoal { id: id.to_owned(), pool: pool.to_owned(), amount, progress }
    }

    fn document(form: Form, expired: bool, goals: usize) -> Stored {
        Stored::Document(Document {
            form,
            version: 41,
            valid_until_ms: Some(VALID_UNTIL),
            expired,
            goals,
        })
    }

    #[test]
    fn a_goal_row_has_the_reading_the_amount_and_the_gap() {
        let view = view(
            json!([pool("pool.gold", true, Some(1200))]),
            json!([]),
            Some(active(
                V2,
                json!([target("gold", "pool.gold", 5000)]),
                json!([computed("gold", "pool.gold", 1200, 3800)]),
                false,
            )),
        );
        let page = page(&view);
        let progress = Progress::Computed { current: 1200, at_ms: OBSERVED, gap: 3800 };
        assert_eq!(
            page.rows,
            vec![GoalRow::Goal(PoolRow {
                pool: "pool.gold".to_owned(),
                reading: Reading::Read { current: 1200, at_ms: OBSERVED },
                goals: Goals::Set(vec![goal("gold", "pool.gold", 5000, progress)]),
            })]
        );
        assert!(!progress.met());
        assert_eq!(page.stored, document(Form::Current, false, 1));
        assert_eq!(page.evaluated_at_ms, EVALUATED);
        assert!(!page.no_catalog);
        assert!(!page.stale_schema);
    }

    #[test]
    fn a_gap_of_zero_is_met_and_a_pending_reading_says_why() {
        let view = view(
            json!([pool("pool.gold", true, Some(6000)), pool("pool.wood", true, None)]),
            json!([]),
            Some(active(
                V2,
                json!([target("gold", "pool.gold", 5000), target("wood", "pool.wood", 80)]),
                json!([
                    computed("gold", "pool.gold", 6000, 0),
                    awaiting("wood", "pool.wood", "low_confidence"),
                ]),
                false,
            )),
        );
        let page = page(&view);
        let met = Progress::Computed { current: 6000, at_ms: OBSERVED, gap: 0 };
        assert!(met.met());
        let awaiting = Progress::Awaiting(Pending::LowConfidence);
        assert!(!awaiting.met());
        assert_eq!(
            page.rows,
            vec![
                GoalRow::Goal(PoolRow {
                    pool: "pool.gold".to_owned(),
                    reading: Reading::Read { current: 6000, at_ms: OBSERVED },
                    goals: Goals::Set(vec![goal("gold", "pool.gold", 5000, met)]),
                }),
                GoalRow::Goal(PoolRow {
                    pool: "pool.wood".to_owned(),
                    reading: Reading::Pending(Some(Pending::Missing)),
                    goals: Goals::Set(vec![goal("wood", "pool.wood", 80, awaiting)]),
                }),
            ]
        );
    }

    #[test]
    fn every_pending_reason_is_read() {
        for (spelling, reason) in [
            ("missing", Pending::Missing),
            ("expired", Pending::Expired),
            ("low_confidence", Pending::LowConfidence),
            ("invalid_value", Pending::InvalidValue),
        ] {
            let view = view(
                json!([pool("pool.gold", true, Some(10))]),
                json!([]),
                Some(active(
                    V2,
                    json!([target("gold", "pool.gold", 50)]),
                    json!([awaiting("gold", "pool.gold", spelling)]),
                    false,
                )),
            );
            let expected = Goals::Set(vec![goal("gold", "pool.gold", 50, Progress::Awaiting(reason))]);
            match &page(&view).rows[..] {
                [GoalRow::Goal(row)] => assert_eq!(row.goals, expected, "{spelling}"),
                rows => panic!("{spelling}: {rows:?}"),
            }
        }
    }

    #[test]
    fn rows_go_by_kind_top_to_bottom_each_in_the_runtimes_order() {
        let view = view(
            json!([
                pool("pool.stamina", false, Some(90)),
                pool("pool.gold", true, Some(1)),
                pool("pool.wood", true, Some(2)),
            ]),
            json!([
                closed_pool("pool.gems", "unmapped_task"),
                closed_pool("pool.keys", "resource_not_observable"),
                closed_pool("pool.tokens", "resource_out_of_scope"),
                closed_pool("pool.relics", "unknown_resource"),
            ]),
            None,
        );
        let page = page(&view);
        let open = |pool: &str, current: i64| PoolRow {
            pool: pool.to_owned(),
            reading: Reading::Read { current, at_ms: OBSERVED },
            goals: Goals::Set(Vec::new()),
        };
        let closed = |pool: &str, why: Closed| GoalRow::Closed { pool: pool.to_owned(), why };
        assert_eq!(
            page.rows,
            vec![
                GoalRow::Goal(open("pool.gold", 1)),
                GoalRow::Goal(open("pool.wood", 2)),
                GoalRow::NoStep(open("pool.stamina", 90)),
                closed("pool.gems", Closed::UnmappedTask),
                closed("pool.keys", Closed::NotObservable),
                closed("pool.tokens", Closed::OutOfScope),
                closed("pool.relics", Closed::Other("unknown_resource".to_owned())),
            ]
        );
        assert_eq!(page.stored, Stored::None);
    }

    #[test]
    fn a_pool_without_a_step_keeps_a_goal_an_agent_set_on_it() {
        let view = view(
            json!([pool("pool.stamina", false, Some(90))]),
            json!([]),
            Some(active(
                V2,
                json!([target("stamina", "pool.stamina", 120)]),
                json!([computed("stamina", "pool.stamina", 90, 30)]),
                false,
            )),
        );
        let progress = Progress::Computed { current: 90, at_ms: OBSERVED, gap: 30 };
        assert_eq!(
            page(&view).rows,
            vec![GoalRow::NoStep(PoolRow {
                pool: "pool.stamina".to_owned(),
                reading: Reading::Read { current: 90, at_ms: OBSERVED },
                goals: Goals::Set(vec![goal("stamina", "pool.stamina", 120, progress)]),
            })]
        );
    }

    #[test]
    fn a_goal_whose_pool_is_not_offered_any_more_says_so_and_why() {
        let view = view(
            json!([pool("pool.gold", true, Some(1))]),
            json!([closed_pool("pool.gems", "unmapped_task"), closed_pool("pool.keys", "unmapped_task")]),
            Some(active(
                V2,
                json!([
                    target("gone", "pool.retired", 7),
                    target("gems", "pool.gems", 300),
                ]),
                json!([computed("gems", "pool.gems", 120, 180)]),
                false,
            )),
        );
        let gems = Progress::Computed { current: 120, at_ms: OBSERVED, gap: 180 };
        assert_eq!(
            page(&view).rows,
            vec![
                GoalRow::Goal(PoolRow {
                    pool: "pool.gold".to_owned(),
                    reading: Reading::Read { current: 1, at_ms: OBSERVED },
                    goals: Goals::Set(Vec::new()),
                }),
                // pool.gems has its goal's row instead.
                GoalRow::Closed { pool: "pool.keys".to_owned(), why: Closed::UnmappedTask },
                GoalRow::NoLongerOffered {
                    goal: goal("gone", "pool.retired", 7, Progress::NoCondition),
                    why: None,
                },
                GoalRow::NoLongerOffered {
                    goal: goal("gems", "pool.gems", 300, gems),
                    why: Some(Closed::UnmappedTask),
                },
            ]
        );
    }

    #[test]
    fn an_old_form_is_shown_read_only_with_every_goal_of_a_pool() {
        let old_target = |id: &str, amount: u64| {
            json!({
                "id": id,
                "resource": "pool.gold",
                "condition": {"kind": "at_least", "amount": amount},
                "scale": 100,
                "importance_milli": 500,
                "rule": "shortfall_linear",
                "apply": {"mode": "override", "weight": "score_stage"},
                "tasks": ["task.daily"],
            })
        };
        let view = view(
            json!([pool("pool.gold", true, Some(40))]),
            json!([]),
            Some(active(
                V1,
                json!([old_target("first", 50), old_target("second", 30)]),
                json!([computed("first", "pool.gold", 40, 10), computed("second", "pool.gold", 40, 0)]),
                false,
            )),
        );
        let page = page(&view);
        let first = Progress::Computed { current: 40, at_ms: OBSERVED, gap: 10 };
        let second = Progress::Computed { current: 40, at_ms: OBSERVED, gap: 0 };
        assert_eq!(
            page.rows,
            vec![GoalRow::Goal(PoolRow {
                pool: "pool.gold".to_owned(),
                reading: Reading::Read { current: 40, at_ms: OBSERVED },
                goals: Goals::Set(vec![
                    goal("first", "pool.gold", 50, first),
                    goal("second", "pool.gold", 30, second),
                ]),
            })]
        );
        assert_eq!(page.stored, document(Form::Old, false, 2));
    }

    #[test]
    fn a_newer_form_is_not_interpreted_and_says_so() {
        let schema = "actingcommand.resource-targets.v3";
        let view = view(
            json!([pool("pool.gold", true, Some(40))]),
            json!([closed_pool("pool.gems", "unmapped_task")]),
            Some(active(
                schema,
                json!([target("gems", "pool.gems", 300), {"name": "route", "value": "a"}]),
                json!([]),
                false,
            )),
        );
        let page = page(&view);
        assert_eq!(
            page.rows,
            vec![
                GoalRow::Goal(PoolRow {
                    pool: "pool.gold".to_owned(),
                    reading: Reading::Read { current: 40, at_ms: OBSERVED },
                    goals: Goals::Unknown,
                }),
                GoalRow::Closed { pool: "pool.gems".to_owned(), why: Closed::UnmappedTask },
            ]
        );
        assert_eq!(page.stored, document(Form::Newer(schema.to_owned()), false, 2));
    }

    #[test]
    fn a_target_that_does_not_read_is_a_row_of_its_own() {
        let view = view(
            json!([pool("pool.gold", true, Some(40))]),
            json!([]),
            Some(active(
                V2,
                json!([
                    target("gold", "pool.gold", 50),
                    {"id": "odd", "resource": "pool.gold", "condition": {"kind": "at_most", "amount": 5}},
                    {"id": "short"},
                ]),
                json!([computed("gold", "pool.gold", 40, 10)]),
                false,
            )),
        );
        let progress = Progress::Computed { current: 40, at_ms: OBSERVED, gap: 10 };
        assert_eq!(
            page(&view).rows,
            vec![
                GoalRow::Goal(PoolRow {
                    pool: "pool.gold".to_owned(),
                    reading: Reading::Read { current: 40, at_ms: OBSERVED },
                    goals: Goals::Set(vec![goal("gold", "pool.gold", 50, progress)]),
                }),
                GoalRow::Unread { position: 1 },
                GoalRow::Unread { position: 2 },
            ]
        );
    }

    #[test]
    fn an_expired_document_is_said_expired() {
        let view = view(
            json!([pool("pool.gold", true, Some(40))]),
            json!([]),
            Some(active(
                V2,
                json!([target("gold", "pool.gold", 50)]),
                json!([computed("gold", "pool.gold", 40, 10)]),
                true,
            )),
        );
        assert_eq!(page(&view).stored, document(Form::Current, true, 1));
    }

    #[test]
    fn a_validity_not_stated_is_kept_as_not_stated() {
        let mut active = active(
            V2,
            json!([target("gold", "pool.gold", 50)]),
            json!([computed("gold", "pool.gold", 40, 10)]),
            false,
        );
        active.as_object_mut().expect("an object").remove("valid_until_unix_ms");
        let view = view(json!([pool("pool.gold", true, Some(40))]), json!([]), Some(active));
        match page(&view).stored {
            Stored::Document(document) => assert_eq!(document.valid_until_ms, None),
            Stored::None => panic!("a stored document"),
        }
    }

    #[test]
    fn without_a_catalog_every_stored_goal_is_no_longer_offered() {
        let mut view = view(
            json!([]),
            json!([]),
            Some(active(V2, json!([target("gold", "pool.gold", 50)]), json!([]), false)),
        );
        view.catalog_hash = None;
        let page = page(&view);
        assert!(page.no_catalog);
        assert_eq!(
            page.rows,
            vec![GoalRow::NoLongerOffered {
                goal: goal("gold", "pool.gold", 50, Progress::NoCondition),
                why: None,
            }]
        );
    }

    #[test]
    fn a_stale_schema_mark_is_kept() {
        let mut view = view(json!([pool("pool.gold", true, None)]), json!([]), None);
        view.stale_schema = Some(true);
        let page = page(&view);
        assert!(page.stale_schema);
        assert_eq!(page.stored, Stored::None);
        view.stale_schema = Some(false);
        assert!(!super::page(&view).stale_schema);
    }
}
