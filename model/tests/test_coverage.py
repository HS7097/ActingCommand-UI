"""#424 ACM-PRICE PRICE-2b-2: the coverage of the pools, the static components and the status of each price, the
declared zero (`no_use`) and the estimate of a provisional derive (addendum 2.1 §3.1–§3.3 and §3.7; the design's §4.6
and its test 16). Every fixture is synthetic, with neutral ids, and every expected value is worked out by hand in its
docstring. CI runs these tests on Windows and on Ubuntu."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli

CODES = json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
PROVENANCE = [{"kind": "logic", "ref": "fixture"}]
DAY = {"id": "day", "unit": "day", "reset_minute": 0, "utc_offset_minutes": 0}
NUMERAIRE = "value.economy"
RULES = ("ours", "model", "rules", "economy.json")


def write(path: Path, doc: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


def resource(entity_id: str) -> dict[str, Any]:
    return {"id": entity_id, "kind": "resource"}


def activity(entity_id: str) -> dict[str, Any]:
    return {"id": entity_id, "kind": "activity"}


def rule(rule_id: str, kind: str, subject: str, *, status: str = "confirmed", **logic: Any) -> dict[str, Any]:
    return {"id": rule_id, "kind": kind, "subject": subject, "logic": logic, "provenance": PROVENANCE, "status": status}


def flow(rule_id: str, subject: str, **logic: Any) -> dict[str, Any]:
    return rule(rule_id, "flow", subject, **logic)


def limit(rule_id: str, subject: str, maximum: str) -> dict[str, Any]:
    return rule(rule_id, "limit", subject, max=maximum, per="day")


def goal(rule_id: str, subject: str, value: str) -> dict[str, Any]:
    return rule(rule_id, "objective", subject, tier="main", value=value)


def no_use(rule_id: str, subject: str, facts: list[str] | None) -> dict[str, Any]:
    """A declared zero; with `facts`, a conditional one (its `holds_when` and the fact keys that check it)."""
    condition = {} if facts is None else {"holds_when": "fixture condition", "facts": facts}
    return rule(rule_id, "no_use", subject, reason="fixture: nothing takes it", **condition)


def derive(rule_id: str, subject: str, value: str, estimate: dict[str, Any] | None = None, **members: Any) -> dict:
    """A provisional derive, with its estimate when one is given."""
    logic = {"value": value, **({"estimate": estimate} if estimate is not None else {})}
    return {**rule(rule_id, "derive", subject, status="provisional"), "logic": logic, **members}


def model(folder: Path, entities: list, rules: list, unknowns: list | None = None) -> Path:
    """A neutral model folder with one rule file and an empty account state; returns the folder."""
    base = folder / "ours" / "model"
    write(base / "model.json", {
        "schema_version": "acmodel.model.v1", "model_id": "coverage", "files": {"rules": ["rules/economy.json"], "data": []},
        "periods": [DAY], "numeraire": NUMERAIRE, "engines": [],
    })
    write(base / "sources.json", {"schema_version": "acmodel.sources.v1", "sources": []})
    write(base / "exports.json", {"schema_version": "acmodel.exports.v1", "thresholds": [], "tables": [], "scoring": []})
    write(base / "rules" / "economy.json", {
        "schema_version": "acmodel.rules.v1", "entities": [*entities, {"id": NUMERAIRE, "kind": "value"}],
        "rules": rules, "unknowns": unknowns or [],
    })
    write(folder / "state.json", {
        "schema_version": "acmodel.state.v1", "model_id": "coverage", "label": "acct-1", "readings": {}, "entities": {},
        "cleared": [],
    })
    return folder


@pytest.fixture
def acmodel(make_system, app_dir):
    """`acmodel <command> <model> [--state <state>]` on a fixture folder: (exit status, envelope). The envelope is one
    acmodel.cli.v1 line whose errors and warnings agree with contracts/codes.json."""

    def run(command: str, folder: Path, *flags: str) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        state = [] if command == "check" else ["--state", str(folder / "state.json")]
        status = cli.main([command, str(folder / "ours" / "model"), *state, *flags], system)
        [line] = system.stdout.getvalue().splitlines()
        doc = json.loads(line)
        assert doc["command"] == command and doc["ok"] is (doc["errors"] == [])
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


def rules_path(folder: Path) -> str:
    return str(folder.joinpath(*RULES))


def without_dual_checks(doc: dict[str, Any]) -> list[dict[str, Any]]:
    """The warnings but HiGHS's marginals, a cross-check only (`lp_dual_disagrees`), which these tests are not about."""
    return [entry for entry in doc["warnings"] if entry["code"] != "lp_dual_disagrees"]


# §3.2: the declared zero and its two checks, the example of a part and a block in neutral ids ----------------------


def zero_fixture(folder: Path, block: list[str] | None, *, declared: bool = True, trade: bool = False) -> Path:
    """The furniture-part and carbon example of addendum 2.1 §3.2 in neutral ids. v is the main goal, worth 1; act.daily
    brings in 10 v once a day. act.mine brings in 2 block once a day, and act.break turns 1 block into 4 part. part has
    no other use: it is declared zero, under the condition `fact.rooms_full` (every room full). block's only use leads
    to part. With `declared`, block is declared zero too, with the fact keys `block` (None: unconditionally). With
    `trade`, act.trade turns 10 part into 1 v.

    Rules, in this order: flow.daily, limit.daily, flow.mine, limit.mine, flow.break, zero.part, [zero.block],
    [flow.trade], goal.v."""
    rules = [
        flow("flow.daily", "act.daily", produces={"res.v": "10"}), limit("limit.daily", "act.daily", "1"),
        flow("flow.mine", "act.mine", produces={"res.block": "2"}), limit("limit.mine", "act.mine", "1"),
        flow("flow.break", "act.break", consumes={"res.block": "1"}, produces={"res.part": "4"}),
        no_use("zero.part", "res.part", ["fact.rooms_full"]),
        *([no_use("zero.block", "res.block", block)] if declared else []),
        *([flow("flow.trade", "act.trade", consumes={"res.part": "10"}, produces={"res.v": "1"})] if trade else []),
        goal("goal.v", "res.v", "1"),
    ]
    entities = [resource("res.v"), resource("res.block"), resource("res.part"), activity("act.daily"),
                activity("act.mine"), activity("act.break"), *([activity("act.trade")] if trade else [])]
    return model(folder, entities, rules)


@pytest.mark.parametrize(
    ("block", "ref"),
    [
        pytest.param(["fact.base_max"], "/rules/6/logic/facts", id="own_condition_only"),
        pytest.param(None, "/rules/6/logic", id="unconditional"),
    ],
)
def test_a_zero_whose_use_leads_to_a_conditional_zero_must_carry_its_condition(tmp_path, acmodel, block, ref):
    """§3.2, `no_use_condition_incomplete`. act.break takes block and brings in part, which is zero only where every
    room is full (`fact.rooms_full`). So block is zero only there too: its declaration must hold that fact key. With
    only its own (`fact.base_max`), or with none at all, it is refused at its facts (or at its logic, which has none),
    and the missing key is named."""
    folder = zero_fixture(tmp_path, block)

    status, doc = acmodel("check", folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "no_use_condition_incomplete",
        "detail": {
            "path": rules_path(folder), "id": "zero.block", "pool": "res.block", "downstream": "res.part",
            "missing": ["fact.rooms_full"],
        },
        "ref": ref,
    }]


def test_a_zero_that_has_a_use_is_contradicted_and_a_use_toward_a_zero_is_none(tmp_path, acmodel):
    """§3.2, `no_use_contradicted`. With act.trade, part has a use: act.trade takes it and brings in v, the main goal,
    which is covered. part's declaration is refused (its ground: `use`, by act.trade). block's declaration stands: its
    only use, act.break, brings in part, a declared zero, and a use toward a zero is no use, never a contradiction. It
    holds part's fact key, so nothing else is refused."""
    folder = zero_fixture(tmp_path, ["fact.base_max", "fact.rooms_full"], trade=True)

    status, doc = acmodel("check", folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "no_use_contradicted",
        "detail": {
            "path": rules_path(folder), "id": "zero.part", "pool": "res.part", "grounds": ["use"],
            "activities": ["act.trade"],
        },
        "ref": "/rules/5",
    }]


@pytest.mark.parametrize("declared", [True, False], ids=["declared", "undeclared"])
def test_a_use_that_leads_only_to_zeros_covers_nothing(tmp_path, acmodel, declared):
    """§3.1 and §3.2, both checks passing. block's declaration holds its own fact key and part's, so the check passes
    and both are verified zero. Without block's declaration, block is not covered (its only use leads to part, a
    declared zero) and is not zero either: it is unknown (`not_covered`, `zero_uses`).

    By hand, the steady state: act.daily brings in 10 v a day, V = 10; block and part are worth nothing (0), v 1. The
    static components: act.break and act.mine with block and part; act.daily with v and its sink goal.v. No rule names
    an unknown and nothing is provisional, so both components are confirmed, with no gap."""
    folder = zero_fixture(tmp_path, ["fact.base_max", "fact.rooms_full"], declared=declared)

    status, doc = acmodel("economy", folder, "--steady-only")

    assert (status, doc["errors"], without_dual_checks(doc)) == (0, [], [])
    steady = doc["data"]["steady"]
    assert (steady["status"], steady["objective"]) == ("confirmed", "10")
    assert steady["prices"] == {"res.block": ["0"], "res.part": ["0"], "res.v": ["1"]}
    block = (
        {"class": "verified_zero", "status": "confirmed", "reasons": ["no_use"], "no_use": "zero.block"} if declared
        else {"class": "unknown", "status": "confirmed", "reasons": ["not_covered", "zero_uses"]}
    )
    assert steady["coverage"] == {
        "pools": {
            "res.block": {**block, "component": "act.break"},
            "res.part": {
                "class": "verified_zero", "status": "confirmed", "reasons": ["no_use"], "no_use": "zero.part",
                "component": "act.break",
            },
            "res.v": {"class": "covered", "status": "confirmed", "reasons": ["goal"], "component": "act.daily"},
        },
        "components": {
            "act.break": {
                "members": ["act.break", "act.mine", "res.block", "res.part"], "status": "confirmed", "gap": False,
                "gaps": {}, "reasons": [],
            },
            "act.daily": {
                "members": ["act.daily", "goal.v", "res.v"], "status": "confirmed", "gap": False, "gaps": {},
                "reasons": [],
            },
        },
        "counts": {
            "covered": 1, "structural_gap": 0, "verified_zero": 2 if declared else 1, "unknown": 0 if declared else 1,
            "with_estimates": 0,
        },
        "estimates": [],
    }
    assert steady["price_status"] == {r: {"status": "confirmed", "reasons": []} for r in ("res.block", "res.part", "res.v")}


# §3.3: structural gaps and independent components; the design's test 16 ----------------------------------------


def test_a_gap_component_is_pending_and_an_independent_one_keeps_its_class(tmp_path, acmodel):
    """§3.3. Three static components with no edge between them.
    - act.a and act.p take r (100 a day) and bring in v (the main goal, worth 1). act.a turns 10 r into 1 v, at most 3
      times a day; act.p's flow is pending (how much v it gives is unk.p, from 1 to 3), so the steady state leaves it
      out (`excluded_pending`). Its component is a structural gap: r and v.
    - act.lesson turns 1 book into 1 w (a main goal, worth 3), and nothing brings book in: book has no steady supply,
      and its component is a structural gap: book and w.
    - act.get brings in 5 g (a main goal, worth 2) once a day: independent of both gaps, so g stays covered and
      confirmed.

    By hand: act.a runs 3 times (3 v) and act.get once (10): V0 = 13, and A* = 13 (both are protected). r is worth 0
    (70 are left over), v 1, g 2; one more book gives one more w (3), and one less cannot be (no high end), and the same
    for w. Every price of the two gaps is provisional, for its gap; g's is confirmed; the steady state is provisional."""
    unknown = {
        "id": "unk.p", "kind": "interval", "question": "fixture", "interval": {"low": "1", "high": "3"},
        "status": "pending", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }
    pending = {
        **flow("flow.p", "act.p", consumes={"res.r": "10"}, produces={"res.v": "unknown:unk.p"}), "status": "pending",
        "unknowns": ["unk.p"],
    }
    folder = model(
        tmp_path,
        [resource("res.r"), resource("res.v"), resource("res.book"), resource("res.w"), resource("res.g"),
         activity("act.a"), activity("act.p"), activity("act.lesson"), activity("act.get")],
        [
            rule("regen.r", "regen", "res.r", amount="100", every="1d"),
            flow("flow.a", "act.a", consumes={"res.r": "10"}, produces={"res.v": "1"}), limit("limit.a", "act.a", "3"),
            pending, limit("limit.p", "act.p", "20"),
            flow("flow.lesson", "act.lesson", consumes={"res.book": "1"}, produces={"res.w": "1"}),
            flow("flow.get", "act.get", produces={"res.g": "5"}), limit("limit.get", "act.get", "1"),
            goal("goal.v", "res.v", "1"), goal("goal.w", "res.w", "3"), goal("goal.g", "res.g", "2"),
        ],
        [unknown],
    )

    status, doc = acmodel("economy", folder, "--steady-only")

    assert (status, doc["errors"], without_dual_checks(doc)) == (0, [], [])
    steady = doc["data"]["steady"]
    assert (steady["status"], steady["objective"], steady["acquisition"]) == ("provisional", "13", "13")
    assert steady["prices"] == {
        "res.book": [{"low": "3", "high": None}], "res.g": ["2"], "res.r": ["0"], "res.v": ["1"],
        "res.w": [{"low": "3", "high": None}],
    }
    assert steady["excluded_pending"] == [{"flow": "flow.p", "activity": "act.p", "unknowns": ["unk.p"]}]
    gap = {"status": "provisional"}
    assert steady["coverage"] == {
        "pools": {
            "res.book": {**gap, "class": "structural_gap", "reasons": ["no_steady_supply"], "component": "act.lesson"},
            "res.g": {"class": "covered", "status": "confirmed", "reasons": ["goal"], "component": "act.get"},
            "res.r": {**gap, "class": "structural_gap", "reasons": ["excluded_pending"], "component": "act.a"},
            "res.v": {**gap, "class": "structural_gap", "reasons": ["excluded_pending"], "component": "act.a"},
            "res.w": {**gap, "class": "structural_gap", "reasons": ["no_steady_supply"], "component": "act.lesson"},
        },
        "components": {
            "act.a": {
                "members": ["act.a", "act.p", "goal.v", "res.r", "res.v"], **gap, "gap": True,
                "gaps": {"excluded_pending": ["act.p"]}, "reasons": ["excluded_pending"],
            },
            "act.get": {
                "members": ["act.get", "goal.g", "res.g"], "status": "confirmed", "gap": False, "gaps": {}, "reasons": [],
            },
            "act.lesson": {
                "members": ["act.lesson", "goal.w", "res.book", "res.w"], **gap, "gap": True,
                "gaps": {"no_steady_supply": ["res.book"]}, "reasons": ["no_steady_supply"],
            },
        },
        "counts": {"covered": 1, "structural_gap": 4, "verified_zero": 0, "unknown": 0, "with_estimates": 0},
        "estimates": [],
    }
    assert steady["price_status"] == {
        "res.book": {"status": "provisional", "reasons": ["no_steady_supply"]},
        "res.g": {"status": "confirmed", "reasons": []},
        "res.r": {"status": "provisional", "reasons": ["excluded_pending"]},
        "res.v": {"status": "provisional", "reasons": ["excluded_pending"]},
        "res.w": {"status": "provisional", "reasons": ["no_steady_supply"]},
    }


ESTIMATE_B = {"low": "2", "high": "4", "basis": "statistics", "covers": {"activities": ["act.b"]}}


def test_a_provisional_component_leaves_a_disconnected_one_confirmed(tmp_path, acmodel):
    """The design's test 16, and §4.6's rule that an unknown a rule names makes its component provisional. Three
    components, no edge between them; each act.<x> brings in its x once a day, x a main goal worth 1:
    - act.a brings in 2 a, all confirmed;
    - act.b brings in b by `pt.b` (3), a provisional derive whose estimate is 2 to 4 (covering act.b);
    - act.c brings in 1 c by a confirmed flow that names the unknown unk.c in its `unknowns` (no amount reads it).

    By hand: V0 = A* = 2 + 3 + 1 = 6; a, b and c are worth 1 each. a's price is confirmed; b's is provisional (an input
    of its component is), and so is c's (a rule of its component names an unknown, though it is confirmed). The steady
    state is provisional. The estimates it read: pt.b, its value 3 and its estimate as written."""
    unknown = {
        "id": "unk.c", "kind": "interval", "question": "fixture", "interval": {"low": "0", "high": "1"},
        "status": "provisional", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }
    folder = model(
        tmp_path,
        [resource("res.a"), resource("res.b"), resource("res.c"), activity("act.a"), activity("act.b"),
         activity("act.c")],
        [
            flow("flow.a", "act.a", produces={"res.a": "2"}), limit("limit.a", "act.a", "1"),
            derive("pt.b", "act.b", "3", ESTIMATE_B),
            flow("flow.b", "act.b", produces={"res.b": "rule:pt.b"}), limit("limit.b", "act.b", "1"),
            {**flow("flow.c", "act.c", produces={"res.c": "1"}), "unknowns": ["unk.c"]}, limit("limit.c", "act.c", "1"),
            goal("goal.a", "res.a", "1"), goal("goal.b", "res.b", "1"), goal("goal.c", "res.c", "1"),
        ],
        [unknown],
    )

    status, doc = acmodel("economy", folder, "--steady-only")

    assert (status, doc["errors"], without_dual_checks(doc)) == (0, [], [])
    steady = doc["data"]["steady"]
    assert (steady["status"], steady["objective"], steady["acquisition"]) == ("provisional", "6", "6")
    assert steady["prices"] == {"res.a": ["1"], "res.b": ["1"], "res.c": ["1"]}
    assert steady["price_status"] == {
        "res.a": {"status": "confirmed", "reasons": []},
        "res.b": {"status": "provisional", "reasons": ["inputs"]},
        "res.c": {"status": "provisional", "reasons": ["inputs"]},
    }
    coverage = steady["coverage"]
    assert coverage["pools"] == {
        "res.a": {"class": "covered", "status": "confirmed", "reasons": ["goal"], "component": "act.a"},
        "res.b": {"class": "covered", "status": "provisional", "reasons": ["goal"], "component": "act.b"},
        "res.c": {"class": "covered", "status": "provisional", "reasons": ["goal"], "component": "act.c"},
    }
    assert coverage["components"] == {
        name: {"members": [name, f"goal.{name[4:]}", f"res.{name[4:]}"], "status": found, "gap": False, "gaps": {},
               "reasons": [] if found == "confirmed" else ["inputs"]}
        for name, found in (("act.a", "confirmed"), ("act.b", "provisional"), ("act.c", "provisional"))
    }
    assert coverage["counts"] == {
        "covered": 3, "structural_gap": 0, "verified_zero": 0, "unknown": 0, "with_estimates": 2,
    }
    assert coverage["estimates"] == [{"rule": "pt.b", "value": "3", "estimate": ESTIMATE_B}]


# §3.7: the estimate of a provisional derive and its checks 1–4 ----------------------------------------------------

OFFER = {
    "id": "unk.offer", "kind": "variants", "question": "fixture",
    "variants": {"cheap": {"price": "10"}, "dear": {"price": "30"}},
    "status": "provisional", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
}
RATE = {
    "id": "unk.rate", "kind": "distribution", "question": "fixture",
    "distribution": {"family": "beta_from_samples", "samples": 40, "successes": 10},
    "status": "provisional", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
}
COVERS = {"activities": ["act.x"]}


def estimate_fixture(folder: Path, value: str, estimate: dict[str, Any], **members: Any) -> Path:
    """One provisional derive pt.x (rule 0) of `value` with `estimate`, about act.x, which nothing reads; the unknowns
    unk.offer (a variant's price: 10 or 30) and unk.rate (a rate from 10 successes in 40 samples)."""
    return model(folder, [activity("act.x")], [derive("pt.x", "act.x", value, estimate, **members)], [OFFER, RATE])


@pytest.mark.parametrize(
    ("estimate", "status", "reason", "ref", "key", "expected"),
    [
        pytest.param({"of": "unknown:unk.offer.price", "low": "0", "high": "1"}, "provisional", "member_conflict",
                     "/rules/0/logic/estimate/low", "low", ["of", "low"], id="of_and_low"),
        pytest.param({}, "provisional", "member_missing", "/rules/0/logic/estimate/of", "of", ["of", "low"],
                     id="neither"),
        pytest.param({"low": "0"}, "provisional", "member_missing", "/rules/0/logic/estimate/high", "high",
                     ["low", "high"], id="low_alone"),
        pytest.param({"low": "0", "high": "1"}, "confirmed", "member_unknown", "/rules/0/logic/estimate", "estimate",
                     None, id="on_a_confirmed_rule"),
    ],
)
def test_check_1_an_estimate_writes_one_of_of_and_low_high(tmp_path, acmodel, estimate, status, reason, ref, key, expected):
    """Check 1 (§3.7): exactly one of `of` and the pair `low`/`high`, which go together; and an estimate only on a
    provisional rule (on a confirmed one it is `member_unknown`). The reader refuses each (`model_file_invalid`); the
    passing case is check 2's."""
    folder = estimate_fixture(tmp_path, "1/2", {**estimate, "basis": "design", "covers": COVERS}, status=status)

    code, doc = acmodel("check", folder)

    assert code == 1
    detail = {"reason": reason, "path": rules_path(folder), "format": "acmodel.rules.v1", "key": key}
    assert doc["errors"] == [{"code": "model_file_invalid", "detail": {
        **detail, **({"expected": expected} if expected is not None else {})}, "ref": ref}]


@pytest.mark.parametrize(
    ("value", "estimate", "low", "high"),
    [
        pytest.param("5", {"low": "0", "high": "1", "basis": "design"}, "0", "1", id="low_high"),
        pytest.param("40", {"of": "unknown:unk.offer.price", "basis": "variants"}, "10", "30", id="of_variants"),
    ],
)
def test_check_2_the_point_value_lies_in_the_range(tmp_path, acmodel, value, estimate, low, high):
    """Check 2 (§3.7), where the rule is computed (`acmodel run`). The range of `low`/`high` is 0 to 1, and that of
    `of`, a variant's price, 10 to 30 (each variant once). A point inside (1/2, 20) passes and the rule is computed, a
    provisional value; one outside (5, 40) blocks the rule (`estimate_outside_range`, at its estimate)."""
    inside = {"5": "1/2", "40": "20"}[value]
    folder = estimate_fixture(tmp_path / "inside", inside, {**estimate, "covers": COVERS})

    status, doc = acmodel("run", folder)

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["rules"]["pt.x"]["status"], doc["data"]["rules"]["pt.x"]["value"]) == ("provisional", inside)

    folder = estimate_fixture(tmp_path / "outside", value, {**estimate, "covers": COVERS})

    status, doc = acmodel("run", folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "estimate_outside_range",
        "detail": {"path": rules_path(folder), "id": "pt.x", "value": [value], "low": low, "high": high},
        "ref": "/rules/0/logic/estimate", "needed_by": ["rule:pt.x"],
    }]
    assert doc["data"]["rules"]["pt.x"]["status"] == "blocked"


SUPPLEMENT = {"kind": "measurement", "ref": "fixture", "date": "2026-10-31", "role": "supplement", "searched": []}


@pytest.mark.parametrize(
    ("value", "estimate", "provenance", "missing"),
    [
        pytest.param("20", {"low": "10", "high": "30", "basis": "variants"}, PROVENANCE, "variants_unknown",
                     id="variants_without_a_variants_unknown"),
        pytest.param("2", {"low": "1", "high": "3", "basis": "samples"}, PROVENANCE, "supplement",
                     id="samples_without_a_supplement"),
        pytest.param("1/4", {"of": "unknown:unk.rate", "basis": "samples"}, [SUPPLEMENT], "samples",
                     id="a_rate_without_its_samples"),
    ],
)
def test_check_3_an_estimate_has_what_its_basis_needs(tmp_path, acmodel, value, estimate, provenance, missing):
    """Check 3 (§3.7). `variants` needs an `of` that reads a variants unknown's member; `samples` a supplement in the
    provenance, and, when `of` reads a rate from samples, one that carries its samples and successes. Each failing
    case is refused at the basis (`estimate_basis_invalid`); the same estimate with what it lacks passes."""
    folder = estimate_fixture(tmp_path / "lacks", value, {**estimate, "covers": COVERS}, provenance=provenance)

    status, doc = acmodel("check", folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "estimate_basis_invalid",
        "detail": {"path": rules_path(folder), "id": "pt.x", "basis": estimate["basis"], "missing": missing},
        "ref": "/rules/0/logic/estimate/basis",
    }]

    fixed = {
        "variants_unknown": ({"of": "unknown:unk.offer.price", "basis": "variants"}, PROVENANCE),
        "supplement": (estimate, [SUPPLEMENT]),
        "samples": (estimate, [{**SUPPLEMENT, "samples": 40, "successes": 10}]),
    }[missing]
    folder = estimate_fixture(tmp_path / "has", value, {**fixed[0], "covers": COVERS}, provenance=fixed[1])

    status, doc = acmodel("check", folder)

    assert (status, doc["errors"]) == (0, [])


@pytest.mark.parametrize("covered", [["act.a"], ["act.a", "act.b"]], ids=["act_b_left_out", "both"])
def test_check_4_every_activity_that_reads_the_estimate_is_covered(tmp_path, acmodel, covered):
    """Check 4 (§3.7). pt.x (2, estimated 1 to 3) is read by flow.a of act.a directly and by flow.b of act.b through
    pt.y, a confirmed derive of twice pt.x. Covering only act.a, the estimate would be used for act.b, which it does not
    cover: `estimate_scope_exceeded` (act.b, read by flow.b), at its covers. Covering both passes."""
    estimate = {"low": "1", "high": "3", "basis": "design", "covers": {"activities": covered}}
    folder = model(
        tmp_path,
        [resource("res.v"), activity("act.a"), activity("act.b")],
        [
            derive("pt.x", "act.a", "2", estimate),
            {**rule("pt.y", "derive", "act.b"), "logic": {"value": "rule:pt.x * 2"}},
            flow("flow.a", "act.a", produces={"res.v": "rule:pt.x"}),
            flow("flow.b", "act.b", produces={"res.v": "rule:pt.y"}),
        ],
    )

    status, doc = acmodel("check", folder)

    if covered == ["act.a", "act.b"]:
        assert (status, doc["errors"]) == (0, [])
        return
    assert status == 1
    assert doc["errors"] == [{
        "code": "estimate_scope_exceeded",
        "detail": {"path": rules_path(folder), "id": "pt.x", "activities": ["act.b"], "read_by": ["flow.b"]},
        "ref": "/rules/0/logic/estimate/covers/activities",
    }]
