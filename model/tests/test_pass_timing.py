"""#424 PASS-TIMING (#430 ruling 4): a pass of the impact assessment runs no tie probe, which its vector never reads, and
each pass records how long each stage of it took (`impact[].passes[].timing_ms` in prices.json; contracts/formats.md,
"`acmodel export prices`", "A pass" and "Timing").

The fixtures are the hand-solvable neutral models of test_economy.py and test_prices.py; each docstring says by hand what
the fixture gives. The vector as it was computed before this slice (`before`) is kept here, line for line, to compare
the two paths. Times are read from `flow.CLOCK`, which some tests replace with a clock that moves a whole number of
milliseconds at each reading (`ticking`), so that what a stage was given is known. CI runs these tests on Windows and on
Ubuntu."""

from __future__ import annotations

import itertools
import json
import shutil
from pathlib import Path
from typing import Any

from acmodel.cache import EVALUATORS
from acmodel.cache.keys import canonical
from acmodel.core import flow as economy_flow
from acmodel.core.common import solve
from acmodel.core.graph import Graph, Moved
from acmodel.core.run import evaluate_model
from acmodel.export import policy
from acmodel.export import prices as price_export
from test_economy import activity, flow, goal, limit, resource, shared_fixture
from test_price_reuse import reuse, snapshot
from test_prices import (  # noqa: F401 (the fixture `acmodel`)
    ESTIMATE_B,
    STEADY_ROOT,
    TIMING,
    acmodel,
    base,
    delivered,
    export,
    flips_fixture,
    kept,
    priced,
    provisional,
    two_parts,
    untimed,
    untimed_doc,
)
from test_probe_gap import extended

STEADY = economy_flow.NODES[economy_flow.STEADY]
# The stages every solved pass ends at least once, whatever the model (`flow._Solver.vector`, `tiers`, `joint`): the
# others are S1 and S2 (only when A has a term) and the range probes (only for an activity that K may need).
ALWAYS = ("model", "optimum", "dual", "projections", "intervals", "most", "selection", "max_min", "check")


def before(solver: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """`flow._Solver.vector` as it was before PASS-TIMING, line for line: the three solves with their tie probes
    (`tiers()`), then the projections and the lean selection."""
    tiers = solver.tiers()
    on_face = economy_flow._restricted(tiers.dual, tiers.program, tiers.base.x)
    prices, _, _ = solver.projections(tiers, on_face, None)
    joint, _ = solver.joint(
        tiers.dual, tiers.point, tiers.base.value, prices, on_face, tiers.base.x, tiers.optimal, tiers.seed,
        lean=True,
    )
    data = {
        "prices": {resource: entry["value"] for resource, entry in joint["prices"].items()},
        "activities": {activity: entry["value"] for activity, entry in joint["activities"].items()},
    }
    return data, []


def watched_ties(monkeypatch: Any) -> list[list[str]]:
    """Each run of the tie probes (`flow._Solver.ties`), by the ties it found, in order."""
    found: list[list[str]] = []
    ties = economy_flow._Solver.ties

    def watched(solver: Any, *args: Any) -> Any:
        result = ties(solver, *args)
        found.append(list(result[0]))
        return result

    monkeypatch.setattr(economy_flow._Solver, "ties", watched)
    return found


def ticking(monkeypatch: Any, step_ms: int) -> None:
    """`flow.CLOCK` moved `step_ms` milliseconds at each reading: a stage that ends k times since the stage before it
    was given k readings' worth, so each stage that runs is at least `step_ms`, and two steps give two timings."""
    readings = itertools.count(1)
    monkeypatch.setattr(economy_flow, "CLOCK", lambda: next(readings) * step_ms * economy_flow.NANOSECONDS_PER_MS)


def graph_of(folder: Path) -> Graph:
    """The fixture's model evaluated for its state, without the cache: its graph."""
    model = folder / "ours" / "model"
    return evaluate_model(model, folder / "state.json", datasets=folder / "datasets", max_bits=127).graph


def solved(graph: Graph, key: tuple[str, ...]) -> Any:
    solve([key], graph.compute, graph.results)
    return graph.results[key]


def passes(doc: dict[str, Any]) -> list[dict[str, Any]]:
    return [each for group in doc["impact"] for each in group["passes"]]


def timings(doc: dict[str, Any]) -> list[dict[str, Any]]:
    return [each[TIMING] for each in passes(doc)]


def timed_at(value: Any, where: str = "") -> list[str]:
    """The JSON pointer of every member whose name holds `timing` anywhere in `value` (a pass's `timing_ms`, and since
    #430 C-2a the steady state's `steady_timing_ms`)."""
    found: list[str] = []
    if isinstance(value, dict):
        for key, inner in value.items():
            pointer = f"{where}/{key}"
            if "timing" in key:
                found.append(pointer)
            found += timed_at(inner, pointer)
    elif isinstance(value, list):
        for index, inner in enumerate(value):
            found += timed_at(inner, f"{where}/{index}")
    return found


def tie_beside_estimate(folder: Path) -> Path:
    """The tie of test_economy.py (`shared_fixture`: s comes at 100 a day; act.get turns 10 s into 10 g, the main goal
    worth 1, at most 10 a day; act.train turns 10 s into 1 m; the demand row takes 1 m for a point worth 10) and, with no
    edge to it, test_prices.py's b: act.b brings in `pt.b` b once a day, pt.b a provisional derive of 3 estimated from 2
    to 4 (`ESTIMATE_B`), b a main goal worth 1.

    By hand: act.get and act.b are protected (a limit counts each, and act.get's only input has a regen, act.b has none),
    so A* = 100 + pt.b; V0 = V* = 100 + pt.b, no conflict. Some optimum of S3 runs act.train and none of S2 does: act.train
    is `objective_tier_tie`, and its component (s, g, m) a structural gap with no estimate. b's component read pt.b, so it
    is assessed: two passes, pt.b at 2 and at 4. In each, act.train is still a tie, and the vector is the steady
    state's: s 1, g 1, m 10 (D* is that one point), b 1 (one b more or less is one unit more or less for its goal);
    act.get and act.train are worth 0, act.b pt.b > 0. So no price moves and nothing flips."""
    rules = [
        provisional("pt.b", "act.b", "3", ESTIMATE_B),
        flow("flow.b", "act.b", produces={"res.b": "rule:pt.b"}), limit("limit.b", "act.b", "1", "day"),
        goal("goal.b", "res.b", "1"),
    ]
    entities = [resource("res.b", "fact.b"), activity("act.b")]
    return priced(extended(shared_fixture(folder, "10", most="10"), entities, rules, readings={"fact.b": 0}))


B_GROUP = {
    "rules": ["pt.b"], "unknowns": [],
    "passes": [
        {"at": {"pt.b": "low"}, "values": {"pt.b": "2"}, "by": "solved"},
        {"at": {"pt.b": "high"}, "values": {"pt.b": "4"}, "by": "solved"},
    ],
    "prices": {}, "flips": [],
}


# The vector without the tie probes ---------------------------------------------------------------------------------


def test_the_vector_runs_no_tie_probe_and_each_coordinate_is_what_it_was(tmp_path, monkeypatch):
    """The tie of test_economy.py (`shared_fixture`, a point worth 10, act.get at most 10 a day). By hand there: V0 = A* =
    V* = 100; some optimum of S3 runs act.train and none of S2 does, so act.train is `objective_tier_tie`; D* is the one
    point π_s = 1, π_g = 1, π_m = 10, so act.get is worth 10 - 10 = 0 and act.train 10 - 10 = 0.

    The vector of the model now runs no tie probe; as it was computed before (`before`), its tie probes run once and
    find act.train. Both give that point, coordinate by coordinate, and the same activity values, and what the cache
    would keep of the node is the same, member by member (so this slice left `economy` at version 11: a kept vector is
    still right; it is 12 since #430's second try of an LP on D*). The steady state of the same graph still runs the
    probes and still warns of the tie, and its joint vector is the vector's. The node holds its stages' times, which the
    cache's encoding leaves out and equality does not read."""
    folder = shared_fixture(tmp_path, "10", most="10")
    ties = watched_ties(monkeypatch)
    graph = graph_of(folder)
    del ties[:]

    found = solved(graph, economy_flow.VECTOR_NODE)
    probed_by_the_vector = list(ties)
    steady = solved(graph, STEADY)
    probed_by_the_steady_state = ties[len(probed_by_the_vector):]
    with monkeypatch.context() as patched:
        patched.setattr(economy_flow._Solver, "vector", before)
        earlier = graph_of(folder)
        del ties[:]
        was = solved(earlier, economy_flow.VECTOR_NODE)
        probed_before = list(ties)

    assert (probed_by_the_vector, probed_by_the_steady_state, probed_before) == ([], [["act.train"]], [["act.train"]])
    assert not found.blocked and not was.blocked
    assert found.data == was.data == {
        "prices": {"res.g": "1", "res.m": "10", "res.s": "1"}, "activities": {"act.get": "0", "act.train": "0"},
    }
    assert {pool: entry["value"] for pool, entry in steady.data["joint"]["prices"].items()} == found.data["prices"]
    assert [entry["code"] for entry in steady.warnings] == ["objective_tier_tie"]
    kept_now, kept_before = economy_flow.encode(found, []), economy_flow.encode(was, [])
    assert kept_now == kept_before and sorted(kept_now) == sorted(economy_flow.FORMAT["economy"])
    assert found == was
    assert EVALUATORS["economy"].version == 12
    assert list(found.timing) == list(economy_flow.STAGES) and all(type(ms) is int for ms in found.timing.values())


def test_a_pass_beside_a_tie_exports_what_it_exported_with_the_tie_probes(tmp_path, acmodel, monkeypatch):
    """`tie_beside_estimate`: b's two passes, each beside act.train's tie. Exported now, the tie probes run once, for the
    steady state; exported with the vector as it was (`before`), they also run in each pass, and find act.train there
    too. The two exports write the same files: every price's provenance file byte for byte, and prices.json too once
    each pass's `timing_ms` is taken out; the group is the one worked out by hand (nothing moves, nothing flips)."""
    folder = tie_beside_estimate(tmp_path)
    out = base(folder) / "out"
    ties = watched_ties(monkeypatch)

    status, doc = export(acmodel, folder)
    now, probed_now = snapshot(out), list(ties)
    del ties[:]
    with monkeypatch.context() as patched:
        patched.setattr(economy_flow._Solver, "vector", before)
        status_before, doc_before = export(acmodel, folder)
    was, probed_before = snapshot(out), list(ties)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert (status_before, doc_before["errors"], doc_before["warnings"]) == (0, [], [])
    assert probed_now == [["act.train"]]  # the steady state's
    assert probed_before == [["act.train"]] * 3  # the steady state's, then each pass's
    prices, prices_before = json.loads(now["prices.json"]), json.loads(was["prices.json"])
    assert untimed(prices["impact"]) == [B_GROUP]
    assert [entry["skipped"] for entry in prices["estimates"]] == [None]
    assert policy.delivered(untimed_doc(prices)) == policy.delivered(untimed_doc(prices_before))
    assert sorted(now) == sorted(was)
    for name in now:
        if name != "prices.json":
            assert now[name] == was[name], name


# The timing of each pass -------------------------------------------------------------------------------------------


def test_each_pass_times_every_stage_and_one_not_solved_says_how_it_was_found(tmp_path, acmodel, monkeypatch):
    """With a clock that moves 1 ms at each reading (`ticking`):
    - flips_fixture (test_prices.py): its two passes (pt.k at 1/2 and at 2) are solved: `by` `solved`. Each
      `timing_ms` has every stage of `flow.STAGES` as a whole number of milliseconds, `lps` (#430 C-2a: each stage and
      `certificate`, each with five integers), and `total`. A has no term there (no limit or income cap counts act.p or
      act.q), so S1 and S2 are not solved: `acquisition` and `value` are 0; every stage that always runs is at least
      1 ms; `total`, the whole pass, is at least the sum of the stages.
    - `tie_beside_estimate`: A has a term (act.get, act.b), so S1 and S2 are solved in each pass: at least 1 ms each.
    - two_parts with b estimated from 3 to 4, 3 being pt.b's point value: the pass at `low` is the steady state's own
      vector, not solved again: `by` `steady_state`, `stages` null, every count of `lps` 0, and its `total`; the pass
      at `high` is solved."""
    ticking(monkeypatch, 1)
    flips = flips_fixture(tmp_path / "flips")
    tied = tie_beside_estimate(tmp_path / "tied")
    at_point = two_parts(tmp_path / "at_point", {**ESTIMATE_B, "low": "3"})

    for folder in (flips, tied, at_point):
        status, doc = export(acmodel, folder)
        assert (status, doc["errors"], doc["warnings"]) == (0, [], []), folder

    for each in passes(delivered(flips)) + passes(delivered(tied)):
        timing = each[TIMING]
        assert sorted(timing) == ["lps", "stages", "total"] and each["by"] == "solved"
        assert sorted(timing["lps"]) == sorted((*economy_flow.STAGES, "certificate"))
        assert all(
            sorted(entry) == sorted(economy_flow.LP_COUNTS) and all(type(n) is int for n in entry.values())
            for entry in timing["lps"].values()
        )
        stages = timing["stages"]
        assert sorted(stages) == sorted(economy_flow.STAGES)
        assert all(type(ms) is int and ms >= 0 for ms in (*stages.values(), timing["total"]))
        assert all(stages[stage] >= 1 for stage in ALWAYS), stages
        assert timing["total"] >= sum(stages.values())
    assert [(timing["stages"]["acquisition"], timing["stages"]["value"]) for timing in timings(delivered(flips))] == [
        (0, 0), (0, 0)
    ]
    assert all(timing["stages"]["acquisition"] >= 1 and timing["stages"]["value"] >= 1
               for timing in timings(delivered(tied)))
    low, high = passes(delivered(at_point))
    assert (low["at"], low["values"], high["at"], high["values"]) == (
        {"pt.b": "low"}, {"pt.b": "3"}, {"pt.b": "high"}, {"pt.b": "4"}
    )
    assert (low["by"], low[TIMING]["stages"]) == ("steady_state", None)
    assert all(n == 0 for entry in low[TIMING]["lps"].values() for n in entry.values())
    assert type(low[TIMING]["total"]) is int and low[TIMING]["total"] >= 0
    assert (high["by"], sorted(high[TIMING]["stages"])) == ("solved", sorted(economy_flow.STAGES))


def test_a_vector_the_cache_gives_holds_no_timing_and_its_pass_says_so(tmp_path, acmodel, monkeypatch):
    """A pass's graph computes without the cache (`graph.Moved`), so an export never takes a vector from it. Here each
    pass's vector is taken through the cache's encoding, as `Graph.cached` keeps a value and serves it again
    (`Graph.encode`, canonical JSON, `Graph.decode`): the value served is the same, but holds no timing, so the pass has
    none (#430 C-2a: its `timing_ms` is null, and its `by` `solved`, as only a solve gives a vector node).
    flips_fixture's prices.json is then, without its timing, the plain export's byte for byte, and so is every other
    file."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    export(acmodel, folder)
    plain = snapshot(out)
    compute = Moved.compute
    served: list[Any] = []

    def through_the_cache(graph: Any, key: tuple[str, ...]) -> Any:
        found = compute(graph, key)
        if key != economy_flow.VECTOR_NODE:
            return found
        again = graph.decode("economy", canonical(graph.encode(found)))
        served.append((found.timing is not None, again.timing, again.data == found.data))
        return again

    monkeypatch.setattr(Moved, "compute", through_the_cache)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert served == [(True, None, True), (True, None, True)]
    cached = snapshot(out)
    found = json.loads(cached["prices.json"])
    assert [(each["by"], each[TIMING]) for each in passes(found)] == [("solved", None), ("solved", None)]
    assert policy.delivered(untimed_doc(found)) == policy.delivered(untimed_doc(json.loads(plain["prices.json"])))
    for name in plain:
        if name != "prices.json":
            assert cached[name] == plain[name], name


def test_without_its_timing_prices_json_is_the_export_that_timed_nothing_and_nothing_else_holds_any(
    tmp_path, acmodel, app_dir, monkeypatch
):
    """flips_fixture, exported three times: with a clock that moves 1 ms at each reading, with one that moves 3 ms, and
    with prices.json written without any timing (#430 C-2a: neither the passes' `timing_ms` nor the steady state's
    `steady_timing_ms`; the export as it was before PASS-TIMING, but for the vector's own path, which the tests above
    compare). The two timed files differ in their passes' `timing_ms`; taken out, with `steady_timing_ms`, each is byte
    for byte the untimed file. The timing is in each pass's `timing_ms` and in `steady_timing_ms` and nowhere else of
    prices.json; no price's provenance file holds any timing, and each is the same in the three exports, byte for byte,
    so is each `binds`; the shared tree under the state root holds none either."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    build = price_export.build

    def timing_nothing(*args: Any) -> Any:
        files, *rest = build(*args)
        doc = json.loads(files[price_export.PRICES])
        return ({**files, price_export.PRICES: policy.delivered(untimed_doc(doc))}, *rest)

    written = []
    for step in (1, 3, None):
        with monkeypatch.context() as patched:
            if step is None:
                patched.setattr(price_export, "build", timing_nothing)
            else:
                ticking(patched, step)
            status, doc = export(acmodel, folder)
        assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
        written.append(snapshot(out))
    one, three, none = written
    first, second, plain = (json.loads(files["prices.json"]) for files in written)

    assert timings(first) != timings(second)
    assert timed_at(plain) == []
    assert policy.delivered(untimed_doc(first)) == policy.delivered(untimed_doc(second)) == none["prices.json"]
    assert timed_at(first) == [f"/impact/0/passes/{index}/{TIMING}" for index in range(2)] + ["/steady_timing_ms"]
    assert sorted(one) == sorted(three) == sorted(none)
    for name in none:
        if name != "prices.json":
            assert one[name] == three[name] == none[name], name
            assert timed_at(json.loads(one[name])) == [], name
    assert [entry["binds"] for entry in first["prices"]] == [entry["binds"] for entry in plain["prices"]]
    assert first["shared"] == second["shared"] == plain["shared"]
    tree = json.loads(kept(app_dir, first["shared"][STEADY_ROOT]["sha256"]).read_text(encoding="utf-8"))
    assert timed_at(tree) == []


def test_an_assessment_taken_over_keeps_the_timing_of_the_run_that_computed_it(tmp_path, acmodel, monkeypatch):
    """`--reuse-assessment` (PRICE-REUSE) takes the groups over as they are, each pass's `by` and `timing_ms` with them:
    they record the run that computed them. flips_fixture, exported with a clock of 1 ms a reading and its out folder
    kept aside, then exported in full again with a clock of 3 ms; the export that takes the kept assessment over writes
    the kept groups, timing and all, not the full export's, and is the full export's prices.json once
    `reused_assessment`, the passes' `timing_ms` and `steady_timing_ms` (#430 C-2a: this run's own) are taken out."""
    folder = flips_fixture(tmp_path)
    out, earlier_out = base(folder) / "out", tmp_path / "earlier"
    with monkeypatch.context() as patched:
        ticking(patched, 1)
        export(acmodel, folder)
    shutil.copytree(out, earlier_out)
    with monkeypatch.context() as patched:
        ticking(patched, 3)
        export(acmodel, folder)
    full = json.loads((out / "prices.json").read_bytes())

    status, doc = reuse(acmodel, folder, earlier_out)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    earlier = json.loads((earlier_out / "prices.json").read_bytes())
    taken = json.loads((out / "prices.json").read_bytes())
    assert taken["impact"] == earlier["impact"]
    assert timings(taken) == timings(earlier) != timings(full)
    assert all(each["by"] == "solved" for each in passes(taken))
    taken.pop("reused_assessment")
    assert policy.delivered(untimed_doc(taken)) == policy.delivered(untimed_doc(full))
