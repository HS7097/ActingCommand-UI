"""PY-2: the strict readers read the worked example (design section 3.8) and refuse,
each with a JSON pointer, a repeated key, a float, NaN or Infinity, an integer
beyond +-(2**53 - 1), a BOM, an unknown member and an id outside the grammar."""

from __future__ import annotations

import io
from pathlib import Path

import pytest

from acmodel.envelope import Failure, build, write
from acmodel.model import read_data, read_model, read_prefs_defaults, read_rules, read_sources, read_state

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
MODEL = EXAMPLE / "ours" / "model"
RULES = MODEL / "rules" / "a.json"
STATE = EXAMPLE / "state" / "acct-1.json"
PREFS = EXAMPLE / "ours" / "prefs" / "defaults.json"


def variant(tmp_path: Path, source: Path, old: str, new: str) -> Path:
    """A copy of a fixture with one unique piece of text replaced."""
    text = source.read_text(encoding="utf-8")
    assert text.count(old) == 1, old
    path = tmp_path / source.name
    path.write_bytes(text.replace(old, new).encode("utf-8"))
    return path


def refusals(read, path: Path, check_envelope) -> list[tuple[str, str, dict]]:
    """(reason, ref, other values) of each error. The errors also make one valid
    envelope: codes, values and the exit status agree with contracts/codes.json."""
    with pytest.raises(Failure) as caught:
        read(path)
    errors = caught.value.errors
    stdout = io.StringIO()
    status = write(build(None, {}, errors), stdout, io.StringIO())
    check_envelope(stdout.getvalue(), status)
    assert status == 1
    assert {entry["code"] for entry in errors} == {"model_file_invalid"}
    assert {entry["detail"]["path"] for entry in errors} == {str(path)}
    return [
        (
            entry["detail"]["reason"],
            entry["ref"],
            {key: value for key, value in entry["detail"].items() if key not in ("reason", "path", "format")},
        )
        for entry in errors
    ]


def test_the_worked_example_reads():
    model = read_model(MODEL / "model.json")
    assert model["files"] == {"rules": ["rules/a.json"], "data": ["data/costs.json"]}

    rules = [read_rules(MODEL / name) for name in model["files"]["rules"]]
    assert [entity["id"] for entity in rules[0]["entities"]] == [
        "res.a",
        "fac.s",
        "act.collect",
        "act.run",
        "act.daily",
    ]
    assert [rule["id"] for rule in rules[0]["rules"]] == [
        "cap.a.hard",
        "cap.a.soft",
        "cap.s.storage",
        "regen.s",
        "flow.collect",
        "flow.run",
        "flow.daily",
        "limit.daily",
        "margin.a",
        "redline.a",
        "reserve.a",
        "floor.daily",
        "loss.collect_at_redline",
    ]
    assert [unknown["id"] for unknown in rules[0]["unknowns"]] == ["unk.collect"]

    data = [read_data(MODEL / name) for name in model["files"]["data"]]
    assert [(row["activity"], row["a"]) for row in data[0]["rows"]] == [("act.run", "70"), ("act.daily", "15")]

    assert [source["id"] for source in read_sources(MODEL / "sources.json")["sources"]] == ["src.w1", "src.d2"]

    readings = read_state(STATE)["readings"]
    assert {key: reading["value"] for key, reading in readings.items()} == {
        "resource.a": 3900,
        "resource.a_capacity": 3470,
        "resource.s_stock": 800,
    }

    prefs = read_prefs_defaults(PREFS)
    assert prefs["kinds"] == {"spend.real_money": "safety", "plan.order": "strategy", "plan.targets": "strategy"}
    assert prefs["neutral"] == {"plan.order": "optimum", "plan.targets": []}


def test_a_repeated_key_is_refused_at_the_second_key(tmp_path, check_envelope):
    head = '{"id": "cap.a.hard", "kind": "cap", "subject": "res.a",'
    path = variant(tmp_path, RULES, head, head + ' "subject": "res.b",')

    assert refusals(read_rules, path, check_envelope) == [
        ("key_duplicate", "/rules/0/subject", {"key": "subject"}),
    ]


@pytest.mark.parametrize(
    ("literal", "reason"),
    [
        ("3900.5", "float_literal"),
        ("39e2", "float_literal"),
        ("NaN", "constant_invalid"),
        ("-Infinity", "constant_invalid"),
        ("9007199254740992", "integer_out_of_range"),
        ("-9007199254740992", "integer_out_of_range"),
    ],
)
def test_a_number_outside_the_safe_integers_is_refused_with_its_pointer(tmp_path, check_envelope, literal, reason):
    path = variant(tmp_path, STATE, '"value": 3900,', f'"value": {literal},')

    assert refusals(read_state, path, check_envelope) == [
        (reason, "/readings/resource.a/value", {"literal": literal}),
    ]


def test_a_bom_is_refused_at_the_root_pointer(tmp_path, check_envelope):
    path = tmp_path / "acct-1.json"
    path.write_bytes(b"\xef\xbb\xbf" + STATE.read_bytes())

    assert refusals(read_state, path, check_envelope) == [("bom_present", "", {})]


@pytest.mark.parametrize(
    ("read", "source", "old", "new", "ref"),
    [
        (read_rules, RULES, '"value": "5000",', '"value": "5000", "overflow": "x",', "/rules/0/logic/overflow"),
        (read_prefs_defaults, PREFS, '"schema_version": 1,', '"schema_version": 1, "overflow": "x",', "/overflow"),
    ],
)
def test_an_unknown_member_is_refused(tmp_path, check_envelope, read, source, old, new, ref):
    path = variant(tmp_path, source, old, new)

    assert refusals(read, path, check_envelope) == [("member_unknown", ref, {"key": "overflow"})]


@pytest.mark.parametrize(
    ("old", "new", "ref", "literal"),
    [
        ('{"id": "cap.a.hard",', '{"id": "Cap.A.hard",', "/rules/0/id", "Cap.A.hard"),
        ('{"id": "unk.collect",', '{"id": "collect",', "/unknowns/0/id", "collect"),
    ],
)
def test_an_id_outside_the_grammar_is_refused(tmp_path, check_envelope, old, new, ref, literal):
    path = variant(tmp_path, RULES, old, new)

    assert refusals(read_rules, path, check_envelope) == [("id_invalid", ref, {"literal": literal})]
