"""Values and static types of the expression language.

Values are exact: `Fraction` for numbers, `Duration` for time, and `bool`. There are no floats.

Every number has a dimension, its `Type`:
- `amount:<resource>`, `time` (held as a `Duration`), `count`, `value` or `none`;
- `number`, the type of a number literal. It takes the dimension of what it is combined with, but
  never `time`: a duration is always written with its unit.
"""

from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction
from typing import Any

from acmodel.envelope import Failure, error

KINDS = ("bool", "number", "none", "count", "value", "time", "amount")


@dataclass(frozen=True)
class Duration:
    """A span of time, held exactly in seconds."""

    seconds: Fraction

    def __post_init__(self) -> None:
        if type(self.seconds) is not Fraction:
            raise TypeError(f"Duration.seconds must be a Fraction, not {type(self.seconds).__name__}")


Value = Fraction | bool | Duration


@dataclass(frozen=True)
class Type:
    """`bool`, or a number with its dimension; `resource` is set exactly for `amount`."""

    kind: str
    resource: str | None = None

    def __post_init__(self) -> None:
        if self.kind not in KINDS or (self.kind == "amount") != (self.resource is not None):
            raise ValueError(f"not an expression type: {self.kind!r} {self.resource!r}")

    @property
    def token(self) -> str:
        return f"amount:{self.resource}" if self.kind == "amount" else self.kind


BOOL = Type("bool")
NUMBER = Type("number")
NONE = Type("none")
COUNT = Type("count")
VALUE = Type("value")
TIME = Type("time")


def amount(resource: str) -> Type:
    return Type("amount", resource)


def failure(code: str, ref: str | None, **detail: Any) -> Failure:
    """A registered error; `ref` is the JSON pointer of the expression in its file."""
    return Failure(error(code, detail, ref))
