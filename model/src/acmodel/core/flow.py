"""The economy: resource flows and constraints as linear programs, solved with scipy's HiGHS (#424 design 4.3
and 4.4, design-py 4.3; contracts/formats.md, "The economy").

Two nodes of the derivation graph (`core.graph`), each computed after what it reads and kept in the cache:
- ("economy", "steady"): the values, game-wide. A cyclic steady state over the least common multiple of the
  periods' lengths within `economy.max_cycle_steps` (a longer period and a month relaxed by rate and listed in
  `relaxed`, #424 ACM-PRICE PRICE-3), built
  from the rules and the data only, in the neutral reference state (the graph's "reference" nodes: a cap with a
  reading counts at its `reference`, #424 ACM-PRICE G-A and ruling 126 A), and solved as LPs. Value comes only
  through the goal sinks of the objective rules (PRICE-2a, below). It gives the optimum per cycle; the price
  of every resource in every step, the projection of its balance row's dual onto the dual's optimal face D*
  (below, PRICE-2b-1), exact when both ends agree and an interval otherwise; the price of every cap and limit,
  by re-solves at plus and minus one grain (`lp.grain`); the value of each activity at those prices; the joint
  price vector and the activity judgements (below); and the least overflow and lost production among the
  optimal solutions. The re-solves of the resources' prices at plus and minus one grain (`grain`) and HiGHS's
  marginals are only cross-checks (`lp_dual_disagrees`; a marginal agrees with an exact price within the relative
  tolerance `MARGINAL_TOLERANCE`, 1e-7). A pending flow whose amounts read unknowns is left out and listed
  (`excluded_pending`; the ACM-PRICE addendum, part 1). Each price has the status of its static component, and the
  coverage section classes each pool (`price_status`, `coverage`: `core.coverage`, PRICE-2b-2).
- ("economy", "plan"): one account's plan. From the state's readings over `economy.plan_horizon` steps, with
  integer counts and an indicator per read cap and step, solved as a MILP within `search.milp_node_limit`
  nodes and never a time limit; the final stocks are priced by the steady state. A plan the node limit stops is
  `search_budget_exhausted`, with `proven: false`.

**The program** (generated from the rules, never written by hand). One step is one day; a steady cycle is the
least common multiple of the periods' lengths in days (day 1, week 7, a `days` period its `length`), each taken
in order of length while the multiple stays within `economy.max_cycle_steps`. A period that is not taken, and a
month, is relaxed: what counts per period counts over the whole program, at its value times the program's days
over the period's (a month is 146097/4800 days). The plan relaxes a `days` period and a month too. Per activity
and step a count; per resource and store a
stock at every step boundary; per hard-capped resource an overflow (income the cap blocks; for a void_at_reset
cap only in the step before each reset of its period); per income cap and step what it voids; per store with a
storage cap, and per resource whose regen
stops at a cap (steady), its lost production; per move whose amount needs the account state (a collection that
takes what the store holds) the amount it moves, at most the store's storage cap per execution. Rows, in
canonical order (by kind, id and step): the balances, the bounds of overflow by income, the cyclic rows (steady),
the caps, the demand rows' bounds (steady), the floors of the tier solves (steady: A ≥ A*, V ≥ V*), the income caps
per window, the limits per window, the moves, the probes of a tie (steady), the regen stops (steady), the read caps'
indicators (plan), the opening stocks (plan) and the bounds of what an income cap voids by the income it counts.
Columns are sorted by variable id.

**The objective** (#424 ACM-PRICE 2.2 and addendum 2.1 §4; PRICE-2a). The numeraire is a `value` entity, which
holds no stock: what counts is what the goal sinks bring per cycle. Per step a column per main objective (it
takes one unit of its resource for `value` value units) and per row of the secondary objective's demand table (it
takes the row's costs, from the costs table by key, for `points × value + main` value units, at most `count ×
cycle days ÷ horizon days` a cycle). Two functions are defined once each and used for both the solves and the
checks: the value V (`total_value`, what the sinks bring) and the protected acquisition A (`acquisition`: what the
protected acquisitions P bring of the main goals at their values, less what an income cap that counts one of them
voids of it, and the `main` of each demand row). An activity is in P (`acquirers`) when it brings in a main goal
item, a limit or an income cap counts it, and each of its inputs (a non-zero amount it consumes, a store or resource
it moves out of) is time: a resource or store with a regen that is not a slot, or a slot that only acquisitions use;
another acquisition is a conversion, compared inside V. Three solves:
- S3, V alone: its optimum V0 is the objective, and its program is the one every price re-solve changes;
- S1, A alone: its optimum A* (none when A has no term: A* is 0 and S2 is S3);
- S2, V with A ≥ A*: its optimum V*. V0 > V* is `objective_tier_conflict`, which blocks the steady state;
  an activity that some optimum of S3 uses and no optimum of S2 does is `objective_tier_tie` (a warning).
All of them compare exact fractions; a row A ≥ A* or V ≥ V* is checked exactly like every other row. An activity
whose dual slack at the exact optimal point of S3's dual is above 0 runs in no optimum (complementary slackness,
`_never`; PRICE-2c), so it needs no tie probe and no probe of `most`; a probe that cannot be verified exactly is
`lp_probe_unverified` (a warning, its activity a structural gap) and never blocks the steady state.

**The joint price vector** (#424 ACM-PRICE addendum 2.1 §2.2, ruling 127 iii; PRICE-2b-1). The exported prices are
one point of D*, never a pick per pool. Steps:
1. the dual of S3's program, generated from its rows (`_dual`): a variable per row (free for an equality, as the
   difference of two non-negative ones), one per variable with an upper bound, a row per column; D* adds the
   strong-duality row, the dual's cost at V0 (`FACE`). That HiGHS's least cost is V0 is checked exactly
   (`duality`), and so is every solution on the dual against every dual row and the face
   (`lp_numeric_check_failed`, stage `dual`, which blocks the steady state: no price without the whole vector);
2. the projection: a pool's coordinate is the mean over the cycle's steps of its balance rows' duals, and its
   interval `[low, high]` its least and greatest value on D*, an unbounded end null;
3. the selection (`joint`), each step an exact LP: (1) a pool bounded below only that a main goal values is fixed
   at its least value (the value of one more), by id; (2) the lexicographic max-min over K: the pools of a bounded
   interval and the activities that no optimum runs and whose value reaches 0 somewhere on D* (`max_min`); (3) a
   pool whose ends agree has its one value; and last, each pool the selection withholds (bounded below only and no
   main goal: `no_steady_supply`; without a low end: `no_room`), by id, at its low end on the face left, else its
   high end, else 0. The vector is then checked exactly as a point of D*;
4. the activity judgements: an activity's value at the vector and its range on D*, the most an optimum of S3 runs
   it (`in_optimum` when above 0), and the loss per execution of running it more (`forcing_cost`, from the dual of
   S3 with that most as a floor; `input_bound` when that dual is unbounded: nothing is left to run it again).

**The re-solves at ±grain** (PRICE-2d, `_Solver.regrain`). One grain more or less of a resource's income moves only right
sides of S3's program, so each re-solve starts from S3: S3's basis proves it where it stays primal feasible
(`exact.shifted`, `exact.ranging`); else HiGHS's instance of S3 gets only the moved row ends and goes on from S3's
basis, and its vertex is solved by updating S3's factorised basis (`exact.reoptimum`); else, as before, a new LP.

**Exactness.** An LP's solution is the vertex of HiGHS's basis (its column and row statuses), solved exactly and made
exactly optimal with at most `lp.max_pivots` exact pivots (`core.exact`, PRICE-2c): a certified optimum. Where that
fails, and for a MILP, the variables at a bound are fixed and the rows they leave tight solved exactly, else each value
is converted with `Fraction(x).limit_denominator(lp.max_denominator)`. Every solution is checked exactly against every
bound, integrality and row (`lp_numeric_check_failed`). Objective values, prices and losses are computed from the
checked fractions, never from the solver's floats.

**Infeasibility.** An infeasible program names a minimal set of conflicting rule groups (`lp_infeasible`): each
group (a regen, cap, limit or read cap, and in a plan each opening stock) is dropped in turn, in id order, and
stays dropped while the rest is still infeasible.

**Refusals.** What this version does not model is `expr_eval_failed` (`evaluation_unsupported`, operator
`economy`), never approximated; `found` names it.
"""

from __future__ import annotations

import math
import time
from dataclasses import dataclass, field, replace
from fractions import Fraction
from typing import TYPE_CHECKING, Any, Callable

from acmodel.check.checker import Checker, Entry
from acmodel.check.walk import at
from acmodel.codes import (
    EXPR_EVAL_FAILED,
    LP_DUAL_DISAGREES,
    LP_INFEASIBLE,
    LP_NUMERIC_CHECK_FAILED,
    LP_PROBE_UNVERIFIED,
    LP_SOLVE_FAILED,
    OBJECTIVE_TIER_CONFLICT,
    OBJECTIVE_TIER_TIE,
    RULE_PENDING_UNBOUNDED,
    SEARCH_BUDGET_EXHAUSTED,
    STATE_READING_MISSING,
)
from acmodel.config import EconomySettings
from acmodel.core import coverage, exact
from acmodel.core.common import CONFIRMED, PROVISIONAL, RULE_STATUS, Need, worst
from acmodel.core.unknown import read as unknowns_read, rule_status
from acmodel.envelope import Failure, error
from acmodel.expr import Binary, Literal, Reference
from acmodel.expr.interval import Interval, Undecided

if TYPE_CHECKING:
    from acmodel.core.graph import Graph, Scenario

STEP_SECONDS = 86400  # one step is one day, the shortest period unit
WEEK_STEPS = 7
# A month's days: the mean of the Gregorian calendar's 400 years (#424 ACM-PRICE 3.3), exact.
MONTH_DAYS = Fraction(146097, 4800)
STEADY, PLAN = "steady", "plan"
NODES = {STEADY: ("economy", STEADY), PLAN: ("economy", PLAN)}
OUTPUTS = {STEADY: "economy:steady", PLAN: "economy:plan"}
# The rule kinds the economy reads, and of them the kinds it does not model yet. A pool's declared zero (`no_use`)
# is read by the coverage section only (`core.coverage`, PRICE-2b-2).
RULE_KINDS = ("cap", "cooldown", "exchange", "flow", "gate", "limit", "no_use", "objective", "regen")
NOT_MODELLED = ("cooldown", "exchange")
MAIN = "main"  # an objective's tier: a main goal; the other is `secondary`, the reference demand
DEMAND_COLUMNS = ("count", "points", "main")  # a demand row's columns, as the static check requires them
# The rows of the tier solves (PRICE-2a): A ≥ A* holds S2 and the faces of S2; V ≥ V* the faces of an optimum.
ACQUISITION, VALUE = "acquisition", "value"
HARD_REACH = ("block_all_income", "discard")
VOID = "void_at_reset"  # a hard cap the steady state reads as `discard` at the resets of its `per` only
STORAGE_REACH = ("block_all_income", "discard", "stop_production")
READING_GROUP = "reading:"
# The prefix of the graph nodes each program reads first: the steady state's from the reference state, the
# plan's from the rules and the data only (and then, where they need it, the account state's).
PREFIX = {STEADY: ("reference",), PLAN: ("general",)}
# Why a price of the steady state is provisional (`price_status`): the coverage section's reasons of its static
# component (`core.coverage`, PRICE-2b-2).
# How an economy node is kept in the cache: with what the steady state read, the provenance of its outputs.
FORMAT = {"economy": ["data", "status", "per_account", "warnings", "blocked", "reads"]}
_STATE = object()  # a node from the rules and the data only that would need the account state

Key = tuple[str, str, int]
_Pair = tuple[Fraction | None, Fraction | None]  # a price: its low and high ends, None for unbounded

# The joint price vector (PRICE-2b-1; addendum 2.1 §2.2). The stages of the solves on the dual (its check, the
# projections, the selection, the ranges and the forcing costs) and of the judgements' primal solves (the most an
# optimum runs an activity).
DUAL, JUDGEMENT = "dual", "judgement"
FACE: Key = ("duality", "face", 0)  # the strong-duality row: the dual's cost at the primal optimum
LEVEL: Key = ("t", "level", 0)  # the max-min's level, a variable of its stage
PROBE: Key = ("t", "probe", 0)  # the distance of one coordinate, a variable of the probe of a stage
# The economy's graph outputs (#424 ACM-PRICE 4.1): `price:<resource>` and the diagnostic `activity_value:<activity>`,
# and the ids of the max-min's coordinates.
PRICE, ACTIVITY_VALUE = "price:", "activity_value:"
GRAPH_OUTPUTS = ("price", "activity_value")
# How a pool's coordinate is chosen (`by`): its ends agree; the max-min; its least value (the value of one more);
# its greatest (the value of one less); 0, unbounded both ways.
EXACT, MAX_MIN, ONE_MORE, ONE_LESS, ZERO = "exact", "max_min", "one_more", "one_less", "zero"
# Why the selection withholds a pool's coordinate: one less is infeasible and no main goal values it (it has no
# steady supply); one more is infeasible (no room: nothing in the cycle can take one more unit, as where a regen
# stops at a cap and the pending flows that would use it are left out).
NO_STEADY_SUPPLY, NO_ROOM = "no_steady_supply", "no_room"
# τ, how far HiGHS's marginal may lie from an exact price it cross-checks (`lp_dual_disagrees`, check `marginal`):
# the marginal m agrees with an end p of the price when |m - p| <= τ · max(1, |p|). It is HiGHS's default dual
# feasibility tolerance, 1e-7. A marginal is a float, made a fraction by `limit_denominator(lp.max_denominator)`, and
# an exact price can have a denominator far above it (a model whose numbers carry four decimals gives some near 6e28),
# so the two never agree exactly. Only this comparison of a float with an exact price has a tolerance: the check
# `grain`, of two exact fractions, and every exactness check stay exact.
MARGINAL_TOLERANCE = Fraction(1, 10**7)
# A per-activity probe that cannot be verified exactly (PRICE-2c): `lp_probe_unverified`, a warning, and its activity a
# structural gap of its static component (`core.coverage`). The probes, by the judgement they answer: a tie
# (`objective_tier_tie`), the most an optimum runs it, its value's range on D*, its forcing cost. Why: HiGHS left no
# valid basis (`no_basis`), its basis gave no certified vertex (an `exact` reason), or the probe's LP failed.
TIE, MOST, RANGE, FORCING = "tie", "most", "range", "forcing_cost"
NO_BASIS, SOLVE_FAILED = "no_basis", "solve_failed"
# HiGHS's simplex strategies (`SimplexStrategy`): the dual simplex, which `linprog` sets for every solve, and the primal
# simplex, which goes on from a feasible basis when only the objective changes (`_highs_again`, PRICE-2c).
DUAL_SIMPLEX, PRIMAL_SIMPLEX = 1, 4


@dataclass(frozen=True)
class Economy:
    """The value of an economy node: its output as `economy` shows it, or the errors that block it; and what it
    read, the graph nodes, data rows and gates its outputs rest on (the provenance of `price:` and
    `activity_value:`, PRICE-2b-1)."""

    data: dict[str, Any] | None = None
    status: str = CONFIRMED
    per_account: bool = False
    warnings: tuple[dict[str, Any], ...] = ()
    blocked_by: frozenset[int] = frozenset()
    reads: tuple[tuple[str, ...], ...] = ()

    @property
    def blocked(self) -> bool:
        return bool(self.blocked_by)


def spec_keys(settings: EconomySettings, solve: str) -> dict[str, int]:
    """The configuration keys a node's value depends on, for its spec in the cache."""
    keys = {"max_denominator": settings.max_denominator, "grain": settings.grain}
    if solve == STEADY:
        keys["max_cycle_steps"] = settings.max_cycle_steps
        keys["max_pivots"] = settings.max_pivots  # PRICE-2c: which probes are verified
    if solve == PLAN:
        keys.update(node_limit=settings.node_limit, plan_horizon=settings.plan_horizon)
    return keys


def model_leaf(checker: Checker) -> dict[str, Any]:
    """The leaf ("economy_model",): everything about the model's structure the economy reads: the periods, the
    numeraire, the entities and the rules of the kinds it reads, with their files and pointers."""
    manifest = checker.model.manifest

    def listed(entries: Any) -> list[list[Any]]:
        ordered = sorted(entries, key=lambda entry: entry.id)
        return [[entry.id, entry.kind, entry.path, entry.ref, entry.doc] for entry in ordered]

    return {
        "path": manifest.path,
        "periods": manifest.doc["periods"],
        "numeraire": manifest.doc["numeraire"],
        "entities": listed(e for e in checker.entities if checker.entries.get(e.id) is e),
        "rules": listed(r for r in checker.rules if checker.active(r) and r.kind in RULE_KINDS),
    }


def encode(result: Economy, blocked: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "data": result.data, "status": result.status, "per_account": result.per_account,
        "warnings": list(result.warnings), "blocked": blocked, "reads": [list(item) for item in result.reads],
    }


def decode(doc: dict[str, Any], register: Callable[[list[dict[str, Any]]], frozenset[int]]) -> Economy:
    return Economy(
        doc["data"], doc["status"], doc["per_account"], tuple(doc["warnings"]), register(doc["blocked"]),
        tuple(tuple(item) for item in doc["reads"]),
    )


def render(result: Economy) -> dict[str, Any]:
    """An economy output as `economy` shows it."""
    return {"status": "blocked"} if result.blocked else result.data


def output(result: Economy, name: str) -> dict[str, Any] | None:
    """The economy's graph output `price:<resource>` or `activity_value:<activity>` (#424 ACM-PRICE 4.1; PRICE-2b-1)
    as `explain` shows it, from a steady state that is not blocked: its `status`, `per_account` (false: the steady
    state is game-wide) and its entry of `joint`; a price adds the `reasons` of its status (`price_status`). An
    activity's value is a diagnosis only: nothing binds or exports it. None when the steady state has no such
    output."""
    kind, target = name.split(":", 1)
    joint = result.data["joint"]
    if kind == "price":
        found = joint["prices"].get(target)
        if found is None:
            return None
        status = result.data["price_status"][target]
        return {"status": status["status"], "reasons": status["reasons"], "per_account": False, **found}
    found = joint["activities"].get(target)
    return None if found is None else {"status": result.status, "per_account": False, **found}


def _refused(path: str, ref: str, found: str) -> dict[str, Any]:
    detail = {"path": path, "reason": "evaluation_unsupported", "operator": "economy", "position": 0, "found": found}
    return error(EXPR_EVAL_FAILED, detail, ref)


def _token(key: tuple[Any, ...]) -> str:
    return ":".join(str(part) for part in key)


def _text(number: Fraction | None) -> str | None:
    return None if number is None else str(number)


def _price(pair: tuple[Fraction | None, Fraction | None]) -> Any:
    """An exact price or value as its text; an interval as `{low, high}`, null for an unbounded end."""
    low, high = pair
    if low is not None and low == high:
        return str(low)
    return {"low": _text(low), "high": _text(high)}


def _low(shown: Any) -> Fraction:
    """The low end of a re-solved price (`grain`) as `_price` shows it: the value of one more unit of income. It is
    null only where the steady state cannot hold one more unit, at a regen stop, which the plan refuses (`while`)
    before it reads a price."""
    low = shown if isinstance(shown, str) else shown["low"]
    if low is None:
        raise ValueError("the plan read a steady-state price without a low end")
    return Fraction(low)


def _near(marginal: Fraction, end: Fraction) -> bool:
    """Whether HiGHS's marginal agrees with an exact end of a price within `MARGINAL_TOLERANCE`, relative to the end
    and at least absolute: |m - p| <= τ · max(1, |p|), computed exactly."""
    return abs(marginal - end) <= MARGINAL_TOLERANCE * max(Fraction(1), abs(end))


def _add(table: dict[Any, Fraction], key: Any, amount: Fraction) -> None:
    table[key] = table.get(key, Fraction(0)) + amount


def _nonzero(coefficients: dict[Key, Fraction]) -> dict[Key, Fraction]:
    return {key: value for key, value in coefficients.items() if value != 0}


# The structure ------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _Part:
    """One amount of a flow: what one execution of its activity consumes, produces or moves."""

    flow: str
    path: str
    activity: str
    member: str  # consumes, produces or moves
    resource: str | None  # consumes, produces
    source: str | None  # moves
    target: str | None  # moves
    ref: str  # the pointer of its amount

    @property
    def key(self) -> tuple[str, ...]:
        if self.member == "moves":
            return ("flow", self.flow, "moves")
        return ("flow", self.flow, self.member, self.resource)


@dataclass
class _Shape:
    """What the economy reads of the model's structure, before any value; and what it refuses."""

    path: str  # model.json
    cycle: int  # steps
    windows: dict[str, int]  # period id -> the steps of one of its windows
    resources: list[str]
    stores: dict[str, str]  # store -> the resource it holds
    goals: list[Entry]  # the main objectives, by id
    secondary: Entry | None  # the secondary objective (at most one: its subject is the numeraire)
    activities: list[str]
    excluded: list[str]  # activities behind a gate of origin person (#424 design 4.7)
    parts: list[_Part]
    regens: list[Entry]
    stops: dict[str, tuple[Entry, Entry]]  # steady: resource -> (its regen, the cap its regen stops at)
    hard: dict[str, Entry]  # resource -> its hard cap
    void: dict[str, int]  # steady: resource -> the steps of the period at whose resets its void_at_reset cap voids
    storage: dict[str, Entry]  # store -> its storage cap
    soft: list[Entry]  # read caps that block incomes
    limits: list[tuple[Entry, tuple[str, ...]]]  # a limit and the activities it counts
    incomes: list[tuple[Entry, tuple[str, ...]]]  # an income cap and the activities whose income it counts
    relaxed: dict[str, Fraction]  # period id -> its days, for the periods the program relaxes by rate
    gates: list[tuple[str, Entry]]  # (flow, a gate it requires)
    refused: list[dict[str, Any]]


def _stops_production(checker: Checker, graph: Graph, rule: Entry) -> bool:
    """A regen's `while` that says what the store's storage cap already does: production stops at the cap
    (`stock:<store> < cap:<its storage cap>`)."""
    store = checker.entries[rule.doc["subject"]]
    cap = checker.storage_cap(store.id) if store.kind == "store" else None
    if cap is None or cap.doc["logic"]["on_reach"] not in STORAGE_REACH:
        return False
    root = graph.typed.expressions[(rule.path, at(rule.ref, "logic", "while"))].root
    return (
        isinstance(root, Binary) and root.op == "<"
        and isinstance(root.left, Reference) and (root.left.kind, root.left.target) == ("stock", store.id)
        and isinstance(root.right, Reference) and root.right.kind in ("cap", "rule") and root.right.target == cap.id
    )


def _stops_at(checker: Checker, graph: Graph, rule: Entry) -> Entry | None:
    """The cap at which a resource's regen stops (#424 ACM-PRICE G-B): its `while` is `amount:<resource> <
    cap:<cap>` (or `rule:<cap>`) with a cap of that resource that has a `value` or a `reference`, or `amount:<resource>
    < read:<fact>` with the one cap of that resource whose reading the fact is and that has a `reference`. None for
    any other `while`."""
    resource = rule.doc["subject"]
    root = graph.typed.expressions[(rule.path, at(rule.ref, "logic", "while"))].root
    if not (
        isinstance(root, Binary) and root.op == "<"
        and isinstance(root.left, Reference) and (root.left.kind, root.left.target) == ("amount", resource)
        and isinstance(root.right, Reference)
    ):
        return None
    right = root.right
    caps = [
        cap for cap in checker.rules if checker.active(cap) and cap.kind == "cap" and cap.doc["subject"] == resource
    ]

    def has(cap: Entry, *members: str) -> bool:
        return any(member in cap.doc["logic"] for member in members)

    if right.kind in ("cap", "rule"):
        found = [cap for cap in caps if cap.id == right.target and has(cap, "value", "reference")]
    elif right.kind == "read":
        found = [cap for cap in caps if cap.doc["logic"].get("reading") == right.target and has(cap, "reference")]
    else:
        return None
    return found[0] if len(found) == 1 else None


def _periods(
    periods: list[dict[str, Any]], steady: bool, most: int
) -> tuple[int, dict[str, int], dict[str, Fraction]]:
    """The steady cycle (#424 ACM-PRICE 3.4), the periods read as they are (period id -> the steps of one
    window), and those relaxed by rate (period id -> its days).

    The steady cycle is the least common multiple of the periods of a fixed length (day 1, week 7, `days` its
    `length`), taken shortest first (then by id) while the multiple stays within `economy.max_cycle_steps`; the
    others are relaxed, and so is a month, which has no fixed length (146097/4800 days, 3.3). The plan starts at a
    reset of every day and week period; it has no start date to place a `days` period's `anchor_date` or a month's
    reset, so it relaxes them (the steady cycle is not its own)."""
    fixed: dict[str, int] = {}
    relaxed: dict[str, Fraction] = {}
    for period in periods:
        unit = period["unit"]
        if unit == "month" or (unit == "days" and not steady):
            relaxed[period["id"]] = MONTH_DAYS if unit == "month" else Fraction(period["length"])
        else:
            fixed[period["id"]] = period["length"] if unit == "days" else WEEK_STEPS if unit == "week" else 1
    if not steady:
        return 1, fixed, relaxed
    cycle, windows = 1, {}
    for period, steps in sorted(fixed.items(), key=lambda item: (item[1], item[0])):
        joined = math.lcm(cycle, steps)
        if joined <= most:
            cycle, windows[period] = joined, steps
        else:
            relaxed[period] = Fraction(steps)
    return cycle, windows, relaxed


def slots(checker: Checker, typed: Any) -> frozenset[str]:
    """The slot resources (#424 ACM-PRICE 3.1): each resource with an active hard cap whose `value` is the number
    0. Its stock never carries over a step boundary, so a slot is counted in slot-hours that are used in the step
    they come in or lost; it is no Runtime pool, and its price is a diagnosis only (it cannot be bound)."""
    found = set()
    for rule in checker.rules:
        if not (checker.active(rule) and rule.kind == "cap" and rule.doc["logic"]["cap_kind"] == "hard"):
            continue
        subject = checker.entries.get(rule.doc["subject"])
        if subject is None or subject.kind != "resource" or "value" not in rule.doc["logic"]:
            continue
        root = typed.expressions[(rule.path, at(rule.ref, "logic", "value"))].root
        if isinstance(root, Literal) and type(root.value) is Fraction and root.value == 0:
            found.add(subject.id)
    return frozenset(found)


def _brings(part: _Part, resource: str) -> bool:
    """Whether an amount of a flow brings the resource in: it produces it, or moves into it."""
    return (part.member == "produces" and part.resource == resource) or (
        part.member == "moves" and part.target == resource
    )


def _shape(graph: Graph, solve: str) -> _Shape:
    checker = graph.checker
    manifest = checker.model.manifest
    steady = solve == STEADY
    refused: list[dict[str, Any]] = []
    cycle, windows, relaxed = _periods(manifest.doc["periods"], steady, graph.economy.max_cycle_steps)
    entities = sorted((e for e in checker.entities if checker.entries.get(e.id) is e), key=lambda e: e.id)
    rules = sorted((r for r in checker.rules if checker.active(r) and r.kind in RULE_KINDS), key=lambda r: r.id)

    def of(kind: str) -> list[Entry]:
        return [rule for rule in rules if rule.kind == kind]

    for rule in rules:
        if rule.kind in NOT_MODELLED:
            refused.append(_refused(rule.path, rule.ref, rule.kind))
    excluded: set[str] = set()
    gates: list[tuple[str, Entry]] = []
    flows: list[tuple[Entry, str]] = []
    for rule in of("flow"):
        subject = checker.entries[rule.doc["subject"]]
        if subject.kind != "activity":
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
            continue
        requires = [checker.entries[gate] for gate in rule.doc["logic"].get("requires", ())]
        gates.extend((rule.id, gate) for gate in requires)
        if any(gate.doc["logic"]["origin"] == "person" for gate in requires):
            excluded.add(subject.id)  # real payment: never in a computed plan or value
        for index, gate in enumerate(requires):
            logic = gate.doc["logic"]
            if logic["origin"] != "person" and (logic["irreversible"] or "requires" in logic or "prefs" in logic):
                refused.append(_refused(rule.path, at(rule.ref, "logic", "requires", index), "gate"))
        flows.append((rule, subject.id))
    parts: list[_Part] = []
    for rule, activity in flows:
        if activity in excluded:
            continue
        logic = rule.doc["logic"]
        for member in ("consumes", "produces"):
            for resource in sorted(logic.get(member, {})):
                ref = at(rule.ref, "logic", member, resource)
                parts.append(_Part(rule.id, rule.path, activity, member, resource, None, None, ref))
        if "moves" in logic:
            moves, ref = logic["moves"], at(rule.ref, "logic", "moves", "amount")
            parts.append(_Part(rule.id, rule.path, activity, "moves", None, moves["from"], moves["to"], ref))
    regens: list[Entry] = []
    stops: dict[str, tuple[Entry, Entry]] = {}
    for rule in of("regen"):
        subject = checker.entries[rule.doc["subject"]]
        if subject.kind not in ("resource", "store"):
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
            continue
        if "while" in rule.doc["logic"] and subject.kind == "resource":
            stop = _stops_at(checker, graph, rule) if steady else None  # the plan does not model it
            if stop is None:
                refused.append(_refused(rule.path, at(rule.ref, "logic", "while"), "while"))
                continue
            stops[subject.id] = (rule, stop)
        elif "while" in rule.doc["logic"] and not _stops_production(checker, graph, rule):
            refused.append(_refused(rule.path, at(rule.ref, "logic", "while"), "while"))
            continue
        regens.append(rule)
    hard: dict[str, Entry] = {}
    void: dict[str, int] = {}
    storage: dict[str, Entry] = {}
    soft: list[Entry] = []
    incomes: list[tuple[Entry, tuple[str, ...]]] = []
    allowed = {
        "hard": ("resource", HARD_REACH + ((VOID,) if steady else ())), "storage": ("store", STORAGE_REACH),
        "soft_read": ("resource", ("block_income_from",)), "income": ("resource", HARD_REACH),
    }
    for rule in of("cap"):
        logic, subject = rule.doc["logic"], checker.entries[rule.doc["subject"]]
        reach = "block_income_from" if isinstance(logic["on_reach"], dict) else logic["on_reach"]
        kind, reaches = allowed[logic["cap_kind"]]
        if subject.kind != kind:
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
        elif reach not in reaches:
            refused.append(_refused(rule.path, at(rule.ref, "logic", "on_reach"), reach))
        elif reach == VOID and logic.get("per") in relaxed:  # it voids at resets the program does not place
            refused.append(_refused(rule.path, at(rule.ref, "logic", "per"), VOID))
        elif logic["cap_kind"] == "hard":
            hard[subject.id] = rule
            if reach == VOID:  # its resets: those of its period, else every day
                void[subject.id] = windows.get(logic.get("per", ""), 1)
        elif logic["cap_kind"] == "storage":
            storage[subject.id] = rule
        elif logic["cap_kind"] == "income":  # what the activities it lists bring in, else all that bring it in
            earners = logic.get("from", sorted({part.activity for part in parts if _brings(part, subject.id)}))
            incomes.append((rule, tuple(earners)))
        else:
            soft.append(rule)
    limits: list[tuple[Entry, tuple[str, ...]]] = []
    for rule in of("limit"):
        subject = checker.entries[rule.doc["subject"]]
        if subject.kind != "activity":
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
        else:
            limits.append((rule, (subject.id, *rule.doc["logic"].get("shared_with", ()))))
    objectives = of("objective")  # the static check holds a main goal to a resource, the secondary to the numeraire
    secondary = [rule for rule in objectives if rule.doc["logic"]["tier"] != MAIN]
    return _Shape(
        path=manifest.path,
        cycle=cycle,
        windows=windows,
        resources=[e.id for e in entities if e.kind == "resource"],
        stores={e.id: e.doc["holds"] for e in entities if e.kind == "store"},
        goals=[rule for rule in objectives if rule.doc["logic"]["tier"] == MAIN],
        secondary=secondary[0] if secondary else None,
        activities=sorted({activity for _, activity in flows} - excluded),
        excluded=sorted(excluded),
        parts=parts,
        regens=regens,
        stops=stops,
        hard=hard,
        void=void,
        storage=storage,
        soft=soft,
        limits=limits,
        incomes=incomes,
        relaxed=relaxed,
        gates=gates,
        refused=refused,
    )


# The values -----------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _Demand:
    """One row of the secondary objective's demand table, a goal sink of the steady state (#424 ACM-PRICE 2.2 (3),
    2.3): what one execution takes (the costs table's row of the same key), the value units it brings (`points ×
    value + main`), of them those A counts (`main`), and the executions a cycle allows (`count × cycle days ÷ horizon
    days`)."""

    key: str  # the row's key cells, joined with commas
    costs: tuple[tuple[str, Fraction], ...]  # (resource, amount), by resource; no amount of 0
    value: Fraction
    main: Fraction
    most: Fraction


@dataclass(frozen=True)
class _Model:
    """One program's evaluated inputs: everything the builder needs, as exact numbers."""

    solve: str
    path: str
    steps: int
    resources: tuple[str, ...]
    stores: tuple[str, ...]
    activities: tuple[str, ...]
    excluded: tuple[str, ...]
    net: dict[tuple[str, str], Fraction]  # (activity, resource or store) -> net amount per execution
    income: dict[tuple[str, str], Fraction]  # (activity, resource) -> what one execution brings in
    # activity -> what it takes from: the resources it consumes a non-zero amount of, the entities it moves out of
    inputs: dict[str, tuple[str, ...]]
    slots: frozenset[str]  # the slot resources of the model (`slots`)
    goals: tuple[tuple[str, str, Fraction], ...]  # steady: (main objective, its resource, the value of one unit)
    demand: tuple[_Demand, ...]  # steady: the secondary objective's demand rows, by key
    secondary: str | None  # steady: the secondary objective, the group of the demand rows' bounds
    moves: tuple[tuple[str, str, str, str], ...]  # (flow, activity, store, target): takes what the store holds
    regens: tuple[tuple[str, str, tuple[Fraction, ...]], ...]  # (rule, resource or store, income per step)
    stops: dict[str, tuple[str, str, Fraction]]  # resource -> (its regen, the cap it stops at, that cap's value)
    hard: dict[str, tuple[str, Fraction]]  # resource -> (cap, value)
    void: dict[str, int]  # resource -> the steps between the resets at which its hard cap voids
    storage: dict[str, tuple[str, Fraction]]  # store -> (cap, value)
    limits: tuple[tuple[str, tuple[str, ...], int, Fraction], ...]  # (rule, activities, window steps, max)
    # (cap, resource, the activities whose income it counts, window steps, value): an income cap (PRICE-3)
    incomes: tuple[tuple[str, str, tuple[str, ...], int, Fraction], ...]
    # rule -> (its period, the period's days, the program's days over them): a limit or income cap whose period
    # the program relaxes by rate; its value counts times the last over one window of the whole program
    relaxed: dict[str, tuple[str, Fraction, Fraction]]
    soft: tuple[tuple[str, str, Fraction, tuple[str, ...]], ...]  # (cap, resource, its reading, blocked moves)
    start: dict[str, tuple[str | None, Fraction]]  # plan: entity -> (its group, its opening stock)
    unread: tuple[str, ...]  # plan: entities that need no reading, started at 0
    weights: dict[str, Fraction]  # plan: its objective, the value of each entity's gain over the program
    grain: Fraction
    # steady: entity -> the pending flows left out (`excluded_pending`) that take from it, by id (PRICE-2b-1: why a
    # price has no low end)
    left_out: dict[str, tuple[str, ...]] = field(default_factory=dict)

    @property
    def entities(self) -> tuple[str, ...]:
        return tuple(sorted((*self.resources, *self.stores)))

    def groups(self) -> list[str]:
        """The rule groups the infeasibility analysis drops in turn, in id order."""
        found = {rule for rule, _, _ in self.regens}
        found |= {cap for cap, _ in (*self.hard.values(), *self.storage.values())}
        found |= {cap for _, cap, _ in self.stops.values()}
        found |= {rule for rule, _, _, _ in self.limits} | {cap for cap, _, _, _ in self.soft}
        found |= {cap for cap, _, _, _, _ in self.incomes}
        found |= {group for group, _ in self.start.values() if group is not None}
        if self.demand and self.secondary is not None:  # the demand rows' bounds
            found.add(self.secondary)
        return sorted(found)

    def priced(self) -> list[tuple[str, Fraction]]:
        """The rules with a value that the steady state prices: caps with a value (a regen's stop and an income
        cap included, each cap once) and limits."""
        found = dict([*self.hard.values(), *self.storage.values()])
        found.update((cap, value) for _, cap, value in self.stops.values())
        found.update((cap, value) for cap, _, _, _, value in self.incomes)
        found.update((rule, maximum) for rule, _, _, maximum in self.limits)
        return sorted(found.items())

    def scale(self, rule: str) -> Fraction:
        """What one unit of a limit's or an income cap's value counts in the program: 1, or for a relaxed period
        the program's days over the period's."""
        return self.relaxed[rule][2] if rule in self.relaxed else Fraction(1)

    def shown_relaxed(self) -> list[dict[str, str]]:
        """The rules the program relaxes, by id: each with its period, the period's days and the value it counts
        with over the program."""
        values = {rule: value for rule, value in self.priced()}
        return [
            {"rule": rule, "per": per, "days": str(days), "value": str(values[rule] * scale)}
            for rule, (per, days, scale) in sorted(self.relaxed.items())
        ]

    def touches_store(self, activity: str) -> bool:
        return any(key == activity and entity in self.stores for key, entity in self.net) or any(
            move[1] == activity for move in self.moves
        )


@dataclass(frozen=True)
class _Item:
    owner: Entry
    key: tuple[str, ...]
    ref: str
    part: _Part | None = None


@dataclass(frozen=True)
class _Unknowns:
    """What `_Reader.read` gives for a node that read unknowns, which the economy cannot read: it solves single
    values. `unknowns` are those the node read."""

    unknowns: frozenset[str]


class _Reader:
    """Reads the graph nodes one program needs: in the reference state for the steady state, from the rules and the
    data only and with the account state where needed for the plan. It records the status of each item it reads
    and the unknowns that leave it unresolved (#424 ACM-PRICE 4.6), the errors that block the program, what it
    refuses, and in the steady state the pending flows it leaves out (`excluded_pending`)."""

    def __init__(self, graph: Graph, shape: _Shape, solve: str) -> None:
        self.graph = graph
        self.shape = shape
        self.plan = solve == PLAN
        self.solve = solve
        self.prefix = PREFIX[solve]
        # What each item read: an item's key (a flow amount, a regen member, a cap or limit, ("requires", flow,
        # gate), and for the plan the steady state) -> its status and the unknowns it depends on unresolved.
        self.statuses: dict[tuple[str, ...], tuple[str, frozenset[str]]] = {}
        self.pending: dict[str, set[str]] = {}  # steady: a pending flow left out -> the unknowns its amounts read
        # What the program read, in order, as a graph node's scenario records it (`graph`, "Reads"): the graph nodes,
        # the data rows and the gates consulted (`entry`). The provenance of the economy's outputs (PRICE-2b-1).
        self.reads: dict[tuple[str, ...], None] = {}
        self.blocked: set[int] = set()
        self.refused = list(shape.refused)
        entries = graph.checker.entries
        self.stock_facts = {
            entries[entity].doc["reading"] for entity in (*shape.resources, *shape.stores)
            if "reading" in entries[entity].doc
        }

    def note(self, key: tuple[str, ...], status: str, unresolved: Any = ()) -> None:
        """What item `key` read: the worst status so far, and the unknowns it depends on without reading them."""
        before, left = self.statuses.get(key, (CONFIRMED, frozenset()))
        self.statuses[key] = (worst(before, status), left | frozenset(unresolved))

    def consult(self, key: tuple[str, ...], entry: Entry) -> None:
        own, unresolved = rule_status(entry)
        self.note(key, own, unresolved)

    def inputs(self) -> str:
        """The status of what the program read: the worst status of its items, but those of the pending flows it
        left out, and provisional when any of them depends on an unknown it leaves unresolved."""
        status, unresolved = CONFIRMED, False
        for key, (found, left) in self.statuses.items():
            if key[0] in ("flow", "requires") and key[1] in self.pending:
                continue
            status, unresolved = worst(status, found), unresolved or bool(left)
        return worst(status, PROVISIONAL) if unresolved else status

    def excluded_pending(self) -> list[dict[str, Any]]:
        """The pending flows the steady state left out, by id: each with its activity and the unknowns it read."""
        entries = self.graph.checker.entries
        return [
            {"flow": flow, "activity": entries[flow].doc["subject"], "unknowns": sorted(self.pending[flow])}
            for flow in sorted(self.pending)
        ]

    def refuse(self, path: str, ref: str, found: str) -> None:
        self.refused.append(_refused(path, ref, found))

    def need(self, keys: list[tuple[str, ...]]) -> None:
        for key in keys:
            if key not in self.graph.results:
                raise Need(key)

    def read(self, key: tuple[str, ...], item: tuple[str, ...]) -> Any:
        """The one scenario of a node, noted for `item`; `_STATE` for a node from the rules and the data only
        that would need the account state; `_Unknowns` for one that read unknowns; None when it is blocked."""
        self.graph.depend(key)
        self.reads[key] = None
        result = self.graph.results[key]
        if result.blocked:
            self.blocked.update(result.blocked_by)
            return None
        if result.needs_state:
            return _STATE
        scenario, *more = result.scenarios
        if more or scenario.unknowns or scenario.free or type(scenario.value) in (Interval, Undecided):
            return _Unknowns(frozenset(unknowns_read(result.scenarios)))
        self.note(item, scenario.status, scenario.unresolved)
        return scenario

    def leaves_out(self, item: _Item) -> bool:
        """Whether the steady state leaves out the flow of an item that read unknowns, instead of refusing it: a
        flow amount of a pending rule (the ACM-PRICE addendum, part 1). Any other is refused (`unknown`)."""
        return not self.plan and item.part is not None and item.owner.doc["status"] == "pending"

    def reads_stock(self, scenario: Scenario) -> bool:
        """Whether a value read, through any node, the reading of a stock the program itself changes."""
        seen: set[tuple[str, ...]] = set()
        todo = list(scenario.reads)
        while todo:
            item = todo.pop()
            if item in seen:
                continue
            seen.add(item)
            if item[0] == "reading" and item[1] in self.stock_facts:
                return True
            for other in getattr(self.graph.results.get(item), "scenarios", ()):
                todo.extend(other.reads)
        return False

    def items(self) -> list[_Item]:
        shape, entries = self.shape, self.graph.checker.entries
        found = [_Item(entries[part.flow], part.key, part.ref, part) for part in shape.parts]
        for rule in shape.regens:
            for member in ("amount", "every"):
                found.append(_Item(rule, ("regen", rule.id, member), at(rule.ref, "logic", member)))
        caps = [*shape.hard.values(), *shape.storage.values()]
        caps += [cap for _, cap in shape.stops.values() if cap.id not in {other.id for other in caps}]
        for cap in caps:  # the steady state reads a reading's `reference` (G-A), the plan the reading
            logic = cap.doc["logic"]
            member = "reference" if not self.plan and "reference" in logic else "reading"
            found.append(_Item(cap, ("rule", cap.id), at(cap.ref, "logic", "value" if "value" in logic else member)))
        for cap, _ in shape.incomes:
            found.append(_Item(cap, ("rule", cap.id), at(cap.ref, "logic", "value")))
        for rule, _ in shape.limits:
            found.append(_Item(rule, ("rule", rule.id), at(rule.ref, "logic", "max")))
        if not self.plan:  # the goal sinks' values (PRICE-2a); the plan values its final stocks instead
            objectives = [*shape.goals, *([shape.secondary] if shape.secondary is not None else [])]
            for rule in objectives:
                members = ("value",) if rule.doc["logic"]["tier"] == MAIN else ("value", "horizon")
                for member in members:
                    found.append(_Item(rule, ("objective", rule.id, member), at(rule.ref, "logic", member)))
        return found

    def values(self, items: list[_Item]) -> tuple[dict[tuple[str, ...], Any], set[str]]:
        """The value of each item, and the flows whose move takes what its store holds."""
        graph, plan, prefix = self.graph, self.plan, self.prefix
        self.need([prefix + item.key for item in items])
        if plan:
            self.need([
                item.key for item in items
                if graph.results[prefix + item.key].needs_state
                and not (item.part and item.part.member == "moves")
            ])
        values: dict[tuple[str, ...], Any] = {}
        variable: set[str] = set()
        for item in items:
            self.consult(item.key, item.owner)
            found = self.read(prefix + item.key, item.key)
            if found is _STATE:
                if item.part is not None and item.part.member == "moves":
                    variable.add(item.part.flow)  # a collection: it takes what the store holds
                    continue
                if not plan:  # the steady state is game-wide: it never reads the account state
                    self.refuse(item.owner.path, item.ref, "state")
                    continue
                found = self.read(item.key, item.key)
                if found is not None and not isinstance(found, _Unknowns) and self.reads_stock(found):
                    self.refuse(item.owner.path, item.ref, "stock")  # a stock the plan changes as it goes
                    continue
            if isinstance(found, _Unknowns):
                if self.leaves_out(item):  # left out and listed, not refused (the ACM-PRICE addendum, part 1)
                    self.pending.setdefault(item.part.flow, set()).update(found.unknowns)
                else:
                    self.refuse(item.owner.path, item.ref, "unknown")
                continue
            if found is not None:
                values[item.key] = found.value
        for flow, gate in self.shape.gates:
            self.consult(("requires", flow, gate.id), gate)
            self.reads[("entry", gate.id)] = None
        return values, variable

    def model(self, settings: EconomySettings) -> _Model | None:
        shape, graph, plan = self.shape, self.graph, self.plan
        items = self.items()
        if plan:
            self.need([("rule", cap.id) for cap in shape.soft] + [NODES[STEADY]])
        values, variable = self.values(items)
        steps = settings.plan_horizon if plan else shape.cycle
        steady: Economy | None = None
        if plan:  # the final stocks are priced by the steady state, at the step the plan ends in
            graph.depend(NODES[STEADY])
            self.reads[NODES[STEADY]] = None
            steady = graph.results[NODES[STEADY]]
            if steady.blocked:
                self.blocked.update(steady.blocked_by)
                return None
            self.note(NODES[STEADY], steady.status)
        regens = []
        for rule in shape.regens:
            amount, every = values.get(("regen", rule.id, "amount")), values.get(("regen", rule.id, "every"))
            if amount is None or every is None:
                continue
            incomes = self.incomes(rule, amount, every.seconds, steps)
            if incomes is not None:
                regens.append((rule.id, rule.doc["subject"], incomes))
        hard = {r: (cap.id, values[("rule", cap.id)]) for r, cap in shape.hard.items() if ("rule", cap.id) in values}
        storage = {
            f: (cap.id, values[("rule", cap.id)]) for f, cap in shape.storage.items() if ("rule", cap.id) in values
        }
        regenerated = {rule for rule, _, _ in regens}
        stops = {
            r: (rule.id, cap.id, values[("rule", cap.id)]) for r, (rule, cap) in shape.stops.items()
            if ("rule", cap.id) in values and rule.id in regenerated
        }
        left_out = {part.activity for part in shape.parts if part.flow in self.pending}
        net: dict[tuple[str, str], Fraction] = {}
        income: dict[tuple[str, str], Fraction] = {}
        moves = []
        for part in shape.parts:
            if part.flow in self.pending:  # a pending flow the steady state leaves out
                continue
            if part.member == "moves" and part.flow in variable:
                if part.source not in storage:  # only a store's storage cap bounds what one move takes
                    self.refuse(part.path, part.ref, "state")
                    continue
                moves.append((part.flow, part.activity, part.source, part.target))
                continue
            amount = values.get(part.key)
            if amount is None:
                continue
            if part.member == "consumes":
                _add(net, (part.activity, part.resource), -amount)
                continue
            target = part.resource if part.member == "produces" else part.target
            if part.member == "moves":
                _add(net, (part.activity, part.source), -amount)
            _add(net, (part.activity, target), amount)
            _add(income, (part.activity, target), max(amount, Fraction(0)))
        activities = set(shape.activities) - left_out
        # What each activity takes from (PRICE-2a, the inputs of §4.1): a non-zero amount it consumes, or an entity
        # it moves out of; an amount of 0 is no input.
        inputs: dict[str, set[str]] = {}
        for part in shape.parts:
            if part.activity not in activities:
                continue
            if part.member == "consumes" and values.get(part.key):
                inputs.setdefault(part.activity, set()).add(part.resource)
            elif part.member == "moves" and (part.flow in variable or values.get(part.key)):
                inputs.setdefault(part.activity, set()).add(part.source)
        goals = () if plan else tuple(
            (rule.id, rule.doc["subject"], values[("objective", rule.id, "value")])
            for rule in shape.goals if ("objective", rule.id, "value") in values
        )
        demand = () if plan else self.demand(values, steps)
        relaxed: dict[str, tuple[str, Fraction, Fraction]] = {}

        def window(rule: Entry) -> int:
            """The steps of one window of the rule's period; a relaxed period's is the whole program."""
            per = rule.doc["logic"]["per"]
            if per not in shape.relaxed:
                return shape.windows[per]
            relaxed[rule.id] = (per, shape.relaxed[per], Fraction(steps) / shape.relaxed[per])
            return steps

        limits = tuple(
            (rule.id, tuple(sorted(set(members) & activities)), window(rule), values[("rule", rule.id)])
            for rule, members in shape.limits if ("rule", rule.id) in values
        )
        capped = tuple(
            (
                cap.id, cap.doc["subject"], tuple(sorted(set(earners) & activities)), window(cap),
                values[("rule", cap.id)],
            )
            for cap, earners in shape.incomes if ("rule", cap.id) in values
        )
        soft = self.read_caps(variable, hard, storage) if plan else ()
        start, unread = self.opening(net, moves, regens, hard, storage, soft) if plan else ({}, ())
        if self.refused or self.blocked:
            return None
        weights: dict[str, Fraction] = {}  # the steady state's objective is its goal sinks' value (`total_value`)
        if steady is not None:  # read once nothing is refused: a price without a low end comes only with a refusal
            cycle = steady.data["cycle"]["steps"]  # one more unit of income, re-solved (`grain`), as before PRICE-2b-1
            weights = {r: _low(steady.data["grain"]["prices"][r][steps % cycle]) for r in shape.resources}
        left_out: dict[str, set[str]] = {}  # what the pending flows left out take from (PRICE-2b-1, `no_room`)
        for part in shape.parts:
            taken = part.resource if part.member == "consumes" else part.source if part.member == "moves" else None
            if part.flow in self.pending and taken is not None:
                left_out.setdefault(taken, set()).add(part.flow)
        return _Model(
            solve=self.solve,
            path=shape.path,
            steps=steps,
            resources=tuple(shape.resources),
            stores=tuple(sorted(shape.stores)),
            activities=tuple(a for a in shape.activities if a in activities),
            excluded=tuple(shape.excluded),
            net=net,
            income=income,
            inputs={activity: tuple(sorted(found)) for activity, found in sorted(inputs.items())},
            slots=slots(graph.checker, graph.typed) & frozenset(shape.resources),
            goals=goals,
            demand=demand,
            secondary=None if plan or shape.secondary is None else shape.secondary.id,
            moves=tuple(moves),
            regens=tuple(regens),
            stops=stops,
            hard=hard,
            void={r: shape.void[r] for r in hard if r in shape.void},
            storage=storage,
            limits=limits,
            incomes=capped,
            relaxed=relaxed,
            soft=soft,
            start=start,
            unread=tuple(unread),
            weights=weights,
            grain=Fraction(settings.grain),
            left_out={entity: tuple(sorted(flows)) for entity, flows in sorted(left_out.items())},
        )

    def incomes(self, rule: Entry, amount: Fraction, seconds: Fraction, steps: int) -> tuple[Fraction, ...] | None:
        """A regen's income in each step: accrued when it comes at least once a step, else its whole amount in
        the steps a whole number of steps apart, from the first."""
        ref = at(rule.ref, "logic", "every")
        if seconds <= 0:
            detail = {
                "path": rule.path, "reason": "value_invalid", "operator": "regen", "position": 0, "expected": "positive",
            }
            self.refused.append(error(EXPR_EVAL_FAILED, detail, ref))
            return None
        if seconds <= STEP_SECONDS:
            return (amount * STEP_SECONDS / seconds,) * steps
        apart = seconds / STEP_SECONDS
        if apart.denominator != 1 or (not self.plan and self.shape.cycle % apart != 0):
            self.refuse(rule.path, ref, "every")
            return None
        return tuple(amount if step % apart == 0 else Fraction(0) for step in range(steps))

    def demand(self, values: dict[tuple[str, ...], Any], steps: int) -> tuple[_Demand, ...]:
        """The steady state's demand rows (#424 ACM-PRICE 2.3): each row of the secondary objective's demand table,
        by key, with the costs table's row of that key (the static check joined them). A row is data, never the
        account state, so the reference demand is neutral (126 A). A horizon that is not positive is `value_invalid`;
        an external table, whose rows this version does not read, is refused (`external_dataset`); a pending row,
        which names no unknown, blocks (`rule_pending_unbounded`, `no_unknown`)."""
        rule = self.shape.secondary
        if rule is None:
            return ()
        lam, horizon = values.get(("objective", rule.id, "value")), values.get(("objective", rule.id, "horizon"))
        if lam is None or horizon is None:  # refused or blocked: reported already
            return ()
        logic, graph = rule.doc["logic"], self.graph
        if horizon.seconds <= 0:
            detail = {
                "path": rule.path, "reason": "value_invalid", "operator": "horizon", "position": 0, "expected": "positive",
            }
            self.refused.append(error(EXPR_EVAL_FAILED, detail, at(rule.ref, "logic", "horizon")))
            return ()
        tables = {}
        for member in ("demand", "costs"):
            graph.depend(("table", logic[member]))
            tables[member] = graph.checker.tables[logic[member]]
            if tables[member].rows is None:
                self.refuse(rule.path, at(rule.ref, "logic", member), "external_dataset")
                return ()
            graph.depend(("table_keys", logic[member]))
        demand, costs = tables["demand"], tables["costs"]
        columns = sorted(name for name in costs.file.doc["columns"] if name not in costs.keys)
        found = []
        for key in sorted(demand.rows):
            row, cost = self.row(logic["demand"], demand, key), self.row(logic["costs"], costs, key)
            if row is None or cost is None:
                continue
            count, points, main = (Fraction(row[name]) for name in DEMAND_COLUMNS)
            amounts = tuple((name, Fraction(cost[name])) for name in columns if Fraction(cost[name]) != 0)
            most = count * steps * STEP_SECONDS / horizon.seconds
            found.append(_Demand(",".join(key), amounts, points * lam + main, main, most))
        return tuple(found)

    def row(self, table_id: str, table: Any, key: tuple[str, ...]) -> dict[str, Any] | None:
        """A data row the steady state reads, noted with its status; None for a pending row, which blocks."""
        self.graph.depend(("row", table_id, *key))
        self.reads[("row", table_id, *key)] = None
        index = table.rows[key]
        row = table.file.doc["rows"][index]
        if row["status"] == "pending":
            detail = {"path": table.file.path, "id": f"{table_id}[{','.join(key)}]", "reason": "no_unknown"}
            self.refused.append(error(RULE_PENDING_UNBOUNDED, detail, at("", "rows", index, "status")))
            return None
        self.note(("row", table_id, *key), RULE_STATUS[row["status"]])
        return row

    def read_caps(self, variable: set[str], hard: dict[str, Any], storage: dict[str, Any]) -> tuple[Any, ...]:
        """The plan's read caps: each with its reading and the collections it blocks at or above it."""
        found = []
        for cap in self.shape.soft:
            self.consult(("rule", cap.id), cap)
            scenario = self.read(("rule", cap.id), ("rule", cap.id))
            if isinstance(scenario, _Unknowns):
                self.refuse(cap.path, at(cap.ref, "logic", "reading"), "unknown")
                continue
            if scenario is None or scenario is _STATE:
                continue
            resource = cap.doc["subject"]
            if resource not in hard:  # the indicator needs a bound on the stock: the resource's hard cap
                self.refuse(cap.path, at(cap.ref, "subject"), "unbounded_stock")
                continue
            flows: set[str] = set()
            for index, activity in enumerate(cap.doc["logic"]["on_reach"]["block_income_from"]):
                for part in self.shape.parts:
                    brings = (part.member == "produces" and part.resource == resource) or (
                        part.member == "moves" and part.target == resource
                    )
                    if part.activity != activity or not brings:
                        continue
                    if part.member == "moves" and part.flow in variable and part.source in storage:
                        flows.add(part.flow)
                    else:  # only a collection out of a store is blocked in this version
                        self.refuse(cap.path, at(cap.ref, "logic", "on_reach", "block_income_from", index), "income")
            found.append((cap.id, resource, scenario.value, tuple(sorted(flows))))
        return tuple(found)

    def opening(self, net, moves, regens, hard, storage, soft) -> tuple[dict[str, Any], list[str]]:
        """The plan's opening stocks: each entity's reading. An entity that declares none needs none when nothing
        can lower or cap its stock; it starts at 0, and only its gain counts."""
        lowered = {entity for (_, entity), amount in net.items() if amount < 0}
        lowered |= {source for _, _, source, _ in moves} | set(hard) | set(storage)
        lowered |= {resource for _, resource, _, _ in soft}
        lowered |= {target for _, target, incomes in regens if any(amount < 0 for amount in incomes)}
        entries, readings = self.graph.checker.entries, self.graph.state["readings"]
        start: dict[str, tuple[str | None, Fraction]] = {}
        unread: list[str] = []
        for entity in sorted((*self.shape.resources, *self.shape.stores)):
            entry = entries[entity]
            fact = entry.doc.get("reading")
            if fact is None:
                if entity in lowered:
                    self.refused.append(error(STATE_READING_MISSING, {"path": entry.path, "entity": entity}, entry.ref))
                else:
                    start[entity] = (None, Fraction(0))
                    unread.append(entity)
                continue
            self.graph.depend(("reading", fact))
            if fact not in readings:
                detail = {"path": entry.path, "fact": fact}
                self.refused.append(error(STATE_READING_MISSING, detail, at(entry.ref, "reading")))
                continue
            start[entity] = (READING_GROUP + fact, Fraction(readings[fact]["value"]))
        return start, unread


# The program ----------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _Row:
    key: Key
    coefficients: dict[Key, Fraction]
    sense: str  # eq or le
    rhs: Fraction
    group: str | None  # the rule (or opening stock) whose value it holds; None for the structure


@dataclass(frozen=True)
class _Program:
    variables: dict[Key, tuple[Fraction, Fraction | None, bool]]  # key -> (lower, upper or None, integer)
    rows: tuple[_Row, ...]
    objective: dict[Key, Fraction]  # maximised


def _floor(name: str, coefficients: dict[Key, Fraction], bound: Fraction) -> tuple[_Row, ...]:
    """The row that holds a function of the program at or above `bound` (A ≥ A* or V ≥ V*, PRICE-2a): `(floor,
    name, 0)`, its terms negated, at most -bound. No row for a function without a term, which is 0 everywhere."""
    if not coefficients:
        return ()
    return (_Row(("floor", name, 0), {key: -value for key, value in coefficients.items()}, "le", -bound, None),)


def _program(
    model: _Model,
    *,
    extra: dict[tuple[str, int], Fraction] | None = None,
    values: dict[str, Fraction] | None = None,
    omit: frozenset[str] = frozenset(),
    objective: dict[Key, Fraction] | None = None,
    added: tuple[_Row, ...] = (),
    least_losses: bool = False,
) -> _Program:
    """The program of `model`. `extra` adds income to a resource in a step and `values` replaces the value of a
    cap or limit (the price re-solves); `omit` drops rule groups (the infeasibility analysis). Its objective is,
    for the steady state, the value V of its goal sinks (`total_value`: S3, and every price re-solve) and, for the
    plan, the stocks' gain at `weights`; `objective` replaces it ({} for none; `acquisition` for S1). `added`
    adds rows (the floors of `_floor`, a probe). With `least_losses`, the overflow and lost production are
    minimised instead (what an income cap voids is not: the overflow is the least)."""
    extra, values = extra or {}, values or {}
    zero, one, steps, plan = Fraction(0), Fraction(1), model.steps, model.solve == PLAN
    variables: dict[Key, tuple[Fraction, Fraction | None, bool]] = {}
    rows: list[_Row] = []
    income: dict[tuple[str, int], tuple[Fraction, Fraction]] = {}  # (entity, step) -> (all, the positive part)
    regenerated: dict[tuple[str, int], Fraction] = {}  # (regen, step) -> its income in the step

    def receive(entity: str, step: int, amount: Fraction) -> None:
        total, positive = income.get((entity, step), (zero, zero))
        income[(entity, step)] = (total + amount, positive + max(amount, zero))

    for rule, target, incomes in model.regens:
        if rule not in omit:
            for step, amount in enumerate(incomes):
                receive(target, step, amount)
                regenerated[(rule, step)] = amount
    for (entity, step), amount in sorted(extra.items()):
        receive(entity, step, amount)
    hard = {r: (cap, values.get(cap, value)) for r, (cap, value) in model.hard.items() if cap not in omit}
    storage = {f: (cap, values.get(cap, value)) for f, (cap, value) in model.storage.items() if cap not in omit}
    stops = {r: (rule, cap, values.get(cap, value)) for r, (rule, cap, value) in model.stops.items() if cap not in omit}
    most = {f: values.get(cap, value) for f, (cap, value) in model.storage.items()}  # what one move takes at most
    every = {r: model.void.get(r, 1) for r in hard}  # the steps between the boundaries at which a hard cap holds

    def received(entity: str, step: int) -> tuple[Fraction, Fraction]:
        return income.get((entity, step), (zero, zero))

    def regen_income(entity: str, step: int) -> Fraction:
        """What the regen of a resource whose regen stops brings in a step (G-B)."""
        return regenerated.get((stops[entity][0], step), zero)

    def overflows(entity: str, step: int) -> bool:
        """Whether a hard cap blocks income in a step: in every step, and for a void_at_reset cap only in the step
        that ends at a reset of its period (G-C)."""
        return entity in hard and (step + 1) % every[entity] == 0

    for activity in model.activities:
        for step in range(steps):
            variables[("x", activity, step)] = (zero, None, plan)
    # The goal sinks (steady, PRICE-2a): per step the executions of each main objective's (`g`) and each demand
    # row's (`q`); what one execution takes of each entity.
    sinks: dict[str, list[tuple[str, str, Fraction]]] = {}
    for rule, resource, _ in model.goals:
        sinks.setdefault(resource, []).append(("g", rule, one))
    for row in model.demand:
        for resource, amount in row.costs:
            sinks.setdefault(resource, []).append(("q", row.key, amount))
    for step in range(steps):
        for rule, _, _ in model.goals:
            variables[("g", rule, step)] = (zero, None, False)
        for row in model.demand:
            variables[("q", row.key, step)] = (zero, None, False)
    for entity in model.entities:
        for step in range(steps + 1):
            variables[("s", entity, step)] = (zero, None, False)
        for step in range(steps):
            if overflows(entity, step):
                variables[("o", entity, step)] = (zero, None, False)
            if entity in storage:  # production lost at the storage cap: at most what the store produces
                variables[("l", entity, step)] = (zero, received(entity, step)[1], False)
            if entity in stops:  # production lost where the regen stops: at most what the regen brings
                variables[("l", entity, step)] = (zero, max(regen_income(entity, step), zero), False)
    for flow, _, _, _ in model.moves:
        for step in range(steps):
            variables[("m", flow, step)] = (zero, None, False)
    voids: dict[str, list[str]] = {}  # resource -> its income caps, each voiding in every step (PRICE-3)
    for cap, resource, _, _, _ in model.incomes:
        if cap not in omit:
            voids.setdefault(resource, []).append(cap)
            for step in range(steps):
                variables[("v", cap, step)] = (zero, None, False)
    for entity in model.entities:
        inflows: list[dict[Key, Fraction]] = []
        for step in range(steps):
            coefficients = {("s", entity, step + 1): one, ("s", entity, step): -one}
            for activity in model.activities:
                amount = model.net.get((activity, entity))
                if amount:
                    coefficients[("x", activity, step)] = -amount
            for kind, name, amount in sinks.get(entity, ()):  # what a goal sink takes
                coefficients[(kind, name, step)] = coefficients.get((kind, name, step), zero) + amount
            inflow: dict[Key, Fraction] = {}
            for activity in model.activities:
                amount = model.income.get((activity, entity))
                if amount:
                    inflow[("x", activity, step)] = amount
            for flow, _, source, target in model.moves:
                key = ("m", flow, step)
                if target == entity:
                    coefficients[key] = coefficients.get(key, zero) - one
                    inflow[key] = inflow.get(key, zero) + one
                if source == entity:
                    coefficients[key] = coefficients.get(key, zero) + one
            for kind in ("o", "l"):
                if (kind, entity, step) in variables:
                    coefficients[(kind, entity, step)] = one
            for cap in voids.get(entity, ()):  # what an income cap voids never reaches the stock
                coefficients[("v", cap, step)] = one
            inflows.append(inflow)
            rows.append(_Row(("balance", entity, step), _nonzero(coefficients), "eq", received(entity, step)[0], None))
            if overflows(entity, step):  # what overflows is income the cap blocks, never stock already held
                bound: dict[Key, Fraction] = {("o", entity, step): one}
                positive = zero
                for blocked in range(step + 1 - every[entity], step + 1):  # the steps since the last reset
                    for key, amount in inflows[blocked].items():
                        bound[key] = bound.get(key, zero) - amount
                    for cap in voids.get(entity, ()):  # nor what an income cap voids of the same income
                        bound[("v", cap, blocked)] = bound.get(("v", cap, blocked), zero) + one
                    positive += received(entity, blocked)[1]
                rows.append(_Row(("overflow", entity, step), _nonzero(bound), "le", positive, None))
    if not plan:  # a steady state ends each cycle with at least the stock it began with; a capped one with it
        for entity in model.entities:  # the numeraire is a value entity, no stock: its gain is the sinks' value
            cyclic = {("s", entity, 0): one, ("s", entity, steps): -one}
            sense = "eq" if entity in hard or entity in storage or entity in stops else "le"
            rows.append(_Row(("cyclic", entity, 0), cyclic, sense, zero, None))
    for entity, (cap, value) in sorted((*hard.items(), *storage.items())):
        for step in range(0, steps + 1, every.get(entity, 1)):  # a void_at_reset cap holds at its resets only
            rows.append(_Row(("cap", cap, step), {("s", entity, step): one}, "le", value, cap))
    for entity, (rule, cap, value) in sorted(stops.items()):
        # The regen stops at the cap (G-B): what the step consumes first, then what the regen brings, up to the cap.
        for step in range(steps):
            row = {("s", entity, step): one, ("l", entity, step): -one}
            for activity in model.activities:
                used = model.income.get((activity, entity), zero) - model.net.get((activity, entity), zero)
                if used:
                    row[("x", activity, step)] = -used
            for kind, name, amount in sinks.get(entity, ()):
                row[(kind, name, step)] = row.get((kind, name, step), zero) - amount
            rows.append(_Row(("stop", cap, step), _nonzero(row), "le", value - regen_income(entity, step), cap))
    for rule, members, window, maximum in model.limits:
        if rule in omit:
            continue
        for index, first in enumerate(range(0, steps, window)):
            counted = {("x", a, step): one for a in members for step in range(first, min(first + window, steps))}
            if counted:
                most_runs = values.get(rule, maximum) * model.scale(rule)  # a relaxed period's by rate
                rows.append(_Row(("limit", rule, index), counted, "le", most_runs, rule))
    if model.secondary not in omit:  # a demand row runs at most `count × cycle days ÷ horizon days` a cycle
        for row in model.demand:
            counted = {("q", row.key, step): one for step in range(steps)}
            rows.append(_Row(("demand", row.key, 0), counted, "le", row.most, model.secondary))
    for cap, resource, members, window, value in model.incomes:
        if cap in omit:
            continue
        # An income cap (PRICE-3): per window of its period, what the activities it lists bring in, less what it
        # voids, is at most its value; it voids at most what they bring in, step by step.
        for index, first in enumerate(range(0, steps, window)):
            counted: dict[Key, Fraction] = {}
            for step in range(first, min(first + window, steps)):
                earned = {
                    ("x", activity, step): model.income[(activity, resource)] for activity in members
                    if model.income.get((activity, resource))
                }
                earned.update(
                    (("m", flow, step), one) for flow, activity, _, target in model.moves
                    if target == resource and activity in members
                )
                bound = {("v", cap, step): one, **{key: -amount for key, amount in earned.items()}}
                rows.append(_Row(("voided", cap, step), bound, "le", zero, None))
                for key, amount in earned.items():
                    counted[key] = counted.get(key, zero) + amount
                counted[("v", cap, step)] = -one
            most_kept = values.get(cap, value) * model.scale(cap)
            rows.append(_Row(("income", cap, index), counted, "le", most_kept, cap))
    for flow, activity, source, _ in model.moves:
        for step in range(steps):
            bound = {("m", flow, step): one, ("x", activity, step): -most[source]}
            rows.append(_Row(("move", flow, step), _nonzero(bound), "le", zero, None))
    for cap, resource, reading, flows in model.soft:
        if cap in omit:
            continue
        big = model.hard[resource][1] - reading + model.grain  # the stock never exceeds its hard cap
        if big <= 0:  # the stock never reaches the read cap: nothing is ever blocked
            continue
        for step in range(steps):
            allowed = ("b", cap, step)
            variables[allowed] = (zero, one, True)
            row = {("s", resource, step): one, allowed: big}
            rows.append(_Row(("soft", cap, step), row, "le", reading - model.grain + big, cap))
            for flow in flows:
                source = next(move[2] for move in model.moves if move[0] == flow)
                taken = most[source] + received(source, step)[1]  # at most a full store and what it makes
                row = {("m", flow, step): one, allowed: -taken}
                rows.append(_Row(("soft", f"{cap} {flow}", step), _nonzero(row), "le", zero, cap))
    for entity, (group, amount) in model.start.items():
        if group is None or group not in omit:
            rows.append(_Row(("start", entity, 0), {("s", entity, 0): one}, "eq", amount, group))
    rows.extend(added)
    goal: dict[Key, Fraction] = {}
    if least_losses:
        goal = {key: -one for key in variables if key[0] in ("o", "l")}
    elif objective is not None:
        goal = dict(objective)
    elif plan:  # the stocks' gain, each at the steady state's price
        for entity, weight in model.weights.items():
            _add(goal, ("s", entity, steps), weight)
            _add(goal, ("s", entity, 0), -weight)
    else:
        goal = total_value(model)
    return _Program(variables, tuple(rows), _nonzero(goal))


def _income_moves(model: _Model, resource: str, step: int, amount: Fraction) -> dict[Key, Fraction] | None:
    """The rows of the steady program whose right side `amount` more income of `resource` in `step` moves, as
    `_program`'s `extra` moves them, each by how much (PRICE-2d): its balance row of that step, and for a resource with
    a hard cap, when the amount is above 0 (only income that comes counts), the overflow row of the step that closes
    the cap's window holding that step. None where the income moves more than right sides: a store's bound on lost
    production, or what a move under a read cap may take from it."""
    if model.solve != STEADY or resource in model.storage or resource in model.stores:
        return None
    if any(move[2] == resource for move in model.moves if any(move[0] in flows for _, _, _, flows in model.soft)):
        return None
    moves: dict[Key, Fraction] = {("balance", resource, step): amount}
    if resource in model.hard and amount > 0:
        every = model.void.get(resource, 1)
        for closing in range(step, min(step + every, model.steps)):
            if (closing + 1) % every == 0:
                moves[("overflow", resource, closing)] = amount
    return moves


# The dual (PRICE-2b-1; #424 ACM-PRICE addendum 2.1 §2.2 step 2) -------------------------------------------------


@dataclass(frozen=True)
class _Dual:
    """The dual of a program, generated from the program's rows, never written by hand.

    The program maximises c·x over its rows (equalities, and at-mosts) with 0 ≤ x ≤ u. The dual minimises its
    `cost`, Σ_i rhs_i·y_i + Σ_j u_j·w_j, over:
    - a variable per row: non-negative for an at-most (`y`), free for an equality, which a `_Program` (numeric lower
      bounds only) holds as the difference of two non-negative ones (`y+` less `y-`);
    - a non-negative variable per variable with an upper bound (`w`);
    - a row per column j (`dual`), Σ_i A_ij·y_i + w_j ≥ c_j, written as the at-most −Σ_i A_ij·y_i − w_j ≤ −c_j.
    `duals` maps each row of the program to its dual, a combination of these variables, and `by_column` each column of
    the program to its dual row (PRICE-2c: its dual slack, `_never`)."""

    variables: dict[Key, tuple[Fraction, Fraction | None, bool]]
    rows: tuple[_Row, ...]
    cost: dict[Key, Fraction]
    duals: dict[Key, dict[Key, Fraction]]
    by_column: dict[Key, _Row] = field(default_factory=dict)

    def face(self, optimum: Fraction) -> tuple[_Row, ...]:
        """Its rows and the strong-duality row, its cost at the program's optimum (`FACE`): their solutions are D*,
        the dual's optimal face."""
        return (*self.rows, _Row(FACE, dict(self.cost), "eq", optimum, None))


def _dual(program: _Program) -> _Dual:
    """The dual of `program` (`_Dual`). Its variables must be continuous and start at 0, as the steady state's are;
    any other is a defect."""
    zero, one = Fraction(0), Fraction(1)
    variables: dict[Key, tuple[Fraction, Fraction | None, bool]] = {}
    duals: dict[Key, dict[Key, Fraction]] = {}
    cost: dict[Key, Fraction] = {}
    for row in sorted(program.rows, key=lambda row: row.key):
        token = _token(row.key)
        duals[row.key] = {("y+", token, 0): one, ("y-", token, 0): -one} if row.sense == "eq" else {("y", token, 0): one}
        for key, sign in duals[row.key].items():
            variables[key] = (zero, None, False)
            _add(cost, key, sign * row.rhs)
    uppers: dict[Key, Key] = {}
    for key in sorted(program.variables):
        low, high, integer = program.variables[key]
        if low != 0 or integer:
            raise ValueError(f"the dual needs continuous variables from 0: {_token(key)}")
        if high is not None:
            uppers[key] = ("w", _token(key), 0)
            variables[uppers[key]] = (zero, None, False)
            _add(cost, uppers[key], high)
    columns: dict[Key, dict[Key, Fraction]] = {key: {} for key in program.variables}
    for row in program.rows:
        for key, coefficient in row.coefficients.items():
            for dual, sign in duals[row.key].items():
                _add(columns[key], dual, sign * coefficient)
    rows = []
    by_column: dict[Key, _Row] = {}
    for key in sorted(program.variables):
        coefficients = {dual: -amount for dual, amount in columns[key].items()}
        if key in uppers:
            coefficients[uppers[key]] = -one
        cost_j = -program.objective.get(key, zero)
        by_column[key] = _Row(("dual", _token(key), 0), _nonzero(coefficients), "le", cost_j, None)
        rows.append(by_column[key])
    return _Dual(variables, tuple(rows), _nonzero(cost), duals, by_column)


def _distance(
    name: str, function: dict[Key, Fraction], low: Fraction, high: Fraction, two_sided: bool, at: Key | Fraction
) -> list[_Row]:
    """The rows that hold a coordinate of the max-min (addendum 2.1 §2.2 step 4.2) at a normalised distance of at
    least `at` from the ends of its range [low, high] on D*: `at` is a variable (the level, or a probe) or a level.
    A pool's distance is min(f − low, high − f) ÷ (high − low); an activity's value, whose high end is 0, is pushed
    below it only (`two_sided` false): (high − f) ÷ (high − low)."""
    span = high - low
    negated = {key: -value for key, value in function.items()}
    if isinstance(at, Fraction):
        found = [_Row(("level", f"{name} high", 0), dict(function), "le", high - span * at, None)]
        if two_sided:
            found.append(_Row(("level", f"{name} low", 0), negated, "le", -low - span * at, None))
        return found
    found = [_Row(("level", f"{name} high", 0), {**function, at: span}, "le", high, None)]
    if two_sided:
        found.append(_Row(("level", f"{name} low", 0), {**negated, at: span}, "le", -low, None))
    return found


# The objective functions (PRICE-2a; #424 ACM-PRICE addendum 2.1 §4.1): each defined once, for the solves and the
# checks alike ---------------------------------------------------------------------------------------------------


def acquirers(model: _Model) -> list[dict[str, Any]]:
    """Each activity that acquires a main goal item, by id: it brings one in (it produces it, or moves into it).
    Its record holds `goals` (those items), `bounds` (the limits and income caps that count it), `inputs` (what it
    takes from: a non-zero amount it consumes, an entity it moves out of) and `spends`, the inputs that are not
    time. An input is time when it is a resource or store with a regen (one that brings something) that is not a
    slot, or a slot that only acquisitions take from. The activity is protected, in P, when a bound counts it and it
    spends nothing (`protects`); otherwise it is a conversion, compared inside V like any other activity."""
    worth = {resource for _, resource, _ in model.goals}
    brought: dict[str, set[str]] = {}
    for (activity, resource), amount in model.income.items():
        if resource in worth and amount > 0:
            brought.setdefault(activity, set()).add(resource)
    for _, activity, _, target in model.moves:
        if target in worth:
            brought.setdefault(activity, set()).add(target)
    bounds: dict[str, set[str]] = {}
    for rule, members, _, _ in model.limits:
        for activity in members:
            bounds.setdefault(activity, set()).add(rule)
    for cap, _, members, _, _ in model.incomes:
        for activity in members:
            bounds.setdefault(activity, set()).add(cap)
    regenerated = {target for _, target, incomes in model.regens if any(amount > 0 for amount in incomes)}
    takers: dict[str, set[str]] = {}  # entity -> the activities that take from it
    for activity, inputs in model.inputs.items():
        for entity in inputs:
            takers.setdefault(entity, set()).add(activity)

    def time(entity: str) -> bool:
        if entity in model.slots:
            return takers[entity] <= set(brought)
        return entity in regenerated

    found = []
    for activity in sorted(brought):
        inputs = list(model.inputs.get(activity, ()))
        found.append({
            "activity": activity, "goals": sorted(brought[activity]), "bounds": sorted(bounds.get(activity, ())),
            "inputs": inputs, "spends": [entity for entity in inputs if not time(entity)],
        })
    return found


def protects(record: dict[str, Any]) -> bool:
    """Whether an acquisition (`acquirers`) is protected: in P (§4.1)."""
    return bool(record["bounds"]) and not record["spends"]


def acquisition(model: _Model) -> dict[Key, Fraction]:
    """A, the protected acquisition (§4.1): over the cycle, what the activities of P bring in of each main goal item
    (net of what they take of it), at its value; less what each income cap on such an item that counts an activity
    of P voids, at that value; and the `main` of each demand row's executions. Its terms; none when nothing counts."""
    worth = {resource: weight for _, resource, weight in model.goals}
    protected = {record["activity"] for record in acquirers(model) if protects(record)}
    found: dict[Key, Fraction] = {}
    for step in range(model.steps):
        for activity in sorted(protected):
            for resource, weight in worth.items():
                amount = model.net.get((activity, resource))
                if amount:
                    _add(found, ("x", activity, step), weight * amount)
        for flow, activity, _, target in model.moves:
            if activity in protected and target in worth:
                _add(found, ("m", flow, step), worth[target])
        for cap, resource, members, _, _ in model.incomes:
            if resource in worth and protected & set(members):
                _add(found, ("v", cap, step), -worth[resource])
        for row in model.demand:
            if row.main:
                _add(found, ("q", row.key, step), row.main)
    return _nonzero(found)


def total_value(model: _Model) -> dict[Key, Fraction]:
    """V, the value of the goal sinks over the cycle (§4.1): each main goal item it takes at its value, and each
    demand row's executions at `points × value + main`. Its terms."""
    found: dict[Key, Fraction] = {}
    for step in range(model.steps):
        for rule, _, weight in model.goals:
            _add(found, ("g", rule, step), weight)
        for row in model.demand:
            _add(found, ("q", row.key, step), row.value)
    return _nonzero(found)


# The solver ----------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _Solution:
    status: str  # optimal, infeasible, or unproven (a MILP stopped with an incumbent)
    x: dict[Key, Fraction] | None = None
    value: Fraction | None = None
    eq: dict[Key, float] | None = None  # HiGHS's marginals of the equality rows
    le: dict[Key, float] | None = None  # and of the other rows
    nodes: int = 0
    raw_status: int = 0
    # PRICE-2c: an LP's optimum is certified when x is the exactly optimal vertex of a basis (`core.exact`); else
    # why not (`refusal`: an `exact` reason, or `no_basis`) and after how many exact pivots
    certified: bool = False
    refusal: str | None = None
    pivots: int = 0
    # a certified optimum's program as `core.exact` read it, its vertex and the arrays (the order of its rows)
    basis: tuple[Any, Any, Any] | None = None
    solver: Any = None  # PRICE-2d: S3's HiGHS instance (`lp` with `keep`), for its re-solves at ±grain (`_Grained`)


@dataclass(frozen=True)
class _Arrays:
    columns: list[Key]
    rows: list[_Row]
    eq: list[_Row]
    le: list[_Row]
    c: Any
    a_eq: Any
    b_eq: Any
    a_le: Any
    b_le: Any
    a_all: Any
    lower: list[float]
    upper: list[float | None]
    integer: list[int]


def _float(number: Fraction) -> float:
    """float(number), as `numbers.Rational` computes it (the integers' true division), without its dispatch."""
    return number.numerator / number.denominator


def _matrix(selected: list[_Row], columns: list[Key]) -> Any:
    """Rows as a sparse matrix over `columns`; None for no rows (the solver is given none)."""
    import numpy as np
    from scipy import sparse

    if not selected:
        return None
    index = {key: position for position, key in enumerate(columns)}
    data, row_index, column_index = [], [], []
    for position, row in enumerate(selected):
        for key in sorted(row.coefficients):
            data.append(_float(row.coefficients[key]))
            row_index.append(position)
            column_index.append(index[key])
    places = (np.array(row_index, dtype=np.int64), np.array(column_index, dtype=np.int64))
    return sparse.csr_array((np.array(data, dtype=np.float64), places), shape=(len(selected), len(columns)))


def _costs(program: _Program, columns: list[Key]) -> Any:
    """The objective over `columns`, negated (scipy minimises)."""
    import numpy as np

    index = {key: position for position, key in enumerate(columns)}
    c = np.zeros(len(columns), dtype=np.float64)
    for key, weight in program.objective.items():
        c[index[key]] = -_float(weight)
    return c


def _arrays(program: _Program) -> _Arrays:
    """The program as scipy wants it: sparse rows in canonical order, columns by variable id, and the objective
    negated (scipy minimises). All the rows in one matrix (`a_all`) only for a MILP, which reads them so."""
    import numpy as np

    columns = sorted(program.variables)
    rows = sorted(program.rows, key=lambda row: row.key)
    eq = [row for row in rows if row.sense == "eq"]
    le = [row for row in rows if row.sense == "le"]
    bounds = [program.variables[key] for key in columns]
    integer = [1 if integer else 0 for _, _, integer in bounds]
    return _Arrays(
        columns=columns, rows=rows, eq=eq, le=le, c=_costs(program, columns),
        a_eq=_matrix(eq, columns), b_eq=np.array([_float(row.rhs) for row in eq], dtype=np.float64),
        a_le=_matrix(le, columns), b_le=np.array([_float(row.rhs) for row in le], dtype=np.float64),
        a_all=_matrix(rows, columns) if any(integer) else None,
        lower=[_float(low) for low, _, _ in bounds],
        upper=[None if high is None else _float(high) for _, high, _ in bounds],
        integer=integer,
    )


def _violation(program: _Program, x: dict[Key, Fraction]) -> dict[str, str] | None:
    """The first bound, integrality or row (in canonical order) that `x` breaks exactly, or None."""
    for key in sorted(program.variables):
        low, high, integer = program.variables[key]
        value = x[key]
        if value < low:
            return {"constraint": _token(key), "found": str(value), "bound": f">={low}"}
        if high is not None and value > high:
            return {"constraint": _token(key), "found": str(value), "bound": f"<={high}"}
        if integer and value.denominator != 1:
            return {"constraint": _token(key), "found": str(value), "bound": "integer"}
    for row in sorted(program.rows, key=lambda row: row.key):
        lhs = sum((value * x[key] for key, value in row.coefficients.items() if x[key]), Fraction(0))  # 0s add nothing
        if (lhs != row.rhs) if row.sense == "eq" else (lhs > row.rhs):
            sense = "=" if row.sense == "eq" else "<="
            return {"constraint": _token(row.key), "found": str(lhs), "bound": f"{sense}{row.rhs}"}
    return None


def _close(number: float, target: Fraction) -> bool:
    """Whether a solver's float stands for `target`, at the solvers' feasibility tolerance scale."""
    return abs(number - float(target)) <= 1e-6 * (1 + abs(float(target)))


def _vertex(program: _Program, floats: dict[Key, float]) -> dict[Key, Fraction] | None:
    """The exact vertex a solution stands for: the variables at a bound, and the integer ones rounded, fixed;
    the others solved exactly from the rows they leave tight. None when those rows do not determine them."""
    fixed: dict[Key, Fraction] = {}
    free: list[Key] = []
    for key in sorted(program.variables):
        low, high, integer = program.variables[key]
        value = floats[key]
        if integer:
            fixed[key] = Fraction(round(value))
        elif _close(value, low):
            fixed[key] = low
        elif high is not None and _close(value, high):
            fixed[key] = high
        else:
            free.append(key)
    system = []
    for row in sorted(program.rows, key=lambda row: row.key):
        lhs = sum(float(value) * floats[key] for key, value in row.coefficients.items())
        if row.sense == "eq" or _close(lhs, row.rhs):
            known = (value * fixed[key] for key, value in row.coefficients.items() if key in fixed)
            rest = row.rhs - sum(known, Fraction(0))
            system.append(({key: value for key, value in row.coefficients.items() if key not in fixed}, rest))
    solved = _solve_exact(system, free)
    return None if solved is None else {**fixed, **solved}


def _solve_exact(system: list[tuple[dict[Key, Fraction], Fraction]], unknowns: list[Key]) -> dict[Key, Fraction] | None:
    """The one exact solution of a linear system by Gauss-Jordan elimination over fractions; None when it has
    none or more than one."""
    pivots: dict[Key, tuple[dict[Key, Fraction], Fraction]] = {}  # key -> its row, reduced, coefficient 1
    for coefficients, rhs in system:
        row, value = dict(coefficients), rhs
        for key in [key for key in row if key in pivots]:
            factor = row.pop(key)
            pivot_row, pivot_value = pivots[key]
            for other, coefficient in pivot_row.items():
                if other != key:
                    row[other] = row.get(other, Fraction(0)) - factor * coefficient
            value -= factor * pivot_value
        row = {key: coefficient for key, coefficient in row.items() if coefficient != 0}
        if not row:
            if value != 0:
                return None
            continue
        key = min(row)
        scale = row[key]
        row = {other: coefficient / scale for other, coefficient in row.items()}
        value /= scale
        for other_key, (other_row, other_value) in list(pivots.items()):
            factor = other_row.get(key)
            if factor:
                reduced = {k: c - factor * row.get(k, Fraction(0)) for k, c in other_row.items() if k != key}
                for k, c in row.items():
                    if k not in other_row and k != key:
                        reduced[k] = -factor * c
                reduced[other_key] = Fraction(1)
                pivots[other_key] = ({k: c for k, c in reduced.items() if c != 0}, other_value - factor * value)
        pivots[key] = (row, value)
    if set(pivots) != set(unknowns):
        return None
    found = {}
    for key, (row, value) in pivots.items():
        if any(other != key for other in row):
            return None
        found[key] = value
    return found


def _marginals(rows: list[_Row], values: Any) -> dict[Key, float]:
    return {row.key: float(value) for row, value in zip(rows, values, strict=True)}


@dataclass(frozen=True)
class _Run:
    """One HiGHS run of an LP (`_highs`): its status as scipy's `linprog` maps HiGHS's model status (0 optimal, 1 a
    limit, 2 infeasible, 3 unbounded, 4 anything else) with HiGHS's text, and for an optimum the column values, the row
    duals (scipy's marginals; the at-most rows first, then the equalities) and the basis statuses (the columns, then
    the rows in that order; None when HiGHS holds no valid basis)."""

    status: int
    message: str
    x: Any = None
    duals: Any = None
    statuses: tuple[int, ...] | None = None
    solver: Any = None  # the HiGHS instance that holds the LP, for a warm start (`_highs_again`)


def _highs(arrays: _Arrays, presolve: bool) -> _Run:
    """An LP solved by HiGHS as `scipy.optimize.linprog(method="highs")` solves it (the same model: the at-most rows,
    then the equalities; the same options), through the HiGHS bindings scipy bundles and `linprog` calls
    (`scipy.optimize._highspy._core`; scipy is pinned in requirements.txt), so that the basis statuses can be read too
    (PRICE-2c): the exact vertex is rebuilt from them (`core.exact`)."""
    import numpy as np
    from scipy import sparse
    from scipy.optimize._highspy import _core as highs

    n = len(arrays.columns)
    blocks = [block for block in (arrays.a_le, arrays.a_eq) if block is not None]
    matrix = sparse.csc_array(sparse.vstack(blocks) if blocks else sparse.csc_array((0, n), dtype=np.float64))
    infinity = highs.kHighsInf
    row_lower = np.concatenate((np.full(len(arrays.le), -infinity), arrays.b_eq)).astype(np.float64)
    row_upper = np.concatenate((arrays.b_le, arrays.b_eq)).astype(np.float64)
    lp = highs.HighsLp()
    lp.num_col_ = n
    lp.num_row_ = matrix.shape[0]
    lp.a_matrix_.num_col_ = n
    lp.a_matrix_.num_row_ = matrix.shape[0]
    lp.a_matrix_.format_ = highs.MatrixFormat.kColwise
    lp.col_cost_ = arrays.c
    lp.col_lower_ = np.array(arrays.lower, dtype=np.float64)
    lp.col_upper_ = np.array([infinity if high is None else high for high in arrays.upper], dtype=np.float64)
    lp.row_lower_ = row_lower
    lp.row_upper_ = row_upper
    lp.a_matrix_.start_ = matrix.indptr
    lp.a_matrix_.index_ = matrix.indices
    lp.a_matrix_.value_ = matrix.data
    solver = highs._Highs()
    if solver.passOptions(_options(presolve, DUAL_SIMPLEX)) == highs.HighsStatus.kError:
        return _Run(4, "passOptions")
    if solver.passModel(lp) == highs.HighsStatus.kError:
        return _Run(2, solver.modelStatusToString(highs.HighsModelStatus.kModelError))
    return _highs_run(solver, n, matrix.shape[0])


def _options(presolve: bool, strategy: int) -> Any:
    """HiGHS's options as `linprog` sets them (no output, no debugging), with presolve and a simplex strategy."""
    from scipy.optimize._highspy import _core as highs

    options = highs.HighsOptions()
    options.presolve = "on" if presolve else "off"
    options.output_flag = False
    options.log_to_console = False
    options.highs_debug_level = 0  # kHighsDebugLevelNone, as linprog sets it
    options.simplex_strategy = strategy
    return options


def _highs_again(solver: Any, arrays: _Arrays) -> _Run:
    """The LP that HiGHS instance `solver` holds, with the objective of `arrays` instead (PRICE-2c): the same rows and
    bounds, so the basis of its last solve is still primal feasible, and HiGHS's primal simplex goes on from it,
    without presolve (which would set the basis aside), instead of solving from scratch."""
    import numpy as np
    from scipy.optimize._highspy import _core as highs

    n = len(arrays.columns)
    if solver.passOptions(_options(False, PRIMAL_SIMPLEX)) == highs.HighsStatus.kError:
        return _Run(4, "passOptions")
    solver.changeColsCost(n, np.arange(n, dtype=np.int32), np.asarray(arrays.c, dtype=np.float64))
    return _highs_run(solver, n, len(arrays.le) + len(arrays.eq))


def _highs_run(solver: Any, n: int, m: int) -> _Run:
    """Run HiGHS on the LP it holds (`_highs`) and read its status, solution and basis."""
    import numpy as np
    from scipy.optimize._highspy import _core as highs

    solver.run()
    found = solver.getModelStatus()
    text = solver.modelStatusToString(found)
    statuses = {
        highs.HighsModelStatus.kOptimal: 0, highs.HighsModelStatus.kTimeLimit: 1,
        highs.HighsModelStatus.kIterationLimit: 1, highs.HighsModelStatus.kInfeasible: 2,
        highs.HighsModelStatus.kModelError: 2, highs.HighsModelStatus.kUnbounded: 3,
    }
    status = statuses.get(found, 4)
    if status != 0:
        return _Run(status, text, solver=solver)
    solution, basis = solver.getSolution(), solver.getBasis()
    columns = [int(value) for value in basis.col_status]
    rows = [int(value) for value in basis.row_status]
    valid = bool(getattr(basis, "valid", True)) and len(columns) == n and len(rows) == m
    return _Run(
        0, text, np.array(solution.col_value, dtype=np.float64), np.array(solution.row_dual, dtype=np.float64),
        tuple(columns + rows) if valid else None, solver,
    )


def _highs_moved(
    solver: Any, saved: Any, ends: dict[int, tuple[float, float]], restore: dict[int, tuple[float, float]], n: int, m: int
) -> tuple[_Run, bool]:
    """S3's LP, which HiGHS instance `solver` holds, with the rows `ends` (by place: the at-most rows, then the
    equalities) given those ends instead (PRICE-2d, a re-solve at ±grain): from S3's basis `saved`, still dual feasible
    since no cost changed, HiGHS's dual simplex goes on without presolve (which would set the basis aside). Then the rows
    get their ends `restore` back, so that the instance holds S3's LP again; and whether that worked."""
    from scipy.optimize._highspy import _core as highs

    failed = highs.HighsStatus.kError
    run: _Run | None = None
    for place, (low, high) in ends.items():
        if solver.changeRowBounds(place, low, high) == failed:
            run = _Run(4, "changeRowBounds")
            break
    if run is None and solver.setBasis(saved) == failed:
        run = _Run(4, "setBasis")
    if run is None and solver.passOptions(_options(False, DUAL_SIMPLEX)) == failed:
        run = _Run(4, "passOptions")
    if run is None:
        run = _highs_run(solver, n, m)
    restored = [solver.changeRowBounds(place, low, high) != failed for place, (low, high) in restore.items()]
    return run, all(restored)


def _exact_problem(program: _Program, arrays: _Arrays) -> exact.Problem:
    """The program as `core.exact` reads it, in the order HiGHS was given it (`_highs`): the columns by id, the
    at-most rows and then the equalities, and the objective maximised."""
    index = {key: position for position, key in enumerate(arrays.columns)}
    ordered = (*arrays.le, *arrays.eq)
    lower: list[Fraction | None] = [program.variables[key][0] for key in arrays.columns]
    upper: list[Fraction | None] = [program.variables[key][1] for key in arrays.columns]
    lower += [None if row.sense == "le" else row.rhs for row in ordered]
    upper += [row.rhs for row in ordered]
    rows = tuple({index[key]: value for key, value in row.coefficients.items()} for row in ordered)
    cost = tuple(program.objective.get(key, Fraction(0)) for key in arrays.columns)
    return exact.Problem(cost, tuple(lower), tuple(upper), rows)


@dataclass(frozen=True)
class _Grained:
    """S3's certified optimum, held for its re-solves at plus and minus one grain (PRICE-2d; `_Solver.regrain`): its
    program and solution, its HiGHS instance and the basis HiGHS ended at, its program as `core.exact` reads it with its
    basis factorised (`exact.factored`), and its rows in HiGHS's order (the at-most rows, then the equalities): their
    places by key, their float ends as HiGHS was given them, and for each column the places of the rows that hold it."""

    program: _Program
    base: _Solution
    solver: Any
    saved: Any
    arrays: _Arrays
    factored: exact.Factored
    ordered: tuple[_Row, ...]
    places: dict[Key, int]
    ends: tuple[tuple[float, float], ...]
    holders: dict[Key, tuple[int, ...]]
    factorised: float = 0.0  # the seconds S3's basis took to factorise (`_Solver.hold`)

    def moved(self, moves: dict[int, Fraction]) -> exact.Problem:
        """S3's program as `core.exact` reads it, with the ends of the rows `moves` (by place) moved by those amounts:
        both ends of an equality, the one end of an at-most."""
        problem = self.factored.problem
        n = len(problem.cost)
        lower, upper = list(problem.lower), list(problem.upper)
        for place, amount in moves.items():
            k = n + place
            lower[k] = None if lower[k] is None else lower[k] + amount
            upper[k] = None if upper[k] is None else upper[k] + amount
        return exact.Problem(problem.cost, tuple(lower), tuple(upper), problem.rows)

    def violation(self, x: dict[Key, Fraction], moves: dict[int, Fraction]) -> dict[str, str] | None:
        """`_violation` of x as a solution of S3's program with the right sides of the rows `moves` moved: the first
        bound or row it breaks exactly, in canonical order. The bounds of the variables whose value differs from S3's
        and the rows that hold one of them, or whose right side moved, are each computed again from x; every other bound
        and row is S3's, with the same right side and the same values, which S3's own exact check passed."""
        program, before = self.program, self.base.x
        changed = [key for key in self.arrays.columns if x[key] is not before[key] and x[key] != before[key]]
        for key in sorted(changed):
            low, high, integer = program.variables[key]
            value = x[key]
            if value < low:
                return {"constraint": _token(key), "found": str(value), "bound": f">={low}"}
            if high is not None and value > high:
                return {"constraint": _token(key), "found": str(value), "bound": f"<={high}"}
            if integer and value.denominator != 1:
                return {"constraint": _token(key), "found": str(value), "bound": "integer"}
        places = set(moves)
        for key in changed:
            places.update(self.holders.get(key, ()))
        for place in sorted(places, key=lambda place: self.ordered[place].key):
            row = self.ordered[place]
            rhs = row.rhs + moves.get(place, Fraction(0))
            lhs = sum((value * x[key] for key, value in row.coefficients.items() if x[key]), Fraction(0))
            if (lhs != rhs) if row.sense == "eq" else (lhs > rhs):
                sense = "=" if row.sense == "eq" else "<="
                return {"constraint": _token(row.key), "found": str(lhs), "bound": f"{sense}{rhs}"}
        return None


@dataclass(frozen=True)
class _Unverified:
    """A per-activity probe that could not be verified exactly (PRICE-2c): the detail of its warning
    `lp_probe_unverified` beyond `path`, `probe` and `activity`."""

    detail: dict[str, Any]


def _restricted(dual: _Dual, program: _Program, x: dict[Key, Fraction]) -> tuple[dict[Key, Any], tuple[_Row, ...]]:
    """D*, the optimal face of a program's dual, in the form complementary slackness with an exact optimum x* of the
    program gives it (PRICE-2c): the dual's variables and rows with, for each at-most row that x* leaves slack, its
    dual fixed at 0; for each variable below its upper bound at x*, its `w` fixed at 0; and for each variable above 0
    at x*, its dual row an equality.

    A dual feasible point is optimal exactly when it is complementary to one primal optimum (c·x* ≤ (Aᵀy + w)·x* =
    y·(Ax*) + w·x* ≤ b·y + u·w holds with equality exactly then, `_never`), so these are D*'s points, with no
    strong-duality row: a set HiGHS solves on far faster than the face, whose dense cost row it can take minutes on. x*
    must be an exact optimum: S3's (exactly feasible, and exactly optimal once `duality` has found a dual point of the
    same cost), or for a forcing cost an optimum of S3 that runs the activity its `most` (the solution of its `most`
    probe, exactly feasible on S3's optimal face; S3's own when complementary slackness gave `most` 0). The LPs that
    read only a value on D* (the projections, the intervals, the ranges, the forcing costs) are solved on it; the
    selection and its exact check keep the face."""
    zero = Fraction(0)
    variables: dict[Key, Any] = dict(dual.variables)
    for row in program.rows:
        if row.sense == "le" and sum((a * x[key] for key, a in row.coefficients.items() if x[key]), zero) < row.rhs:
            for key in dual.duals[row.key]:
                variables[key] = (zero, zero, False)
    rows = []
    for key in sorted(program.variables):
        low, high, _ = program.variables[key]
        if high is not None and x[key] < high:
            variables[("w", _token(key), 0)] = (zero, zero, False)
        row = dual.by_column[key]
        rows.append(replace(row, sense="eq") if x[key] > low else row)
    return variables, tuple(rows)


def _one(pair: _Pair) -> bool:
    """Whether an interval is one exact value (its two ends agree)."""
    return pair[0] is not None and pair[0] == pair[1]


def _pinned(model: _Model, resource: str, grain: _Pair) -> bool:
    """Whether a resource's re-solves at plus and minus one grain in a step (`grain`, their chords) pin its projection
    on D* to one value, so that no LP on D* is needed for it (PRICE-2c).

    Let f(δ) be the optimum with δ more income of the resource in the step: the program's balance row of that step has
    its right side raised by δ and nothing else changes, so f is the optimum of a maximum as a function of one right
    side, concave. Its right derivative is the least of that row's dual on D* (the projection's `low`) and its left
    derivative the greatest (`high`), and a concave function's chords enclose its derivatives:
        (f(g) − f(0)) / g ≤ f'₊(0) ≤ f'₋(0) ≤ (f(0) − f(−g)) / g.
    So when both chords exist and are equal, all four are: the projection is exactly that value. One grain more also
    raises what a hard cap's overflow may be (the income it blocks), a second right side, so a resource with a hard cap
    never takes this way; nor does a pair of chords that differ, or that lacks one (an end infeasible)."""
    low, high = grain
    return resource not in model.hard and low is not None and low == high


def _never(dual: _Dual, point: dict[Key, Fraction], keys: list[Key]) -> bool:
    """Whether complementary slackness proves that no optimum of a program sets any of `keys` above 0 (PRICE-2c):
    each one's dual slack at an exact optimal point (y, w) of the program's dual (`_dual`, checked by `duality`) is
    above 0.

    The program maximises c·x over its rows (at-most and equalities) with 0 ≤ x ≤ u, and the dual minimises b·y + u·w
    over Aᵀy + w ≥ c, y ≥ 0 on the at-most rows, w ≥ 0. A column's dual slack s_j = (Aᵀy + w − c)_j is its dual row's
    right side less its left side, at least 0. For any feasible x and (y, w):
        c·x ≤ (Aᵀy + w)·x = y·(Ax) + w·x ≤ b·y + u·w,
    the first since x ≥ 0 and s ≥ 0, the second since y·(Ax) ≤ y·b (y ≥ 0 where Ax ≤ b, Ax = b elsewhere) and w·x ≤ w·u.
    At an optimum x* and the optimal (y, w) both ends are the optimum (strong duality: `duality` checks exactly that
    the cost at (y, w) is V0), so the first inequality holds with equality: Σ_j s_j·x*_j = 0, a sum of terms each at
    least 0, each therefore 0. So s_j > 0 gives x*_j = 0 in every optimum x*, and no probe is needed for it. A slack
    of 0 proves nothing (another optimum may still run it): only those are probed."""
    for key in keys:
        row = dual.by_column[key]
        if row.rhs - sum((value * point[name] for name, value in row.coefficients.items()), Fraction(0)) <= 0:
            return False
    return True


@dataclass(frozen=True)
class _Tiers:
    """The steady state's three solves (`_Solver.tiers`; PRICE-2a), which PRICE-2b's dual reads."""

    program: _Program  # S3's: V alone; every price re-solve changes it
    base: _Solution  # S3's optimum: its value V0 is the objective
    acquired: Fraction  # A*, S1's optimum (0 when A has no term)
    second: _Solution  # S2's optimum (S3's when A has no term)
    floors: tuple[_Row, ...]  # S2's added row: A ≥ A*
    face: tuple[_Row, ...]  # the rows of the lexicographic optima: A ≥ A*, V ≥ V*
    records: list[dict[str, Any]]  # the acquisitions (`acquirers`), protected or not
    ties: tuple[str, ...]  # `objective_tier_tie`
    dual: _Dual  # S3's dual (PRICE-2b-1), checked exactly (`duality`)
    point: dict[Key, Fraction]  # an exact optimal point of it: the dual slacks of complementary slackness (PRICE-2c)
    unverified: tuple[dict[str, Any], ...] = ()  # `lp_probe_unverified` of the tie probes (PRICE-2c)


class _Solver:
    def __init__(self, model: _Model, settings: EconomySettings) -> None:
        self.model = model
        self.settings = settings
        # PRICE-2c: the last LP solved with `warm` (its variables and rows, held so that they stay the objects they are;
        # its arrays; the HiGHS instance that holds it), for the next LP on the same rows with another objective
        self.warm: tuple[Any, Any, _Arrays, Any] | None = None
        self.last_problem: exact.Problem | None = None  # the last LP's program as `core.exact` read it (`vertex`)
        self.grained: _Grained | None = None  # PRICE-2d: S3, held for its re-solves at ±grain (`regrain`)
        # PRICE-2d: the most variables an update may let enter S3's basis, and the seconds of the new rebuilds so far
        # (`regrain`: they choose the exact way only, never a value)
        self.update_most = 0
        self.rebuilds: list[float] = []
        # PRICE-2d: how each side of a resource's re-solves at ±grain was found (`grain_way`); a diagnosis, never output
        self.grain_ways: dict[str, int] = {}

    def failure(self, code: str, **detail: Any) -> Failure:
        return Failure(error(code, {"path": self.model.path, "solve": self.model.solve, **detail}))

    def exact(self, program: _Program, columns: list[Key], raw: Any, stage: str) -> dict[Key, Fraction]:
        """The solution as exact fractions, checked exactly against every bound, integrality and row: a MILP's, and
        an LP's whose basis gives no certified vertex (`vertex`, PRICE-2c).

        First the vertex itself: the variables at a bound (and the integer ones, rounded) are fixed, and the
        rows they leave tight are solved exactly for the others. When that system does not determine them, or its
        solution breaks the check, each value is converted with `limit_denominator(lp.max_denominator)` instead.
        What still breaks the check is `lp_numeric_check_failed`; nothing is rounded into place."""
        floats: dict[Key, float] = {}
        for key, number in zip(columns, raw, strict=True):
            number = float(number)
            if not math.isfinite(number):
                raise self.failure(
                    LP_NUMERIC_CHECK_FAILED, stage=stage, constraint=_token(key), found=repr(number), bound="finite"
                )
            floats[key] = number
        vertex = _vertex(program, floats)
        if vertex is not None and _violation(program, vertex) is None:
            return vertex
        denominator = self.settings.max_denominator
        x = {key: Fraction(value).limit_denominator(denominator) for key, value in floats.items()}
        problem = _violation(program, x)
        if problem is not None:
            raise self.failure(LP_NUMERIC_CHECK_FAILED, stage=stage, **problem)
        return x

    def objective(self, program: _Program, x: dict[Key, Fraction]) -> Fraction:
        return sum((weight * x[key] for key, weight in program.objective.items()), Fraction(0))

    def vertex(self, program: _Program, arrays: _Arrays, run: _Run, stage: str) -> tuple[dict[Key, Fraction], Any]:
        """An LP's optimum as exact fractions (PRICE-2c), and how it was found: an `exact.Vertex` when HiGHS's basis
        (with at most `lp.max_pivots` exact pivots) gives the exactly optimal vertex, which is then checked exactly
        against every bound and row as every solution is; else the reason it does not (an `exact.Refused`, or None for
        no basis), and the solution rebuilt and checked as before (`exact`): feasible, but not certified optimal."""
        found = None
        if run.statuses is not None:
            self.last_problem = _exact_problem(program, arrays)
            found = exact.optimum(self.last_problem, run.statuses, self.settings.max_pivots)
            if isinstance(found, exact.Vertex):
                x = dict(zip(arrays.columns, found.x, strict=True))
                problem = _violation(program, x)
                if problem is not None:  # a certified vertex meets every row by construction: a defect
                    raise self.failure(LP_NUMERIC_CHECK_FAILED, stage=stage, **problem)
                return x, found
        return self.exact(program, arrays.columns, run.x, stage), found

    def lp(
        self, program: _Program, stage: str, *, check: bool = True, unbounded: bool = False, warm: bool = False,
        keep: bool = False,
    ) -> _Solution:
        """An LP, solved by HiGHS (`_highs`) and made exact from its basis (`vertex`). With `unbounded`, an objective
        without a maximum is the status `unbounded` (an end of a projection that is null, PRICE-2b-1); else it is
        `lp_solve_failed`. With `keep` (S3, PRICE-2d), the solution holds the HiGHS instance that solved it.

        With `warm` (the LPs on one face of the dual, which differ only in their objective: the projections, the
        intervals, the ranges; PRICE-2c), an LP whose variables and rows are those of the last such LP, the same
        objects, is given to the HiGHS instance that solved it, with its objective changed, so HiGHS starts from the
        basis it ended at. Only where HiGHS starts differs: the exact vertex and its certificate are the new LP's own."""
        result = None
        if warm and self.warm is not None and self.warm[0] is program.variables and self.warm[1] is program.rows:
            arrays = replace(self.warm[2], c=_costs(program, self.warm[2].columns))
            result = _highs_again(self.warm[3], arrays)
        else:
            arrays = _arrays(program)
            result = _highs(arrays, True)
            if warm and result.solver is not None:
                self.warm = (program.variables, program.rows, arrays, result.solver)
        if result.status not in (0, 2, 3):  # "unbounded or infeasible" from presolve, or numerical trouble
            result = _highs(arrays, False)
        if result.status == 2:
            return _Solution("infeasible")
        if result.status == 3 and unbounded:
            return _Solution("unbounded")
        if result.status == 3:
            raise self.failure(LP_SOLVE_FAILED, stage=stage, reason="unbounded", status=3)
        if result.status != 0:
            raise self.failure(
                LP_SOLVE_FAILED, stage=stage, reason="solver_status", status=int(result.status), raw_text=result.message
            )
        if not check:
            return _Solution("optimal")
        x, found = self.vertex(program, arrays, result, stage)
        duals = list(result.duals)
        le = _marginals(arrays.le, duals[: len(arrays.le)])
        eq = _marginals(arrays.eq, duals[len(arrays.le):])
        certified = isinstance(found, exact.Vertex)
        return _Solution(
            "optimal", x, self.objective(program, x), eq, le, certified=certified,
            refusal=None if certified else (found.reason if found is not None else NO_BASIS),
            pivots=0 if found is None else found.pivots,
            basis=(self.last_problem, found, arrays) if certified else None,
            solver=result.solver if keep else None,
        )

    def probe(
        self, program: _Program, stage: str, *, unbounded: bool = False, warm: bool = False
    ) -> _Solution | _Unverified:
        """The LP of a per-activity probe (PRICE-2c): its solution, certified or not (`lp`), or `_Unverified` when it
        could not be solved and checked (`solve_failed`, with the error it raised: what blocked the whole steady state
        before PRICE-2c), or HiGHS finds the probe infeasible although an optimum meets it."""
        try:
            found = self.lp(program, stage, unbounded=unbounded, warm=warm)
        except Failure as failure:
            first = failure.errors[0]
            detail: dict[str, Any] = {"reason": SOLVE_FAILED, "stage": stage, "error": first["code"]}
            for name in ("constraint", "found", "bound"):
                if name in first["detail"]:
                    detail[name] = str(first["detail"][name])
            return _Unverified(detail)
        if found.status == "infeasible":
            return _Unverified({"reason": SOLVE_FAILED, "stage": stage, "error": LP_SOLVE_FAILED})
        return found

    def certain(self, found: _Solution | _Unverified, stage: str) -> _Solution | _Unverified:
        """A probe's answer when it is exact: an unbounded status, or a certified optimum; else `_Unverified`, with why
        its basis gave no certificate."""
        if isinstance(found, _Unverified) or found.status == "unbounded" or found.certified:
            return found
        detail: dict[str, Any] = {"reason": found.refusal or NO_BASIS, "stage": stage}
        if found.pivots:
            detail["pivots"] = found.pivots
        return _Unverified(detail)

    def unverified(self, probe: str, activity: str, found: _Unverified) -> dict[str, Any]:
        """The warning `lp_probe_unverified` of one probe of an activity (PRICE-2c)."""
        return error(LP_PROBE_UNVERIFIED, {"path": self.model.path, "probe": probe, "activity": activity, **found.detail})

    def milp(self, program: _Program, stage: str, *, check: bool = True) -> _Solution:
        """A MILP within `search.milp_node_limit` nodes. HiGHS proves optimality with a relative gap of 0; a
        stop before that with an incumbent is `unproven`."""
        import numpy as np
        from scipy.optimize import Bounds, LinearConstraint, milp

        arrays = _arrays(program)
        limit = self.settings.node_limit

        def call(presolve: bool) -> Any:
            options: dict[str, Any] = {"node_limit": limit, "mip_rel_gap": 0.0, "disp": False}
            if not presolve:
                options["presolve"] = False
            lower = np.array(arrays.lower, dtype=np.float64)
            upper = np.array([np.inf if high is None else high for high in arrays.upper], dtype=np.float64)
            constraints = None
            if arrays.rows:
                low = [float(row.rhs) if row.sense == "eq" else -np.inf for row in arrays.rows]
                high = [float(row.rhs) for row in arrays.rows]
                low, high = np.array(low, dtype=np.float64), np.array(high, dtype=np.float64)
                every = arrays.a_all if arrays.a_all is not None else _matrix(arrays.rows, arrays.columns)
                constraints = LinearConstraint(every, low, high)
            integrality = np.array(arrays.integer, dtype=np.int64)
            bounds = Bounds(lower, upper)
            return milp(arrays.c, integrality=integrality, bounds=bounds, constraints=constraints, options=options)

        def incumbent(result: Any) -> bool:
            return result.x is not None and all(math.isfinite(float(value)) for value in result.x)

        result = call(True)
        if result.status == 4 and not incumbent(result):  # "unbounded or infeasible" from presolve
            result = call(False)
        nodes = int(getattr(result, "mip_node_count", None) or 0)
        if result.status == 2:
            return _Solution("infeasible", nodes=nodes)
        if result.status == 3:
            raise self.failure(LP_SOLVE_FAILED, stage=stage, reason="unbounded", status=3)
        if result.status != 0 and not incumbent(result):
            raise self.failure(
                LP_SOLVE_FAILED, stage=stage, reason="no_solution", status=int(result.status), nodes=nodes,
                node_limit=limit, raw_text=str(result.message),
            )
        status = "optimal" if result.status == 0 else "unproven"
        if not check:
            return _Solution(status, nodes=nodes, raw_status=int(result.status))
        x = self.exact(program, arrays.columns, result.x, stage)
        return _Solution(status, x, self.objective(program, x), nodes=nodes, raw_status=int(result.status))

    def solve(self, program: _Program, stage: str, **options: Any) -> _Solution:
        return self.milp(program, stage, **options) if self.model.solve == PLAN else self.lp(program, stage, **options)

    def feasible(self, program: _Program) -> bool | None:
        """Whether a program has a solution; None when that could not be decided (the node limit, numerics)."""
        try:
            solution = self.solve(program, "filter", check=False)
        except Failure:
            return None
        return solution.status != "infeasible"

    def conflict(self) -> dict[str, Any]:
        """`lp_infeasible` with a minimal set of conflicting groups: each group is dropped in turn, in id order,
        and stays dropped while the rest is still infeasible. A group whose check could not be decided is kept
        and listed in `unchecked`."""
        model = self.model
        dropped: set[str] = set()
        unchecked: list[str] = []
        for group in model.groups():
            verdict = self.feasible(_program(model, omit=frozenset(dropped | {group}), objective={}))
            if verdict is False:
                dropped.add(group)
            elif verdict is None:
                unchecked.append(group)
        kept = [group for group in model.groups() if group not in dropped]
        detail: dict[str, Any] = {
            "path": model.path, "solve": model.solve,
            "rules": [group for group in kept if not group.startswith(READING_GROUP)],
        }
        readings = [group[len(READING_GROUP):] for group in kept if group.startswith(READING_GROUP)]
        if readings:
            detail["readings"] = readings
        if unchecked:
            detail["unchecked"] = unchecked
        return error(LP_INFEASIBLE, detail)

    def price(
        self, optimum: Fraction, resolve: Callable[[Fraction], Fraction | None], known: _Pair = (None, None)
    ) -> _Pair:
        """The value of one more grain (the optimum re-solved with +grain, `resolve(grain)`) and of one grain less
        (`resolve(-grain)`), each per unit; `resolve` gives None for a re-solve that is infeasible. Equal, they are an
        exact price; else an interval, unbounded above when one grain less is infeasible and below when one grain more
        is. One grain more of a resource is infeasible where the cycle cannot hold it: a resource whose regen stops at a
        cap ends the cycle with exactly its opening stock and loses at most what its regen brings, so when the program
        can use no more of it and already loses all its regen brings (its only users left out as pending, say), there
        is no room for one more. A cap's or a limit's value one grain higher only relaxes the program. `known` holds a
        side that S3's basis already proves (`chords`, PRICE-2c), which is then not re-solved."""
        grain = self.model.grain
        right, left = known
        if right is None:
            more = resolve(grain)
            right = None if more is None else (more - optimum) / grain
        if left is None:
            less = resolve(-grain)
            left = None if less is None else (optimum - less) / grain
        if right is None or left is None:
            return right, left
        return min(right, left), max(right, left)

    def resolved(self, program: _Program) -> Fraction | None:
        """The optimum of a price re-solve, a new LP (`lp`); None when it is infeasible."""
        found = self.lp(program, "price")
        return found.value if found.status == "optimal" else None

    def grain_way(self, way: str) -> None:
        self.grain_ways[way] = self.grain_ways.get(way, 0) + 1

    def hold(self, program: _Program, base: _Solution) -> _Grained | None:
        """S3, held for its re-solves at ±grain (`_Grained`, PRICE-2d), with the time its basis took to factorise (the
        first estimate of a new rebuild's, `regrain`); None when its optimum is not certified (no basis to start from:
        each re-solve is then a new LP, as before)."""
        if base.basis is None:
            return None
        problem, vertex, arrays = base.basis
        started = time.perf_counter()
        factored = exact.factored(problem, vertex)
        factorised = time.perf_counter() - started
        if factored is None:
            return None
        n = len(problem.cost)
        self.update_most = len(problem.rows)
        ordered = (*arrays.le, *arrays.eq)
        holders: dict[Key, list[int]] = {}
        for place, row in enumerate(ordered):
            for key in row.coefficients:
                holders.setdefault(key, []).append(place)
        infinity = math.inf
        ends = tuple(
            (-infinity if problem.lower[n + place] is None else _float(problem.lower[n + place]),
             infinity if problem.upper[n + place] is None else _float(problem.upper[n + place]))
            for place in range(len(ordered))
        )
        saved = None if base.solver is None else base.solver.getBasis()
        return _Grained(
            program, base, base.solver, saved, arrays, factored, ordered, {row.key: i for i, row in enumerate(ordered)},
            ends, {key: tuple(places) for key, places in holders.items()}, factorised,
        )

    def grain_moves(self, resource: str, step: int, delta: Fraction) -> dict[int, Fraction] | None:
        """The rows of S3's program, by place, whose right side `delta` more income of `resource` in `step` moves, and by
        how much (`_income_moves`); None where it moves more than right sides."""
        moves = _income_moves(self.model, resource, step, delta)
        if moves is None or self.grained is None or any(key not in self.grained.places for key in moves):
            return None
        return {self.grained.places[key]: amount for key, amount in moves.items()}

    def chords(self, resource: str, step: int) -> tuple[_Pair, _Pair]:
        """What S3's certified basis proves of a resource's price in a step without an LP (PRICE-2c, PRICE-2d), each None
        where it does not: the chords of its re-solves at plus and minus one grain, and its projection's ends. When one
        grain more (or less) of income leaves the basis primal feasible, it stays optimal and the optimum moves by
        exactly the moved right sides times their rows' duals in that basis (`exact.shifted`; for a resource without a
        hard cap, its balance row's alone, `exact.ranging`): that is the chord. When a little more (or less) does, the
        optimum is linear on that side with that slope: that is the derivative there, the projection's `low` (or
        `high`), only for a resource without a hard cap, whose grain moves its balance row's right side alone
        (`_pinned`). Each solve is one with the basis factorised once (`exact.factored`)."""
        grained = self.grained
        if grained is None:
            return (None, None), (None, None)
        factored, grain = grained.factored, Fraction(self.model.grain)
        if resource not in self.model.hard:
            row = grained.places[("balance", resource, step)]
            up, down, rises, falls = exact.ranging(factored.problem, factored.vertex, row, grain, factored)
            dual = factored.vertex.duals[row]
            return ((dual if up else None), (dual if down else None)), ((dual if rises else None), (dual if falls else None))
        found: list[Fraction | None] = []
        for delta in (grain, -grain):  # the chords (f(g) − f(0)) / g and (f(0) − f(−g)) / g
            moves = self.grain_moves(resource, step, delta)
            change = None if moves is None else exact.shifted(factored, moves)
            found.append(None if change is None else change / delta)
        return (found[0], found[1]), (None, None)

    def update_budget(self) -> float:
        """The seconds an update of S3's basis may take (`regrain`): what a new rebuild takes, the mean of this steady
        state's so far, at first twice the time S3's basis took to factorise."""
        if self.rebuilds:
            return sum(self.rebuilds) / len(self.rebuilds)
        return 2 * self.grained.factorised if self.grained is not None else 0.0

    def regrain(self, resource: str, step: int, delta: Fraction) -> Fraction | None:
        """The optimum of S3's program with `delta` more income of `resource` in `step` (a re-solve at ±grain,
        PRICE-2d), exact; None when it is infeasible. The same value as a new LP of that program, found from S3:
        - HiGHS's instance of S3 gets only the moved row ends (`_income_moves`) and goes on from S3's basis (dual
          simplex, no presolve: `_highs_moved`);
        - its basis is solved exactly by updating S3's factorised basis (`exact.reoptimum`), and where that does not
          certify it or the basis is far from S3's, by `optimum`'s new elimination with its exact pivots. Far: the
          update stops once it has taken as long as a new rebuild takes (their mean so far, at first twice S3's
          factorisation), and then no later update lets more variables enter than this one had (`update_most`). The
          time chooses the way only: what comes of it is always what `optimum` gives at that basis (a vertex the
          update certifies is that basis's, which `optimum` certifies with no pivot);
        - the vertex is checked exactly against every bound and row it changes (`_Grained.violation`).
        Where HiGHS does not end optimal there (an infeasible re-solve: a new LP decides that, as before), where no
        certified vertex comes of it, and where the income moves more than right sides, the re-solve is a new LP, as
        before (`resolved`)."""
        grained = self.grained
        moves = None if grained is None or grained.solver is None else self.grain_moves(resource, step, delta)
        if moves is not None:
            problem = grained.moved(moves)
            n, m = len(problem.cost), len(problem.rows)
            ends = {
                place: (-math.inf if problem.lower[n + place] is None else _float(problem.lower[n + place]),
                        math.inf if problem.upper[n + place] is None else _float(problem.upper[n + place]))
                for place in moves
            }
            run, restored = _highs_moved(
                grained.solver, grained.saved, ends, {place: grained.ends[place] for place in moves}, n, m
            )
            if not restored:
                raise self.failure(
                    LP_SOLVE_FAILED, stage="price", reason="solver_status", status=4, raw_text="changeRowBounds"
                )
            if run.status == 0 and run.statuses is not None:
                deadline = time.perf_counter() + self.update_budget()

                def stop(entered: int) -> bool:  # the update has taken as long as a new rebuild takes
                    if time.perf_counter() <= deadline:
                        return False
                    self.update_most = min(self.update_most, entered)
                    return True

                vertex: exact.Vertex | exact.Refused | str = exact.reoptimum(
                    problem, grained.factored, run.statuses, self.update_most, stop
                )
                way = "warm"
                if isinstance(vertex, str):  # why the update did not certify it: a new elimination, as any LP's
                    way = f"warm_eliminated_{vertex}"
                    started = time.perf_counter()
                    vertex = exact.optimum(problem, run.statuses, self.settings.max_pivots)
                    self.rebuilds.append(time.perf_counter() - started)
                if isinstance(vertex, exact.Vertex):
                    x = dict(zip(grained.arrays.columns, vertex.x, strict=True))
                    broken = grained.violation(x, moves)
                    if broken is not None:  # a certified vertex meets every row by construction: a defect
                        raise self.failure(LP_NUMERIC_CHECK_FAILED, stage="price", **broken)
                    self.grain_way(way)
                    return self.objective(grained.program, x)
                self.grain_way("new_lp_not_certified")
            else:
                self.grain_way(f"new_lp_status_{run.status}")
        else:
            self.grain_way("new_lp_no_basis" if grained is None or grained.solver is None else "new_lp_moves_more")
        return self.resolved(_program(self.model, extra={(resource, step): delta}))

    def marginal(self, dual: float) -> Fraction:
        if not math.isfinite(dual):
            raise self.failure(
                LP_NUMERIC_CHECK_FAILED, stage="optimum", constraint="marginal", found=repr(dual), bound="finite"
            )
        return Fraction(dual).limit_denominator(self.settings.max_denominator)

    def disagrees(self, target: str, step: int | None, marginal: Fraction, price: _Pair) -> list[dict[str, Any]]:
        """`lp_dual_disagrees` (check `marginal`): HiGHS's marginal outside the price's interval, a resource's
        projection on D* or a rule's re-solves, by more than `MARGINAL_TOLERANCE` relative to the end it passes. The
        price stands."""
        low, high = price
        above_low = low is None or low <= marginal or _near(marginal, low)
        below_high = high is None or marginal <= high or _near(marginal, high)
        if above_low and below_high:
            return []
        detail: dict[str, Any] = {"path": self.model.path, "target": target, "check": "marginal", "marginal": str(marginal)}
        if low is not None:
            detail["low"] = str(low)
        if step is not None:
            detail["step"] = step
        if high is not None:
            detail["high"] = str(high)
        return [error(LP_DUAL_DISAGREES, detail)]

    def outside(self, target: str, step: int, price: _Pair, grain: _Pair) -> list[dict[str, Any]]:
        """`lp_dual_disagrees` (check `grain`; PRICE-2b-1): a resource's projection on D* not inside the interval of
        its re-solves at plus and minus one grain, which holds it whenever the two measure the same income (the
        projection's ends are the exact derivatives, the re-solves' their chords). The projection stands."""
        low, high = price
        grain_low, grain_high = grain
        below = grain_low is None or (low is not None and grain_low <= low)
        above = grain_high is None or (high is not None and high <= grain_high)
        if below and above:
            return []
        detail: dict[str, Any] = {"path": self.model.path, "target": target, "step": step, "check": "grain"}
        for name, value in (("low", low), ("high", high), ("grain_low", grain_low), ("grain_high", grain_high)):
            if value is not None:
                detail[name] = str(value)
        return [error(LP_DUAL_DISAGREES, detail)]

    def extreme(
        self, variables: dict[Key, tuple[Fraction, Fraction | None, bool]], rows: Any, function: dict[Key, Fraction],
        sense: int,
    ) -> Fraction | None:
        """The greatest (`sense` 1) or least (-1) value of a linear function on the solutions of `rows` (D*, or a face
        of it), exact; None when it is unbounded. The rows always have a solution: they hold the dual point that the
        strong-duality check found (`lp_solve_failed`, stage `dual`, otherwise)."""
        program = _Program(variables, tuple(rows), _nonzero({key: sense * value for key, value in function.items()}))
        found = self.lp(program, DUAL, unbounded=True, warm=True)
        if found.status == "unbounded":
            return None
        if found.status != "optimal":
            raise self.failure(LP_SOLVE_FAILED, stage=DUAL, reason="solver_status", status=2)
        return sense * found.value

    def ends(
        self, variables: dict[Key, tuple[Fraction, Fraction | None, bool]], rows: Any, function: dict[Key, Fraction]
    ) -> _Pair:
        """A linear function's least and greatest values on the solutions of `rows`, None where unbounded."""
        return self.extreme(variables, rows, function, -1), self.extreme(variables, rows, function, 1)

    def probed_ends(
        self, variables: dict[Key, tuple[Fraction, Fraction | None, bool]], rows: Any, function: dict[Key, Fraction]
    ) -> _Pair | _Unverified:
        """`ends` as a per-activity probe (an activity's range on D*, PRICE-2c): each end certified or unbounded, else
        `_Unverified` for the first that is not."""
        found: list[Fraction | None] = []
        for sense in (-1, 1):
            program = _Program(variables, tuple(rows), _nonzero({key: sense * value for key, value in function.items()}))
            end = self.certain(self.probe(program, DUAL, unbounded=True, warm=True), DUAL)
            if isinstance(end, _Unverified):
                return end
            found.append(None if end.status == "unbounded" else sense * end.value)
        return found[0], found[1]

    def duality(self, dual: _Dual, optimum: Fraction) -> dict[Key, Fraction]:
        """The strong-duality check (addendum 2.1 §2.2 step 2): the dual's least cost, solved and checked exactly
        against every dual row, must be the primal optimum V0, exactly. Otherwise `lp_numeric_check_failed` (stage
        `dual`, at the strong-duality row), which blocks the steady state: no price is exportable without it. Returns
        that dual point: an exact optimal point of the dual, whose slacks decide by complementary slackness which
        activities no optimum runs (`_never`, PRICE-2c)."""
        found = self.lp(_Program(dual.variables, dual.rows, {key: -value for key, value in dual.cost.items()}), DUAL)
        if found.status != "optimal":  # the primal has an optimum, so its dual has one
            raise self.failure(LP_SOLVE_FAILED, stage=DUAL, reason="solver_status", status=2)
        least = -found.value
        if least != optimum:
            raise self.failure(
                LP_NUMERIC_CHECK_FAILED, stage=DUAL, constraint=_token(FACE), found=str(least), bound=f"={optimum}"
            )
        return found.x

    def max_min(
        self, variables: dict[Key, tuple[Fraction, Fraction | None, bool]], rows: list[_Row],
        coordinates: dict[str, tuple[dict[Key, Fraction], Fraction, Fraction, bool]],
    ) -> tuple[dict[str, Fraction], list[dict[str, Any]]]:
        """The lexicographic max-min of addendum 2.1 §2.2 step 4.2 over K (`coordinates`: id -> its function, the low
        and high ends of its range on D*, and whether it is two-sided, `_distance`), on the solutions of `rows`. A
        stage maximises the level t that every free coordinate's distance reaches; a coordinate whose distance is t
        in every optimum of the stage (its probe, the most it reaches while the others reach t, is t) is fixed at its
        value there, one value by convexity, and its row added to `rows`. Each stage fixes at least one, so it ends;
        none fixed is `lp_numeric_check_failed` (stage `dual`). Returns the fixed values and the stages, each
        `{level, fixed}` (the coordinates it fixed, by id)."""
        zero, one = Fraction(0), Fraction(1)
        free = dict(coordinates)
        fixed: dict[str, Fraction] = {}
        stages: list[dict[str, Any]] = []
        while free:
            names = sorted(free)
            levels = [row for name in names for row in _distance(name, *free[name], LEVEL)]
            stage = _Program({**variables, LEVEL: (zero, None, False)}, (*rows, *levels), {LEVEL: one})
            best = self.optimal(stage, DUAL)
            level = best.value
            newly = []
            for name in names:
                others = [row for other in names if other != name for row in _distance(other, *free[other], level)]
                probe = _Program(
                    {**variables, PROBE: (zero, None, False)}, (*rows, *others, *_distance(name, *free[name], PROBE)),
                    {PROBE: one},
                )
                if self.optimal(probe, DUAL).value <= level:
                    newly.append(name)
            if not newly:
                raise self.failure(
                    LP_NUMERIC_CHECK_FAILED, stage=DUAL, constraint="max_min", found=str(level), bound="fixed"
                )
            for name in newly:
                function = free.pop(name)[0]
                fixed[name] = sum((coefficient * best.x[key] for key, coefficient in function.items()), zero)
                rows.append(_Row(("fix", name, 0), function, "eq", fixed[name], None))
            stages.append({"level": str(level), "fixed": newly})
        return fixed, stages

    def joint(
        self, dual: _Dual, point: dict[Key, Fraction], optimum: Fraction, prices: dict[str, list[_Pair]],
        on_face: tuple[dict[Key, Any], tuple[_Row, ...]], optimal: dict[Key, Fraction],
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        """The joint price vector and the activity judgements (addendum 2.1 §2.2 steps 3-5; ruling 127 iii): the
        steady state's `joint`, and the warnings of its probes. `prices` are the projections per step (`steady`),
        `on_face` D* as the intervals and the ranges read it (`_restricted`), and `optimal` S3's optimum.

        - **The projection.** A pool's coordinate is the mean over the cycle of its balance rows' duals; its
          `interval` is its least and greatest value on D*, an unbounded end null.
        - **The selection**, each an exact LP on D* and the coordinates fixed so far, in this order:
          1. a pool bounded below only (`high` null: one less is infeasible) that a main goal values: its least
             value, the value of one more (its goal's weight), by id (`one_more`);
          2. the max-min over K (`max_min`): the pools of a bounded interval (`low` < `high`), and each activity
             that no optimum runs whose value reaches 0 on D* and no higher, from a finite least value (pushed
             strictly below 0, so that a value of 0 is only an optimum's marginal activity);
          3. a pool whose ends agree has its one value (`exact`);
          then each pool still free, which the selection withholds (`withheld`), by id, at its low end on the face
          left (`one_more`), else at its high end (`one_less`: a pool without a low end, mirroring step 1), else at
          0 (`zero`). Being free through the max-min, a withheld coordinate never moves an exported one. The
          vector is checked exactly as a point of D*.
        - **The judgements** of each activity: its `value` at the vector and its `range` on D* (null for one that
          moves out of or into a store: what it moves depends on what the store holds); `most`, the most an optimum
          of S3 runs it over the cycle (null when unbounded), and `in_optimum`, whether that is above 0;
          `forcing_cost`, the loss per execution of running it more than `most`: the greatest dual of the row
          "its executions at least `most`" on the optimal face of S3 with that row, and `input_bound` when that
          dual is unbounded (nothing is left in the cycle to run it again; `forcing_cost` null). Unbounded `most`
          costs nothing more: 0.
        - **Complementary slackness** (PRICE-2c, `_never`): an activity whose dual slack is above 0 in every step at
          the exact optimal dual `point` runs in no optimum, so its `most` is 0 without a probe.
        - **Constant on D*** (PRICE-2c): a pool whose projection is exact in every step (`_pinned`, or its LPs) is
          constant on D*, its mean too, so its interval is that mean with no LP; and an activity whose value reads
          only such pools has a constant value on D*, its range, with no probe.
        - **Unverified probes** (PRICE-2c): a probe of `most`, `range` or `forcing_cost` whose LP gives no certified
          optimum is `lp_probe_unverified` (returned with the section), its members null and its name listed in the
          activity's `unverified`; an activity whose `most` or `range` is unverified is left out of K (whether it is
          an optimum's is not known), and its component is a structural gap (`core.coverage`). `forcing_cost` reads
          `most`, so it is unverified with it."""
        model, steps, zero, one = self.model, self.model.steps, Fraction(0), Fraction(1)
        variables, face = dual.variables, dual.face(optimum)
        warnings: list[dict[str, Any]] = []
        unverified: dict[str, list[str]] = {activity: [] for activity in model.activities}

        def note(probe: str, activity: str, found: _Unverified) -> None:
            unverified[activity].append(probe)
            warnings.append(self.unverified(probe, activity, found))

        pools: dict[str, dict[Key, Fraction]] = {}
        for resource in model.resources:
            mean: dict[Key, Fraction] = {}
            for step in range(steps):
                for key, sign in dual.duals[("balance", resource, step)].items():
                    _add(mean, key, sign / steps)
            pools[resource] = _nonzero(mean)
        intervals: dict[str, _Pair] = {}
        for resource in model.resources:
            if all(_one(pair) for pair in prices[resource]):  # constant on D* in every step, so is its mean
                mean = sum((low for low, _ in prices[resource]), zero) / steps
                intervals[resource] = (mean, mean)
            else:
                intervals[resource] = self.ends(*on_face, pools[resource])
        floor = _floor(VALUE, total_value(model), optimum)  # S3's optimal face
        most: dict[str, Fraction | None] = {}
        runs: dict[str, dict[Key, Fraction]] = {}  # activity -> its `most` probe's solution
        worth: dict[str, dict[Key, Fraction]] = {}
        ranges: dict[str, _Pair] = {}
        for activity in model.activities:
            counts = {("x", activity, step): one for step in range(steps)}
            if _never(dual, point, list(counts)):  # complementary slackness: no optimum runs it
                most[activity] = zero
            else:
                found = self.certain(
                    self.probe(_program(model, objective=counts, added=floor), JUDGEMENT, unbounded=True), JUDGEMENT
                )
                if isinstance(found, _Unverified):
                    most[activity] = None
                    note(MOST, activity, found)
                else:
                    most[activity] = None if found.status == "unbounded" else found.value
                    if found.status != "unbounded":  # an optimum of S3 that runs it `most`: its forcing cost's x*
                        runs[activity] = found.x
            if model.touches_store(activity):
                continue
            value: dict[Key, Fraction] = {}
            for resource in model.resources:
                amount = model.net.get((activity, resource))
                if amount:
                    for key, coefficient in pools[resource].items():
                        _add(value, key, amount * coefficient)
            worth[activity] = _nonzero(value)
            read = [resource for resource in model.resources if model.net.get((activity, resource))]
            if all(_one(intervals[resource]) for resource in read):
                constant = sum((model.net[(activity, resource)] * intervals[resource][0] for resource in read), zero)
                ranges[activity] = (constant, constant)  # every pool it reads is constant on D*, so is its value
                continue
            ends = self.probed_ends(*on_face, worth[activity])
            if isinstance(ends, _Unverified):
                note(RANGE, activity, ends)
            else:
                ranges[activity] = ends
        rows = list(face)
        chosen: dict[str, tuple[Fraction, str]] = {}
        goals = {resource for _, resource, _ in model.goals}

        def fix(resource: str, value: Fraction | None, by: str) -> None:
            if value is None:  # an end that D* bounds is bounded on every face of it
                raise self.failure(LP_NUMERIC_CHECK_FAILED, stage=DUAL, constraint=PRICE + resource, found="null", bound=by)
            rows.append(_Row(("fix", PRICE + resource, 0), pools[resource], "eq", value, None))
            chosen[resource] = (value, by)

        for resource in model.resources:  # steps 1 and 3
            low, high = intervals[resource]
            if low is not None and low == high:
                chosen[resource] = (low, EXACT)
            elif low is not None and high is None and resource in goals:
                fix(resource, self.extreme(variables, rows, pools[resource], -1), ONE_MORE)
        coordinates: dict[str, tuple[dict[Key, Fraction], Fraction, Fraction, bool]] = {}
        for resource in model.resources:
            low, high = intervals[resource]
            if low is not None and high is not None and low < high:
                coordinates[PRICE + resource] = (pools[resource], low, high, True)
        for activity, (low, high) in ranges.items():
            if MOST in unverified[activity]:  # whether an optimum runs it is not known: it is not in K
                continue
            if most[activity] == 0 and low is not None and high == 0 and low < 0:
                coordinates[ACTIVITY_VALUE + activity] = (worth[activity], low, high, False)
        fixed, stages = self.max_min(variables, rows, coordinates)  # step 2
        for resource in model.resources:
            if PRICE + resource in fixed:
                chosen[resource] = (fixed[PRICE + resource], MAX_MIN)
        for resource in model.resources:  # the withheld coordinates, free through the max-min
            if resource in chosen:
                continue
            low = self.extreme(variables, rows, pools[resource], -1)
            if low is not None:
                fix(resource, low, ONE_MORE)
                continue
            high = self.extreme(variables, rows, pools[resource], 1)
            fix(resource, high if high is not None else zero, ONE_LESS if high is not None else ZERO)
        self.optimal(_Program(variables, tuple(rows), {}), DUAL)  # the vector is a point of D*, checked exactly
        vector = {resource: chosen[resource][0] for resource in model.resources}
        prices = {
            resource: {
                "value": str(vector[resource]), "interval": _price(intervals[resource]), "by": chosen[resource][1],
                "withheld": self.withheld(resource, intervals[resource], goals),
            }
            for resource in model.resources
        }
        activities: dict[str, Any] = {}
        for activity in model.activities:
            count, cost = most[activity], zero
            if MOST in unverified[activity]:  # the forcing cost reads `most`
                unverified[activity].append(FORCING)
                cost = None
            elif count is not None:
                forced = _Row(
                    ("forced", activity, 0), {("x", activity, step): -one for step in range(steps)}, "le", -count, None
                )
                forced_program = _program(model, added=(forced,))
                forcing = _dual(forced_program)
                restricted = _restricted(forcing, forced_program, runs.get(activity, optimal))
                program = _Program(*restricted, _nonzero(dict(forcing.duals[forced.key])))
                found = self.certain(self.probe(program, DUAL, unbounded=True), DUAL)
                if isinstance(found, _Unverified):
                    note(FORCING, activity, found)
                    cost = None
                else:
                    cost = None if found.status == "unbounded" else found.value
            value = None
            if activity in worth:
                value = sum(
                    (amount * vector[entity] for (name, entity), amount in model.net.items()
                     if name == activity and entity in vector),
                    zero,
                )
            doubtful = unverified[activity]
            activities[activity] = {
                "value": _text(value),
                "range": _price(ranges[activity]) if activity in ranges else None,
                "in_optimum": None if MOST in doubtful else count is None or count > 0,
                "most": _text(count),
                "forcing_cost": _text(cost),
                "input_bound": None if FORCING in doubtful else cost is None,
                "unverified": list(doubtful),
            }
        return {"prices": prices, "max_min": stages, "activities": activities}, warnings

    def withheld(self, resource: str, interval: _Pair, goals: set[str]) -> dict[str, Any] | None:
        """Why the selection withholds a pool's coordinate, or None (addendum 2.1 §2.2 step 4.1 and its mirror):
        - `no_room`: one more unit is infeasible (`low` null): nothing in the cycle can take it. It names what
          bounds the pool there: its regen stop (`regen`, `stop`) and its hard cap (`cap`), and the pending flows
          left out that would take from it (`excluded`);
        - `no_steady_supply`: one less unit is infeasible (`high` null) and no main goal values it: it has no steady
          supply, a structural gap (§3.1)."""
        low, high = interval
        model = self.model
        if low is None:
            found: dict[str, Any] = {"reason": NO_ROOM}
            if resource in model.stops:
                found["regen"], found["stop"] = model.stops[resource][0], model.stops[resource][1]
            if resource in model.hard:
                found["cap"] = model.hard[resource][0]
            found["excluded"] = list(model.left_out.get(resource, ()))
            return found
        if high is None and resource not in goals:
            return {"reason": NO_STEADY_SUPPLY}
        return None

    def rule_dual(self, program: _Program, base: _Solution, rule: str) -> float:
        """HiGHS's derivative of the optimum by a rule's value: its rows' marginals (times what a unit counts, for
        a relaxed period), and for a storage cap also the moves it bounds (the cap is their coefficient)."""
        rows = ("cap", "limit", "stop", "income")
        dual = float(self.model.scale(rule)) * sum(
            -base.le[row.key] for row in program.rows if row.group == rule and row.key[0] in rows
        )
        for flow, activity, source, _ in self.model.moves:
            if self.model.storage.get(source, ("",))[0] == rule:
                for step in range(self.model.steps):
                    dual += -base.le[("move", flow, step)] * float(base.x[("x", activity, step)])
        return dual

    def optimal(self, program: _Program, stage: str) -> _Solution:
        """The optimum of a program that has one: the program of the optimum with rows that a solution of it meets
        (a floor at an optimum, a probe), or another objective over the same rows."""
        solution = self.lp(program, stage)
        if solution.status != "optimal":
            raise self.failure(LP_SOLVE_FAILED, stage=stage, reason="solver_status", status=2)
        return solution

    def tiers(self) -> _Tiers:
        """The three solves (#424 ACM-PRICE addendum 2.1 §4.2). S3 maximises V alone: an infeasible one is
        `lp_infeasible`, and its optimum V0 is the objective. S1 maximises A, giving A*; S2 maximises V with A ≥ A*,
        giving V*. When A has no term, A* is 0 and S2 is S3. V* above V0 cannot be (S3 relaxes S2) and is
        `lp_numeric_check_failed`; V0 above V*, exactly, is `objective_tier_conflict`. Then S3's dual, checked exactly
        (`duality`; PRICE-2c moves it before the ties, which read its optimal point), and the ties."""
        model = self.model
        value, acquired = total_value(model), acquisition(model)
        program = _program(model)
        base = self.lp(program, "optimum", keep=True)
        if base.status != "optimal":
            raise Failure(self.conflict())
        records = acquirers(model)
        if not acquired:
            face = _floor(VALUE, value, base.value)
            dual = _dual(program)
            point = self.duality(dual, base.value)
            return _Tiers(program, base, Fraction(0), base, (), face, records, (), dual, point)
        first = self.optimal(_program(model, objective=acquired), ACQUISITION)
        floors = _floor(ACQUISITION, acquired, first.value)
        second = self.optimal(_program(model, added=floors), VALUE)
        if second.value > base.value:
            raise self.failure(
                LP_NUMERIC_CHECK_FAILED, stage=VALUE, constraint=VALUE, found=str(second.value), bound=f"<={base.value}"
            )
        if base.value > second.value:
            raise Failure(self.tier_conflict(acquired, records, first.value, second, base))
        face = floors + _floor(VALUE, value, second.value)
        dual = _dual(program)
        point = self.duality(dual, base.value)
        ties, unverified = self.ties(face, _floor(VALUE, value, base.value), second, base, dual, point)
        return _Tiers(
            program, base, first.value, second, floors, face, records, tuple(ties), dual, point, tuple(unverified)
        )

    def tier_conflict(
        self, acquired: dict[Key, Fraction], records: list[dict[str, Any]], most: Fraction, second: _Solution,
        base: _Solution,
    ) -> dict[str, Any]:
        """`objective_tier_conflict`: by value alone, the secondary goal takes what the protected acquisition needs
        (V0 > V*). It lists what of A is smaller in the optimum of S3 than in that of S2, each by the term of A it
        owns: the protected `activities` (what they bring in), the demand rows' `steps` (their `main`) and the income
        `caps` (what they void); and P (`protected`), to check against the guide what the criteria protect."""
        model = self.model
        flows = {flow: activity for flow, activity, _, _ in model.moves}
        member = {"x": "activities", "m": "activities", "q": "steps", "v": "caps"}

        def parts(x: dict[Key, Fraction]) -> dict[tuple[str, str], Fraction]:
            found: dict[tuple[str, str], Fraction] = {}
            for key, weight in acquired.items():
                kind, name, _ = key
                _add(found, (member[kind], flows.get(name, name) if kind == "m" else name), weight * x[key])
            return found

        before, after = parts(second.x), parts(base.x)
        detail: dict[str, Any] = {
            "path": model.path, "acquisition": str(most), "value": str(second.value), "unconstrained": str(base.value),
            "activities": [],
            "protected": [record["activity"] for record in records if protects(record)],
        }
        for (name, owner), amount in sorted(before.items()):
            if after[(name, owner)] < amount:
                detail.setdefault(name, []).append(owner)
        return error(OBJECTIVE_TIER_CONFLICT, detail)

    def ties(
        self, face: tuple[_Row, ...], unconstrained: tuple[_Row, ...], second: _Solution, base: _Solution,
        dual: _Dual, point: dict[Key, Fraction],
    ) -> tuple[list[str], list[dict[str, Any]]]:
        """`objective_tier_tie` (§4.2): the activities that some optimum of S3 uses and no optimum of S2 does. By
        value alone they are as good as the acquisition, and they would lower A. Returns them, and the warnings of the
        probes that could not be verified (PRICE-2c).

        - **No probe by complementary slackness** (PRICE-2c, `_never`): an activity whose dual slack at the exact
          optimal point of S3's dual is above 0 in every step runs in no optimum of S3, so in none of S2 either (V* =
          V0: S2's optima are optima of S3): it is no tie.
        - **The probes.** Any other activity that the optimum of S2 found does not use: the most of it, at most 1 a
          cycle (so the probe is bounded), on the optima of S2 (`face`: A ≥ A*, V ≥ V*) and then, when that is 0, on
          those of S3 (V ≥ V0). An answer above 0 holds exactly for any exact feasible point (a point of the face that
          runs it); an answer of 0 only for a certified optimum (`core.exact`). The tie is decided when the answers
          that decide it are exact: a probe on S2 above 0 (no tie); one on S3 at a certified 0 (no tie); one on S3
          above 0, or the optimum of S3 using it, with one on S2 at a certified 0 (a tie). Otherwise it is
          unverified: the warning `lp_probe_unverified` (`probe` tie), never "no tie", and its component counts as a
          structural gap (`core.coverage`)."""
        model, one = self.model, Fraction(1)
        found: list[str] = []
        warnings: list[dict[str, Any]] = []
        for activity in model.activities:
            keys = [("x", activity, step) for step in range(model.steps)]
            if _never(dual, point, keys):
                continue
            if any(second.x[key] > 0 for key in keys):
                continue
            probe = (_Row(("probe", activity, 0), {key: one for key in keys}, "le", one, None),)
            objective = {key: one for key in keys}
            on_second = self.probe(_program(model, objective=objective, added=face + probe), TIE)
            if isinstance(on_second, _Solution) and on_second.value > 0:
                continue  # an optimum of S2 runs it
            second_zero = isinstance(on_second, _Solution) and on_second.certified
            used = any(base.x[key] > 0 for key in keys)
            on_base: _Solution | _Unverified | None = None
            if not used:
                on_base = self.probe(_program(model, objective=objective, added=unconstrained + probe), TIE)
                if isinstance(on_base, _Solution) and on_base.value > 0:
                    used = True
                elif isinstance(on_base, _Solution) and on_base.certified:
                    continue  # no optimum of S3 runs it
            if used and second_zero:
                found.append(activity)
                continue
            doubtful = on_second if not second_zero else on_base
            warnings.append(self.unverified(TIE, activity, self.certain(doubtful, TIE)))
        return found, warnings

    def steady(self) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        """The steady state. A resource's price in a step is the projection of its balance row's dual on D*
        (PRICE-2b-1); its re-solves at plus and minus one grain (`grain`) and HiGHS's marginal cross-check it."""
        model, steps = self.model, self.model.steps
        tiers = self.tiers()
        program, base, optimum = tiers.program, tiers.base, tiers.base.value
        least = self.lp(_program(model, added=tiers.face, least_losses=True), "losses")
        if least.status != "optimal":
            raise self.failure(LP_SOLVE_FAILED, stage="losses", reason="solver_status", status=2)
        warnings = [error(OBJECTIVE_TIER_TIE, {"path": model.path, "activity": activity}) for activity in tiers.ties]
        warnings += tiers.unverified
        dual = tiers.dual  # S3's, which every price re-solve changes, checked exactly (`duality`)
        face = dual.face(optimum)
        on_face = _restricted(dual, program, base.x)  # D*, for the LPs that read only a value on it
        self.grained = self.hold(program, base)  # S3 with its basis factorised, for the re-solves at ±grain (PRICE-2d)
        prices: dict[str, list[_Pair]] = {}
        grains: dict[str, list[_Pair]] = {}
        marginals: dict[str, list[Fraction]] = {}
        for resource in model.resources:
            prices[resource], grains[resource], marginals[resource] = [], [], []
            for step in range(steps):
                known, sides = self.chords(resource, step)
                for side in known:
                    if side is not None:
                        self.grain_way("basis")
                grain = self.price(optimum, lambda delta: self.regrain(resource, step, delta), known)
                if _pinned(model, resource, grain):
                    price = grain  # the chords pin the projection (PRICE-2c): no LP on D*
                else:  # each end that S3's basis does not prove is an LP on D*
                    function = dual.duals[("balance", resource, step)]
                    low = sides[0] if sides[0] is not None else self.extreme(*on_face, function, -1)
                    high = sides[1] if sides[1] is not None else self.extreme(*on_face, function, 1)
                    price = (low, high)
                marginal = self.marginal(-base.eq[("balance", resource, step)])
                warnings += self.disagrees(resource, step, marginal, price)
                warnings += self.outside(resource, step, price, grain)
                prices[resource].append(price)
                grains[resource].append(grain)
                marginals[resource].append(marginal)
        rule_prices: dict[str, tuple[Fraction, Fraction | None]] = {}
        rule_marginals: dict[str, Fraction] = {}
        for rule, value in model.priced():
            price = self.price(optimum, lambda delta: self.resolved(_program(model, values={rule: value + delta})))
            marginal = self.marginal(self.rule_dual(program, base, rule))
            warnings += self.disagrees(rule, None, marginal, price)
            rule_prices[rule], rule_marginals[rule] = price, marginal
        values: dict[str, Any] = {}
        for activity in model.activities:
            if model.touches_store(activity):  # what one execution moves depends on what the store holds
                values[activity] = None
                continue
            values[activity] = [_price(self.value(activity, prices, step)) for step in range(steps)]
        joint, judged = self.joint(dual, tiers.point, optimum, prices, on_face, base.x)
        warnings += judged
        data = {
            "cycle": {"steps": steps, "step_seconds": STEP_SECONDS},
            "objective": str(optimum),
            "acquisition": str(tiers.acquired),
            "protected": [record for record in tiers.records if protects(record)],
            "conversions": [record for record in tiers.records if not protects(record)],
            "prices": {resource: [_price(price) for price in prices[resource]] for resource in model.resources},
            "rule_prices": {rule: _price(price) for rule, price in rule_prices.items()},
            "activity_values": values,
            "joint": joint,
            "losses": {
                "overflow": {r: str(self.total(least, "o", r)) for r in sorted(model.hard)},
                "lost_production": {f: str(self.total(least, "l", f)) for f in sorted({*model.storage, *model.stops})},
            },
            "grain": {"prices": {resource: [_price(pair) for pair in grains[resource]] for resource in model.resources}},
            "marginals": {
                "prices": {resource: [str(m) for m in marginals[resource]] for resource in model.resources},
                "rule_prices": {rule: str(m) for rule, m in rule_marginals.items()},
            },
            "excluded": list(model.excluded),
            "relaxed": model.shown_relaxed(),
        }
        return data, warnings

    def total(self, solution: _Solution, kind: str, entity: str) -> Fraction:
        """An overflow or a loss over the cycle (a void_at_reset cap overflows only before its resets)."""
        x = solution.x
        return sum((x.get((kind, entity, step), Fraction(0)) for step in range(self.model.steps)), Fraction(0))

    def value(self, activity: str, prices: dict[str, list[_Pair]], step: int) -> tuple[Fraction | None, Fraction | None]:
        """An activity's value per execution in a step: the prices of what it produces less what it consumes,
        an interval where a price is one (null where unbounded)."""
        low: Fraction | None = Fraction(0)
        high: Fraction | None = Fraction(0)
        for resource in self.model.resources:
            amount = self.model.net.get((activity, resource))
            if not amount:
                continue
            cheap, dear = prices[resource][step]
            ends = (None if cheap is None else amount * cheap, None if dear is None else amount * dear)
            down, up = (ends[0], ends[1]) if amount > 0 else (ends[1], ends[0])
            low = None if low is None or down is None else low + down
            high = None if high is None or up is None else high + up
        return low, high

    def plan(self) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        model, steps = self.model, self.model.steps
        solution = self.milp(_program(model), "optimum")
        if solution.status == "infeasible":
            raise Failure(self.conflict())
        warnings = []
        if solution.status == "unproven":
            detail = {
                "path": model.path, "solve": PLAN, "node_limit": self.settings.node_limit, "nodes": solution.nodes,
                "status": solution.raw_status,
            }
            warnings.append(error(SEARCH_BUDGET_EXHAUSTED, detail))
        x = solution.x
        data = {
            "steps": steps,
            "proven": solution.status == "optimal",
            "objective": str(solution.value),
            "counts": {a: [str(x[("x", a, step)]) for step in range(steps)] for a in model.activities},
            "moved": {flow: [str(x[("m", flow, step)]) for step in range(steps)] for flow, _, _, _ in model.moves},
            "end": {entity: str(x[("s", entity, steps)]) for entity in model.entities},
            "unread": list(model.unread),
            "excluded": list(model.excluded),
            "relaxed": model.shown_relaxed(),
        }
        return data, warnings


# The nodes ------------------------------------------------------------------------------------------------


def compute(graph: Graph, solve: str) -> Economy:
    """The economy node `solve` (steady or plan) of `graph`. It raises `Need` until every node it reads is
    computed, as every graph node does."""
    graph.depend(("economy_model",))
    shape = _shape(graph, solve)
    reader = _Reader(graph, shape, solve)
    model = reader.model(graph.economy)
    if model is None:
        return Economy(blocked_by=graph.register(reader.refused) | frozenset(reader.blocked))
    status = reader.inputs()
    solver = _Solver(model, graph.economy)
    try:
        data, warnings = solver.steady() if solve == STEADY else solver.plan()
    except Failure as failure:
        return Economy(blocked_by=graph.register(failure.errors))
    if solve == STEADY:
        # Each price has the status of its static component (#424 ACM-PRICE 4.6, addendum 2.1 §3.3; PRICE-2b-2): a
        # component that left out a pending flow, or holds another structural gap, is provisional, and one that does
        # not keeps its own status. The steady state is the worst of them.
        pending = reader.excluded_pending()
        found, prices = coverage.report(graph, reader, model, warnings)
        status = worst(status, *([PROVISIONAL] if pending else []), *(price["status"] for price in prices.values()))
        data["excluded_pending"] = pending
        data["price_status"] = prices
        data["coverage"] = found
    per_account = solve == PLAN
    return Economy(
        {"status": status, "per_account": per_account, **data}, status, per_account, tuple(warnings),
        reads=tuple(reader.reads),
    )
