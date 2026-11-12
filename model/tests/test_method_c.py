"""#430 METHOD-C C-1 ("D* invariance"): the steady state hands the impact assessment what the certificate reads, held in
memory only, and computes and keeps what it did; S3's program built from a graph without solving it; a pass's program
compared with the steady state's (J, or where it differs elsewhere); and a steady state that came from the cache,
computed again once before any pass (contracts/formats.md, "`acmodel export prices`", "The certificate", items 1 and
11). Every pass that moves a value is still solved here.

The fixtures are the hand-solvable neutral models of test_economy.py, test_price_vector.py and test_prices.py, and one of
their own (`moves_fixture`); each docstring says by hand what the fixture gives. The economy node and the steady state as
they were computed before this slice (`before_compute`, `before_steady`) are kept here, line for line, to compare the two
paths. CI runs these tests on Windows and on Ubuntu."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from contextlib import closing
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from acmodel.cache import EVALUATORS, Session, canonical, open_store
from acmodel.check.coverage import judge
from acmodel.codes import EXIT_STATUS
from acmodel.core import coverage
from acmodel.core import flow as economy_flow
from acmodel.core import impact
from acmodel.core.common import PROVISIONAL, solve, worst
from acmodel.core.graph import Graph, Moved
from acmodel.core.run import evaluate_model, run_economy
from acmodel.envelope import Failure
from acmodel.export import policy
from acmodel.export import prices as price_export
from test_cache import LIBRARIES
from test_economy import (  # noqa: F401 (the fixture `economy`)
    DAY,
    WEEK,
    activity,
    cap,
    cyclic_fixture,
    economy,
    flow,
    goal,
    limit,
    model,
    model_path,
    regen,
    resource,
    shared_fixture,
)
from test_price_reuse import snapshot
from test_price_vector import ex2
from test_prices import (  # noqa: F401 (the fixture `acmodel`)
    HEX,
    acmodel,
    base,
    export,
    flips_fixture,
    provisional,
    untimed,
    untimed_doc,
)

STEADY, VECTOR = economy_flow.NODES[economy_flow.STEADY], economy_flow.VECTOR_NODE
# flips_fixture's one group, without its passes' `timing_ms` (test_prices.py works it out): s's price over [1, 2], and
# act.p and act.q flip.
FLIPS_GROUP = {
    "rules": ["pt.k"], "unknowns": [],
    "passes": [{"at": {"pt.k": "low"}, "values": {"pt.k": "1/2"}}, {"at": {"pt.k": "high"}, "values": {"pt.k": "2"}}],
    "prices": {"res.s": {"low": "1", "high": "2"}}, "flips": ["act.p", "act.q"],
}


def before_compute(graph: Graph, solve_name: str) -> Any:
    """`flow.compute` as it was before METHOD-C C-1, line for line (the module's names qualified, its argument `solve`
    renamed): it reads the model in place and holds nothing beside the value."""
    clock = economy_flow._Clock() if solve_name == economy_flow.VECTOR else None
    graph.depend(("economy_model",))
    built = economy_flow.STEADY if solve_name in (economy_flow.QUICK, economy_flow.VECTOR) else solve_name
    shape = economy_flow._shape(graph, built)
    reader = economy_flow._Reader(graph, shape, built)
    model = reader.model(graph.economy)
    if model is None:
        return economy_flow.Economy(blocked_by=graph.register(reader.refused) | frozenset(reader.blocked))
    status = reader.inputs()
    solver = economy_flow._Solver(model, graph.economy, clock)
    solver.lap("model")
    solves = {
        economy_flow.QUICK: solver.quick, economy_flow.VECTOR: solver.vector, economy_flow.STEADY: solver.steady,
        economy_flow.PLAN: solver.plan,
    }
    try:
        data, warnings = solves[solve_name]()
    except Failure as failure:
        return economy_flow.Economy(blocked_by=graph.register(failure.errors))
    if solve_name == economy_flow.VECTOR:
        return economy_flow.Economy(data, status, False, (), reads=tuple(reader.reads), timing=clock.ms())
    if solve_name == economy_flow.QUICK:
        found, _ = coverage.report(graph, reader, model, warnings)
        shown = {
            "status": economy_flow.QUICK, **data, "coverage": found, "coverage_unknown": list(coverage.NEEDS_LATER_STAGES),
        }
        return economy_flow.Economy(shown, economy_flow.QUICK, False, tuple(warnings), reads=tuple(reader.reads))
    if solve_name == economy_flow.STEADY:
        pending = reader.excluded_pending()
        found, prices = coverage.report(graph, reader, model, warnings)
        status = worst(status, *([PROVISIONAL] if pending else []), *(price["status"] for price in prices.values()))
        data["excluded_pending"] = pending
        data["price_status"] = prices
        data["coverage"] = found
    per_account = solve_name == economy_flow.PLAN
    return economy_flow.Economy(
        {"status": status, "per_account": per_account, **data}, status, per_account, tuple(warnings),
        reads=tuple(reader.reads),
    )


def before_steady(self: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """`flow._Solver.steady` as it was before METHOD-C C-1, line for line (the module's names qualified): it keeps
    nothing of what it computed with."""
    model, steps = self.model, self.model.steps
    tiers = self.tiers()
    program, base, optimum = tiers.program, tiers.base, tiers.base.value
    least = self.least_losses(tiers)
    warnings = self.tier_warnings(tiers)
    dual = tiers.dual
    on_face = economy_flow._restricted(dual, program, base.x)
    prices, grains, marginals = self.projections(tiers, on_face, warnings)
    rule_prices: dict[str, Any] = {}
    rule_marginals: dict[str, Fraction] = {}
    for rule, value in model.priced():
        price = self.price(
            optimum, lambda delta: self.resolved(economy_flow._program(model, values={rule: value + delta}))
        )
        marginal = self.marginal(self.rule_dual(program, base, rule))
        warnings += self.disagrees(rule, None, marginal, price)
        rule_prices[rule], rule_marginals[rule] = price, marginal
    values: dict[str, Any] = {}
    for name in model.activities:
        if model.touches_store(name):
            values[name] = None
            continue
        values[name] = [economy_flow._price(self.value(name, prices, step)) for step in range(steps)]
    joint, judged = self.joint(dual, tiers.point, optimum, prices, on_face, base.x, tiers.optimal, tiers.seed)
    warnings += judged
    data = {
        "cycle": {"steps": steps, "step_seconds": economy_flow.STEP_SECONDS},
        **self.shown_tiers(tiers),
        "prices": {resource: [economy_flow._price(price) for price in prices[resource]] for resource in model.resources},
        "rule_prices": {rule: economy_flow._price(price) for rule, price in rule_prices.items()},
        "activity_values": values,
        "joint": joint,
        "losses": self.shown_losses(least),
        "grain": {
            "prices": {resource: [economy_flow._price(pair) for pair in grains[resource]] for resource in model.resources}
        },
        "marginals": {
            "prices": {resource: [str(m) for m in marginals[resource]] for resource in model.resources},
            "rule_prices": {rule: str(m) for rule, m in rule_marginals.items()},
        },
        "excluded": list(model.excluded),
        "relaxed": model.shown_relaxed(),
    }
    return data, warnings


def as_before(patched: pytest.MonkeyPatch) -> None:
    """The economy as it was before this slice: the node and the steady state (`before_compute`, `before_steady`), and an
    assessment that reads the steady state it is given, wherever it came from."""
    patched.setattr(economy_flow, "compute", before_compute)
    patched.setattr(economy_flow._Solver, "steady", before_steady)
    patched.setattr(impact, "_again", lambda graph, steady: steady)


def graph_of(folder: Path, cache: Session | None = None) -> Graph:
    """The fixture's model evaluated for its state, through `cache` when one is given: its graph."""
    return evaluate_model(
        base(folder), folder / "state.json", datasets=folder / "datasets", max_bits=127, cache=cache
    ).graph


def solved(graph: Graph, key: tuple[str, ...]) -> Any:
    solve([key], graph.compute, graph.results)
    return graph.results[key]


def moved(graph: Graph, rule: str, value: Fraction) -> Moved:
    """A pass's graph: `rule` at `value`, every other rule at its point value (`graph.Moved`)."""
    return Moved(graph, Moved.base(graph, {rule}), {rule: value})


def moves_fixture(folder: Path) -> Path:
    """A week of seven daily steps (the model has a day and a week period), every pt.* a provisional derive: s comes at
    `pt.regen` (10) a day; act.p turns 1 s into 1 v; act.q turns 1 s into `pt.k` (1) v, at most `pt.most` (8) a day; v,
    the main goal worth `pt.weight` (1), has a hard cap of `pt.cap` (100), so what comes in may overflow. pt.idle (5) is
    read by no rule the economy reads.

    By hand, S3's program over the seven steps: act.q's execution column of step t (`x:act.q:t`) is in s's balance row
    of t (1, what it consumes), in v's balance row of t (-pt.k) and v's overflow bound of t (-pt.k, the income the cap
    may block), and in its limit's row of t (1); its right side is pt.most. s's balance row of t has pt.regen on its
    right side; each of v's cap rows (one per step boundary, 0 to 7) pt.cap; v's goal sink of t is worth pt.weight in the
    objective. So moving pt.k changes act.q's seven columns in balance rows and overflow bounds only; pt.cap, pt.most
    and pt.regen a right side; pt.weight the objective; pt.idle nothing."""
    rules = [
        provisional("pt.regen", "res.s", "10"), regen("regen.s", "res.s", "rule:pt.regen", "1d"),
        flow("flow.p", "act.p", consumes={"res.s": "1"}, produces={"res.v": "1"}),
        provisional("pt.k", "act.q", "1"),
        flow("flow.q", "act.q", consumes={"res.s": "1"}, produces={"res.v": "rule:pt.k"}),
        provisional("pt.most", "act.q", "8"), limit("limit.q", "act.q", "rule:pt.most", "day"),
        provisional("pt.cap", "res.v", "100"),
        cap("cap.v.hard", "res.v", "hard", "block_all_income", value="rule:pt.cap"),
        provisional("pt.weight", "res.v", "1"),
        provisional("pt.idle", "act.p", "5"),
    ]
    entities = [resource("res.s", "fact.s"), resource("res.v", "fact.v"), activity("act.p"), activity("act.q")]
    goals = [goal("goal.v", "res.v", "rule:pt.weight")]
    return model(folder, [DAY, WEEK], entities, rules, {"fact.s": 0, "fact.v": 0}, goals=goals)


# 1. The steady state and the vector compute and keep what they did ------------------------------------------------------


@pytest.mark.parametrize(
    "fixture",
    [
        pytest.param(ex2, id="ex2"),
        pytest.param(cyclic_fixture, id="cyclic"),
        pytest.param(flips_fixture, id="flips"),
        pytest.param(lambda folder: shared_fixture(folder, "10", most="10"), id="tie"),
    ],
)
def test_the_steady_state_and_the_vector_are_what_they_were_and_hold_the_rest_beside_their_value(
    tmp_path, monkeypatch, fixture
):
    """EX2 (K holds three pools and an activity), cyclic_fixture (seven steps, a protected acquisition, a hard cap),
    flips_fixture (a provisional derive) and test_economy.py's tie (`objective_tier_tie`, its warnings): the steady state
    and the vector of each, computed now and as they were before this slice (`as_before`), are equal and are kept by the
    cache's encoding (`Graph.encode`, canonical JSON) byte for byte alike, with the members it always kept. Only the
    steady state computed now holds what the certificate reads, beside its value (`held`); the one computed as before,
    and every vector, hold none."""
    folder = fixture(tmp_path)
    graph = graph_of(folder)
    found, vector = solved(graph, STEADY), solved(graph, VECTOR)
    with monkeypatch.context() as patched:
        as_before(patched)
        earlier = graph_of(folder)
        was, was_vector = solved(earlier, STEADY), solved(earlier, VECTOR)

    assert not found.blocked and not vector.blocked
    assert found == was and vector == was_vector
    assert canonical(graph.encode(found)) == canonical(earlier.encode(was))
    assert canonical(graph.encode(vector)) == canonical(earlier.encode(was_vector))
    assert sorted(economy_flow.encode(found, [])) == sorted(economy_flow.FORMAT["economy"])
    assert isinstance(found.held, economy_flow.Held)
    assert (was.held, vector.held, was_vector.held) == (None, None, None)
    assert EVALUATORS["economy"].version == 12


def kept_nodes(state_root: Path) -> list[tuple[Any, ...]]:
    """Every value the cache under `state_root` keeps: its kind, node, spec, key (its spec and its trace), value hash and
    trace, by key."""
    with closing(sqlite3.connect(state_root / "cache" / "index.sqlite")) as index:
        return index.execute("SELECT kind, node, spec, key, value, deps FROM nodes ORDER BY key").fetchall()


def test_the_cache_keeps_what_it_kept_and_the_economy_version_is_12(tmp_path, economy, monkeypatch):
    """cyclic_fixture's steady state and plan by `acmodel economy`, through a cache of its own, now and as before this
    slice (`as_before`): the same answer, the same cache entries (each node's spec, key, value hash and trace), the
    economy's two among them, and the same run outputs, byte for byte. The economy's evaluator version is 12: this slice
    left it at 11, and #430's second try of an LP on D* moved it."""
    folder = cyclic_fixture(tmp_path / "model")
    now_root, before_root = tmp_path / "now", tmp_path / "before"

    status, doc = economy(folder, paths={"state_root": str(now_root)})
    with monkeypatch.context() as patched:
        as_before(patched)
        status_before, doc_before = economy(folder, paths={"state_root": str(before_root)})

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert (status_before, doc_before["errors"], doc_before["warnings"]) == (0, [], [])
    assert doc["data"] == doc_before["data"]
    now, was = kept_nodes(now_root), kept_nodes(before_root)
    assert now == was
    assert [kind for kind, *_ in now].count("economy") == 2  # the steady state and the plan
    run_id = doc["data"]["run_id"]
    outputs = [root / "runs" / run_id / "outputs.json" for root in (now_root, before_root)]
    assert outputs[0].read_bytes() == outputs[1].read_bytes()
    assert EVALUATORS["economy"].version == 12


def test_an_export_writes_what_it_wrote_before(tmp_path, acmodel, monkeypatch):
    """flips_fixture exported now and as before this slice (`as_before`): the same files; every price's provenance file
    byte for byte, and prices.json too once each pass's `timing_ms` is taken out, the group the one worked out by hand.
    What the steady state holds beside its value is in none of them."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"

    status, doc = export(acmodel, folder)
    now = snapshot(out)
    with monkeypatch.context() as patched:
        as_before(patched)
        status_before, doc_before = export(acmodel, folder)
    was = snapshot(out)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert (status_before, doc_before["errors"], doc_before["warnings"]) == (0, [], [])
    prices, prices_before = json.loads(now["prices.json"]), json.loads(was["prices.json"])
    assert untimed(prices["impact"]) == [FLIPS_GROUP]
    assert policy.delivered(untimed_doc(prices)) == policy.delivered(untimed_doc(prices_before))
    assert sorted(now) == sorted(was)
    for name in now:
        if name != "prices.json":
            assert now[name] == was[name], name


# 2. S3's program built without solving it ---------------------------------------------------------------------------


def recorded_programs(monkeypatch: Any) -> list[Any]:
    """S3's program of each run of the three solves (`flow._Solver.tiers`), in order: the program each solve built."""
    found: list[Any] = []
    tiers = economy_flow._Solver.tiers

    def recorded(solver: Any, **options: Any) -> Any:
        result = tiers(solver, **options)
        found.append(result.program)
        return result

    monkeypatch.setattr(economy_flow._Solver, "tiers", recorded)
    return found


def alike(built: Any, solved_program: Any) -> None:
    """Two programs row by row and column by column: the same row keys in the same order, each row's sense, right side,
    group and coefficients; the same columns with the same bounds; the same objective."""
    assert [row.key for row in built.rows] == [row.key for row in solved_program.rows]
    for row, other in zip(built.rows, solved_program.rows):
        assert (row.sense, row.rhs, row.group, row.coefficients) == (
            other.sense, other.rhs, other.group, other.coefficients
        ), row.key
    assert built.variables == solved_program.variables
    assert built.objective == solved_program.objective
    assert built == solved_program


@pytest.mark.parametrize(
    "fixture",
    [pytest.param(flips_fixture, id="flips"), pytest.param(moves_fixture, id="moves")],
)
def test_the_program_built_without_solving_is_the_one_each_solve_builds(tmp_path, monkeypatch, fixture):
    """flips_fixture (one step) and moves_fixture (seven, with a hard cap's overflow bounds): S3's program built from the
    steady state's graph without solving it (`flow.build_program`) is the one the steady state's solve builds, row by row
    and column by column, and is the program the steady state hands on (`held.program`); built from pt.k's pass at 2
    (`graph.Moved`) before the pass is solved, it is the one that pass's vector builds. The two differ (pt.k is a
    coefficient of S3's program), and building solves nothing (only the two solves ran the three solves)."""
    graph = graph_of(fixture(tmp_path))
    programs = recorded_programs(monkeypatch)

    steady = solved(graph, STEADY)
    built = economy_flow.build_program(graph)
    moved_graph = moved(graph, "pt.k", Fraction(2))
    built_moved = economy_flow.build_program(moved_graph)
    vector = solved(moved_graph, VECTOR)

    assert not steady.blocked and not vector.blocked
    assert not built.blocked and not built_moved.blocked
    assert len(programs) == 2
    alike(built.program, programs[0])
    alike(built_moved.program, programs[1])
    assert steady.held.program is programs[0]
    assert built.program != built_moved.program


# 3. A pass's program compared with the steady state's ----------------------------------------------------------------

J_OF_Q = tuple(("x", "act.q", step) for step in range(7))


@pytest.mark.parametrize(
    ("rule", "value", "columns", "elsewhere"),
    [
        pytest.param("pt.k", Fraction(2), J_OF_Q, None, id="one_flow_amount"),
        pytest.param(
            "pt.cap", Fraction(99), None,
            {"part": "row", "row": "cap:cap.v.hard:0", "member": "rhs", "steady": "100", "pass": "99"}, id="cap",
        ),
        pytest.param(
            "pt.most", Fraction(9), None,
            {"part": "row", "row": "limit:limit.q:0", "member": "rhs", "steady": "8", "pass": "9"}, id="limit",
        ),
        pytest.param(
            "pt.regen", Fraction(12), None,
            {"part": "row", "row": "balance:res.s:0", "member": "rhs", "steady": "10", "pass": "12"}, id="regen",
        ),
        pytest.param(
            "pt.weight", Fraction(2), None,
            {"part": "objective", "column": "g:goal.v:0", "steady": "1", "pass": "2"}, id="objective_weight",
        ),
        pytest.param("pt.idle", Fraction(6), (), None, id="unread"),
    ],
)
def test_comparing_a_passs_program_gives_j_or_where_it_differs_elsewhere(tmp_path, rule, value, columns, elsewhere):
    """moves_fixture works out where each pt.* sits in S3's program. Each pass's program, built from its graph without
    solving it, compared with the steady state's (`flow.compare_programs`):
    - pt.k at 2 changes act.q's coefficients only, in v's balance rows and overflow bounds: J is exactly act.q's seven
      columns, by step, and act.q its one activity;
    - pt.cap, pt.most and pt.regen move a right side: the program differs elsewhere, first at the first such row by key
      (v's cap at boundary 0, act.q's limit in window 0, s's balance in step 0);
    - pt.weight moves the objective, a column outside J (v's goal sink in step 0): elsewhere too;
    - pt.idle reaches no coefficient of the program: J is empty, the two programs one.
    The steady state's program compared with itself has an empty J."""
    graph = graph_of(moves_fixture(tmp_path))
    steady = economy_flow.build_program(graph).program
    found = economy_flow.compare_programs(steady, economy_flow.build_program(moved(graph, rule, value)).program)

    assert found == economy_flow.Compared(columns, elsewhere)
    assert found.activities == (("act.q",) if columns else ())
    assert economy_flow.compare_programs(steady, steady) == economy_flow.Compared(())


# 4. What the steady state hands on gives its vector again ------------------------------------------------------------


def test_what_the_steady_state_hands_on_selects_its_joint_vector_again(tmp_path):
    """EX2 (test_price_vector.py works it out): V0 = 1; D* is π_v = 1, π_a + π_b + π_c = 1, each at least 0, and π_a +
    π_b at least 2/3; K is the pools a and b (each on [0, 1]) and c (on [0, 1/3]), and act.pair (its value on [-1/3, 0]);
    the vector is a 3/7, b 3/7, c 1/7, v 1, and act.pair reads -4/21, act.set 0.

    The selection done again from what the steady state hands on alone (`held`): a solver of the same model given S3's
    dual and y*, V0, D*'s complementary-slackness form, x* and S3's optimal face, and no projection that spares an LP
    (each interval is found again on D*), lean as a pass's vector. It gives the steady state's `joint`, coordinate by
    coordinate, with the same max-min stages and activity values, and chooses the same K, with the same functions and
    ends, the K handed on. The vector and the values handed on are the steady state's, exact; x* is S3's optimum, at V0;
    the program is the one built without solving."""
    graph = graph_of(ex2(tmp_path))
    steady = solved(graph, STEADY)
    held = steady.held
    built = economy_flow.build_program(graph)
    solver = economy_flow._Solver(built.model, graph.economy)
    no_projection = {pool: [(None, None)] * built.model.steps for pool in built.model.resources}

    joint, _ = solver.joint(
        held.dual, held.point, held.value, no_projection, held.restricted, held.x, held.optimal, None, lean=True
    )

    shown = steady.data["joint"]
    coordinates = {pool: entry["value"] for pool, entry in shown["prices"].items()}
    assert coordinates == {"res.a": "3/7", "res.b": "3/7", "res.c": "1/7", "res.v": "1"}
    assert {pool: entry["value"] for pool, entry in joint["prices"].items()} == coordinates
    assert joint["max_min"] == shown["max_min"]
    values = {name: entry["value"] for name, entry in shown["activities"].items()}
    assert values == {"act.pair": "-4/21", "act.set": "0"}
    assert {name: entry["value"] for name, entry in joint["activities"].items()} == values
    assert held.vector == {pool: Fraction(value) for pool, value in coordinates.items()} == solver.chosen.vector
    assert held.values == {name: Fraction(value) for name, value in values.items()} == solver.chosen.values
    assert {name: (low, high, two) for name, (_, low, high, two) in held.coordinates.items()} == {
        "price:res.a": (0, 1, True), "price:res.b": (0, 1, True), "price:res.c": (0, Fraction(1, 3), True),
        "activity_value:act.pair": (Fraction(-1, 3), 0, False),
    }
    assert solver.chosen.coordinates == held.coordinates
    assert held.value == 1 and economy_flow._violation(held.program, held.x) is None
    assert sum((coefficient * held.x[key] for key, coefficient in held.program.objective.items()), Fraction(0)) == 1
    assert held.program == built.program
    assert (held.protected, held.acquisition, held.acquired) == ((), {}, 0)  # no limit or income cap: A has no term


def test_the_steady_state_hands_on_p_a_and_a_star(tmp_path):
    """cyclic_fixture (test_economy.py): act.d is protected (its only input has a regen and a limit counts it), so P is
    act.d and A is what it brings in of v, the main goal worth 1: 1 an execution in each of the seven steps. A* = V0 = 6
    (six executions a week, as the steady state shows)."""
    graph = graph_of(cyclic_fixture(tmp_path))
    held = solved(graph, STEADY).held

    assert held.protected == ("act.d",)
    assert held.acquisition == {("x", "act.d", step): 1 for step in range(7)}
    assert (held.acquired, held.value) == (6, 6)


# 5. A steady state from the cache ---------------------------------------------------------------------------------------


def watched(monkeypatch: Any) -> list[Any]:
    """What runs, in order: each steady state solved (`steady`) and each pass's vector (`vector`); each node computed
    through the cache (`("cached", key)`) and each value kept in it (`("kept", kind)`); and each assessment's start,
    with whether it holds what the certificate reads (`("held", bool)`)."""
    events: list[Any] = []
    steady, vector = economy_flow._Solver.steady, economy_flow._Solver.vector
    cached, save, start = Graph.cached, Session.save, impact._Assessor.__init__

    def solving_steady(solver: Any) -> Any:
        events.append("steady")
        return steady(solver)

    def solving_vector(solver: Any) -> Any:
        events.append("vector")
        return vector(solver)

    def through_the_cache(graph: Any, key: tuple[str, ...]) -> Any:
        events.append(("cached", key))
        return cached(graph, key)

    def keeping(session: Any, kind: str, *args: Any) -> Any:
        events.append(("kept", kind))
        return save(session, kind, *args)

    def starting(assessor: Any, graph: Any, found: Any) -> None:
        start(assessor, graph, found)
        events.append(("held", assessor.held is not None))

    monkeypatch.setattr(economy_flow._Solver, "steady", solving_steady)
    monkeypatch.setattr(economy_flow._Solver, "vector", solving_vector)
    monkeypatch.setattr(Graph, "cached", through_the_cache)
    monkeypatch.setattr(Session, "save", keeping)
    monkeypatch.setattr(impact._Assessor, "__init__", starting)
    return events


def assessment_of(graph: Graph, steady: Any) -> impact.Assessment:
    """The impact assessment of `steady`, with what `export prices` gives it (`export.prices.build`)."""
    checker = graph.checker
    reach = impact.reached(graph, steady, judge(checker))
    gaps = {name for name, component in steady.data["coverage"]["components"].items() if component["gap"]}
    lacking = [derive for derive in reach if "estimate" not in checker.entries[derive].doc["logic"]]
    missing = set().union(*(reach[derive] for derive in lacking)) if lacking else set()
    return impact.assess(graph, steady, reach, gaps, missing)


def cache_event(event: Any) -> bool:
    return isinstance(event, tuple) and event[0] in ("cached", "kept")


def test_a_steady_state_the_cache_gives_is_computed_again_once_without_it_and_then_assessed(tmp_path, monkeypatch):
    """flips_fixture's steady state kept in a cache by `acmodel economy --steady-only` (`run_economy`), then given by it
    in a second session: a value the cache gives holds none of what the certificate reads. The impact assessment of that
    graph computes the steady state again exactly once, at its start, before any pass: `flow.compute` itself, so the
    steady state is neither looked up in the cache nor kept in it meanwhile. Computed again it is the cached one byte for
    byte as the cache keeps it, so the assessment starts with what the certificate reads and solves the two passes
    flips_fixture works out, as an assessment of a steady state computed in the run does."""
    folder = flips_fixture(tmp_path / "model")
    root = tmp_path / "state"
    with open_store(root, max_bytes=2**31) as store:
        _, errors, _ = run_economy(
            base(folder), folder / "state.json", datasets=folder / "datasets", max_bits=127,
            cache=Session(store, libraries=LIBRARIES), steady_only=True,
        )
    events = watched(monkeypatch)
    with open_store(root, max_bytes=2**31) as store:
        session = Session(store, libraries=LIBRARIES)
        graph = graph_of(folder, session)
        solve([STEADY], graph.cached, graph.results)
        served = graph.results[STEADY]
        work = session.work()["by_kind"]["economy"]
        del events[:]
        found = assessment_of(graph, served)
        during = list(events)
    fresh_graph = graph_of(folder)
    fresh = assessment_of(fresh_graph, solved(fresh_graph, STEADY))

    assert errors == []
    assert work == {"computed": 0, "reused": 1, "cutoff": 0}
    assert served.held is None and not served.blocked
    assert [event for event in during if not cache_event(event)] == ["steady", ("held", True), "vector", "vector"]
    economy_in_the_cache = [
        event for event in during if event == ("kept", "economy") or (event[0] == "cached" and event[1][0] == "economy")
    ]
    assert economy_in_the_cache == []
    assert untimed(found.groups) == untimed(fresh.groups) == [FLIPS_GROUP]
    assert (found.ranges, found.flips, found.assessed, found.skipped) == (
        fresh.ranges, fresh.flips, fresh.assessed, fresh.skipped
    )


def served_from_the_cache(monkeypatch: Any, *, differs: bool) -> tuple[list[str], list[str]]:
    """`export prices` with its steady state served as the cache serves one (`export.prices._steady`: computed, then taken
    through the cache's encoding, `Graph.encode`, canonical JSON and `Graph.decode`, as `Graph.cached` keeps a value and
    gives it back, and put in the run's graph for every reader): it holds none of what the certificate reads. With
    `differs`, each steady state solved after the first shows another objective: a cache that holds a steady state this
    tool computes otherwise. Returns what ran, in order (each steady state solved, `steady`; the one served, `served`;
    each pass's vector, `vector`), and the sha256 of each served value's canonical JSON."""
    events: list[str] = []
    served: list[str] = []
    computed, steady, vector = price_export._steady, economy_flow._Solver.steady, economy_flow._Solver.vector

    def solving_steady(solver: Any) -> Any:
        data, warnings = steady(solver)
        events.append("steady")
        if differs and events.count("steady") > 1:
            data = {**data, "objective": "-" + data["objective"]}
        return data, warnings

    def solving_vector(solver: Any) -> Any:
        events.append("vector")
        return vector(solver)

    def from_the_cache(run: Any) -> Any:
        found = computed(run)
        blob = canonical(run.graph.encode(found))
        again = run.graph.decode("economy", blob)
        run.graph.results[STEADY] = again
        events.append("served")
        served.append(hashlib.sha256(blob).hexdigest())
        return again

    monkeypatch.setattr(economy_flow._Solver, "steady", solving_steady)
    monkeypatch.setattr(economy_flow._Solver, "vector", solving_vector)
    monkeypatch.setattr(price_export, "_steady", from_the_cache)
    return events, served


def test_a_cached_steady_state_computed_again_alike_is_exported_as_one_computed_in_the_run(
    tmp_path, acmodel, monkeypatch
):
    """flips_fixture exported, then exported again with its steady state served as the cache serves one
    (`served_from_the_cache`): the assessment computes it again once, before any pass, finds it the served one byte for
    byte and goes on with it: the two passes are solved, and every file is the plain export's (prices.json once each
    pass's `timing_ms` is taken out)."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    export(acmodel, folder)
    plain = snapshot(out)
    events, _ = served_from_the_cache(monkeypatch, differs=False)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert events == ["steady", "served", "steady", "vector", "vector"]
    again = snapshot(out)
    found = json.loads(again["prices.json"])
    assert untimed(found["impact"]) == [FLIPS_GROUP]
    assert policy.delivered(untimed_doc(found)) == policy.delivered(untimed_doc(json.loads(plain["prices.json"])))
    assert sorted(again) == sorted(plain)
    for name in plain:
        if name != "prices.json":
            assert again[name] == plain[name], name


def test_a_cached_steady_state_the_tool_computes_otherwise_is_a_mismatch_and_nothing_is_assessed_or_written(
    tmp_path, acmodel, app_dir, monkeypatch
):
    """flips_fixture exported with its steady state served as the cache serves one, and computed again otherwise (another
    objective; `served_from_the_cache`): `steady_state_mismatch` (exit 6), `expected` the sha256 of the served value's
    canonical JSON and `found` that of the one computed again. The steady state is computed again exactly once and no
    pass is solved: the assessment never falls back to solving every pass. Nothing is written: no out folder, no shared
    tree under the state root."""
    folder = flips_fixture(tmp_path)
    events, served = served_from_the_cache(monkeypatch, differs=True)

    status, doc = export(acmodel, folder)

    assert status == 6
    [found] = doc["errors"]
    assert (found["code"], found["detail"]["path"], found["detail"]["output"]) == (
        "steady_state_mismatch", model_path(folder), "economy:steady"
    )
    assert found["detail"]["expected"] == served[0]
    assert HEX.fullmatch(found["detail"]["found"]) and found["detail"]["found"] != served[0]
    assert events == ["steady", "served", "steady"]
    assert snapshot(base(folder) / "out") == {}
    assert snapshot(app_dir / "state" / "provenance") == {}


def test_the_two_codes_of_the_catalog_are_registered_with_their_exit_statuses():
    """`steady_state_mismatch` and `certificate_mismatch` (contracts/codes.json, #430 METHOD-C) are registered codes,
    each with the exit status the catalog gives it, 6."""
    catalog = json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))
    for code in ("steady_state_mismatch", "certificate_mismatch"):
        assert EXIT_STATUS[code] == catalog["codes"][code]["exit"] == 6, code
