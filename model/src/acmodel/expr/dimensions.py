"""The dimension check: it gives every node of a parsed expression its type.

Rules (a mismatch is `expr_dimension_mismatch` with the operator and the operands' types):
- `+`, `-`, the comparisons, `min`, `max`, `clamp` and the branches of `if` need one dimension. A number
  literal (`number`) takes the other operand's dimension, except `time`. Booleans compare only with `==`
  and `!=`.
- `*`: one side is `number`, `none` or `count`, and the result has the other side's dimension.
- `/`: `x / number` and `x / none` and `x / count` keep the dimension of `x`; `x / x` is `none`.
- `floor`, `ceil` and `round_half_even` take a number that is not `time` and keep its dimension.
- `not`, `and` and `or` take booleans, as does the condition of `if`; unary `-` takes a number.
- `sum` keeps the dimension of its body; `count` takes a condition and gives `count`.
- `cost(a, r)` and `yield(a, r)` give `amount:r`; `max_single_income(r, cap)` gives `amount:r`.
- References and `lookup` get their types from the model through a `TypeEnvironment`.
"""

from __future__ import annotations

from dataclasses import replace
from typing import Protocol

from acmodel.codes import EXPR_DIMENSION_MISMATCH
from acmodel.envelope import Failure
from acmodel.expr.syntax import (
    COMPARISONS,
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
from acmodel.expr.values import BOOL, COUNT, NONE, NUMBER, TIME, Duration, Type, amount, failure

_SCALARS = (NUMBER, NONE, COUNT)


class TypeEnvironment(Protocol):
    def reference(self, ref: Reference) -> Type: ...

    def lookup(self, table: str, column: str, keys: tuple[Type, ...]) -> Type: ...


def check(expression: Expression, env: TypeEnvironment) -> Expression:
    """Return the expression with every node typed."""
    return replace(expression, root=_Checker(env, expression.ref).node(expression.root))


class _Checker:
    def __init__(self, env: TypeEnvironment, ref: str | None) -> None:
        self.env = env
        self.ref = ref

    def mismatch(self, operator: str, position: int, *operands: Type) -> Failure:
        return failure(
            EXPR_DIMENSION_MISMATCH,
            self.ref,
            operator=operator,
            operands=[t.token for t in operands],
            position=position,
        )

    def unify(self, operator: str, position: int, *types: Type) -> Type:
        fixed = {t for t in types if t != NUMBER}
        if BOOL in fixed or len(fixed) > 1 or (TIME in fixed and NUMBER in types):
            raise self.mismatch(operator, position, *types)
        return fixed.pop() if fixed else NUMBER

    def node(self, node: Node) -> Node:
        if isinstance(node, Literal):
            value = node.value
            return replace(node, type=BOOL if type(value) is bool else TIME if type(value) is Duration else NUMBER)
        if isinstance(node, Reference):
            return replace(node, type=_known(self.env.reference(node)))
        if isinstance(node, Unary):
            operand = self.node(node.operand)
            if (node.op == "not") != (operand.type == BOOL):
                raise self.mismatch(node.op, node.position, operand.type)
            return replace(node, operand=operand, type=operand.type)
        if isinstance(node, Binary):
            left, right = self.node(node.left), self.node(node.right)
            kind = self.binary(node.op, node.position, left.type, right.type)
            return replace(node, left=left, right=right, type=kind)
        if isinstance(node, Aggregate):
            body = self.node(node.body)
            if (node.function == "count") != (body.type == BOOL):
                raise self.mismatch(node.function, node.position, body.type)
            return replace(node, body=body, type=COUNT if node.function == "count" else body.type)
        return self.call(node)

    def binary(self, op: str, position: int, a: Type, b: Type) -> Type:
        if op in ("and", "or"):
            if a == b == BOOL:
                return BOOL
            raise self.mismatch(op, position, a, b)
        if op in ("==", "!=") and a == b == BOOL:
            return BOOL
        if op in COMPARISONS:
            self.unify(op, position, a, b)
            return BOOL
        if op in ("+", "-"):
            return self.unify(op, position, a, b)
        if BOOL not in (a, b):
            if op == "*":
                for scalar in _SCALARS:
                    if scalar in (a, b):
                        return b if a == scalar else a
            elif b == NUMBER:
                return a
            elif a == b:
                return NONE
            elif b in (NONE, COUNT):
                return a
        raise self.mismatch(op, position, a, b)

    def call(self, node: Call) -> Call:
        name = node.function
        if name in FLOW_FUNCTIONS:
            return replace(node, type=amount(node.args[0] if name == "max_single_income" else node.args[1]))
        if name == "lookup":
            keys = tuple(self.node(key) for key in node.args[2:])
            kind = _known(self.env.lookup(node.args[0], node.args[1], tuple(key.type for key in keys)))
            return replace(node, args=node.args[:2] + keys, type=kind)
        args = tuple(self.node(arg) for arg in node.args)
        types = [arg.type for arg in args]
        if name == "if":
            if types[0] != BOOL:
                raise self.mismatch(name, node.position, *types)
            kind = BOOL if types[1] == types[2] == BOOL else self.unify(name, node.position, *types[1:])
        elif name in ("floor", "ceil", "round_half_even"):
            if types[0] in (BOOL, TIME):
                raise self.mismatch(name, node.position, *types)
            kind = types[0]
        else:
            kind = self.unify(name, node.position, *types)
        return replace(node, args=args, type=kind)


def _known(kind: object) -> Type:
    if not isinstance(kind, Type):
        raise TypeError(f"a type environment returned {type(kind).__name__}, not a Type")
    return kind
