"""The coverage of the pools, judged statically (#424 ACM-PRICE PRICE-2b-2; addendum 2.1 §3.1–§3.3 and §3.7;
contracts/formats.md, "Coverage").

From the rules and the data only: no value is computed and nothing is solved. An amount counts unless it is written
as the number 0 (`"0"`): what any other expression comes to only the run knows, so it counts.

- **Uses.** An activity (the subject of an active flow that no gate of origin `person` keeps out of the economy)
  takes from what it consumes and what its moves take from, and brings in what it produces and what its moves bring
  into.
- **Covered** (§3.1): a pool reaches a goal sink through activities when it has its own goal term (a main objective),
  a place in the costs table (a demand row of the secondary objective takes it), or a use: an activity that takes it
  and brings in a covered pool. A pool declared zero (`no_use`) never covers what leads to it: a use that leads only to
  declared zeros is no use (§3.2).
- **No steady supply** (§3.3): a pool that an activity or a demand row takes, that no regen (of an amount not written
  as 0 or below) and no activity brings in, and that has no goal term of its own.
- **Static components** (#424 ACM-PRICE design §4.6): the connected components of the flow graph. Its nodes are the
  resources, stores and activities, and the goal sinks (each main objective, and the secondary objective); its edges
  are the amounts of the flows, the activities a cap blocks or counts, a limit's shared activities, and each sink's
  pools. Each rule the economy reads is attached to the node it is about.

The checks of `acmodel check` here need every reference resolved, so the check makes them only when it found nothing
else: `no_use_contradicted` and `no_use_condition_incomplete` (§3.2), and the estimate's checks 3 and 4 (§3.7,
`estimate_basis_invalid`, `estimate_scope_exceeded`). Check 1 is the reader's (`member_conflict`, `member_missing`), and
check 2, the point value within the range, needs the values: the run makes it where the rule is computed
(`estimate_outside_range`, `core.graph`). The economy reads this judgement for its coverage section (`core.coverage`).
"""

from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction
from typing import TYPE_CHECKING, Any

from acmodel.check.walk import at, nodes
from acmodel.codes import (
    ESTIMATE_BASIS_INVALID,
    ESTIMATE_SCOPE_EXCEEDED,
    NO_USE_CONDITION_INCOMPLETE,
    NO_USE_CONTRADICTED,
)
from acmodel.envelope import Failure, error
from acmodel.expr import Literal, Reference, Unary, parse

if TYPE_CHECKING:
    from acmodel.check.checker import Checker, Entry

# The grounds of a covered pool (§3.1), in this order.
GOAL, COSTS, USE = "goal", "costs", "use"
# What a basis needs that the estimate lacks (check 3).
VARIANTS_UNKNOWN, SUPPLEMENT, SAMPLES = "variants_unknown", "supplement", "samples"


def sign(text: str) -> int | None:
    """The sign of an amount written as a number literal (`"0"`, `"10"`, `"-10"`): 1, 0 or -1; None for any other
    expression, whose value only the run knows (a `random:<id>` product too)."""
    try:
        root = parse(text, ref="").root
    except Failure:
        return None
    negated = isinstance(root, Unary) and root.op == "-"
    if negated:
        root = root.operand
    if not (isinstance(root, Literal) and type(root.value) is Fraction):
        return None
    value = -root.value if negated else root.value
    return (value > 0) - (value < 0)


def _counts(text: str) -> bool:
    """Whether an amount counts: anything not written as the number 0."""
    return sign(text) != 0


@dataclass
class Static:
    """The static judgement of one model (module docstring)."""

    resources: list[str]
    stores: list[str]
    slots: frozenset[str]  # resources with an active hard cap of the number 0 (formats.md, "Slots"): no pools
    takes: dict[str, set[str]]  # activity -> the resources and stores it takes from
    brings: dict[str, set[str]]  # activity -> the resources and stores it brings in
    goals: dict[str, list[str]]  # resource -> its main objectives
    secondary: Entry | None  # the secondary objective, a sink of its own
    costs: set[str]  # the resources a demand row takes
    zeros: dict[str, Entry]  # pool -> its `no_use` rule
    covered: dict[str, list[str]]  # resource or store -> its grounds: goal, costs, use (in that order)
    users: dict[str, list[str]]  # resource or store -> the activities whose use covers it, by id
    zero_uses: set[str]  # not covered, and some use leads to a declared zero
    unsupplied: list[str]  # pools without a steady supply (module docstring), by id
    attached: dict[str, list[Entry]]  # node -> the rules attached to it
    parent: dict[str, str]  # the static components, as a union-find over the nodes

    @property
    def pools(self) -> list[str]:
        """The pools: the resources that are not slots, by id."""
        return [resource for resource in self.resources if resource not in self.slots]

    def root(self, node: str) -> str:
        while self.parent[node] != node:
            self.parent[node] = self.parent[self.parent[node]]
            node = self.parent[node]
        return node

    def components(self) -> dict[str, str]:
        """node -> its component, named by its first member in id order."""
        groups: dict[str, list[str]] = {}
        for node in self.parent:
            groups.setdefault(self.root(node), []).append(node)
        return {node: min(members) for members in groups.values() for node in members}

    def leads(self, pool: str) -> list[str]:
        """The declared zeros that a use of `pool` brings in directly, by id."""
        found = {
            other for activity, taken in self.takes.items() if pool in taken
            for other in self.brings.get(activity, ()) if other != pool and other in self.zeros
        }
        return sorted(found)


def _paid(checker: Checker, flow: Entry) -> bool:
    """Whether a flow is behind a gate of origin person (real payment): never in the economy (#424 design 4.7)."""
    gates = (checker.entries.get(gate) for gate in flow.doc["logic"].get("requires", ()))
    return any(gate is not None and gate.kind == "gate" and gate.doc["logic"]["origin"] == "person" for gate in gates)


def _cell(value: Any) -> bool:
    """Whether a cell of a costs table is not 0 (the reader made it an integer or a decimal string)."""
    try:
        return Fraction(str(value)) != 0
    except (ValueError, ZeroDivisionError):
        return True


def judge(checker: Checker) -> Static:
    """The static judgement of the model `checker` accepted (module docstring)."""
    active = sorted((rule for rule in checker.rules if checker.active(rule)), key=lambda rule: rule.id)
    entities = [entity for entity in checker.entities if checker.entries.get(entity.id) is entity]
    resources = sorted(entity.id for entity in entities if entity.kind == "resource")
    stores = sorted(entity.id for entity in entities if entity.kind == "store")
    activities = sorted(entity.id for entity in entities if entity.kind == "activity")

    def of(kind: str) -> list[Entry]:
        return [rule for rule in active if rule.kind == kind]

    slots = frozenset(
        cap.doc["subject"] for cap in of("cap")
        if cap.doc["logic"]["cap_kind"] == "hard" and cap.doc["subject"] in resources
        and "value" in cap.doc["logic"] and sign(cap.doc["logic"]["value"]) == 0
    )
    takes: dict[str, set[str]] = {}
    brings: dict[str, set[str]] = {}
    attached: dict[str, list[Entry]] = {}
    for flow in of("flow"):
        activity, logic = flow.doc["subject"], flow.doc["logic"]
        attached.setdefault(activity, []).append(flow)
        for gate in logic.get("requires", ()):
            attached[activity].append(checker.entries[gate])
        if _paid(checker, flow):
            continue
        for resource, text in logic.get("consumes", {}).items():
            if _counts(text):
                takes.setdefault(activity, set()).add(resource)
        for resource, text in logic.get("produces", {}).items():
            if _counts(text):
                brings.setdefault(activity, set()).add(resource)
        if "moves" in logic and _counts(logic["moves"]["amount"]):
            takes.setdefault(activity, set()).add(logic["moves"]["from"])
            brings.setdefault(activity, set()).add(logic["moves"]["to"])
    goals: dict[str, list[str]] = {}
    secondaries: list[Entry] = []
    for rule in of("objective"):
        if rule.doc["logic"]["tier"] == "main":
            goals.setdefault(rule.doc["subject"], []).append(rule.id)
        else:
            secondaries.append(rule)
        attached.setdefault(rule.id, []).append(rule)
    secondary = secondaries[0] if secondaries else None
    costs: set[str] = set()
    if secondary is not None:
        demand, table = (checker.tables.get(secondary.doc["logic"][member]) for member in ("demand", "costs"))
        if demand is not None and table is not None and demand.rows is not None and table.rows is not None:
            columns = [name for name in table.file.doc["columns"] if name not in table.keys]
            for key in demand.rows:
                if key in table.rows:
                    row = table.file.doc["rows"][table.rows[key]]
                    costs.update(name for name in columns if _cell(row[name]))
    zeros = {rule.doc["subject"]: rule for rule in of("no_use")}
    # Covered: the least set closed under the grounds; a declared zero covers nothing that leads to it.
    covered: dict[str, list[str]] = {}
    for entity in (*resources, *stores):
        grounds = ([GOAL] if entity in goals else []) + ([COSTS] if entity in costs else [])
        if grounds:
            covered[entity] = grounds
    users: dict[str, set[str]] = {}
    changed = True
    while changed:
        changed = False
        for activity in sorted(takes):
            ahead = {other for other in brings.get(activity, ()) if other in covered and other not in zeros}
            for entity in sorted(takes[activity]):
                if not ahead - {entity}:
                    continue
                users.setdefault(entity, set()).add(activity)
                if USE not in covered.get(entity, []):
                    covered[entity] = [*covered.get(entity, []), USE]
                    changed = True
    zero_uses = {
        entity for activity, taken in takes.items() for entity in taken
        if entity not in covered and any(other != entity and other in zeros for other in brings.get(activity, ()))
    }
    supplied = {target for found in brings.values() for target in found}
    for rule in of("regen"):
        attached.setdefault(rule.doc["subject"], []).append(rule)
        if sign(rule.doc["logic"]["amount"]) not in (0, -1):
            supplied.add(rule.doc["subject"])
    consumed = {entity for taken in takes.values() for entity in taken} | costs
    unsupplied = [
        resource for resource in resources
        if resource not in slots and resource in consumed and resource not in supplied and resource not in goals
    ]
    # The static components.
    parent = {node: node for node in (*resources, *stores, *activities, *(rule.id for rule in of("objective")))}

    def join(first: str, second: str) -> None:
        if first in parent and second in parent:
            static.parent[static.root(first)] = static.root(second)

    static = Static(
        resources=resources, stores=stores, slots=slots, takes=takes, brings=brings, goals=goals, secondary=secondary,
        costs=costs, zeros=zeros, covered=covered, users={entity: sorted(found) for entity, found in users.items()},
        zero_uses=zero_uses, unsupplied=unsupplied, attached=attached, parent=parent,
    )
    for activity in sorted(set(takes) | set(brings)):
        for entity in sorted(takes.get(activity, set()) | brings.get(activity, set())):
            join(activity, entity)
    for rule in of("cap"):
        logic, subject = rule.doc["logic"], rule.doc["subject"]
        attached.setdefault(subject, []).append(rule)
        listed = logic["on_reach"]["block_income_from"] if isinstance(logic["on_reach"], dict) else []
        for activity in (*listed, *logic.get("from", ())):
            join(subject, activity)
    for rule in of("limit"):
        attached.setdefault(rule.doc["subject"], []).append(rule)
        for activity in rule.doc["logic"].get("shared_with", ()):
            join(rule.doc["subject"], activity)
    for resource, objectives in goals.items():
        for objective in objectives:
            join(objective, resource)
    if secondary is not None:
        for resource in sorted(costs):
            join(secondary.id, resource)
    return static


def problems(checker: Checker) -> list[dict[str, Any]]:
    """The static check's coverage errors (module docstring), in the order of the rules' ids."""
    static = judge(checker)
    found: list[dict[str, Any]] = []
    for pool, rule in sorted(static.zeros.items()):
        logic = rule.doc["logic"]
        grounds = static.covered.get(pool)
        if grounds:  # a use toward another declared zero is no use, so it never contradicts
            detail: dict[str, Any] = {"path": rule.path, "id": rule.id, "pool": pool, "grounds": list(grounds)}
            if USE in grounds:
                detail["activities"] = static.users[pool]
            found.append(error(NO_USE_CONTRADICTED, detail, rule.ref))
        own = set(logic.get("facts", ()))
        for downstream in static.leads(pool):  # the conditions of the zeros its uses lead to are its own too
            missing = sorted(set(static.zeros[downstream].doc["logic"].get("facts", ())) - own)
            if missing:
                detail = {"path": rule.path, "id": rule.id, "pool": pool, "downstream": downstream, "missing": missing}
                where = at(rule.ref, "logic", "facts") if "facts" in logic else at(rule.ref, "logic")
                found.append(error(NO_USE_CONDITION_INCOMPLETE, detail, where))
    found.extend(_estimates(checker))
    return found


def _unknowns(checker: Checker, text: str) -> list[tuple[Entry, str | None]]:
    """The unknowns an expression reads: each `unknown:` reference's unknown and member (None for an interval or a
    distribution), the longest unknown id that the target begins with, as the static check resolves it."""
    found = []
    for item in nodes(parse(text, ref="").root):
        if not (isinstance(item, Reference) and item.kind == "unknown"):
            continue
        target = item.target
        for cut in [len(target), *(index for index in range(len(target) - 1, -1, -1) if target[index] == ".")]:
            entry = checker.entries.get(target[:cut])
            if entry is not None and entry.category == "unknown":
                found.append((entry, target[cut + 1:] if cut < len(target) else None))
                break
    return found


def _estimates(checker: Checker) -> list[dict[str, Any]]:
    """The estimate's checks 3 and 4 (§3.7) for each active provisional derive with an `estimate`, by id."""
    readers: dict[str, set[str]] = {}  # node -> the rules whose expressions read it (`Checker.links`)
    for source, links in checker.links.items():
        for target, _, _ in links:
            readers.setdefault(target, set()).add(source)
    found: list[dict[str, Any]] = []
    rules = sorted((r for r in checker.rules if checker.active(r) and r.kind == "derive"), key=lambda r: r.id)
    for rule in rules:
        estimate = rule.doc["logic"].get("estimate")
        if estimate is None:
            continue
        base = at(rule.ref, "logic", "estimate")
        read = _unknowns(checker, estimate["of"]) if "of" in estimate else []
        missing = None
        if estimate["basis"] == "variants" and not any(entry.kind == "variants" and member for entry, member in read):
            missing = VARIANTS_UNKNOWN  # a choice at run time is the variants of an unknown
        elif estimate["basis"] == "samples":
            supplements = [
                item for item in rule.doc["provenance"] if item["kind"] == "measurement" and item.get("role") == "supplement"
            ]
            rates = [entry for entry, _ in read if entry.doc.get("distribution", {}).get("family") == "beta_from_samples"]
            if not supplements:
                missing = SUPPLEMENT
            elif rates and not any("samples" in item for item in supplements):
                missing = SAMPLES  # a rate from samples names its samples and successes
        if missing is not None:
            detail = {"path": rule.path, "id": rule.id, "basis": estimate["basis"], "missing": missing}
            found.append(error(ESTIMATE_BASIS_INVALID, detail, at(base, "basis")))
        # Check 4: every activity that reads the rule, directly or through other derives, is covered.
        reading: dict[str, set[str]] = {}  # activity -> the rules of it that read the estimated rule
        todo, seen = [f"rule:{rule.id}"], set()
        while todo:
            node = todo.pop()
            for source in sorted(readers.get(node, ())):
                entry = checker.entries.get(source[len("rule:"):]) if source.startswith("rule:") else None
                if entry is None or source in seen:
                    continue
                seen.add(source)
                if entry.kind == "derive":
                    todo.append(source)
                    continue
                subject = checker.entries.get(entry.doc["subject"])
                if subject is not None and subject.kind == "activity":
                    reading.setdefault(subject.id, set()).add(entry.id)
        outside = sorted(set(reading) - set(estimate["covers"]["activities"]))
        if outside:
            detail = {
                "path": rule.path, "id": rule.id, "activities": outside,
                "read_by": sorted(rule_id for activity in outside for rule_id in reading[activity]),
            }
            found.append(error(ESTIMATE_SCOPE_EXCEEDED, detail, at(base, "covers", "activities")))
    return found
