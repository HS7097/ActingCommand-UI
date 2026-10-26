"""The export policy (#424 design 6.6) of the values the tables and the scoring documents take from the model.

| Output | Table cell | Scoring placeholder |
|---|---|---|
| confirmed | its value | its value |
| provisional | `null` (an unresolved estimate) unless accepted | refused unless accepted |
| blocked | refused | refused |
| per-account (it read the account state) | refused | refused |

- **Acceptances** are `acceptances` of exports.json (`core.provenance.acceptances`); an accepted provisional value
  is delivered, and its provenance file records the acceptance.
- **A number** is the output's exact value times the declared `scale`, rounded by the declared `round` (`floor`,
  `ceil` or `half_even`), never implicitly. Every scenario of the output (each variant, and both ends of an
  enclosure) must give the same integer; rounding never decreases, so the true value then gives it too.
  Otherwise the value is refused (`not_single`), never one of them picked. An integer beyond ±(2**53 − 1) is
  `out_of_range`.
- **A boolean** must be one value in every scenario; an undecided one is `not_single`.

A refusal is `export_refused` at the cell or placeholder; the caller adds the errors that block a blocked output
(`blocking`).
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, field
from fractions import Fraction
from typing import Any, Callable

from acmodel.codes import EXPORT_REFUSED, EXPR_DIMENSION_MISMATCH, RULE_REFERENCE_MISSING
from acmodel.core.common import BLOCKED, CONFIRMED
from acmodel.core.provenance import STATE_NODES, assemble, output_result, outputs
from acmodel.core.run import Run
from acmodel.envelope import error
from acmodel.expr import BOOL
from acmodel.model.jsonfile import MAX_SAFE_INTEGER

ROUNDING: dict[str, Callable[[Fraction], int]] = {"floor": math.floor, "ceil": math.ceil, "half_even": round}
# Where each kind of output keeps its provenance file, in the model folder (the thresholds export's too).
PROVENANCE_FOLDERS = {"rule": "rules", "general_logic": "general_logic", "threshold": "thresholds"}


@dataclass(frozen=True)
class Resolved:
    """A value an export may deliver: `value` is None for a provisional output that is not accepted (a table
    writes `null`); otherwise `document` is its provenance document."""

    output: str
    value: int | bool | None
    accepted: bool
    document: dict[str, Any] | None


@dataclass(frozen=True)
class Refused:
    """Why an output cannot be delivered: the `reason` of `export_refused` and its other values."""

    reason: str
    values: dict[str, Any] = field(default_factory=dict)


def provenance_name(output: str) -> str:
    """The provenance file of an output, relative to the model folder: `out/provenance/<kind>/<id>.json`."""
    kind, name = output.split(":", 1)
    return f"out/provenance/{PROVENANCE_FOLDERS[kind]}/{name}.json"


def delivered(doc: Any) -> bytes:
    """A provenance file: sorted keys, two-space indent, UTF-8, LF line ends, one final newline (as the
    thresholds export writes them)."""
    return (json.dumps(doc, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def scaled(value: Fraction, scale: int, mode: str) -> int:
    """`value` times `scale`, rounded by `mode`, exactly."""
    return int(ROUNDING[mode](value * scale))


def refused(path: str, output: str, ref: str, found: Refused) -> dict[str, Any]:
    """The `export_refused` error of one cell or placeholder (`ref` in the file `path`)."""
    return error(EXPORT_REFUSED, {"path": path, "output": output, "reason": found.reason, **found.values}, ref)


def blocking(run: Run, refusals: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """The errors of the run that block the outputs refused as `blocked`, each once."""
    blocked = {entry["detail"]["output"] for entry in refusals if entry["detail"].get("reason") == "blocked"}
    return [entry for entry in run.errors if blocked.intersection(entry["needed_by"])]


class Policy:
    """The export policy over one run: each output is resolved once, however many cells deliver it.
    `refuse_provisional`: a provisional output that is not accepted is refused (scoring) instead of `null`
    (tables)."""

    def __init__(self, run: Run, *, refuse_provisional: bool) -> None:
        self.run = run
        self.known = outputs(run)
        self.refuse_provisional = refuse_provisional
        self.memo: dict[tuple[str, tuple[int, str] | None], Resolved | Refused] = {}

    def reference(self, output: str, boolean: bool, operator: str, path: str, ref: str) -> dict[str, Any] | None:
        """The error of a cell or placeholder (`ref`, at its `$model`) that names no output of the model
        (`rule_reference_missing`, `expected` `output`), or one of the wrong type (`expr_dimension_mismatch`,
        `operator` the export: a boolean where a number is needed, or the reverse); None when it fits."""
        key = self.known.get(output)
        if key is None:
            return error(RULE_REFERENCE_MISSING, {"path": path, "target": output, "expected": ["output"]}, ref)
        kind = self.run.graph.typed.outputs.get(key)
        if kind is None or (kind == BOOL) == boolean:
            return None
        expected = "bool" if boolean else "number"
        detail = {"path": path, "operator": operator, "operands": [expected, kind.token], "position": 0}
        return error(EXPR_DIMENSION_MISMATCH, detail, ref)

    def resolve(self, output: str, rounding: tuple[int, str] | None) -> Resolved | Refused:
        """The value of `output` as an export delivers it (see the module), or why it cannot be.
        `rounding` is the declared (`scale`, `round`) of a number, and None for a boolean."""
        key = (output, rounding)
        if key not in self.memo:
            self.memo[key] = self._resolve(output, rounding)
        return self.memo[key]

    def _number_member(self, output: str) -> str:
        """`offset` for a threshold of the `fact_offset` form, `value` for any other output."""
        kind, name = output.split(":", 1)
        if kind == "threshold" and self.run.data["thresholds"][name]["form"] == "fact_offset":
            return "offset"
        return "value"

    def _resolve(self, output: str, rounding: tuple[int, str] | None) -> Resolved | Refused:
        run = self.run
        shown = output_result(run, output)
        if shown["status"] == BLOCKED:
            return Refused("blocked")
        document = assemble(run, output)
        if shown["per_account"] or any(node["node"] in STATE_NODES for node in document["nodes"].values()):
            return Refused("per_account")
        accepted = shown["status"] != CONFIRMED  # provisional: delivered only under its acceptance
        if accepted and "acceptance" not in document:
            return Refused("provisional") if self.refuse_provisional else Resolved(output, None, False, None)
        member = self._number_member(output)
        scenarios = shown["variants"] if "variants" in shown else [shown]
        if rounding is None:
            found = {scenario.get(member) for scenario in scenarios}
            if found not in ({True}, {False}):  # an undecided boolean shows null
                return Refused("not_single", {"value": "false..true"})
            return Resolved(output, found.pop(), accepted, document)
        scale, mode = rounding
        numbers: set[int] = set()
        for scenario in scenarios:
            ends = [scenario[member]] if member in scenario else [scenario["range"]["low"], scenario["range"]["high"]]
            numbers.update(scaled(Fraction(end), scale, mode) for end in ends)
        if len(numbers) != 1:
            return Refused("not_single", {"value": f"{min(numbers)}..{max(numbers)}"})
        number = numbers.pop()
        if abs(number) > MAX_SAFE_INTEGER:
            return Refused("out_of_range", {"value": str(number)})
        return Resolved(output, number, accepted, document)
