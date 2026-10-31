"""Exact vertices of a linear program from the solver's basis, with a bounded exact pivoting fallback (#424 PRICE-2c;
contracts/formats.md, "Exactness and failures").

The program is taken in the form HiGHS solves: maximise c·x over the rows L_i ≤ a_i·x ≤ U_i and the bounds
l_j ≤ x_j ≤ u_j, an end None where it is unbounded. Each row i has a logical variable r_i = a_i·x between the row's
bounds, so the constraints are A·x − r = 0 over n + m variables: the n structural ones (the columns), then the m
logical ones (the rows), in that order.

**A basis** is m of them, the basic ones; each other one is nonbasic, fixed at a bound. It comes from HiGHS's column
and row basis statuses (`HighsBasisStatus`: `kLower`, `kBasic`, `kUpper`, `kZero` for a free variable at 0,
`kNonbasic`), never from values close to a bound: a nonbasic column is at its lower or upper bound, and a nonbasic row
is tight at its lower or upper end. The basic variables are then solved exactly, in fractions, from B·z_B = −N·z_N
(`solve`, a sparse elimination), and the row duals from Bᵀ·y = c_B.

**The certificate.** The basis is exactly optimal when both hold exactly:
- primal feasibility: every basic variable is within its bounds;
- dual feasibility: the reduced cost d_k = c_k − (column k)·y of each nonbasic variable has the sign of an optimum (a
  maximum): at most 0 where it could rise, at least 0 where it could fall; a fixed variable may have either.
Then the vertex x is an optimum and y an optimal dual, so their objectives agree exactly (weak duality is then strong
duality). Nothing is compared with a tolerance.

**The fallback.** Where HiGHS's basis is not exactly optimal (its tolerances are 1e-7), at most `cap` exact pivots
(`lp.max_pivots`): a primal simplex step when the basis is primal feasible, a dual simplex step when it is dual
feasible, each with Bland's rule (the least index enters, and leaves among ties), every quantity re-solved exactly from
the new basis. A basis that is neither, singular, or not optimal after `cap` pivots is refused (`Refused`), and the
caller decides what that means (`core.flow`: the old reconstruction and the exact check for a solve, "unverified" for a
probe)."""

from __future__ import annotations

import heapq
from dataclasses import dataclass
from fractions import Fraction
from typing import Hashable, Sequence

# HiGHS's HighsBasisStatus values (scipy's bundled HiGHS, `scipy.optimize._highspy._core.HighsBasisStatus`).
LOWER, BASIC, UPPER, ZERO, NONBASIC = 0, 1, 2, 3, 4

# Why a basis gives no certified vertex (`Refused.reason`).
BASIS_INVALID = "basis_invalid"  # not m basic variables, or a nonbasic one at an end that is unbounded
SINGULAR = "singular"  # B has no inverse
PIVOT_CAP = "pivot_cap"  # still not optimal after `cap` pivots
NOT_FEASIBLE = "not_feasible"  # neither primal nor dual feasible: no simplex step of either kind applies
UNBOUNDED = "unbounded"  # a primal step finds no bound in its direction
INFEASIBLE = "infeasible"  # a dual step finds no variable to enter


@dataclass(frozen=True)
class Problem:
    """maximise Σ cost_j·x_j over L_i ≤ Σ_j rows_i[j]·x_j ≤ U_i and lower_j ≤ x_j ≤ upper_j. `lower` and `upper`
    hold the n columns' bounds and then the m rows' ends, None where unbounded."""

    cost: tuple[Fraction, ...]
    lower: tuple[Fraction | None, ...]
    upper: tuple[Fraction | None, ...]
    rows: tuple[dict[int, Fraction], ...]


@dataclass(frozen=True)
class Vertex:
    """An exactly optimal basic solution: the structural values `x`, the row duals `duals` (y, in the program's
    maximising sense: Σ_i a_ij·y_i = c_j for a basic column) and the pivots it took from HiGHS's basis."""

    x: tuple[Fraction, ...]
    duals: tuple[Fraction, ...]
    pivots: int


@dataclass(frozen=True)
class Refused:
    """No certified vertex: why (`reason`), after how many pivots."""

    reason: str
    pivots: int


def solve(
    equations: Sequence[dict[Hashable, Fraction]], rhs: Sequence[Fraction], unknowns: Sequence[Hashable]
) -> dict[Hashable, Fraction] | None:
    """The one solution of a square sparse linear system, exactly; None when it has none or more than one.

    Sparse Gaussian elimination over fractions. Each step takes an unknown that only one remaining equation holds (a
    column singleton: no fill), else the shortest remaining equation and in it the unknown that the fewest equations
    hold (Markowitz's choice; ties by order of `unknowns`); it eliminates that unknown from the other equations. Then
    back substitution, last pivot first. Every coefficient is exact; a coefficient that cancels is dropped."""
    size = len(unknowns)
    if len(equations) != size:
        return None
    rank = {unknown: position for position, unknown in enumerate(unknowns)}
    rows: list[dict[Hashable, Fraction]] = []
    values: list[Fraction] = []
    holders: dict[Hashable, set[int]] = {unknown: set() for unknown in unknowns}
    for index, equation in enumerate(equations):
        row = {unknown: coefficient for unknown, coefficient in equation.items() if coefficient != 0}
        for unknown in row:
            if unknown not in holders:
                raise ValueError(f"an equation holds {unknown!r}, which is not an unknown")
            holders[unknown].add(index)
        rows.append(row)
        values.append(Fraction(rhs[index]))
    active = set(range(size))
    shortest = [(len(row), index) for index, row in enumerate(rows)]
    heapq.heapify(shortest)
    singles = [unknown for unknown in unknowns if len(holders[unknown]) == 1]
    done: set[Hashable] = set()
    order: list[tuple[int, Hashable]] = []
    while active:
        pivot: tuple[int, Hashable] | None = None
        while singles and pivot is None:
            unknown = singles.pop()
            if unknown not in done and len(holders[unknown]) == 1:
                pivot = (next(iter(holders[unknown])), unknown)
        if pivot is None:
            while shortest:
                length, index = heapq.heappop(shortest)
                if index in active and length == len(rows[index]):
                    break
            else:
                return None
            row = rows[index]
            if not row:
                return None  # a remaining equation with no unknown left: the system is singular
            unknown = min(row, key=lambda name: (len(holders[name]), rank[name]))
            pivot = (index, unknown)
        index, unknown = pivot
        row, value = rows[index], values[index]
        coefficient = row[unknown]
        for other in sorted(holders[unknown] - {index}):
            target = rows[other]
            factor = target.pop(unknown) / coefficient
            for name, amount in row.items():
                if name == unknown:
                    continue
                updated = target.get(name, Fraction(0)) - factor * amount
                if updated:
                    if name not in target:
                        holders[name].add(other)
                    target[name] = updated
                elif name in target:
                    del target[name]
                    holders[name].discard(other)
                    if len(holders[name]) == 1 and name not in done:
                        singles.append(name)
            values[other] -= factor * value
            heapq.heappush(shortest, (len(target), other))
        holders[unknown] = set()
        active.discard(index)
        done.add(unknown)
        for name in row:
            if name != unknown:
                holders[name].discard(index)
                if len(holders[name]) == 1:
                    singles.append(name)
                elif not holders[name] and name not in done:
                    return None  # an unknown that no remaining equation holds: the system is singular
        order.append((index, unknown))
    found: dict[Hashable, Fraction] = {}
    for index, unknown in reversed(order):
        row = rows[index]
        rest = sum((amount * found[name] for name, amount in row.items() if name != unknown), Fraction(0))
        found[unknown] = (values[index] - rest) / row[unknown]
    return found


def _start(problem: Problem, statuses: Sequence[int]) -> tuple[list[int], dict[int, Fraction]] | None:
    """The basic variables and the nonbasic ones' values, from HiGHS's statuses (columns, then rows); None when they
    are not a basis of m variables or a nonbasic one sits at an end that is unbounded."""
    m = len(problem.rows)
    basic = [k for k, status in enumerate(statuses) if status == BASIC]
    if len(basic) != m or len(statuses) != len(problem.lower):
        return None
    at: dict[int, Fraction] = {}
    for k, status in enumerate(statuses):
        if status == BASIC:
            continue
        low, high = problem.lower[k], problem.upper[k]
        if status == LOWER:
            end = low
        elif status == UPPER:
            end = high
        elif status == ZERO:
            end = Fraction(0) if (low is None or low <= 0) and (high is None or high >= 0) else None
        else:  # kNonbasic: at a finite end, else free at 0
            end = low if low is not None else high if high is not None else Fraction(0)
        if end is None:
            return None
        at[k] = end
    return basic, at


def optimum(problem: Problem, statuses: Sequence[int], cap: int) -> Vertex | Refused:
    """The exactly optimal vertex of `problem` from HiGHS's basis `statuses` (one per column, then one per row), with
    at most `cap` exact pivots (module docstring); `Refused` when there is none."""
    n, m = len(problem.cost), len(problem.rows)
    columns: list[dict[int, Fraction]] = [{} for _ in range(n)]
    for i, row in enumerate(problem.rows):
        for j, coefficient in row.items():
            if coefficient:
                columns[j][i] = coefficient
    columns.extend({i: Fraction(-1)} for i in range(m))  # the logical variable of row i: A·x − r = 0
    cost = [*problem.cost, *([Fraction(0)] * m)]
    lower, upper = problem.lower, problem.upper
    started = _start(problem, statuses)
    if started is None:
        return Refused(BASIS_INVALID, 0)
    basic, at = started
    everything = range(m)
    pivots = 0
    while True:
        b_rows: list[dict[int, Fraction]] = [{} for _ in everything]
        for k in basic:
            for i, coefficient in columns[k].items():
                b_rows[i][k] = coefficient
        rhs = [Fraction(0)] * m
        for k, value in at.items():
            if value:
                for i, coefficient in columns[k].items():
                    rhs[i] -= coefficient * value
        z_basic = solve(b_rows, rhs, basic)
        if z_basic is None:
            return Refused(SINGULAR, pivots)
        z = {**at, **z_basic}
        dual_equations = [columns[k] for k in basic]
        y = solve(dual_equations, [cost[k] for k in basic], list(everything))
        if y is None:
            return Refused(SINGULAR, pivots)

        def reduced(k: int) -> Fraction:
            return cost[k] - sum((coefficient * y[i] for i, coefficient in columns[k].items()), Fraction(0))

        def rises(k: int) -> bool:  # a nonbasic variable that could increase
            return upper[k] is None or at[k] < upper[k]

        def falls(k: int) -> bool:
            return lower[k] is None or at[k] > lower[k]

        primal = [
            k for k in sorted(basic)
            if (lower[k] is not None and z[k] < lower[k]) or (upper[k] is not None and z[k] > upper[k])
        ]
        costs = {k: reduced(k) for k in at}
        dual = [k for k in sorted(at) if (rises(k) and costs[k] > 0) or (falls(k) and costs[k] < 0)]
        if not primal and not dual:
            return Vertex(tuple(z[j] for j in range(n)), tuple(y[i] for i in everything), pivots)
        if pivots >= cap:
            return Refused(PIVOT_CAP, pivots)
        position = {k: p for p, k in enumerate(basic)}
        if not primal:  # a primal simplex step: the least index whose reduced cost improves enters
            entering = dual[0]
            sign = 1 if rises(entering) and costs[entering] > 0 else -1
            alpha = solve(b_rows, [columns[entering].get(i, Fraction(0)) for i in everything], basic)
            if alpha is None:
                return Refused(SINGULAR, pivots)
            best: tuple[Fraction, int, int] | None = None  # (step, leaving variable, the bound it reaches)
            for k in sorted(basic):
                rate = -sign * alpha[k]  # how z_k moves per unit of the entering variable's move
                if rate < 0 and lower[k] is not None:
                    step, side = (z[k] - lower[k]) / -rate, LOWER
                elif rate > 0 and upper[k] is not None:
                    step, side = (upper[k] - z[k]) / rate, UPPER
                else:
                    continue
                if best is None or step < best[0]:
                    best = (step, k, side)
            span = None
            if lower[entering] is not None and upper[entering] is not None:
                span = upper[entering] - lower[entering]
            if span is not None and (best is None or span < best[0]):  # it reaches its other bound first
                at[entering] = upper[entering] if sign > 0 else lower[entering]
            elif best is None:
                return Refused(UNBOUNDED, pivots)
            else:
                _, leaving, side = best
                basic[position[leaving]] = entering
                del at[entering]
                at[leaving] = lower[leaving] if side == LOWER else upper[leaving]
        elif not dual:  # a dual simplex step: the least infeasible basic variable leaves, at the bound it breaks
            leaving = primal[0]
            rising = lower[leaving] is not None and z[leaving] < lower[leaving]
            direction = 1 if rising else -1
            unit = [Fraction(1) if k == leaving else Fraction(0) for k in basic]
            rho = solve(dual_equations, unit, list(everything))
            if rho is None:
                return Refused(SINGULAR, pivots)
            choice: tuple[Fraction, int] | None = None
            for k in sorted(at):
                alpha_k = sum((coefficient * rho[i] for i, coefficient in columns[k].items()), Fraction(0))
                if not alpha_k:
                    continue
                if (rises(k) and alpha_k * direction < 0) or (falls(k) and alpha_k * direction > 0):
                    ratio = abs(costs[k]) / abs(alpha_k)
                    if choice is None or ratio < choice[0]:
                        choice = (ratio, k)
            if choice is None:
                return Refused(INFEASIBLE, pivots)
            entering = choice[1]
            basic[position[leaving]] = entering
            del at[entering]
            at[leaving] = lower[leaving] if rising else upper[leaving]
        else:
            return Refused(NOT_FEASIBLE, pivots)
        pivots += 1
