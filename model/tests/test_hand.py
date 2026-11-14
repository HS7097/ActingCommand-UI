"""#466 W4a: the fixed values written by hand (`hand` of bindings.json), `bindings check --strict-values`, and the fixed
values the Runtime refuses beside another member of their rule (contracts/formats.md, "A value written by hand", "Fixed
values the Runtime refuses", "`acmodel bindings check`").

Every fixture is the neutral one of test_values: its value val.a.day is 2448 milli, confirmed, and a schedule.json of one
rule a line. A hand entry's `status` and `ref` are texts the tool stores and never reads, so the tests write any text.
The pools that the conflicts need are written here: pool.v with a valuation block, pool.w with the pending marker, pool.z
without a valuation."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli
from acmodel.codes import EXIT_STATUS
from test_economy import CODES
from test_values import (
    DAY_DONE,
    DAY_RULE,
    DAY_RUN,
    NIGHT_DONE,
    NIGHT_RULE,
    NIGHT_RUN,
    WORD,
    base,
    edit,
    one_line,
    values_repo,
)

VALUE = "/fixed_value_milli"
POOLS = (
    "[" + ",".join((
        '{"id":"pool.v","capacity":10,"valuation":{"name":"N","unit":"u","scale":1,"base_weight_milli":4}}',
        '{"id":"pool.w","capacity":10,"valuation":"pending"}',
        '{"id":"pool.z","capacity":10}',
    )) + "]"
)
HAND_RULE = '{"id":"rule.h","package_id":"pkg.h","fixed_value_milli":500,"priority":3}'


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


def bindings(acmodel, folder: Path, command: str, *flags: str) -> tuple[int, dict[str, Any]]:
    return acmodel("bindings", command, base(folder), "--state", folder / "state.json", *flags)


def register(folder: Path, *hands: dict[str, Any], values: tuple[tuple[str, str], ...] = ()) -> None:
    """bindings.json with the value bindings `values` (a value of exports.json and its rule) and the hand entries
    `hands`."""
    entries = [{"value": value, "file": "schedule.json", "rule_id": rule_id, "path": VALUE} for value, rule_id in values]
    edit(base(folder) / "bindings.json", lambda doc: doc.update(bindings=entries, hand=list(hands)))


def hand(rule_id: str, status: str = "written by hand", ref: str = "R-7", **optional: str) -> dict[str, Any]:
    return {"rule_id": rule_id, "status": status, "ref": ref, **optional}


def held(raw: bytes, *packages: str) -> bytes:
    """A schedule.json of `one_line` with these packages held, after its pools."""
    assert raw.endswith(b"}\n")
    entries = ",".join('{"package_id":"' + package + '","reason":"waits"}' for package in packages)
    return raw[:-2] + (',"held":[' + entries + "]}\n").encode("utf-8")


def done(rule_id: str, package: str, *members: str) -> str:
    """A rule whose value val.a.day already wrote, with its origin, and `members` after its priority."""
    return ('{"id":"' + rule_id + '","package_id":"' + package + '","fixed_value_milli":2448,"fixed_value_origin":"model",'
            + '"priority":1' + "".join("," + member for member in members) + "}")


def detail(folder: Path, entry: str, rule_id: str, **values: str) -> dict[str, Any]:
    return {"path": str(folder / "schedule.json"), "entry": entry, "rule_id": rule_id, **values}


# Values written by hand ---------------------------------------------------------------------------------------------


def test_a_value_written_by_hand_is_listed_as_written_and_never_written(tmp_path, acmodel):
    """rule.a.day's value is the model's (a value binding), rule.h's a person's (a hand entry). `check` passes and lists
    the hand entry with its texts as written, which it never reads; `apply` writes nothing of it; `--strict-values`
    counts both fixed values, each named once."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([DAY_DONE, HAND_RULE]))
    texts = {"status": "placeholder (lower bound)", "ref": "board item 12, row 3",
             "to_replace_with": "a value of the model, once its price is accepted", "inherited_from": "rule.a.day",
             "note": "any words at all"}
    register(folder, {"rule_id": "rule.h", **texts}, values=(("val.a.day", "rule.a.day"),))
    before = schedule.read_bytes()

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["hand"] == [{"entry": "/hand/0", "rule_id": "rule.h", "literal": 500, **texts}]
    assert doc["data"]["strict_values"] is False
    assert "fixed_values" not in doc["data"]

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["data"]["changed"], doc["data"]["written"]) == (0, [], [], [])
    assert schedule.read_bytes() == before

    status, doc = bindings(acmodel, folder, "check", "--strict-values")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["strict_values"], doc["data"]["fixed_values"]) == (True, 2)


def test_strict_values_refuses_a_fixed_value_that_no_binding_or_hand_entry_names(tmp_path, acmodel):
    """rule.a.night writes a fixed value that nothing names, and its package is held: it is counted all the same.
    Without the flag, and with `--strict` alone (which scans the predicates' and pools' literals, not the values),
    `check` passes; with `--strict-values` it is `fixed_value_unregistered`. A rule without a fixed value is not counted."""
    folder = values_repo(tmp_path, [DAY_RUN])
    plain = '{"id":"rule.plain","package_id":"pkg.p","priority":1}'
    night = NIGHT_RULE.replace('"pkg.a"', '"pkg.n"')
    schedule = folder / "schedule.json"
    schedule.write_bytes(held(one_line([DAY_DONE, HAND_RULE, night, plain]), "pkg.n"))
    register(folder, hand("rule.h"), values=(("val.a.day", "rule.a.day"),))

    assert bindings(acmodel, folder, "check")[0] == 0
    status, doc = bindings(acmodel, folder, "check", "--strict")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["strict"], doc["data"]["strict_values"], doc["data"]["literals"]) == (True, False, 0)
    assert "fixed_values" not in doc["data"]

    status, doc = bindings(acmodel, folder, "check", "--strict-values")

    assert (status, doc["data"]) == (6, {})
    assert doc["errors"] == [{
        "code": "fixed_value_unregistered", "detail": {"path": str(schedule), "rule_id": "rule.a.night"},
        "ref": "/rules/2/fixed_value_milli",
    }]

    register(folder, hand("rule.h"), hand("rule.a.night"), values=(("val.a.day", "rule.a.day"),))
    status, doc = bindings(acmodel, folder, "check", "--strict", "--strict-values")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["literals"], doc["data"]["fixed_values"]) == (0, 3)


def test_a_rule_is_named_once_among_the_value_bindings_and_the_hand_entries(tmp_path, acmodel):
    """A hand entry of a rule that a value binding names, and a second hand entry of one rule: each is its own code (not
    model_id_duplicate), found before the schedule is read, in check and in apply alike; nothing is written."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([DAY_RULE, HAND_RULE]))
    register(folder, hand("rule.a.day"), hand("rule.h"), hand("rule.h", status="again"),
             values=(("val.a.day", "rule.a.day"),))
    before = schedule.read_bytes()
    path = str(base(folder) / "bindings.json")
    expected = [
        {"code": "hand_also_bound",
         "detail": {"path": path, "entry": "/hand/0", "rule_id": "rule.a.day", "binding": "/bindings/0",
                    "value": "val.a.day"},
         "ref": "/hand/0"},
        {"code": "hand_duplicate",
         "detail": {"path": path, "entry": "/hand/2", "rule_id": "rule.h", "first_entry": "/hand/1"},
         "ref": "/hand/2"},
    ]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["data"], doc["errors"]) == (1, {}, expected)
    assert schedule.read_bytes() == before
    assert not (folder / "schedule.json.part").exists()


def test_a_hand_entrys_rule_holds_its_value_and_no_origin(tmp_path, acmodel):
    """A rule without a fixed value (`hand_value_missing`), one marked as the model's (`hand_origin_model`, and not also
    `origin_unbound`), one with another origin (`origin_invalid`, which the Runtime refuses), one whose value is no
    integer and an id that names no rule (`binding_mismatch` `literal_missing`, as for a value binding). `apply`
    refuses each and writes nothing."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([
        '{"id":"rule.none","package_id":"pkg.x","priority":1}',
        '{"id":"rule.m","package_id":"pkg.x","fixed_value_milli":5,"fixed_value_origin":"model","priority":1}',
        '{"id":"rule.p","package_id":"pkg.x","fixed_value_milli":6,"fixed_value_origin":"pack","priority":1}',
        '{"id":"rule.s","package_id":"pkg.x","fixed_value_milli":"7","priority":1}',
    ]))
    register(folder, hand("rule.none"), hand("rule.m"), hand("rule.p"), hand("rule.s"), hand("rule.gone"))
    before = schedule.read_bytes()
    expected = [
        {"code": "hand_value_missing", "detail": detail(folder, "/hand/0", "rule.none"), "ref": "/rules/0"},
        {"code": "hand_origin_model", "detail": detail(folder, "/hand/1", "rule.m"),
         "ref": "/rules/1/fixed_value_origin"},
        {"code": "origin_invalid", "detail": detail(folder, "/hand/2", "rule.p", found="pack"),
         "ref": "/rules/2/fixed_value_origin"},
        {"code": "binding_mismatch",
         "detail": detail(folder, "/hand/3", "rule.s", reason="literal_missing", found="string"),
         "ref": "/rules/3/fixed_value_milli"},
        {"code": "binding_mismatch", "detail": detail(folder, "/hand/4", "rule.gone", reason="literal_missing"),
         "ref": "/rules"},
    ]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (6, expected)
    assert schedule.read_bytes() == before


def test_a_hand_entry_names_no_file_and_no_path_and_writes_its_texts(tmp_path, acmodel):
    """The literal of a hand entry is always its rule's fixed value, so `file` and `path` are unknown members; `status`
    and `ref` are non-empty texts, and `inherited_from` has the shape of a rule id. The reader refuses each."""
    folder = values_repo(tmp_path, [DAY_RUN])
    (folder / "schedule.json").write_bytes(one_line([HAND_RULE]))
    register(folder, {"rule_id": "rule.h", "status": "", "ref": "R-7", "file": "schedule.json", "path": VALUE,
                      "inherited_from": "Rule H"})
    path = str(base(folder) / "bindings.json")

    def refused(reason: str, ref: str, **values: str) -> dict[str, Any]:
        return {"code": "model_file_invalid",
                "detail": {"reason": reason, "path": path, "format": "acmodel.bindings.v1", **values}, "ref": ref}

    status, doc = bindings(acmodel, folder, "check")

    assert status == 1
    assert doc["errors"] == [
        refused("member_unknown", "/hand/0/file", key="file"),
        refused("member_unknown", "/hand/0/path", key="path"),
        refused("value_invalid", "/hand/0/status", literal=""),
        refused("id_invalid", "/hand/0/inherited_from", literal="Rule H"),
    ]


def test_a_hand_written_value_needs_the_word_too(tmp_path, acmodel):
    """The Runtime reads a fixed value only under `benefit_fixed_value`, whoever wrote it: a schedule.json whose
    `requires` lacks it is refused for a hand entry as for a value binding, and the word is not added."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([HAND_RULE], requires='["fact_reference"]'))
    register(folder, hand("rule.h"))

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == [{"code": "requires_word_missing", "detail": {"path": str(schedule), "word": WORD},
                              "ref": "/requires"}]


def test_apply_writes_a_value_binding_beside_a_hand_entry_and_no_byte_of_the_hand_rule(tmp_path, acmodel):
    """fix1. rule.a.night's value is the model's and stale (7000, no origin: val.a.night is 9072), rule.h's a person's.
    `apply` writes rule.a.night's value and origin as it would without the hand entry, and rule.h keeps every byte; then
    `check --strict-values` passes and lists the hand entry."""
    folder = values_repo(tmp_path, [NIGHT_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([NIGHT_RULE, HAND_RULE]))
    register(folder, hand("rule.h"), values=(("val.a.night", "rule.a.night"),))

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert schedule.read_bytes() == one_line([NIGHT_DONE, HAND_RULE])
    assert doc["data"]["written"] == [str(schedule)]
    assert [(found["rule_id"], found["from"], found["to"], found["inserted"]) for found in doc["data"]["changed"]] == [
        ("rule.a.night", 7000, 9072, ["fixed_value_origin"]),
    ]

    status, doc = bindings(acmodel, folder, "check", "--strict-values")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["hand"] == [
        {"entry": "/hand/0", "rule_id": "rule.h", "literal": 500, "status": "written by hand", "ref": "R-7"},
    ]
    assert doc["data"]["fixed_values"] == 2


def test_a_hand_entrys_ref_is_a_non_empty_text_as_its_status_is(tmp_path, acmodel):
    """fix1. An empty `ref` is refused by the reader, as an empty `status` is, in check and in apply alike."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([HAND_RULE]))
    register(folder, hand("rule.h", ref=""))
    before = schedule.read_bytes()
    expected = [{
        "code": "model_file_invalid",
        "detail": {"reason": "value_invalid", "path": str(base(folder) / "bindings.json"), "format": "acmodel.bindings.v1",
                   "literal": ""},
        "ref": "/hand/0/ref",
    }]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (1, expected)
    assert schedule.read_bytes() == before


def test_a_hand_entrys_value_is_no_threshold_bindings_literal_and_no_fixed_entrys(tmp_path, acmodel):
    """fix1. rule.h's fixed value is a person's, and a threshold binding names it too: `apply` would write the threshold's
    number into it (`hand_also_threshold`). rule.k's is a fixed entry's literal too: one number registered twice
    (`hand_also_fixed`). Each is its own code, neither `hand_also_bound` (a value binding's) nor `model_id_duplicate`,
    found before the schedule is read, in check and in apply alike; nothing is written. A threshold binding of another
    literal of a hand entry's rule (rule.k's priority) is none of these."""
    folder = values_repo(tmp_path, [DAY_RUN])
    edit(base(folder) / "exports.json",
         lambda doc: doc["thresholds"].append({"id": "thr.a", "from": "rate.a", "form": "constant"}))
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([HAND_RULE, '{"id":"rule.k","package_id":"pkg.k","fixed_value_milli":600,"priority":3}']))
    entries = [
        {"threshold": "thr.a", "file": "schedule.json", "rule_id": "rule.h", "path": VALUE},
        {"threshold": "thr.a", "file": "schedule.json", "rule_id": "rule.k", "path": "/priority"},
    ]
    fixed = [{"file": "schedule.json", "rule_id": "rule.k", "path": VALUE, "reason": "no quantity", "ref": "R-8"}]
    edit(base(folder) / "bindings.json",
         lambda doc: doc.update(bindings=entries, fixed=fixed, hand=[hand("rule.h"), hand("rule.k")]))
    before = schedule.read_bytes()
    path = str(base(folder) / "bindings.json")
    expected = [
        {"code": "hand_also_threshold",
         "detail": {"path": path, "entry": "/hand/0", "rule_id": "rule.h", "binding": "/bindings/0", "threshold": "thr.a"},
         "ref": "/hand/0"},
        {"code": "hand_also_fixed",
         "detail": {"path": path, "entry": "/hand/1", "rule_id": "rule.k", "fixed_entry": "/fixed/0"},
         "ref": "/hand/1"},
    ]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["data"], doc["errors"]) == (1, {}, expected)
    assert schedule.read_bytes() == before
    assert not (folder / "schedule.json.part").exists()


def test_a_hand_written_value_outside_what_the_runtime_compiles_is_refused(tmp_path, acmodel):
    """fix1. The Runtime's compiler takes a fixed value in 1..2^53 - 1 only (a value of 0 is no value): rule.z's 0 and
    rule.n's -5 are each `hand_value_out_of_range`, at the member, in check and in apply, rule.n's though its package is
    held (as a value binding's number below 1 is `value_not_positive`, held or not); rule.one's 1 passes. Above 2^53 - 1
    the strict reader refuses the file first."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(held(one_line([
        '{"id":"rule.z","package_id":"pkg.x","fixed_value_milli":0,"priority":1}',
        '{"id":"rule.n","package_id":"pkg.n","fixed_value_milli":-5,"priority":1}',
        '{"id":"rule.one","package_id":"pkg.x","fixed_value_milli":1,"priority":1}',
    ]), "pkg.n"))
    register(folder, hand("rule.z"), hand("rule.n"), hand("rule.one"))
    before = schedule.read_bytes()
    expected = [
        {"code": "hand_value_out_of_range", "detail": detail(folder, "/hand/0", "rule.z", found="0"),
         "ref": "/rules/0/fixed_value_milli"},
        {"code": "hand_value_out_of_range", "detail": detail(folder, "/hand/1", "rule.n", found="-5"),
         "ref": "/rules/1/fixed_value_milli"},
    ]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"], doc["warnings"]) == (6, expected, [])
    assert schedule.read_bytes() == before


def test_two_value_bindings_of_one_rule_are_a_duplicate_with_or_without_strict_values(tmp_path, acmodel):
    """fix1. Two value bindings of one rule name one literal: `model_id_duplicate`, before the schedule is read, with
    `--strict-values` as without it (the flag adds no code for "named twice")."""
    folder = values_repo(tmp_path, [DAY_RUN, NIGHT_RUN])
    (folder / "schedule.json").write_bytes(one_line([DAY_DONE]))
    register(folder, values=(("val.a.day", "rule.a.day"), ("val.a.night", "rule.a.day")))
    path = str(base(folder) / "bindings.json")
    expected = [{
        "code": "model_id_duplicate",
        "detail": {"path": path, "namespace": "literal", "id": "schedule.json:rule.a.day:/fixed_value_milli",
                   "first_path": path, "first_ref": "/bindings/0"},
        "ref": "/bindings/1",
    }]

    for flags in ((), ("--strict-values",)):
        status, doc = bindings(acmodel, folder, "check", *flags)

        assert (status, doc["data"], doc["errors"]) == (1, {}, expected)


# Fixed values the Runtime refuses -------------------------------------------------------------------------------------


def test_a_bound_value_beside_a_member_the_runtime_refuses_it_by_is_refused_one_code_each(tmp_path, acmodel):
    """Five rules bound to val.a.day. rule.r writes the realization mark; rule.v produces pool.z (no valuation: no
    conflict) and pool.v (a block); rule.w produces pool.w (the pending marker); rule.m declares itself maintenance and
    has no value yet, which `apply` would insert; rule.z only produces pool.z. Each conflict is its own error, at its
    member, and `apply` refuses them all and writes nothing, not even rule.m's value."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([
        done("rule.r", "pkg.r", '"realization":true'),
        done("rule.v", "pkg.v", '"produces":[{"pool_id":"pool.z","amount":1},{"pool_id":"pool.v","amount":1}]'),
        done("rule.w", "pkg.w", '"produces":[{"pool_id":"pool.w","amount":1}]'),
        '{"id":"rule.m","package_id":"pkg.m","maintenance":true,"priority":9}',
        done("rule.z", "pkg.z", '"produces":[{"pool_id":"pool.z","amount":1}]'),
    ], pools=POOLS))
    rules = ("rule.r", "rule.v", "rule.w", "rule.m", "rule.z")
    register(folder, values=tuple(("val.a.day", rule_id) for rule_id in rules))
    before = schedule.read_bytes()

    def bound(index: int, **values: str) -> dict[str, Any]:
        return detail(folder, f"/bindings/{index}", rules[index], value="val.a.day", **values)

    conflicts = [
        {"code": "fixed_value_beside_realization", "detail": bound(0), "ref": "/rules/0/realization"},
        {"code": "fixed_value_beside_priced_product", "detail": bound(1, pool_id="pool.v"),
         "ref": "/rules/1/produces/1"},
        {"code": "fixed_value_beside_pending_product", "detail": bound(2, pool_id="pool.w"),
         "ref": "/rules/2/produces/0"},
        {"code": "fixed_value_beside_maintenance", "detail": bound(3), "ref": "/rules/3/maintenance"},
    ]

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == conflicts + [
        {"code": "binding_mismatch", "detail": bound(3, reason="value", expected="2448"), "ref": "/rules/3"},
        {"code": "origin_missing", "detail": bound(3), "ref": "/rules/3"},
    ]

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"], doc["errors"]) == (6, {}, conflicts)
    assert schedule.read_bytes() == before
    assert not (folder / "schedule.json.part").exists()


def test_on_a_held_rule_the_conflicts_are_warnings_until_the_hold_is_lifted(tmp_path, acmodel):
    """The composer leaves a held package's rules out of the catalog, so the Runtime refuses none of these yet: the
    realization mark, a block and the pending marker are warnings, and `apply` writes rule.r's value. A maintenance
    declaration is the reader's, which reads every rule, so it stays an error on a held rule. Once the hold is lifted,
    the warnings are errors."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    rules = [
        '{"id":"rule.r","package_id":"pkg.r","realization":true,"priority":1}',
        done("rule.v", "pkg.v", '"produces":[{"pool_id":"pool.v","amount":1}]'),
        done("rule.w", "pkg.w", '"produces":[{"pool_id":"pool.w","amount":1}]'),
    ]
    schedule.write_bytes(held(one_line(rules, pools=POOLS), "pkg.r", "pkg.v", "pkg.w", "pkg.m"))
    names = ("rule.r", "rule.v", "rule.w", "rule.m")
    register(folder, values=tuple(("val.a.day", rule_id) for rule_id in names[:3]))

    def bound(index: int, **values: str) -> dict[str, Any]:
        return detail(folder, f"/bindings/{index}", names[index], value="val.a.day", **values)

    later = [
        {"code": "held_fixed_value_beside_realization", "detail": bound(0), "ref": "/rules/0/realization"},
        {"code": "held_fixed_value_beside_priced_product", "detail": bound(1, pool_id="pool.v"),
         "ref": "/rules/1/produces/0"},
        {"code": "held_fixed_value_beside_pending_product", "detail": bound(2, pool_id="pool.w"),
         "ref": "/rules/2/produces/0"},
    ]

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == [
        {"code": "binding_mismatch", "detail": bound(0, reason="value", expected="2448"), "ref": "/rules/0"},
        {"code": "origin_missing", "detail": bound(0), "ref": "/rules/0"},
    ]
    assert doc["warnings"] == later

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], later)
    written = [done("rule.r", "pkg.r").replace('"priority":1', '"realization":true,"priority":1'), *rules[1:]]
    assert schedule.read_bytes() == held(one_line(written, pools=POOLS), "pkg.r", "pkg.v", "pkg.w", "pkg.m")
    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], later)

    schedule.write_bytes(held(one_line([*written, done("rule.m", "pkg.m", '"maintenance":true')], pools=POOLS),
                              "pkg.r", "pkg.v", "pkg.w", "pkg.m"))
    register(folder, values=tuple(("val.a.day", rule_id) for rule_id in names))
    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == [{"code": "fixed_value_beside_maintenance", "detail": bound(3), "ref": "/rules/3/maintenance"}]
    assert doc["warnings"] == later

    schedule.write_bytes(one_line(written, pools=POOLS))
    register(folder, values=tuple(("val.a.day", rule_id) for rule_id in names[:3]))
    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == [{**warning, "code": warning["code"].removeprefix("held_")} for warning in later]
    assert doc["warnings"] == []


def test_a_hand_written_value_the_runtime_refuses_is_refused_too_and_refuses_the_whole_apply(tmp_path, acmodel):
    """rule.hr's value is written by hand beside the realization mark: an error, as for a bound value (without `value`),
    and `apply` writes not even rule.a.day's bound value. rule.hh produces a priced pool but is held: a warning."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(held(one_line([
        DAY_RULE,
        '{"id":"rule.hr","package_id":"pkg.hr","fixed_value_milli":40,"realization":true,"priority":1}',
        '{"id":"rule.hh","package_id":"pkg.hh","fixed_value_milli":41,"priority":1,'
        '"produces":[{"pool_id":"pool.v","amount":1}]}',
    ], pools=POOLS), "pkg.hh"))
    register(folder, hand("rule.hr"), hand("rule.hh"), values=(("val.a.day", "rule.a.day"),))
    before = schedule.read_bytes()
    day = detail(folder, "/bindings/0", "rule.a.day", value="val.a.day")
    refused = {"code": "fixed_value_beside_realization", "detail": detail(folder, "/hand/0", "rule.hr"),
               "ref": "/rules/1/realization"}
    later = [{"code": "held_fixed_value_beside_priced_product",
              "detail": detail(folder, "/hand/1", "rule.hh", pool_id="pool.v"), "ref": "/rules/2/produces/0"}]

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"] == [
        {"code": "binding_mismatch", "detail": {**day, "reason": "value", "expected": "2448"}, "ref": "/rules/0"},
        {"code": "origin_missing", "detail": day, "ref": "/rules/0"},
        refused,
    ]
    assert doc["warnings"] == later

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (6, [refused], later)
    assert schedule.read_bytes() == before


# The catalog ------------------------------------------------------------------------------------------------------------


def test_the_codes_of_w4a_are_registered_with_their_category_and_exit_status():
    """Each code is one reason: an error the Runtime's refusal makes one (exit 6), a held rule's the warning it stays
    until the hold is lifted; a rule named twice in bindings.json is an input error (exit 1), as a duplicate is. fix1: a
    hand entry's value that a threshold binding or a fixed entry names too is that input error; one outside the range the
    Runtime compiles is an error of the schedule (exit 6)."""
    assert set(EXIT_STATUS) == set(CODES)
    new = {
        "hand_also_bound": ("error", 1), "hand_duplicate": ("error", 1), "hand_value_missing": ("error", 6),
        "hand_origin_model": ("error", 6), "fixed_value_unregistered": ("error", 6),
        "hand_also_threshold": ("error", 1), "hand_also_fixed": ("error", 1), "hand_value_out_of_range": ("error", 6),
        "fixed_value_beside_realization": ("error", 6), "fixed_value_beside_priced_product": ("error", 6),
        "fixed_value_beside_pending_product": ("error", 6), "fixed_value_beside_maintenance": ("error", 6),
        "held_fixed_value_beside_realization": ("warning", 0),
        "held_fixed_value_beside_priced_product": ("warning", 0),
        "held_fixed_value_beside_pending_product": ("warning", 0),
    }
    assert {code: (CODES[code]["category"], CODES[code]["exit"]) for code in new} == new
    assert {code: EXIT_STATUS[code] for code in new} == {code: status for code, (_, status) in new.items()}
