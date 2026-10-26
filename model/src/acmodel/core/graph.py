"""The derivation graph and its evaluation (contracts/formats.md, "The run").

**Nodes.** Each is computed after the nodes it reads:
- ("rule", id): the value of a derive (`value`, then its `round`), a cap (`value`, or its reading) or a
  limit (`max`);
- ("general_logic", id): a derive's `general_logic`, evaluated beside its value;
- ("offset", id): a derive's `live.offset`, the offset of its fact-offset form;
- ("flow", id, "consumes" | "produces", resource) and ("flow", id, "moves"): the amounts of a flow, which
  `cost`, `yield` and `max_single_income` read;
- ("member", unknown id, member): a member of a variants unknown;
- ("point", unknown id): an interval or distribution unknown at its points (`core.unknown`);
- ("threshold", id): a threshold of exports.json;
- ("general", <a rule, flow, member or point key>): that node from the rules and the data only, which
  `max_single_income` reads. It never reads the account state; a node that would need it is marked so
  (`needs_state`), and so is every such node that reads it;
- ("sampled", <an output's key>): with `unknowns.samples`, the Monte Carlo tallies of an output that reads a
  distribution (`_SampleGraph`).

**Scenarios.** A node that reads an unknown has one value per point of it: each variant, or the points of
an interval or distribution (`core.unknown`). The choices are found as the evaluation reaches them, so a
node depends only on the unknowns its value actually reads.

**Status.** A scenario's status is the worst of what it read: a rule's or data row's status, a reading
(confirmed). An unknown is judged by its effect instead (`core.unknown`, the no-effect rule): a scenario
records the unknowns it chose and those it depends on without choosing them (`unresolved`: what a pending
rule names, and the points a maximum ran over), and an output is provisional when one of them changes it at
export precision. A node that cannot be computed is blocked with its errors, and so is every node that
reads it; the others are still computed.

**Reads.** A scenario also records what its value read (`provenance` assembles the tree from them): the
graph nodes, and these, each a tuple of strings:
- ("row", table, *key): a data row;
- ("entry", rule id): a rule consulted for its logic without its value (the cap of `max_single_income`,
  and an income whose amount needs the account state, which the store's storage cap bounds instead);
- ("reading", fact), ("cleared", id), ("entity", class, id, attribute), ("entities", class): the account
  state.

**Cache** (#424 design section 5, `acmodel.cache`). With a cache session, every node computes through
`cached`: it is reused while its verifying trace holds, and computed again otherwise. A node's trace is
everything its computation read, in order, each with its value hash (`depend`). It holds every item of
the provenance record above (`_Environment.read` records both) and more:
- the graph nodes it read (`result`);
- leaves, each a tuple of strings and hashed from the inputs of this run (`leaf`): ("entry", id), the
  declaration (with its file, pointer and whether it is active); ("row", table, *key); ("reading", fact),
  the value only, so a fresh reading of the same value changes nothing; ("cleared", id);
  ("entity", class, id, attribute); ("entities", class), the ids of the class; and what the evaluation
  consults about the model's structure: ("typed", file, pointer), the typed expression; ("threshold_entry",
  id); ("table", id), its file, key, columns and form; ("table_keys", id), its rows' key cells in order;
  ("flow_parts", function, first, second); ("storage_cap", store); ("unknown_target", target).
A state value is depended on before its presence is checked, so a missing one is part of the trace too.
"""

from __future__ import annotations

import json
import math
import time
from dataclasses import dataclass, fields, replace
from fractions import Fraction
from typing import TYPE_CHECKING, Any, Callable

from acmodel.cache.keys import canonical, canonical_text, digest
from acmodel.check.checker import Checker, Entry
from acmodel.check.walk import at, nodes
from acmodel.codes import (
    DATA_REFERENCE_MISSING,
    EXPR_EVAL_FAILED,
    EXPR_OVERFLOW,
    RULE_PENDING_UNBOUNDED,
    RULING_OVERRIDES_GENERAL_LOGIC,
    STATE_ENTITY_MISSING,
    STATE_READING_MISSING,
)
from acmodel.config import Unknowns
from acmodel.core.common import (
    BLOCKED,
    CONFIRMED,
    RULE_STATUS,
    VALUE_RULES,
    Need,
    flow_parts,
    solve,
    unknown_target,
    worst,
)
from acmodel.core.types import Typed
from acmodel.core.unknown import (
    SAMPLED_FORMAT,
    Sampled,
    bounds,
    check_range,
    decode_tallies,
    encode_tallies,
    family,
    pick,
    points,
    quantize,
    render_sampled,
    rule_status,
    sample,
    sensitivity,
    status,
    stream,
    tallies,
    tally,
    unsupported,
    words,
)
from acmodel.envelope import Failure, error
from acmodel.expr import Call, Duration, Reference, Value, evaluate
from acmodel.model import readers

if TYPE_CHECKING:
    from acmodel.cache.session import Session

# The kinds of graph node (the first part of a node's key); every other first part names a leaf.
NODE_KINDS = ("rule", "general_logic", "offset", "flow", "member", "point", "threshold", "general", "sampled")


@dataclass(frozen=True)
class Scenario:
    unknowns: tuple[tuple[str, str], ...]  # (unknown id, point), sorted; empty when none was read
    value: Value
    status: str  # confirmed or provisional, from what the value read other than unknowns
    per_account: bool  # the value read the account state
    reads: tuple[tuple[str, ...], ...] = ()  # what the value read, in the order it was read
    unresolved: tuple[str, ...] = ()  # unknowns the value depends on without choosing them, sorted


@dataclass(frozen=True)
class Result:
    scenarios: tuple[Scenario, ...] = ()
    blocked_by: frozenset[int] = frozenset()  # the root errors that block it; empty when it is computed
    needs_state: bool = False  # a ("general", …) node that the rules and the data alone do not give

    @property
    def blocked(self) -> bool:
        return bool(self.blocked_by)

    @property
    def status(self) -> str:
        return BLOCKED if self.blocked else worst(*(scenario.status for scenario in self.scenarios))


# How a result is kept in the cache: every field of a scenario, so a field added to `Scenario` changes
# this, and with it every node's spec: values kept in another shape are never read back.
VALUE_FORMAT = {"scenario": [field.name for field in fields(Scenario)], "result": ["scenarios", "blocked", "needs_state"]}


def _encode_value(value: Value) -> Any:
    if type(value) is bool:
        return value
    if type(value) is Duration:
        return {"seconds": str(value.seconds)}
    return str(value)


def _decode_value(raw: Any) -> Value:
    if type(raw) is bool:
        return raw
    if type(raw) is dict:
        return Duration(Fraction(raw["seconds"]))
    return Fraction(raw)


def _tuples(raw: Any) -> Any:
    """JSON lists back to the tuples they were written from."""
    return tuple(_tuples(item) for item in raw) if type(raw) is list else raw


class _Branch(Exception):
    """The evaluation reached a variants unknown that its scenario has not chosen yet."""

    def __init__(self, unknown: str) -> None:
        super().__init__(unknown)
        self.unknown = unknown


class _Blocked(Exception):
    def __init__(self, errors: frozenset[int]) -> None:
        super().__init__(sorted(errors))
        self.errors = errors


class _NeedsState(Exception):
    """A ("general", …) node reached a value of the account state, which it never reads."""


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


def _at(ref: str) -> Callable[[_Environment, dict[str, str]], Value]:
    """The value of the expression at `ref`, for `Graph.expression`."""
    return lambda env, _: env.evaluate(ref)


class _Environment:
    """The value environment of one node in one scenario."""

    prefix: tuple[str, ...] = ()  # of the nodes its references read

    def __init__(
        self, graph: Graph, owner: Entry, unknowns: dict[str, str], status: str, unresolved: tuple[str, ...]
    ) -> None:
        self.graph = graph
        self.owner = owner
        self.ref = ""  # the pointer of the expression being evaluated (`evaluate`)
        self.unknowns = unknowns
        self.status = status
        self.unresolved = set(unresolved)
        self.per_account = False
        self.reads: dict[tuple[str, ...], None] = {}  # what the value read, once each, in order

    def evaluate(self, ref: str) -> Value:
        """The value of the owner's expression at `ref` in this scenario."""
        self.ref = ref
        self.graph.depend(("typed", self.owner.path, ref))
        return evaluate(self.graph.typed.expressions[(self.owner.path, ref)], self, max_bits=self.graph.max_bits)

    def fail(self, code: str, **detail: Any) -> None:
        raise Failure(error(code, {key: value for key, value in detail.items() if value is not None}, self.ref))

    def read(self, item: tuple[str, ...]) -> None:
        """What the value read: the scenario's provenance record, and the node's trace."""
        self.reads[item] = None
        self.graph.depend(item)

    def use(self, scenario: Scenario) -> Value:
        self.status = worst(self.status, scenario.status)
        self.unresolved.update(scenario.unresolved)
        self.per_account = self.per_account or scenario.per_account
        return scenario.value

    def consult(self, rule: Entry) -> None:
        """A rule read for its logic: its status counts, and a pending rule's unknowns stay unresolved."""
        own, unresolved = rule_status(rule)
        self.status = worst(self.status, own)
        self.unresolved.update(unresolved)

    def consistent(self, result: Result) -> list[Scenario]:
        return [s for s in result.scenarios if all(self.unknowns.get(u, v) == v for u, v in s.unknowns)]

    def node(self, key: tuple[str, ...]) -> Value:
        """The value of node `key` in this scenario; a choice it needs and this scenario lacks branches it."""
        candidates = self.consistent(self.graph.result(self.prefix + key))
        for scenario in candidates:
            if all(unknown in self.unknowns for unknown, _ in scenario.unknowns):
                self.read(self.prefix + key)
                return self.use(scenario)
        raise _Branch(next(u for u, _ in candidates[0].unknowns if u not in self.unknowns))

    # The environment the evaluator calls ----------------------------------------------------------

    def reference(self, ref: Reference) -> Value:
        checker, kind, target = self.graph.checker, ref.kind, ref.target
        if kind in ("rule", "cap"):
            return self.node(("rule", target))
        if kind == "unknown":
            self.graph.depend(("unknown_target", target))
            entry, member = unknown_target(checker, target)
            if member is not None:
                return self.node(("member", entry.id, member))
            found = unsupported(entry)
            if found is not None:
                raise _unsupported("reference", ref.position, found, self.ref)
            return self.node(("point", entry.id))
        if kind == "read":
            return self.reading(target, ref.position)
        if kind in ("amount", "stock"):
            self.graph.depend(("entry", target))
            fact = checker.entries[target].doc.get("reading")
            if fact is None:
                self.per_account = True
                self.fail(STATE_READING_MISSING, entity=target, position=ref.position)
            return self.reading(fact, ref.position)
        if kind == "cleared":
            self.per_account = True
            self.read(("cleared", target))
            return target in self.graph.cleared
        if kind == "entity":
            return self.attribute(ref)
        if kind == "data":
            return self.cell(ref)
        raise _unsupported("reference", ref.position, kind, self.ref)  # metric, count, pref

    def members(self, ref: Reference) -> list[tuple[str, ...]]:
        if ref.kind == "data":
            table = self.table(ref.target, ref.position, "reference")
            self.graph.depend(("table_keys", ref.target))
            for key in table.rows:  # the set is every row of the table
                self.read(("row", ref.target, *key))
            return list(table.rows)
        self.per_account = True
        self.read(("entities", ref.target))
        entities = self.graph.state["entities"]
        if ref.target not in entities:
            self.fail(STATE_ENTITY_MISSING, missing="entity_class", entity_class=ref.target, position=ref.position)
        return [(entity,) for entity in sorted(entities[ref.target])]

    def flow(self, function: str, first: str, second: str) -> Value:
        self.graph.depend(("flow_parts", function, first, second))
        parts = flow_parts(self.graph.checker, function, first, second)
        if function != "max_single_income":
            total: Value = Fraction(0)
            for key in parts:
                total += self.node(key)
            return total
        return self.max_single_income(first, second, parts)

    def max_single_income(self, resource: str, cap: str, parts: list[tuple[str, ...]]) -> Value:
        """The largest single income the cap blocks, from the rules and the data only, in every environment:
        over the incomes and over the variants this scenario leaves open. An income counts with its amount,
        and a move out of a store with at most the store's storage cap (one move takes at most a full store).
        An income that would need the account state and has no such cap is `evaluation_unsupported`
        (`found` `state`); the state is never read in its place."""
        checker, found = self.graph.checker, []
        self.read(("entry", cap))  # which incomes it blocks
        for key in parts:
            self.graph.depend(("entry", key[1]))
            self.consult(checker.entries[key[1]])
            bound = self.store_bound(key)
            try:
                largest = self.largest(("general",) + key)
            except _NeedsState:
                if bound is None:
                    raise self.state_unsupported(resource, cap) from None
                self.read(("entry", key[1]))  # the income, which the storage cap bounds
                found.append(bound)
                continue
            found.append(largest if bound is None else min(largest, bound, key=_magnitude))
        return max(found, key=_magnitude)

    def largest(self, key: tuple[str, ...]) -> Value:
        """The largest value of node `key` over the points this scenario leaves open. The maximum chooses
        none of them, so their unknowns stay unresolved."""
        scenarios = self.consistent(self.graph.result(key))
        self.read(key)
        self.unresolved.update(u for s in scenarios for u, _ in s.unknowns if u not in self.unknowns)
        return max((self.use(s) for s in scenarios), key=_magnitude)

    def store_bound(self, part: tuple[str, ...]) -> Value | None:
        """For a move out of a store: the largest value of the store's storage cap, from the rules and the
        data only; None for any other income, or when that cap is a reading."""
        if part[2] != "moves":
            return None
        checker = self.graph.checker
        source = checker.entries[part[1]].doc["logic"]["moves"]["from"]
        self.graph.depend(("entry", source))
        store = checker.entries.get(source)
        if store is not None and store.kind == "store":
            self.graph.depend(("storage_cap", store.id))
        cap = checker.storage_cap(store.id) if store is not None and store.kind == "store" else None
        if cap is None:
            return None
        try:
            return self.largest(("general", "rule", cap.id))
        except _NeedsState:
            return None

    def state_unsupported(self, resource: str, cap: str) -> Failure:
        expression = self.graph.typed.expressions[(self.owner.path, self.ref)]
        position = next(
            item.position for item in nodes(expression.root)
            if isinstance(item, Call) and item.function == "max_single_income" and item.args == (resource, cap)
        )
        return _unsupported("max_single_income", position, "state", self.ref)

    def lookup(self, table_id: str, column: str, keys: tuple[Value, ...]) -> Value:
        position = self.graph.typed.lookups[(self.owner.path, self.ref)][(table_id, column)]
        table = self.table(table_id, position, "lookup")
        self.graph.depend(("table_keys", table_id))
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
        self.read(("reading", fact))
        readings = self.graph.state["readings"]
        if fact not in readings:
            self.fail(STATE_READING_MISSING, fact=fact, position=position)
        return Fraction(readings[fact]["value"])

    def attribute(self, ref: Reference) -> Any:
        """`entity:<class>[<id>].<attr>`: the entity and its attribute must be in the state."""
        self.per_account = True
        entities, entity_class, (entity,), name = self.graph.state["entities"], ref.target, ref.key, ref.attr
        self.read(("entity", entity_class, entity, name))
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
        self.graph.depend(("entry", entity_class))
        spec = self.graph.checker.entries[entity_class].doc["attributes"][name]
        return _convert(spec, attributes[name])

    def table(self, table_id: str, position: int, operator: str):
        self.graph.depend(("table", table_id))
        table = self.graph.checker.tables[table_id]
        if table.rows is None:  # an external dataset: its rows are read by a later slice
            raise _unsupported(operator, position, "external_dataset", self.ref)
        return table

    def cell(self, ref: Reference) -> Any:
        table = self.table(ref.target, ref.position, "reference")
        return self.row_value(ref.target, table, ref.key, ref.attr, ref.position)

    def row_value(self, table_id: str, table, key: tuple[str, ...], column: str, position: int | None) -> Any:
        self.read(("row", table_id, *key))
        row = table.file.doc["rows"][table.rows[key]]
        if row["status"] == "pending":  # a row cannot name an unknown, so a pending row has no range
            self.fail(
                RULE_PENDING_UNBOUNDED, id=f"{table_id}[{','.join(key)}]", reason="no_unknown", position=position
            )
        self.status = worst(self.status, RULE_STATUS[row["status"]])
        return _convert(table.file.doc["columns"][column], row[column])


class _General(_Environment):
    """The environment of a ("general", …) node: the rules and the data only. Its references read
    ("general", …) nodes, and a value of the account state ends the node (`_NeedsState`) instead of
    being read."""

    prefix = ("general",)
    STATE = ("read", "amount", "stock", "cleared", "entity")

    def reference(self, ref: Reference) -> Value:
        if ref.kind in self.STATE:
            raise _NeedsState()
        return super().reference(ref)

    def members(self, ref: Reference) -> list[tuple[str, ...]]:
        if ref.kind != "data":  # the entities of a class are the account's
            raise _NeedsState()
        return super().members(ref)


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
        settings: Unknowns = Unknowns(),
        cache: Session | None = None,
    ) -> None:
        self.checker = checker
        self.typed = typed
        self.state = state
        self.cleared = frozenset(state["cleared"])
        self.exports = exports
        self.thresholds = {threshold["id"]: threshold for threshold in exports["thresholds"]}
        self.max_bits = max_bits
        self.settings = settings  # the keys under `unknowns`
        # The distribution unknowns that sampling draws, and the sample of each (`_SampleGraph` only).
        self.distributions = sorted(
            u.id for u in checker.unknowns
            if checker.entries.get(u.id) is u and u.kind == "distribution" and "distribution" in u.doc
            and unsupported(u) is None
        )
        self.words: dict[str, int] = {}
        self.results: dict[tuple[str, ...], Result | Sampled] = {}
        self.errors: list[dict[str, Any]] = []  # the root errors, each once
        self._seen: dict[str, int] = {}
        self.cache = cache
        self.hashes: dict[tuple[str, ...], str] = {}  # with a cache: each computed node's value hash
        self._leaves: dict[tuple[str, ...], str] = {}  # each leaf's value hash in this run
        self._trace: dict[tuple[str, ...], str] | None = None  # what the node being computed read, in order

    # The cache --------------------------------------------------------------------------------------

    def depend(self, item: tuple[str, ...]) -> None:
        """Record that the node being computed read `item`, a graph node or a leaf, with its value hash."""
        if self._trace is None or item in self._trace:
            return
        self._trace[item] = self.hashes[item] if item[0] in NODE_KINDS else self.leaf_hash(item)

    def leaf_hash(self, item: tuple[str, ...]) -> str:
        if item not in self._leaves:
            self._leaves[item] = digest(canonical(self.leaf(item)))
        return self._leaves[item]

    def leaf(self, item: tuple[str, ...]) -> Any:
        """The value of a leaf in this run's inputs (None for what is absent)."""
        kind, args, checker, state = item[0], item[1:], self.checker, self.state
        if kind == "entry":
            entry = checker.entries.get(args[0])
            if entry is None:
                return None
            return {
                "category": entry.category, "kind": entry.kind, "path": entry.path, "ref": entry.ref,
                "active": checker.active(entry), "doc": entry.doc,
            }
        if kind == "typed":
            expression = self.typed.expressions.get((args[0], args[1]))
            if expression is None:
                return None
            lookups = self.typed.lookups.get((args[0], args[1]), {})
            return {
                "expression": repr(expression),
                "lookups": sorted([table, column, position] for (table, column), position in lookups.items()),
            }
        if kind == "threshold_entry":
            return self.thresholds.get(args[0])
        if kind == "reading":
            reading = state["readings"].get(args[0])
            return None if reading is None else reading["value"]
        if kind == "cleared":
            return args[0] in self.cleared
        if kind == "entity":
            entity_class, entity, name = args
            entities = state["entities"]
            if entity_class not in entities:
                return {"missing": "entity_class"}
            if entity not in entities[entity_class]:
                return {"missing": "entity"}
            attributes = entities[entity_class][entity]
            return {"missing": "attribute"} if name not in attributes else {"value": attributes[name]}
        if kind == "entities":
            found = state["entities"].get(args[0])
            return None if found is None else sorted(found)
        table = checker.tables.get(args[0]) if kind in ("table", "table_keys", "row") else None
        if kind == "table":
            if table is None:
                return None
            doc = table.file.doc
            return {"path": table.file.path, "key": list(table.keys), "columns": doc["columns"], "external": table.rows is None}
        if kind == "table_keys":
            if table is None or table.rows is None:
                return None
            rows = table.file.doc["rows"]
            return [[rows[index][name] for name in table.keys] for index in table.rows.values()]
        if kind == "row":
            index = None if table is None or table.rows is None else table.rows.get(tuple(args[1:]))
            return None if index is None else table.file.doc["rows"][index]
        if kind == "flow_parts":
            return [list(part) for part in flow_parts(checker, args[0], args[1], args[2])]
        if kind == "storage_cap":
            cap = checker.storage_cap(args[0])
            return None if cap is None else cap.id
        if kind == "unknown_target":
            entry, member = unknown_target(checker, args[0])
            return [entry.id, entry.kind, member]
        raise ValueError(f"no leaf of kind {kind!r}")

    def current(self, item: tuple[str, ...]) -> str:
        """The value hash of `item` in this run, for checking a trace; a graph node is computed first."""
        if item[0] in NODE_KINDS:
            if item not in self.results:
                raise Need(item)
            return self.hashes[item]
        return self.leaf_hash(item)

    def cached(self, key: tuple[str, ...]) -> Result:
        """`compute` through the cache: the kept value of a trace that still holds, or the node computed
        again, its trace and value kept."""
        cache, kind = self.cache, key[0]
        spec_doc: dict[str, Any] = {"node": list(key), "max_bits": self.max_bits, "format": VALUE_FORMAT}
        if kind == "sampled":  # its value is its tallies, which these keys decide
            spec_doc.update(format=SAMPLED_FORMAT, unknowns=self.settings.keyed())
        spec = cache.spec(kind, spec_doc)
        hit = cache.lookup(kind, spec, self.current)
        if hit is not None:
            value, blob = hit
            self.hashes[key] = value
            return self.decode(kind, blob)
        self._trace = {}
        started = time.perf_counter_ns()
        try:
            result = self.compute(key)
            trace = [[list(item), value] for item, value in self._trace.items()]
        finally:
            self._trace = None
        blob = canonical(self.encode(result))
        compute_us = (time.perf_counter_ns() - started) // 1000
        self.hashes[key] = cache.save(kind, spec, list(key), trace, blob, compute_us)
        return result

    def encode(self, result: Result | Sampled) -> dict[str, Any]:
        blocked = sorted((self.errors[index] for index in result.blocked_by), key=canonical_text)
        if isinstance(result, Sampled):
            return {"tallies": encode_tallies(result), "blocked": blocked}
        scenarios = [
            {name: _encode_value(getattr(s, name)) if name == "value" else getattr(s, name) for name in VALUE_FORMAT["scenario"]}
            for s in result.scenarios
        ]
        return {"scenarios": scenarios, "blocked": blocked, "needs_state": result.needs_state}

    def decode(self, kind: str, blob: bytes) -> Result | Sampled:
        doc = json.loads(blob)
        if kind == "sampled":
            return Sampled(decode_tallies(doc["tallies"]), self._register(doc["blocked"]))
        scenarios = tuple(
            Scenario(**{name: _decode_value(raw) if name == "value" else _tuples(raw) for name, raw in s.items()})
            for s in doc["scenarios"]
        )
        return Result(scenarios, self._register(doc["blocked"]), doc["needs_state"])

    # Results ----------------------------------------------------------------------------------------

    def result(self, key: tuple[str, ...]) -> Result:
        if key not in self.results:
            raise Need(key)
        self.depend(key)
        result = self.results[key]
        if result.blocked:
            raise _Blocked(result.blocked_by)
        if result.needs_state:
            raise _NeedsState()
        return result

    def blocked(self, errors: list[dict[str, Any]], path: str) -> Result:
        return Result(blocked_by=self._register([{**entry, "detail": {"path": path, **entry["detail"]}} for entry in errors]))

    def _register(self, errors: list[dict[str, Any]]) -> frozenset[int]:
        """The indices of `errors` among the run's root errors, each kept once."""
        indices = set()
        for entry in errors:
            text = json.dumps(entry, sort_keys=True)
            if text not in self._seen:
                self._seen[text] = len(self.errors)
                self.errors.append(entry)
            indices.add(self._seen[text])
        return frozenset(indices)

    # Nodes --------------------------------------------------------------------------------------

    def compute(self, key: tuple[str, ...]) -> Result | Sampled:
        if key[0] == "threshold":
            return self.threshold(key[1])
        if key[0] == "sampled":
            return self.sampled(key[1:])
        general = key[0] == "general"  # the same node from the rules and the data only
        if general:
            key = key[1:]
        self.depend(("entry", key[1]))  # first: what the node is, and so what it reads next
        entry = self.checker.entries[key[1]]
        if key[0] == "member":
            def member(env: _Environment, unknowns: dict[str, str]) -> Value:
                if entry.id not in unknowns:
                    raise _Branch(entry.id)
                return env.evaluate(at(entry.ref, "variants", unknowns[entry.id], key[2]))

            return self.expression(entry, member, CONFIRMED, general=general)
        if key[0] == "point":
            return self.point(entry, general)
        (own, unresolved), logic = rule_status(entry), entry.doc["logic"]
        if "when" in entry.doc:  # a conditional value: evaluated by a later slice
            return self.blocked(_unsupported("when", 0, "when", at(entry.ref, "when")).errors, entry.path)
        if key[0] == "rule" and entry.kind == "cap" and "value" not in logic:
            return Result(needs_state=True) if general else self.cap_reading(entry, own, unresolved)
        if key[0] == "rule":
            member = ("max",) if entry.kind == "limit" else ("value",)
            ref = at(entry.ref, "logic", *member)
            return self.expression(entry, _at(ref), own, logic.get("round"), general=general, unresolved=unresolved)
        if key[0] == "general_logic":
            return self.expression(entry, _at(at(entry.ref, "logic", "general_logic")), own, unresolved=unresolved)
        if key[0] == "offset":
            return self.expression(entry, _at(at(entry.ref, "logic", "live", "offset")), own, unresolved=unresolved)
        ref = at(entry.ref, "logic", "moves", "amount") if key[2] == "moves" else at(entry.ref, "logic", key[2], key[3])
        self.depend(("typed", entry.path, ref))
        if (entry.path, ref) not in self.typed.expressions:  # a `random:<id>` product
            return self.blocked(_unsupported(key[2], 0, "random", ref).errors, entry.path)
        return self.expression(entry, _at(ref), own, general=general, unresolved=unresolved)

    def expression(
        self,
        owner: Entry,
        value_of: Callable[[_Environment, dict[str, str]], Value],
        own: str,
        mode: str | None = None,
        *,
        general: bool = False,
        unresolved: tuple[str, ...] = (),
    ) -> Result:
        """The value `value_of` computes in each scenario's environment (from the rules and the data only
        when `general`), once per scenario."""
        scenarios = []
        todo: list[tuple[tuple[str, str], ...]] = [()]
        while todo:
            unknowns = todo.pop()
            chosen = dict(unknowns)
            env = (_General if general else _Environment)(self, owner, chosen, own, unresolved)
            try:
                value = value_of(env, chosen)
            except _Branch as branch:
                self.depend(("entry", branch.unknown))
                names = points(self.checker.entries[branch.unknown])
                todo.extend(unknowns + ((branch.unknown, name),) for name in reversed(names))
                continue
            except _Blocked as blocked:
                return Result(blocked_by=blocked.errors)
            except _NeedsState:
                return Result(needs_state=True)
            except Failure as failure:
                return self.blocked(failure.errors, owner.path)
            scenarios.append(Scenario(
                tuple(sorted(unknowns)), _rounded(mode, value), env.status, env.per_account, tuple(env.reads),
                tuple(sorted(env.unresolved)),
            ))
        return Result(tuple(sorted(scenarios, key=lambda scenario: scenario.unknowns)))

    def point(self, entry: Entry, general: bool) -> Result:
        """An interval or distribution unknown at each of its points (`core.unknown`), or at its sample in a
        `_SampleGraph`. Every bound is evaluated in each scenario and must be in order (`range_invalid`)."""
        refs, word, empirical = bounds(entry), self.words.get(entry.id), family(entry) == "empirical"

        def value_of(env: _Environment, unknowns: dict[str, str]) -> Value:
            if word is not None:
                name = pick(word, len(refs)) if empirical else None
            elif entry.id not in unknowns:
                raise _Branch(entry.id)
            else:
                name = unknowns[entry.id]
            if empirical:
                return env.evaluate(refs[name])
            values = {bound: env.evaluate(ref) for bound, ref in refs.items()}
            check_range(entry, values)
            return values[name] if word is None else sample(entry, values, word, self.settings.resolution)

        return self.expression(entry, value_of, CONFIRMED, general=general)

    def cap_reading(self, cap: Entry, own: str, unresolved: tuple[str, ...]) -> Result:
        fact = cap.doc["logic"]["reading"]
        self.depend(("reading", fact))
        readings = self.state["readings"]
        if fact not in readings:
            failure = Failure(error(STATE_READING_MISSING, {"fact": fact}, at(cap.ref, "logic", "reading")))
            return self.blocked(failure.errors, cap.path)
        return Result((Scenario((), Fraction(readings[fact]["value"]), own, True, (("reading", fact),), unresolved),))

    # Sampling ---------------------------------------------------------------------------------------

    def involves(self, result: Result | Sampled) -> bool:
        """Whether a computed node read a distribution unknown."""
        if not isinstance(result, Result):
            return False
        return any(unknown in self.distributions for s in result.scenarios for unknown, _ in s.unknowns)

    def place(self, output: tuple[str, ...]) -> tuple[str, str]:
        """The file and pointer an error of an output's sampling points at: the expression it comes from."""
        kind, name = output
        if kind == "threshold":
            threshold = self.thresholds[name]
            kind, name = ("offset" if threshold["form"] == "fact_offset" else "rule"), threshold["from"]
        entry = self.checker.entries[name]
        member = {"general_logic": ("general_logic",), "offset": ("live", "offset")}.get(kind)
        if member is None:
            member = ("max",) if entry.kind == "limit" else ("value",)
        return entry.path, at(entry.ref, "logic", *member)

    def sampled(self, output: tuple[str, ...]) -> Sampled:
        """The Monte Carlo tallies of an output that reads a distribution: for each of `unknowns.samples`
        samples, the output computed with every distribution unknown at that sample (`_SampleGraph`), and its
        value at export precision tallied per choice of its other unknowns. A sample that cannot be computed,
        or a tally beyond the int64 range (`expr_overflow`, operator `tally`), blocks the output."""
        if output not in self.results:
            raise Need(output)
        self.depend(output)
        settings = self.settings
        streams = {unknown: stream(unknown) for unknown in self.distributions}
        base = {key: result for key, result in self.results.items() if isinstance(result, Result) and not self.involves(result)}
        found: dict[tuple[tuple[str, str], ...], list[int]] = {}
        for start in range(0, settings.samples, settings.chunk_size):
            count = min(settings.chunk_size, settings.samples - start)
            chunk = {unknown: words(settings.seed, sid, start, count) for unknown, sid in streams.items()}
            for index in range(count):
                graph = _SampleGraph(self, base, {unknown: chunk[unknown][index] for unknown in streams})
                solve([output], graph.compute, graph.results)
                result = graph.results[output]
                if result.blocked:
                    return Sampled(blocked_by=result.blocked_by)
                for scenario in result.scenarios:
                    over = tally(found, scenario.unknowns, quantize(scenario.value, settings.scale))
                    if over is not None:
                        path, ref = self.place(output)
                        detail = {"operator": "tally", "bits": abs(over).bit_length(), "max_bits": 63, "position": 0}
                        return Sampled(blocked_by=self.blocked([error(EXPR_OVERFLOW, detail, ref)], path).blocked_by)
        return Sampled(tallies(found))

    def blocked_by(self, key: tuple[str, ...]) -> frozenset[int]:
        """The root errors that block an output: its own, and its sampling's."""
        sampled = self.results.get(("sampled", *key))
        return self.results[key].blocked_by | (sampled.blocked_by if sampled is not None else frozenset())

    def threshold(self, threshold_id: str) -> Result:
        self.depend(("threshold_entry", threshold_id))
        threshold = self.thresholds[threshold_id]
        key = ("offset" if threshold["form"] == "fact_offset" else "rule", threshold["from"])
        if key not in self.results:
            raise Need(key)
        self.depend(key)
        result = self.results[key]
        if result.blocked:
            return result
        mode = threshold.get("round")
        return Result(tuple(replace(s, value=_rounded(mode, s.value), reads=(key,)) for s in result.scenarios))

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
        """An output as `run` shows it. Its status follows the no-effect rule (`core.unknown`); an output that
        read an unknown lists its `sensitivity`, and a sampled one its `sampled` tallies."""
        result, settings = self.results[key], self.settings
        if self.blocked_by(key):
            return {"status": BLOCKED, "type": self.typed.outputs[key].token}
        scenarios = result.scenarios
        out: dict[str, Any] = {"status": status(scenarios, settings.scale), "type": self.typed.outputs[key].token}
        out["per_account"] = any(s.per_account for s in scenarios)
        if len(scenarios) == 1 and not scenarios[0].unknowns:
            out[name] = _json(scenarios[0].value)
        else:
            out["variants"] = [{"unknowns": dict(s.unknowns), name: _json(s.value)} for s in scenarios]
            if all(type(s.value) is not bool for s in scenarios):
                values = sorted((s.value for s in scenarios), key=_magnitude)
                out["range"] = {"low": _json(values[0]), "high": _json(values[-1])}
        listing = sensitivity(scenarios, settings.scale, settings.top_n)
        if listing is not None:
            out["sensitivity"] = listing
        sampled = self.results.get(("sampled", *key))
        if sampled is not None:
            out["sampled"] = render_sampled(sampled, settings.scale)
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
        compute = self.compute if self.cache is None else self.cached
        solve([key for _, key in outputs], compute, self.results)
        if self.settings.samples:
            solve([("sampled", *key) for _, key in outputs if self.involves(self.results[key])], compute, self.results)
        needed_by: dict[int, list[str]] = {index: [] for index in range(len(self.errors))}
        for name, key in outputs:
            for index in self.blocked_by(key):
                needed_by[index].append(name)
        errors = [{**entry, "needed_by": sorted(needed_by[index])} for index, entry in enumerate(self.errors)]
        # Fully ordered, so the order never depends on which nodes were computed and which were reused.
        errors.sort(key=lambda entry: (entry["detail"]["path"], entry.get("ref", ""), entry["code"], canonical_text(entry)))
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


class _SampleGraph(Graph):
    """The graph at one sample of the distributions (`Graph.sampled`): every distribution unknown is at its
    sample, and the other unknowns at their points.

    The nodes that read no distribution are the main graph's, and are not computed again. This graph computes
    without the cache, inside the computation of the main graph's sampled node: what it reads it forwards to
    that node's trace, so the sampled node is reused exactly while everything its samples read is unchanged.
    It registers its errors among the main graph's.
    """

    def __init__(self, main: Graph, base: dict[tuple[str, ...], Result], sample: dict[str, int]) -> None:
        super().__init__(main.checker, main.typed, main.state, main.exports, max_bits=main.max_bits, settings=main.settings)
        self.main = main
        self.results = dict(base)
        self.copied = base.keys()
        self.errors, self._seen = main.errors, main._seen
        self.words = sample

    def depend(self, item: tuple[str, ...]) -> None:
        if item[0] in NODE_KINDS and item not in self.copied:
            return  # computed in this graph, which forwards what it reads as it reads it
        self.main.depend(item)
