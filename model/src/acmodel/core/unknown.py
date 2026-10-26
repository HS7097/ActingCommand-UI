"""Unknowns (#424 design 3.5, 4.5 and 4.8; contracts/formats.md, "Unknowns"): the points an unknown is
evaluated at, the one-at-a-time sensitivity of each output with the no-effect rule, and the Monte Carlo
sampling of the distributions. The graph (`core.graph`) calls these; the keys are `config.Unknowns`.

**Points.** A node that reads an unknown is computed once per point of it (a scenario), and once per point
of every further unknown it reads under that choice:
- `variants`: each variant;
- `interval`, and the `uniform` and `triangular` distributions: `low` and `high`;
- `empirical`: each of its values, named by position (`0`, `1`, ...).
An output's range is its lowest and highest value over its scenarios. For variants and empirical values
that is every value the unknown can take. For an interval or a continuous distribution it is the range
whenever the output is monotone in the unknown, which one-at-a-time sensitivity assumes too (design 4.8);
the same unknown is one value in a scenario, so `x - x` is 0. `beta_from_samples` has no points here: its
bounds come with the random processes (Wilson), so reading it is `evaluation_unsupported`.

**Sensitivity.** Two scenarios of an output that chose different points of one unknown, and the same point
of every other unknown both chose, are neighbours in it. The unknown's impact is the largest change between
such neighbours at export precision: round half even of value x `unknowns.scale` (1000: milli); a boolean
counts as 0 or 1. Each output lists its `unknowns.top_n` largest impacts, ties by id.

**The no-effect rule.** An unknown makes an output provisional only when its impact is above zero. So does
an unknown the output depends on without choosing it (it is unresolved): one that a pending rule it read
names, since that rule is uncertain only through it, or one whose points a maximum ran over. An unresolved
unknown that the output chose too, with impact zero, changes nothing.

**Sampling** (`unknowns.samples`, none by default). Every distribution unknown has its own stream, keyed by
(`unknowns.seed`, its stream id). Sample s is one block of `numpy.random.Philox` at counter s, so the chunk
k of `unknowns.chunk_size` samples starts at counter k x chunk_size, and no chunking changes a sample. A draw
on (0, 1) is the middle of one of `unknowns.resolution` cells. A uniform value is low + (high - low) x draw;
a triangular one comes from its inverse distribution function, the square root rounded down to a multiple
of 1 / resolution; an empirical one is the value at position floor(word x n / 2**64). All of it is integer
and exact, so a sample is the same on every platform and under every string-hash seed. The tallies are
integers held to the int64 range.
"""

from __future__ import annotations

import hashlib
import math
from dataclasses import dataclass
from fractions import Fraction
from typing import Any, Iterable, Protocol

from acmodel.check.checker import Entry
from acmodel.check.walk import at
from acmodel.codes import EXPR_EVAL_FAILED
from acmodel.core.common import CONFIRMED, PROVISIONAL, RULE_STATUS, worst
from acmodel.envelope import Failure, error
from acmodel.expr import Duration, Value

STREAM_SCHEMA = "acmodel.stream.v1"
INT64 = (-(2**63), 2**63 - 1)
BOUNDS = ("low", "mode", "high")
# How a sampled node is kept in the cache (`graph.VALUE_FORMAT` is the other kinds').
SAMPLED_FORMAT = {"tally": ["unknowns", "count", "total", "low", "high"], "result": ["tallies", "blocked"]}


class Chosen(Protocol):
    """What the sensitivity reads of a scenario (`graph.Scenario`)."""

    unknowns: tuple[tuple[str, str], ...]
    value: Value
    status: str
    unresolved: tuple[str, ...]


def _magnitude(value: Value) -> Any:
    return value.seconds if type(value) is Duration else value


# Points ---------------------------------------------------------------------------------------------


def family(entry: Entry) -> str:
    """`interval`, `variants`, or the family of a distribution."""
    return entry.doc["distribution"]["family"] if entry.kind == "distribution" else entry.kind


def unsupported(entry: Entry) -> str | None:
    """What reading this unknown cannot evaluate yet (`beta_from_samples`), or None."""
    return "beta_from_samples" if family(entry) == "beta_from_samples" else None


def points(entry: Entry) -> list[str]:
    """The points of an unknown that has its range form, sorted."""
    kind = family(entry)
    if kind == "variants":
        return sorted(entry.doc["variants"])
    if kind == "empirical":
        return sorted(str(index) for index in range(len(entry.doc["distribution"]["values"])))
    return ["high", "low"]


def bounds(entry: Entry) -> dict[str, str]:
    """The JSON pointer of each expression of an interval or a distribution: `low`, `mode` and `high` in that
    order, or the values of an empirical one by position."""
    if entry.kind == "interval":
        return {name: at(entry.ref, "interval", name) for name in ("low", "high")}
    form = entry.doc["distribution"]
    if form["family"] == "empirical":
        return {str(index): at(entry.ref, "distribution", "values", index) for index in range(len(form["values"]))}
    return {name: at(entry.ref, "distribution", name) for name in BOUNDS if name in form}


def check_range(entry: Entry, values: dict[str, Value]) -> None:
    """low <= mode <= high, or `expr_eval_failed` (`range_invalid`) at the range form."""
    ordered = [_magnitude(values[name]) for name in BOUNDS if name in values]
    if any(first > second for first, second in zip(ordered, ordered[1:])):
        detail = {"reason": "range_invalid", "operator": family(entry), "position": 0}
        raise Failure(error(EXPR_EVAL_FAILED, detail, at(entry.ref, entry.kind)))


def rule_status(entry: Entry) -> tuple[str, tuple[str, ...]]:
    """A rule's own status, and the unknowns it leaves unresolved. A pending rule is usable only through the
    unknowns it names (the static check makes sure one has a range), so it counts as confirmed with those
    unresolved; any other rule counts as its status."""
    if entry.doc["status"] == "pending":
        return CONFIRMED, tuple(sorted(entry.doc.get("unknowns", ())))
    return RULE_STATUS[entry.doc["status"]], ()


# Sensitivity and the no-effect rule -----------------------------------------------------------------


def quantize(value: Value, scale: int) -> int:
    """A value at export precision: round half even of value x scale; true is scale and false 0."""
    if type(value) is bool:
        return scale if value else 0
    return round(_magnitude(value) * scale)


def impacts(scenarios: Iterable[Chosen], scale: int) -> dict[str, int]:
    """Each unknown the scenarios chose, with its impact in units of 1 / scale, by id."""
    marked = [(dict(scenario.unknowns), quantize(scenario.value, scale)) for scenario in scenarios]
    found = {unknown: 0 for choices, _ in marked for unknown in choices}
    for index, (first, value) in enumerate(marked):
        for second, other in marked[index + 1:]:
            apart = [unknown for unknown, point in first.items() if second.get(unknown, point) != point]
            if len(apart) == 1:
                found[apart[0]] = max(found[apart[0]], abs(value - other))
    return dict(sorted(found.items()))


def status(scenarios: Iterable[Chosen], scale: int) -> str:
    """The status of an output that is not blocked: the worst of what it read, and provisional when an
    unknown it chose has an impact or an unknown it left unresolved is not shown to have none."""
    scenarios = list(scenarios)
    base = worst(*(scenario.status for scenario in scenarios))
    effect = impacts(scenarios, scale)
    unresolved = {unknown for scenario in scenarios for unknown in scenario.unresolved}
    if any(effect.values()) or any(effect.get(unknown, 1) for unknown in unresolved):
        return worst(base, PROVISIONAL)
    return base


def sensitivity(scenarios: Iterable[Chosen], scale: int, top_n: int) -> list[dict[str, str]] | None:
    """The `top_n` largest impacts of the unknowns an output chose, ties by id; None when it chose none."""
    effect = impacts(scenarios, scale)
    if not effect:
        return None
    ranked = sorted(effect.items(), key=lambda item: (-item[1], item[0]))[:top_n]
    return [{"unknown": unknown, "impact": str(Fraction(impact, scale))} for unknown, impact in ranked]


# Sampling -------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Tally:
    """The samples of an output under one choice of its other unknowns, at export precision."""

    unknowns: tuple[tuple[str, str], ...]
    count: int
    total: int
    low: int
    high: int


@dataclass(frozen=True)
class Sampled:
    """A sampled node: one tally per choice of the output's other unknowns, or the errors that block it."""

    tallies: tuple[Tally, ...] = ()
    blocked_by: frozenset[int] = frozenset()


def stream(unknown_id: str) -> int:
    """The Philox stream id of an unknown: the first 8 bytes of a sha256 over its id, never `hash()`."""
    return int.from_bytes(hashlib.sha256(f"{STREAM_SCHEMA}:{unknown_id}".encode("utf-8")).digest()[:8], "big")


def words(seed: int, stream_id: int, start: int, count: int) -> list[int]:
    """One word for each of the samples start ... start + count - 1 of a stream: the first of the four words
    of its Philox block. The generator starts at counter `start` and gives whole blocks, so the block of a
    sample does not depend on where its chunk starts. numpy is imported only when sampling."""
    import numpy

    generator = numpy.random.Philox(
        key=numpy.array([seed, stream_id], dtype=numpy.uint64),
        counter=numpy.array([start, 0, 0, 0], dtype=numpy.uint64),
    )
    return [int(word) for word in generator.random_raw(4 * count).reshape(count, 4)[:, 0]]


def draw(word: int, resolution: int) -> Fraction:
    """A draw on (0, 1): the middle of cell floor(word x resolution / 2**64) of `resolution` equal cells."""
    return Fraction(2 * ((word * resolution) >> 64) + 1, 2 * resolution)


def pick(word: int, count: int) -> str:
    """The point (position) of the empirical value that a sample takes: floor(word x count / 2**64)."""
    return str((word * count) >> 64)


def _root(number: Fraction, resolution: int) -> Fraction:
    """The square root of `number` >= 0, rounded down to a multiple of 1 / resolution."""
    return Fraction(math.isqrt(math.floor(number * resolution * resolution)), resolution)


def sample(entry: Entry, values: dict[str, Value], word: int, resolution: int) -> Value:
    """The value of a uniform or triangular distribution at the sample `word`, from its bounds `values`."""
    low, high = _magnitude(values["low"]), _magnitude(values["high"])
    width, u = high - low, draw(word, resolution)
    if family(entry) == "uniform":
        number = low + width * u
    else:
        mode = _magnitude(values["mode"])
        if width == 0:
            number = low
        elif u * width < mode - low:
            number = low + _root(u * width * (mode - low), resolution)
        else:
            number = high - _root((1 - u) * width * (high - mode), resolution)
    return Duration(number) if type(values["low"]) is Duration else number


def tally(found: dict[tuple[tuple[str, str], ...], list[int]], unknowns: tuple[tuple[str, str], ...], value: int) -> int | None:
    """Add one sampled value to the tally of `unknowns` ([count, total, low, high]). Return the number that
    would leave the int64 range instead, or None."""
    count, total, low, high = found.get(unknowns, [0, 0, value, value])
    for number in (value, count + 1, total + value):
        if not INT64[0] <= number <= INT64[1]:
            return number
    found[unknowns] = [count + 1, total + value, min(low, value), max(high, value)]
    return None


def tallies(found: dict[tuple[tuple[str, str], ...], list[int]]) -> tuple[Tally, ...]:
    return tuple(Tally(unknowns, *numbers) for unknowns, numbers in sorted(found.items()))


def render_sampled(sampled: Sampled, scale: int) -> list[dict[str, Any]]:
    """Each tally as `run` shows it: the samples, their mean and their lowest and highest value at export
    precision, as exact rationals."""
    return [
        {
            "unknowns": dict(item.unknowns),
            "samples": item.count,
            "mean": str(Fraction(item.total, item.count * scale)),
            "low": str(Fraction(item.low, scale)),
            "high": str(Fraction(item.high, scale)),
        }
        for item in sampled.tallies
    ]


def encode_tallies(sampled: Sampled) -> list[Any]:
    return [[[list(pair) for pair in item.unknowns], item.count, item.total, item.low, item.high] for item in sampled.tallies]


def decode_tallies(raw: list[Any]) -> tuple[Tally, ...]:
    return tuple(Tally(tuple(tuple(pair) for pair in item[0]), *item[1:]) for item in raw)
