"""PY-6: `bindings check` and `bindings apply` on the worked example's bindings (design sections 3.8, 6.3).

The example binds `thr.run.trigger` (offset -250) to the fact reference of rule `run.trigger` in
`schedule.json`, and `thr.daily.floor` (215) to the typed value of rule `daily.run` and to the pack policy file;
the literal 1 of rule `x.single` is fixed, with its reason.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
MODEL = "ours/model"
STATE = "state/acct-1.json"
SCHEDULE = "schedule.json"
POLICY = "packs/p.daily/resources/operations/p.daily/policies/floor.json"
TRIGGER = "/rules/0/trigger/predicates/0"  # the fact predicate of rule run.trigger in schedule.json
SINGLE = "/trigger/predicates/1/value/value"  # the fixed literal of rule x.single, relative to the rule
FIXED_REASON = "'at least one attempt left' is the predicate's meaning, not a quantity"


@pytest.fixture
def example(tmp_path: Path) -> Path:
    """A copy of the worked example: the model, its bindings, schedule.json and the pack policy file."""
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


def bindings(acmodel, folder: Path, command: str, *flags: str) -> tuple[int, dict]:
    return acmodel("bindings", command, str(folder / MODEL), "--state", str(folder / STATE), *flags)


def edit(path: Path, change: Callable[[Any], None]) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


def errors(doc: dict) -> list[tuple]:
    return [(entry["code"], entry.get("ref"), entry["detail"]) for entry in doc["errors"]]


def test_both_schedule_literals_and_the_policy_literal_equal_their_thresholds(example, acmodel):
    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["strict"] is False
    assert doc["data"]["bindings"] == [
        {"entry": "/bindings/0", "threshold": "thr.run.trigger", "form": "fact_offset", "file": SCHEDULE,
         "rule_id": "run.trigger", "path": "/trigger/predicates/0/value/offset", "literal": -250},
        {"entry": "/bindings/1", "threshold": "thr.daily.floor", "form": "constant", "file": SCHEDULE,
         "rule_id": "daily.run", "path": "/trigger/predicates/0/value/value", "literal": 215},
        {"entry": "/bindings/2", "threshold": "thr.daily.floor", "form": "constant", "file": POLICY,
         "path": "/threshold", "literal": 215},
    ]


def test_a_mismatch_names_the_pointer_and_both_values(example, acmodel):
    def offset(doc):
        doc["rules"][0]["trigger"]["predicates"][0]["value"]["offset"] = -200

    def floor(doc):
        doc["threshold"] = 210

    edit(example / SCHEDULE, offset)
    edit(example / POLICY, floor)

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["data"]) == (6, {})
    assert errors(doc) == [
        ("binding_mismatch", TRIGGER + "/value/offset", {
            "path": str(example / SCHEDULE), "entry": "/bindings/0", "reason": "value", "threshold": "thr.run.trigger",
            "rule_id": "run.trigger", "expected": "-250", "found": "-200"}),
        ("binding_mismatch", "/threshold", {
            "path": str(example / POLICY), "entry": "/bindings/2", "reason": "value", "threshold": "thr.daily.floor",
            "expected": "215", "found": "210"}),
    ]


@pytest.mark.parametrize(
    ("holder", "reason", "expected"),
    [("", "fact_key", "resource.a"), ("/value", "reference_fact", "resource.a_capacity")],
    ids=["predicate_fact_key", "reference_fact_key"],
)
def test_a_wrong_fact_key_is_a_mismatch(example, acmodel, holder, reason, expected):
    def wrong(doc):
        predicate = doc["rules"][0]["trigger"]["predicates"][0]
        (predicate["value"] if holder else predicate)["fact_key"] = "resource.b"

    edit(example / SCHEDULE, wrong)

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert errors(doc) == [("binding_mismatch", TRIGGER + holder + "/fact_key", {
        "path": str(example / SCHEDULE), "entry": "/bindings/0", "reason": reason, "threshold": "thr.run.trigger",
        "rule_id": "run.trigger", "expected": expected, "found": "resource.b"})]


def test_an_unbound_literal_is_refused_under_strict(example, acmodel):
    edit(example / MODEL / "bindings.json", lambda doc: doc["fixed"].clear())

    assert bindings(acmodel, example, "check")[0] == 0
    status, doc = bindings(acmodel, example, "check", "--strict")

    assert status == 6
    assert errors(doc) == [("binding_unbound_literal", "/rules/2" + SINGLE, {
        "path": str(example / SCHEDULE), "rule_id": "x.single", "pointer": SINGLE, "literal": "1"})]


def test_a_fixed_literal_passes_with_its_reason(example, acmodel):
    status, doc = bindings(acmodel, example, "check", "--strict")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["strict"], doc["data"]["literals"]) == (True, 3)
    assert doc["data"]["fixed"] == [{
        "entry": "/fixed/0", "file": SCHEDULE, "rule_id": "x.single", "path": SINGLE, "literal": 1,
        "reason": FIXED_REASON, "ref": "#425 A item 10"}]


def test_a_live_part_that_is_not_a_fact_plus_a_constant_is_unsupported(example, acmodel):
    """resource.a >= resource.a_capacity - resource.s_stock: the live offset reads a second fact."""
    model = example / MODEL
    derive = {
        "id": "redline.b", "kind": "derive", "subject": "res.a",
        "logic": {"value": "read:resource.a_capacity - read:resource.s_stock",
                  "live": {"fact": "resource.a", "comparison": "greater_than_or_equal",
                           "reference_fact": "resource.a_capacity", "offset": "0 - read:resource.s_stock"}},
        "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed",
    }
    rule = {"id": "mixed.trigger", "package_id": "example.mixed", "trigger": {"kind": "all", "predicates": [
        {"kind": "fact", "fact_key": "resource.a", "comparison": "greater_than_or_equal",
         "value": {"type": "fact", "fact_key": "resource.a_capacity", "offset": -800}, "max_age_ms": 3600000}]}}
    binding = {"threshold": "thr.run.mixed", "file": SCHEDULE, "rule_id": "mixed.trigger",
               "path": "/trigger/predicates/0/value/offset"}
    edit(model / "rules" / "a.json", lambda doc: doc["rules"].append(derive))
    edit(model / "exports.json",
         lambda doc: doc["thresholds"].append({"id": "thr.run.mixed", "from": "redline.b", "form": "fact_offset"}))
    edit(example / SCHEDULE, lambda doc: doc["rules"].append(rule))
    edit(model / "bindings.json", lambda doc: doc["bindings"].append(binding))

    status, doc = bindings(acmodel, example, "check")

    assert status == 5
    assert errors(doc) == [("binding_unsupported", "/thresholds/2", {
        "path": str(model / "exports.json"), "output": "threshold:thr.run.mixed",
        "reads": ["reading:resource.s_stock"]})]


def crlf(lines: list[str], **values: int) -> bytes:
    text = "\r\n".join(lines) + "\r\n"
    for name, value in values.items():
        text = text.replace(f"@{name}@", str(value))
    return text.encode("utf-8")


# A schedule.json in CRLF. Its priorities hold the same digits as the old literals, so only a replacement by
# position leaves them as they are.
SCHEDULE_LINES = [
    "{",
    '  "schema_version": "actingcommand.schedule.v1",',
    '  "requires": ["fact_reference"],',
    '  "rules": [',
    '    {"id": "run.trigger", "package_id": "example.run", "priority": 25,',
    '     "trigger": {"kind": "all", "predicates": [',
    '       {"kind": "fact", "fact_key": "resource.a", "comparison": "greater_than_or_equal",',
    '        "value": {"type": "fact", "fact_key": "resource.a_capacity", "offset": @RUN@},',
    '        "max_age_ms": 3600000}]}},',
    '    {"id": "daily.run", "package_id": "example.daily", "priority": 2150,',
    '     "trigger": {"kind": "all", "predicates": [',
    '       {"kind": "fact", "fact_key": "resource.a", "comparison": "greater_than_or_equal",',
    '        "value": {"type": "integer", "value": @DAILY@},',
    '        "max_age_ms": 3600000}]}},',
    '    {"id": "x.single", "package_id": "example.single",',
    '     "trigger": {"kind": "all", "predicates": [',
    '       {"kind": "timeline_active", "event_id": "x.window"},',
    '       {"kind": "fact", "fact_key": "resource.x_remaining", "comparison": "greater_than_or_equal",',
    '        "value": {"type": "integer", "value": 1}, "max_age_ms": 60000}]}}',
    "  ]",
    "}",
]
POLICY_LINES = ["{", '  "policy_id": "p.daily.floor",', '  "threshold": @FLOOR@', "}"]


def test_apply_on_a_crlf_file_changes_exactly_the_literal_bytes(example, acmodel):
    """The file is generated here, so no checkout changes its line ends. The new literals are longer or shorter
    than the old ones, and two of them are in one file."""
    (example / SCHEDULE).write_bytes(crlf(SCHEDULE_LINES, RUN=-25, DAILY=2150))
    (example / POLICY).write_bytes(crlf(POLICY_LINES, FLOOR=21))
    schedule, policy = crlf(SCHEDULE_LINES, RUN=-250, DAILY=215), crlf(POLICY_LINES, FLOOR=215)
    assert schedule.count(b"\n") == schedule.count(b"\r\n") and policy.count(b"\n") == policy.count(b"\r\n")

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert (example / SCHEDULE).read_bytes() == schedule
    assert (example / POLICY).read_bytes() == policy
    assert doc["data"]["changed"] == [
        {"entry": "/bindings/0", "threshold": "thr.run.trigger", "file": SCHEDULE, "rule_id": "run.trigger",
         "path": "/trigger/predicates/0/value/offset", "from": -25, "to": -250},
        {"entry": "/bindings/1", "threshold": "thr.daily.floor", "file": SCHEDULE, "rule_id": "daily.run",
         "path": "/trigger/predicates/0/value/value", "from": 2150, "to": 215},
        {"entry": "/bindings/2", "threshold": "thr.daily.floor", "file": POLICY, "path": "/threshold",
         "from": 21, "to": 215},
    ]
    assert doc["data"]["written"] == sorted([str(example / SCHEDULE), str(example / POLICY)])
    assert bindings(acmodel, example, "check", "--strict")[0] == 0
