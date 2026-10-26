"""The economy: resource flows and constraints as linear programs, solved with scipy's HiGHS (#424 design 4.3
and 4.4, design-py 4.3; contracts/formats.md, "The economy").

Two nodes of the derivation graph (`core.graph`), each computed after what it reads and kept in the cache:
- ("economy", "steady"): the values, game-wide. A cyclic steady state over the longest declared period, built
  from the rules and the data only (the graph's "general" nodes) and solved as an LP. It gives the optimum per
  cycle; the price of every resource in every step and of every cap and limit, by re-solves at plus and minus
  one grain (`lp.grain`), exact when both sides agree and an interval otherwise; the value of each activity at
  those prices; and the least overflow and lost production among the optimal solutions. HiGHS's marginals are
  only a cross-check (`lp_dual_disagrees`).
- ("economy", "plan"): one account's plan. From the state's readings over `economy.plan_horizon` steps, with
  integer counts and an indicator per read cap and step, solved as a MILP within `search.milp_node_limit`
  nodes and never a time limit; the final stocks are priced by the steady state. A plan the node limit stops is
  `search_budget_exhausted`, with `proven: false`.

**The program** (generated from the rules, never written by hand). One step is one day; a cycle is one week
when the model declares a week period, else one day. Per activity and step a count; per resource and store a
stock at every step boundary; per hard-capped resource an overflow (income the cap blocks); per store with a
storage cap its lost production; per move whose amount needs the account state (a collection that takes what
the store holds) the amount it moves, at most the store's storage cap per execution. Rows, in canonical order
(by kind, id and step): the balances, the bounds of overflow by income, the cyclic rows (steady), the caps, the
limits per window, the moves, the read caps' indicators (plan) and the opening stocks (plan). Columns are sorted
by variable id.

**Exactness.** Every solution is converted with `Fraction(x).limit_denominator(lp.max_denominator)` and checked
exactly against every bound, integrality and row (`lp_numeric_check_failed`). Objective values, prices and
losses are computed from the checked fractions, never from the solver's floats.

**Infeasibility.** An infeasible program names a minimal set of conflicting rule groups (`lp_infeasible`): each
group (a regen, cap, limit or read cap, and in a plan each opening stock) is dropped in turn, in id order, and
stays dropped while the rest is still infeasible.

**Refusals.** What this version does not model is `expr_eval_failed` (`evaluation_unsupported`, operator
`economy`), never approximated; `found` names it.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from fractions import Fraction
from typing import TYPE_CHECKING, Any, Callable

from acmodel.check.checker import Checker, Entry
from acmodel.check.walk import at
from acmodel.codes import (
    EXPR_EVAL_FAILED,
    LP_DUAL_DISAGREES,
    LP_INFEASIBLE,
    LP_NUMERIC_CHECK_FAILED,
    LP_SOLVE_FAILED,
    SEARCH_BUDGET_EXHAUSTED,
    STATE_READING_MISSING,
)
from acmodel.config import EconomySettings
from acmodel.core.common import CONFIRMED, PROVISIONAL, Need, worst
from acmodel.core.unknown import rule_status
from acmodel.envelope import Failure, error
from acmodel.expr import Binary, Reference
from acmodel.expr.interval import Interval, Undecided

if TYPE_CHECKING:
    from acmodel.core.graph import Graph, Scenario

STEP_SECONDS = 86400  # one step is one day, the shortest period unit
WEEK_STEPS = 7
STEADY, PLAN = "steady", "plan"
NODES = {STEADY: ("economy", STEADY), PLAN: ("economy", PLAN)}
OUTPUTS = {STEADY: "economy:steady", PLAN: "economy:plan"}
# The rule kinds the economy reads, and of them the kinds it does not model yet.
RULE_KINDS = ("cap", "cooldown", "exchange", "flow", "gate", "limit", "objective", "regen")
NOT_MODELLED = ("cooldown", "exchange", "objective")
HARD_REACH = ("block_all_income", "discard")
STORAGE_REACH = ("block_all_income", "discard", "stop_production")
READING_GROUP = "reading:"
# How an economy node is kept in the cache.
FORMAT = {"economy": ["data", "status", "per_account", "warnings", "blocked"]}
_STATE = object()  # a node from the rules and the data only that would need the account state

Key = tuple[str, str, int]
_Pair = tuple[Fraction, Fraction | None]  # a price: its low and high ends, None for unbounded


@dataclass(frozen=True)
class Economy:
    """The value of an economy node: its output as `economy` shows it, or the errors that block it."""

    data: dict[str, Any] | None = None
    status: str = CONFIRMED
    per_account: bool = False
    warnings: tuple[dict[str, Any], ...] = ()
    blocked_by: frozenset[int] = frozenset()

    @property
    def blocked(self) -> bool:
        return bool(self.blocked_by)


def spec_keys(settings: EconomySettings, solve: str) -> dict[str, int]:
    """The configuration keys a node's value depends on, for its spec in the cache."""
    keys = {"max_denominator": settings.max_denominator, "grain": settings.grain}
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
        "warnings": list(result.warnings), "blocked": blocked,
    }


def decode(doc: dict[str, Any], register: Callable[[list[dict[str, Any]]], frozenset[int]]) -> Economy:
    return Economy(doc["data"], doc["status"], doc["per_account"], tuple(doc["warnings"]), register(doc["blocked"]))


def render(result: Economy) -> dict[str, Any]:
    """An economy output as `economy` shows it."""
    return {"status": "blocked"} if result.blocked else result.data


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
    """The low end of a price as `_price` shows it: the value of one more unit."""
    return Fraction(shown if isinstance(shown, str) else shown["low"])


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
    numeraire: str
    activities: list[str]
    excluded: list[str]  # activities behind a gate of origin person (#424 design 4.7)
    parts: list[_Part]
    regens: list[Entry]
    hard: dict[str, Entry]  # resource -> its hard cap
    storage: dict[str, Entry]  # store -> its storage cap
    soft: list[Entry]  # read caps that block incomes
    limits: list[tuple[Entry, tuple[str, ...]]]  # a limit and the activities it counts
    gates: list[Entry]
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


def _shape(graph: Graph) -> _Shape:
    checker = graph.checker
    manifest = checker.model.manifest
    refused: list[dict[str, Any]] = []
    windows: dict[str, int] = {}
    for index, period in enumerate(manifest.doc["periods"]):
        if period["unit"] == "month":  # a month has no fixed number of steps
            refused.append(_refused(manifest.path, at("", "periods", index, "unit"), "month"))
        else:
            windows[period["id"]] = WEEK_STEPS if period["unit"] == "week" else 1
    entities = sorted((e for e in checker.entities if checker.entries.get(e.id) is e), key=lambda e: e.id)
    rules = sorted((r for r in checker.rules if checker.active(r) and r.kind in RULE_KINDS), key=lambda r: r.id)

    def of(kind: str) -> list[Entry]:
        return [rule for rule in rules if rule.kind == kind]

    for rule in rules:
        if rule.kind in NOT_MODELLED:
            refused.append(_refused(rule.path, rule.ref, rule.kind))
    excluded: set[str] = set()
    gates: list[Entry] = []
    flows: list[tuple[Entry, str]] = []
    for rule in of("flow"):
        subject = checker.entries[rule.doc["subject"]]
        if subject.kind != "activity":
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
            continue
        requires = [checker.entries[gate] for gate in rule.doc["logic"].get("requires", ())]
        gates.extend(requires)
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
    for rule in of("regen"):
        subject = checker.entries[rule.doc["subject"]]
        if subject.kind not in ("resource", "store"):
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
        elif "while" in rule.doc["logic"] and not _stops_production(checker, graph, rule):
            refused.append(_refused(rule.path, at(rule.ref, "logic", "while"), "while"))
        else:
            regens.append(rule)
    hard: dict[str, Entry] = {}
    storage: dict[str, Entry] = {}
    soft: list[Entry] = []
    allowed = {
        "hard": ("resource", HARD_REACH), "storage": ("store", STORAGE_REACH),
        "soft_read": ("resource", ("block_income_from",)),
    }
    for rule in of("cap"):
        logic, subject = rule.doc["logic"], checker.entries[rule.doc["subject"]]
        reach = "block_income_from" if isinstance(logic["on_reach"], dict) else logic["on_reach"]
        kind, reaches = allowed[logic["cap_kind"]]
        if subject.kind != kind:
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
        elif reach not in reaches:
            refused.append(_refused(rule.path, at(rule.ref, "logic", "on_reach"), reach))
        elif logic["cap_kind"] == "hard":
            hard[subject.id] = rule
        elif logic["cap_kind"] == "storage":
            storage[subject.id] = rule
        else:
            soft.append(rule)
    limits: list[tuple[Entry, tuple[str, ...]]] = []
    for rule in of("limit"):
        subject = checker.entries[rule.doc["subject"]]
        if subject.kind != "activity":
            refused.append(_refused(rule.path, at(rule.ref, "subject"), subject.kind))
        elif rule.doc["logic"]["per"] in windows:  # a month period is refused above
            limits.append((rule, (subject.id, *rule.doc["logic"].get("shared_with", ()))))
    return _Shape(
        path=manifest.path,
        cycle=max(windows.values(), default=1),
        windows=windows,
        resources=[e.id for e in entities if e.kind == "resource"],
        stores={e.id: e.doc["holds"] for e in entities if e.kind == "store"},
        numeraire=manifest.doc["numeraire"],
        activities=sorted({activity for _, activity in flows} - excluded),
        excluded=sorted(excluded),
        parts=parts,
        regens=regens,
        hard=hard,
        storage=storage,
        soft=soft,
        limits=limits,
        gates=gates,
        refused=refused,
    )


# The values -----------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class _Model:
    """One program's evaluated inputs: everything the builder needs, as exact numbers."""

    solve: str
    path: str
    steps: int
    resources: tuple[str, ...]
    stores: tuple[str, ...]
    numeraire: str
    activities: tuple[str, ...]
    excluded: tuple[str, ...]
    net: dict[tuple[str, str], Fraction]  # (activity, resource or store) -> net amount per execution
    income: dict[tuple[str, str], Fraction]  # (activity, resource) -> what one execution brings in
    moves: tuple[tuple[str, str, str, str], ...]  # (flow, activity, store, target): takes what the store holds
    regens: tuple[tuple[str, str, tuple[Fraction, ...]], ...]  # (rule, resource or store, income per step)
    hard: dict[str, tuple[str, Fraction]]  # resource -> (cap, value)
    storage: dict[str, tuple[str, Fraction]]  # store -> (cap, value)
    limits: tuple[tuple[str, tuple[str, ...], int, Fraction], ...]  # (rule, activities, window steps, max)
    soft: tuple[tuple[str, str, Fraction, tuple[str, ...]], ...]  # (cap, resource, its reading, blocked moves)
    start: dict[str, tuple[str | None, Fraction]]  # plan: entity -> (its group, its opening stock)
    unread: tuple[str, ...]  # plan: entities that need no reading, started at 0
    weights: dict[str, Fraction]  # the objective: the value of each entity's gain over the program
    grain: Fraction

    @property
    def entities(self) -> tuple[str, ...]:
        return tuple(sorted((*self.resources, *self.stores)))

    def groups(self) -> list[str]:
        """The rule groups the infeasibility analysis drops in turn, in id order."""
        found = {rule for rule, _, _ in self.regens}
        found |= {cap for cap, _ in (*self.hard.values(), *self.storage.values())}
        found |= {rule for rule, _, _, _ in self.limits} | {cap for cap, _, _, _ in self.soft}
        found |= {group for group, _ in self.start.values() if group is not None}
        return sorted(found)

    def priced(self) -> list[tuple[str, Fraction]]:
        """The rules with a value that the steady state prices: caps with a value and limits."""
        found = [*self.hard.values(), *self.storage.values(), *((rule, maximum) for rule, _, _, maximum in self.limits)]
        return sorted(found)

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


class _Reader:
    """Reads the graph nodes one program needs: from the rules and the data only for the steady state, with the
    account state where needed for the plan. It records the status, the unknowns left unresolved, the errors
    that block the program and what it refuses."""

    def __init__(self, graph: Graph, shape: _Shape, solve: str) -> None:
        self.graph = graph
        self.shape = shape
        self.plan = solve == PLAN
        self.solve = solve
        self.status = CONFIRMED
        self.unresolved: set[str] = set()
        self.blocked: set[int] = set()
        self.refused = list(shape.refused)
        entries = graph.checker.entries
        self.stock_facts = {
            entries[entity].doc["reading"] for entity in (*shape.resources, *shape.stores)
            if "reading" in entries[entity].doc
        }

    def consult(self, entry: Entry) -> None:
        own, unresolved = rule_status(entry)
        self.status = worst(self.status, own)
        self.unresolved.update(unresolved)

    def refuse(self, path: str, ref: str, found: str) -> None:
        self.refused.append(_refused(path, ref, found))

    def need(self, keys: list[tuple[str, ...]]) -> None:
        for key in keys:
            if key not in self.graph.results:
                raise Need(key)

    def read(self, key: tuple[str, ...], path: str, ref: str) -> Any:
        """The one scenario of a node; `_STATE` for a node from the rules and the data only that would need the
        account state; None when it is blocked or read unknowns (refused: the economy solves single values)."""
        self.graph.depend(key)
        result = self.graph.results[key]
        if result.blocked:
            self.blocked.update(result.blocked_by)
            return None
        if result.needs_state:
            return _STATE
        scenario, *more = result.scenarios
        if more or scenario.unknowns or scenario.free or type(scenario.value) in (Interval, Undecided):
            self.refuse(path, ref, "unknown")
            return None
        self.status = worst(self.status, scenario.status)
        self.unresolved.update(scenario.unresolved)
        return scenario

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
        for cap in (*shape.hard.values(), *shape.storage.values()):
            member = "value" if "value" in cap.doc["logic"] else "reading"
            found.append(_Item(cap, ("rule", cap.id), at(cap.ref, "logic", member)))
        for rule, _ in shape.limits:
            found.append(_Item(rule, ("rule", rule.id), at(rule.ref, "logic", "max")))
        return found

    def values(self, items: list[_Item]) -> tuple[dict[tuple[str, ...], Any], set[str]]:
        """The value of each item, and the flows whose move takes what its store holds."""
        graph, plan = self.graph, self.plan
        self.need([("general",) + item.key for item in items])
        if plan:
            self.need([
                item.key for item in items
                if graph.results[("general",) + item.key].needs_state
                and not (item.part and item.part.member == "moves")
            ])
        values: dict[tuple[str, ...], Any] = {}
        variable: set[str] = set()
        for item in items:
            self.consult(item.owner)
            found = self.read(("general",) + item.key, item.owner.path, item.ref)
            if found is _STATE:
                if item.part is not None and item.part.member == "moves":
                    variable.add(item.part.flow)  # a collection: it takes what the store holds
                    continue
                if not plan:  # the steady state is game-wide: it never reads the account state
                    self.refuse(item.owner.path, item.ref, "state")
                    continue
                found = self.read(item.key, item.owner.path, item.ref)
                if found is not None and self.reads_stock(found):  # a stock the plan changes as it goes
                    self.refuse(item.owner.path, item.ref, "stock")
                    continue
            if found is not None:
                values[item.key] = found.value
        for gate in self.shape.gates:
            self.consult(gate)
        return values, variable

    def model(self, settings: EconomySettings) -> _Model | None:
        shape, graph, plan = self.shape, self.graph, self.plan
        items = self.items()
        if plan:
            self.need([("rule", cap.id) for cap in shape.soft] + [NODES[STEADY]])
        values, variable = self.values(items)
        steps = settings.plan_horizon if plan else shape.cycle
        weights = {shape.numeraire: Fraction(1)}
        if plan:  # the final stocks are priced by the steady state, at the step the plan ends in
            graph.depend(NODES[STEADY])
            steady = graph.results[NODES[STEADY]]
            if steady.blocked:
                self.blocked.update(steady.blocked_by)
                return None
            self.status = worst(self.status, steady.status)
            cycle = steady.data["cycle"]["steps"]
            weights = {r: _low(steady.data["prices"][r][steps % cycle]) for r in shape.resources}
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
        net: dict[tuple[str, str], Fraction] = {}
        income: dict[tuple[str, str], Fraction] = {}
        moves = []
        for part in shape.parts:
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
        activities = set(shape.activities)
        limits = tuple(
            (
                rule.id, tuple(sorted(set(members) & activities)), shape.windows[rule.doc["logic"]["per"]],
                values[("rule", rule.id)],
            )
            for rule, members in shape.limits if ("rule", rule.id) in values
        )
        soft = self.read_caps(variable, hard, storage) if plan else ()
        start, unread = self.opening(net, moves, regens, hard, storage, soft) if plan else ({}, ())
        if self.refused or self.blocked:
            return None
        return _Model(
            solve=self.solve,
            path=shape.path,
            steps=steps,
            resources=tuple(shape.resources),
            stores=tuple(sorted(shape.stores)),
            numeraire=shape.numeraire,
            activities=tuple(shape.activities),
            excluded=tuple(shape.excluded),
            net=net,
            income=income,
            moves=tuple(moves),
            regens=tuple(regens),
            hard=hard,
            storage=storage,
            limits=limits,
            soft=soft,
            start=start,
            unread=tuple(unread),
            weights=weights,
            grain=Fraction(settings.grain),
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

    def read_caps(self, variable: set[str], hard: dict[str, Any], storage: dict[str, Any]) -> tuple[Any, ...]:
        """The plan's read caps: each with its reading and the collections it blocks at or above it."""
        found = []
        for cap in self.shape.soft:
            self.consult(cap)
            scenario = self.read(("rule", cap.id), cap.path, at(cap.ref, "logic", "reading"))
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


def _program(
    model: _Model,
    *,
    extra: dict[tuple[str, int], Fraction] | None = None,
    values: dict[str, Fraction] | None = None,
    omit: frozenset[str] = frozenset(),
    weights: dict[str, Fraction] | None = None,
    floor: Fraction | None = None,
) -> _Program:
    """The program of `model`. `extra` adds income to a resource in a step and `values` replaces the value of a
    cap or limit (the price re-solves); `omit` drops rule groups (the infeasibility analysis); `weights`
    replaces the objective ({} for none); with `floor`, the objective must reach it and the overflow and lost
    production are minimised instead."""
    extra, values = extra or {}, values or {}
    zero, one, steps, plan = Fraction(0), Fraction(1), model.steps, model.solve == PLAN
    variables: dict[Key, tuple[Fraction, Fraction | None, bool]] = {}
    rows: list[_Row] = []
    income: dict[tuple[str, int], tuple[Fraction, Fraction]] = {}  # (entity, step) -> (all, the positive part)

    def receive(entity: str, step: int, amount: Fraction) -> None:
        total, positive = income.get((entity, step), (zero, zero))
        income[(entity, step)] = (total + amount, positive + max(amount, zero))

    for rule, target, incomes in model.regens:
        if rule not in omit:
            for step, amount in enumerate(incomes):
                receive(target, step, amount)
    for (entity, step), amount in sorted(extra.items()):
        receive(entity, step, amount)
    hard = {r: (cap, values.get(cap, value)) for r, (cap, value) in model.hard.items() if cap not in omit}
    storage = {f: (cap, values.get(cap, value)) for f, (cap, value) in model.storage.items() if cap not in omit}
    most = {f: values.get(cap, value) for f, (cap, value) in model.storage.items()}  # what one move takes at most

    def received(entity: str, step: int) -> tuple[Fraction, Fraction]:
        return income.get((entity, step), (zero, zero))

    for activity in model.activities:
        for step in range(steps):
            variables[("x", activity, step)] = (zero, None, plan)
    for entity in model.entities:
        for step in range(steps + 1):
            variables[("s", entity, step)] = (zero, None, False)
        for step in range(steps):
            if entity in hard:
                variables[("o", entity, step)] = (zero, None, False)
            if entity in storage:  # production lost at the storage cap: at most what the store produces
                variables[("l", entity, step)] = (zero, received(entity, step)[1], False)
    for flow, _, _, _ in model.moves:
        for step in range(steps):
            variables[("m", flow, step)] = (zero, None, False)
    for entity in model.entities:
        for step in range(steps):
            coefficients = {("s", entity, step + 1): one, ("s", entity, step): -one}
            for activity in model.activities:
                amount = model.net.get((activity, entity))
                if amount:
                    coefficients[("x", activity, step)] = -amount
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
            if entity in hard:
                coefficients[("o", entity, step)] = one
            if entity in storage:
                coefficients[("l", entity, step)] = one
            total, positive = received(entity, step)
            rows.append(_Row(("balance", entity, step), _nonzero(coefficients), "eq", total, None))
            if entity in hard:  # what overflows is income the cap blocks, never stock already held
                bound = {("o", entity, step): one, **{key: -amount for key, amount in inflow.items()}}
                rows.append(_Row(("overflow", entity, step), _nonzero(bound), "le", positive, None))
    if not plan:  # a steady state ends each cycle with at least the stock it began with; a capped one with it
        for entity in model.entities:
            if entity != model.numeraire:
                cyclic = {("s", entity, 0): one, ("s", entity, steps): -one}
                sense = "eq" if entity in hard or entity in storage else "le"
                rows.append(_Row(("cyclic", entity, 0), cyclic, sense, zero, None))
    for entity, (cap, value) in sorted((*hard.items(), *storage.items())):
        for step in range(steps + 1):
            rows.append(_Row(("cap", cap, step), {("s", entity, step): one}, "le", value, cap))
    for rule, members, window, maximum in model.limits:
        if rule in omit:
            continue
        for index, first in enumerate(range(0, steps, window)):
            counted = {("x", a, step): one for a in members for step in range(first, min(first + window, steps))}
            if counted:
                rows.append(_Row(("limit", rule, index), counted, "le", values.get(rule, maximum), rule))
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
    objective: dict[Key, Fraction] = {}
    for entity, weight in (model.weights if weights is None else weights).items():
        _add(objective, ("s", entity, steps), weight)
        _add(objective, ("s", entity, 0), -weight)
    objective = _nonzero(objective)
    if floor is not None:
        rows.append(_Row(("optimum", "", 0), {key: -value for key, value in objective.items()}, "le", -floor, None))
        objective = {key: -one for key in variables if key[0] in ("o", "l")}
    return _Program(variables, tuple(rows), objective)


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


def _arrays(program: _Program) -> _Arrays:
    """The program as scipy wants it: sparse rows in canonical order, columns by variable id, and the objective
    negated (scipy minimises)."""
    import numpy as np
    from scipy import sparse

    columns = sorted(program.variables)
    index = {key: position for position, key in enumerate(columns)}
    rows = sorted(program.rows, key=lambda row: row.key)
    eq = [row for row in rows if row.sense == "eq"]
    le = [row for row in rows if row.sense == "le"]

    def matrix(selected: list[_Row]) -> Any:
        if not selected:  # no rows of this sense: the solver is given none
            return None
        data, row_index, column_index = [], [], []
        for position, row in enumerate(selected):
            for key in sorted(row.coefficients):
                data.append(float(row.coefficients[key]))
                row_index.append(position)
                column_index.append(index[key])
        places = (np.array(row_index, dtype=np.int64), np.array(column_index, dtype=np.int64))
        return sparse.csr_array((np.array(data, dtype=np.float64), places), shape=(len(selected), len(columns)))

    c = np.zeros(len(columns), dtype=np.float64)
    for key, weight in program.objective.items():
        c[index[key]] = -float(weight)
    bounds = [program.variables[key] for key in columns]
    return _Arrays(
        columns=columns, rows=rows, eq=eq, le=le, c=c,
        a_eq=matrix(eq), b_eq=np.array([float(row.rhs) for row in eq], dtype=np.float64),
        a_le=matrix(le), b_le=np.array([float(row.rhs) for row in le], dtype=np.float64),
        a_all=matrix(rows),
        lower=[float(low) for low, _, _ in bounds],
        upper=[None if high is None else float(high) for _, high, _ in bounds],
        integer=[1 if integer else 0 for _, _, integer in bounds],
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
        lhs = sum((value * x[key] for key, value in row.coefficients.items()), Fraction(0))
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


def _marginals(section: Any, rows: list[_Row]) -> dict[Key, float]:
    values = [] if section is None else [float(value) for value in section.marginals]
    return {row.key: value for row, value in zip(rows, values, strict=True)}


class _Solver:
    def __init__(self, model: _Model, settings: EconomySettings) -> None:
        self.model = model
        self.settings = settings

    def failure(self, code: str, **detail: Any) -> Failure:
        return Failure(error(code, {"path": self.model.path, "solve": self.model.solve, **detail}))

    def exact(self, program: _Program, columns: list[Key], raw: Any, stage: str) -> dict[Key, Fraction]:
        """The solution as exact fractions, checked exactly against every bound, integrality and row.

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

    def lp(self, program: _Program, stage: str, *, check: bool = True) -> _Solution:
        from scipy.optimize import linprog

        arrays = _arrays(program)

        def call(presolve: bool) -> Any:
            return linprog(
                arrays.c,
                A_ub=arrays.a_le if arrays.le else None, b_ub=arrays.b_le if arrays.le else None,
                A_eq=arrays.a_eq if arrays.eq else None, b_eq=arrays.b_eq if arrays.eq else None,
                bounds=list(zip(arrays.lower, arrays.upper)), method="highs", options={"presolve": presolve},
            )

        result = call(True)
        if result.status not in (0, 2, 3):  # "unbounded or infeasible" from presolve, or numerical trouble
            result = call(False)
        if result.status == 2:
            return _Solution("infeasible")
        if result.status == 3:
            raise self.failure(LP_SOLVE_FAILED, stage=stage, reason="unbounded", status=3)
        if result.status != 0:
            raise self.failure(
                LP_SOLVE_FAILED, stage=stage, reason="solver_status", status=int(result.status), raw_text=str(result.message)
            )
        if not check:
            return _Solution("optimal")
        x = self.exact(program, arrays.columns, result.x, stage)
        eq = _marginals(getattr(result, "eqlin", None), arrays.eq) if arrays.eq else {}
        le = _marginals(getattr(result, "ineqlin", None), arrays.le) if arrays.le else {}
        return _Solution("optimal", x, self.objective(program, x), eq, le)

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
                constraints = LinearConstraint(arrays.a_all, low, high)
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
            verdict = self.feasible(_program(model, omit=frozenset(dropped | {group}), weights={}))
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

    def price(self, optimum: Fraction, change: Callable[[Fraction], dict[str, Any]]) -> _Pair:
        """The value of one more grain (re-solved with `change(+grain)`) and of one grain less (`change(-grain)`),
        each per unit. Equal, they are an exact price; else an interval, unbounded above when one grain less is
        infeasible."""
        grain = self.model.grain
        more = self.lp(_program(self.model, **change(grain)), "price")
        if more.status != "optimal":
            raise self.failure(LP_SOLVE_FAILED, stage="price", reason="solver_status", status=2)
        less = self.lp(_program(self.model, **change(-grain)), "price")
        right = (more.value - optimum) / grain
        if less.status != "optimal":
            return right, None
        left = (optimum - less.value) / grain
        return min(right, left), max(right, left)

    def marginal(self, dual: float) -> Fraction:
        if not math.isfinite(dual):
            raise self.failure(
                LP_NUMERIC_CHECK_FAILED, stage="optimum", constraint="marginal", found=repr(dual), bound="finite"
            )
        return Fraction(dual).limit_denominator(self.settings.max_denominator)

    def disagrees(self, target: str, step: int | None, marginal: Fraction, price: _Pair) -> list[dict[str, Any]]:
        low, high = price
        if low <= marginal and (high is None or marginal <= high):
            return []
        detail: dict[str, Any] = {"path": self.model.path, "target": target, "marginal": str(marginal), "low": str(low)}
        if step is not None:
            detail["step"] = step
        if high is not None:
            detail["high"] = str(high)
        return [error(LP_DUAL_DISAGREES, detail)]

    def rule_dual(self, program: _Program, base: _Solution, rule: str) -> float:
        """HiGHS's derivative of the optimum by a rule's value: its rows' marginals, and for a storage cap also
        the moves it bounds (the cap is their coefficient)."""
        dual = sum(-base.le[row.key] for row in program.rows if row.group == rule and row.key[0] in ("cap", "limit"))
        for flow, activity, source, _ in self.model.moves:
            if self.model.storage.get(source, ("",))[0] == rule:
                for step in range(self.model.steps):
                    dual += -base.le[("move", flow, step)] * float(base.x[("x", activity, step)])
        return dual

    def steady(self) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        model, steps = self.model, self.model.steps
        program = _program(model)
        base = self.lp(program, "optimum")
        if base.status != "optimal":
            raise Failure(self.conflict())
        optimum = base.value
        least = self.lp(_program(model, floor=optimum), "losses")
        if least.status != "optimal":
            raise self.failure(LP_SOLVE_FAILED, stage="losses", reason="solver_status", status=2)
        warnings: list[dict[str, Any]] = []
        prices: dict[str, list[tuple[Fraction, Fraction | None]]] = {}
        marginals: dict[str, list[Fraction]] = {}
        for resource in model.resources:
            prices[resource], marginals[resource] = [], []
            for step in range(steps):
                price = self.price(optimum, lambda delta: {"extra": {(resource, step): delta}})
                marginal = self.marginal(-base.eq[("balance", resource, step)])
                warnings += self.disagrees(resource, step, marginal, price)
                prices[resource].append(price)
                marginals[resource].append(marginal)
        rule_prices: dict[str, tuple[Fraction, Fraction | None]] = {}
        rule_marginals: dict[str, Fraction] = {}
        for rule, value in model.priced():
            price = self.price(optimum, lambda delta: {"values": {rule: value + delta}})
            marginal = self.marginal(self.rule_dual(program, base, rule))
            warnings += self.disagrees(rule, None, marginal, price)
            rule_prices[rule], rule_marginals[rule] = price, marginal
        values: dict[str, Any] = {}
        for activity in model.activities:
            if model.touches_store(activity):  # what one execution moves depends on what the store holds
                values[activity] = None
                continue
            values[activity] = [_price(self.value(activity, prices, step)) for step in range(steps)]
        data = {
            "cycle": {"steps": steps, "step_seconds": STEP_SECONDS},
            "objective": str(optimum),
            "prices": {resource: [_price(price) for price in prices[resource]] for resource in model.resources},
            "rule_prices": {rule: _price(price) for rule, price in rule_prices.items()},
            "activity_values": values,
            "losses": {
                "overflow": {r: str(self.total(least, "o", r)) for r in sorted(model.hard)},
                "lost_production": {f: str(self.total(least, "l", f)) for f in sorted(model.storage)},
            },
            "marginals": {
                "prices": {resource: [str(m) for m in marginals[resource]] for resource in model.resources},
                "rule_prices": {rule: str(m) for rule, m in rule_marginals.items()},
            },
            "excluded": list(model.excluded),
        }
        return data, warnings

    def total(self, solution: _Solution, kind: str, entity: str) -> Fraction:
        return sum((solution.x[(kind, entity, step)] for step in range(self.model.steps)), Fraction(0))

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
            ends = (amount * cheap, None if dear is None else amount * dear)
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
        }
        return data, warnings


# The nodes ------------------------------------------------------------------------------------------------


def compute(graph: Graph, solve: str) -> Economy:
    """The economy node `solve` (steady or plan) of `graph`. It raises `Need` until every node it reads is
    computed, as every graph node does."""
    graph.depend(("economy_model",))
    shape = _shape(graph)
    reader = _Reader(graph, shape, solve)
    model = reader.model(graph.economy)
    if model is None:
        return Economy(blocked_by=graph.register(reader.refused) | frozenset(reader.blocked))
    status = worst(reader.status, PROVISIONAL) if reader.unresolved else reader.status
    solver = _Solver(model, graph.economy)
    try:
        data, warnings = solver.steady() if solve == STEADY else solver.plan()
    except Failure as failure:
        return Economy(blocked_by=graph.register(failure.errors))
    per_account = solve == PLAN
    return Economy({"status": status, "per_account": per_account, **data}, status, per_account, tuple(warnings))
