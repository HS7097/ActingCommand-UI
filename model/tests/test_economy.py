"""PY-10: the economy (`acmodel economy`), its steady state and plan solved by scipy's HiGHS, on hand-solvable
neutral fixtures. Every expected number below is worked out by hand in the fixture's docstring; CI runs these
tests on Windows and on Ubuntu, so both legs must give the same exact values."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli

CODES = json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
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


def model(folder: Path, periods: list, entities: list, rules: list, readings: dict[str, int]) -> Path:
    """A neutral repository folder with one rule file, and its account state; returns the folder."""
    base = folder / "ours" / "model"
    write(base / "model.json", {
        "schema_version": "acmodel.model.v1", "model_id": "economy",
        "files": {"rules": ["rules/economy.json"], "data": []}, "periods": periods, "numeraire": "res.v",
        "engines": [],
    })
    write(base / "sources.json", {"schema_version": "acmodel.sources.v1", "sources": []})
    write(base / "exports.json", {"schema_version": "acmodel.exports.v1", "thresholds": [], "tables": [], "scoring": []})
    write(base / "rules" / "economy.json", {
        "schema_version": "acmodel.rules.v1", "entities": entities, "rules": rules, "unknowns": [],
    })
    write(folder / "state.json", {
        "schema_version": "acmodel.state.v1", "model_id": "economy", "label": "acct-1",
        "readings": {fact: {"value": value, "observed_at_unix_ms": 0, "source": "fixture"} for fact, value in readings.items()},
        "entities": {}, "cleared": [],
    })
    return folder


@pytest.fixture
def economy(make_system, app_dir, write_config):
    """`acmodel economy` on a fixture folder, with the given configuration keys: (exit status, envelope). The
    envelope is one acmodel.cli.v1 line whose errors and warnings agree with contracts/codes.json."""

    def run(folder: Path, **sections: Any) -> tuple[int, dict[str, Any]]:
        if sections:
            write_config({"schema_version": "acmodel.config.v1", **sections})
        system = make_system(prefix=app_dir / "venv")
        argv = ["economy", str(folder / "ours" / "model"), "--state", str(folder / "state.json")]
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
    depends on what the store holds."""
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
    "prices": {"res.c": ["1/10"], "res.s": ["0"], "res.v": ["1"]},
    "rule_prices": {
        "cap.f.storage": "1/10", "limit.a": "1", "limit.collect": {"low": "8", "high": "70"},
    },
    "activity_values": {"act.a": ["1"], "act.b": ["0"], "act.collect": None},
    "losses": {"overflow": {}, "lost_production": {"fac.f": "80"}},
    "excluded": [],
}


def cyclic_fixture(folder: Path) -> Path:
    """r comes as 70 at each weekly reset into a hard cap of 50; act.d uses 10 r for 1 v, at most once a day.

    By hand, a week of seven daily steps (the model has a week period): starting empty, the reset step takes
    70, act.d uses 10, the stock stops at the cap (50) and 10 overflow. The six other days have 50 left: five
    executions. Six v a week, and the week ends with the stock it began with. One more r on the reset step only
    overflows (worth 0); on any other day it gives a tenth more of an execution (1/10), and one less takes a
    tenth away. The cap one higher keeps one more r for those days: 1/10, act.d's value per unit of r, the
    marginal activity. A daily limit of 2 uses 20 on the reset step, so nothing overflows (+1); a limit of 0
    loses all 6. act.d is worth 1 on the reset step and 0 on the others."""
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
    "prices": {"res.r": ["0"] + ["1/10"] * 6, "res.v": SEVEN},
    "rule_prices": {"cap.r.hard": "1/10", "limit.d": {"low": "1", "high": "6"}},
    "activity_values": {"act.d": ["1"] + ["0"] * 6},
    "losses": {"overflow": {"res.r": "10"}, "lost_production": {}},
    "excluded": [],
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
    3 - 1 = 2, act.b 1 and act.c 0."""
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
        "prices": {"res.r": ["1/10"] * 7, "res.v": SEVEN},
        "rule_prices": {"limit.a.day": {"low": "0", "high": "3"}, "limit.ab.week": "2", "limit.b.day": "0"},
        "activity_values": {"act.a": ["2"] * 7, "act.b": SEVEN, "act.c": ["0"] * 7},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
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
    limit one higher finds no r (0); one lower loses 2."""
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
        "prices": {"res.r": [{"low": "0", "high": "1/5"}], "res.v": ["1"]},
        "rule_prices": {"limit.a": {"low": "0", "high": "2"}},
        "activity_values": {"act.a": [{"low": "0", "high": "2"}]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
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


def test_the_node_limit_gives_search_budget_exhausted_unproven(tmp_path, economy):
    """A one-day plan that is a knapsack with four resources: 30 activities, each at most once a week, each
    using some of every resource for some v. The steady state has all the resources it needs (each comes at a
    seventh of the week's need, and more), so they are worth 0 there and the plan's final stocks of them count
    nothing. The plan has only about half of each: choosing the activities is a multi-dimensional knapsack,
    which a node limit of one cannot prove optimal."""
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

    assert (status, doc["errors"]) == (0, [])
    [warning] = doc["warnings"]
    assert warning["code"] == "search_budget_exhausted"
    assert {key: warning["detail"][key] for key in ("solve", "node_limit")} == {"solve": "plan", "node_limit": 1}
    assert steady(doc)["prices"]["res.r0"] == ["0"] * 7
    plan = doc["data"]["plan"]
    assert (plan["status"], plan["steps"], plan["proven"]) == ("confirmed", 1, False)
