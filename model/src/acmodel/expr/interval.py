"""Interval arithmetic for the values that continuous unknowns leave open (#424 design 4.2).

An interval or a continuous distribution unknown (`core.unknown`) is one `Interval`, not a set of points. Every
operator then gives a **sound enclosure**: each value the unknowns allow lies within [low, high], and the bounds
are exact rationals. An enclosure is never narrower than the true range; it can be wider.

**The monotonic check.** An `Interval` records, for each continuous unknown it depends on, the direction it is
proven monotone in: `1` (rising), `-1` (falling) or `0` (not proven). The check is structural:
- `+`, `-`, unary `-`, `min`, `max`, `clamp`, `floor`, `ceil`, `round_half_even`, `sum` and the flow sums are
  monotone in each operand, so an unknown keeps its direction through them (`-` turns it around on its right);
- `*` and `/` are monotone in one operand when the other has one sign over its enclosure; an operand that may
  be either sign leaves the unknowns of the other unproven;
- an unknown that reaches the value in both directions (`x - x`) is unproven;
- a comparison, `and`, `or`, `not`, `if`, `count` and `lookup` are not monotone: a condition that the unknowns
  leave undecided (`Undecided`) gives the enclosure of both branches, with every unknown unproven.
When every unknown is proven monotone (`exact`), the enclosure is attained at a corner of the unknowns, so it is
the exact range. Otherwise it is sound and may be wider.

**Refusals** (raised as `IntervalRefused`, which the evaluator reports as `expr_eval_failed`): a division by an
enclosure that holds 0 (`division_by_zero`), and `clamp` bounds that may cross (`clamp_bounds_invalid`).
"""

from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction
from typing import Any, Callable, Iterable

from acmodel.expr.values import Duration


@dataclass(frozen=True)
class Interval:
    """A sound enclosure [low, high] (Fractions, or Durations for time), low < high, with the direction each
    continuous unknown it depends on is proven monotone in (`signs`, sorted by unknown)."""

    low: Any
    high: Any
    signs: tuple[tuple[str, int], ...]

    @property
    def exact(self) -> bool:
        """Whether the enclosure is the exact range: every unknown is proven monotone."""
        return all(sign for _, sign in self.signs)


@dataclass(frozen=True)
class Undecided:
    """A boolean that the continuous `unknowns` leave open: true for some of their values, false for others."""

    unknowns: tuple[str, ...]


class IntervalRefused(Exception):
    """An operation that an enclosure makes unsound: `reason` is `division_by_zero` or `clamp_bounds_invalid`."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


def magnitude(value: Any) -> Any:
    return value.seconds if type(value) is Duration else value


def is_open(value: Any) -> bool:
    """Whether a value is an enclosure or an undecided boolean, which this module computes with."""
    return type(value) in (Interval, Undecided)


def unknowns(value: Any) -> tuple[str, ...]:
    if type(value) is Interval:
        return tuple(unknown for unknown, _ in value.signs)
    if type(value) is Undecided:
        return value.unknowns
    return ()


def bounds(value: Any) -> tuple[Any, Any, dict[str, int]]:
    """(low, high, signs) of a number, a duration or an enclosure."""
    if type(value) is Interval:
        return value.low, value.high, dict(value.signs)
    return value, value, {}


def make(low: Any, high: Any, signs: dict[str, int]) -> Any:
    """The enclosure [low, high]; a single value when the bounds meet (it is then exact, whatever the signs)."""
    if magnitude(low) == magnitude(high):
        return low
    return Interval(low, high, tuple(sorted(signs.items())))


def combine(*maps: dict[str, int]) -> dict[str, int]:
    """The directions of a value that is monotone (rising) in each of several operands: an unknown keeps its
    direction when every operand agrees on it, and is unproven otherwise."""
    found: dict[str, int] = {}
    for signs in maps:
        for unknown, sign in signs.items():
            found[unknown] = sign if found.get(unknown, sign) == sign else 0
    return found


def _scaled(signs: dict[str, int], factor: int) -> dict[str, int]:
    return {unknown: sign * factor for unknown, sign in signs.items()}


def _sign(low: Any, high: Any) -> int:
    """1 when an enclosure is >= 0, -1 when it is <= 0, 0 when it may be either."""
    if magnitude(low) >= 0:
        return 1
    if magnitude(high) <= 0:
        return -1
    return 0


def _lowest(values: Iterable[Any]) -> Any:
    return min(values, key=magnitude)


def _highest(values: Iterable[Any]) -> Any:
    return max(values, key=magnitude)


# Point operations on numbers and durations, as the evaluator's.


def _add(a: Any, b: Any) -> Any:
    return Duration(a.seconds + b.seconds) if type(a) is Duration else a + b


def _negate(a: Any) -> Any:
    return Duration(-a.seconds) if type(a) is Duration else -a


def _multiply(a: Any, b: Any) -> Any:
    if type(a) is Duration:
        return Duration(a.seconds * b)
    if type(b) is Duration:
        return Duration(a * b.seconds)
    return a * b


def _divide(a: Any, b: Any) -> Any:
    if type(a) is not Duration:
        return a / b
    return a.seconds / b.seconds if type(b) is Duration else Duration(a.seconds / b)


# Interval operations ------------------------------------------------------------------------------


def add(a: Any, b: Any) -> Any:
    (al, ah, asg), (bl, bh, bsg) = bounds(a), bounds(b)
    return make(_add(al, bl), _add(ah, bh), combine(asg, bsg))


def negate(a: Any) -> Any:
    low, high, signs = bounds(a)
    return make(_negate(high), _negate(low), _scaled(signs, -1))


def subtract(a: Any, b: Any) -> Any:
    return add(a, negate(b))


def multiply(a: Any, b: Any) -> Any:
    (al, ah, asg), (bl, bh, bsg) = bounds(a), bounds(b)
    if not is_open(a) and magnitude(a) == 0:  # 0 times anything is 0
        return _multiply(a, bl)
    if not is_open(b) and magnitude(b) == 0:
        return _multiply(al, b)
    products = [_multiply(x, y) for x in (al, ah) for y in (bl, bh)]
    signs = combine(_scaled(asg, _sign(bl, bh)), _scaled(bsg, _sign(al, ah)))
    return make(_lowest(products), _highest(products), signs)


def divide(a: Any, b: Any) -> Any:
    (al, ah, asg), (bl, bh, bsg) = bounds(a), bounds(b)
    if magnitude(bl) <= 0 <= magnitude(bh):
        raise IntervalRefused("division_by_zero")
    quotients = [_divide(x, y) for x in (al, ah) for y in (bl, bh)]
    signs = combine(_scaled(asg, _sign(bl, bh)), _scaled(bsg, -_sign(al, ah)))
    return make(_lowest(quotients), _highest(quotients), signs)


def minimum(values: list[Any]) -> Any:
    parts = [bounds(value) for value in values]
    return make(_lowest(p[0] for p in parts), _lowest(p[1] for p in parts), combine(*(p[2] for p in parts)))


def maximum(values: list[Any]) -> Any:
    parts = [bounds(value) for value in values]
    return make(_highest(p[0] for p in parts), _highest(p[1] for p in parts), combine(*(p[2] for p in parts)))


def clamp(x: Any, low: Any, high: Any) -> Any:
    """clamp is monotone in all three; bounds that may cross are refused."""
    (xl, xh, xs), (ll, lh, ls), (hl, hh, hs) = bounds(x), bounds(low), bounds(high)
    if magnitude(lh) > magnitude(hl):
        raise IntervalRefused("clamp_bounds_invalid")

    def one(value: Any, floor: Any, ceiling: Any) -> Any:
        return floor if magnitude(value) < magnitude(floor) else ceiling if magnitude(value) > magnitude(ceiling) else value

    return make(one(xl, ll, hl), one(xh, lh, hh), combine(xs, ls, hs))


def rounded(function: Callable[[Any], int], a: Any) -> Any:
    """floor, ceil and round half even are non-decreasing (a duration rounds its seconds)."""
    low, high, signs = bounds(a)

    def one(value: Any) -> Any:
        whole = Fraction(function(magnitude(value)))
        return Duration(whole) if type(value) is Duration else whole

    return make(one(low), one(high), signs)


def compare(op: str, a: Any, b: Any) -> bool | Undecided:
    """A comparison of two numbers, durations or booleans, any of them open: decided when it holds for every
    value or for none, undecided otherwise."""
    involved = tuple(sorted(set(unknowns(a)) | set(unknowns(b))))
    if type(a) is bool or type(b) is bool or type(a) is Undecided or type(b) is Undecided:  # == or != of booleans
        if is_open(a) or is_open(b):
            return Undecided(involved)
        return (a == b) if op == "==" else (a != b)
    (al, ah, _), (bl, bh, _) = bounds(a), bounds(b)
    al, ah, bl, bh = (magnitude(value) for value in (al, ah, bl, bh))
    decided = {
        "<": (ah < bl, al >= bh), "<=": (ah <= bl, al > bh),
        ">": (al > bh, ah <= bl), ">=": (al >= bh, ah < bl),
        "==": (al == ah == bl == bh, ah < bl or al > bh), "!=": (ah < bl or al > bh, al == ah == bl == bh),
    }[op]
    if decided[0]:
        return True
    if decided[1]:
        return False
    return Undecided(involved)


def hull(a: Any, b: Any, condition: Undecided) -> Any:
    """The value of `if` under an undecided condition: both branches, every unknown unproven."""
    involved = sorted(set(condition.unknowns) | set(unknowns(a)) | set(unknowns(b)))
    if type(a) in (bool, Undecided):
        return a if type(a) is bool and a == b else Undecided(tuple(involved))
    (al, ah, _), (bl, bh, _) = bounds(a), bounds(b)
    return make(_lowest((al, bl)), _highest((ah, bh)), {unknown: 0 for unknown in involved})


def open_count(item: Undecided) -> Interval:
    """What an undecided condition adds to a `count`: 0 or 1."""
    return Interval(Fraction(0), Fraction(1), tuple((unknown, 0) for unknown in item.unknowns))
