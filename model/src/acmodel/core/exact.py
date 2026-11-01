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
probe).

**A program that moves only some row ends of a certified one** (PRICE-2d: S3's re-solves at plus and minus one grain).
Its basis is factorised once (`factored`: the elimination of `solve` on the square part of B, recorded by `factor`, so
that any further right side is solved with the recorded steps, `Factor`, without a new elimination). Then:
- `shifted`: when the basis stays primal feasible with the moved ends, it stays optimal (dual feasibility does not read
  the ends), and the optimum moves by exactly Σ duals[row]·move, with no LP (`ranging` is its one-row form);
- `reoptimum`: otherwise HiGHS's new basis, from a warm start, is solved by updating the factorisation in product form
  (B'⁻¹ = E_k ⋯ E_1·B⁻¹, one eta matrix per variable that enters), and certified as `optimum` certifies a basis; a
  basis that this does not certify, or one so far from S3's that the update would cost more than a new elimination
  (the caller bounds it), is left to `optimum`."""

from __future__ import annotations

import heapq
import math
from dataclasses import dataclass, field
from fractions import Fraction
from typing import Callable, Hashable, Sequence

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


@dataclass(frozen=True)
class Factor:
    """The elimination of `solve` on a square system, recorded (`factor`; PRICE-2d) as A = L·U in the order of its pivots,
    so that more right sides are solved without a new elimination: each step (its pivot equation p, its unknown, and for
    each equation t it was eliminated from, L's multiplier l: the step takes l times equation p from equation t); the
    equations as the elimination left them, in integers, triangular in the pivots' order (pivot k's holds its unknown
    and later pivots' only); and `scales`, σ_p with equation p's integers σ_p times U's row p.

    A·x = b: the steps on b (Gaussian elimination: b_t less l·b_p, a step whose b_p is 0 skipped), then back
    substitution, U's row p being equation p over σ_p (`solve`). Aᵀ·y = c: Uᵀ·w = c forward, first pivot first, then
    Lᵀ·y = w, each step transposed, last first (b_p less l·b_t over its t: `solve_transposed`). Both exact; a zero is
    never stored."""

    steps: tuple[tuple[int, Hashable, tuple[tuple[int, Fraction], ...]], ...]
    rows: tuple[dict[Hashable, int], ...]
    scales: tuple[Fraction, ...]

    def solve(self, rhs: dict[int, Fraction]) -> dict[Hashable, Fraction]:
        """x with A·x = rhs (by equation, an absent one 0): its unknowns that are not 0."""
        w: dict[int, Fraction] = {i: Fraction(value) for i, value in rhs.items() if value}
        for index, _, eliminated in self.steps:
            pivot = w.get(index)
            if not pivot:
                continue
            for other, multiplier in eliminated:
                left = w.get(other, Fraction(0)) - multiplier * pivot
                if left:
                    w[other] = left
                else:
                    w.pop(other, None)
        found: dict[Hashable, Fraction] = {}
        for index, unknown, _ in reversed(self.steps):
            row = self.rows[index]
            value = self.scales[index] * w[index] if index in w else Fraction(0)
            for name, amount in row.items():
                if name != unknown:
                    known = found.get(name)
                    if known:
                        value -= amount * known
            if value:
                found[unknown] = value / row[unknown]
        return found

    def solve_transposed(self, cost: dict[Hashable, Fraction]) -> dict[int, Fraction]:
        """y with Aᵀ·y = cost (by unknown, an absent one 0): its entries, by equation, that are not 0."""
        rest: dict[Hashable, Fraction] = {name: Fraction(value) for name, value in cost.items() if value}
        w: dict[int, Fraction] = {}
        for index, unknown, _ in self.steps:  # Uᵀ·w = cost, first pivot first
            value = rest.pop(unknown, None)
            if not value:
                continue
            row = self.rows[index]
            share = value / row[unknown]  # w_p over σ_p
            w[index] = self.scales[index] * share
            for name, amount in row.items():
                if name != unknown:
                    left = rest.get(name, Fraction(0)) - amount * share
                    if left:
                        rest[name] = left
                    else:
                        rest.pop(name, None)
        for index, _, eliminated in reversed(self.steps):  # Lᵀ·y = w: each step transposed, last first
            total = w.get(index, Fraction(0))
            for other, multiplier in eliminated:
                value = w.get(other)
                if value:
                    total -= multiplier * value
            if total:
                w[index] = total
            else:
                w.pop(index, None)
        return w


def factor(equations: Sequence[dict[Hashable, Fraction]], unknowns: Sequence[Hashable]) -> Factor | None:
    """`solve`'s elimination of a square sparse system, with the same choice of pivots, recorded (`Factor`); None when
    the system is singular. The elimination is `solve`'s, fraction-free (the divisor of an equation is the greatest
    common divisor of its coefficients; `solve` divides its right side too); each equation's σ follows it, from the
    scale that makes it integers: an elimination (c·t − f·p) ÷ g with c the pivot's coefficient and f the target's makes
    σ_t c·σ_t ÷ g, and L's multiplier is f·σ_p ÷ (c·σ_t), with σ_t as it was before."""
    size = len(unknowns)
    if len(equations) != size:
        return None
    rank = {unknown: position for position, unknown in enumerate(unknowns)}
    rows: list[dict[Hashable, int]] = []
    scales: list[Fraction] = []
    holders: dict[Hashable, set[int]] = {unknown: set() for unknown in unknowns}
    for index, equation in enumerate(equations):
        given = {unknown: Fraction(coefficient) for unknown, coefficient in equation.items() if coefficient != 0}
        scale = math.lcm(1, *(coefficient.denominator for coefficient in given.values()))
        row = {unknown: c.numerator * (scale // c.denominator) for unknown, c in given.items()}
        for unknown in row:
            if unknown not in holders:
                raise ValueError(f"an equation holds {unknown!r}, which is not an unknown")
            holders[unknown].add(index)
        rows.append(row)
        scales.append(Fraction(scale))
    active = set(range(size))
    shortest = [(len(row), index) for index, row in enumerate(rows)]
    heapq.heapify(shortest)
    singles = [unknown for unknown in unknowns if len(holders[unknown]) == 1]
    done: set[Hashable] = set()
    steps: list[tuple[int, Hashable, tuple[tuple[int, Fraction], ...]]] = []
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
                return None
            unknown = min(row, key=lambda name: (len(holders[name]), rank[name]))
            pivot = (index, unknown)
        index, unknown = pivot
        row = rows[index]
        coefficient = row[unknown]
        eliminated: list[tuple[int, Fraction]] = []
        for other in sorted(holders[unknown] - {index}):
            target = rows[other]
            factor_ = target.pop(unknown)
            multiplier = factor_ * scales[index] / (coefficient * scales[other])
            if coefficient != 1:
                for name in target:
                    target[name] *= coefficient
            for name, amount in row.items():
                if name == unknown:
                    continue
                updated = target.get(name, 0) - factor_ * amount
                if updated:
                    if name not in target:
                        holders[name].add(other)
                    target[name] = updated
                elif name in target:
                    del target[name]
                    holders[name].discard(other)
                    if len(holders[name]) == 1 and name not in done:
                        singles.append(name)
            common = math.gcd(*target.values()) if target else 1
            if common > 1:
                for name in target:
                    target[name] //= common
            else:
                common = 1
            scales[other] = scales[other] * coefficient / common
            eliminated.append((other, multiplier))
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
                    return None
        steps.append((index, unknown, tuple(eliminated)))
    return Factor(tuple(steps), tuple(rows), tuple(scales))


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
        # PRICE-2e: the primal and the dual system are one matrix and its transpose, so one elimination (`factor`, the
        # elimination of `solve` with its pivots, recorded) solves both: B·z_B by its steps, Bᵀ·y by them transposed.
        # Each is the one exact solution of its system, the same as two eliminations give; singular exactly as before.
        recorded = factor(equations, structural)
        if recorded is None:
            return Refused(SINGULAR, pivots)
        solved = recorded.solve({e: value for e, value in enumerate(rhs) if value})
        x_basic = {k: solved.get(k, Fraction(0)) for k in structural}
        z = {**at, **x_basic}
        for k in basic:
            if k >= n:  # the terms at 0 add nothing (the sums skip them, exactly)
                z[k] = sum((a * z[j] for j, a in problem.rows[k - n].items() if z[j]), Fraction(0))
        transposed = recorded.solve_transposed({j: cost[j] for j in structural if cost[j]})
        y_tight = {tight[e]: value for e, value in transposed.items()}
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


def ranging(
    problem: Problem, vertex: Vertex, row: int, step: Fraction, factored: Factored | None = None
) -> tuple[bool, bool, bool, bool]:
    """Whether `vertex`'s basis stays primal feasible when both ends of equality row `row` (its logical nonbasic) move
    up by `step`, down by `step`, up a little, down a little. Where it does, it stays optimal (dual feasibility does
    not read the right sides), so the optimum moves by exactly the move times duals[row] that far: a chord of `step`
    is then duals[row], and a little gives the derivative on that side, duals[row]. All False when the row's logical
    is basic; nothing is solved but the basis system once, with the row's unit move as its right side: by `factored`'s
    recorded factorisation of this basis when it is given (PRICE-2d), else by a new elimination.

    "A little" holds when each basic variable at a bound moves off it inward or not at all, and "by step" when each
    basic variable's value moved by step stays within its bounds."""
    n, m = len(problem.cost), len(problem.rows)
    basic = set(vertex.basic)
    if not basic or n + row in basic or problem.lower[n + row] != problem.upper[n + row]:
        return False, False, False, False
    if factored is not None:
        changes = factored.ftran({row: Fraction(1)})
    else:
        structural = [k for k in vertex.basic if k < n]
        tight = [i for i in range(m) if n + i not in basic]
        chosen = set(structural)
        equations = [{j: a for j, a in problem.rows[i].items() if j in chosen and a} for i in tight]
        moved = solve(equations, [Fraction(1) if i == row else Fraction(0) for i in tight], structural)
        if moved is None:
            return False, False, False, False
        changes = {}
        for k in vertex.basic:
            if k < n:
                changes[k] = moved[k]
            else:
                rows = problem.rows[k - n].items()
                changes[k] = sum((a * moved[j] for j, a in rows if j in chosen and moved[j]), Fraction(0))
    values, lower, upper = vertex.values, problem.lower, problem.upper
    holds = {(1, True): True, (-1, True): True, (1, False): True, (-1, False): True}  # (direction, by step)
    for k in vertex.basic:
        change = changes.get(k)
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


# Programs that move only some row ends of a certified one (PRICE-2d) -------------------------------------------------


@dataclass(frozen=True)
class Factored:
    """A certified vertex's basis, factorised once (`factored`), for the programs that differ from its own only in the
    ends of some rows. B·z = v and Bᵀ·y = u are solved as `optimum` solves them, on the square part of B (the tight rows,
    those whose logical is nonbasic, and the basic columns), here with `factor`'s recorded steps (`ftran`, `btran`);
    `reduced` holds every variable's reduced cost at the vertex's duals (0 for a basic one)."""

    problem: Problem
    vertex: Vertex
    columns: tuple[dict[int, Fraction], ...]  # the n + m columns of A·x − r = 0
    basic: frozenset[int]
    tight: dict[int, int]  # a tight row -> its equation in `factor`
    order: tuple[int, ...]  # the tight rows, by equation
    structural: frozenset[int]  # the basic columns
    factor: Factor
    reduced: tuple[Fraction, ...]

    def ftran(self, v: dict[int, Fraction]) -> dict[int, Fraction]:
        """z_B with B·z_B = v (by row, an absent one 0): the basic variables that are not 0. A basic logical r_i, of a
        row that is not tight, is its row's value less v_i."""
        n = len(self.problem.cost)
        given = {self.tight[i]: value for i, value in v.items() if value and i in self.tight}
        found: dict[int, Fraction] = self.factor.solve(given)
        logical: dict[int, Fraction] = {}
        for j, value in list(found.items()):
            for i, a in self.columns[j].items():
                if i not in self.tight:
                    logical[i] = logical.get(i, Fraction(0)) + a * value
        for i, value in v.items():
            if value and i not in self.tight:
                logical[i] = logical.get(i, Fraction(0)) - value
        for i, value in logical.items():
            if value:
                found[n + i] = value
        return found

    def btran(self, u: dict[int, Fraction]) -> dict[int, Fraction]:
        """y with Bᵀ·y = u (by basic variable, an absent one 0): the rows' entries that are not 0. A basic logical's
        column is −e_i, so its row's entry is −u; the tight rows' come from the basic columns."""
        n = len(self.problem.cost)
        y: dict[int, Fraction] = {k - n: -Fraction(value) for k, value in u.items() if value and k >= n}
        rest: dict[int, Fraction] = {j: Fraction(value) for j, value in u.items() if value and j < n}
        for i, dual in y.items():
            for j, a in self.problem.rows[i].items():
                if j in self.structural:
                    rest[j] = rest.get(j, Fraction(0)) - a * dual
        for position, value in self.factor.solve_transposed(rest).items():
            y[self.order[position]] = value
        return y


def factored(problem: Problem, vertex: Vertex) -> Factored | None:
    """`vertex`'s basis factorised (`Factored`); None when it is singular, or a basic variable's reduced cost at its
    duals is not 0 (a defect: the duals solve Bᵀ·y = c_B)."""
    n, m = len(problem.cost), len(problem.rows)
    if len(vertex.basic) != m or len(vertex.values) != n + m:
        return None
    columns: list[dict[int, Fraction]] = [{} for _ in range(n)]
    for i, row in enumerate(problem.rows):
        for j, coefficient in row.items():
            if coefficient:
                columns[j][i] = coefficient
    columns.extend({i: Fraction(-1)} for i in range(m))
    basic = frozenset(vertex.basic)
    structural = [k for k in vertex.basic if k < n]
    order = tuple(i for i in range(m) if n + i not in basic)
    chosen = set(structural)
    found = factor([{j: a for j, a in problem.rows[i].items() if j in chosen and a} for i in order], structural)
    if found is None:
        return None
    cost = [*problem.cost, *([Fraction(0)] * m)]
    y = vertex.duals
    reduced = tuple(
        cost[k] - sum((a * y[i] for i, a in columns[k].items() if y[i]), Fraction(0)) for k in range(n + m)
    )
    if any(reduced[k] for k in basic):
        return None
    tight = {i: position for position, i in enumerate(order)}
    return Factored(problem, vertex, tuple(columns), basic, tight, order, frozenset(chosen), found, reduced)


def _eta(z: dict[int, Fraction], p: int, q: int, alpha: dict[int, Fraction]) -> None:
    """z, the basic values of a basis, carried to the basis where q replaces p (α = B⁻¹·a_q, α_p not 0): z_q = z_p ÷ α_p
    and every other z_k less α_k·z_q. Zeros are not stored."""
    ratio = z.pop(p, Fraction(0)) / alpha[p]
    if not ratio:
        return
    for k, a in alpha.items():
        if k != p:
            left = z.get(k, Fraction(0)) - a * ratio
            if left:
                z[k] = left
            else:
                z.pop(k, None)
    z[q] = ratio


def shifted(base: Factored, moves: dict[int, Fraction]) -> Fraction | None:
    """How much the optimum moves when the ends of rows `moves` (row -> amount: both ends of an equality, the one end of
    an at-most) move by those amounts, from `base`'s basis, when it stays primal feasible, and so optimal (its dual
    feasibility does not read the ends): exactly Σ duals[row]·amount. None when it does not, or a moved row's logical is
    nonbasic away from its ends. Only the basic variables whose value or bounds move are compared; the others keep the
    values and bounds the certificate checked."""
    problem, vertex = base.problem, base.vertex
    n = len(problem.cost)
    values, lower, upper = vertex.values, problem.lower, problem.upper
    v: dict[int, Fraction] = {}
    for i, amount in moves.items():
        k = n + i
        if k in base.basic or not amount:
            continue
        if values[k] != lower[k] and values[k] != upper[k]:
            return None
        v[i] = amount
    change = base.ftran(v)
    for k in {*change, *(n + i for i in moves if n + i in base.basic)}:
        value = values[k] + change.get(k, Fraction(0))
        low, high = lower[k], upper[k]
        if k >= n and k - n in moves:
            amount = moves[k - n]
            low = None if low is None else low + amount
            high = None if high is None else high + amount
        if (low is not None and value < low) or (high is not None and value > high):
            return None
    return sum((vertex.duals[i] * amount for i, amount in moves.items()), Fraction(0))


_Etas = list[tuple[int, int, dict[int, Fraction]]]
# Why a basis is not solved by updating a factorisation (`reoptimum`): its statuses are not a basis; it is singular; it
# is so far from base's that the update would cost more than a new rebuild (as the caller bounds it, `_moved`); or it
# is not exactly optimal.
UPDATE_NOT_BASIS, UPDATE_SINGULAR, UPDATE_FAR, UPDATE_NOT_OPTIMAL = "not_basis", "singular", "far", "not_optimal"


def _moved(
    problem: Problem, base: Factored, statuses: Sequence[int], most: int, stop: Callable[[int], bool] | None = None
) -> tuple[list[int], dict[int, Fraction], _Etas, dict[int, Fraction], list[int]] | str:
    """HiGHS's basis `statuses` of `problem` (base's rows and costs, other ends) solved by updating base's factorisation
    (`reoptimum`): its basic variables, the nonbasic ones' values, the etas, every variable's value, and the basic ones
    outside their bounds, by index. Else why not: the statuses are not a basis, the basis is singular, or it is far
    from base's: more than `most` variables enter (decided before any solve), or `stop`, asked before each one enters
    with how many have entered so far, says the update has cost enough (the update solves once with base's
    factorisation for each and applies every eta so far, where a new rebuild may cost less: the caller decides).

    A variable nonbasic in both bases at the same value (the same object, or an equal one) adds nothing to Δv; a basic
    variable whose value and bounds did not move keeps base's value, within base's bounds."""
    if problem.cost is not base.problem.cost or problem.rows is not base.problem.rows:
        raise ValueError("a program re-solved from a factorised basis must share its rows and costs")
    n, m = len(problem.cost), len(problem.rows)
    started = _start(problem, statuses)
    if started is None:
        return UPDATE_NOT_BASIS
    basic, at = started
    vertex, columns = base.vertex, base.columns
    new = set(basic)
    leaving = sorted(base.basic - new)
    if len(leaving) != len(new - base.basic):
        return UPDATE_NOT_BASIS
    if len(leaving) > most:
        return UPDATE_FAR
    etas: _Etas = []
    for entered, q in enumerate(sorted(new - base.basic)):
        if stop is not None and stop(entered):
            return UPDATE_FAR
        alpha = base.ftran(columns[q])
        for p, entered, earlier in etas:
            _eta(alpha, p, entered, earlier)
        p = next((k for k in leaving if alpha.get(k)), None)
        if p is None:
            return UPDATE_SINGULAR
        leaving.remove(p)
        etas.append((p, q, alpha))
    values, lower, upper = vertex.values, problem.lower, problem.upper
    base_lower, base_upper = base.problem.lower, base.problem.upper
    zero = Fraction(0)
    shift: dict[int, Fraction] = {}
    for k in range(n + m):
        before = None if k in base.basic else values[k]
        after = at.get(k)
        if after is before or (after is not None and before is not None and after == before):
            continue
        moved = (zero if after is None else after) - (zero if before is None else before)
        if moved:
            for i, a in columns[k].items():
                shift[i] = shift.get(i, zero) - a * moved
    change = base.ftran({i: amount for i, amount in shift.items() if amount})
    z = {k: values[k] for k in vertex.basic if values[k]}
    touched = set(change)
    for k, amount in change.items():
        value = values[k] + amount
        if value:
            z[k] = value
        else:
            z.pop(k, None)
    for p, q, alpha in etas:
        _eta(z, p, q, alpha)
        touched.update(alpha)
        touched.add(q)
    full = dict(at)
    for k in basic:
        full[k] = z.get(k, zero)
    broken = []
    for k in sorted(basic):
        if k not in touched and k in base.basic and lower[k] is base_lower[k] and upper[k] is base_upper[k]:
            continue
        value = full[k]
        if (lower[k] is not None and value < lower[k]) or (upper[k] is not None and value > upper[k]):
            broken.append(k)
    return basic, at, etas, full, broken


def reoptimum(
    problem: Problem, base: Factored, statuses: Sequence[int], most: int, stop: Callable[[int], bool] | None = None
) -> Vertex | str:
    """The exactly optimal vertex of `problem`, which has `base`'s rows and costs and other ends, at HiGHS's basis
    `statuses`, solved by updating `base`'s factorisation instead of a new elimination; else why not (`UPDATE_…`: not a
    basis, singular, far from base's (more than `most` variables enter, or `stop` ends the update: `_moved`), or not
    exactly optimal, primal and dual feasible): the caller then takes `optimum`, a new elimination with its exact
    pivots.

    The update is the product form of the inverse. Each variable that enters, by index, replaces one that leaves, the
    least whose entry in α = B⁻¹·a_q (its column's solve in the basis so far) is not 0, which exists exactly when the
    new basis is not singular; so B'⁻¹ = E_k ⋯ E_1·B⁻¹, each E the eta matrix of one α (`_eta`). The new basic values
    are E·(z_B + B⁻¹·Δv), Δv the change of the right side −N·z_N (the nonbasic variables that moved, entered or left);
    the new duals y + B⁻ᵀ·Δu, Δu the change of the basic costs that the etas carry back to base's basis; each reduced
    cost base's less its column times the change of the duals. Then the certificate of `optimum`, exactly: every
    basic variable within its bounds, every nonbasic one's reduced cost of the sign of a maximum."""
    moved = _moved(problem, base, statuses, most, stop)
    if isinstance(moved, str):
        return moved
    basic, at, etas, full, broken = moved
    if broken:
        return UPDATE_NOT_OPTIMAL
    n, m = len(problem.cost), len(problem.rows)
    vertex = base.vertex
    values, lower, upper = vertex.values, problem.lower, problem.upper
    base_lower, base_upper = base.problem.lower, base.problem.upper
    zero = Fraction(0)
    # The duals. A nonbasic variable whose reduced cost did not move, at the same value and bounds as in base, keeps
    # base's sign of a maximum.
    u = {k: problem.cost[k] for k in basic if k < n and problem.cost[k]}
    for p, q, alpha in reversed(etas):
        total = u.pop(q, zero) - sum((a * u[k] for k, a in alpha.items() if k != p and k in u), zero)
        if total:
            u[p] = total / alpha[p]
    carried: dict[int, Fraction] = {}
    for k in vertex.basic:
        amount = u.get(k, zero) - (problem.cost[k] if k < n else zero)
        if amount:
            carried[k] = amount
    dy = base.btran(carried)
    y = list(vertex.duals)
    moved_costs: dict[int, Fraction] = {}
    for i, amount in dy.items():
        y[i] += amount
        for j, a in problem.rows[i].items():
            moved_costs[j] = moved_costs.get(j, zero) - a * amount
        moved_costs[n + i] = moved_costs.get(n + i, zero) + amount
    for k, value in at.items():
        if (
            k not in moved_costs and k not in base.basic and value is values[k]
            and lower[k] is base_lower[k] and upper[k] is base_upper[k]
        ):
            continue
        cost = base.reduced[k] + moved_costs.get(k, zero)
        rises = upper[k] is None or value < upper[k]
        falls = lower[k] is None or value > lower[k]
        if (rises and cost > 0) or (falls and cost < 0):
            return UPDATE_NOT_OPTIMAL
    return Vertex(tuple(full[j] for j in range(n)), tuple(y), 0, tuple(basic), tuple(full[k] for k in range(n + m)))
