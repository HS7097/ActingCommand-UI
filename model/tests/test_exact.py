"""#424 PRICE-2c: exact vertices from the solver's basis (`core.exact`), the bounded exact pivoting fallback, and a
synthetic regression fixture of the shape of a large model (about 110 activities, numbers with four decimals, several
resources with only some activities in the optimum), whose time CI bounds.

The pivoting tests give `exact.optimum` a basis by hand, as HiGHS's statuses would give it, on one small program worked
out below; the fixture runs `acmodel economy` and checks its result against the optimum computed here exactly."""

from __future__ import annotations

import time
from fractions import Fraction
from pathlib import Path

from acmodel.core import exact
from test_economy import (
    DAY,
    activity,
    economy,  # noqa: F401 (the fixture `acmodel economy`)
    flow,
    goal,
    limit,
    model,
    regen,
    resource,
    steady,
)

F = Fraction
L, B, U = exact.LOWER, exact.BASIC, exact.UPPER

# maximise 3x + 2y over x + y ≤ 4 (row 0), x + 3y ≤ 9 (row 1), 0 ≤ x ≤ 3, y ≥ 0. By hand: x = 3 at its bound and y = 1
# fill row 0; row 1 has slack (6 < 9). The optimum is 11. Its basis: y and row 1's logical are basic; x is at its upper
# bound and row 0 at its upper end. The duals: y's column gives y0 + 3·y1 = 2 and row 1 is basic (y1 = 0), so y0 = 2;
# x's reduced cost 3 - y0 = 1 ≥ 0 at its upper bound, and row 0's, y0 = 2 ≥ 0 at its upper end: exactly optimal.
PROBLEM = exact.Problem(
    cost=(F(3), F(2)),
    lower=(F(0), F(0), None, None),
    upper=(F(3), None, F(4), F(9)),
    rows=({0: F(1), 1: F(1)}, {0: F(1), 1: F(3)}),
)
OPTIMUM = ((F(3), F(1)), (F(2), F(0)))


def test_the_solve_is_exact_and_refuses_a_singular_system():
    """x + y = 3, 2x - z = 0, y + z = 7/2: by hand z = 2x, y = 3 - x, 3 - x + 2x = 7/2, so x = 1/2, y = 5/2, z = 1. A
    system whose second equation is twice its first has no single solution."""
    found = exact.solve([{"x": F(1), "y": F(1)}, {"x": F(2), "z": F(-1)}, {"y": F(1), "z": F(1)}], [F(3), F(0), F(7, 2)],
                        ["x", "y", "z"])
    assert found == {"x": F(1, 2), "y": F(5, 2), "z": F(1)}
    assert exact.solve([{"x": F(1), "y": F(1)}, {"x": F(2), "y": F(2)}], [F(1), F(2)], ["x", "y"]) is None


def test_an_optimal_basis_gives_the_exact_vertex_and_its_duals_with_no_pivot():
    found = exact.optimum(PROBLEM, (U, B, U, B), cap=20)
    assert found == exact.Vertex(*OPTIMUM, pivots=0)


def test_a_feasible_basis_that_is_not_optimal_takes_primal_pivots():
    """From the slack basis (x = y = 0, both rows basic): x enters (the least index that improves) and reaches its
    upper bound 3 before row 0 fills (a bound flip, pivot 1); then y enters and row 0 leaves at 4 (y = 1; pivot 2)."""
    assert exact.optimum(PROBLEM, (L, L, B, B), cap=20) == exact.Vertex(*OPTIMUM, pivots=2)


def test_a_dual_feasible_basis_that_is_not_feasible_takes_a_dual_pivot():
    """x basic with row 0 at its end gives x = 4, above its bound 3; the duals (y0 = 3, y1 = 0) leave y's reduced cost
    2 - 3 = -1 at its lower bound and row 0's 3 at its upper end: dual feasible. x leaves at 3, y enters (the least
    ratio, |-1|/1 against row 0's 3/1): the optimum in one pivot."""
    assert exact.optimum(PROBLEM, (B, L, U, B), cap=20) == exact.Vertex(*OPTIMUM, pivots=1)


def test_the_pivots_stop_at_their_cap_and_a_basis_must_have_one_basic_variable_per_row():
    assert exact.optimum(PROBLEM, (L, L, B, B), cap=1) == exact.Refused(exact.PIVOT_CAP, 1)
    assert exact.optimum(PROBLEM, (L, L, B, B), cap=0) == exact.Refused(exact.PIVOT_CAP, 0)
    assert exact.optimum(PROBLEM, (B, B, B, U), cap=20) == exact.Refused(exact.BASIS_INVALID, 0)
    assert exact.optimum(PROBLEM, (U, B, L, B), cap=20) == exact.Refused(exact.BASIS_INVALID, 0)  # row 0 has no lower end


def test_a_basis_proves_on_which_sides_it_stays_optimal_when_a_right_side_moves():
    """maximise x over x + y = s (an equality), 0 ≤ x ≤ 3, y ≥ 0. With s = 4 the optimum is x = 3 at its bound and y =
    1 basic; y's column gives the row's dual 0 (x's reduced cost 1 ≥ 0 at its upper bound). Moving s moves y alone,
    one for one: by 2 up y is 3, by 2 down -1 (infeasible), a little either way stays feasible. With s = 3, y = 0 is
    basic at its bound: up still holds, down not even a little."""
    four = exact.Problem(cost=(F(1), F(0)), lower=(F(0), F(0), F(4)), upper=(F(3), None, F(4)), rows=({0: F(1), 1: F(1)},))
    vertex = exact.optimum(four, (U, B, U), cap=20)
    assert (vertex.x, vertex.duals) == ((F(3), F(1)), (F(0),))
    assert exact.ranging(four, vertex, 0, F(2)) == (True, False, True, True)
    three = exact.Problem(cost=(F(1), F(0)), lower=(F(0), F(0), F(3)), upper=(F(3), None, F(3)), rows=({0: F(1), 1: F(1)},))
    vertex = exact.optimum(three, (U, B, L), cap=20)
    assert vertex.x == (F(3), F(0))
    assert exact.ranging(three, vertex, 0, F(1)) == (True, False, True, False)


# The regression fixture --------------------------------------------------------------------------------------------

ACTIVITIES = 110
GOALS = ("1.2345", "0.9876", "1.5021", "1.1107")  # the value of one unit of res.m1 .. res.m4
SUPPLY = 1000  # res.s a day
MOST = 5  # a day, for each limited activity
SECONDS = 120  # CI's bound on the run, in seconds (the fixture takes a few seconds; the bound catches a blowup)


def _decimal(units: int) -> str:
    """A number of ten-thousandths, written with four decimals."""
    return f"{units // 10000}.{units % 10000:04d}"


def _shape() -> list[tuple[str, int, int, int, bool]]:
    """Each activity: (id, its material 1-4, the s one execution consumes and the material it produces, both in
    ten-thousandths, and whether a daily limit counts it). Every fifth activity is limited and converts s at a better
    rate than any unlimited one: its s at most 1.4999, its yield at least 2, against at least 2 s for at most 0.8999."""
    found = []
    for index in range(ACTIVITIES):
        limited = index % 5 == 0
        if limited:
            consumed, produced = 10000 + index * 37 % 5000, 20000 + index * 53 % 9000
        else:
            consumed, produced = 20000 + index * 71 % 9000, 5000 + index * 89 % 4000
        found.append((f"act.p{index:03d}", index % 4 + 1, consumed, produced, limited))
    return found


def regression_fixture(folder: Path) -> Path:
    """res.s comes at 1000 a day; res.m1 .. res.m4 are main goals worth `GOALS`; each of the 110 activities turns some s
    into one material (`_shape`). The 22 limited ones (at most 5 a day) are protected acquisitions; the others are
    conversions."""
    entities = [resource("res.s", "fact.s"), *(resource(f"res.m{k}", f"fact.m{k}") for k in range(1, 5))]
    rules = [regen("regen.s", "res.s", str(SUPPLY), "1d")]
    for name, material, consumed, produced, limited in _shape():
        entities.append(activity(name))
        rules.append(flow(
            "flow." + name[4:], name, consumes={"res.s": _decimal(consumed)},
            produces={f"res.m{material}": _decimal(produced)},
        ))
        if limited:
            rules.append(limit("limit." + name[4:], name, str(MOST), "day"))
    goals = [goal(f"goal.m{k}", f"res.m{k}", GOALS[k - 1]) for k in range(1, 5)]
    readings = {"fact.s": 0, **{f"fact.m{k}": 0 for k in range(1, 5)}}
    return model(folder, [DAY], entities, rules, readings, goals=goals)


def test_a_large_model_with_four_decimals_is_solved_exactly_and_in_bounded_time(tmp_path, economy):
    """By hand, a one-day cycle. One execution of an activity is worth w·b for the s a it consumes (w its material's
    goal). Every limited activity is worth more per unit of s than any unlimited one (at least 0.9876 × 2 / 1.4999,
    against at most 1.5021 × 0.8999 / 2), so each runs its 5 a day, and all the s left goes to the unlimited activity
    with the best rate ρ* = max w·b/a (one, checked below). So V0 = Σ_limited 5·w·b + (1000 - Σ_limited 5·a)·ρ*, the
    acquisition A* is the limited activities' part, and V* = V0. A unit of s is worth ρ* and one of each material its
    goal's value. The optimum runs exactly the 22 limited activities and the best unlimited one; every other activity
    has a reduced cost below 0 (worth less than ρ* per unit of s), so complementary slackness spares its probes, and
    every probe left is verified: no warning. The run takes under `SECONDS` on CI."""
    folder = regression_fixture(tmp_path)
    shape = _shape()
    worth = {k: Fraction(GOALS[k - 1]) for k in range(1, 5)}
    rates = {
        name: worth[material] * Fraction(produced, 10000) / Fraction(consumed, 10000)
        for name, material, consumed, produced, limited in shape if not limited
    }
    best = max(rates.values())
    [marginal] = [name for name, rate in rates.items() if rate == best]
    limited = [(name, material, consumed, produced) for name, material, consumed, produced, bound in shape if bound]
    acquired = sum((MOST * worth[material] * Fraction(produced, 10000) for _, material, _, produced in limited), F(0))
    left = SUPPLY - sum((MOST * Fraction(consumed, 10000) for _, _, consumed, _ in limited), F(0))

    started = time.perf_counter()
    status, doc = economy(folder, "--steady-only")
    elapsed = time.perf_counter() - started

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = steady(doc)
    assert (found["objective"], found["acquisition"]) == (str(acquired + left * best), str(acquired))
    assert found["prices"] == {"res.s": [str(best)], **{f"res.m{k}": [str(worth[k])] for k in range(1, 5)}}
    activities = doc["data"]["steady"]["joint"]["activities"]
    assert {name for name, judged in activities.items() if judged["in_optimum"]} == {
        *(name for name, _, _, _ in limited), marginal,
    }
    assert all(judged["unverified"] == [] for judged in activities.values())
    assert elapsed < SECONDS, elapsed
