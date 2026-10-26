"""PY-4: `acmodel run` and the derivation graph on the worked example (design section 3.8), and the three
checks that the static check leaves to the run, each refusing one change of the example with its code
and JSON pointer: dimensions, preference keys, and the entity ids of the state."""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest

from acmodel import cli

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
CODES = json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
RULES = "ours/model/rules/a.json"
COSTS = "ours/model/data/costs.json"
STATE = "state/acct-1.json"
THRESHOLDS = {
    "thr.daily.floor": {
        "form": "constant", "status": "confirmed", "type": "amount:res.a", "per_account": False, "value": "215",
    },
    "thr.run.trigger": {
        "form": "fact_offset", "fact": "resource.a", "comparison": "greater_than_or_equal",
        "reference_fact": "resource.a_capacity",
        "status": "confirmed", "type": "amount:res.a", "per_account": False, "offset": "-250",
    },
}


@pytest.fixture
def example(tmp_path: Path) -> Path:
    """A copy of the worked example: the model, the preference defaults and the account state."""
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


def edit(folder: Path, name: str, old: str, new: str) -> None:
    """Replace one unique piece of text in a file of the example."""
    path = folder / name
    text = path.read_text(encoding="utf-8")
    assert text.count(old) == 1, old
    path.write_bytes(text.replace(old, new).encode("utf-8"))


@pytest.fixture
def run(make_system, app_dir, check_envelope):
    """`acmodel run <model> --state <state>` on an example folder: (exit status, envelope)."""

    def run_example(folder: Path) -> tuple[int, dict]:
        system = make_system(prefix=app_dir / "venv")
        argv = ["run", str(folder / "ours" / "model"), "--state", str(folder / STATE)]
        status = cli.main(argv, system)
        doc = check_envelope(system.stdout.getvalue(), status)
        assert doc["command"] == "run"
        return status, doc

    return run_example


def test_the_worked_example_gives_the_offset_and_the_floor(example, run):
    status, doc = run(example)

    assert status == 0
    assert doc["errors"] == []
    assert doc["data"]["thresholds"] == THRESHOLDS
    # The per-account red line of section 3.8: 3470 - 250.
    assert doc["data"]["rules"]["redline.a"] == {
        "kind": "derive", "status": "confirmed", "type": "amount:res.a", "per_account": True, "value": "3220",
    }


def test_without_the_read_cap_the_red_line_is_blocked_and_the_thresholds_are_computed(example, run):
    edit(example, STATE, '"resource.a_capacity": {"value": 3470, "observed_at_unix_ms": 1767225600000, "source": "fixture"},', "")

    status, doc = run(example)

    assert status == 1
    assert doc["data"]["rules"]["redline.a"] == {"kind": "derive", "status": "blocked", "type": "amount:res.a"}
    assert {
        "code": "state_reading_missing",
        "detail": {"path": str(example / RULES), "fact": "resource.a_capacity", "position": 0},
        "ref": "/rules/9/logic/value",
        "needed_by": ["rule:redline.a"],
    } in doc["errors"]
    assert {entry["code"] for entry in doc["errors"]} == {"state_reading_missing"}
    assert doc["data"]["thresholds"] == THRESHOLDS


def test_a_provisional_input_gives_a_provisional_output(example, run):
    edit(example, COSTS, '"a": "15", "status": "confirmed"', '"a": "15", "status": "provisional"')

    status, doc = run(example)

    assert status == 0
    assert doc["data"]["rules"]["floor.daily"]["status"] == "provisional"
    assert doc["data"]["thresholds"] == {
        **THRESHOLDS, "thr.daily.floor": {**THRESHOLDS["thr.daily.floor"], "status": "provisional"},
    }


def test_the_ruling_that_overrides_the_general_logic_is_noted(example, run):
    status, doc = run(example)

    assert status == 0
    [note] = doc["data"]["notes"]
    assert note == {
        "code": "ruling_overrides_general_logic",
        "detail": {"path": str(example / RULES), "rule": "margin.a", "value": ["250"], "general_logic": ["800"]},
        "ref": "/rules/8/logic/general_logic",
    }
    spec = CODES[note["code"]]
    assert (spec["category"], spec["exit"]) == ("info", 0)
    assert set(note["detail"]) == set(spec["values"])


@pytest.mark.parametrize("stock", [500, 900])
def test_the_general_logic_does_not_change_with_the_account_state(example, run, stock):
    """`max_single_income` reads the rules and the data only, never a reading: another stock reading gives
    the same general logic (the largest collect is a full store, its storage cap 800) and the same note."""
    edit(example, STATE, '"resource.s_stock": {"value": 800,', f'"resource.s_stock": {{"value": {stock},')

    status, doc = run(example)

    assert status == 0
    general = doc["data"]["rules"]["margin.a"]["general_logic"]
    assert (general["value"], general["per_account"]) == ("800", False)
    assert doc["data"]["notes"] == [{
        "code": "ruling_overrides_general_logic",
        "detail": {"path": str(example / RULES), "rule": "margin.a", "value": ["250"], "general_logic": ["800"]},
        "ref": "/rules/8/logic/general_logic",
    }]


# The checks the static check leaves to the run: (edits of the example, then the one error: code, file,
# ref, other values). Each refuses before anything is computed (data is empty), or blocks what needs it.
GATE = (
    '"rules": [\n'
    '    {"id": "gate.run", "kind": "gate", "subject": "act.run",\n'
    '     "logic": {"irreversible": false, "origin": "any", "requires": "pref:plan.limit > 0"},\n'
    '     "provenance": [{"kind": "logic", "ref": "#425 6"}], "status": "confirmed"},'
)
UNIT_CLASS = '"entities": [\n    {"id": "unit", "kind": "entity_class", "attributes": {"level": {"type": "integer", "dimension": "count"}}},'
UNIT_LEVEL = (
    '"rules": [\n'
    '    {"id": "level.u9", "kind": "derive", "subject": "unit", "logic": {"value": "entity:unit[u9].level"},\n'
    '     "provenance": [{"kind": "logic", "ref": "#425 7"}], "status": "confirmed"},'
)
RUN_CHECKS = [
    pytest.param(
        [(RULES, '"value": "read:resource.a_capacity - rule:margin.a"', '"value": "read:resource.a_capacity + 1h"')],
        ("expr_dimension_mismatch", RULES, "/rules/9/logic/value",
         {"operator": "+", "operands": ["amount:res.a", "time"], "position": 25}),
        id="dimension",
    ),
    pytest.param(
        [(RULES, '"rules": [', GATE)],
        ("rule_reference_missing", RULES, "/rules/0/logic/requires",
         {"target": "plan.limit", "expected": ["pref"], "position": 0}),
        id="pref_key_not_in_defaults",
    ),
    pytest.param(
        [
            (RULES, '"entities": [', UNIT_CLASS),
            (RULES, '"rules": [', UNIT_LEVEL),
            (STATE, '"entities": {}', '"entities": {"unit": {"u1": {"level": 2}}}'),
        ],
        ("state_entity_missing", RULES, "/rules/0/logic/value",
         {"missing": "entity", "entity_class": "unit", "entity": "u9", "position": 0}),
        id="state_entity_id",
    ),
]


@pytest.mark.parametrize(("edits", "expected"), RUN_CHECKS)
def test_each_run_check_refuses_with_its_code_and_pointer(example, run, edits, expected):
    for name, old, new in edits:
        edit(example, name, old, new)
    code, name, ref, values = expected

    status, doc = run(example)

    assert status == 1
    assert [(e["code"], e["ref"], e["detail"]) for e in doc["errors"]] == [
        (code, ref, {"path": str(example / name), **values})
    ]
    if code == "state_entity_missing":  # a state input blocks what needs it; the rest is computed
        assert doc["errors"][0]["needed_by"] == ["rule:level.u9"]
        assert doc["data"]["thresholds"] == THRESHOLDS
    else:
        assert doc["data"] == {}
