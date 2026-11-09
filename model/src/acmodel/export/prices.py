"""`acmodel export prices` (#424 ACM-PRICE PRICE-4a; addendum 2.1 §3.1–§3.4 and §3.7; the design's §4.1, §4.2 and §4.6;
contracts/formats.md, "`acmodel export prices`").

It computes the economy's steady state for the state, without the cache, assesses the impact of its estimates
(`core.impact`), and writes into the model folder:
- `out/provenance/prices/<pool>.json`: the provenance document of each pool's price, as `explain price:<pool>` prints it,
  and the acceptance it is delivered under, if any;
- `out/prices.json` (`acmodel.prices.v1`), last: each pool's price or `pending` with why, its class, the joint vector's
  entry, its impact range and flips, and the digest an acceptance binds; the estimates the steady state read and the
  assessment's groups.

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
from acmodel.codes import ACCEPTANCE_INCOMPLETE, ACCEPTANCE_STALE, ESTIMATE_MISSING, EXPORT_REFUSED
from acmodel.config import EconomySettings, Unknowns
from acmodel.core import flow, impact
from acmodel.core.common import CONFIRMED, solve
from acmodel.core.coverage import CLASSES, COVERED, STRUCTURAL_GAP, UNKNOWN, VERIFIED_ZERO
from acmodel.core.graph import GENERAL, REFERENCE
from acmodel.core.provenance import acceptances, economy_documents, relative
from acmodel.core.run import Run, evaluate_model
from acmodel.core.versions import sha256, versions
from acmodel.envelope import Failure, error
from acmodel.export import jcs
from acmodel.export.policy import delivered
from acmodel.export.thresholds import write_file
from acmodel.expr import Duration
from acmodel.model.jsonfile import refusal
from acmodel.model.readers import EXPORTS

SCHEMA_VERSION = "acmodel.prices.v1"
PRICES = "out/prices.json"
PROVENANCE = "out/provenance/prices"
PENDING = "pending"
GAME = "game"
# Why a price is pending, in the order they are listed (`acmodel.model.readers.PENDING_REASONS`).
WITHHELD, NOT_ACCEPTED, STALE, INCOMPLETE, SCOPE_PARTIAL = (
    "withheld", "not_accepted", "acceptance_stale", "acceptance_incomplete", "scope_partial",
)


def binds(doc: dict[str, Any]) -> str:
    """What an acceptance of a price binds (addendum 2.1 §3.4): the sha256 of the canonical JSON (JCS, `export.jcs`) of
    its provenance document as `explain` prints it, without the acceptance it is delivered under."""
    return digest(jcs.dumps({key: value for key, value in doc.items() if key != "acceptance"}))


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


def build(run: Run) -> tuple[dict[str, bytes], dict[str, Any], list[dict[str, Any]]]:
    """The files of the export, by their path relative to the model folder and in the order they are written
    (`prices.json` last), what the envelope's `data` adds, and the warnings. A refusal raises `Failure`."""
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
    documents = economy_documents(run, [flow.PRICE + pool for pool in pools])
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
        "unit_milli": unit,
        "prices": entries,
        "slots": sorted(resource for resource in data["joint"]["prices"] if resource not in coverage["pools"]),
        "estimates": estimates,
        "impact": assessment.groups,
        "counts": counts,
    }
    return {name: delivered(doc) for name, doc in files.items()}, {"unit_milli": unit, "counts": counts}, warnings


def export_prices(
    path: Path | str,
    state_path: Path | str,
    *,
    datasets: Path | str,
    max_bits: int,
    unknowns: Unknowns = Unknowns(),
    economy: EconomySettings = EconomySettings(),
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Evaluate the model for the state and its steady state, then write the price provenance files and
    `out/prices.json`, or refuse. Returns the envelope's `data` and the warnings."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns, economy=economy)
    files, data, warnings = build(run)
    written = []
    for name, payload in files.items():  # prices.json last
        target = run.folder / name
        write_file(target, payload)
        written.append(str(target))
    return {**run.head, **data, "written": sorted(written)}, warnings
