"""The run's model checks, before anything is computed (contracts/formats.md, "The run").

`check(checker, exports, load_prefs)` gives every expression of the model its type, or raises `Failure`
with every problem found:
- **dimensions** (`expr_dimension_mismatch`): inside every expression, with the types of `read:`, `pref:`
  and `metric:` values, and between an expression and the dimension its member expects;
- **preference keys** (`rule_reference_missing`, `expected` `pref`): every key that an expression or a
  gate's `prefs` names is in the preference defaults;
- **what the evaluation needs to be sound** (`rule_reference_missing`): a `rule:` reference names a
  derive, cap or limit; a cap's subject is a resource or a store; `cost`, `yield` and
  `max_single_income` have a flow behind them; each threshold names a derive in the form it needs.

The third check the static check leaves to the run, the entity ids of the state, is the evaluation's
(`graph`).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable

from acmodel.check.checker import RANDOM_PRODUCT, Checker, Entry
from acmodel.check.files import File
from acmodel.check.walk import at, expressions, nodes
from acmodel.codes import EXPR_DIMENSION_MISMATCH, MODEL_ID_DUPLICATE, RULE_REFERENCE_MISSING
from acmodel.core.common import (
    VALUE_RULES,
    Need,
    cap_dimension,
    dimension,
    fact_dimensions,
    fits,
    flow_parts,
    held,
    solve,
    unify,
    unknown_target,
)
from acmodel.envelope import Failure, error
from acmodel.expr import BOOL, COUNT, NONE, NUMBER, TIME, VALUE, Call, Expression, Reference, Type, amount, parse
from acmodel.expr import check as typecheck
from acmodel.expr.syntax import FLOW_FUNCTIONS
from acmodel.model import readers
from acmodel.model.shape import json_type


class _Skip(Exception):
    """A type that cannot be given, because what it rests on was refused with an error already reported."""


@dataclass(frozen=True)
class Source:
    """One expression of the model: the rule or unknown that holds it, its JSON pointer and its text."""

    owner: Entry
    ref: str
    text: str

    @property
    def role(self) -> tuple[str, ...]:
        """The pointer below the owner as tokens: ("logic", "value"), ("when",), ..."""
        return tuple(self.ref[len(self.owner.ref) + 1:].split("/"))


@dataclass(frozen=True)
class Typed:
    """What the graph evaluates: every typed expression, the type of each output, and where in each
    expression its `lookup` calls are (`lookup` reaches the environment without its position)."""

    expressions: dict[tuple[str, str], Expression]  # (file, pointer) -> typed expression
    outputs: dict[tuple[str, str], Type]  # ("rule" | "general_logic" | "threshold", id) -> type
    lookups: dict[tuple[str, str], dict[tuple[str, str], int]]  # (file, pointer) -> (table, column) -> position


def _mismatch(operator: str, operands: list[Type | str], position: int, ref: str) -> Failure:
    tokens = [kind.token if isinstance(kind, Type) else kind for kind in operands]
    return Failure(error(EXPR_DIMENSION_MISMATCH, {"operator": operator, "operands": tokens, "position": position}, ref))


def _missing(target: str, expected: list[str], ref: str, **values: Any) -> Failure:
    return Failure(error(RULE_REFERENCE_MISSING, {"target": target, "expected": expected, **values}, ref))


class _Environment:
    """The type environment of one expression."""

    def __init__(self, types: _Types, source: Source) -> None:
        self.types = types
        self.source = source

    def reference(self, ref: Reference) -> Type:
        return self.types.reference(ref, self.source)

    def lookup(self, table: str, column: str, keys: tuple[Type, ...]) -> Type:
        position = self.types.lookups[(self.source.owner.path, self.source.ref)][(table, column)]
        spec = self.types.checker.tables[table].file.doc["columns"][column]
        return self.types.column(spec, position, self.source)


class _Types:
    def __init__(self, checker: Checker, exports: File, load_prefs: Callable[[], dict[str, Any]]) -> None:
        self.checker = checker
        self.exports = exports
        self.errors: list[dict[str, Any]] = []
        self.memo: dict[tuple[str, ...], Type | None] = {}
        self.typed: dict[tuple[str, str], Expression] = {}
        self.sources = list(self._sources())
        self.by_key = {(source.owner.path, source.ref): source for source in self.sources}
        self.parsed = {key: parse(source.text, ref=source.ref) for key, source in self.by_key.items()}
        self.lookups = {key: self._lookups(parsed) for key, parsed in self.parsed.items()}
        self.facts = fact_dimensions(checker)
        self.prefs = load_prefs()["defaults"] if self._reads_prefs() else None

    # What is checked ---------------------------------------------------------------------------

    def _sources(self):
        checker = self.checker
        for rule in checker.rules:
            if not checker.active(rule):
                continue
            produces = at(rule.ref, "logic", "produces") + "/"
            for ref, text in expressions(readers.RULE, rule.doc, rule.ref):
                if not (ref.startswith(produces) and RANDOM_PRODUCT.fullmatch(text)):
                    yield Source(rule, ref, text)
        for unknown in checker.unknowns:
            if checker.entries.get(unknown.id) is unknown:
                for ref, text in expressions(readers.UNKNOWN, unknown.doc, unknown.ref):
                    yield Source(unknown, ref, text)

    @staticmethod
    def _lookups(parsed: Expression) -> dict[tuple[str, str], int]:
        found: dict[tuple[str, str], int] = {}
        for node in nodes(parsed.root):
            if isinstance(node, Call) and node.function == "lookup":
                found.setdefault((node.args[0], node.args[1]), node.position)
        return found

    def _reads_prefs(self) -> bool:
        if any(
            isinstance(node, Reference) and node.kind == "pref"
            for parsed in self.parsed.values()
            for node in nodes(parsed.root)
        ):
            return True
        return any(
            self.checker.active(rule) and rule.kind == "gate" and rule.doc["logic"].get("prefs")
            for rule in self.checker.rules
        )

    def record(self, errors: list[dict[str, Any]], path: str) -> None:
        self.errors.extend({**entry, "detail": {"path": path, **entry["detail"]}} for entry in errors)

    def run(self) -> Typed:
        checker = self.checker
        keys: list[tuple[str, ...]] = [("expr", *key) for key in self.by_key]
        for unknown in checker.unknowns:
            if checker.entries.get(unknown.id) is not unknown or not unknown.doc.get(unknown.kind):
                continue
            if unknown.kind == "variants":
                members = sorted({member for variant in unknown.doc["variants"].values() for member in variant})
                keys.extend(("member", unknown.id, member) for member in members)
            else:
                keys.append(("range", unknown.id))
        solve(keys, self.compute, self.memo)
        for source in self.sources:
            typed = self.typed.get((source.owner.path, source.ref))
            if typed is not None:
                self.expect(source, typed)
                self.lookup_keys(source, typed)
            self.flows(source)
        self.caps_and_gates()
        outputs = self.outputs()
        outputs.update(self.thresholds())
        if self.errors:
            raise Failure(*self.errors)
        return Typed(self.typed, outputs, self.lookups)

    # Types -------------------------------------------------------------------------------------

    def need(self, key: tuple[str, ...]) -> Type:
        if key not in self.memo:
            raise Need(key)
        kind = self.memo[key]
        if kind is None:
            raise _Skip(key)
        return kind

    def compute(self, key: tuple[str, ...]) -> Type | None:
        if key[0] == "expr":
            return self.expression(self.by_key[key[1], key[2]])
        unknown = self.checker.entries[key[1]]
        if key[0] == "member":
            refs = [at(unknown.ref, "variants", name, key[2]) for name, members in unknown.doc["variants"].items()
                    if key[2] in members]
            return self.joined(unknown, refs, "variants")
        refs = [ref for ref, _ in expressions(readers.UNKNOWN, unknown.doc, unknown.ref)]
        if not refs:  # beta_from_samples: a probability
            return NONE
        return self.joined(unknown, refs, unknown.kind)

    def expression(self, source: Source) -> Type | None:
        key = (source.owner.path, source.ref)
        try:
            typed = typecheck(self.parsed[key], _Environment(self, source))
        except Failure as failure:
            self.record(failure.errors, source.owner.path)
            return None
        except _Skip:
            return None
        self.typed[key] = typed
        return typed.type

    def joined(self, unknown: Entry, refs: list[str], operator: str) -> Type | None:
        """The one type of the expressions at `refs` (the variants of a member, the bounds of a range)."""
        found = [self.memo.get(("expr", unknown.path, ref)) for ref in refs]
        for ref in refs:
            if ("expr", unknown.path, ref) not in self.memo:
                raise Need(("expr", unknown.path, ref))
        if any(kind is None for kind in found):
            return None
        joined = found[0]
        for ref, kind in zip(refs[1:], found[1:]):
            together = unify(joined, kind)
            if together is None:
                position = self.typed[(unknown.path, ref)].root.position
                self.record(_mismatch(operator, [joined, kind], position, ref).errors, unknown.path)
                return None
            joined = together
        return joined

    def reference(self, ref: Reference, source: Source) -> Type:
        checker, kind, target = self.checker, ref.kind, ref.target
        if kind == "amount":
            return amount(target)
        if kind == "stock":
            return amount(checker.entries[target].doc["holds"])
        if kind == "cap":
            return self.cap(target)
        if kind == "count":
            return COUNT
        if kind == "cleared":
            return BOOL
        if kind == "metric":
            return VALUE
        if kind == "read":
            return self.fact(target, ref.position, source.ref)
        if kind == "rule":
            return self.rule(target, ref, source)
        if kind == "unknown":
            entry, member = unknown_target(checker, target)
            return self.need(("member", entry.id, member) if member is not None else ("range", entry.id))
        if kind == "pref":
            return self.pref(target, ref, source)
        if kind == "entity":
            return self.column(checker.entries[target].doc["attributes"][ref.attr], ref.position, source)
        if kind == "data":
            return self.column(checker.tables[target].file.doc["columns"][ref.attr], ref.position, source)
        raise ValueError(f"no type for reference kind {kind!r}")  # param: the static check refuses it

    def cap(self, rule_id: str) -> Type:
        kind = cap_dimension(self.checker, self.checker.entries[rule_id])
        if kind is None:  # caps_and_gates reports the subject
            raise _Skip(rule_id)
        return kind

    def fact(self, key: str, position: int, ref: str) -> Type:
        found = self.facts.get(key, set())
        if len(found) > 1:
            raise _mismatch("read", sorted(kind.token for kind in found), position, ref)
        if not found:  # declared only by a cap whose subject is refused
            raise _Skip(key)
        return next(iter(found))

    def rule(self, rule_id: str, ref: Reference, source: Source) -> Type:
        entry = self.checker.entries[rule_id]
        if entry.kind == "derive":
            return self.need(("expr", entry.path, at(entry.ref, "logic", "value")))
        if entry.kind == "cap":
            return self.cap(rule_id)
        if entry.kind == "limit":
            return COUNT
        raise _missing(
            rule_id, ["rule." + kind for kind in VALUE_RULES], source.ref, found=entry.token, position=ref.position
        )

    def pref(self, key: str, ref: Reference, source: Source) -> Type:
        if key not in self.prefs:
            raise _missing(key, ["pref"], source.ref, position=ref.position)
        value = self.prefs[key]
        if type(value) is bool:
            return BOOL
        if type(value) is int:  # a preference carries no dimension: it fits like a number literal
            return NUMBER
        raise _mismatch("reference", [json_type(value)], ref.position, source.ref)

    def column(self, spec: dict[str, Any], position: int, source: Source) -> Type:
        if spec["type"] in ("integer", "decimal"):
            return dimension(spec["dimension"])
        if spec["type"] == "boolean":
            return BOOL
        raise _mismatch("reference", [spec["type"]], position, source.ref)

    # What each member expects --------------------------------------------------------------------

    def expect(self, source: Source, typed: Expression) -> None:
        owner, role, found = source.owner, source.role, typed.type
        if owner.category != "rule":
            return
        if owner.kind == "derive":
            self.derive(source, typed)
            return
        expected = [("when", BOOL)] if role == ("when",) else self.expected(owner, role)
        for operator, kind in expected:
            if not fits(kind, found):
                self.record(_mismatch(operator, [kind, found], typed.root.position, source.ref).errors, owner.path)
                return

    def expected(self, owner: Entry, role: tuple[str, ...]) -> list[tuple[str, Type]]:
        checker, kind, logic = self.checker, owner.kind, owner.doc["logic"]
        member = role[1:] if role[0] == "logic" else ()
        if kind == "cap" and member == ("value",):
            dim = cap_dimension(checker, owner)
            return [("value", dim)] if dim is not None else []
        if kind == "regen":
            resource = held(checker, owner.doc["subject"])
            amounts = [("amount", amount(resource))] if resource is not None else []
            return {("amount",): amounts, ("every",): [("every", TIME)], ("while",): [("while", BOOL)]}.get(member, [])
        if kind in ("flow", "exchange") and len(member) == 2 and member[0] in ("consumes", "produces", "gives", "takes"):
            return [(member[0], amount(member[1]))]
        if kind == "flow" and member == ("moves", "amount"):
            ends = [held(checker, logic["moves"][end]) for end in ("from", "to")]
            return [("moves", amount(resource)) for resource in ends if resource is not None]
        fixed = {
            ("flow", ("batch", "min")): ("batch", COUNT),
            ("flow", ("batch", "max")): ("batch", COUNT),
            ("exchange", ("limit",)): ("limit", COUNT),
            ("limit", ("max",)): ("max", COUNT),
            ("cooldown", ("min_interval",)): ("min_interval", TIME),
            ("gate", ("requires",)): ("requires", BOOL),
            ("objective", ("horizon",)): ("horizon", TIME),
        }
        return [fixed[kind, member]] if (kind, member) in fixed else []

    def derive(self, source: Source, typed: Expression) -> None:
        owner, role, found = source.owner, source.role, typed.type
        logic, position = owner.doc["logic"], typed.root.position

        def refuse(operator: str, operands: list[Type]) -> None:
            self.record(_mismatch(operator, operands, position, source.ref).errors, owner.path)

        if role == ("when",):
            if found != BOOL:
                refuse("when", [BOOL, found])
        elif role == ("logic", "value"):
            if "round" in logic and found in (BOOL, TIME):
                refuse("round", [found])
        elif role == ("logic", "general_logic"):
            value = self.memo.get(("expr", owner.path, at(owner.ref, "logic", "value")))
            if value is not None and unify(value, found) is None:
                refuse("general_logic", [value, found])
        elif role == ("logic", "live", "offset"):
            declared = [self.facts.get(logic["live"][name], set()) for name in ("fact", "reference_fact")]
            for kinds in declared:
                if len(kinds) > 1:  # one fact key declared with two dimensions
                    refuse("read", sorted(kinds, key=lambda kind: kind.token))
                    return
            if not all(declared):  # declared only by a cap whose subject is refused
                return
            fact, reference = (next(iter(kinds)) for kinds in declared)
            if unify(fact, reference) is None:
                refuse("live", [fact, reference])
            elif not fits(reference, found):
                refuse("offset", [reference, found])

    def single_fact(self, key: str) -> Type | None:
        found = self.facts.get(key, set())
        return next(iter(found)) if len(found) == 1 else None

    def lookup_keys(self, source: Source, typed: Expression) -> None:
        """Each key of a `lookup` fits its key column: a number of the column's dimension, or a boolean."""
        for node in nodes(typed.root):
            if not (isinstance(node, Call) and node.function == "lookup"):
                continue
            table = self.checker.tables[node.args[0]]
            for name, key in zip(table.keys, node.args[2:]):
                spec = table.file.doc["columns"].get(name, {"type": "id"})  # an undeclared key column holds ids
                if spec["type"] in ("integer", "decimal"):
                    expected: Type | str = dimension(spec["dimension"])
                    ok = fits(expected, key.type)
                elif spec["type"] == "boolean":
                    expected, ok = BOOL, key.type == BOOL
                else:
                    expected, ok = spec["type"], False
                if not ok:
                    self.record(_mismatch("lookup", [expected, key.type], node.position, source.ref).errors, source.owner.path)
                    return

    # What the evaluation needs --------------------------------------------------------------------

    def flows(self, source: Source) -> None:
        """`cost`, `yield` and `max_single_income` read at least one flow amount; an empty sum or maximum
        would be a number without a rule behind it."""
        for node in nodes(self.parsed[(source.owner.path, source.ref)].root):
            if isinstance(node, Call) and node.function in FLOW_FUNCTIONS:
                first, second = node.args[0], node.args[1]
                if not flow_parts(self.checker, node.function, first, second):
                    target = second if node.function == "max_single_income" else first
                    failure = _missing(target, ["rule.flow"], source.ref, position=node.position)
                    self.record(failure.errors, source.owner.path)

    def caps_and_gates(self) -> None:
        checker = self.checker
        for rule in checker.rules:
            if not checker.active(rule):
                continue
            if rule.kind == "cap" and cap_dimension(checker, rule) is None:
                subject = checker.entries[rule.doc["subject"]]
                failure = _missing(subject.id, ["resource", "store"], at(rule.ref, "subject"), found=subject.token)
                self.record(failure.errors, rule.path)
            elif rule.kind == "gate":
                for index, key in enumerate(rule.doc["logic"].get("prefs", ())):
                    if key not in self.prefs:
                        self.record(_missing(key, ["pref"], at(rule.ref, "logic", "prefs", index)).errors, rule.path)

    def outputs(self) -> dict[tuple[str, str], Type]:
        checker, found = self.checker, {}
        for rule in checker.rules:
            if not checker.active(rule) or rule.kind not in VALUE_RULES:
                continue
            if rule.kind == "limit":
                found["rule", rule.id] = COUNT
            elif rule.kind == "cap":
                kind = cap_dimension(checker, rule)
                if kind is not None:
                    found["rule", rule.id] = kind
            else:
                for output, member in (("rule", "value"), ("general_logic", "general_logic")):
                    kind = self.memo.get(("expr", rule.path, at(rule.ref, "logic", member)))
                    if kind is not None:
                        found[output, rule.id] = kind
        return found

    def thresholds(self) -> dict[tuple[str, str], Type]:
        """Each threshold of exports.json names an active derive (one with `live` for `fact_offset`), once."""
        checker, exports, found = self.checker, self.exports, {}
        seen: dict[str, str] = {}
        for index, threshold in enumerate(exports.doc["thresholds"]):
            ref = at("", "thresholds", index)
            if threshold["id"] in seen:
                self.errors.append(error(MODEL_ID_DUPLICATE, {
                    "path": exports.path, "namespace": "threshold", "id": threshold["id"],
                    "first_path": exports.path, "first_ref": seen[threshold["id"]],
                }, at(ref, "id")))
                continue
            seen[threshold["id"]] = at(ref, "id")
            rule = checker.need(exports.path, at(ref, "from"), threshold["from"], ("rule.derive",))
            self.errors.extend(checker.errors)
            checker.errors.clear()
            if rule is None:
                continue
            logic = rule.doc["logic"]
            if threshold["form"] == "constant":
                kind = self.memo.get(("expr", rule.path, at(rule.ref, "logic", "value")))
            elif "live" not in logic:
                failure = _missing(rule.id, ["rule.derive.live"], at(ref, "from"), found=rule.token)
                self.record(failure.errors, exports.path)
                continue
            else:
                offset = self.memo.get(("expr", rule.path, at(rule.ref, "logic", "live", "offset")))
                reference = self.single_fact(logic["live"]["reference_fact"])
                kind = unify(reference, offset) if reference is not None and offset is not None else None
            if kind == BOOL:
                self.record(_mismatch("threshold", [BOOL], 0, at(ref, "from")).errors, exports.path)
            elif kind is not None:
                found["threshold", threshold["id"]] = kind
        return found


def check(checker: Checker, exports: File, load_prefs: Callable[[], dict[str, Any]]) -> Typed:
    """The types of the model that `checker` accepted, or `Failure` with every problem.

    `load_prefs` reads the preference defaults; it is called only when the model reads a preference.
    """
    return _Types(checker, exports, load_prefs).run()
