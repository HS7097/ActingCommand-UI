"""Unknowns (#424 design 3.5, 4.2, 4.5 and 4.8; contracts/formats.md, "Unknowns"): how the graph (`core.graph`)
evaluates them, the sensitivity of each output with the no-effect rule, and the Monte Carlo sampling of the
distributions. The keys are `config.Unknowns`.

**Discrete unknowns** are evaluated at points: a node that reads one is computed once per point of it (a
scenario), and once per point of every further one it reads under that choice. The points of `variants` are its
variants, those of an `empirical` distribution its values by position (`0`, `1`, ...).

**Continuous unknowns** (`interval`, and the `uniform` and `triangular` distributions) are one enclosure each,
[low, high], and every node that reads one is computed by interval arithmetic (`expr.interval`). So the range of
every output is a sound enclosure: never narrower than the true range. It is exact where the structural
monotonic check proves it (each unknown reaches the value only through operators monotone in one and the same
direction), and may be wider elsewhere. Bounds that may be out of order (low above high, a triangular mode
outside them) are `range_invalid`. `beta_from_samples` is not evaluated yet: its bounds come with the random
processes (Wilson), so reading it is `evaluation_unsupported`.

**The no-effect rule** decides on the sound range. An output that read unknowns is provisional when its range
holds more than one value at export precision (value x `unknowns.scale`, 1000: milli, round half even; a
boolean counts as 0 or 1, an undecided one as both). So is an output that depends on an unknown it did not read
(an unresolved one): one that a pending rule it read names (that rule is uncertain only through it), or one
whose points a maximum ran over.

**Sensitivity** lists, for each unknown an output read, its impact: the largest change of the output at export
precision while that unknown moves and the others are held at one of their points (each point of a discrete
one; the low and the high of a continuous one). Between two scenarios that differ in one discrete unknown, the
change is the largest difference of their enclosures. For a continuous unknown it is the width of the output's
enclosure with that unknown free: from the run itself when the output reads no other continuous unknown, and
otherwise from a `pinned` computation per choice of ends of the others (the `impacts` node). Each output lists
its `sensitivity.top_n` largest impacts, ties by id.

**Sampling** (`unknowns.samples`, none by default). Every distribution unknown has its own stream, keyed by
(`unknowns.seed`, its stream id). Sample s is one block of `numpy.random.Philox` at counter s, so the chunk k of
`unknowns.chunk_size` samples starts at counter k x chunk_size, and no chunking changes a sample. A draw on
(0, 1) is the middle of one of `unknowns.resolution` cells. A uniform value is low + (high - low) x draw; a
triangular one comes from its inverse distribution function, the square root rounded down to a multiple of
1 / resolution; an empirical one is the value at position floor(word x n / 2**64). All of it is integer and
exact, so a sample is the same on every platform and under every string-hash seed. The intervals stay free, so a
sampled value can be an enclosure; the tallies keep its low and its high, as integers held to the int64 range.
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
from acmodel.expr.interval import Interval, Undecided, bounds as value_bounds, combine, magnitude, make

STREAM_SCHEMA = "acmodel.stream.v1"
INT64 = (-(2**63), 2**63 - 1)
BOUNDS = ("low", "mode", "high")
PINS = ("high", "low")  # the ends a continuous unknown is held at for the impacts of the others
# How the sampled and impacts nodes are kept in the cache (`graph.VALUE_FORMAT` is the other kinds').
SAMPLED_FORMAT = {
    "tally": ["unknowns", "count", "total_low", "total_high", "low", "high"], "result": ["tallies", "blocked"],
}
IMPACTS_FORMAT = {"impact": ["unknown", "impact"], "result": ["impacts", "blocked"]}


class Chosen(Protocol):
    """What the sensitivity reads of a scenario (`graph.Scenario`)."""

    unknowns: tuple[tuple[str, str], ...]
    value: Value
    status: str
    unresolved: tuple[str, ...]
    free: tuple[str, ...]


# Kinds and points -----------------------------------------------------------------------------------


def family(entry: Entry) -> str:
    """`interval`, `variants`, or the family of a distribution."""
    return entry.doc["distribution"]["family"] if entry.kind == "distribution" else entry.kind


def continuous(entry: Entry) -> bool:
    """An interval, uniform or triangular unknown: one enclosure, computed by interval arithmetic."""
    return family(entry) in ("interval", "uniform", "triangular")


def unsupported(entry: Entry) -> str | None:
    """What reading this unknown cannot evaluate yet (`beta_from_samples`), or None."""
    return "beta_from_samples" if family(entry) == "beta_from_samples" else None


def points(entry: Entry) -> list[str]:
    """The points of a discrete unknown (variants, empirical), sorted."""
    if entry.kind == "variants":
        return sorted(entry.doc["variants"])
    return sorted(str(index) for index in range(len(entry.doc["distribution"]["values"])))


def bounds(entry: Entry) -> dict[str, str]:
    """The JSON pointer of each expression of an interval or distribution: `low`, `mode` and `high` in that
    order, or the values of an empirical one by position."""
    if entry.kind == "interval":
        return {name: at(entry.ref, "interval", name) for name in ("low", "high")}
    form = entry.doc["distribution"]
    if form["family"] == "empirical":
        return {str(index): at(entry.ref, "distribution", "values", index) for index in range(len(form["values"]))}
    return {name: at(entry.ref, "distribution", name) for name in BOUNDS if name in form}


def check_range(entry: Entry, values: dict[str, Value]) -> None:
    """low <= mode <= high for every value their own unknowns allow, or `expr_eval_failed` (`range_invalid`) at
    the range form: bounds that may cross are refused, never guessed."""
    ordered = [value_bounds(values[name]) for name in BOUNDS if name in values]
    if any(magnitude(first[1]) > magnitude(second[0]) for first, second in zip(ordered, ordered[1:])):
        detail = {"reason": "range_invalid", "operator": family(entry), "position": 0}
        raise Failure(error(EXPR_EVAL_FAILED, detail, at(entry.ref, entry.kind)))


def enclosure(entry: Entry, values: dict[str, Value]) -> Value:
    """A continuous unknown as an enclosure: from the lowest value of its low bound to the highest of its high
    bound. It rises with itself and with both bounds, so it keeps their directions."""
    (low, _, low_signs), (_, high, high_signs) = value_bounds(values["low"]), value_bounds(values["high"])
    return make(low, high, combine({entry.id: 1}, low_signs, high_signs))


def rule_status(entry: Entry) -> tuple[str, tuple[str, ...]]:
    """A rule's own status, and the unknowns it leaves unresolved. A pending rule is usable only through the
    unknowns it names (the static check makes sure one has a range), so it counts as confirmed with those
    unresolved; any other rule counts as its status."""
    if entry.doc["status"] == "pending":
        return CONFIRMED, tuple(sorted(entry.doc.get("unknowns", ())))
    return RULE_STATUS[entry.doc["status"]], ()


# Export precision, the no-effect rule and sensitivity ------------------------------------------------


def quantize(value: Value, scale: int) -> tuple[int, int]:
    """The lowest and highest value at export precision: round half even of value x scale; true is scale and
    false 0, an undecided boolean both."""
    if type(value) is bool:
        return (scale, scale) if value else (0, 0)
    if type(value) is Undecided:
        return 0, scale
    low, high, _ = value_bounds(value)
    return round(magnitude(low) * scale), round(magnitude(high) * scale)


def exact(value: Value) -> bool:
    """Whether a value's range is exact: a single value, or an enclosure the monotonic check proves."""
    return value.exact if type(value) is Interval else type(value) is not Undecided


def read(scenarios: Iterable[Chosen]) -> set[str]:
    """The unknowns an output read: the discrete ones it chose and the continuous ones it computed with."""
    return {unknown for s in scenarios for unknown in [u for u, _ in s.unknowns] + list(s.free)}


def status(scenarios: Iterable[Chosen], scale: int) -> str:
    """The status of an output that is not blocked: the worst of what it read, and provisional when its sound
    range holds more than one value at export precision, or when it depends on an unknown it did not read."""
    scenarios = list(scenarios)
    base = worst(*(scenario.status for scenario in scenarios))
    quanta = [quantize(scenario.value, scale) for scenario in scenarios]
    spread = min(low for low, _ in quanta) != max(high for _, high in quanta)
    unresolved = {unknown for scenario in scenarios for unknown in scenario.unresolved} - read(scenarios)
    return worst(base, PROVISIONAL) if spread or unresolved else base


def impacts(scenarios: Iterable[Chosen], scale: int, pinned: dict[str, int] | None) -> dict[str, int]:
    """Each unknown an output read, with its impact in units of 1 / scale (see the module text). `pinned` holds
    the impacts of the continuous unknowns computed with the others at their ends (the `impacts` node), when the
    output reads more than one."""
    marked = [(dict(s.unknowns), set(s.free), quantize(s.value, scale)) for s in scenarios]
    found = {unknown: 0 for unknown in read(scenarios)}
    for index, (first, _, (low, high)) in enumerate(marked):
        for second, _, (other_low, other_high) in marked[index + 1:]:
            apart = [unknown for unknown, point in first.items() if second.get(unknown, point) != point]
            if len(apart) == 1:
                found[apart[0]] = max(found[apart[0]], high - other_low, other_high - low)
    if pinned is not None:
        found.update(pinned)
    else:
        for _, free, (low, high) in marked:
            for unknown in free:
                found[unknown] = max(found[unknown], high - low)
    return dict(sorted(found.items()))


def sensitivity(scenarios: Iterable[Chosen], scale: int, top_n: int, pinned: dict[str, int] | None) -> list[dict[str, str]] | None:
    """The `top_n` largest impacts of the unknowns an output read, ties by id; None when it read none."""
    effect = impacts(scenarios, scale, pinned)
    if not effect:
        return None
    ranked = sorted(effect.items(), key=lambda item: (-item[1], item[0]))[:top_n]
    return [{"unknown": unknown, "impact": str(Fraction(impact, scale))} for unknown, impact in ranked]


@dataclass(frozen=True)
class Impacts:
    """An impacts node: the impact of each continuous unknown an output reads, with the other continuous ones
    held at their ends, or the errors that block it."""

    impacts: tuple[tuple[str, int], ...] = ()
    blocked_by: frozenset[int] = frozenset()


def widths(scenarios: Iterable[Chosen], unknown: str, scale: int) -> int:
    """The largest width at export precision of the scenarios that computed with `unknown`."""
    found = 0
    for scenario in scenarios:
        if unknown in scenario.free:
            low, high = quantize(scenario.value, scale)
            found = max(found, high - low)
    return found


# Sampling -------------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Tally:
    """The samples of an output under one choice of its discrete unknowns, at export precision: the totals of
    their lows and highs, and the lowest low and the highest high."""

    unknowns: tuple[tuple[str, str], ...]
    count: int
    total_low: int
    total_high: int
    low: int
    high: int


@dataclass(frozen=True)
class Sampled:
    """A sampled node: one tally per choice of the output's discrete unknowns, or the errors that block it."""

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
    """The value of a uniform or triangular distribution at the sample `word`, from its bounds `values` (single
    values: the caller refuses bounds that other unknowns leave open)."""
    low, high = magnitude(values["low"]), magnitude(values["high"])
    width, u = high - low, draw(word, resolution)
    if family(entry) == "uniform":
        number = low + width * u
    else:
        mode = magnitude(values["mode"])
        if width == 0:
            number = low
        elif u * width < mode - low:
            number = low + _root(u * width * (mode - low), resolution)
        else:
            number = high - _root((1 - u) * width * (high - mode), resolution)
    return Duration(number) if type(values["low"]) is Duration else number


def tally(found: dict[tuple[tuple[str, str], ...], list[int]], unknowns: tuple[tuple[str, str], ...], value: tuple[int, int]) -> int | None:
    """Add one sampled value (its low and high at export precision) to the tally of `unknowns`
    ([count, total low, total high, low, high]). Return the number that would leave the int64 range instead,
    or None."""
    low, high = value
    count, total_low, total_high, lowest, highest = found.get(unknowns, [0, 0, 0, low, high])
    for number in (low, high, count + 1, total_low + low, total_high + high):
        if not INT64[0] <= number <= INT64[1]:
            return number
    found[unknowns] = [count + 1, total_low + low, total_high + high, min(lowest, low), max(highest, high)]
    return None


def tallies(found: dict[tuple[tuple[str, str], ...], list[int]]) -> tuple[Tally, ...]:
    return tuple(Tally(unknowns, *numbers) for unknowns, numbers in sorted(found.items()))


def render_sampled(sampled: Sampled, scale: int) -> list[dict[str, Any]]:
    """Each tally as `run` shows it, at export precision as exact rationals: the samples, the enclosure of their
    mean, and their lowest and highest value."""
    return [
        {
            "unknowns": dict(item.unknowns),
            "samples": item.count,
            "mean": {"low": str(Fraction(item.total_low, item.count * scale)),
                     "high": str(Fraction(item.total_high, item.count * scale))},
            "low": str(Fraction(item.low, scale)),
            "high": str(Fraction(item.high, scale)),
        }
        for item in sampled.tallies
    ]


def encode_tallies(sampled: Sampled) -> list[Any]:
    return [[[list(pair) for pair in item.unknowns], item.count, item.total_low, item.total_high, item.low, item.high]
            for item in sampled.tallies]


def decode_tallies(raw: list[Any]) -> tuple[Tally, ...]:
    return tuple(Tally(tuple(tuple(pair) for pair in item[0]), *item[1:]) for item in raw)
