"""`acmodel bindings check` and `acmodel bindings apply` (#424 design 6.3; design-py 6.3; #464 W1).

`bindings.json` (`acmodel.bindings.v1`, beside `model.json`) ties literals of the resource repository to
thresholds and values of `exports.json`: a literal of a `schedule.json` rule or pool (by its `rule_id` or `pool_id` and a
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

**Values** (#464 W1). A binding of a value (`value`, a value of exports.json) names the fixed value of a `schedule.json`
rule (`/fixed_value_milli`). Its number is computed as `export values` computes it (`export.values.evaluate`), from the
same run and the delivered prices: a value that export leaves pending is refused (`export_refused`, `pending`), and one it
refuses is refused the same way. The rule's fixed value must be that number and say who wrote it,
`fixed_value_origin: "model"` (`binding_mismatch` `value`, `origin_missing`); the rule's `consumes` must name exactly the
value's `cost_paid_by_pool` (`value_consumes_mismatch`); and `schedule.json`'s `requires` must hold
`benefit_fixed_value` (`requires_word_missing`; the tool never adds it). `apply` writes the number and the origin, and
inserts either member a rule lacks (the fixed value after `package_id`, the origin after the fixed value; `splice`), each
in the style of the member it follows (`insert_style_unknown` when there is none to copy). It never removes a member.

**The origin guard**, always on: every rule of `schedule.json` whose `fixed_value_origin` is `"model"`, and every pool
whose `valuation_origin` is, must be named by a value binding (`origin_unbound`): only the model's export writes `model`.

`check` compares every bound literal with its threshold or value (`binding_mismatch`), and with `--strict` refuses
every integer literal of a `fact` or `resource_projection` predicate of a `schedule.json` rule, and every
quantity of a `schedule.json` pool (#424 ACM-POOL), that is neither bound nor fixed (`binding_unbound_literal`).
`apply` writes each bound literal that differs, changing only the literal's bytes, and for a value the origin and the
members it inserts; it refuses everything `check` refuses but a differing value or origin, and then writes nothing. It
writes each file as `<file>.part` first, reads the file again, and renames the part over it only when the file is still
the bytes it read (`file_changed` otherwise, and the parts are removed). Neither binds a pool that is a slot resource of
the model (`binding_mismatch`, `slot`; #424 ACM-PRICE 3.1): a slot is no Runtime pool.

`check` also reports the coverage of the prices (#424 ACM-PRICE PRICE-4a; addendum 2.1 §3.1) in `data.coverage`: what
`export prices` delivered in `out/prices.json`, read back, as the repository's schedule.json meets it (each pool by its
class, each rule that declares effects by the worst class of its pools, the counts). It computes no steady state: that
is the export's. Without the file it is null; a file of other versions of the model is the warning `prices_stale`. And
whether `out/values.json` is of the current inputs (`data.values`; the warning `values_stale` otherwise, #464 W1).
"""

from __future__ import annotations

import copy
import json
import os
from fractions import Fraction
from pathlib import Path
from typing import Any, Iterator

from acmodel.bindings import splice
from acmodel.bindings.scanner import MISSING, parent, resolve, tokens
from acmodel.bindings.targets import (
    POLICY_FORMAT,
    SCHEDULE_FORMAT,
    Target,
    fact_predicate,
    fact_reference,
    integer,
    pool_literals,
    predicate_literals,
    read_target,
    shown,
)
from acmodel.cache.keys import digest
from acmodel.check.files import File
from acmodel.check.walk import at
from acmodel.codes import (
    BINDING_MISMATCH,
    BINDING_UNBOUND_LITERAL,
    BINDING_UNSUPPORTED,
    EXPORT_REFUSED,
    EXPORT_WRITE_FAILED,
    FILE_CHANGED,
    INSERT_STYLE_UNKNOWN,
    MODEL_ID_DUPLICATE,
    ORIGIN_MISSING,
    ORIGIN_UNBOUND,
    PRICES_STALE,
    REQUIRES_WORD_MISSING,
    RULE_REFERENCE_MISSING,
    VALUE_CONSUMES_MISMATCH,
    VALUES_STALE,
)
from acmodel.config import Unknowns
from acmodel.core.flow import slots
from acmodel.core.provenance import STATE_NODES, assemble
from acmodel.core.run import Run, evaluate_model
from acmodel.core.versions import versions
from acmodel.envelope import Failure, error
from acmodel.export import values as value_export
from acmodel.export.thresholds import threshold_entry
from acmodel.model import read_bindings, read_prices, read_values
from acmodel.model.jsonfile import read, refusal
from acmodel.model.readers import SCHEDULE_FILE
from acmodel.model.shape import json_type

BINDINGS = "bindings.json"
PRICES = "out/prices.json"  # what `export prices` writes (`export.prices`)
# #464 W1: a value's members in a rule of schedule.json, and the origin of a pool's valuation.
MILLI, ORIGIN, ANCHOR, POOL_ORIGIN = "fixed_value_milli", "fixed_value_origin", "package_id", "valuation_origin"
MODEL = "model"
REQUIRED_WORD = "benefit_fixed_value"  # what schedule.json's `requires` must hold for a fixed value
PART = ".part"  # `apply` writes `<file>.part`, then renames it over the file
RATIO_LOW, RATIO_HIGH = Fraction(1, 2), Fraction(2)  # a changed value outside these times its old one is flagged


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
    """Each binding names a threshold or a value of exports.json, and no literal is named twice."""
    thresholds = {threshold["id"] for threshold in run.exports.doc["thresholds"]}
    values = {value["id"] for value in run.exports.doc.get("values", [])}
    errors, seen = [], {}
    for ref, entry in _entries(bindings):
        for member, known in (("threshold", thresholds), ("value", values)):
            if member in entry and entry[member] not in known:
                detail = {"path": bindings.path, "target": entry[member], "expected": [member]}
                errors.append(error(RULE_REFERENCE_MISSING, detail, at(ref, member)))
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
    """The repository and every file the entries name, and `schedule.json` when it is there (the origin guard reads it)
    or under `--strict`, each read once."""
    repo = run.folder.parent.parent
    names = dict.fromkeys(entry["file"] for _, entry in _entries(bindings))
    if strict or (repo / SCHEDULE_FILE).exists():
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
    for name in dict.fromkeys(entry["threshold"] for entry in bindings.doc["bindings"] if "threshold" in entry):
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


def _valued(run: Run, bindings: File) -> tuple[dict[str, value_export.Valued], list[dict[str, Any]]]:
    """Each bound value as `export values` computes it (#464 W1), and the refusals of the others: a value it refuses, and
    one it leaves pending (`export_refused`, `pending`, one error per reason, `value` the reason as `<reason>:<item>`).
    Without a value binding nothing of the values is read; with one, the prices must be current (`prices_stale`) and every
    value of exports.json must name what it names."""
    wanted = dict.fromkeys(entry["value"] for entry in bindings.doc["bindings"] if "value" in entry)
    if not wanted:
        return {}, []
    prices = value_export.current_prices(run)
    _raise(value_export.problems(run))
    found = value_export.evaluate(run, prices, set(wanted))
    values, errors = {}, []
    for name in wanted:
        valued = found[name]
        if valued.refusals:
            errors.extend(valued.refusals)
        elif valued.status == value_export.PENDING:
            errors.extend(
                error(EXPORT_REFUSED, {"path": run.exports.path, "output": valued.output, "reason": "pending",
                                       "value": token}, valued.ref)
                for token in valued.tokens()
            )
        else:
            values[name] = valued
    return values, errors


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
        detail.update({key: self.entry[key] for key in ("threshold", "value", "rule_id", "pool_id") if key in self.entry})
        if ref is None:
            ref = at("", self.holder) if self.ref is None else self.ref
        return error(BINDING_MISMATCH, {**detail, **values}, ref)

    def slot(self, slots: frozenset[str]) -> dict[str, Any] | None:
        """`slot` when the entry names a pool of `schedule.json` that is a slot resource of the model (#424
        ACM-PRICE 3.1, `core.flow.slots`): a slot is no Runtime pool, and its price is a diagnosis only, so nothing
        binds it. At the pool, or at /pools when no pool has that id."""
        pool = self.entry.get("pool_id")
        if pool not in slots:
            return None
        pools = self.target.pools
        return self.mismatch("slot", at("", "pools", pools[pool]) if pool in pools else at("", "pools"))

    def missing(self) -> dict[str, Any] | None:
        """`literal_missing` when the entry names no integer literal."""
        if integer(self.value):
            return None
        return self.mismatch("literal_missing", **({} if self.value is MISSING else {"found": json_type(self.value)}))

    def compare(self, value: dict[str, Any]) -> tuple[list[dict[str, Any]], int | None]:
        """The problems of a bound literal other than its value, and the value it must hold (None when the
        form is wrong). A `fact_offset` literal is the offset of a fact reference in a `fact` predicate whose
        two fact keys are the threshold's; a constant is any integer literal but such an offset (the offset of
        a fact reference elsewhere, in a `resource_projection` predicate or a pool's capacity or line, is one)."""
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
        keys = ("threshold", "value", "file", "rule_id", "pool_id", "path")
        out.update({key: self.entry[key] for key in keys if key in self.entry})
        return out


class _Layouts:
    """Each file's layout (`splice.Layout`), found once and only for a file that is edited."""

    def __init__(self) -> None:
        self.found: dict[str, splice.Layout] = {}

    def of(self, target: Target) -> splice.Layout:
        if target.name not in self.found:
            self.found[target.name] = splice.Layout(target.raw)
        return self.found[target.name]


class _ValuePlan:
    """What a value binding needs of its rule (#464 W1): the problems both `check` and `apply` refuse; the differences
    `check` reports and `apply` writes (the fixed value, the origin); the edits that write them, and the members they
    insert."""

    def __init__(self, literal: _Literal, valued: value_export.Valued, layouts: _Layouts) -> None:
        self.literal, self.valued = literal, valued
        self.problems: list[dict[str, Any]] = []
        self.differences: list[dict[str, Any]] = []
        self.edits: list[splice.Edit] = []
        self.inserted: list[str] = []
        self.rule_ref: str | None = None if literal.ref is None else parent(literal.ref)
        self.origin: Any = MISSING
        if self.rule_ref is None:  # no rule of that id
            self.problems.append(literal.mismatch("literal_missing"))
            return
        self.rule = rule = resolve(literal.target.doc, self.rule_ref)
        self.origin = rule.get(ORIGIN, MISSING)
        self._compare()
        if not self.problems:
            self._edits(layouts.of(literal.target))

    def _detail(self, **values: Any) -> dict[str, Any]:
        literal = self.literal
        return {"path": str(literal.target.path), "entry": literal.entry_ref, "value": self.valued.id,
                "rule_id": literal.entry["rule_id"], **values}

    def _compare(self) -> None:
        literal, valued, rule, rule_ref = self.literal, self.valued, self.rule, self.rule_ref
        value = literal.value
        if value is not MISSING and not integer(value):
            self.problems.append(literal.mismatch("literal_missing", found=json_type(value)))
        consumes = rule.get("consumes")
        found = sorted({
            entry["pool_id"] for entry in (consumes if isinstance(consumes, list) else [])
            if isinstance(entry, dict) and isinstance(entry.get("pool_id"), str)
        })
        expected = sorted(set(valued.definition.get("cost_paid_by_pool", [])))
        if found != expected:
            where = at(rule_ref, "consumes") if "consumes" in rule else rule_ref
            self.problems.append(error(VALUE_CONSUMES_MISMATCH, self._detail(expected=expected, found=found), where))
        if value is MISSING or (integer(value) and value != valued.milli):
            found_value = {} if value is MISSING else {"found": str(value)}
            ref = literal.ref if value is not MISSING else rule_ref
            self.differences.append(literal.mismatch("value", ref, expected=str(valued.milli), **found_value))
        if self.origin != MODEL:
            where = rule_ref if self.origin is MISSING else at(rule_ref, ORIGIN)
            found_origin = {} if self.origin is MISSING else {"found": shown(self.origin)}
            self.differences.append(error(ORIGIN_MISSING, self._detail(**found_origin), where))

    def _edits(self, layout: splice.Layout) -> None:
        literal, rule_ref, value, milli = self.literal, self.rule_ref, self.literal.value, self.valued.milli
        members: list[tuple[str, bytes]] = []
        number = str(milli).encode("ascii")
        if value is MISSING:
            members.append((MILLI, number))
        elif value != milli:
            self.edits.append(_literal_edit(layout, literal.target, literal.ref, milli))
        quoted = json.dumps(MODEL).encode("ascii")
        if self.origin is MISSING:
            members.append((ORIGIN, quoted))
        elif self.origin != MODEL:
            self.edits.append(layout.replace(at(rule_ref, ORIGIN), quoted))
        if not members:
            return
        anchor = ANCHOR if value is MISSING else MILLI
        edit = layout.insert_after(at(rule_ref, anchor), members) if anchor in self.rule else None
        if edit is None:
            detail = self._detail(anchor=anchor, member=members[0][0])
            self.problems.append(error(INSERT_STYLE_UNKNOWN, detail, rule_ref))
            self.edits = []
            return
        self.edits.append(edit)
        self.inserted = [key for key, _ in members]

    def expect(self, doc: Any) -> None:
        """The document the edits make: the rule with its fixed value and its origin, each in its place or right after
        the member it follows."""
        rule = splice.placed(resolve(doc, self.rule_ref), ANCHOR, MILLI, self.valued.milli)
        splice.assign(doc, self.rule_ref, splice.placed(rule, MILLI, ORIGIN, MODEL))

    def described(self) -> dict[str, Any]:
        """What `check` shows of the binding: the literal and the origin as they are (null when absent), the value as
        `export values` computes it, its status, the acceptances it rests on and what it leaves out."""
        literal, valued = self.literal, self.valued
        entry = valued.entry()
        return {
            **literal.describe(),
            "literal": None if literal.value is MISSING else literal.value,
            "origin": None if self.origin is MISSING else self.origin,
            "milli": valued.milli,
            "status": entry["status"],
            "acceptances": entry["acceptances"],
            "partial": entry["partial"],
            "exclude": entry["exclude"],
        }

    def changed(self) -> dict[str, Any]:
        """What `apply` shows of a change: the fixed value from and to (`from` null when inserted), their ratio (null
        when there was none) flagged outside one half to two, the origin from and to, and the members inserted."""
        value, milli = self.literal.value, self.valued.milli
        ratio = None if value is MISSING or value == 0 else Fraction(milli, value)
        return {
            **self.literal.describe(),
            "from": None if value is MISSING else value,
            "to": milli,
            "ratio": None if ratio is None else str(ratio),
            "ratio_flagged": ratio is not None and not RATIO_LOW <= ratio <= RATIO_HIGH,
            "origin": {"from": None if self.origin is MISSING else self.origin, "to": MODEL},
            "inserted": self.inserted,
        }


def _literal_edit(layout: splice.Layout, target: Target, ref: str, value: int) -> splice.Edit:
    """The literal at `ref` written as `value`; the scanner's span must hold the literal the reader read."""
    start, end = layout.values[ref]
    if json.loads(target.raw[start:end]) != resolve(target.doc, ref):
        raise ValueError(f"the scanner's span of {ref} does not hold its literal")
    return splice.Edit(start, end, str(value).encode("ascii"))


def _requires(targets: dict[str, Target], bindings: File) -> list[dict[str, Any]]:
    """`requires_word_missing` when a value is bound and schedule.json's `requires` lacks `benefit_fixed_value`, the
    word the Runtime reads a fixed value under: the tool never adds it."""
    schedule = targets.get(SCHEDULE_FILE)
    if schedule is None or not any("value" in entry for entry in bindings.doc["bindings"]):
        return []
    requires = schedule.doc.get("requires")
    if isinstance(requires, list) and REQUIRED_WORD in requires:
        return []
    detail = {"path": str(schedule.path), "word": REQUIRED_WORD}
    return [error(REQUIRES_WORD_MISSING, detail, "/requires" if "requires" in schedule.doc else "")]


def _unbound_origins(targets: dict[str, Target], bindings: File) -> list[dict[str, Any]]:
    """The origin guard (#464 W1), always on: `origin_unbound` for each rule of schedule.json whose `fixed_value_origin`
    is `"model"` and that no value binding names, then for each pool whose `valuation_origin` is (no binding names a
    pool's valuation yet)."""
    schedule = targets.get(SCHEDULE_FILE)
    if schedule is None:
        return []
    bound = {entry["rule_id"] for entry in bindings.doc["bindings"] if "value" in entry}
    errors = []
    for holder, member, items, origin in (
        ("rules", "rule_id", schedule.rules, ORIGIN), ("pools", "pool_id", schedule.pools, POOL_ORIGIN)
    ):
        for item_id, index in items.items():
            item = schedule.doc[holder][index]
            if item.get(origin) == MODEL and not (holder == "rules" and item_id in bound):
                detail = {"path": str(schedule.path), member: item_id, "member": origin}
                errors.append(error(ORIGIN_UNBOUND, detail, at("", holder, index, origin)))
    return errors


def _unbound(schedule: Target, bindings: File) -> tuple[int, list[dict[str, Any]]]:
    """How many literals `--strict` scans in schedule.json, and an error for each that is neither bound nor fixed:
    those the `fact` and `resource_projection` predicates of its rules compare with, then the quantities of its
    pools (#424 ACM-POOL: capacity, regeneration amount and period; RL-M1: the stop line and the hard line)."""
    named = {
        ("rules" if "rule_id" in entry else "pools", entry.get("rule_id", entry.get("pool_id")), entry["path"])
        for _, entry in _entries(bindings) if entry["file"] == SCHEDULE_FILE
    }
    count, errors = 0, []
    for holder, member, items, literals in (
        ("rules", "rule_id", schedule.rules, predicate_literals),
        ("pools", "pool_id", schedule.pools, pool_literals),
    ):
        for item_id, index in items.items():
            for ref, literal in literals(schedule.doc[holder][index]):
                count += 1
                if (holder, item_id, ref) not in named:
                    detail = {"path": str(schedule.path), member: item_id, "pointer": ref, "literal": str(literal)}
                    errors.append(error(BINDING_UNBOUND_LITERAL, detail, at("", holder, index) + ref))
    return count, errors


# The four classes of a pool (#424 ACM-PRICE addendum 2.1 §3.1), from the best to the worst: a rule takes the worst of
# the pools it touches.
CLASS_ORDER = ("covered", "verified_zero", "structural_gap", "unknown")


def _effects(rule: dict[str, Any]) -> list[str]:
    """The pools a schedule.json rule declares that it produces or consumes (`produces`, `consumes`: lists of entries
    with a `pool_id`), each once, in order. What is not of that shape is the validator's to refuse."""
    found: dict[str, None] = {}
    for member in ("produces", "consumes"):
        entries = rule.get(member)
        for entry in entries if isinstance(entries, list) else ():
            if isinstance(entry, dict) and isinstance(entry.get("pool_id"), str):
                found[entry["pool_id"]] = None
    return list(found)


def _coverage(run: Run, repo: Path, targets: dict[str, Target]) -> tuple[dict[str, Any] | None, list[dict[str, Any]]]:
    """The coverage `export prices` delivered (`out/prices.json`, #424 ACM-PRICE PRICE-4a; addendum 2.1 §3.1), as the
    repository's schedule.json meets it: each of its pools (a pool the model lacks, or a slot, is unknown) and each of
    its rules that declares effects (the worst class of the pools it touches), with the counts by class. None when no
    prices were exported. A prices.json of other versions of the model is shown with `current` false and the warning
    `prices_stale`."""
    path = run.folder / PRICES
    if not path.exists():
        return None, []
    doc = read_prices(path)
    if doc["model_id"] != run.head["model_id"]:
        raise Failure(refusal(
            "value_invalid", path, doc["schema_version"], "/model_id", expected=[run.head["model_id"]],
            literal=doc["model_id"],
        ))
    model = versions(run.checker, run.folder)
    stale = [member for member in ("rules", "data", "tool") if doc["versions"][member] != model[member]]
    warnings = [error(PRICES_STALE, {"path": str(path), "members": stale}, "/versions")] if stale else []
    by_pool = {entry["pool"]: entry for entry in doc["prices"]}
    slots = set(doc["slots"])
    schedule = targets.get(SCHEDULE_FILE)
    if schedule is None and (repo / SCHEDULE_FILE).exists():
        schedule = read_target(repo, SCHEDULE_FILE)

    def classed(pool: str) -> dict[str, Any]:
        entry = by_pool.get(pool)
        if entry is None:  # the model lacks it (or it is a slot, which is no pool): unknown, never 0
            reason = "slot" if pool in slots else "not_in_model"
            return {"class": "unknown", "status": "provisional", "reasons": [reason], "price": "pending",
                    "pending": ["unknown"]}
        return {key: entry[key] for key in ("class", "status", "reasons", "price", "pending")}

    pools: dict[str, Any] = {}
    rules: dict[str, Any] = {}
    if schedule is not None:
        for pool in schedule.pools:
            pools[pool] = classed(pool)
        for rule_id, index in schedule.rules.items():
            touched = _effects(schedule.doc["rules"][index])
            if not touched:
                continue
            found = [classed(pool) for pool in touched]
            rules[rule_id] = {
                "class": max((entry["class"] for entry in found), key=CLASS_ORDER.index),
                "status": "provisional" if any(entry["status"] != "confirmed" for entry in found) else "confirmed",
                "pools": touched,
                "pending": [pool for pool, entry in zip(touched, found) if entry["price"] == "pending"],
            }
    counts = {
        "pools": {kind: sum(1 for entry in pools.values() if entry["class"] == kind) for kind in CLASS_ORDER},
        "rules": {kind: sum(1 for entry in rules.values() if entry["class"] == kind) for kind in CLASS_ORDER},
        "model": doc["counts"],
    }
    return {"file": str(path), "current": not stale, "pools": pools, "rules": rules, "counts": counts}, warnings


def _values_file(run: Run) -> tuple[dict[str, Any] | None, list[dict[str, Any]]]:
    """Whether `out/values.json` (#464 W1) is of the current inputs: the model's versions (`rules`, `data`, `tool`), the
    bytes of `out/prices.json` (`prices`) and of exports.json (`exports`). None when there is no such file; one that is
    not current is the warning `values_stale`, with the members that differ."""
    path = run.folder / value_export.VALUES
    if not path.exists():
        return None, []
    doc = read_values(path)
    if doc["model_id"] != run.head["model_id"]:
        raise Failure(refusal(
            "value_invalid", path, doc["schema_version"], "/model_id", expected=[run.head["model_id"]],
            literal=doc["model_id"],
        ))
    model = versions(run.checker, run.folder)
    stale = [member for member in ("rules", "data", "tool") if doc["versions"][member] != model[member]]
    prices = run.folder / PRICES
    prices_sha = digest(read(prices, "acmodel.prices.v2")) if prices.exists() else None
    if doc["prices"]["sha256"] != prices_sha:
        stale.append("prices")
    if doc["exports"]["sha256"] != value_export.exports_sha(run):
        stale.append("exports")
    warnings = [error(VALUES_STALE, {"path": str(path), "members": stale})] if stale else []
    return {"file": str(path), "current": not stale, "stale": stale}, warnings


def check_bindings(
    path: Path | str,
    state_path: Path | str,
    *,
    datasets: Path | str,
    max_bits: int,
    strict: bool,
    unknowns: Unknowns = Unknowns(),
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """`acmodel bindings check [--strict]`: every bound literal equals its exported threshold or value (with its origin);
    the origin guard; with `strict`, no literal of a `fact` or `resource_projection` predicate of a schedule.json rule,
    and no quantity of one of its pools, is left unbound. `data.coverage` is what `export prices` delivered, as
    schedule.json meets it (`_coverage`), and `data.values` whether out/values.json is current (`_values_file`); the
    warnings are their `prices_stale` and `values_stale`."""
    run, bindings = _load(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    _raise(_references(run, bindings))
    repo, targets = _targets(run, bindings, strict)
    values, errors = _values(run, bindings)
    valued, refused = _valued(run, bindings)
    errors.extend(refused)
    errors.extend(_requires(targets, bindings))
    bound, fixed = [], []
    slot_ids = slots(run.checker, run.graph.typed)
    layouts = _Layouts()
    for ref, entry in _entries(bindings):
        literal = _Literal(targets, ref, entry)
        if "value" in entry:
            if entry["value"] in valued:
                plan = _ValuePlan(literal, valued[entry["value"]], layouts)
                errors.extend(plan.problems + plan.differences)
                if not plan.problems:
                    bound.append(plan.described())
            continue
        missing = literal.slot(slot_ids) or literal.missing()
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
    errors.extend(_unbound_origins(targets, bindings))
    data = {**run.head, "repo": str(repo), "strict": strict, "bindings": bound, "fixed": fixed}
    if strict:
        data["literals"], unbound = _unbound(targets[SCHEDULE_FILE], bindings)
        errors.extend(unbound)
    _raise(errors)
    data["coverage"], warnings = _coverage(run, repo, targets)
    data["values"], stale = _values_file(run)
    return data, warnings + stale


def _failed(path: Path, exc: OSError) -> dict[str, Any]:
    return error(EXPORT_WRITE_FAILED, {"path": str(path), "exception_type": type(exc).__name__, "raw_text": str(exc)})


def _part(target: Target) -> Path:
    return target.path.with_name(target.path.name + PART)


def _stage(target: Target, raw: bytes) -> Path:
    """The new bytes of a file written beside it, as `<file>.part`."""
    part = _part(target)
    part.write_bytes(raw)
    return part


def _current(path: Path) -> bytes:
    """A file's bytes as they are now, read again just before its part is renamed over it."""
    return path.read_bytes()


def _replace(part: Path, path: Path) -> None:
    os.replace(part, path)


def _discard(parts: list[Path]) -> list[dict[str, Any]]:
    """The parts this run wrote, removed again; one that cannot be is `export_write_failed`."""
    errors = []
    for part in parts:
        try:
            part.unlink(missing_ok=True)
        except OSError as exc:
            errors.append(_failed(part, exc))
    return errors


def _commit(staged: list[tuple[Target, bytes]]) -> None:
    """Write every file: first each as its part, then, when every file is still the bytes that were read, each part
    renamed over its file. A file changed since (or no longer readable) is `file_changed`, and every part is removed
    again, so nothing is written. A part that cannot be written or renamed is `export_write_failed`; the files renamed
    before it stay written, and `apply` can run again."""
    parts: list[Path] = []
    for target, raw in staged:
        try:
            parts.append(_stage(target, raw))
        except OSError as exc:
            raise Failure(_failed(_part(target), exc), *_discard([*parts, _part(target)])) from exc
    changed = []
    for target, _ in staged:
        try:
            same = _current(target.path) == target.raw
        except OSError:
            same = False
        if not same:
            changed.append(error(FILE_CHANGED, {"path": str(target.path)}))
    if changed:
        raise Failure(*changed, *_discard(parts))
    for index, ((target, _), part) in enumerate(zip(staged, parts)):
        try:
            _replace(part, target.path)
        except OSError as exc:
            raise Failure(_failed(target.path, exc), *_discard(parts[index:])) from exc


def apply_bindings(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns = Unknowns()
) -> dict[str, Any]:
    """`acmodel bindings apply`: write each bound literal that differs from its exported threshold or value, changing
    only its bytes, and for a value its origin and the members it inserts. Anything else `check` refuses is refused, and
    then nothing is written. The edited bytes must read back as the document the edits make, and every file is renamed
    from its part only when it is still the bytes that were read (`_commit`)."""
    run, bindings = _load(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    _raise(_references(run, bindings))
    repo, targets = _targets(run, bindings, False)
    values, errors = _values(run, bindings)
    valued, refused = _valued(run, bindings)
    errors.extend(refused)
    errors.extend(_requires(targets, bindings))
    edits: dict[str, list[splice.Edit]] = {}
    expected: dict[str, Any] = {}  # the document each edited file must read back as
    changed = []
    slot_ids = slots(run.checker, run.graph.typed)
    layouts = _Layouts()
    for ref, entry in _entries(bindings):
        literal = _Literal(targets, ref, entry)
        name = entry["file"]
        if "value" in entry:
            if entry["value"] not in valued:  # refused above
                continue
            plan = _ValuePlan(literal, valued[entry["value"]], layouts)
            errors.extend(plan.problems)
            if plan.problems or not plan.edits:
                continue
            edits.setdefault(name, []).extend(plan.edits)
            plan.expect(expected.setdefault(name, copy.deepcopy(literal.target.doc)))
            changed.append(plan.changed())
            continue
        missing = literal.slot(slot_ids) or literal.missing()
        if missing is not None:
            errors.append(missing)
            continue
        if entry.get("threshold") not in values:  # a fixed literal, or a refused threshold
            continue
        problems, value = literal.compare(values[entry["threshold"]])
        errors.extend(problems)
        if not problems and literal.value != value:
            edits.setdefault(name, []).append(_literal_edit(layouts.of(literal.target), literal.target, literal.ref, value))
            splice.assign(expected.setdefault(name, copy.deepcopy(literal.target.doc)), literal.ref, value)
            changed.append({**literal.describe(), "from": literal.value, "to": value})
    errors.extend(_unbound_origins(targets, bindings))
    _raise(errors)
    staged = []
    for name, file_edits in edits.items():
        target = targets[name]
        raw = splice.apply(target.raw, file_edits)
        splice.check(raw, expected[name], target.path, SCHEDULE_FORMAT if name == SCHEDULE_FILE else POLICY_FORMAT)
        staged.append((target, raw))
    _commit(staged)
    written = [str(target.path) for target, _ in staged]
    return {**run.head, "repo": str(repo), "changed": changed, "written": sorted(written)}
