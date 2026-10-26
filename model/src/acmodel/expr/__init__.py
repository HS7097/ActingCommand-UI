"""The expression language of the model files (design section 3.4), exact and game-neutral.

Use: `parse(text, ref=...)`, then `check(expression, types)`, then
`evaluate(checked, values, max_bits=config.expr_max_bits)`. The grammar is in `syntax`, the
dimension rules in `dimensions`, and evaluation in `evaluate`. The model supplies references,
sets, flows and lookups through the two environments; this package knows no model.

Codes: `expr_parse_error`, `expr_dimension_mismatch`, `expr_overflow`, `expr_eval_failed`.
"""

from acmodel.expr.dimensions import TypeEnvironment, check
from acmodel.expr.evaluate import ValueEnvironment, evaluate
from acmodel.expr.syntax import (
    BINARY_OPERATORS,
    FUNCTIONS,
    REFERENCE_KINDS,
    UNARY_OPERATORS,
    Aggregate,
    Binary,
    Call,
    Expression,
    Literal,
    Reference,
    Unary,
    parse,
)
from acmodel.expr.values import BOOL, COUNT, NONE, NUMBER, TIME, VALUE, Duration, Type, Value, amount

__all__ = [
    "BINARY_OPERATORS", "BOOL", "COUNT", "FUNCTIONS", "NONE", "NUMBER", "REFERENCE_KINDS", "TIME",
    "UNARY_OPERATORS", "VALUE", "Aggregate", "Binary", "Call", "Duration", "Expression", "Literal",
    "Reference", "Type", "TypeEnvironment", "Unary", "Value", "ValueEnvironment", "amount", "check",
    "evaluate", "parse",
]
