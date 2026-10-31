"""The coverage section of the economy's steady state, and the status of each price (#424 ACM-PRICE PRICE-2b-2;
addendum 2.1 §3.1–§3.3, §3.7; the design's §4.6; contracts/formats.md, "Coverage").

It reads the static judgement (`check.coverage`: the uses, what is covered, the declared zeros, the pools without a
steady supply and the static components) and what the steady state read (`core.flow._Reader`: the status of each item,
the pending flows it left out, and the ties of its solves). Nothing here solves a program.

**The status of a price** is that of its static component: the worst status of the items of that component the steady
state read (amounts, regen members, caps and limits, objective members, demand and costs rows, gates), each with the
status of everything its value read; provisional when one of them depends on an unknown it leaves unresolved, or when
a rule of the component, or a rule its expressions reach, names an unknown (`unknowns`), whatever its own status
(`inputs`). A component that holds a structural gap is provisional too, for each gap it holds:
- `excluded_pending`, an activity the steady state left out (a pending flow whose amounts read unknowns);
- `no_steady_supply`, a pool without a steady supply;
- `objective_tier_tie`, an activity of S3's optima and none of S2's;
- `probe_unverified`, an activity a probe of which could not be verified exactly (`lp_probe_unverified`, PRICE-2c):
  whether it is a tie, the most an optimum runs it, its range or its forcing cost. Conservative: an unverified tie is
  never read as no tie.

The pending flows the steady state left out count as `excluded_pending` only: their own items and the unknowns they
name are not `inputs`. A component that holds no gap keeps its own status, whatever another component holds.

**The class of a pool** (§3.1), one of four:
- `verified_zero`: it is declared zero (`no_use`), and the static check found the declaration sound
  (`no_use_contradicted`, `no_use_condition_incomplete`);
- `structural_gap`: its component holds a gap;
- `covered`: it reaches a goal sink (its grounds: `goal`, `costs`, `use`);
- `unknown`: anything else (`not_covered`, and `zero_uses` when its only uses lead to declared zeros).

**For PRICE-4** (not here): a covered pool whose price is 0 names the constraint that makes it 0 (an LP fact); a
verified zero is exported only with its acceptance and `scope`; a pool of `schedule.json` that the model lacks is
unknown; `estimate_missing` is each provisional derive under `estimates` whose `estimate` is null.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from acmodel.check.coverage import judge
from acmodel.codes import LP_PROBE_UNVERIFIED, OBJECTIVE_TIER_TIE
from acmodel.core.common import CONFIRMED, PROVISIONAL, worst
from acmodel.expr import Duration

if TYPE_CHECKING:
    from acmodel.check.checker import Checker, Entry
    from acmodel.core.graph import Graph

# The classes of a pool (§3.1).
COVERED, STRUCTURAL_GAP, VERIFIED_ZERO, UNKNOWN = "covered", "structural_gap", "verified_zero", "unknown"
CLASSES = (COVERED, STRUCTURAL_GAP, VERIFIED_ZERO, UNKNOWN)
# The structural gaps a component may hold (§3.3), in this order, and why it is provisional otherwise.
EXCLUDED_PENDING, NO_STEADY_SUPPLY, TIER_TIE = "excluded_pending", "no_steady_supply", "objective_tier_tie"
PROBE_UNVERIFIED = "probe_unverified"  # PRICE-2c
GAPS = (EXCLUDED_PENDING, NO_STEADY_SUPPLY, TIER_TIE, PROBE_UNVERIFIED)
INPUTS = "inputs"
# The steady state reads its nodes in the reference state (`core.flow.PREFIX`).
REFERENCE = "reference"


def _node(checker: Checker, secondary: Entry | None, key: tuple[str, ...]) -> str | None:
    """The node of the static flow graph an item of the steady state is about: a flow amount and a gate, the flow's
    activity; a regen member, a cap or a limit, its subject; an objective member, the objective's sink; a demand or
    costs row, the secondary objective's sink."""
    kind = key[0]
    if kind in ("flow", "requires", "regen", "rule"):
        return checker.entries[key[1]].doc["subject"]
    if kind == "objective":
        return key[1]
    if kind == "row":
        return None if secondary is None else secondary.id
    return None


def _reached(checker: Checker, rule: Entry) -> list[Entry]:
    """The rules that a rule's expressions reach, through every rule in between (`Checker.links`), by id."""
    found: dict[str, Entry] = {}
    todo = [f"rule:{rule.id}"]
    while todo:
        for target, _, _ in checker.links.get(todo.pop(), ()):
            entry = checker.entries.get(target[len("rule:"):]) if target.startswith("rule:") else None
            if entry is not None and entry.id not in found and entry is not rule:
                found[entry.id] = entry
                todo.append(target)
    return [found[name] for name in sorted(found)]


def _text(value: Any) -> str:
    if type(value) is bool:
        return "true" if value else "false"
    return str(value.seconds if type(value) is Duration else value)


def report(graph: Graph, reader: Any, model: Any, warnings: list[dict[str, Any]]) -> tuple[dict[str, Any], dict]:
    """The steady state's `coverage` section and its `price_status` (module docstring).

    `reader` is the steady state's `_Reader`, `model` its `_Model` and `warnings` the warnings of its solves. What it
    reads beyond the economy's own trace (the rules its expressions reach, and the nodes the estimates are found
    through) it depends on, so the cache recomputes the section when they change."""
    checker = graph.checker
    static = judge(checker)
    component = static.components()
    entries = checker.entries
    left_out = set(reader.pending)
    status: dict[str, str] = {}
    inputs: set[str] = set()
    for key, (found, unresolved) in reader.statuses.items():
        if key[0] in ("flow", "requires") and key[1] in left_out:
            continue  # a left-out flow counts as excluded_pending only
        node = _node(checker, static.secondary, key)
        if node is None or node not in component:
            continue
        name = component[node]
        status[name] = worst(status.get(name, CONFIRMED), found)
        if found != CONFIRMED or unresolved:
            inputs.add(name)
    for node, rules in sorted(static.attached.items()):
        for rule in rules:
            if rule.kind == "flow" and rule.id in left_out:
                continue
            for entry in (rule, *_reached(checker, rule)):
                graph.depend(("entry", entry.id))
                if entry.doc.get("unknowns"):  # named by a rule of the component, even a confirmed one
                    inputs.add(component[node])
    gaps: dict[str, dict[str, set[str]]] = {}
    for flow in left_out:
        activity = entries[flow].doc["subject"]
        gaps.setdefault(component[activity], {}).setdefault(EXCLUDED_PENDING, set()).add(activity)
    for pool in static.unsupplied:
        gaps.setdefault(component[pool], {}).setdefault(NO_STEADY_SUPPLY, set()).add(pool)
    for warning in warnings:
        gap = {OBJECTIVE_TIER_TIE: TIER_TIE, LP_PROBE_UNVERIFIED: PROBE_UNVERIFIED}.get(warning["code"])
        if gap is not None:
            activity = warning["detail"]["activity"]
            gaps.setdefault(component[activity], {}).setdefault(gap, set()).add(activity)

    def reasons(name: str) -> list[str]:
        return [gap for gap in GAPS if gap in gaps.get(name, {})] + ([INPUTS] if name in inputs else [])

    def judged(name: str) -> str:
        return PROVISIONAL if reasons(name) else status.get(name, CONFIRMED)

    members: dict[str, list[str]] = {}
    for node, name in component.items():
        members.setdefault(name, []).append(node)
    resources = set(static.resources)
    components = {
        name: {
            "members": sorted(found), "status": judged(name), "gap": bool(gaps.get(name)),
            "gaps": {gap: sorted(gaps[name][gap]) for gap in GAPS if gap in gaps.get(name, {})},
            "reasons": reasons(name),
        }
        for name, found in sorted(members.items()) if resources & set(found)
    }
    pools: dict[str, dict[str, Any]] = {}
    for pool in static.pools:
        name = component[pool]
        shown: dict[str, Any] = {"component": name, "status": judged(name)}
        if pool in static.zeros:
            rule = static.zeros[pool]
            shown.update(
                {"class": VERIFIED_ZERO, "reasons": ["no_use"], "no_use": rule.id},
                status=CONFIRMED if rule.doc["status"] == "confirmed" else PROVISIONAL,
            )
        elif gaps.get(name):
            shown.update({"class": STRUCTURAL_GAP, "reasons": [gap for gap in GAPS if gap in gaps[name]]})
        elif pool in static.covered:
            shown.update({"class": COVERED, "reasons": list(static.covered[pool])})
        else:
            shown.update({"class": UNKNOWN, "reasons": ["not_covered", *(["zero_uses"] if pool in static.zero_uses else [])]})
        pools[pool] = shown
    counts = {kind: sum(1 for shown in pools.values() if shown["class"] == kind) for kind in CLASSES}
    counts["with_estimates"] = sum(
        1 for shown in pools.values() if shown["class"] == COVERED and shown["status"] == PROVISIONAL
    )
    section = {
        "pools": pools, "components": components, "counts": counts,
        "estimates": _estimates(graph, reader, left_out),
    }
    prices = {
        resource: {"status": judged(component[resource]), "reasons": reasons(component[resource])}
        for resource in model.resources
    }
    return section, prices


def _estimates(graph: Graph, reader: Any, left_out: set[str]) -> list[dict[str, Any]]:
    """Each provisional derive the steady state read, through any node, by id: its value and its `estimate` (null
    when it has none), as written."""
    checker, found = graph.checker, {}
    todo = [
        (REFERENCE, *key) for key in reader.statuses
        if key[0] in ("flow", "regen", "rule", "objective") and not (key[0] == "flow" and key[1] in left_out)
    ]
    seen: set[tuple[str, ...]] = set()
    while todo:
        key = todo.pop()
        if key in seen:
            continue
        seen.add(key)
        result = graph.results.get(key)
        scenarios = getattr(result, "scenarios", None)
        if not scenarios:
            continue
        graph.depend(key)
        if key[:2] == (REFERENCE, "rule"):
            entry = checker.entries[key[2]]
            if entry.kind == "derive" and entry.doc["status"] == "provisional":
                graph.depend(("entry", entry.id))
                found[entry.id] = {
                    "rule": entry.id, "value": _text(scenarios[0].value), "estimate": entry.doc["logic"].get("estimate"),
                }
        for scenario in scenarios:
            todo.extend(item for item in scenario.reads if item not in seen)
    return [found[name] for name in sorted(found)]
