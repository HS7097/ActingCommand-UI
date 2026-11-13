"""#430: an LP on D* that HiGHS finds infeasible. Every LP on D* (a projection's end, a pool's interval, each step of the
selection, each stage and probe of the max-min, the vector's exact check) has a solution by construction: an exact point
the solve already holds meets it. So HiGHS's infeasible there is solved again from scratch without presolve, and when HiGHS
says it again, that point is checked exactly against the LP's every bound and row: it meets them all,
`lp_numeric_check_failed`; it breaks one, `lp_invariant_broken`, at that row. The model's own programs (S3 and the rest)
take HiGHS's infeasible as their answer, as before (contracts/formats.md, "Exactness and failures", "Other failures").

HiGHS's answers are given here as a test needs them (`Highs`, `arm_at`): each run is HiGHS's own, and only the status and
the text it answers are replaced, at the first LP on D* of one step. The fixtures are the hand-solvable neutral models of
test_economy.py, test_price_vector.py and test_prices.py, and two of their own (`withheld_fixture`, `no_point_fixture`);
each docstring says by hand what the fixture gives. The methods of the solver as they were before this slice are kept
here, line for line (the module's names qualified), to compare the two where no LP on D* fails. CI runs these tests on
Windows and on Ubuntu."""

from __future__ import annotations

import json
from dataclasses import replace
from fractions import Fraction
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel.codes import EXIT_STATUS
from acmodel.core import flow as economy_flow
from acmodel.export import policy
from test_economy import (  # noqa: F401 (the fixture `economy`)
    CODES,
    DAY,
    activity,
    cap,
    cyclic_fixture,
    economy,
    flow,
    limit,
    model,
    model_path,
    regen,
    resource,
)
from test_method_c import kept_nodes
from test_price_reuse import snapshot
from test_price_vector import ex2
from test_prices import acmodel, base, export, flips_fixture, untimed_doc  # noqa: F401 (the fixture `acmodel`)

STEADY_ONLY = "--steady-only"
INFEASIBLE = (2, "Infeasible")  # scipy's status of HiGHS's kInfeasible, and HiGHS's text of it


def withheld_fixture(folder: Path) -> Path:
    """test_economy.py's regen stop that can hold no more: r's regen (10 a day) stops at its natural cap (reference 50)
    and its only user, act.p (a pending flow), is left out, so nothing uses r; act.a brings 2 v a day for nothing, at most
    3 times. By hand (test_economy.py works it out): V0 = 6; one more r is infeasible, so r's interval on D* has no low end
    and its high end is 0. r is in no step of the selection and not in K, so the selection withholds it (`no_room`) and
    fixes it last, at its greatest value on the face left, 0 (`one_less`), after its least, unbounded, is solved."""
    unknown = {
        "id": "unk.p", "kind": "interval", "question": "fixture", "interval": {"low": "1", "high": "3"},
        "status": "pending", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }
    act_p = {**flow("flow.p", "act.p", consumes={"res.r": "10"}, produces={"res.v": "unknown:unk.p"}), "status": "pending"}
    return model(
        folder, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.a"), activity("act.p")],
        [
            regen("regen.r", "res.r", "10", "1d", **{"while": "amount:res.r < read:fact.r_cap"}),
            cap("cap.r.natural", "res.r", "soft_read", {"block_income_from": [], "other_income": "notice_only"},
                reading="fact.r_cap", reference="50"),
            flow("flow.a", "act.a", produces={"res.v": "2"}),
            limit("limit.a", "act.a", "3", "day"),
            {**act_p, "unknowns": ["unk.p"]},
            limit("limit.p", "act.p", "1", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.r_cap": 30},
        [unknown],
    )


def no_point_fixture(folder: Path) -> Path:
    """withheld_fixture's r with nothing that brings it in, takes it or loses it: its regen brings 0 a day and stops at
    its natural cap (reference 50), and no flow uses r; act.a brings 2 v a day for nothing, at most 3 times. By hand: V0
    = 6, v is worth exactly 1. r's regen stops at a cap, so its stock ends the cycle exactly where it began, and the
    regen brings 0, so r loses at most 0: one more r has nowhere to go and one less nowhere to come from, both
    infeasible. So r's interval on D* has neither end; r is in no step of the selection and not in K, and the selection
    withholds it (`no_room`): its least and its greatest on the face left are both unbounded, so it is fixed at 0
    (`zero`), and no LP's solution reached that row: the solve holds no point for the next LP on D*, the vector's
    exact check (`flow._Solver.joint`, `fix`)."""
    return model(
        folder, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.a")],
        [
            regen("regen.r", "res.r", "0", "1d", **{"while": "amount:res.r < read:fact.r_cap"}),
            cap("cap.r.natural", "res.r", "soft_read", {"block_income_from": [], "other_income": "notice_only"},
                reading="fact.r_cap", reference="50"),
            flow("flow.a", "act.a", produces={"res.v": "2"}),
            limit("limit.a", "act.a", "3", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.r_cap": 30},
    )


def contradictory_fixture(folder: Path) -> Path:
    """test_economy.py's contradictory rules: r drains by 10 a day, and its only income is act.p, 5 r an execution, at
    most once a day; a steady state would need two executions a day, so the limit and the drain contradict each other
    (`lp_infeasible`, rules limit.p and regen.drain), and nothing else does."""
    return model(
        folder, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), resource("res.w", "fact.w"), activity("act.p"),
         activity("act.q")],
        [
            regen("regen.drain", "res.r", "-10", "1d"),
            flow("flow.p", "act.p", produces={"res.r": "5"}),
            limit("limit.p", "act.p", "1", "day"),
            regen("regen.w", "res.w", "10", "1d"),
            flow("flow.q", "act.q", consumes={"res.w": "5"}, produces={"res.v": "1"}),
            limit("limit.q", "act.q", "2", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.w": 0},
    )


# HiGHS's answers ----------------------------------------------------------------------------------------------------


class Highs:
    """HiGHS's runs of the LPs (`flow._highs`, `flow._highs_again`), each recorded by whether it ran with presolve (a
    warm start never does: None). While `answers` holds any, each next run answers the first of them instead (a status
    and HiGHS's text; the run itself is HiGHS's own, so its instance is held as ever), and its place in `runs` is
    recorded in `forced`."""

    def __init__(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self.runs: list[bool | None] = []
        self.answers: list[tuple[int, str]] = []
        self.forced: list[int] = []
        cold, warm = economy_flow._highs, economy_flow._highs_again

        def answer(run: Any, presolve: bool | None) -> Any:
            self.runs.append(presolve)
            if not self.answers:
                return run
            status, text = self.answers.pop(0)
            self.forced.append(len(self.runs) - 1)
            return replace(run, status=status, message=text, x=None, duals=None, statuses=None)

        monkeypatch.setattr(economy_flow, "_highs", lambda arrays, presolve: answer(cold(arrays, presolve), presolve))
        monkeypatch.setattr(
            economy_flow, "_highs_again", lambda solver, arrays, added=(): answer(warm(solver, arrays, added), None)
        )


def arm_at(
    monkeypatch: pytest.MonkeyPatch, highs: Highs, step: str, answers: list[tuple[int, str]],
    spoil: Callable[[Any, dict], dict] | None = None,
) -> dict[str, Any]:
    """`highs` answers `answers` from the first LP on D* of `step` on (the first `flow._Solver.lp` given a witness of that
    step). With `spoil`, that LP's witness point is replaced by what `spoil` makes of it. Returns what that LP was given:
    its `program`, its `point`, and the place in `highs.runs` of its first run (`first`)."""
    lp = economy_flow._Solver.lp
    seen: dict[str, Any] = {}

    def armed(solver: Any, program: Any, stage: str, **options: Any) -> Any:
        witness = options.get("witness")
        if not seen and witness is not None and witness[0] == step:
            point = witness[1] if spoil is None else spoil(program, witness[1])
            options["witness"] = (step, point)
            seen.update(program=program, point=point, first=len(highs.runs))
            highs.answers[:] = list(answers)
        return lp(solver, program, stage, **options)

    monkeypatch.setattr(economy_flow._Solver, "lp", armed)
    return seen


def spoilt(program: Any, point: dict) -> dict:
    """`point` with one variable 1 higher: the first variable without an upper bound of the first equality row (in
    canonical order) that holds one. No bound breaks, and that row does (its coefficient is not 0)."""
    for row in sorted(program.rows, key=lambda row: row.key):
        if row.sense != "eq":
            continue
        for key in sorted(row.coefficients):
            if program.variables[key][1] is None:
                return {**point, key: point[key] + 1}
    raise AssertionError("no equality row holds a variable without an upper bound")


def run(economy: Any, folder: Path, root: Path) -> tuple[int, dict[str, Any]]:
    """`acmodel economy --steady-only` with a state root of its own, so that nothing comes from another run's cache."""
    return economy(folder, STEADY_ONLY, paths={"state_root": str(root)})


def steady_lps(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, Any]]:
    """What HiGHS ran in each stage of each steady state computed (#430 C-2a, `flow.Economy.lps`), in order."""
    found: list[dict[str, Any]] = []
    compute = economy_flow.compute

    def recording(graph: Any, solve: str) -> Any:
        result = compute(graph, solve)
        if solve == economy_flow.STEADY and result.lps is not None:
            found.append(result.lps)
        return result

    monkeypatch.setattr(economy_flow, "compute", recording)
    return found


# The steps of the LPs on D* and the fixtures that solve one: ex2 has two pools' intervals, a max-min of two stages and
# the vector's check; withheld_fixture a pool the selection withholds; cyclic_fixture a pool with a hard cap, r, whose
# projection's ends S3's basis never proves (`_Solver.chords`, `_pinned`), so each is an LP on D* (#430 C-2a).
SITES = [
    pytest.param(ex2, economy_flow.INTERVAL, id="interval"),
    pytest.param(ex2, economy_flow.MAX_MIN, id="max_min"),
    pytest.param(ex2, economy_flow.CHECK, id="check"),
    pytest.param(withheld_fixture, economy_flow.SELECTION, id="selection"),
    pytest.param(cyclic_fixture, economy_flow.PROJECTION, id="projection"),
]
# The stage of the steady state's timing (#430 C-2a, `flow.STEADY_STAGES`) that each step's LPs on D* run in.
STAGE_OF = {
    economy_flow.PROJECTION: "projections", economy_flow.INTERVAL: "intervals", economy_flow.MAX_MIN: "max_min",
    economy_flow.CHECK: "check", economy_flow.SELECTION: "selection",
}


# 1. Infeasible once: solved again without presolve, and nothing changes ---------------------------------------------


@pytest.mark.parametrize(("fixture", "step"), SITES)
def test_an_lp_on_d_star_highs_finds_infeasible_once_is_solved_again_without_presolve_and_nothing_changes(
    tmp_path, economy, monkeypatch, fixture, step
):
    """EX2 (test_price_vector.py: the vector a 3/7, b 3/7, c 1/7, v 1, in two max-min stages), withheld_fixture (r
    withheld at 0) and cyclic_fixture (r's hard cap: the ends of its projection, #430 C-2a): HiGHS answers the first LP
    on D* of the step infeasible. That LP is solved a second time, from scratch and without presolve (`_highs` with
    presolve off, the very next run), which HiGHS solves; the steady state is then the one computed with no such answer,
    coordinate by coordinate, every judgement and warning alike. The second try is counted, once, in the stage the
    step's LPs run in (#430 C-2a: `again_infeasible`), and no other second try is."""
    folder = fixture(tmp_path / "model")
    status, clean = run(economy, folder, tmp_path / "clean")
    with monkeypatch.context() as patched:
        highs = Highs(patched)
        seen = arm_at(patched, highs, step, [INFEASIBLE])
        counted = steady_lps(patched)
        status_again, again = run(economy, folder, tmp_path / "again")

    assert (status, clean["errors"]) == (0, [])
    assert (status_again, again["errors"]) == (0, [])
    assert highs.forced == [seen["first"]]
    assert highs.runs[seen["first"] + 1] is False
    assert again["data"]["steady"] == clean["data"]["steady"]
    assert again["warnings"] == clean["warnings"]
    [lps] = counted
    tries = [(stage, entry["again_infeasible"], entry["again_other"]) for stage, entry in lps.items()]
    assert [found for found in tries if found[1:] != (0, 0)] == [(STAGE_OF[step], 1, 0)]


# 2. and 3. Infeasible twice: the point tells a numerical failure from an LP built wrong -------------------------------


@pytest.mark.parametrize(("fixture", "step"), SITES)
def test_an_lp_on_d_star_highs_finds_infeasible_twice_that_its_point_meets_is_lp_numeric_check_failed(
    tmp_path, economy, monkeypatch, fixture, step
):
    """HiGHS answers the first LP on D* of the step infeasible on both its tries (the second without presolve). The exact
    point the solve holds for it meets every bound and row of it (checked here too), so the answer is HiGHS's numerical
    failure: `lp_numeric_check_failed`, stage dual, the step, at HiGHS's status (constraint solver_status, found 2, bound
    =0), with HiGHS's text and the two tries. It blocks the steady state."""
    folder = fixture(tmp_path)
    with monkeypatch.context() as patched:
        highs = Highs(patched)
        seen = arm_at(patched, highs, step, [INFEASIBLE, INFEASIBLE])
        status, doc = run(economy, folder, tmp_path / "state")

    assert economy_flow._unmet(seen["program"], seen["point"]) is None
    assert highs.forced == [seen["first"], seen["first"] + 1]
    assert highs.runs[seen["first"] + 1] is False
    assert status == CODES["lp_numeric_check_failed"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "lp_numeric_check_failed",
        "detail": {
            "path": model_path(folder), "solve": "steady", "stage": "dual", "step": step, "constraint": "solver_status",
            "found": "2", "bound": "=0", "raw_text": "Infeasible", "tries": 2,
        },
        "needed_by": ["economy:steady"],
    }]
    assert doc["data"]["steady"] == {"status": "blocked"}


@pytest.mark.parametrize(("fixture", "step"), SITES)
def test_an_lp_on_d_star_highs_finds_infeasible_twice_that_its_point_breaks_is_lp_invariant_broken(
    tmp_path, economy, monkeypatch, fixture, step
):
    """As above, with the point the solve holds for that LP spoilt (`spoilt`: one variable without an upper bound 1
    higher, so that an equality row it is in breaks and no bound does). The point no longer meets the LP, so the LP is not
    the one the solve meant to build: `lp_invariant_broken`, stage dual, the step, at the first bound or row the point
    breaks (a row, as `_violation` finds it), with HiGHS's text and the two tries. It blocks the steady state."""
    folder = fixture(tmp_path)
    with monkeypatch.context() as patched:
        highs = Highs(patched)
        seen = arm_at(patched, highs, step, [INFEASIBLE, INFEASIBLE], spoil=spoilt)
        status, doc = run(economy, folder, tmp_path / "state")

    broken = economy_flow._violation(seen["program"], seen["point"])
    assert broken is not None
    assert broken["constraint"] in {economy_flow._token(row.key) for row in seen["program"].rows}
    assert highs.forced == [seen["first"], seen["first"] + 1]
    assert status == CODES["lp_invariant_broken"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "lp_invariant_broken",
        "detail": {
            "path": model_path(folder), "solve": "steady", "stage": "dual", "step": step, **broken,
            "raw_text": "Infeasible", "tries": 2,
        },
        "needed_by": ["economy:steady"],
    }]
    assert doc["data"]["steady"] == {"status": "blocked"}


def test_an_lp_on_d_star_with_no_point_in_hand_that_highs_finds_infeasible_twice_is_lp_solve_failed(
    tmp_path, economy, monkeypatch
):
    """`no_point_fixture`: r is withheld (`no_room`) and fixed at 0 unbounded both ways (`zero`), so the solve holds no
    point for the next LP on D*, the vector's exact check. HiGHS answers that check infeasible on both its tries (the
    second without presolve): with no point to tell a numerical failure from an LP built wrong, it is `lp_solve_failed`,
    stage dual, reason solver_status, status 2, with the step (check), HiGHS's text and the two tries. It blocks the
    steady state."""
    folder = no_point_fixture(tmp_path / "model")
    status, clean = run(economy, folder, tmp_path / "clean")
    with monkeypatch.context() as patched:
        highs = Highs(patched)
        seen = arm_at(patched, highs, economy_flow.CHECK, [INFEASIBLE, INFEASIBLE])
        status_again, doc = run(economy, folder, tmp_path / "again")

    assert (status, clean["errors"]) == (0, [])
    withheld = clean["data"]["steady"]["joint"]["prices"]["res.r"]
    assert (withheld["value"], withheld["by"], withheld["withheld"]["reason"]) == ("0", "zero", "no_room")
    assert clean["data"]["steady"]["joint"]["prices"]["res.v"]["value"] == "1"
    assert seen["point"] is None
    assert highs.forced == [seen["first"], seen["first"] + 1]
    assert highs.runs[seen["first"] + 1] is False
    assert status_again == CODES["lp_solve_failed"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "lp_solve_failed",
        "detail": {
            "path": model_path(folder), "solve": "steady", "stage": "dual", "reason": "solver_status", "status": 2,
            "step": "check", "raw_text": "Infeasible", "tries": 2,
        },
        "needed_by": ["economy:steady"],
    }]
    assert doc["data"]["steady"] == {"status": "blocked"}


# 4. The model's own program: infeasible is its answer, with no second try -------------------------------------------


def test_the_models_own_infeasible_program_is_lp_infeasible_and_highs_runs_it_once(tmp_path, economy, monkeypatch):
    """contradictory_fixture, with every answer of HiGHS that is not an optimum given as infeasible (status 2), so that
    no other status takes the second try that "Other failures" gives it. S3 is the first LP: HiGHS runs it once, with
    presolve, and its infeasible is the answer: `lp_infeasible`, the limit and the drain, as before. No LP of the conflict
    search after it has a witness (none is on D*), and HiGHS runs each once, with presolve."""
    folder = contradictory_fixture(tmp_path)
    calls: list[tuple[str, Any, list]] = []  # each LP: its stage, its witness, and HiGHS's runs of it (with presolve?)
    lp, cold = economy_flow._Solver.lp, economy_flow._highs

    def recorded(solver: Any, program: Any, stage: str, **options: Any) -> Any:
        calls.append((stage, options.get("witness"), []))
        return lp(solver, program, stage, **options)

    def infeasible(arrays: Any, presolve: bool) -> Any:
        calls[-1][2].append(presolve)
        found = cold(arrays, presolve)
        return found if found.status == 0 else replace(found, status=2, message="Infeasible")

    monkeypatch.setattr(economy_flow._Solver, "lp", recorded)
    monkeypatch.setattr(economy_flow, "_highs", infeasible)
    status, doc = economy(folder, STEADY_ONLY)

    assert status == 1
    assert doc["errors"] == [{
        "code": "lp_infeasible",
        "detail": {"path": model_path(folder), "solve": "steady", "rules": ["limit.p", "regen.drain"]},
        "needed_by": ["economy:steady"],
    }]
    assert calls[0] == ("optimum", None, [True])
    assert len(calls) > 1
    assert all(witness is None and runs == [True] for _, witness, runs in calls)


# 5. lp_solve_failed names the step and HiGHS's text ------------------------------------------------------------------


def test_the_duals_least_cost_infeasible_is_lp_solve_failed_with_its_step_and_highs_text(tmp_path, economy, monkeypatch):
    """EX2, with HiGHS answering the dual's least cost infeasible (its primal has an optimum, so it has one): no second
    try (it is no LP on D*), and `lp_solve_failed`, stage dual, status 2, names its step (least_cost) and HiGHS's text."""
    folder = ex2(tmp_path)
    duality = economy_flow._Solver.duality
    with monkeypatch.context() as patched:
        highs = Highs(patched)
        first: list[int] = []

        def armed(solver: Any, dual: Any, optimum: Any) -> Any:
            first.append(len(highs.runs))
            highs.answers[:] = [INFEASIBLE]
            return duality(solver, dual, optimum)

        patched.setattr(economy_flow._Solver, "duality", armed)
        status, doc = run(economy, folder, tmp_path / "state")

    assert highs.forced == first and len(highs.runs) == first[0] + 1
    assert status == CODES["lp_solve_failed"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "lp_solve_failed",
        "detail": {
            "path": model_path(folder), "solve": "steady", "stage": "dual", "reason": "solver_status", "status": 2,
            "step": "least_cost", "raw_text": "Infeasible",
        },
        "needed_by": ["economy:steady"],
    }]


def test_an_lp_on_d_star_that_ends_with_another_status_is_lp_solve_failed_with_its_step_text_and_tries(
    tmp_path, economy, monkeypatch
):
    """EX2, with HiGHS answering the vector's check infeasible, and then, on its second try without presolve, another
    status (4, with its own text): `lp_solve_failed`, stage dual, reason solver_status, status 4, with the step (check),
    HiGHS's text of that answer and the two tries."""
    folder = ex2(tmp_path)
    with monkeypatch.context() as patched:
        highs = Highs(patched)
        seen = arm_at(patched, highs, economy_flow.CHECK, [INFEASIBLE, (4, "Solve error")])
        status, doc = run(economy, folder, tmp_path / "state")

    assert highs.forced == [seen["first"], seen["first"] + 1]
    assert status == CODES["lp_solve_failed"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "lp_solve_failed",
        "detail": {
            "path": model_path(folder), "solve": "steady", "stage": "dual", "reason": "solver_status", "status": 4,
            "raw_text": "Solve error", "step": "check", "tries": 2,
        },
        "needed_by": ["economy:steady"],
    }]


# 6. Where no LP on D* fails, everything is what it was ----------------------------------------------------------------


def before_lp(
    self: Any, program: Any, stage: str, *, check: bool = True, unbounded: bool = False, warm: bool = False,
    keep: bool = False, seed: Any = None,
) -> Any:
    """`flow._Solver.lp` as it was before this slice, line for line (the module's names qualified)."""
    result = None
    held = self.warm.pop(id(program.variables), None) if warm else None
    added = None if held is None else economy_flow._added(held[1], program.rows)
    if added is not None:
        arrays = economy_flow._extended(held[2], program, added)
        result = economy_flow._highs_again(held[3], arrays, added)
        if result.solver is not None:  # it ran: the instance holds these rows now (else it is dropped)
            self.warm[id(program.variables)] = (program.variables, program.rows, arrays, held[3])
    else:
        arrays = economy_flow._arrays(program)
        result = economy_flow._highs(arrays, True) if seed is None else economy_flow._highs_seeded(arrays, seed)
        if warm and result.solver is not None:
            self.warm[id(program.variables)] = (program.variables, program.rows, arrays, result.solver)
            while len(self.warm) > economy_flow.WARM_FACES:  # the face whose LPs came longest ago
                self.warm.pop(next(iter(self.warm)))
    if result.status not in (0, 2, 3):  # "unbounded or infeasible" from presolve, or numerical trouble
        result = economy_flow._highs(arrays, False)
    if result.status == 2:
        return economy_flow._Solution("infeasible")
    if result.status == 3 and unbounded:
        return economy_flow._Solution("unbounded")
    if result.status == 3:
        raise self.failure(economy_flow.LP_SOLVE_FAILED, stage=stage, reason="unbounded", status=3)
    if result.status != 0:
        raise self.failure(
            economy_flow.LP_SOLVE_FAILED, stage=stage, reason="solver_status", status=int(result.status),
            raw_text=result.message,
        )
    if not check:
        return economy_flow._Solution("optimal")
    x, found = self.vertex(program, arrays, result, stage)
    duals = list(result.duals)
    le = economy_flow._marginals(arrays.le, duals[: len(arrays.le)])
    eq = economy_flow._marginals(arrays.eq, duals[len(arrays.le):])
    certified = isinstance(found, economy_flow.exact.Vertex)
    return economy_flow._Solution(
        "optimal", x, self.objective(program, x), eq, le, certified=certified,
        refusal=None if certified else (found.reason if found is not None else economy_flow.NO_BASIS),
        pivots=0 if found is None else found.pivots,
        basis=(self.last_problem, found, arrays) if certified else None,
        solver=result.solver if keep else None,
    )


def before_extreme(self: Any, variables: Any, rows: Any, function: Any, sense: int) -> Any:
    """`flow._Solver.extreme` as it was before this slice, line for line (the module's names qualified)."""
    program = economy_flow._Program(
        variables, tuple(rows), economy_flow._nonzero({key: sense * value for key, value in function.items()})
    )
    found = self.lp(program, economy_flow.DUAL, unbounded=True, warm=True)
    if found.status == "unbounded":
        return None
    if found.status != "optimal":
        raise self.failure(economy_flow.LP_SOLVE_FAILED, stage=economy_flow.DUAL, reason="solver_status", status=2)
    return sense * found.value


def before_ends(self: Any, variables: Any, rows: Any, function: Any) -> Any:
    """`flow._Solver.ends` as it was before this slice."""
    return self.extreme(variables, rows, function, -1), self.extreme(variables, rows, function, 1)


def before_duality(self: Any, dual: Any, optimum: Fraction) -> Any:
    """`flow._Solver.duality` as it was before this slice, line for line (the module's names qualified)."""
    found = self.lp(
        economy_flow._Program(dual.variables, dual.rows, {key: -value for key, value in dual.cost.items()}),
        economy_flow.DUAL,
    )
    if found.status != "optimal":  # the primal has an optimum, so its dual has one
        raise self.failure(economy_flow.LP_SOLVE_FAILED, stage=economy_flow.DUAL, reason="solver_status", status=2)
    least = -found.value
    if least != optimum:
        raise self.failure(
            economy_flow.LP_NUMERIC_CHECK_FAILED, stage=economy_flow.DUAL, constraint=economy_flow._token(economy_flow.FACE),
            found=str(least), bound=f"={optimum}",
        )
    return found.x


def before_max_min(self: Any, variables: Any, rows: list, coordinates: dict) -> Any:
    """`flow._Solver.max_min` as it was before this slice, line for line (the module's names qualified)."""
    zero, one = Fraction(0), Fraction(1)
    free = dict(coordinates)
    fixed: dict[str, Fraction] = {}
    stages: list[dict[str, Any]] = []
    while free:
        names = sorted(free)
        levels = [row for name in names for row in economy_flow._distance(name, *free[name], economy_flow.LEVEL)]
        stage = economy_flow._Program(
            {**variables, economy_flow.LEVEL: (zero, None, False)}, (*rows, *levels), {economy_flow.LEVEL: one}
        )
        best = self.optimal(stage, economy_flow.DUAL)
        level = best.value
        newly = []
        for name in names:
            others = [
                row for other in names if other != name for row in economy_flow._distance(other, *free[other], level)
            ]
            probe = economy_flow._Program(
                {**variables, economy_flow.PROBE: (zero, None, False)},
                (*rows, *others, *economy_flow._distance(name, *free[name], economy_flow.PROBE)),
                {economy_flow.PROBE: one},
            )
            if self.optimal(probe, economy_flow.DUAL).value <= level:
                newly.append(name)
        if not newly:
            raise self.failure(
                economy_flow.LP_NUMERIC_CHECK_FAILED, stage=economy_flow.DUAL, constraint="max_min", found=str(level),
                bound="fixed",
            )
        for name in newly:
            function = free.pop(name)[0]
            fixed[name] = sum((coefficient * best.x[key] for key, coefficient in function.items()), zero)
            rows.append(economy_flow._Row(("fix", name, 0), function, "eq", fixed[name], None))
        stages.append({"level": str(level), "fixed": newly})
    return fixed, stages


def before_joint(
    self: Any, dual: Any, point: dict, optimum: Fraction, prices: dict, on_face: Any, optimal: dict, primal: Any,
    seed: Any, *, lean: bool = False,
) -> Any:
    """`flow._Solver.joint` as it was before this slice, line for line (the module's names qualified, its docstring left
    out)."""
    f = economy_flow
    model, steps, zero, one = self.model, self.model.steps, Fraction(0), Fraction(1)
    variables = on_face[0]
    warnings: list[dict[str, Any]] = []
    unverified: dict[str, list[str]] = {activity: [] for activity in model.activities}

    def note(probe: str, activity: str, found: Any) -> None:
        unverified[activity].append(probe)
        warnings.append(self.unverified(probe, activity, found))

    pools: dict[str, dict] = {}
    for resource in model.resources:
        mean: dict = {}
        for step in range(steps):
            for key, sign in dual.duals[("balance", resource, step)].items():
                f._add(mean, key, sign / steps)
        pools[resource] = f._nonzero(mean)
    intervals: dict[str, Any] = {}
    for resource in model.resources:
        if all(f._one(pair) for pair in prices[resource]):  # constant on D* in every step, so is its mean
            mean = sum((low for low, _ in prices[resource]), zero) / steps
            intervals[resource] = (mean, mean)
        else:
            intervals[resource] = self.ends(*on_face, pools[resource])
    self.lap("intervals")
    most: dict[str, Fraction | None] = {}
    runs: dict[str, dict] = {}  # activity -> its `most` probe's solution
    worth: dict[str, dict] = {}
    ranges: dict[str, Any] = {}
    for activity in model.activities:
        counts = {("x", activity, step): one for step in range(steps)}
        if f._never(dual, point, list(counts)):  # complementary slackness: no optimum runs it
            most[activity] = zero
        else:  # on S3's optimal face (PRICE-2e: in its complementary-slackness form, no row V ≥ V0)
            program = f._Program(*primal, counts)
            found = self.certain(
                self.probe(program, f.JUDGEMENT, unbounded=True, warm=True, seed=seed), f.JUDGEMENT
            )
            if isinstance(found, f._Unverified):
                most[activity] = None
                note(f.MOST, activity, found)
            else:
                most[activity] = None if found.status == "unbounded" else found.value
                if found.status != "unbounded":  # an optimum of S3 that runs it `most`: its forcing cost's x*
                    runs[activity] = found.x
        self.lap("most")
        if model.touches_store(activity):
            continue
        value: dict = {}
        for resource in model.resources:
            amount = model.net.get((activity, resource))
            if amount:
                for key, coefficient in pools[resource].items():
                    f._add(value, key, amount * coefficient)
        worth[activity] = f._nonzero(value)
        read = [resource for resource in model.resources if model.net.get((activity, resource))]
        if all(f._one(intervals[resource]) for resource in read):
            constant = sum((model.net[(activity, resource)] * intervals[resource][0] for resource in read), zero)
            ranges[activity] = (constant, constant)  # every pool it reads is constant on D*, so is its value
            continue
        if lean and most[activity] != 0:  # some optimum runs it, or that is not known: never in K
            continue
        ends = self.probed_ends(*on_face, worth[activity])
        if isinstance(ends, f._Unverified):
            note(f.RANGE, activity, ends)
        else:
            ranges[activity] = ends
        self.lap("ranges")
    rows = list(on_face[1])  # D* without the strong-duality row (PRICE-2e), and the fixing rows after it
    chosen: dict[str, tuple[Fraction, str]] = {}
    goals = {resource for _, resource, _ in model.goals}

    def fix(resource: str, value: Fraction | None, by: str) -> None:
        if value is None:  # an end that D* bounds is bounded on every face of it
            raise self.failure(
                f.LP_NUMERIC_CHECK_FAILED, stage=f.DUAL, constraint=f.PRICE + resource, found="null", bound=by
            )
        rows.append(f._Row(("fix", f.PRICE + resource, 0), pools[resource], "eq", value, None))
        chosen[resource] = (value, by)

    for resource in model.resources:  # steps 1 and 3
        low, high = intervals[resource]
        if low is not None and low == high:
            chosen[resource] = (low, f.EXACT)
        elif low is not None and high is None and resource in goals:
            fix(resource, self.extreme(variables, rows, pools[resource], -1), f.ONE_MORE)
    self.lap("selection")
    coordinates: dict[str, Any] = {}
    for resource in model.resources:
        low, high = intervals[resource]
        if low is not None and high is not None and low < high:
            coordinates[f.PRICE + resource] = (pools[resource], low, high, True)
    for activity, (low, high) in ranges.items():
        if f.MOST in unverified[activity]:  # whether an optimum runs it is not known: it is not in K
            continue
        if most[activity] == 0 and low is not None and high == 0 and low < 0:
            coordinates[f.ACTIVITY_VALUE + activity] = (worth[activity], low, high, False)
    fixed, stages = self.max_min(variables, rows, coordinates)  # step 2
    for resource in model.resources:
        if f.PRICE + resource in fixed:
            chosen[resource] = (fixed[f.PRICE + resource], f.MAX_MIN)
    self.lap("max_min")
    for resource in model.resources:  # the withheld coordinates, free through the max-min
        if resource in chosen:
            continue
        low = self.extreme(variables, rows, pools[resource], -1)
        if low is not None:
            fix(resource, low, f.ONE_MORE)
            continue
        high = self.extreme(variables, rows, pools[resource], 1)
        fix(resource, high if high is not None else zero, f.ONE_LESS if high is not None else f.ZERO)
    self.lap("selection")
    self.member(dual, f._Program(variables, tuple(rows), {}), optimum)  # the vector is a point of D*, checked exactly
    self.lap("check")
    vector = {resource: chosen[resource][0] for resource in model.resources}
    prices = {
        resource: {
            "value": str(vector[resource]), "interval": f._price(intervals[resource]), "by": chosen[resource][1],
            "withheld": self.withheld(resource, intervals[resource], goals),
        }
        for resource in model.resources
    }
    activities: dict[str, Any] = {}
    at_vector: dict[str, Fraction | None] = {}
    for activity in model.activities:
        count, cost = most[activity], zero
        if lean:  # the vector reads no forcing cost
            cost = None
        elif f.MOST in unverified[activity]:  # the forcing cost reads `most`
            unverified[activity].append(f.FORCING)
            cost = None
        elif count is not None:
            forced = f._Row(
                ("forced", activity, 0), {("x", activity, step): -one for step in range(steps)}, "le", -count, None
            )
            forced_program = f._program(model, added=(forced,))
            forcing = f._dual(forced_program)
            restricted = f._restricted(forcing, forced_program, runs.get(activity, optimal))
            program = f._Program(*restricted, f._nonzero(dict(forcing.duals[forced.key])))
            found = self.certain(self.probe(program, f.DUAL, unbounded=True), f.DUAL)
            if isinstance(found, f._Unverified):
                note(f.FORCING, activity, found)
                cost = None
            else:
                cost = None if found.status == "unbounded" else found.value
        value = None
        if activity in worth:
            value = sum(
                (amount * vector[entity] for (name, entity), amount in model.net.items()
                 if name == activity and entity in vector),
                zero,
            )
        at_vector[activity] = value
        doubtful = unverified[activity]
        activities[activity] = {
            "value": f._text(value),
            "range": f._price(ranges[activity]) if activity in ranges else None,
            "in_optimum": None if f.MOST in doubtful else count is None or count > 0,
            "most": f._text(count),
            "forcing_cost": f._text(cost),
            "input_bound": None if f.FORCING in doubtful else cost is None,
            "unverified": list(doubtful),
        }
    self.chosen = f._Chosen(coordinates, vector, at_vector)  # #430 METHOD-C: what it chose, exact (`Held`)
    return {"prices": prices, "max_min": stages, "activities": activities}, warnings


def before_member(self: Any, dual: Any, program: Any, optimum: Fraction) -> None:
    """`flow._Solver.member` as it was before this slice, line for line (the module's names qualified)."""
    found = self.lp(program, economy_flow.DUAL, warm=True)
    if found.status != "optimal":
        raise self.failure(economy_flow.LP_SOLVE_FAILED, stage=economy_flow.DUAL, reason="solver_status", status=2)
    cost = sum((value * found.x[key] for key, value in dual.cost.items() if found.x[key]), Fraction(0))
    if cost != optimum:
        raise self.failure(
            economy_flow.LP_NUMERIC_CHECK_FAILED, stage=economy_flow.DUAL, constraint=economy_flow._token(economy_flow.FACE),
            found=str(cost), bound=f"={optimum}",
        )


def before_optimal(self: Any, program: Any, stage: str) -> Any:
    """`flow._Solver.optimal` as it was before this slice, line for line (the module's names qualified)."""
    solution = self.lp(program, stage)
    if solution.status != "optimal":
        raise self.failure(economy_flow.LP_SOLVE_FAILED, stage=stage, reason="solver_status", status=2)
    return solution


def before_projections(self: Any, tiers: Any, on_face: Any, warnings: list | None) -> Any:
    """`flow._Solver.projections` as it was before this slice, line for line (the module's names qualified)."""
    model, program, base, optimum, dual = self.model, tiers.program, tiers.base, tiers.base.value, tiers.dual
    self.grained = self.hold(program, base)  # S3 with its basis factorised, for the re-solves at ±grain (PRICE-2d)
    prices: dict[str, list] = {}
    grains: dict[str, list] = {}
    marginals: dict[str, list] = {}
    for resource in model.resources:
        prices[resource], grains[resource], marginals[resource] = [], [], []
        for step in range(model.steps):
            known, sides = self.chords(resource, step)
            for side in known:
                if side is not None:
                    self.grain_way("basis")
            grain = self.price(optimum, lambda delta: self.regrain(resource, step, delta), known)
            if economy_flow._pinned(model, resource, grain):
                price = grain  # the chords pin the projection (PRICE-2c): no LP on D*
            else:  # each end that S3's basis does not prove is an LP on D*
                function = dual.duals[("balance", resource, step)]
                low = sides[0] if sides[0] is not None else self.extreme(*on_face, function, -1)
                high = sides[1] if sides[1] is not None else self.extreme(*on_face, function, 1)
                price = (low, high)
            if warnings is not None:
                marginal = self.marginal(-base.eq[("balance", resource, step)])
                warnings += self.disagrees(resource, step, marginal, price)
                warnings += self.outside(resource, step, price, grain)
                marginals[resource].append(marginal)
            prices[resource].append(price)
            grains[resource].append(grain)
    return prices, grains, marginals


def before_least_losses(self: Any, tiers: Any) -> Any:
    """`flow._Solver.least_losses` as it was before this slice, line for line (the module's names qualified)."""
    least = self.lp(economy_flow._program(self.model, added=tiers.face, least_losses=True), "losses")
    if least.status != "optimal":
        raise self.failure(economy_flow.LP_SOLVE_FAILED, stage="losses", reason="solver_status", status=2)
    return least


BEFORE = {
    "lp": before_lp, "extreme": before_extreme, "ends": before_ends, "duality": before_duality,
    "max_min": before_max_min, "joint": before_joint, "member": before_member, "optimal": before_optimal,
    "projections": before_projections, "least_losses": before_least_losses,
}


def as_before(patched: pytest.MonkeyPatch) -> None:
    """The solver as it was before this slice: every method this slice changes, as it was (`BEFORE`)."""
    for name, method in BEFORE.items():
        patched.setattr(economy_flow._Solver, name, method)


@pytest.mark.parametrize(
    "fixture",
    [
        pytest.param(ex2, id="ex2"),
        pytest.param(cyclic_fixture, id="cyclic"),
        pytest.param(withheld_fixture, id="withheld"),
    ],
)
def test_where_no_lp_on_d_star_fails_the_steady_state_and_the_cache_are_what_they_were(
    tmp_path, economy, monkeypatch, fixture
):
    """EX2 (intervals, two max-min stages, the check), cyclic_fixture (seven steps, a hard cap) and withheld_fixture (a
    pool withheld, its least unbounded): the steady state by `acmodel economy`, through a cache of its own, now and as
    before this slice (`as_before`): the same answer, the same cache entries (each node's spec, key, value hash and trace)
    and the same run outputs, byte for byte."""
    folder = fixture(tmp_path / "model")
    now_root, before_root = tmp_path / "now", tmp_path / "before"

    status, doc = run(economy, folder, now_root)
    with monkeypatch.context() as patched:
        as_before(patched)
        status_before, doc_before = run(economy, folder, before_root)

    assert (status, doc["errors"]) == (0, [])
    assert (status_before, doc_before["errors"], doc_before["warnings"]) == (0, [], doc["warnings"])
    assert doc["data"] == doc_before["data"]
    assert kept_nodes(now_root) == kept_nodes(before_root)
    run_id = doc["data"]["run_id"]
    outputs = [root / "runs" / run_id / "outputs.json" for root in (now_root, before_root)]
    assert outputs[0].read_bytes() == outputs[1].read_bytes()


def test_where_no_lp_on_d_star_fails_an_export_writes_what_it_wrote_before(tmp_path, acmodel, monkeypatch):
    """flips_fixture (test_prices.py: two passes, pt.k at 1/2 and at 2) exported now and as before this slice
    (`as_before`): the same files; every price's provenance file byte for byte, and prices.json too once each pass's
    `timing_ms` is taken out."""
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
    assert policy.delivered(untimed_doc(prices)) == policy.delivered(untimed_doc(prices_before))
    assert sorted(now) == sorted(was)
    for name in now:
        if name != "prices.json":
            assert now[name] == was[name], name


# 7. The code -----------------------------------------------------------------------------------------------------------


def test_lp_invariant_broken_is_registered_with_its_exit_status():
    """`lp_invariant_broken` (contracts/codes.json, #430) is a registered error, with the exit status the catalog gives
    it, 1, as the other errors of the economy's LPs."""
    assert CODES["lp_invariant_broken"]["category"] == "error"
    assert EXIT_STATUS["lp_invariant_broken"] == CODES["lp_invariant_broken"]["exit"] == 1
    assert EXIT_STATUS["lp_numeric_check_failed"] == EXIT_STATUS["lp_solve_failed"] == 1
