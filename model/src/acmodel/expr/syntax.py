"""Lexer, parser and syntax tree of the expression language (design section 3.4).

Grammar, lowest precedence first:

    expression  := disjunction
    disjunction := conjunction ("or" conjunction)*
    conjunction := negation ("and" negation)*
    negation    := "not" negation | comparison
    comparison  := sum [("<" | "<=" | ">" | ">=" | "==" | "!=") sum]     one comparison, no chains
    sum         := product (("+" | "-") product)*
    product     := unary (("*" | "/") unary)*
    unary       := "-" unary | primary
    primary     := number | duration | "true" | "false" | reference | call | "(" expression ")"

Tokens:
- A number is `0`, `12` or `0.25`: decimal digits, held exactly, with no sign, exponent or leading
  zero. A duration is a number with the unit `s`, `m`, `h` or `d` and no space: `90m`, `1.5h`.
- A reference is `<kind>:<id>` for each kind of REFERENCE_KINDS, with two keyed forms:
  `data:<table>[<key>,...].<column>` and `entity:<class>[<id>].<attr>`. Without the key, `data:<table>`
  and `entity:<class>` name a set, which only `sum` and `count` take.
- An id is `[a-z][a-z0-9_.-]{0,95}` and runs to the first other character, so a `-` right after a
  reference belongs to it: write `amount:a - 1`, not `amount:a-1`.
- In the body of `sum(<set>: ...)` and `count(<set>: ...)`, the key `$` names the current member of
  that set: `sum(entity:u: entity:u[$].level)`.

Calls (FUNCTIONS):
- `min(a, b, ...)`, `max(a, b, ...)`, `clamp(x, low, high)`, `floor(x)`, `ceil(x)`,
  `round_half_even(x)`, and `if(condition, then, else)`, which evaluates only the chosen branch;
- `sum(<set>: expression)`, `count(<set>: condition)`, `lookup(<table>, <column>, <key>, ...)`;
- `cost(<activity>, <resource>)`, `yield(<activity>, <resource>)` and
  `max_single_income(<resource>, <cap rule>)`, whose arguments are ids.

Every refusal is `expr_parse_error` with its `reason` and the character `position`.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from fractions import Fraction
from typing import Any

from acmodel.codes import EXPR_PARSE_ERROR
from acmodel.expr.values import Duration, Type, Value, failure

REFERENCE_KINDS = (
    "amount", "cap", "cleared", "count", "data", "entity", "metric",
    "param", "pref", "read", "rule", "stock", "unknown",
)
SET_KINDS = ("data", "entity")
CURRENT = "$"
COMPARISONS = ("<", "<=", ">", ">=", "==", "!=")
BINARY_OPERATORS = ("+", "-", "*", "/", *COMPARISONS, "and", "or")
UNARY_OPERATORS = ("-", "not")
FLOW_FUNCTIONS = ("cost", "yield", "max_single_income")
FUNCTIONS = (
    "ceil", "clamp", "cost", "count", "floor", "if", "lookup",
    "max", "max_single_income", "min", "round_half_even", "sum", "yield",
)
ID_MAX = 96

# (fewest, most) arguments; None is no upper bound. sum and count have their own form.
_ARITY = {
    "ceil": (1, 1), "clamp": (3, 3), "cost": (2, 2), "floor": (1, 1), "if": (3, 3),
    "lookup": (3, None), "max": (2, None), "max_single_income": (2, 2), "min": (2, None),
    "round_half_even": (1, 1), "yield": (2, 2),
}
_LEADING_IDS = {"cost": 2, "yield": 2, "max_single_income": 2, "lookup": 2}
_WORD = re.compile(r"[a-z][a-z0-9_.-]*")
_KEY = re.compile(r"\$|[a-z0-9][a-z0-9_.-]*")
_NUMBER = re.compile(r"(0|[1-9][0-9]*)(?:\.([0-9]+))?")
_NUMBER_RUN = re.compile(r"[a-z0-9_.]*")
_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400}
_NUMBER_TAIL = frozenset("abcdefghijklmnopqrstuvwxyz0123456789_.")
_OPERATORS = ("<=", ">=", "==", "!=", "+", "-", "*", "/", "<", ">", "(", ")", ",", ":")
_SPACE = frozenset(" \t\r\n")


@dataclass(frozen=True)
class Literal:
    value: Value
    position: int
    type: Type | None = None


@dataclass(frozen=True)
class Reference:
    kind: str
    target: str
    key: tuple[str, ...] | None
    attr: str | None
    position: int
    type: Type | None = None

    @property
    def text(self) -> str:
        if self.key is None:
            return f"{self.kind}:{self.target}"
        return f"{self.kind}:{self.target}[{','.join(self.key)}].{self.attr}"


@dataclass(frozen=True)
class Unary:
    op: str
    operand: Node
    position: int
    type: Type | None = None


@dataclass(frozen=True)
class Binary:
    op: str
    left: Node
    right: Node
    position: int
    type: Type | None = None


@dataclass(frozen=True)
class Call:
    """The arguments of the flow functions and the first two of `lookup` are ids (str)."""

    function: str
    args: tuple[Node | str, ...]
    position: int
    type: Type | None = None


@dataclass(frozen=True)
class Aggregate:
    function: str
    over: Reference
    body: Node
    position: int
    type: Type | None = None


Node = Literal | Reference | Unary | Binary | Call | Aggregate


@dataclass(frozen=True)
class Expression:
    """A parsed expression; after `check` every node carries its type."""

    text: str
    ref: str | None
    root: Node

    @property
    def type(self) -> Type | None:
        return self.root.type


@dataclass(frozen=True)
class _Token:
    kind: str  # number, word, reference, op, end
    text: str
    position: int
    value: Any = None


def parse(text: str, *, ref: str | None = None) -> Expression:
    """Parse one expression; `ref` (the JSON pointer of the string) goes into every error."""
    if type(text) is not str:
        raise TypeError(f"an expression is a str, not {type(text).__name__}")
    parser = _Parser(_Lexer(text, ref).tokens(), ref)
    root = parser.expression()
    if parser.peek().kind != "end":
        parser.unexpected()
    return Expression(text, ref, root)


def _refuse(ref: str | None, reason: str, position: int, token: str | None) -> None:
    detail: dict[str, Any] = {"reason": reason, "position": position}
    if token is not None:
        detail["token"] = token
    raise failure(EXPR_PARSE_ERROR, ref, **detail)


class _Lexer:
    def __init__(self, text: str, ref: str | None) -> None:
        self.text = text
        self.ref = ref

    def tokens(self) -> list[_Token]:
        text, pos, out = self.text, 0, []
        while pos < len(text):
            char = text[pos]
            if char in _SPACE:
                pos += 1
                continue
            if "0" <= char <= "9":
                token, pos = self.number(pos)
            elif "a" <= char <= "z":
                token, pos = self.word(pos)
            else:
                op = next((op for op in _OPERATORS if text.startswith(op, pos)), None)
                if op is None:
                    _refuse(self.ref, "character_invalid", pos, char)
                token, pos = _Token("op", op, pos), pos + len(op)
            out.append(token)
        out.append(_Token("end", "", len(text)))
        return out

    def number(self, pos: int) -> tuple[_Token, int]:
        text = self.text
        match = _NUMBER.match(text, pos)
        digits, fraction = match.group(1), match.group(2) or ""
        value = Fraction(int(digits + fraction), 10 ** len(fraction))
        end = match.end()
        unit = text[end : end + 1]
        if unit in _UNITS and text[end + 1 : end + 2] not in _NUMBER_TAIL:
            return _Token("number", text[pos : end + 1], pos, Duration(value * _UNITS[unit])), end + 1
        if unit and unit in _NUMBER_TAIL:
            _refuse(self.ref, "number_malformed", pos, _NUMBER_RUN.match(text, pos).group())
        return _Token("number", text[pos:end], pos, value), end

    def word(self, pos: int) -> tuple[_Token, int]:
        word = _WORD.match(self.text, pos).group()
        end = pos + len(word)
        if not self.text.startswith(":", end):
            return _Token("word", word, pos), end
        if word not in REFERENCE_KINDS:
            _refuse(self.ref, "reference_kind_unknown", pos, word)
        target, end = self.ident(end + 1, pos)
        key = attr = None
        if self.text.startswith("[", end):
            if word not in SET_KINDS:
                _refuse(self.ref, "reference_malformed", pos, self.text[pos : end + 1])
            parts, end = [], end + 1
            while True:
                match = _KEY.match(self.text, end)
                if match is None or len(match.group()) > ID_MAX:
                    _refuse(self.ref, "reference_malformed", pos, self.text[pos:end])
                parts.append(match.group())
                end = match.end()
                if word == "data" and self.text.startswith(",", end):
                    end += 1
                    continue
                break
            if not self.text.startswith("].", end) or (CURRENT in parts and len(parts) > 1):
                _refuse(self.ref, "reference_malformed", pos, self.text[pos:end])
            attr, end = self.ident(end + 2, pos)
            key = tuple(parts)
        reference = Reference(word, target, key, attr, pos)
        return _Token("reference", self.text[pos:end], pos, reference), end

    def ident(self, pos: int, start: int) -> tuple[str, int]:
        match = _WORD.match(self.text, pos)
        if match is None or len(match.group()) > ID_MAX:
            _refuse(self.ref, "reference_malformed", start, self.text[start:pos])
        return match.group(), match.end()


class _Parser:
    def __init__(self, tokens: list[_Token], ref: str | None) -> None:
        self.tokens = tokens
        self.index = 0
        self.ref = ref
        self.sets: list[tuple[str, str]] = []  # the sets of the enclosing sum and count calls

    def peek(self, ahead: int = 0) -> _Token:
        return self.tokens[min(self.index + ahead, len(self.tokens) - 1)]

    def take(self) -> _Token:
        token = self.peek()
        self.index += 1
        return token

    def at(self, kind: str, *texts: str) -> bool:
        token = self.peek()
        return token.kind == kind and token.text in texts

    def expect(self, op: str) -> None:
        if not self.at("op", op):
            self.unexpected()
        self.take()

    def unexpected(self) -> None:
        token = self.peek()
        if token.kind == "end":
            _refuse(self.ref, "end_unexpected", token.position, None)
        _refuse(self.ref, "token_unexpected", token.position, token.text)

    def expression(self) -> Node:
        left = self.conjunction()
        while self.at("word", "or"):
            token = self.take()
            left = Binary("or", left, self.conjunction(), token.position)
        return left

    def conjunction(self) -> Node:
        left = self.negation()
        while self.at("word", "and"):
            token = self.take()
            left = Binary("and", left, self.negation(), token.position)
        return left

    def negation(self) -> Node:
        if self.at("word", "not"):
            token = self.take()
            return Unary("not", self.negation(), token.position)
        left = self.sum()
        if self.at("op", *COMPARISONS):
            token = self.take()
            return Binary(token.text, left, self.sum(), token.position)
        return left

    def sum(self) -> Node:
        left = self.product()
        while self.at("op", "+", "-"):
            token = self.take()
            left = Binary(token.text, left, self.product(), token.position)
        return left

    def product(self) -> Node:
        left = self.unary()
        while self.at("op", "*", "/"):
            token = self.take()
            left = Binary(token.text, left, self.unary(), token.position)
        return left

    def unary(self) -> Node:
        if self.at("op", "-"):
            token = self.take()
            return Unary("-", self.unary(), token.position)
        return self.primary()

    def primary(self) -> Node:
        token = self.peek()
        if token.kind == "number":
            self.take()
            return Literal(token.value, token.position)
        if token.kind == "reference":
            self.take()
            return self.value_reference(token.value)
        if self.at("op", "("):
            self.take()
            node = self.expression()
            self.expect(")")
            return node
        if self.at("word", "true", "false"):
            self.take()
            return Literal(token.text == "true", token.position)
        if token.kind == "word" and self.peek(1).kind == "op" and self.peek(1).text == "(":
            if token.text not in FUNCTIONS:
                _refuse(self.ref, "function_unknown", token.position, token.text)
            return self.call()
        self.unexpected()

    def value_reference(self, reference: Reference) -> Reference:
        if reference.kind in SET_KINDS and reference.key is None:
            _refuse(self.ref, "reference_malformed", reference.position, reference.text)
        if reference.key == (CURRENT,) and (reference.kind, reference.target) not in self.sets:
            _refuse(self.ref, "member_unbound", reference.position, reference.text)
        return reference

    def call(self) -> Node:
        name = self.take()
        self.take()
        if name.text in ("sum", "count"):
            token = self.peek()
            if token.kind != "reference" or token.value.kind not in SET_KINDS or token.value.key is not None:
                _refuse(self.ref, "argument_invalid", token.position, token.text or None)
            self.take()
            self.expect(":")
            self.sets.append((token.value.kind, token.value.target))
            body = self.expression()
            self.sets.pop()
            self.expect(")")
            return Aggregate(name.text, token.value, body, name.position)
        args: list[Node | str] = []
        while not (not args and self.at("op", ")")):
            if len(args) < _LEADING_IDS.get(name.text, 0):
                token = self.take()
                if token.kind != "word" or len(token.text) > ID_MAX:
                    _refuse(self.ref, "argument_invalid", token.position, token.text or None)
                args.append(token.text)
            else:
                args.append(self.expression())
            if not self.at("op", ","):
                break
            self.take()
        self.expect(")")
        fewest, most = _ARITY[name.text]
        if len(args) < fewest or (most is not None and len(args) > most):
            _refuse(self.ref, "arity_invalid", name.position, name.text)
        return Call(name.text, tuple(args), name.position)
