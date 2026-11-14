"""`acmodel export valuations` (#466 W3; contracts/formats.md, "`acmodel export valuations`"): the base weight W of each
pool valuation that a valuation binding of bindings.json ties to the model, from the prices `export prices` delivered and
the pool's own scale Q in schedule.json, written into the model folder:
- `out/provenance/valuations/<pool>.json`: the provenance document of each, a pending one too;
- `out/valuations.json` (`acmodel.valuations.v1`), last: every valuation binding with its W, or `pending` and why, its
  status and the acceptances it rests on, and how far its rounding is off. `out/values.json` is never touched: the values
  have their own file and format (`export values`).

**W** is the milli of value of Q steps of the pool (the Runtime's scheduling contract, "Fixed values written in the pack":
consuming `c` costs `c·W/Q`): the pool's price in milli (prices.json's `milli`, the price times κ) times Q, rounded as the
binding declares (`round`). Q, the pool's `scale`, is the schedule's, and so are the block's `name` and `unit`: they are
read, never written.

**Range and precision**: three reasons, three codes, each with a suggested scale (`suggest`), so that no code holds two
reasons:
- W rounds to 0 while the price is above 0 (`valuation_rounds_to_zero`): refused. A W of 0 says the pool costs nothing
  to use, which would be wrong without a word;
- W lies beyond 0..1,000,000 (`valuation_out_of_range`), what the Runtime takes; a price below 0 too: refused;
- W is off its exact value by more than `PRECISION_BOUND` of it (`precision_low`): a warning only. W is written, and
  whether Q changes is the resource side's to decide.
The suggested scale is the least power of ten whose W is in range and within the bound, or none when no power of ten
within Q's range does: the tool never suggests a scale that does not hold, and never changes Q.

**Every valuation is listed**: one whose price is pending (as `export values` reads prices, `values.PriceReader`) is
written `pending` with why. A binding whose pool is not there, is a slot, holds the pending marker for its valuation or no
block with an integer W and a scale in range, a resource the model lacks, a W refused for its range, or prices.json of
other versions or another κ, refuses the whole export, and nothing is written.

`bindings check|apply` take a bound valuation from here too (`weigh`), so a binding holds only what this export writes.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from fractions import Fraction
from pathlib import Path
from typing import Any

from acmodel.cache.keys import digest
from acmodel.check.walk import at
from acmodel.codes import (
    BINDING_MISMATCH,
    MODEL_ID_DUPLICATE,
    PRECISION_LOW,
    VALUATION_OUT_OF_RANGE,
    VALUATION_PENDING_MARKER,
    VALUATION_ROUNDS_TO_ZERO,
)
from acmodel.config import Unknowns
from acmodel.core.common import CONFIRMED, PROVISIONAL
from acmodel.core.flow import slots
from acmodel.core.provenance import SCHEMA_VERSION as PROVENANCE_FORMAT
from acmodel.core.provenance import relative
from acmodel.core.run import Run, evaluate_model
from acmodel.core.versions import sha256, versions
from acmodel.envelope import Failure, error
from acmodel.export.policy import delivered, scaled
from acmodel.export.thresholds import write_file
from acmodel.export.values import PENDING, PRICES, PriceReader, Prices, current_prices
from acmodel.model import read_bindings, readers
from acmodel.model.jsonfile import MAX_SAFE_INTEGER, read
from acmodel.model.shape import json_type

SCHEMA_VERSION = "acmodel.valuations.v1"
VALUATIONS = "out/valuations.json"
PROVENANCE = "out/provenance/valuations"
BINDINGS = "bindings.json"
OUTPUT = "valuation:"  # a valuation's output id, `valuation:<pool>`, as its provenance document and its refusals name it
WHOLE = "valuations"  # the output of a refusal of the whole export (`prices_stale`)
# W's range (the Runtime's scheduling contract, "Pool Valuation": `base_weight_milli` 0..=1,000,000), and Q's.
WEIGHT_MAX = 1_000_000
# The precision bound: W may be off its exact value by at most this much of it (0.5 %), else `precision_low`.
PRECISION_BOUND = Fraction(1, 200)
# The scales a suggestion is taken from: the powers of ten within Q's range, 1..2^53 - 1, the least first.
SCALES = tuple(10 ** power for power in range(16))
BLOCK, WEIGHT, SCALE = "valuation", "base_weight_milli", "scale"  # a pool's valuation block and the two members read
MARKER = "pending"  # the Runtime's marker of a valuation not known yet (`"valuation": "pending"`)
NONE = "none"  # a suggested scale, in an error's detail, when no power of ten holds


def _raise(errors: list[dict[str, Any]]) -> None:
    if errors:
        raise Failure(*errors)


def _integer(value: Any) -> bool:
    """A JSON integer; a boolean is not one."""
    return type(value) is int


def _shown(value: Any) -> str:
    """A value as an error shows it: a string or an integer as its text, anything else as its JSON type."""
    return str(value) if isinstance(value, str) or _integer(value) else json_type(value)


def fits(exact: Fraction, weight: int) -> bool:
    """Whether W (`weight`, rounded from `exact`) holds: in 0..1,000,000, of a price not below 0, and within the bound."""
    return 0 <= weight <= WEIGHT_MAX and exact >= 0 and abs(weight - exact) <= PRECISION_BOUND * exact


def suggest(milli: Fraction, mode: str) -> int | None:
    """The least power of ten Q whose W (the price in milli times Q, rounded by `mode`) holds (`fits`), or None when none
    within Q's range does: a price beyond 1,000,000 milli a step, or below 0, or one so small that 10^15 steps do not reach
    the bound."""
    for scale in SCALES:
        if fits(milli * scale, scaled(milli, scale, mode)):
            return scale
    return None


@dataclass
class Weighed:
    """One valuation binding as `export valuations` computes it (`weigh`)."""

    ref: str  # the binding's pointer in bindings.json
    entry: dict[str, Any]  # the binding
    schedule: str  # schedule.json's path
    index: int | None = None  # the pool's index in schedule.json
    scale: int | None = None  # Q, as the pool holds it
    status: str = CONFIRMED
    milli: Fraction | None = None  # the price in milli of one step
    exact: Fraction | None = None  # milli times Q
    weight: int | None = None  # W, rounded
    precision: dict[str, Any] | None = None  # how far W is off, when beyond the bound
    acceptances: dict[str, dict[str, str]] = field(default_factory=dict)
    pending: list[dict[str, Any]] = field(default_factory=list)
    refusals: list[dict[str, Any]] = field(default_factory=list)
    warnings: list[dict[str, Any]] = field(default_factory=list)
    priced: bool = False  # its price's entry of prices.json was read (a pending one too)

    @property
    def pool_id(self) -> str:
        return self.entry["pool_id"]

    @property
    def resource(self) -> str:
        return self.entry["valuation"]

    @property
    def output(self) -> str:
        return OUTPUT + self.pool_id

    @property
    def pool_ref(self) -> str:
        return at("", "pools", self.index)

    @property
    def scale_ref(self) -> str:
        return at(self.pool_ref, BLOCK, SCALE)

    def tokens(self) -> list[str]:
        """Each reason it is pending, as `<reason>:<item>`."""
        return [f"{entry['reason']}:{entry['item']}" for entry in self.pending]

    def detail(self, **values: Any) -> dict[str, Any]:
        return {"path": self.schedule, "entry": self.ref, "valuation": self.resource, "pool_id": self.pool_id, **values}

    def document_entry(self) -> dict[str, Any]:
        """Its entry of valuations.json."""
        return {
            "pool_id": self.pool_id,
            "valuation": self.resource,
            "status": self.status,
            "round": self.entry["round"],
            "scale": self.scale,
            "milli": None if self.milli is None else str(self.milli),
            "base_weight_milli": self.weight,
            "exact_weight_milli": None if self.exact is None else str(self.exact),
            "precision": self.precision,
            "acceptances": [self.acceptances[name] for name in sorted(self.acceptances)],
            "pending": list(self.pending),
            "provenance": f"{PROVENANCE}/{self.pool_id}.json",  # a pending valuation's too
        }

    def mismatch(self, reason: str, ref: str, **values: Any) -> None:
        self.refusals.append(error(BINDING_MISMATCH, {**self.detail(reason=reason), **values}, ref))


def _pool(weighed: Weighed, pools: dict[str, int], doc: Any, slot_ids: frozenset[str]) -> None:
    """The pool a valuation binding names and its scale Q: a pool of schedule.json (`binding_mismatch`,
    `literal_missing`, at /pools otherwise) that is no slot (`slot`), whose valuation is not the pending marker
    (`valuation_pending_marker`) but a block (`literal_missing`) with an integer W (`literal_missing`) and an integer Q
    in 1..2^53 - 1 (`scale`)."""
    pool_id = weighed.pool_id
    if pool_id in slot_ids:  # no Runtime pool (#424 ACM-PRICE 3.1), as for a threshold's pool
        weighed.mismatch("slot", at("", "pools", pools[pool_id]) if pool_id in pools else at("", "pools"))
        return
    if pool_id not in pools:
        weighed.mismatch("literal_missing", at("", "pools"))
        return
    weighed.index = pools[pool_id]
    pool = doc["pools"][weighed.index]
    block_ref = at(weighed.pool_ref, BLOCK)
    if BLOCK not in pool:
        weighed.mismatch("literal_missing", weighed.pool_ref)
        return
    block = pool[BLOCK]
    if block == MARKER:
        weighed.refusals.append(error(VALUATION_PENDING_MARKER, weighed.detail(), block_ref))
        return
    if not isinstance(block, dict):
        weighed.mismatch("literal_missing", block_ref, found=json_type(block))
        return
    if not _integer(block.get(WEIGHT)):
        found = {} if WEIGHT not in block else {"found": json_type(block[WEIGHT])}
        weighed.mismatch("literal_missing", at(block_ref, WEIGHT) if found else block_ref, **found)
        return
    scale = block.get(SCALE)
    if not _integer(scale) or not 1 <= scale <= MAX_SAFE_INTEGER:
        found = {} if SCALE not in block else {"found": _shown(scale)}
        weighed.mismatch("scale", weighed.scale_ref if found else block_ref, **found)
        return
    weighed.scale = scale


def _range(weighed: Weighed, mode: str) -> None:
    """W of the price and Q, and the one code of the three that holds, if any, each with its suggested scale."""
    milli, scale = weighed.milli, weighed.scale
    exact = milli * scale
    weight = scaled(milli, scale, mode)
    weighed.exact, weighed.weight = exact, weight
    suggested = suggest(milli, mode)
    shown = {"scale": str(scale), "exact": str(exact), "weight": str(weight),
             "suggested_scale": NONE if suggested is None else str(suggested)}
    if exact > 0 and weight == 0:
        weighed.refusals.append(error(VALUATION_ROUNDS_TO_ZERO, weighed.detail(**shown), weighed.scale_ref))
    elif exact < 0 or weight > WEIGHT_MAX:
        weighed.refusals.append(error(VALUATION_OUT_OF_RANGE, weighed.detail(**shown), weighed.scale_ref))
    elif abs(weight - exact) > PRECISION_BOUND * exact:
        off = abs(weight - exact) / exact
        weighed.precision = {"error": str(off), "bound": str(PRECISION_BOUND), "suggested_scale": suggested}
        detail = weighed.detail(**shown, error=str(off), bound=str(PRECISION_BOUND))
        weighed.warnings.append(error(PRECISION_LOW, detail, weighed.scale_ref))


def weigh(
    reader: PriceReader, schedule: Path | str, pools: dict[str, int], doc: Any, slot_ids: frozenset[str], ref: str,
    entry: dict[str, Any],
) -> Weighed:
    """One valuation binding (`ref` its pointer in bindings.json) over the prices and schedule.json (its `path`, its
    pools' indexes by id and its document): its pool and Q (`_pool`), then the price of its resource, then W and its
    range and precision (`_range`). A pending price leaves it `pending`, with why; a price delivered under an acceptance
    makes it `provisional`, listing that acceptance."""
    weighed = Weighed(ref, entry, str(schedule))
    _pool(weighed, pools, doc, slot_ids)
    if weighed.refusals:
        return weighed
    found = reader.read(weighed.resource)
    weighed.priced = found.listed
    if found.pending is not None:
        weighed.pending.append(found.pending)
        weighed.status = PENDING
        return weighed
    if found.acceptance is not None:
        output, given = found.acceptance
        weighed.acceptances[output] = {"output": output, "ref": given["ref"], "until": given["until"]}
    weighed.milli = found.milli
    _range(weighed, entry["round"])
    weighed.status = PROVISIONAL if weighed.acceptances else CONFIRMED
    return weighed


def bound(bindings: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    """(pointer in bindings.json, binding) of each valuation binding, in order."""
    return [(at("", "bindings", index), entry) for index, entry in enumerate(bindings["bindings"]) if "valuation" in entry]


def problems(run: Run, path: str, entries: list[tuple[str, dict[str, Any]]]) -> list[dict[str, Any]]:
    """Each valuation binding names a resource of the model (`rule_reference_missing`, `expected` `resource`, at its
    `valuation`), as the run checks its own references."""
    checker, errors = run.checker, []
    for ref, entry in entries:
        checker.need(path, at(ref, "valuation"), entry["valuation"], ("resource",))
        errors.extend(checker.errors)
        checker.errors.clear()
    return errors


def bindings_sha(run: Run) -> str:
    """The sha256 of bindings.json's bytes, as valuations.json records it."""
    return digest(read(run.folder / BINDINGS, readers.BINDINGS.name))


def _document(run: Run, weighed: Weighed, entry: dict[str, Any], prices: Prices) -> dict[str, Any]:
    """The provenance document of a valuation, a pending one too: a node for the valuation (its binding in bindings.json,
    the scale it read in schedule.json, and the price it read), and the price's node as prices.json has it, pending or not
    (a price prices.json does not list has none)."""
    nodes: dict[str, Any] = {}
    reads = []
    if weighed.priced:
        price = prices.by_pool[weighed.resource]
        name = "price:" + weighed.resource
        reads.append(name)
        nodes[name] = {
            "node": "price", "resource": weighed.resource, "file": relative(run, str(prices.path)), "sha256": prices.sha,
            "class": price["class"], "status": price["status"], "price": price["price"], "milli": price["milli"],
            "provenance": price["provenance"], "reads": [],
        }
    nodes[weighed.output] = {
        "node": "valuation", "pool_id": weighed.pool_id, "resource": weighed.resource,
        "file": relative(run, str(run.folder / BINDINGS)), "ref": weighed.ref, "sha256": sha256(weighed.entry),
        "schedule": {"file": readers.SCHEDULE_FILE, "ref": weighed.scale_ref, "scale": weighed.scale},
        "reads": reads,
    }
    doc = {
        "schema_version": PROVENANCE_FORMAT,
        "model_id": run.head["model_id"],
        "output": weighed.output,
        "result": {key: value for key, value in entry.items() if key != "provenance"},
        "versions": versions(run.checker, run.folder),
        "nodes": dict(sorted(nodes.items())),
    }
    if weighed.acceptances:
        doc["acceptances"] = [weighed.acceptances[name] for name in sorted(weighed.acceptances)]
    return doc


def build(run: Run) -> tuple[dict[str, bytes], dict[str, Any], list[dict[str, Any]]]:
    """The files of the export, by their path relative to the model folder and in the order they are written
    (`valuations.json` last), the counts `data` shows, and the warnings (`precision_low`). A refusal raises `Failure`, and
    nothing is written."""
    from acmodel.bindings.targets import read_target  # here: the bindings package imports this module

    path = run.folder / BINDINGS
    entries = bound(read_bindings(path))
    prices = current_prices(run, WHOLE)
    errors = problems(run, str(path), entries)
    seen: dict[str, str] = {}
    for ref, entry in entries:  # one valuation per pool: each has its provenance file by the pool's id
        if entry["pool_id"] in seen:
            detail = {"path": str(path), "namespace": "literal", "id": f"{entry['file']}:{entry['pool_id']}:{entry['path']}",
                      "first_path": str(path), "first_ref": seen[entry["pool_id"]]}
            errors.append(error(MODEL_ID_DUPLICATE, detail, ref))
        else:
            seen[entry["pool_id"]] = ref
    _raise(errors)
    found: list[Weighed] = []
    if entries:
        schedule = read_target(run.folder.parent.parent, readers.SCHEDULE_FILE)
        reader, slot_ids = PriceReader(run, prices), slots(run.checker, run.graph.typed)
        found = [weigh(reader, schedule.path, schedule.pools, schedule.doc, slot_ids, ref, entry) for ref, entry in entries]
    _raise([refusal for weighed in found for refusal in weighed.refusals])
    files: dict[str, Any] = {}
    listed = []
    for weighed in sorted(found, key=lambda each: each.pool_id):
        entry = weighed.document_entry()
        files[entry["provenance"]] = _document(run, weighed, entry, prices)
        listed.append(entry)
    counts = {status: sum(1 for entry in listed if entry["status"] == status) for status in (CONFIRMED, PROVISIONAL, PENDING)}
    counts["precision_low"] = sum(1 for entry in listed if entry["precision"] is not None)
    files[VALUATIONS] = {
        "schema_version": SCHEMA_VERSION,
        "model_id": run.head["model_id"],
        "versions": versions(run.checker, run.folder),
        "prices": {"file": PRICES, "sha256": prices.sha, "versions": prices.doc["versions"]},
        "bindings": {"file": relative(run, str(path)), "sha256": bindings_sha(run)},
        "schedule": {"file": readers.SCHEDULE_FILE},
        "unit_milli": prices.doc["unit_milli"],
        "precision_bound": str(PRECISION_BOUND),
        "valuations": listed,
        "counts": counts,
    }
    warnings = [warning for weighed in found for warning in weighed.warnings]
    return {name: delivered(doc) for name, doc in files.items()}, {"counts": counts}, warnings


def export_valuations(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns = Unknowns()
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Evaluate the model for the state, then write the provenance files of the bound valuations and `valuations.json`,
    or refuse; the warnings are their `precision_low`."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    files, data, warnings = build(run)
    written = []
    for name, payload in files.items():  # valuations.json last
        target = run.folder / name
        write_file(target, payload)
        written.append(str(target))
    return {**run.head, **data, "written": sorted(written)}, warnings
