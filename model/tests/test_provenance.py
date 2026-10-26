"""PY-5: provenance, `thresholds.json` and `explain` on the worked example (design sections 3.8, 6.3, 6.4),
and the refusal of a per-account value from every shared output (#424, ruling 66)."""

from __future__ import annotations

import json
import re
import shutil
from pathlib import Path

import pytest

from acmodel import __version__, cli

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
MODEL = "ours/model"
EXPORTS = "ours/model/exports.json"
STATE = "state/acct-1.json"
THRESHOLDS = ("thr.daily.floor", "thr.run.trigger")
Q7 = {"kind": "ruling", "ref": "Q-7", "where": "board #N comment C", "date": "2026-01-02"}
Q8 = {"kind": "ruling", "ref": "Q-8", "where": "board #N comment D", "date": "2026-01-02"}
D2 = {"kind": "source", "source": "src.d2", "locator": "row 12", "revision": "r100"}


@pytest.fixture
def example(tmp_path: Path) -> Path:
    """A copy of the worked example: the model, the preference defaults and the account state."""
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


@pytest.fixture
def acmodel(make_system, app_dir, check_envelope):
    """One command on an example folder, in the venv: (exit status, envelope)."""

    def call(*argv: str) -> tuple[int, dict]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main(list(argv), system)
        return status, check_envelope(system.stdout.getvalue(), status)

    return call


def export(acmodel, folder: Path) -> tuple[int, dict]:
    return acmodel("export", "thresholds", str(folder / MODEL), "--state", str(folder / STATE))


def read(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def provenance_file(folder: Path, threshold: str) -> Path:
    return folder / MODEL / "out" / "provenance" / "thresholds" / f"{threshold}.json"


def test_the_provenance_leaves_are_the_two_rulings_and_the_data_row(example, acmodel):
    status, doc = export(acmodel, example)

    assert (status, doc["errors"]) == (0, [])
    trees = {threshold: read(provenance_file(example, threshold))["nodes"] for threshold in THRESHOLDS}
    assert {name: node["reads"] for name, node in trees["thr.run.trigger"].items()} == {
        "threshold:thr.run.trigger": ["offset:redline.a"],
        "offset:redline.a": ["rule:margin.a"],
        "rule:margin.a": [],
    }
    assert {name: node["reads"] for name, node in trees["thr.daily.floor"].items()} == {
        "threshold:thr.daily.floor": ["rule:floor.daily"],
        "rule:floor.daily": ["flow:flow.daily:consumes:res.a", "rule:reserve.a"],
        "flow:flow.daily:consumes:res.a": ["row:costs[act.daily]"],
        "row:costs[act.daily]": [],
        "rule:reserve.a": [],
    }
    leaves = {
        threshold: {name: node["provenance"] for name, node in nodes.items() if not node["reads"]}
        for threshold, nodes in trees.items()
    }
    assert leaves == {
        "thr.run.trigger": {"rule:margin.a": [Q7]},
        "thr.daily.floor": {"row:costs[act.daily]": [D2], "rule:reserve.a": [Q8]},
    }
    row = trees["thr.daily.floor"]["row:costs[act.daily]"]
    assert (row["table"], row["key"], row["file"], row["ref"]) == ("costs", ["act.daily"], "data/costs.json", "/rows/1")


def test_thresholds_json_holds_both_thresholds_in_their_forms_with_status_and_provenance_file(example, acmodel):
    status, doc = export(acmodel, example)

    assert (status, doc["errors"]) == (0, [])
    out = read(example / MODEL / "out" / "thresholds.json")
    versions = out.pop("versions")
    assert out == {
        "schema_version": "acmodel.thresholds.v1",
        "model_id": "example",
        "thresholds": [
            {"id": "thr.daily.floor", "form": "constant", "value": 215, "dimension": "amount:res.a",
             "status": "confirmed", "provenance": "out/provenance/thresholds/thr.daily.floor.json"},
            {"id": "thr.run.trigger", "form": "fact_offset", "fact": "resource.a",
             "comparison": "greater_than_or_equal", "reference_fact": "resource.a_capacity", "offset": -250,
             "dimension": "amount:res.a", "status": "confirmed",
             "provenance": "out/provenance/thresholds/thr.run.trigger.json"},
        ],
    }
    assert set(versions) == {"rules", "data", "tool"} and versions["tool"] == __version__
    assert all(re.fullmatch("[0-9a-f]{64}", versions[name]) for name in ("rules", "data"))
    for entry in out["thresholds"]:
        provenance = read(example / MODEL / entry["provenance"])
        assert provenance["output"] == "threshold:" + entry["id"]
        assert (provenance["result"]["status"], provenance["versions"]) == ("confirmed", versions)
    assert doc["data"]["written"] == sorted(
        [str(example / MODEL / "out" / "thresholds.json")] + [str(provenance_file(example, t)) for t in THRESHOLDS]
    )


@pytest.mark.parametrize("threshold", THRESHOLDS)
def test_explain_prints_the_same_tree(example, acmodel, threshold):
    export(acmodel, example)

    status, doc = acmodel("explain", str(example / MODEL), "threshold:" + threshold, "--state", str(example / STATE))

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["output"] == "threshold:" + threshold
    assert doc["data"]["provenance"] == read(provenance_file(example, threshold))


def test_a_per_account_value_is_refused_and_nothing_is_written(example, acmodel):
    """A threshold of the red line itself (3470 - 250) reads the account's read cap: it is refused, and
    neither thresholds.json nor any provenance file is written, the other thresholds included."""
    path = example / EXPORTS
    text = path.read_text(encoding="utf-8")
    old = '"round": "ceil"}]'
    assert text.count(old) == 1
    line = '"round": "ceil"},\n   {"id": "thr.run.line", "from": "redline.a", "form": "constant"}]'
    path.write_bytes(text.replace(old, line).encode("utf-8"))
    before = sorted(item.relative_to(example) for item in example.rglob("*"))

    status, doc = export(acmodel, example)

    assert status == 5
    assert doc["data"] == {}
    assert [(entry["code"], entry["ref"], entry["detail"]) for entry in doc["errors"]] == [(
        "export_refused", "/thresholds/2",
        {"path": str(path), "output": "threshold:thr.run.line", "reason": "per_account"},
    )]
    assert sorted(item.relative_to(example) for item in example.rglob("*")) == before
