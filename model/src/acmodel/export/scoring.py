"""`acmodel export scoring` (#424 design 6.2, 6.6; design-py 6.2): the scoring templates of exports.json, filled
with the model's numbers.

The resource side authors each template (`scoring` of exports.json: `id` and `template`, a file relative to the
model folder): gates, terms, `on_unknown`, tie-breaks, the structure of the decision. Its numbers come from the
model as placeholders, `{"$model": "<output id>", "scale": <integer>, "round": "floor" | "ceil" | "half_even"}`,
wherever a number stands (a `weight_milli`, a lookup's `value_milli`, a threshold's `at_least`, ...); the tool
changes nothing else of the template.

- **Reading.** The template is strict JSON (contracts/formats.md, "Reading JSON"); a placeholder holds exactly
  those three members (`model_file_invalid`, format `scoring.template`).
- **Filling** (`fill`). Each placeholder names an output of a number (`rule_reference_missing`,
  `expr_dimension_mismatch`) and becomes its value by the export policy (`export.policy`). A placeholder whose
  output is blocked, per-account, provisional and not accepted, or not one integer refuses the whole document
  (`export_refused`): a scoring document is never partly filled.
- **Writing.** Where `scoring.json` lives, and the validator that knows that path, are not decided yet (367-D1).
  Until they are, a template that fills is refused as `export_refused` (`validator_path_unknown`), and nothing
  is written.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any, Iterator

from acmodel.check.walk import at
from acmodel.codes import EXPORT_REFUSED, MODEL_ID_DUPLICATE
from acmodel.config import Unknowns
from acmodel.core.run import Run, evaluate_model
from acmodel.envelope import Failure, error
from acmodel.export.policy import Policy, Refused, blocking, refused
from acmodel.model.jsonfile import MAX_SAFE_INTEGER, load
from acmodel.model.readers import OUTPUT_ID, ROUND
from acmodel.model.shape import Context, Integer, Object

FORMAT = "scoring.template"
PLACEHOLDER = "$model"
PLACEHOLDER_SHAPE = Object({PLACEHOLDER: OUTPUT_ID, "scale": Integer(1, MAX_SAFE_INTEGER), "round": ROUND})


def placeholders(doc: Any, ref: str = "") -> Iterator[tuple[str, dict[str, Any]]]:
    """(pointer, placeholder) of every object of `doc` that has a `$model` member, in document order; a
    placeholder holds no other placeholder."""
    if isinstance(doc, dict):
        if PLACEHOLDER in doc:
            yield ref, doc
            return
        for key, value in doc.items():
            yield from placeholders(value, at(ref, key))
    elif isinstance(doc, list):
        for index, value in enumerate(doc):
            yield from placeholders(value, at(ref, index))


def _filled(doc: Any, values: dict[str, int], ref: str = "") -> Any:
    """`doc` with each placeholder (by its pointer) replaced by its value."""
    if ref in values:
        return values[ref]
    if isinstance(doc, dict):
        return {key: _filled(value, values, at(ref, key)) for key, value in doc.items()}
    if isinstance(doc, list):
        return [_filled(value, values, at(ref, index)) for index, value in enumerate(doc)]
    return doc


def fill(run: Run, policy: Policy, index: int) -> dict[str, Any]:
    """The template of `scoring[index]` of exports.json with every placeholder resolved, or `Failure` with every
    problem: nothing of it is filled unless all of it is."""
    entry = run.exports.doc["scoring"][index]
    path = run.folder / entry["template"]
    doc = load(path, FORMAT)
    ctx = Context(path, FORMAT)
    found = list(placeholders(doc))
    for ref, placeholder in found:
        PLACEHOLDER_SHAPE.check(placeholder, ref, ctx)
    if ctx.errors:
        raise Failure(*ctx.errors)
    errors, values = [], {}
    for ref, placeholder in found:
        output = placeholder[PLACEHOLDER]
        problem = policy.reference(output, False, "scoring", str(path), at(ref, PLACEHOLDER))
        if problem is not None:
            errors.append(problem)
            continue
        resolved = policy.resolve(output, (placeholder["scale"], placeholder["round"]))
        if isinstance(resolved, Refused):
            errors.append(refused(str(path), output, ref, resolved))
        else:
            values[ref] = resolved.value
    if errors:
        raise Failure(*errors)
    return _filled(doc, values)


def export_scoring(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns = Unknowns()
) -> dict[str, Any]:
    """Evaluate the model for the state and fill every scoring template; refuse every one that does not fill,
    and, until its path and validator are decided, every one that does (`validator_path_unknown`)."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    exports, errors = run.exports, []
    first: dict[str, str] = {}
    for index, entry in enumerate(exports.doc["scoring"]):
        ref = at("", "scoring", index, "id")
        if entry["id"] in first:
            errors.append(error(MODEL_ID_DUPLICATE, {
                "path": exports.path, "namespace": "scoring", "id": entry["id"], "first_path": exports.path,
                "first_ref": first[entry["id"]],
            }, ref))
        first.setdefault(entry["id"], ref)
    if errors:
        raise Failure(*errors)
    policy = Policy(run, refuse_provisional=True)
    for index, entry in enumerate(exports.doc["scoring"]):
        try:
            fill(run, policy, index)
        except Failure as failure:
            errors.extend(failure.errors)
            continue
        detail = {"path": exports.path, "output": "scoring:" + entry["id"], "reason": "validator_path_unknown"}
        errors.append(error(EXPORT_REFUSED, detail, at("", "scoring", index)))
    if errors:  # with the errors that block a blocked output, once
        raise Failure(*errors, *blocking(run, errors))
    return {**run.head, "written": []}
