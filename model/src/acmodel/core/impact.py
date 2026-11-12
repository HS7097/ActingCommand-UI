"""The impact assessment of the estimates (#424 ACM-PRICE addendum 2.1 §3.4 and §3.7; PRICE-4a; contracts/formats.md,
"`acmodel export prices`").

A provisional derive's `estimate` says what range its point value stands for. The Runtime's zero band holds rounding
only (127 ii), so the uncertainty of the parameters is assessed here, on the model side: the estimates are moved to the
ends of their ranges, or to each of their variants, and the joint price vector is computed again, to see how far each
price moves and which activities change sign. `export prices` reads it for the acceptances of provisional prices
(`binds`, `flips`).

- **What the steady state read.** A provisional derive that the steady state read, through any node, belongs to the
  static components of the items that read it (`reached`): a flow amount's activity, a regen's, cap's or limit's subject,
  an objective's sink. The pending flows it left out count as gaps only, as in the coverage section.
- **What is assessed.** Each such derive with an `estimate` in a component that is neither a structural gap nor holds a
  provisional derive without an estimate (`estimate_missing`): every price of those components is pending whatever the
  estimates (the first version exports no part of a gap, §3.3), so moving them would change nothing that is exported.
  Each other one is listed as skipped, with why.
- **Groups** (§3.7). The estimates written with `of` that read a common unknown move together: one group, by what their
  `of` reads in the reference state (the discrete unknowns it chose, the continuous ones it computed with, a random rule
  that it reads as its enclosure among them). A pass per point of its discrete unknowns, each with each end of each
  continuous one (every combination). An estimate written with `low` and `high` is a group of its own, with two passes:
  the lowest `low` and the highest `high`. Each moved value is rounded as the derive rounds its value.
- **A pass** is the vector of the steady state (`flow.VECTOR`) with the group's derives at those values and every other
  rule at its point value; a rule that reads a moved one moves with it (`graph.Moved`). A pass whose values are all the
  point values is the steady state's own vector and is not solved again. The vector runs no tie probe (PASS-TIMING): it
  reads no tie, so its coordinates are those it had with them (`flow._Solver.vector`).
- **Timing** (PASS-TIMING). Each pass records how long it took (`timing_ms`): `source`, how its vector was found
  (`solved`, in the pass's own graph; `steady_state`, the steady state's own, not solved again; `cache`, a vector node's
  value that came from the cache, which keeps no timing); `stages`, each stage's whole milliseconds by `flow.STAGES`
  (`Economy.timing`; null unless solved); and `total`, the whole pass's (`_Assessor.vector`); both on `flow.CLOCK`.
  It is shown in that one place: the vector node's value, the provenance documents, the shared tree, `estimates`,
  `counts` and every other member hold none of it, so no `binds` moves with it.
- **What it finds**: each pool's price range over the steady state and every pass, and the activities whose class flips:
  the sign of their value at the vector (positive, zero, negative) is not the same in the steady state and in every pass.
  The Runtime's zero band, which needs the pools' scales and the rules' effects, is PRICE-4b's; an activity that moves out
  of or into a store has no value, and no class.
- **Refusals**: an estimate whose `of`, `low` or `high` is blocked or needs the account state, and a pass whose vector is
  blocked, refuse the export (`export_refused`, `impact_blocked`, the estimated derive as its output, with the errors that
  block it, each needed by it): nothing is approximated.
- **What the certificate reads** (#430 METHOD-C C-1; contracts/formats.md, "The certificate"). The assessment takes what
  the steady state's own computation found that the certificate reads (`flow.Held`, `flow.Economy.held`), in memory
  only, and keeps it (`_Assessor.held`); a pass that moves a value is still solved as above, so no vector, `timing_ms`
  or `source` changes.
- **A steady state from the cache** (item 11). The cache keeps none of it (`flow.encode`), so an assessment whose steady
  state came from the cache computes it again once, at its start, before any pass, without the cache
  (`flow.steady_again`). Its value as the cache keeps it (`Graph.encode`, canonical JSON) must be the cached one's byte
  for byte; otherwise `steady_state_mismatch` (exit 6: `expected` the sha256 of the cached value's, `found` of the one
  computed again), and nothing is assessed or written: the assessment never falls back to solving every pass. Then the
  assessment reads the one computed again; nothing it holds goes into the cache. `export prices` computes without the
  cache, so an export never takes this way.
"""

from __future__ import annotations

import itertools
from dataclasses import dataclass, field
from fractions import Fraction
from typing import TYPE_CHECKING, Any

from acmodel.cache.keys import canonical, digest
from acmodel.check.walk import at
from acmodel.codes import EXPORT_REFUSED, STEADY_STATE_MISMATCH
from acmodel.core import flow
from acmodel.core.common import solve
from acmodel.core.coverage import node_of
from acmodel.core.graph import GENERAL, REFERENCE, Moved, _SubGraph, rounded
from acmodel.core.unknown import PINS, points
from acmodel.envelope import Failure, error
from acmodel.expr import Duration, interval
from acmodel.expr.interval import is_open

if TYPE_CHECKING:
    from acmodel.check.checker import Checker, Entry
    from acmodel.check.coverage import Static
    from acmodel.core.graph import Graph, Result

# Why an estimate the steady state read is not assessed: every price of its components is pending whatever it is.
GAP, MISSING = "structural_gap", "estimate_missing"
LOW, HIGH = "low", "high"
# PASS-TIMING: how a pass's vector was found (`timing_ms.source`, module docstring): solved in the pass's own graph, every
# stage timed; the steady state's own, not solved again; or a vector node's value from the cache, with no stage times.
TIMING, SOLVED, STEADY_STATE, CACHE = "timing_ms", "solved", "steady_state", "cache"


def _derive(checker: Checker, key: tuple[str, ...]) -> str | None:
    """The provisional derive whose value the node `key` is, in any environment; None for any other node."""
    node = key[1:] if key[0] in (GENERAL, REFERENCE) else key
    if len(node) != 2 or node[0] != "rule":
        return None
    entry = checker.entries.get(node[1])
    if entry is None or entry.kind != "derive" or entry.doc["status"] != "provisional":
        return None
    return entry.id


def reached(graph: Graph, steady: flow.Economy, static: Static) -> dict[str, set[str]]:
    """Each provisional derive the steady state read, through any node, -> the static components of the items that
    read it (module docstring). The items of the pending flows it left out are not read here."""
    checker = graph.checker
    component = static.components()
    left_out = {entry["flow"] for entry in steady.data["excluded_pending"]}
    roots: dict[str, list[tuple[str, ...]]] = {}
    for key in steady.reads:
        if key[0] != REFERENCE or (key[1] == "flow" and key[2] in left_out):
            continue
        node = node_of(checker, static.secondary, key[1:])
        if node is not None and node in component:
            roots.setdefault(component[node], []).append(key)
    found: dict[str, set[str]] = {}
    for name, keys in sorted(roots.items()):
        seen: set[tuple[str, ...]] = set()
        todo = list(keys)
        while todo:
            key = todo.pop()
            if key in seen:
                continue
            seen.add(key)
            derive = _derive(checker, key)
            if derive is not None:
                found.setdefault(derive, set()).add(name)
            for scenario in getattr(graph.results.get(key), "scenarios", ()):
                todo.extend(item for item in scenario.reads if item not in seen)
    return found


@dataclass
class _Estimate:
    entry: Entry
    form: str  # `of`, or `low` for the pair `low` and `high`
    point: Any  # the derive's point value in the reference state, as the steady state reads it
    results: dict[str, Result] = field(default_factory=dict)  # each member's value in the reference state
    discrete: set[str] = field(default_factory=set)  # the unknowns `of` chose
    free: set[str] = field(default_factory=set)  # the continuous unknowns (and random rules) `of` computed with


@dataclass
class Assessment:
    """What the assessment found (module docstring): `groups` as `out/prices.json` shows them; each pool's price range
    over the steady state and every pass (`ranges`); the activities whose class flips; the components whose estimates it
    assessed; and the derives it skipped, with why."""

    groups: list[dict[str, Any]]
    ranges: dict[str, tuple[Fraction, Fraction]]
    flips: set[str]
    assessed: set[str]
    skipped: dict[str, str]


def _sign(value: Fraction | None) -> int | None:
    return None if value is None else (value > 0) - (value < 0)


def _magnitude(value: Any) -> Fraction:
    return value.seconds if type(value) is Duration else value


def _text(value: Any) -> str:
    return str(_magnitude(value))


class _Assessor:
    def __init__(self, graph: Graph, steady: flow.Economy) -> None:
        self.graph = graph
        self.checker = graph.checker
        joint = steady.data["joint"]
        self.prices = {pool: Fraction(entry["value"]) for pool, entry in joint["prices"].items()}
        self.values = {
            activity: None if entry["value"] is None else Fraction(entry["value"])
            for activity, entry in joint["activities"].items()
        }
        # #430 METHOD-C: what the certificate reads, from the steady state's own computation (module docstring, "What
        # the certificate reads"; `_again` makes sure it is there). Kept, and read by nothing yet: every pass is solved.
        self.held: flow.Held | None = steady.held

    def refuse(self, entry: Entry, label: str, blocked_by: frozenset[int], *, reason: str = "impact_blocked") -> Failure:
        """`export_refused` (`impact_blocked`) of an estimated derive, at its estimate, and the errors that block it."""
        output = "rule:" + entry.id
        detail = {"path": entry.path, "output": output, "reason": reason, "value": label}
        found = [self.graph.errors[index] for index in sorted(blocked_by)]
        return Failure(
            error(EXPORT_REFUSED, detail, at(entry.ref, "logic", "estimate")),
            *({**root, "needed_by": [output]} for root in found),
        )

    def member(self, graph: Graph, entry: Entry, member: str, label: str) -> Result:
        """The value of a member of an estimate (`of`, `low`, `high`) in the reference state of `graph`."""
        found = graph.evaluated(entry, at(entry.ref, "logic", "estimate", member), REFERENCE)
        if found.blocked:
            raise self.refuse(entry, label, found.blocked_by)
        if found.needs_state:  # an estimate that reads the account state says nothing for every account
            raise self.refuse(entry, label, frozenset(), reason="per_account")
        return found

    def estimate(self, derive: str) -> _Estimate:
        entry = self.checker.entries[derive]
        key = (REFERENCE, "rule", derive)
        solve([key], self.graph.compute, self.graph.results)
        values = {scenario.value for scenario in getattr(self.graph.results[key], "scenarios", ())}
        estimate = entry.doc["logic"]["estimate"]
        found = _Estimate(entry, "of" if "of" in estimate else LOW, values.pop() if len(values) == 1 else None)
        for member in ("of",) if found.form == "of" else (LOW, HIGH):
            found.results[member] = self.member(self.graph, entry, member, member)
        if found.form == "of":
            for scenario in found.results["of"].scenarios:
                found.discrete.update(unknown for unknown, _ in scenario.unknowns)
                found.free.update(scenario.free)
        return found

    def groups(self, estimates: list[_Estimate]) -> list[list[_Estimate]]:
        """The groups (module docstring), each by the ids of its derives; the groups in the order of their first."""
        parent = {item.entry.id: item.entry.id for item in estimates}

        def root(name: str) -> str:
            while parent[name] != name:
                name = parent[name]
            return name

        owner: dict[str, str] = {}
        for item in estimates:
            if item.form != "of":
                continue
            for unknown in sorted(item.discrete | item.free):
                if unknown in owner:
                    parent[root(item.entry.id)] = root(owner[unknown])
                else:
                    owner[unknown] = item.entry.id
        grouped: dict[str, list[_Estimate]] = {}
        for item in estimates:
            grouped.setdefault(root(item.entry.id), []).append(item)
        return sorted(grouped.values(), key=lambda group: group[0].entry.id)

    def passes(self, group: list[_Estimate]) -> list[tuple[dict[str, str], dict[str, Any]]]:
        """Each pass of a group: where its unknowns (or its estimate's ends) stand, and each derive's moved value."""
        if group[0].form != "of":
            item = group[0]
            lows = [interval.bounds(scenario.value)[0] for scenario in item.results[LOW].scenarios]
            highs = [interval.bounds(scenario.value)[1] for scenario in item.results[HIGH].scenarios]
            ends = {LOW: min(lows, key=_magnitude), HIGH: max(highs, key=_magnitude)}
            return [({item.entry.id: end}, {item.entry.id: rounded(item.entry, ends[end])}) for end in (LOW, HIGH)]
        discrete = sorted({unknown for item in group for unknown in item.discrete})
        free = sorted({unknown for item in group for unknown in item.free})
        choices = [points(self.checker.entries[unknown]) for unknown in discrete]
        found = []
        for chosen in itertools.product(*choices):
            for ends in itertools.product(sorted(PINS, reverse=True), repeat=len(free)):
                where = {**dict(zip(discrete, chosen)), **dict(zip(free, ends))}
                label = ",".join(f"{name}={point}" for name, point in sorted(where.items()))
                pinned = dict(zip(free, ends))
                graph = _SubGraph(self.graph, self.graph.subgraph_base(set(free)), pins=pinned) if free else self.graph
                values = {}
                for item in group:
                    result = self.member(graph, item.entry, "of", label)
                    fits = [
                        scenario for scenario in result.scenarios
                        if all(where.get(unknown) == point for unknown, point in scenario.unknowns)
                    ]
                    if len(fits) != 1 or is_open(fits[0].value):  # every unknown it reads stands at one point
                        raise ValueError(f"the estimate of {item.entry.id} is not one value at {label}")
                    values[item.entry.id] = rounded(item.entry, fits[0].value)
                found.append((where, values))
        return found

    def vector(
        self, group: list[_Estimate], label: str, values: dict[str, Any], base: dict
    ) -> tuple[dict, dict, dict[str, Any]]:
        """The vector of one pass: each pool's coordinate and each activity's value; and its `timing_ms` (module
        docstring), which nothing here reads."""
        started = flow.CLOCK()
        if all(item.point is not None and values[item.entry.id] == item.point for item in group):
            return self.prices, self.values, _timing(STEADY_STATE, None, started)  # the steady state's own vector
        graph = Moved(self.graph, base, values)
        solve([flow.VECTOR_NODE], graph.compute, graph.results)
        result = graph.results[flow.VECTOR_NODE]
        if result.blocked:
            raise self.refuse(group[0].entry, label, result.blocked_by)
        prices = {pool: Fraction(value) for pool, value in result.data["prices"].items()}
        values_at = {
            activity: None if value is None else Fraction(value) for activity, value in result.data["activities"].items()
        }
        stages = result.timing
        return prices, values_at, _timing(SOLVED if stages is not None else CACHE, stages, started)


def _timing(source: str, stages: dict[str, int] | None, started: int) -> dict[str, Any]:
    """A pass's `timing_ms` (module docstring): how its vector was found, its stages' times (None unless solved) and the
    whole pass's, from `started` on `flow.CLOCK`, in whole milliseconds rounded down."""
    total = (flow.CLOCK() - started) // flow.NANOSECONDS_PER_MS
    return {"source": source, "stages": None if stages is None else dict(stages), "total": total}


def _again(graph: Graph, steady: flow.Economy) -> flow.Economy:
    """The steady state the assessment reads (module docstring, "A steady state from the cache"): `steady` itself when it
    holds what the certificate reads, as one computed in this run does; otherwise (the cache gave it) the steady state
    computed again once, without the cache (`flow.steady_again`), whose value as the cache keeps it (`Graph.encode`,
    canonical JSON) must be `steady`'s byte for byte (the cache gives back the bytes it kept: decoding them and encoding
    the value again is the same text), else `steady_state_mismatch`."""
    if steady.held is not None:
        return steady
    again = flow.steady_again(graph)
    kept, found = canonical(graph.encode(steady)), canonical(graph.encode(again))
    if found != kept:
        detail = {
            "path": graph.checker.model.manifest.path, "output": flow.OUTPUTS[flow.STEADY], "expected": digest(kept),
            "found": digest(found),
        }
        raise Failure(error(STEADY_STATE_MISMATCH, detail))
    return again


def assess(
    graph: Graph, steady: flow.Economy, reach: dict[str, set[str]], gaps: set[str], missing: set[str]
) -> Assessment:
    """The impact assessment (module docstring) of the steady state `steady` of `graph`. `reach` is what `reached`
    gives, `gaps` the components that are structural gaps and `missing` those that hold a provisional derive without an
    estimate. A steady state the cache gave is computed again first, before anything else (`_again`)."""
    steady = _again(graph, steady)
    assessor = _Assessor(graph, steady)
    checker = graph.checker
    skipped: dict[str, str] = {}
    estimates: list[_Estimate] = []
    assessed: set[str] = set()
    for derive in sorted(reach):
        if "estimate" not in checker.entries[derive].doc["logic"]:
            continue
        open_components = reach[derive] - gaps - missing
        if not open_components:
            skipped[derive] = GAP if reach[derive] <= gaps else MISSING
            continue
        estimates.append(assessor.estimate(derive))
        assessed |= reach[derive]
    ranges = {pool: (price, price) for pool, price in assessor.prices.items()}
    at_base: dict[str, set[int]] = {
        activity: {_sign(value)} for activity, value in assessor.values.items() if value is not None
    }
    signs = {activity: set(found) for activity, found in at_base.items()}
    shown: list[dict[str, Any]] = []
    for group in assessor.groups(estimates):
        rules = {item.entry.id for item in group}
        base = Moved.base(graph, rules)
        group_ranges = {pool: (price, price) for pool, price in assessor.prices.items()}
        group_signs = {activity: set(found) for activity, found in at_base.items()}
        passes = []
        for where, values in assessor.passes(group):
            label = ",".join(f"{name}={point}" for name, point in sorted(where.items()))
            prices, at_values, timing = assessor.vector(group, label, values, base)
            for pool, price in prices.items():
                low, high = group_ranges[pool]
                group_ranges[pool] = (min(low, price), max(high, price))
            for activity, value in at_values.items():
                if value is not None and activity in group_signs:
                    group_signs[activity].add(_sign(value))
            passes.append({
                "at": dict(sorted(where.items())), "values": {d: _text(v) for d, v in sorted(values.items())},
                TIMING: timing,
            })
        moved = {pool: pair for pool, pair in group_ranges.items() if pair[0] != pair[1]}
        flipped = sorted(activity for activity, found in group_signs.items() if len(found) > 1)
        for pool, (low, high) in group_ranges.items():
            before = ranges[pool]
            ranges[pool] = (min(before[0], low), max(before[1], high))
        for activity, found in group_signs.items():
            signs[activity] |= found
        unknowns = sorted({unknown for item in group for unknown in item.discrete | item.free})
        shown.append({
            "rules": sorted(rules),
            "unknowns": unknowns,
            "passes": passes,
            "prices": {pool: {"low": str(low), "high": str(high)} for pool, (low, high) in sorted(moved.items())},
            "flips": flipped,
        })
    flips = {activity for activity, found in signs.items() if len(found) > 1}
    return Assessment(shown, ranges, flips, assessed, skipped)
