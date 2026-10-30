"""The static check of a model (contracts/formats.md, "The static check").

It runs on what the readers accepted and computes nothing. Every problem is one registered error
with the file in `detail.path` and the JSON pointer in `ref`; all of them are reported together.
The provenance classes (#424 ACM-PROV, `rule_provenance_unsourced`) are warnings, and errors only
under `--strict-provenance`.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from acmodel import ids
from acmodel.check.cycles import Links, loops
from acmodel.check.external import verify
from acmodel.check.files import File, Model
from acmodel.check.walk import at, expressions, nodes
from acmodel.codes import (
    DATA_REFERENCE_MISSING,
    MODEL_ID_DUPLICATE,
    RULE_CONFLICT,
    RULE_CYCLE,
    RULE_PENDING_UNBOUNDED,
    RULE_PREF_MISPLACED,
    RULE_PROVENANCE_MISSING,
    RULE_PROVENANCE_UNSOURCED,
    RULE_REFERENCE_MISSING,
    RULE_STATUS_INVALID,
    STATE_REFERENCE_MISSING,
)
from acmodel.envelope import Failure, error
from acmodel.expr import Call, Literal, Reference, parse
from acmodel.expr.syntax import CURRENT, FLOW_FUNCTIONS
from acmodel.model import readers

RULE_STATUSES = ("confirmed", "pending", "provisional")
ROW_STATUSES = RULE_STATUSES
UNKNOWN_STATUSES = ("pending", "provisional")

# What each reference kind of an expression names, as `expected` lists it. The other kinds
# (metric, unknown, pref, read, entity, data) have their own resolution below.
NAMED = {
    "rule": ("rule",),
    "cap": ("rule.cap",),
    "count": ("rule.limit",),
    "stock": ("store",),
    "amount": ("resource",),
    "cleared": ("entity",),
    "param": ("param",),  # the format declares no parameters, so this never resolves
}
# A product of a flow may be `random:<id>` instead of an expression (#424 design 3.3).
RANDOM_PRODUCT = re.compile(f"random:({ids.ID})")
# Kinds whose rules hold one value per subject and slot (formats.md, "Conflicts").
SUBJECT_ONLY = ("regen", "flow", "exchange", "cooldown", "random", "objective")
# The provenance kinds that give a number its provenance by themselves (#424 ACM-PROV): a source, a ruling, and a
# design value (logic). A supplement that names the sources it searched does too, and a guide row counts as the
# kind underneath it.
NUMBER_SOURCES = ("source", "ruling", "logic")
NUMBER_COLUMNS = ("integer", "decimal")


def gives_number(item: dict[str, Any]) -> bool:
    """Whether a provenance entry can be the provenance of a number: never a behaviour confirmation, a pending
    check, a measurement without a role or a guide row without the class underneath it."""
    if item["kind"] in NUMBER_SOURCES or (item["kind"] == "guide" and "source" in item):
        return True
    return item.get("role") == "supplement" and bool(item.get("searched"))


def described(item: dict[str, Any]) -> str:
    """A provenance entry as `rule_provenance_unsourced` lists it (`found`): its kind, then its role, or `source`
    for a guide row on a source."""
    if "role" in item:
        return f"{item['kind']}:{item['role']}"
    return "guide:source" if item["kind"] == "guide" and "source" in item else item["kind"]


def _slot(kind: str, logic: dict[str, Any]) -> str | None:
    """The slot of a rule; None for the kinds whose rules never conflict (gate, derive, engine, weight_ref)."""
    if kind == "cap":
        return logic["cap_kind"]
    if kind == "limit":
        return logic["per"]
    return "" if kind in SUBJECT_ONLY else None


@dataclass(frozen=True)
class Entry:
    """A declaration in the model's one id namespace: an entity (periods too), a rule or an unknown."""

    id: str
    category: str  # entity, rule or unknown
    kind: str
    doc: dict[str, Any]
    path: str
    ref: str

    @property
    def token(self) -> str:
        """What the entry is, as `expected` and `found` name it."""
        if self.category == "rule":
            return "rule." + self.kind
        return "unknown" if self.category == "unknown" else self.kind


@dataclass(frozen=True)
class Table:
    file: File
    keys: tuple[str, ...]
    rows: dict[tuple[str, ...], int] | None  # key -> row index; None for an external dataset


def _matches(entry: Entry, expected: tuple[str, ...]) -> bool:
    return any(
        token == entry.token
        or (token == "entity" and entry.category == "entity")
        or (token == "rule" and entry.category == "rule")
        for token in expected
    )


def _cell(value: Any) -> str:
    """A key cell as the key of a data reference spells it."""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _cuts(text: str) -> list[int]:
    """The offsets of the dots in `text`, from the right: where an id may end and a member begin."""
    return [index for index in range(len(text) - 1, -1, -1) if text[index] == "."]


class Checker:
    def __init__(self, model: Model, datasets: Path, *, strict_provenance: bool = False) -> None:
        self.model = model
        self.datasets = datasets
        self.strict_provenance = strict_provenance
        self.errors: list[dict[str, Any]] = []
        self.warnings: list[dict[str, Any]] = []  # the provenance classes, unless strict_provenance
        self.supplements: list[dict[str, Any]] = []  # every supplement entry the tool reads (--report-supplements)
        self.numbered: set[tuple[str, str]] = set()  # (path, ref) of the entries whose expressions hold a number
        self.entries: dict[str, Entry] = {}  # the first declaration of each id
        self.entities: list[Entry] = []
        self.rules: list[Entry] = []
        self.unknowns: list[Entry] = []
        self.tables: dict[str, Table] = {}
        self.sources: set[str] = set()
        self.engines: set[str] = set()
        self.superseded: dict[str, str] = {}  # rule id -> the first rule that supersedes it
        self.facts: set[str] = set()
        self.flows_of: dict[str, list[str]] = {}  # activity -> its active flow rules
        self.flows_into: dict[str, list[str]] = {}  # resource -> active flows that produce it or move into it
        self.nodes: list[str] = []
        self.links: Links = {}

    def run(self) -> dict[str, Any]:
        self.declare()
        self.supersede()
        self.derive()
        manifest = self.model.manifest
        self.need(manifest.path, "/numeraire", manifest.doc["numeraire"], ("resource",))
        for entity in self.entities:
            self.entity(entity)
        for rule in self.rules:
            self.rule(rule)
        for unknown in self.unknowns:
            self.unknown(unknown)
        for file in self.model.data:
            self.table(file)
        self.conflicts()
        self.cycles()
        if self.errors:
            raise Failure(*self.errors)
        return {
            "path": manifest.path,
            "model_id": manifest.doc["model_id"],
            "entities": sum(1 for entry in self.entries.values() if entry.category == "entity"),
            "rules": sum(1 for rule in self.rules if self.active(rule)),
            "superseded": len(self.superseded),
            "unknowns": sum(1 for entry in self.entries.values() if entry.category == "unknown"),
            "tables": len(self.tables),
        }

    def problem(self, code: str, path: str, ref: str, **values: Any) -> None:
        self.errors.append(error(code, {"path": path, **values}, ref))

    def unsourced(self, path: str, ref: str, reason: str, entry: str, entry_id: str, **values: Any) -> None:
        """A provenance class problem (#424 ACM-PROV): a warning, or an error under `--strict-provenance`."""
        detail = {"path": path, "reason": reason, "entry": entry, "id": entry_id, **values}
        (self.errors if self.strict_provenance else self.warnings).append(error(RULE_PROVENANCE_UNSOURCED, detail, ref))

    def active(self, entry: Entry) -> bool:
        """The first declaration of its id, and not superseded: what the tool reads."""
        return self.entries.get(entry.id) is entry and entry.id not in self.superseded

    def storage_cap(self, store: str) -> Entry | None:
        """The active `storage` cap of a store (two are a conflict), which bounds what one move takes out."""
        for rule in self.rules:
            if (
                self.active(rule) and rule.kind == "cap" and rule.doc["subject"] == store
                and rule.doc["logic"]["cap_kind"] == "storage"
            ):
                return rule
        return None

    # Declarations ----------------------------------------------------------------------------

    def add(self, entry: Entry) -> None:
        first = self.entries.get(entry.id)
        if first is None:
            self.entries[entry.id] = entry
            return
        self.problem(
            MODEL_ID_DUPLICATE, entry.path, at(entry.ref, "id"),
            namespace="model", id=entry.id, first_path=first.path, first_ref=at(first.ref, "id"),
        )

    def first(self, seen: dict[str, tuple[str, str]], namespace: str, key: str, path: str, ref: str) -> bool:
        if key not in seen:
            seen[key] = (path, ref)
            return True
        first_path, first_ref = seen[key]
        self.problem(
            MODEL_ID_DUPLICATE, path, ref, namespace=namespace, id=key, first_path=first_path, first_ref=first_ref
        )
        return False

    def declare(self) -> None:
        manifest = self.model.manifest
        for index, period in enumerate(manifest.doc["periods"]):
            self.add(Entry(period["id"], "entity", "period", period, manifest.path, at("", "periods", index)))
        for file in self.model.rules:
            for member, category, target in (
                ("entities", "entity", self.entities),
                ("rules", "rule", self.rules),
                ("unknowns", "unknown", self.unknowns),
            ):
                for index, doc in enumerate(file.doc[member]):
                    entry = Entry(doc["id"], category, doc["kind"], doc, file.path, at("", member, index))
                    target.append(entry)
                    self.add(entry)
        seen: dict[str, tuple[str, str]] = {}
        sources = self.model.sources
        for index, source in enumerate(sources.doc["sources"]):
            if self.first(seen, "source", source["id"], sources.path, at("", "sources", index, "id")):
                self.sources.add(source["id"])
            if "class" not in source:
                self.unsourced(sources.path, at("", "sources", index), "class_missing", "source", source["id"])
        seen = {}
        for index, engine in enumerate(manifest.doc["engines"]):
            if self.first(seen, "engine", engine["engine_id"], manifest.path, at("", "engines", index, "engine_id")):
                self.engines.add(engine["engine_id"])
        seen = {}
        for file in self.model.data:
            doc = file.doc
            if not self.first(seen, "table", doc["table"], file.path, "/table"):
                continue
            keys = (doc["key"],) if isinstance(doc["key"], str) else tuple(doc["key"])
            rows = None
            if "rows" in doc:
                rows, row_seen = {}, {}
                for index, row in enumerate(doc["rows"]):
                    key = tuple(_cell(row[name]) for name in keys)
                    if self.first(row_seen, "row", ",".join(key), file.path, at("", "rows", index)):
                        rows[key] = index
            self.tables[doc["table"]] = Table(file, keys, rows)

    def supersede(self) -> None:
        for rule in self.rules:
            for index, target in enumerate(rule.doc.get("supersedes", ())):
                if self.need(rule.path, at(rule.ref, "supersedes", index), target, ("rule",), active=False):
                    self.superseded.setdefault(target, rule.id)

    def derive(self) -> None:
        """The declared fact keys, the flows of each activity and resource, and the graph's nodes."""
        for entity in self.entities:
            if "reading" in entity.doc:
                self.facts.add(entity.doc["reading"])
        for rule in self.rules:
            if not self.active(rule):
                continue
            logic = rule.doc["logic"]
            self.nodes.append("rule:" + rule.id)
            if rule.kind == "cap" and "reading" in logic:
                self.facts.add(logic["reading"])
            if rule.kind == "flow":
                self.flows_of.setdefault(rule.doc["subject"], []).append(rule.id)
                into = set(logic.get("produces", {}))
                if "moves" in logic:
                    into.add(logic["moves"]["to"])
                for resource in sorted(into):
                    self.flows_into.setdefault(resource, []).append(rule.id)
        for unknown in self.unknowns:
            if self.entries.get(unknown.id) is not unknown:
                continue
            if unknown.kind == "variants":
                members = sorted({member for variant in unknown.doc.get("variants", {}).values() for member in variant})
                self.nodes.extend(f"unknown:{unknown.id}.{member}" for member in members)
            else:
                self.nodes.append("unknown:" + unknown.id)

    # References ------------------------------------------------------------------------------

    def need(
        self,
        path: str,
        ref: str,
        target: str,
        expected: tuple[str, ...],
        *,
        position: int | None = None,
        active: bool = True,
    ) -> Entry | None:
        """The entry `target` names if it is of an expected kind (and, if `active`, not superseded)."""
        entry = self.entries.get(target)
        values: dict[str, Any] = {"target": target, "expected": list(expected)}
        if entry is not None and _matches(entry, expected):
            if not active or entry.id not in self.superseded:
                return entry
            values["found"] = entry.token
            values["superseded_by"] = self.superseded[entry.id]
        elif entry is not None:
            values["found"] = entry.token
        if position is not None:
            values["position"] = position
        self.problem(RULE_REFERENCE_MISSING, path, ref, **values)
        return None

    def fact(self, path: str, ref: str, key: str, position: int | None = None) -> None:
        if key not in self.facts:
            values: dict[str, Any] = {"missing": "reading", "target": key}
            if position is not None:
                values["position"] = position
            self.problem(STATE_REFERENCE_MISSING, path, ref, **values)

    def link(self, node: str | None, target: str, path: str, ref: str) -> None:
        if node is not None:
            self.links.setdefault(node, []).append((target, path, ref))

    def status(self, path: str, ref: str, entry_id: str, value: str, allowed: tuple[str, ...]) -> None:
        if value not in allowed:
            self.problem(RULE_STATUS_INVALID, path, ref, id=entry_id, literal=value, expected=list(allowed))

    def provenance(
        self, path: str, ref: str, entry: str, entry_id: str, items: list[dict[str, Any]], *, read: bool = True
    ) -> None:
        """A provenance list at `ref`: not empty, and every source it cites or searched is in sources.json. Of
        what the tool reads (`read`; a superseded rule is not), a measurement names its role and a guide row the
        class underneath it, and each supplement is listed for `--report-supplements` (#424 ACM-PROV)."""
        if not items:
            self.problem(RULE_PROVENANCE_MISSING, path, ref, entry=entry, id=entry_id)
        for index, item in enumerate(items):
            item_ref = at(ref, index)
            if "source" in item and item["source"] not in self.sources:  # a source, or the source of a guide row
                self.problem(
                    RULE_REFERENCE_MISSING, path, at(item_ref, "source"), target=item["source"], expected=["source"]
                )
            for position, searched in enumerate(item.get("searched", ())):
                if searched not in self.sources:
                    self.problem(
                        RULE_REFERENCE_MISSING, path, at(item_ref, "searched", position), target=searched,
                        expected=["source"],
                    )
            if not read:
                continue
            if item["kind"] == "measurement" and "role" not in item:
                self.unsourced(path, item_ref, "role_missing", entry, entry_id)
            elif item["kind"] == "guide" and "source" not in item and "role" not in item:
                self.unsourced(path, item_ref, "guide_class_missing", entry, entry_id)
            if item.get("role") == "supplement":
                self.supplements.append(
                    {"path": path, "ref": item_ref, "entry": entry, "id": entry_id, "provenance": item}
                )

    def numbers(
        self, path: str, ref: str, entry: str, entry_id: str, status: str, items: list[dict[str, Any]], held: bool
    ) -> None:
        """Every number of a confirmed rule or data row (`held`: it holds one) has at least one provenance entry
        that can be a number's (#424 ACM-PROV). An empty list is `rule_provenance_missing` already."""
        if status == "confirmed" and held and items and not any(gives_number(item) for item in items):
            self.unsourced(path, ref, "number_unsourced", entry, entry_id, found=[described(item) for item in items])

    def dimension(self, path: str, ref: str, column: dict[str, Any]) -> None:
        dimension = column.get("dimension", "")
        if dimension.startswith("amount:"):
            self.need(path, ref, dimension[len("amount:"):], ("resource",))

    # Entries ---------------------------------------------------------------------------------

    def entity(self, entity: Entry) -> None:
        if entity.kind == "store":
            self.need(entity.path, at(entity.ref, "holds"), entity.doc["holds"], ("resource",))
        elif entity.kind == "entity_class":
            for name, column in entity.doc["attributes"].items():
                self.dimension(entity.path, at(entity.ref, "attributes", name, "dimension"), column)

    def rule(self, rule: Entry) -> None:
        doc, path = rule.doc, rule.path
        self.status(path, at(rule.ref, "status"), rule.id, doc["status"], RULE_STATUSES)
        superseded = rule.id in self.superseded
        provenance = at(rule.ref, "provenance")
        self.provenance(path, provenance, "rule", rule.id, doc["provenance"], read=not superseded)
        if superseded:
            return  # kept for history and never read
        self.need(path, at(rule.ref, "subject"), doc["subject"], ("entity",))
        for index, unknown in enumerate(doc.get("unknowns", ())):
            self.need(path, at(rule.ref, "unknowns", index), unknown, ("unknown",))
        node = "rule:" + rule.id if self.active(rule) else None
        self.logic(rule, node)
        general = at(rule.ref, "logic", "general_logic")  # compared with the value, never read by it
        for ref, text in expressions(readers.RULE, doc, rule.ref):
            self.expression(rule, None if ref == general else node, ref, text)
        self.pending(rule)
        # Its numbers: those of its expressions, and a drop's samples.
        held = (path, rule.ref) in self.numbered or "samples" in doc["logic"].get("drop", {})
        self.numbers(path, provenance, "rule", rule.id, doc["status"], doc["provenance"], held)

    def logic(self, rule: Entry, node: str | None) -> None:
        """The ids in a rule's logic that are not inside expressions."""
        logic, base, path, kind = rule.doc["logic"], at(rule.ref, "logic"), rule.path, rule.kind
        named: list[tuple[str, str, tuple[str, ...]]] = []
        if kind == "cap" and isinstance(logic["on_reach"], dict):
            for index, activity in enumerate(logic["on_reach"]["block_income_from"]):
                named.append((at(base, "on_reach", "block_income_from", index), activity, ("activity",)))
        elif kind == "flow":
            for member in ("consumes", "produces"):
                named.extend((at(base, member, name), name, ("resource",)) for name in logic.get(member, {}))
            if "moves" in logic:
                for end in ("from", "to"):
                    named.append((at(base, "moves", end), logic["moves"][end], ("resource", "store")))
            for index, gate in enumerate(logic.get("requires", ())):
                ref = at(base, "requires", index)
                entry = self.need(path, ref, gate, ("rule.gate",))
                if entry is not None:
                    self.link(node, "rule:" + entry.id, path, ref)
        elif kind == "exchange":
            for member in ("gives", "takes"):
                named.extend((at(base, member, name), name, ("resource",)) for name in logic[member])
        elif kind == "limit":
            named.append((at(base, "per"), logic["per"], ("period",)))
            for index, activity in enumerate(logic.get("shared_with", ())):
                named.append((at(base, "shared_with", index), activity, ("activity",)))
        elif kind == "random" and "outcomes" in logic:
            for index, outcome in enumerate(logic["outcomes"]):
                named.extend((at(base, "outcomes", index, "then", name), name, ("resource",)) for name in outcome["then"])
        elif kind == "derive" and "live" in logic:
            for member in ("fact", "reference_fact"):
                self.fact(path, at(base, "live", member), logic["live"][member])
        elif kind == "engine" and logic["engine"] not in self.engines:
            self.problem(RULE_REFERENCE_MISSING, path, at(base, "engine"), target=logic["engine"], expected=["engine"])
        elif kind == "objective":
            named.append((at(base, "numeraire"), logic["numeraire"], ("resource",)))
        elif kind == "weight_ref":
            named.append((at(base, "compares_to"), logic["compares_to"], ("rule",)))
        for ref, target, expected in named:
            self.need(path, ref, target, expected)

    def pending(self, rule: Entry) -> None:
        """A pending rule is usable only through an unknown with a range (#424 design 3.3)."""
        if rule.doc["status"] != "pending":
            return
        listed = rule.doc.get("unknowns", [])
        found = [self.entries[name] for name in listed if name in self.entries and self.entries[name].category == "unknown"]
        if any(entry.doc.get(entry.kind) for entry in found):
            return
        ref = at(rule.ref, "status")
        if not listed:
            self.problem(RULE_PENDING_UNBOUNDED, rule.path, ref, id=rule.id, reason="no_unknown")
        elif found:  # when none of them exists, the missing references say so
            self.problem(
                RULE_PENDING_UNBOUNDED, rule.path, ref,
                id=rule.id, reason="unknown_unbounded", unknowns=[entry.id for entry in found],
            )

    def unknown(self, unknown: Entry) -> None:
        doc, path = unknown.doc, unknown.path
        self.status(path, at(unknown.ref, "status"), unknown.id, doc["status"], UNKNOWN_STATUSES)
        self.provenance(path, at(unknown.ref, "provenance"), "unknown", unknown.id, doc["provenance"])
        primary = self.entries.get(unknown.id) is unknown
        if unknown.kind == "variants":
            for name, members in doc.get("variants", {}).items():
                for member, text in members.items():
                    node = f"unknown:{unknown.id}.{member}" if primary else None
                    self.expression(unknown, node, at(unknown.ref, "variants", name, member), text)
        else:
            node = "unknown:" + unknown.id if primary else None
            for ref, text in expressions(readers.UNKNOWN, doc, unknown.ref):
                self.expression(unknown, node, ref, text)

    def table(self, file: File) -> None:
        doc, path = file.doc, file.path
        for name, column in doc["columns"].items():
            self.dimension(path, at("", "columns", name, "dimension"), column)
        if "provenance" in doc:
            self.provenance(path, "/provenance", "table", doc["table"], doc["provenance"])
        keys = (doc["key"],) if isinstance(doc["key"], str) else tuple(doc["key"])
        # A row's numbers are its cells of the integer and decimal value columns; key cells name the row.
        held = any(column["type"] in NUMBER_COLUMNS for name, column in doc["columns"].items() if name not in keys)
        for index, row in enumerate(doc.get("rows", ())):
            ref = at("", "rows", index)
            row_id = ",".join(_cell(row[name]) for name in keys)
            self.status(path, at(ref, "status"), row_id, row["status"], ROW_STATUSES)
            if "provenance" in row:
                self.provenance(path, at(ref, "provenance"), "row", row_id, row["provenance"])
                self.numbers(path, at(ref, "provenance"), "row", row_id, row["status"], row["provenance"], held)
            elif not doc.get("provenance"):
                self.problem(RULE_PROVENANCE_MISSING, path, ref, entry="row", id=row_id)
            else:  # the table's provenance
                self.numbers(path, ref, "row", row_id, row["status"], doc["provenance"], held)
        if "external" in doc:
            external = doc["external"]
            if external["source"] not in self.sources:
                self.problem(RULE_REFERENCE_MISSING, path, "/external/source", target=external["source"], expected=["source"])
            self.errors.extend(verify(path, external, self.datasets))

    # Expressions -----------------------------------------------------------------------------

    def expression(self, owner: Entry, node: str | None, ref: str, text: str) -> None:
        path = owner.path
        if owner.token == "rule.flow" and ref.startswith(at(owner.ref, "logic", "produces") + "/"):
            match = RANDOM_PRODUCT.fullmatch(text)
            if match is not None:
                entry = self.need(path, ref, match.group(1), ("rule.random",))
                if entry is not None:
                    self.link(node, "rule:" + entry.id, path, ref)
                return
        try:
            parsed = parse(text, ref=ref)
        except Failure as failure:
            for entry in failure.errors:
                self.errors.append({**entry, "detail": {**entry["detail"], "path": path}})
            return
        for item in nodes(parsed.root):
            if isinstance(item, Literal) and not isinstance(item.value, bool):  # a number or a duration
                self.numbered.add((path, owner.ref))
            if isinstance(item, Reference):
                self.reference(owner, node, ref, item)
            elif isinstance(item, Call) and item.function in FLOW_FUNCTIONS:
                self.flow_function(owner, node, ref, item)
            elif isinstance(item, Call) and item.function == "lookup":
                self.lookup(owner, ref, item)

    def reference(self, owner: Entry, node: str | None, ref: str, item: Reference) -> None:
        path, kind, target, position = owner.path, item.kind, item.target, item.position
        if kind in NAMED:
            entry = self.need(path, ref, target, NAMED[kind], position=position)
            if entry is not None and entry.category == "rule":
                self.link(node, "rule:" + entry.id, path, ref)
        elif kind == "metric":
            self.metric(owner, node, ref, item)
        elif kind == "unknown":
            self.unknown_reference(owner, node, ref, item)
        elif kind == "pref":
            self.pref(owner, ref, item)
        elif kind == "read":
            self.fact(path, ref, target, position)
        elif kind == "entity":
            entry = self.entries.get(target)
            if entry is None or entry.token != "entity_class":
                self.problem(STATE_REFERENCE_MISSING, path, ref, missing="entity_class", target=target, position=position)
            elif item.attr is not None and item.attr not in entry.doc["attributes"]:
                self.problem(
                    STATE_REFERENCE_MISSING, path, ref,
                    missing="attribute", target=item.attr, entity_class=target, position=position,
                )
        elif kind == "data":
            self.data(owner, ref, item)
        else:  # a reference kind the expression language added without a check here: a defect
            raise ValueError(f"the static check has no rule for reference kind {kind!r}")

    def metric(self, owner: Entry, node: str | None, ref: str, item: Reference) -> None:
        """`metric:<engine rule>.<name>`: the longest engine rule id that leaves a declared metric."""
        target = item.target
        for cut in _cuts(target):
            entry = self.entries.get(target[:cut])
            if (
                entry is not None
                and entry.token == "rule.engine"
                and entry.id not in self.superseded
                and target[cut + 1:] in entry.doc["logic"]["metrics"]
            ):
                self.link(node, "rule:" + entry.id, owner.path, ref)
                return
        self.problem(RULE_REFERENCE_MISSING, owner.path, ref, target=target, expected=["metric"], position=item.position)

    def unknown_reference(self, owner: Entry, node: str | None, ref: str, item: Reference) -> None:
        """`unknown:<id>` (interval, distribution) or `unknown:<id>.<member>` (a member of every variant)."""
        path, target, position = owner.path, item.target, item.position
        for cut in [len(target), *_cuts(target)]:
            entry = self.entries.get(target[:cut])
            if entry is None or entry.category != "unknown":
                continue
            form = entry.doc.get(entry.kind)
            if not form:
                self.problem(
                    RULE_PENDING_UNBOUNDED, path, ref,
                    id=owner.id, reason="unknown_unbounded", unknowns=[entry.id], position=position,
                )
                return
            member = target[cut + 1:]
            if cut == len(target) and entry.kind != "variants":
                self.link(node, "unknown:" + entry.id, path, ref)
                return
            if cut < len(target) and entry.kind == "variants" and all(member in variant for variant in form.values()):
                self.link(node, f"unknown:{entry.id}.{member}", path, ref)
                return
            self.problem(RULE_REFERENCE_MISSING, path, ref, target=target, expected=["unknown_member"], position=position)
            return
        self.problem(RULE_REFERENCE_MISSING, path, ref, target=target, expected=["unknown"], position=position)

    def pref(self, owner: Entry, ref: str, item: Reference) -> None:
        """`pref:` only inside a gate or an objective's terms (#424 design D-2)."""
        if owner.token == "rule.gate":
            return
        if owner.token == "rule.objective" and ref.startswith(at(owner.ref, "logic", "terms") + "/"):
            return
        self.problem(
            RULE_PREF_MISPLACED, owner.path, ref,
            id=owner.id, kind=owner.kind if owner.category == "rule" else "unknown",
            reference=item.text, position=item.position,
        )

    def data(self, owner: Entry, ref: str, item: Reference) -> None:
        path, position = owner.path, item.position
        table = self.tables.get(item.target)
        if table is None:
            self.problem(DATA_REFERENCE_MISSING, path, ref, missing="table", table=item.target, position=position)
        elif item.key is None:
            return  # the whole table, as the set of a sum or count
        elif item.attr not in table.file.doc["columns"]:
            self.problem(
                DATA_REFERENCE_MISSING, path, ref, missing="column", table=item.target, column=item.attr, position=position
            )
        elif item.key != (CURRENT,) and (
            len(item.key) != len(table.keys) or (table.rows is not None and item.key not in table.rows)
        ):
            self.problem(
                DATA_REFERENCE_MISSING, path, ref, missing="row", table=item.target, key=list(item.key), position=position
            )

    def lookup(self, owner: Entry, ref: str, item: Call) -> None:
        path, position = owner.path, item.position
        name, column, keys = item.args[0], item.args[1], item.args[2:]
        table = self.tables.get(name)
        if table is None:
            self.problem(DATA_REFERENCE_MISSING, path, ref, missing="table", table=name, position=position)
        elif column not in table.file.doc["columns"]:
            self.problem(DATA_REFERENCE_MISSING, path, ref, missing="column", table=name, column=column, position=position)
        elif len(keys) != len(table.keys):
            self.problem(DATA_REFERENCE_MISSING, path, ref, missing="row", table=name, position=position)

    def flow_function(self, owner: Entry, node: str | None, ref: str, item: Call) -> None:
        """`cost(a, r)` and `yield(a, r)` read the flows of a; `max_single_income(r, cap)` the cap, the flows
        into r, and the storage cap of each store such a flow moves out of."""
        path, position = owner.path, item.position
        first, second = item.args[0], item.args[1]
        if item.function == "max_single_income":
            resource = self.need(path, ref, first, ("resource",), position=position)
            cap = self.need(path, ref, second, ("rule.cap",), position=position)
            flows = self.flows_into.get(first, []) if resource else []
            targets = ([cap.id] if cap is not None else []) + flows
            for flow in flows:
                moves = self.entries[flow].doc["logic"].get("moves", {})
                source = self.entries.get(moves["from"]) if moves.get("to") == first else None
                bound = self.storage_cap(source.id) if source is not None and source.kind == "store" else None
                if bound is not None and bound.id not in targets:
                    targets.append(bound.id)
        else:
            activity = self.need(path, ref, first, ("activity",), position=position)
            self.need(path, ref, second, ("resource",), position=position)
            targets = self.flows_of.get(first, []) if activity is not None else []
        for target in targets:
            self.link(node, "rule:" + target, path, ref)

    # Across rules ----------------------------------------------------------------------------

    def conflicts(self) -> None:
        """Two active rules of one kind on one subject and slot (#424 design D-3)."""
        seen: dict[tuple[str, str, str], Entry] = {}
        for rule in self.rules:
            if not self.active(rule):
                continue
            slot = _slot(rule.kind, rule.doc["logic"])
            if slot is None:
                continue
            first = seen.setdefault((rule.kind, rule.doc["subject"], slot), rule)
            if first is rule:
                continue
            values = {"id": rule.id, "conflicts_with": first.id, "kind": rule.kind, "subject": rule.doc["subject"]}
            if slot:
                values["slot"] = slot
            self.problem(RULE_CONFLICT, rule.path, rule.ref, **values)

    def cycles(self) -> None:
        superseding: Links = {}
        primary = [rule for rule in self.rules if self.entries.get(rule.id) is rule]
        for rule in primary:
            for index, target in enumerate(rule.doc.get("supersedes", ())):
                entry = self.entries.get(target)
                if entry is not None and entry.category == "rule":
                    superseding.setdefault("rule:" + rule.id, []).append(
                        ("rule:" + target, rule.path, at(rule.ref, "supersedes", index))
                    )
        for via, graph_nodes, links in (
            ("supersedes", ["rule:" + rule.id for rule in primary], superseding),
            ("reference", self.nodes, self.links),
        ):
            for members, path, ref in loops(graph_nodes, links):
                self.problem(RULE_CYCLE, path, ref, members=members, via=via)
