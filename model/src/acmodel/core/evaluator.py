"""The evaluator digest that `export prices` records (#424 PRICE-REUSE; contracts/cli.md, "Cache", "The evaluator
digest"; contracts/formats.md, "`acmodel export prices`").

`versions` (`core.versions`) names the model's inputs; it does not name what computed from them: the tool's own code,
whose `__version__` nobody bumps on a change, the libraries and the configuration. The digest names those, as the cache
key of the steady state's node does (`cache.keys`, `core.graph.Graph.cached`), so that `export prices --reuse-assessment`
takes an earlier impact assessment over only when the same code, libraries and configuration would compute it again:

- `source`: the sha256 of the canonical JSON (`core.versions.sha256`) of [path, sha256 of the file's bytes] of every `.py`
  file of the acmodel package (`PACKAGE`, the folder of `acmodel/__init__.py`), each path relative to it with `/`, sorted
  by path; nothing under a `__pycache__` folder, and no other file (`.pyc` neither), is read;
- `libraries`: the versions of the libraries the steady state's node kind declares (`cache.keys.EVALUATORS["economy"]`),
  as the cache's tokens take them (`cache.keys.libraries`: the Python minor version, the pinned packages);
- `config`: the configuration keys of the steady state's spec in the cache: `expr.max_bits`, which every node's spec holds,
  and the keys `core.flow.spec_keys` gives the steady state (`lp.max_denominator`, `lp.grain`,
  `economy.max_cycle_steps`, `lp.max_pivots`), by their names there;
- `sha256`: the sha256 of the canonical JSON of the other three.

The three lists are taken from where the cache key is made, never repeated here, so a library or a key the steady state's
node comes to declare is in the digest too; a test holds the digest's configuration to the spec `Graph.cached` builds.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Mapping

import acmodel
from acmodel.cache.keys import EVALUATORS, digest
from acmodel.config import EconomySettings
from acmodel.core import flow
from acmodel.core.versions import sha256

# The node whose cache key the digest follows: the steady state, a node of kind `economy` (`core.flow.NODES`).
KIND, SOLVE = flow.NODES[flow.STEADY]
PACKAGE = Path(acmodel.__file__).parent
SOURCE_SUFFIX = ".py"
SKIPPED_FOLDER = "__pycache__"


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


def config(max_bits: int, economy: EconomySettings) -> dict[str, int]:
    """The configuration keys of the steady state's spec in the cache (`Graph.cached`): `max_bits` and
    `core.flow.spec_keys` of the steady state."""
    return {"max_bits": max_bits, **flow.spec_keys(economy, SOLVE)}


def record(found: Mapping[str, str], max_bits: int, economy: EconomySettings, package: Path = PACKAGE) -> dict[str, Any]:
    """The evaluator digest as prices.json records it (module docstring): `sha256`, `source`, `libraries` and `config`.
    `found` holds the versions the tool runs with (`cache.keys.libraries`)."""
    body = {"source": source(package), "libraries": libraries(found), "config": config(max_bits, economy)}
    return {"sha256": sha256(body), **body}
