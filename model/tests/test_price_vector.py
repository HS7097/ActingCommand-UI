"""#424 ACM-PRICE PRICE-2b-1: the joint price vector and the activity judgements (addendum 2.1 §2), on the worked
examples EX1-EX6 of §2.4 in neutral ids (the tests of §2.5), a price without a low end, and the tolerance of HiGHS's
marginal against an exact price whose denominator is very large.

Every fixture is a one-day cycle in which res.v is the one main goal, worth 1 value unit (`test_economy.model`), so a
unit of v is worth exactly 1 and stands for the examples' "value". Each docstring works out by hand D*, the optimal
face of the steady state's dual, as the prices of one unit of each pool it allows (the coordinates every output below
reads), and from it every expected number. CI runs these tests on Windows and on Ubuntu, so both legs must give the
same exact values."""

from __future__ import annotations

import json
from dataclasses import replace
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli
from acmodel.core import flow as economy_flow
from test_economy import (
    CODES,
    DAY,
    activity,
    cap,
    economy,  # noqa: F401 (the fixture `acmodel economy`)
    flow,
    goal,
    limit,
    model,
    model_path,
    regen,
    resource,
    shared_fixture,
    steady,
    training,
    write,
)

EXACT_V = {"value": "1", "interval": "1", "by": "exact", "withheld": None}  # res.v, the main goal worth 1


def between(low: str | None, high: str | None) -> dict[str, str | None]:
    return {"low": low, "high": high}


def priced(value: str, interval: Any, by: str, withheld: dict[str, Any] | None = None) -> dict[str, Any]:
    """A pool's entry of `joint.prices`: its coordinate in the vector, its interval on D*, how the selection chose it
    and why it withholds it (or None)."""
    return {"value": value, "interval": interval, "by": by, "withheld": withheld}


def judged(value: str | None, span: Any, most: str | None, *, forcing_cost: str | None = None) -> dict[str, Any]:
    """An activity's entry of `joint.activities`: its value at the vector, its range on D*, the most an optimum runs
    it, and the loss per execution of running it more; none (null) is `input_bound`; every probe verified (PRICE-2c:
    `unverified` empty)."""
    return {
        "value": value, "range": span, "in_optimum": most is None or Fraction(most) > 0, "most": most,
        "forcing_cost": forcing_cost, "input_bound": forcing_cost is None, "unverified": [],
    }


def joint(doc: dict[str, Any]) -> dict[str, Any]:
    return doc["data"]["steady"]["joint"]


def grain(doc: dict[str, Any]) -> dict[str, Any]:
    return doc["data"]["steady"]["grain"]["prices"]


def example(
    folder: Path, pools: list[str], regens: dict[str, str], flows: dict[str, tuple[dict, dict]],
    limits: dict[str, str] | None = None, goals: list | None = None,
) -> Path:
    """A one-day fixture: res.v and `pools` (`res.<x>`, each read from `fact.<x>`); `regens`, pool to what it brings a
    day; `flows`, activity (`act.<x>`, its flow `flow.<x>`) to what one execution consumes and produces; `limits`,
    activity to its most a day (`limit.<x>`); `goals`, the objective rules (by default res.v worth 1)."""
    entities = [resource("res.v", "fact.v"), *(resource(pool, "fact." + pool[4:]) for pool in pools)]
    entities += [activity(name) for name in flows]
    rules = [regen("regen." + pool[4:], pool, amount, "1d") for pool, amount in regens.items()]
    for name, (consumes, produces) in flows.items():
        logic = {**({"consumes": consumes} if consumes else {}), **({"produces": produces} if produces else {})}
        rules.append(flow("flow." + name[4:], name, **logic))
    rules += [limit("limit." + name[4:], name, most, "day") for name, most in (limits or {}).items()]
    readings = {"fact.v": 0, **{"fact." + pool[4:]: 0 for pool in pools}}
    return model(folder, [DAY], entities, rules, readings, goals=goals)


SET = ({"res.a": "1", "res.b": "1", "res.c": "1"}, {"res.v": "1"})  # one each of a, b and c for 1 v


def ex1(folder: Path) -> Path:
    """EX1: a, b and c come at 1 a day each; act.set turns one of each into 1 v."""
    return example(folder, ["res.a", "res.b", "res.c"], {"res.a": "1", "res.b": "1", "res.c": "1"}, {"act.set": SET})


def ex2(folder: Path) -> Path:
    """EX2: EX1, and act.pair turns one a and one b into 2/3 v."""
    flows = {"act.pair": ({"res.a": "1", "res.b": "1"}, {"res.v": "2 / 3"}), "act.set": SET}
    return example(folder, ["res.a", "res.b", "res.c"], {"res.a": "1", "res.b": "1", "res.c": "1"}, flows)


def ex5(folder: Path) -> Path:
    """EX5: a comes at 1 a day; act.sink turns one a into 2 v, at most once a day."""
    return example(folder, ["res.a"], {"res.a": "1"}, {"act.sink": ({"res.a": "1"}, {"res.v": "2"})}, {"act.sink": "1"})


UNIT = between("0", "1")
EX1_JOINT = {
    "prices": {**{pool: priced("1/3", UNIT, "max_min") for pool in ("res.a", "res.b", "res.c")}, "res.v": EXACT_V},
    "max_min": [{"level": "1/3", "fixed": ["price:res.a", "price:res.b", "price:res.c"]}],
    "activities": {"act.set": judged("0", "0", "1")},
}
EX2_JOINT = {
    "prices": {
        "res.a": priced("3/7", UNIT, "max_min"), "res.b": priced("3/7", UNIT, "max_min"),
        "res.c": priced("1/7", between("0", "1/3"), "max_min"), "res.v": EXACT_V,
    },
    "max_min": [
        {"level": "3/7", "fixed": ["price:res.a", "price:res.b", "price:res.c"]},
        {"level": "4/7", "fixed": ["activity_value:act.pair"]},
    ],
    "activities": {
        "act.pair": judged("-4/21", between("-1/3", "0"), "0", forcing_cost="1/3"),
        "act.set": judged("0", "0", "1"),
    },
}
EX5_JOINT = {
    "prices": {"res.a": priced("1", between("0", "2"), "max_min"), "res.v": EXACT_V},
    "max_min": [{"level": "1/2", "fixed": ["price:res.a"]}],
    "activities": {"act.sink": judged("1", between("0", "2"), "1")},
}


@pytest.fixture
def explain(make_system, app_dir):
    """`acmodel explain` of one output on a fixture folder: (exit status, envelope)."""

    def run(folder: Path, output: str) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        argv = ["explain", str(folder / "ours" / "model"), output, "--state", str(folder / "state.json")]
        status = cli.main(argv, system)
        [line] = system.stdout.getvalue().splitlines()
        doc = json.loads(line)
        assert doc["command"] == "explain" and doc["ok"] is (doc["errors"] == [])
        assert status == (CODES[doc["errors"][0]["code"]]["exit"] if doc["errors"] else 0)
        return status, doc

    return run


# §2.5 test 1: a set of pools, and a producer of several -------------------------------------------------------------


def test_a_set_of_pools_and_their_producer_are_priced_by_one_joint_vector(tmp_path, economy):
    """EX1. By hand: V0 = 1, one set a day. D* is π_v = 1 and π_a + π_b + π_c = 1, each at least 0 (the set's dual row
    and the day's one unit of each), so each pool's interval is [0, 1], in its step too: one more a alone finds no b
    or c (0), one less loses the set (1). The max-min raises the three together, π ≥ t and π ≤ 1 - t summing to 1:
    t = 1/3 in one stage, the vector (1/3, 1/3, 1/3). It sums to the set's 1 v, so act.set reads 1 - 3 × 1/3 = 0,
    where the per-pool midpoints (1/2 each) would read it 1 - 3/2 = -1/2. act.set runs once in every optimum and
    cannot run again: the day brings no more a, b or c (input_bound).

    EX3: s comes at 1 a day; act.break turns one s into one each of a, b and c, which have no other source; act.set as
    in EX1. By hand: V0 = 1; D* is π_s = π_v = 1 and π_a + π_b + π_c = 1, each at least 0. s is exact (one more s is
    one more break and set: 1). The vector is (1/3, 1/3, 1/3) again, so act.break reads 3 × 1/3 - 1 = 0, where the
    midpoints would give it +1/2, a false gain the Runtime would dispatch. Both run once in every optimum, input_bound.
    Every vector is checked against D* as worked out here, exactly."""
    status, doc = economy(ex1(tmp_path / "ex1"), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == grain(doc) == {"res.a": [UNIT], "res.b": [UNIT], "res.c": [UNIT], "res.v": ["1"]}
    assert joint(doc) == EX1_JOINT
    vector = {pool: Fraction(entry["value"]) for pool, entry in joint(doc)["prices"].items()}
    assert vector["res.v"] == 1 and min(vector.values()) >= 0
    assert vector["res.a"] + vector["res.b"] + vector["res.c"] == 1  # in D*
    assert joint(doc)["activities"]["act.set"]["value"] != str(1 - 3 * Fraction(1, 2))  # no midpoint's false value

    flows = {"act.break": ({"res.s": "1"}, {"res.a": "1", "res.b": "1", "res.c": "1"}), "act.set": SET}
    status, doc = economy(example(tmp_path / "ex3", ["res.a", "res.b", "res.c", "res.s"], {"res.s": "1"}, flows),
                          "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == {"res.a": [UNIT], "res.b": [UNIT], "res.c": [UNIT], "res.s": ["1"], "res.v": ["1"]}
    assert joint(doc) == {
        "prices": {
            **{pool: priced("1/3", UNIT, "max_min") for pool in ("res.a", "res.b", "res.c")},
            "res.s": priced("1", "1", "exact"), "res.v": EXACT_V,
        },
        "max_min": [{"level": "1/3", "fixed": ["price:res.a", "price:res.b", "price:res.c"]}],
        "activities": {"act.break": judged("0", "0", "1"), "act.set": judged("0", "0", "1")},
    }
    vector = {pool: Fraction(entry["value"]) for pool, entry in joint(doc)["prices"].items()}
    assert vector["res.s"] == vector["res.v"] == 1 and min(vector.values()) >= 0
    assert vector["res.a"] + vector["res.b"] + vector["res.c"] == 1  # in D*


# §2.5 test 2: complements ----------------------------------------------------------------------------------------------


def test_complements_are_exact_when_one_is_in_excess_and_sum_to_their_set_when_degenerate(tmp_path, economy):
    """EX4: a comes at 2 a day and b at 1; act.set turns one of each into 1 v. By hand: V0 = 1. a is in excess (one
    more or one less a changes nothing) and b binds (one more b is one more set, one less one fewer): D* is π_a = 0
    and π_b = 1, both exact, so nothing is selected. act.set reads 1 - 0 - 1 = 0; it runs once and the day brings no
    more b (input_bound).

    EX1, degenerate (each pool in [0, 1]): the vector sums to what the set brings, π_v = 1."""
    flows = {"act.set": ({"res.a": "1", "res.b": "1"}, {"res.v": "1"})}
    status, doc = economy(example(tmp_path / "ex4", ["res.a", "res.b"], {"res.a": "2", "res.b": "1"}, flows),
                          "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == {"res.a": ["0"], "res.b": ["1"], "res.v": ["1"]}
    assert joint(doc) == {
        "prices": {"res.a": priced("0", "0", "exact"), "res.b": priced("1", "1", "exact"), "res.v": EXACT_V},
        "max_min": [],
        "activities": {"act.set": judged("0", "0", "1")},
    }

    status, doc = economy(ex1(tmp_path / "ex1"), "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    prices = joint(doc)["prices"]
    assert sum(Fraction(prices[pool]["value"]) for pool in ("res.a", "res.b", "res.c")) == Fraction(prices["res.v"]["value"])


# §2.5 test 3: degeneracy -----------------------------------------------------------------------------------------------


def test_a_degenerate_pool_alone_takes_its_midpoint(tmp_path, economy):
    """EX5. By hand: V0 = 2 (one sink a day, 2 v). D* is π_v = 1 and π_a in [0, 2]: one more a finds the sink at its
    limit (0), one less loses the sink's 2 v (2); the limit's dual makes up the rest of the sink's 2. The midpoint 1
    is jointly feasible and makes no activity outside the optima read 0, so the max-min keeps it: t = 1/2. act.sink
    reads 2 - 1 = 1, its range 2 - π_a on D* is [0, 2]; it runs once, and its limit lets it run no more (input_bound)."""
    status, doc = economy(ex5(tmp_path), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == grain(doc) == {"res.a": [between("0", "2")], "res.v": ["1"]}
    assert joint(doc) == EX5_JOINT


def test_an_activity_that_no_optimum_runs_reads_strictly_below_zero(tmp_path, economy):
    """EX2. By hand: V0 = 1, the set; act.pair (2/3 for an a and a b) is in no optimum (it would leave c unused). D* is
    π_v = 1, π_a + π_b + π_c = 1, each at least 0, and π_a + π_b ≥ 2/3 (act.pair's dual row), so a and b are in
    [0, 1] and c in [0, 1/3] (one less c turns the set into a pair: 1/3). act.pair reads 2/3 - π_a - π_b = π_c - 1/3,
    in [-1/3, 0] on D*: it is in K. The equal shares (1/3, 1/3, 1/3) are in D* but read act.pair at 2/3 - 2/3 = 0, a
    false zero. The max-min: π_a, π_b ≥ t and at most 1 - t, π_c ≥ t/3, and act.pair's distance (1/3 - π_c) × 3 ≥ t;
    with π_a + π_b = 1 - π_c ≤ 1 - t/3 and at least 2t, t = 3/7 at (3/7, 3/7, 1/7), where a, b and c are at t (each
    probe stays at 3/7) and act.pair at 4/7. The second stage fixes act.pair: 2/3 - 6/7 = -4/21, at 4/7. Running it
    once moves an a and a b from the set (1) to it (2/3): it costs 1/3 an execution."""
    status, doc = economy(ex2(tmp_path), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == {"res.a": [UNIT], "res.b": [UNIT], "res.c": [between("0", "1/3")], "res.v": ["1"]}
    assert joint(doc) == EX2_JOINT
    assert Fraction(joint(doc)["activities"]["act.pair"]["value"]) < 0


def test_two_symmetric_degenerate_pools_take_equal_values(tmp_path, economy):
    """a and b come at 1 a day each; act.pair turns one of each into 1 v. By hand: V0 = 1; D* is π_v = 1 and π_a + π_b
    = 1, each at least 0, symmetric in a and b. The max-min (π ≥ t, π ≤ 1 - t, summing to 1) gives t = 1/2 and fixes
    both at 1/2 in one stage."""
    flows = {"act.pair": ({"res.a": "1", "res.b": "1"}, {"res.v": "1"})}
    status, doc = economy(example(tmp_path, ["res.a", "res.b"], {"res.a": "1", "res.b": "1"}, flows), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert joint(doc) == {
        "prices": {"res.a": priced("1/2", UNIT, "max_min"), "res.b": priced("1/2", UNIT, "max_min"), "res.v": EXACT_V},
        "max_min": [{"level": "1/2", "fixed": ["price:res.a", "price:res.b"]}],
        "activities": {"act.pair": judged("0", "0", "1")},
    }


def reversed_arrays(original: Any) -> Any:
    """`_arrays` with its columns and rows given to HiGHS in reverse order: the same program, which the solver then
    walks another way, so that on a degenerate face it may stop at another optimal vertex."""
    import numpy as np
    from scipy import sparse

    def arrays(program: Any) -> Any:
        found = original(program)
        order = np.arange(len(found.columns))[::-1]

        def permuted(matrix: Any, count: int) -> Any:
            if matrix is None:
                return None
            return sparse.csr_array(matrix.toarray()[np.arange(count)[::-1]][:, order])

        return economy_flow._Arrays(
            columns=found.columns[::-1], rows=found.rows[::-1], eq=found.eq[::-1], le=found.le[::-1], c=found.c[order],
            a_eq=permuted(found.a_eq, len(found.eq)), b_eq=found.b_eq[::-1].copy(),
            a_le=permuted(found.a_le, len(found.le)), b_le=found.b_le[::-1].copy(),
            a_all=permuted(found.a_all, len(found.rows)), lower=found.lower[::-1], upper=found.upper[::-1],
            integer=found.integer[::-1],
        )

    return arrays


def test_the_vector_does_not_depend_on_the_dual_point_the_solver_returns(tmp_path, economy, monkeypatch):
    """EX2 twice: as it is, and with every program given to HiGHS in reverse order of its columns and rows, which
    changes the optimal vertex HiGHS stops at on a degenerate face (EX2's D* is a triangle's edge). The selection reads
    only optimal values, never a point HiGHS returns, so the vector and every judgement are equal term by term."""
    status, plain = economy(ex2(tmp_path / "plain"), "--steady-only")
    monkeypatch.setattr(economy_flow, "_arrays", reversed_arrays(economy_flow._arrays))
    turned_status, turned = economy(ex2(tmp_path / "reversed"), "--steady-only")

    assert (status, plain["errors"], turned_status, turned["errors"]) == (0, [], 0, [])
    assert joint(turned) == joint(plain) == EX2_JOINT
    assert steady(turned)["prices"] == steady(plain)["prices"]


# §2.5 test 4: a batch across a threshold ---------------------------------------------------------------------------


def test_the_projection_is_exact_where_the_re_solves_cross_a_kink_and_a_forced_conversion_costs(tmp_path, economy):
    """EX6: s comes at 10.5 a day; act.conv turns one s into one g; act.sinkg turns one g into 2 v, at most 10 a day;
    act.sinkh turns one s into 1 v. By hand: V0 = 10 × 2 + 1/2 = 41/2. D* is one point: π_v = 1, π_s = 1 (act.sinkh's
    dual row, and the cost 21/2 π_s + 10 π_limit = 41/2 with π_limit ≥ 2 - π_g ≥ 1), π_g = 1. The projection is exact,
    1 for s and for g. The re-solves one grain up and down cross the kink at 10 within the grain: one more s goes to
    act.sinkh (1), but one less leaves 9.5 for act.conv, and act.sinkg loses half a run (3/2): [1, 3/2], and so for g
    (one less g needs 11 s for 10 sinks: 3/2). They are a cross-check only (`grain`), which holds the projection.
    act.conv reads 1 - 1 = 0 and runs 10 times in every optimum; one more takes an s from act.sinkh (1) for a g that no
    sink can use: forcing_cost 1, so it is no zero-band conversion. act.sinkh (half a run) costs 1 more the same way;
    act.sinkg is at its limit (input_bound).

    A batch that uses all its input: s comes at 10 a day and act.big turns 10 s into 15 v. By hand: V0 = 15, π_s =
    3/2 exact; act.big reads 15 - 10 × 3/2 = 0, runs once, and the day holds no s for another run (input_bound)."""
    flows = {
        "act.conv": ({"res.s": "1"}, {"res.g": "1"}), "act.sinkg": ({"res.g": "1"}, {"res.v": "2"}),
        "act.sinkh": ({"res.s": "1"}, {"res.v": "1"}),
    }
    status, doc = economy(
        example(tmp_path / "ex6", ["res.g", "res.s"], {"res.s": "10.5"}, flows, {"act.sinkg": "10"}), "--steady-only"
    )

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["objective"] == "41/2"
    assert steady(doc)["prices"] == {"res.g": ["1"], "res.s": ["1"], "res.v": ["1"]}
    assert grain(doc) == {"res.g": [between("1", "3/2")], "res.s": [between("1", "3/2")], "res.v": ["1"]}
    assert joint(doc) == {
        "prices": {"res.g": priced("1", "1", "exact"), "res.s": priced("1", "1", "exact"), "res.v": EXACT_V},
        "max_min": [],
        "activities": {
            "act.conv": judged("0", "0", "10", forcing_cost="1"),
            "act.sinkg": judged("1", "1", "10"),
            "act.sinkh": judged("0", "0", "1/2", forcing_cost="1"),
        },
    }

    flows = {"act.big": ({"res.s": "10"}, {"res.v": "15"})}
    status, doc = economy(example(tmp_path / "batch", ["res.s"], {"res.s": "10"}, flows), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert joint(doc) == {
        "prices": {"res.s": priced("3/2", "3/2", "exact"), "res.v": EXACT_V},
        "max_min": [],
        "activities": {"act.big": judged("0", "0", "1")},
    }


# §2.5 test 5: pools bounded below only ---------------------------------------------------------------------------------


def bounded_below_fixture(folder: Path) -> Path:
    """act.free brings 1 v a day for nothing, at most once. d is a main goal worth 3 that nothing brings in; act.spend
    turns one d into 2 v. p is no goal and nothing brings it in either; act.research turns one p into 5 v."""
    flows = {
        "act.free": ({}, {"res.v": "1"}), "act.research": ({"res.p": "1"}, {"res.v": "5"}),
        "act.spend": ({"res.d": "1"}, {"res.v": "2"}),
    }
    return example(
        folder, ["res.d", "res.p"], {}, flows, {"act.free": "1"},
        goals=[goal("goal.v", "res.v", "1"), goal("goal.d", "res.d", "3")],
    )


BOUNDED_BELOW_JOINT = {
    "prices": {
        "res.d": priced("3", between("3", None), "one_more"),
        "res.p": priced("5", between("5", None), "one_more", {"reason": "no_steady_supply"}),
        "res.v": EXACT_V,
    },
    "max_min": [],
    "activities": {
        "act.free": judged("1", "1", "1"),
        "act.research": judged("0", between(None, "0"), "0"),
        "act.spend": judged("-1", between(None, "-1"), "0"),
    },
}


def test_a_pool_bounded_below_only_takes_its_goals_weight_and_without_a_goal_is_withheld(tmp_path, economy):
    """By hand: V0 = 1 (act.free). Neither d nor p has any supply, so one less of either is infeasible (high null).
    One more d goes to its goal's sink (3, above act.spend's 2): π_d is at least 3 on D* (its sink's dual row), with
    no upper end; step 1 fixes it at 3, its weight, and it is exportable. One more p is one research (5): π_p is at
    least 5, with no upper end; no goal values p, so it has no steady supply: withheld (`no_steady_supply`), and fixed
    last at its low end, 5. No pool is degenerate and bounded, and the activities that no optimum runs read at most -1
    (act.spend, 2 - π_d) or reach 0 only from minus infinity (act.research, 5 - π_p): K is empty. act.free reads 1
    and its limit lets it run no more; act.spend and act.research cannot run at all, having no input (input_bound)."""
    status, doc = economy(bounded_below_fixture(tmp_path), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    prices = {"res.d": [between("3", None)], "res.p": [between("5", None)], "res.v": ["1"]}
    assert steady(doc)["prices"] == grain(doc) == prices
    assert joint(doc) == BOUNDED_BELOW_JOINT


def price_result(doc: dict[str, Any], pool: str) -> dict[str, Any]:
    """What `price:<pool>` shows besides its entry of `joint`: the status and reasons of `price_status`, the price's
    own, which is its static component's (PRICE-2b-2), and `per_account` false."""
    found = doc["data"]["steady"]["price_status"][pool]
    return {"status": found["status"], "reasons": found["reasons"], "per_account": False}


def test_explain_prints_a_price_and_an_activity_value_with_the_steady_states_tree(tmp_path, economy, explain):
    """The graph outputs `price:<resource>` and `activity_value:<activity>` (#424 ACM-PRICE 4.1) of the fixture above:
    each its entry of `joint`, a price with its own status (`price_status`) and an activity's value with the steady
    state's, and the tree of what the steady state read (its flows, limits and objectives in the reference state). An
    id that names no activity is not an output.

    The price's status is its static component's (PRICE-2b-2, `core.coverage`). By hand: act.spend joins d to v,
    act.research p to v, act.free and goal.v are on v and goal.d on d, so all of them are one component. p is taken
    (by act.research), nothing brings it in (no regen, no activity) and it has no goal of its own: it has no steady
    supply, so the component is a structural gap, and d's price is provisional for `no_steady_supply`, as is the
    steady state."""
    folder = bounded_below_fixture(tmp_path)
    _, economy_doc = economy(folder, "--steady-only")

    status, doc = explain(folder, "price:res.d")

    assert (status, doc["errors"]) == (0, [])
    provenance = doc["data"]["provenance"]
    assert (doc["data"]["output"], provenance["output"]) == ("price:res.d", "price:res.d")
    assert price_result(economy_doc, "res.d") == {
        "status": "provisional", "reasons": ["no_steady_supply"], "per_account": False,
    }
    assert provenance["result"] == {**price_result(economy_doc, "res.d"), **BOUNDED_BELOW_JOINT["prices"]["res.d"]}
    nodes = provenance["nodes"]
    assert nodes["price:res.d"] == {"node": "price", "resource": "res.d", "reads": ["economy:steady"]}
    assert nodes["economy:steady"]["node"] == "economy" and nodes["economy:steady"]["file"] == "model.json"
    read = set(nodes["economy:steady"]["reads"])
    assert {
        "reference:flow:flow.spend:consumes:res.d", "reference:flow:flow.spend:produces:res.v",
        "reference:objective:goal.d:value", "reference:rule:limit.free",
    } <= read
    assert nodes["reference:objective:goal.d:value"]["reference"] is True
    assert nodes["reference:objective:goal.d:value"]["ref"].endswith("/logic/value")

    status, doc = explain(folder, "activity_value:act.spend")

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["provenance"]["result"] == {
        "status": steady(economy_doc)["status"], "per_account": False, **BOUNDED_BELOW_JOINT["activities"]["act.spend"]
    }

    status, doc = explain(folder, "activity_value:res.d")

    assert status == CODES["usage_invalid"]["exit"]
    assert [entry["detail"] for entry in doc["errors"]] == [{"reason": "output_unknown", "argument": "activity_value:res.d"}]


# A price without a low end (the coordinator's note of 10-31 15:5x) --------------------------------------------------


def stop_fixture(folder: Path) -> Path:
    """The fixture of test_economy's regen-stop regression (PRICE-2a): r's regen (10 a day) stops at its natural cap
    (reference 50), and its only user, act.p (10 r for some v, a pending flow that reads unk.p), is left out; act.a
    brings 2 v a day for nothing, at most 3 times."""
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


NO_ROOM = {"reason": "no_room", "regen": "regen.r", "stop": "cap.r.natural", "excluded": ["flow.p"]}


def test_a_price_without_a_low_end_is_fixed_at_its_high_end_and_withheld(tmp_path, economy, explain):
    """By hand, one daily step (test_economy's regen-stop regression): r ends the cycle with the stock it began with,
    its regen loses all it brings (10, at most what it brings), and nothing in the program uses r. D*, for r: the
    duals of its balance row (y), its stop row (z ≥ 0) and of its loss's bound (w ≥ 0) with w ≥ z - y, and the cost
    10y + 40z + 10w = 0 (the rest of the cost, act.a's limit, makes V0 = 6): either y ≥ z, so y = z = 0, or y < z,
    so z = 0 and w = -y: y is any value up to 0. One more r has no room (null), one less loses one r less (0): the
    projection is [null, 0], as the re-solves give it. The price is withheld (`no_room`), naming the regen stop and
    the pending flow left out that would use r; free through the selection, it is fixed last at its high end, the
    value of one less unit: 0 (`one_less`, the mirror of step 1). v is exact; act.a reads 2, runs 3 times, and its
    limit lets it run no more. The steady state is provisional (act.p is left out), and so is the price; the plan
    still refuses the regen stop (`while`), so it never reads a price without a low end.

    The price's status is its static component's (PRICE-2b-2): act.p takes r and brings in v, and act.a and goal.v are
    on v, so r, v, act.a and act.p are one component, which holds the gap `excluded_pending` (act.p); nothing in it
    is provisional otherwise (flow.p names unk.p, but a flow left out counts as `excluded_pending` only)."""
    folder = stop_fixture(tmp_path)

    _, economy_doc = economy(folder)

    assert economy_doc["warnings"] == []
    assert all(entry["needed_by"] == ["economy:plan"] for entry in economy_doc["errors"])  # the steady state stands
    assert economy_doc["data"]["plan"] == {"status": "blocked"}
    assert steady(economy_doc)["status"] == "provisional"
    assert steady(economy_doc)["price_status"] == {
        resource: {"status": "provisional", "reasons": ["excluded_pending"]} for resource in ("res.r", "res.v")
    }
    assert steady(economy_doc)["prices"] == grain(economy_doc) == {"res.r": [between(None, "0")], "res.v": ["1"]}
    assert joint(economy_doc) == {
        "prices": {"res.r": priced("0", between(None, "0"), "one_less", NO_ROOM), "res.v": EXACT_V},
        "max_min": [],
        "activities": {"act.a": judged("2", "2", "3")},
    }

    status, doc = explain(folder, "price:res.r")

    assert (status, doc["errors"]) == (0, [])
    provenance = doc["data"]["provenance"]
    assert provenance["result"] == {
        **price_result(economy_doc, "res.r"), **priced("0", between(None, "0"), "one_less", NO_ROOM),
    }
    assert provenance["result"]["status"] == "provisional"  # computed without act.p
    nodes = provenance["nodes"]
    assert {"reference:regen:regen.r:amount", "reference:rule:cap.r.natural", "reference:flow:flow.p:consumes:res.r"} <= set(
        nodes["economy:steady"]["reads"]
    )
    assert (nodes["reference:rule:cap.r.natural"]["ref"], nodes["reference:rule:cap.r.natural"]["reference"]) == (
        "/rules/1/logic/reference", True
    )
    assert nodes["reference:regen:regen.r:amount"]["ref"] == "/rules/0/logic/amount"


# §2.5 test 6: a failed check blocks ----------------------------------------------------------------------------------


def test_a_dual_point_that_fails_its_exact_check_blocks_every_price(tmp_path, economy, explain, monkeypatch):
    """EX5, with one dual coordinate spoilt. The dual's least cost is a + L (the duals of a's balance row, which brings
    1 a day, and of the sink's limit of 1), 2 = V0 at every optimum of it. Here HiGHS's answer for that solve is given
    the limit's dual half a unit higher, and the exact vertex is not rebuilt from it, so the fractions stand as given:
    they still meet every dual row (the limit's dual only adds slack to the sink's), but the cost is 5/2, not V0. The
    strong-duality check refuses it exactly (`lp_numeric_check_failed`, stage `dual`, at `duality:face:0`), which
    blocks the steady state: no price is exportable, `explain` of a price reports the error, and nothing is written.
    (PRICE-2c rebuilds an LP's vertex from HiGHS's basis first, so that solve is given no basis here: its answer is
    rebuilt from the floats as before.)"""
    folder = ex5(tmp_path)
    limit_dual = ("y", "limit:limit.sink:0", 0)
    exact, vertex, certify = economy_flow._Solver.exact, economy_flow._vertex, economy_flow._Solver.vertex

    def least_cost(program: Any) -> bool:  # the dual's least cost: a dual program without the strong-duality row
        keys = {row.key for row in program.rows}
        return economy_flow.FACE not in keys and any(key[0] == "dual" for key in keys)

    def spoilt(solver: Any, program: Any, columns: list, raw: Any, stage: str) -> Any:
        if stage == "dual" and least_cost(program):
            raw = [float(value) + (0.5 if key == limit_dual else 0.0) for key, value in zip(columns, raw, strict=True)]
        return exact(solver, program, columns, raw, stage)

    def without_basis(solver: Any, program: Any, arrays: Any, run: Any, stage: str) -> Any:
        if stage == "dual" and least_cost(program):
            run = replace(run, statuses=None)
        return certify(solver, program, arrays, run, stage)

    monkeypatch.setattr(economy_flow._Solver, "exact", spoilt)
    monkeypatch.setattr(economy_flow._Solver, "vertex", without_basis)
    monkeypatch.setattr(
        economy_flow, "_vertex", lambda program, floats: None if least_cost(program) else vertex(program, floats)
    )
    refused = {
        "code": "lp_numeric_check_failed",
        "detail": {
            "path": model_path(folder), "solve": "steady", "stage": "dual", "constraint": "duality:face:0",
            "found": "5/2", "bound": "=2",
        },
    }

    status, doc = economy(folder, "--steady-only")

    assert status == CODES["lp_numeric_check_failed"]["exit"] == 1
    assert doc["errors"] == [{**refused, "needed_by": ["economy:steady"]}]
    assert doc["data"]["steady"] == {"status": "blocked"}

    status, doc = explain(folder, "price:res.a")

    assert status == 1
    assert doc["errors"] == [{**refused, "needed_by": ["price:res.a"]}]
    assert not (folder / "ours" / "model" / "out").exists()


# §2.5 test 7: a cross-check disagrees ---------------------------------------------------------------------------------


def test_a_marginal_outside_the_projection_is_a_warning_and_the_vector_stands(tmp_path, economy, monkeypatch):
    """EX5, with HiGHS's marginal of a's balance row given as 5 (its marginal -5: a price is the negated marginal of
    a maximisation solved as a minimisation). a's projection is [0, 2], so the cross-check warns (`lp_dual_disagrees`,
    check `marginal`); the price, the vector and the judgements stand as D* gives them."""
    marginals = economy_flow._marginals

    def shifted(section: Any, rows: Any) -> dict:
        found = marginals(section, rows)
        if ("balance", "res.a", 0) in found:
            found[("balance", "res.a", 0)] = -5.0
        return found

    monkeypatch.setattr(economy_flow, "_marginals", shifted)
    folder = ex5(tmp_path)

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == [{
        "code": "lp_dual_disagrees",
        "detail": {
            "path": model_path(folder), "target": "res.a", "step": 0, "check": "marginal", "marginal": "5", "low": "0",
            "high": "2",
        },
    }]
    assert steady(doc)["prices"] == {"res.a": [between("0", "2")], "res.v": ["1"]}
    assert joint(doc) == EX5_JOINT


# The tolerance of the marginal cross-check (MARGINAL_TOLERANCE, τ = 1e-7) -------------------------------------------

# L = 2 + 1/1000003 = 2000007/1000003: an exact price whose denominator, the prime 1000003, no marginal can have (a
# float made a fraction with a denominator of at most lp.max_denominator, 10^6), as the prices of a model whose numbers
# carry four decimals (denominators near 6e28), where a strict comparison warned 59 times on a real model. Every number
# its solves give is 0 or at least 1 (L - 1 = 1 + 1/1000003 is the least), so HiGHS's doubles stand for them, the exact
# vertex rebuilds each from the rows it leaves tight, and every exact check passes. (Not so with a fractional part below
# a double's resolution: 1 + 1/(3 × 10^20) is the double 1.0, the re-solve of one v less runs the goal's sink 1/(3 ×
# 10^20) times, read as 0, and its exact check refuses the steady state, `lp_numeric_check_failed` at balance:res.v:0.)
LARGE = Fraction(2000007, 1000003)
LARGE_JOINT = {
    "prices": {"res.a": priced(str(LARGE), str(LARGE), "exact"), "res.v": EXACT_V},
    "max_min": [],
    "activities": {"act.conv": judged("0", "0", "1")},
}


def large_fixture(folder: Path) -> Path:
    """a comes at 1 a day; act.conv turns one a into 2 + 1/1000003 v (`LARGE`)."""
    flows = {"act.conv": ({"res.a": "1"}, {"res.v": "2 + 1 / 1000003"})}
    return example(folder, ["res.a"], {"res.a": "1"}, flows)


def test_a_marginal_within_the_tolerance_of_a_price_with_a_large_denominator_agrees(tmp_path, economy):
    """By hand: act.conv runs once a day and its L = 2 + 1/1000003 = 2000007/1000003 v go to the goal at 1 each, so
    V0 = L. D* is π_v = 1 (the goal's sink runs in every optimum) and π_a = L·π_v = L (act.conv runs in every optimum,
    so its dual row is tight): a's price is exact, L, and so are its re-solves (one more a is one more conversion,
    2L - L = L; one less one fewer, L - 0 = L); v's re-solves are (L + 1) - L = 1 and L - (L - 1) = 1, the sink
    running L - 1 = 1000004/1000003 times. act.conv reads L·1 - 1·L = 0, runs once and the day brings no more a
    (input_bound).

    HiGHS solves the program in doubles, where L is 2.000000999997 (within 1.4e-16), and its marginal of a's balance
    row is -L, the price L. `limit_denominator(10^6)` cannot give the denominator 1000003: L's continued fraction is
    [2; 1000003], whose quotient 1000003 passes 10^6, so the candidates are 2 and 2 + 1/10^6, and the nearer is the
    marginal m = 2000001/1000000 (for any double from about 5e-7 below L to 3.5e-12 above it). By hand
    m - L = 1/10^6 - 1/1000003 = 3/(10^6 × 1000003) = 3/1000003000000, about 3.0e-12: not 0, so a strict comparison
    would warn (m above `high` L), and at most τ · max(1, L) = L/10^7 = 2000007/10000030000000, about 2.0e-7: within
    τ, so the cross-check agrees and there is no warning."""
    status, doc = economy(large_fixture(tmp_path), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == grain(doc) == {"res.a": ["2000007/1000003"], "res.v": ["1"]}
    marginal = Fraction(doc["data"]["steady"]["marginals"]["prices"]["res.a"][0])
    assert marginal == Fraction(2000001, 1000000) and marginal != LARGE  # a strict comparison would warn
    assert marginal - LARGE == Fraction(3, 1000003000000)
    assert abs(marginal - LARGE) <= economy_flow.MARGINAL_TOLERANCE * max(1, LARGE)
    assert joint(doc) == LARGE_JOINT


def test_a_marginal_beyond_the_tolerance_is_still_a_warning(tmp_path, economy, monkeypatch):
    """The fixture above, with HiGHS's marginal of a's balance row given as -2.000003 (the price 2.000003, the double
    nearest 2000003/1000000, whose denominator is 10^6), so `limit_denominator(10^6)` gives that fraction back: the
    marginal is 2000003/1000000. By hand its distance from a's exact price L = 2000007/1000003 is 3/10^6 - 1/1000003
    = (3 × 1000003 - 10^6)/(10^6 × 1000003), that is 2000009/1000003000000, about 2.0e-6, above τ · max(1, L) = L/10^7
    = 2000007/10000030000000 (about 2.0e-7) by a factor 20000090/2000007, about 10: outside the price by ten times the
    tolerance, so the cross-check still warns (`lp_dual_disagrees`, check `marginal`). The price, the vector and the
    judgements stand as D* gives them."""
    marginals = economy_flow._marginals

    def shifted(section: Any, rows: Any) -> dict:
        found = marginals(section, rows)
        if ("balance", "res.a", 0) in found:
            found[("balance", "res.a", 0)] = -2.000003
        return found

    monkeypatch.setattr(economy_flow, "_marginals", shifted)
    folder = large_fixture(tmp_path)

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == [{
        "code": "lp_dual_disagrees",
        "detail": {
            "path": model_path(folder), "target": "res.a", "step": 0, "check": "marginal",
            "marginal": "2000003/1000000", "low": str(LARGE), "high": str(LARGE),
        },
    }]
    assert steady(doc)["prices"] == {"res.a": [str(LARGE)], "res.v": ["1"]}
    assert joint(doc) == LARGE_JOINT


# PRICE-2c: complementary slackness, and a probe that cannot be verified ----------------------------------------------


def surplus_fixture(folder: Path) -> Path:
    """s comes at 105 a day. act.get turns 10 s into 10 g (the main goal, worth 1), at most 10 a day; act.train turns 10 s
    into 1 m, act.waste 10 s into 1/10 m; the demand row `m` takes 1 m for 1 point, a point worth 1/2, at most 100 a
    day."""
    return model(
        folder, [DAY],
        [resource("res.g", "fact.g"), resource("res.s", "fact.s"), resource("res.m", "fact.m"), activity("act.get"),
         activity("act.train"), activity("act.waste")],
        [
            regen("regen.s", "res.s", "105", "1d"),
            flow("flow.get", "act.get", consumes={"res.s": "10"}, produces={"res.g": "10"}),
            limit("limit.get", "act.get", "10", "day"),
            flow("flow.train", "act.train", consumes={"res.s": "10"}, produces={"res.m": "1"}),
            flow("flow.waste", "act.waste", consumes={"res.s": "10"}, produces={"res.m": "0.1"}),
        ],
        {"fact.g": 0, "fact.s": 0, "fact.m": 0},
        goals=[goal("goal.g", "res.g", "1"), training("0.5", "1d")], demand=[("m", "100", "1", "0")],
        costs=[("m", {"res.m": "1"})],
    )


def test_complementary_slackness_spares_the_probes_of_an_activity_no_optimum_runs(tmp_path, economy, monkeypatch):
    """PRICE-2c (`core.flow._never`). By hand: act.get runs its 10 a day (100 s for 100 g: A* = 100) and act.train the 5
    s left (half a run: half an m, which the demand row takes for a quarter of a value unit): V0 = V* = 401/4. The
    optimum is unique, and so is D*: π_g = 1, π_m = 1/2 (the demand row runs half a time, below its bound), π_s = π_m /
    10 = 1/20 (act.train runs), limit.get's dual 10 - 10 × 1/20 = 19/2. act.waste's dual slack is 10 × 1/20 - 1/10 ×
    1/2 = 9/20 > 0, so by complementary slackness no optimum of S3 runs it, and none of S2: neither a tie probe nor a
    probe of its `most` is solved. Its judgements still read exactly: it is worth 1/10 × 1/2 - 10 × 1/20 = -9/20 on all
    of D*, runs 0 times, and forced to run it loses its slack, 9/20 an execution. act.get and act.train run in the
    optimum (slack 0), so `most` probes them; the optimum of S2 runs both, so no tie probe is needed."""
    probes = []
    probe = economy_flow._Solver.probe

    def recorded(solver: Any, program: Any, stage: str, **options: Any) -> Any:
        if stage in ("tie", "judgement"):
            probes.append((stage, sorted({key[1] for key in program.objective if key[0] == "x"})))
        return probe(solver, program, stage, **options)

    monkeypatch.setattr(economy_flow._Solver, "probe", recorded)

    status, doc = economy(surplus_fixture(tmp_path), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert (steady(doc)["objective"], steady(doc)["acquisition"]) == ("401/4", "100")
    assert steady(doc)["prices"] == {"res.g": ["1"], "res.m": ["1/2"], "res.s": ["1/20"]}
    assert probes == [("judgement", ["act.get"]), ("judgement", ["act.train"])]
    assert joint(doc)["activities"]["act.waste"] == judged("-9/20", "-9/20", "0", forcing_cost="9/20")


def test_a_judgement_probe_that_cannot_be_verified_is_unverified_and_a_structural_gap(tmp_path, economy, monkeypatch):
    """PRICE-2c. The fixture above, with HiGHS's basis withheld from the probe of the most an optimum runs act.train, so
    that its answer (1/2, rebuilt from the rows its variables leave tight) is feasible but not certified optimal. It is
    not guessed: `most` and `in_optimum` are null, and so are `forcing_cost` and `input_bound`, which read `most`; the
    activity lists both as `unverified`, and its value and range, which do not read it, stand. The warning
    `lp_probe_unverified` names the probe and why (`no_basis`). act.train's static component, which holds every pool, is
    a structural gap (`probe_unverified`), so every price is provisional, and so is the steady state, which is not
    blocked: the price vector is the one the plain fixture gives."""
    certify = economy_flow._Solver.vertex

    def without_basis(solver: Any, program: Any, arrays: Any, run: Any, stage: str) -> Any:
        if stage == "judgement" and set(program.objective) == {("x", "act.train", 0)}:
            run = replace(run, statuses=None)
        return certify(solver, program, arrays, run, stage)

    plain_status, plain = economy(surplus_fixture(tmp_path / "plain"), "--steady-only")
    monkeypatch.setattr(economy_flow._Solver, "vertex", without_basis)
    folder = surplus_fixture(tmp_path / "unverified")

    status, doc = economy(folder, "--steady-only")

    assert (plain_status, plain["warnings"], status, doc["errors"]) == (0, [], 0, [])
    assert doc["warnings"] == [{
        "code": "lp_probe_unverified",
        "detail": {
            "path": model_path(folder), "probe": "most", "activity": "act.train", "stage": "judgement",
            "reason": "no_basis",
        },
    }]
    assert joint(doc)["activities"]["act.train"] == {
        "value": "0", "range": "0", "in_optimum": None, "most": None, "forcing_cost": None, "input_bound": None,
        "unverified": ["most", "forcing_cost"],
    }
    assert joint(plain)["activities"]["act.train"]["unverified"] == []
    assert joint(doc)["prices"] == joint(plain)["prices"]
    found = doc["data"]["steady"]
    assert found["coverage"]["components"]["act.get"]["gaps"] == {"probe_unverified": ["act.train"]}
    assert found["price_status"] == {
        resource: {"status": "provisional", "reasons": ["probe_unverified"]} for resource in ("res.g", "res.m", "res.s")
    }
    assert found["status"] == "provisional"


# PRICE-2f: `economy --steady-only --quick`, the quick look ----------------------------------------------------------


QUICK = ("--steady-only", "--quick")


def economy_work(doc: dict[str, Any]) -> dict[str, int]:
    return doc["data"]["work"]["by_kind"]["economy"]


def test_the_quick_look_shows_its_own_member_and_stops_before_the_later_stages(tmp_path, economy, monkeypatch):
    """The fixture of the tie (test_economy.py): by hand V0 = A* = 100, act.train a tie, and s worth 1, g 1, m 10, one
    value each, so its one optimal dual is known. What `quick` shares with `steady` is as `steady` writes it, the tie
    warning is there, and no later stage runs. A later stage's `probe_unverified` is named as not known, not absent."""
    folder = shared_fixture(tmp_path, "10", most="10")
    with monkeypatch.context() as patched:
        for stage in ("hold", "chords", "price", "joint"):
            patched.setattr(economy_flow._Solver, stage, lambda *args, **kwargs: pytest.fail("a later stage ran"))
        status, doc = economy(folder, *QUICK)

    assert (status, doc["errors"]) == (0, [])
    assert sorted(doc["data"]) == ["model_id", "path", "quick", "run_id", "state", "work"]
    found = doc["data"]["quick"]
    assert sorted(found) == [
        "acquisition", "conversions", "coverage", "coverage_unknown", "dual_prices", "excluded", "losses", "objective",
        "protected", "relaxed", "status",
    ]
    assert (found["status"], found["objective"], found["acquisition"]) == ("quick", "100", "100")
    assert found["dual_prices"] == {"res.g": ["1"], "res.m": ["10"], "res.s": ["1"]}
    assert found["coverage_unknown"] == ["probe_unverified"]
    assert found["coverage"]["components"]["act.get"]["gaps"] == {"objective_tier_tie": ["act.train"]}

    _, full = economy(folder, "--steady-only")

    whole = full["data"]["steady"]
    for member in ("objective", "acquisition", "protected", "conversions", "losses", "excluded", "relaxed", "coverage"):
        assert found[member] == whole[member], member
    assert doc["warnings"] == full["warnings"] == [
        {"code": "objective_tier_tie", "detail": {"path": model_path(folder), "activity": "act.train"}}
    ]


def test_the_quick_looks_dual_is_one_point_of_the_interval(tmp_path, economy):
    """EX1: D* is π_v = 1 and π_a + π_b + π_c = 1, each at least 0, so each interval is [0, 1], which the full run
    shows. The quick look shows one point of D*, whichever HiGHS ends at: it meets those conditions, and no interval."""
    folder = ex1(tmp_path)

    status, doc = economy(folder, *QUICK)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    point = doc["data"]["quick"]["dual_prices"]
    assert sorted(point) == ["res.a", "res.b", "res.c", "res.v"]
    assert all(len(steps) == 1 and isinstance(steps[0], str) for steps in point.values())
    values = {pool: Fraction(steps[0]) for pool, steps in point.items()}
    assert values["res.v"] == 1 and min(values.values()) >= 0
    assert values["res.a"] + values["res.b"] + values["res.c"] == 1


def test_a_blocked_quick_look_blocks_as_the_steady_state_does(tmp_path, economy):
    """The conflict of test_economy.py (V0 200 > V* 150): the same error, needed by `economy:quick`."""
    folder = shared_fixture(tmp_path, "20", income="50")

    status, doc = economy(folder, *QUICK)

    assert status == CODES["objective_tier_conflict"]["exit"]
    assert doc["data"]["quick"] == {"status": "blocked"} and "steady" not in doc["data"]
    assert [(entry["code"], entry["needed_by"]) for entry in doc["errors"]] == [("objective_tier_conflict", ["economy:quick"])]


@pytest.mark.parametrize(
    "extra",
    [("economy", "M", "--state", "S", "--quick"), ("run", "M", "--state", "S", "--quick"), ("check", "M", "--quick")],
    ids=["economy_without_steady_only", "run", "check"],
)
def test_quick_anywhere_else_is_refused_as_a_bad_argument(make_system, app_dir, extra):
    system = make_system(prefix=app_dir / "venv")

    status = cli.main(list(extra), system)

    doc = json.loads(system.stdout.getvalue())
    assert status == CODES["usage_invalid"]["exit"]
    assert [(entry["code"], entry["detail"]) for entry in doc["errors"]] == [
        ("usage_invalid", {"reason": "argument_unexpected", "argument": "--quick"})
    ]


def test_a_quick_run_neither_fills_nor_reuses_the_steady_state(tmp_path, economy):
    """Own node, own key, own run id: after a quick run a full run computes the steady state, to what a clean cache
    gives it; the quick look is reused by itself only, and a steady state in the cache gives it nothing."""
    folder = ex2(tmp_path / "fixture")
    _, clean = economy(folder, "--steady-only", paths={"state_root": str(tmp_path / "clean")})

    _, quick = economy(folder, *QUICK, paths={"state_root": str(tmp_path / "shared")})
    assert economy_work(quick) == {"computed": 1, "reused": 0, "cutoff": 0}
    assert quick["data"]["run_id"] != clean["data"]["run_id"]

    _, full = economy(folder, "--steady-only")
    assert economy_work(full) == {"computed": 1, "reused": 0, "cutoff": 0}
    assert full["data"]["run_id"] == clean["data"]["run_id"]
    assert full["data"]["steady"] == clean["data"]["steady"]
    assert "quick" not in full["data"]

    _, again = economy(folder, *QUICK)
    assert economy_work(again) == {"computed": 0, "reused": 1, "cutoff": 0}
    assert again["data"]["quick"] == quick["data"]["quick"]

    _, first = economy(folder, *QUICK, paths={"state_root": str(tmp_path / "clean")})  # it holds a steady state only
    assert economy_work(first) == {"computed": 1, "reused": 0, "cutoff": 0}


def test_an_export_and_explain_after_a_quick_run_behave_as_if_it_never_ran(tmp_path, economy, explain, make_system, app_dir):
    """An export reads no cache and delivers no economy output: a template that names a price is refused, with no steady
    state as after a quick run, by the same error. `explain` computes the steady state itself, from no cache either."""
    folder = ex2(tmp_path / "fixture")
    base = folder / "ours" / "model"
    placeholder = {"$model": "price:res.a", "scale": 1000, "round": "floor"}
    write(base / "scoring.template.json", {"schema_version": "example.selection.v1", "weight_milli": placeholder})
    exports = json.loads((base / "exports.json").read_text(encoding="utf-8"))
    write(base / "exports.json", {**exports, "scoring": [{"id": "pick", "template": "scoring.template.json"}]})

    def export() -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main(["export", "scoring", str(base), "--state", str(folder / "state.json")], system)
        return status, json.loads(system.stdout.getvalue())

    before, explained = export(), explain(folder, "price:res.a")

    status, doc = economy(folder, *QUICK, paths={"state_root": str(tmp_path / "state")})
    assert (status, doc["data"]["quick"]["status"]) == (0, "quick")

    assert export() == before
    assert before[0] == CODES["model_file_invalid"]["exit"]
    assert [(entry["code"], entry["detail"]["literal"]) for entry in before[1]["errors"]] == [
        ("model_file_invalid", "price:res.a")
    ]
    assert explain(folder, "price:res.a") == explained
    assert explained[0] == 0
    assert "economy:quick" not in explained[1]["data"]["provenance"]["nodes"]
