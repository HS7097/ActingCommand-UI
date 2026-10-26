"""Evaluation of a checked expression, exactly.

- Every value is a `Fraction`, a `bool` or a `Duration`; a value from the environment of any other
  type, or of a type its check did not give, is refused (`expr_eval_failed`, `value_invalid`).
- Every literal, value read and intermediate result whose numerator or denominator needs more than
  `max_bits` bits (configuration key `expr.max_bits`) is `expr_overflow`; durations count in seconds.
- Division by zero and `clamp` with low above high are `expr_eval_failed`.
- `and`, `or` and `if` evaluate only what decides the result. `sum` and `count` visit the members of
  their set in sorted order.
- A value that continuous unknowns leave open is an enclosure (`Interval`) or an undecided boolean
  (`Undecided`), and every operator then computes by interval arithmetic (`expr.interval`): a division by
  an enclosure that holds 0 is `division_by_zero`, `clamp` bounds that may cross are `clamp_bounds_invalid`,
  an undecided condition gives both branches, and a `lookup` key that is left open is
  `evaluation_unsupported` (`found` `interval`).
"""

from __future__ import annotations

import math
import operator
from dataclasses import replace
from fractions import Fraction
from typing import Any, Callable, Iterable, Protocol

from acmodel.codes import EXPR_EVAL_FAILED, EXPR_OVERFLOW
from acmodel.expr.syntax import (
    COMPARISONS,
    CURRENT,
    FLOW_FUNCTIONS,
    Aggregate,
    Binary,
    Call,
    Expression,
    Literal,
    Node,
    Reference,
    Unary,
)
from acmodel.expr import interval
from acmodel.expr.interval import Interval, IntervalRefused, Undecided, is_open
from acmodel.expr.values import BOOL, TIME, Duration, Value, failure


class ValueEnvironment(Protocol):
    def reference(self, ref: Reference) -> Value: ...

    def members(self, ref: Reference) -> Iterable[tuple[str, ...]]: ...

    def flow(self, function: str, first: str, second: str) -> Value: ...

    def lookup(self, table: str, column: str, keys: tuple[Value, ...]) -> Value: ...


def evaluate(expression: Expression, env: ValueEnvironment, *, max_bits: int) -> Value:
    """Evaluate an expression that `check` has typed."""
    if expression.type is None:
        raise ValueError("evaluate takes a checked expression")
    if type(max_bits) is not int or max_bits < 1:
        raise ValueError(f"max_bits must be a positive int, not {max_bits!r}")
    return _Evaluator(env, max_bits, expression.ref).value(expression.root, {})


def _magnitude(value: Value) -> Any:
    return value.seconds if type(value) is Duration else value


def _add(a: Any, b: Any) -> Value:
    return Duration(a.seconds + b.seconds) if type(a) is Duration else a + b


def _subtract(a: Any, b: Any) -> Value:
    return Duration(a.seconds - b.seconds) if type(a) is Duration else a - b


def _multiply(a: Any, b: Any) -> Value:
    if type(a) is Duration:
        return Duration(a.seconds * b)
    if type(b) is Duration:
        return Duration(a * b.seconds)
    return a * b


def _divide(a: Any, b: Any) -> Value:
    if type(a) is not Duration:
        return a / b
    return a.seconds / b.seconds if type(b) is Duration else Duration(a.seconds / b)


def _negate(a: Any) -> Value:
    return Duration(-a.seconds) if type(a) is Duration else -a


_ARITHMETIC: dict[str, Callable[[Any, Any], Value]] = {
    "+": _add, "-": _subtract, "*": _multiply, "/": _divide,
}
_COMPARE = {
    "<": operator.lt, "<=": operator.le, ">": operator.gt,
    ">=": operator.ge, "==": operator.eq, "!=": operator.ne,
}
_ROUNDING = {"floor": math.floor, "ceil": math.ceil, "round_half_even": round}


class _Evaluator:
    def __init__(self, env: ValueEnvironment, max_bits: int, ref: str | None) -> None:
        self.env = env
        self.max_bits = max_bits
        self.ref = ref

    def fail(self, code: str, node: Node, **detail: Any) -> None:
        raise failure(code, self.ref, position=node.position, **detail)

    def fit(self, value: Value, op: str, node: Node) -> Value:
        if type(value) in (bool, Undecided):
            return value
        for number in (value.low, value.high) if type(value) is Interval else (value,):
            number = _magnitude(number)
            bits = max(abs(number.numerator).bit_length(), number.denominator.bit_length())
            if bits > self.max_bits:
                self.fail(EXPR_OVERFLOW, node, operator=op, bits=bits, max_bits=self.max_bits)
        return value

    def admit(self, value: object, node: Node, op: str) -> Value:
        expected = bool if node.type == BOOL else Duration if node.type == TIME else Fraction
        found = bool if type(value) is Undecided else type(value.low) if type(value) is Interval else type(value)
        if found is not expected:
            self.fail(
                EXPR_EVAL_FAILED, node, reason="value_invalid", operator=op,
                expected=node.type.token, found=type(value).__name__,
            )
        return self.fit(value, op, node)

    def refused(self, refusal: IntervalRefused, node: Node, op: str) -> None:
        self.fail(EXPR_EVAL_FAILED, node, reason=refusal.reason, operator=op)

    def value(self, node: Node, bound: dict[tuple[str, str], tuple[str, ...]]) -> Value:
        if isinstance(node, Literal):
            return self.fit(node.value, "literal", node)
        if isinstance(node, Reference):
            ref = replace(node, key=bound[node.kind, node.target]) if node.key == (CURRENT,) else node
            return self.admit(self.env.reference(ref), node, "reference")
        if isinstance(node, Unary):
            operand = self.value(node.operand, bound)
            if node.op == "not":
                return operand if type(operand) is Undecided else not operand
            return self.fit(interval.negate(operand) if is_open(operand) else _negate(operand), "-", node)
        if isinstance(node, Binary):
            return self.binary(node, bound)
        if isinstance(node, Aggregate):
            return self.aggregate(node, bound)
        return self.call(node, bound)

    def binary(self, node: Binary, bound: dict[tuple[str, str], tuple[str, ...]]) -> Value:
        op = node.op
        left = self.value(node.left, bound)
        if op in ("and", "or"):
            decisive = op == "or"  # what decides the result without the right operand
            if type(left) is bool and left is decisive:
                return left
            right = self.value(node.right, bound)
            if type(left) is bool:
                return right
            if type(right) is bool and right is decisive:
                return right
            return Undecided(tuple(sorted(set(left.unknowns) | set(interval.unknowns(right)))))
        right = self.value(node.right, bound)
        if is_open(left) or is_open(right):
            try:
                if op in COMPARISONS:
                    return interval.compare(op, left, right)
                arithmetic = {"+": interval.add, "-": interval.subtract, "*": interval.multiply, "/": interval.divide}
                return self.fit(arithmetic[op](left, right), op, node)
            except IntervalRefused as refusal:
                self.refused(refusal, node, op)
        if op in COMPARISONS:
            return _COMPARE[op](_magnitude(left), _magnitude(right))
        if op == "/" and _magnitude(right) == 0:
            self.fail(EXPR_EVAL_FAILED, node, reason="division_by_zero", operator=op)
        return self.fit(_ARITHMETIC[op](left, right), op, node)

    def aggregate(self, node: Aggregate, bound: dict[tuple[str, str], tuple[str, ...]]) -> Value:
        members = list(self.env.members(node.over))
        shaped = all(type(m) is tuple and m and all(type(p) is str for p in m) for m in members)
        if not shaped or len(set(members)) != len(members):
            self.fail(EXPR_EVAL_FAILED, node, reason="value_invalid", operator=node.function, expected="members")
        total: Value = Duration(Fraction(0)) if node.type == TIME else Fraction(0)
        for member in sorted(members):
            item = self.value(node.body, {**bound, (node.over.kind, node.over.target): member})
            if node.function == "count":
                item = interval.open_count(item) if type(item) is Undecided else Fraction(1 if item else 0)
            total = interval.add(total, item) if is_open(total) or is_open(item) else _add(total, item)
            total = self.fit(total, node.function, node)
        return total

    def call(self, node: Call, bound: dict[tuple[str, str], tuple[str, ...]]) -> Value:
        name = node.function
        if name in FLOW_FUNCTIONS:
            return self.admit(self.env.flow(name, node.args[0], node.args[1]), node, name)
        if name == "lookup":
            keys = tuple(self.value(key, bound) for key in node.args[2:])
            if any(is_open(key) for key in keys):
                self.fail(EXPR_EVAL_FAILED, node, reason="evaluation_unsupported", operator=name, found="interval")
            return self.admit(self.env.lookup(node.args[0], node.args[1], keys), node, name)
        if name == "if":
            condition = self.value(node.args[0], bound)
            if type(condition) is Undecided:
                values = [self.value(arg, bound) for arg in node.args[1:]]
                return self.fit(interval.hull(values[0], values[1], condition), name, node)
            return self.value(node.args[1] if condition else node.args[2], bound)
        values = [self.value(arg, bound) for arg in node.args]
        if any(is_open(value) for value in values):
            try:
                if name in ("min", "max"):
                    return (interval.minimum if name == "min" else interval.maximum)(values)
                if name == "clamp":
                    return interval.clamp(*values)
                return self.fit(interval.rounded(_ROUNDING[name], values[0]), name, node)
            except IntervalRefused as refusal:
                self.refused(refusal, node, name)
        if name == "min":
            return min(values, key=_magnitude)
        if name == "max":
            return max(values, key=_magnitude)
        if name == "clamp":
            x, low, high = (_magnitude(v) for v in values)
            if low > high:
                self.fail(EXPR_EVAL_FAILED, node, reason="clamp_bounds_invalid", operator=name)
            return values[1] if x < low else values[2] if x > high else values[0]
        return self.fit(Fraction(_ROUNDING[name](values[0])), name, node)
