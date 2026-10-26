"""`acmodel export thresholds` (#424 design 6.3, 6.6): `thresholds.json` (`acmodel.thresholds.v1`) and one
provenance file per threshold, written into the model folder:
- `out/thresholds.json`: each threshold in its form, with its dimension, status and provenance file;
- `out/provenance/thresholds/<id>.json`: its provenance document (`core.provenance`), as `explain` prints it.

**Shared outputs carry no per-account value** (#424, ruling 66). Both files are delivered with the
resource repository. A threshold is exported only when it is confirmed, reads nothing of the account state,
and is an integer in the JSON-safe range. Otherwise it is refused (`export_refused`, exit 5), with the errors
that block it, and nothing at all is written. A per-account value is never written here: `run` and
`explain` show it locally.

Provenance files are written first and `thresholds.json` last, so that `thresholds.json` never names a
provenance file that was not written. A write that fails is `export_write_failed`. Nothing is deleted,
not even the provenance file of a threshold that `exports.json` no longer lists.
"""

from __future__ import annotations

import json
from fractions import Fraction
from pathlib import Path
from typing import Any

from acmodel.check.walk import at
from acmodel.codes import EXPORT_REFUSED, EXPORT_WRITE_FAILED
from acmodel.config import Unknowns
from acmodel.core.common import BLOCKED, CONFIRMED
from acmodel.core.provenance import STATE_NODES, assemble
from acmodel.core.run import Run, evaluate_model
from acmodel.core.versions import versions
from acmodel.envelope import Failure, error
from acmodel.model.jsonfile import MAX_SAFE_INTEGER

SCHEMA_VERSION = "acmodel.thresholds.v1"
THRESHOLDS = "out/thresholds.json"
PROVENANCE = "out/provenance/thresholds"
LIVE = ("fact", "comparison", "reference_fact")


def _dump(doc: Any) -> bytes:
    """A delivered file: sorted keys, two-space indent, UTF-8, LF line ends, one final newline."""
    return (json.dumps(doc, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def _refusal(run: Run, index: int, output: str, reason: str, **values: Any) -> dict[str, Any]:
    detail = {"path": run.exports.path, "output": output, "reason": reason, **values}
    return error(EXPORT_REFUSED, detail, at("", "thresholds", index))


def threshold_entry(run: Run, index: int, threshold: dict[str, Any], files: dict[str, Any]) -> dict[str, Any]:
    """The thresholds.json entry of one threshold, its provenance document added to `files`; or the
    `export_refused` error. `bindings check|apply` take their values from here too, so they bind only what
    this export writes."""
    output, form = "threshold:" + threshold["id"], threshold["form"]
    shown = run.data["thresholds"][threshold["id"]]
    if shown["status"] == BLOCKED:
        return _refusal(run, index, output, "blocked")
    doc = assemble(run, output)
    if shown["per_account"] or any(node["node"] in STATE_NODES for node in doc["nodes"].values()):
        return _refusal(run, index, output, "per_account")
    if shown["status"] != CONFIRMED:  # acceptances of provisional values come with the export policy
        return _refusal(run, index, output, "provisional")
    member = "value" if form == "constant" else "offset"
    # A confirmed output that read unknowns lists a value per scenario; they are one integer, or differ below
    # export precision (the no-effect rule), and then one of them is not an integer.
    values = [shown[member]] if member in shown else sorted({v[member] for v in shown["variants"]}, key=Fraction)
    odd = next((value for value in values if Fraction(value).denominator != 1), None)
    if odd is not None:
        return _refusal(run, index, output, "not_integer", value=odd)
    if len(values) != 1:  # two integers differ by a whole unit, which is never without effect
        raise ValueError(f"confirmed threshold {threshold['id']!r} with the values {values}")
    number = Fraction(values[0])
    if abs(number) > MAX_SAFE_INTEGER:
        return _refusal(run, index, output, "out_of_range", value=values[0])
    name = f"{PROVENANCE}/{threshold['id']}.json"
    files[name] = doc
    entry = {
        "id": threshold["id"], "form": form, "dimension": shown["type"], "status": shown["status"], "provenance": name,
    }
    if form == "fact_offset":
        entry.update({key: shown[key] for key in LIVE})
    entry[member] = int(number)
    return entry


def _write(target: Path, doc: Any) -> None:
    try:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(_dump(doc))
    except OSError as exc:
        detail = {"path": str(target), "exception_type": type(exc).__name__, "raw_text": str(exc)}
        raise Failure(error(EXPORT_WRITE_FAILED, detail)) from exc


def export_thresholds(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns = Unknowns()
) -> dict[str, Any]:
    """Evaluate the model for the state, then write `thresholds.json` and the provenance files, or refuse."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    files: dict[str, Any] = {}
    entries, refusals = [], []
    for index, threshold in sorted(enumerate(run.exports.doc["thresholds"]), key=lambda pair: pair[1]["id"]):
        entry = threshold_entry(run, index, threshold, files)
        (refusals if entry.get("code") == EXPORT_REFUSED else entries).append(entry)
    if refusals:
        blocked = {entry["detail"]["output"] for entry in refusals if entry["detail"]["reason"] == "blocked"}
        raise Failure(*refusals, *(entry for entry in run.errors if blocked.intersection(entry["needed_by"])))
    files[THRESHOLDS] = {
        "schema_version": SCHEMA_VERSION,
        "model_id": run.head["model_id"],
        "versions": versions(run.checker, run.folder),
        "thresholds": entries,
    }
    written = []
    for name, doc in files.items():  # thresholds.json last
        target = run.folder / name
        _write(target, doc)
        written.append(str(target))
    return {**run.head, "written": sorted(written)}
