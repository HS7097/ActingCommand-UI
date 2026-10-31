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
import math
from dataclasses import dataclass, field
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
    maximising sense: Σ_i a_ij·y_i = c_j for a basic column) and the pivots it took from HiGHS's basis; and, for
    `stays`, its basic variables and every variable's value (`values`: the columns, then the rows' logicals)."""

    x: tuple[Fraction, ...]
    duals: tuple[Fraction, ...]
    pivots: int
    basic: tuple[int, ...] = field(default=(), compare=False)
    values: tuple[Fraction, ...] = field(default=(), compare=False)


@dataclass(frozen=True)
class Refused:
    """No certified vertex: why (`reason`), after how many pivots."""

    reason: str
    pivots: int


def solve(
    equations: Sequence[dict[Hashable, Fraction]], rhs: Sequence[Fraction], unknowns: Sequence[Hashable]
) -> dict[Hashable, Fraction] | None:
    """The one solution of a square sparse linear system, exactly; None when it has none or more than one.

    Sparse Gaussian elimination, fraction-free: each equation is scaled to integers (by the least common multiple of
    its denominators), an elimination replaces an equation by the pivot coefficient times it less its coefficient times
    the pivot equation, and the result is divided by the greatest common divisor of its terms, so every number stays
    an exact integer and small. Each step takes an unknown that only one remaining equation holds (a column singleton:
    no fill), else the shortest remaining equation and in it the unknown that the fewest equations hold (Markowitz's
    choice; ties by order of `unknowns`); it eliminates that unknown from the other equations. Then back substitution
    in fractions, last pivot first. A coefficient that cancels is dropped."""
    size = len(unknowns)
    if len(equations) != size:
        return None
    rank = {unknown: position for position, unknown in enumerate(unknowns)}
    rows: list[dict[Hashable, int]] = []
    values: list[int] = []
    holders: dict[Hashable, set[int]] = {unknown: set() for unknown in unknowns}
    for index, equation in enumerate(equations):
        given = {unknown: Fraction(coefficient) for unknown, coefficient in equation.items() if coefficient != 0}
        value = Fraction(rhs[index])
        scale = math.lcm(value.denominator, *(coefficient.denominator for coefficient in given.values()))
        row = {unknown: c.numerator * (scale // c.denominator) for unknown, c in given.items()}
        for unknown in row:
            if unknown not in holders:
                raise ValueError(f"an equation holds {unknown!r}, which is not an unknown")
            holders[unknown].add(index)
        rows.append(row)
        values.append(value.numerator * (scale // value.denominator))
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
            factor = target.pop(unknown)
            if coefficient != 1:  # target ← coefficient·target − factor·row
                for name in target:
                    target[name] *= coefficient
            for name, amount in row.items():
                if name == unknown:
                    continue
                updated = target.get(name, 0) - factor * amount
                if updated:
                    if name not in target:
                        holders[name].add(other)
                    target[name] = updated
                elif name in target:
                    del target[name]
                    holders[name].discard(other)
                    if len(holders[name]) == 1 and name not in done:
                        singles.append(name)
            values[other] = coefficient * values[other] - factor * value
            common = math.gcd(values[other], *target.values())
            if common > 1:
                for name in target:
                    target[name] //= common
                values[other] //= common
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
        rest = sum((amount * found[name] for name, amount in row.items() if name != unknown and found[name]), Fraction(0))
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
        # B is block-triangular: a basic logical r_i is its row's value, and a basic logical's row has a zero dual.
        # So both systems shrink to the tight rows (the rows whose logical is nonbasic) and the basic columns, a
        # square system as large as the basic columns; then each basic logical is its row's value.
        structural = [k for k in basic if k < n]
        tight = [i for i in everything if n + i in at]
        chosen = set(structural)
        equations, rhs = [], []
        for i in tight:
            row = problem.rows[i]
            equations.append({j: a for j, a in row.items() if j in chosen and a})
            rhs.append(at[n + i] - sum((a * at[j] for j, a in row.items() if at.get(j)), Fraction(0)))
        x_basic = solve(equations, rhs, structural)
        if x_basic is None:
            return Refused(SINGULAR, pivots)
        z = {**at, **x_basic}
        for k in basic:
            if k >= n:  # the terms at 0 add nothing (the sums skip them, exactly)
                z[k] = sum((a * z[j] for j, a in problem.rows[k - n].items() if z[j]), Fraction(0))
        held = set(tight)
        transposed = [{i: a for i, a in columns[j].items() if i in held} for j in structural]
        y_tight = solve(transposed, [cost[j] for j in structural], tight)
        if y_tight is None:
            return Refused(SINGULAR, pivots)
        y = [y_tight.get(i, Fraction(0)) for i in everything]

        def reduced(k: int) -> Fraction:
            return cost[k] - sum((coefficient * y[i] for i, coefficient in columns[k].items() if y[i]), Fraction(0))

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
            return Vertex(tuple(z[j] for j in range(n)), tuple(y), pivots, tuple(basic), tuple(z[k] for k in range(n + m)))
        if pivots >= cap:
            return Refused(PIVOT_CAP, pivots)
        position = {k: p for p, k in enumerate(basic)}
        if not primal:  # a primal simplex step: the least index whose reduced cost improves enters
            entering = dual[0]
            sign = 1 if rises(entering) and costs[entering] > 0 else -1
            b_rows: list[dict[int, Fraction]] = [{} for _ in everything]  # B, row by row
            for k in basic:
                for i, coefficient in columns[k].items():
                    b_rows[i][k] = coefficient
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
            rho = solve([columns[k] for k in basic], unit, list(everything))  # Bᵀ·ρ = e: B⁻¹'s row of `leaving`
            if rho is None:
                return Refused(SINGULAR, pivots)
            choice: tuple[Fraction, int] | None = None
            for k in sorted(at):
                alpha_k = sum((coefficient * rho[i] for i, coefficient in columns[k].items() if rho[i]), Fraction(0))
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


def ranging(problem: Problem, vertex: Vertex, row: int, step: Fraction) -> tuple[bool, bool, bool, bool]:
    """Whether `vertex`'s basis stays primal feasible when both ends of equality row `row` (its logical nonbasic) move
    up by `step`, down by `step`, up a little, down a little. Where it does, it stays optimal (dual feasibility does
    not read the right sides), so the optimum moves by exactly the move times duals[row] that far: a chord of `step`
    is then duals[row], and a little gives the derivative on that side, duals[row]. All False when the row's logical
    is basic; nothing is solved but the basis system once, with the row's unit move as its right side.

    "A little" holds when each basic variable at a bound moves off it inward or not at all, and "by step" when each
    basic variable's value moved by step stays within its bounds."""
    n, m = len(problem.cost), len(problem.rows)
    basic = set(vertex.basic)
    if not basic or n + row in basic or problem.lower[n + row] != problem.upper[n + row]:
        return False, False, False, False
    structural = [k for k in vertex.basic if k < n]
    tight = [i for i in range(m) if n + i not in basic]
    chosen = set(structural)
    equations = [{j: a for j, a in problem.rows[i].items() if j in chosen and a} for i in tight]
    moved = solve(equations, [Fraction(1) if i == row else Fraction(0) for i in tight], structural)
    if moved is None:
        return False, False, False, False
    values, lower, upper = vertex.values, problem.lower, problem.upper
    holds = {(1, True): True, (-1, True): True, (1, False): True, (-1, False): True}  # (direction, by step)
    for k in vertex.basic:
        if k < n:
            change = moved[k]
        else:
            change = sum((a * moved[j] for j, a in problem.rows[k - n].items() if j in chosen and moved[j]), Fraction(0))
        if not change:
            continue
        for sign in (1, -1):
            rate = sign * change  # how z_k moves per unit of the move
            end = lower[k] if rate < 0 else upper[k]
            if end is None:
                continue
            if values[k] == end:  # at the bound it moves towards: not even a little
                holds[(sign, False)] = holds[(sign, True)] = False
            elif (rate < 0 and values[k] + step * rate < end) or (rate > 0 and values[k] + step * rate > end):
                holds[(sign, True)] = False
    return holds[(1, True)], holds[(-1, True)], holds[(1, False)], holds[(-1, False)]
