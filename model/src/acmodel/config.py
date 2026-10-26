"""Configuration: `%LOCALAPPDATA%\\ActingCommand\\acmodel\\config.json` (`acmodel.config.v1`).

The file is optional and every key has a default. A file that exists is read
strictly: UTF-8 JSON, no duplicate keys, no unknown members, typed values.
The host caps (`compute.priority`, `compute.job.*`, `host.*`) are read by `acmodel.host.caps`, the engine
host's keys (`compute.workers.*`, `engine.*`, `engines.*`) by `acmodel.engine.settings`; the keys under
`unknowns` (sampling, export precision) and `sensitivity.top_n` are `Unknowns`; the economy's keys (`lp.*`,
`search.milp_node_limit`, `economy.*`) are `EconomySettings`.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from acmodel.codes import CONFIG_INVALID
from acmodel.engine import settings as engine_settings
from acmodel.envelope import Failure, error
from acmodel.host import caps as host_caps

SCHEMA_VERSION = "acmodel.config.v1"
LOCATION_VARIABLE = "LOCALAPPDATA"
APP_DIR = ("ActingCommand", "acmodel")
DEFAULT_BLAS_THREADS = 1
# The i128 range of the first (Rust) draft: numerators and denominators of at most 127 bits.
DEFAULT_EXPR_MAX_BITS = 127
DEFAULT_CACHE_MAX_BYTES = 2 * 1024**3

_MEMBERS = {
    "schema_version": None,
    "paths": {"venv", "state_root", "datasets"},
    "compute": {"blas_threads", "priority", "job", "workers"},
    "expr": {"max_bits"},
    "cache": {"max_bytes"},
    "host": {"memory", "cpu", "quiet_windows", "pause_file", "pause_poll_ms"},
    "engine": engine_settings.ENGINE_MEMBERS,
    "engines": engine_settings.ENGINES_MEMBERS,
    "unknowns": {"scale", "samples", "seed", "chunk_size", "resolution"},
    "sensitivity": {"top_n"},
    "lp": {"max_denominator", "grain"},
    "search": {"milp_node_limit"},
    "economy": {"plan_horizon"},
}


@dataclass(frozen=True)
class Unknowns:
    """The keys under `unknowns`, and `sensitivity.top_n` (#424 design 4.5, 4.8; `core.unknown`).

    - `top_n` (`sensitivity.top_n`): how many unknowns each output's sensitivity lists;
    - `scale`: the export precision an impact is measured at (1000: milli);
    - `samples`: the Monte Carlo samples of the distributions per output, 0 for none;
    - `seed`: the first word of every Philox key (the second is the unknown's stream id);
    - `chunk_size`: the samples drawn per Philox call; it never changes a result;
    - `resolution`: the cells of a uniform draw on (0, 1).
    """

    top_n: int = 5
    scale: int = 1000
    samples: int = 0
    seed: int = 0
    chunk_size: int = 1024
    resolution: int = 1_000_000

    def keyed(self) -> dict[str, int]:
        """The keys that can change an output, for the run id and the spec of the sampled nodes: all but
        `chunk_size`."""
        return {"top_n": self.top_n, "scale": self.scale, "samples": self.samples, "seed": self.seed,
                "resolution": self.resolution}


# The range of each key: (section, key) to (lowest, highest or None).
UNKNOWNS_RANGES = {
    ("sensitivity", "top_n"): (1, None),
    ("unknowns", "scale"): (1, None),
    ("unknowns", "samples"): (0, 2**63 - 1),
    ("unknowns", "seed"): (0, 2**64 - 1),
    ("unknowns", "chunk_size"): (1, None),
    ("unknowns", "resolution"): (1, 2**64),
}


@dataclass(frozen=True)
class EconomySettings:
    """The economy's keys (#424 design 4.3 and 4.4, design-py 4.3; `core.flow`).

    - `max_denominator` (`lp.max_denominator`): the largest denominator a solver's float becomes as an exact
      fraction before the exact check;
    - `grain` (`lp.grain`): the grain of every amount, one unit: the step of the price re-solves, and how far
      below a read cap a collection must start;
    - `node_limit` (`search.milp_node_limit`): the most nodes a MILP may explore; the only budget, never a time;
    - `plan_horizon` (`economy.plan_horizon`): the steps (days) of an account's plan.
    """

    max_denominator: int = 1_000_000
    grain: int = 1
    node_limit: int = 100_000
    plan_horizon: int = 7

    def keyed(self) -> dict[str, int]:
        """Every key, for the run id of `economy`."""
        return {"max_denominator": self.max_denominator, "grain": self.grain, "node_limit": self.node_limit,
                "plan_horizon": self.plan_horizon}


# The range of each economy key: (section, key) to (field, lowest, highest or None). HiGHS counts nodes in a
# 32-bit integer.
ECONOMY_RANGES = {
    ("lp", "max_denominator"): ("max_denominator", 1, None),
    ("lp", "grain"): ("grain", 1, None),
    ("search", "milp_node_limit"): ("node_limit", 1, 2**31 - 1),
    ("economy", "plan_horizon"): ("plan_horizon", 1, None),
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


def _absolute(section: dict[str, Any], key: str, default: Path, path: Path) -> Path:
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
    sections = {name: _section(doc, name, path) for name in ("unknowns", "sensitivity")}
    for (name, key), (low, high) in UNKNOWNS_RANGES.items():
        value = sections[name].get(key)
        if key in sections[name] and (type(value) is not int or value < low or (high is not None and value > high)):
            raise _invalid("value_invalid", path, _pointer(name, key))

    economy_sections = {name: _section(doc, name, path) for name in ("lp", "search", "economy")}
    economy: dict[str, int] = {}
    for (name, key), (field, low, high) in ECONOMY_RANGES.items():
        if key not in economy_sections[name]:
            continue
        value = economy_sections[name][key]
        if type(value) is not int or value < low or (high is not None and value > high):
            raise _invalid("value_invalid", path, _pointer(name, key))
        economy[field] = value

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
        unknowns=Unknowns(**sections["unknowns"], **sections["sensitivity"]),
        economy=EconomySettings(**economy),
    )
