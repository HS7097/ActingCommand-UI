"""#424 PROBE-GAP: a forcing cost that the pivot cap left unverified, when it is its activity's only unverified probe, is
no structural gap (contracts/formats.md, "Coverage", "A forcing cost alone is no gap"; `core.coverage._no_gap`). Every
other unverified probe, and every other gap, is one as before.

The fixtures are those of test_price_vector.py and test_economy.py, in neutral ids, some with an activity or a component
more. A probe is made unverified the way HiGHS and the exact pivoting leave one (`refusing`): its LP's solution stands,
exact, but its basis gives no certified vertex, for the reason given: `exact.Refused` with `pivot_cap` and its pivots
(as when the 20 pivots of `lp.max_pivots` run out), or no basis. Each docstring says by hand which probes those are and
what the plain fixture gives; every expected value follows from it. The warnings of `acmodel economy` are in the order
of their canonical JSON (`core.run.run_economy`), so the expected ones are put in that order. CI runs these tests on
Windows and on Ubuntu."""

from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path
from typing import Any, Callable

from acmodel.cache import EVALUATORS, Session, canonical_text, open_store
from acmodel.core import coverage, exact
from acmodel.core import flow as economy_flow
from acmodel.core.run import run_economy
from test_economy import (
    CODES,
    activity,
    economy,  # noqa: F401 (the fixture `acmodel economy`)
    flow,
    goal,
    limit,
    model_path,
    resource,
    shared_fixture,
    write,
)
from test_price_vector import QUICK, ex2, joint, judged, large_fixture, surplus_fixture
from test_prices import acmodel, delivered, export, priced  # noqa: F401 (the fixture `acmodel`)

PIVOTS = 20  # `lp.max_pivots` by default
AT_CAP = exact.Refused(exact.PIVOT_CAP, PIVOTS)
NO_BASIS = None  # HiGHS left no basis: `_Solver.vertex` has nothing to certify
SURPLUS_POOLS = ("res.g", "res.m", "res.s")
# act.waste of the surplus fixture (test_price_vector.py): worth -9/20 on all of D*, in no optimum, and forced to run it
# loses its dual slack, 9/20 an execution; and the same with its forcing cost unverified.
WASTE = judged("-9/20", "-9/20", "0", forcing_cost="9/20")
WASTE_UNVERIFIED = {**WASTE, "forcing_cost": None, "input_bound": None, "unverified": ["forcing_cost"]}
LIBRARIES = {"python": "3.13", "numpy": "2.5.4", "scipy": "1.18.1"}

Picks = Callable[[Any, str], bool]


def forcing(activity_id: str) -> Picks:
    """The LP of an activity's forcing cost (`core.flow._Solver.joint`, stage `dual`): on the dual of S3 with the row
    "its executions at least `most`" (`forced`), the greatest dual of that row."""
    objective = {("y", f"forced:{activity_id}:0", 0)}
    return lambda program, stage: stage == "dual" and set(program.objective) == objective


def most(activity_id: str) -> Picks:
    """The LP of the most an optimum of S3 runs an activity (stage `judgement`), in a one-step cycle."""
    objective = {("x", activity_id, 0)}
    return lambda program, stage: stage == "judgement" and set(program.objective) == objective


def value_range(*pools: str) -> Picks:
    """The LPs of each end of the range on D* of an activity whose value reads exactly `pools` (stage `dual`), in a
    one-step cycle: a function of the duals of those pools' balance rows only."""
    rows = {f"balance:{pool}:0" for pool in pools}
    return lambda program, stage: stage == "dual" and {key[1] for key in program.objective} == rows


def refusing(monkeypatch: Any, *refusals: tuple[Picks, exact.Refused | None]) -> None:
    """Each LP that one of `refusals` picks keeps its exact solution, but its basis gives no certified vertex: the exact
    pivoting refuses it (`exact.Refused`), or HiGHS left no basis (None), as `_Solver.vertex` reports either."""
    certify = economy_flow._Solver.vertex

    def vertex(solver: Any, program: Any, arrays: Any, run: Any, stage: str) -> Any:
        x, found = certify(solver, program, arrays, run, stage)
        for picks, refused in refusals:
            if picks(program, stage):
                return x, refused
        return x, found

    monkeypatch.setattr(economy_flow._Solver, "vertex", vertex)


def unverified(folder: Path, probe: str, activity_id: str, stage: str, reason: str, **more: Any) -> dict[str, Any]:
    """The warning `lp_probe_unverified` of one probe."""
    detail = {"path": model_path(folder), "probe": probe, "activity": activity_id, "stage": stage, "reason": reason}
    return {"code": "lp_probe_unverified", "detail": {**detail, **more}}


def in_order(*warnings: dict[str, Any]) -> list[dict[str, Any]]:
    """Warnings in the order `acmodel economy` writes them: by their canonical JSON."""
    return sorted(warnings, key=canonical_text)


def extended(
    folder: Path, entities: list, rules: list, unknowns: list = (), readings: dict[str, int] | None = None
) -> Path:
    """A fixture folder with more entities, rules and unknowns in its rule file, and more readings in its state."""
    path = folder / "ours" / "model" / "rules" / "economy.json"
    doc = json.loads(path.read_text(encoding="utf-8"))
    write(path, {
        **doc, "entities": [*doc["entities"], *entities], "rules": [*doc["rules"], *rules],
        "unknowns": [*doc["unknowns"], *unknowns],
    })
    if readings:
        state = json.loads((folder / "state.json").read_text(encoding="utf-8"))
        state["readings"].update(
            {fact: {"value": value, "observed_at_unix_ms": 0, "source": "fixture"} for fact, value in readings.items()}
        )
        write(folder / "state.json", state)
    return folder


def steady_of(doc: dict[str, Any]) -> dict[str, Any]:
    return doc["data"]["steady"]


def statuses(found: str, reasons: list[str], pools: tuple[str, ...] = SURPLUS_POOLS) -> dict[str, dict[str, Any]]:
    """`price_status` of `pools`, each `found` for `reasons`."""
    return {pool: {"status": found, "reasons": list(reasons)} for pool in pools}


# The probe PROBE-GAP lets go, and the same probe for another reason -------------------------------------------------


def test_a_forcing_cost_alone_at_the_cap_is_no_gap_and_moves_no_price(tmp_path, economy, acmodel, monkeypatch):
    """The surplus fixture (test_price_vector.py). By hand there: D* is the one point π_g = 1, π_m = 1/2, π_s = 1/20, so
    act.waste's dual slack is 10 × 1/20 - 1/10 × 1/2 = 9/20 > 0: no optimum runs it, so neither its tie nor its `most`
    is probed; every pool it reads is constant on D*, so its range is not probed either; its forcing cost, 9/20, the LP
    on the dual of S3 with the row "act.waste at least 0", is its only probe. Here that LP's basis is still not optimal
    after the 20 exact pivots (`pivot_cap`).

    The warning stands and names it; `forcing_cost` and `input_bound` are null and `unverified` lists it. Nothing else
    moves: the steady state is the plain fixture's member by member but act.waste's entry of `joint.activities`, so its
    one static component holds no gap, and every price is confirmed for no reason, as the plain fixture's. `export
    prices` writes the same prices.json (acmodel.prices.v2, its entries' members as before), each price with the same
    `binds`: its provenance document reads the price's entry, its status and reasons, never a forcing cost."""
    plain_folder = priced(surplus_fixture(tmp_path / "plain"))
    plain_status, plain = economy(plain_folder, "--steady-only")
    plain_exported = export(acmodel, plain_folder)
    refusing(monkeypatch, (forcing("act.waste"), AT_CAP))
    folder = priced(surplus_fixture(tmp_path / "pivot_cap"))

    status, doc = economy(folder, "--steady-only")

    assert (plain_status, plain["errors"], plain["warnings"], status, doc["errors"]) == (0, [], [], 0, [])
    assert doc["warnings"] == [unverified(folder, "forcing_cost", "act.waste", "dual", "pivot_cap", pivots=PIVOTS)]
    assert joint(plain)["activities"]["act.waste"] == WASTE
    assert joint(doc)["activities"]["act.waste"] == WASTE_UNVERIFIED
    found, shown = steady_of(doc), steady_of(plain)
    assert {**found, "joint": None} == {**shown, "joint": None}
    assert {**joint(doc), "activities": None} == {**joint(plain), "activities": None}
    others = [name for name in joint(plain)["activities"] if name != "act.waste"]
    assert [joint(doc)["activities"][name] for name in others] == [joint(plain)["activities"][name] for name in others]
    assert (found["coverage"]["components"]["act.get"]["gap"], found["coverage"]["components"]["act.get"]["gaps"]) == (
        False, {}
    )
    assert found["price_status"] == statuses("confirmed", [])
    assert found["status"] == "confirmed"

    exported = export(acmodel, folder)

    assert (plain_exported[0], plain_exported[1]["errors"], exported[0], exported[1]["errors"]) == (0, [], 0, [])
    prices = delivered(folder)
    assert prices == delivered(plain_folder)
    assert prices["schema_version"] == "acmodel.prices.v2"
    assert {entry["pool"]: (entry["class"], entry["status"], entry["pending"]) for entry in prices["prices"]} == {
        pool: ("covered", "confirmed", []) for pool in SURPLUS_POOLS
    }
    assert all(sorted(entry) == sorted([
        "pool", "class", "status", "reasons", "component", "price", "milli", "pending", "joint", "degenerate",
        "varies_by_step", "impact", "flips", "binds", "acceptance", "provenance",
    ]) for entry in prices["prices"])


def test_a_forcing_cost_unverified_for_another_reason_is_still_a_gap(tmp_path, economy, monkeypatch):
    """The same probe, but HiGHS left no basis for it (`no_basis`). PROBE-GAP lets the pivot cap alone go, so this is a
    gap as before: act.waste is `probe_unverified`, its component (every pool) a structural gap, every price
    provisional for it, and the steady state too. The judgement shows the same nulls, and the vector is the plain
    fixture's."""
    _, plain = economy(surplus_fixture(tmp_path / "plain"), "--steady-only")
    refusing(monkeypatch, (forcing("act.waste"), NO_BASIS))
    folder = surplus_fixture(tmp_path / "no_basis")

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == [unverified(folder, "forcing_cost", "act.waste", "dual", "no_basis")]
    assert joint(doc)["activities"]["act.waste"] == WASTE_UNVERIFIED
    assert {**joint(doc), "activities": None} == {**joint(plain), "activities": None}
    found = steady_of(doc)
    assert found["coverage"]["components"]["act.get"]["gaps"] == {"probe_unverified": ["act.waste"]}
    assert found["price_status"] == statuses("provisional", ["probe_unverified"])
    assert found["status"] == "provisional"


def test_a_forcing_cost_at_the_cap_beside_another_unverified_probe_is_a_gap(tmp_path, economy, monkeypatch):
    """EX2 (test_price_vector.py). By hand there: act.pair runs in no optimum (`most` 0), its value 2/3 π_v - π_a - π_b
    ranges over [-1/3, 0] on D* (two LPs, a and b not constant there), and its forcing cost is 1/3. Here both are
    unverified: no basis for its range's LPs, and the pivot cap for its forcing cost. Both warnings stand, `unverified`
    lists both in its order, and act.pair, left out of K for its range (its value then read at another vector, not
    checked here), makes its one component, every pool of EX2, a structural gap: a forcing cost beside another
    unverified probe lifts nothing."""
    refusing(monkeypatch, (value_range("res.a", "res.b", "res.v"), NO_BASIS), (forcing("act.pair"), AT_CAP))
    folder = ex2(tmp_path)

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == in_order(
        unverified(folder, "range", "act.pair", "dual", "no_basis"),
        unverified(folder, "forcing_cost", "act.pair", "dual", "pivot_cap", pivots=PIVOTS),
    )
    pair = {key: value for key, value in joint(doc)["activities"]["act.pair"].items() if key != "value"}
    assert pair == {
        "range": None, "in_optimum": False, "most": "0", "forcing_cost": None, "input_bound": None,
        "unverified": ["range", "forcing_cost"],
    }
    found = steady_of(doc)
    name = found["coverage"]["pools"]["res.a"]["component"]
    assert found["coverage"]["components"][name]["gaps"] == {"probe_unverified": ["act.pair"]}
    assert found["price_status"] == statuses("provisional", ["probe_unverified"], ("res.a", "res.b", "res.c", "res.v"))
    assert found["status"] == "provisional"


# The other gaps of the same component, and another component ---------------------------------------------------------

UNK_P = {
    "id": "unk.p", "kind": "interval", "question": "fixture", "interval": {"low": "1", "high": "3"},
    "status": "pending", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
}
PENDING_P = [
    {**flow("flow.p", "act.p", consumes={"res.s": "10"}, produces={"res.g": "unknown:unk.p"}), "status": "pending",
     "unknowns": ["unk.p"]},
    limit("limit.p", "act.p", "1", "day"),
]


def surplus_pending(folder: Path) -> Path:
    """The surplus fixture with act.p, whose flow is pending: one act.p turns 10 s into unk.p g, an interval from 1 to 3
    not measured yet, at most once a day. The steady state leaves it out (`excluded_pending`); it takes s and brings in
    g, so it is in the one static component, which is a structural gap for it."""
    return extended(surplus_fixture(folder), [activity("act.p")], PENDING_P, [UNK_P])


def test_a_forcing_cost_alone_beside_a_pending_flow_leaves_that_gap_as_it_was(tmp_path, economy, monkeypatch):
    """The shape of a large model whose pending flow keeps its component a gap: `surplus_pending`, and act.waste's
    forcing cost, as in the surplus fixture its only probe, at the pivot cap. act.p is out of the program, so act.waste
    reads as before. The component is still a structural gap, for act.p alone: `gaps` is `excluded_pending` only, each
    pool `structural_gap` for that one reason and each price provisional for it, as without the refused probe."""
    _, plain = economy(surplus_pending(tmp_path / "plain"), "--steady-only")
    refusing(monkeypatch, (forcing("act.waste"), AT_CAP))
    folder = surplus_pending(tmp_path / "pivot_cap")

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"], plain["warnings"]) == (0, [], [])
    assert doc["warnings"] == [unverified(folder, "forcing_cost", "act.waste", "dual", "pivot_cap", pivots=PIVOTS)]
    found = steady_of(doc)
    assert found["excluded_pending"] == [{"flow": "flow.p", "activity": "act.p", "unknowns": ["unk.p"]}]
    component = found["coverage"]["components"]["act.get"]
    assert (component["gap"], component["gaps"], component["reasons"]) == (
        True, {"excluded_pending": ["act.p"]}, ["excluded_pending"]
    )
    assert {pool: (shown["class"], shown["reasons"]) for pool, shown in found["coverage"]["pools"].items()} == {
        pool: ("structural_gap", ["excluded_pending"]) for pool in SURPLUS_POOLS
    }
    assert found["price_status"] == statuses("provisional", ["excluded_pending"])
    assert (found["coverage"], found["price_status"]) == (steady_of(plain)["coverage"], steady_of(plain)["price_status"])


def tie_with_waste(folder: Path) -> Path:
    """The tie of test_economy.py (`shared_fixture`: s 100 a day, act.get 10 s for 10 g at most 10 a day, act.train
    10 s for 1 m, a point worth 10) with act.waste, which turns 10 s into 1/10 m."""
    waste = flow("flow.waste", "act.waste", consumes={"res.s": "10"}, produces={"res.m": "0.1"})
    return extended(shared_fixture(folder, "10", most="10"), [activity("act.waste")], [waste])


def test_a_forcing_cost_alone_beside_a_tie_leaves_the_tie_a_gap(tmp_path, economy, monkeypatch):
    """`tie_with_waste`. By hand: V0 = A* = V* = 100 as in the tie; some optimum of S3 puts all s into act.train, so
    10 π_s = π_m, and some leaves act.get below its limit, so its limit's dual is 0 and 10 π_s = 10 π_g: D* is the one
    point π_s = 1, π_g = 1, π_m = 10. act.waste's dual slack is 10 × 1 - 1/10 × 10 = 9 > 0: no optimum runs it (no tie
    probe, no probe of `most`), its range is constant, and its forcing cost, 9, is its only probe. act.train is still
    `objective_tier_tie`. With act.waste's forcing cost at the pivot cap, the component is a structural gap for the tie
    alone, as without the refused probe."""
    _, plain = economy(tie_with_waste(tmp_path / "plain"), "--steady-only")
    refusing(monkeypatch, (forcing("act.waste"), AT_CAP))
    folder = tie_with_waste(tmp_path / "pivot_cap")

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert plain["warnings"] == [
        {"code": "objective_tier_tie", "detail": {"path": model_path(tmp_path / "plain"), "activity": "act.train"}}
    ]
    assert doc["warnings"] == in_order(
        {"code": "objective_tier_tie", "detail": {"path": model_path(folder), "activity": "act.train"}},
        unverified(folder, "forcing_cost", "act.waste", "dual", "pivot_cap", pivots=PIVOTS),
    )
    assert joint(plain)["activities"]["act.waste"] == judged("-9", "-9", "0", forcing_cost="9")
    found = steady_of(doc)
    assert found["coverage"]["components"]["act.get"]["gaps"] == {"objective_tier_tie": ["act.train"]}
    assert found["price_status"] == statuses("provisional", ["objective_tier_tie"])
    assert (found["coverage"], found["price_status"]) == (steady_of(plain)["coverage"], steady_of(plain)["price_status"])


def surplus_and_x(folder: Path) -> Path:
    """The surplus fixture and a second static component with no edge to it: act.x brings in 1 x once a day, x a main
    goal worth 1."""
    rules = [
        flow("flow.x", "act.x", produces={"res.x": "1"}), limit("limit.x", "act.x", "1", "day"),
        goal("goal.x", "res.x", "1"),
    ]
    entities = [resource("res.x", "fact.x"), activity("act.x")]
    return extended(surplus_fixture(folder), entities, rules, readings={"fact.x": 0})


def test_each_component_keeps_its_own_judgement(tmp_path, economy, monkeypatch):
    """`surplus_and_x`. act.x runs once in every optimum (its limit binds), so its dual slack is 0 and its `most` is
    probed (1). Two probes unverified: act.waste's forcing cost at the pivot cap, its only one, and act.x's `most`, no
    basis. The surplus component holds no gap and keeps its confirmed prices; act.x's is a structural gap for act.x
    (`probe_unverified`), its one price provisional; so is the steady state."""
    refusing(monkeypatch, (forcing("act.waste"), AT_CAP), (most("act.x"), NO_BASIS))
    folder = surplus_and_x(tmp_path)

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == in_order(
        unverified(folder, "most", "act.x", "judgement", "no_basis"),
        unverified(folder, "forcing_cost", "act.waste", "dual", "pivot_cap", pivots=PIVOTS),
    )
    found = steady_of(doc)
    components = found["coverage"]["components"]
    assert {name: (components[name]["gap"], components[name]["gaps"]) for name in ("act.get", "act.x")} == {
        "act.get": (False, {}), "act.x": (True, {"probe_unverified": ["act.x"]}),
    }
    assert components["act.get"]["status"] == "confirmed"
    assert found["price_status"] == {
        **statuses("confirmed", []), **statuses("provisional", ["probe_unverified"], ("res.x",)),
    }
    assert found["status"] == "provisional"


# What stays as it was: a cross-check, the quick look, the cache, the contract -----------------------------------------


def test_a_marginal_that_disagrees_is_still_no_gap(tmp_path, economy, monkeypatch):
    """`lp_dual_disagrees` is a cross-check and no gap, before PROBE-GAP and after: the large fixture of
    test_price_vector.py with HiGHS's marginal of a's balance row ten tolerances away from a's exact price. The warning
    stands; the coverage, each price's status and the steady state's are the plain fixture's, with no gap."""
    _, plain = economy(large_fixture(tmp_path / "plain"), "--steady-only")
    marginals = economy_flow._marginals

    def shifted(section: Any, rows: Any) -> dict:
        found = marginals(section, rows)
        if ("balance", "res.a", 0) in found:
            found[("balance", "res.a", 0)] = -2.000003
        return found

    monkeypatch.setattr(economy_flow, "_marginals", shifted)

    status, doc = economy(large_fixture(tmp_path / "shifted"), "--steady-only")

    assert (status, doc["errors"], plain["warnings"]) == (0, [], [])
    assert [entry["code"] for entry in doc["warnings"]] == ["lp_dual_disagrees"]
    found, shown = steady_of(doc), steady_of(plain)
    assert not any(component["gap"] for component in found["coverage"]["components"].values())
    assert (found["coverage"], found["price_status"], found["status"]) == (
        shown["coverage"], shown["price_status"], shown["status"]
    )


def test_the_quick_look_still_names_what_it_cannot_rule_out(tmp_path, economy, monkeypatch):
    """PRICE-2f, unchanged: the quick look runs the tie probes only (here none: act.waste runs in no optimum, the optimum
    of S2 runs the two others), so it still names `probe_unverified` as not known (`coverage_unknown`). With act.waste's
    forcing cost at the pivot cap, the full run's one warning is that probe, and its coverage is the quick look's: the
    probe the full run adds makes no gap."""
    refusing(monkeypatch, (forcing("act.waste"), AT_CAP))
    folder = surplus_fixture(tmp_path)

    status, quick = economy(folder, *QUICK)
    full_status, full = economy(folder, "--steady-only")

    assert (status, quick["errors"], quick["warnings"], full_status, full["errors"]) == (0, [], [], 0, [])
    assert quick["data"]["quick"]["coverage_unknown"] == ["probe_unverified"]
    assert full["warnings"] == [unverified(folder, "forcing_cost", "act.waste", "dual", "pivot_cap", pivots=PIVOTS)]
    assert quick["data"]["quick"]["coverage"] == steady_of(full)["coverage"]


def test_the_economy_kind_is_version_12_and_no_entry_of_version_10_is_reused(tmp_path):
    """PROBE-GAP bumped the `economy` kind's evaluator version to 11, and #430's second try of an LP on D* to 12
    (`cache/keys.py`): a steady state that version 10 kept is never served, so it is computed again, to the same output; a
    second run of one version reuses its own, as before. The run id moves with the version."""
    assert EVALUATORS["economy"].version == 12
    folder = surplus_fixture(tmp_path / "fixture")
    state_root = tmp_path / "state"
    older = {**EVALUATORS, "economy": replace(EVALUATORS["economy"], version=10)}

    def solved(evaluators: dict[str, Any]) -> dict[str, Any]:
        with open_store(state_root, max_bytes=2**31) as store:
            session = Session(store, libraries=LIBRARIES, evaluators=evaluators)
            data, errors, _ = run_economy(
                folder / "ours" / "model", folder / "state.json", datasets=state_root / "datasets", max_bits=127,
                cache=session, steady_only=True,
            )
        assert errors == []
        return data

    before, again, after = solved(older), solved(older), solved(EVALUATORS)

    computed_once = {"computed": 1, "reused": 0, "cutoff": 0}
    assert before["work"]["by_kind"]["economy"] == after["work"]["by_kind"]["economy"] == computed_once
    assert again["work"]["by_kind"]["economy"] == {"computed": 0, "reused": 1, "cutoff": 0}
    assert before["run_id"] == again["run_id"] != after["run_id"]
    assert after["steady"] == before["steady"]


def test_the_contract_surface_is_unchanged():
    """PROBE-GAP changes which warnings make a gap and nothing else of the contract: `lp_probe_unverified` keeps its
    category, its members and its vocabularies (codes.json); the gaps and the quick look's unknowns are those of PRICE-2c
    and PRICE-2f; and the probe the coverage section lets go is the one the steady state names, at the reason the exact
    pivoting gives. prices.json stays acmodel.prices.v2, with its entries' members (the export above)."""
    spec = CODES["lp_probe_unverified"]
    assert (spec["category"], spec["exit"]) == ("warning", 0)
    assert spec["values"] == {
        "path": {"type": "path", "required": True},
        "probe": {"type": "vocab", "required": True, "members": ["tie", "most", "range", "forcing_cost"]},
        "activity": {"type": "token", "required": True},
        "stage": {"type": "vocab", "required": True, "members": ["tie", "judgement", "dual"]},
        "reason": {
            "type": "vocab", "required": True,
            "members": [
                "no_basis", "basis_invalid", "singular", "pivot_cap", "not_feasible", "unbounded", "infeasible",
                "solve_failed",
            ],
        },
        "pivots": {"type": "integer", "required": False},
        "error": {"type": "vocab", "required": False, "members": ["lp_numeric_check_failed", "lp_solve_failed"]},
        "constraint": {"type": "token", "required": False},
        "found": {"type": "token", "required": False},
        "bound": {"type": "token", "required": False},
    }
    assert coverage.GAPS == ("excluded_pending", "no_steady_supply", "objective_tier_tie", "probe_unverified")
    assert coverage.NEEDS_LATER_STAGES == ("probe_unverified",)
    assert (coverage.FORCING_COST, coverage.PIVOT_CAP) == (economy_flow.FORCING, exact.PIVOT_CAP) == (
        "forcing_cost", "pivot_cap"
    )
