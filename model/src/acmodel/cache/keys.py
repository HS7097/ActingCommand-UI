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
# PRICE-2b-1: `economy` 7: the steady state solves its dual and checks it exactly; a resource's price per step is
# its projection on the dual's optimal face (the re-solves move to `grain`); it shows the joint price vector and the
# activity judgements (`joint`); an economy node keeps what it read (`reads`), the provenance of `price:` and
# `activity_value:`; and a HiGHS marginal within `MARGINAL_TOLERANCE` of its exact price (relative, 1e-7) is no
# longer `lp_dual_disagrees`.
# PRICE-2c: `economy` 8: an LP's exact solution is the vertex of HiGHS's basis, made exactly optimal by at most
# `lp.max_pivots` exact pivots (in the steady state's spec); complementary slackness at the exact optimal dual spares
# the tie and `most` probes of an activity no optimum runs; a per-activity probe that cannot be verified exactly is
# `lp_probe_unverified` (a warning, its activity a structural gap, `probe_unverified`) instead of blocking the steady
# state; each activity judgement lists what is `unverified`. The values on D* are the same, found with fewer and smaller
# LPs: S3's basis proves a chord or a projection's end where it stays optimal, a projection pinned by its re-solves'
# chords needs none, a pool or activity constant on D* none, and the others are solved on D* in its
# complementary-slackness form, with HiGHS warm-started between LPs on the same rows.
# PRICE-2d: `economy` 9: a resource's re-solves at plus and minus one grain start from S3 (S3's basis proves a chord of
# a resource with a hard cap too; otherwise HiGHS's instance of S3 with the moved row ends, warm-started, and its vertex
# solved by updating S3's factorised basis where HiGHS ends near it, else by a new elimination). Each is the exact
# optimum, as a re-solve whose basis was certified gave it before; a re-solve whose new LP was not certified gave a
# feasible value that need not be the optimum, so a value can change there, and version 8's entries are not reused.
# PRICE-2e: `economy` 10: every LP on D* (the selection's steps and the vector's check too, which also reads the
# strong-duality row exactly) is solved on D*'s complementary-slackness form, and each probe of S3's optima (`most`,
# the ties) on S3's optimal face in its complementary-slackness form at the exact optimal dual, neither with the dense
# row of the objective; the selection and the `most` probes are warm-started; an LP's basis solves its primal and its
# dual with one elimination. Every optimal value is the same set's, so a certified one is the same; but an LP of the
# selection whose basis is not certified gives a feasible value that need not be the optimum, and a probe may be
# certified on one form and not on the other (`lp_probe_unverified`), so a value can change there, and version 9's
# entries are not reused.
EVALUATORS: dict[str, Evaluator] = {
    "economy": Evaluator(10, ("numpy", "python", "scipy")),
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
