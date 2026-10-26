"""The engine host's configuration keys (contracts/cli.md, "Configuration" and "Engine host").

- `compute.workers.max`: the most worker processes alive at once, the verify worker included.
- `engine.*`: the hash seeds, the verify sample, reset, recycling, the timeouts and the restart budget.
- `engines.roots`: where each engine's home is on this host (engine id to absolute path). It is local
  configuration, never repository content (#424 design D-4).

Every key has a default; config.py reads the sections strictly and hands them here.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from acmodel import ids

DEFAULT_WORKERS_MAX = 2
DEFAULT_HASH_SEED = 0
DEFAULT_VERIFY_HASH_SEED = 1
DEFAULT_VERIFY_SAMPLE_MILLI = 10
DEFAULT_RESET_EACH_BATCH = True
DEFAULT_WORKER_SOFT_MEMORY_MIB = 896
DEFAULT_WORKER_MAX_REQUESTS = 0  # never recycled by count
DEFAULT_REQUEST_TIMEOUT_MS = 600_000
DEFAULT_START_TIMEOUT_MS = 120_000
DEFAULT_RESTART_BUDGET = 3
MAX_HASH_SEED = 4_294_967_295  # the range of PYTHONHASHSEED
MAX_COUNT = 2**53 - 1

ENGINE_MEMBERS = {
    "hash_seed", "verify_hash_seed", "verify_sample_milli", "reset_each_batch", "worker_soft_memory_mib",
    "worker_max_requests", "request_timeout_ms", "start_timeout_ms", "restart_budget",
}
ENGINES_MEMBERS = {"roots"}
WORKERS_MEMBERS = {"max"}

_ID = re.compile(ids.ID)

# invalid(reason, pointer parts, **values) -> the config_invalid Failure to raise (config.py).
Invalid = Callable[..., Exception]


@dataclass(frozen=True)
class EngineSettings:
    workers_max: int
    hash_seed: int
    verify_hash_seed: int
    verify_sample_milli: int
    reset_each_batch: bool
    worker_soft_memory_mib: int
    worker_max_requests: int
    request_timeout_ms: int
    start_timeout_ms: int
    restart_budget: int
    roots: dict[str, Path]


def _integer(section: dict[str, Any], parts: tuple[str, ...], default: int, low: int, high: int, invalid: Invalid) -> int:
    value = section.get(parts[-1], default)
    if type(value) is not int or not low <= value <= high:
        raise invalid("value_invalid", parts)
    return value


def read(
    compute: dict[str, Any], engine: dict[str, Any], engines: dict[str, Any], worker_memory_mib: int, invalid: Invalid
) -> EngineSettings:
    """The settings from the `compute`, `engine` and `engines` sections. config.py has checked the members of
    `engine` and `engines`; `worker_memory_mib` is the Job's per-process limit."""
    workers = compute.get("workers", {})
    if not isinstance(workers, dict):
        raise invalid("value_invalid", ("compute", "workers"))
    for key in sorted(workers):
        if key not in WORKERS_MEMBERS:
            raise invalid("member_unknown", ("compute", "workers", key), key=key)

    def number(key: str, default: int, low: int, high: int = MAX_COUNT) -> int:
        return _integer(engine, ("engine", key), default, low, high, invalid)

    reset = engine.get("reset_each_batch", DEFAULT_RESET_EACH_BATCH)
    if type(reset) is not bool:
        raise invalid("value_invalid", ("engine", "reset_each_batch"))
    settings = EngineSettings(
        workers_max=_integer(workers, ("compute", "workers", "max"), DEFAULT_WORKERS_MAX, 1, MAX_COUNT, invalid),
        hash_seed=number("hash_seed", DEFAULT_HASH_SEED, 0, MAX_HASH_SEED),
        verify_hash_seed=number("verify_hash_seed", DEFAULT_VERIFY_HASH_SEED, 0, MAX_HASH_SEED),
        verify_sample_milli=number("verify_sample_milli", DEFAULT_VERIFY_SAMPLE_MILLI, 0, 1000),
        reset_each_batch=reset,
        worker_soft_memory_mib=number("worker_soft_memory_mib", DEFAULT_WORKER_SOFT_MEMORY_MIB, 1),
        worker_max_requests=number("worker_max_requests", DEFAULT_WORKER_MAX_REQUESTS, 0),
        request_timeout_ms=number("request_timeout_ms", DEFAULT_REQUEST_TIMEOUT_MS, 1),
        start_timeout_ms=number("start_timeout_ms", DEFAULT_START_TIMEOUT_MS, 1),
        restart_budget=number("restart_budget", DEFAULT_RESTART_BUDGET, 0),
        roots=_roots(engines.get("roots", {}), invalid),
    )
    # The verify run must hash differently, or it cannot see hash-order dependence.
    if settings.verify_hash_seed == settings.hash_seed:
        raise invalid("value_invalid", ("engine", "verify_hash_seed"))
    # A soft limit at or above the Job's hard one would never recycle before the Job stops the worker.
    if settings.worker_soft_memory_mib >= worker_memory_mib:
        raise invalid("value_invalid", ("engine", "worker_soft_memory_mib"))
    return settings


def _roots(value: Any, invalid: Invalid) -> dict[str, Path]:
    if not isinstance(value, dict):
        raise invalid("value_invalid", ("engines", "roots"))
    roots = {}
    for engine_id in sorted(value):
        path = value[engine_id]
        if not _ID.fullmatch(engine_id):
            raise invalid("value_invalid", ("engines", "roots", engine_id))
        if not isinstance(path, str) or not path or not Path(path).is_absolute():
            raise invalid("value_invalid", ("engines", "roots", engine_id))
        roots[engine_id] = Path(path)
    return roots
