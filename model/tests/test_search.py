"""PY-13: the generic search (#424 design 4.6, design-py 4.6), on the neutral fake team engine of
tests/fakes/team_home.

- Enumeration equals brute force on fixtures (the approximation-loss report included).
- A warm start after a change or a new entity equals a cold search.
- Results do not depend on the worker count or the screening chunk size.
- The numpy screen equals a pure-Python reference on fixtures.
- Exhaustion gives search_budget_exhausted with proven: false.
- Ties break by id, then index.

The brute force and the reference here use plain iteration, and the fake engine's value is computed again here.
The workers are real processes; the host's side of the OS is a fake, and nothing sleeps.
"""

from __future__ import annotations

import dataclasses
import itertools
import json
import shutil
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from acmodel import config
from acmodel.config import SearchSettings
from acmodel.core import search
from acmodel.core.search import Entity, Problem, Quota, Slot, Team
from acmodel.engine import manifest
from acmodel.engine.pool import Pool

TESTS = Path(__file__).resolve().parent
TEAM_HOME = TESTS / "fakes" / "team_home"
CODES = json.loads((TESTS.parent / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
LIBRARIES = {"python": "3.13", "numpy": "2.5.4", "scipy": "1.18.1"}
HEURISTIC = SearchSettings(top_k=4, exact_max=0)

# Invented units: id to (power, kind, tags).
UNITS = {
    "u01": (7, "k1", ("front",)),
    "u02": (5, "k2", ("front",)),
    "u03": (9, "k1", ("front", "rare")),
    "u04": (4, "k3", ("back",)),
    "u05": (6, "k2", ("front",)),
    "u06": (8, "k3", ("back", "rare")),
    "u07": (3, "k1", ("back",)),
    "u08": (5, "k2", ("back",)),
}
# Two disjoint teams: t1 has a lead from the front and two others, at most one rare; t2 has two that are not
# rare, at least one from the back.
TEAMS = (
    Team(
        "t1", (Slot("lead", 1, requires=("front",)), Slot("rest", 2)), (Quota("rare", high=1),),
        {"weights": {"lead": 2, "rest": 1}, "pair": 3, "spread": 4},
    ),
    Team(
        "t2", (Slot("all", 2, excludes=("rare",)),), (Quota("back", low=1),),
        {"weights": {"all": 1}, "pair": 2, "spread": 1},
    ),
)


class FakeOs:
    """The host's side of each worker: every worker passes the Job check, and uses little memory."""

    def verify_child(self, pid: int) -> None:
        pass

    def private_mib(self, pid: int) -> int:
        return 64


@pytest.fixture
def make_pool(tmp_path, write_config, local_app_data):
    """A pool on a copy of the fake team engine, with `compute.workers.max` as given and no verify sample."""
    home = tmp_path / "engines" / "team"
    shutil.copytree(TEAM_HOME, home)
    pools: list[Pool] = []

    def make(*, workers: int = 1, scratch: str = "scratch") -> Pool:
        write_config({
            "schema_version": "acmodel.config.v1",
            "compute": {"workers": {"max": workers}},
            "engine": {"verify_sample_milli": 0},
            "engines": {"roots": {"team": str(home)}},
        })
        cfg = config.load({"LOCALAPPDATA": str(local_app_data)})
        engine = manifest.load("team", roots=cfg.engine.roots, datasets=cfg.datasets, libraries=LIBRARIES)
        pool = Pool(
            engine, cfg.engine, scratch_root=tmp_path / scratch, log_dir=tmp_path / "logs", os_layer=FakeOs(),
            blas_threads=cfg.blas_threads,
        )
        pools.append(pool)
        return pool

    yield make
    for pool in pools:
        pool.close()


def problem(units: dict[str, tuple[int, str, tuple[str, ...]]], teams: tuple[Team, ...] = TEAMS) -> Problem:
    entities = tuple(Entity(uid, tags, {"power": power, "kind": kind}) for uid, (power, kind, tags) in units.items())
    return Problem("value", teams, entities)


def registered(entry: dict[str, Any], category: str) -> dict[str, Any]:
    """The entry, after checking that its code is registered with this category and its values are that code's."""
    assert set(entry) <= {"code", "detail", "ref"}, entry
    spec = CODES[entry["code"]]
    assert spec["category"] == category, entry
    assert set(entry["detail"]) <= set(spec["values"]), entry
    for key, value in spec["values"].items():
        assert not value["required"] or key in entry["detail"], (entry, key)
        assert "members" not in value or key not in entry["detail"] or entry["detail"][key] in value["members"]
    return entry


# The brute force: plain iteration over everything.


def legal_rows(tags: dict[str, tuple[str, ...]], team: Team) -> list[tuple[tuple[str, ...], ...]]:
    """Every legal row of a team, as its slots' members, each slot in id order."""
    ids = sorted(tags)

    def eligible(uid: str, slot: Slot) -> bool:
        return set(slot.requires) <= set(tags[uid]) and not set(slot.excludes) & set(tags[uid])

    rows = []
    choices = [itertools.combinations([u for u in ids if eligible(u, slot)], slot.count) for slot in team.slots]
    for parts in itertools.product(*choices):
        members = [member for part in parts for member in part]
        if len(set(members)) < len(members):
            continue
        counts = [sum(quota.tag in tags[member] for member in members) for quota in team.quotas]
        if all(q.low <= count and (q.high is None or count <= q.high) for q, count in zip(team.quotas, counts)):
            rows.append(parts)
    return rows


def flat(parts: tuple[tuple[str, ...], ...]) -> tuple[str, ...]:
    return tuple(member for part in parts for member in part)


def tie_key(picks: tuple[tuple[tuple[str, ...], ...], ...]) -> tuple[Any, Any]:
    """Ties by id (members in id order, team by team), then index (members slot by slot)."""
    return tuple(tuple(sorted(flat(parts))) for parts in picks), tuple(flat(parts) for parts in picks)


def team_value(units: dict[str, Any], team: Team, parts: tuple[tuple[str, ...], ...]) -> int:
    """The fake team engine's value, computed again."""
    context = team.context
    total = sum(units[m][0] * context["weights"][slot.slot_id] for slot, part in zip(team.slots, parts) for m in part)
    same = sum(1 for a, b in itertools.combinations([units[m][1] for m in flat(parts)], 2) if a == b)
    return total + context["pair"] * same + (context["spread"] if same == 0 else 0)


def shown(teams: tuple[Team, ...], picks: Any, values: list[int]) -> list[dict[str, Any]]:
    return [
        {
            "team": team.team_id,
            "slots": [{"slot": slot.slot_id, "members": list(part)} for slot, part in zip(team.slots, parts)],
            "value": str(value),
        }
        for team, parts, value in zip(teams, picks, values)
    ]


def test_enumeration_equals_brute_force(make_pool):
    found = search.search(problem(UNITS), make_pool(), SearchSettings())

    tags = {uid: unit[2] for uid, unit in UNITS.items()}
    rows = [legal_rows(tags, team) for team in TEAMS]
    best = None
    for picks in itertools.product(*rows):
        members = [member for parts in picks for member in flat(parts)]
        if len(set(members)) < len(members):
            continue
        values = [team_value(UNITS, team, parts) for team, parts in zip(TEAMS, picks)]
        candidate = (-sum(values), tie_key(picks), picks, values)
        if best is None or candidate[:2] < best[:2]:
            best = candidate
    # The additive pick: each team in order takes the legal row with the most power among the units left.
    used: set[str] = set()
    pick = []
    for team_rows in rows:
        free = [parts for parts in team_rows if not used & set(flat(parts))]
        chosen = min(free, key=lambda parts: (-sum(UNITS[m][0] for m in flat(parts)), tie_key((parts,))))
        pick.append(chosen)
        used |= set(flat(chosen))
    pick_values = [team_value(UNITS, team, parts) for team, parts in zip(TEAMS, pick)]
    optimum, loss = -best[0], -best[0] - sum(pick_values)
    space = sum(len(team_rows) for team_rows in rows)

    assert found.warnings == []
    assert found.data == {
        "method": "exact",
        "proven": True,
        "start": "cold",
        "value": str(optimum),
        "teams": shown(TEAMS, best[2], best[3]),
        "space": space,
        "approximation": {
            "additive": shown(TEAMS, pick, pick_values),
            "value": str(sum(pick_values)),
            "loss": str(loss),
            "relative": str(Fraction(loss, optimum)),
            "exact": True,
        },
        "work": {"screened": space, "evaluated": space},
    }


def screened(state: search.State) -> list[set[tuple[tuple[str, ...], int, int]]]:
    """Each team's screened rows with their scores, by entity id."""
    return [
        {
            (tuple(state.ids[index] for index in row), score, additive)
            for row, score, additive in zip(table.rows.tolist(), table.score.tolist(), table.additive.tolist())
        }
        for table in state.tables
    ]


def comparable(result: search.Result) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """The result without `start` and the screened count, which tell a warm start from a cold one."""
    data = {key: value for key, value in result.data.items() if key != "start"}
    data["work"] = {key: value for key, value in data["work"].items() if key != "screened"}
    return data, result.warnings


def test_a_warm_start_after_a_change_or_a_new_entity_equals_a_cold_search(make_pool):
    pool = make_pool()
    first = search.search(problem(UNITS), pool, HEURISTIC)
    assert first.data["start"] == "cold"

    # A change: one unit's power, which moves its predict term and its value.
    changed = {**UNITS, "u03": (2, "k1", ("front", "rare"))}
    warm = search.search(problem(changed), pool, HEURISTIC, warm=first.state)
    cold = search.search(problem(changed), pool, HEURISTIC)
    assert (warm.data["start"], cold.data["start"]) == ("warm", "cold")
    assert comparable(warm) == comparable(cold)
    assert screened(warm.state) == screened(cold.state)
    assert 0 < warm.data["work"]["screened"] < cold.data["work"]["screened"] == cold.data["space"]

    # A new entity, warm from the warm state.
    added = {**changed, "u09": (6, "k3", ("front",))}
    warm = search.search(problem(added), pool, HEURISTIC, warm=warm.state)
    cold = search.search(problem(added), pool, HEURISTIC)
    assert (warm.data["start"], cold.data["start"]) == ("warm", "cold")
    assert comparable(warm) == comparable(cold)
    assert screened(warm.state) == screened(cold.state)
    assert 0 < warm.data["work"]["screened"] < cold.data["work"]["screened"] == cold.data["space"]
    assert cold.data["method"] == "heuristic" and cold.data["proven"] is False


def test_results_do_not_depend_on_the_worker_count_or_the_screening_chunk_size(make_pool):
    reference = search.search(problem(UNITS), make_pool(workers=1, scratch="first"), HEURISTIC)
    assert (reference.data["method"], reference.data["proven"]) == ("heuristic", False)

    for workers, chunk in ((2, HEURISTIC.screen_chunk_rows), (1, 1), (2, 7)):
        pool = make_pool(workers=workers, scratch=f"w{workers}-c{chunk}")
        found = search.search(problem(UNITS), pool, dataclasses.replace(HEURISTIC, screen_chunk_rows=chunk))
        assert (found.data, found.warnings) == (reference.data, reference.warnings)
        assert pool.counters["peak_workers"] == workers  # two workers did run in parallel where allowed


def reference_screen(
    team: Team, tags: dict[str, tuple[str, ...]], additive: dict[str, int], pairwise: dict[tuple[str, str], int]
) -> list[tuple[tuple[str, ...], int, int]]:
    """The screen by plain iteration: every legal row, its score and additive score, highest first, ties by the
    members in id order, then slot by slot."""
    found = []
    for parts in legal_rows(tags, team):
        members = flat(parts)
        single = sum(additive.get(member, 0) for member in members)
        pairs = sum(pairwise.get(tuple(sorted(pair)), 0) for pair in itertools.combinations(members, 2))
        found.append((members, single + pairs, single))
    found.sort(key=lambda item: (-item[1], tuple(sorted(item[0])), item[0]))
    return found


def test_the_numpy_screen_equals_a_pure_python_reference():
    team = Team("t", (Slot("a", 2, requires=("x",)), Slot("b", 2, excludes=("z",))), (Quota("y", low=1, high=2),))
    # Ids compare by code point: "a10" comes before "a9".
    tags = {
        "a10": ("x", "y"), "a9": ("x",), "b": ("x", "z"), "c1": ("y",), "c2": ("x", "y"), "d": (), "e": ("x",),
        "f": ("y", "z"),
    }
    additive = {"a10": 5, "a9": 5, "b": -3, "c1": 0, "c2": 5, "d": 2, "e": -1}
    pairwise = {("a10", "a9"): 4, ("a9", "d"): -2, ("b", "e"): 7, ("c1", "d"): 1, ("a10", "c2"): -4}
    entities = [Entity(uid, entity_tags) for uid, entity_tags in tags.items()]
    expected = reference_screen(team, tags, additive, pairwise)
    # The fixture has ties: one set of members in two arrangements, and equal scores of other sets.
    assert len({frozenset(members) for members, _, _ in expected}) < len(expected)

    for chunk in (1, 4, 1 << 20):
        assert search.screen_team(team, entities, additive, pairwise, chunk_rows=chunk) == expected


def test_exhaustion_gives_search_budget_exhausted_with_proven_false(make_pool):
    one = TEAMS[:1]
    found = search.search(problem(UNITS, one), make_pool(), SearchSettings(node_limit=10))

    [warning] = found.warnings
    assert registered(warning, "warning") == {
        "code": "search_budget_exhausted",
        "detail": {"solve": "search", "phase": "evaluate", "node_limit": 10, "nodes": 10},
    }
    rows = legal_rows({uid: unit[2] for uid, unit in UNITS.items()}, one[0])
    assert (found.data["method"], found.data["proven"], found.data["space"]) == ("exact", False, len(rows))
    assert found.data["work"]["evaluated"] == 10
    # The proof mode evaluates the additive pick first, then the rows in tie order; the budget stops it after 10,
    # and the result is the best of those.
    pick = min(rows, key=lambda parts: (-sum(UNITS[m][0] for m in flat(parts)), tie_key((parts,))))
    ordered = [pick] + sorted((parts for parts in rows if parts != pick), key=lambda parts: tie_key((parts,)))
    best = min(ordered[:10], key=lambda parts: (-team_value(UNITS, one[0], parts), tie_key((parts,))))
    assert found.data["teams"] == shown(one, [best], [team_value(UNITS, one[0], best)])
    assert found.data["approximation"]["exact"] is False


def test_ties_break_by_id_then_index(make_pool):
    # Every arrangement of two of the three units is worth 8: the result is the first by id ({a10, a9}), then by
    # index (a10 in the first slot).
    team = Team("t", (Slot("a", 1), Slot("b", 1)), (), {"weights": {"a": 1, "b": 1}, "pair": 0, "spread": 0})
    units = {"b": (4, "k1", ()), "a9": (4, "k2", ()), "a10": (4, "k3", ())}
    pool = make_pool()

    for settings, method in ((SearchSettings(), "exact"), (SearchSettings(top_k=1, exact_max=0), "heuristic")):
        found = search.search(problem(units, (team,)), pool, settings)
        assert (found.data["method"], found.data["space"]) == (method, 6)
        assert found.data["teams"] == [{
            "team": "t",
            "slots": [{"slot": "a", "members": ["a10"]}, {"slot": "b", "members": ["a9"]}],
            "value": "8",
        }]
