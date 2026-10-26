"""The generic search for teams (#424 design 4.6, design-py 4.6; contracts/formats.md, "The search").

**The problem.** Disjoint teams to fill from one roster. An entity is an opaque id with tags, which only the
legality rules read, and data, which only the engine reads.
- A team's **slots**, in order: each takes `count` distinct entities that have every tag of its `requires` and
  none of its `excludes`.
- A team's **quotas**: at least `low` and at most `high` of its members have the quota's tag.
- **Disjoint teams**: an entity is a member of at most one team.

A team's **row** is its members slot by slot, each slot's members in id order. Its value is the problem's metric
of the engine's `evaluate`, exact (the `Fraction` of the number, never rounded); an assignment's value is the sum
of its teams' values, and the search maximises it.

**Steps.**
1. *Screen.* Every legal row of each team is enumerated with numpy: the product of its slots' combinations, in
   chunks of `search.screen_chunk_rows` rows, less the rows that repeat an entity or break a quota. Each row is
   scored by gathers of the engine's `predict` terms (an additive term per entity, a pairwise term per pair), each
   rounded half to even at `search.screen_scale` to an int64. The screen only decides which rows are evaluated
   first; every value shown comes from `evaluate`.
2. *Proof mode.* When all teams have at most `search.exact_max` legal rows together, every row is evaluated and
   the best assignment is proven.
3. *Otherwise* the top `search.top_k` rows of each team by screen score are evaluated. The best assignment of the
   evaluated rows is taken, the rows its local swaps give (a member replaced by an entity no team uses, two members
   exchanged between two slots or two teams) that are legal and not evaluated yet are evaluated, and the best
   assignment is taken again, until its swaps give no new row. While the evaluated rows hold no assignment of all
   the teams, the next `search.top_k` rows of each team are evaluated.

The **best assignment** of one team is its best evaluated row; of two or more, a depth-first search over their
evaluated rows, best first, bounded by the best values of the teams left.

**Budget.** `search.node_limit` bounds the team rows one search evaluates, and the nodes of each assignment of two
or more teams; never a time. Reaching it is the warning `search_budget_exhausted`; the result is the best found,
`proven: false`.

**Ties** break by id, then index: of two rows (or assignments) of equal value, first the one whose members, team by
team and each team in id order, come first in id order; then the one whose members slot by slot come first. The
screen orders equal scores the same way, so no order depends on the enumeration, the chunks or the workers.

**Warm start.** A search returns its `State`: the roster it screened, the rounded terms and every screened row with
its scores. Given it, a search of the same teams screens only the rows that hold an entity that is new, gone, or
whose tags or terms changed (a changed pairwise term changes both its entities); the other rows keep their scores.
Its result is a cold search's: the same rows with the same scores, evaluated alike.

**Approximation loss.** The additive pick is what the additive terms alone choose: each team in order takes its
legal row with the largest sum of additive terms among the entities the earlier teams left. Its rows are evaluated
first. The loss is the best value less the pick's value: exact when the search is proven, a lower bound otherwise.

**Failures** are `search_failed`, and the engine host's codes for what an engine answers; every one ends the
search. A problem is built by the core, so a malformed one is a defect (`ValueError`), not an input.
"""

from __future__ import annotations

import itertools
import math
from dataclasses import dataclass, field
from fractions import Fraction
from typing import TYPE_CHECKING, Any, Iterator, Mapping, Sequence

import numpy as np

from acmodel.cache.keys import canonical
from acmodel.codes import ENGINE_PROTOCOL_ERROR, SEARCH_BUDGET_EXHAUSTED, SEARCH_FAILED
from acmodel.config import SearchSettings
from acmodel.engine import client, protocol
from acmodel.engine.protocol import ENTITIES
from acmodel.envelope import Failure, error

if TYPE_CHECKING:
    from acmodel.cache.session import Session
    from acmodel.engine.pool import Pool

INT64_MAX = 2**63 - 1
EXACT, HEURISTIC = "exact", "heuristic"
COLD, WARM = "cold", "warm"
EVALUATE, ASSIGN = "evaluate", "assign"
PREDICT_MEMBERS = ("additive", "depends_on", "pairwise")

Row = tuple[int, ...]  # a team's members slot by slot, as indices into the roster's sorted ids
Signature = tuple[dict[str, int], dict[tuple[str, str], int]]  # rounded additive and pairwise terms, non-zero


@dataclass(frozen=True)
class Slot:
    slot_id: str
    count: int
    requires: tuple[str, ...] = ()
    excludes: tuple[str, ...] = ()


@dataclass(frozen=True)
class Quota:
    tag: str
    low: int = 0
    high: int | None = None


@dataclass(frozen=True)
class Team:
    team_id: str
    slots: tuple[Slot, ...]
    quotas: tuple[Quota, ...] = ()
    context: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class Entity:
    entity_id: str
    tags: tuple[str, ...] = ()
    data: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class Problem:
    metric: str
    teams: tuple[Team, ...]
    entities: tuple[Entity, ...]


@dataclass(frozen=True)
class _Table:
    """One team's screened rows: members (roster indices, slot by slot), screen score and additive score."""

    rows: np.ndarray
    score: np.ndarray
    additive: np.ndarray


@dataclass(frozen=True)
class State:
    """What a search leaves for a warm start (see the module's text)."""

    shape: bytes
    ids: tuple[str, ...]
    tags: tuple[frozenset[str], ...]
    terms: tuple[Signature | None, ...]
    tables: tuple[_Table, ...]


@dataclass(frozen=True)
class Result:
    """A search's result: `data` as formats.md ("The search") shows it, its warnings, and its state."""

    data: dict[str, Any]
    warnings: list[dict[str, Any]]
    state: State


@dataclass(frozen=True)
class _Roster:
    ids: tuple[str, ...]  # sorted by code point
    index: dict[str, int]
    tags: tuple[frozenset[str], ...]
    data: tuple[dict[str, Any], ...]


@dataclass(frozen=True)
class _Layout:
    """One team on one roster: who may fill each slot, where each slot sits in a row, the quotas."""

    team: Team
    eligible: tuple[np.ndarray, ...]  # per slot: roster indices, ascending
    eligible_sets: tuple[frozenset[int], ...]
    bounds: tuple[tuple[int, int], ...]  # per slot: its positions in a row
    slot_of: tuple[int, ...]  # per position: its slot
    quotas: tuple[tuple[np.ndarray, frozenset[int], int, int | None], ...]  # mask, holders, low, high
    members: tuple[int, ...]  # every entity some slot may take, ascending
    parts: tuple[np.ndarray, ...]  # per slot: its combinations, in lexicographic order


@dataclass(frozen=True)
class _Terms:
    additive: np.ndarray  # int64 per roster entity
    pairwise: np.ndarray  # int64, roster by roster, symmetric


def _key(row: Row) -> tuple[Row, Row]:
    """A row's tie order: its members in id order, then slot by slot (indices follow the ids' order)."""
    return tuple(sorted(row)), row


def _row(array: np.ndarray) -> Row:
    return tuple(array.tolist())


def _text(value: Fraction | None) -> str | None:
    return None if value is None else str(value)


def _failed(reason: str, **values: Any) -> Failure:
    return Failure(error(SEARCH_FAILED, {"reason": reason, **values}))


# The problem ------------------------------------------------------------------------------------------------


def _check(problem: Problem) -> None:
    """A problem the core builds is its own: a malformed one is a defect, raised as `ValueError`."""

    def text(value: Any, what: str) -> None:
        if type(value) is not str or not value:
            raise ValueError(f"{what} is not a non-empty string: {value!r}")

    def json_object(value: Any, what: str) -> None:
        if type(value) is not dict:
            raise ValueError(f"{what} is not a JSON object")
        try:
            protocol.check_json(value)
        except protocol.NotJson as exc:
            raise ValueError(f"{what} is not JSON at {exc.pointer!r}: {exc.found}") from None

    text(problem.metric, "the metric")
    if not problem.teams:
        raise ValueError("a search needs at least one team")
    seen: set[str] = set()
    for entity in problem.entities:
        text(entity.entity_id, "an entity id")
        if entity.entity_id in seen:
            raise ValueError(f"entity {entity.entity_id!r} is listed twice")
        seen.add(entity.entity_id)
        for tag in entity.tags:
            text(tag, "a tag")
        json_object(entity.data, f"the data of entity {entity.entity_id!r}")
    seen = set()
    for team in problem.teams:
        text(team.team_id, "a team id")
        if team.team_id in seen:
            raise ValueError(f"team {team.team_id!r} is listed twice")
        seen.add(team.team_id)
        json_object(team.context, f"the context of team {team.team_id!r}")
        if not team.slots:
            raise ValueError(f"team {team.team_id!r} has no slot")
        slots: set[str] = set()
        for slot in team.slots:
            text(slot.slot_id, "a slot id")
            if slot.slot_id in slots:
                raise ValueError(f"slot {slot.slot_id!r} of team {team.team_id!r} is listed twice")
            slots.add(slot.slot_id)
            if type(slot.count) is not int or slot.count < 1:
                raise ValueError(f"slot {slot.slot_id!r} of team {team.team_id!r} has no positive count")
            for tag in (*slot.requires, *slot.excludes):
                text(tag, "a tag")
        for quota in team.quotas:
            text(quota.tag, "a tag")
            if type(quota.low) is not int or quota.low < 0:
                raise ValueError(f"a quota of team {team.team_id!r} has a negative or non-integer low")
            if quota.high is not None and (type(quota.high) is not int or quota.high < quota.low):
                raise ValueError(f"a quota of team {team.team_id!r} has a high below its low")


def _roster(problem: Problem) -> _Roster:
    entities = sorted(problem.entities, key=lambda entity: entity.entity_id)
    ids = tuple(entity.entity_id for entity in entities)
    return _Roster(
        ids=ids,
        index={entity: index for index, entity in enumerate(ids)},
        tags=tuple(frozenset(entity.tags) for entity in entities),
        data=tuple(entity.data for entity in entities),
    )


def _layout(team: Team, roster: _Roster, max_rows: int) -> _Layout:
    """The team's layout; `space_too_large` before anything is enumerated when its slot product has more than
    `max_rows` rows."""
    eligible, bounds, slot_of = [], [], []
    start = 0
    for position, slot in enumerate(team.slots):
        need, avoid = frozenset(slot.requires), frozenset(slot.excludes)
        chosen = [index for index, tags in enumerate(roster.tags) if need <= tags and not avoid & tags]
        eligible.append(np.array(chosen, dtype=np.int32))
        bounds.append((start, start + slot.count))
        slot_of.extend([position] * slot.count)
        start += slot.count
    rows = math.prod(math.comb(len(chosen), slot.count) for chosen, slot in zip(eligible, team.slots))
    if rows > max_rows:
        raise _failed("space_too_large", team=team.team_id, rows=rows, screen_max_rows=max_rows)
    quotas = []
    for quota in team.quotas:
        holders = [index for index, tags in enumerate(roster.tags) if quota.tag in tags]
        mask = np.zeros(len(roster.ids), dtype=bool)
        mask[np.array(holders, dtype=np.intp)] = True
        quotas.append((mask, frozenset(holders), quota.low, quota.high))
    return _Layout(
        team=team,
        eligible=tuple(eligible),
        eligible_sets=tuple(frozenset(chosen.tolist()) for chosen in eligible),
        bounds=tuple(bounds),
        slot_of=tuple(slot_of),
        quotas=tuple(quotas),
        members=tuple(sorted(set().union(*(chosen.tolist() for chosen in eligible)))),
        # An empty product has no row at all: then no slot's combinations are made, however many one has.
        parts=tuple(
            _subsets(chosen, slot.count) if rows else np.zeros((0, slot.count), dtype=np.int32)
            for chosen, slot in zip(eligible, team.slots)
        ),
    )


def _shape(problem: Problem, settings: SearchSettings) -> bytes:
    """What a warm start needs unchanged: the teams' slots and quotas, and the screen's scale."""
    teams = [
        [
            team.team_id,
            [[slot.slot_id, slot.count, sorted(slot.requires), sorted(slot.excludes)] for slot in team.slots],
            [[quota.tag, quota.low, quota.high] for quota in team.quotas],
        ]
        for team in problem.teams
    ]
    return canonical({"scale": settings.screen_scale, "teams": teams})


# The screen -------------------------------------------------------------------------------------------------


def _subsets(chosen: np.ndarray, count: int) -> np.ndarray:
    """Every `count`-subset of the ascending indices `chosen`, in lexicographic order: (subsets, count) int32."""
    total = math.comb(len(chosen), count)
    flat = np.fromiter(
        itertools.chain.from_iterable(itertools.combinations(chosen.tolist(), count)), dtype=np.int32,
        count=total * count,
    )
    return flat.reshape(total, count)


def _product(parts: list[np.ndarray], chunk: int) -> Iterator[np.ndarray]:
    """The rows of the product of the slots' combinations, in chunks of `chunk` rows: row i of the product picks,
    in each slot, the combination its mixed-radix digits name, so the chunks never change a row."""
    sizes = [len(part) for part in parts]
    total = math.prod(sizes)
    strides = [math.prod(sizes[position + 1:]) for position in range(len(sizes))]
    for start in range(0, total, chunk):
        flat = np.arange(start, min(start + chunk, total), dtype=np.int64)
        yield np.concatenate(
            [part[(flat // stride) % size] for part, size, stride in zip(parts, sizes, strides)], axis=1
        )


def _legal(rows: np.ndarray, layout: _Layout) -> np.ndarray:
    """Which rows repeat no entity and meet every quota."""
    keep = np.ones(len(rows), dtype=bool)
    if len(layout.bounds) > 1:
        ordered = np.sort(rows, axis=1)
        keep &= (ordered[:, 1:] != ordered[:, :-1]).all(axis=1)
    for mask, _, low, high in layout.quotas:
        count = mask[rows].sum(axis=1)
        keep &= count >= low
        if high is not None:
            keep &= count <= high
    return keep


def _enumerate(layout: _Layout, chunk: int, touching: np.ndarray | None) -> Iterator[np.ndarray]:
    """A team's legal rows, in chunks: all of them, or (`touching`, a mask over the roster) exactly those that
    hold an entity of the mask. These are the rows whose first slot with such an entity is slot p, for each p:
    the slots before p take combinations without one, slot p one with one, the slots after it any."""
    parts = list(layout.parts)
    if touching is None:
        plans = [parts]
    else:
        hits = [touching[part].any(axis=1) for part in parts]
        plans = [
            [part[~hit] for part, hit in zip(parts[:first], hits[:first])]
            + [parts[first][hits[first]]]
            + parts[first + 1:]
            for first in range(len(parts))
        ]
    for plan in plans:
        for rows in _product(plan, chunk):
            yield rows[_legal(rows, layout)]


def _score(rows: np.ndarray, terms: _Terms | None) -> tuple[np.ndarray, np.ndarray]:
    """Each row's screen score (its additive terms and its pairs' pairwise terms) and additive score."""
    if terms is None:
        zeros = np.zeros(len(rows), dtype=np.int64)
        return zeros, zeros.copy()
    additive = terms.additive[rows].sum(axis=1, dtype=np.int64)
    score = additive.copy()
    width = rows.shape[1]
    for first in range(width):
        for second in range(first + 1, width):
            score += terms.pairwise[rows[:, first], rows[:, second]]
    return score, additive


def _screen(layout: _Layout, terms: _Terms | None, chunk: int, touching: np.ndarray | None) -> _Table:
    found: list[tuple[np.ndarray, np.ndarray, np.ndarray]] = []
    for rows in _enumerate(layout, chunk, touching):
        found.append((rows, *_score(rows, terms)))
    width = len(layout.slot_of)
    if not found:
        empty = np.zeros(0, dtype=np.int64)
        return _Table(np.zeros((0, width), dtype=np.int32), empty, empty.copy())
    return _Table(*(np.concatenate([item[part] for item in found]) for part in range(3)))


def _join(first: _Table, second: _Table) -> _Table:
    return _Table(
        np.concatenate([first.rows, second.rows]),
        np.concatenate([first.score, second.score]),
        np.concatenate([first.additive, second.additive]),
    )


def _order(score: np.ndarray, rows: np.ndarray) -> np.ndarray:
    """The positions of the rows by score, highest first; ties by id (the members in id order), then by index (the
    members slot by slot). Indices follow the ids' order, so this is a total order of the rows themselves."""
    width = rows.shape[1]
    ordered = np.sort(rows, axis=1)
    keys = [rows[:, column] for column in range(width - 1, -1, -1)]
    keys += [ordered[:, column] for column in range(width - 1, -1, -1)]
    keys.append(-score)  # no overflow: every score is within the int64 range on both sides (`_matrices`)
    return np.lexsort(keys)


def _top(score: np.ndarray, rows: np.ndarray, count: int) -> np.ndarray:
    """The positions of the first `count` rows in screen order."""
    total = len(score)
    if total > count:
        kth = np.partition(score, total - count)[total - count]
        candidates = np.flatnonzero(score >= kth)
    else:
        candidates = np.arange(total)
    return candidates[_order(score[candidates], rows[candidates])][:count]


def _matrices(layout: _Layout, roster: _Roster, signature: Signature) -> _Terms:
    """The rounded terms as gather tables; `screen_overflow` when a row's sum could leave the int64 range."""
    additive, pairwise = signature
    width = len(layout.slot_of)
    largest = max((abs(value) for value in additive.values()), default=0)
    largest_pair = max((abs(value) for value in pairwise.values()), default=0)
    if width * largest + width * (width - 1) // 2 * largest_pair > INT64_MAX:
        raise _failed("screen_overflow", team=layout.team.team_id)
    count = len(roster.ids)
    single = np.zeros(count, dtype=np.int64)
    pairs = np.zeros((count, count), dtype=np.int64)
    for entity, value in additive.items():
        single[roster.index[entity]] = value
    for (first, second), value in pairwise.items():
        one, other = roster.index[first], roster.index[second]
        pairs[one, other] = pairs[other, one] = value
    return _Terms(single, pairs)


def screen_team(
    team: Team,
    entities: Sequence[Entity],
    additive: Mapping[str, int],
    pairwise: Mapping[tuple[str, str], int],
    *,
    chunk_rows: int,
    max_rows: int = SearchSettings.screen_max_rows,
) -> list[tuple[tuple[str, ...], int, int]]:
    """One team's legal rows in screen order, each (members slot by slot, screen score, additive score), from
    rounded terms: an absent entity or pair has 0, and a pair is (first, second) in id order. The search screens
    with this same code."""
    problem = Problem("screen", (team,), tuple(entities))
    _check(problem)
    roster = _roster(problem)
    layout = _layout(team, roster, max_rows)
    table = _screen(layout, _matrices(layout, roster, (dict(additive), dict(pairwise))), chunk_rows, None)
    return [
        (tuple(roster.ids[index] for index in table.rows[position].tolist()), int(table.score[position]),
         int(table.additive[position]))
        for position in _order(table.score, table.rows).tolist()
    ]


def _kept(warm: State, team: int, roster: _Roster, signature: Signature | None) -> tuple[_Table, np.ndarray]:
    """From the warm state: the team's rows whose entities all kept their tags and terms (in this roster's
    indices, with their scores), and the mask of the entities that did not, new and gone ones included."""
    old_tags = dict(zip(warm.ids, warm.tags))
    new_tags = dict(zip(roster.ids, roster.tags))
    changed = {entity for entity in old_tags.keys() | new_tags.keys() if old_tags.get(entity) != new_tags.get(entity)}
    old_single, old_pairs = warm.terms[team] or ({}, {})
    new_single, new_pairs = signature or ({}, {})
    changed.update(e for e in old_single.keys() | new_single.keys() if old_single.get(e, 0) != new_single.get(e, 0))
    for pair in old_pairs.keys() | new_pairs.keys():
        if old_pairs.get(pair, 0) != new_pairs.get(pair, 0):
            changed.update(pair)
    table = warm.tables[team]
    gone = np.array([entity in changed for entity in warm.ids], dtype=bool)
    keep = ~gone[table.rows].any(axis=1)
    remap = np.array([roster.index.get(entity, -1) for entity in warm.ids], dtype=np.int32)
    kept = _Table(remap[table.rows[keep]], table.score[keep], table.additive[keep])
    return kept, np.array([entity in changed for entity in roster.ids], dtype=bool)


def _additive_pick(tables: list[_Table], count: int) -> tuple[Row, ...] | None:
    """Each team in order takes its first row by additive score among the entities the earlier teams left; None
    when one finds none."""
    used = np.zeros(count, dtype=bool)
    picked = []
    for table in tables:
        free = np.flatnonzero(~used[table.rows].any(axis=1))
        if not len(free):
            return None
        [first] = _top(table.additive[free], table.rows[free], 1)
        row = table.rows[free[first]]
        picked.append(_row(row))
        used[row] = True
    return tuple(picked)


# The search -------------------------------------------------------------------------------------------------


class _Run:
    """One search: the roster, the layouts, the evaluated rows and the budget."""

    def __init__(self, problem: Problem, pool: Pool, settings: SearchSettings, session: Session | None) -> None:
        self.problem = problem
        self.pool = pool
        self.settings = settings
        self.session = session
        self.engine_id = pool.engine.engine_id
        self.roster = _roster(problem)
        self.layouts = [_layout(team, self.roster, settings.screen_max_rows) for team in problem.teams]
        self.values: list[dict[Row, Fraction]] = [{} for _ in problem.teams]
        self.evaluated = 0
        self.exhausted: dict[str, int] = {}  # phase to the nodes it stopped at

    # The engine

    def predict(self) -> list[Signature]:
        ids = self.roster.ids
        requests = []
        for layout in self.layouts:
            team = layout.team
            requests.append({
                "team": team.team_id,
                "context": team.context,
                "slots": [
                    {"slot": slot.slot_id, "count": slot.count, "eligible": [ids[index] for index in chosen.tolist()]}
                    for slot, chosen in zip(team.slots, layout.eligible)
                ],
                ENTITIES: {ids[index]: self.roster.data[index] for index in layout.members},
            })
        outcomes = client.call(self.pool, "predict", requests, session=self.session)
        problems = [outcome.error for outcome in outcomes if outcome.error is not None]
        if problems:
            raise Failure(*problems)
        signatures = [
            self.terms(index, [ids[member] for member in layout.members], outcome.result, problems)
            for index, (layout, outcome) in enumerate(zip(self.layouts, outcomes))
        ]
        if problems:
            raise Failure(*problems)
        return signatures

    def terms(
        self, request: int, members: list[str], result: dict[str, Any], problems: list[dict[str, Any]]
    ) -> Signature:
        """A predict result's terms, rounded at `search.screen_scale`; each problem of its shape is one
        `engine_protocol_error` (`result_invalid`) in `problems`."""
        known = frozenset(members)
        scale = self.settings.screen_scale

        def refuse(at: str, expected: list[str]) -> None:
            problems.append(error(ENGINE_PROTOCOL_ERROR, {
                "engine_id": self.engine_id, "op": "predict", "request": request, "reason": "result_invalid",
                "expected": expected,
            }, at))

        def rounded(value: Any, at: str) -> int | None:
            if type(value) not in (int, float):
                refuse(at, ["number"])
                return None
            return round(Fraction(value) * scale)

        single: dict[str, int] = {}
        pairs: dict[tuple[str, str], int] = {}
        for key in sorted(result):
            if key not in PREDICT_MEMBERS:
                refuse(protocol.pointer("", key), list(PREDICT_MEMBERS))
        given = result.get("additive")
        if type(given) is not dict:
            refuse("/additive", ["object"])
        else:
            for entity in members:
                if entity not in given:
                    refuse(protocol.pointer("/additive", entity), ["number"])
            for entity in sorted(given):
                at = protocol.pointer("/additive", entity)
                if entity not in known:
                    refuse(at, ["an id of the request's entities"])
                    continue
                value = rounded(given[entity], at)
                if value:
                    single[entity] = value
        given = result.get("pairwise", {})
        if type(given) is not dict:
            refuse("/pairwise", ["object"])
            return single, pairs
        for first in sorted(given):
            at = protocol.pointer("/pairwise", first)
            if first not in known or type(given[first]) is not dict:
                refuse(at, ["an id of the request's entities", "object"])
                continue
            for second in sorted(given[first]):
                where = protocol.pointer(at, second)
                if second not in known or second <= first:
                    refuse(where, ["a later id of the request's entities"])
                    continue
                value = rounded(given[first][second], where)
                if value:
                    pairs[(first, second)] = value
        return single, pairs

    def request(self, team: int, row: Row) -> dict[str, Any]:
        layout, ids = self.layouts[team], self.roster.ids
        return {
            "team": layout.team.team_id,
            "context": layout.team.context,
            "slots": [
                {"slot": slot.slot_id, "members": [ids[index] for index in row[start:stop]]}
                for slot, (start, stop) in zip(layout.team.slots, layout.bounds)
            ],
            ENTITIES: {ids[index]: self.roster.data[index] for index in sorted(row)},
        }

    def evaluate(self, wanted: list[tuple[int, Row]]) -> None:
        """Evaluate the (team, row) pairs not evaluated yet, in the order given, as far as the budget goes."""
        fresh: list[tuple[int, Row]] = []
        seen: set[tuple[int, Row]] = set()
        for item in wanted:
            if item[1] not in self.values[item[0]] and item not in seen:
                seen.add(item)
                fresh.append(item)
        room = self.settings.node_limit - self.evaluated
        if len(fresh) > room:
            fresh = fresh[:room]
            self.exhausted.setdefault(EVALUATE, self.settings.node_limit)
        if not fresh:
            return
        requests = [self.request(team, row) for team, row in fresh]
        outcomes = client.call(self.pool, "evaluate", requests, session=self.session)
        self.evaluated += len(fresh)
        problems = []
        metric = self.problem.metric
        for index, ((team, row), outcome) in enumerate(zip(fresh, outcomes)):
            if outcome.error is not None:
                problems.append(outcome.error)
            elif metric not in outcome.result["metrics"]:
                problems.append(error(ENGINE_PROTOCOL_ERROR, {
                    "engine_id": self.engine_id, "op": "evaluate", "request": index, "reason": "result_invalid",
                    "expected": [metric],
                }, protocol.pointer("/metrics", metric)))
            else:
                self.values[team][row] = Fraction(outcome.result["metrics"][metric])
        if problems:
            raise Failure(*problems)

    # Assignments

    def assign(self) -> tuple[Fraction, tuple[Row, ...]] | None:
        """The best assignment of the evaluated rows (ties by id, then index), or None when they hold none."""
        lists = [sorted(values.items(), key=lambda item: (-item[1], _key(item[0]))) for values in self.values]
        if any(not items for items in lists):
            return None
        if len(lists) == 1:
            row, value = lists[0][0]
            return value, (row,)
        tails = [Fraction(0)] * (len(lists) + 1)
        for depth in range(len(lists) - 1, -1, -1):
            tails[depth] = tails[depth + 1] + lists[depth][0][1]
        limit = self.settings.node_limit
        best: list[Any] = [None]  # (value, members in id order per team, rows)
        nodes = 0
        stopped = False

        def visit(depth: int, total: Fraction, used: frozenset[int], chosen: tuple[Row, ...]) -> None:
            nonlocal nodes, stopped
            if depth == len(lists):
                primary = tuple(tuple(sorted(row)) for row in chosen)
                current = best[0]
                if current is None or total > current[0] or (total == current[0] and (primary, chosen) < current[1:]):
                    best[0] = (total, primary, chosen)
                return
            for row, value in lists[depth]:
                if nodes >= limit:
                    stopped = True
                    return
                nodes += 1
                current = best[0]
                if current is not None:
                    bound = total + value + tails[depth + 1]
                    if bound < current[0]:  # the list runs best first: no later row reaches the best either
                        return
                    prefix = tuple(tuple(sorted(row)) for row in chosen) + (tuple(sorted(row)),)
                    if bound == current[0] and prefix > current[1][: depth + 1]:
                        continue  # at best a tie, which the best wins by id
                if used.isdisjoint(row):
                    visit(depth + 1, total + value, used | frozenset(row), chosen + (row,))
                    if stopped:
                        return

        visit(0, Fraction(0), frozenset(), ())
        if stopped:
            self.exhausted.setdefault(ASSIGN, limit)
        return None if best[0] is None else (best[0][0], best[0][2])

    def canon(self, team: int, row: Row, changes: dict[int, int]) -> Row:
        members = [changes.get(position, member) for position, member in enumerate(row)]
        bounds = self.layouts[team].bounds
        return tuple(itertools.chain.from_iterable(sorted(members[start:stop]) for start, stop in bounds))

    def legal(self, team: int, row: Row) -> bool:
        for _, holders, low, high in self.layouts[team].quotas:
            count = sum(1 for member in row if member in holders)
            if count < low or (high is not None and count > high):
                return False
        return True

    def neighbours(self, rows: tuple[Row, ...]) -> list[tuple[int, Row]]:
        """The legal rows the local swaps of an assignment give, in team and tie order: a member replaced by an
        entity no team uses, two members of two slots of a team exchanged, two members of two teams exchanged."""
        used = frozenset(itertools.chain.from_iterable(rows))
        found: set[tuple[int, Row]] = set()

        def propose(team: int, candidate: Row) -> None:
            if self.legal(team, candidate):
                found.add((team, candidate))

        for team, row in enumerate(rows):
            layout = self.layouts[team]
            for position, member in enumerate(row):
                slot = layout.slot_of[position]
                for entity in layout.eligible[slot].tolist():
                    if entity not in used:
                        propose(team, self.canon(team, row, {position: entity}))
                for other in range(position + 1, len(row)):
                    there = layout.slot_of[other]
                    swaps = member in layout.eligible_sets[there] and row[other] in layout.eligible_sets[slot]
                    if there != slot and swaps:
                        propose(team, self.canon(team, row, {position: row[other], other: member}))
            for later in range(team + 1, len(rows)):
                theirs, their_layout = rows[later], self.layouts[later]
                for position, member in enumerate(row):
                    for other, entity in enumerate(theirs):
                        if (entity in layout.eligible_sets[layout.slot_of[position]]
                                and member in their_layout.eligible_sets[their_layout.slot_of[other]]):
                            mine = self.canon(team, row, {position: entity})
                            yours = self.canon(later, theirs, {other: member})
                            if self.legal(team, mine) and self.legal(later, yours):
                                found.update(((team, mine), (later, yours)))
        return sorted(found, key=lambda item: (item[0], _key(item[1])))

    def improve(self, tables: list[_Table], seeded: list[tuple[int, Row]]) -> tuple[Fraction, tuple[Row, ...]] | None:
        """The heuristic: the top rows by screen score, then local swaps until they give no new row."""
        top_k = self.settings.top_k
        taken = [0] * len(tables)

        def widen() -> list[tuple[int, Row]]:
            more = []
            for team, table in enumerate(tables):
                positions = _top(table.score, table.rows, taken[team] + top_k)[taken[team]:]
                taken[team] += top_k
                more.extend((team, _row(table.rows[position])) for position in positions.tolist())
            return more

        self.evaluate(seeded + widen())
        while True:
            best = self.assign()
            if self.exhausted:
                return best
            if best is None:
                if all(taken[team] >= len(table.rows) for team, table in enumerate(tables)):
                    return None  # every legal row is evaluated, and no assignment fills all the teams
                self.evaluate(widen())
                continue
            fresh = [item for item in self.neighbours(best[1]) if item[1] not in self.values[item[0]]]
            if not fresh:
                return best
            self.evaluate(fresh)

    # Output

    def show(self, rows: tuple[Row, ...]) -> list[dict[str, Any]]:
        ids = self.roster.ids
        shown = []
        for team, row in enumerate(rows):
            layout = self.layouts[team]
            shown.append({
                "team": layout.team.team_id,
                "slots": [
                    {"slot": slot.slot_id, "members": [ids[index] for index in row[start:stop]]}
                    for slot, (start, stop) in zip(layout.team.slots, layout.bounds)
                ],
                "value": _text(self.values[team].get(row)),
            })
        return shown

    def approximation(self, pick: tuple[Row, ...] | None, best: Fraction, proven: bool) -> dict[str, Any]:
        if pick is None:
            return {"additive": None, "value": None, "loss": None, "relative": None, "exact": False}
        values = [self.values[team].get(row) for team, row in enumerate(pick)]
        value = None if None in values else sum(values, Fraction(0))
        loss = None if value is None else best - value
        relative = loss / best if loss is not None and best > 0 else None
        return {
            "additive": self.show(pick), "value": _text(value), "loss": _text(loss), "relative": _text(relative),
            "exact": proven and loss is not None,
        }


def search(
    problem: Problem, pool: Pool, settings: SearchSettings, *, session: Session | None = None,
    warm: State | None = None,
) -> Result:
    """Search the problem with the pool's engine (formats.md, "The search"). `session` caches the engine's
    results; `warm` is a previous search's state."""
    _check(problem)
    engine = pool.engine
    if "evaluate" not in engine.ops:
        raise _failed("op_missing", engine_id=engine.engine_id, op="evaluate")
    run = _Run(problem, pool, settings, session)
    predicts = "predict" in engine.ops
    signatures: list[Signature | None] = list(run.predict()) if predicts else [None] * len(problem.teams)
    shape = _shape(problem, settings)
    reuse = warm is not None and warm.shape == shape
    tables: list[_Table] = []
    screened = 0
    for team, layout in enumerate(run.layouts):
        signature = signatures[team]
        terms = None if signature is None else _matrices(layout, run.roster, signature)
        kept, touching = _kept(warm, team, run.roster, signature) if reuse else (None, None)
        table = _screen(layout, terms, settings.screen_chunk_rows, touching)
        screened += len(table.rows)
        tables.append(table if kept is None else _join(kept, table))
    for layout, table in zip(run.layouts, tables):
        if not len(table.rows):
            raise _failed("no_legal_assignment", team=layout.team.team_id)
    space = sum(len(table.rows) for table in tables)
    exact = space <= settings.exact_max
    if not exact and not predicts:
        raise _failed("op_missing", engine_id=engine.engine_id, op="predict", space=space, exact_max=settings.exact_max)
    pick = _additive_pick(tables, len(run.roster.ids)) if predicts else None
    seeded = list(enumerate(pick or ()))
    if exact:
        for team, table in enumerate(tables):
            order = _order(np.zeros(len(table.rows), dtype=np.int64), table.rows)
            seeded.extend((team, _row(table.rows[position])) for position in order.tolist())
        run.evaluate(seeded)
        best = run.assign()
    else:
        best = run.improve(tables, seeded)
    if best is None:
        if run.exhausted:
            phase = EVALUATE if EVALUATE in run.exhausted else ASSIGN
            raise _failed("budget_exhausted", node_limit=settings.node_limit, phase=phase)
        raise _failed("no_legal_assignment")
    proven = exact and not run.exhausted
    value, rows = best
    data = {
        "method": EXACT if exact else HEURISTIC,
        "proven": proven,
        "start": WARM if reuse else COLD,
        "value": str(value),
        "teams": run.show(rows),
        "space": space,
        "approximation": run.approximation(pick, value, proven) if predicts else None,
        "work": {"screened": screened, "evaluated": run.evaluated},
    }
    warnings = [
        error(SEARCH_BUDGET_EXHAUSTED, {"solve": "search", "phase": phase, "node_limit": nodes, "nodes": nodes})
        for phase, nodes in sorted(run.exhausted.items(), key=lambda item: (EVALUATE, ASSIGN).index(item[0]))
    ]
    state = State(shape, run.roster.ids, run.roster.tags, tuple(signatures), tuple(tables))
    return Result(data, warnings, state)
