"""The readers of the model's input files (contracts/formats.md).

Each reader returns the file's JSON document once it is fully checked: the JSON
level, the `schema_version`, every member's type, the id grammar, and no unknown
or missing member. Otherwise it raises `Failure` with one `model_file_invalid`
error per problem, each with the JSON pointer of the member in `ref`.

Meaning across files (references, duplicate ids, status values, provenance
present, expressions) is the static check's, not the readers'.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from acmodel import ids
from acmodel.envelope import Failure
from acmodel.model.jsonfile import MAX_SAFE_INTEGER, load, pointer
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
    "objective": Object({"numeraire": ID, "terms": ListOf(EXPR), "horizon": EXPR}),
    "weight_ref": Object({"file": TEXT, "pointer": POINTER, "compares_to": ID}),
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


RULE = Tagged(
    "kind",
    {
        kind: Object(
            {"id": ID, "subject": ID, "logic": logic, "provenance": PROVENANCE, "status": TEXT},
            {"when": EXPR, "unknowns": ListOf(UNKNOWN_ID), "supersedes": ListOf(ID), **NOTE},
            _drop_samples if kind == "random" else None,
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
            "acceptances": ListOf(
                Object({"output": OUTPUT_ID, "accept_provisional": Object({"ref": TEXT, "until": TEXT})}, NOTE)
            ),
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


LITERAL = {"file": REPO_FILE, "path": POINTER}
BINDINGS = Format(
    "acmodel.bindings.v1",
    "acmodel.bindings.v1",
    Object(
        {
            "schema_version": ANY,
            "bindings": ListOf(Object({"threshold": ID, **LITERAL}, {**SCHEDULE_ITEMS, **NOTE}, _schedule_item)),
            "fixed": ListOf(
                Object({**LITERAL, "reason": TEXT, "ref": TEXT}, {**SCHEDULE_ITEMS, **NOTE}, _schedule_item)
            ),
        },
        NOTE,
    ),
)


def _read(path: Path | str, fmt: Format) -> dict[str, Any]:
    path = Path(path)
    doc = load(path, fmt.name)
    ctx = Context(path, fmt.name)
    if not isinstance(doc, dict):
        ctx.refuse("type_invalid", "", expected=["object"])
    elif "schema_version" not in doc:
        ctx.refuse("member_missing", "/schema_version", key="schema_version")
    elif Choice(fmt.version).check(doc["schema_version"], "/schema_version", ctx):
        fmt.shape.check(doc, "", ctx)
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


def read_bindings(path: Path | str) -> dict[str, Any]:
    """`bindings.json` (`acmodel.bindings.v1`): where each threshold's literal sits, and the fixed literals."""
    return _read(path, BINDINGS)
