"""The evaluator digest that `export prices` records (#424 PRICE-REUSE; contracts/cli.md, "Cache", "The evaluator
digest"; contracts/formats.md, "`acmodel export prices`").

`versions` (`core.versions`) names the model's inputs; it does not name what computed from them: the tool's own code,
whose `__version__` nobody bumps on a change, the libraries and the configuration. The digest names those, so that
`export prices --reuse-assessment` takes an earlier impact assessment over only when the same code, libraries and
configuration would compute it again:

- `source`: the sha256 of the canonical JSON (`core.versions.sha256`) of [path, sha256 of the file's bytes] of every `.py`
  file of the acmodel package (`PACKAGE`, the folder of `acmodel/__init__.py`), each path relative to it with `/`, sorted
  by path; nothing under a `__pycache__` folder, and no other file (`.pyc` neither), is read;
- `libraries`: the versions of the libraries the steady state's node kind declares (`cache.keys.EVALUATORS["economy"]`),
  as the cache's tokens take them (`cache.keys.libraries`: the Python minor version, the pinned packages);
- `config`: every configuration key (`acmodel.config.keys`, from the table the configuration is read by) but those
  `NEUTRAL` lists, which never change a result, and those `PLAN_ONLY` lists, which only the plan reads; each by its
  dotted name, with its value as read or by default (`acmodel.config.FIELDS`), sorted by name. An integer outside the
  range canonical JSON holds is its decimal text;
- `sha256`: the sha256 of the canonical JSON of the other three.

A key is in the digest unless a list names it with why it may be left out, so a key the configuration comes to have is
in it from the start. `keys` refuses (`ValueError`, so every export fails) a list that names what is no key, a key both
lists name, and a key of the digest that `FIELDS` cannot read. The libraries are taken from where the cache key is made,
never repeated here; a test holds the digest's configuration to hold every key of the spec `Graph.cached` builds for the
steady state's node.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Mapping

import acmodel
from acmodel import config as configuration
from acmodel.cache.keys import EVALUATORS, digest
from acmodel.core import flow
from acmodel.core.versions import sha256
from acmodel.model.jsonfile import MAX_SAFE_INTEGER

# The node whose cache key the libraries follow: the steady state, a node of kind `economy` (`core.flow.NODES`).
KIND, SOLVE = flow.NODES[flow.STEADY]
PACKAGE = Path(acmodel.__file__).parent
SOURCE_SUFFIX = ".py"
SKIPPED_FOLDER = "__pycache__"

# The configuration keys that never change what a computation that finishes computes, each with why (contracts/cli.md,
# "Cache", "The evaluator digest", lists them too): where files are; the cache; the host caps and the gate (whether,
# when and how fast the work runs); the time and the memory a process may take and the restarts a pool may make (past
# them a computation fails, it never computes otherwise); the verify sample; the two chunk sizes. A limit that a
# computation meets the same way each time (`expr.max_bits`, `random.max_transitions`, `search.screen_max_rows`) is
# never here: past it the computation is refused, so a reuse must be refused where the assessment would be.
NEUTRAL = {
    "cache.max_bytes": "the size cap of the cache: an evicted value is computed again, the same; export prices computes "
                       "without the cache",
    "cache.wait_ms": "how long a connection waits for the cache's lock: past it the command fails (cache_unavailable)",
    "compute.job.cpu_rate_milli": "the CPU cap of the Job: how fast the work runs, never what it computes",
    "compute.job.worker_memory_mib": "the memory limit of each process in the Job: past it the process is stopped, a "
                                     "failure",
    "compute.priority": "the priority class: how soon the work is scheduled, never what it computes",
    "engine.request_timeout_ms": "how long a request may take: past it the batch fails (engine_failed, timeout)",
    "engine.restart_budget": "how many workers that ended abnormally a pool replaces: past it the call fails "
                             "(restart_budget_exhausted)",
    "engine.start_timeout_ms": "how long a worker may take to start or to stop: past it the call fails or the worker is "
                               "killed",
    "engine.verify_hash_seed": "the hash seed of the verify worker: the verify sample runs calls again only to refuse an "
                               "engine whose outcome is not a function of its request, and never changes a result kept",
    "engine.verify_sample_milli": "the share of an engine call's requests the verify sample runs again: it refuses a "
                                  "nondeterministic engine, and never changes a result kept",
    "engines.roots": "where each engine's home is on this host: what an engine computes is named by its fingerprint, "
                     "which every engine call's cache key holds, not by where it is; a local path",
    "export.validator_timeout_ms": "how long the validator of export tables may take: past it that export fails; export "
                                   "prices runs no validator",
    "host.cpu.pause_load_milli": "the gate: the CPU load that holds it, so whether and when a batch runs",
    "host.cpu.window_ms": "the gate: the span the CPU load is averaged over",
    "host.memory.pause_below_mib": "the gate: the available memory a batch pauses below",
    "host.memory.report_plan_above_mib": "the gate: which runs are reported before they start",
    "host.memory.start_min_mib": "the gate: the available memory a batch starts at",
    "host.pause_file": "the gate: the file whose presence holds it; a local path",
    "host.pause_poll_ms": "the gate: how often a waiting gate checks again",
    "host.quiet_windows": "the gate: the hours without compute",
    "paths.actinglab": "the validator of export tables: it accepts or refuses a table and never changes one; export "
                       "prices does not run it; a local path",
    "paths.datasets": "where the copies of the external datasets are: each is the file named by the sha256 its data file "
                      "declares (in the data version), and only that hash is checked; a local path",
    "paths.state_root": "where the cache, the runs' outputs and the shared trees are kept: export prices computes without "
                        "the cache, and a shared tree is compared by its digest; a local path",
    "paths.venv": "where the venv is: the libraries it holds are `libraries`, the code the tool runs is `source`; a local "
                  "path",
    "search.screen_chunk_rows": "the rows of a slot product the search's screen takes per chunk: it never changes a "
                                "result",
    "unknowns.chunk_size": "the samples drawn per Philox call: each sample is drawn by its own counter, so it never "
                           "changes a result",
    "watch.debounce_ms": "how long watch gathers changes into one refresh: when a refresh runs, never what it computes",
    "watch.poll_interval_ms": "how often watch scans its files: when a refresh runs, never what it computes",
}
# The configuration keys only the plan reads, each with where: neither the steady state, nor its vector, nor the impact
# assessment reads them.
PLAN_ONLY = {
    "economy.plan_horizon": "only the plan reads it: the steps of an account's plan (`core.flow._Reader.model`; the "
                            "steady state's steps are its cycle), and the plan's spec (`core.flow.spec_keys`)",
    "search.milp_node_limit": "only the plan reads it: the node budget of its MILP (`core.flow._Solver.milp`, which only "
                              "the plan's solves call; the steady state and its vector solve LPs), and the plan's spec "
                              "(`core.flow.spec_keys`)",
}


def source_files(package: Path = PACKAGE) -> list[str]:
    """Every `.py` file of the package, by its path relative to `package` with `/`, sorted; none in a `__pycache__`."""
    found = []
    for folder, folders, files in os.walk(package):
        folders[:] = [name for name in folders if name != SKIPPED_FOLDER]
        for name in files:
            if name.endswith(SOURCE_SUFFIX):
                found.append((Path(folder) / name).relative_to(package).as_posix())
    return sorted(found)


def source(package: Path = PACKAGE) -> str:
    """The digest of the package's source (module docstring, `source`)."""
    return sha256([[name, digest((package / name).read_bytes())] for name in source_files(package)])


def libraries(found: Mapping[str, str]) -> dict[str, str]:
    """The versions of the libraries the steady state's node kind declares, from those the tool runs with
    (`cache.keys.libraries`)."""
    return {name: found[name] for name in sorted(EVALUATORS[KIND].libraries)}


def keys() -> list[str]:
    """The configuration keys of the digest, sorted: every key (`acmodel.config.keys`) but those `NEUTRAL` and
    `PLAN_ONLY` list. A list that names what is no key, a key both name, and a key of the digest that
    `acmodel.config.FIELDS` cannot read are a `ValueError`, which names them."""
    every = configuration.keys()
    listed = set(NEUTRAL) | set(PLAN_ONLY)
    stray = sorted((listed - set(every)) | (set(NEUTRAL) & set(PLAN_ONLY)))
    if stray:
        raise ValueError(f"the evaluator digest's lists name what is no configuration key, or both name it: {stray}")
    taken = [key for key in every if key not in listed]
    unread = [key for key in taken if key not in configuration.FIELDS]
    if unread:
        raise ValueError(f"configuration keys neither in the evaluator digest nor listed as left out: {unread}")
    return taken


def _recorded(key: str, value: Any) -> int | str | bool:
    """A configuration value as the digest records it: a boolean, a string, or an integer, as its decimal text when it
    lies outside the range canonical JSON holds (`export.jcs`). Anything else is a `ValueError`."""
    if type(value) is int:
        return value if abs(value) <= MAX_SAFE_INTEGER else str(value)
    if type(value) in (bool, str):
        return value
    raise ValueError(f"the evaluator digest holds integers, strings and booleans; {key} is {type(value).__name__}")


def config(settings: configuration.Config) -> dict[str, int | str | bool]:
    """The digest's configuration (module docstring, `config`): each key of `keys`, by its dotted name, with its value in
    `settings`."""
    return {key: _recorded(key, configuration.value(settings, key)) for key in keys()}


def record(found: Mapping[str, str], settings: configuration.Config, package: Path = PACKAGE) -> dict[str, Any]:
    """The evaluator digest as prices.json records it (module docstring): `sha256`, `source`, `libraries` and `config`.
    `found` holds the versions the tool runs with (`cache.keys.libraries`), `settings` the configuration."""
    body = {"source": source(package), "libraries": libraries(found), "config": config(settings)}
    return {"sha256": sha256(body), **body}
