"""The files that hold bound literals: the game root's `schedule.json` and pack policy files.

Each is read once with the tool's strict JSON reader (`model.jsonfile`), and the same bytes are what `apply`
splices. The tool needs only JSON pointers and the literals: whether `schedule.json` is valid as a whole is
the validator's job (`actinglab`, resource CI). What it does need of `schedule.json` it checks: an object of
`schema_version` `actingcommand.schedule.v1` whose `rules` and `pools` (each absent means none) are objects
with distinct string `id`s, since a binding names its rule or its pool by id.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator

from acmodel.check.walk import at
from acmodel.envelope import Failure
from acmodel.model import jsonfile
from acmodel.model.readers import SCHEDULE_FILE
from acmodel.model.shape import Context, json_type

SCHEDULE_FORMAT = "actingcommand.schedule.v1"
POLICY_FORMAT = "json"  # a pack file is read as strict JSON; its own format is the validator's


@dataclass(frozen=True)
class Target:
    name: str  # as bindings.json names it, relative to the repository root
    path: Path
    raw: bytes
    doc: Any
    rules: dict[str, int] | None  # schedule.json: the index of each rule by its id
    pools: dict[str, int] | None  # schedule.json: the index of each pool by its id


def _items(doc: dict[str, Any], member: str, ctx: Context) -> dict[str, int]:
    """The index of each item of the list `member` of schedule.json (absent means none) by its id."""
    found: dict[str, int] = {}
    if not isinstance(doc.get(member, []), list):
        ctx.refuse("type_invalid", at("", member), expected=["array"])
        return found
    for index, item in enumerate(doc.get(member, [])):
        ref = at("", member, index)
        if not isinstance(item, dict):
            ctx.refuse("type_invalid", ref, expected=["object"])
        elif "id" not in item:
            ctx.refuse("member_missing", at(ref, "id"), key="id")
        elif not isinstance(item["id"], str):
            ctx.refuse("type_invalid", at(ref, "id"), expected=["string"])
        elif item["id"] in found:
            ctx.refuse("value_invalid", at(ref, "id"), literal=item["id"])
        else:
            found[item["id"]] = index
    return found


def _schedule(doc: Any, path: Path) -> tuple[dict[str, int], dict[str, int]]:
    """The index of each rule and of each pool of schedule.json by its id."""
    ctx = Context(path, SCHEDULE_FORMAT)
    rules: dict[str, int] = {}
    pools: dict[str, int] = {}
    if not isinstance(doc, dict):
        ctx.refuse("type_invalid", "", expected=["object"])
    elif doc.get("schema_version") != SCHEDULE_FORMAT:
        ctx.refuse("value_invalid", "/schema_version", expected=[SCHEDULE_FORMAT])
    else:
        rules, pools = _items(doc, "rules", ctx), _items(doc, "pools", ctx)
    if ctx.errors:
        raise Failure(*ctx.errors)
    return rules, pools


def read_target(repo: Path, name: str) -> Target:
    """The file `name` of the repository at `repo`, or `Failure` (`model_file_invalid`)."""
    path = repo / name
    fmt = SCHEDULE_FORMAT if name == SCHEDULE_FILE else POLICY_FORMAT
    raw = jsonfile.read(path, fmt)
    doc = jsonfile.parse(raw, path, fmt)
    rules, pools = _schedule(doc, path) if name == SCHEDULE_FILE else (None, None)
    return Target(name, path, raw, doc, rules, pools)


def integer(value: Any) -> bool:
    """A JSON integer; a boolean is not one."""
    return type(value) is int


def shown(value: Any) -> str:
    """A value as an error shows it: a string or an integer as its text, anything else as its JSON type."""
    return str(value) if isinstance(value, str) or integer(value) else json_type(value)


def fact_predicate(value: Any) -> bool:
    """A `fact` predicate of the scheduling format: `{"kind": "fact", "fact_key", "comparison", "value": {…}}`."""
    return (
        isinstance(value, dict) and value.get("kind") == "fact" and "comparison" in value
        and isinstance(value.get("value"), dict)
    )


def fact_reference(value: Any) -> bool:
    """A fact reference (#386 CF-4): `{"type": "fact", "fact_key": B, "offset": N}`."""
    return isinstance(value, dict) and value.get("type") == "fact"


def projection_predicate(value: Any) -> bool:
    """A `resource_projection` predicate of the scheduling format: `{"kind": "resource_projection", "pool_id",
    "comparison", "value": N}`, whose `value` may also be a fact reference (#386 CF-5)."""
    return (
        isinstance(value, dict) and value.get("kind") == "resource_projection" and "comparison" in value
        and "value" in value
    )


def predicate_literals(rule: dict[str, Any]) -> Iterator[tuple[str, int]]:
    """(pointer relative to the rule, integer) of each integer literal that a predicate of the rule compares
    with: of a `fact` predicate, its typed value's `value/value` or its fact reference's `value/offset`; of a
    `resource_projection` predicate, its `value` or its fact reference's `value/offset`. In document order, at
    any depth (under `all`, `any`, `not` and any member that holds predicates)."""
    stack: list[tuple[Any, str]] = [(rule, "")]
    while stack:
        value, ref = stack.pop()
        if fact_predicate(value):
            member = "offset" if fact_reference(value["value"]) else "value"
            if integer(value["value"].get(member)):
                yield at(ref, "value", member), value["value"][member]
        elif projection_predicate(value):
            operand = value["value"]
            if integer(operand):
                yield at(ref, "value"), operand
            elif fact_reference(operand) and integer(operand.get("offset")):
                yield at(ref, "value", "offset"), operand["offset"]
        if isinstance(value, dict):
            stack.extend(reversed([(child, at(ref, key)) for key, child in value.items()]))
        elif isinstance(value, list):
            stack.extend(reversed([(child, at(ref, index)) for index, child in enumerate(value)]))


def pool_literals(pool: dict[str, Any]) -> Iterator[tuple[str, int]]:
    """(pointer relative to the pool, integer) of each quantity of a pool of the scheduling format (#424 ACM-POOL),
    in this order: its `capacity`, or the offset of its fact reference (`capacity/offset`, #386 CF-5), then its
    regeneration's `projection/amount` and `projection/per_ms`. Its `value_source`, `group_delay` and `valuation`
    are settings of the Runtime and the scorer, as a rule's `max_age_ms` and weights are, and are not listed."""
    capacity = pool.get("capacity")
    if integer(capacity):
        yield "/capacity", capacity
    elif fact_reference(capacity) and integer(capacity.get("offset")):
        yield "/capacity/offset", capacity["offset"]
    projection = pool.get("projection")
    if isinstance(projection, dict):
        for member in ("amount", "per_ms"):
            if integer(projection.get(member)):
                yield at("", "projection", member), projection[member]
