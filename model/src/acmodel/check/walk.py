"""Walks over what the readers accepted and over parsed expressions, in document order."""

from __future__ import annotations

from typing import Any, Iterator

from acmodel.expr import Aggregate, Binary, Call, Unary
from acmodel.expr.syntax import Node
from acmodel.model import readers
from acmodel.model.jsonfile import pointer
from acmodel.model.shape import Either, ListOf, MapOf, Object, Spec, Tagged, json_type


def at(ref: str, *tokens: str | int) -> str:
    """The JSON pointer of `tokens` below `ref`."""
    for token in tokens:
        ref = pointer(ref, token)
    return ref


def expressions(spec: Spec, value: Any, ref: str) -> Iterator[tuple[str, str]]:
    """(pointer, text) of every expression in `value`, which the reader checked against `spec`.

    The readers mark each expression member with their one `EXPR` spec, so this follows them and
    never lists expression members of its own.
    """
    if spec is readers.EXPR:
        yield ref, value
    elif isinstance(spec, Tagged):
        shape = spec.shapes[value[spec.tag]]
        yield from expressions(shape, {key: member for key, member in value.items() if key != spec.tag}, ref)
    elif isinstance(spec, Object):
        for name, member in value.items():
            inner = spec.required[name] if name in spec.required else spec.optional[name]
            yield from expressions(inner, member, pointer(ref, name))
    elif isinstance(spec, ListOf):
        for index, item in enumerate(value):
            yield from expressions(spec.item, item, pointer(ref, index))
    elif isinstance(spec, MapOf):
        for name, member in value.items():
            yield from expressions(spec.value, member, pointer(ref, name))
    elif isinstance(spec, Either):
        yield from expressions(spec.by_type[json_type(value)], value, ref)


def nodes(node: Node) -> Iterator[Node]:
    """Every node of a parsed expression, each before its operands, left to right."""
    yield node
    if isinstance(node, Unary):
        yield from nodes(node.operand)
    elif isinstance(node, Binary):
        yield from nodes(node.left)
        yield from nodes(node.right)
    elif isinstance(node, Call):
        for arg in node.args:
            if not isinstance(arg, str):
                yield from nodes(arg)
    elif isinstance(node, Aggregate):
        yield from nodes(node.over)
        yield from nodes(node.body)
