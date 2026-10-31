"""PY-3: the static check. The worked example (design section 3.8) checks clean, and each rule
refuses one change of it with its code and JSON pointer. The readers and the expression lexer
share one id grammar."""

from __future__ import annotations

import hashlib
import io
import json
import shutil
from pathlib import Path

import pytest

from acmodel import cli
from acmodel.check import check_model
from acmodel.envelope import Failure, build, write
from acmodel.expr import parse
from acmodel.model import readers
from acmodel.model.shape import Context

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example" / "ours" / "model"
RULES = "rules/a.json"
COSTS = "data/costs.json"


@pytest.fixture
def model(tmp_path: Path) -> Path:
    """A copy of the worked example's model folder."""
    target = tmp_path / "model"
    shutil.copytree(EXAMPLE, target)
    return target


def edit(folder: Path, name: str, old: str, new: str) -> None:
    """Replace one unique piece of text in a file of the model."""
    path = folder / name
    text = path.read_text(encoding="utf-8")
    assert text.count(old) == 1, old
    path.write_bytes(text.replace(old, new).encode("utf-8"))


def refusals(folder: Path, datasets: Path, check_envelope) -> list[tuple[str, str, str, dict]]:
    """(code, file, ref, other values) of each error. The errors also make one valid envelope:
    codes, values and the exit status agree with contracts/codes.json."""
    with pytest.raises(Failure) as caught:
        check_model(folder, datasets=datasets)
    errors = caught.value.errors
    stdout = io.StringIO()
    status = write(build("check", {}, errors), stdout, io.StringIO())
    check_envelope(stdout.getvalue(), status)
    assert status == 1
    return [
        (
            entry["code"],
            entry["detail"]["path"],
            entry["ref"],
            {key: value for key, value in entry["detail"].items() if key != "path"},
        )
        for entry in errors
    ]


def test_the_worked_example_checks_clean(make_system, app_dir, check_envelope):
    system = make_system(prefix=app_dir / "venv")

    status = cli.main(["check", str(EXAMPLE)], system)

    doc = check_envelope(system.stdout.getvalue(), status)
    assert status == 0
    assert doc["command"] == "check"
    assert doc["data"] == {
        "path": str(EXAMPLE / "model.json"),
        "model_id": "example",
        "entities": 7,  # with the numeraire, a value entity (#424 ACM-PRICE PRICE-2a)
        "rules": 13,
        "superseded": 0,
        "unknowns": 1,
        "tables": 1,
    }


# (edits of the example, then the one error: code, file, ref, values; a value "first_path" is a
# file of the model, given relative to its folder)
REFUSALS = [
    pytest.param(
        [(RULES, '"value": "read:resource.a_capacity - rule:margin.a"',
          '"value": "read:resource.a_capacity - rule:margin.b"')],
        ("rule_reference_missing", RULES, "/rules/9/logic/value",
         {"target": "margin.b", "expected": ["rule"], "position": 27}),
        id="rule_reference_missing",
    ),
    pytest.param(
        [(RULES, "data:costs[act.run].a", "data:costs[act.walk].a")],
        ("data_reference_missing", RULES, "/rules/5/logic/consumes/res.a",
         {"missing": "row", "table": "costs", "key": ["act.walk"], "position": 0}),
        id="data_reference_missing",
    ),
    pytest.param(
        [(RULES, "read:resource.a_capacity - amount:res.a", "read:resource.b_capacity - amount:res.a")],
        ("state_reference_missing", RULES, "/unknowns/0/variants/up_to_cap/amount",
         {"missing": "reading", "target": "resource.b_capacity", "position": 17}),
        id="state_reference_missing",
    ),
    pytest.param(
        [(RULES, '{"id": "loss.collect_at_redline",', '{"id": "margin.a",')],
        ("model_id_duplicate", RULES, "/rules/12/id",
         {"namespace": "model", "id": "margin.a", "first_path": RULES, "first_ref": "/rules/8/id"}),
        id="duplicate_id",
    ),
    pytest.param(
        [(RULES, '"provenance": [{"kind": "source", "source": "src.w1", "locator": "page P1 §2"}]',
          '"provenance": []')],
        ("rule_provenance_missing", RULES, "/rules/0/provenance", {"entry": "rule", "id": "cap.a.hard"}),
        id="empty_provenance",
    ),
    pytest.param(
        [(RULES, '"status": "pending"}', '"status": "verified"}')],
        ("rule_status_invalid", RULES, "/rules/4/status",
         {"id": "flow.collect", "literal": "verified", "expected": ["confirmed", "pending", "provisional"]}),
        id="bad_status",
    ),
    pytest.param(
        [
            (RULES, '{"id": "cap.s.storage", "kind": "cap", "subject": "fac.s",',
             '{"id": "cap.s.storage", "kind": "cap", "subject": "res.a",'),
            (RULES, '"cap_kind": "storage", "value": "800"', '"cap_kind": "hard", "value": "800"'),
        ],
        ("rule_conflict", RULES, "/rules/2",
         {"id": "cap.s.storage", "conflicts_with": "cap.a.hard", "kind": "cap", "subject": "res.a", "slot": "hard"}),
        id="rule_conflict",
    ),
    pytest.param(
        [(RULES, '"logic": {"value": "200"},', '"logic": {"value": "rule:floor.daily"},')],
        ("rule_cycle", RULES, "/rules/11/logic/value",
         {"members": ["rule:floor.daily", "rule:reserve.a"], "via": "reference"}),
        id="rule_cycle",
    ),
    pytest.param(
        [(RULES, '"provenance": [{"kind": "logic", "ref": "#425 一.5"}], "status": "confirmed"}',
          '"provenance": [{"kind": "logic", "ref": "#425 一.5"}], "status": "pending"}')],
        ("rule_pending_unbounded", RULES, "/rules/6/status", {"id": "flow.daily", "reason": "no_unknown"}),
        id="rule_pending_unbounded",
    ),
    pytest.param(
        [(RULES, '"logic": {"value": "200"},', '"logic": {"value": "pref:plan.reserve"},')],
        ("rule_pref_misplaced", RULES, "/rules/10/logic/value",
         {"id": "reserve.a", "kind": "derive", "reference": "pref:plan.reserve", "position": 0}),
        id="pref_outside_gate_or_objective",
    ),
]


@pytest.mark.parametrize(("edits", "expected"), REFUSALS)
def test_each_rule_refuses_with_its_code_and_pointer(model, tmp_path, check_envelope, edits, expected):
    for name, old, new in edits:
        edit(model, name, old, new)
    code, name, ref, values = expected
    values = {key: str(model / value) if key == "first_path" else value for key, value in values.items()}

    assert refusals(model, tmp_path / "datasets", check_envelope) == [(code, str(model / name), ref, values)]


@pytest.mark.parametrize("copy", [None, b"another version\n"], ids=["missing", "hash_mismatch"])
def test_an_external_dataset_is_verified_by_its_hash(model, tmp_path, check_envelope, copy):
    declared = hashlib.sha256(b"act.run,70\nact.daily,15\n").hexdigest()
    (model / COSTS).write_text(
        json.dumps({
            "schema_version": "acmodel.data.v1",
            "table": "costs",
            "key": "activity",
            "columns": {"a": {"type": "integer", "dimension": "amount:res.a"}},
            "external": {"kind": "external_dataset", "source": "src.d2", "sha256": declared, "fetched_at": "2026-01-01"},
        }),
        encoding="utf-8",
    )
    datasets = tmp_path / "datasets"
    datasets.mkdir()
    dataset = datasets / declared
    if copy is None:
        expected = ("data_external_missing", "/external",
                    {"sha256": declared, "source": "src.d2", "dataset": str(dataset)})
    else:
        dataset.write_bytes(copy)
        expected = ("data_external_hash_mismatch", "/external/sha256",
                    {"sha256": declared, "found": hashlib.sha256(copy).hexdigest(), "dataset": str(dataset)})

    code, ref, values = expected
    assert refusals(model, datasets, check_envelope) == [(code, str(model / COSTS), ref, values)]


@pytest.mark.parametrize(
    ("grammar", "kind", "text", "accepted"),
    [
        (readers.ID, "rule", "a" + "b" * 95, True),
        (readers.ID, "rule", "a" + "b" * 96, False),
        (readers.FACT_KEY, "read", "resource.a:cap-1", True),
        (readers.FACT_KEY, "read", "9" * 128, True),
        (readers.FACT_KEY, "read", "9" * 129, False),
    ],
    ids=["id_96", "id_97", "fact_key_with_colon", "fact_key_128", "fact_key_129"],
)
def test_the_readers_and_the_expression_lexer_share_the_id_grammars(grammar, kind, text, accepted):
    assert grammar.check(text, "/id", Context("rules.json", "acmodel.rules.v1")) is accepted

    if accepted:
        reference = parse(f"{kind}:{text}").root
        assert (reference.kind, reference.target) == (kind, text)
    else:
        with pytest.raises(Failure) as caught:
            parse(f"{kind}:{text}")
        [entry] = caught.value.errors
        assert (entry["code"], entry["detail"]["reason"]) == ("expr_parse_error", "reference_malformed")
