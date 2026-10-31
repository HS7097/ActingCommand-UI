"""The hashes of the cache (#424 design section 5.1): canonical JSON, evaluator versions, node keys and run ids.

- **Canonical JSON** (for hashing only): keys sorted, no whitespace, UTF-8, no floats or NaN. It never
  depends on Python's string-hash randomisation (design-py D-P1, D-P2): nothing here calls `hash()`.
- **A node's spec hash** = sha256 of `acmodel.node.v1`, the node kind, that kind's evaluator version
  (with the versions of the libraries the kind declares, D-P5) and the canonical node spec (the node and
  the configuration keys in its spec).
- **A node key** = sha256 of its spec hash and the sorted (input, value hash) pairs of everything it read.
- **A run id** = sha256 of the tool (name, version, evaluator versions) and the run's inputs.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from acmodel import TOOL, __version__

NODE_SCHEMA = "acmodel.node.v1"
RUN_SCHEMA = "acmodel.run.v1"


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode("utf-8")


def canonical_text(value: Any) -> str:
    return canonical(value).decode("utf-8")


def digest(data: bytes) -> str:
    """sha256, as 64 lowercase hex digits."""
    return hashlib.sha256(data).hexdigest()


@dataclass(frozen=True)
class Evaluator:
    """How one node kind is evaluated: its version, bumped on any change of what the kind computes (so one
    kind's change never voids the other kinds' entries), and the libraries whose versions it depends on:
    `python` (the minor version), `numpy`, `scipy`."""

    version: int
    libraries: tuple[str, ...] = ()


# The kinds of the derivation graph (core/graph.py). They compute with exact fractions only, so none of
# them declares a library; the Monte Carlo tallies (`sampled`) draw from numpy's Philox, so they declare
# numpy and the Python minor version (design-py D-P5), and the screening kinds of later slices will too.
# Version 2: unknowns are judged by their effect (PY-8), so a scenario records what it leaves unresolved and
# the continuous unknowns it computed with, whose values are enclosures (interval arithmetic). `regen` is a
# regen's amount or interval (exact, like the other derivation kinds); `economy` is the economy's steady
# state or plan (core/flow.py), solved by scipy's HiGHS on numpy arrays, so it declares numpy, scipy and the
# Python minor version (design-py D-P5).
# `engine` is an engine call (engine/client.py): its spec holds the engine's fingerprint, which already
# covers the Python minor version and the library versions (design-py D-P5), so it declares none.
# Version 3 (2 for `impacts`, `point` and `sampled`): the random processes (PY-11). A `beta_from_samples`
# unknown, which any expression may read, is its Wilson enclosure instead of a refusal, and a random rule has a
# value (`core.random`: the exact pity chain and the Wilson bounds compute with integers and fractions only, so
# they declare no library).
# #424 ACM-PRICE PRICE-1: `reference` is a node in the reference state (version 1, a derivation kind like
# `general`), and the steady state reads it; `economy` 2: the steady state reads caps' references, a resource's
# regen stop and void_at_reset caps, leaves out pending flows whose unknowns it cannot read, and shows
# `excluded_pending` and `price_status`.
# PRICE-3: `economy` 3: income caps, `days` periods, the steady cycle as the least common multiple of the
# periods within `economy.max_cycle_steps` (in the steady state's spec), and months and longer periods relaxed by
# rate and shown in `relaxed`.
# PRICE-2a: `economy` 4: the numeraire is a value entity fed by goal sinks, and the steady state solves the
# acquisition A, then the value V with A held, then V alone (S1, S2, S3), refusing a tier conflict and showing
# `acquisition`, `protected` and `conversions`; `objective` (version 1) is an objective's value or horizon.
# `economy` 5: a price whose re-solve with one grain more is infeasible (a regen stop that can hold no more) shows
# a null `low` instead of blocking the steady state, so a steady state that version 4 kept blocked is solved again.
# PRICE-2b-2: `economy` 6: the steady state shows its `coverage` section (the four classes of the pools, the static
# components and the estimates it read) and gives each price the status of its static component; it reads the `no_use`
# rules. A derive's `estimate` is checked where its value is computed (`rule`, `general`, `reference`), but no model
# that held one was read before, so no value of those kinds changes and they keep their versions.
EVALUATORS: dict[str, Evaluator] = {
    "economy": Evaluator(6, ("numpy", "python", "scipy")),
    "engine": Evaluator(1),
    "flow": Evaluator(3),
    "general": Evaluator(3),
    "general_logic": Evaluator(3),
    "impacts": Evaluator(2),
    "member": Evaluator(3),
    "objective": Evaluator(1),
    "offset": Evaluator(3),
    "point": Evaluator(2),
    "reference": Evaluator(1),
    "regen": Evaluator(1),
    "rule": Evaluator(3),
    "sampled": Evaluator(2, ("numpy", "python")),
    "threshold": Evaluator(2),
}


def libraries(python_version: Sequence[int], packages: Mapping[str, str]) -> dict[str, str]:
    """The library versions a kind may declare: the Python minor version and the pinned packages."""
    return {"python": ".".join(str(part) for part in python_version[:2]), **packages}


def evaluator_tokens(evaluators: Mapping[str, Evaluator], versions: Mapping[str, str]) -> dict[str, Any]:
    """Each kind's evaluator version as it enters the keys: its version and its libraries' versions."""
    tokens: dict[str, Any] = {}
    for kind in sorted(evaluators):
        evaluator = evaluators[kind]
        missing = [name for name in evaluator.libraries if name not in versions]
        if missing:  # a kind that declares a library the tool does not report: a defect
            raise ValueError(f"evaluator {kind!r} declares libraries without a version: {missing}")
        tokens[kind] = {
            "version": evaluator.version,
            "libraries": {name: versions[name] for name in sorted(evaluator.libraries)},
        }
    return tokens


def spec_hash(kind: str, token: Any, spec: Any) -> str:
    return digest(canonical([NODE_SCHEMA, kind, token, spec]))


def node_key(spec: str, reads: Sequence[Sequence[Any]]) -> str:
    """The key of one computation of a node: its spec hash and the sorted [input, value hash] pairs."""
    return digest(canonical([spec, sorted(reads, key=canonical_text)]))


def run_id(tokens: Mapping[str, Any], inputs: Mapping[str, Any]) -> str:
    tool = {"name": TOOL, "version": __version__, "evaluators": dict(tokens)}
    return digest(canonical({"schema_version": RUN_SCHEMA, "tool": tool, "inputs": dict(inputs)}))
