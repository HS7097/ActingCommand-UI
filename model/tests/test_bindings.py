"""PY-6: `bindings check` and `bindings apply` on the worked example's bindings (design sections 3.8, 6.3).

The example binds `thr.run.trigger` (offset -250) to the fact reference of rule `run.trigger` in
`schedule.json`, and `thr.daily.floor` (215) to the typed value of rule `daily.run` and to the pack policy file;
the literal 1 of rule `x.single` is fixed, with its reason.

P80 (#424 6457254776), the tests after the CRLF one: `--strict` also counts the operands of `resource_projection`
predicates, and a binding names a pool of `schedule.json` by `pool_id` and a pointer into the pool.

ACM-POOL (#424), the last test: `--strict` also scans the quantities of the pools of `schedule.json`.
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


# A rule whose trigger tests a pool: a written amount (215), and the read capacity less a margin (#386 CF-5).
PROJECTION_RULE = {"id": "pool.run", "package_id": "example.pool", "trigger": {"kind": "all", "predicates": [
    {"kind": "resource_projection", "pool_id": "pool.a", "comparison": "greater_than_or_equal", "value": 215},
    {"kind": "resource_projection", "pool_id": "pool.a", "comparison": "less_than",
     "value": {"type": "fact", "fact_key": "resource.a_capacity", "offset": -250}}]}}
PROJECTION = "/rules/3"  # PROJECTION_RULE, appended to the example's three rules
AMOUNT, MARGIN = "/trigger/predicates/0/value", "/trigger/predicates/1/value/offset"  # relative to the rule
# The margin as a constant threshold: 0 - rule:margin.a = -250.
MARGIN_RULE = {"id": "margin.pool", "kind": "derive", "subject": "res.a", "logic": {"value": "0 - rule:margin.a"},
               "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed"}
# A pool of schedule.json that regains 20 every 30 minutes, where the model says every hour.
POOL = {"id": "pool.s", "capacity": 800, "projection": {"amount": 20, "per_ms": 1800000},
        "observation": {"kind": "fact", "fact_key": "resource.s_stock"},
        "value_source": {"kind": "ledger_fact", "minimum_confidence_milli": 1}, "group_delay": None}
PERIOD_RULE = {"id": "period.s", "kind": "derive", "subject": "fac.s", "logic": {"value": "3600000"},
               "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed"}


def add_threshold(model: Path, rule: dict, threshold: str) -> None:
    """A derive rule of the example model and a constant threshold of it."""
    edit(model / "rules" / "a.json", lambda doc: doc["rules"].append(rule))
    edit(model / "exports.json",
         lambda doc: doc["thresholds"].append({"id": threshold, "from": rule["id"], "form": "constant"}))


def test_strict_also_counts_the_operands_of_a_resource_projection(example, acmodel):
    edit(example / SCHEDULE, lambda doc: doc["rules"].append(PROJECTION_RULE))

    assert bindings(acmodel, example, "check")[0] == 0
    status, doc = bindings(acmodel, example, "check", "--strict")

    assert status == 6
    assert errors(doc) == [
        ("binding_unbound_literal", PROJECTION + pointer, {
            "path": str(example / SCHEDULE), "rule_id": "pool.run", "pointer": pointer, "literal": literal})
        for pointer, literal in ((AMOUNT, "215"), (MARGIN, "-250"))
    ]


def test_the_operands_of_a_resource_projection_bind_to_constant_thresholds(example, acmodel):
    """The amount takes thr.daily.floor (215). The margin is the offset of a fact reference, but not in a `fact`
    predicate, so a constant threshold binds it."""
    model = example / MODEL
    add_threshold(model, MARGIN_RULE, "thr.pool.margin")
    edit(example / SCHEDULE, lambda doc: doc["rules"].append(PROJECTION_RULE))
    edit(model / "bindings.json", lambda doc: doc["bindings"].extend([
        {"threshold": "thr.daily.floor", "file": SCHEDULE, "rule_id": "pool.run", "path": AMOUNT},
        {"threshold": "thr.pool.margin", "file": SCHEDULE, "rule_id": "pool.run", "path": MARGIN}]))

    status, doc = bindings(acmodel, example, "check", "--strict")

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["literals"] == 5
    assert doc["data"]["bindings"][3:] == [
        {"entry": "/bindings/3", "threshold": "thr.daily.floor", "form": "constant", "file": SCHEDULE,
         "rule_id": "pool.run", "path": AMOUNT, "literal": 215},
        {"entry": "/bindings/4", "threshold": "thr.pool.margin", "form": "constant", "file": SCHEDULE,
         "rule_id": "pool.run", "path": MARGIN, "literal": -250},
    ]


def test_a_constant_at_the_offset_of_a_fact_predicate_is_still_the_wrong_form(example, acmodel):
    edit(example / MODEL / "bindings.json", lambda doc: doc["bindings"][0].update(threshold="thr.daily.floor"))

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert errors(doc) == [("binding_mismatch", TRIGGER + "/value/offset", {
        "path": str(example / SCHEDULE), "entry": "/bindings/0", "reason": "form", "threshold": "thr.daily.floor",
        "rule_id": "run.trigger"})]


def test_a_binding_names_a_pool_by_its_id_and_a_pointer_into_it(example, acmodel):
    model = example / MODEL
    add_threshold(model, PERIOD_RULE, "thr.pool.period")
    edit(example / SCHEDULE, lambda doc: doc.update(pools=[POOL]))
    edit(model / "bindings.json", lambda doc: doc["bindings"].append(
        {"threshold": "thr.pool.period", "file": SCHEDULE, "pool_id": "pool.s", "path": "/projection/per_ms"}))
    entry = {"entry": "/bindings/3", "threshold": "thr.pool.period", "file": SCHEDULE, "pool_id": "pool.s",
             "path": "/projection/per_ms"}

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert errors(doc) == [("binding_mismatch", "/pools/0/projection/per_ms", {
        "path": str(example / SCHEDULE), "entry": "/bindings/3", "reason": "value", "threshold": "thr.pool.period",
        "pool_id": "pool.s", "expected": "3600000", "found": "1800000"})]

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["changed"] == [{**entry, "from": 1800000, "to": 3600000}]
    assert json.loads((example / SCHEDULE).read_text(encoding="utf-8"))["pools"] == [
        {**POOL, "projection": {"amount": 20, "per_ms": 3600000}}]

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["bindings"][3] == {**entry, "form": "constant", "literal": 3600000}

    # ACM-POOL: --strict scans the pool's quantities now; the bound period passes, the other two do not.
    status, doc = bindings(acmodel, example, "check", "--strict")

    assert status == 6
    assert errors(doc) == [
        ("binding_unbound_literal", "/pools/0" + pointer, {
            "path": str(example / SCHEDULE), "pool_id": "pool.s", "pointer": pointer, "literal": literal})
        for pointer, literal in (("/capacity", "800"), ("/projection/amount", "20"))
    ]


def bindings_refusal(folder: Path, reason: str, ref: str, **values: object) -> tuple:
    detail = {"reason": reason, "path": str(folder / MODEL / "bindings.json"), "format": "acmodel.bindings.v1"}
    return "model_file_invalid", "/bindings/3/" + ref, {**detail, **values}


@pytest.mark.parametrize(
    ("binding", "status", "expected"),
    [
        ({"file": SCHEDULE, "pool_id": "pool.s", "path": "/capacity"}, 6, lambda folder: (
            "binding_mismatch", "/pools", {
                "path": str(folder / SCHEDULE), "entry": "/bindings/3", "reason": "literal_missing",
                "threshold": "thr.daily.floor", "pool_id": "pool.s"})),
        ({"file": SCHEDULE, "rule_id": "daily.run", "pool_id": "pool.s", "path": "/capacity"}, 1,
         lambda folder: bindings_refusal(folder, "member_conflict", "pool_id", key="pool_id",
                                         expected=["rule_id", "pool_id"])),
        ({"file": SCHEDULE, "path": "/capacity"}, 1,
         lambda folder: bindings_refusal(folder, "member_missing", "rule_id", key="rule_id",
                                         expected=["rule_id", "pool_id"])),
        ({"file": POLICY, "pool_id": "pool.s", "path": "/threshold"}, 1,
         lambda folder: bindings_refusal(folder, "member_unknown", "pool_id", key="pool_id")),
    ],
    ids=["no_pool_of_that_id", "rule_id_and_pool_id", "neither_in_schedule_json", "pool_id_in_a_pack_file"],
)
def test_a_schedule_json_binding_names_exactly_one_rule_or_pool(example, acmodel, binding, status, expected):
    edit(example / MODEL / "bindings.json",
         lambda doc: doc["bindings"].append({"threshold": "thr.daily.floor", **binding}))

    result, doc = bindings(acmodel, example, "check")

    assert result == status
    assert errors(doc) == [expected(example)]


# ACM-POOL: a pool whose capacity is the account's read cap (#386 CF-5) and which regains 1 every hour. Its
# minimum confidence is a setting of the Runtime, not a quantity, and is not scanned.
READ_POOL = {"id": "pool.r", "capacity": {"type": "fact", "fact_key": "resource.a_capacity", "offset": 0},
             "projection": {"amount": 1, "per_ms": 3600000},
             "observation": {"kind": "fact", "fact_key": "resource.a"},
             "value_source": {"kind": "ledger_fact", "minimum_confidence_milli": 1}, "group_delay": None}
STEP_RULE = {"id": "step.r", "kind": "derive", "subject": "res.a", "logic": {"value": "1"},
             "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed"}


def test_strict_scans_the_quantities_of_a_pool(example, acmodel):
    """Unbound, the capacity's offset, the amount and the period (projection/per_ms) are refused; bound to their
    thresholds, or fixed with a reason, they pass."""
    model = example / MODEL
    add_threshold(model, PERIOD_RULE, "thr.pool.period")
    add_threshold(model, STEP_RULE, "thr.pool.step")
    edit(example / SCHEDULE, lambda doc: doc.update(pools=[READ_POOL]))

    assert bindings(acmodel, example, "check")[0] == 0
    status, doc = bindings(acmodel, example, "check", "--strict")

    assert status == 6
    assert errors(doc) == [
        ("binding_unbound_literal", "/pools/0" + pointer, {
            "path": str(example / SCHEDULE), "pool_id": "pool.r", "pointer": pointer, "literal": literal})
        for pointer, literal in (("/capacity/offset", "0"), ("/projection/amount", "1"),
                                 ("/projection/per_ms", "3600000"))
    ]

    def bind(doc):
        doc["bindings"].extend([
            {"threshold": "thr.pool.step", "file": SCHEDULE, "pool_id": "pool.r", "path": "/projection/amount"},
            {"threshold": "thr.pool.period", "file": SCHEDULE, "pool_id": "pool.r", "path": "/projection/per_ms"}])
        doc["fixed"].append({"file": SCHEDULE, "pool_id": "pool.r", "path": "/capacity/offset",
                             "reason": "the read cap itself, without a margin", "ref": "fixture"})

    edit(model / "bindings.json", bind)

    status, doc = bindings(acmodel, example, "check", "--strict")

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["literals"] == 6  # three of the rules, three of the pool
    assert doc["data"]["bindings"][3:] == [
        {"entry": "/bindings/3", "threshold": "thr.pool.step", "form": "constant", "file": SCHEDULE,
         "pool_id": "pool.r", "path": "/projection/amount", "literal": 1},
        {"entry": "/bindings/4", "threshold": "thr.pool.period", "form": "constant", "file": SCHEDULE,
         "pool_id": "pool.r", "path": "/projection/per_ms", "literal": 3600000},
    ]
    assert doc["data"]["fixed"][1:] == [{
        "entry": "/fixed/1", "file": SCHEDULE, "pool_id": "pool.r", "path": "/capacity/offset", "literal": 0,
        "reason": "the read cap itself, without a margin", "ref": "fixture"}]


# #424 ACM-PRICE PRICE-3, design test 8: slot.s is a slot, written in slot-hours (two slots: 2 an hour under a hard
# cap of 0, formats.md "Slots"). It is no Runtime pool.
SLOT_RULES = [
    {"id": "regen.slot.s", "kind": "regen", "subject": "slot.s", "logic": {"amount": "2", "every": "1h"},
     "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed"},
    {"id": "cap.slot.s", "kind": "cap", "subject": "slot.s",
     "logic": {"cap_kind": "hard", "value": "0", "on_reach": "discard"},
     "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed"},
]


def test_a_slot_resource_cannot_be_bound_as_a_pool(example, acmodel):
    """A pool of schedule.json named after the slot, with its capacity bound to a threshold: `check` and `apply`
    both refuse the entry (`binding_mismatch`, `slot`, at the pool), and `apply` writes nothing."""

    def slot(doc):
        doc["entities"].append({"id": "slot.s", "kind": "resource"})
        doc["rules"].extend(SLOT_RULES)

    edit(example / MODEL / "rules" / "a.json", slot)
    edit(example / SCHEDULE, lambda doc: doc.update(pools=[{**POOL, "id": "slot.s"}]))
    edit(example / MODEL / "bindings.json", lambda doc: doc["bindings"].append(
        {"threshold": "thr.daily.floor", "file": SCHEDULE, "pool_id": "slot.s", "path": "/capacity"}))
    before = (example / SCHEDULE).read_bytes()
    expected = [("binding_mismatch", "/pools/0", {
        "path": str(example / SCHEDULE), "entry": "/bindings/3", "reason": "slot", "threshold": "thr.daily.floor",
        "pool_id": "slot.s"})]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, example, command)

        assert (status, doc["data"], errors(doc)) == (6, {}, expected)
    assert (example / SCHEDULE).read_bytes() == before
