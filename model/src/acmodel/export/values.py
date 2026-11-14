"""`acmodel export values` (#464 W1; contracts/formats.md, "`acmodel export values`"): what one run of a schedule's rule is
worth, from the model's derivation graph and the prices `export prices` delivered, written into the model folder:
- `out/provenance/values/<id>.json`: the provenance document of each value, a pending one too (with its pending and why,
  and what it could read), so that no file of an earlier export stays behind with other versions;
- `out/values.json` (`acmodel.values.v1`), last: every value of exports.json's `values`, each with its number or
  `pending` and why, its status and the acceptances it rests on, and what one run brings in and takes out of each pool.

**A value** is a list of terms, each an activity of the model run `times` per period (`per`), what its active flows bring
in and take out, or a resource, `amount` of it `times` per period; `per` `run` means once per run of the rule. `runs` says
how often the rule runs per period, so each term counts per run: times x the days of `runs.per` / (the days of `per` x
`runs.count`), the days of a period by the model's own definition (a day 1, a week 7, a `days` period its length, a month
146097/4800, as the economy reads them, `core.flow`). The value in thousandths of a priority is, over the pools, what one
run brings in less what it takes out, times the pool's price in milli (prices.json's `milli`, the price times κ); the
pools of `cost_paid_by_pool` are those the rule's `consumes` declares, whose spending the Runtime takes off itself, so
what the run takes out of them is not; the pools of `exclude` are left out whole, each with its reason (the value is then
`partial`). Then `round`, which is declared, never implicit. Everything is exact.

It computes the derivation graph only, as `export thresholds` does: no steady state and no LP. A quantity is a number or
`rule:<id>`, the value of a rule of the run (a derive, cap, limit, or a random rule's pity chain or drop: what `rule:`
reads in an expression); anything else is refused by its reader, and `rule:` of a rule without a value is
`rule_reference_missing`, never guessed.

**Every value is listed** (as prices.json lists every pool): a value is `pending`, with each reason, when a quantity is
blocked, provisional and not accepted (`acceptances` of exports.json accept `rule:<id>`; a flow's amount is no output, so
a provisional one is pending), not one number, or a pool it counts has no exported price, or one delivered under an
acceptance that exports.json no longer holds as prices.json recorded it (withdrawn or changed since: the prices are
exported again); otherwise `provisional` when it rests on an acceptance (of a quantity or of a price), each listed with
its `ref` and `until`, and `confirmed`. A value
that read the account state (`per_account`), one that rounds below 1 (`value_not_positive`: a value of 0 is no value) or
beyond the JSON-safe range (`out_of_range`) refuses the whole export, and nothing is written. So does a prices.json that
is not of the model's versions or of exports.json's κ (`export_refused`, `prices_stale`): the prices are read, never
computed here.

`bindings check|apply` take a bound value from here too (`evaluate`), so a binding holds only what this export writes.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from fractions import Fraction
from pathlib import Path
from typing import Any

from acmodel.cache.keys import digest
from acmodel.check.walk import at
from acmodel.codes import (
    EXPORT_REFUSED,
    EXPR_DIMENSION_MISMATCH,
    MODEL_ID_DUPLICATE,
    RULE_REFERENCE_MISSING,
    VALUE_NOT_POSITIVE,
)
from acmodel.config import Unknowns
from acmodel.core.common import BLOCKED, CONFIRMED, PROVISIONAL, has_value, solve
from acmodel.core.flow import MONTH_DAYS, WEEK_STEPS
from acmodel.core.provenance import SCHEMA_VERSION as PROVENANCE_FORMAT
from acmodel.core.provenance import (
    STATE_NODES,
    acceptance,
    acceptances,
    assemble,
    node_id,
    output_result,
    relative,
    tree,
)
from acmodel.core.run import Run, evaluate_model
from acmodel.core.unknown import status as unknowns_status
from acmodel.core.versions import sha256, versions
from acmodel.envelope import Failure, error
from acmodel.export.policy import delivered, scaled
from acmodel.export.thresholds import write_file
from acmodel.expr import BOOL, Duration
from acmodel.model import read_prices, readers
from acmodel.model.jsonfile import MAX_SAFE_INTEGER, read, refusal

SCHEMA_VERSION = "acmodel.values.v1"
VALUES = "out/values.json"
PROVENANCE = "out/provenance/values"
PRICES = "out/prices.json"
PENDING = "pending"
# A price delivered under an acceptance that exports.json no longer holds as prices.json recorded it (formats.md,
# "`acmodel export values`", step 6): withdrawn since, or changed since. The value waits for `export prices`.
WITHDRAWN, CHANGED = "price_acceptance_withdrawn", "price_acceptance_changed"
OUTPUT = "value:"  # a value's output id, as its provenance document and its refusals name it
WHOLE = "values"  # the output of a refusal of the whole export (`prices_stale`)
# What `rule:<id>` may name (formats.md, "Model checks of the run", "Rule values"): a rule with a value.
VALUE_RULES = ("rule.derive", "rule.cap", "rule.limit", "rule.random")
RANDOM_VALUES = ["rule.random.pity_chain", "rule.random.drop"]
RULE = "rule:"


def _raise(errors: list[dict[str, Any]]) -> None:
    if errors:
        raise Failure(*errors)


@dataclass(frozen=True)
class Prices:
    """`out/prices.json` as `export prices` delivered it: its file, its bytes and its document."""

    path: Path
    raw: bytes
    doc: dict[str, Any]

    @property
    def by_pool(self) -> dict[str, dict[str, Any]]:
        return {entry["pool"]: entry for entry in self.doc["prices"]}

    @property
    def sha(self) -> str:
        return digest(self.raw)


def current_prices(run: Run, whole: str = WHOLE) -> Prices:
    """The prices the values are counted in: `out/prices.json` of the model folder, read back by its reader
    (`model_file_invalid` when it is not there), of the model (`value_invalid` at `/model_id`), of the model's versions
    and in exports.json's κ. A file of other versions, or of another κ, is `export_refused` (`prices_stale`, `output`
    `whole`: `values`, or `valuations` for the pools' valuations, #466 W3), one error per member that differs (`value` the
    member: `rules`, `data`, `tool`, `unit_milli`): the prices are exported again first. `tool` is the tool's own version,
    so after an upgrade of acmodel every prices.json is stale until `export prices` runs again (formats.md, step 2)."""
    path = run.folder / PRICES
    raw = read(path, readers.PRICES.name)
    doc = read_prices(path, raw)
    if doc["model_id"] != run.head["model_id"]:
        raise Failure(refusal(
            "value_invalid", path, doc["schema_version"], "/model_id", expected=[run.head["model_id"]],
            literal=doc["model_id"],
        ))
    model = versions(run.checker, run.folder)
    stale = [(member, at("", "versions", member)) for member in ("rules", "data", "tool")
             if doc["versions"][member] != model[member]]
    if run.exports.doc.get("prices", {}).get("unit_milli") != doc["unit_milli"]:
        stale.append(("unit_milli", "/unit_milli"))
    _raise([
        error(EXPORT_REFUSED, {"path": str(path), "output": whole, "reason": "prices_stale", "value": member}, ref)
        for member, ref in stale
    ])
    return Prices(path, raw, doc)


@dataclass(frozen=True)
class PriceRead:
    """What a value or a pool's valuation (#466 W3) reads of one pool's price in prices.json (formats.md, "`acmodel export
    values`", step 6): the price in milli, or why it is pending (`{reason, item}`, with the price's `pending` reasons for
    `price_pending`); whether prices.json lists the pool (its entry was read); and the acceptance it was delivered under
    (`price:<pool>` and its `{ref, until}`), if any."""

    milli: Fraction | None
    listed: bool
    pending: dict[str, Any] | None = None
    acceptance: tuple[str, dict[str, Any]] | None = None


class PriceReader:
    """The prices of one prices.json as values and valuations read them, each checked against exports.json's acceptances
    now (read once, when a price needs them)."""

    def __init__(self, run: Run, prices: Prices) -> None:
        self.run, self.prices = run, prices
        self.by_pool = prices.by_pool
        self.accepted: dict[str, int] | None = None  # exports.json's acceptances now, by output

    def withdrawn_or_changed(self, pool: str, recorded: dict[str, Any]) -> str | None:
        """Whether the acceptance a price was delivered under is still exports.json's as prices.json recorded it (its
        entry there, `entry` aside, note included): None when it is, `WITHDRAWN` when exports.json holds none of
        `price:<pool>` now, `CHANGED` when it holds another."""
        if self.accepted is None:
            self.accepted = acceptances(self.run)
        index = self.accepted.get("price:" + pool)
        if index is None:
            return WITHDRAWN
        held = {key: value for key, value in recorded.items() if key != "entry"}
        return None if self.run.exports.doc["acceptances"][index] == held else CHANGED

    def read(self, pool: str) -> PriceRead:
        """A pool's price in milli, as prices.json delivered it; a price delivered under an acceptance (a provisional one,
        or a verified zero) carries that acceptance, but only while exports.json still holds it as prices.json recorded it
        (`price_acceptance_withdrawn`, `price_acceptance_changed` otherwise: the prices are exported again)."""
        entry = self.by_pool.get(pool)
        if entry is None:  # a slot, or no pool of the model
            return PriceRead(None, False, {"reason": "price_missing", "item": pool})
        if entry["price"] == PENDING or entry["milli"] is None:
            return PriceRead(None, True, {"reason": "price_pending", "item": pool, "pending": list(entry["pending"])})
        if entry["class"] == "verified_zero" or entry["status"] != CONFIRMED:
            kind = "accept_zero" if entry["class"] == "verified_zero" else "accept_provisional"
            recorded = entry["acceptance"] if isinstance(entry["acceptance"], dict) else {}
            given = recorded.get(kind)
            if given is None:
                return PriceRead(None, True, {"reason": "price_pending", "item": pool, "pending": ["not_accepted"]})
            moved = self.withdrawn_or_changed(pool, recorded)
            if moved is not None:
                return PriceRead(None, True, {"reason": moved, "item": pool})
            return PriceRead(Fraction(entry["milli"]), True, None, ("price:" + pool, given))
        return PriceRead(Fraction(entry["milli"]), True)


def period_days(run: Run, period_id: str) -> Fraction:
    """The days of a period of model.json, as the economy reads them (`core.flow`): a day 1, a week 7, a `days` period its
    `length`, a month 146097/4800."""
    period = next(entry for entry in run.checker.model.manifest.doc["periods"] if entry["id"] == period_id)
    unit = period["unit"]
    if unit == "month":
        return MONTH_DAYS
    if unit == "days":
        return Fraction(period["length"])
    return Fraction(WEEK_STEPS if unit == "week" else 1)


def problems(run: Run) -> list[dict[str, Any]]:
    """What the values of exports.json name, checked as the run checks its references (`rule_reference_missing`, with
    `expected` and `found`): each value's id once (`model_id_duplicate`, `value`); a term's activity, an activity with an
    active flow (`rule.flow`, at the activity, otherwise: what it brings would be no amount at all), and its resource; each
    `per` a period or `run`, and `runs.per` a period; the pools of `cost_paid_by_pool` and `exclude` resources; and each
    `rule:<id>` a rule with a value, never a boolean one, and for a resource's amount never an amount of another
    resource (`expr_dimension_mismatch`, `operator` `value` or `amount`)."""
    checker, path, typed = run.checker, run.exports.path, run.graph.typed.outputs
    errors: list[dict[str, Any]] = []
    seen: dict[str, str] = {}

    def need(ref: str, target: str, expected: tuple[str, ...]) -> Any:
        entry = checker.need(path, ref, target, expected)
        errors.extend(checker.errors)
        checker.errors.clear()
        return entry

    def quantity(ref: str, text: str, resource: str | None) -> None:
        if not text.startswith(RULE):
            return
        target = text[len(RULE):]
        entry = need(ref, target, VALUE_RULES)
        if entry is None:
            return
        if not has_value(entry):
            detail = {"path": path, "target": target, "expected": RANDOM_VALUES, "found": "rule.random.outcomes"}
            errors.append(error(RULE_REFERENCE_MISSING, detail, ref))
            return
        kind = typed.get(("rule", target))
        if kind == BOOL:
            detail = {"path": path, "operator": "value", "operands": ["number", BOOL.token], "position": 0}
            errors.append(error(EXPR_DIMENSION_MISMATCH, detail, ref))
        elif resource is not None and kind is not None and kind.kind == "amount" and kind.resource != resource:
            detail = {"path": path, "operator": "amount", "operands": [f"amount:{resource}", kind.token], "position": 0}
            errors.append(error(EXPR_DIMENSION_MISMATCH, detail, ref))

    for index, value in enumerate(run.exports.doc.get("values", [])):
        ref = at("", "values", index)
        if value["id"] in seen:
            detail = {"path": path, "namespace": "value", "id": value["id"], "first_path": path,
                      "first_ref": seen[value["id"]]}
            errors.append(error(MODEL_ID_DUPLICATE, detail, at(ref, "id")))
        else:
            seen[value["id"]] = at(ref, "id")
        for term_index, term in enumerate(value["terms"]):
            where = at(ref, "terms", term_index)
            if "activity" in term:
                activity = term["activity"]
                if need(at(where, "activity"), activity, ("activity",)) is not None and not checker.flows_of.get(activity):
                    detail = {"path": path, "target": activity, "expected": ["rule.flow"]}
                    errors.append(error(RULE_REFERENCE_MISSING, detail, at(where, "activity")))
            else:
                found = need(at(where, "resource"), term["resource"], ("resource",))
                quantity(at(where, "amount"), term["amount"], None if found is None else term["resource"])
            quantity(at(where, "times"), term["times"], None)
            if term["per"] != readers.PER_RUN:
                need(at(where, "per"), term["per"], ("period",))
        need(at(ref, "runs", "per"), value["runs"]["per"], ("period",))
        for pool_index, pool in enumerate(value.get("cost_paid_by_pool", [])):
            need(at(ref, "cost_paid_by_pool", pool_index), pool, ("resource",))
        for pool_index, entry in enumerate(value.get("exclude", [])):
            need(at(ref, "exclude", pool_index, "resource"), entry["resource"], ("resource",))
    return errors


def _number(value: Any) -> Fraction | None:
    """A single exact number of a scenario (a duration in seconds); None for an enclosure or a boolean."""
    if type(value) is Fraction:
        return value
    if type(value) is Duration:
        return value.seconds
    return None


def _single(shown: dict[str, Any]) -> Fraction | None:
    """The one number of an output as `run` shows it, or None when its scenarios hold more than one or a range."""
    if "variants" in shown:
        found = {entry.get("value") for entry in shown["variants"]}
        return Fraction(found.pop()) if len(found) == 1 and None not in found else None
    value = shown.get("value")
    return None if value is None or isinstance(value, bool) else Fraction(value)


def _expected(run: Run, nodes: dict[str, dict[str, Any]]) -> bool:
    """Whether a quantity is an expected value: it read a random rule (a drop's chance, a pity chain's expected pulls) or
    a derive with an `estimate` (formats.md, "Declared zeros and estimates")."""
    for node in nodes.values():
        entry = run.checker.entries.get(node["id"]) if node.get("node") == "rule" else None
        if entry is not None and (
            entry.kind == "random" or (entry.kind == "derive" and "estimate" in entry.doc["logic"])
        ):
            return True
    return False


@dataclass(frozen=True)
class _Quantity:
    number: Fraction
    expected: bool


@dataclass
class Valued:
    """One value of exports.json as `export values` computes it (`evaluate`)."""

    id: str
    index: int
    definition: dict[str, Any]
    status: str = CONFIRMED
    milli: int | None = None
    exact: Fraction | None = None
    acceptances: dict[str, dict[str, str]] = field(default_factory=dict)
    pending: list[dict[str, Any]] = field(default_factory=list)
    yields: list[dict[str, Any]] | None = None
    refusals: list[dict[str, Any]] = field(default_factory=list)
    nodes: dict[str, dict[str, Any]] = field(default_factory=dict)  # the provenance nodes of what it read
    roots: list[str] = field(default_factory=list)  # the outputs and flow amounts it read, and the prices
    priced: list[str] = field(default_factory=list)  # the pools whose entry of prices.json it read (pending ones too)

    @property
    def ref(self) -> str:
        return at("", "values", self.index)

    @property
    def output(self) -> str:
        return OUTPUT + self.id

    @property
    def partial(self) -> bool:
        return bool(self.definition.get("exclude"))

    def tokens(self) -> list[str]:
        """Each reason it is pending, as `<reason>:<item>`."""
        return [f"{entry['reason']}:{entry['item']}" for entry in self.pending]

    def entry(self) -> dict[str, Any]:
        """Its entry of values.json."""
        definition = self.definition
        return {
            "id": self.id,
            "status": self.status,
            "fixed_value_milli": self.milli,
            "exact_milli": None if self.exact is None else str(self.exact),
            "round": definition["round"],
            "acceptances": [self.acceptances[name] for name in sorted(self.acceptances)],
            "pending": list(self.pending),
            "partial": self.partial,
            "exclude": list(definition.get("exclude", [])),
            "cost_paid_by_pool": list(definition.get("cost_paid_by_pool", [])),
            "runs": definition["runs"],
            "yields": self.yields,
            "provenance": f"{PROVENANCE}/{self.id}.json",  # a pending value's too
        }


class _Evaluation:
    """The computation of one value over one run and its prices."""

    def __init__(self, run: Run, prices: Prices, index: int, definition: dict[str, Any]) -> None:
        self.run, self.prices = run, prices
        self.valued = Valued(definition["id"], index, definition)
        self.taken: dict[str, Fraction] = {}
        self.brought: dict[str, Fraction] = {}
        self.marked: set[str] = set()  # the pools an expected quantity reaches
        self.reader = PriceReader(run, prices)

    def refuse(self, ref: str, reason: str, **values: Any) -> None:
        detail = {"path": self.run.exports.path, "output": self.valued.output, "reason": reason, **values}
        self.valued.refusals.append(error(EXPORT_REFUSED, detail, ref))

    def wait(self, reason: str, item: str, **values: Any) -> None:
        self.valued.pending.append({"reason": reason, "item": item, **values})

    def read(self, root: str, nodes: dict[str, dict[str, Any]]) -> None:
        if root not in self.valued.roots:
            self.valued.roots.append(root)
        self.valued.nodes.update(nodes)

    def accept(self, output: str, given: dict[str, Any]) -> None:
        self.valued.acceptances[output] = {"output": output, "ref": given["ref"], "until": given["until"]}

    def quantity(self, ref: str, text: str) -> _Quantity | None:
        """A number, or the value of `rule:<id>` as the export policy delivers it: never blocked, never of the account
        state (refused), provisional only under its acceptance, one number."""
        if not text.startswith(RULE):
            return _Quantity(Fraction(text), False)
        run, output = self.run, text
        shown = output_result(run, output)
        if shown["status"] == BLOCKED:
            self.wait("blocked", output)
            return None
        document = assemble(run, output)
        if shown["per_account"] or any(node["node"] in STATE_NODES for node in document["nodes"].values()):
            self.refuse(ref, "per_account")
            return None
        if shown["status"] != CONFIRMED:
            accepted = acceptance(run, output)
            if accepted is None:
                self.wait("provisional", output)
                return None
            self.accept(output, accepted["accept_provisional"])
        number = _single(shown)
        if number is None:
            span = shown.get("range")
            self.wait("not_single", output, **({} if span is None else {"value": f"{span['low']}..{span['high']}"}))
            return None
        self.read(output, document["nodes"])
        return _Quantity(number, _expected(run, document["nodes"]))

    def amount(self, ref: str, key: tuple[str, ...]) -> _Quantity | None:
        """A flow's amount (a graph node, no output: so a provisional one cannot be accepted and is pending)."""
        run, graph = self.run, self.run.graph
        solve([key], graph.compute, graph.results)
        result, name = graph.results[key], node_id(key)
        if result.blocked:
            self.wait("blocked", name)
            return None
        nodes = tree(run, key)
        if any(scenario.per_account for scenario in result.scenarios) or any(
            node["node"] in STATE_NODES for node in nodes.values()
        ):
            self.refuse(ref, "per_account")
            return None
        if unknowns_status(result.scenarios, graph.settings.scale) != CONFIRMED:
            self.wait("provisional", name)
            return None
        found = {_number(scenario.value) for scenario in result.scenarios}
        if len(found) != 1 or None in found:
            self.wait("not_single", name)
            return None
        self.read(name, nodes)
        return _Quantity(found.pop(), _expected(run, nodes))

    def flows(self, ref: str, activity: str) -> list[tuple[str, Fraction, bool]] | None:
        """What one execution of `activity` brings in (positive) and takes out (negative) of each resource, by each
        amount of its active flows: what they produce, consume and move (into or out of a resource; a store holds no
        priced pool)."""
        checker = self.run.checker
        parts: list[tuple[str, Fraction, bool]] = []
        complete = True
        for flow_id in checker.flows_of[activity]:
            logic = checker.entries[flow_id].doc["logic"]
            keys = [(("flow", flow_id, "produces", pool), pool, 1) for pool in logic.get("produces", {})]
            keys += [(("flow", flow_id, "consumes", pool), pool, -1) for pool in logic.get("consumes", {})]
            moves = logic.get("moves")
            if moves is not None:
                for end, sign in (("from", -1), ("to", 1)):
                    if checker.entries[moves[end]].kind == "resource":
                        keys.append((("flow", flow_id, "moves"), moves[end], sign))
            for key, pool, sign in keys:
                found = self.amount(ref, key)
                if found is None:
                    complete = False
                    continue
                parts.append((pool, sign * found.number, found.expected))
        return parts if complete else None

    def add(self, pool: str, amount: Fraction, expected: bool) -> None:
        target = self.brought if amount >= 0 else self.taken
        target[pool] = target.get(pool, Fraction(0)) + abs(amount)
        (self.taken if amount >= 0 else self.brought).setdefault(pool, Fraction(0))
        if expected:
            self.marked.add(pool)

    def price(self, pool: str) -> Fraction | None:
        """A pool's price in milli, as prices.json delivered it (`PriceReader.read`); a price delivered under an acceptance
        adds that acceptance, and a pending one the reason it waits for."""
        found = self.reader.read(pool)
        if found.listed:  # its entry is read, and its provenance names it, pending or not
            self.valued.priced.append(pool)
        if found.pending is not None:
            self.valued.pending.append(found.pending)
            return None
        if found.acceptance is not None:
            self.accept(*found.acceptance)
        return found.milli

    def compute(self) -> Valued:
        valued, definition = self.valued, self.valued.definition
        runs = definition["runs"]
        per_run = period_days(self.run, runs["per"]) / Fraction(runs["count"])
        complete = True
        for term_index, term in enumerate(definition["terms"]):
            where = at(valued.ref, "terms", term_index)
            times = self.quantity(at(where, "times"), term["times"])
            factor = Fraction(1) if term["per"] == readers.PER_RUN else per_run / period_days(self.run, term["per"])
            if "resource" in term:
                amount = self.quantity(at(where, "amount"), term["amount"])
                if times is None or amount is None:
                    complete = False
                    continue
                self.add(term["resource"], amount.number * times.number * factor, amount.expected or times.expected)
                continue
            parts = self.flows(at(where, "activity"), term["activity"])
            if times is None or parts is None:
                complete = False
                continue
            for pool, amount_of, expected in parts:
                self.add(pool, amount_of * times.number * factor, expected or times.expected)
        if valued.refusals:
            return valued
        if not complete:
            valued.status = PENDING
            return valued
        paid = set(definition.get("cost_paid_by_pool", []))
        excluded = {entry["resource"] for entry in definition.get("exclude", [])}
        pools = sorted(set(self.brought) | set(self.taken))
        valued.yields = [
            {
                "pool": pool, "in": str(self.brought[pool]), "out": str(self.taken[pool]),
                "expected": pool in self.marked, "paid_by_pool": pool in paid, "excluded": pool in excluded,
            }
            for pool in pools
        ]
        total = Fraction(0)
        for pool in pools:  # what it brings in, less what it takes out but where the Runtime takes it off itself
            counted = self.brought[pool] - (0 if pool in paid else self.taken[pool])
            if pool in excluded or counted == 0:  # nothing of its price counts
                continue
            milli = self.price(pool)
            if milli is not None:
                total += counted * milli
        if valued.pending:
            valued.status = PENDING
            return valued
        number = scaled(total, 1, definition["round"])
        if number < 1:
            detail = {"path": self.run.exports.path, "value": valued.id, "milli": str(number), "exact": str(total)}
            valued.refusals.append(error(VALUE_NOT_POSITIVE, detail, valued.ref))
            return valued
        if number > MAX_SAFE_INTEGER:
            self.refuse(valued.ref, "out_of_range", value=str(number))
            return valued
        valued.milli, valued.exact = number, total
        valued.status = PROVISIONAL if valued.acceptances else CONFIRMED
        return valued


def evaluate(run: Run, prices: Prices, wanted: set[str] | None = None) -> dict[str, Valued]:
    """Each value of exports.json (those of `wanted` only, when given), by id, computed over the run and the prices. The
    values' references must have been checked (`problems`)."""
    found: dict[str, Valued] = {}
    for index, definition in enumerate(run.exports.doc.get("values", [])):
        if (wanted is None or definition["id"] in wanted) and definition["id"] not in found:
            found[definition["id"]] = _Evaluation(run, prices, index, definition).compute()
    return found


def exports_sha(run: Run) -> str:
    """The sha256 of exports.json's bytes, as values.json records it."""
    return digest(read(Path(run.exports.path), readers.EXPORTS.name))


def _document(run: Run, valued: Valued, entry: dict[str, Any], prices: Prices) -> dict[str, Any]:
    """The provenance document of a value, a pending one too: a node for the value (its entry in exports.json, which
    reads the outputs and flow amounts it computes with and the prices it counts in), the trees of what it read, and a
    node per price whose entry of prices.json it read, as prices.json has it, pending or not (the price's own provenance
    is its file there). Of a pending value it holds what it could read, and `result` says it is pending and why."""
    by_pool = prices.by_pool
    nodes = dict(valued.nodes)
    for pool in valued.priced:
        price = by_pool[pool]
        nodes["price:" + pool] = {
            "node": "price", "resource": pool, "file": relative(run, str(prices.path)), "sha256": prices.sha,
            "class": price["class"], "status": price["status"], "price": price["price"], "milli": price["milli"],
            "provenance": price["provenance"], "reads": [],
        }
    nodes[valued.output] = {
        "node": "value", "id": valued.id, "file": relative(run, run.exports.path), "ref": valued.ref,
        "sha256": sha256(valued.definition),
        "reads": sorted(set(valued.roots) | {"price:" + pool for pool in valued.priced}),
    }
    doc = {
        "schema_version": PROVENANCE_FORMAT,
        "model_id": run.head["model_id"],
        "output": valued.output,
        "result": {key: value for key, value in entry.items() if key != "provenance"},
        "versions": versions(run.checker, run.folder),
        "nodes": dict(sorted(nodes.items())),
    }
    if valued.acceptances:
        doc["acceptances"] = [valued.acceptances[name] for name in sorted(valued.acceptances)]
    return doc


def build(run: Run) -> tuple[dict[str, bytes], dict[str, Any]]:
    """The files of the export, by their path relative to the model folder and in the order they are written
    (`values.json` last), and the counts `data` shows. A refusal raises `Failure`, and nothing is written."""
    if "values" not in run.exports.doc:  # an export of nothing is a mistake, as `export prices` without κ
        raise Failure(refusal("member_missing", run.exports.path, readers.EXPORTS.name, "/values", key="values"))
    prices = current_prices(run)
    _raise(problems(run))
    found = evaluate(run, prices)
    _raise([entry for valued in found.values() for entry in valued.refusals])
    files: dict[str, Any] = {}
    entries = []
    for name in sorted(found):  # every value's provenance file, a pending one's too: none stays of an earlier export
        valued = found[name]
        entry = valued.entry()
        files[entry["provenance"]] = _document(run, valued, entry, prices)
        entries.append(entry)
    counts = {status: sum(1 for entry in entries if entry["status"] == status) for status in (CONFIRMED, PROVISIONAL, PENDING)}
    counts["partial"] = sum(1 for entry in entries if entry["partial"])
    files[VALUES] = {
        "schema_version": SCHEMA_VERSION,
        "model_id": run.head["model_id"],
        "versions": versions(run.checker, run.folder),
        "prices": {"file": PRICES, "sha256": prices.sha, "versions": prices.doc["versions"]},
        "exports": {"file": relative(run, run.exports.path), "sha256": exports_sha(run)},
        "unit_milli": prices.doc["unit_milli"],
        "values": entries,
        "counts": counts,
    }
    return {name: delivered(doc) for name, doc in files.items()}, {"counts": counts}


def export_values(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, unknowns: Unknowns = Unknowns()
) -> dict[str, Any]:
    """Evaluate the model for the state, then write the provenance files of the exported values and `values.json`, or
    refuse."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    files, data = build(run)
    written = []
    for name, payload in files.items():  # values.json last
        target = run.folder / name
        write_file(target, payload)
        written.append(str(target))
    return {**run.head, **data, "written": sorted(written)}
