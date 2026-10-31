"""PY-10: the economy (`acmodel economy`), its steady state and plan solved by scipy's HiGHS, on hand-solvable
neutral fixtures. Every expected number below is worked out by hand in the fixture's docstring; CI runs these
tests on Windows and on Ubuntu, so both legs must give the same exact values."""

from __future__ import annotations

import json
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli
from acmodel.core import flow as economy_flow
from acmodel.envelope import Failure

CODES =json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
PROVENANCE = [{"kind": "logic", "ref": "fixture"}]
DAY = {"id": "day", "unit": "day", "reset_minute": 0, "utc_offset_minutes": 0}
WEEK = {"id": "week", "unit": "week", "reset_minute": 0, "utc_offset_minutes": 0, "reset_weekday": "mon"}
SEVEN = ["1"] * 7


def resource(entity_id: str, fact: str) -> dict[str, Any]:
    return {"id": entity_id, "kind": "resource", "reading": fact}


def store(entity_id: str, holds: str, fact: str) -> dict[str, Any]:
    return {"id": entity_id, "kind": "store", "holds": holds, "reading": fact}


def activity(entity_id: str) -> dict[str, Any]:
    return {"id": entity_id, "kind": "activity"}


def rule(rule_id: str, kind: str, subject: str, **logic: Any) -> dict[str, Any]:
    return {"id": rule_id, "kind": kind, "subject": subject, "logic": logic, "provenance": PROVENANCE, "status": "confirmed"}


def flow(rule_id: str, subject: str, **logic: Any) -> dict[str, Any]:
    return rule(rule_id, "flow", subject, **logic)


def limit(rule_id: str, subject: str, maximum: str, per: str, **logic: Any) -> dict[str, Any]:
    return rule(rule_id, "limit", subject, max=maximum, per=per, **logic)


def regen(rule_id: str, subject: str, amount: str, every: str, **logic: Any) -> dict[str, Any]:
    return rule(rule_id, "regen", subject, amount=amount, every=every, **logic)


def cap(rule_id: str, subject: str, cap_kind: str, on_reach: Any, **logic: Any) -> dict[str, Any]:
    return rule(rule_id, "cap", subject, cap_kind=cap_kind, on_reach=on_reach, **logic)


def write(path: Path, doc: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


NUMERAIRE = "value.economy"


def goal(rule_id: str, subject: str, value: str) -> dict[str, Any]:
    """A main objective: one unit of `subject` is worth `value` value units (PRICE-2a)."""
    return rule(rule_id, "objective", subject, tier="main", value=value)


def training(value: str, horizon: str) -> dict[str, Any]:
    """The secondary objective: the demand table `refdemand`, its costs in `training_costs`, a point worth `value`."""
    return rule(
        "goal.training", "objective", NUMERAIRE, tier="secondary", demand="refdemand", costs="training_costs",
        value=value, horizon=horizon,
    )


def tables(base: Path, demand: list[tuple[str, str, str, str]], costs: list[tuple[str, dict[str, str]]]) -> list[str]:
    """The secondary objective's two tables: `demand` as (step, count, points, main), `costs` as (step, resource to
    amount; 0 where a step lists none). Returns their files."""
    columns = {"count": "count", "points": "none", "main": "none"}
    write(base / "data" / "refdemand.json", {
        "schema_version": "acmodel.data.v1", "table": "refdemand", "key": "step",
        "columns": {name: {"type": "decimal", "dimension": dimension} for name, dimension in columns.items()},
        "rows": [
            {"step": step, "count": count, "points": points, "main": main, "status": "confirmed"}
            for step, count, points, main in demand
        ],
        "provenance": PROVENANCE,
    })
    resources = sorted({resource for _, amounts in costs for resource in amounts})
    write(base / "data" / "training_costs.json", {
        "schema_version": "acmodel.data.v1", "table": "training_costs", "key": "step",
        "columns": {resource: {"type": "decimal", "dimension": f"amount:{resource}"} for resource in resources},
        "rows": [
            {"step": step, **{resource: amounts.get(resource, "0") for resource in resources}, "status": "confirmed"}
            for step, amounts in costs
        ],
        "provenance": PROVENANCE,
    })
    return ["data/refdemand.json", "data/training_costs.json"]


def model(
    folder: Path, periods: list, entities: list, rules: list, readings: dict[str, int], unknowns: list | None = None,
    *, goals: list | None = None, demand: list | None = None, costs: list | None = None,
) -> Path:
    """A neutral repository folder with one rule file, and its account state; returns the folder.

    The numeraire is the value entity `value.economy` (#424 ACM-PRICE PRICE-2a), and `goals` are the objective rules,
    after the others. By default res.v is the one main goal, worth 1 value unit: what the fixtures of PY-10, PRICE-1
    and PRICE-3 counted value in when res.v was the numeraire, so that their values stay as they were. With `demand`
    and `costs`, the secondary objective's two tables (`tables`)."""
    base = folder / "ours" / "model"
    data = tables(base, demand, costs or []) if demand is not None else []
    write(base / "model.json", {
        "schema_version": "acmodel.model.v1", "model_id": "economy",
        "files": {"rules": ["rules/economy.json"], "data": data}, "periods": periods, "numeraire": NUMERAIRE,
        "engines": [],
    })
    write(base / "sources.json", {"schema_version": "acmodel.sources.v1", "sources": []})
    write(base / "exports.json", {"schema_version": "acmodel.exports.v1", "thresholds": [], "tables": [], "scoring": []})
    write(base / "rules" / "economy.json", {
        "schema_version": "acmodel.rules.v1", "entities": [*entities, {"id": NUMERAIRE, "kind": "value"}],
        "rules": [*rules, *(goals if goals is not None else [goal("goal.v", "res.v", "1")])], "unknowns": unknowns or [],
    })
    write(folder / "state.json", {
        "schema_version": "acmodel.state.v1", "model_id": "economy", "label": "acct-1",
        "readings": {fact: {"value": value, "observed_at_unix_ms": 0, "source": "fixture"} for fact, value in readings.items()},
        "entities": {}, "cleared": [],
    })
    return folder


@pytest.fixture
def economy(make_system, app_dir, write_config):
    """`acmodel economy` on a fixture folder, with the given flags and configuration keys: (exit status,
    envelope). The envelope is one acmodel.cli.v1 line whose errors and warnings agree with contracts/codes.json."""

    def run(folder: Path, *flags: str, **sections: Any) -> tuple[int, dict[str, Any]]:
        if sections:
            write_config({"schema_version": "acmodel.config.v1", **sections})
        system = make_system(prefix=app_dir / "venv")
        argv = ["economy", str(folder / "ours" / "model"), "--state", str(folder / "state.json"), *flags]
        status = cli.main(argv, system)
        [line] = system.stdout.getvalue().splitlines()
        doc = json.loads(line)
        assert doc["command"] == "economy" and doc["ok"] is (doc["errors"] == [])
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


def steady(doc: dict[str, Any]) -> dict[str, Any]:
    """The steady state without HiGHS's marginals, which are a cross-check only (and need not be unique)."""
    return {key: value for key, value in doc["data"]["steady"].items() if key != "marginals"}


def settled(*resources: str) -> dict[str, Any]:
    """What a confirmed steady state that left no pending flow out shows of it (#424 ACM-PRICE addendum, part 1):
    no `excluded_pending`, and each price confirmed, for no reason."""
    return {
        "excluded_pending": [],
        "price_status": {resource: {"status": "confirmed", "reasons": []} for resource in resources},
    }


def acquirer(activity: str, goals: list[str], bounds: list[str], inputs: list[str], spends: list[str] = ()) -> dict:
    """An activity that acquires a main goal item, as the steady state lists it (PRICE-2a, addendum 2.1 §4.1)."""
    return {"activity": activity, "goals": goals, "bounds": bounds, "inputs": inputs, "spends": list(spends)}


def acquisitions(acquisition: str, protected: list[dict] = (), conversions: list[dict] = ()) -> dict[str, Any]:
    """What the steady state shows of its tiers (PRICE-2a): A* and the acquisitions, protected (P) and not."""
    return {"acquisition": acquisition, "protected": list(protected), "conversions": list(conversions)}


def store_fixture(folder: Path) -> Path:
    """A store that fills 20 an hour (480 a day) up to 400, collected at most once a day; what is collected is
    used by act.a (10 c and 1 s for 2 v, at most 30 a day) and act.b (10 c for 1 v, no limit); s comes at 100
    a day.

    By hand, a day (the cycle; the model has a day period only): one collection takes the full store, 400, and
    80 of the 480 produced are lost. act.a runs 30 times (300 c, 30 s, 60 v), act.b takes the other 100 c (10
    v): 70 v. One more c goes to act.b, a tenth of a v, and so does one less: c is worth 1/10. s is slack (70 of
    it left over each day): worth 0. A storage cap one higher collects one more c, 1/10; act.a's limit one
    higher moves 10 c from act.b to act.a, +1; a second collection takes the other 80 (+8) and none loses all
    70. act.a is worth 2 - 1 = 1 per execution and act.b, the marginal activity, 0; a collection's worth
    depends on what the store holds.

    The tiers (PRICE-2a): act.a and act.b acquire v, the main goal, but both spend c, which has no regen: they are
    conversions, so nothing is protected and A* is 0."""
    return model(
        folder, [DAY],
        [resource("res.v", "fact.v"), resource("res.c", "fact.c"), resource("res.s", "fact.s"),
         store("fac.f", "res.c", "fact.f"), activity("act.collect"), activity("act.a"), activity("act.b")],
        [
            cap("cap.f.storage", "fac.f", "storage", "stop_production", value="400"),
            regen("regen.f", "fac.f", "20", "1h", **{"while": "stock:fac.f < cap:cap.f.storage"}),
            regen("regen.s", "res.s", "100", "1d"),
            flow("flow.collect", "act.collect", moves={"from": "fac.f", "to": "res.c", "amount": "stock:fac.f"}),
            limit("limit.collect", "act.collect", "1", "day"),
            flow("flow.a", "act.a", consumes={"res.c": "10", "res.s": "1"}, produces={"res.v": "2"}),
            limit("limit.a", "act.a", "30", "day"),
            flow("flow.b", "act.b", consumes={"res.c": "10"}, produces={"res.v": "1"}),
        ],
        {"fact.v": 0, "fact.c": 0, "fact.s": 0, "fact.f": 0},
    )


STORE_STEADY = {
    "status": "confirmed", "per_account": False,
    "cycle": {"steps": 1, "step_seconds": 86400},
    "objective": "70",
    **acquisitions("0", conversions=[
        acquirer("act.a", ["res.v"], ["limit.a"], ["res.c", "res.s"], ["res.c"]),
        acquirer("act.b", ["res.v"], [], ["res.c"], ["res.c"]),
    ]),
    "prices": {"res.c": ["1/10"], "res.s": ["0"], "res.v": ["1"]},
    "rule_prices": {
        "cap.f.storage": "1/10", "limit.a": "1", "limit.collect": {"low": "8", "high": "70"},
    },
    "activity_values": {"act.a": ["1"], "act.b": ["0"], "act.collect": None},
    "losses": {"overflow": {}, "lost_production": {"fac.f": "80"}},
    "excluded": [],
    "relaxed": [],
    **settled("res.c", "res.s", "res.v"),
}


def cyclic_fixture(folder: Path) -> Path:
    """r comes as 70 at each weekly reset into a hard cap of 50; act.d uses 10 r for 1 v, at most once a day.

    By hand, a week of seven daily steps (the model has a week period): starting empty, the reset step takes
    70, act.d uses 10, the stock stops at the cap (50) and 10 overflow. The six other days have 50 left: five
    executions. Six v a week, and the week ends with the stock it began with. One more r on the reset step only
    overflows (worth 0); on any other day it gives a tenth more of an execution (1/10), and one less takes a
    tenth away. The cap one higher keeps one more r for those days: 1/10, act.d's value per unit of r, the
    marginal activity. A daily limit of 2 uses 20 on the reset step, so nothing overflows (+1); a limit of 0
    loses all 6. act.d is worth 1 on the reset step and 0 on the others.

    The tiers (PRICE-2a): act.d is protected (its only input, r, has a regen, and a limit counts it); the most it can
    bring in is the same 6 v, so A* is 6 and the optimum of V keeps it."""
    return model(
        folder, [DAY, WEEK],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.d")],
        [
            regen("regen.r", "res.r", "70", "7d"),
            cap("cap.r.hard", "res.r", "hard", "block_all_income", value="50"),
            flow("flow.d", "act.d", consumes={"res.r": "10"}, produces={"res.v": "1"}),
            limit("limit.d", "act.d", "1", "day"),
        ],
        {"fact.v": 0, "fact.r": 0},
    )


CYCLIC_STEADY = {
    "status": "confirmed", "per_account": False,
    "cycle": {"steps": 7, "step_seconds": 86400},
    "objective": "6",
    **acquisitions("6", [acquirer("act.d", ["res.v"], ["limit.d"], ["res.r"])]),
    "prices": {"res.r": ["0"] + ["1/10"] * 6, "res.v": SEVEN},
    "rule_prices": {"cap.r.hard": "1/10", "limit.d": {"low": "1", "high": "6"}},
    "activity_values": {"act.d": ["1"] + ["0"] * 6},
    "losses": {"overflow": {"res.r": "10"}, "lost_production": {}},
    "excluded": [],
    "relaxed": [],
    **settled("res.r", "res.v"),
}


def test_a_slack_resource_has_price_zero(tmp_path, economy):
    status, doc = economy(store_fixture(tmp_path))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"]["res.s"] == ["0"]
    assert steady(doc) == STORE_STEADY


@pytest.mark.parametrize(
    ("fixture", "expected", "cap_id", "marginal_activity", "resource_id", "step"),
    [
        pytest.param(cyclic_fixture, CYCLIC_STEADY, "cap.r.hard", "act.d", "res.r", 1, id="hard"),
        pytest.param(store_fixture, STORE_STEADY, "cap.f.storage", "act.b", "res.c", 0, id="storage"),
    ],
)
def test_a_binding_cap_prices_at_the_marginal_activitys_value(
    tmp_path, economy, fixture, expected, cap_id, marginal_activity, resource_id, step
):
    status, doc = economy(fixture(tmp_path))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = steady(doc)
    assert found == expected
    # The marginal activity breaks even (value 0) and converts 10 of the resource into 1 v: 1/10 a unit,
    # which is the price of the resource and of one more unit of the binding cap.
    assert found["activity_values"][marginal_activity][step] == "0"
    assert found["rule_prices"][cap_id] == found["prices"][resource_id][step] == "1/10"


@pytest.mark.parametrize(
    ("reading", "moved", "objective"),
    [
        pytest.param(599, "400", "60", id="below"),
        pytest.param(600, "0", "20", id="at"),
        pytest.param(601, "0", "20", id="above"),
    ],
)
def test_collection_is_blocked_at_or_above_the_read_cap(tmp_path, economy, reading, moved, objective):
    """A one-day plan. The read cap of c is 600 (its reading); c has a hard cap of 1000. The full store holds
    400 and makes 480 a day; one collection a day takes up to the full store. act.a (10 c for 2 v, at most 20)
    and act.b (10 c for 1 v) use c; the steady state prices c at 1/10 (act.b is its marginal activity) and v
    at 1, which value the plan's final stocks.

    By hand: below the read cap the collection takes 400 (worth 40 at 1/10 a unit) and act.a runs 20 times
    (+1 each over keeping the c): 60. At or above it the collection is blocked and moves nothing; act.a still
    runs 20 times from the stock: 20."""
    folder = model(
        tmp_path, [DAY],
        [resource("res.v", "fact.v"), resource("res.c", "fact.c"), store("fac.f", "res.c", "fact.f"),
         activity("act.collect"), activity("act.a"), activity("act.b")],
        [
            cap("cap.c.hard", "res.c", "hard", "block_all_income", value="1000"),
            cap("cap.c.read", "res.c", "soft_read", {"block_income_from": ["act.collect"], "other_income": "notice_only"},
                reading="fact.c_capacity"),
            cap("cap.f.storage", "fac.f", "storage", "stop_production", value="400"),
            regen("regen.f", "fac.f", "20", "1h", **{"while": "stock:fac.f < cap:cap.f.storage"}),
            flow("flow.collect", "act.collect", moves={"from": "fac.f", "to": "res.c", "amount": "stock:fac.f"}),
            limit("limit.collect", "act.collect", "1", "day"),
            flow("flow.a", "act.a", consumes={"res.c": "10"}, produces={"res.v": "2"}),
            limit("limit.a", "act.a", "20", "day"),
            flow("flow.b", "act.b", consumes={"res.c": "10"}, produces={"res.v": "1"}),
        ],
        {"fact.v": 0, "fact.c": reading, "fact.c_capacity": 600, "fact.f": 400},
    )

    status, doc = economy(folder, economy={"plan_horizon": 1})

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc)["prices"] == {"res.c": ["1/10"], "res.v": ["1"]}
    plan = doc["data"]["plan"]
    assert (plan["status"], plan["per_account"], plan["steps"], plan["proven"]) == ("confirmed", True, 1, True)
    assert plan["moved"] == {"flow.collect": [moved]}
    assert plan["counts"]["act.a"] == ["20"]
    assert plan["objective"] == objective


def test_a_full_store_stops_production_and_the_loss_is_counted(tmp_path, economy):
    status, doc = economy(store_fixture(tmp_path))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = steady(doc)
    assert found["losses"] == {"overflow": {}, "lost_production": {"fac.f": "80"}}
    assert found["objective"] == "70"


def test_per_period_and_shared_limits(tmp_path, economy):
    """r comes at 100 a day, ten executions of 10 r. act.a (3 v) may run twice a day, act.b (2 v) five times a
    day, and the two share a weekly limit of 10; act.c (1 v) has no limit.

    By hand, a week: 70 executions in all. act.a takes the 10 shared ones (14 would fit its daily limit),
    act.b none, act.c the other 60: 90 v. r is worth 1/10 (act.c is the marginal activity) on every day. One
    more shared execution moves one from act.c to act.a: +2, and one less -2. act.a's daily limit one higher
    changes nothing (the weekly limit binds), one lower (seven a week) lets act.b take the three freed shared
    executions from act.c: 3 lost. act.b's daily limit does not bind either way. Per execution act.a is worth
    3 - 1 = 2, act.b 1 and act.c 0.

    The tiers (PRICE-2a): act.a and act.b are protected (r has a regen; their limits count them), act.c, with no
    limit, is a conversion. A* is what act.a brings in when it takes the 10 shared executions, 30, which the optimum
    of V keeps."""
    folder = model(
        tmp_path, [DAY, WEEK],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.a"), activity("act.b"), activity("act.c")],
        [
            regen("regen.r", "res.r", "100", "1d"),
            flow("flow.a", "act.a", consumes={"res.r": "10"}, produces={"res.v": "3"}),
            limit("limit.a.day", "act.a", "2", "day"),
            flow("flow.b", "act.b", consumes={"res.r": "10"}, produces={"res.v": "2"}),
            limit("limit.b.day", "act.b", "5", "day"),
            limit("limit.ab.week", "act.a", "10", "week", shared_with=["act.b"]),
            flow("flow.c", "act.c", consumes={"res.r": "10"}, produces={"res.v": "1"}),
        ],
        {"fact.v": 0, "fact.r": 0},
    )

    status, doc = economy(folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 7, "step_seconds": 86400},
        "objective": "90",
        **acquisitions(
            "30",
            [
                acquirer("act.a", ["res.v"], ["limit.a.day", "limit.ab.week"], ["res.r"]),
                acquirer("act.b", ["res.v"], ["limit.ab.week", "limit.b.day"], ["res.r"]),
            ],
            [acquirer("act.c", ["res.v"], [], ["res.r"])],
        ),
        "prices": {"res.r": ["1/10"] * 7, "res.v": SEVEN},
        "rule_prices": {"limit.a.day": {"low": "0", "high": "3"}, "limit.ab.week": "2", "limit.b.day": "0"},
        "activity_values": {"act.a": ["2"] * 7, "act.b": SEVEN, "act.c": ["0"] * 7},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.r", "res.v"),
    }


def test_a_cyclic_steady_state(tmp_path, economy):
    status, doc = economy(cyclic_fixture(tmp_path))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = steady(doc)
    assert (found["cycle"], found["objective"]) == ({"steps": 7, "step_seconds": 86400}, "6")
    assert found["prices"]["res.r"] == ["0", "1/10", "1/10", "1/10", "1/10", "1/10", "1/10"]
    assert found["losses"]["overflow"] == {"res.r": "10"}
    assert found == CYCLIC_STEADY


def test_contradictory_rules_are_named(tmp_path, economy):
    """r drains by 10 a day, and its only income is act.p, 5 r an execution, at most once a day. A steady state
    would need two executions a day: the limit and the drain contradict each other, and nothing else does."""
    folder = model(
        tmp_path, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), resource("res.w", "fact.w"), activity("act.p"),
         activity("act.q")],
        [
            regen("regen.drain", "res.r", "-10", "1d"),
            flow("flow.p", "act.p", produces={"res.r": "5"}),
            limit("limit.p", "act.p", "1", "day"),
            regen("regen.w", "res.w", "10", "1d"),
            flow("flow.q", "act.q", consumes={"res.w": "5"}, produces={"res.v": "1"}),
            limit("limit.q", "act.q", "2", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.w": 0},
    )

    status, doc = economy(folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "lp_infeasible",
        "detail": {"path": str(folder / "ours" / "model" / "model.json"), "solve": "steady", "rules": ["limit.p", "regen.drain"]},
        "needed_by": ["economy:plan", "economy:steady"],
    }]
    assert doc["data"]["steady"] == {"status": "blocked"}
    assert doc["data"]["plan"] == {"status": "blocked"}


def test_a_degenerate_fixture_gives_a_price_interval(tmp_path, economy):
    """r comes at 30 a day, exactly what act.a (10 r for 2 v) needs at its limit of 3 a day.

    By hand: 6 v a day. One more r cannot be used (worth 0); one less takes a tenth of an execution away
    (1/5). The price is the interval [0, 1/5], and so act.a is worth [2 - 10 * 1/5, 2 - 0] = [0, 2]. Its
    limit one higher finds no r (0); one lower loses 2. act.a is protected (PRICE-2a): A* is its 6 v."""
    folder = model(
        tmp_path, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.a")],
        [
            regen("regen.r", "res.r", "30", "1d"),
            flow("flow.a", "act.a", consumes={"res.r": "10"}, produces={"res.v": "2"}),
            limit("limit.a", "act.a", "3", "day"),
        ],
        {"fact.v": 0, "fact.r": 0},
    )

    status, doc = economy(folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "6",
        **acquisitions("6", [acquirer("act.a", ["res.v"], ["limit.a"], ["res.r"])]),
        "prices": {"res.r": [{"low": "0", "high": "1/5"}], "res.v": ["1"]},
        "rule_prices": {"limit.a": {"low": "0", "high": "2"}},
        "activity_values": {"act.a": [{"low": "0", "high": "2"}]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.r", "res.v"),
    }


def _weights(count: int, resources: int) -> list[list[int]]:
    """Fixed pseudo-random weights from a linear congruential sequence: neutral, and the same everywhere."""
    state, found = 12345, []
    for _ in range(count):
        row = []
        for _ in range(resources + 1):
            state = (1103515245 * state + 12345) % 2**31
            row.append(200 + state % 801)
        found.append(row)
    return found


def test_the_node_limit_gives_search_budget_exhausted_unproven(tmp_path, economy, monkeypatch):
    """A one-day plan that is a knapsack with four resources: 30 activities, each at most once a week, each
    using some of every resource for some v. The steady state has all the resources it needs (each comes at a
    seventh of the week's need, and more), so they are worth 0 there at every step and v is worth 1: the plan's
    objective is the v it gains. The plan has about half of each resource, and starts with the reading plus one
    day's income: half the total need, rounded down.

    The node limit is reached through a seam, not through the fixture. Real HiGHS proves this knapsack optimal
    at its root (CI, scipy 1.18.1), so a node limit of one is never reached. No fixture can be shown to stay
    open at the root for every HiGHS version. Presolve scales an integer row to coprime coefficients and rounds
    its right-hand side, and the mod-k separator cuts parity, so a Jeroslow-type parity closes at once. What
    else closes a root (cuts, root heuristics, restarts) changes from one version to the next.

    So scipy's HiGHS wrapper still solves the plan's MILP, with the options the economy gives it, and the
    incumbent is HiGHS's own. Only its model status is replaced, by the one HiGHS sets once `mip_max_nodes`
    nodes are explored: kSolutionLimit (16, from `HighsMipSolverData::checkLimits`). HiGHS's message for it and
    a node count of 1 go with it. scipy's `milp` then converts it as it converts a real stop at the node limit.
    scipy 1.18.1 does not map kSolutionLimit, so that is its status 4, with `x`."""
    import scipy.optimize._highspy._core as highs
    import scipy.optimize._milp as scipy_milp

    solve, stop = scipy_milp._highs_wrapper, highs.HighsModelStatus.kSolutionLimit
    calls: list[tuple[dict[str, Any], dict[str, Any]]] = []

    def stopped_at_the_node_limit(*args: Any) -> dict[str, Any]:
        found = solve(*args)
        calls.append((dict(args[-1]), found))
        return {**found, "status": stop, "message": highs._Highs().modelStatusToString(stop), "mip_node_count": 1}

    monkeypatch.setattr(scipy_milp, "_highs_wrapper", stopped_at_the_node_limit)
    count, kinds = 30, 4
    weights = _weights(count, kinds)
    resources = [f"res.r{index}" for index in range(kinds)]
    totals = [sum(row[index] for row in weights) for index in range(kinds)]
    entities = [resource("res.v", "fact.v"), *(resource(r, f"fact.{r[4:]}") for r in resources)]
    rules = []
    for index, total in enumerate(totals):
        rules.append(regen(f"regen.r{index}", resources[index], str(-(-(total + 1) // 7)), "1d"))
    for number, row in enumerate(weights):
        name = f"act.k{number:02d}"
        entities.append(activity(name))
        consumes = {resources[index]: str(row[index]) for index in range(kinds)}
        rules.append(flow(f"flow.k{number:02d}", name, consumes=consumes, produces={"res.v": str(row[kinds])}))
        rules.append(limit(f"limit.k{number:02d}", name, "1", "week"))
    readings = {"fact.v": 0}
    for index, total in enumerate(totals):
        readings[f"fact.r{index}"] = total // 2 - (-(-(total + 1) // 7))
    folder = model(tmp_path, [DAY, WEEK], entities, rules, readings)

    status, doc = economy(folder, economy={"plan_horizon": 1}, search={"milp_node_limit": 1})

    # One MILP, the plan's: within the node limit, with a relative gap of 0 and no time limit. It stopped with
    # an incumbent, so it is not tried again.
    [(options, found)] = calls
    assert (options["mip_max_nodes"], options["mip_rel_gap"], "time_limit" in options) == (1, 0.0, False)
    assert found["x"] is not None
    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == [{
        "code": "search_budget_exhausted",
        "detail": {
            "path": str(folder / "ours" / "model" / "model.json"), "solve": "plan", "node_limit": 1, "nodes": 1,
            "status": 4,
        },
    }]
    assert steady(doc)["prices"] == {"res.v": SEVEN, **{r: ["0"] * 7 for r in resources}}
    plan = doc["data"]["plan"]
    assert (plan["status"], plan["per_account"], plan["steps"], plan["proven"]) == ("confirmed", True, 1, False)
    # The plan is feasible: each activity runs once or not at all, and no stock ends below 0.
    runs = []
    for number in range(count):
        [executions] = plan["counts"][f"act.k{number:02d}"]
        assert executions in ("0", "1")
        runs.append(int(executions))
    for index, total in enumerate(totals):
        left = total // 2 - sum(row[index] * ran for row, ran in zip(weights, runs))
        assert left >= 0
        assert plan["end"][resources[index]] == str(left)
    gained = sum(row[kinds] * ran for row, ran in zip(weights, runs))
    assert plan["end"]["res.v"] == plan["objective"] == str(gained)
    assert (plan["moved"], plan["unread"], plan["excluded"]) == ({}, [], [])


# #424 ACM-PRICE PRICE-1: the steady state unblocked -----------------------------------------------------------


def refused(folder: Path, ref: str, found: str, *needed_by: str) -> dict[str, Any]:
    """The economy's refusal of what it does not model (`expr_eval_failed`, `evaluation_unsupported`) at `ref` of
    the fixture's rule file, with the outputs it blocks."""
    detail = {
        "path": str(folder / "ours" / "model" / "rules" / "economy.json"), "reason": "evaluation_unsupported",
        "operator": "economy", "position": 0, "found": found,
    }
    return {"code": "expr_eval_failed", "detail": detail, "ref": ref, "needed_by": list(needed_by)}


def read_cap_fixture(folder: Path, **reference: str) -> Path:
    """The cyclic fixture (r comes as 70 at each weekly reset; act.d uses 10 r for 1 v, at most once a day), its
    hard cap read from the account state (20 here), and with `reference` the cap's value in the reference state."""
    return model(
        folder, [DAY, WEEK],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.d")],
        [
            regen("regen.r", "res.r", "70", "7d"),
            cap("cap.r.hard", "res.r", "hard", "block_all_income", reading="fact.r_cap", **reference),
            flow("flow.d", "act.d", consumes={"res.r": "10"}, produces={"res.v": "1"}),
            limit("limit.d", "act.d", "1", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.r_cap": 20},
    )


def test_the_steady_state_reads_a_caps_reference_and_the_plan_its_reading(tmp_path, economy):
    """Design test 10 (G-A). With the reference 50, the steady state is the cyclic fixture's, whatever the reading.
    The plan reads the cap 20 from the state: by hand, the reset step takes 70, act.d uses 10 and 40 overflow; two
    more days use the 20 left, so the plan gains 3 v (r ends at 0, and is worth 0 at the step the plan ends in).
    Without the reference, the steady state would need the account state: it is refused (`state`)."""
    status, doc = economy(read_cap_fixture(tmp_path / "reference", reference="50"))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == CYCLIC_STEADY
    plan = doc["data"]["plan"]
    assert (plan["status"], plan["proven"], plan["objective"], plan["end"]["res.v"]) == ("confirmed", True, "3", "3")

    folder = read_cap_fixture(tmp_path / "reading")
    status, doc = economy(folder)

    assert status == 1
    assert doc["errors"] == [refused(folder, "/rules/1/logic/reading", "state", "economy:plan", "economy:steady")]
    assert doc["data"]["steady"] == doc["data"]["plan"] == {"status": "blocked"}


def test_a_resources_regen_stops_at_its_caps_reference_and_the_loss_is_counted(tmp_path, economy):
    """Design test 11 (G-B). The cyclic fixture, but r has no hard cap: its regen stops at its natural cap, a read
    cap whose reference is 50 (the reading, 30, is the plan's).

    By hand, a week: on the reset step act.d uses 10 and the regen fills r up to 50, so 10 of the 70 are lost; act.d
    runs on five more days: 6 v. Only the regen stops at the cap: one more r on any day gives a tenth of an execution
    more, and one less a tenth less (1/10 on every day, and act.d is worth 0). The cap one higher loses one r less,
    one lower one more (1/10). A daily limit of 2 uses 20 on the reset step, so nothing is lost (+1); a limit of 0
    loses all 6. act.d is protected (PRICE-2a): r has a regen, if one that stops; A* is its 6 v."""
    folder = model(
        tmp_path, [DAY, WEEK],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.d")],
        [
            regen("regen.r", "res.r", "70", "7d", **{"while": "amount:res.r < read:fact.r_cap"}),
            cap("cap.r.natural", "res.r", "soft_read", {"block_income_from": [], "other_income": "notice_only"},
                reading="fact.r_cap", reference="50"),
            flow("flow.d", "act.d", consumes={"res.r": "10"}, produces={"res.v": "1"}),
            limit("limit.d", "act.d", "1", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.r_cap": 30},
    )

    _, doc = economy(folder)

    assert doc["warnings"] == []
    assert all(entry["needed_by"] == ["economy:plan"] for entry in doc["errors"])  # nothing blocks the steady state
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 7, "step_seconds": 86400},
        "objective": "6",
        **acquisitions("6", [acquirer("act.d", ["res.v"], ["limit.d"], ["res.r"])]),
        "prices": {"res.r": ["1/10"] * 7, "res.v": SEVEN},
        "rule_prices": {"cap.r.natural": "1/10", "limit.d": {"low": "1", "high": "6"}},
        "activity_values": {"act.d": ["0"] * 7},
        "losses": {"overflow": {}, "lost_production": {"res.r": "10"}},
        "excluded": [],
        "relaxed": [],
        **settled("res.r", "res.v"),
    }


def test_a_regen_stop_that_can_hold_no_more_has_a_price_unbounded_below(tmp_path, economy):
    """A regression of PRICE-2a. r's regen (10 a day) stops at its natural cap (reference 50), and its only user,
    act.p (10 r for some v, a pending flow), is left out, so nothing in the program uses r. act.a brings 2 v a day
    for nothing, at most 3 times. Until PRICE-2a the numeraire was a resource without a cyclic row; now it is the
    value entity, and a resource like r, which no longer has that exemption, made the re-solve with one more r
    infeasible, which blocked the steady state (`lp_solve_failed`, stage `price`, status 2).

    By hand, one daily step: r ends the cycle with the stock it began with and nothing uses it, so all 10 its regen
    brings are lost (`lost_production` 10), at most what the regen brings. One more r has nowhere to go: the re-solve
    is infeasible, so the price has no low end (null); one less only loses one r less, so its high end is 0. act.a runs
    3 times: 6 v, which the main goal takes at 1 each, so V0 = 6; v is worth 1, act.a's limit 2 (a fourth execution
    brings 2 v), act.a 2 per execution, and r's cap 0 (one higher or lower changes nothing). act.p's limit counts no
    activity of the program (0). act.a is protected (no input, a limit): A* = 6 = V*. Every price is provisional, for
    act.p's sake. The plan does not model a regen stop (`while`), so it is refused, never crashed."""
    unknown = {
        "id": "unk.p", "kind": "interval", "question": "fixture", "interval": {"low": "1", "high": "3"},
        "status": "pending", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }
    act_p = {**flow("flow.p", "act.p", consumes={"res.r": "10"}, produces={"res.v": "unknown:unk.p"}), "status": "pending"}
    folder = model(
        tmp_path, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.a"), activity("act.p")],
        [
            regen("regen.r", "res.r", "10", "1d", **{"while": "amount:res.r < read:fact.r_cap"}),
            cap("cap.r.natural", "res.r", "soft_read", {"block_income_from": [], "other_income": "notice_only"},
                reading="fact.r_cap", reference="50"),
            flow("flow.a", "act.a", produces={"res.v": "2"}),
            limit("limit.a", "act.a", "3", "day"),
            {**act_p, "unknowns": ["unk.p"]},
            limit("limit.p", "act.p", "1", "day"),
        ],
        {"fact.v": 0, "fact.r": 0, "fact.r_cap": 30},
        [unknown],
    )

    _, doc = economy(folder)

    assert doc["warnings"] == []
    assert all(entry["needed_by"] == ["economy:plan"] for entry in doc["errors"])  # nothing blocks the steady state
    assert doc["data"]["plan"] == {"status": "blocked"}
    assert steady(doc) == {
        "status": "provisional", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "6",
        **acquisitions("6", [acquirer("act.a", ["res.v"], ["limit.a"], [])]),
        "prices": {"res.r": [{"low": None, "high": "0"}], "res.v": ["1"]},
        "rule_prices": {"cap.r.natural": "0", "limit.a": "2", "limit.p": "0"},
        "activity_values": {"act.a": ["2"]},
        "losses": {"overflow": {}, "lost_production": {"res.r": "10"}},
        "excluded": [],
        "relaxed": [],
        "excluded_pending": [{"flow": "flow.p", "activity": "act.p", "unknowns": ["unk.p"]}],
        "price_status": {
            resource: {"status": "provisional", "reasons": ["excluded_pending"]} for resource in ("res.r", "res.v")
        },
    }


def pending_fixture(folder: Path, produces: str, status: str, **unknowns: list[str]) -> Path:
    """r comes at 100 a day. act.a turns 10 r into 1 v, at most 3 times a day; act.p turns 10 r into `produces` v,
    at most 20 times a day, by a flow of `status` (naming `unknowns`). unk.p, how much v one act.p gives, is an
    interval from 1 to 3 that no measurement has narrowed yet.

    act.p's limit never binds (r allows 10 executions a day). It makes act.p a protected acquisition as act.a is
    (PRICE-2a, addendum 2.1 §4.1): without a limit, act.p would be an unprotected acquisition of the same r, and
    moving r from act.a to it would lower A, `objective_tier_conflict`, which is not what these tests are about."""
    act_p = {**flow("flow.p", "act.p", consumes={"res.r": "10"}, produces={"res.v": produces}), "status": status}
    unknown = {
        "id": "unk.p", "kind": "interval", "question": "fixture", "interval": {"low": "1", "high": "3"},
        "status": "pending", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }
    return model(
        folder, [DAY],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.a"), activity("act.p")],
        [
            regen("regen.r", "res.r", "100", "1d"),
            flow("flow.a", "act.a", consumes={"res.r": "10"}, produces={"res.v": "1"}),
            limit("limit.a", "act.a", "3", "day"),
            {**act_p, **unknowns},
            limit("limit.p", "act.p", "20", "day"),
        ],
        {"fact.v": 0, "fact.r": 0},
        [unknown],
    )


def test_a_pending_flow_with_an_unresolved_unknown_is_left_out_and_every_price_is_provisional(tmp_path, economy):
    """The ACM-PRICE addendum, part 1. act.p's flow is pending: how much v it gives is unk.p, not measured yet. The
    steady state leaves act.p out and lists it, instead of refusing the whole game. By hand, without it: act.a runs
    3 times a day (3 v; its limit is worth 1), and the other 70 r are left over (worth 0). Every price is
    provisional: it was computed without act.p. act.p's limit counts no activity of the program (0). act.a is
    protected (PRICE-2a): A* is its 3 v."""
    _, doc = economy(pending_fixture(tmp_path, "unknown:unk.p", "pending", unknowns=["unk.p"]))

    assert doc["warnings"] == []
    assert all(entry["needed_by"] == ["economy:plan"] for entry in doc["errors"])  # nothing blocks the steady state
    assert steady(doc) == {
        "status": "provisional", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "3",
        **acquisitions("3", [acquirer("act.a", ["res.v"], ["limit.a"], ["res.r"])]),
        "prices": {"res.r": ["0"], "res.v": ["1"]},
        "rule_prices": {"limit.a": "1", "limit.p": "0"},
        "activity_values": {"act.a": ["1"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        "excluded_pending": [{"flow": "flow.p", "activity": "act.p", "unknowns": ["unk.p"]}],
        "price_status": {
            resource: {"status": "provisional", "reasons": ["excluded_pending"]} for resource in ("res.r", "res.v")
        },
    }


def test_a_pending_flow_whose_unknown_is_resolved_is_back_and_nothing_is_provisional_for_it(tmp_path, economy):
    """The same flow once unk.p is measured: act.p gives 2 v, and its flow is confirmed. It is back in the steady
    state, and no price is provisional for its sake. By hand: act.p's 2 v for 10 r beat act.a's 1, so all 100 r go
    to act.p, 20 v a day; r is worth 1/5, act.p 0 and act.a 1 - 2 = -1, and neither limit binds anything (0). Both
    are protected (PRICE-2a); A* is the 20 v of the same optimum."""
    status, doc = economy(pending_fixture(tmp_path, "2", "confirmed"))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "20",
        **acquisitions("20", [
            acquirer("act.a", ["res.v"], ["limit.a"], ["res.r"]), acquirer("act.p", ["res.v"], ["limit.p"], ["res.r"]),
        ]),
        "prices": {"res.r": ["1/5"], "res.v": ["1"]},
        "rule_prices": {"limit.a": "0", "limit.p": "0"},
        "activity_values": {"act.a": ["-1"], "act.p": ["0"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.r", "res.v"),
    }


def test_a_flow_that_reads_an_unknown_and_is_not_pending_still_refuses(tmp_path, economy):
    """A provisional flow that reads unk.p is refused as before (`unknown`), and with it the steady state and the
    plan."""
    folder = pending_fixture(tmp_path, "unknown:unk.p", "provisional")

    status, doc = economy(folder)

    assert status == 1
    assert doc["errors"] == [
        refused(folder, "/rules/3/logic/produces/res.v", "unknown", "economy:plan", "economy:steady")
    ]
    assert doc["data"]["steady"] == doc["data"]["plan"] == {"status": "blocked"}


# #424 ACM-PRICE PRICE-3: income caps, `days` periods, the cycle and its relaxed periods, slot-hours ------------

MONTH = {"id": "month", "unit": "month", "reset_minute": 0, "utc_offset_minutes": 0, "reset_day": 1}
SEASON = {
    "id": "season", "unit": "days", "reset_minute": 0, "utc_offset_minutes": 0, "length": 14,
    "anchor_date": "2026-10-05",
}


def income_fixture(folder: Path, periods: list, per: str, value: str, s_a_day: str) -> Path:
    """s comes at `s_a_day` a day. act.a turns 10 s into 10 r, and an income cap lets act.a bring in at most `value`
    r per `per` (the rest is voided); act.b turns 1 r into 1 v, with no limit."""
    return model(
        folder, periods,
        [resource("res.v", "fact.v"), resource("res.s", "fact.s"), resource("res.r", "fact.r"), activity("act.a"),
         activity("act.b")],
        [
            regen("regen.s", "res.s", s_a_day, "1d"),
            flow("flow.a", "act.a", consumes={"res.s": "10"}, produces={"res.r": "10"}),
            cap("cap.r.income", "res.r", "income", "discard", value=value, per=per, **{"from": ["act.a"]}),
            flow("flow.b", "act.b", consumes={"res.r": "1"}, produces={"res.v": "1"}),
        ],
        {"fact.v": 0, "fact.s": 0, "fact.r": 0},
    )


def test_a_weekly_income_cap_that_holds_prices_what_only_buys_its_income_at_zero(tmp_path, economy):
    """Design test 5. s comes at 10 a day: act.a could bring in 70 r a week, but the weekly income cap keeps 30 of
    it and voids the rest; act.b turns the 30 r into 30 v.

    By hand, a week (7 steps): 30 v. While the cap holds, s, whose only use is income the cap voids, is worth 0 on
    every day: with one s more or one less, act.a can still bring in far more than 30 r. The cap's price is above
    0: one unit more keeps one more r, which act.b turns into 1 v, and one unit less loses it (1). An r that comes
    from outside the capped income is not counted by the cap, and act.b turns it into 1 v (1 on every day). act.a
    is worth its 10 r at 1 less its 10 s at 0: 10, the cap's price for each r it brings in (as an activity at its
    binding limit is worth the limit's price); act.b, the marginal activity, 0. act.b, which acquires v, spends r,
    which has no regen: a conversion (PRICE-2a), and A* is 0."""
    status, doc = economy(income_fixture(tmp_path, [DAY, WEEK], "week", "30", "10"))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = steady(doc)
    assert found["prices"]["res.s"] == ["0"] * 7
    assert Fraction(found["rule_prices"]["cap.r.income"]) > 0
    assert found == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 7, "step_seconds": 86400},
        "objective": "30",
        **acquisitions("0", conversions=[acquirer("act.b", ["res.v"], [], ["res.r"], ["res.r"])]),
        "prices": {"res.r": SEVEN, "res.s": ["0"] * 7, "res.v": SEVEN},
        "rule_prices": {"cap.r.income": "1"},
        "activity_values": {"act.a": ["10"] * 7, "act.b": ["0"] * 7},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.r", "res.s", "res.v"),
    }


def test_a_monthly_income_cap_is_relaxed_by_rate_and_listed(tmp_path, economy):
    """Design test 6. As above, with s at 1000 a day and an income cap of 6957 r a month. The day and the week make
    the cycle 7 steps. A month is 146097/4800 = 48699/1600 days, so the steady state relaxes the cap to its rate,
    6957 × 7 ÷ (146097/4800) = 1600 r a cycle (146097 = 21 × 6957), and lists it in `relaxed`. The plan relaxes it
    over its 7 steps the same way.

    By hand: 1600 v a week (the 7000 s could bring in 7000 r). s is worth 0, r and v 1, as above. One unit more of
    the monthly cap keeps 1600/6957 more r a week, and one less that much less: its price is 1600/6957."""
    status, doc = economy(income_fixture(tmp_path, [DAY, WEEK, MONTH], "month", "6957", "1000"))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    relaxed = [{"rule": "cap.r.income", "per": "month", "days": "48699/1600", "value": "1600"}]
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 7, "step_seconds": 86400},
        "objective": "1600",
        **acquisitions("0", conversions=[acquirer("act.b", ["res.v"], [], ["res.r"], ["res.r"])]),
        "prices": {"res.r": SEVEN, "res.s": ["0"] * 7, "res.v": SEVEN},
        "rule_prices": {"cap.r.income": "1600/6957"},
        "activity_values": {"act.a": ["10"] * 7, "act.b": ["0"] * 7},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": relaxed,
        **settled("res.r", "res.s", "res.v"),
    }
    assert doc["data"]["plan"]["relaxed"] == relaxed


def season_fixture(folder: Path) -> Path:
    """r comes at 10 a day. act.e turns 10 r into 2 v, at most 3 times a 14-day season; act.c turns 10 r into 1 v,
    with no limit."""
    return model(
        folder, [DAY, WEEK, SEASON],
        [resource("res.v", "fact.v"), resource("res.r", "fact.r"), activity("act.c"), activity("act.e")],
        [
            regen("regen.r", "res.r", "10", "1d"),
            flow("flow.e", "act.e", consumes={"res.r": "10"}, produces={"res.v": "2"}),
            limit("limit.e", "act.e", "3", "season"),
            flow("flow.c", "act.c", consumes={"res.r": "10"}, produces={"res.v": "1"}),
        ],
        {"fact.v": 0, "fact.r": 0},
    )


def test_a_14_day_period_makes_a_14_step_cycle_and_counts_per_14_days(tmp_path, economy):
    """Design test 7. A day, a week and a 14-day season: the cycle is their least common multiple, 14 steps, within
    `economy.max_cycle_steps` (28), and act.e's limit counts per 14-day window.

    By hand: 140 r a cycle, 14 executions: act.e 3 (6 v) and act.c 11 (11 v), so 17 v (a weekly window would allow
    act.e 6 and give 20). r is worth 1/10 on every day (act.c is the marginal activity); act.e is worth 2 - 1 = 1
    and act.c 0; the limit one higher moves one execution from act.c to act.e, and one lower back (1).

    With `economy.max_cycle_steps` 7 the season does not fit: the cycle is 7 steps, and the limit is relaxed to
    3 × 7 ÷ 14 = 3/2 a cycle and listed. By hand: 7 executions, act.e 3/2 (3 v) and act.c 11/2: 17/2 v. One more of
    the season's limit is 1/2 more of act.e a cycle: 1/2.

    The tiers (PRICE-2a): act.e is protected, act.c (no limit) a conversion; A* is act.e's 6 v, and 3 relaxed."""
    season_acquirers = (
        [acquirer("act.e", ["res.v"], ["limit.e"], ["res.r"])], [acquirer("act.c", ["res.v"], [], ["res.r"])],
    )
    status, doc = economy(season_fixture(tmp_path / "fits"))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 14, "step_seconds": 86400},
        "objective": "17",
        **acquisitions("6", *season_acquirers),
        "prices": {"res.r": ["1/10"] * 14, "res.v": ["1"] * 14},
        "rule_prices": {"limit.e": "1"},
        "activity_values": {"act.c": ["0"] * 14, "act.e": ["1"] * 14},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.r", "res.v"),
    }

    status, doc = economy(season_fixture(tmp_path / "relaxed"), economy={"max_cycle_steps": 7})

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 7, "step_seconds": 86400},
        "objective": "17/2",
        **acquisitions("3", *season_acquirers),
        "prices": {"res.r": ["1/10"] * 7, "res.v": SEVEN},
        "rule_prices": {"limit.e": "1/2"},
        "activity_values": {"act.c": ["0"] * 7, "act.e": SEVEN},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [{"rule": "limit.e", "per": "season", "days": "14", "value": "3/2"}],
        **settled("res.r", "res.v"),
    }


def test_slot_hours_price_the_activity_they_bound(tmp_path, economy):
    """Design test 8, the slot-hours writing (formats.md, "Slots"). Two slots: slot.s regains 2 an hour (48
    slot-hours a day) under a hard cap of 0, so no hour carries over; act.m holds a slot for 8 hours and gives 1 v.

    By hand, a day: 6 executions use the 48 hours, 6 v. One hour more or less is 1/8 of an execution: a slot-hour
    is worth 1/8, so act.m, bound by the slots, is worth before its hours (1 v) the slot's price times its 8 hours,
    and 0 with them. The cap one higher keeps an hour the cycle cannot use (0); one lower is infeasible (null).
    Nothing overflows at the optimum. That a slot cannot be bound as a pool is test_bindings.py's
    `test_a_slot_resource_cannot_be_bound_as_a_pool`. act.m's slot is time (only it takes from the slot), but no
    limit counts it: a conversion (PRICE-2a), and A* is 0."""
    folder = model(
        tmp_path, [DAY],
        [resource("res.v", "fact.v"), resource("slot.s", "fact.slot"), activity("act.m")],
        [
            regen("regen.slot.s", "slot.s", "2", "1h"),
            cap("cap.slot.s", "slot.s", "hard", "discard", value="0"),
            flow("flow.m", "act.m", consumes={"slot.s": "8"}, produces={"res.v": "1"}),
        ],
        {"fact.v": 0, "fact.slot": 0},
    )

    status, doc = economy(folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = steady(doc)
    assert Fraction(found["prices"]["slot.s"][0]) * 8 == Fraction(found["prices"]["res.v"][0]) * 1
    assert found == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "6",
        **acquisitions("0", conversions=[acquirer("act.m", ["res.v"], [], ["slot.s"])]),
        "prices": {"res.v": ["1"], "slot.s": ["1/8"]},
        "rule_prices": {"cap.slot.s": {"low": "0", "high": None}},
        "activity_values": {"act.m": ["0"]},
        "losses": {"overflow": {"slot.s": "0"}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.v", "slot.s"),
    }


def test_steady_only_reports_the_steady_state_alone(tmp_path, economy):
    """The coordinator's addition to PRICE-3 (`--steady-only`): a rule regression needs the steady state alone.
    The cyclic fixture with a state sample that lacks r's reading. The steady state never reads the account state,
    but the plan opens at the readings: without the flag the plan is blocked (`state_reading_missing`, needed by the
    plan only) and the exit status is 1; with it, only the steady state is computed and reported, and it exits 0."""
    folder = cyclic_fixture(tmp_path)
    state = json.loads((folder / "state.json").read_text(encoding="utf-8"))
    del state["readings"]["fact.r"]
    write(folder / "state.json", state)
    rules = str(folder / "ours" / "model" / "rules" / "economy.json")

    status, doc = economy(folder)

    assert status == CODES["state_reading_missing"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "state_reading_missing", "detail": {"path": rules, "fact": "fact.r"}, "ref": "/entities/1/reading",
        "needed_by": ["economy:plan"],
    }]
    assert (steady(doc), doc["data"]["plan"]) == (CYCLIC_STEADY, {"status": "blocked"})

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert "plan" not in doc["data"]
    assert steady(doc) == CYCLIC_STEADY


# #424 ACM-PRICE PRICE-2a: the objective, its goal sinks and the three solves (addendum 2.1 §4.4) ----------------


def model_path(folder: Path) -> str:
    return str(folder / "ours" / "model" / "model.json")


def spend_fixture(folder: Path, point: str) -> Path:
    """g is the main goal, worth 1 a unit in its sink. act.get brings in 10 g a day (no input, at most once a day:
    protected); act.research turns 1 g into 1 m; m also comes at 1 a day. The secondary goal: the demand row `m`
    takes 1 m for 1 point, a point worth `point`, at most 100 a day."""
    return model(
        folder, [DAY],
        [resource("res.g", "fact.g"), resource("res.m", "fact.m"), activity("act.get"), activity("act.research")],
        [
            flow("flow.get", "act.get", produces={"res.g": "10"}),
            limit("limit.get", "act.get", "1", "day"),
            flow("flow.research", "act.research", consumes={"res.g": "1"}, produces={"res.m": "1"}),
            regen("regen.m", "res.m", "1", "1d"),
        ],
        {"fact.g": 0, "fact.m": 0},
        goals=[goal("goal.g", "res.g", "1"), training(point, "1d")], demand=[("m", "100", "1", "0")],
        costs=[("m", {"res.m": "1"})],
    )


GET = acquirer("act.get", ["res.g"], ["limit.get"], [])


@pytest.mark.parametrize(
    ("point", "expected"),
    [
        pytest.param("2", {
            "objective": "22", "prices": {"res.g": ["2"], "res.m": ["2"]}, "rule_prices": {"limit.get": "20"},
            "activity_values": {"act.get": ["20"], "act.research": ["0"]},
        }, id="research"),
        pytest.param("0.5", {
            "objective": "21/2", "prices": {"res.g": ["1"], "res.m": ["1/2"]}, "rule_prices": {"limit.get": "10"},
            "activity_values": {"act.get": ["10"], "act.research": ["-1/2"]},
        }, id="sink"),
    ],
)
def test_a_main_goal_item_may_be_spent_and_its_acquisition_stays_whole(tmp_path, economy, point, expected):
    """Addendum 2.1 §4.4, test 1. The use side of a main goal item is compared inside V; A counts what act.get
    brings in (10 g a day, A* = 10), and spending g never lowers it.

    By hand, a point worth 2: a g turned into an m is worth 2 in the demand row, more than its 1 in g's sink, so all
    10 g go to act.research, and the 11 m (with the one that comes) to the demand row: V0 = 22, V* = 22 (act.get
    still brings in its 10). One more g or m is 2 more, one less 2 less; act.get is worth 10 × 2 = 20 (its limit one
    higher brings 10 g more, 20), act.research 2 - 2 = 0. A point worth 1/2: research loses (1/2 < 1), so the 10 g go
    to the sink and the one m that comes to the demand row: V0 = V* = 21/2. g is worth 1, m 1/2 (one more is a half
    point more, one less a half point less), act.get 10, act.research 1/2 - 1 = -1/2. Both are priced."""
    status, doc = economy(spend_fixture(tmp_path, point), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": expected["objective"],
        **acquisitions("10", [GET]),
        "prices": expected["prices"],
        "rule_prices": expected["rule_prices"],
        "activity_values": expected["activity_values"],
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.g", "res.m"),
    }


def shared_fixture(folder: Path, point: str, *, most: str = "", income: str = "") -> Path:
    """s comes at 100 a day. act.get turns 10 s into 10 g (the main goal, worth 1), counted by a daily income cap of
    `income`, or else limited to `most` a day; act.train turns 10 s into 1 m; the demand row `m` takes 1 m for 1
    point, a point worth `point`, at most 100 a day."""
    bound = (
        cap("cap.g.income", "res.g", "income", "discard", value=income, per="day", **{"from": ["act.get"]}) if income
        else limit("limit.get", "act.get", most, "day")
    )
    return model(
        folder, [DAY],
        [resource("res.g", "fact.g"), resource("res.s", "fact.s"), resource("res.m", "fact.m"), activity("act.get"),
         activity("act.train")],
        [
            regen("regen.s", "res.s", "100", "1d"),
            flow("flow.get", "act.get", consumes={"res.s": "10"}, produces={"res.g": "10"}),
            bound,
            flow("flow.train", "act.train", consumes={"res.s": "10"}, produces={"res.m": "1"}),
        ],
        {"fact.g": 0, "fact.s": 0, "fact.m": 0},
        goals=[goal("goal.g", "res.g", "1"), training(point, "1d")], demand=[("m", "100", "1", "0")],
        costs=[("m", {"res.m": "1"})],
    )


def test_the_protected_acquisition_keeps_its_shared_input(tmp_path, economy):
    """Addendum 2.1 §4.4, test 2. act.get is protected (s has a regen; the income cap counts it): A is what it keeps
    of its g, at most 50 a day, so A* = 50.

    By hand, a point worth 20: 10 s are worth 20 in act.train and 10 in act.get, so V alone puts all 100 s into
    act.train, V0 = 10 × 20 = 200. Holding A at 50 keeps 50 s for act.get (50) and gives the other 50 to act.train
    (100): V* = 150 < 200. The steady state is refused (`objective_tier_conflict`). It lists act.get, the protected
    acquisition whose income the training takes (§4.2: what of A the optimum of S3 lowers), and P.

    A point worth 1/2: act.get fills its cap (5 executions, 50 g) and act.train takes the other 50 s (5 m, 5/2):
    V0 = V* = 105/2, and act.get is in the optimum. s is worth 1/20 (a tenth of act.train), g 1, m 1/2; the cap one
    higher keeps one more g for a tenth of act.get's s, 1 - 1/20 = 19/20; act.get is worth 10 - 10/20 = 19/2 and
    act.train 1/2 - 1/2 = 0."""
    folder = shared_fixture(tmp_path / "conflict", "20", income="50")

    status, doc = economy(folder, "--steady-only")

    assert status == CODES["objective_tier_conflict"]["exit"] == 1
    assert doc["errors"] == [{
        "code": "objective_tier_conflict",
        "detail": {
            "path": model_path(folder), "acquisition": "50", "value": "150", "unconstrained": "200",
            "activities": ["act.get"], "protected": ["act.get"],
        },
        "needed_by": ["economy:steady"],
    }]
    assert doc["data"]["steady"] == {"status": "blocked"}

    status, doc = economy(shared_fixture(tmp_path / "kept", "0.5", income="50"), "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "105/2",
        **acquisitions("50", [acquirer("act.get", ["res.g"], ["cap.g.income"], ["res.s"])]),
        "prices": {"res.g": ["1"], "res.m": ["1/2"], "res.s": ["1/20"]},
        "rule_prices": {"cap.g.income": "19/20"},
        "activity_values": {"act.get": ["19/2"], "act.train": ["0"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.g", "res.m", "res.s"),
    }


def test_a_demand_row_with_a_main_column_counts_once_in_a_and_once_in_v(tmp_path, economy):
    """Addendum 2.1 §4.4, test 3. No main goal; the demand row `unlock` takes 10 m for 2 points (a point worth 1) and
    5 value units for the main goal later (`main`), at most once a day. s comes at 20 a day, and act.make turns 10 s
    into 10 m.

    By hand: A is only the row's `main`, so S1 runs the row once: A* = 5 (S1 uses it). V counts it once, with its
    points: 2 × 1 + 5 = 7 = V0 = V* (not 5 + 7). m is plentiful (20 a day for 10), so m and s are worth 0, and so is
    act.make. The neutral reference state has every mode at its top tier, so the research channel's `main` is 0 for
    now: this tests only how it counts."""
    folder = model(
        tmp_path, [DAY],
        [resource("res.s", "fact.s"), resource("res.m", "fact.m"), activity("act.make")],
        [
            regen("regen.s", "res.s", "20", "1d"),
            flow("flow.make", "act.make", consumes={"res.s": "10"}, produces={"res.m": "10"}),
        ],
        {"fact.s": 0, "fact.m": 0},
        goals=[training("1", "1d")], demand=[("unlock", "1", "2", "5")], costs=[("unlock", {"res.m": "10"})],
    )

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "7",
        **acquisitions("5"),
        "prices": {"res.m": ["0"], "res.s": ["0"]},
        "rule_prices": {},
        "activity_values": {"act.make": ["0"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.m", "res.s"),
    }


def test_nothing_is_valued_twice(tmp_path, economy):
    """Addendum 2.1 §4.4, test 4 (its second point, an activity's direct reward in ACM-PRICE-E, is not this slice).

    A conversion is not in A. act.login brings in 100 d a day (no input, once a day: protected); act.buy turns 10 d
    into 1 g. d and g are both main goals, worth 1 and 15. act.buy spends d, which has no regen: a conversion. By
    hand: V alone buys 10 g with all the d, V0 = 150 > 100; A counts act.login's 100 d only, A* = 100, which buying
    does not lower, so V* = 150. In V each is at its own price: d 3/2 (a tenth of a g), g 15; act.buy is worth
    15 - 10 × 3/2 = 0, act.login 100 × 3/2 = 150, as is its limit.

    A main goal item that a demand row also costs has one price, from its use at the margin. act.get brings in 20 g a
    day (10 an execution, at most twice); the demand row `use` takes 1 g for a point worth 3, at most 4 a day. By
    hand: 4 g to the row (12) and 16 to g's sink (16), V0 = V* = 28, A* = 20. One g more or less goes to or comes from
    the sink (the row is at its bound): g is worth 1, not 1 + 3; act.get is worth 10, and so is its limit."""
    folder = model(
        tmp_path / "conversion", [DAY],
        [resource("res.d", "fact.d"), resource("res.g", "fact.g"), activity("act.login"), activity("act.buy")],
        [
            flow("flow.login", "act.login", produces={"res.d": "100"}),
            limit("limit.login", "act.login", "1", "day"),
            flow("flow.buy", "act.buy", consumes={"res.d": "10"}, produces={"res.g": "1"}),
        ],
        {"fact.d": 0, "fact.g": 0},
        goals=[goal("goal.d", "res.d", "1"), goal("goal.g", "res.g", "15")],
    )

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "150",
        **acquisitions(
            "100", [acquirer("act.login", ["res.d"], ["limit.login"], [])],
            [acquirer("act.buy", ["res.g"], [], ["res.d"], ["res.d"])],
        ),
        "prices": {"res.d": ["3/2"], "res.g": ["15"]},
        "rule_prices": {"limit.login": "150"},
        "activity_values": {"act.buy": ["0"], "act.login": ["150"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.d", "res.g"),
    }

    folder = model(
        tmp_path / "cost", [DAY],
        [resource("res.g", "fact.g"), activity("act.get")],
        [flow("flow.get", "act.get", produces={"res.g": "10"}), limit("limit.get", "act.get", "2", "day")],
        {"fact.g": 0},
        goals=[goal("goal.g", "res.g", "1"), training("3", "1d")], demand=[("use", "4", "1", "0")],
        costs=[("use", {"res.g": "1"})],
    )

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "28",
        **acquisitions("20", [GET]),
        "prices": {"res.g": ["1"]},
        "rule_prices": {"limit.get": "10"},
        "activity_values": {"act.get": ["10"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.g"),
    }


def test_an_activity_as_good_as_the_acquisition_by_value_alone_is_a_tie(tmp_path, economy):
    """Addendum 2.1 §4.4, test 5. As test 2, but act.get may run 10 times a day (all of s) and a point is worth 10:
    10 s are worth 10 in act.get and 10 in act.train.

    By hand: V0 = 100 however s is split, A* = 100 (all s to act.get), V* = 100: no conflict. Some optimum of S3 runs
    act.train, no optimum of S2 does: `objective_tier_tie` for act.train, a warning. Priced from S3: s is worth 1
    (a tenth of either), g 1, m 10, both activities 0, and act.get's limit 0 (one more finds no s; one less moves
    10 s to act.train, worth the same)."""
    folder = shared_fixture(tmp_path, "10", most="10")

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == [
        {"code": "objective_tier_tie", "detail": {"path": model_path(folder), "activity": "act.train"}}
    ]
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "100",
        **acquisitions("100", [acquirer("act.get", ["res.g"], ["limit.get"], ["res.s"])]),
        "prices": {"res.g": ["1"], "res.m": ["10"], "res.s": ["1"]},
        "rule_prices": {"limit.get": "0"},
        "activity_values": {"act.get": ["0"], "act.train": ["0"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "relaxed": [],
        **settled("res.g", "res.m", "res.s"),
    }


def test_a_solution_a_millionth_short_of_a_star_is_refused_exactly(tmp_path, economy, monkeypatch):
    """Addendum 2.1 §4.4, test 6 (§4.3: A ≥ A* is compared exactly, with no tolerance). act.d brings in 1 v an
    execution, at most 100 a day: A* = 100. S2's optimum passes the exact check of S2's program, whose row
    `floor:acquisition:0` holds A at A*. The same solution with act.d and v's sink a millionth lower (every other row
    still holds) is refused by it, and so is a solver's answer that stands for it (`lp_numeric_check_failed`, stage
    `value`, the stage of S2): nothing is rounded into place."""
    captured = []
    tiers = economy_flow._Solver.tiers

    def recorded(solver: Any) -> Any:
        found = tiers(solver)
        captured.append((solver, found))
        return found

    monkeypatch.setattr(economy_flow._Solver, "tiers", recorded)
    folder = model(
        tmp_path, [DAY],
        [resource("res.v", "fact.v"), activity("act.d")],
        [flow("flow.d", "act.d", produces={"res.v": "1"}), limit("limit.d", "act.d", "100", "day")],
        {"fact.v": 0},
    )

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert (steady(doc)["objective"], steady(doc)["acquisition"]) == ("100", "100")
    [(solver, found)] = captured
    program = economy_flow._program(solver.model, added=found.floors)  # S2's program
    x = dict(found.second.x)
    assert economy_flow._violation(program, x) is None
    short = Fraction(1, 10**6)
    x[("x", "act.d", 0)] -= short
    x[("g", "goal.v", 0)] -= short
    problem = {"constraint": "floor:acquisition:0", "found": "-99999999/1000000", "bound": "<=-100"}
    assert economy_flow._violation(program, x) == problem

    monkeypatch.setattr(economy_flow, "_vertex", lambda program, floats: None)  # a vertex its rows do not determine
    columns = sorted(program.variables)
    with pytest.raises(Failure) as refused:
        solver.exact(program, columns, [float(x[key]) for key in columns], "value")
    assert refused.value.errors == [{
        "code": "lp_numeric_check_failed",
        "detail": {"path": model_path(folder), "solve": "steady", "stage": "value", **problem},
    }]


def test_a_preference_in_an_objective_is_misplaced(tmp_path, economy):
    """Addendum 2.1 §4.4, test 7 (ACM-PRICE 2.2 (7), the change to D-2): an objective reads no preference."""
    folder = model(
        tmp_path, [DAY], [resource("res.v", "fact.v")], [], {"fact.v": 0},
        goals=[goal("goal.v", "res.v", "pref:plan.weight")],
    )

    status, doc = economy(folder, "--steady-only")

    assert status == CODES["rule_pref_misplaced"]["exit"] == 1
    assert [(entry["code"], entry["ref"], entry["detail"]) for entry in doc["errors"]] == [(
        "rule_pref_misplaced", "/rules/0/logic/value",
        {
            "path": str(folder / "ours" / "model" / "rules" / "economy.json"), "id": "goal.v", "kind": "objective",
            "reference": "pref:plan.weight", "position": 0,
        },
    )]


def test_the_protected_acquisitions_are_decided_criterion_by_criterion(tmp_path, economy):
    """Addendum 2.1 §4.4, test 8: who is in P (§4.1), one activity a case. g is the main goal, worth 1.
    - act.free: no input, at most once a day (10 g): protected;
    - act.capped: 10 s (s comes at 100 a day) for 10 g, counted by a daily income cap of 30: protected;
    - act.unbounded: 10 s for 5 g, with neither a limit nor an income cap: a conversion (no bound);
    - act.convert: 2 h (no regen; act.side makes it from slot.shared) for 3 g, at most 100 a day: a conversion
      (it spends h);
    - act.slot_own: 2 hours of slot.own, which only it takes from, for 1 g, at most 100 a day: protected;
    - act.slot_shared: 2 hours of slot.shared, which act.side also takes, for 1 g, at most 100 a day: a conversion
      (it spends the shared slot);
    - act.zero: 0 h (an amount of 0 is no input) for 1 g, once a day: protected.
    Each slot gives 24 hours a day.

    By hand: V alone runs act.free (10), act.capped to its cap (3 executions, 30), act.unbounded on the other 70 s
    (35), act.side on all 24 hours of slot.shared and act.convert on its 24 h (12 executions, 36: 3/2 an hour,
    against act.slot_shared's 1/2), act.slot_own 12 times (12) and act.zero once (1): V0 = 124. A* = 10 + 30 + 12 + 1
    = 53, which that optimum keeps: V* = 124, and no tie."""

    def hard_zero(rule_id: str, subject: str) -> dict[str, Any]:
        return cap(rule_id, subject, "hard", "discard", value="0")

    folder = model(
        tmp_path, [DAY],
        [resource("res.g", "fact.g"), resource("res.s", "fact.s"), resource("res.h", "fact.h"),
         resource("slot.own", "fact.own"), resource("slot.shared", "fact.shared"), activity("act.free"),
         activity("act.capped"), activity("act.unbounded"), activity("act.side"), activity("act.convert"),
         activity("act.slot_own"), activity("act.slot_shared"), activity("act.zero")],
        [
            regen("regen.s", "res.s", "100", "1d"),
            regen("regen.own", "slot.own", "1", "1h"),
            hard_zero("cap.own", "slot.own"),
            regen("regen.shared", "slot.shared", "1", "1h"),
            hard_zero("cap.shared", "slot.shared"),
            flow("flow.free", "act.free", produces={"res.g": "10"}),
            limit("limit.free", "act.free", "1", "day"),
            flow("flow.capped", "act.capped", consumes={"res.s": "10"}, produces={"res.g": "10"}),
            cap("cap.g.income", "res.g", "income", "discard", value="30", per="day", **{"from": ["act.capped"]}),
            flow("flow.unbounded", "act.unbounded", consumes={"res.s": "10"}, produces={"res.g": "5"}),
            flow("flow.side", "act.side", consumes={"slot.shared": "1"}, produces={"res.h": "1"}),
            flow("flow.convert", "act.convert", consumes={"res.h": "2"}, produces={"res.g": "3"}),
            limit("limit.convert", "act.convert", "100", "day"),
            flow("flow.slot_own", "act.slot_own", consumes={"slot.own": "2"}, produces={"res.g": "1"}),
            limit("limit.slot_own", "act.slot_own", "100", "day"),
            flow("flow.slot_shared", "act.slot_shared", consumes={"slot.shared": "2"}, produces={"res.g": "1"}),
            limit("limit.slot_shared", "act.slot_shared", "100", "day"),
            flow("flow.zero", "act.zero", consumes={"res.h": "0"}, produces={"res.g": "1"}),
            limit("limit.zero", "act.zero", "1", "day"),
        ],
        {"fact.g": 0, "fact.s": 0, "fact.h": 0, "fact.own": 0, "fact.shared": 0},
        goals=[goal("goal.g", "res.g", "1")],
    )

    status, doc = economy(folder, "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    # HiGHS's marginals are a cross-check only (lp_dual_disagrees), not what this test is about; no other warning.
    assert [entry for entry in doc["warnings"] if entry["code"] != "lp_dual_disagrees"] == []
    found = steady(doc)
    assert (found["status"], found["objective"]) == ("confirmed", "124")
    assert {key: found[key] for key in ("acquisition", "protected", "conversions")} == acquisitions(
        "53",
        [
            acquirer("act.capped", ["res.g"], ["cap.g.income"], ["res.s"]),
            acquirer("act.free", ["res.g"], ["limit.free"], []),
            acquirer("act.slot_own", ["res.g"], ["limit.slot_own"], ["slot.own"]),
            acquirer("act.zero", ["res.g"], ["limit.zero"], []),
        ],
        [
            acquirer("act.convert", ["res.g"], ["limit.convert"], ["res.h"], ["res.h"]),
            acquirer("act.slot_shared", ["res.g"], ["limit.slot_shared"], ["slot.shared"], ["slot.shared"]),
            acquirer("act.unbounded", ["res.g"], [], ["res.s"]),
        ],
    )


def crafting_fixture(folder: Path, point: str) -> Path:
    """The reduced steady state of addendum 2.1 §4.5 (its `ak_tier_check.py`), in neutral ids, a day a step and a week
    a cycle. g is the main goal, worth 1. s comes at 240 a day (1680 a week). act.raid turns 25 s into 350 g, its
    income capped at 1800 g a week; the daily task brings in 100 g a day and the weekly one 500 g a week, without
    input. act.stage turns 36 s into 10000 c. slot.f has 4 slots (96 hours a day, 672 a week, none kept): act.ingot
    makes an ingot in 1.2 hours, act.trade turns it into 500 c; act.shard makes a shard of 1 hour and 1600 c, and
    act.order turns 2 shards into 20 g. c stands for the secondary goal: the demand row `c` takes 1 c for 1 point, a
    point worth `point`, far from its bound (a million a week)."""
    return model(
        folder, [DAY, WEEK],
        [resource("res.g", "fact.g"), resource("res.s", "fact.s"), resource("res.c", "fact.c"),
         resource("res.ingot", "fact.ingot"), resource("res.shard", "fact.shard"), resource("slot.f", "fact.f"),
         activity("act.raid"), activity("act.daily"), activity("act.weekly"), activity("act.stage"),
         activity("act.ingot"), activity("act.trade"), activity("act.shard"), activity("act.order")],
        [
            regen("regen.s", "res.s", "240", "1d"),
            regen("regen.f", "slot.f", "4", "1h"),
            cap("cap.f", "slot.f", "hard", "discard", value="0"),
            flow("flow.raid", "act.raid", consumes={"res.s": "25"}, produces={"res.g": "350"}),
            cap("cap.g.income", "res.g", "income", "discard", value="1800", per="week", **{"from": ["act.raid"]}),
            flow("flow.daily", "act.daily", produces={"res.g": "100"}),
            limit("limit.daily", "act.daily", "1", "day"),
            flow("flow.weekly", "act.weekly", produces={"res.g": "500"}),
            limit("limit.weekly", "act.weekly", "1", "week"),
            flow("flow.stage", "act.stage", consumes={"res.s": "36"}, produces={"res.c": "10000"}),
            flow("flow.ingot", "act.ingot", consumes={"slot.f": "1.2"}, produces={"res.ingot": "1"}),
            flow("flow.trade", "act.trade", consumes={"res.ingot": "1"}, produces={"res.c": "500"}),
            flow("flow.shard", "act.shard", consumes={"slot.f": "1", "res.c": "1600"}, produces={"res.shard": "1"}),
            flow("flow.order", "act.order", consumes={"res.shard": "2"}, produces={"res.g": "20"}),
        ],
        {fact: 0 for fact in ("fact.g", "fact.s", "fact.c", "fact.ingot", "fact.shard", "fact.f")},
        goals=[goal("goal.g", "res.g", "1"), training(point, "7d")], demand=[("c", "1000000", "1", "0")],
        costs=[("c", {"res.c": "1"})],
    )


CRAFTING_PROTECTED = [
    acquirer("act.daily", ["res.g"], ["limit.daily"], []),
    acquirer("act.raid", ["res.g"], ["cap.g.income"], ["res.s"]),
    acquirer("act.weekly", ["res.g"], ["limit.weekly"], []),
]


@pytest.mark.parametrize(
    ("point", "objective"),
    [pytest.param("1 / 150", "487600/63", id="1/150"), pytest.param("1 / 1000", "5527000/847", id="1/1000")],
)
def test_a_crafting_conversion_no_longer_conflicts(tmp_path, economy, point, objective):
    """Addendum 2.1 §4.4, test 9, the regression of §4.5. act.order spends shards (no regen): a conversion, compared
    inside V. P is the raid (s has a regen; the income cap counts it) and the two tasks (no input, a limit), so A* =
    1800 + 700 + 500 = 3000 at every point value.

    By hand, a point worth 1/150 (per s: the raid 14, the stage 10000/150/36 = 50/27; per slot-hour: an ingot traded
    500/150/1.2 = 25/9, a shard ordered 10 - 1600/150 = -2/3): the raid fills its cap (36/7 executions, 900/7 s), the
    stage takes the other 10860/7 s (905/21 executions) and the factory makes 560 ingots, so 905/21 × 10000 + 560 ×
    500 = 14930000/21 c go to the demand row: V0 = 3000 + 14930000/21/150 = 487600/63. That optimum keeps A at 3000,
    so V* = V0 and the steady state is priced. (Had the order counted in A, as in addendum 2's criterion, S1 would
    have taken the factory for shards, A* = 5527000/847, and V* = 5527000/847 < V0: the old conflict.)

    A point worth 1/1000 (an ingot 5/12 an hour, a shard 42/5): the raid fills its cap, the stage takes the rest of
    s, and the factory splits its hours so that its c and the stage's pay for the shards: 298600/847 hours of shards
    and 270584/847 of ingots, no c left for the demand row (its dual, 3/605 a c, is above 1/1000). V0 = 3000 + 10 ×
    298600/847 = 5527000/847, and A at 3000: no conflict either way."""
    status, doc = economy(crafting_fixture(tmp_path, point), "--steady-only")

    assert (status, doc["errors"]) == (0, [])
    # HiGHS's marginals are a cross-check only (lp_dual_disagrees), not what this test is about; no other warning.
    assert [entry for entry in doc["warnings"] if entry["code"] != "lp_dual_disagrees"] == []
    found = steady(doc)
    assert (found["status"], found["objective"]) == ("confirmed", objective)
    assert {key: found[key] for key in ("acquisition", "protected", "conversions")} == acquisitions(
        "3000", CRAFTING_PROTECTED, [acquirer("act.order", ["res.g"], [], ["res.shard"], ["res.shard"])]
    )


def test_a_point_worth_more_than_the_raid_still_conflicts_and_lists_only_the_raid(tmp_path, economy):
    """Addendum 2.1 §4.4, test 9, the third row of §4.5: a point worth 1/10 makes the stage worth 10000/10/36 = 250/9
    an s, above the raid's 14. V alone gives the stage all 1680 s (140/3 executions) and the factory 560 ingots:
    1400000/3 + 280000 = 2240000/3 c, V0 = 1200 + 224000/3 = 227600/3. Holding A at 3000 keeps the raid at its cap:
    905/21 stage executions and the same ingots, V* = 3000 + (9050000/21 + 280000)/10 = 1556000/21 < V0. The steady
    state is refused (`objective_tier_conflict`), and of what A holds only the raid is lower in the optimum of S3 (the
    tasks bring in the same 1200; the order is no longer in A)."""
    folder = crafting_fixture(tmp_path, "1 / 10")

    status, doc = economy(folder, "--steady-only")

    assert status == 1
    assert doc["errors"] == [{
        "code": "objective_tier_conflict",
        "detail": {
            "path": model_path(folder), "acquisition": "3000", "value": "1556000/21", "unconstrained": "227600/3",
            "activities": ["act.raid"], "protected": ["act.daily", "act.raid", "act.weekly"],
        },
        "needed_by": ["economy:steady"],
    }]
    assert doc["data"]["steady"] == {"status": "blocked"}
