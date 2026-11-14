"""#464 W1, the model's writer of the schedule's values: `acmodel export values` (`out/values.json` and the value provenance
files), the value bindings of `bindings check` and `bindings apply` (the fixed value and its origin, written in place or
inserted in the file's own style), the origin guard, and `apply` writing each file as its part and renaming it only when
the file did not change meanwhile (contracts/formats.md, "`acmodel export values`" and "Bindings").

Every fixture is neutral: four pools, two derived rates, one activity with one flow, and an `out/prices.json` written
here as `export prices` writes it (the values read the prices, they never compute a steady state). With κ = 1000 the
prices in milli are: pool.a 1/250 → 4, pool.b 1/10 → 100, pool.c 1/20 → 50 (provisional, accepted), pool.d pending.
Each docstring works the values out by hand."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
from fractions import Fraction
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli
from acmodel.bindings import commands
from acmodel.check.checker import Checker
from acmodel.check.files import load
from acmodel.codes import EXIT_STATUS
from acmodel.core.versions import sha256 as doc_sha256
from acmodel.core.versions import versions
from test_economy import CODES, DAY, WEEK, activity, flow, model, regen, resource, rule, write

MODEL = ("ours", "model")
UNIT = 1000
POOLS = ("pool.a", "pool.b", "pool.c", "pool.d")
WORD = "benefit_fixed_value"
EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"


@pytest.fixture
def acmodel(make_system, app_dir):
    """One command in the venv: (exit status, envelope). The envelope is one acmodel.cli.v1 line whose errors and
    warnings agree with contracts/codes.json (each code of its category, with its values), and the exit status the
    first error's."""

    def run(*argv: Any) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main([str(arg) for arg in argv], system)
        [line] = system.stdout.getvalue().splitlines()
        doc = json.loads(line)
        assert doc["ok"] is (doc["errors"] == [])
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


def base(folder: Path) -> Path:
    return folder.joinpath(*MODEL)


def edit(path: Path, change: Callable[[Any], None]) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def model_versions(folder: Path) -> dict[str, str]:
    """The model's versions, as every export records them."""
    return versions(Checker(load(folder), folder), folder)


def export_values(acmodel, folder: Path) -> tuple[int, dict[str, Any]]:
    return acmodel("export", "values", base(folder), "--state", folder / "state.json")


def bindings(acmodel, folder: Path, command: str) -> tuple[int, dict[str, Any]]:
    return acmodel("bindings", command, base(folder), "--state", folder / "state.json")


# The fixture ----------------------------------------------------------------------------------------------------------

# rate.a (324) is confirmed; rate.b (96) is provisional, and exports.json accepts it.
RATE_B = {"output": "rule:rate.b", "ref": "R-1", "until": "a device check"}
PRICE_C = {"output": "price:pool.c", "ref": "P-1", "until": "a price check"}
FIXTURE_RULES = [
    rule("rate.a", "derive", "pool.a", value="324"),
    {**rule("rate.b", "derive", "pool.c", value="96"), "status": "provisional"},
    flow("flow.make", "act.make", produces={"pool.a": "400", "pool.c": "15"}, consumes={"pool.b": "2"}),
]


def price(pool: str, value: str, *, status: str = "confirmed", pending: tuple[str, ...] = (),
          acceptance: dict[str, Any] | None = None) -> dict[str, Any]:
    """A pool's entry of prices.json, as `export prices` writes it."""
    return {
        "pool": pool, "class": "covered", "status": status, "reasons": [], "component": "part." + pool, "price": value,
        "milli": None if value == "pending" else str(Fraction(value) * UNIT), "pending": list(pending), "joint": None,
        "degenerate": False, "varies_by_step": False, "impact": None, "flips": [], "binds": "0" * 64,
        "acceptance": acceptance, "provenance": f"out/provenance/prices/{pool}.json",
    }


# pool.c's price is delivered under this acceptance of exports.json (its second), which prices.json records as it was.
ACCEPTANCE_C = {
    "output": "price:pool.c",
    "accept_provisional": {"ref": "P-1", "until": "a price check", "binds": "0" * 64, "scope": "game", "flips": []},
}
ACCEPTED_C = {"entry": "/acceptances/1", **ACCEPTANCE_C}
PRICES = [
    price("pool.a", "1/250"), price("pool.b", "1/10"), price("pool.c", "1/20", status="provisional", acceptance=ACCEPTED_C),
    price("pool.d", "pending", status="provisional", pending=("not_accepted",)),
]


def write_prices(folder: Path, entries: list[dict[str, Any]] = PRICES, **other: str) -> None:
    """out/prices.json of the model as it is now, as `export prices` would deliver it (`other` versions instead, by
    member)."""
    write(base(folder) / "out" / "prices.json", {
        "schema_version": "acmodel.prices.v2", "model_id": "economy", "versions": {**model_versions(base(folder)), **other},
        "shared": {}, "unit_milli": UNIT, "prices": entries, "slots": [], "estimates": [], "impact": [], "counts": {},
    })


def values_repo(folder: Path, values: list[dict[str, Any]], *extra: dict[str, Any], periods: list | None = None) -> Path:
    """The neutral repository: the model (with `extra` rules, and `periods`, by default a day and a week), exports.json
    with κ, the values and the acceptances of rate.b and of pool.c's price, an empty bindings.json, and the prices."""
    entities = [resource(pool, "fact." + pool) for pool in POOLS] + [activity("act.make")]
    model(folder, periods or [DAY, WEEK], entities, [*FIXTURE_RULES, *extra], {"fact." + pool: 0 for pool in POOLS},
          goals=[])
    edit(base(folder) / "exports.json", lambda doc: doc.update(
        prices={"unit_milli": UNIT}, values=values,
        acceptances=[
            {"output": RATE_B["output"], "accept_provisional": {"ref": RATE_B["ref"], "until": RATE_B["until"]}},
            ACCEPTANCE_C,
        ],
    ))
    write(base(folder) / "bindings.json", {"schema_version": "acmodel.bindings.v1", "bindings": [], "fixed": []})
    write_prices(folder)
    return folder


def term(resource_id: str, amount: str, times: str, per: str) -> dict[str, Any]:
    return {"resource": resource_id, "amount": amount, "times": times, "per": per, "ref": "fixture"}


def runs(count: str, per: str) -> dict[str, Any]:
    return {"count": count, "per": per, "ref": "fixture"}


# 324 an hour for 17 hours, over nine runs a day: 612 of pool.a a run, 2448 milli. For 7 hours in one run: 2268, 9072.
DAY_RUN = {"id": "val.a.day", "terms": [term("pool.a", "rule:rate.a", "17", "day")], "runs": runs("9", "day"),
           "round": "half_even"}
NIGHT_RUN = {"id": "val.a.night", "terms": [term("pool.a", "rule:rate.a", "7", "day")], "runs": runs("1", "day"),
             "round": "half_even"}
# 96 an hour for 24 hours over two runs: 1152 of pool.c, 57600 milli; rate.b and pool.c's price are both accepted.
COLLECT = {"id": "val.b.collect", "terms": [term("pool.c", "rule:rate.b", "24", "day")], "runs": runs("2", "day"),
           "round": "half_even"}
# pool.d's price is pending.
WAITING = {"id": "val.d.pending", "terms": [term("pool.d", "5", "1", "run")], "runs": runs("1", "day"), "round": "floor"}


def pool(name: str, brought: str = "0", taken: str = "0", *, paid: bool = False, excluded: bool = False) -> dict:
    return {"pool": name, "in": brought, "out": taken, "expected": False, "paid_by_pool": paid, "excluded": excluded}


def entry(definition: dict[str, Any], status: str, milli: int | None, yields: list[dict[str, Any]] | None, *,
          acceptances: tuple[dict[str, str], ...] = (), pending: tuple[dict[str, Any], ...] = (),
          exact: str | None = None) -> dict[str, Any]:
    """A value's entry of values.json (`exact`, the exact value, when it is no integer)."""
    return {
        "id": definition["id"], "status": status, "fixed_value_milli": milli,
        "exact_milli": exact if exact is not None else (None if milli is None else str(milli)), "round": definition["round"],
        "acceptances": list(acceptances), "pending": list(pending), "partial": bool(definition.get("exclude")),
        "exclude": definition.get("exclude", []), "cost_paid_by_pool": definition.get("cost_paid_by_pool", []),
        "runs": definition["runs"], "yields": yields,
        "provenance": f"out/provenance/values/{definition['id']}.json",  # a pending value's too
    }


def delivered(folder: Path) -> dict[str, Any]:
    return json.loads((base(folder) / "out" / "values.json").read_text(encoding="utf-8"))


def provenance(folder: Path, value_id: str) -> dict[str, Any]:
    path = base(folder) / "out" / "provenance" / "values" / f"{value_id}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def result(definition: dict[str, Any], *args: Any, **kwargs: Any) -> dict[str, Any]:
    """A provenance document's `result`: the value's entry without `provenance`."""
    return {key: value for key, value in entry(definition, *args, **kwargs).items() if key != "provenance"}


# export values --------------------------------------------------------------------------------------------------------


def test_export_values_lists_every_value_with_its_number_or_why_it_waits(tmp_path, acmodel):
    """Day 2448 and night 9072 are confirmed; collect 57600 rests on two acceptances (rate.b's and pool.c's price), so it
    is provisional and lists both; pool.d's price is pending, so that value is pending with why, and its provenance file
    says so, of the current versions, with the price it read. values.json records the model's versions and the sha256 of
    the bytes of prices.json and exports.json."""
    folder = values_repo(tmp_path, [DAY_RUN, NIGHT_RUN, COLLECT, WAITING])
    out = base(folder) / "out"

    status, doc = export_values(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["counts"] == {"confirmed": 2, "provisional": 1, "pending": 1, "partial": 0}
    names = ["val.a.day", "val.a.night", "val.b.collect", "val.d.pending"]
    assert doc["data"]["written"] == sorted(
        [str(out / "values.json")] + [str(out / "provenance" / "values" / f"{name}.json") for name in names]
    )
    raw = (out / "values.json").read_bytes()
    written = delivered(folder)
    assert raw == (json.dumps(written, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
    assert {key: written[key] for key in ("schema_version", "model_id", "versions", "prices", "exports", "unit_milli")} == {
        "schema_version": "acmodel.values.v1",
        "model_id": "economy",
        "versions": model_versions(base(folder)),
        "prices": {"file": "out/prices.json", "sha256": sha(out / "prices.json"), "versions": model_versions(base(folder))},
        "exports": {"file": "exports.json", "sha256": sha(base(folder) / "exports.json")},
        "unit_milli": UNIT,
    }
    assert written["counts"] == doc["data"]["counts"]
    assert written["values"] == [
        entry(DAY_RUN, "confirmed", 2448, [pool("pool.a", "612")]),
        entry(NIGHT_RUN, "confirmed", 9072, [pool("pool.a", "2268")]),
        entry(COLLECT, "provisional", 57600, [pool("pool.c", "1152")], acceptances=(PRICE_C, RATE_B)),
        entry(WAITING, "pending", None, [pool("pool.d", "5")],
              pending=({"reason": "price_pending", "item": "pool.d", "pending": ["not_accepted"]},)),
    ]
    waiting = provenance(folder, "val.d.pending")
    assert (waiting["output"], waiting["versions"], waiting["result"]) == (
        "value:val.d.pending", model_versions(base(folder)), {
            key: value for key, value in written["values"][3].items() if key != "provenance"})
    assert waiting["result"]["status"] == "pending"
    assert waiting["nodes"] == {
        "value:val.d.pending": {
            "node": "value", "id": "val.d.pending", "file": "exports.json", "ref": "/values/3",
            "sha256": doc_sha256(WAITING), "reads": ["price:pool.d"],
        },
        "price:pool.d": {
            "node": "price", "resource": "pool.d", "file": "out/prices.json", "sha256": sha(out / "prices.json"),
            "class": "covered", "status": "provisional", "price": "pending", "milli": None,
            "provenance": "out/provenance/prices/pool.d.json", "reads": [],
        },
    }
    assert "acceptances" not in waiting

    day = json.loads((out / "provenance" / "values" / "val.a.day.json").read_text(encoding="utf-8"))
    assert (day["schema_version"], day["output"], day["versions"]) == (
        "acmodel.provenance.v1", "value:val.a.day", model_versions(base(folder)))
    assert day["result"] == {key: value for key, value in written["values"][0].items() if key != "provenance"}
    assert set(day["nodes"]) == {"value:val.a.day", "rule:rate.a", "price:pool.a"}
    assert day["nodes"]["value:val.a.day"] == {
        "node": "value", "id": "val.a.day", "file": "exports.json", "ref": "/values/0", "sha256": doc_sha256(DAY_RUN),
        "reads": ["price:pool.a", "rule:rate.a"],
    }
    assert day["nodes"]["price:pool.a"] == {
        "node": "price", "resource": "pool.a", "file": "out/prices.json", "sha256": sha(out / "prices.json"),
        "class": "covered", "status": "confirmed", "price": "1/250", "milli": "4",
        "provenance": "out/provenance/prices/pool.a.json", "reads": [],
    }
    assert "acceptances" not in day
    collect = json.loads((out / "provenance" / "values" / "val.b.collect.json").read_text(encoding="utf-8"))
    assert collect["acceptances"] == [PRICE_C, RATE_B]


# One run of act.make brings in 400 pool.a and 15 pool.c and takes out 2 pool.b. pool.b is the rule's declared cost, which
# the Runtime takes off itself; pool.c is left out with its reason. 400 x 4 = 1600, confirmed and partial.
MAKE = {
    "id": "val.make", "terms": [{"activity": "act.make", "times": "1", "per": "run"}], "runs": runs("1", "week"),
    "cost_paid_by_pool": ["pool.b"], "exclude": [{"resource": "pool.c", "reason": "its price waits", "ref": "R-2"}],
    "round": "half_even",
}
# The same, whole: 1600 + 15 x 50 = 2350, provisional by pool.c's price.
MAKE_WHOLE = {"id": "val.make.whole", "terms": MAKE["terms"], "runs": runs("1", "week"), "cost_paid_by_pool": ["pool.b"],
              "round": "half_even"}
# And without the declared cost: 2350 - 2 x 100 = 2150; once a week over one run a week is one execution a run.
MAKE_SPENT = {"id": "val.make.spent", "terms": [{"activity": "act.make", "times": "1", "per": "week"}],
              "runs": runs("1", "week"), "round": "half_even"}


def test_an_activity_counts_what_its_flows_bring_and_take_and_the_declared_cost_and_the_left_out_pools(tmp_path, acmodel):
    folder = values_repo(tmp_path, [MAKE, MAKE_WHOLE, MAKE_SPENT])

    status, doc = export_values(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["counts"] == {"confirmed": 1, "provisional": 2, "pending": 0, "partial": 1}
    assert delivered(folder)["values"] == [
        entry(MAKE, "confirmed", 1600, [
            pool("pool.a", "400"), pool("pool.b", taken="2", paid=True), pool("pool.c", "15", excluded=True)]),
        entry(MAKE_SPENT, "provisional", 2150, [pool("pool.a", "400"), pool("pool.b", taken="2"), pool("pool.c", "15")],
              acceptances=(PRICE_C,)),
        entry(MAKE_WHOLE, "provisional", 2350, [
            pool("pool.a", "400"), pool("pool.b", taken="2", paid=True), pool("pool.c", "15")], acceptances=(PRICE_C,)),
    ]
    make = json.loads((base(folder) / "out" / "provenance" / "values" / "val.make.json").read_text(encoding="utf-8"))
    assert make["nodes"]["value:val.make"]["reads"] == [
        "flow:flow.make:consumes:pool.b", "flow:flow.make:produces:pool.a", "flow:flow.make:produces:pool.c",
        "price:pool.a",
    ]


def test_a_value_below_one_is_refused_and_nothing_is_written(tmp_path, acmodel):
    """1/10000 pool.a a run at 4 milli is 1/2500 milli: below 1 once rounded down. A value of 0 is no value."""
    tiny = {"id": "val.tiny", "terms": [term("pool.a", "1/10000", "1", "run")], "runs": runs("1", "day"), "round": "floor"}
    folder = values_repo(tmp_path, [DAY_RUN, tiny])

    status, doc = export_values(acmodel, folder)

    assert (status, doc["data"]) == (5, {})
    assert doc["errors"] == [{
        "code": "value_not_positive",
        "detail": {"path": str(base(folder) / "exports.json"), "value": "val.tiny", "milli": "0", "exact": "1/2500"},
        "ref": "/values/1",
    }]
    assert not (base(folder) / "out" / "values.json").exists()
    assert not (base(folder) / "out" / "provenance" / "values").exists()


def test_prices_of_other_versions_or_another_unit_are_an_error_not_a_warning(tmp_path, acmodel):
    folder = values_repo(tmp_path, [DAY_RUN])
    prices = str(base(folder) / "out" / "prices.json")
    edit(base(folder) / "rules" / "economy.json", lambda doc: doc["rules"][0].update(note="edited"))

    status, doc = export_values(acmodel, folder)

    assert (status, doc["data"]) == (5, {})
    assert doc["errors"] == [{
        "code": "export_refused",
        "detail": {"path": prices, "output": "values", "reason": "prices_stale", "value": "rules"},
        "ref": "/versions/rules",
    }]

    write_prices(folder)
    edit(base(folder) / "exports.json", lambda doc: doc.update(prices={"unit_milli": 500}))

    status, doc = export_values(acmodel, folder)

    assert status == 5
    assert doc["errors"] == [{
        "code": "export_refused",
        "detail": {"path": prices, "output": "values", "reason": "prices_stale", "value": "unit_milli"},
        "ref": "/unit_milli",
    }]
    assert not (base(folder) / "out" / "values.json").exists()


def test_a_quantity_that_read_the_account_state_is_refused(tmp_path, acmodel):
    reading = rule("rate.read", "derive", "pool.a", value="read:fact.pool.a")
    value = {"id": "val.read", "terms": [term("pool.a", "rule:rate.read", "1", "run")], "runs": runs("1", "day"),
             "round": "floor"}
    folder = values_repo(tmp_path, [value], reading)

    status, doc = export_values(acmodel, folder)

    assert status == 5
    assert doc["errors"] == [{
        "code": "export_refused",
        "detail": {"path": str(base(folder) / "exports.json"), "output": "value:val.read", "reason": "per_account"},
        "ref": "/values/0/terms/0/amount",
    }]


def test_rule_of_a_rule_without_a_value_is_named_not_guessed(tmp_path, acmodel):
    """A regen has no value that `rule:` reads (formats.md, "Rule values"), so its amount is no quantity."""
    value = {"id": "val.regen", "terms": [term("pool.a", "rule:regen.a", "1", "run")], "runs": runs("1", "day"),
             "round": "floor"}
    folder = values_repo(tmp_path, [value], regen("regen.a", "pool.a", "1", "1h"))

    status, doc = export_values(acmodel, folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "rule_reference_missing",
        "detail": {"path": str(base(folder) / "exports.json"), "target": "regen.a",
                   "expected": ["rule.derive", "rule.cap", "rule.limit", "rule.random"], "found": "rule.regen"},
        "ref": "/values/0/terms/0/amount",
    }]


# Three periods of the model's own, by ids that name no unit: a week, a month and a three-day period.
CYCLE_W = {"id": "cycle.w", "unit": "week", "reset_minute": 0, "utc_offset_minutes": 0, "reset_weekday": "mon"}
CYCLE_M = {"id": "cycle.m", "unit": "month", "reset_minute": 0, "utc_offset_minutes": 0, "reset_day": 1}
CYCLE_D = {"id": "cycle.d", "unit": "days", "reset_minute": 0, "utc_offset_minutes": 0, "length": 3,
           "anchor_date": "2026-01-05"}
# 10 pool.a a week, one run a day: 10/7 a run, 40/7 milli, 6 rounded.
WEEKLY = {"id": "val.weekly", "terms": [term("pool.a", "10", "1", "cycle.w")], "runs": runs("1", "day"),
          "round": "half_even"}
# 1000 pool.a a month (146097/4800 days), one run a day: 1000 x 4800/146097 = 1600000/48699 a run, 6400000/48699 milli
# (131.42...), 131 rounded.
MONTHLY = {"id": "val.monthly", "terms": [term("pool.a", "1000", "1", "cycle.m")], "runs": runs("1", "day"),
           "round": "half_even"}
# 10 pool.a every three days, two runs a week: a run is 7/2 days, so 10 x (7/2)/3 = 35/3 a run, 140/3 milli, 47 rounded.
EVERY_THREE = {"id": "val.every_three", "terms": [term("pool.a", "10", "1", "cycle.d")], "runs": runs("2", "cycle.w"),
               "round": "half_even"}


def test_a_term_counts_per_run_by_the_days_of_the_models_own_periods(tmp_path, acmodel):
    """A week is 7 days, a month 146097/4800, a `days` period its length, as the model defines them (not by the ids):
    each value is exact before it is rounded once."""
    folder = values_repo(tmp_path, [WEEKLY, MONTHLY, EVERY_THREE], periods=[DAY, CYCLE_W, CYCLE_M, CYCLE_D])

    status, doc = export_values(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert delivered(folder)["values"] == [
        entry(EVERY_THREE, "confirmed", 47, [pool("pool.a", "35/3")], exact="140/3"),
        entry(MONTHLY, "confirmed", 131, [pool("pool.a", "1600000/48699")], exact="6400000/48699"),
        entry(WEEKLY, "confirmed", 6, [pool("pool.a", "10/7")], exact="40/7"),
    ]
    assert Fraction(6400000, 48699) == Fraction(1000 * 4800, 146097) * 4


def test_a_value_that_becomes_pending_has_its_provenance_file_written_again(tmp_path, acmodel):
    """val.a.day is exported; then the model changes and pool.a's price is pending in the prices exported again. The
    export writes the value's provenance file again, of the current versions, saying it is pending and why: no file of
    the earlier export stays behind with other versions (the resource side's check of `out/` would find it stale), nor
    one that holds a number that no longer holds."""
    folder = values_repo(tmp_path, [DAY_RUN])
    path = base(folder) / "out" / "provenance" / "values" / "val.a.day.json"
    assert export_values(acmodel, folder)[0] == 0
    before = provenance(folder, "val.a.day")
    assert before["result"]["status"] == "confirmed"

    edit(base(folder) / "rules" / "economy.json", lambda doc: doc["rules"][0].update(note="edited"))
    write_prices(folder, [price("pool.a", "pending", status="provisional", pending=("not_accepted",)), *PRICES[1:]])

    status, doc = export_values(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert str(path) in doc["data"]["written"]
    after = provenance(folder, "val.a.day")
    waits = {"reason": "price_pending", "item": "pool.a", "pending": ["not_accepted"]}
    assert after["versions"] == model_versions(base(folder)) != before["versions"]
    assert after["result"] == result(DAY_RUN, "pending", None, [pool("pool.a", "612")], pending=(waits,))
    assert set(after["nodes"]) == {"value:val.a.day", "rule:rate.a", "price:pool.a"}
    assert (after["nodes"]["price:pool.a"]["price"], after["nodes"]["price:pool.a"]["milli"]) == ("pending", None)


def test_a_provisional_quantity_not_accepted_or_a_pool_without_a_price_is_pending(tmp_path, acmodel):
    """rate.c is provisional and exports.json does not accept it; prices.json lists no pool.b. Each value is pending, with
    its provenance file."""
    unaccepted = {"id": "val.unaccepted", "terms": [term("pool.a", "rule:rate.c", "1", "run")], "runs": runs("1", "day"),
                  "round": "floor"}
    unpriced = {"id": "val.unpriced", "terms": [term("pool.b", "3", "1", "run")], "runs": runs("1", "day"),
                "round": "floor"}
    folder = values_repo(tmp_path, [unaccepted, unpriced], {**rule("rate.c", "derive", "pool.a", value="5"),
                                                             "status": "provisional"})
    write_prices(folder, [listed for listed in PRICES if listed["pool"] != "pool.b"])

    status, doc = export_values(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["counts"] == {"confirmed": 0, "provisional": 0, "pending": 2, "partial": 0}
    assert delivered(folder)["values"] == [
        entry(unaccepted, "pending", None, None, pending=({"reason": "provisional", "item": "rule:rate.c"},)),
        entry(unpriced, "pending", None, [pool("pool.b", "3")], pending=({"reason": "price_missing", "item": "pool.b"},)),
    ]
    assert provenance(folder, "val.unpriced")["nodes"]["value:val.unpriced"]["reads"] == []


def test_a_value_beyond_the_safe_range_is_refused_and_nothing_is_written(tmp_path, acmodel):
    """2^53 pool.a a run at 4 milli is 2^55 milli, beyond 2^53 - 1."""
    huge = {"id": "val.huge", "terms": [term("pool.a", str(2 ** 53), "1", "run")], "runs": runs("1", "day"),
            "round": "floor"}
    folder = values_repo(tmp_path, [DAY_RUN, huge])

    status, doc = export_values(acmodel, folder)

    assert (status, doc["data"]) == (5, {})
    assert doc["errors"] == [{
        "code": "export_refused",
        "detail": {"path": str(base(folder) / "exports.json"), "output": "value:val.huge", "reason": "out_of_range",
                   "value": str(2 ** 55)},
        "ref": "/values/1",
    }]
    assert not (base(folder) / "out" / "values.json").exists()
    assert not (base(folder) / "out" / "provenance" / "values").exists()


def test_how_often_a_term_happens_and_how_often_the_rule_runs_are_above_zero(tmp_path, acmodel):
    """`times` 0 or negative, and `runs.count` 0 or negative, are refused by the reader of exports.json: a term that
    happens no times is no term, and a negative count would turn what comes in into what goes out."""
    values = [
        {"id": "val.never", "terms": [term("pool.a", "1", "0", "run")], "runs": runs("1", "day"), "round": "floor"},
        {"id": "val.back", "terms": [term("pool.a", "1", "-2", "day")], "runs": runs("1", "day"), "round": "floor"},
        {"id": "val.no_runs", "terms": [term("pool.a", "1", "1", "run")], "runs": runs("0/3", "day"), "round": "floor"},
        {"id": "val.minus", "terms": [term("pool.a", "1", "1", "run")], "runs": runs("-1", "day"), "round": "floor"},
    ]
    folder = values_repo(tmp_path, values)
    exports = str(base(folder) / "exports.json")

    status, doc = export_values(acmodel, folder)

    assert status == 1
    assert sorted(doc["errors"], key=lambda found: found["ref"]) == [
        {"code": "model_file_invalid",
         "detail": {"reason": "value_invalid", "path": exports, "format": "acmodel.exports.v1", "literal": literal_},
         "ref": ref}
        for ref, literal_ in (
            ("/values/0/terms/0/times", "0"), ("/values/1/terms/0/times", "-2"), ("/values/2/runs/count", "0/3"),
            ("/values/3/runs/count", "-1"),
        )
    ]


# bindings: a value's literal and its origin -----------------------------------------------------------------------------

DAY_RULE = '{"id":"rule.a.day","package_id":"pkg.a","priority":5}'
NIGHT_RULE = '{"id":"rule.a.night","package_id":"pkg.a","fixed_value_milli":7000,"priority":5}'
COLLECT_RULE = '{"id":"rule.b.collect","package_id":"pkg.b","fixed_value_milli":3,"priority":2}'
DAY_DONE = '{"id":"rule.a.day","package_id":"pkg.a","fixed_value_milli":2448,"fixed_value_origin":"model","priority":5}'
NIGHT_DONE = '{"id":"rule.a.night","package_id":"pkg.a","fixed_value_milli":9072,"fixed_value_origin":"model","priority":5}'
COLLECT_DONE = (
    '{"id":"rule.b.collect","package_id":"pkg.b","fixed_value_milli":57600,"fixed_value_origin":"model","priority":2}'
)


def one_line(rules: list[str], *, requires: str = f'["{WORD}"]', pools: str = "[]") -> bytes:
    """A schedule.json of one rule a line and no spaces, in LF."""
    return (
        '{"schema_version":"actingcommand.schedule.v1","requires":' + requires + ',"rules":[\n' + ",\n".join(rules)
        + "\n],\"pools\":" + pools + "}\n"
    ).encode("utf-8")


def bind(folder: Path, *pairs: tuple[str, str]) -> None:
    """Value bindings, each a value of exports.json and the rule of schedule.json whose fixed value it is."""
    entries = [{"value": value, "file": "schedule.json", "rule_id": rule_id, "path": "/fixed_value_milli"}
               for value, rule_id in pairs]
    edit(base(folder) / "bindings.json", lambda doc: doc.update(bindings=entries))


def literal(index: int, value: str, rule_id: str) -> dict[str, Any]:
    return {"entry": f"/bindings/{index}", "value": value, "file": "schedule.json", "rule_id": rule_id,
            "path": "/fixed_value_milli"}


def test_apply_writes_each_value_and_its_origin_and_changes_no_other_byte(tmp_path, acmodel, monkeypatch):
    """Before: rule.a.day has neither member, rule.a.night a stale value and no origin, rule.b.collect a hand-written
    value and no origin. `check` names each difference; `apply` inserts the missing members after `package_id` and the
    value (in the file's own style: no spaces), replaces the stale values in place, writes the file through its part, and
    every other byte stays. Then `check` passes and lists each value's status and the acceptances it rests on."""
    folder = values_repo(tmp_path, [DAY_RUN, NIGHT_RUN, COLLECT])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([DAY_RULE, NIGHT_RULE, COLLECT_RULE]))
    bind(folder, ("val.a.day", "rule.a.day"), ("val.a.night", "rule.a.night"), ("val.b.collect", "rule.b.collect"))
    where = str(schedule)

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6

    def detail(index: int, value: str, rule_id: str, **values: str) -> dict[str, Any]:
        return {"path": where, "entry": f"/bindings/{index}", "value": value, "rule_id": rule_id, **values}

    assert doc["errors"] == [
        {"code": "binding_mismatch", "detail": detail(0, "val.a.day", "rule.a.day", reason="value", expected="2448"),
         "ref": "/rules/0"},
        {"code": "origin_missing", "detail": detail(0, "val.a.day", "rule.a.day"), "ref": "/rules/0"},
        {"code": "binding_mismatch",
         "detail": detail(1, "val.a.night", "rule.a.night", reason="value", expected="9072", found="7000"),
         "ref": "/rules/1/fixed_value_milli"},
        {"code": "origin_missing", "detail": detail(1, "val.a.night", "rule.a.night"), "ref": "/rules/1"},
        {"code": "binding_mismatch",
         "detail": detail(2, "val.b.collect", "rule.b.collect", reason="value", expected="57600", found="3"),
         "ref": "/rules/2/fixed_value_milli"},
        {"code": "origin_missing", "detail": detail(2, "val.b.collect", "rule.b.collect"), "ref": "/rules/2"},
    ]

    renamed: list[tuple[str, bool, str]] = []

    def rename(part: Path, path: Path) -> None:
        renamed.append((part.name, part.exists(), path.name))
        os.replace(part, path)

    monkeypatch.setattr(commands, "_replace", rename)
    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert schedule.read_bytes() == one_line([DAY_DONE, NIGHT_DONE, COLLECT_DONE])
    assert renamed == [("schedule.json.part", True, "schedule.json")]
    assert not (folder / "schedule.json.part").exists()
    assert doc["data"]["written"] == [where]
    model_origin = {"from": None, "to": "model"}
    assert doc["data"]["changed"] == [
        {**literal(0, "val.a.day", "rule.a.day"), "from": None, "to": 2448, "ratio": None, "ratio_flagged": False,
         "origin": model_origin, "inserted": ["fixed_value_milli", "fixed_value_origin"]},
        {**literal(1, "val.a.night", "rule.a.night"), "from": 7000, "to": 9072, "ratio": str(Fraction(9072, 7000)),
         "ratio_flagged": False, "origin": model_origin, "inserted": ["fixed_value_origin"]},
        {**literal(2, "val.b.collect", "rule.b.collect"), "from": 3, "to": 57600, "ratio": "19200",
         "ratio_flagged": True, "origin": model_origin, "inserted": ["fixed_value_origin"]},
    ]

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    shown = {"literal": None, "origin": "model", "partial": False, "exclude": []}
    assert doc["data"]["bindings"] == [
        {**literal(0, "val.a.day", "rule.a.day"), **shown, "literal": 2448, "milli": 2448, "status": "confirmed",
         "acceptances": []},
        {**literal(1, "val.a.night", "rule.a.night"), **shown, "literal": 9072, "milli": 9072, "status": "confirmed",
         "acceptances": []},
        {**literal(2, "val.b.collect", "rule.b.collect"), **shown, "literal": 57600, "milli": 57600,
         "status": "provisional", "acceptances": [PRICE_C, RATE_B]},
    ]
    assert doc["data"]["values"] is None  # no out/values.json

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"]["changed"], doc["data"]["written"]) == (0, [], [])


# The same two rules over several lines, indented, in CRLF.
INDENTED = [
    "{",
    '  "schema_version": "actingcommand.schedule.v1",',
    f'  "requires": ["fact_reference", "{WORD}"],',
    '  "rules": [',
    "    {",
    '      "id": "rule.a.day",',
    '      "package_id": "pkg.a",',
    "@DAY@",
    '      "priority": 5',
    "    },",
    "    {",
    '      "id": "rule.a.night",',
    '      "package_id": "pkg.a",',
    '      "fixed_value_milli": @NIGHT@,',
    "@ORIGIN@",
    '      "priority": 5',
    "    }",
    "  ]",
    "}",
]


def indented(day: list[str], night: str, origin: list[str]) -> bytes:
    lines: list[str] = []
    for line in INDENTED:
        if line == "@DAY@":
            lines.extend(day)
        elif line == "@ORIGIN@":
            lines.extend(origin)
        else:
            lines.append(line.replace("@NIGHT@", night))
    return ("\r\n".join(lines) + "\r\n").encode("utf-8")


def test_apply_keeps_crlf_and_writes_inserted_members_with_the_files_own_line_ends_and_indent(tmp_path, acmodel):
    folder = values_repo(tmp_path, [DAY_RUN, NIGHT_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(indented([], "7000", []))
    bind(folder, ("val.a.day", "rule.a.day"), ("val.a.night", "rule.a.night"))
    origin = ['      "fixed_value_origin": "model",']
    expected = indented(['      "fixed_value_milli": 2448,', *origin], "9072", origin)
    assert expected.count(b"\n") == expected.count(b"\r\n")

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert schedule.read_bytes() == expected
    assert bindings(acmodel, folder, "check")[0] == 0


def test_a_pending_value_refuses_the_whole_apply(tmp_path, acmodel):
    """val.d.pending waits for pool.d's price: `apply` refuses, and writes not even the other binding's value."""
    folder = values_repo(tmp_path, [DAY_RUN, NIGHT_RUN, COLLECT, WAITING])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([DAY_RULE, '{"id":"rule.d","package_id":"pkg.d","priority":1}']))
    before = schedule.read_bytes()
    bind(folder, ("val.a.day", "rule.a.day"), ("val.d.pending", "rule.d"))
    refused = {
        "code": "export_refused",
        "detail": {"path": str(base(folder) / "exports.json"), "output": "value:val.d.pending", "reason": "pending",
                   "value": "price_pending:pool.d"},
        "ref": "/values/3",
    }

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"], doc["errors"]) == (5, {}, [refused])
    assert schedule.read_bytes() == before
    assert not (folder / "schedule.json.part").exists()

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"][0]) == (5, refused)


def test_a_model_origin_no_binding_names_is_refused(tmp_path, acmodel):
    """A value written by hand and marked `model`, and a pool's valuation marked `model`: only the model's export writes
    model, so each is `origin_unbound`, in check and in apply."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line(
        ['{"id":"rule.x","package_id":"pkg.x","fixed_value_milli":5,"fixed_value_origin":"model","priority":1}'],
        pools='[{"id":"pool.a","valuation":{"base_weight_milli":4},"valuation_origin":"model"}]',
    ))
    before = schedule.read_bytes()
    expected = [
        {"code": "origin_unbound", "detail": {"path": str(schedule), "rule_id": "rule.x", "member": "fixed_value_origin"},
         "ref": "/rules/0/fixed_value_origin"},
        {"code": "origin_unbound", "detail": {"path": str(schedule), "pool_id": "pool.a", "member": "valuation_origin"},
         "ref": "/pools/0/valuation_origin"},
    ]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, folder, command)

        assert (status, doc["errors"]) == (6, expected)
    assert schedule.read_bytes() == before

    bind(folder, ("val.a.day", "rule.x"))
    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert [error["code"] for error in doc["errors"]] == ["binding_mismatch", "origin_unbound"]
    assert doc["errors"][1]["detail"] == {"path": str(schedule), "pool_id": "pool.a", "member": "valuation_origin"}


def test_a_schedule_whose_requires_lacks_the_word_is_refused_and_the_word_is_not_added(tmp_path, acmodel):
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line([DAY_RULE], requires='["fact_reference"]'))
    before = schedule.read_bytes()
    bind(folder, ("val.a.day", "rule.a.day"))

    status, doc = bindings(acmodel, folder, "apply")

    assert status == 6
    assert doc["errors"] == [{"code": "requires_word_missing", "detail": {"path": str(schedule), "word": WORD},
                              "ref": "/requires"}]
    assert schedule.read_bytes() == before


def test_the_declared_cost_must_be_the_bound_rules_consumes(tmp_path, acmodel):
    """val.make leaves pool.b's spending to the Runtime: the rule must declare that it consumes pool.b."""
    folder = values_repo(tmp_path, [MAKE])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line(['{"id":"rule.make","package_id":"pkg.m","priority":1}']))
    bind(folder, ("val.make", "rule.make"))

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    assert doc["errors"][0] == {
        "code": "value_consumes_mismatch",
        "detail": {"path": str(schedule), "entry": "/bindings/0", "value": "val.make", "rule_id": "rule.make",
                   "expected": ["pool.b"], "found": []},
        "ref": "/rules/0",
    }
    assert bindings(acmodel, folder, "apply")[0] == 6

    consumes = ',"consumes":[{"pool_id":"pool.b","amount":2}]}'
    schedule.write_bytes(one_line(['{"id":"rule.make","package_id":"pkg.m","priority":1' + consumes]))

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert schedule.read_bytes() == one_line([
        '{"id":"rule.make","package_id":"pkg.m","fixed_value_milli":1600,"fixed_value_origin":"model","priority":1'
        + consumes
    ])
    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["bindings"][0]["partial"], doc["data"]["bindings"][0]["exclude"]) == (True, MAKE["exclude"])


def test_a_member_with_no_separator_to_copy_is_not_inserted(tmp_path, acmodel):
    """`package_id` is the rule's last member: there is no separator after it to copy, so nothing is guessed."""
    folder = values_repo(tmp_path, [DAY_RUN])
    schedule = folder / "schedule.json"
    schedule.write_bytes(one_line(['{"id":"rule.z","package_id":"pkg.z"}']))
    before = schedule.read_bytes()
    bind(folder, ("val.a.day", "rule.z"))

    status, doc = bindings(acmodel, folder, "apply")

    assert status == 6
    assert doc["errors"] == [{
        "code": "insert_style_unknown",
        "detail": {"path": str(schedule), "entry": "/bindings/0", "value": "val.a.day", "rule_id": "rule.z",
                   "anchor": "package_id", "member": "fixed_value_milli"},
        "ref": "/rules/0",
    }]
    assert schedule.read_bytes() == before


def test_check_says_whether_out_values_json_is_current(tmp_path, acmodel):
    folder = values_repo(tmp_path, [DAY_RUN])
    assert export_values(acmodel, folder)[0] == 0
    path = str(base(folder) / "out" / "values.json")

    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["warnings"]) == (0, [])
    assert doc["data"]["values"] == {"file": path, "current": True, "stale": []}

    edit(base(folder) / "exports.json", lambda exports: exports.update(note="edited"))
    status, doc = bindings(acmodel, folder, "check")

    assert status == 0
    assert doc["data"]["values"] == {"file": path, "current": False, "stale": ["exports"]}
    assert doc["warnings"] == [{"code": "values_stale", "detail": {"path": path, "members": ["exports"]}}]


def test_an_origin_other_than_model_is_refused_and_never_rewritten(tmp_path, acmodel):
    """The Runtime reads an absent origin as the pack's and `"model"` beside a fixed value as the model's, and refuses any
    other. A bound rule whose origin says `pack`, or holds no text at all, is `origin_invalid`: `check` reports it with the
    other differences, and `apply` refuses and writes nothing, not even the other binding's value."""
    folder = values_repo(tmp_path, [DAY_RUN, NIGHT_RUN])
    schedule = folder / "schedule.json"
    bind(folder, ("val.a.day", "rule.a.day"), ("val.a.night", "rule.a.night"))

    def night(origin: str) -> str:
        return '{"id":"rule.a.night","package_id":"pkg.a","fixed_value_milli":9072,"fixed_value_origin":' + origin + "}"

    def invalid(found: str) -> dict[str, Any]:
        return {
            "code": "origin_invalid",
            "detail": {"path": str(schedule), "entry": "/bindings/1", "value": "val.a.night", "rule_id": "rule.a.night",
                       "found": found},
            "ref": "/rules/1/fixed_value_origin",
        }

    schedule.write_bytes(one_line([DAY_RULE, night('"pack"')]))
    before = schedule.read_bytes()

    status, doc = bindings(acmodel, folder, "check")

    assert status == 6
    day = {"path": str(schedule), "entry": "/bindings/0", "value": "val.a.day", "rule_id": "rule.a.day"}
    assert doc["errors"] == [
        {"code": "binding_mismatch", "detail": {**day, "reason": "value", "expected": "2448"}, "ref": "/rules/0"},
        {"code": "origin_missing", "detail": day, "ref": "/rules/0"},
        invalid("pack"),
    ]

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["data"], doc["errors"]) == (6, {}, [invalid("pack")])
    assert schedule.read_bytes() == before
    assert not (folder / "schedule.json.part").exists()

    schedule.write_bytes(one_line([DAY_DONE, night("true")]))
    before = schedule.read_bytes()

    status, doc = bindings(acmodel, folder, "apply")

    assert (status, doc["errors"]) == (6, [invalid("boolean")])
    assert schedule.read_bytes() == before


def test_a_value_binding_needs_prices_of_the_current_versions_the_tools_too(tmp_path, acmodel):
    """With a value binding, `check` reads the prices to compute the value: prices.json of other versions is the error
    `prices_stale`, not the coverage's warning. The tool's own version counts too, so after an upgrade the prices are
    exported again before `check` passes."""
    folder = values_repo(tmp_path, [DAY_RUN])
    (folder / "schedule.json").write_bytes(one_line([DAY_DONE]))
    bind(folder, ("val.a.day", "rule.a.day"))
    prices = str(base(folder) / "out" / "prices.json")
    assert bindings(acmodel, folder, "check")[0] == 0

    def stale(member: str) -> list[dict[str, Any]]:
        return [{
            "code": "export_refused",
            "detail": {"path": prices, "output": "values", "reason": "prices_stale", "value": member},
            "ref": f"/versions/{member}",
        }]

    write_prices(folder, tool="0.0.9")
    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"]) == (5, stale("tool"))

    write_prices(folder)
    edit(base(folder) / "rules" / "economy.json", lambda doc: doc["rules"][0].update(note="edited"))
    status, doc = bindings(acmodel, folder, "check")

    assert (status, doc["errors"]) == (5, stale("rules"))


def test_a_price_whose_acceptance_was_changed_or_withdrawn_since_is_pending_and_check_says_why(tmp_path, acmodel):
    """pool.c's price was delivered under an acceptance. exports.json changes it, then withdraws it, and the prices are
    not exported again (exports.json is no part of the versions, so prices.json stays current): the value that counts
    pool.c waits for the prices, with why, and `check` names that reason for its binding."""
    folder = values_repo(tmp_path, [COLLECT])
    (folder / "schedule.json").write_bytes(one_line([COLLECT_DONE]))
    bind(folder, ("val.b.collect", "rule.b.collect"))
    assert bindings(acmodel, folder, "check")[0] == 0

    for change, reason in (
        (lambda entries: entries[1]["accept_provisional"].update(until="another check"), "price_acceptance_changed"),
        (lambda entries: entries.pop(1), "price_acceptance_withdrawn"),
    ):
        edit(base(folder) / "exports.json", lambda doc: change(doc["acceptances"]))

        status, doc = export_values(acmodel, folder)

        assert (status, doc["errors"]) == (0, [])
        [written] = delivered(folder)["values"]
        assert (written["status"], written["fixed_value_milli"], written["pending"]) == (
            "pending", None, [{"reason": reason, "item": "pool.c"}])
        assert provenance(folder, "val.b.collect")["result"]["pending"] == written["pending"]

        status, doc = bindings(acmodel, folder, "check")

        assert (status, doc["errors"]) == (5, [{
            "code": "export_refused",
            "detail": {"path": str(base(folder) / "exports.json"), "output": "value:val.b.collect", "reason": "pending",
                       "value": f"{reason}:pool.c"},
            "ref": "/values/0",
        }])


# Writing through a part: thresholds too ---------------------------------------------------------------------------------


@pytest.fixture
def example(tmp_path: Path) -> Path:
    """A copy of the worked example (test_bindings), its schedule's trigger offset written as -25 instead of the
    threshold's -250, every other byte as it is."""
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    schedule = target / "schedule.json"
    raw = schedule.read_bytes()
    assert raw.count(b'"offset": -250') == 1
    schedule.write_bytes(raw.replace(b'"offset": -250', b'"offset": -25'))
    return target


def example_apply(acmodel, folder: Path) -> tuple[int, dict[str, Any]]:
    return acmodel("bindings", "apply", base(folder), "--state", folder / "state" / "acct-1.json")


def test_a_threshold_is_written_through_its_part_too(example, acmodel, monkeypatch):
    original = (EXAMPLE / "schedule.json").read_bytes()
    renamed: list[tuple[str, bool]] = []

    def rename(part: Path, path: Path) -> None:
        renamed.append((part.name, part.exists()))
        os.replace(part, path)

    monkeypatch.setattr(commands, "_replace", rename)
    status, doc = example_apply(acmodel, example)

    assert (status, doc["errors"]) == (0, [])
    assert (example / "schedule.json").read_bytes() == original
    assert renamed == [("schedule.json.part", True)]
    assert doc["data"]["changed"] == [{
        "entry": "/bindings/0", "threshold": "thr.run.trigger", "file": "schedule.json", "rule_id": "run.trigger",
        "path": "/trigger/predicates/0/value/offset", "from": -25, "to": -250,
    }]


def test_a_file_changed_before_the_rename_is_refused_and_left_as_it_is(example, acmodel, monkeypatch):
    """Another writer changes schedule.json after `apply` read it and wrote its part: the part is not renamed over it
    (`file_changed`), it is removed again, and the other writer's bytes stay."""
    schedule = example / "schedule.json"
    stage = commands._stage

    def meanwhile(target, raw: bytes) -> Path:
        part = stage(target, raw)
        target.path.write_bytes(target.raw + b"\n")
        return part

    monkeypatch.setattr(commands, "_stage", meanwhile)
    before = schedule.read_bytes()

    status, doc = example_apply(acmodel, example)

    assert (status, doc["data"]) == (5, {})
    assert doc["errors"] == [{"code": "file_changed", "detail": {"path": str(schedule)}}]
    assert schedule.read_bytes() == before + b"\n"
    assert not (example / "schedule.json.part").exists()


# The catalog ------------------------------------------------------------------------------------------------------------


def test_every_code_is_registered_with_its_exit_status():
    """codes.py and contracts/codes.json list the same codes with the same exit statuses; the codes of #464 W1 are of
    their category."""
    assert set(EXIT_STATUS) == set(CODES)
    assert {code: CODES[code]["exit"] for code in CODES} == EXIT_STATUS
    new = {
        "value_not_positive": ("error", 5), "origin_missing": ("error", 6), "origin_invalid": ("error", 6),
        "origin_unbound": ("error", 6),
        "value_consumes_mismatch": ("error", 6), "requires_word_missing": ("error", 6),
        "insert_style_unknown": ("error", 6), "file_changed": ("error", 5), "values_stale": ("warning", 0),
    }
    assert {code: (CODES[code]["category"], CODES[code]["exit"]) for code in new} == new
    assert {"prices_stale", "pending"} <= set(CODES["export_refused"]["values"]["reason"]["members"])
