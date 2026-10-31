"""Configuration: `%LOCALAPPDATA%\\ActingCommand\\acmodel\\config.json` (`acmodel.config.v1`).

The file is optional and every key has a default. A file that exists is read
strictly: UTF-8 JSON, no duplicate keys, no unknown members, typed values.
The host caps (`compute.priority`, `compute.job.*`, `host.*`) are read by `acmodel.host.caps`, the engine
host's keys (`compute.workers.*`, `engine.*`, `engines.*`) by `acmodel.engine.settings`, `watch.*` by
`acmodel.watch.settings`; the keys under
`unknowns` (sampling, export precision, Wilson bounds), `sensitivity.top_n` and the keys under `random` are
`Unknowns`; the economy's keys (`lp.*`, `search.milp_node_limit`, `economy.*`) are `EconomySettings`, the
generic search's (the other keys under `search`) are `SearchSettings`, and the validator of the exports is
`paths.actinglab` (no default: an export that needs it refuses without it) and `export.validator_timeout_ms`.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from fractions import Fraction
from pathlib import Path
from typing import Any, Mapping

from acmodel.codes import CONFIG_INVALID
from acmodel.engine import settings as engine_settings
from acmodel.envelope import Failure, error
from acmodel.host import caps as host_caps
from acmodel.watch import settings as watch_settings

SCHEMA_VERSION = "acmodel.config.v1"
LOCATION_VARIABLE = "LOCALAPPDATA"
APP_DIR = ("ActingCommand", "acmodel")
DEFAULT_BLAS_THREADS = 1
# The i128 range of the first (Rust) draft: numerators and denominators of at most 127 bits.
DEFAULT_EXPR_MAX_BITS = 127
DEFAULT_CACHE_MAX_BYTES = 2 * 1024**3
DEFAULT_VALIDATOR_TIMEOUT_MS = 120_000

_MEMBERS = {
    "schema_version": None,
    "paths": {"venv", "state_root", "datasets", "actinglab"},
    "compute": {"blas_threads", "priority", "job", "workers"},
    "expr": {"max_bits"},
    "cache": {"max_bytes"},
    "host": {"memory", "cpu", "quiet_windows", "pause_file", "pause_poll_ms"},
    "engine": engine_settings.ENGINE_MEMBERS,
    "engines": engine_settings.ENGINES_MEMBERS,
    "unknowns": {"scale", "samples", "seed", "chunk_size", "resolution", "wilson_digits", "drop_z"},
    "sensitivity": {"top_n"},
    "random": {"max_transitions", "enclosure_digits"},
    "lp": {"max_denominator", "grain"},
    "search": {
        "milp_node_limit", "top_k", "exact_max", "node_limit", "screen_scale", "screen_chunk_rows", "screen_max_rows",
    },
    "economy": {"plan_horizon", "max_cycle_steps"},
    "export": {"validator_timeout_ms"},
    "watch": watch_settings.MEMBERS,
}
# `unknowns.drop_z`: a positive decimal string, so that no float reaches the Wilson bounds.
DECIMAL_Z = re.compile(r"(0|[1-9][0-9]*)(\.[0-9]+)?")


@dataclass(frozen=True)
class Unknowns:
    """The keys under `unknowns`, `sensitivity.top_n` and the keys under `random` (#424 design 4.5, 4.8;
    `core.unknown`, `core.random`).

    - `top_n` (`sensitivity.top_n`): how many unknowns each output's sensitivity lists;
    - `scale`: the export precision an impact is measured at (1000: milli);
    - `samples`: the Monte Carlo samples of the distributions per output, 0 for none;
    - `seed`: the first word of every Philox key (the second is the unknown's stream id);
    - `chunk_size`: the samples drawn per Philox call; it never changes a result;
    - `resolution`: the cells of a uniform draw on (0, 1);
    - `wilson_digits`: the decimals the Wilson bounds of a rate from samples are rounded to, the low bound down
      and the high bound up;
    - `drop_z`: the z of the Wilson bounds, a decimal string (1.959964: 95 %);
    - `max_transitions` (`random.max_transitions`): the most transitions a pity chain may need;
    - `enclosure_digits` (`random.enclosure_digits`): the decimals of the enclosure that carries a random rule's
      value when its numerator or denominator exceeds `expr.max_bits`.
    """

    top_n: int = 5
    scale: int = 1000
    samples: int = 0
    seed: int = 0
    chunk_size: int = 1024
    resolution: int = 1_000_000
    wilson_digits: int = 6
    drop_z: str = "1.959964"
    max_transitions: int = 1_000_000
    enclosure_digits: int = 12

    def keyed(self) -> dict[str, int | str]:
        """The keys that can change an output, for the run id and the spec of the sampled nodes: all but
        `chunk_size`."""
        return {"top_n": self.top_n, "scale": self.scale, "samples": self.samples, "seed": self.seed,
                "resolution": self.resolution, "wilson_digits": self.wilson_digits, "drop_z": self.drop_z,
                "max_transitions": self.max_transitions, "enclosure_digits": self.enclosure_digits}


# The range of each key: (section, key) to (lowest, highest or None).
UNKNOWNS_RANGES = {
    ("sensitivity", "top_n"): (1, None),
    ("unknowns", "scale"): (1, None),
    ("unknowns", "samples"): (0, 2**63 - 1),
    ("unknowns", "seed"): (0, 2**64 - 1),
    ("unknowns", "chunk_size"): (1, None),
    ("unknowns", "resolution"): (1, 2**64),
    ("unknowns", "wilson_digits"): (1, 100),
    ("random", "max_transitions"): (1, None),
    ("random", "enclosure_digits"): (1, 30),
}


@dataclass(frozen=True)
class EconomySettings:
    """The economy's keys (#424 design 4.3 and 4.4, design-py 4.3; `core.flow`).

    - `max_denominator` (`lp.max_denominator`): the largest denominator a solver's float becomes as an exact
      fraction before the exact check;
    - `grain` (`lp.grain`): the grain of every amount, one unit: the step of the price re-solves, and how far
      below a read cap a collection must start;
    - `node_limit` (`search.milp_node_limit`): the most nodes a MILP may explore; the only budget, never a time;
    - `plan_horizon` (`economy.plan_horizon`): the steps (days) of an account's plan;
    - `max_cycle_steps` (`economy.max_cycle_steps`): the most steps (days) of the steady state's cycle, the least
      common multiple of the periods' lengths; a period that would make it longer is relaxed by rate (#424
      ACM-PRICE PRICE-3).
    """

    max_denominator: int = 1_000_000
    grain: int = 1
    node_limit: int = 100_000
    plan_horizon: int = 7
    max_cycle_steps: int = 28

    def keyed(self) -> dict[str, int]:
        """Every key, for the run id of `economy`."""
        return {"max_denominator": self.max_denominator, "grain": self.grain, "node_limit": self.node_limit,
                "plan_horizon": self.plan_horizon, "max_cycle_steps": self.max_cycle_steps}


# The range of each economy key: (section, key) to (field, lowest, highest or None). HiGHS counts nodes in a
# 32-bit integer.
ECONOMY_RANGES = {
    ("lp", "max_denominator"): ("max_denominator", 1, None),
    ("lp", "grain"): ("grain", 1, None),
    ("search", "milp_node_limit"): ("node_limit", 1, 2**31 - 1),
    ("economy", "plan_horizon"): ("plan_horizon", 1, None),
    ("economy", "max_cycle_steps"): ("max_cycle_steps", 1, None),
}


@dataclass(frozen=True)
class SearchSettings:
    """The generic search's keys (#424 design 4.6, design-py 4.6; `core.search`).

    - `top_k` (`search.top_k`): the rows of each team, best by screen score, that are evaluated first;
    - `exact_max` (`search.exact_max`): the proof mode evaluates every legal row when all teams have at most this
      many together; 0: never;
    - `node_limit` (`search.node_limit`): the most team rows one search evaluates, and the most nodes of each
      assignment of two or more disjoint teams; the only budget, never a time;
    - `screen_scale` (`search.screen_scale`): the scale at which the engine's predict terms become 64-bit integers;
    - `screen_chunk_rows` (`search.screen_chunk_rows`): the rows of a team's slot product screened per chunk; it
      never changes a result;
    - `screen_max_rows` (`search.screen_max_rows`): the most rows a team's slot product may have.
    """

    top_k: int = 32
    exact_max: int = 5000
    node_limit: int = 100_000
    screen_scale: int = 1_000_000
    screen_chunk_rows: int = 2**20
    screen_max_rows: int = 2**23


# The range of each search key: key to (lowest, highest).
SEARCH_RANGES = {
    "top_k": (1, 2**53 - 1),
    "exact_max": (0, 2**53 - 1),
    "node_limit": (1, 2**53 - 1),
    "screen_scale": (1, 2**53 - 1),
    "screen_chunk_rows": (1, 2**53 - 1),
    "screen_max_rows": (1, 2**53 - 1),
}


@dataclass(frozen=True)
class Config:
    path: Path
    venv: Path
    state_root: Path
    datasets: Path
    blas_threads: int
    expr_max_bits: int
    host: host_caps.HostCaps
    cache_max_bytes: int
    engine: engine_settings.EngineSettings
    unknowns: Unknowns
    economy: EconomySettings
    search: SearchSettings
    actinglab: Path | None  # `paths.actinglab`: the Runtime's validator of the exports; None when not configured
    validator_timeout_ms: int  # `export.validator_timeout_ms`
    watch: watch_settings.WatchSettings  # the keys under `watch` (acmodel.watch.settings)


class _DuplicateKey(ValueError):
    def __init__(self, key: str) -> None:
        super().__init__(key)
        self.key = key


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    doc: dict[str, Any] = {}
    for key, value in pairs:
        if key in doc:
            raise _DuplicateKey(key)
        doc[key] = value
    return doc


def _pointer(*parts: str) -> str:
    return "".join("/" + part.replace("~", "~0").replace("/", "~1") for part in parts)


def _invalid(reason: str, path: Path, ref: str | None = None, **values: Any) -> Failure:
    return Failure(error(CONFIG_INVALID, {"reason": reason, "path": str(path), **values}, ref))


def load(environ: Mapping[str, str]) -> Config:
    base = environ.get(LOCATION_VARIABLE, "")
    if not base or not Path(base).is_absolute():
        raise Failure(error(CONFIG_INVALID, {"reason": "location_invalid", "variable": LOCATION_VARIABLE}))
    root = Path(base, *APP_DIR)
    path = root / "config.json"
    try:
        raw = path.read_bytes()
    except FileNotFoundError:
        return _build({"schema_version": SCHEMA_VERSION}, path, root)
    except OSError as exc:
        raise _invalid("unreadable", path, exception_type=type(exc).__name__, raw_text=str(exc)) from exc
    try:
        doc = json.loads(raw.decode("utf-8"), object_pairs_hook=_no_duplicates)
    except _DuplicateKey as exc:
        raise _invalid("key_duplicate", path, key=exc.key) from exc
    except ValueError as exc:  # bad UTF-8, a BOM, or not JSON
        raise _invalid("malformed", path, exception_type=type(exc).__name__, raw_text=str(exc)) from exc
    return _build(doc, path, root)


def _section(doc: dict[str, Any], name: str, path: Path) -> dict[str, Any]:
    section = doc.get(name, {})
    if not isinstance(section, dict):
        raise _invalid("value_invalid", path, _pointer(name))
    for key in sorted(section):
        if key not in _MEMBERS[name]:
            raise _invalid("member_unknown", path, _pointer(name, key), key=key)
    return section


def _absolute(section: dict[str, Any], key: str, default: Any, path: Path) -> Any:
    """`paths.<key>`: an absolute path, or `default` (a path, or None for a key without one)."""
    if key not in section:
        return default
    value = section[key]
    if not isinstance(value, str) or not value or not Path(value).is_absolute():
        raise _invalid("value_invalid", path, _pointer("paths", key))
    return Path(value)


def _build(doc: Any, path: Path, root: Path) -> Config:
    if not isinstance(doc, dict):
        raise _invalid("value_invalid", path, "")
    for key in sorted(doc):
        if key not in _MEMBERS:
            raise _invalid("member_unknown", path, _pointer(key), key=key)
    if "schema_version" not in doc:
        raise _invalid("member_missing", path, _pointer("schema_version"), key="schema_version")
    if doc["schema_version"] != SCHEMA_VERSION:
        raise _invalid("value_invalid", path, _pointer("schema_version"))
    paths = _section(doc, "paths", path)
    compute = _section(doc, "compute", path)
    threads = compute.get("blas_threads", DEFAULT_BLAS_THREADS)
    if type(threads) is not int or threads < 1:
        raise _invalid("value_invalid", path, _pointer("compute", "blas_threads"))
    max_bits = _section(doc, "expr", path).get("max_bits", DEFAULT_EXPR_MAX_BITS)
    if type(max_bits) is not int or max_bits < 1:
        raise _invalid("value_invalid", path, _pointer("expr", "max_bits"))
    cache_max_bytes = _section(doc, "cache", path).get("max_bytes", DEFAULT_CACHE_MAX_BYTES)
    if type(cache_max_bytes) is not int or cache_max_bytes < 1:
        raise _invalid("value_invalid", path, _pointer("cache", "max_bytes"))
    state_root = _absolute(paths, "state_root", root / "state", path)
    sections = {name: _section(doc, name, path) for name in ("unknowns", "sensitivity", "random")}
    for (name, key), (low, high) in UNKNOWNS_RANGES.items():
        value = sections[name].get(key)
        if key in sections[name] and (type(value) is not int or value < low or (high is not None and value > high)):
            raise _invalid("value_invalid", path, _pointer(name, key))
    drop_z = sections["unknowns"].get("drop_z", Unknowns.drop_z)
    if type(drop_z) is not str or not DECIMAL_Z.fullmatch(drop_z) or not Fraction(drop_z) > 0:
        raise _invalid("value_invalid", path, _pointer("unknowns", "drop_z"))

    economy_sections = {name: _section(doc, name, path) for name in ("lp", "search", "economy")}
    economy: dict[str, int] = {}
    for (name, key), (field, low, high) in ECONOMY_RANGES.items():
        if key not in economy_sections[name]:
            continue
        value = economy_sections[name][key]
        if type(value) is not int or value < low or (high is not None and value > high):
            raise _invalid("value_invalid", path, _pointer(name, key))
        economy[field] = value
    search: dict[str, int] = {}
    for key, (low, high) in SEARCH_RANGES.items():
        if key not in economy_sections["search"]:
            continue
        value = economy_sections["search"][key]
        if type(value) is not int or not low <= value <= high:
            raise _invalid("value_invalid", path, _pointer("search", key))
        search[key] = value

    timeout_ms = _section(doc, "export", path).get("validator_timeout_ms", DEFAULT_VALIDATOR_TIMEOUT_MS)
    if type(timeout_ms) is not int or timeout_ms < 1:
        raise _invalid("value_invalid", path, _pointer("export", "validator_timeout_ms"))

    def invalid(reason: str, parts: tuple[str, ...], **values: Any) -> Failure:
        return _invalid(reason, path, _pointer(*parts), **values)

    host = host_caps.read(compute, _section(doc, "host", path), state_root, invalid)
    engine = engine_settings.read(
        compute, _section(doc, "engine", path), _section(doc, "engines", path), host.worker_memory_mib, invalid
    )
    return Config(
        path=path,
        venv=_absolute(paths, "venv", root / "venv", path),
        state_root=state_root,
        datasets=_absolute(paths, "datasets", root / "datasets", path),
        blas_threads=threads,
        expr_max_bits=max_bits,
        host=host,
        cache_max_bytes=cache_max_bytes,
        engine=engine,
        unknowns=Unknowns(**sections["unknowns"], **sections["sensitivity"], **sections["random"]),
        economy=EconomySettings(**economy),
        search=SearchSettings(**search),
        actinglab=_absolute(paths, "actinglab", None, path),
        validator_timeout_ms=timeout_ms,
        watch=watch_settings.read(_section(doc, "watch", path), invalid),
    )
