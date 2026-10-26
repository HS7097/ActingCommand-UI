"""Declarative shapes for JSON documents.

Each spec checks one value at a JSON pointer and records every problem as a
`model_file_invalid` refusal; `check` returns whether the value was valid.
Nothing has a default: a missing required member is refused, and so is an
unknown one.
"""

from __future__ import annotations

import json
import re
from datetime import date
from pathlib import Path
from typing import Any, Callable, Mapping

from acmodel.model.jsonfile import pointer, refusal

Rule = Callable[[dict[str, Any], str, "Context"], bool]


class Context:
    def __init__(self, path: Path | str, fmt: str) -> None:
        self.path = str(path)
        self.fmt = fmt
        self.errors: list[dict[str, Any]] = []

    def refuse(self, reason: str, ref: str, **values: Any) -> bool:
        self.errors.append(refusal(reason, self.path, self.fmt, ref, **values))
        return False


def json_type(value: Any) -> str:
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return "null"


def _shown(value: Any) -> dict[str, str]:
    """The offending scalar as text, for `literal`; containers are not repeated."""
    if isinstance(value, str):
        return {"literal": value}
    if value is None or isinstance(value, (bool, int)):
        return {"literal": json.dumps(value)}
    return {}


class Spec:
    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        raise NotImplementedError


class AnyValue(Spec):
    """Any JSON value (already strict at the JSON level)."""

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        return True


class Text(Spec):
    """A non-empty string; with a pattern, the whole string must match it."""

    def __init__(self, pattern: str | None = None, reason: str = "value_invalid") -> None:
        self.pattern = re.compile(pattern) if pattern is not None else None
        self.reason = reason

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if not isinstance(value, str):
            return ctx.refuse("type_invalid", ref, expected=["string"])
        if not value or (self.pattern is not None and not self.pattern.fullmatch(value)):
            return ctx.refuse(self.reason, ref, literal=value)
        return True


class Date(Text):
    """A calendar date `YYYY-MM-DD`."""

    def __init__(self) -> None:
        super().__init__(r"[0-9]{4}-[0-9]{2}-[0-9]{2}")

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if not super().check(value, ref, ctx):
            return False
        try:
            date.fromisoformat(value)
        except ValueError:
            return ctx.refuse("value_invalid", ref, literal=value)
        return True


class Choice(Spec):
    """One of fixed scalars, compared together with their JSON type."""

    def __init__(self, *members: str | int) -> None:
        self.members = members

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if any(json_type(value) == json_type(member) and value == member for member in self.members):
            return True
        return ctx.refuse("value_invalid", ref, expected=[str(member) for member in self.members], **_shown(value))


class Integer(Spec):
    def __init__(self, low: int, high: int) -> None:
        self.low = low
        self.high = high

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if json_type(value) != "integer":
            return ctx.refuse("type_invalid", ref, expected=["integer"])
        if not self.low <= value <= self.high:
            return ctx.refuse("value_invalid", ref, literal=str(value))
        return True


class Boolean(Spec):
    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if json_type(value) != "boolean":
            return ctx.refuse("type_invalid", ref, expected=["boolean"])
        return True


class Either(Spec):
    """A value whose JSON type selects its spec (`string=`, `array=`, ...)."""

    def __init__(self, **by_type: Spec) -> None:
        self.by_type = by_type

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        spec = self.by_type.get(json_type(value))
        if spec is None:
            return ctx.refuse("type_invalid", ref, expected=list(self.by_type))
        return spec.check(value, ref, ctx)


class ListOf(Spec):
    def __init__(self, item: Spec, min_items: int = 0) -> None:
        self.item = item
        self.min_items = min_items

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if not isinstance(value, list):
            return ctx.refuse("type_invalid", ref, expected=["array"])
        ok = len(value) >= self.min_items or ctx.refuse("value_invalid", ref)
        results = [self.item.check(item, pointer(ref, index), ctx) for index, item in enumerate(value)]
        return ok and all(results)


class MapOf(Spec):
    """An object with free member names, each checked by `key`."""

    def __init__(self, key: Text, value: Spec) -> None:
        self.key = key
        self.value = value

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if not isinstance(value, dict):
            return ctx.refuse("type_invalid", ref, expected=["object"])
        ok = True
        for name, member in value.items():
            at = pointer(ref, name)
            ok = self.key.check(name, at, ctx) and ok
            ok = self.value.check(member, at, ctx) and ok
        return ok


class Object(Spec):
    """An object with fixed members; `rule` runs only when all members are valid."""

    def __init__(
        self,
        required: Mapping[str, Spec],
        optional: Mapping[str, Spec] | None = None,
        rule: Rule | None = None,
    ) -> None:
        self.required = dict(required)
        self.optional = dict(optional or {})
        self.rule = rule

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if not isinstance(value, dict):
            return ctx.refuse("type_invalid", ref, expected=["object"])
        ok = True
        for name in value:
            if name not in self.required and name not in self.optional:
                ok = ctx.refuse("member_unknown", pointer(ref, name), key=name)
        for name, spec in self.required.items():
            if name not in value:
                ok = ctx.refuse("member_missing", pointer(ref, name), key=name)
            else:
                ok = spec.check(value[name], pointer(ref, name), ctx) and ok
        for name, spec in self.optional.items():
            if name in value:
                ok = spec.check(value[name], pointer(ref, name), ctx) and ok
        if ok and self.rule is not None:
            ok = self.rule(value, ref, ctx)
        return ok


class Tagged(Spec):
    """An object whose member `tag` names its shape; the shape checks the other members."""

    def __init__(self, tag: str, shapes: Mapping[str, Object]) -> None:
        self.tag = tag
        self.shapes = dict(shapes)

    def check(self, value: Any, ref: str, ctx: Context) -> bool:
        if not isinstance(value, dict):
            return ctx.refuse("type_invalid", ref, expected=["object"])
        if self.tag not in value:
            return ctx.refuse("member_missing", pointer(ref, self.tag), key=self.tag)
        name = value[self.tag]
        shape = self.shapes.get(name) if isinstance(name, str) else None
        if shape is None:
            return ctx.refuse("value_invalid", pointer(ref, self.tag), expected=list(self.shapes), **_shown(name))
        return shape.check({key: member for key, member in value.items() if key != self.tag}, ref, ctx)


def exactly_one(*names: str) -> Rule:
    """A rule: exactly one of the optional members `names` is present."""

    def rule(value: dict[str, Any], ref: str, ctx: Context) -> bool:
        present = [name for name in names if name in value]
        if len(present) == 1:
            return True
        if not present:
            return ctx.refuse("member_missing", pointer(ref, names[0]), key=names[0], expected=list(names))
        return ctx.refuse("member_conflict", pointer(ref, present[1]), key=present[1], expected=list(names))

    return rule
