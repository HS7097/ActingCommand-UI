"""`acmodel export prices` (#424 ACM-PRICE PRICE-4a; addendum 2.1 §3.1–§3.4 and §3.7; the design's §4.1, §4.2 and §4.6;
contracts/formats.md, "`acmodel export prices`").

It computes the economy's steady state for the state, without the cache, assesses the impact of its estimates
(`core.impact`), keeps the steady state's tree once under the state root (PRICE-4a2: `provenance/<model_id>/<sha256>.json`,
never in the repository: it is the bulk of every price's provenance, about a megabyte for the smallest model), and
writes into the model folder:
- `out/provenance/prices/<pool>.json`: the provenance document of each pool's price: its own nodes, and in `shared` the
  steady state's tree named by its digest (with that tree's nodes, the document `explain price:<pool>` prints), and the
  acceptance it is delivered under, if any;
- `out/prices.json` (`acmodel.prices.v2`), last: each pool's price or `pending` with why, its class, the joint vector's
  entry, its impact range and flips, and the digest an acceptance binds; the shared tree's digest with the versions that
  compute it again (`shared`); the estimates the steady state read, each with its components, and the assessment's
  groups. A file of `acmodel.prices.v1`, written before, also listed in each pool's entry the estimates its component
  read; it is still read and verified.

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

**The evaluator digest and the reused assessment** (#424 PRICE-REUSE). prices.json records the evaluator digest
(`evaluator`, `core.evaluator`): the tool's source, the libraries of the steady state's cache key, and every configuration
key but those listed as never changing a result and those only the plan reads; neither the versions nor the price
provenance files hold it, so no `binds` moves with it. With `reuse` (the out folder of an earlier export,
`--reuse-assessment`), the impact assessment, which reads neither exports.json nor the account state, is taken over
from that folder's prices.json instead of computed again, but only when its versions, its
evaluator digest and, once the steady state is computed, its shared tree's digest are this export's; otherwise
`reuse_refused`, one error per member that differs, and nothing is written. Each pool's `flips` and `impact`, each
estimate's `assessed` and `skipped`, and the groups (`impact`) come from it; everything else is computed as without the
option, and prices.json records where they came from (`reused_assessment`). Without that member it is the full export's
file byte for byte, but for each pass's `timing_ms` (PASS-TIMING, `core.impact`): taken over with its group, it is the
earlier run's.

**Timing** (PASS-TIMING). Each pass of the groups records how long it took (`timing_ms`, `core.impact`); nothing else
the export writes holds it: not the price provenance files (so no `binds`), the shared tree, `estimates` nor `counts`.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from fractions import Fraction
from pathlib import Path
from typing import Any, Mapping

from acmodel.cache.keys import digest
from acmodel.check.coverage import judge
from acmodel.check.walk import at
from acmodel.codes import (
    ACCEPTANCE_INCOMPLETE,
    ACCEPTANCE_STALE,
    ESTIMATE_MISSING,
    EXPORT_REFUSED,
    REUSE_REFUSED,
    SHARED_TREE_MISMATCH,
    SHARED_TREE_STALE,
)
from acmodel.config import Config, EconomySettings, Unknowns
from acmodel.core import evaluator, flow, impact
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
from acmodel.model import read_prices, readers
from acmodel.model.jsonfile import load, read, refusal
from acmodel.model.readers import EXPORTS

SCHEMA_VERSION = "acmodel.prices.v2"
PRICES = "out/prices.json"
PROVENANCE = "out/provenance/prices"
TREES = "provenance"  # under the state root: the shared trees, by model and digest (PRICE-4a2)
PENDING = "pending"
GAME = "game"
# Why a price is pending, in the order they are listed (`acmodel.model.readers.PENDING_REASONS`).
WITHHELD, NOT_ACCEPTED, STALE, INCOMPLETE, SCOPE_PARTIAL = (
    "withheld", "not_accepted", "acceptance_stale", "acceptance_incomplete", "scope_partial",
)
# PRICE-REUSE: prices.json's evaluator digest (`core.evaluator`), and the member an export that took its assessment over
# from an earlier one records it in, with the members it took over.
EVALUATOR, REUSED = "evaluator", "reused_assessment"
TAKEN = ("estimates.assessed", "estimates.skipped", "impact", "prices.flips", "prices.impact")
# Why `reuse_refused`: the earlier file records no evaluator digest, or a member differs; `none` stands for a side that
# has no such member.
NOT_RECORDED, DIFFERS, NONE = "not_recorded", "differs", "none"


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


@dataclass(frozen=True)
class Earlier:
    """The prices.json of an earlier export whose impact assessment this one takes over (PRICE-REUSE): where it is, its
    bytes and its document."""

    path: Path
    raw: bytes
    doc: dict[str, Any]

    def marker(self) -> dict[str, Any]:
        """What prices.json records of it (`reused_assessment`): the sha256 of its bytes, its versions, the digests of
        its shared trees and of its evaluator, and the members taken over."""
        return {
            "prices_sha256": digest(self.raw),
            "versions": self.doc["versions"],
            "shared": {root: entry["sha256"] for root, entry in sorted(self.doc["shared"].items())},
            "evaluator": self.doc[EVALUATOR]["sha256"],
            "members": list(TAKEN),
        }


def read_earlier(folder: Path) -> Earlier:
    """`prices.json` in the out folder of an earlier export (`--reuse-assessment`), read as `verify prices` reads it
    (`model_file_invalid` when it is not there or not a prices file)."""
    file = folder / Path(PRICES).name
    raw = read(file, readers.PRICES.name)
    return Earlier(file, raw, read_prices(file, raw))


@dataclass(frozen=True)
class _Found:
    """What the impact assessment gives the export, computed or taken over: each pool's `flips` and `impact`, each
    estimate's `skipped` (None when it was assessed), and the groups."""

    flips: dict[str, list[str]]
    impact: dict[str, dict[str, str] | None]
    skipped: dict[str, str | None]
    groups: list[dict[str, Any]]


def _assessed(
    run: Run, steady: flow.Economy, reach: dict[str, set[str]], gaps: set[str], missing: set[str], lacking: list[str],
    touching: dict[str, set[str]],
) -> _Found:
    """The impact assessment (`core.impact`) of the steady state, as the export shows it."""
    assessment = impact.assess(run.graph, steady, reach, gaps, missing)
    pools = steady.data["coverage"]["pools"]
    shown: dict[str, dict[str, str] | None] = {}
    for pool in pools:
        found = assessment.ranges[pool] if pools[pool]["component"] in assessment.assessed else None
        shown[pool] = None if found is None else {"low": str(found[0]), "high": str(found[1])}
    return _Found(
        flips={pool: sorted(touching.get(pool, set()) & assessment.flips) for pool in pools},
        impact=shown,
        skipped={derive: impact.MISSING if derive in lacking else assessment.skipped.get(derive) for derive in reach},
        groups=assessment.groups,
    )


def _token(value: Any) -> str:
    """A value of `reuse_refused` (`expected`, `found`): `none` for an absent one, `true` or `false` for a boolean (a
    configuration key of the evaluator digest)."""
    if value is None:
        return NONE
    if type(value) is bool:
        return "true" if value else "false"
    return str(value)


def _refused(earlier: Earlier, member: str, ref: str, expected: Any, found: Any, reason: str = DIFFERS) -> dict[str, Any]:
    detail = {"path": str(earlier.path), "member": member, "reason": reason, "expected": _token(expected),
              "found": _token(found)}
    return error(REUSE_REFUSED, detail, ref)


def differences(earlier: Earlier, model: dict[str, str], recorded: dict[str, Any]) -> list[dict[str, Any]]:
    """`reuse_refused` of each member of the earlier prices.json that is not this export's, before the steady state is
    computed: each of the versions (`rules`, `data`, `tool`), then the evaluator digest. An earlier file without one
    (written before PRICE-REUSE, or of v1) is `not_recorded`. A digest of the evaluator that differs is listed member by
    member (`source`, each library, each configuration key), and as `sha256` only when none of them differs."""
    doc, found = earlier.doc, []
    for name in ("rules", "data", "tool"):
        if doc["versions"][name] != model[name]:
            where = at("", "versions", name)
            found.append(_refused(earlier, f"versions.{name}", where, doc["versions"][name], model[name]))
    before = doc.get(EVALUATOR)
    if before is None:
        found.append(_refused(earlier, EVALUATOR, at("", EVALUATOR), None, recorded["sha256"], NOT_RECORDED))
        return found
    body = []
    if before["source"] != recorded["source"]:
        where = at("", EVALUATOR, "source")
        body.append(_refused(earlier, "evaluator.source", where, before["source"], recorded["source"]))
    for group in ("libraries", "config"):
        for key in sorted(set(before[group]) | set(recorded[group])):
            if before[group].get(key) != recorded[group].get(key):
                member, where = f"evaluator.{group}.{key}", at("", EVALUATOR, group, key)
                body.append(_refused(earlier, member, where, before[group].get(key), recorded[group].get(key)))
    if not body and before["sha256"] != recorded["sha256"]:
        where = at("", EVALUATOR, "sha256")
        body.append(_refused(earlier, "evaluator.sha256", where, before["sha256"], recorded["sha256"]))
    return found + body


def _taken(earlier: Earlier, shared: dict[str, str], pools: list[str], reach: dict[str, set[str]]) -> _Found:
    """The earlier export's assessment, once its shared trees are this steady state's (`reuse_refused`, `shared.<root>`,
    otherwise), the members taken over are as `export prices` writes them (`model_file_invalid` otherwise), and it lists
    the same pools and estimates (`reuse_refused`, `prices` or `estimates`, otherwise: an earlier file edited by hand)."""
    doc = earlier.doc
    recorded = {root: entry["sha256"] for root, entry in doc["shared"].items()}
    errors = [
        _refused(earlier, f"shared.{root}", at("", "shared", root), recorded.get(root), shared.get(root))
        for root in sorted(set(recorded) | set(shared)) if recorded.get(root) != shared.get(root)
    ]
    if errors:
        raise Failure(*errors)
    readers.check_assessment(doc, earlier.path)
    entries = {entry["pool"]: entry for entry in doc["prices"]}
    estimates = {entry["rule"]: entry for entry in doc["estimates"]}
    for member, before, now in (("prices", set(entries), set(pools)), ("estimates", set(estimates), set(reach))):
        errors += [_refused(earlier, member, at("", member), name, None) for name in sorted(before - now)]
        errors += [_refused(earlier, member, at("", member), None, name) for name in sorted(now - before)]
    if errors:
        raise Failure(*errors)
    return _Found(
        flips={pool: entries[pool]["flips"] for pool in pools},
        impact={pool: entries[pool]["impact"] for pool in pools},
        skipped={derive: estimates[derive]["skipped"] for derive in reach},
        groups=doc["impact"],
    )


def build(
    run: Run, recorded: dict[str, Any], earlier: Earlier | None = None
) -> tuple[dict[str, bytes], dict[str, Any], list[dict[str, Any]], dict[str, Any]]:
    """The files of the export, by their path relative to the model folder and in the order they are written
    (`prices.json` last), what the envelope's `data` adds, the warnings, and the steady state's tree document, which the
    price documents name by its digest and which is kept under the state root only (PRICE-4a2). `recorded` is the
    evaluator digest prices.json records (PRICE-REUSE); with `earlier`, the impact assessment is taken over from it
    (`_taken`) instead of computed. A refusal raises `Failure`."""
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
    touching: dict[str, set[str]] = {}
    for activity in set(static.takes) | set(static.brings):
        for entity in static.takes.get(activity, set()) | static.brings.get(activity, set()):
            touching.setdefault(entity, set()).add(activity)
    if earlier is None:
        assessment = _assessed(run, steady, reach, gaps, missing, lacking, touching)
    pools = sorted(coverage["pools"])
    tree = steady_tree(run)
    shared = {tree["root"]: tree_digest(tree)}
    if earlier is not None:  # compared before anything is taken over: the assessment is never run
        assessment = _taken(earlier, shared, pools, reach)
    documents = economy_documents(run, [flow.PRICE + pool for pool in pools], shared)
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
        flips = assessment.flips[pool]
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
            "impact": assessment.impact[pool],
            "flips": flips,
            "binds": digest_of,
            "acceptance": None if acceptance is None else {"entry": at("", "acceptances", index), **acceptance},
            "provenance": name,
        })
    estimates = []
    for derive in sorted(reach):
        logic = checker.entries[derive].doc["logic"]
        skipped = assessment.skipped[derive]
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
        EVALUATOR: recorded,
    }
    shown: dict[str, Any] = {"unit_milli": unit, "counts": counts}
    if earlier is not None:
        marker = earlier.marker()
        files[PRICES][REUSED] = marker
        shown[REUSED] = {"file": str(earlier.path), **marker}
    return {name: delivered(doc) for name, doc in files.items()}, shown, warnings, tree


def export_prices(
    path: Path | str,
    state_path: Path | str,
    *,
    state_root: Path,
    datasets: Path | str,
    max_bits: int,
    libraries: Mapping[str, str],
    settings: Config,
    unknowns: Unknowns = Unknowns(),
    economy: EconomySettings = EconomySettings(),
    reuse: Path | str | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Evaluate the model for the state and its steady state, then keep the steady state's tree under the state root
    (PRICE-4a2) and write the price provenance files and `out/prices.json`, or refuse. Returns the envelope's `data`
    and the warnings. `libraries` holds the versions the tool runs with (`cache.keys.libraries`), and `settings` the
    configuration that `max_bits`, `unknowns` and `economy` come from, for the evaluator digest (PRICE-REUSE). With
    `reuse`, the out folder of an earlier export, its prices.json is read first, and its impact assessment is taken over
    when its versions, evaluator digest and shared tree are this export's; otherwise `reuse_refused`, and nothing is
    written."""
    if (max_bits, unknowns, economy) != (settings.expr_max_bits, settings.unknowns, settings.economy):
        raise ValueError("export_prices: max_bits, unknowns and economy differ from those of settings, which the "
                         "evaluator digest records")
    earlier = None if reuse is None else read_earlier(Path(reuse))
    recorded = evaluator.record(libraries, settings)
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns, economy=economy)
    if earlier is not None:
        refused = differences(earlier, versions(run.checker, run.folder), recorded)
        if refused:
            raise Failure(*refused)
    files, data, warnings, tree = build(run, recorded, earlier)
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

    1. prices.json is read, `acmodel.prices.v2` or `acmodel.prices.v1` (`model_file_invalid` when it is not there or not
       a prices file; a file of another model is `value_invalid` at `/model_id`).
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
            "value_invalid", file, doc["schema_version"], "/model_id", expected=[run.head["model_id"]],
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
