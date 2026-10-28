"""`acmodel bindings check` and `acmodel bindings apply` (#424 design 6.3; design-py 6.3).

`bindings.json` (`acmodel.bindings.v1`, beside `model.json`) ties literals of the resource repository to
thresholds of `exports.json`: a literal of a `schedule.json` rule or pool (by its `rule_id` or `pool_id` and a
pointer into the rule or the pool) or of a pack file (by a pointer into the file). Its `fixed` entries are
literals that are no quantity, each with a reason and a reference. File names are relative to the repository,
the folder two above the model folder (`<repo>/ours/model`, #424 design 3.1).

The values come from one run of the model for an account state, exactly as `export thresholds` computes
them (`export.thresholds.threshold_entry`), so a binding holds only what the export writes:
- a threshold that the export refuses (blocked, provisional, a constant that read the account state, not
  an integer, out of range) is refused the same way (`export_refused`), with the errors that block it;
- a `fact_offset` threshold whose live offset read the account state is `binding_unsupported`: a fact
  reference holds one fact plus a constant (#386 CF-4), and the tool never writes an account's value as a
  constant.

`check` compares every bound literal with its threshold (`binding_mismatch`), and with `--strict` refuses
every integer literal of a `fact` or `resource_projection` predicate of a `schedule.json` rule that is neither
bound nor fixed (`binding_unbound_literal`). `apply` writes each bound literal that differs, changing only the
literal's bytes; it refuses everything `check` refuses but a differing value, and then writes nothing.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Iterator

from acmodel.bindings.scanner import MISSING, parent, resolve, spans, tokens
from acmodel.bindings.targets import (
    Target,
    fact_predicate,
    fact_reference,
    integer,
    predicate_literals,
    read_target,
    shown,
)
from acmodel.check.files import File
from acmodel.check.walk import at
from acmodel.codes import (
    BINDING_MISMATCH,
    BINDING_UNBOUND_LITERAL,
    BINDING_UNSUPPORTED,
    EXPORT_REFUSED,
    EXPORT_WRITE_FAILED,
    MODEL_ID_DUPLICATE,
    RULE_REFERENCE_MISSING,
)
from acmodel.config import Unknowns
from acmodel.core.provenance import STATE_NODES, assemble
from acmodel.core.run import Run, evaluate_model
from acmodel.envelope import Failure, error
from acmodel.export.thresholds import threshold_entry
from acmodel.model import read_bindings
from acmodel.model.readers import SCHEDULE_FILE
from acmodel.model.shape import json_type

BINDINGS = "bindings.json"


def _raise(errors: list[dict[str, Any]]) -> None:
    if errors:
        raise Failure(*errors)


def _entries(bindings: File) -> Iterator[tuple[str, dict[str, Any]]]:
    """(pointer in bindings.json, entry) of every binding, then of every fixed literal."""
    for member in ("bindings", "fixed"):
        for index, entry in enumerate(bindings.doc[member]):
            yield at("", member, index), entry


def _load(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns
) -> tuple[Run, File]:
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    bindings_path = run.folder / BINDINGS
    return run, File(str(bindings_path), read_bindings(bindings_path))


def _references(run: Run, bindings: File) -> list[dict[str, Any]]:
    """Each binding names a threshold of exports.json, and no literal is named twice."""
    thresholds = {threshold["id"] for threshold in run.exports.doc["thresholds"]}
    errors, seen = [], {}
    for ref, entry in _entries(bindings):
        if "threshold" in entry and entry["threshold"] not in thresholds:
            detail = {"path": bindings.path, "target": entry["threshold"], "expected": ["threshold"]}
            errors.append(error(RULE_REFERENCE_MISSING, detail, at(ref, "threshold")))
        key = (entry["file"], entry.get("rule_id"), entry.get("pool_id"), entry["path"])
        if key in seen:
            name = ":".join(part for part in key if part is not None)
            detail = {"path": bindings.path, "namespace": "literal", "id": name, "first_path": bindings.path,
                      "first_ref": seen[key]}
            errors.append(error(MODEL_ID_DUPLICATE, detail, ref))
        else:
            seen[key] = ref
    return errors


def _targets(run: Run, bindings: File, strict: bool) -> tuple[Path, dict[str, Target]]:
    """The repository and every file the entries name (and `schedule.json` under `--strict`), each read once."""
    repo = run.folder.parent.parent
    names = dict.fromkeys(entry["file"] for _, entry in _entries(bindings))
    if strict:
        names.setdefault(SCHEDULE_FILE)
    targets, errors = {}, []
    for name in names:
        try:
            targets[name] = read_target(repo, name)
        except Failure as failure:
            errors.extend(failure.errors)
    _raise(errors)
    return repo, targets


def _values(run: Run, bindings: File) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]]]:
    """The thresholds.json entry of each bound threshold, as `export thresholds` writes it, and the refusals
    of the others, followed by the errors that block them."""
    thresholds = run.exports.doc["thresholds"]
    index = {threshold["id"]: (position, threshold) for position, threshold in enumerate(thresholds)}
    values, refusals = {}, []
    for name in dict.fromkeys(entry["threshold"] for entry in bindings.doc["bindings"]):
        position, threshold = index[name]
        entry = threshold_entry(run, position, threshold, {})
        if entry.get("code") != EXPORT_REFUSED:
            values[name] = entry
        elif entry["detail"]["reason"] == "per_account" and threshold["form"] == "fact_offset":
            output = "threshold:" + name
            nodes = assemble(run, output)["nodes"]
            reads = sorted(node for node, value in nodes.items() if value["node"] in STATE_NODES)
            detail = {"path": run.exports.path, "output": output, "reads": reads}
            refusals.append(error(BINDING_UNSUPPORTED, detail, entry["ref"]))
        else:
            refusals.append(entry)
    blocked = {
        entry["detail"]["output"] for entry in refusals
        if entry["code"] == EXPORT_REFUSED and entry["detail"]["reason"] == "blocked"
    }
    return values, refusals + [entry for entry in run.errors if blocked.intersection(entry["needed_by"])]


class _Literal:
    """One entry of bindings.json and the literal it names in its file."""

    def __init__(self, targets: dict[str, Target], ref: str, entry: dict[str, Any]) -> None:
        self.entry_ref, self.entry = ref, entry
        self.target = target = targets[entry["file"]]
        self.holder = ""  # schedule.json: the list that holds the entry's rule or pool
        if target.rules is None or target.pools is None:
            self.ref: str | None = entry["path"]
        else:
            # The reader lets exactly one of rule_id and pool_id name a rule or a pool of schedule.json.
            self.holder, items, item = (
                ("rules", target.rules, entry["rule_id"]) if "rule_id" in entry
                else ("pools", target.pools, entry["pool_id"])
            )
            self.ref = at("", self.holder, items[item]) + entry["path"] if item in items else None
        self.value = MISSING if self.ref is None else resolve(target.doc, self.ref)

    def mismatch(self, reason: str, ref: str | None = None, **values: Any) -> dict[str, Any]:
        """`binding_mismatch` at `ref`, by default the literal's pointer (or /rules or /pools, without the rule
        or the pool)."""
        detail = {"path": str(self.target.path), "entry": self.entry_ref, "reason": reason}
        detail.update({key: self.entry[key] for key in ("threshold", "rule_id", "pool_id") if key in self.entry})
        if ref is None:
            ref = at("", self.holder) if self.ref is None else self.ref
        return error(BINDING_MISMATCH, {**detail, **values}, ref)

    def missing(self) -> dict[str, Any] | None:
        """`literal_missing` when the entry names no integer literal."""
        if integer(self.value):
            return None
        return self.mismatch("literal_missing", **({} if self.value is MISSING else {"found": json_type(self.value)}))

    def compare(self, value: dict[str, Any]) -> tuple[list[dict[str, Any]], int | None]:
        """The problems of a bound literal other than its value, and the value it must hold (None when the
        form is wrong). A `fact_offset` literal is the offset of a fact reference in a `fact` predicate whose
        two fact keys are the threshold's; a constant is any integer literal but such an offset (the offset of
        a fact reference elsewhere, in a `resource_projection` predicate or a pool's capacity, is one)."""
        doc, ref = self.target.doc, self.ref
        at_offset = (
            tokens(ref)[-2:] == ["value", "offset"] and fact_reference(resolve(doc, parent(ref)))
            and fact_predicate(resolve(doc, parent(ref, 2)))
        )
        if value["form"] == "constant":
            return ([self.mismatch("form")], None) if at_offset else ([], value["value"])
        if not at_offset:
            return [self.mismatch("form")], None
        problems = []
        keys = (("fact_key", "fact", parent(ref, 2)), ("reference_fact", "reference_fact", parent(ref)))
        for reason, member, holder in keys:  # the predicate's fact key, then its reference's
            found = resolve(doc, holder).get("fact_key")
            if found != value[member]:
                key_ref = at(holder, "fact_key")
                problems.append(self.mismatch(reason, key_ref, expected=value[member], found=shown(found)))
        return problems, value["offset"]

    def describe(self) -> dict[str, Any]:
        out = {"entry": self.entry_ref}
        keys = ("threshold", "file", "rule_id", "pool_id", "path")
        out.update({key: self.entry[key] for key in keys if key in self.entry})
        return out


def _unbound(schedule: Target, bindings: File) -> tuple[int, list[dict[str, Any]]]:
    """How many literals the `fact` and `resource_projection` predicates of the rules of schedule.json compare
    with, and an error for each that is neither bound nor fixed. A pool's literals are not scanned."""
    named = {
        (entry["rule_id"], entry["path"]) for _, entry in _entries(bindings)
        if entry["file"] == SCHEDULE_FILE and "rule_id" in entry
    }
    count, errors = 0, []
    for rule_id, index in schedule.rules.items():
        for ref, literal in predicate_literals(schedule.doc["rules"][index]):
            count += 1
            if (rule_id, ref) not in named:
                detail = {"path": str(schedule.path), "rule_id": rule_id, "pointer": ref, "literal": str(literal)}
                errors.append(error(BINDING_UNBOUND_LITERAL, detail, at("", "rules", index) + ref))
    return count, errors


def check_bindings(
    path: Path | str,
    state_path: Path | str,
    *,
    datasets: Path | str,
    max_bits: int,
    strict: bool,
    unknowns: Unknowns = Unknowns(),
) -> dict[str, Any]:
    """`acmodel bindings check [--strict]`: every bound literal equals its exported threshold; with `strict`,
    no literal of a `fact` or `resource_projection` predicate of a schedule.json rule is left unbound."""
    run, bindings = _load(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    _raise(_references(run, bindings))
    repo, targets = _targets(run, bindings, strict)
    values, errors = _values(run, bindings)
    bound, fixed = [], []
    for ref, entry in _entries(bindings):
        literal = _Literal(targets, ref, entry)
        missing = literal.missing()
        if missing is not None:
            errors.append(missing)
        elif "threshold" not in entry:
            reason = {"reason": entry["reason"], "ref": entry["ref"]}
            fixed.append({**literal.describe(), "literal": literal.value, **reason})
        elif entry["threshold"] in values:
            value = values[entry["threshold"]]
            problems, expected = literal.compare(value)
            errors.extend(problems)
            if expected is not None and literal.value != expected:
                errors.append(literal.mismatch("value", expected=str(expected), found=str(literal.value)))
            bound.append({**literal.describe(), "form": value["form"], "literal": literal.value})
    data = {**run.head, "repo": str(repo), "strict": strict, "bindings": bound, "fixed": fixed}
    if strict:
        data["literals"], unbound = _unbound(targets[SCHEDULE_FILE], bindings)
        errors.extend(unbound)
    _raise(errors)
    return data


def _splice(target: Target, edits: list[tuple[str, int]]) -> bytes:
    """The file's bytes with each literal at its pointer replaced, and every other byte kept."""
    found, raw = spans(target.raw), target.raw
    for ref, value in sorted(edits, key=lambda edit: found[edit[0]][0], reverse=True):
        start, end = found[ref]
        if json.loads(raw[start:end]) != resolve(target.doc, ref):
            raise ValueError(f"the scanner's span of {ref} does not hold its literal")
        raw = raw[:start] + str(value).encode("ascii") + raw[end:]
    doc = json.loads(raw)
    if any(resolve(doc, ref) != value for ref, value in edits):
        raise ValueError("a spliced literal does not read back")
    return raw


def _write(target: Target, raw: bytes) -> None:
    try:
        target.path.write_bytes(raw)
    except OSError as exc:
        detail = {"path": str(target.path), "exception_type": type(exc).__name__, "raw_text": str(exc)}
        raise Failure(error(EXPORT_WRITE_FAILED, detail)) from exc


def apply_bindings(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns = Unknowns()
) -> dict[str, Any]:
    """`acmodel bindings apply`: write each bound literal that differs from its exported threshold, changing
    only its bytes. Anything else `check` refuses is refused, and then nothing is written."""
    run, bindings = _load(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    _raise(_references(run, bindings))
    repo, targets = _targets(run, bindings, False)
    values, errors = _values(run, bindings)
    edits: dict[str, list[tuple[str, int]]] = {}
    changed = []
    for ref, entry in _entries(bindings):
        literal = _Literal(targets, ref, entry)
        missing = literal.missing()
        if missing is not None:
            errors.append(missing)
            continue
        if entry.get("threshold") not in values:  # a fixed literal, or a refused threshold
            continue
        problems, expected = literal.compare(values[entry["threshold"]])
        errors.extend(problems)
        if not problems and literal.value != expected:
            edits.setdefault(entry["file"], []).append((literal.ref, expected))
            changed.append({**literal.describe(), "from": literal.value, "to": expected})
    _raise(errors)
    written = []
    for name, file_edits in edits.items():
        target = targets[name]
        _write(target, _splice(target, file_edits))
        written.append(str(target.path))
    return {**run.head, "repo": str(repo), "changed": changed, "written": sorted(written)}
