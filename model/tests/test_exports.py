"""PY-14: the exports (#424 design 6.1, 6.2, 6.4, 6.6; design-py 6.1, 6.2) on a neutral fixture (`fixtures/tables`):
the JCS writer; `export tables` with a fake validator script (`fakes/validator.py`, never a real binary): its
refusal with the previous bytes back, no validator at all, provisional cells, blocked cells and the rounding
modes; the scoring template fill; and provenance files that equal `explain`."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli
from acmodel.core.run import evaluate_model
from acmodel.envelope import Failure
from acmodel.export import jcs
from acmodel.export.policy import Policy
from acmodel.export.scoring import fill

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "tables"
FAKE_VALIDATOR = Path(__file__).resolve().parent / "fakes" / "validator.py"
MODEL = "ours/model"
EXPORTS = "ours/model/exports.json"
TEMPLATE = "ours/model/scoring.template.json"
STATE = "state/acct-1.json"
REFUSAL = ["table_value_out_of_range", "/rows/1/2"]
ACCEPTANCE = {"output": "rule:v.est", "accept_provisional": {"ref": "R-9", "until": "check list item 2"}}
ROUNDED = [["half", 2, 3, 2, 2500], ["odd", 3, 4, 4, 3500], ["neg", -3, -2, -2, -2500], ["ratio", 0, 1, 1, 666]]


@pytest.fixture
def folder(tmp_path: Path) -> Path:
    """A copy of the fixture: the repository root, with the model under ours/model and the account state."""
    target = tmp_path / "repo"
    shutil.copytree(FIXTURE, target)
    return target


@pytest.fixture
def acmodel(make_system, app_dir, write_config, check_envelope, tmp_path):
    """One command in the venv: (exit status, envelope). With `validator`, `paths.actinglab` names the fake
    validator script, which runs as a process of its own with the instructions `refuse`; `log` lists its calls."""
    log = tmp_path / "validator.log"

    def call(*argv: str, validator: bool = True, refuse: dict[str, list[str]] | None = None) -> tuple[int, dict]:
        if validator:
            write_config({"schema_version": "acmodel.config.v1", "paths": {"actinglab": str(FAKE_VALIDATOR)}})
        control = json.dumps({"log": str(log), "refuse": refuse or {}})

        def run_captured(cmd: list[str], *, timeout_ms: int) -> tuple[int, bytes]:
            assert cmd[0] == str(FAKE_VALIDATOR)
            done = subprocess.run(
                [sys.executable, "-I", "-B", *cmd], stdout=subprocess.PIPE, timeout=timeout_ms / 1000, check=False,
                env={**os.environ, "ACMODEL_FAKE_VALIDATOR": control},
            )
            return done.returncode, done.stdout

        system = make_system(prefix=app_dir / "venv")
        system.run_captured = run_captured
        status = cli.main(list(argv), system)
        return status, check_envelope(system.stdout.getvalue(), status)

    def calls() -> list[dict[str, Any]]:
        if not log.exists():
            return []
        return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]

    call.calls = calls
    return call


def export_tables(acmodel, folder: Path, **options: Any) -> tuple[int, dict]:
    return acmodel("export", "tables", str(folder / MODEL), "--state", str(folder / STATE), "--repo", str(folder),
                   **options)


def edit(path: Path, change: Callable[[Any], None]) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


def tree(folder: Path) -> list[tuple[str, bytes | None]]:
    """Every file (with its bytes) and folder (None) under `folder`."""
    return sorted(
        (item.relative_to(folder).as_posix(), item.read_bytes() if item.is_file() else None)
        for item in folder.rglob("*")
    )


def table(folder: Path, table_id: str) -> dict[str, Any]:
    return json.loads((folder / "tables" / f"{table_id}.json").read_bytes())


def errors(doc: dict) -> list[tuple]:
    return [(entry["code"], entry.get("ref"), entry["detail"]) for entry in doc["errors"]]


def test_jcs_sorts_a_non_bmp_key_by_utf16_and_writes_no_unsafe_integer_and_no_float():
    # U+1F600 is D83D DE00 in UTF-16, below U+FB01; by code point it is above it.
    doc = {"ﬁ": 1, "\U0001f600": [True, None, -3, 2**53 - 1], "a": "q\"\\\n\u0001 "}
    assert jcs.dumps(doc) == (
        '{"a":"q\\"\\\\\\n\\u0001 ","\U0001f600":[true,null,-3,9007199254740991],"ﬁ":1}'.encode("utf-8")
    )

    for value, ref in ((2**53, ""), (-(2**53), ""), ({"k": [1, 2**60]}, "/k/1")):
        with pytest.raises(jcs.NotCanonical) as refused:
            jcs.dumps(value)
        assert (refused.value.reason, refused.value.ref) == ("integer_out_of_range", ref)
    for value, ref in ((1.0, ""), (0.5, ""), ({"k": 2.0}, "/k"), ([float("nan")], "/0")):
        with pytest.raises(jcs.NotCanonical) as refused:
            jcs.dumps(value)
        assert (refused.value.reason, refused.value.ref) == ("float", ref)


def test_a_validator_refusal_is_export_table_invalid_with_its_problem_and_pointer_and_the_previous_bytes_return(
    folder, acmodel
):
    previous = b'{"previous": "bytes"}\n'
    (folder / "tables").mkdir()
    (folder / "tables" / "unit_values.json").write_bytes(previous)
    before = tree(folder)

    status, doc = export_tables(acmodel, folder, refuse={"tables/unit_values.json": REFUSAL})

    assert (status, doc["data"]) == (5, {})
    message = "tables/unit_values.json: standard_pack_problem=table_value_out_of_range json_pointer=/rows/1/2"
    assert errors(doc) == [("export_table_invalid", "/rows/1/2", {
        "path": str(folder / "tables" / "unit_values.json"), "table_id": "unit_values",
        "problem": "table_value_out_of_range", "validator_code": "resource_declaration_invalid", "raw_text": message,
    })]
    # Both tables were validated while their new bytes were in place; then every file got its previous bytes
    # back: the refused table its old ones, and the other table and the provenance files, which were not
    # there, are gone again with the folders the export made.
    calls = acmodel.calls()
    assert [call["argv"] for call in calls] == [
        ["--json", "resource", "validate", "--repo", str(folder), "--changed-path", f"tables/{name}.json"]
        for name in ("unit_values", "rounding")
    ]
    assert calls[0]["bytes"].encode("utf-8") not in (previous, b"")
    assert tree(folder) == before


def test_without_a_validator_the_export_is_export_validator_missing_and_writes_nothing(folder, acmodel):
    before = tree(folder)

    status, doc = export_tables(acmodel, folder, validator=False)

    assert (status, doc["data"]) == (5, {})
    assert errors(doc) == [("export_validator_missing", None, {"key": "paths.actinglab", "reason": "not_configured"})]
    assert tree(folder) == before
    assert acmodel.calls() == []


def test_a_provisional_value_is_null_unless_accepted(folder, acmodel):
    status, doc = export_tables(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    columns = [
        {"name": "unit", "type": "key"},
        {"name": "tier", "type": "enum_string", "allowed": ["low", "high"]},
        {"name": "value_milli", "type": "integer", "min": -1000000000, "max": 1000000000, "nullable": True,
         "valid_until_unix_s": 1769904000},
        {"name": "usable", "type": "boolean"},
    ]
    assert (folder / "tables" / "unit_values.json").read_bytes() == jcs.dumps({
        "schema_version": "actingcommand.table.v1", "game": "example", "table_id": "unit_values",
        "row_key": ["unit"], "columns": columns, "rows": [["u1", "high", 1200000, True], ["u2", "low", None, True]],
    })
    assert doc["data"]["tables"][0] == {
        "table_id": "unit_values", "file": str(folder / "tables" / "unit_values.json"),
        "nulled": [{"row": 1, "column": "value_milli", "output": "rule:v.est"}], "accepted": [],
    }
    assert not (folder / MODEL / "out" / "provenance" / "rules" / "v.est.json").exists()

    edit(folder / EXPORTS, lambda exports: exports.update(acceptances=[ACCEPTANCE]))
    status, doc = export_tables(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert table(folder, "unit_values")["rows"] == [["u1", "high", 1200000, True], ["u2", "low", 240000, True]]
    assert (doc["data"]["tables"][0]["nulled"], doc["data"]["tables"][0]["accepted"]) == ([], ["rule:v.est"])
    provenance = json.loads((folder / MODEL / "out" / "provenance" / "rules" / "v.est.json").read_text("utf-8"))
    assert provenance["result"]["status"] == "provisional"
    accepted = provenance["acceptance"]
    assert re.fullmatch("[0-9a-f]{64}", accepted.pop("sha256"))
    assert accepted == {
        "file": "exports.json", "ref": "/acceptances/0", "accept_provisional": ACCEPTANCE["accept_provisional"],
    }


def test_a_blocked_value_is_export_refused_and_nothing_is_written(folder, acmodel):
    def block(exports: dict[str, Any]) -> None:
        exports["tables"][0]["rows"][0]["value_milli"] = {"$model": "rule:v.missing"}

    edit(folder / EXPORTS, block)
    before = tree(folder)

    status, doc = export_tables(acmodel, folder)

    assert (status, doc["data"]) == (5, {})
    assert [entry["code"] for entry in doc["errors"]] == ["export_refused", "state_reading_missing"]
    assert errors(doc)[0] == ("export_refused", "/tables/0/rows/0/value_milli", {
        "path": str(folder / EXPORTS), "output": "rule:v.missing", "reason": "blocked",
    })
    assert (doc["errors"][1]["detail"]["fact"], doc["errors"][1]["needed_by"]) == ("resource.b", ["rule:v.missing"])
    assert tree(folder) == before
    assert acmodel.calls() == []


def test_the_rounding_modes(folder, acmodel):
    """2.5, 3.5, -2.5 and 2/3 at scale 1 by floor, ceil and half_even, and at scale 1000 by floor."""
    status, doc = export_tables(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert table(folder, "rounding")["rows"] == ROUNDED


def test_a_scoring_template_resolves(folder, acmodel, tmp_path):
    run = evaluate_model(folder / MODEL, folder / STATE, datasets=tmp_path / "datasets", max_bits=127)

    filled = fill(run, Policy(run, refuse_provisional=True), 0)

    assert filled == {
        "schema_version": "example.selection.v1",
        "terms": [{"id": "base", "weight_milli": 1200000}, {"id": "share", "weight_milli": 666}],
        "gates": [{"fact": "resource.a", "at_least": 3}],
        "on_unknown": "skip",
        "tie_break": ["id"],
    }
    # Where scoring.json lives and which validator reads it are not decided (367-D1): nothing is written.
    before = tree(folder)
    status, doc = acmodel("export", "scoring", str(folder / MODEL), "--state", str(folder / STATE))
    assert (status, errors(doc)) == (5, [("export_refused", "/scoring/0", {
        "path": str(folder / EXPORTS), "output": "scoring:pick", "reason": "validator_path_unknown",
    })])
    assert tree(folder) == before


def test_one_provisional_placeholder_refuses_the_whole_scoring_document(folder, acmodel, tmp_path):
    edit(folder / TEMPLATE, lambda template: template["terms"][1]["weight_milli"].update({"$model": "rule:v.est"}))
    refusal = ("export_refused", "/terms/1/weight_milli", {
        "path": str(folder / TEMPLATE), "output": "rule:v.est", "reason": "provisional",
    })
    run = evaluate_model(folder / MODEL, folder / STATE, datasets=tmp_path / "datasets", max_bits=127)

    with pytest.raises(Failure) as refused:
        fill(run, Policy(run, refuse_provisional=True), 0)

    assert [(entry["code"], entry.get("ref"), entry["detail"]) for entry in refused.value.errors] == [refusal]
    before = tree(folder)
    status, doc = acmodel("export", "scoring", str(folder / MODEL), "--state", str(folder / STATE))
    assert (status, errors(doc)) == (5, [refusal])
    assert tree(folder) == before


def test_provenance_files_equal_explain(folder, acmodel):
    edit(folder / EXPORTS, lambda exports: exports.update(acceptances=[ACCEPTANCE]))
    status, doc = export_tables(acmodel, folder)
    assert (status, doc["errors"]) == (0, [])
    files = sorted((folder / MODEL / "out" / "provenance" / "rules").glob("*.json"))
    assert [path.stem for path in files] == ["v.est", "v.flag", "v.half", "v.neg", "v.odd", "v.ratio", "v.whole"]

    for path in files:
        status, shown = acmodel("explain", str(folder / MODEL), "rule:" + path.stem, "--state", str(folder / STATE))

        assert (status, shown["errors"]) == (0, [])
        assert shown["data"]["provenance"] == json.loads(path.read_text(encoding="utf-8"))
