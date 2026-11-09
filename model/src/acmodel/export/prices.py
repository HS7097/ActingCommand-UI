"""`acmodel export prices` (#424 ACM-PRICE PRICE-4a; addendum 2.1 §3.1–§3.4 and §3.7; the design's §4.1, §4.2 and §4.6;
contracts/formats.md, "`acmodel export prices`").

It computes the economy's steady state for the state, without the cache, assesses the impact of its estimates
(`core.impact`), keeps the steady state's tree once under the state root (PRICE-4a2: `provenance/<model_id>/<sha256>.json`,
never in the repository: it is the bulk of every price's provenance, about a megabyte for the smallest model), and
writes into the model folder:
- `out/provenance/prices/<pool>.json`: the provenance document of each pool's price: its own nodes, and in `shared` the
  steady state's tree named by its digest (with that tree's nodes, the document `explain price:<pool>` prints), and the
  acceptance it is delivered under, if any;
- `out/prices.json` (`acmodel.prices.v1`), last: each pool's price or `pending` with why, its class, the joint vector's
  entry, its impact range and flips, and the digest an acceptance binds; the shared tree's digest with the versions that
  compute it again (`shared`); the estimates the steady state read and the assessment's groups.

`acmodel verify prices` (`verify_prices`) computes the steady state's tree again from the versions prices.json records
and checks it against the recorded digest (`shared_tree_stale`, `shared_tree_mismatch`).

**What is exported** (§3.1): a covered pool whose price is confirmed, as it is; a covered pool with estimates under a
current acceptance (`accept_provisional`: its `binds` is the digest of the price's provenance document as `explain`
prints it, its `flips` holds every activity of the pool whose class flips in the assessment, its `scope` the whole game);
a verified zero, 0, under its `accept_zero` (a conditional one only when its scope is the whole game: the same
`schedule.json` pool is every instance's). Everything else is `pending`, with each reason that holds: a structural gap,
an unknown pool, a withheld coordinate, a component that read a provisional derive without an estimate
(`estimate_missing`), no acceptance, a stale one (`acceptance_stale`), an incomplete one (`acceptance_incomplete`), a
scope short of the game. The game's price unit `prices.unit_milli` (κ) gives each exported price in thousandths of a
priority too (`milli`); prices of two games are never compared (137 K2).

**Refusals**, each writing nothing (exit 5 for `export_refused`): exports.json without `prices` (`model_file_invalid`,
`member_missing`); a steady state that is blocked (`export_refused`, `blocked`, with the errors that block it, as
`economy:steady`); and the assessment's own (`core.impact`). The warnings `estimate_missing`, `acceptance_stale` and
`acceptance_incomplete` say why a price is pending; they never change the exit status. A write that fails is
`export_write_failed`. Nothing is deleted.
"""

from __future__ import annotations

import json
from fractions import Fraction
from pathlib import Path
from typing import Any

from acmodel.cache.keys import digest
from acmodel.check.coverage import judge
from acmodel.check.walk import at
from acmodel.codes import (
    ACCEPTANCE_INCOMPLETE,
    ACCEPTANCE_STALE,
    ESTIMATE_MISSING,
    EXPORT_REFUSED,
    SHARED_TREE_MISMATCH,
    SHARED_TREE_STALE,
)
from acmodel.config import EconomySettings, Unknowns
from acmodel.core import flow, impact
from acmodel.core.common import CONFIRMED, solve
from acmodel.core.coverage import CLASSES, COVERED, STRUCTURAL_GAP, UNKNOWN, VERIFIED_ZERO
from acmodel.core.graph import GENERAL, REFERENCE
from acmodel.core.provenance import SCHEMA_VERSION as PROVENANCE_FORMAT
from acmodel.core.provenance import acceptances, economy_documents, relative, steady_tree
from acmodel.core.run import Run, evaluate_model
from acmodel.core.versions import sha256, versions
from acmodel.envelope import Failure, error
from acmodel.export import jcs
from acmodel.export.policy import delivered
from acmodel.export.thresholds import write_file
from acmodel.expr import Duration
from acmodel.model import read_prices
from acmodel.model.jsonfile import load, refusal
from acmodel.model.readers import EXPORTS
from acmodel.model.readers import PRICES as PRICES_FORMAT

SCHEMA_VERSION = "acmodel.prices.v1"
PRICES = "out/prices.json"
PROVENANCE = "out/provenance/prices"
TREES = "provenance"  # under the state root: the shared trees, by model and digest (PRICE-4a2)
PENDING = "pending"
GAME = "game"
# Why a price is pending, in the order they are listed (`acmodel.model.readers.PENDING_REASONS`).
WITHHELD, NOT_ACCEPTED, STALE, INCOMPLETE, SCOPE_PARTIAL = (
    "withheld", "not_accepted", "acceptance_stale", "acceptance_incomplete", "scope_partial",
)


def binds(doc: dict[str, Any]) -> str:
    """What an acceptance of a price binds (addendum 2.1 §3.4): the sha256 of the canonical JSON (JCS, `export.jcs`) of
    its provenance document as delivered, without the acceptance it is delivered under. The document names the steady
    state's tree by its digest (`shared`, PRICE-4a2), so a change of that tree changes what every price binds."""
    return digest(jcs.dumps({key: value for key, value in doc.items() if key != "acceptance"}))


def tree_digest(tree: dict[str, Any]) -> str:
    """The digest a provenance document names a shared tree by (PRICE-4a2): the sha256 of the canonical JSON (JCS) of
    its tree document (`core.provenance.steady_tree`)."""
    return digest(jcs.dumps(tree))


def tree_file(state_root: Path, model_id: str, sha: str) -> Path:
    """Where the state root keeps a shared tree (PRICE-4a2): `provenance/<model_id>/<sha256>.json`, by its digest, so
    that the trees of several versions stand side by side and none is overwritten by another."""
    return state_root / TREES / model_id / f"{sha}.json"


def _intact(target: Path, sha: str) -> bool:
    """Whether `target` holds a tree document whose digest is `sha`."""
    try:
        doc = json.loads(target.read_bytes().decode("utf-8"))
        return tree_digest(doc) == sha
    except (OSError, ValueError, jcs.NotCanonical):  # absent, unreadable, not JSON, or no tree document
        return False


def keep_tree(state_root: Path, model_id: str, tree: dict[str, Any], sha: str) -> tuple[Path, bool]:
    """The shared tree kept under the state root: its file, and whether it was written now (it is not when an intact
    copy is there already). A write that fails is `export_write_failed`."""
    target = tree_file(state_root, model_id, sha)
    if _intact(target, sha):
        return target, False
    write_file(target, delivered(tree))
    return target, True


def _point(run: Run, derive: str) -> str | None:
    """A provisional derive's point value, as the steady state read it (the reference state, or the general logic)."""
    for key in ((REFERENCE, "rule", derive), (GENERAL, "rule", derive)):
        result = run.graph.results.get(key)
        scenarios = getattr(result, "scenarios", ())
        if scenarios:
            value = scenarios[0].value
            return str(value.seconds if type(value) is Duration else value)
    return None


def _steady(run: Run) -> flow.Economy:
    """The steady state, computed on the run's graph with the keys under `lp` and `economy`; a blocked one refuses."""
    key, graph = flow.NODES[flow.STEADY], run.graph
    solve([key], graph.compute, graph.results)
    steady = graph.results[key]
    if steady.blocked:
        output = flow.OUTPUTS[flow.STEADY]
        detail = {"path": run.exports.path, "output": output, "reason": "blocked"}
        found = [graph.errors[index] for index in sorted(steady.blocked_by)]
        raise Failure(error(EXPORT_REFUSED, detail, "/prices"), *({**root, "needed_by": [output]} for root in found))
    return steady


def build(run: Run) -> tuple[dict[str, bytes], dict[str, Any], list[dict[str, Any]], dict[str, Any]]:
    """The files of the export, by their path relative to the model folder and in the order they are written
    (`prices.json` last), what the envelope's `data` adds, the warnings, and the steady state's tree document, which the
    price documents name by its digest and which is kept under the state root only (PRICE-4a2). A refusal raises
    `Failure`."""
    exports, checker = run.exports, run.checker
    if "prices" not in exports.doc:  # the game's price unit is needed, and has no default
        raise Failure(refusal("member_missing", exports.path, EXPORTS.name, "/prices", key="prices"))
    unit = exports.doc["prices"]["unit_milli"]
    accepted = acceptances(run)
    steady = _steady(run)
    data = steady.data
    coverage = data["coverage"]
    static = judge(checker)
    reach = impact.reached(run.graph, steady, static)
    gaps = {name for name, component in coverage["components"].items() if component["gap"]}
    lacking = sorted(derive for derive in reach if "estimate" not in checker.entries[derive].doc["logic"])
    missing = set().union(*(reach[derive] for derive in lacking)) if lacking else set()
    assessment = impact.assess(run.graph, steady, reach, gaps, missing)
    pools = sorted(coverage["pools"])
    tree = steady_tree(run)
    shared = {tree["root"]: tree_digest(tree)}
    documents = economy_documents(run, [flow.PRICE + pool for pool in pools], shared)
    touching: dict[str, set[str]] = {}
    for activity in set(static.takes) | set(static.brings):
        for entity in static.takes.get(activity, set()) | static.brings.get(activity, set()):
            touching.setdefault(entity, set()).add(activity)
    warnings: list[dict[str, Any]] = []
    for derive in lacking:
        entry = checker.entries[derive]
        touched = sorted(pool for pool in pools if coverage["pools"][pool]["component"] in reach[derive])
        detail = {"path": entry.path, "id": derive, "pools": touched}
        warnings.append(error(ESTIMATE_MISSING, detail, at(entry.ref, "logic")))
    files: dict[str, Any] = {}
    entries = []
    for pool in pools:
        shown, joint = coverage["pools"][pool], data["joint"]["prices"][pool]
        component, document = shown["component"], documents[flow.PRICE + pool]
        digest_of = binds(document)
        flips = sorted(touching.get(pool, set()) & assessment.flips)
        index = accepted.get(flow.PRICE + pool)
        acceptance = None if index is None else exports.doc["acceptances"][index]
        pending: list[str] = []
        if shown["class"] in (STRUCTURAL_GAP, UNKNOWN):
            pending.append(shown["class"])
        if shown["class"] == COVERED and joint["withheld"] is not None:
            pending.append(WITHHELD)
        if shown["class"] != VERIFIED_ZERO and component in missing:
            pending.append(ESTIMATE_MISSING)
        kind = None
        if not pending and (shown["class"] == VERIFIED_ZERO or shown["status"] != CONFIRMED):
            kind = "accept_zero" if shown["class"] == VERIFIED_ZERO else "accept_provisional"
            given = None if acceptance is None else acceptance.get(kind)
            if given is None:
                pending.append(NOT_ACCEPTED)
            elif kind == "accept_provisional":
                output, where = flow.PRICE + pool, at("", "acceptances", index, kind)
                if given["binds"] != digest_of:
                    pending.append(STALE)
                    detail = {"path": exports.path, "output": output, "binds": given["binds"], "expected": digest_of}
                    warnings.append(error(ACCEPTANCE_STALE, detail, at(where, "binds")))
                lacked = sorted(set(flips) - set(given["flips"]))
                if lacked:
                    pending.append(INCOMPLETE)
                    detail = {"path": exports.path, "output": output, "missing": lacked}
                    warnings.append(error(ACCEPTANCE_INCOMPLETE, detail, at(where, "flips")))
                if given["scope"] != GAME:
                    pending.append(SCOPE_PARTIAL)
            elif "facts" in checker.entries[shown["no_use"]].doc["logic"] and given["scope"] != GAME:
                pending.append(SCOPE_PARTIAL)  # a conditional zero holds where it was verified, never on every instance
        price = PENDING if pending else ("0" if shown["class"] == VERIFIED_ZERO else joint["value"])
        name = f"{PROVENANCE}/{pool}.json"
        if not pending and kind is not None:  # delivered under its acceptance, which the provenance file records
            document = {
                **document,
                "acceptance": {
                    "file": relative(run, exports.path), "ref": at("", "acceptances", index), "sha256": sha256(acceptance),
                    kind: acceptance[kind],
                },
            }
        files[name] = document
        impact_range = assessment.ranges[pool] if component in assessment.assessed else None
        entries.append({
            "pool": pool,
            "class": shown["class"],
            "status": shown["status"],
            "reasons": shown["reasons"],
            "component": component,
            "price": price,
            "milli": None if pending else str(Fraction(price) * unit),
            "pending": pending,
            "joint": joint,
            "degenerate": isinstance(joint["interval"], dict),
            "varies_by_step": len({json.dumps(step, sort_keys=True) for step in data["prices"][pool]}) > 1,
            "estimates": sorted(derive for derive, components in reach.items() if component in components),
            "impact": None if impact_range is None else {"low": str(impact_range[0]), "high": str(impact_range[1])},
            "flips": flips,
            "binds": digest_of,
            "acceptance": None if acceptance is None else {"entry": at("", "acceptances", index), **acceptance},
            "provenance": name,
        })
    estimates = []
    for derive in sorted(reach):
        logic = checker.entries[derive].doc["logic"]
        skipped = impact.MISSING if derive in lacking else assessment.skipped.get(derive)
        estimates.append({
            "rule": derive, "value": _point(run, derive), "estimate": logic.get("estimate"),
            "components": sorted(reach[derive]), "assessed": skipped is None, "skipped": skipped,
        })
    counts = {kind: sum(1 for entry in entries if entry["class"] == kind) for kind in CLASSES}
    counts["exported"] = sum(1 for entry in entries if not entry["pending"])
    counts["pending"] = len(entries) - counts["exported"]
    files[PRICES] = {
        "schema_version": SCHEMA_VERSION,
        "model_id": run.head["model_id"],
        "versions": versions(checker, run.folder),
        "shared": {
            root: {"sha256": sha, "versions": tree["versions"], "nodes": len(tree["nodes"])} for root, sha in shared.items()
        },
        "unit_milli": unit,
        "prices": entries,
        "slots": sorted(resource for resource in data["joint"]["prices"] if resource not in coverage["pools"]),
        "estimates": estimates,
        "impact": assessment.groups,
        "counts": counts,
    }
    shown = {"unit_milli": unit, "counts": counts}
    return {name: delivered(doc) for name, doc in files.items()}, shown, warnings, tree


def export_prices(
    path: Path | str,
    state_path: Path | str,
    *,
    state_root: Path,
    datasets: Path | str,
    max_bits: int,
    unknowns: Unknowns = Unknowns(),
    economy: EconomySettings = EconomySettings(),
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Evaluate the model for the state and its steady state, then keep the steady state's tree under the state root
    (PRICE-4a2) and write the price provenance files and `out/prices.json`, or refuse. Returns the envelope's `data`
    and the warnings."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns, economy=economy)
    files, data, warnings, tree = build(run)
    sha = tree_digest(tree)
    kept, wrote = keep_tree(state_root, run.head["model_id"], tree, sha)  # first: a price document names it
    written = [str(kept)] if wrote else []
    for name, payload in files.items():  # prices.json last
        target = run.folder / name
        write_file(target, payload)
        written.append(str(target))
    shared = {tree["root"]: {"sha256": sha, "file": str(kept)}}
    return {**run.head, **data, "shared": shared, "written": sorted(written)}, warnings


def _verified_steady(run: Run) -> flow.Economy:
    """The steady state computed again for `verify prices`; a blocked one is reported with the errors that block it,
    each needed by `economy:steady`."""
    key, graph = flow.NODES[flow.STEADY], run.graph
    solve([key], graph.compute, graph.results)
    steady = graph.results[key]
    if steady.blocked:
        output = flow.OUTPUTS[flow.STEADY]
        raise Failure(*({**graph.errors[index], "needed_by": [output]} for index in sorted(steady.blocked_by)))
    return steady


def verify_prices(
    path: Path | str,
    state_path: Path | str,
    *,
    state_root: Path,
    datasets: Path | str,
    max_bits: int,
    unknowns: Unknowns = Unknowns(),
    economy: EconomySettings = EconomySettings(),
) -> dict[str, Any]:
    """`acmodel verify prices` (PRICE-4a2): the shared trees that `out/prices.json` names, checked against the model.

    1. prices.json is read (`model_file_invalid` when it is not there or not a prices file; a file of another model is
       `value_invalid` at `/model_id`).
    2. A tree whose recorded `versions` are not the model's cannot be computed again from this model:
       `shared_tree_stale`, with the members that differ, and nothing is computed.
    3. Otherwise the steady state is computed again (with the keys under `lp` and `economy`, as the export does), and
       its tree's digest must be the recorded one (`shared_tree_mismatch`, `recomputed`); so must the digest each
       price's provenance file names (`shared_tree_mismatch`, `referenced`, at that file's `shared`).
    4. A tree that checks is kept under the state root when it is not there intact (`restored`).

    Every problem is reported at once (exit 6); it writes nothing else."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns, economy=economy)
    file = run.folder / PRICES
    doc = read_prices(file)
    if doc["model_id"] != run.head["model_id"]:
        raise Failure(refusal(
            "value_invalid", file, PRICES_FORMAT.name, "/model_id", expected=[run.head["model_id"]],
            literal=doc["model_id"],
        ))
    model = versions(run.checker, run.folder)
    errors = []
    for root, recorded in sorted(doc["shared"].items()):
        differ = [member for member in ("rules", "data", "tool") if recorded["versions"][member] != model[member]]
        if differ:
            detail = {"path": str(file), "root": root, "members": differ}
            errors.append(error(SHARED_TREE_STALE, detail, at("", "shared", root, "versions")))
    if errors:
        raise Failure(*errors)
    _verified_steady(run)
    tree = steady_tree(run)
    found = {tree["root"]: tree_digest(tree)}
    shown: dict[str, Any] = {}
    for root in sorted(set(doc["shared"]) | set(found)):  # a tree recorded and not computed, or computed and not recorded
        recorded, computed = doc["shared"].get(root), found.get(root)
        expected = None if recorded is None else recorded["sha256"]
        if computed != expected:
            detail = {
                "path": str(file), "root": root, "reason": "recomputed", "expected": expected or "none",
                "found": computed or "none",
            }
            where = at("", "shared") if recorded is None else at("", "shared", root, "sha256")
            errors.append(error(SHARED_TREE_MISMATCH, detail, where))
        if recorded is not None:
            shown[root] = {"sha256": expected, "recomputed": computed, "nodes": recorded["nodes"]}
    for entry in doc["prices"]:
        price_file = run.folder / entry["provenance"]
        written = load(price_file, PROVENANCE_FORMAT)
        named = written.get("shared") if isinstance(written, dict) else None
        named = named if isinstance(named, dict) else {}
        for root, recorded in sorted(doc["shared"].items()):
            if named.get(root) != recorded["sha256"]:
                detail = {
                    "path": str(price_file), "root": root, "reason": "referenced", "expected": recorded["sha256"],
                    "found": named.get(root) or "none",
                }
                errors.append(error(SHARED_TREE_MISMATCH, detail, at("", "shared", root)))
    if errors:
        raise Failure(*errors)
    for root, entry in shown.items():
        kept, restored = keep_tree(state_root, run.head["model_id"], tree, entry["sha256"])
        entry.update(file=str(kept), restored=restored)
    return {**run.head, "file": str(file), "shared": shown}
