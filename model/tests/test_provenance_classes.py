"""#424 ACM-PROV: the classes of provenance in `acmodel check`.

A measurement (or a guide row that is a live reading) says its `role`: `supplement`, a neutral constant no public
source gives, with the sources it searched; or `behaviour`, which is never the provenance of a number. A source of
`sources.json` says its `class`. Every number of a confirmed rule or data row needs a source, a supplement that
searched, a ruling or a logic entry, else `rule_provenance_unsourced`. These are warnings by default and errors
under `check --strict-provenance`; `bindings check --strict` does not read them. `--report-supplements` lists the
supplemented values.

The worked example (`fixtures/example`) is written with the classes; `old_shape` takes them out again, as the
rule files of the resource repositories are today.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
CODES = json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
MODEL = "ours/model"
RULES = "ours/model/rules/a.json"
COSTS = "ours/model/data/costs.json"
SOURCES = "ours/model/sources.json"
STATE = "state/acct-1.json"

SOURCE = {"kind": "source", "source": "src.w1", "locator": "page P1 §2"}
SUPPLEMENT = {"kind": "measurement", "ref": "M-9", "date": "2026-01-04", "role": "supplement", "searched": ["src.w1"]}
BEHAVIOUR = {"kind": "measurement", "ref": "M-8", "date": "2026-01-04", "role": "behaviour"}
RULING = {"kind": "ruling", "ref": "Q-9", "where": "board #N comment E", "date": "2026-01-02"}
LOGIC = {"kind": "logic", "ref": "fixture"}
PENDING = {"kind": "pending_check", "ref": "device check list item 2"}


@pytest.fixture
def example(tmp_path: Path) -> Path:
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


def envelope(stdout: str, status: int) -> dict[str, Any]:
    """The one acmodel.cli.v1 line, whose errors and warnings agree with contracts/codes.json, and whose exit
    status is the first error's."""
    lines = stdout.splitlines()
    assert len(lines) == 1, stdout
    doc = json.loads(lines[0])
    assert doc["schema_version"] == "acmodel.cli.v1"
    assert doc["ok"] is (doc["errors"] == [])
    for entry in doc["errors"] + doc["warnings"]:
        values = CODES[entry["code"]]["values"]
        assert set(entry["detail"]) <= set(values), entry
        for key, spec in values.items():
            if spec["required"]:
                assert key in entry["detail"], (entry, key)
            if "members" in spec and key in entry["detail"]:
                assert entry["detail"][key] in spec["members"], (entry, key)
    assert status == (CODES[doc["errors"][0]["code"]]["exit"] if doc["errors"] else 0)
    return doc


@pytest.fixture
def acmodel(make_system, app_dir):
    """One command, in the venv: (exit status, envelope)."""

    def call(*argv: str) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main(list(argv), system)
        return status, envelope(system.stdout.getvalue(), status)

    return call


def check(acmodel, folder: Path, *flags: str) -> tuple[int, dict[str, Any]]:
    return acmodel("check", str(folder / MODEL), *flags)


def edit(path: Path, change: Callable[[Any], None]) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1, ensure_ascii=False), encoding="utf-8")


def provenance(folder: Path, index: int, items: list[dict[str, Any]]) -> None:
    """The provenance of rule `index` of the example's rule file."""
    edit(folder / RULES, lambda doc: doc["rules"][index].update(provenance=items))


def problems(entries: list[dict[str, Any]]) -> list[tuple]:
    return [(entry["code"], entry.get("ref"), entry["detail"]) for entry in entries]


def unsourced(folder: Path, name: str, ref: str, reason: str, entry: str, entry_id: str, **values: Any) -> tuple:
    detail = {"path": str(folder / name), "reason": reason, "entry": entry, "id": entry_id, **values}
    return "rule_provenance_unsourced", ref, detail


# Each way a number passes, and each role with its members: cap.a.hard holds the number 5000; cap.a.soft holds
# none, so a behaviour confirmation alone is enough for it.
@pytest.mark.parametrize(
    ("index", "items"),
    [
        (0, [SOURCE]),
        (0, [SUPPLEMENT]),
        (0, [{**SUPPLEMENT, "samples": 12, "successes": 3}]),
        (0, [RULING]),
        (0, [LOGIC]),
        (0, [{"kind": "guide", "ref": "guide 05 line 3", "source": "src.w1"}]),
        (0, [{"kind": "guide", "ref": "guide 05 line 4", "role": "supplement", "searched": ["src.w1", "src.d2"]}]),
        (0, [SOURCE, BEHAVIOUR]),
        (1, [BEHAVIOUR]),
    ],
    ids=["source", "supplement", "supplement_with_samples", "ruling", "logic", "guide_on_a_source",
         "guide_as_a_supplement", "behaviour_beside_a_source", "behaviour_without_a_number"],
)
def test_each_way_a_number_is_sourced_passes(example, acmodel, index, items):
    provenance(example, index, items)

    status, doc = check(acmodel, example, "--strict-provenance")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert "supplements" not in doc["data"]


def refused(folder: Path, reason: str, ref: str, **values: Any) -> tuple:
    detail = {"reason": reason, "path": str(folder / RULES), "format": "acmodel.rules.v1", **values}
    return "model_file_invalid", ref, detail


PAIR = ["samples", "successes"]


@pytest.mark.parametrize(
    ("item", "reason", "member", "values"),
    [
        ({key: value for key, value in SUPPLEMENT.items() if key != "searched"}, "member_missing", "searched",
         {"key": "searched"}),
        ({**SUPPLEMENT, "samples": 12}, "member_missing", "successes", {"key": "successes", "expected": PAIR}),
        ({**SUPPLEMENT, "samples": 3, "successes": 4}, "value_invalid", "successes", {"literal": "4"}),
        ({**BEHAVIOUR, "searched": ["src.w1"]}, "member_unknown", "searched", {"key": "searched"}),
        ({"kind": "measurement", "ref": "M-9", "date": "2026-01-04", "searched": ["src.w1"]}, "member_missing",
         "role", {"key": "role"}),
        ({**BEHAVIOUR, "role": "guess"}, "value_invalid", "role",
         {"expected": ["supplement", "behaviour"], "literal": "guess"}),
        ({"kind": "guide", "ref": "guide 05", "source": "src.w1", "role": "behaviour"}, "member_conflict", "role",
         {"key": "role", "expected": ["source", "role"]}),
    ],
    ids=["supplement_without_searched", "samples_without_successes", "successes_above_samples",
         "searched_beside_behaviour", "searched_without_a_role", "another_role", "guide_with_source_and_role"],
)
def test_a_role_without_its_members_is_refused(example, acmodel, item, reason, member, values):
    provenance(example, 0, [item])

    status, doc = check(acmodel, example)

    assert status == 1
    assert problems(doc["errors"]) == [refused(example, reason, f"/rules/0/provenance/0/{member}", **values)]


def test_a_supplemented_drop_rate_carries_its_samples(example, acmodel):
    drop = {"id": "drop.run", "kind": "random", "subject": "act.run", "logic": {"drop": {"rate": "0.25"}},
            "provenance": [SUPPLEMENT], "status": "confirmed"}
    edit(example / RULES, lambda doc: doc["rules"].append(drop))

    status, doc = check(acmodel, example)

    assert status == 1
    assert problems(doc["errors"]) == [
        refused(example, "member_missing", "/rules/13/provenance/0/samples", key="samples", expected=PAIR)]

    provenance(example, 13, [{**SUPPLEMENT, "samples": 12, "successes": 3}])

    status, doc = check(acmodel, example, "--strict-provenance")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])


@pytest.mark.parametrize("value", ["wiki", "drop_stats", "datamine", "official", "forum"])
def test_each_source_class_reads_and_another_is_refused(example, acmodel, value):
    edit(example / SOURCES, lambda doc: doc["sources"][0].update({"class": value}))

    status, doc = check(acmodel, example, "--strict-provenance")

    if value != "forum":
        assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
        return
    assert status == 1
    assert problems(doc["errors"]) == [("model_file_invalid", "/sources/0/class", {
        "reason": "value_invalid", "path": str(example / SOURCES), "format": "acmodel.sources.v1",
        "expected": ["wiki", "drop_stats", "datamine", "official"], "literal": "forum"})]


@pytest.mark.parametrize(
    ("name", "items", "ref", "entry", "entry_id", "found"),
    [
        (RULES, [BEHAVIOUR], "/rules/0/provenance", "rule", "cap.a.hard", ["measurement:behaviour"]),
        (RULES, [PENDING], "/rules/0/provenance", "rule", "cap.a.hard", ["pending_check"]),
        (RULES, [{**SUPPLEMENT, "searched": []}], "/rules/0/provenance", "rule", "cap.a.hard",
         ["measurement:supplement"]),
        (COSTS, [BEHAVIOUR], "/rows/0/provenance", "row", "act.run", ["measurement:behaviour"]),
    ],
    ids=["rule_behaviour_only", "rule_without_a_number_source", "supplement_that_searched_nothing",
         "row_behaviour_only"],
)
def test_a_number_without_a_number_source_is_unsourced(example, acmodel, name, items, ref, entry, entry_id, found):
    if name == RULES:
        provenance(example, 0, items)
    else:
        edit(example / COSTS, lambda doc: doc["rows"][0].update(provenance=items))

    status, doc = check(acmodel, example, "--strict-provenance")

    assert (status, doc["data"], doc["warnings"]) == (1, {}, [])
    assert problems(doc["errors"]) == [
        unsourced(example, name, ref, "number_unsourced", entry, entry_id, found=found)]


@pytest.mark.parametrize("underneath", [None, {"source": "src.w1"}, {"role": "supplement", "searched": ["src.d2"]}],
                         ids=["without", "on_a_source", "as_a_supplement"])
def test_a_guide_row_names_the_class_underneath_it(example, acmodel, underneath):
    provenance(example, 0, [{"kind": "guide", "ref": "guide 05 line 3", **(underneath or {})}])

    status, doc = check(acmodel, example)

    assert (status, doc["errors"]) == (0, [])
    if underneath is not None:
        assert doc["warnings"] == []
        return
    assert problems(doc["warnings"]) == [
        unsourced(example, RULES, "/rules/0/provenance/0", "guide_class_missing", "rule", "cap.a.hard"),
        unsourced(example, RULES, "/rules/0/provenance", "number_unsourced", "rule", "cap.a.hard", found=["guide"]),
    ]


def old_shape(folder: Path) -> None:
    """The example as it was written before the classes: no source has a class and no measurement a role."""
    edit(folder / SOURCES, lambda doc: [source.pop("class") for source in doc["sources"]])
    edit(folder / RULES, lambda doc: doc["rules"][1]["provenance"][1].pop("role"))
    edit(folder / COSTS, lambda doc: doc["rows"][0].update(provenance=[
        {"kind": "measurement", "ref": "M-1", "date": "2026-01-01"}]))


def test_the_classes_warn_by_default_and_refuse_under_strict_provenance(example, acmodel):
    old_shape(example)
    expected = [
        unsourced(example, SOURCES, "/sources/0", "class_missing", "source", "src.w1"),
        unsourced(example, SOURCES, "/sources/1", "class_missing", "source", "src.d2"),
        unsourced(example, RULES, "/rules/1/provenance/1", "role_missing", "rule", "cap.a.soft"),
        unsourced(example, COSTS, "/rows/0/provenance/0", "role_missing", "row", "act.run"),
        unsourced(example, COSTS, "/rows/0/provenance", "number_unsourced", "row", "act.run", found=["measurement"]),
    ]

    status, doc = check(acmodel, example)

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["model_id"] == "example"
    assert problems(doc["warnings"]) == expected

    status, doc = check(acmodel, example, "--strict-provenance")

    assert (status, doc["data"], doc["warnings"]) == (1, {}, [])
    assert problems(doc["errors"]) == expected

    # The bindings check's --strict does not read them: it loads the same model and passes.
    status, doc = acmodel("bindings", "check", str(example / MODEL), "--state", str(example / STATE), "--strict")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])


def test_report_supplements_lists_every_supplemented_value(example, acmodel):
    guide = {"kind": "guide", "ref": "guide 05 line 4", "role": "supplement", "searched": ["src.d2"]}
    provenance(example, 0, [SOURCE, guide])
    row = {"kind": "measurement", "ref": "M-1", "date": "2026-01-01", "role": "supplement", "searched": ["src.w1"]}

    status, doc = check(acmodel, example, "--report-supplements")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["supplements"] == [
        {"path": str(example / RULES), "ref": "/rules/0/provenance/1", "entry": "rule", "id": "cap.a.hard",
         "provenance": guide},
        {"path": str(example / COSTS), "ref": "/rows/0/provenance/0", "entry": "row", "id": "act.run",
         "provenance": row},
    ]
    assert "supplements" not in check(acmodel, example)[1]["data"]
