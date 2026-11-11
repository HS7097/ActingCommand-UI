"""#424 ACM-PRICE PRICE-4a: `acmodel export prices` (`out/prices.json`, the price provenance files), the four classes of
coverage, `estimate_missing`, the impact assessment of the estimates and the acceptances of the prices (`binds`, `scope`,
`flips`, `accept_zero`; `acceptance_stale`, `acceptance_incomplete`), and the coverage `bindings check` reads back
(addendum 2.1 §3.1–§3.4 and §3.7; the design's §4.1, §4.2, §4.6).

Every fixture is a hand-solvable neutral model (one-day cycle, main goals worth 1 value unit a unit unless said), with
the game's price unit κ = 1000 (`prices.unit_milli`). Each docstring works the prices out by hand. CI runs these tests on
Windows and on Ubuntu, so both legs must give the same exact values."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli
from acmodel.core import provenance
from acmodel.export import jcs
from test_coverage import zero_fixture
from test_economy import CODES, DAY, PROVENANCE, activity, flow, goal, limit, model, regen, resource, write
from test_price_vector import ex1

MODEL = ("ours", "model")
UNIT = 1000
HEX = re.compile("[0-9a-f]{64}")


@pytest.fixture
def acmodel(make_system, app_dir):
    """One command in the venv on a fixture folder (`export prices`, `bindings check` or `explain`): (exit status,
    envelope). The envelope is one acmodel.cli.v1 line whose errors and warnings agree with contracts/codes.json."""

    def run(*argv: str) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main(list(argv), system)
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


def export(acmodel, folder: Path) -> tuple[int, dict[str, Any]]:
    return acmodel("export", "prices", str(folder.joinpath(*MODEL)), "--state", str(folder / "state.json"))


def base(folder: Path) -> Path:
    return folder.joinpath(*MODEL)


def edit(path: Path, change: Callable[[Any], None]) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


def priced(folder: Path) -> Path:
    """The game's price unit in exports.json: κ = 1000."""
    edit(base(folder) / "exports.json", lambda exports: exports.update(prices={"unit_milli": UNIT}))
    return folder


def accept(folder: Path, *acceptances: dict[str, Any]) -> None:
    edit(base(folder) / "exports.json", lambda exports: exports.update(acceptances=list(acceptances)))


def delivered(folder: Path) -> dict[str, Any]:
    return json.loads((base(folder) / "out" / "prices.json").read_text(encoding="utf-8"))


def entries(folder: Path) -> dict[str, dict[str, Any]]:
    return {entry["pool"]: entry for entry in delivered(folder)["prices"]}


def picked(entry: dict[str, Any], *keys: str) -> dict[str, Any]:
    return {key: entry[key] for key in keys}


TIMING = "timing_ms"


def untimed(groups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """The assessment's groups without each pass's `timing_ms` (PASS-TIMING), which records how long that run took and so
    differs from run to run: each pass's `at` and `values`, as before."""
    return [
        {**group, "passes": [{key: value for key, value in each.items() if key != TIMING} for each in group["passes"]]}
        for group in groups
    ]


def untimed_doc(doc: dict[str, Any]) -> dict[str, Any]:
    """prices.json without its passes' `timing_ms` (`untimed`)."""
    return {**doc, "impact": untimed(doc["impact"])}


def canonical_digest(doc: Any) -> str:
    """The sha256 of a document's canonical JSON (JCS), as a provenance document names a shared tree by."""
    return hashlib.sha256(jcs.dumps(doc)).hexdigest()


def kept(app_dir: Path, sha: str) -> Path:
    """Where the state root keeps the shared tree whose digest is `sha` (PRICE-4a2): the state root is the default one,
    under the configuration folder, and the fixtures' model is `economy`."""
    return app_dir / "state" / "provenance" / "economy" / f"{sha}.json"


def kept_tree(app_dir: Path, doc: dict[str, Any]) -> dict[str, Any]:
    """The steady state's tree that a price's provenance document names, as the state root keeps it."""
    [(root, sha)] = doc["shared"].items()
    assert root == "economy:steady"
    return json.loads(kept(app_dir, sha).read_text(encoding="utf-8"))


def resolved(doc: dict[str, Any], tree: dict[str, Any]) -> dict[str, Any]:
    """A price's provenance document with the shared tree it names put back, and without the acceptance it is delivered
    under: the document `explain` prints."""
    whole = {key: value for key, value in doc.items() if key not in ("shared", "acceptance")}
    return {**whole, "nodes": {**doc["nodes"], **tree["nodes"]}}


def provisional(rule_id: str, subject: str, value: str, estimate: dict[str, Any] | None = None) -> dict[str, Any]:
    """A provisional derive, with its estimate when one is given."""
    logic = {"value": value, **({"estimate": estimate} if estimate is not None else {})}
    return {"id": rule_id, "kind": "derive", "subject": subject, "logic": logic, "provenance": PROVENANCE,
            "status": "provisional"}


def rules_path(folder: Path) -> str:
    return str(base(folder) / "rules" / "economy.json")


def exports_path(folder: Path) -> str:
    return str(base(folder) / "exports.json")


ESTIMATE_B = {"low": "2", "high": "4", "basis": "statistics", "covers": {"activities": ["act.b"]}}


def two_parts(folder: Path, estimate: dict[str, Any] | None) -> Path:
    """Two static components with no edge between them. act.a brings in 2 a once a day, all confirmed. act.b brings in
    `pt.b` b once a day: pt.b (rule 2) is a provisional derive of 3, with `estimate` (or none). a and b are main goals,
    worth 1 each. By hand: V0 = 2 + 3 = 5; one more or one less a (or b) is one unit more or less for its goal, so a and b
    are worth exactly 1, whatever pt.b is. a's price is confirmed, b's provisional (its component read pt.b)."""
    rules = [
        flow("flow.a", "act.a", produces={"res.a": "2"}), limit("limit.a", "act.a", "1", "day"),
        provisional("pt.b", "act.b", "3", estimate),
        flow("flow.b", "act.b", produces={"res.b": "rule:pt.b"}), limit("limit.b", "act.b", "1", "day"),
    ]
    entities = [resource("res.a", "fact.a"), resource("res.b", "fact.b"), activity("act.a"), activity("act.b")]
    goals = [goal("goal.a", "res.a", "1"), goal("goal.b", "res.b", "1")]
    return priced(model(folder, [DAY], entities, rules, {"fact.a": 0, "fact.b": 0}, goals=goals))


EXACT_ONE = {"value": "1", "interval": "1", "by": "exact", "withheld": None}


# The failing-first test: a provisional derive without an estimate -------------------------------------------------


def test_a_provisional_derive_without_an_estimate_leaves_the_prices_it_touches_pending(tmp_path, acmodel):
    """Addendum 2.1 §3.4 and §3.7 (PRICE-4a's red test: before this slice `export prices` does not exist). pt.b has no
    estimate, and the steady state reads it (act.b's amount): its impact cannot be assessed, so b's price is pending
    (`estimate_missing`), whatever exports.json accepts, and the warning names pt.b and the pools it touches. a's
    component never read it: a's price is confirmed and exported, 1, or 1000 thousandths at κ = 1000."""
    folder = two_parts(tmp_path, None)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert doc["warnings"] == [{
        "code": "estimate_missing", "detail": {"path": rules_path(folder), "id": "pt.b", "pools": ["res.b"]},
        "ref": "/rules/2/logic",
    }]
    found = entries(folder)
    assert picked(found["res.a"], "class", "status", "price", "milli", "pending", "joint", "impact") == {
        "class": "covered", "status": "confirmed", "price": "1", "milli": "1000", "pending": [], "joint": EXACT_ONE,
        "impact": None,
    } and "estimates" not in found["res.a"]
    assert picked(found["res.b"], "class", "status", "price", "milli", "pending", "component") == {
        "class": "covered", "status": "provisional", "price": "pending", "milli": None, "pending": ["estimate_missing"],
        "component": "act.b",
    } and "estimates" not in found["res.b"]
    prices = delivered(folder)
    assert prices["estimates"] == [{
        "rule": "pt.b", "value": "3", "estimate": None, "components": ["act.b"], "assessed": False,
        "skipped": "estimate_missing",
    }]
    assert prices["impact"] == []
    assert prices["counts"] == {
        "covered": 2, "structural_gap": 0, "verified_zero": 0, "unknown": 0, "exported": 1, "pending": 1,
    }
    assert doc["data"]["counts"] == prices["counts"] and doc["data"]["unit_milli"] == UNIT


# §2.5: the fixed small models; the price file and its provenance --------------------------------------------------


def test_the_joint_vector_of_a_set_of_pools_is_exported_with_its_provenance(tmp_path, acmodel, app_dir):
    """§2.5 test 1 (EX1): a, b and c come at 1 a day each, and act.set turns one of each into 1 v. Their joint vector is
    (1/3, 1/3, 1/3) on intervals [0, 1] (test_price_vector.py works it out), all confirmed: each is exported, 1/3, or
    1000/3 thousandths at κ = 1000, and marked degenerate (two ends on D*); v is exactly 1. Each pool's provenance file,
    with the steady state's tree it names put back (PRICE-4a2: the tree is kept once, under the state root), is the
    document `explain price:<pool>` prints, and its `binds` the sha256 of that file's canonical JSON."""
    folder = priced(ex1(tmp_path))

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    for pool in ("res.a", "res.b", "res.c"):
        assert picked(found[pool], "class", "status", "price", "milli", "pending", "degenerate", "varies_by_step") == {
            "class": "covered", "status": "confirmed", "price": "1/3", "milli": "1000/3", "pending": [],
            "degenerate": True, "varies_by_step": False,
        }
        assert found[pool]["joint"] == {
            "value": "1/3", "interval": {"low": "0", "high": "1"}, "by": "max_min", "withheld": None,
        }
    assert picked(found["res.v"], "price", "milli", "degenerate") == {"price": "1", "milli": "1000", "degenerate": False}
    prices = delivered(folder)
    assert (prices["schema_version"], prices["unit_milli"], prices["slots"]) == ("acmodel.prices.v2", UNIT, [])
    assert prices["counts"]["exported"] == 4 and prices["estimates"] == [] and prices["impact"] == []
    assert sorted(path.name for path in (base(folder) / "out" / "provenance" / "prices").iterdir()) == [
        "res.a.json", "res.b.json", "res.c.json", "res.v.json",
    ]

    status, shown = acmodel("explain", str(base(folder)), "price:res.a", "--state", str(folder / "state.json"))

    assert (status, shown["errors"]) == (0, [])
    written = json.loads((base(folder) / found["res.a"]["provenance"]).read_text(encoding="utf-8"))
    assert resolved(written, kept_tree(app_dir, written)) == shown["data"]["provenance"]
    assert HEX.fullmatch(found["res.a"]["binds"])
    assert found["res.a"]["binds"] != found["res.b"]["binds"]  # each document is rooted at its own price


def test_without_the_games_price_unit_nothing_is_computed_or_written(tmp_path, acmodel):
    """κ has no default: exports.json without `prices` refuses the export (`model_file_invalid`, `member_missing`)."""
    folder = ex1(tmp_path)

    status, doc = export(acmodel, folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "model_file_invalid",
        "detail": {"reason": "member_missing", "path": exports_path(folder), "format": "acmodel.exports.v1",
                   "key": "prices"},
        "ref": "/prices",
    }]
    assert not (base(folder) / "out").exists()


def test_a_blocked_steady_state_refuses_the_export_and_writes_nothing(tmp_path, acmodel):
    """A confirmed flow whose amount reads an unknown blocks the steady state (it solves single values): the export is
    `export_refused` (`blocked`, as `economy:steady`), with the error that blocks it, and writes nothing."""
    unknown = {
        "id": "unk.x", "kind": "interval", "question": "fixture", "interval": {"low": "1", "high": "2"},
        "status": "provisional", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }
    rules = [flow("flow.a", "act.a", produces={"res.v": "unknown:unk.x"}), limit("limit.a", "act.a", "1", "day")]
    folder = priced(model(tmp_path, [DAY], [resource("res.v", "fact.v"), activity("act.a")], rules, {"fact.v": 0},
                          [unknown]))

    status, doc = export(acmodel, folder)

    assert status == 5
    assert doc["errors"][0] == {
        "code": "export_refused",
        "detail": {"path": exports_path(folder), "output": "economy:steady", "reason": "blocked"}, "ref": "/prices",
    }
    assert [entry["code"] for entry in doc["errors"][1:]] == ["expr_eval_failed"]
    assert doc["errors"][1]["needed_by"] == ["economy:steady"]
    assert not (base(folder) / "out").exists()


# §3.4: the impact assessment and the acceptance of a provisional price -----------------------------------------------


def test_an_accepted_provisional_price_is_exported_until_what_it_binds_changes(tmp_path, acmodel):
    """§3.4. pt.b (3) is estimated from 2 to 4 (`low`, `high`). Its group has two passes, pt.b at 2 and at 4; b stays
    worth exactly 1 in both (each b is one unit for its goal), and act.b's value, pt.b × 1, stays positive: b's price
    range is [1, 1] and nothing flips. Without an acceptance b's price is pending (`not_accepted`). Accepted, with the
    `binds` that prices.json shows, scope `game` and no flips, it is exported (1, 1000 thousandths) and its provenance
    file records the acceptance. Then the estimate changes (high 5): the price's provenance document changes with it, so
    the acceptance binds another one: pending again (`acceptance_stale`), with the warning."""
    folder = two_parts(tmp_path, ESTIMATE_B)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    assert picked(found["res.b"], "price", "pending", "impact", "flips", "component", "acceptance") == {
        "price": "pending", "pending": ["not_accepted"], "impact": {"low": "1", "high": "1"}, "flips": [],
        "component": "act.b", "acceptance": None,
    } and "estimates" not in found["res.b"]
    assert found["res.a"]["impact"] is None  # its component read no estimate
    prices = delivered(folder)
    assert prices["estimates"] == [{
        "rule": "pt.b", "value": "3", "estimate": ESTIMATE_B, "components": ["act.b"], "assessed": True, "skipped": None,
    }]
    assert untimed(prices["impact"]) == [{
        "rules": ["pt.b"], "unknowns": [],
        "passes": [{"at": {"pt.b": "low"}, "values": {"pt.b": "2"}}, {"at": {"pt.b": "high"}, "values": {"pt.b": "4"}}],
        "prices": {}, "flips": [],
    }]
    binds = found["res.b"]["binds"]
    given = {"ref": "R-1", "until": "check list item 1", "binds": binds, "scope": "game", "flips": []}
    accept(folder, {"output": "price:res.b", "accept_provisional": given})

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    assert picked(found["res.b"], "price", "milli", "pending", "binds") == {
        "price": "1", "milli": "1000", "pending": [], "binds": binds,
    }
    assert found["res.b"]["acceptance"] == {"entry": "/acceptances/0", "output": "price:res.b", "accept_provisional": given}
    written = json.loads((base(folder) / found["res.b"]["provenance"]).read_text(encoding="utf-8"))
    recorded = written["acceptance"]
    assert HEX.fullmatch(recorded.pop("sha256"))
    assert recorded == {"file": "exports.json", "ref": "/acceptances/0", "accept_provisional": given}

    edit(base(folder) / "rules" / "economy.json",
         lambda rules: rules["rules"][2]["logic"]["estimate"].update(high="5"))
    status, doc = export(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    found = entries(folder)
    assert found["res.b"]["binds"] != binds
    assert picked(found["res.b"], "price", "pending") == {"price": "pending", "pending": ["acceptance_stale"]}
    assert doc["warnings"] == [{
        "code": "acceptance_stale",
        "detail": {"path": exports_path(folder), "output": "price:res.b", "binds": binds,
                   "expected": found["res.b"]["binds"]},
        "ref": "/acceptances/0/accept_provisional/binds",
    }]
    assert "acceptance" not in json.loads((base(folder) / found["res.b"]["provenance"]).read_text(encoding="utf-8"))


ESTIMATE_K = {"low": "1/2", "high": "2", "basis": "statistics", "covers": {"activities": ["act.q"]}}


def flips_fixture(folder: Path) -> Path:
    """s comes at 10 a day; act.p turns 1 s into 1 v, act.q turns 1 s into `pt.k` v, pt.k (rule 2) a provisional derive
    of 1 estimated from 1/2 to 2; v is the main goal, worth 1. By hand, at pt.k = 1 both uses are as good: V0 = 10, s is
    worth exactly 1 (one more s is one more v), and act.p and act.q are worth 1 - 1 = 0. At pt.k = 1/2, act.p is the
    better use: s stays 1, act.p reads 0 and act.q 1/2 - 1 = -1/2. At pt.k = 2, act.q is: s is worth 2, act.p reads
    1 - 2 = -1 and act.q 2 - 2 = 0. So s's price ranges over [1, 2], v's stays 1, and both activities flip (0 to a
    negative value); each takes s and brings in v."""
    rules = [
        regen("regen.s", "res.s", "10", "1d"),
        flow("flow.p", "act.p", consumes={"res.s": "1"}, produces={"res.v": "1"}),
        provisional("pt.k", "act.q", "1", ESTIMATE_K),
        flow("flow.q", "act.q", consumes={"res.s": "1"}, produces={"res.v": "rule:pt.k"}),
    ]
    entities = [resource("res.s", "fact.s"), resource("res.v", "fact.v"), activity("act.p"), activity("act.q")]
    return priced(model(folder, [DAY], entities, rules, {"fact.s": 0, "fact.v": 0}))


def test_the_assessment_moves_an_estimate_to_its_ends_and_an_acceptance_must_list_every_flip(tmp_path, acmodel):
    """§3.4: the two passes of pt.k (flips_fixture works them out) give s the range [1, 2] and flip act.p and act.q.
    An acceptance of s that lists only act.p is incomplete: pending (`acceptance_incomplete`), with the warning naming
    act.q. Listing both, it is exported at the steady state's price, 1."""
    folder = flips_fixture(tmp_path)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    assert picked(found["res.s"], "class", "status", "price", "pending", "impact", "flips") == {
        "class": "covered", "status": "provisional", "price": "pending", "pending": ["not_accepted"],
        "impact": {"low": "1", "high": "2"}, "flips": ["act.p", "act.q"],
    }
    assert picked(found["res.v"], "impact", "flips") == {"impact": {"low": "1", "high": "1"}, "flips": ["act.p", "act.q"]}
    assert untimed(delivered(folder)["impact"]) == [{
        "rules": ["pt.k"], "unknowns": [],
        "passes": [{"at": {"pt.k": "low"}, "values": {"pt.k": "1/2"}}, {"at": {"pt.k": "high"}, "values": {"pt.k": "2"}}],
        "prices": {"res.s": {"low": "1", "high": "2"}}, "flips": ["act.p", "act.q"],
    }]
    given = {"ref": "R-2", "until": "check list item 2", "binds": found["res.s"]["binds"], "scope": "game",
             "flips": ["act.p"]}
    accept(folder, {"output": "price:res.s", "accept_provisional": given})

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    assert picked(entries(folder)["res.s"], "price", "pending") == {"price": "pending", "pending": ["acceptance_incomplete"]}
    assert doc["warnings"] == [{
        "code": "acceptance_incomplete",
        "detail": {"path": exports_path(folder), "output": "price:res.s", "missing": ["act.q"]},
        "ref": "/acceptances/0/accept_provisional/flips",
    }]

    accept(folder, {"output": "price:res.s", "accept_provisional": {**given, "flips": ["act.p", "act.q"]}})
    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert picked(entries(folder)["res.s"], "price", "milli", "pending") == {"price": "1", "milli": "1000", "pending": []}


def test_an_acceptance_short_of_the_whole_game_does_not_deliver(tmp_path, acmodel):
    """§3.2 and §3.4: the same schedule.json pool is every instance's, so a scope of some instances leaves the price
    pending (`scope_partial`)."""
    folder = flips_fixture(tmp_path)
    export(acmodel, folder)
    flips = ["act.p", "act.q"]
    given = {"ref": "R-2", "until": "item", "binds": entries(folder)["res.s"]["binds"], "scope": ["inst-1"], "flips": flips}
    accept(folder, {"output": "price:res.s", "accept_provisional": given})

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert picked(entries(folder)["res.s"], "price", "pending") == {"price": "pending", "pending": ["scope_partial"]}


OFFER = {
    "id": "unk.offer", "kind": "variants", "question": "fixture",
    "variants": {"cheap": {"price": "10"}, "dear": {"price": "30"}},
    "status": "provisional", "provenance": [{"kind": "pending_check", "ref": "fixture"}],
}
ESTIMATE_PRICE = {"of": "unknown:unk.offer.price", "basis": "variants", "covers": {"activities": ["act.buy"]}}


def test_an_estimate_of_variants_is_moved_to_each_variant(tmp_path, acmodel):
    """§3.7: an estimate written with `of` moves to each point of the unknown it reads. c comes at 60 a day; act.buy
    turns `pt.price` c into 1 g, g the main goal worth 1; pt.price is 20, the mean of an offer that costs 10 (cheap) or
    30 (dear). By hand: at 20, c is worth 1/20 (one more c is 1/20 more g); at 10, 1/10; at 30, 1/30. So c's price range
    is [1/30, 1/10]; act.buy reads 1 - price × c's price = 0 in every pass, so nothing flips."""
    rules = [
        regen("regen.c", "res.c", "60", "1d"),
        provisional("pt.price", "act.buy", "20", ESTIMATE_PRICE),
        flow("flow.buy", "act.buy", consumes={"res.c": "rule:pt.price"}, produces={"res.g": "1"}),
    ]
    entities = [resource("res.c", "fact.c"), resource("res.g", "fact.g"), activity("act.buy")]
    folder = priced(model(tmp_path, [DAY], entities, rules, {"fact.c": 0, "fact.g": 0}, [OFFER],
                          goals=[goal("goal.g", "res.g", "1")]))

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    assert picked(found["res.c"], "price", "pending", "joint", "impact", "flips") == {
        "price": "pending", "pending": ["not_accepted"],
        "joint": {"value": "1/20", "interval": "1/20", "by": "exact", "withheld": None},
        "impact": {"low": "1/30", "high": "1/10"}, "flips": [],
    }
    assert untimed(delivered(folder)["impact"]) == [{
        "rules": ["pt.price"], "unknowns": ["unk.offer"],
        "passes": [
            {"at": {"unk.offer": "cheap"}, "values": {"pt.price": "10"}},
            {"at": {"unk.offer": "dear"}, "values": {"pt.price": "30"}},
        ],
        "prices": {"res.c": {"low": "1/30", "high": "1/10"}}, "flips": [],
    }]


# §3.2: a verified zero and its acceptance ------------------------------------------------------------------------------


def test_a_verified_zero_is_exported_only_under_its_accept_zero_over_the_whole_game(tmp_path, acmodel):
    """§3.1 and §3.2 (test_coverage.py's zero fixture): part is a conditional zero (every room full) and block one too,
    both sound. Without acceptances both are pending (`not_accepted`); v, covered and confirmed, is exported. With an
    `accept_zero` over the whole game, part is exported as 0; block's, over one instance only, leaves it pending
    (`scope_partial`): a conditional zero holds only where it was verified."""
    folder = priced(zero_fixture(tmp_path, ["fact.base_max", "fact.rooms_full"]))

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    for pool in ("res.block", "res.part"):
        assert picked(found[pool], "class", "price", "pending") == {
            "class": "verified_zero", "price": "pending", "pending": ["not_accepted"],
        }
    assert picked(found["res.v"], "class", "price", "pending") == {"class": "covered", "price": "1", "pending": []}

    accept(
        folder,
        {"output": "price:res.part", "accept_zero": {"ref": "R-3", "scope": "game", "until": "rooms checked"}},
        {"output": "price:res.block", "accept_zero": {"ref": "R-4", "scope": ["inst-1"], "until": "base checked"}},
    )
    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    assert picked(found["res.part"], "price", "milli", "pending") == {"price": "0", "milli": "0", "pending": []}
    assert picked(found["res.block"], "price", "pending") == {"price": "pending", "pending": ["scope_partial"]}
    written = json.loads((base(folder) / found["res.part"]["provenance"]).read_text(encoding="utf-8"))
    assert written["acceptance"]["accept_zero"] == {"ref": "R-3", "scope": "game", "until": "rooms checked"}


# The acceptances in exports.json ---------------------------------------------------------------------------------------

GIVEN = {"ref": "R", "until": "item"}
FULL = {**GIVEN, "binds": "0" * 64, "scope": "game", "flips": []}


@pytest.mark.parametrize(
    ("acceptance", "reason", "ref", "key"),
    [
        pytest.param({"output": "price:res.a", "accept_provisional": GIVEN}, "member_missing",
                     "/acceptances/0/accept_provisional/binds", "binds", id="price_without_binds"),
        pytest.param({"output": "rule:limit.a", "accept_provisional": FULL}, "member_unknown",
                     "/acceptances/0/accept_provisional/binds", "binds", id="rule_with_binds"),
        pytest.param({"output": "rule:limit.a", "accept_zero": {"ref": "R", "scope": "game", "until": "item"}},
                     "member_unknown", "/acceptances/0/accept_zero", "accept_zero", id="rule_with_accept_zero"),
        pytest.param({"output": "price:res.a", "accept_provisional": FULL,
                      "accept_zero": {"ref": "R", "scope": "game", "until": "item"}},
                     "member_conflict", "/acceptances/0/accept_zero", "accept_zero", id="both"),
    ],
)
def test_the_reader_checks_what_an_acceptance_of_a_price_holds(tmp_path, acmodel, acceptance, reason, ref, key):
    """An acceptance of a price holds `binds`, `scope` and `flips` in its `accept_provisional`, or an `accept_zero`
    instead; one of any other output holds neither (`model_file_invalid`)."""
    folder = two_parts(tmp_path, ESTIMATE_B)
    accept(folder, acceptance)

    status, doc = export(acmodel, folder)

    assert status == 1
    assert {entry["code"] for entry in doc["errors"]} == {"model_file_invalid"}
    [refused] = [entry for entry in doc["errors"] if entry["ref"] == ref]
    assert (refused["detail"]["reason"], refused["detail"]["key"]) == (reason, key)
    assert refused["detail"]["format"] == "acmodel.exports.v1"


def test_an_acceptance_of_a_price_names_a_pool_of_the_model(tmp_path, acmodel):
    """`price:<pool>` of no resource of the model names no output: `rule_reference_missing` (`expected` `output`)."""
    folder = two_parts(tmp_path, ESTIMATE_B)
    accept(folder, {"output": "price:res.none", "accept_provisional": FULL})

    status, doc = export(acmodel, folder)

    assert status == 1
    assert doc["errors"] == [{
        "code": "rule_reference_missing",
        "detail": {"path": exports_path(folder), "target": "price:res.none", "expected": ["output"]},
        "ref": "/acceptances/0/output",
    }]


# bindings check: the coverage of schedule.json ---------------------------------------------------------------------------


def test_bindings_check_reports_the_delivered_coverage_as_the_schedule_meets_it(tmp_path, acmodel):
    """§3.1 (the design's §4.6). two_parts with b estimated: a exported, b pending. schedule.json holds the pools res.a
    and res.x (which the model lacks: unknown), the rule r.a (produces res.a, consumes res.x: the worst class, unknown),
    the rule r.b (produces res.b: covered, provisional, its price pending) and a rule without effects. Before any export
    the coverage is null; after it, it is what prices.json holds; once a rule file changes, prices.json is of another
    version of the model: `current` false and the warning `prices_stale`."""
    folder = two_parts(tmp_path, ESTIMATE_B)
    write(base(folder) / "bindings.json", {"schema_version": "acmodel.bindings.v1", "bindings": [], "fixed": []})
    write(folder / "schedule.json", {
        "schema_version": "actingcommand.schedule.v1",
        "rules": [
            {"id": "r.a", "package_id": "p", "produces": [{"pool_id": "res.a", "amount": 2}],
             "consumes": [{"pool_id": "res.x", "amount": 1}]},
            {"id": "r.b", "package_id": "p", "produces": [{"pool_id": "res.b", "amount": 3}]},
            {"id": "r.none", "package_id": "p"},
        ],
        "pools": [{"id": "res.a"}, {"id": "res.x"}],
    })
    argv = ("bindings", "check", str(base(folder)), "--state", str(folder / "state.json"))

    status, doc = acmodel(*argv)

    assert (status, doc["errors"], doc["warnings"], doc["data"]["coverage"]) == (0, [], [], None)

    export(acmodel, folder)
    status, doc = acmodel(*argv)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    unknown = {"class": "unknown", "status": "provisional", "reasons": ["not_in_model"], "price": "pending",
               "pending": ["unknown"]}
    coverage = doc["data"]["coverage"]
    assert coverage == {
        "file": str(base(folder) / "out" / "prices.json"),
        "current": True,
        "pools": {
            "res.a": {"class": "covered", "status": "confirmed", "reasons": ["goal"], "price": "1", "pending": []},
            "res.x": unknown,
        },
        "rules": {
            "r.a": {"class": "unknown", "status": "provisional", "pools": ["res.a", "res.x"], "pending": ["res.x"]},
            "r.b": {"class": "covered", "status": "provisional", "pools": ["res.b"], "pending": ["res.b"]},
        },
        "counts": {
            "pools": {"covered": 1, "verified_zero": 0, "structural_gap": 0, "unknown": 1},
            "rules": {"covered": 1, "verified_zero": 0, "structural_gap": 0, "unknown": 1},
            "model": delivered(folder)["counts"],
        },
    }

    edit(base(folder) / "rules" / "economy.json", lambda rules: rules["rules"][0].update(note="edited"))
    status, doc = acmodel(*argv)

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["coverage"]["current"] is False
    assert doc["warnings"] == [{
        "code": "prices_stale", "detail": {"path": str(base(folder) / "out" / "prices.json"), "members": ["rules"]},
        "ref": "/versions",
    }]


# PRICE-4a2: one steady state's tree, shared by every price's provenance ------------------------------------------------

STEADY_ROOT = "economy:steady"


def redescribed(patched: pytest.MonkeyPatch) -> None:
    """The steady state's node described with one member more, as a tool that describes it otherwise would, while the
    model and its versions stay as they are: only the shared tree changes."""
    describe = provenance._describe

    def described(run: Any, item: tuple[str, ...]) -> dict[str, Any]:
        node = describe(run, item)
        return {**node, "note": "described anew"} if item == ("economy", "steady") else node

    patched.setattr(provenance, "_describe", described)


def test_the_prices_name_one_shared_steady_state_tree_by_its_digest(tmp_path, acmodel, app_dir):
    """PRICE-4a2's red test (before it, every price's provenance file held the whole steady state's tree, which every
    price reads). EX1's four prices: the tree is kept once, under the state root by the sha256 of its canonical JSON
    (`acmodel.provenance.tree.v1`, rooted at economy:steady, with the model's versions), never in the model folder. Each
    price's file holds its own node only (the price, which reads economy:steady) and names the tree in `shared` by that
    digest, the same for every pool; prices.json records the digest with the versions that compute it again and the
    tree's node count, and each price's `binds` is still the digest of its own file. A second export finds the tree
    intact and does not write it again."""
    folder = priced(ex1(tmp_path))

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    prices = delivered(folder)
    [(root, recorded)] = prices["shared"].items()
    sha = recorded["sha256"]
    assert root == STEADY_ROOT and HEX.fullmatch(sha)
    assert recorded["versions"] == prices["versions"]
    tree = json.loads(kept(app_dir, sha).read_text(encoding="utf-8"))
    assert canonical_digest(tree) == sha
    assert (tree["schema_version"], tree["model_id"], tree["root"], tree["versions"]) == (
        "acmodel.provenance.tree.v1", "economy", STEADY_ROOT, prices["versions"],
    )
    assert tree["nodes"][STEADY_ROOT]["node"] == "economy" and recorded["nodes"] == len(tree["nodes"])
    assert not any(name.startswith("price:") for name in tree["nodes"])
    assert doc["data"]["shared"] == {STEADY_ROOT: {"sha256": sha, "file": str(kept(app_dir, sha))}}
    assert str(kept(app_dir, sha)) in doc["data"]["written"]
    found = entries(folder)
    for pool in ("res.a", "res.b", "res.c", "res.v"):
        written = json.loads((base(folder) / found[pool]["provenance"]).read_text(encoding="utf-8"))
        assert written["shared"] == {STEADY_ROOT: sha}
        assert written["nodes"] == {f"price:{pool}": {"node": "price", "resource": pool, "reads": [STEADY_ROOT]}}
        assert found[pool]["binds"] == canonical_digest(written)
    out = base(folder) / "out"
    assert sorted(path.relative_to(out).as_posix() for path in out.rglob("*") if path.is_file()) == [
        "prices.json", "provenance/prices/res.a.json", "provenance/prices/res.b.json", "provenance/prices/res.c.json",
        "provenance/prices/res.v.json",
    ]

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert delivered(folder)["shared"] == prices["shared"]
    assert str(kept(app_dir, sha)) not in doc["data"]["written"]


def test_a_change_of_the_shared_tree_alone_moves_what_every_price_binds(tmp_path, acmodel, monkeypatch):
    """PRICE-4a2 (§3.4: `binds` is the digest of the price's provenance document). flips_fixture: s and v are one
    component, which read pt.k, so both prices are provisional; accepted with the binds prices.json shows and every flip
    (act.p, act.q; test_the_assessment_moves_... works them out), both are exported. Then the steady state's node is
    described with one member more while the model stays as it is: the versions, each price's own nodes and its result
    are unchanged, yet the shared tree's digest is another, so is what each price's file names, and so is each price's
    `binds`. Both acceptances bind a document that is no longer the price's: both prices are pending again
    (`acceptance_stale`, one warning each)."""
    folder = flips_fixture(tmp_path)
    export(acmodel, folder)
    before = entries(folder)
    flips = ["act.p", "act.q"]
    accept(
        folder,
        {"output": "price:res.s", "accept_provisional": {
            "ref": "R-5", "until": "item", "binds": before["res.s"]["binds"], "scope": "game", "flips": flips}},
        {"output": "price:res.v", "accept_provisional": {
            "ref": "R-6", "until": "item", "binds": before["res.v"]["binds"], "scope": "game", "flips": flips}},
    )

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = entries(folder)
    for pool in ("res.s", "res.v"):
        assert picked(found[pool], "status", "price", "pending") == {"status": "provisional", "price": "1", "pending": []}
    prices = delivered(folder)
    files = {pool: json.loads((base(folder) / found[pool]["provenance"]).read_text(encoding="utf-8")) for pool in found}

    with monkeypatch.context() as patched:
        redescribed(patched)
        status, doc = export(acmodel, folder)

    assert (status, doc["errors"]) == (0, [])
    after, moved = entries(folder), delivered(folder)
    assert moved["versions"] == prices["versions"]
    sha, moved_sha = prices["shared"][STEADY_ROOT]["sha256"], moved["shared"][STEADY_ROOT]["sha256"]
    assert moved_sha != sha
    for pool in ("res.s", "res.v"):
        written = json.loads((base(folder) / after[pool]["provenance"]).read_text(encoding="utf-8"))
        assert (written["nodes"], written["result"]) == (files[pool]["nodes"], files[pool]["result"])
        assert (files[pool]["shared"], written["shared"]) == ({STEADY_ROOT: sha}, {STEADY_ROOT: moved_sha})
        assert after[pool]["binds"] != before[pool]["binds"]
        assert picked(after[pool], "price", "pending") == {"price": "pending", "pending": ["acceptance_stale"]}
    assert doc["warnings"] == [
        {
            "code": "acceptance_stale",
            "detail": {"path": exports_path(folder), "output": f"price:{pool}", "binds": before[pool]["binds"],
                       "expected": after[pool]["binds"]},
            "ref": f"/acceptances/{index}/accept_provisional/binds",
        }
        for index, pool in enumerate(("res.s", "res.v"))
    ]


def test_verify_prices_computes_the_shared_tree_again_and_names_what_does_not_match(tmp_path, acmodel, app_dir, monkeypatch):
    """PRICE-4a2: the tree stays out of the repository, so `verify prices` computes it again from the model and checks it
    against what prices.json records. Before any export there is nothing to verify (`model_file_invalid`). After EX1's
    export it checks: the digest computed again is the recorded one, the state root's copy is intact (`restored`
    false). A spoilt copy is written again (`restored` true). Then each mismatch has its name (exit 6):
    - the tool describing the tree otherwise, the model unchanged: `shared_tree_mismatch`, `recomputed`, at the
      recorded digest;
    - a price's file naming another tree: `shared_tree_mismatch`, `referenced`, at that file's `shared`;
    - the model changed since the export (a rule's note): its versions are no longer the recorded ones, the tree cannot
      be computed again from them, `shared_tree_stale` with the members that differ, and nothing else is computed."""
    folder = priced(ex1(tmp_path))
    argv = ("verify", "prices", str(base(folder)), "--state", str(folder / "state.json"))
    prices_file = str(base(folder) / "out" / "prices.json")

    status, doc = acmodel(*argv)

    assert status == 1
    assert [(entry["code"], entry["detail"]["reason"]) for entry in doc["errors"]] == [("model_file_invalid", "unreadable")]

    export(acmodel, folder)
    recorded = delivered(folder)["shared"][STEADY_ROOT]
    sha, tree_file = recorded["sha256"], kept(app_dir, recorded["sha256"])
    status, doc = acmodel(*argv)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["file"] == prices_file
    assert doc["data"]["shared"] == {STEADY_ROOT: {
        "sha256": sha, "recomputed": sha, "nodes": recorded["nodes"], "file": str(tree_file), "restored": False,
    }}

    tree_file.write_text("{}", encoding="utf-8")
    status, doc = acmodel(*argv)

    assert (status, doc["errors"]) == (0, [])
    assert doc["data"]["shared"][STEADY_ROOT]["restored"] is True
    assert canonical_digest(json.loads(tree_file.read_text(encoding="utf-8"))) == sha

    with monkeypatch.context() as patched:
        redescribed(patched)
        status, doc = acmodel(*argv)

    assert status == 6
    [refused] = doc["errors"]
    assert (refused["code"], refused["ref"]) == ("shared_tree_mismatch", "/shared/economy:steady/sha256")
    assert {key: refused["detail"][key] for key in ("path", "root", "reason", "expected")} == {
        "path": prices_file, "root": STEADY_ROOT, "reason": "recomputed", "expected": sha,
    }
    assert HEX.fullmatch(refused["detail"]["found"]) and refused["detail"]["found"] != sha

    price_file = base(folder) / "out" / "provenance" / "prices" / "res.b.json"
    edit(price_file, lambda written: written.update(shared={STEADY_ROOT: "f" * 64}))
    status, doc = acmodel(*argv)

    assert status == 6
    assert doc["errors"] == [{
        "code": "shared_tree_mismatch",
        "detail": {"path": str(price_file), "root": STEADY_ROOT, "reason": "referenced", "expected": sha, "found": "f" * 64},
        "ref": "/shared/economy:steady",
    }]

    edit(base(folder) / "rules" / "economy.json", lambda rules: rules["rules"][0].update(note="edited"))
    status, doc = acmodel(*argv)

    assert status == 6
    assert doc["errors"] == [{
        "code": "shared_tree_stale",
        "detail": {"path": prices_file, "root": STEADY_ROOT, "members": ["rules"]},
        "ref": "/shared/economy:steady/versions",
    }]
