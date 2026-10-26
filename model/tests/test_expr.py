"""PY-1: the expression language: parse, dimension check and exact evaluation.

The fake model uses the neutral numbers of the design's worked example (res.a, fac.s, act.daily).
"""

import json
from fractions import Fraction
from pathlib import Path

import pytest

from acmodel import config
from acmodel.envelope import Failure
from acmodel.expr import (
    BINARY_OPERATORS,
    BOOL,
    COUNT,
    FUNCTIONS,
    NONE,
    NUMBER,
    TIME,
    UNARY_OPERATORS,
    VALUE,
    Duration,
    amount,
    check,
    evaluate,
    parse,
)

CODES = json.loads(
    (Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8")
)["codes"]
REF = "/rules/0/logic/value"
RES_A = amount("res.a")
LARGEST = str(2**127 - 1)

# reference text -> type; the `$` forms are what the check sees inside sum and count
TYPES = {
    "read:resource.a_capacity": RES_A,
    "read:resource.f": RES_A,
    "amount:res.a": RES_A,
    "amount:res.b": amount("res.b"),
    "stock:fac.s": RES_A,
    "cap:cap.s.storage": RES_A,
    "rule:margin.a": NUMBER,
    "rule:reserve.a": NUMBER,
    "data:costs[act.run].a": RES_A,
    "data:costs[act.daily].a": RES_A,
    "data:costs[$].a": RES_A,
    "count:limit.daily": COUNT,
    "pref:p.reserve": RES_A,
    "param:p.wait": TIME,
    "unknown:unk.collect.taken_at_margin": RES_A,
    "metric:eng.e1.score": VALUE,
    "entity:unit[u1].level": NONE,
    "entity:unit[$].level": NONE,
    "cleared:gate.g1": BOOL,
    "cleared:gate.g2": BOOL,
}
VALUES = {
    "read:resource.a_capacity": Fraction(3470),
    "read:resource.f": 0.5,  # a float from the model is refused
    "amount:res.a": Fraction(3900),
    "stock:fac.s": Fraction(800),
    "cap:cap.s.storage": Fraction(800),
    "rule:margin.a": Fraction(250),
    "rule:reserve.a": Fraction(200),
    "data:costs[act.run].a": Fraction(70),
    "data:costs[act.daily].a": Fraction(15),
    "count:limit.daily": Fraction(2),
    "pref:p.reserve": Fraction(100),
    "param:p.wait": Duration(Fraction(1800)),
    "unknown:unk.collect.taken_at_margin": Fraction(250),
    "metric:eng.e1.score": Fraction(1200),
    "entity:unit[u1].level": Fraction(2),
    "entity:unit[u2].level": Fraction(3),
    "entity:unit[u3].level": Fraction(4),
    "cleared:gate.g1": True,
    "cleared:gate.g2": False,
}
MEMBERS = {"entity:unit": [("u3",), ("u1",), ("u2",)], "data:costs": [("act.run",), ("act.daily",)]}
FLOWS = {
    ("cost", "act.daily", "res.a"): Fraction(15),
    ("yield", "act.collect", "res.a"): Fraction(800),
    ("max_single_income", "res.a", "cap.a.soft"): Fraction(800),
}


class FakeTypes:
    def reference(self, ref):
        return TYPES[ref.text]

    def lookup(self, table, column, keys):
        return {("rates", "rate"): NONE}[table, column]


class FakeValues:
    def reference(self, ref):
        return VALUES[ref.text]

    def members(self, ref):
        return MEMBERS[ref.text]

    def flow(self, function, first, second):
        return FLOWS[function, first, second]

    def lookup(self, table, column, keys):
        return {("rates", "rate", (Fraction(2),)): Fraction(1, 4)}[table, column, keys]


def run(text, max_bits=config.DEFAULT_EXPR_MAX_BITS):
    return evaluate(check(parse(text, ref=REF), FakeTypes()), FakeValues(), max_bits=max_bits)


def refusal(excinfo):
    """The one error of the Failure, checked against contracts/codes.json."""
    [entry] = excinfo.value.errors
    assert entry["ref"] == REF
    values = CODES[entry["code"]]["values"]
    for key, value in entry["detail"].items():
        spec = values[key]
        if spec["type"] == "list:token":
            assert type(value) is list and all(type(item) is str for item in value), (key, value)
        else:
            assert type(value) is {"integer": int, "token": str, "name": str, "vocab": str}[spec["type"]]
        assert value in spec.get("members", [value]), (key, value)
    assert {key for key, spec in values.items() if spec["required"]} <= set(entry["detail"])
    return entry


def seconds(n):
    return Duration(Fraction(n))


TABLE = [
    ("42", Fraction(42)),
    ("0.25", Fraction(1, 4)),
    ("45s", seconds(45)),
    ("90m", seconds(5400)),
    ("1.5h", seconds(5400)),
    ("2d", seconds(172800)),
    ("true", True),
    ("false", False),
    ("1 + 2 * 3", Fraction(7)),
    ("(1 + 2) * 3", Fraction(9)),
    ("7 - 2 - 1", Fraction(4)),
    ("8 / 2 / 2", Fraction(2)),
    ("10 / 4", Fraction(5, 2)),
    ("-2 * 3", Fraction(-6)),
    ("- -2", Fraction(2)),
    ("not 1 > 2", True),
    ("true or false and false", True),
    ("not true or true", True),
    ("1 <= 1 and 2 >= 3", False),
    ("1 == 1.0", True),
    ("1 != 2", True),
    ("true == false", False),
    ("1h + 30m", seconds(5400)),
    ("2 * 1.5h", seconds(10800)),
    ("1h / 4", seconds(900)),
    ("1d / 1h", Fraction(24)),
    ("90m > 1h", True),
    ("-1h", seconds(-3600)),
    ("min(1h, 30m)", seconds(1800)),
    ("read:resource.a_capacity - rule:margin.a", Fraction(3220)),
    ("0 - rule:margin.a", Fraction(-250)),
    ("cost(act.daily, res.a) + rule:reserve.a", Fraction(215)),
    ("cap:cap.s.storage - unknown:unk.collect.taken_at_margin", Fraction(550)),
    ("min(stock:fac.s, read:resource.a_capacity - amount:res.a)", Fraction(-430)),
    ("stock:fac.s < cap:cap.s.storage", False),
    ("max_single_income(res.a, cap.a.soft) - rule:margin.a", Fraction(550)),
    ("yield(act.collect, res.a)", Fraction(800)),
    ("data:costs[act.run].a * count:limit.daily", Fraction(140)),
    ("pref:p.reserve + 1", Fraction(101)),
    ("param:p.wait + 30m", seconds(3600)),
    ("metric:eng.e1.score / 1000", Fraction(6, 5)),
    ("cleared:gate.g1 and not cleared:gate.g2", True),
    ("max(1, 2.5, 2)", Fraction(5, 2)),
    ("clamp(5, 0, 3)", Fraction(3)),
    ("clamp(0 - 1, 0, 3)", Fraction(0)),
    ("clamp(2, 0, 3)", Fraction(2)),
    ("floor(7 / 2)", Fraction(3)),
    ("ceil(7 / 2)", Fraction(4)),
    ("floor(0 - 7 / 2)", Fraction(-4)),
    ("round_half_even(5 / 2)", Fraction(2)),
    ("round_half_even(7 / 2)", Fraction(4)),
    ("if(1 < 2, 10, 20)", Fraction(10)),
    ("if(true, 1, 1 / 0)", Fraction(1)),
    ("false and 1 / 0 > 0", False),
    ("sum(entity:unit: entity:unit[$].level)", Fraction(9)),
    ("count(entity:unit: entity:unit[$].level >= 3)", Fraction(2)),
    ("sum(data:costs: data:costs[$].a)", Fraction(85)),
    ("lookup(rates, rate, entity:unit[u1].level)", Fraction(1, 4)),
]


@pytest.mark.parametrize(("text", "expected"), TABLE, ids=[text for text, _ in TABLE])
def test_parse_eval_table(text, expected):
    result = run(text)

    assert type(result) is type(expected)
    assert result == expected


REFUSALS = [
    ("1 +", "expr_parse_error", {"reason": "end_unexpected", "position": 3}),
    ("(1 + 2", "expr_parse_error", {"reason": "end_unexpected", "position": 6}),
    ("1 < 2 < 3", "expr_parse_error", {"reason": "token_unexpected", "position": 6, "token": "<"}),
    ("1e5", "expr_parse_error", {"reason": "number_malformed", "position": 0, "token": "1e5"}),
    ("1 & 2", "expr_parse_error", {"reason": "character_invalid", "position": 2, "token": "&"}),
    ("entity:unit + 1", "expr_parse_error", {"reason": "reference_malformed", "position": 0, "token": "entity:unit"}),
    (
        "rule:margin.a[x].y",
        "expr_parse_error",
        {"reason": "reference_malformed", "position": 0, "token": "rule:margin.a["},
    ),
    (
        "entity:unit[$].level",
        "expr_parse_error",
        {"reason": "member_unbound", "position": 0, "token": "entity:unit[$].level"},
    ),
    ("min(1)", "expr_parse_error", {"reason": "arity_invalid", "position": 0, "token": "min"}),
    ("cost(1, res.a)", "expr_parse_error", {"reason": "argument_invalid", "position": 5, "token": "1"}),
    ("1 / 0", "expr_eval_failed", {"reason": "division_by_zero", "operator": "/", "position": 2}),
    ("1h / 0", "expr_eval_failed", {"reason": "division_by_zero", "operator": "/", "position": 3}),
    ("clamp(1, 3, 2)", "expr_eval_failed", {"reason": "clamp_bounds_invalid", "operator": "clamp", "position": 0}),
    (
        "read:resource.f + 1",
        "expr_eval_failed",
        {"reason": "value_invalid", "operator": "reference", "position": 0, "expected": "amount:res.a", "found": "float"},
    ),
]


@pytest.mark.parametrize(("text", "code", "detail"), REFUSALS, ids=[text for text, _, _ in REFUSALS])
def test_parse_eval_table_refusals(text, code, detail):
    with pytest.raises(Failure) as excinfo:
        run(text)

    entry = refusal(excinfo)
    assert (entry["code"], entry["detail"]) == (code, detail)


def test_decimals_add_exactly():
    result = run("0.1+0.2")

    assert type(result) is Fraction
    assert result == Fraction(3, 10)


@pytest.mark.parametrize(
    ("configured", "text", "operator", "bits", "position"),
    [
        (None, LARGEST + " + 1", "+", 128, len(LARGEST) + 1),
        (8, "256", "literal", 9, 0),
        (8, "255 + 1", "+", 9, 4),
        (8, "1 / 255 / 2", "/", 9, 8),
        (8, "16 * 16", "*", 9, 3),
        (8, "5m", "literal", 9, 0),
    ],
    ids=["default_127", "literal", "sum", "denominator", "product", "duration_seconds"],
)
def test_expr_overflow_beyond_max_bits(write_config, local_app_data, configured, text, operator, bits, position):
    if configured is not None:
        write_config({"schema_version": "acmodel.config.v1", "expr": {"max_bits": configured}})
    max_bits = config.load({"LOCALAPPDATA": str(local_app_data)}).expr_max_bits
    assert max_bits == (127 if configured is None else configured)
    largest_within = LARGEST if configured is None else "255"
    assert run(largest_within, max_bits) == int(largest_within)

    with pytest.raises(Failure) as excinfo:
        run(text, max_bits)

    entry = refusal(excinfo)
    assert entry["code"] == "expr_overflow"
    assert entry["detail"] == {"operator": operator, "bits": bits, "max_bits": max_bits, "position": position}


@pytest.mark.parametrize(
    ("text", "operands"),
    [
        ("amount:res.a + 1h", ["amount:res.a", "time"]),
        ("amount:res.a + amount:res.b", ["amount:res.a", "amount:res.b"]),
    ],
    ids=["amount_plus_time", "two_resources"],
)
def test_expr_dimension_mismatch(text, operands):
    with pytest.raises(Failure) as excinfo:
        check(parse(text, ref=REF), FakeTypes())

    entry = refusal(excinfo)
    assert entry["code"] == "expr_dimension_mismatch"
    assert entry["detail"] == {"operator": "+", "operands": operands, "position": 13}


@pytest.mark.parametrize(
    ("text", "detail"),
    [
        ("bogus:x + 1", {"reason": "reference_kind_unknown", "position": 0, "token": "bogus"}),
        ("1 + frob(2)", {"reason": "function_unknown", "position": 4, "token": "frob"}),
    ],
    ids=["reference_kind", "function"],
)
def test_unknown_reference_kind_or_function_is_refused(text, detail):
    with pytest.raises(Failure) as excinfo:
        parse(text, ref=REF)

    entry = refusal(excinfo)
    assert entry["code"] == "expr_parse_error"
    assert entry["detail"] == detail


# Each operator and function, over every kind of operand it takes.
SAMPLES = {
    ("binary", "+"): ["1 + 2", "1h + 1m"],
    ("binary", "-"): ["3 - 1.5", "1h - 1m"],
    ("binary", "*"): ["2 * 0.5", "2 * 1h", "1h * 2"],
    ("binary", "/"): ["1 / 3", "1h / 2", "1h / 1m"],
    ("binary", "<"): ["1 < 2", "1m < 1h"],
    ("binary", "<="): ["1 <= 2", "1m <= 1h"],
    ("binary", ">"): ["1 > 2", "1m > 1h"],
    ("binary", ">="): ["1 >= 2", "1m >= 1h"],
    ("binary", "=="): ["1 == 2", "1m == 60s", "true == true"],
    ("binary", "!="): ["1 != 2", "1m != 60s", "true != false"],
    ("binary", "and"): ["true and false"],
    ("binary", "or"): ["false or true"],
    ("unary", "-"): ["-1.5", "-1h"],
    ("unary", "not"): ["not true"],
    ("function", "min"): ["min(1, 2)", "min(1h, 2m)"],
    ("function", "max"): ["max(1, 2)", "max(1h, 2m)"],
    ("function", "clamp"): ["clamp(1, 0, 2)", "clamp(1h, 0s, 30m)"],
    ("function", "floor"): ["floor(1 / 3)"],
    ("function", "ceil"): ["ceil(1 / 3)"],
    ("function", "round_half_even"): ["round_half_even(1 / 3)"],
    ("function", "if"): ["if(true, 1, 2)", "if(false, 1h, 2h)", "if(true, true, false)"],
    ("function", "sum"): ["sum(entity:unit: entity:unit[$].level)", "sum(entity:unit: 1h)"],
    ("function", "count"): ["count(data:costs: data:costs[$].a > 20)"],
    ("function", "lookup"): ["lookup(rates, rate, 2)"],
    ("function", "cost"): ["cost(act.daily, res.a)"],
    ("function", "yield"): ["yield(act.collect, res.a)"],
    ("function", "max_single_income"): ["max_single_income(res.a, cap.a.soft)"],
}


def test_every_operator_and_function_returns_only_fraction_bool_or_duration():
    assert set(SAMPLES) == (
        {("binary", op) for op in BINARY_OPERATORS}
        | {("unary", op) for op in UNARY_OPERATORS}
        | {("function", name) for name in FUNCTIONS}
    )
    for texts in SAMPLES.values():
        for text in texts:
            result = run(text)
            assert type(result) in (Fraction, bool, Duration), (text, result)
            if type(result) is Duration:
                assert type(result.seconds) is Fraction, text
