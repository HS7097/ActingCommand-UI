"""#466 W3, the model's writer of the pools' valuations and of the limited mark: `acmodel export valuations`
(`out/valuations.json`, apart from `out/values.json`, and the valuation provenance files), the valuation bindings of
`bindings check` and `bindings apply` (W written in place, `valuation_origin` inserted after the block in the file's own
style, Q never written), the three range and precision codes with their suggested scale, the origin guard of the pools,
and the limited mark of the value bindings (contracts/formats.md, "`acmodel export valuations`", "A pool's valuation" and
"A value's rule").

Every fixture is neutral, the one of test_values: four pools, one activity with one flow, and an `out/prices.json`
written as `export prices` writes it. With κ = 1000 the prices in milli are pool.a 4 (or another price a test writes),
pool.b 100, pool.c 50 (provisional, accepted) and pool.d pending. The schedule's pools carry other ids than the model's
resources where a test needs several pools of one price: a valuation binding names both. Each docstring works W out by
hand: W = the price in milli × the pool's scale Q, rounded as the binding declares."""

from __future__ import annotations

import json
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli
from acmodel.codes import EXIT_STATUS
from acmodel.core.versions import sha256 as doc_sha256
from acmodel.export.valuations import PRECISION_BOUND, fits, suggest
from test_economy import CODES, activity, limit
from test_values import (
    DAY_RUN,
    MAKE,
    MAKE_SPENT,
    PRICE_C,
    PRICES,
    WORD,
    base,
    edit,
    model_versions,
    one_line,
    price,
    sha,
    values_repo,
    write_prices,
)

PATH = "/valuation/base_weight_milli"
BOUND = str(PRECISION_BOUND)


@pytest.fixture
def acmodel(make_system, app_dir):
    """One command in the venv: (exit status, envelope). The envelope is one acmodel.cli.v1 line whose errors and
    warnings agree with contracts/codes.json (each code of its category, with its values), and the exit status the
    first error's."""

    def run(*argv: Any) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main([str(arg) for arg in argv], system)
        [line] = system.stdout.getvalue().splitlines()
        doc = json.loads(line)
        assert doc["ok"] is (doc["errors"] == [])
        for kind, entries in (("error", doc["errors"]), ("warning", doc["warnings"])):
            for entry in entries:
                spec = CODES[entry["code"]]
                assert spec["category"] == kind, entry
                assert set(entry["detail"]) <= set(spec["values"]), entry
                for key, value in spec["values"].items():
                    assert not value["required"] or key in entry["detail"], (entry, key)
                    assert "members" not in value or key not in entry["detail"] or entry["detail"][key] in value["members"]
        assert status == (CODES[doc["errors"][0]["code"]]["exit"] if doc["errors"] else 0)
        return status, doc

    return run


def export_valuations(acmodel, folder: Path) -> tuple[int, dict[str, Any]]:
    return acmodel("export", "valuations", base(folder), "--state", folder / "state.json")


def bindings(acmodel, folder: Path, command: str) -> tuple[int, dict[str, Any]]:
    return acmodel("bindings", command, base(folder), "--state", folder / "state.json")


def bind(folder: Path, *valuations: tuple[str, str, str], values: tuple[tuple[str, str], ...] = ()) -> None:
    """bindings.json with the value bindings `values` (a value and its rule), then a valuation binding for each of
    `valuations` (a resource of the model, the pool of schedule.json and its rounding)."""
    entries: list[dict[str, Any]] = [
        {"value": value, "file": "schedule.json", "rule_id": rule_id, "path": "/fixed_value_milli"}
        for value, rule_id in values
    ]
    entries += [
        {"valuation": resource, "file": "schedule.json", "pool_id": pool_id, "path": PATH, "round": rounding}
        for resource, pool_id, rounding in valuations
    ]
    edit(base(folder) / "bindings.json", lambda doc: doc.update(bindings=entries))


def priced(folder: Path, value: str) -> None:
    """prices.json with pool.a's price `value` (κ 1000), the other pools as the fixture has them."""
    write_prices(folder, [price("pool.a", value), *PRICES[1:]])


def pool(pool_id: str, scale: Any, weight: Any, *, origin: str | None = None) -> str:
    """A pool of one line, without spaces: its valuation block, its origin when given, then its capacity."""
    block = '{"name":"N","unit":"u","scale":' + json.dumps(scale) + ',"base_weight_milli":' + json.dumps(weight) + "}"
    after = "" if origin is None else ',"valuation_origin":' + origin
    return '{"id":"' + pool_id + '","valuation":' + block + after + ',"capacity":10}'


def schedule(*pools: str, rules: tuple[str, ...] = (), requires: str = f'["{WORD}"]') -> bytes:
    """A schedule.json of one rule a line and no spaces, in LF, with these pools."""
    return one_line(list(rules), requires=requires, pools="[" + ",".join(pools) + "]")


def entry(pool_id: str, resource: str, status: str, rounding: str, scale: int, milli: str | None, weight: int | None,
          *, exact: str | None = None, precision: dict[str, Any] | None = None,
          acceptances: tuple[dict[str, str], ...] = (), pending: tuple[dict[str, Any], ...] = ()) -> dict[str, Any]:
    """A valuation's entry of valuations.json (`exact`, W before rounding, when it is no integer)."""
    return {
        "pool_id": pool_id, "valuation": resource, "status": status, "round": rounding, "scale": scale, "milli": milli,
        "base_weight_milli": weight,
        "exact_weight_milli": exact if exact is not None else (None if weight is None else str(weight)),
        "precision": precision, "acceptances": list(acceptances), "pending": list(pending),
        "provenance": f"out/provenance/valuations/{pool_id}.json",
    }


def described(index: int, resource: str, pool_id: str, rounding: str) -> dict[str, Any]:
    return {"entry": f"/bindings/{index}", "valuation": resource, "file": "schedule.json", "pool_id": pool_id,
            "path": PATH, "round": rounding}


def delivered(folder: Path) -> dict[str, Any]:
    return json.loads((base(folder) / "out" / "valuations.json").read_text(encoding="utf-8"))


# export valuations ----------------------------------------------------------------------------------------------------


def test_export_valuations_writes_w_into_its_own_file_and_values_json_keeps_every_byte(tmp_path, acmodel):
    """pool.b: 100 milli × Q 1000 = 100000, confirmed. pool.c: 50 × 10 = 500, provisional on its price's acceptance.
    pool.d's price is pending, so that valuation is pending with why. The valuations go into out/valuations.json, of its
    own format; out/values.json, written before, keeps every byte, and schedule.json too (an export writes no schedule)."""
    folder = values_repo(tmp_path, [DAY_RUN])
    (folder / "schedule.json").write_bytes(schedule(pool("pool.b", 1000, 1), pool("pool.c", 10, 1), pool("pool.d", 1, 1)))
    bind(folder, ("pool.b", "pool.b", "half_even"), ("pool.c", "pool.c", "floor"), ("pool.d", "pool.d", "ceil"))
    out = base(folder) / "out"
    assert acmodel("export", "values", base(folder), "--state", folder / "state.json")[0] == 0
    values = (out / "values.json").read_bytes()
    before = (folder / "schedule.json").read_bytes()

    status, doc = export_valuations(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert (out / "values.json").read_bytes() == values
    assert (folder / "schedule.json").read_bytes() == before
    assert doc["data"]["counts"] == {"confirmed": 1, "provisional": 1, "pending": 1, "precision_low": 0}
    assert doc["data"]["written"] == sorted(
        [str(out / "valuations.json")] + [str(out / "provenance" / "valuations" / f"{name}.json")
                                          for name in ("pool.b", "pool.c", "pool.d")]
    )
    raw = (out / "valuations.json").read_bytes()
    written = delivered(folder)
    assert raw == (json.dumps(written, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
    assert {key: value for key, value in written.items() if key not in ("valuations", "counts")} == {
        "schema_version": "acmodel.valuations.v1",
        "model_id": "economy",
        "versions": model_versions(base(folder)),
        "prices": {"file": "out/prices.json", "sha256": sha(out / "prices.json"), "versions": model_versions(base(folder))},
        "bindings": {"file": "bindings.json", "sha256": sha(base(folder) / "bindings.json")},
        "schedule": {"file": "schedule.json"},
        "unit_milli": 1000,
        "precision_bound": "1/200",
    }
    assert written["counts"] == doc["data"]["counts"]
    assert written["valuations"] == [
        entry("pool.b", "pool.b", "confirmed", "half_even", 1000, "100", 100000),
        entry("pool.c", "pool.c", "provisional", "floor", 10, "50", 500, acceptances=(PRICE_C,)),
        entry("pool.d", "pool.d", "pending", "ceil", 1, None, None,
              pending=({"reason": "price_pending", "item": "pool.d", "pending": ["not_accepted"]},)),
    ]

    document = json.loads((out / "provenance" / "valuations" / "pool.b.json").read_text(encoding="utf-8"))
    binding = json.loads((base(folder) / "bindings.json").read_text(encoding="utf-8"))["bindings"][0]
    assert (document["schema_version"], document["output"], document["versions"]) == (
        "acmodel.provenance.v1", "valuation:pool.b", model_versions(base(folder)))
    assert document["result"] == {key: value for key, value in written["valuations"][0].items() if key != "provenance"}
    assert document["nodes"] == {
        "valuation:pool.b": {
            "node": "valuation", "pool_id": "pool.b", "resource": "pool.b", "file": "bindings.json", "ref": "/bindings/0",
            "sha256": doc_sha256(binding),
            "schedule": {"file": "schedule.json", "ref": "/pools/0/valuation/scale", "scale": 1000},
            "reads": ["price:pool.b"],
        },
        "price:pool.b": {
            "node": "price", "resource": "pool.b", "file": "out/prices.json", "sha256": sha(out / "prices.json"),
            "class": "covered", "status": "confirmed", "price": "1/10", "milli": "100",
            "provenance": "out/provenance/prices/pool.b.json", "reads": [],
        },
    }
    assert "acceptances" not in document
    collected = json.loads((out / "provenance" / "valuations" / "pool.c.json").read_text(encoding="utf-8"))
    assert collected["acceptances"] == [PRICE_C]
    waiting = json.loads((out / "provenance" / "valuations" / "pool.d.json").read_text(encoding="utf-8"))
    assert waiting["result"]["status"] == "pending"
    assert set(waiting["nodes"]) == {"valuation:pool.d", "price:pool.d"}


# bindings: W and the pool's origin ------------------------------------------------------------------------------------

# pool.a at 1/3000 is 1/3 milli a step; Q 1000 gives 1000/3, rounded down 333, up 334, to even 333: each within 0.5 %.
THIRD = "1/3000"


def test_apply_writes_w_as_rounded_and_the_origin_and_never_q(tmp_path, acmodel):
    """Three pools of one price, three roundings. `check` names each difference (W, the absent origin); `apply` writes
    W in place and inserts the origin right after the block in the file's own style (no spaces); Q, `name`, `unit` and
    every other byte stay. Then `check` passes and shows each valuation."""
    folder = values_repo(tmp_path, [DAY_RUN])
    priced(folder, THIRD)
    path = folder / "schedule.json"
    path.write_bytes(schedule(pool("p.floor", 1000, 7), pool("p.ceil", 1000, 7), pool("p.even", 1000, 7)))
    bind(folder, ("pool.a", "p.floor", "floor"), ("pool.a", "p.ceil", "ceil"), ("pool.a", "p.even", "half_even"))
    where = str(path)

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6

    def detail(index: int, pool_id: str, **values: str) -> dict[str, Any]:
        return {"path": where, "entry": f"/bindings/{index}", "valuation": "pool.a", "pool_id": pool_id, **values}

    assert doc["errors"] == [
        error
        for index, (pool_id, weight) in enumerate((("p.floor", "333"), ("p.ceil", "334"), ("p.even", "333")))
        for error in (
            {"code": "binding_mismatch", "detail": detail(index, pool_id, reason="value", expected=weight, found="7"),
             "ref": f"/pools/{index}{PATH}"},
            {"code": "origin_missing", "detail": detail(index, pool_id), "ref": f"/pools/{index}"},
        )
    ]

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    model = '"model"'
    assert path.read_bytes() == schedule(
        pool("p.floor", 1000, 333, origin=model), pool("p.ceil", 1000, 334, origin=model),
        pool("p.even", 1000, 333, origin=model),
    )
    assert doc["data"]["written"] == [where]
    assert doc["data"]["changed"] == [
        {**described(index, "pool.a", pool_id, rounding), "from": 7, "to": weight, "ratio": str(Fraction(weight, 7)),
         "ratio_flagged": True, "origin": {"from": None, "to": "model"}, "inserted": ["valuation_origin"], "scale": 1000,
         "precision": None}
        for index, (pool_id, rounding, weight) in enumerate(
            (("p.floor", "floor", 333), ("p.ceil", "ceil", 334), ("p.even", "half_even", 333)))
    ]

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["bindings"] == [
        {**described(index, "pool.a", pool_id, rounding), "literal": weight, "origin": "model", "weight": weight,
         "scale": 1000, "status": "confirmed", "acceptances": [], "precision": None}
        for index, (pool_id, rounding, weight) in enumerate(
            (("p.floor", "floor", 333), ("p.ceil", "ceil", 334), ("p.even", "half_even", 333)))
    ]
    assert doc["data"]["valuations"] is None  # no out/valuations.json

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"]["changed"], doc["data"]["written"]) == (0, [], [])


# One pool over several lines, indented: the origin goes in on a line of its own, with the file's line end and indent.
INDENTED = [
    "{",
    '  "schema_version": "actingcommand.schedule.v1",',
    f'  "requires": ["{WORD}"],',
    '  "pools": [',
    "    {",
    '      "id": "p.even",',
    '      "valuation": {',
    '        "name": "N",',
    '        "unit": "u",',
    '        "scale": 1000,',
    '        "base_weight_milli": @W@',
    "      },",
    "@ORIGIN@",
    '      "capacity": 10',
    "    }",
    "  ]",
    "}",
]


def indented(weight: str, origin: list[str], end: str) -> bytes:
    lines: list[str] = []
    for line in INDENTED:
        if line == "@ORIGIN@":
            lines.extend(origin)
        else:
            lines.append(line.replace("@W@", weight))
    return (end.join(lines) + end).encode("utf-8")


@pytest.mark.parametrize("end", ["\r\n", "\n"], ids=["crlf", "lf"])
def test_the_origin_is_inserted_after_the_block_and_every_other_byte_stays(tmp_path, acmodel, end):
    folder = values_repo(tmp_path, [DAY_RUN])
    priced(folder, THIRD)
    path = folder / "schedule.json"
    path.write_bytes(indented("7", [], end))
    bind(folder, ("pool.a", "p.even", "half_even"))
    expected = indented("333", ['      "valuation_origin": "model",'], end)
    assert expected.count(b"\n") == expected.count(end.encode("ascii"))

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert path.read_bytes() == expected
    assert bindings(acmodel, folder, "check")[0] == 0


def test_a_block_no_member_follows_has_no_style_to_copy(tmp_path, acmodel):
    """The block is the pool's last member: no separator after it to copy, so the origin is not guessed in."""
    folder = values_repo(tmp_path, [DAY_RUN])
    priced(folder, THIRD)
    path = folder / "schedule.json"
    path.write_bytes(schedule('{"id":"p.last","capacity":10,"valuation":{"scale":1000,"base_weight_milli":333}}'))
    before = path.read_bytes()
    bind(folder, ("pool.a", "p.last", "half_even"))

    status, doc = bindings(acmodel, folder, "apply")

    assert status == 6
    assert doc["errors"] == [{
        "code": "insert_style_unknown",
        "detail": {"path": str(path), "entry": "/bindings/0", "valuation": "pool.a", "pool_id": "p.last",
                   "anchor": "valuation", "member": "valuation_origin"},
        "ref": "/pools/0",
    }]
    assert path.read_bytes() == before


# Range and precision: three reasons, three codes ------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("value", "rounding", "code", "exact", "weight", "suggested"),
    [
        # 1/10000 milli a step: Q 1000 gives 1/10, rounded down 0 at a price above 0. 10^4 steps weigh exactly 1.
        pytest.param("1/10000000", "floor", "valuation_rounds_to_zero", "1/10", "0", "10000", id="rounds_to_zero"),
        # 3000 milli a step: Q 1000 gives 3,000,000, beyond 1,000,000. One step weighs 3000, exactly.
        pytest.param("3", "half_even", "valuation_out_of_range", "3000000", "3000000", "1", id="out_of_range"),
    ],
)
def test_a_w_that_rounds_to_zero_or_leaves_the_range_refuses_everything_with_a_suggested_scale(
    tmp_path, acmodel, value, rounding, code, exact, weight, suggested
):
    """The export writes nothing, and `check` and `apply` refuse the same way; the pool's Q stays as it is."""
    folder = values_repo(tmp_path, [DAY_RUN])
    priced(folder, value)
    path = folder / "schedule.json"
    path.write_bytes(schedule(pool("p.a", 1000, 7)))
    before = path.read_bytes()
    bind(folder, ("pool.a", "p.a", rounding))
    refused = {
        "code": code,
        "detail": {"path": str(path), "entry": "/bindings/0", "valuation": "pool.a", "pool_id": "p.a", "scale": "1000",
                   "exact": exact, "weight": weight, "suggested_scale": suggested},
        "ref": "/pools/0/valuation/scale",
    }

    status, doc = export_valuations(acmodel, folder)

    assert (status, doc["data"], doc["errors"]) == (5, {}, [refused])
    assert not (base(folder) / "out" / "valuations.json").exists()
    assert not (base(folder) / "out" / "provenance" / "valuations").exists()

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (5, [refused])
    assert path.read_bytes() == before


def test_a_w_off_by_more_than_the_bound_is_a_warning_and_is_written(tmp_path, acmodel):
    """1/80 milli a step: Q 1000 gives 25/2, rounded to even 12, off by 1/25 of it (4 %, above 1/200). W is written, the
    precision recorded with the suggested scale 10^4 (125, exactly), and the warning given by each command; Q stays."""
    folder = values_repo(tmp_path, [DAY_RUN])
    priced(folder, "1/80000")
    path = folder / "schedule.json"
    path.write_bytes(schedule(pool("p.a", 1000, 7)))
    bind(folder, ("pool.a", "p.a", "half_even"))
    precision = {"error": "1/25", "bound": BOUND, "suggested_scale": 10000}
    warning = {
        "code": "precision_low",
        "detail": {"path": str(path), "entry": "/bindings/0", "valuation": "pool.a", "pool_id": "p.a", "scale": "1000",
                   "exact": "25/2", "weight": "12", "error": "1/25", "bound": BOUND, "suggested_scale": "10000"},
        "ref": "/pools/0/valuation/scale",
    }

    status, doc = export_valuations(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [warning])
    assert doc["data"]["counts"] == {"confirmed": 1, "provisional": 0, "pending": 0, "precision_low": 1}
    assert delivered(folder)["valuations"] == [
        entry("p.a", "pool.a", "confirmed", "half_even", 1000, "1/80", 12, exact="25/2", precision=precision)]

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [warning])
    assert path.read_bytes() == schedule(pool("p.a", 1000, 12, origin='"model"'))
    [changed] = doc["data"]["changed"]
    assert (changed["from"], changed["to"], changed["scale"], changed["precision"]) == (7, 12, 1000, precision)

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [warning])
    assert doc["data"]["bindings"][0]["precision"] == precision


def test_with_no_power_of_ten_that_holds_the_suggestion_says_none(tmp_path, acmodel):
    """2,000,000 milli a step: even one step weighs beyond 1,000,000, and Q is at least 1, so no scale holds: the error
    says `none` instead of a scale that would not hold."""
    folder = values_repo(tmp_path, [DAY_RUN])
    priced(folder, "2000")
    path = folder / "schedule.json"
    path.write_bytes(schedule(pool("p.a", 1, 7)))
    bind(folder, ("pool.a", "p.a", "half_even"))

    status, doc = export_valuations(acmodel, folder)

    assert status == 5
    assert doc["errors"] == [{
        "code": "valuation_out_of_range",
        "detail": {"path": str(path), "entry": "/bindings/0", "valuation": "pool.a", "pool_id": "p.a", "scale": "1",
                   "exact": "2000000", "weight": "2000000", "suggested_scale": "none"},
        "ref": "/pools/0/valuation/scale",
    }]


def test_the_suggested_scale_is_the_least_power_of_ten_that_holds_or_none():
    """The bound is inclusive: off by exactly 1/200 holds, by more does not. A price below 0, one above 1,000,000 milli a
    step and one that 10^15 steps do not lift to the bound have no suggestion."""
    assert PRECISION_BOUND == Fraction(1, 200)
    assert fits(Fraction(200), 199) and not fits(Fraction(199), 198)
    assert not fits(Fraction(1, 10), 0) and fits(Fraction(0), 0) and not fits(Fraction(-1), 0)
    assert suggest(Fraction(1, 3), "floor") == 1000  # 10^2: 33, off by 1 %; 10^3: 333, off by 0.1 %
    assert suggest(Fraction(4), "floor") == 1
    assert suggest(Fraction(2_000_000), "half_even") is None
    assert suggest(Fraction(-4), "half_even") is None
    assert suggest(Fraction(1, 10 ** 17), "floor") is None


# The word, the origin, the guard, the pending marker, the pending price -------------------------------------------------


def test_a_valuation_needs_the_word_and_an_origin_other_than_model_is_never_rewritten(tmp_path, acmodel):
    folder = values_repo(tmp_path, [DAY_RUN])
    path = folder / "schedule.json"
    bind(folder, ("pool.b", "pool.b", "half_even"))
    path.write_bytes(schedule(pool("pool.b", 10, 1000), requires='["fact_reference"]'))
    before = path.read_bytes()

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (6, [{"code": "requires_word_missing", "detail": {"path": str(path), "word": WORD},
                                            "ref": "/requires"}])
    assert path.read_bytes() == before

    invalid = {
        "code": "origin_invalid",
        "detail": {"path": str(path), "entry": "/bindings/0", "valuation": "pool.b", "pool_id": "pool.b", "found": "pack"},
        "ref": "/pools/0/valuation_origin",
    }
    path.write_bytes(schedule(pool("pool.b", 10, 7, origin='"pack"')))
    before = path.read_bytes()

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == [invalid, {
        "code": "binding_mismatch",
        "detail": {"path": str(path), "entry": "/bindings/0", "reason": "value", "valuation": "pool.b",
                   "pool_id": "pool.b", "expected": "1000", "found": "7"},
        "ref": "/pools/0" + PATH,
    }]

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"], doc["errors"]) == (6, {}, [invalid])
    assert path.read_bytes() == before


def test_the_guard_refuses_a_model_origin_no_valuation_binding_names_and_passes_a_bound_one(tmp_path, acmodel):
    """pool.b at 100 milli and Q 10 weighs 1000, written and marked `model`, and bound: it passes. pool.c says `model` too
    and no binding names it: `origin_unbound`, in check and in apply."""
    folder = values_repo(tmp_path, [DAY_RUN])
    path = folder / "schedule.json"
    model = '"model"'
    path.write_bytes(schedule(pool("pool.b", 10, 1000, origin=model), pool("pool.c", 10, 500, origin=model)))
    bind(folder, ("pool.b", "pool.b", "half_even"))
    unbound = {"code": "origin_unbound", "detail": {"path": str(path), "pool_id": "pool.c", "member": "valuation_origin"},
               "ref": "/pools/1/valuation_origin"}

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (6, [unbound])

    bind(folder, ("pool.b", "pool.b", "half_even"), ("pool.c", "pool.c", "half_even"))

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"]) == (0, [])
    assert [binding["status"] for binding in doc["data"]["bindings"]] == ["confirmed", "provisional"]


@pytest.mark.parametrize(
    ("pools", "error"),
    [
        pytest.param('{"id":"p.a","valuation":"pending","capacity":10}', lambda path: {
            "code": "valuation_pending_marker",
            "detail": {"path": path, "entry": "/bindings/0", "valuation": "pool.b", "pool_id": "p.a"},
            "ref": "/pools/0/valuation"}, id="pending_marker"),
        pytest.param(pool("p.a", 0, 7), lambda path: {
            "code": "binding_mismatch",
            "detail": {"path": path, "entry": "/bindings/0", "reason": "scale", "valuation": "pool.b", "pool_id": "p.a",
                       "found": "0"},
            "ref": "/pools/0/valuation/scale"}, id="scale_zero"),
        pytest.param('{"id":"p.other","capacity":10}', lambda path: {
            "code": "binding_mismatch",
            "detail": {"path": path, "entry": "/bindings/0", "reason": "literal_missing", "valuation": "pool.b",
                       "pool_id": "p.a"},
            "ref": "/pools"}, id="no_pool"),
    ],
)
def test_a_pool_without_a_block_to_weigh_in_is_refused_and_nothing_is_written(tmp_path, acmodel, pools, error):
    """The pending marker has no scale and the Runtime takes an origin only beside a block: the tool never makes one.
    A scale out of range, or no such pool, refuses the same way, in the export, check and apply."""
    folder = values_repo(tmp_path, [DAY_RUN])
    path = folder / "schedule.json"
    path.write_bytes(schedule(pools))
    before = path.read_bytes()
    bind(folder, ("pool.b", "p.a", "half_even"))

    status, doc = export_valuations(acmodel, folder)

    assert (status, doc["errors"]) == (6, [error(str(path))])
    assert not (base(folder) / "out" / "valuations.json").exists()
    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (6, [error(str(path))])
    assert path.read_bytes() == before


def test_a_pending_price_refuses_the_whole_apply(tmp_path, acmodel):
    """pool.d's price is pending: `apply` refuses and writes not even pool.b's W."""
    folder = values_repo(tmp_path, [DAY_RUN])
    path = folder / "schedule.json"
    path.write_bytes(schedule(pool("pool.b", 10, 7), pool("p.d", 10, 7)))
    before = path.read_bytes()
    bind(folder, ("pool.b", "pool.b", "half_even"), ("pool.d", "p.d", "half_even"))
    refused = {
        "code": "export_refused",
        "detail": {"path": str(base(folder) / "bindings.json"), "output": "valuation:p.d", "reason": "pending",
                   "value": "price_pending:pool.d"},
        "ref": "/bindings/1",
    }

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"], doc["errors"]) == (5, {}, [refused])
    assert path.read_bytes() == before
    assert not (folder / "schedule.json.part").exists()

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"][0]) == (5, refused)


def test_check_says_whether_out_valuations_json_is_current(tmp_path, acmodel):
    folder = values_repo(tmp_path, [DAY_RUN])
    (folder / "schedule.json").write_bytes(schedule(pool("pool.b", 10, 1000, origin='"model"')))
    bind(folder, ("pool.b", "pool.b", "half_even"))
    assert export_valuations(acmodel, folder)[0] == 0
    path = str(base(folder) / "out" / "valuations.json")

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["warnings"]) == (0, [])
    assert doc["data"]["valuations"] == {"file": path, "current": True, "stale": []}

    edit(base(folder) / "bindings.json", lambda doc: doc.update(note="edited"))
    status, doc = bindings(acmodel, folder, "check")

    assert status == 0
    assert doc["data"]["valuations"] == {"file": path, "current": False, "stale": ["bindings"]}
    assert doc["warnings"] == [{"code": "valuations_stale", "detail": {"path": path, "members": ["bindings"]}}]


def test_a_valuation_binding_takes_round_and_only_a_valuation_does(tmp_path, acmodel):
    """bindings.json: a valuation without `round` is `member_missing`; a value with one is `member_unknown`."""
    folder = values_repo(tmp_path, [DAY_RUN])
    (folder / "schedule.json").write_bytes(schedule(pool("pool.b", 10, 1000)))
    path = str(base(folder) / "bindings.json")
    refused = {"path": path, "format": "acmodel.bindings.v1"}
    edit(base(folder) / "bindings.json", lambda doc: doc.update(bindings=[
        {"valuation": "pool.b", "file": "schedule.json", "pool_id": "pool.b", "path": PATH},
        {"value": "val.a.day", "file": "schedule.json", "rule_id": "rule.a", "path": "/fixed_value_milli",
         "round": "floor"},
    ]))

    status, doc = bindings(acmodel, folder, "check")

    assert status == 1
    assert doc["errors"] == [
        {"code": "model_file_invalid", "detail": {"reason": "member_missing", **refused, "key": "round"},
         "ref": "/bindings/0/round"},
        {"code": "model_file_invalid", "detail": {"reason": "member_unknown", **refused, "key": "round"},
         "ref": "/bindings/1/round"},
    ]


# The limited mark -------------------------------------------------------------------------------------------------------

CONSUMER = (
    '{"id":"rule.consumer","package_id":"pkg.m","fixed_value_milli":1600,"fixed_value_origin":"model",@MARK@"priority":1,'
    '"consumes":[{"pool_id":"pool.b","amount":2}]}'
)
PLAIN = '{"id":"rule.plain","package_id":"pkg.m","fixed_value_milli":2150,"fixed_value_origin":"model","priority":1}'
MARKED = (
    '{"id":"rule.marked","package_id":"pkg.a","fixed_value_milli":2448,"fixed_value_origin":"model","limited":@LIMITED@,'
    '"priority":5}'
)


def limited_repo(folder: Path) -> Path:
    """The fixture with two limits that count act.make: limit.make, its own, and limit.shared, of another activity and
    shared with it. val.make (1600, pool.b its declared cost) and val.make.spent (2150) count act.make; val.a.day (2448)
    counts a resource only, so it is never limited."""
    values_repo(folder, [MAKE, MAKE_SPENT, DAY_RUN])

    def add(doc: dict[str, Any]) -> None:
        doc["entities"].append(activity("act.other"))
        doc["rules"].extend([
            limit("limit.make", "act.make", "3", "day"),
            limit("limit.shared", "act.other", "1", "day", shared_with=["act.make"]),
        ])

    edit(base(folder) / "rules" / "economy.json", add)
    write_prices(folder)  # of the model's versions now
    bind(folder, values=(("val.make", "rule.consumer"), ("val.make.spent", "rule.plain"), ("val.a.day", "rule.marked")))
    return folder


def test_limited_is_written_on_a_consumer_and_only_reported_elsewhere_never_removed(tmp_path, acmodel):
    """rule.consumer declares `consumes` and the model counts its value as limited: `limited_missing`, and `apply` writes
    `"limited": true` right after the origin. rule.plain is limited too but consumes nothing: only reported. rule.marked
    writes the mark while its value is not limited: only reported, never removed."""
    folder = limited_repo(tmp_path)
    path = folder / "schedule.json"
    path.write_bytes(one_line([CONSUMER.replace("@MARK@", ""), PLAIN, MARKED.replace("@LIMITED@", "true")]))
    where = str(path)
    by = ["limit.make", "limit.shared"]

    def detail(index: int, value: str, rule_id: str, **values: Any) -> dict[str, Any]:
        return {"path": where, "entry": f"/bindings/{index}", "value": value, "rule_id": rule_id, **values}

    reported = [
        {"code": "limited_without_consumes", "detail": detail(1, "val.make.spent", "rule.plain", limited_by=by),
         "ref": "/rules/1"},
        {"code": "limited_unmodelled", "detail": detail(2, "val.a.day", "rule.marked"), "ref": "/rules/2/limited"},
    ]

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (6, [
        {"code": "limited_missing", "detail": detail(0, "val.make", "rule.consumer", limited_by=by), "ref": "/rules/0"},
    ], reported)

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], reported)
    assert path.read_bytes() == one_line(
        [CONSUMER.replace("@MARK@", '"limited":true,'), PLAIN, MARKED.replace("@LIMITED@", "true")])
    [changed] = doc["data"]["changed"]
    assert (changed["rule_id"], changed["from"], changed["to"], changed["inserted"]) == (
        "rule.consumer", 1600, 1600, ["limited"])

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], reported)
    assert [(found["limited"], found["limited_by"]) for found in doc["data"]["bindings"]] == [
        (True, by), (True, by), (False, [])]


def test_limited_goes_in_with_the_origin_when_both_are_absent(tmp_path, acmodel):
    """A consumer without the origin and the mark: both go in after the value, in that order."""
    folder = limited_repo(tmp_path)
    path = folder / "schedule.json"
    consumer = CONSUMER.replace("@MARK@", "").replace(',"fixed_value_origin":"model"', "")
    path.write_bytes(one_line([consumer, PLAIN, MARKED.replace("@LIMITED@", "true")]))

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert path.read_bytes() == one_line(
        [CONSUMER.replace("@MARK@", '"limited":true,'), PLAIN, MARKED.replace("@LIMITED@", "true")])
    assert doc["data"]["changed"][0]["inserted"] == ["fixed_value_origin", "limited"]


def test_a_limited_mark_other_than_true_is_refused_and_never_rewritten(tmp_path, acmodel):
    folder = limited_repo(tmp_path)
    path = folder / "schedule.json"
    path.write_bytes(one_line([CONSUMER.replace("@MARK@", '"limited":true,'), PLAIN, MARKED.replace("@LIMITED@", "false")]))
    before = path.read_bytes()
    invalid = {
        "code": "limited_invalid",
        "detail": {"path": str(path), "entry": "/bindings/2", "value": "val.a.day", "rule_id": "rule.marked",
                   "found": "boolean"},
        "ref": "/rules/2/limited",
    }

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (6, [invalid])
    assert path.read_bytes() == before


# The catalog ------------------------------------------------------------------------------------------------------------


def test_the_codes_of_w3_are_registered_each_of_its_category():
    new = {
        "valuation_rounds_to_zero": ("error", 5), "valuation_out_of_range": ("error", 5),
        "precision_low": ("warning", 0), "valuation_pending_marker": ("error", 6), "valuations_stale": ("warning", 0),
        "limited_missing": ("error", 6), "limited_invalid": ("error", 6), "limited_without_consumes": ("warning", 0),
        "limited_unmodelled": ("warning", 0),
    }
    assert {code: (CODES[code]["category"], CODES[code]["exit"]) for code in new} == new
    assert {code: EXIT_STATUS[code] for code in new} == {code: exit_status for code, (_, exit_status) in new.items()}
    assert "scale" in CODES["binding_mismatch"]["values"]["reason"]["members"]
