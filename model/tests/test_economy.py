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


def model(
    folder: Path, periods: list, entities: list, rules: list, readings: dict[str, int], unknowns: list | None = None
) -> Path:
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
        "schema_version": "acmodel.rules.v1", "entities": entities, "rules": rules, "unknowns": unknowns or [],
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


def settled(*resources: str) -> dict[str, Any]:
    """What a confirmed steady state that left no pending flow out shows of it (#424 ACM-PRICE addendum, part 1):
    no `excluded_pending`, and each price confirmed, for no reason."""
    return {
        "excluded_pending": [],
        "price_status": {resource: {"status": "confirmed", "reasons": []} for resource in resources},
    }


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
    loses all 6."""
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
        "prices": {"res.r": ["1/10"] * 7, "res.v": SEVEN},
        "rule_prices": {"cap.r.natural": "1/10", "limit.d": {"low": "1", "high": "6"}},
        "activity_values": {"act.d": ["0"] * 7},
        "losses": {"overflow": {}, "lost_production": {"res.r": "10"}},
        "excluded": [],
        **settled("res.r", "res.v"),
    }


def pending_fixture(folder: Path, produces: str, status: str, **unknowns: list[str]) -> Path:
    """r comes at 100 a day. act.a turns 10 r into 1 v, at most 3 times a day; act.p turns 10 r into `produces` v,
    with no limit, by a flow of `status` (naming `unknowns`). unk.p, how much v one act.p gives, is an interval from
    1 to 3 that no measurement has narrowed yet."""
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
        ],
        {"fact.v": 0, "fact.r": 0},
        [unknown],
    )


def test_a_pending_flow_with_an_unresolved_unknown_is_left_out_and_every_price_is_provisional(tmp_path, economy):
    """The ACM-PRICE addendum, part 1. act.p's flow is pending: how much v it gives is unk.p, not measured yet. The
    steady state leaves act.p out and lists it, instead of refusing the whole game. By hand, without it: act.a runs
    3 times a day (3 v; its limit is worth 1), and the other 70 r are left over (worth 0). Every price is
    provisional: it was computed without act.p."""
    _, doc = economy(pending_fixture(tmp_path, "unknown:unk.p", "pending", unknowns=["unk.p"]))

    assert doc["warnings"] == []
    assert all(entry["needed_by"] == ["economy:plan"] for entry in doc["errors"])  # nothing blocks the steady state
    assert steady(doc) == {
        "status": "provisional", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "3",
        "prices": {"res.r": ["0"], "res.v": ["1"]},
        "rule_prices": {"limit.a": "1"},
        "activity_values": {"act.a": ["1"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
        "excluded_pending": [{"flow": "flow.p", "activity": "act.p", "unknowns": ["unk.p"]}],
        "price_status": {
            resource: {"status": "provisional", "reasons": ["excluded_pending"]} for resource in ("res.r", "res.v")
        },
    }


def test_a_pending_flow_whose_unknown_is_resolved_is_back_and_nothing_is_provisional_for_it(tmp_path, economy):
    """The same flow once unk.p is measured: act.p gives 2 v, and its flow is confirmed. It is back in the steady
    state, and no price is provisional for its sake. By hand: act.p's 2 v for 10 r beat act.a's 1, so all 100 r go
    to act.p, 20 v a day; r is worth 1/5, act.p 0 and act.a 1 - 2 = -1, and the limit of act.a binds nothing (0)."""
    status, doc = economy(pending_fixture(tmp_path, "2", "confirmed"))

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert steady(doc) == {
        "status": "confirmed", "per_account": False,
        "cycle": {"steps": 1, "step_seconds": 86400},
        "objective": "20",
        "prices": {"res.r": ["1/5"], "res.v": ["1"]},
        "rule_prices": {"limit.a": "0"},
        "activity_values": {"act.a": ["-1"], "act.p": ["0"]},
        "losses": {"overflow": {}, "lost_production": {}},
        "excluded": [],
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
