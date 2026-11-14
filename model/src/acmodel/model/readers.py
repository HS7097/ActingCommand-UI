"""The readers of the model's input files (contracts/formats.md).

Each reader returns the file's JSON document once it is fully checked: the JSON
level, the `schema_version`, every member's type, the id grammar, and no unknown
or missing member. Otherwise it raises `Failure` with one `model_file_invalid`
error per problem, each with the JSON pointer of the member in `ref`.

Meaning across files (references, duplicate ids, status values, provenance
present, expressions) is the static check's, not the readers'.
"""

from __future__ import annotations

from fractions import Fraction
from pathlib import Path
from typing import Any, Callable

from acmodel import ids
from acmodel.envelope import Failure
from acmodel.model.jsonfile import MAX_SAFE_INTEGER, load, parse, pointer
from acmodel.model.shape import (
    AnyValue,
    Boolean,
    Choice,
    Context,
    Date,
    Either,
    Integer,
    ListOf,
    MapOf,
    Object,
    Spec,
    Tagged,
    Text,
    exactly_one,
)

MAX = MAX_SAFE_INTEGER

# Grammars (formats.md, "Ids"), shared with the expression lexer.
ID = Text(ids.ID, "id_invalid")
UNKNOWN_ID = Text(ids.UNKNOWN_ID, "id_invalid")
FACT_KEY = Text(ids.FACT_KEY, "id_invalid")
TEXT = Text()
EXPR = Text()  # parsed by the expression language, not here
DATE = Date()
ANY = AnyValue()
BOOLEAN = Boolean()
SHA256 = Text(r"[0-9a-f]{64}")
VERSION = Text(r"[0-9]+(\.[0-9]+)*")
POINTER = Text(r"(/([^~/]|~[01])*)+")
DIMENSION = Text(r"none|time|count|value|amount:" + ids.ID)
INTEGER_TEXT = Text(r"-?(0|[1-9][0-9]*)")
DECIMAL_TEXT = Text(r"-?(0|[1-9][0-9]*)(\.[0-9]+)?")
SEGMENTS = r"([a-z0-9][a-z0-9_.-]*/)*[a-z0-9][a-z0-9_.-]*\.json"
NOTE = {"note": TEXT}

# The role of a measurement, or of a guide row that is a live reading (#424 ACM-PROV): `supplement`, a neutral
# constant that no public source gives, or `behaviour`, a confirmation of how the game behaves, which is never
# the provenance of a number. The class of a source of sources.json.
ROLES = ("supplement", "behaviour")
SOURCE_CLASSES = ("wiki", "drop_stats", "datamine", "official")
# A supplement names the source ids it searched without finding the value, and a drop rate its samples.
SUPPLEMENT_MEMBERS = ("searched", "samples", "successes")
ROLE_MEMBERS = {
    "role": Choice(*ROLES),
    "searched": ListOf(ID),
    "samples": Integer(1, MAX),
    "successes": Integer(0, MAX),
}


def _role(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """The members of a role: `supplement` needs `searched`, and takes `samples` and `successes` together (at
    most `samples`); `behaviour` takes none of them, and neither does an entry without a role (the static check
    warns of that one)."""
    present = [name for name in SUPPLEMENT_MEMBERS if name in value]
    if value.get("role") != "supplement":
        if present and "role" not in value:
            return ctx.refuse("member_missing", pointer(ref, "role"), key="role")
        for name in present:
            ctx.refuse("member_unknown", pointer(ref, name), key=name)
        return not present
    ok = True
    if "searched" not in value:
        ok = ctx.refuse("member_missing", pointer(ref, "searched"), key="searched")
    pair = [name for name in ("samples", "successes") if name in value]
    if len(pair) == 1:
        other = "successes" if pair[0] == "samples" else "samples"
        ok = ctx.refuse("member_missing", pointer(ref, other), key=other, expected=["samples", "successes"])
    elif pair and value["successes"] > value["samples"]:
        ok = ctx.refuse("value_invalid", pointer(ref, "successes"), literal=str(value["successes"]))
    return ok


def _guide(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A guide row names the class underneath it: a source of sources.json, or the role of the reading it is.
    Not both; a source takes none of a role's members."""
    if "source" not in value:
        return _role(value, ref, ctx)
    if "role" in value:
        return ctx.refuse("member_conflict", pointer(ref, "role"), key="role", expected=["source", "role"])
    present = [name for name in SUPPLEMENT_MEMBERS if name in value]
    for name in present:
        ctx.refuse("member_unknown", pointer(ref, name), key=name)
    return not present


PROVENANCE = ListOf(
    Tagged(
        "kind",
        {
            "source": Object({"source": ID, "locator": TEXT}, {"revision": TEXT, "quote": TEXT}),
            "ruling": Object({"ref": TEXT, "where": TEXT, "date": DATE}),
            "measurement": Object({"ref": TEXT, "date": DATE}, {"by": TEXT, "evidence": TEXT, **ROLE_MEMBERS}, _role),
            "guide": Object({"ref": TEXT}, {"source": ID, **ROLE_MEMBERS}, _guide),
            "logic": Object({"ref": TEXT}),
            "pending_check": Object({"ref": TEXT}, {"by": TEXT}),
        },
    )
)

COLUMN = Tagged(
    "type",
    {
        "integer": Object({"dimension": DIMENSION}),
        "decimal": Object({"dimension": DIMENSION}),
        "boolean": Object({}),
        "enum": Object({"values": ListOf(ID, min_items=1)}),
        "id": Object({}),
    },
)

ENTITY = Tagged(
    "kind",
    {
        "resource": Object({"id": ID}, {"reading": FACT_KEY, **NOTE}),
        "store": Object({"id": ID, "holds": ID}, {"reading": FACT_KEY, **NOTE}),
        "activity": Object({"id": ID}, NOTE),
        "entity_class": Object({"id": ID, "attributes": MapOf(ID, COLUMN)}, NOTE),
        "window": Object(
            {"id": ID, "starts_at_unix_s": Integer(0, MAX), "ends_at_unix_s": Integer(0, MAX)}, NOTE
        ),
        # The unit values are counted in (#424 ACM-PRICE 2.2 (1)): the numeraire. No reading, no pool, never bound.
        "value": Object({"id": ID}, NOTE),
    },
)


def _at_most_samples(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """`successes` at most `samples`."""
    if value["successes"] > value["samples"]:
        return ctx.refuse("value_invalid", pointer(ref, "successes"), literal=str(value["successes"]))
    return True


def _drop(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """`rate`, or `samples` with `successes` (at most `samples`)."""
    if "rate" in value:
        extra = [name for name in ("samples", "successes") if name in value]
        if extra:
            return ctx.refuse("member_conflict", pointer(ref, extra[0]), key=extra[0], expected=["rate", "samples"])
        return True
    for name in ("samples", "successes"):
        if name not in value:
            return ctx.refuse("member_missing", pointer(ref, name), key=name, expected=["rate", "samples"])
    return _at_most_samples(value, ref, ctx)


AMOUNTS = MapOf(ID, EXPR)
ON_REACH = Either(
    string=Choice("block_all_income", "stop_production", "discard", "void_at_reset"),
    object=Object({"block_income_from": ListOf(ID), "other_income": Choice("notice_only")}),
)
_VALUE_OR_READING = exactly_one("value", "reading")


def _cap(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A cap's `reference`, its value in the reference state, goes with a `reading` (#424 ACM-PRICE G-A), and its
    `per`, the period at whose resets it voids, with `on_reach` `void_at_reset` (G-C)."""
    ok = True
    if "reference" in value and "reading" not in value:
        ok = ctx.refuse("member_conflict", pointer(ref, "reference"), key="reference", expected=["value", "reference"])
    if "per" in value and value["on_reach"] != "void_at_reset":
        ok = ctx.refuse("member_unknown", pointer(ref, "per"), key="per") and ok
    return ok


CAP_MEMBERS = {"reference": EXPR, "per": ID}
CAP_BY_VALUE = Object(
    {"on_reach": ON_REACH}, {"value": EXPR, "reading": FACT_KEY, **CAP_MEMBERS},
    lambda value, ref, ctx: _VALUE_OR_READING(value, ref, ctx) and _cap(value, ref, ctx),
)
COMPARISON = Choice(
    "eq", "not_eq", "less_than", "less_than_or_equal", "greater_than", "greater_than_or_equal"
)
# The account condition a statement holds under (#424 ACM-PRICE PRICE-2b-2, addendum 2.1 §3.2 and §3.7): a text,
# and the neutral fact keys the resource side checks it with on the instances.
FACTS = ListOf(FACT_KEY, min_items=1)


def _together(*names: str) -> Callable[[dict[str, Any], str, Context], bool]:
    """A rule: the optional members `names` are all present or all absent (the absent one is `member_missing`)."""

    def rule(value: dict[str, Any], ref: str, ctx: Context) -> bool:
        present = [name for name in names if name in value]
        if not present or len(present) == len(names):
            return True
        missing = next(name for name in names if name not in value)
        return ctx.refuse("member_missing", pointer(ref, missing), key=missing, expected=list(names))

    return rule


def _estimate(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A `pt.*` estimate (addendum 2.1 §3.7, check 1): exactly one of `of` and the pair `low` and `high`, which go
    together."""
    ends = [name for name in ("low", "high") if name in value]
    if "of" in value and ends:
        return ctx.refuse("member_conflict", pointer(ref, ends[0]), key=ends[0], expected=["of", "low"])
    if "of" not in value and not ends:
        return ctx.refuse("member_missing", pointer(ref, "of"), key="of", expected=["of", "low"])
    return _together("low", "high")(value, ref, ctx)


# The estimate of a provisional derive (§3.7): what range its point value stands for, on what basis, for which
# activities and under which account condition. Only the impact assessment and the coverage report read it.
ESTIMATE_BASES = ("variants", "samples", "statistics", "design")
ESTIMATE = Object(
    {
        "basis": Choice(*ESTIMATE_BASES),
        "covers": Object({"activities": ListOf(ID, min_items=1)}, {"holds_when": TEXT, "facts": FACTS}),
    },
    {"of": EXPR, "low": EXPR, "high": EXPR},
    _estimate,
)

LOGIC: dict[str, Spec] = {
    "cap": Tagged(
        "cap_kind",
        {
            "hard": CAP_BY_VALUE,
            "storage": CAP_BY_VALUE,
            "soft_read": Object({"reading": FACT_KEY, "on_reach": ON_REACH}, CAP_MEMBERS, _cap),
            # What the activities `from` (absent: all that bring the resource in) may earn per window of `per`
            # (#424 ACM-PRICE 3.3).
            "income": Object({"value": EXPR, "per": ID, "on_reach": ON_REACH}, {"from": ListOf(ID)}),
        },
    ),
    "regen": Object({"amount": EXPR, "every": EXPR}, {"while": EXPR}),
    "flow": Object(
        {},
        {
            "consumes": AMOUNTS,
            "produces": AMOUNTS,
            "moves": Object({"from": ID, "to": ID, "amount": EXPR}),
            "counts_on": Choice("success", "attempt"),
            "batch": Object({"min": EXPR, "max": EXPR}),
            "requires": ListOf(ID),
        },
    ),
    "exchange": Object({"gives": AMOUNTS, "takes": AMOUNTS}, {"limit": EXPR}),
    "limit": Object({"max": EXPR, "per": ID}, {"shared_with": ListOf(ID)}),
    "cooldown": Object({"min_interval": EXPR}),
    "gate": Object(
        {"irreversible": BOOLEAN, "origin": Choice("any", "person")},
        {"requires": EXPR, "prefs": ListOf(ID)},
    ),
    "random": Object(
        {},
        {
            "pity_chain": Object(
                {"rate": EXPR, "hard_pity": EXPR, "carry_over": BOOLEAN},
                {"p_up": EXPR, "guarantee_after_miss": BOOLEAN, "spark": EXPR},
            ),
            "drop": Object({}, {"rate": EXPR, "samples": Integer(1, MAX), "successes": Integer(0, MAX)}, _drop),
            "outcomes": ListOf(Object({"p": EXPR, "then": AMOUNTS}), min_items=1),
        },
        exactly_one("pity_chain", "drop", "outcomes"),
    ),
    "derive": Object(
        {"value": EXPR},
        {
            "round": Choice("floor", "ceil", "half_even"),
            "general_logic": EXPR,
            "live": Object(
                {"fact": FACT_KEY, "comparison": COMPARISON, "reference_fact": FACT_KEY, "offset": EXPR}
            ),
            "estimate": ESTIMATE,
        },
    ),
    "engine": Object(
        {
            "engine": ID,
            "op": Choice("evaluate", "predict", "search"),
            "inputs": MapOf(ID, EXPR),
            "metrics": ListOf(ID),
        }
    ),
    # A goal sink (#424 ACM-PRICE 2.2 (2), (3); addendum 2.1 §4): a main goal is a resource worth `value` value
    # units each; the secondary goal is the reference demand, its steps (`demand`) joined by key to their costs
    # (`costs`), each point worth `value`, over `horizon`.
    "objective": Tagged(
        "tier",
        {
            "main": Object({"value": EXPR}),
            "secondary": Object({"demand": ID, "costs": ID, "value": EXPR, "horizon": EXPR}),
        },
    ),
    "weight_ref": Object({"file": TEXT, "pointer": POINTER, "compares_to": ID}),
    # A declared zero (#424 ACM-PRICE PRICE-2b-2, addendum 2.1 §3.2): the subject, a pool, has no use; why, and
    # the condition it holds under (absent: always) with the fact keys that check it, which go together.
    "no_use": Object({"reason": TEXT}, {"holds_when": TEXT, "facts": FACTS}, _together("holds_when", "facts")),
}


def _drop_samples(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A supplemented drop rate (#424 ACM-PROV): each `supplement` in the provenance of a `random` rule whose
    `drop` is a `rate` carries the `samples` and `successes` it rests on. A drop from `samples` holds them
    already."""
    if "rate" not in value["logic"].get("drop", {}):
        return True
    ok = True
    for index, item in enumerate(value["provenance"]):
        if item.get("role") == "supplement" and "samples" not in item:
            where = pointer(pointer(pointer(ref, "provenance"), index), "samples")
            ok = ctx.refuse("member_missing", where, key="samples", expected=["samples", "successes"])
    return ok


def _estimated(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """An `estimate` only on a provisional derive (addendum 2.1 §3.7): on any other it is `member_unknown`."""
    if "estimate" in value["logic"] and value["status"] != "provisional":
        return ctx.refuse("member_unknown", pointer(pointer(ref, "logic"), "estimate"), key="estimate")
    return True


RULE_CHECKS = {"random": _drop_samples, "derive": _estimated}
RULE = Tagged(
    "kind",
    {
        kind: Object(
            {"id": ID, "subject": ID, "logic": logic, "provenance": PROVENANCE, "status": TEXT},
            {"when": EXPR, "unknowns": ListOf(UNKNOWN_ID), "supersedes": ListOf(ID), **NOTE},
            RULE_CHECKS.get(kind),
        )
        for kind, logic in LOGIC.items()
    },
)


def _unknown(form: str, spec: Spec) -> Object:
    """An unknown of one kind; its range form is optional here (needing it without one is a run error)."""
    return Object(
        {"id": UNKNOWN_ID, "question": TEXT, "status": TEXT, "provenance": PROVENANCE},
        {form: spec, **NOTE},
    )


UNKNOWN = Tagged(
    "kind",
    {
        "interval": _unknown("interval", Object({"low": EXPR, "high": EXPR})),
        "variants": _unknown("variants", MapOf(ID, MapOf(ID, EXPR))),
        "distribution": _unknown(
            "distribution",
            Tagged(
                "family",
                {
                    "uniform": Object({"low": EXPR, "high": EXPR}),
                    "triangular": Object({"low": EXPR, "mode": EXPR, "high": EXPR}),
                    "beta_from_samples": Object(
                        {"samples": Integer(1, MAX), "successes": Integer(0, MAX)}, {}, _at_most_samples
                    ),
                    "empirical": Object({"values": ListOf(EXPR, min_items=1)}),
                },
            ),
        ),
    },
)

RESERVED_COLUMNS = ("status", "provenance")
CELLS: dict[str, Spec] = {
    "integer": Either(integer=Integer(-MAX, MAX), string=INTEGER_TEXT),
    "decimal": Either(integer=Integer(-MAX, MAX), string=DECIMAL_TEXT),
    "boolean": BOOLEAN,
    "id": ID,
}


def _cell(column: dict[str, Any]) -> Spec:
    if column["type"] == "enum":
        return Choice(*column["values"])
    return CELLS[column["type"]]


def _rows(doc: dict[str, Any], ref: str, ctx: Context) -> bool:
    """Exactly one of `rows` and `external`; each row holds exactly the key and value columns."""
    if not exactly_one("rows", "external")(doc, ref, ctx):
        return False
    keys = [doc["key"]] if isinstance(doc["key"], str) else doc["key"]
    if len(set(keys)) != len(keys):
        return ctx.refuse("value_invalid", "/key")
    ok = True
    for name in keys:
        if name in RESERVED_COLUMNS:
            ok = ctx.refuse("value_invalid", "/key", literal=name)
    for name in doc["columns"]:
        if name in RESERVED_COLUMNS:
            ok = ctx.refuse("value_invalid", pointer("/columns", name), literal=name)
    if not ok or "rows" not in doc:
        return ok
    cells = {name: ID for name in keys}  # a key column without a declared type holds ids
    cells.update({name: _cell(column) for name, column in doc["columns"].items()})
    row = Object({**cells, "status": TEXT}, {"provenance": PROVENANCE})
    return ListOf(row).check(doc["rows"], "/rows", ctx)


def _prefs(doc: dict[str, Any], ref: str, ctx: Context) -> bool:
    """Every key has a default and a kind; exactly the strategy keys have a neutral value."""
    ok = True
    for key in doc["kinds"]:
        if key not in doc["defaults"]:
            ok = ctx.refuse("member_missing", pointer("/defaults", key), key=key)
    for key in doc["defaults"]:
        if key not in doc["kinds"]:
            ok = ctx.refuse("member_missing", pointer("/kinds", key), key=key)
    strategy = [key for key, kind in doc["kinds"].items() if kind == "strategy"]
    for key in doc["neutral"]:
        if key not in strategy:
            ok = ctx.refuse("member_unknown", pointer("/neutral", key), key=key)
    for key in strategy:
        if key not in doc["neutral"]:
            ok = ctx.refuse("member_missing", pointer("/neutral", key), key=key)
    return ok


PERIOD = {"id": ID, "reset_minute": Integer(0, 1439), "utc_offset_minutes": Integer(-720, 840)}
WEEKDAY = Choice("mon", "tue", "wed", "thu", "fri", "sat", "sun")


class Format:
    def __init__(self, name: str, version: str | int, shape: Object) -> None:
        self.name = name
        self.version = version
        self.shape = shape


MODEL = Format(
    "acmodel.model.v1",
    "acmodel.model.v1",
    Object(
        {
            "schema_version": ANY,
            "model_id": ID,
            "files": Object(
                {"rules": ListOf(Text("rules/" + SEGMENTS)), "data": ListOf(Text("data/" + SEGMENTS))}
            ),
            "periods": ListOf(
                Tagged(
                    "unit",
                    {
                        "day": Object(PERIOD, NOTE),
                        "week": Object({**PERIOD, "reset_weekday": WEEKDAY}, NOTE),
                        "month": Object({**PERIOD, "reset_day": Integer(1, 28)}, NOTE),
                        # A period of a fixed number of days, such as a 14-day season (#424 ACM-PRICE 3.4).
                        "days": Object({**PERIOD, "length": Integer(2, 366), "anchor_date": DATE}, NOTE),
                    },
                )
            ),
            "numeraire": ID,
            "engines": ListOf(
                Object({"engine_id": ID, "interface": Choice("acmodel.engine.v2"), "min_version": VERSION})
            ),
        },
        NOTE,
    ),
)

RULES = Format(
    "acmodel.rules.v1",
    "acmodel.rules.v1",
    Object({"schema_version": ANY, "entities": ListOf(ENTITY), "rules": ListOf(RULE), "unknowns": ListOf(UNKNOWN)}),
)

DATA = Format(
    "acmodel.data.v1",
    "acmodel.data.v1",
    Object(
        {
            "schema_version": ANY,
            "table": ID,
            "key": Either(string=ID, array=ListOf(ID, min_items=1)),
            "columns": MapOf(ID, COLUMN),
        },
        {
            "rows": ListOf(ANY),
            "external": Object(
                {"kind": Choice("external_dataset"), "source": ID, "sha256": SHA256, "fetched_at": TEXT}
            ),
            "provenance": PROVENANCE,
            **NOTE,
        },
        _rows,
    ),
)

SOURCES = Format(
    "acmodel.sources.v1",
    "acmodel.sources.v1",
    Object(
        {
            "schema_version": ANY,
            "sources": ListOf(
                Object({"id": ID, "title": TEXT}, {"url": TEXT, "class": Choice(*SOURCE_CLASSES), **NOTE})
            ),
        }
    ),
)

STATE = Format(
    "acmodel.state.v1",
    "acmodel.state.v1",
    Object(
        {
            "schema_version": ANY,
            "model_id": ID,
            "label": ID,
            "readings": MapOf(
                FACT_KEY,
                Object({"value": Integer(-MAX, MAX), "observed_at_unix_ms": Integer(0, MAX), "source": TEXT}),
            ),
            "entities": MapOf(ID, MapOf(ID, MapOf(ID, Either(integer=ANY, string=ANY, boolean=ANY)))),
            "cleared": ListOf(ID),
        }
    ),
)

PREFS_DEFAULTS = Format(
    "prefs.defaults",
    1,
    Object(
        {
            "schema_version": ANY,
            "defaults": MapOf(ID, ANY),
            "kinds": MapOf(ID, Choice("safety", "strategy")),
            "neutral": MapOf(ID, ANY),
        },
        {"game": TEXT, "schema": TEXT, **NOTE},
        _prefs,
    ),
)


ROUND = Choice("floor", "ceil", "half_even")
# An output of the run: `rule:<id>`, `general_logic:<id>` or `threshold:<id>`.
OUTPUT_ID = Text(r"(rule|general_logic|threshold):" + ids.ID, "id_invalid")
# What an acceptance may accept: an output of the run, or the price of a pool (`price:<id>`, PRICE-4a).
ACCEPTED_ID = Text(r"(rule|general_logic|threshold|price):" + ids.ID, "id_invalid")
# Where a delivered value comes from: `{"$model": "<output id>"}`. A table's column declares how a number is
# scaled and rounded; a scoring template's placeholder declares it itself (`export.scoring`).
MODEL_CELL = Object({"$model": OUTPUT_ID})
# The columns whose cells the model fills; the cells of the others are literal strings (identities, not quantities).
FILLED_COLUMNS = ("integer", "boolean")
_COLUMN_OPTIONS = {"nullable": BOOLEAN, "valid_until": ID, **NOTE}
EXPORT_COLUMN = Tagged(
    "type",
    {
        "key": Object({"name": ID}, NOTE),
        "enum_string": Object({"name": ID, "allowed": ListOf(TEXT)}, _COLUMN_OPTIONS),
        "integer": Object(
            {"name": ID, "min": Integer(-MAX, MAX), "max": Integer(-MAX, MAX), "scale": Integer(1, MAX), "round": ROUND},
            _COLUMN_OPTIONS,
        ),
        "boolean": Object({"name": ID}, _COLUMN_OPTIONS),
    },
)


def _export_rows(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """Each column name once; each row holds exactly the columns, a `$model` cell in a column the model fills
    and a literal string in the others."""
    ok, seen = True, set()
    for index, column in enumerate(value["columns"]):
        if column["name"] in seen:
            ok = ctx.refuse("value_invalid", f"{ref}/columns/{index}/name", literal=column["name"])
        seen.add(column["name"])
    if not ok:
        return False
    cells = {column["name"]: MODEL_CELL if column["type"] in FILLED_COLUMNS else TEXT for column in value["columns"]}
    return ListOf(Object(cells)).check(value["rows"], pointer(ref, "rows"), ctx)


EXPORT_TABLE = Object(
    {"table_id": ID, "game": TEXT, "row_key": ListOf(ID), "columns": ListOf(EXPORT_COLUMN), "rows": ListOf(ANY)},
    NOTE,
    _export_rows,
)

PRICE_OUTPUT = "price:"
# Where an acceptance holds (#424 ACM-PRICE addendum 2.1 §3.2, §3.4; PRICE-4a): the whole game, or the aliases of the
# instances the condition was verified on.
SCOPE = Either(string=Choice("game"), array=ListOf(ID, min_items=1))
# The members an acceptance of a price adds to its `accept_provisional`: the digest of the price's provenance file it
# binds, its scope and the activities whose class flips in the impact assessment.
PRICE_ACCEPTANCE = {"binds": SHA256, "scope": SCOPE, "flips": ListOf(ID)}


def _acceptance(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """An acceptance (#424 design 6.6; addendum 2.1 §3.2, §3.4, PRICE-4a): exactly one of `accept_provisional` and
    `accept_zero`. Of a price (`price:<pool>`), `accept_provisional` holds `binds`, `scope` and `flips` too, and
    `accept_zero` may stand instead; of any other output, `accept_provisional` holds `ref` and `until` only."""
    if not exactly_one("accept_provisional", "accept_zero")(value, ref, ctx):
        return False
    price = value["output"].startswith(PRICE_OUTPUT)
    if not price and "accept_zero" in value:
        return ctx.refuse("member_unknown", pointer(ref, "accept_zero"), key="accept_zero")
    if "accept_provisional" not in value:
        return True
    where, accepted, ok = pointer(ref, "accept_provisional"), value["accept_provisional"], True
    for name in PRICE_ACCEPTANCE:
        if price and name not in accepted:
            ok = ctx.refuse("member_missing", pointer(where, name), key=name)
        elif not price and name in accepted:
            ok = ctx.refuse("member_unknown", pointer(where, name), key=name)
    return ok


ACCEPTANCE = Object(
    {"output": ACCEPTED_ID},
    {
        "accept_provisional": Object({"ref": TEXT, "until": TEXT}, PRICE_ACCEPTANCE),
        "accept_zero": Object({"ref": TEXT, "scope": SCOPE, "until": TEXT}),
        **NOTE,
    },
    _acceptance,
)

# #464 W1: the values of the schedule's rules (`acmodel export values`). A quantity is an exact number (an integer, a
# decimal or a ratio of two integers) or `rule:<id>`, the value of a rule of the run; how often (`times`) is never
# negative (only an `amount` is: what a run takes out) and, written as a number, above 0 (`_term`), and a count of runs is
# a positive exact number (`_value`). `per` is a period of model.json or `run` (once per run of the rule).
_NUMBER = r"(0|[1-9][0-9]*)(\.[0-9]+|/[1-9][0-9]*)?"
QUANTITY = Text(r"-?" + _NUMBER + r"|rule:" + ids.ID)
TIMES = Text(_NUMBER + r"|rule:" + ids.ID)
POSITIVE = Text(_NUMBER)
PER_RUN = "run"


def _term(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A term of a value: exactly one of `activity` (an activity, run `times`) and `resource` (a resource, `amount` of it
    `times`); `amount` belongs to a resource's term only; `times`, written as a number, above 0 (`value_invalid`, as a
    count of runs of 0: a term that happens no times is no term, and a negative one would turn what comes in into what
    goes out)."""
    if not exactly_one("activity", "resource")(value, ref, ctx):
        return False
    ok = True
    times = value["times"]
    if not times.startswith("rule:") and Fraction(times) <= 0:
        ok = ctx.refuse("value_invalid", pointer(ref, "times"), literal=times)
    if "resource" in value and "amount" not in value:
        return ctx.refuse("member_missing", pointer(ref, "amount"), key="amount")
    if "activity" in value and "amount" in value:
        return ctx.refuse("member_unknown", pointer(ref, "amount"), key="amount")
    return ok


def _value(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A value: its count of runs above 0; each pool of `cost_paid_by_pool` and of `exclude` named once, and none in
    both."""
    ok = True
    if Fraction(value["runs"]["count"]) <= 0:
        ok = ctx.refuse("value_invalid", pointer(pointer(ref, "runs"), "count"), literal=value["runs"]["count"])
    seen: set[str] = set()
    named = [(pointer(pointer(ref, "cost_paid_by_pool"), index), pool)
             for index, pool in enumerate(value.get("cost_paid_by_pool", []))]
    named += [(pointer(pointer(pointer(ref, "exclude"), index), "resource"), entry["resource"])
              for index, entry in enumerate(value.get("exclude", []))]
    for where, pool in named:
        if pool in seen:
            ok = ctx.refuse("value_invalid", where, literal=pool)
        seen.add(pool)
    return ok


VALUE_TERM = Object(
    {"times": TIMES, "per": ID},
    {"activity": ID, "resource": ID, "amount": QUANTITY, "ref": TEXT, **NOTE},
    _term,
)
VALUE = Object(
    {
        "id": ID,
        "terms": ListOf(VALUE_TERM, min_items=1),
        "runs": Object({"count": POSITIVE, "per": ID, "ref": TEXT}, NOTE),
        "round": ROUND,
    },
    {
        "cost_paid_by_pool": ListOf(ID),
        "exclude": ListOf(Object({"resource": ID, "reason": TEXT, "ref": TEXT}, NOTE)),
        **NOTE,
    },
    _value,
)

EXPORTS = Format(
    "acmodel.exports.v1",
    "acmodel.exports.v1",
    Object(
        {
            "schema_version": ANY,
            "thresholds": ListOf(
                Object(
                    {"id": ID, "from": ID, "form": Choice("constant", "fact_offset")},
                    {"round": ROUND, **NOTE},
                )
            ),
            "tables": ListOf(EXPORT_TABLE),
            "scoring": ListOf(Object({"id": ID, "template": Text(SEGMENTS)}, NOTE)),
        },
        {
            "acceptances": ListOf(ACCEPTANCE),
            # The game's price unit (#424 ACM-PRICE 4.2, PRICE-4a): one value unit is `unit_milli` thousandths of a
            # priority. One per game; prices of two games are never compared (137 K2).
            "prices": Object({"unit_milli": Integer(1, MAX)}, NOTE),
            # #464 W1: what each value of the schedule is made of (`acmodel export values`); absent means none.
            "values": ListOf(VALUE),
            **NOTE,
        },
    ),
)


SCHEDULE_FILE = "schedule.json"
# A file of the resource repository, relative to its root: `/`-separated segments, ending in `.json`.
REPO_FILE = Text(r"([A-Za-z0-9_-][A-Za-z0-9_.-]*/)*[A-Za-z0-9_-][A-Za-z0-9_.-]*\.json")


# The members of a literal that name a rule or a pool of `schedule.json`.
SCHEDULE_ITEMS = {"rule_id": FACT_KEY, "pool_id": FACT_KEY}


def _schedule_item(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """`rule_id` names a rule and `pool_id` a pool of `schedule.json`: exactly one of them is there when `file`
    is `schedule.json`, and neither otherwise."""
    names = list(SCHEDULE_ITEMS)
    if value["file"] == SCHEDULE_FILE:
        return exactly_one(*names)(value, ref, ctx)
    present = [name for name in names if name in value]
    for name in present:
        ctx.refuse("member_unknown", pointer(ref, name), key=name)
    return not present


# #464 W1: a binding of a value (`value`, a value of exports.json) names the fixed value of a rule of schedule.json.
VALUE_PATH = "/fixed_value_milli"


def _binding(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A binding names exactly one of a threshold and a value. A threshold's literal is a rule's or a pool's in
    `schedule.json`, or one of a pack file (`_schedule_item`); a value's is the fixed value of a rule of `schedule.json`:
    `file` is `schedule.json`, `rule_id` names the rule and `path` is `/fixed_value_milli`."""
    if not exactly_one("threshold", "value")(value, ref, ctx):
        return False
    if "threshold" in value:
        return _schedule_item(value, ref, ctx)
    ok = True
    if value["file"] != SCHEDULE_FILE:
        ok = ctx.refuse("value_invalid", pointer(ref, "file"), expected=[SCHEDULE_FILE], literal=value["file"])
    if "pool_id" in value:
        ok = ctx.refuse("member_unknown", pointer(ref, "pool_id"), key="pool_id")
    if "rule_id" not in value:
        ok = ctx.refuse("member_missing", pointer(ref, "rule_id"), key="rule_id")
    if value["path"] != VALUE_PATH:
        ok = ctx.refuse("value_invalid", pointer(ref, "path"), expected=[VALUE_PATH], literal=value["path"])
    return ok


LITERAL = {"file": REPO_FILE, "path": POINTER}
BINDINGS = Format(
    "acmodel.bindings.v1",
    "acmodel.bindings.v1",
    Object(
        {
            "schema_version": ANY,
            "bindings": ListOf(Object(LITERAL, {"threshold": ID, "value": ID, **SCHEDULE_ITEMS, **NOTE}, _binding)),
            "fixed": ListOf(
                Object({**LITERAL, "reason": TEXT, "ref": TEXT}, {**SCHEDULE_ITEMS, **NOTE}, _schedule_item)
            ),
        },
        NOTE,
    ),
)

# `out/prices.json` (#424 ACM-PRICE PRICE-4a), which `export prices` writes (`acmodel.prices.v2`) and `bindings check`
# and `verify prices` read back. The members they read are checked; the analysis they only pass on is any JSON. A file
# of `acmodel.prices.v1`, which `export prices` wrote before, is still read, by its own format.
POOL_CLASSES = ("covered", "structural_gap", "verified_zero", "unknown")
PENDING_REASONS = (
    "structural_gap", "unknown", "withheld", "estimate_missing", "not_accepted", "acceptance_stale",
    "acceptance_incomplete", "scope_partial",
)
NULLABLE_TEXT = Either(string=TEXT, null=ANY)
MODEL_VERSIONS = Object({"rules": SHA256, "data": SHA256, "tool": VERSION})
# A pool's entry in v1. v2 has no `estimates`, the provisional derives its component read, which every pool of that
# component repeated: the top-level `estimates` names each derive's `components`.
PRICE_ENTRY = {
    "pool": ID, "class": Choice(*POOL_CLASSES), "status": Choice("confirmed", "provisional"),
    "reasons": ListOf(TEXT), "component": ID, "price": TEXT, "milli": NULLABLE_TEXT,
    "pending": ListOf(Choice(*PENDING_REASONS)), "joint": ANY, "degenerate": BOOLEAN,
    "varies_by_step": BOOLEAN, "estimates": ListOf(ID), "impact": ANY, "flips": ListOf(ID),
    "binds": SHA256, "acceptance": ANY, "provenance": TEXT,
}


def _prices(version: str, entry: dict[str, Spec], optional: dict[str, Spec] | None = None) -> Format:
    """`out/prices.json` of one version, whose pool entries hold `entry`, with the `optional` top-level members."""
    return Format(
        version,
        version,
        Object(
            {
                "schema_version": ANY,
                "model_id": ID,
                "versions": MODEL_VERSIONS,
                # The trees the price documents share (PRICE-4a2): each root's id to the digest of its tree document,
                # the versions it was computed from, and how many nodes it holds. The tree itself is kept under the
                # state root.
                "shared": MapOf(TEXT, Object({"sha256": SHA256, "versions": MODEL_VERSIONS, "nodes": Integer(1, MAX)})),
                "unit_milli": Integer(1, MAX),
                "prices": ListOf(Object(entry)),
                "slots": ListOf(ID),
                "estimates": ANY,
                "impact": ANY,
                "counts": ANY,
            },
            optional,
        ),
    )


# #424 PRICE-REUSE: what computed the steady state and its assessment besides the model's inputs (`core.evaluator`):
# the digest of the tool's source, the libraries the steady state's node declares and the configuration (every key but
# those that never change a result and the plan's, by its dotted name: an integer, its decimal text when it is beyond
# the safe range, a string or a boolean), and the digest of those three. Every v2 file `export prices` writes from
# PRICE-REUSE on holds it.
EVALUATOR = Object({
    "sha256": SHA256, "source": SHA256, "libraries": MapOf(TEXT, TEXT),
    "config": MapOf(TEXT, Either(integer=Integer(0, MAX), string=TEXT, boolean=BOOLEAN)),
})
# A v2 file whose assessment was taken over from an earlier one (`--reuse-assessment`): the sha256 of that file's bytes,
# its versions, the digests of its shared trees and of its evaluator, and the members taken over.
REUSED_ASSESSMENT = Object({
    "prices_sha256": SHA256, "versions": MODEL_VERSIONS, "shared": MapOf(TEXT, SHA256), "evaluator": SHA256,
    "members": ListOf(TEXT),
})
PRICES_V1 = _prices("acmodel.prices.v1", PRICE_ENTRY)
# #430 C-2a: the steady state's own timing (`steady_timing_ms`), analysis only, so any JSON as the other members of the
# analysis are; a v2 file written before it has none.
PRICES = _prices(
    "acmodel.prices.v2",
    {name: spec for name, spec in PRICE_ENTRY.items() if name != "estimates"},
    {"evaluator": EVALUATOR, "reused_assessment": REUSED_ASSESSMENT, "steady_timing_ms": ANY},
)
# #464 W1: `out/values.json`, which `export values` writes and `bindings check` reads back to tell whether it is current:
# the versions of the model, and the sha256 of the bytes of the prices.json and the exports.json it was computed from.
# The values themselves are only passed on.
VALUES = Format(
    "acmodel.values.v1",
    "acmodel.values.v1",
    Object({
        "schema_version": ANY,
        "model_id": ID,
        "versions": MODEL_VERSIONS,
        "prices": Object({"file": TEXT, "sha256": SHA256, "versions": MODEL_VERSIONS}),
        "exports": Object({"file": TEXT, "sha256": SHA256}),
        "unit_milli": Integer(1, MAX),
        "values": ANY,
        "counts": ANY,
    }),
)
# What `export prices --reuse-assessment` takes over from an earlier v2 file (#424 PRICE-REUSE), which the format above
# passes on as any JSON: each estimate's `assessed` and `skipped`, each pool's `impact` and the groups.
_ESTIMATE = Object({
    "rule": ID, "value": ANY, "estimate": ANY, "components": ListOf(ID), "assessed": BOOLEAN,
    "skipped": Either(string=Choice("structural_gap", "estimate_missing"), null=ANY),
})
_IMPACT_RANGE = Either(object=Object({"low": TEXT, "high": TEXT}), null=ANY)
# #430 METHOD-C, C-2a: what each pass of a group taken over holds (contracts/formats.md, "Taking an assessment over",
# step 3), beside its `at` and `values`: `by`, how its vector was found; `refused_by`, only with `impact.certify` true,
# the condition of the certificate that refused a solved pass, null for any other; and `timing_ms`, null or `{stages,
# lps, total}`: `stages` null or each stage of the vector (`core.flow.STAGES`), `lps` each of those stages and
# `certificate`, each with every member of `core.flow.LP_COUNTS`, every number an integer. The names are the ones
# `core.impact` and `core.flow` write.
PASS_BY = ("solved", "certified", "steady_state")
REFUSED_BY = ("program", "c1", "c2", "c3", "c4", "lp_unverified")
PASS_STAGES = (
    "model", "optimum", "acquisition", "value", "dual", "projections", "intervals", "most", "ranges", "selection",
    "max_min", "check",
)
PASS_LPS = (*PASS_STAGES, "certificate")
LP_COUNTS = ("runs", "again_infeasible", "again_other", "highs_ms", "exact_ms")
_COUNT = Integer(0, MAX)
_TIMING = Object({
    "stages": Either(object=Object({stage: _COUNT for stage in PASS_STAGES}), null=ANY),
    "lps": Object({stage: Object({name: _COUNT for name in LP_COUNTS}) for stage in PASS_LPS}),
    "total": _COUNT,
})


def _refused_by(value: dict[str, Any], ref: str, ctx: Context) -> bool:
    """A pass's `refused_by`, where it has one: one of the conditions when its `by` is `solved`, null otherwise."""
    if "refused_by" not in value:
        return True
    found = value["refused_by"]
    if value["by"] == "solved" and found is None:
        return ctx.refuse("value_invalid", pointer(ref, "refused_by"), expected=list(REFUSED_BY), literal="null")
    if value["by"] != "solved" and found is not None:
        return ctx.refuse("value_invalid", pointer(ref, "refused_by"), expected=["null"], literal=found)
    return True


_PASS = Object(
    {"at": ANY, "values": ANY, "by": Choice(*PASS_BY), "timing_ms": Either(object=_TIMING, null=ANY)},
    {"refused_by": Either(string=Choice(*REFUSED_BY), null=ANY)},
    _refused_by,
)
_GROUP = Object({"rules": ANY, "unknowns": ANY, "passes": ListOf(_PASS), "prices": ANY, "flips": ANY})


def _read(path: Path | str, fmt: Format, *older: Format, raw: bytes | None = None) -> dict[str, Any]:
    """The file's document, checked by the format of its own `schema_version`: `fmt`, or one of the `older` formats a
    reader still reads. Any other version is refused with each of theirs expected; that refusal, and those before the
    version is known, name `fmt`. With `raw`, the file's bytes already read, it is not read again."""
    path = Path(path)
    doc = load(path, fmt.name) if raw is None else parse(raw, path, fmt.name)
    ctx = Context(path, fmt.name)
    known = (fmt, *older)
    if not isinstance(doc, dict):
        ctx.refuse("type_invalid", "", expected=["object"])
    elif "schema_version" not in doc:
        ctx.refuse("member_missing", "/schema_version", key="schema_version")
    elif Choice(*(each.version for each in known)).check(doc["schema_version"], "/schema_version", ctx):
        own = next(each for each in known if each.version == doc["schema_version"])
        ctx.fmt = own.name
        own.shape.check(doc, "", ctx)
    if ctx.errors:
        raise Failure(*ctx.errors)
    return doc


def read_model(path: Path | str) -> dict[str, Any]:
    """`model.json` (`acmodel.model.v1`): the files, periods, numeraire and engines."""
    return _read(path, MODEL)


def read_rules(path: Path | str) -> dict[str, Any]:
    """`rules/<topic>.json` (`acmodel.rules.v1`): entities, rules and unknowns."""
    return _read(path, RULES)


def read_data(path: Path | str) -> dict[str, Any]:
    """`data/<table>.json` (`acmodel.data.v1`): one table, inline rows or an external dataset."""
    return _read(path, DATA)


def read_sources(path: Path | str) -> dict[str, Any]:
    """`sources.json` (`acmodel.sources.v1`): the source ids that provenance cites."""
    return _read(path, SOURCES)


def read_state(path: Path | str) -> dict[str, Any]:
    """An account state snapshot (`acmodel.state.v1`), local only."""
    return _read(path, STATE)


def read_prefs_defaults(path: Path | str) -> dict[str, Any]:
    """`ours/prefs/defaults.json` of a resource repository, read as it is (`schema_version` 1)."""
    return _read(path, PREFS_DEFAULTS)


def read_exports(path: Path | str) -> dict[str, Any]:
    """`exports.json` (`acmodel.exports.v1`): the thresholds, the tables and the scoring templates the model
    delivers, and the acceptances of provisional outputs."""
    return _read(path, EXPORTS)


def read_prices(path: Path | str, raw: bytes | None = None) -> dict[str, Any]:
    """`out/prices.json`, as `export prices` wrote it: the members `bindings check` and `verify prices` read. Each file
    is read by its own version: `acmodel.prices.v2`, which `export prices` writes, or `acmodel.prices.v1`, which it
    wrote before (each pool's entry with its `estimates` too). With `raw`, the bytes of the file already read."""
    return _read(path, PRICES, PRICES_V1, raw=raw)


def check_assessment(doc: dict[str, Any], path: Path | str) -> None:
    """The members of an earlier v2 prices.json (`doc`, read by `read_prices`) that `export prices --reuse-assessment`
    takes over (#424 PRICE-REUSE): each estimate as `export prices` lists it, each pool's `impact` (null, or `low` and
    `high`), and the groups (`impact`, a list), each pass of each group with its `by` and its `timing_ms` (#430 C-2a,
    `_PASS`), and either every pass with a `refused_by` or none. Each problem is `model_file_invalid` at its pointer, a
    pass's at that pass: a file written before C-2a (`timing_ms` with `source` or without `lps`, a pass without `by`) is
    never taken over. Only a file taken over is checked so: `verify prices` and `bindings check` read the groups as any
    JSON, so a file written before C-2a is still read and verified."""
    ctx = Context(path, doc["schema_version"])
    ListOf(_ESTIMATE).check(doc["estimates"], "/estimates", ctx)
    ListOf(_GROUP).check(doc["impact"], "/impact", ctx)
    groups = doc["impact"] if isinstance(doc["impact"], list) else []
    found = [
        (pointer(pointer(pointer("/impact", group), "passes"), index), "refused_by" in each)
        for group, entry in enumerate(groups) if isinstance(entry, dict) and isinstance(entry.get("passes"), list)
        for index, each in enumerate(entry["passes"]) if isinstance(each, dict)
    ]
    if any(has for _, has in found):  # every pass has a `refused_by`, or none has
        for where, has in found:
            if not has:
                ctx.refuse("member_missing", pointer(where, "refused_by"), key="refused_by")
    for index, entry in enumerate(doc["prices"]):
        _IMPACT_RANGE.check(entry["impact"], pointer(pointer("/prices", index), "impact"), ctx)
    if ctx.errors:
        raise Failure(*ctx.errors)


def read_bindings(path: Path | str) -> dict[str, Any]:
    """`bindings.json` (`acmodel.bindings.v1`): where each threshold's literal and each value's sits, and the fixed
    literals."""
    return _read(path, BINDINGS)


def read_values(path: Path | str) -> dict[str, Any]:
    """`out/values.json` (`acmodel.values.v1`), as `export values` wrote it: the members `bindings check` reads to tell
    whether it is current (#464 W1)."""
    return _read(path, VALUES)
