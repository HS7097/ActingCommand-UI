"""The derivation graph and its evaluation (contracts/formats.md, "The run").

**Nodes.** Each is computed after the nodes it reads:
- ("rule", id): the value of a derive (`value`, then its `round`), a cap (`value`, or its reading) or a
  limit (`max`);
- ("general_logic", id): a derive's `general_logic`, evaluated beside its value;
- ("offset", id): a derive's `live.offset`, the offset of its fact-offset form;
- ("flow", id, "consumes" | "produces", resource) and ("flow", id, "moves"): the amounts of a flow, which
  `cost`, `yield` and `max_single_income` read;
- ("member", unknown id, member): a member of a variants unknown;
- ("threshold", id): a threshold of exports.json.

**Scenarios.** A node that reads a variants unknown has one value per choice of variant. The choices are
found as the evaluation reaches them, so a node depends only on the unknowns its value actually reads.

**Status.** A scenario's status is the worst of what it read: a rule's or data row's status (pending
counts as provisional), any unknown (provisional), a reading (confirmed). A node that cannot be computed
is blocked with its errors, and so is every node that reads it; the others are still computed.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, replace
from fractions import Fraction
from typing import Any, Callable

from acmodel.check.checker import Checker, Entry
from acmodel.check.walk import at
from acmodel.codes import (
    DATA_REFERENCE_MISSING,
    EXPR_EVAL_FAILED,
    RULE_PENDING_UNBOUNDED,
    RULING_OVERRIDES_GENERAL_LOGIC,
    STATE_ENTITY_MISSING,
    STATE_READING_MISSING,
)
from acmodel.core.common import (
    BLOCKED,
    PROVISIONAL,
    RULE_STATUS,
    VALUE_RULES,
    Need,
    flow_parts,
    solve,
    unknown_target,
    worst,
)
from acmodel.core.types import Typed
from acmodel.envelope import Failure, error
from acmodel.expr import Duration, Reference, Value, evaluate
from acmodel.model import readers


@dataclass(frozen=True)
class Scenario:
    unknowns: tuple[tuple[str, str], ...]  # (unknown id, variant), sorted; empty when none was read
    value: Value
    status: str  # confirmed or provisional
    per_account: bool  # the value read the account state


@dataclass(frozen=True)
class Result:
    scenarios: tuple[Scenario, ...] = ()
    blocked_by: frozenset[int] = frozenset()  # the root errors that block it; empty when it is computed

    @property
    def blocked(self) -> bool:
        return bool(self.blocked_by)

    @property
    def status(self) -> str:
        return BLOCKED if self.blocked else worst(*(scenario.status for scenario in self.scenarios))


class _Branch(Exception):
    """The evaluation reached a variants unknown that its scenario has not chosen yet."""

    def __init__(self, unknown: str) -> None:
        super().__init__(unknown)
        self.unknown = unknown


class _Blocked(Exception):
    def __init__(self, errors: frozenset[int]) -> None:
        super().__init__(sorted(errors))
        self.errors = errors


def _magnitude(value: Value) -> Any:
    return value.seconds if type(value) is Duration else value


def _rounded(mode: str | None, value: Value) -> Value:
    if mode is None or type(value) is bool:
        return value
    number = _magnitude(value)
    whole = Fraction(math.floor(number) if mode == "floor" else math.ceil(number) if mode == "ceil" else round(number))
    return Duration(whole) if type(value) is Duration else whole


def _text(value: Value) -> str:
    """A value as a token: an exact rational (`-250`, `7/2`; time in seconds), or `true` / `false`."""
    if type(value) is bool:
        return "true" if value else "false"
    return str(_magnitude(value))


def _json(value: Value) -> Any:
    return value if type(value) is bool else _text(value)


def _convert(spec: dict[str, Any], raw: Any) -> Any:
    """A cell or attribute as a value of its column type. What does not fit is passed on as it is, so that
    the evaluation refuses it (`expr_eval_failed`, `value_invalid`)."""
    kind = spec["type"]
    if kind not in ("integer", "decimal"):
        return raw
    if type(raw) is int:
        number = Fraction(raw)
    elif type(raw) is str and (readers.INTEGER_TEXT if kind == "integer" else readers.DECIMAL_TEXT).pattern.fullmatch(raw):
        number = Fraction(raw)
    else:
        return raw
    return Duration(number) if spec["dimension"] == "time" else number


def _unsupported(operator: str, position: int, found: str, ref: str) -> Failure:
    detail = {"reason": "evaluation_unsupported", "operator": operator, "position": position, "found": found}
    return Failure(error(EXPR_EVAL_FAILED, detail, ref))


class _Environment:
    """The value environment of one node in one scenario."""

    def __init__(self, graph: Graph, owner: Entry, ref: str, unknowns: dict[str, str], status: str) -> None:
        self.graph = graph
        self.owner = owner
        self.ref = ref
        self.unknowns = unknowns
        self.status = status
        self.per_account = False

    def fail(self, code: str, **detail: Any) -> None:
        raise Failure(error(code, {key: value for key, value in detail.items() if value is not None}, self.ref))

    def use(self, scenario: Scenario) -> Value:
        self.status = worst(self.status, scenario.status)
        self.per_account = self.per_account or scenario.per_account
        return scenario.value

    def consistent(self, result: Result) -> list[Scenario]:
        return [s for s in result.scenarios if all(self.unknowns.get(u, v) == v for u, v in s.unknowns)]

    def node(self, key: tuple[str, ...]) -> Value:
        """The value of node `key` in this scenario; a choice it needs and this scenario lacks branches it."""
        candidates = self.consistent(self.graph.result(key))
        for scenario in candidates:
            if all(unknown in self.unknowns for unknown, _ in scenario.unknowns):
                return self.use(scenario)
        raise _Branch(next(u for u, _ in candidates[0].unknowns if u not in self.unknowns))

    # The environment the evaluator calls ----------------------------------------------------------

    def reference(self, ref: Reference) -> Value:
        checker, kind, target = self.graph.checker, ref.kind, ref.target
        if kind in ("rule", "cap"):
            return self.node(("rule", target))
        if kind == "unknown":
            entry, member = unknown_target(checker, target)
            if member is None:
                raise _unsupported("reference", ref.position, entry.kind, self.ref)
            return self.node(("member", entry.id, member))
        if kind == "read":
            return self.reading(target, ref.position)
        if kind in ("amount", "stock"):
            fact = checker.entries[target].doc.get("reading")
            if fact is None:
                self.per_account = True
                self.fail(STATE_READING_MISSING, entity=target, position=ref.position)
            return self.reading(fact, ref.position)
        if kind == "cleared":
            self.per_account = True
            return target in self.graph.cleared
        if kind == "entity":
            return self.attribute(ref)
        if kind == "data":
            return self.cell(ref)
        raise _unsupported("reference", ref.position, kind, self.ref)  # metric, count, pref

    def members(self, ref: Reference) -> list[tuple[str, ...]]:
        if ref.kind == "data":
            table = self.table(ref.target, ref.position, "reference")
            return list(table.rows)
        self.per_account = True
        entities = self.graph.state["entities"]
        if ref.target not in entities:
            self.fail(STATE_ENTITY_MISSING, missing="entity_class", entity_class=ref.target, position=ref.position)
        return [(entity,) for entity in sorted(entities[ref.target])]

    def flow(self, function: str, first: str, second: str) -> Value:
        parts = flow_parts(self.graph.checker, function, first, second)
        if function != "max_single_income":
            total: Value = Fraction(0)
            for key in parts:
                total += self.node(key)
            return total
        # The largest single income: over the flows, and over the variants this scenario leaves open.
        return max((self.use(s) for key in parts for s in self.consistent(self.graph.result(key))), key=_magnitude)

    def lookup(self, table_id: str, column: str, keys: tuple[Value, ...]) -> Value:
        position = self.graph.typed.lookups[(self.owner.path, self.ref)][(table_id, column)]
        table = self.table(table_id, position, "lookup")
        columns = table.file.doc["columns"]
        for key, index in table.rows.items():
            row = table.file.doc["rows"][index]
            if all(_convert(columns[name], row[name]) == value for name, value in zip(table.keys, keys)):
                return self.row_value(table_id, table, key, column, position)
        self.fail(
            DATA_REFERENCE_MISSING, missing="row", table=table_id, key=[_text(value) for value in keys], position=position
        )

    # State and data -----------------------------------------------------------------------------------

    def reading(self, fact: str, position: int | None) -> Value:
        self.per_account = True
        readings = self.graph.state["readings"]
        if fact not in readings:
            self.fail(STATE_READING_MISSING, fact=fact, position=position)
        return Fraction(readings[fact]["value"])

    def attribute(self, ref: Reference) -> Any:
        """`entity:<class>[<id>].<attr>`: the entity and its attribute must be in the state."""
        self.per_account = True
        entities, entity_class, (entity,), name = self.graph.state["entities"], ref.target, ref.key, ref.attr
        if entity_class not in entities:
            self.fail(STATE_ENTITY_MISSING, missing="entity_class", entity_class=entity_class, position=ref.position)
        if entity not in entities[entity_class]:
            self.fail(STATE_ENTITY_MISSING, missing="entity", entity_class=entity_class, entity=entity, position=ref.position)
        attributes = entities[entity_class][entity]
        if name not in attributes:
            self.fail(
                STATE_ENTITY_MISSING, missing="attribute", entity_class=entity_class, entity=entity,
                attribute=name, position=ref.position,
            )
        spec = self.graph.checker.entries[entity_class].doc["attributes"][name]
        return _convert(spec, attributes[name])

    def table(self, table_id: str, position: int, operator: str):
        table = self.graph.checker.tables[table_id]
        if table.rows is None:  # an external dataset: its rows are read by a later slice
            raise _unsupported(operator, position, "external_dataset", self.ref)
        return table

    def cell(self, ref: Reference) -> Any:
        table = self.table(ref.target, ref.position, "reference")
        return self.row_value(ref.target, table, ref.key, ref.attr, ref.position)

    def row_value(self, table_id: str, table, key: tuple[str, ...], column: str, position: int | None) -> Any:
        row = table.file.doc["rows"][table.rows[key]]
        if row["status"] == "pending":  # a row cannot name an unknown, so a pending row has no range
            self.fail(
                RULE_PENDING_UNBOUNDED, id=f"{table_id}[{','.join(key)}]", reason="no_unknown", position=position
            )
        self.status = worst(self.status, RULE_STATUS[row["status"]])
        return _convert(table.file.doc["columns"][column], row[column])


class Graph:
    """One run's graph over the model `checker` accepted, typed by `typed`, for one account state."""

    def __init__(
        self,
        checker: Checker,
        typed: Typed,
        state: dict[str, Any],
        exports: dict[str, Any],
        *,
        max_bits: int,
    ) -> None:
        self.checker = checker
        self.typed = typed
        self.state = state
        self.cleared = frozenset(state["cleared"])
        self.thresholds = {threshold["id"]: threshold for threshold in exports["thresholds"]}
        self.max_bits = max_bits
        self.results: dict[tuple[str, ...], Result] = {}
        self.errors: list[dict[str, Any]] = []  # the root errors, each once
        self._seen: dict[str, int] = {}

    def result(self, key: tuple[str, ...]) -> Result:
        if key not in self.results:
            raise Need(key)
        result = self.results[key]
        if result.blocked:
            raise _Blocked(result.blocked_by)
        return result

    def blocked(self, errors: list[dict[str, Any]], path: str) -> Result:
        indices = set()
        for entry in errors:
            entry = {**entry, "detail": {"path": path, **entry["detail"]}}
            text = json.dumps(entry, sort_keys=True)
            if text not in self._seen:
                self._seen[text] = len(self.errors)
                self.errors.append(entry)
            indices.add(self._seen[text])
        return Result(blocked_by=frozenset(indices))

    # Nodes --------------------------------------------------------------------------------------

    def compute(self, key: tuple[str, ...]) -> Result:
        if key[0] == "threshold":
            return self.threshold(key[1])
        entry = self.checker.entries[key[1]]
        if key[0] == "member":
            def member(unknowns: dict[str, str]) -> str:
                if entry.id not in unknowns:
                    raise _Branch(entry.id)
                return at(entry.ref, "variants", unknowns[entry.id], key[2])

            return self.expression(entry, member, PROVISIONAL)
        own, logic = RULE_STATUS[entry.doc["status"]], entry.doc["logic"]
        if "when" in entry.doc:  # a conditional value: evaluated by a later slice
            return self.blocked(_unsupported("when", 0, "when", at(entry.ref, "when")).errors, entry.path)
        if key[0] == "rule" and entry.kind == "cap" and "value" not in logic:
            return self.cap_reading(entry, own)
        if key[0] == "rule":
            member = ("max",) if entry.kind == "limit" else ("value",)
            return self.expression(entry, lambda _: at(entry.ref, "logic", *member), own, logic.get("round"))
        if key[0] == "general_logic":
            return self.expression(entry, lambda _: at(entry.ref, "logic", "general_logic"), own)
        if key[0] == "offset":
            return self.expression(entry, lambda _: at(entry.ref, "logic", "live", "offset"), own)
        ref = at(entry.ref, "logic", "moves", "amount") if key[2] == "moves" else at(entry.ref, "logic", key[2], key[3])
        if (entry.path, ref) not in self.typed.expressions:  # a `random:<id>` product
            return self.blocked(_unsupported(key[2], 0, "random", ref).errors, entry.path)
        return self.expression(entry, lambda _: ref, own)

    def expression(
        self, owner: Entry, select: Callable[[dict[str, str]], str], own: str, mode: str | None = None
    ) -> Result:
        """Evaluate the expression `select` picks, once per scenario."""
        scenarios = []
        todo: list[tuple[tuple[str, str], ...]] = [()]
        while todo:
            unknowns = todo.pop()
            chosen = dict(unknowns)
            try:
                ref = select(chosen)
                env = _Environment(self, owner, ref, chosen, own)
                value = evaluate(self.typed.expressions[(owner.path, ref)], env, max_bits=self.max_bits)
            except _Branch as branch:
                names = sorted(self.checker.entries[branch.unknown].doc["variants"], reverse=True)
                todo.extend(unknowns + ((branch.unknown, name),) for name in names)
                continue
            except _Blocked as blocked:
                return Result(blocked_by=blocked.errors)
            except Failure as failure:
                return self.blocked(failure.errors, owner.path)
            scenarios.append(Scenario(tuple(sorted(unknowns)), _rounded(mode, value), env.status, env.per_account))
        return Result(tuple(sorted(scenarios, key=lambda scenario: scenario.unknowns)))

    def cap_reading(self, cap: Entry, own: str) -> Result:
        fact = cap.doc["logic"]["reading"]
        readings = self.state["readings"]
        if fact not in readings:
            failure = Failure(error(STATE_READING_MISSING, {"fact": fact}, at(cap.ref, "logic", "reading")))
            return self.blocked(failure.errors, cap.path)
        return Result((Scenario((), Fraction(readings[fact]["value"]), own, True),))

    def threshold(self, threshold_id: str) -> Result:
        threshold = self.thresholds[threshold_id]
        key = ("offset" if threshold["form"] == "fact_offset" else "rule", threshold["from"])
        if key not in self.results:
            raise Need(key)
        result = self.results[key]
        if result.blocked:
            return result
        mode = threshold.get("round")
        return Result(tuple(replace(s, value=_rounded(mode, s.value)) for s in result.scenarios))

    # The run --------------------------------------------------------------------------------------

    def outputs(self) -> list[tuple[str, tuple[str, str]]]:
        """(output id, node) of every output: the value rules, the general logic of derives, the thresholds."""
        checker, found = self.checker, []
        for rule in sorted((r for r in checker.rules if checker.active(r)), key=lambda r: r.id):
            if rule.kind in VALUE_RULES:
                found.append(("rule:" + rule.id, ("rule", rule.id)))
                if rule.kind == "derive" and "general_logic" in rule.doc["logic"]:
                    found.append(("general_logic:" + rule.id, ("general_logic", rule.id)))
        found.extend(("threshold:" + tid, ("threshold", tid)) for tid in sorted(self.thresholds))
        return found

    def render(self, key: tuple[str, str], name: str = "value") -> dict[str, Any]:
        result = self.results[key]
        out: dict[str, Any] = {"status": result.status, "type": self.typed.outputs[key].token}
        if result.blocked:
            return out
        scenarios = result.scenarios
        out["per_account"] = any(s.per_account for s in scenarios)
        if len(scenarios) == 1 and not scenarios[0].unknowns:
            out[name] = _json(scenarios[0].value)
            return out
        out["variants"] = [{"unknowns": dict(s.unknowns), name: _json(s.value)} for s in scenarios]
        if all(type(s.value) is not bool for s in scenarios):
            values = sorted((s.value for s in scenarios), key=_magnitude)
            out["range"] = {"low": _json(values[0]), "high": _json(values[-1])}
        return out

    def notes(self) -> list[dict[str, Any]]:
        """`ruling_overrides_general_logic`: a derive whose value differs from its general logic."""
        found = []
        for rule in sorted((r for r in self.checker.rules if self.checker.active(r)), key=lambda r: r.id):
            if rule.kind != "derive" or "general_logic" not in rule.doc["logic"]:
                continue
            value, general = self.results[("rule", rule.id)], self.results[("general_logic", rule.id)]
            if value.blocked or general.blocked:
                continue
            values, generals = ([_text(v) for v in sorted({s.value for s in r.scenarios}, key=_magnitude)]
                                for r in (value, general))
            if values != generals:
                detail = {"path": rule.path, "rule": rule.id, "value": values, "general_logic": generals}
                found.append(error(RULING_OVERRIDES_GENERAL_LOGIC, detail, at(rule.ref, "logic", "general_logic")))
        return found

    def run(self) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        """Compute every output; return the run's `data` and its errors, each with the outputs it blocks."""
        outputs = self.outputs()
        solve([key for _, key in outputs], self.compute, self.results)
        needed_by: dict[int, list[str]] = {index: [] for index in range(len(self.errors))}
        for name, key in outputs:
            for index in self.results[key].blocked_by:
                needed_by[index].append(name)
        errors = [{**entry, "needed_by": sorted(needed_by[index])} for index, entry in enumerate(self.errors)]
        errors.sort(key=lambda entry: (entry["detail"]["path"], entry.get("ref", ""), entry["code"]))
        rules: dict[str, Any] = {}
        for name, key in outputs:
            if key[0] == "rule":
                rules[key[1]] = {"kind": self.checker.entries[key[1]].kind, **self.render(key)}
            elif key[0] == "general_logic":
                rules[key[1]]["general_logic"] = self.render(key)
        thresholds: dict[str, Any] = {}
        for threshold_id in sorted(self.thresholds):
            threshold, key = self.thresholds[threshold_id], ("threshold", threshold_id)
            if threshold["form"] == "constant":
                thresholds[threshold_id] = {"form": "constant", **self.render(key)}
                continue
            live = self.checker.entries[threshold["from"]].doc["logic"]["live"]
            thresholds[threshold_id] = {
                "form": "fact_offset",
                "fact": live["fact"],
                "comparison": live["comparison"],
                "reference_fact": live["reference_fact"],
                **self.render(key, "offset"),
            }
        return {"rules": rules, "thresholds": thresholds, "notes": self.notes()}, errors
