"""#430 METHOD-C C-2a-1: each pass says how its vector was found (`by`), its `timing_ms` counts the LPs HiGHS ran in
each stage with its two kinds of second try apart (`lps`), the steady state records its own timing
(`steady_timing_ms`), and an assessment taken over is checked for both (contracts/formats.md, "`acmodel export
prices`", "A pass", "Timing", "The steady state's timing", "Taking an assessment over"; "Exactness and failures",
"Other failures", "Second tries are counted").

The fixtures are the hand-solvable neutral models of test_economy.py, test_prices.py and test_pass_timing.py; each
docstring says by hand what the fixture gives. HiGHS's answers are given as test_dstar_retry.py gives them (`Highs`):
each run is HiGHS's own, and only the status and the text it answers are replaced. Times are read from `flow.CLOCK`,
which some tests replace with a clock that moves a whole number of milliseconds at each reading (`ticking`). CI runs
these tests on Windows and on Ubuntu."""

from __future__ import annotations

import json
import shutil
from typing import Any, Callable

import pytest

from acmodel.cache import EVALUATORS, canonical
from acmodel.core import flow as economy_flow
from acmodel.core import impact
from acmodel.export import policy
from acmodel.model import readers
from test_dstar_retry import INFEASIBLE, Highs
from test_economy import cyclic_fixture, write
from test_method_c import as_before as as_before_c1
from test_method_c import graph_of, solved
from test_pass_timing import passes, tie_beside_estimate, ticking
from test_price_reuse import reuse, snapshot
from test_prices import (  # noqa: F401 (the fixture `acmodel`)
    ESTIMATE_B,
    STEADY_ROOT,
    acmodel,
    base,
    delivered,
    edit,
    export,
    flips_fixture,
    kept,
    priced,
    two_parts,
)

STEADY, VECTOR = economy_flow.NODES[economy_flow.STEADY], economy_flow.VECTOR_NODE
TIMING, STEADY_TIMING, BY = "timing_ms", "steady_timing_ms", "by"
AGAIN = ("again_infeasible", "again_other")
PASS_LPS = (*economy_flow.STAGES, "certificate")
SIX = ["program", "c1", "c2", "c3", "c4", "lp_unverified"]
P0, P1 = "/impact/0/passes/0", "/impact/0/passes/1"


def named(value: Any, test: Callable[[str], bool], where: str = "") -> list[str]:
    """The JSON pointer of every member whose name passes `test`, anywhere in `value`, in the order of its keys."""
    found: list[str] = []
    if isinstance(value, dict):
        for key, inner in value.items():
            pointer = f"{where}/{key}"
            if test(key):
                found.append(pointer)
            found += named(inner, test, pointer)
    elif isinstance(value, list):
        for index, inner in enumerate(value):
            found += named(inner, test, f"{where}/{index}")
    return found


def shaped(lps: dict[str, Any], stages: tuple[str, ...]) -> bool:
    """`lps` holds exactly `stages`, each with exactly the five members of `flow.LP_COUNTS`, each an integer, at
    least 0."""
    return list(lps) == sorted(stages) and all(
        sorted(entry) == sorted(economy_flow.LP_COUNTS) and all(type(n) is int and n >= 0 for n in entry.values())
        for entry in lps.values()
    )


def lps_of(doc: dict[str, Any]) -> list[dict[str, Any]]:
    """The steady state's `lps` (`steady_timing_ms`), then each pass's, in order."""
    return [doc[STEADY_TIMING]["lps"], *(each[TIMING]["lps"] for each in passes(doc))]


def counts(lps: dict[str, Any]) -> dict[str, tuple[int, int, int]]:
    """Each stage's runs and its two kinds of second try: what never reads the clock."""
    return {stage: (entry["runs"], entry["again_infeasible"], entry["again_other"]) for stage, entry in lps.items()}


def second_tries(doc: dict[str, Any]) -> list[tuple[str, str, str, int]]:
    """Every second try counted in prices.json: where (`steady` or `pass <n>`), the stage, its kind and how many."""
    found = []
    named_lps = [("steady", doc[STEADY_TIMING]["lps"])]
    named_lps += [(f"pass {index}", each[TIMING]["lps"]) for index, each in enumerate(passes(doc))]
    for where, lps in named_lps:
        for stage, entry in lps.items():
            found += [(where, stage, kind, entry[kind]) for kind in AGAIN if entry[kind]]
    return found


def bare(doc: dict[str, Any]) -> dict[str, Any]:
    """prices.json without what records the run that wrote it (contracts/formats.md, "Taking an assessment over", step
    3): `reused_assessment`, each pass's `by` and `timing_ms`, and `steady_timing_ms`."""
    groups = [
        {**group, "passes": [{k: v for k, v in each.items() if k not in (BY, TIMING)} for each in group["passes"]]}
        for group in doc["impact"]
    ]
    return {**{k: v for k, v in doc.items() if k not in ("reused_assessment", STEADY_TIMING)}, "impact": groups}


class Recorded:
    """Each HiGHS run of an LP (`flow._highs`, `_highs_seeded`, `_highs_again`, `_highs_moved`: one run each, also
    where `_highs_seeded` hands its LP to `_highs`), and each end of a stage on a clock (`flow._Clock.lap`), in order.
    The runs since the last end of a stage are the stage's that ends next (`by_clock`: each clock, in the order its
    first stage ended, to its runs by stage); `since` holds the runs no stage has ended after."""

    def __init__(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self.since = 0
        self.inside = False
        self.by_clock: dict[Any, dict[str, int]] = {}
        for name in ("_highs", "_highs_seeded", "_highs_again", "_highs_moved"):
            monkeypatch.setattr(economy_flow, name, self.counting(getattr(economy_flow, name)))
        lap = economy_flow._Clock.lap

        def lapped(clock: Any, stage: str) -> None:
            lap(clock, stage)
            found = self.by_clock.setdefault(clock, dict.fromkeys(clock.stages, 0))
            found[stage] += self.since
            self.since = 0

        monkeypatch.setattr(economy_flow._Clock, "lap", lapped)

    def counting(self, run: Callable[..., Any]) -> Callable[..., Any]:
        def counted(*args: Any, **kwargs: Any) -> Any:
            if self.inside:  # `_highs_seeded` handing its LP to `_highs`: the same run
                return run(*args, **kwargs)
            self.inside = True
            try:
                return run(*args, **kwargs)
            finally:
                self.inside = False
                self.since += 1

        return counted

    def clocks(self, stages: tuple[str, ...]) -> list[dict[str, int]]:
        return [found for clock, found in self.by_clock.items() if clock.stages == stages]


def arm(
    monkeypatch: pytest.MonkeyPatch, highs: Highs, answers: list[tuple[int, str]], *, steady: bool,
    stage: str | None = None, step: str | None = None,
) -> dict[str, Any]:
    """`highs` answers `answers` from the first LP solved on the steady state's clock (`steady`) or on a pass's, of the
    stage `stage` or on D* at the step `step` (its witness's, test_dstar_retry.py's `arm_at`), on. Returns the place in
    `highs.runs` of that LP's first run (`first`)."""
    lp = economy_flow._Solver.lp
    stages = economy_flow.STEADY_STAGES if steady else economy_flow.STAGES
    seen: dict[str, Any] = {}

    def armed(solver: Any, program: Any, name: str, **options: Any) -> Any:
        witness = options.get("witness")
        if (
            not seen and solver.clock is not None and solver.clock.stages == stages
            and (stage is None or name == stage) and (step is None or (witness is not None and witness[0] == step))
        ):
            seen["first"] = len(highs.runs)
            highs.answers[:] = list(answers)
        return lp(solver, program, name, **options)

    monkeypatch.setattr(economy_flow._Solver, "lp", armed)
    return seen


# Contract tests 12 and 13: `by`, and the shapes of `timing_ms` and `steady_timing_ms` ------------------------------


def test_each_pass_says_how_its_vector_was_found_and_the_timing_holds_the_lps_of_every_stage(
    tmp_path, acmodel, monkeypatch
):
    """two_parts with b estimated from 3 to 4, 3 being pt.b's point value (test_pass_timing.py), with a clock that moves
    1 ms at each reading (`ticking`). The pass at `low` is the steady state's own vector, not solved again: `by`
    `steady_state`, `timing_ms` `{stages: null, lps, total}` with every one of the thirteen `lps` (the twelve stages and
    `certificate`) all 0. The pass at `high` is solved: `by` `solved`, `stages` the twelve, `lps` the thirteen, each of
    the five integers, `certificate` all 0, and its exact check is an LP (`lps.check.runs` 1). Every pass has `by`, and
    both kinds are in one fixture.

    prices.json has `steady_timing_ms`: its sixteen stages in the order the steady state runs them, `lps` of each, and
    `total`, at least the sum of the stages. The steady state solves S3 alone in `optimum` (one run), the dual's least
    cost in `dual`, the least losses in `losses`, the re-solves of limit.a and limit.b at plus and minus one grain in
    `rules`, the vector's check in `check`, and a forcing cost of each activity (both run in the optimum) in `forcing`.
    No member of the file is named `source` but the evaluator digest's."""
    ticking(monkeypatch, 1)
    folder = two_parts(tmp_path, {**ESTIMATE_B, "low": "3"})

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    prices = delivered(folder)
    low, high = passes(prices)
    assert all(BY in each for each in passes(prices))
    assert (low[BY], high[BY]) == ("steady_state", "solved")
    assert sorted(low[TIMING]) == sorted(high[TIMING]) == ["lps", "stages", "total"]
    assert low[TIMING]["stages"] is None and type(low[TIMING]["total"]) is int
    assert shaped(low[TIMING]["lps"], PASS_LPS)
    assert all(n == 0 for entry in low[TIMING]["lps"].values() for n in entry.values())
    assert list(high[TIMING]["stages"]) == sorted(economy_flow.STAGES)
    assert all(type(ms) is int for ms in high[TIMING]["stages"].values())
    assert shaped(high[TIMING]["lps"], PASS_LPS)
    assert all(n == 0 for n in high[TIMING]["lps"]["certificate"].values())
    assert high[TIMING]["lps"]["check"]["runs"] == 1

    steady = prices[STEADY_TIMING]
    assert sorted(steady) == ["lps", "stages", "total"]
    assert sorted(steady["stages"]) == sorted(economy_flow.STEADY_STAGES)
    assert all(type(ms) is int for ms in steady["stages"].values())
    assert steady["total"] >= sum(steady["stages"].values())
    assert shaped(steady["lps"], economy_flow.STEADY_STAGES)
    assert steady["lps"]["optimum"]["runs"] == 1
    for stage in ("dual", "losses", "rules", "check", "forcing"):
        assert steady["lps"][stage]["runs"] >= 1, stage
    assert named(prices, lambda key: key == "source") == ["/evaluator/source"]


# N1: the runs counted are HiGHS's runs, in the stage they ran in ------------------------------------------------------


@pytest.mark.parametrize(
    "fixture",
    [
        pytest.param(flips_fixture, id="flips"),
        pytest.param(tie_beside_estimate, id="tie"),
        pytest.param(lambda folder: priced(cyclic_fixture(folder)), id="cyclic"),
    ],
)
def test_each_stages_runs_are_highs_runs_of_that_stage(tmp_path, acmodel, monkeypatch, fixture):
    """Every HiGHS run of an LP is recorded here as it happens, and each end of a stage (`Recorded`). flips_fixture (two
    passes, pt.k at 1/2 and at 2), `tie_beside_estimate` (S1 and S2, and act.train's tie probes in the steady state) and
    cyclic_fixture (seven steps; r's hard cap: each end of its projection is an LP on D*, and its re-solves at ±grain
    start from S3's instance). Each stage's `runs` in `steady_timing_ms` and in each solved pass's `timing_ms`, in
    order, is the number of runs recorded in that stage on that clock; `certificate` is 0; every run fell in a stage."""
    folder = fixture(tmp_path)
    recorded = Recorded(monkeypatch)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    prices = delivered(folder)
    [steady] = recorded.clocks(economy_flow.STEADY_STAGES)
    assert {stage: entry["runs"] for stage, entry in prices[STEADY_TIMING]["lps"].items()} == steady
    solved_passes = [each for each in passes(prices) if each[BY] == "solved"]
    vectors = recorded.clocks(economy_flow.STAGES)
    assert len(solved_passes) == len(vectors) == len(passes(prices))
    for each, found in zip(solved_passes, vectors):
        assert {stage: entry["runs"] for stage, entry in each[TIMING]["lps"].items()} == {**found, "certificate": 0}
    assert recorded.since == 0
    if fixture is tie_beside_estimate:
        assert prices[STEADY_TIMING]["lps"]["ties"]["runs"] >= 1  # act.train's probes, in the steady state only


# N2: the two kinds of second try, counted apart where they ran -----------------------------------------------------


def test_the_two_kinds_of_second_try_are_counted_apart_where_they_ran(tmp_path, acmodel, monkeypatch):
    """Exported as they are, flips_fixture, cyclic_fixture and two_parts (b estimated from 2 to 4) count no second try.
    Then, as HiGHS answers (`Highs`, `arm`):
    - flips_fixture, the vector's check of the first pass (the last LP of that pass, on D*) answered infeasible once: it
      is solved again without presolve (the very next run); that pass's `check` has one run more and `again_infeasible`
      1, and every other count of every pass and of the steady state is the plain export's;
    - cyclic_fixture, the first end of r's projection in the steady state (an LP on D*: r has a hard cap) answered
      infeasible once: `steady_timing_ms.lps.projections` has one run more and `again_infeasible` 1, and nothing else
      counts a second try;
    - two_parts, S3 of the steady state answered with status 4 once: solved again without presolve, counted in
      `again_other` of `optimum`, which then holds S3's two runs (one in the plain export), and nothing else counts a
      second try.
    Each export succeeds with no warning: a second try HiGHS answers is no failure, and the file is the plain export's
    but for its timing."""
    flips = flips_fixture(tmp_path / "flips")
    cyclic = priced(cyclic_fixture(tmp_path / "cyclic"))
    two = two_parts(tmp_path / "two", ESTIMATE_B)
    plain = {}
    for folder in (flips, cyclic, two):
        status, doc = export(acmodel, folder)
        assert (status, doc["errors"], doc["warnings"]) == (0, [], []), folder
        plain[folder] = delivered(folder)
        assert second_tries(plain[folder]) == [], folder

    cases = [
        (flips, [INFEASIBLE], {"steady": False, "step": economy_flow.CHECK}, ("pass 0", "check", "again_infeasible")),
        (cyclic, [INFEASIBLE], {"steady": True, "step": economy_flow.PROJECTION},
         ("steady", "projections", "again_infeasible")),
        (two, [(4, "Solve error")], {"steady": True, "stage": "optimum"}, ("steady", "optimum", "again_other")),
    ]
    for folder, answers, where, (place, stage, kind) in cases:
        with monkeypatch.context() as patched:
            highs = Highs(patched)
            seen = arm(patched, highs, answers, **where)
            status, doc = export(acmodel, folder)

        assert (status, doc["errors"], doc["warnings"]) == (0, [], []), folder
        assert highs.forced == [seen["first"]] and highs.runs[seen["first"] + 1] is False, folder
        found, before = delivered(folder), plain[folder]
        assert second_tries(found) == [(place, stage, kind, 1)], folder
        assert bare(found) == bare(before), folder
        index = 0 if place == "steady" else int(place.split()[1]) + 1
        runs = counts(lps_of(found)[index])[stage][0]
        assert runs == counts(lps_of(before)[index])[stage][0] + 1, folder
        if folder == flips:  # the check is the pass's last LP: every other count is the plain export's
            expected = [counts(lps) for lps in lps_of(before)]
            expected[index][stage] = (runs, 1, 0)
            assert [counts(lps) for lps in lps_of(found)] == expected
        if folder == two:
            assert (counts(lps_of(before)[0])["optimum"], counts(lps_of(found)[0])["optimum"]) == ((1, 0, 0), (2, 0, 1))


# N3: the counts never read the clock ------------------------------------------------------------------------------


def test_the_counts_do_not_depend_on_the_clock_and_each_run_is_timed(tmp_path, acmodel, monkeypatch):
    """flips_fixture exported with a clock that moves 1 ms at each reading and with one that moves 3 ms (`ticking`).
    Each stage's `runs`, `again_infeasible` and `again_other` are the same in the two, in the steady state and in each
    pass; the times are not. The clock is read as each HiGHS run starts and ends, so on a clock of s ms a reading each
    stage's `highs_ms` is s times its `runs`."""
    folder = flips_fixture(tmp_path)
    found = {}
    for step in (1, 3):
        with monkeypatch.context() as patched:
            ticking(patched, step)
            status, doc = export(acmodel, folder)
        assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
        found[step] = delivered(folder)

    assert [counts(lps) for lps in lps_of(found[1])] == [counts(lps) for lps in lps_of(found[3])]
    assert found[1][STEADY_TIMING] != found[3][STEADY_TIMING]
    for step, prices in found.items():
        for lps in lps_of(prices):
            assert all(entry["highs_ms"] == step * entry["runs"] for entry in lps.values()), step


# N4: the timing is in two places, beside the value -------------------------------------------------------------------


def test_the_nodes_hold_their_counts_beside_their_value_and_the_cache_keeps_what_it_kept(tmp_path, monkeypatch):
    """flips_fixture's steady state and vector, computed now and as they were before METHOD-C C-1 (test_method_c.py's
    `as_before`): equal, and kept by the cache's encoding byte for byte alike. Computed now, each holds what HiGHS ran
    in each of its stages (`lps`: the steady state's sixteen, the vector's twelve) and its whole time (`total`), and the
    steady state its stages' times (`timing`); as before, neither the steady state nor the vector holds any of it but
    the vector's stage times. The economy's evaluator version is 12."""
    folder = flips_fixture(tmp_path)
    graph = graph_of(folder)
    found, vector = solved(graph, STEADY), solved(graph, VECTOR)
    with monkeypatch.context() as patched:
        as_before_c1(patched)
        earlier = graph_of(folder)
        was, was_vector = solved(earlier, STEADY), solved(earlier, VECTOR)

    assert not found.blocked and not vector.blocked
    assert found == was and vector == was_vector
    assert canonical(graph.encode(found)) == canonical(earlier.encode(was))
    assert canonical(graph.encode(vector)) == canonical(earlier.encode(was_vector))
    assert list(found.timing) == list(found.lps) == list(economy_flow.STEADY_STAGES)
    assert list(vector.timing) == list(vector.lps) == list(economy_flow.STAGES)
    assert type(found.total) is int and type(vector.total) is int
    assert (was.timing, was.lps, was.total, was_vector.lps, was_vector.total) == (None, None, None, None, None)
    assert EVALUATORS["economy"].version == 12


def test_the_timing_is_in_each_pass_and_in_steady_timing_ms_and_nowhere_else(tmp_path, acmodel, app_dir):
    """flips_fixture exported: a member named for timing is each pass's `timing_ms` and the top-level
    `steady_timing_ms`, and the counts (`lps`, `runs`, `again_*`) are under them only. No price's provenance file and
    not the shared tree under the state root holds any of them."""
    folder = flips_fixture(tmp_path)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    prices = delivered(folder)
    timed = [f"{P0}/{TIMING}", f"{P1}/{TIMING}", f"/{STEADY_TIMING}"]
    assert named(prices, lambda key: "timing" in key) == timed
    counted = named(prices, lambda key: key in ("lps", "runs", *AGAIN))
    assert counted and all(any(where.startswith(place + "/") for place in timed) for where in counted)
    for entry in prices["prices"]:
        written = json.loads((base(folder) / entry["provenance"]).read_text(encoding="utf-8"))
        assert named(written, lambda key: "timing" in key or key in ("lps", "runs", *AGAIN)) == [], entry["pool"]
    tree = json.loads(kept(app_dir, prices["shared"][STEADY_ROOT]["sha256"]).read_text(encoding="utf-8"))
    assert named(tree, lambda key: "timing" in key or key in ("lps", "runs", *AGAIN)) == []


# N5: a file with or without the steady state's timing is read ---------------------------------------------------------


def test_verify_prices_and_bindings_check_read_a_file_of_c2a_and_one_written_before_it(tmp_path, acmodel):
    """two_parts with b estimated from 2 to 4, exported; `verify prices` and `bindings check` read its prices.json,
    which holds `steady_timing_ms` and each pass's `by` and `lps`. Then the file is made as one written before C-2a:
    each pass without `by`, its `timing_ms` `{source, stages, total}` with no `lps`, and no `steady_timing_ms`. Both
    commands read that one too, as before: only an assessment taken over is checked for the shape of its passes
    (`--reuse-assessment`), so the resource repositories' files verify until they are exported again."""
    folder = two_parts(tmp_path, ESTIMATE_B)
    write(base(folder) / "bindings.json", {"schema_version": "acmodel.bindings.v1", "bindings": [], "fixed": []})
    write(folder / "schedule.json", {
        "schema_version": "actingcommand.schedule.v1",
        "rules": [{"id": "r.a", "package_id": "p", "produces": [{"pool_id": "res.a", "amount": 2}]}],
        "pools": [{"id": "res.a"}],
    })
    state = ("--state", str(folder / "state.json"))
    status, doc = export(acmodel, folder)
    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert STEADY_TIMING in delivered(folder) and all(BY in each for each in passes(delivered(folder)))

    def read() -> tuple[Any, Any]:
        status, verified = acmodel("verify", "prices", str(base(folder)), *state)
        assert (status, verified["errors"], verified["warnings"]) == (0, [], [])
        status, checked = acmodel("bindings", "check", str(base(folder)), *state)
        assert (status, checked["errors"], checked["warnings"]) == (0, [], [])
        assert checked["data"]["coverage"]["current"] is True
        return verified["data"]["shared"], checked["data"]["coverage"]

    def written_before(prices: dict[str, Any]) -> None:
        prices.pop(STEADY_TIMING)
        for each in passes(prices):
            each.pop(BY)
            timing = each[TIMING]
            each[TIMING] = {"source": "solved", "stages": timing["stages"], "total": timing["total"]}

    now = read()
    edit(base(folder) / "out" / "prices.json", written_before)
    old = delivered(folder)
    assert STEADY_TIMING not in old and all(BY not in each and "lps" not in each[TIMING] for each in passes(old))

    assert read() == now


# N6 and contract test 16: an assessment taken over -----------------------------------------------------------------


def test_an_export_that_takes_the_assessment_over_records_its_own_steady_states_timing(tmp_path, acmodel, monkeypatch):
    """flips_fixture exported with a clock of 3 ms a reading and its out folder kept aside; then exported in full with a
    clock of 1 ms, and once more taking the kept assessment over, with the same clock. The groups taken over are the
    kept ones, each pass's `by` and `timing_ms` as they are; `steady_timing_ms` is this run's own: on the same clock it
    is the full export's (the same steady state, read as often), not the kept one, whose counts are the same. Without
    `reused_assessment`, each pass's `by` and `timing_ms` and `steady_timing_ms`, the file is the full export's byte for
    byte."""
    folder = flips_fixture(tmp_path)
    out, kept_out = base(folder) / "out", tmp_path / "earlier"
    with monkeypatch.context() as patched:
        ticking(patched, 3)
        export(acmodel, folder)
    shutil.copytree(out, kept_out)
    with monkeypatch.context() as patched:
        ticking(patched, 1)
        export(acmodel, folder)
        full = json.loads((out / "prices.json").read_bytes())
        status, doc = reuse(acmodel, folder, kept_out)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    earlier = json.loads((kept_out / "prices.json").read_bytes())
    taken = json.loads((out / "prices.json").read_bytes())
    assert taken["impact"] == earlier["impact"]
    assert taken[STEADY_TIMING] == full[STEADY_TIMING] != earlier[STEADY_TIMING]
    assert counts(taken[STEADY_TIMING]["lps"]) == counts(earlier[STEADY_TIMING]["lps"])
    assert policy.delivered(bare(taken)) == policy.delivered(bare(full))


def _pass(doc: dict[str, Any], index: int) -> dict[str, Any]:
    return doc["impact"][0]["passes"][index]


def _without_by(doc: dict[str, Any]) -> None:
    _pass(doc, 0).pop(BY)


def _other_by(doc: dict[str, Any]) -> None:
    _pass(doc, 0)[BY] = "cache"


def _with_source(doc: dict[str, Any]) -> None:
    _pass(doc, 0)[TIMING]["source"] = "solved"


def _without_lps(doc: dict[str, Any]) -> None:
    _pass(doc, 0)[TIMING].pop("lps")


def _lps_short(doc: dict[str, Any]) -> None:
    _pass(doc, 0)[TIMING]["lps"].pop("certificate")


def _not_an_integer(doc: dict[str, Any]) -> None:
    _pass(doc, 0)[TIMING]["lps"]["check"]["runs"] = "1"


def _refused_by_in_one(doc: dict[str, Any]) -> None:
    _pass(doc, 0)["refused_by"] = "c1"


def _refused_by_unknown(doc: dict[str, Any]) -> None:
    _pass(doc, 0)["refused_by"], _pass(doc, 1)["refused_by"] = "c9", "c1"


def _refused_by_null_when_solved(doc: dict[str, Any]) -> None:
    _pass(doc, 0)["refused_by"], _pass(doc, 1)["refused_by"] = None, "c2"


def _refused_by_when_certified(doc: dict[str, Any]) -> None:
    _pass(doc, 0).update(by="certified", refused_by="c1")
    _pass(doc, 0)[TIMING]["stages"] = None
    _pass(doc, 1)["refused_by"] = "c2"


def _refused_by_when_steady_state(doc: dict[str, Any]) -> None:
    _pass(doc, 0).update(by="steady_state", refused_by="program")
    _pass(doc, 1)["refused_by"] = "lp_unverified"


@pytest.mark.parametrize(
    ("change", "reason", "ref", "values"),
    [
        pytest.param(_without_by, "member_missing", f"{P0}/by", {"key": "by"}, id="no_by"),
        pytest.param(_other_by, "value_invalid", f"{P0}/by",
                     {"expected": ["solved", "certified", "steady_state"], "literal": "cache"},
                     id="by_not_of_the_three"),
        pytest.param(_with_source, "member_unknown", f"{P0}/{TIMING}/source", {"key": "source"}, id="source"),
        pytest.param(_without_lps, "member_missing", f"{P0}/{TIMING}/lps", {"key": "lps"}, id="no_lps"),
        pytest.param(_lps_short, "member_missing", f"{P0}/{TIMING}/lps/certificate", {"key": "certificate"},
                     id="lps_short_of_a_key"),
        pytest.param(_not_an_integer, "type_invalid", f"{P0}/{TIMING}/lps/check/runs", {"expected": ["integer"]},
                     id="not_an_integer"),
        pytest.param(_refused_by_in_one, "member_missing", f"{P1}/refused_by", {"key": "refused_by"},
                     id="refused_by_in_some_passes"),
        pytest.param(_refused_by_unknown, "value_invalid", f"{P0}/refused_by", {"expected": SIX, "literal": "c9"},
                     id="refused_by_not_of_the_six"),
        pytest.param(_refused_by_null_when_solved, "value_invalid", f"{P0}/refused_by",
                     {"expected": SIX, "literal": "null"}, id="refused_by_null_for_solved"),
        pytest.param(_refused_by_when_certified, "value_invalid", f"{P0}/refused_by",
                     {"expected": ["null"], "literal": "c1"}, id="refused_by_for_certified"),
        pytest.param(_refused_by_when_steady_state, "value_invalid", f"{P0}/refused_by",
                     {"expected": ["null"], "literal": "program"}, id="refused_by_for_steady_state"),
    ],
)
def test_a_pass_taken_over_must_have_its_by_and_a_timing_of_c2a(tmp_path, acmodel, change, reason, ref, values):
    """flips_fixture exported (two solved passes) and its out folder kept aside, then one pass of the kept prices.json
    made otherwise than `export prices` writes it (contract test 16; N6): a pass without `by` or with another word, its
    `timing_ms` with `source` (as PASS-TIMING wrote it), without `lps`, its `lps` short of a key or holding a count
    that is no integer; a `refused_by` in one pass only, not of the six conditions, null for a solved pass, or a
    condition for a `certified` or `steady_state` pass. Taking that assessment over is `model_file_invalid` at that
    pass (exit 1), one error, and nothing is assessed or written."""
    folder = flips_fixture(tmp_path)
    out, kept_out = base(folder) / "out", tmp_path / "earlier"
    export(acmodel, folder)
    shutil.copytree(out, kept_out)
    edit(kept_out / "prices.json", change)
    before = snapshot(out), snapshot(tmp_path / "localappdata")

    status, doc = reuse(acmodel, folder, kept_out)

    assert status == 1
    assert doc["errors"] == [{
        "code": "model_file_invalid",
        "detail": {"reason": reason, "path": str(kept_out / "prices.json"), "format": "acmodel.prices.v2", **values},
        "ref": ref,
    }]
    assert (snapshot(out), snapshot(tmp_path / "localappdata")) == before


def test_a_pass_taken_over_may_be_certified_and_every_pass_may_say_what_refused_it(tmp_path, acmodel):
    """The kept prices.json of flips_fixture with its first pass `certified` (`stages` null, `refused_by` null) and its
    second solved, refused by `c2`: a `by` of the three, a `timing_ms` of C-2a and a `refused_by` in every pass, each as
    the contract has it. It is taken over as it is."""
    folder = flips_fixture(tmp_path)
    out, kept_out = base(folder) / "out", tmp_path / "earlier"
    export(acmodel, folder)
    shutil.copytree(out, kept_out)

    def certified(doc: dict[str, Any]) -> None:
        _pass(doc, 0).update(by="certified", refused_by=None)
        _pass(doc, 0)[TIMING]["stages"] = None
        _pass(doc, 1)["refused_by"] = "c2"

    edit(kept_out / "prices.json", certified)

    status, doc = reuse(acmodel, folder, kept_out)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    taken = json.loads((out / "prices.json").read_bytes())
    assert taken["impact"] == json.loads((kept_out / "prices.json").read_bytes())["impact"]
    assert [(each[BY], each["refused_by"]) for each in passes(taken)] == [("certified", None), ("solved", "c2")]


def test_the_reader_checks_the_names_the_assessment_writes():
    """What `readers.check_assessment` takes for a pass is what `core.impact` and `core.flow` write: the three words of
    `by`, the vector's stages and the certificate's member of `lps`, and the five counts of each."""
    assert readers.PASS_STAGES == economy_flow.STAGES
    assert readers.PASS_LPS == impact.PASS_LPS == PASS_LPS
    assert readers.LP_COUNTS == economy_flow.LP_COUNTS
    assert set(readers.PASS_BY) == {impact.SOLVED, impact.CERTIFIED, impact.STEADY_STATE}
    assert list(readers.REFUSED_BY) == SIX
