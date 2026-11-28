"""#430 METHOD-C C-2a-2a: the certificate ("D* invariance"). A pass that moves a value is offered to it first; it
proves the pass without solving it (`by` `certified`, the steady state's vector) or names the first condition that
refused it (`refused_by`: `program`, `c1`, `c2`, `c3`, `c4`, `lp_unverified`), and the pass is then solved. Its LPs are
counted in the pass's `lps.certificate`. `impact.certify` false solves every pass and records no `refused_by`
(contracts/formats.md, "`acmodel export prices`", "A pass", "The certificate" items 1 to 10, "Timing"; contracts/cli.md,
"Configuration").

Three readings of the contract that it leaves open are taken as the certificate's code takes them (`flow._Certifier`):
an unbounded answer of C1's or C2's LP refuses (reading 1), C3 takes no LP for an activity only when every step's column
of it is in J and lies in balance rows only (reading 2), and an LP of C1 is counted in the pass that solved it (reading
3). The fixtures are hand-solvable neutral models, each docstring working out by hand what the certificate finds; those
of test_economy.py, test_prices.py and test_method_c.py are reused where they say what is needed. CI runs these tests on
Windows and on Ubuntu."""

from __future__ import annotations

import json
import shutil
from fractions import Fraction
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import config
from acmodel.cache import EVALUATORS, canonical
from acmodel.check.coverage import judge
from acmodel.codes import LP_SOLVE_FAILED
from acmodel.core import evaluator, exact, impact
from acmodel.core import flow as economy_flow
from acmodel.core.graph import Moved
from acmodel.envelope import Failure, error
from acmodel.export import jcs
from acmodel.model import readers
from test_dstar_retry import Highs
from test_economy import DAY, activity, cap, flow, goal, limit, model, model_path, regen, resource, training
from test_lp_counts import SIX, Recorded, shaped
from test_method_c import as_before as as_before_c1
from test_method_c import graph_of, moves_fixture, solved
from test_pass_timing import passes
from test_price_reuse import reuse, snapshot
from test_prices import (  # noqa: F401 (the fixture `acmodel`)
    ESTIMATE_B,
    acmodel,
    base,
    delivered,
    export,
    flips_fixture,
    priced,
    provisional,
    rules_path,
    two_parts,
)

STEADY, VECTOR = economy_flow.NODES[economy_flow.STEADY], economy_flow.VECTOR_NODE
TIMING, BY, REFUSED_BY = "timing_ms", "by", "refused_by"
PASS_LPS = (*economy_flow.STAGES, "certificate")
UNCERTIFIED = {"schema_version": "acmodel.config.v1", "impact": {"certify": False}}


def estimate(low: str, high: str, *activities: str) -> dict[str, Any]:
    """An estimate from `low` to `high` that covers `activities` (each that reads the derive)."""
    return {"low": low, "high": high, "basis": "statistics", "covers": {"activities": sorted(activities)}}


# The fixtures ---------------------------------------------------------------------------------------------------------


def dominated_fixture(folder: Path, *, mixed: bool = False) -> Path:
    """One daily step. s comes at 10 a day; act.p turns 1 s into 1 v, v the main goal worth 1; act.j turns `pt.j` s into
    1 v, pt.j a provisional derive of 2 estimated from 3/2 to 3. By hand: act.p runs 10 times, V0 = 10; v's goal sink
    runs, so v is worth 1, and act.p runs, so s is worth 1 too: D* is that one point. act.j's dual slack is pt.j - 1: 1
    at the steady state, 1/2 and 2 at the passes, above 0 everywhere on D*, so no optimum ever runs it, and moving
    pt.j changes no price. J is act.j's one column (its coefficient in s's balance row): C1 is an LP (its slack at y*
    is 1), least 1; C2 an LP per pass, least 1/2 and 2; act.j is in no K (its value is -1 on all of D*) and its column
    lies in balance rows only, so C3 takes no LP (reading 2); no limit or income cap counts anything, so A has no term
    and C4 holds. Both passes are proven: the first pass solves C1's and C2's LPs (2), the second C2's alone (1,
    reading 3).

    With `mixed` (contract test 12), two more estimates beside it: act.p's v is `pt.p`, 1 estimated from 1 to 2, and a,
    a second main goal worth 1 with no edge to the rest, comes from act.a, 1 a day at most once a day, under a hard cap
    of `pt.cap`, 5 estimated from 4 to 6. pt.cap's passes move a cap's value, a right side of S3's program: `program`.
    pt.p at 1 is its point value (`steady_state`); at 2 act.p's column moves, and x* runs it: `c1`, with no LP. act.a is
    protected (a limit counts it, it takes nothing), so A has a term: for pt.j, C4's walk runs. At 3/2 act.j takes less
    s than at the steady state's 2, so s is short: its takers act.p and act.j are stopped, which makes v short (its goal
    sink is stopped, which is allowed) and s long; s and v have no cap and no regen below 0, and act.a is not stopped:
    the walk holds; at 3, s is only long. pt.j's two passes are proven still."""
    rules = [
        regen("regen.s", "res.s", "10", "1d"),
        flow("flow.p", "act.p", consumes={"res.s": "1"}, produces={"res.v": "rule:pt.p" if mixed else "1"}),
        provisional("pt.j", "act.j", "2", estimate("3/2", "3", "act.j")),
        flow("flow.j", "act.j", consumes={"res.s": "rule:pt.j"}, produces={"res.v": "1"}),
    ]
    entities = [resource("res.s", "fact.s"), resource("res.v", "fact.v"), activity("act.p"), activity("act.j")]
    readings = {"fact.s": 0, "fact.v": 0}
    goals = [goal("goal.v", "res.v", "1")]
    if mixed:
        rules += [
            provisional("pt.p", "act.p", "1", estimate("1", "2", "act.p")),
            flow("flow.a", "act.a", produces={"res.a": "1"}), limit("limit.a", "act.a", "1", "day"),
            provisional("pt.cap", "res.a", "5", estimate("4", "6", "act.a")),
            cap("cap.a.hard", "res.a", "hard", "block_all_income", value="rule:pt.cap"),
        ]
        entities += [resource("res.a", "fact.a"), activity("act.a")]
        readings["fact.a"] = 0
        goals.append(goal("goal.a", "res.a", "1"))
    return priced(model(folder, [DAY], entities, rules, readings, goals=goals))


def segment_fixture(folder: Path) -> Path:
    """The original proposal's appendix B3, a D* that is a segment. p and q come at 1 a day each; act.set turns 1 p and
    1 q into 1 v, the main goal worth 1. act.j turns `pt.j` v into 1 p and 2 q, pt.j 2 estimated from 3/2 to 3; act.k
    the same for `pt.k` v, 3 estimated from 2 to 4. By hand: V0 = 1, v is worth 1 and p + q = 1 (act.set runs), each
    at least 0: D* is the segment from y_B = (1, 0) to y' = (0, 1) in (p, q). act.j's dual slack is 2 - p - 2q = 1 - q:
    1 at y_B and 0 at y', so no optimum runs it, but C1 fails for its column (`c1`, in both passes). Its value q - 1
    lies in [-1, 0]: K is p, q and act.j, and the max-min gives p = q = 1/2. At pt.j = 3/2 its slack is 1/2 - q, which
    cuts D* to q at most 1/2: there act.j's value q - 1/2 lies in [-1/2, 0], p in [1/2, 1], q in [0, 1/2], and the
    max-min gives p = 3/4, q = 1/4, so refusing that pass is right.

    act.k's slack is 3 - p - 2q = 2 - q, at least 1 on D*: C1 holds (an LP, solved in pt.k's first pass). At pt.k = 2
    its slack is 1 - q, 0 at y': `c2`. At 4 it is 3 - q, at least 2: C2 holds; act.k is in no K (its value q - 2 is
    below 0) and its one column lies in balance rows only, so C3 takes no LP; A has no term: proven, the first pass
    solving C1's and C2's LPs (2), the second C2's alone (1)."""
    rules = [
        regen("regen.p", "res.p", "1", "1d"), regen("regen.q", "res.q", "1", "1d"),
        flow("flow.set", "act.set", consumes={"res.p": "1", "res.q": "1"}, produces={"res.v": "1"}),
        provisional("pt.j", "act.j", "2", estimate("3/2", "3", "act.j")),
        flow("flow.j", "act.j", consumes={"res.v": "rule:pt.j"}, produces={"res.p": "1", "res.q": "2"}),
        provisional("pt.k", "act.k", "3", estimate("2", "4", "act.k")),
        flow("flow.k", "act.k", consumes={"res.v": "rule:pt.k"}, produces={"res.p": "1", "res.q": "2"}),
    ]
    entities = [
        resource("res.p", "fact.p"), resource("res.q", "fact.q"), resource("res.v", "fact.v"), activity("act.set"),
        activity("act.j"), activity("act.k"),
    ]
    return priced(model(folder, [DAY], entities, rules, {"fact.p": 0, "fact.q": 0, "fact.v": 0}))


def in_k_fixture(folder: Path) -> Path:
    """segment_fixture's p, q, v and act.set, and act.k turning `pt.k` v into 1 p and 2 q, pt.k 2 estimated from 3/2 to
    5/2; act.r brings 1 v for nothing, and act.r and act.k share a daily limit of 1. By hand: act.r runs once (V0 = 2)
    and fills the limit, whose dual is then act.r's value, 1; D* is the segment of segment_fixture. act.k's dual slack
    is that dual less its value, 1 - (q - 1) = 2 - q, at least 1 on D*: C1 holds; at the passes 3/2 - q and 5/2 - q, at
    least 1/2: C2 holds. But no optimum runs act.k and its value q - 1 lies in [-1, 0]: it is in the steady state's K,
    so both passes are refused by `c3`."""
    rules = [
        regen("regen.p", "res.p", "1", "1d"), regen("regen.q", "res.q", "1", "1d"),
        flow("flow.set", "act.set", consumes={"res.p": "1", "res.q": "1"}, produces={"res.v": "1"}),
        flow("flow.r", "act.r", produces={"res.v": "1"}),
        limit("limit.rk", "act.r", "1", "day", shared_with=["act.k"]),
        provisional("pt.k", "act.k", "2", estimate("3/2", "5/2", "act.k")),
        flow("flow.k", "act.k", consumes={"res.v": "rule:pt.k"}, produces={"res.p": "1", "res.q": "2"}),
    ]
    entities = [
        resource("res.p", "fact.p"), resource("res.q", "fact.q"), resource("res.v", "fact.v"), activity("act.set"),
        activity("act.r"), activity("act.k"),
    ]
    return priced(model(folder, [DAY], entities, rules, {"fact.p": 0, "fact.q": 0, "fact.v": 0}))


WALKS = ("potion", "stop", "income", "hard")


def walk_fixture(folder: Path, shape: str) -> Path:
    """C4's walk (contract test 10). st comes at 10 a day; act.farm turns 1 st into 1 v, v the main goal worth 1, at
    most 20 times a day: it is protected (st has a regen, a limit counts it), so A has a term. act.alt turns 1 st into 1
    x; act.j turns 4 st into `pt.j` x, pt.j 1 estimated from 1 to 3/2; act.use takes x. By hand: act.farm runs 10
    times, so st and v are worth 1 and x at most 1 (act.alt never runs): act.j's dual slack 4 - pt.j × x's price is at
    least 5/2 on D* at pt.j 1 and 3/2, so C1 and C2 hold; its value is below 0 and its one column lies in balance rows
    only, so C3 takes no LP. The pass at 1 is the steady state's own; the one at 3/2 brings in more x than the steady
    state does, so x is short and its taker act.use is stopped. Then, by `shape`:
    - `potion`: act.use turns 2 x into 1 st (x worth at least 1/2). Stopped, it makes st short, whose taker act.farm, of
      P, is stopped: `c4`;
    - `stop`: act.use takes 1 x and 1 e, e coming at 5 a day by a regen that stops at e's natural cap (reference 50),
      and act.fill turns 1 st into 1 e. Stopped, act.use makes e long, a regen stop that an activity fills too: `c4`;
    - `income`: act.use turns 1 x into 1 w, and an income cap counts what act.use brings of w (3 a day): `c4`;
    - `hard`: `stop`'s shape with e under a hard cap of 50 and its regen plain. e long has a hard cap and no other, x
      (short and long) no cap and no regen, act.farm is not stopped: the walk holds, and the pass is proven."""
    rules = [
        regen("regen.st", "res.st", "10", "1d"),
        flow("flow.farm", "act.farm", consumes={"res.st": "1"}, produces={"res.v": "1"}),
        limit("limit.farm", "act.farm", "20", "day"),
        flow("flow.alt", "act.alt", consumes={"res.st": "1"}, produces={"res.x": "1"}),
        provisional("pt.j", "act.j", "1", estimate("1", "3/2", "act.j")),
        flow("flow.j", "act.j", consumes={"res.st": "4"}, produces={"res.x": "rule:pt.j"}),
    ]
    entities = [
        resource("res.st", "fact.st"), resource("res.v", "fact.v"), resource("res.x", "fact.x"), activity("act.farm"),
        activity("act.alt"), activity("act.j"), activity("act.use"),
    ]
    readings = {"fact.st": 0, "fact.v": 0, "fact.x": 0}
    if shape == "potion":
        rules.append(flow("flow.use", "act.use", consumes={"res.x": "2"}, produces={"res.st": "1"}))
    elif shape == "income":
        rules += [
            flow("flow.use", "act.use", consumes={"res.x": "1"}, produces={"res.w": "1"}),
            cap("cap.w.income", "res.w", "income", "discard", value="3", per="day", **{"from": ["act.use"]}),
        ]
        entities.append(resource("res.w", "fact.w"))
        readings["fact.w"] = 0
    else:
        rules += [
            flow("flow.use", "act.use", consumes={"res.x": "1", "res.e": "1"}),
            flow("flow.fill", "act.fill", consumes={"res.st": "1"}, produces={"res.e": "1"}),
        ]
        entities += [resource("res.e", "fact.e"), activity("act.fill")]
        readings["fact.e"] = 0
        if shape == "stop":
            rules += [
                regen("regen.e", "res.e", "5", "1d", **{"while": "amount:res.e < read:fact.e_cap"}),
                cap("cap.e.natural", "res.e", "soft_read", {"block_income_from": [], "other_income": "notice_only"},
                    reading="fact.e_cap", reference="50"),
            ]
            readings["fact.e_cap"] = 30
        else:
            rules += [
                regen("regen.e", "res.e", "5", "1d"),
                cap("cap.e.hard", "res.e", "hard", "block_all_income", value="50"),
            ]
    return priced(model(folder, [DAY], entities, rules, readings))


def demand_fixture(folder: Path) -> Path:
    """C4's walk reaching a demand row whose `main` is not 0. No main goal; the demand row `unlock` takes 10 m for 2
    points (a point worth 1) and 5 value units for the main goal later, at most twice a day. s comes at 10 a day;
    act.make turns 10 s into 10 m; act.j turns 2 s into `pt.j` m, pt.j 1 estimated from 1 to 3/2. By hand: act.make
    runs once and the row once, V0 = 7 = A* + 2, A the row's `main`; the row's bound is slack, so m is worth 7/10, and
    so is s (act.make runs): act.j's dual slack 7/10 × (2 - pt.j) is above 0 at 1 and 3/2, its value below 0 and its
    column in balance rows only. P is empty in both; at 3/2 act.j brings in more m, so m is short and the row that
    costs it is stopped: its `main` is 5, `c4`."""
    rules = [
        regen("regen.s", "res.s", "10", "1d"),
        flow("flow.make", "act.make", consumes={"res.s": "10"}, produces={"res.m": "10"}),
        provisional("pt.j", "act.j", "1", estimate("1", "3/2", "act.j")),
        flow("flow.j", "act.j", consumes={"res.s": "2"}, produces={"res.m": "rule:pt.j"}),
    ]
    entities = [resource("res.s", "fact.s"), resource("res.m", "fact.m"), activity("act.make"), activity("act.j")]
    return priced(model(
        folder, [DAY], entities, rules, {"fact.s": 0, "fact.m": 0},
        goals=[training("1", "1d")], demand=[("unlock", "2", "2", "5")], costs=[("unlock", {"res.m": "10"})],
    ))


def pair_fixture(folder: Path, dominated: str, running: str) -> Path:
    """One derive in two activities (contract test 11's order). s comes at 10 a day; `running` turns 2 s into `pt.j` v,
    `dominated` turns `pt.j` s into 1 v, pt.j 2 estimated from 3/2 to 3. By hand: `running` runs 5 times, V0 = 10, s
    and v are worth 1, and `dominated`'s dual slack is pt.j - 1 = 1. Each pass moves both columns: J is the two, by
    activity. C1 of `running` fails with no LP (x* runs it); C1 of `dominated` is an LP, which holds."""
    rules = [
        regen("regen.s", "res.s", "10", "1d"),
        provisional("pt.j", running, "2", estimate("3/2", "3", dominated, running)),
        flow("flow.dominated", dominated, consumes={"res.s": "rule:pt.j"}, produces={"res.v": "1"}),
        flow("flow.running", running, consumes={"res.s": "2"}, produces={"res.v": "rule:pt.j"}),
    ]
    entities = [resource("res.s", "fact.s"), resource("res.v", "fact.v"), activity(dominated), activity(running)]
    return priced(model(folder, [DAY], entities, rules, {"fact.s": 0, "fact.v": 0}))


# Helpers ----------------------------------------------------------------------------------------------------------


def pass_vectors(folder: Path, certify: bool) -> tuple[impact._Assessor, list[dict[str, Any]]]:
    """Every pass of the fixture's assessment, as `impact.assess` computes them (its groups and passes, in order), with
    what `impact._Assessor.vector` gives for each: each pool's coordinate (`prices`) and each activity's value at it
    (`values`), `by`, `refused_by` and `timing_ms`; with `certify` as `impact.certify`. And the assessor."""
    graph = graph_of(folder)
    steady = solved(graph, STEADY)
    assessor = impact._Assessor(graph, steady)
    assessor.certify = certify
    checker = graph.checker
    reach = impact.reached(graph, steady, judge(checker))
    estimates = [assessor.estimate(name) for name in sorted(reach) if "estimate" in checker.entries[name].doc["logic"]]
    found = []
    for group in assessor.groups(estimates):
        moved = Moved.base(graph, {item.entry.id for item in group})
        for where, values in assessor.passes(group):
            label = ",".join(f"{name}={point}" for name, point in sorted(where.items()))
            prices, at_values, by, refused_by, timing = assessor.vector(group, label, values, moved)
            found.append({
                "at": where, "prices": prices, "values": at_values, BY: by, REFUSED_BY: refused_by, TIMING: timing,
            })
    return assessor, found


def decided(found: list[dict[str, Any]]) -> list[tuple[dict[str, str], str, str | None]]:
    """Each pass's `at`, `by` and `refused_by`."""
    return [(each["at"], each[BY], each[REFUSED_BY]) for each in found]


def certificate_runs(found: list[dict[str, Any]]) -> list[int]:
    """How many LPs HiGHS ran for each pass's certificate (`lps.certificate.runs`)."""
    return [each[TIMING]["lps"]["certificate"]["runs"] for each in found]


def stripped(doc: dict[str, Any]) -> dict[str, Any]:
    """prices.json without what records how the run found it (contracts/formats.md, "Timing", "Taking an assessment
    over"): each pass's `by`, `refused_by` and `timing_ms`, `steady_timing_ms` and `reused_assessment`."""
    taken_out = (BY, REFUSED_BY, TIMING)
    groups = [
        {**group, "passes": [{k: v for k, v in each.items() if k not in taken_out} for each in group["passes"]]}
        for group in doc["impact"]
    ]
    return {**{k: v for k, v in doc.items() if k not in ("steady_timing_ms", "reused_assessment")}, "impact": groups}


def counted_lps(monkeypatch: pytest.MonkeyPatch, method: str, fail: Callable[[Any, int], Any]) -> list[int]:
    """`flow._Solver.<method>` (`vertex` or `lp`) of the certificate's LPs (stage `certificate`) replaced as `fail`
    gives it, by the place of the LP among the certificate's LPs of the run, from 1 (`fail(original, place)` returns the
    method to call, the original or another). Returns the places seen."""
    original = getattr(economy_flow._Solver, method)
    seen: list[int] = []

    def counted(solver: Any, *args: Any, **options: Any) -> Any:
        if args[-1 if method == "vertex" else 1] != economy_flow.CERTIFICATE:
            return original(solver, *args, **options)
        seen.append(len(seen) + 1)
        return fail(original, seen[-1])(solver, *args, **options)

    monkeypatch.setattr(economy_flow._Solver, method, counted)
    return seen


# Contract test 6: a pass that no optimum runs is proven, and is the full solve's -------------------------------------


def test_an_activity_no_optimum_runs_is_proven_in_both_passes_and_is_what_solving_finds(tmp_path):
    """dominated_fixture: pt.j's two passes are `certified`, `refused_by` null. With `impact.certify` false both are
    solved, and each pass's pools and activities, act.j (the J activity, computed at the pass's amounts: 1 - 3/2 and
    1 - 3) among them, are the certified pass's, coordinate by coordinate. The certificate's dual rows of a program are
    its dual's (`_dual_rows`, C2's)."""
    folder = dominated_fixture(tmp_path)
    assessor, proven = pass_vectors(folder, True)
    _, full = pass_vectors(folder, False)

    assert decided(proven) == [({"pt.j": "low"}, "certified", None), ({"pt.j": "high"}, "certified", None)]
    assert decided(full) == [({"pt.j": "low"}, "solved", None), ({"pt.j": "high"}, "solved", None)]
    for mine, theirs in zip(proven, full):
        assert (mine["prices"], mine["values"]) == (theirs["prices"], theirs["values"])
    assert [each["values"]["act.j"] for each in proven] == [Fraction(-1, 2), Fraction(-2)]
    assert proven[0]["prices"] == assessor.prices == {"res.s": 1, "res.v": 1}
    held = assessor.held
    columns = tuple(sorted(held.dual.by_column))
    assert economy_flow._dual_rows(held.program, held.dual.duals, columns) == {
        column: held.dual.by_column[column] for column in columns
    }


# Contract test 7: a D* of two ends, slack at one and tight at the other ------------------------------------------


def test_a_column_slack_at_one_end_of_d_star_and_tight_at_the_other_is_refused(tmp_path):
    """segment_fixture: pt.j's two passes are refused by `c1` and solved, and at 3/2 the solved vector is not the steady
    state's (p 3/4, q 1/4, against 1/2 and 1/2): refusing it is right. pt.k's pass at 2 holds C1 but not C2: `c2`,
    solved; at 4 it is proven, and its vector is the one solving it finds."""
    folder = segment_fixture(tmp_path)
    assessor, found = pass_vectors(folder, True)
    _, full = pass_vectors(folder, False)

    assert decided(found) == [
        ({"pt.j": "low"}, "solved", "c1"), ({"pt.j": "high"}, "solved", "c1"),
        ({"pt.k": "low"}, "solved", "c2"), ({"pt.k": "high"}, "certified", None),
    ]
    assert assessor.prices == {"res.p": Fraction(1, 2), "res.q": Fraction(1, 2), "res.v": 1}
    assert found[0]["prices"] == {"res.p": Fraction(3, 4), "res.q": Fraction(1, 4), "res.v": 1}
    assert (found[3]["prices"], found[3]["values"]) == (full[3]["prices"], full[3]["values"])


# Contract test 8: an activity at its limit ------------------------------------------------------------------------


def test_an_activity_that_runs_at_its_limit_is_refused_by_c1_with_no_lp(tmp_path, acmodel):
    """two_parts with b estimated from 2 to 4 (test_prices.py): act.b runs once a day, its limit's whole count, and x*
    runs it, so C1 fails for its column with no LP: both passes are solved, refused by `c1`, and the certificate ran no
    LP for either."""
    folder = two_parts(tmp_path, ESTIMATE_B)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = passes(delivered(folder))
    assert [(each[BY], each[REFUSED_BY]) for each in found] == [("solved", "c1"), ("solved", "c1")]
    assert all(n == 0 for each in found for n in each[TIMING]["lps"]["certificate"].values())


# Contract test 9: C3 -----------------------------------------------------------------------------------------------


def test_a_j_activity_in_k_is_refused_by_c3_and_one_in_balance_rows_only_takes_no_lp_of_c3(tmp_path):
    """in_k_fixture: act.k is in the steady state's K, C1 and C2 hold: both passes are solved, refused by `c3`.
    segment_fixture's pt.k at 4 is proven with C2's LP alone (1): act.k, every step's column of which is in J and lies
    in balance rows only, takes no LP of C3 (reading 2); its first pass, refused by C2, ran C1's and C2's (2)."""
    _, in_k = pass_vectors(in_k_fixture(tmp_path / "in_k"), True)
    _, segment = pass_vectors(segment_fixture(tmp_path / "segment"), True)

    assert decided(in_k) == [({"pt.k": "low"}, "solved", "c3"), ({"pt.k": "high"}, "solved", "c3")]
    assert [each[BY] for each in segment[2:]] == ["solved", "certified"]
    assert certificate_runs(segment)[2:] == [2, 1]


# Contract test 10: C4 ----------------------------------------------------------------------------------------------


@pytest.mark.parametrize("shape", WALKS)
def test_c4_holds_without_a_term_of_a_and_refuses_each_walk_that_fails(tmp_path, shape):
    """dominated_fixture: A has no term, so C4 holds and the passes are proven. walk_fixture (and demand_fixture, with
    `potion`): the pass at 3/2 is refused by `c4` where the walk reaches P's input (`potion`), a regen stop another
    activity fills (`stop`) or an income cap (`income`), or a demand row whose `main` is not 0 (demand_fixture); with
    only a hard cap on e (`hard`) it is proven, and its vector is the one solving it finds. The pass at 1 is the steady
    state's own."""
    dominated, proven = pass_vectors(dominated_fixture(tmp_path / "dominated"), True)
    assert dominated.held.acquisition == {}
    assert [each[BY] for each in proven] == ["certified", "certified"]

    folder = walk_fixture(tmp_path / shape, shape)
    assessor, found = pass_vectors(folder, True)

    assert assessor.held.protected == ("act.farm",) and assessor.held.acquisition
    if shape == "hard":
        assert decided(found) == [({"pt.j": "low"}, "steady_state", None), ({"pt.j": "high"}, "certified", None)]
        _, full = pass_vectors(folder, False)
        assert full[1][BY] == "solved"
        assert (found[1]["prices"], found[1]["values"]) == (full[1]["prices"], full[1]["values"])
    else:
        assert decided(found) == [({"pt.j": "low"}, "steady_state", None), ({"pt.j": "high"}, "solved", "c4")]
    if shape == "potion":
        _, rows = pass_vectors(demand_fixture(tmp_path / "demand"), True)
        assert decided(rows) == [({"pt.j": "low"}, "steady_state", None), ({"pt.j": "high"}, "solved", "c4")]


# Contract test 11: an LP the certificate cannot read refuses it ---------------------------------------------------


def test_an_lp_of_the_certificate_that_is_not_certified_or_fails_refuses_it_and_nothing_warns(
    tmp_path, acmodel, monkeypatch
):
    """dominated_fixture, its certificate's LPs, in order: C1's and C2's in the first pass, C2's in the second.
    - C1's LP at the pivot cap (`exact.Refused`, `pivot_cap`): both passes solved, refused by `lp_unverified` (the
      second reads C1's answer of the column); the export succeeds with no warning;
    - the first pass's C2 LP failing (`lp_solve_failed`): that pass is refused by `lp_unverified`, the second, whose C2
      LP is solved, proven."""
    folder = dominated_fixture(tmp_path)

    with monkeypatch.context() as patched:
        def capped(original: Any, place: int) -> Any:
            if place != 1:
                return original

            def refused(solver: Any, *args: Any) -> Any:
                x, _ = original(solver, *args)
                return x, exact.Refused(exact.PIVOT_CAP, 20)

            return refused

        seen = counted_lps(patched, "vertex", capped)
        status, doc = export(acmodel, folder)
    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert seen == [1]
    assert [(each[BY], each[REFUSED_BY]) for each in passes(delivered(folder))] == [
        ("solved", "lp_unverified"), ("solved", "lp_unverified"),
    ]

    with monkeypatch.context() as patched:
        def failing(original: Any, place: int) -> Any:
            if place != 2:
                return original

            def failed(solver: Any, program: Any, stage: str, **options: Any) -> Any:
                raise solver.failure(LP_SOLVE_FAILED, stage=stage, reason="solver_status", status=4)

            return failed

        seen = counted_lps(patched, "lp", failing)
        status, doc = export(acmodel, folder)
    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert seen == [1, 2, 3]
    assert [(each[BY], each[REFUSED_BY]) for each in passes(delivered(folder))] == [
        ("solved", "lp_unverified"), ("certified", None),
    ]


@pytest.mark.parametrize(
    ("dominated", "running", "refused_by", "runs"),
    [
        pytest.param("act.a", "act.b", "lp_unverified", [1], id="unverified_first"),
        pytest.param("act.b", "act.a", "c1", [], id="failing_first"),
    ],
)
def test_the_first_column_in_order_decides_between_an_unverified_lp_and_a_condition_that_fails(
    tmp_path, monkeypatch, dominated, running, refused_by, runs
):
    """pair_fixture, the C1 LP of the dominated activity's column at the pivot cap. When that column comes first (by
    activity), its unverified LP decides: `lp_unverified`, before the running activity's column, which fails C1. When
    the running activity's column comes first, it fails C1 with no LP, which decides: `c1`, and the LP is never solved.
    Both passes alike (the second reads C1's answers)."""

    def capped(original: Any, place: int) -> Any:
        def refused(solver: Any, *args: Any) -> Any:
            x, _ = original(solver, *args)
            return x, exact.Refused(exact.PIVOT_CAP, 20)

        return refused

    seen = counted_lps(monkeypatch, "vertex", capped)
    _, found = pass_vectors(pair_fixture(tmp_path, dominated, running), True)

    assert [(each[BY], each[REFUSED_BY]) for each in found] == [("solved", refused_by)] * 2
    assert seen == runs


# Contract test 12: `by` and `refused_by` -------------------------------------------------------------------------


def test_each_pass_says_how_it_was_found_and_what_refused_it(tmp_path, acmodel, write_config):
    """dominated_fixture with `mixed`: the three kinds of `by` in one fixture. With `impact.certify` true (the default)
    every pass has `refused_by`: `program` for each pass of the cap's value, null for pt.j's two certified passes and
    for pt.p's at its point value (`steady_state`), `c1` for pt.p's pass at 2, solved. With it false no pass has the
    member, and pt.j's passes are solved."""
    folder = dominated_fixture(tmp_path, mixed=True)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = passes(delivered(folder))
    assert [(each["at"], each[BY], each[REFUSED_BY]) for each in found] == [
        ({"pt.cap": "low"}, "solved", "program"), ({"pt.cap": "high"}, "solved", "program"),
        ({"pt.j": "low"}, "certified", None), ({"pt.j": "high"}, "certified", None),
        ({"pt.p": "low"}, "steady_state", None), ({"pt.p": "high"}, "solved", "c1"),
    ]
    assert all(each[REFUSED_BY] in SIX for each in found if each[BY] == "solved")

    write_config(UNCERTIFIED)
    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = passes(delivered(folder))
    assert all(REFUSED_BY not in each for each in found)
    assert [each[BY] for each in found] == ["solved", "solved", "solved", "solved", "steady_state", "solved"]


# Contract test 13 and N7: a certified pass's timing, and the certificate's LPs counted -----------------------------


def test_a_certified_passs_timing_holds_the_certificates_lps_alone_counted_where_they_ran(
    tmp_path, acmodel, monkeypatch
):
    """dominated_fixture's two certified passes, each HiGHS run recorded as it happens (test_lp_counts.py's `Recorded`):
    each `timing_ms` is `{stages: null, lps, total}`, `lps` the thirteen members, each with the five integers, and only
    `certificate` holds a run: the first pass's C1 and C2 LPs (2), the second's C2 LP (1): C1's column is in both
    passes' J and its LP counts in the first only (reading 3). Each is the number of runs recorded on that pass's
    certificate clock (`CERTIFICATE_STAGES`)."""
    folder = dominated_fixture(tmp_path)
    recorded = Recorded(monkeypatch)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    found = passes(delivered(folder))
    for each in found:
        timing = each[TIMING]
        assert (each[BY], each[REFUSED_BY]) == ("certified", None)
        assert sorted(timing) == ["lps", "stages", "total"] and timing["stages"] is None
        assert type(timing["total"]) is int
        assert shaped(timing["lps"], PASS_LPS)
        assert all(n == 0 for stage, entry in timing["lps"].items() if stage != "certificate" for n in entry.values())
    assert certificate_runs(found) == [2, 1]
    assert recorded.clocks(economy_flow.CERTIFICATE_STAGES) == [{"certificate": 2}, {"certificate": 1}]
    assert recorded.since == 0


def test_a_second_try_of_a_certificates_lp_is_counted_there_and_the_certificate_goes_on(tmp_path, acmodel, monkeypatch):
    """dominated_fixture, the certificate's first LP (C1's, in the first pass) answered with status 4 once (test_dstar_
    retry.py's `Highs`): it is solved again without presolve, the very next run, which answers. That pass's
    `lps.certificate` has one run more (3) and `again_other` 1, `again_infeasible` 0; both passes are still proven, and
    the file is the plain export's but for its timing."""
    folder = dominated_fixture(tmp_path)
    export(acmodel, folder)
    plain = delivered(folder)
    lp = economy_flow._Solver.lp
    seen: dict[str, Any] = {}

    with monkeypatch.context() as patched:
        highs = Highs(patched)

        def armed(solver: Any, program: Any, stage: str, **options: Any) -> Any:
            if not seen and solver.clock is not None and solver.clock.stages == economy_flow.CERTIFICATE_STAGES:
                seen["first"] = len(highs.runs)
                highs.answers[:] = [(4, "Solve error")]
            return lp(solver, program, stage, **options)

        patched.setattr(economy_flow._Solver, "lp", armed)
        status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert highs.forced == [seen["first"]] and highs.runs[seen["first"] + 1] is False
    found = delivered(folder)
    first, second = (each[TIMING]["lps"]["certificate"] for each in passes(found))
    assert (first["runs"], first["again_other"], first["again_infeasible"]) == (3, 1, 0)
    assert (second["runs"], second["again_other"], second["again_infeasible"]) == (1, 0, 0)
    assert [each[BY] for each in passes(found)] == ["certified", "certified"]
    assert stripped(found) == stripped(plain)


# Contract tests 14 and 15: the two settings of `impact.certify` --------------------------------------------------


def test_the_two_settings_export_the_same_files_but_for_how_each_pass_was_found(tmp_path, acmodel, write_config):
    """dominated_fixture with `mixed` exported with `impact.certify` true, then false: without each pass's `by`,
    `refused_by` and `timing_ms` and `steady_timing_ms`, the two prices.json are the same in canonical JSON (JCS) byte
    for byte, and so is every price's provenance file; the evaluator digest is the same."""
    folder = dominated_fixture(tmp_path, mixed=True)
    out = base(folder) / "out"
    export(acmodel, folder)
    certified = snapshot(out)
    write_config(UNCERTIFIED)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    solved_all = snapshot(out)
    assert sorted(certified) == sorted(solved_all)
    first, second = json.loads(certified["prices.json"]), json.loads(solved_all["prices.json"])
    assert [each[BY] for each in passes(first)] != [each[BY] for each in passes(second)]
    assert jcs.dumps(stripped(first)) == jcs.dumps(stripped(second))
    assert first["evaluator"] == second["evaluator"]
    for name in certified:
        if name != "prices.json":
            assert certified[name] == solved_all[name], name


def test_the_setting_is_not_in_the_evaluator_and_an_assessment_is_taken_over_across_it(tmp_path, acmodel, write_config):
    """`impact.certify` never changes a result: it is listed in `NEUTRAL`, not in the digest. dominated_fixture with
    `mixed` exported with it true and kept aside; with it false, the full export and the one that takes the kept
    assessment over: the groups taken over are the kept ones, `refused_by` and all, and without each pass's `by`,
    `refused_by` and `timing_ms`, `steady_timing_ms` and `reused_assessment`, the file is the full export's. Then the
    other way: the false export kept, taken over with it true: its passes have no `refused_by`."""
    assert "impact.certify" in evaluator.NEUTRAL and "impact.certify" not in evaluator.keys()
    folder = dominated_fixture(tmp_path, mixed=True)
    out, kept_true, kept_false = base(folder) / "out", tmp_path / "kept_true", tmp_path / "kept_false"
    export(acmodel, folder)
    shutil.copytree(out, kept_true)

    write_config(UNCERTIFIED)
    export(acmodel, folder)
    full_false = json.loads((out / "prices.json").read_bytes())
    shutil.copytree(out, kept_false)
    status, doc = reuse(acmodel, folder, kept_true)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    taken = json.loads((out / "prices.json").read_bytes())
    earlier = json.loads((kept_true / "prices.json").read_bytes())
    assert taken["impact"] == earlier["impact"]
    assert all(REFUSED_BY in each for each in passes(taken))
    assert stripped(taken) == stripped(full_false)

    write_config({"schema_version": "acmodel.config.v1", "impact": {"certify": True}})
    export(acmodel, folder)
    full_true = json.loads((out / "prices.json").read_bytes())
    status, doc = reuse(acmodel, folder, kept_false)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    taken = json.loads((out / "prices.json").read_bytes())
    assert taken["impact"] == json.loads((kept_false / "prices.json").read_bytes())["impact"]
    assert all(REFUSED_BY not in each for each in passes(taken))
    assert stripped(taken) == stripped(full_true)


# Contract test 17: a pass whose J is empty ------------------------------------------------------------------------


def test_a_pass_whose_j_is_empty_is_proven_with_no_lp(tmp_path):
    """moves_fixture (test_method_c.py): pt.idle reaches no coefficient of S3's program, so a pass that moves it has an
    empty J: `certified`, `refused_by` null, the steady state's vector and values, and its `timing_ms` has no stage and
    no run at all (`certificate` too)."""
    graph = graph_of(moves_fixture(tmp_path))
    assessor = impact._Assessor(graph, solved(graph, STEADY))
    item = impact._Estimate(graph.checker.entries["pt.idle"], impact.LOW, None)

    prices, values, by, refused_by, timing = assessor.vector(
        [item], "pt.idle=high", {"pt.idle": Fraction(6)}, Moved.base(graph, {"pt.idle"})
    )

    assert (by, refused_by) == ("certified", None)
    assert (prices, values) == (assessor.prices, assessor.values)
    assert timing["stages"] is None and shaped(timing["lps"], PASS_LPS)
    assert all(n == 0 for entry in timing["lps"].values() for n in entry.values())
    assert assessor.answers == {}


# N9: the configuration key ----------------------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("given", "detail", "ref"),
    [
        pytest.param({"certify": "true"}, {"reason": "value_invalid"}, "/impact/certify", id="text"),
        pytest.param({"certify": 1}, {"reason": "value_invalid"}, "/impact/certify", id="integer"),
        pytest.param({"certify": False, "other": True}, {"reason": "member_unknown", "key": "other"}, "/impact/other",
                     id="other_key"),
    ],
)
def test_impact_certify_is_a_boolean_true_by_default_and_impact_has_no_other_key(
    write_config, local_app_data, app_dir, given, detail, ref
):
    """`impact.certify` is true without a file and as given when a boolean; text or a number is `config_invalid`
    (`value_invalid`, at `/impact/certify`), and another key under `impact` `member_unknown` (at `/impact/<key>`)."""
    environ = {"LOCALAPPDATA": str(local_app_data)}
    assert config.load(environ).impact_certify is True
    for value in (False, True):
        write_config({"schema_version": "acmodel.config.v1", "impact": {"certify": value}})
        assert config.load(environ).impact_certify is value
    write_config({"schema_version": "acmodel.config.v1", "impact": given})

    with pytest.raises(Failure) as raised:
        config.load(environ)

    assert raised.value.errors == [
        {"code": "config_invalid", "detail": {**detail, "path": str(app_dir / "config.json")}, "ref": ref}
    ]


# N10: what a vector could not verify, beside its value -----------------------------------------------------------


def test_a_vectors_unverified_probes_are_beside_its_value_and_the_cache_keeps_what_it_kept(tmp_path, monkeypatch):
    """flips_fixture's vector, computed now and as before METHOD-C C-1 (test_method_c.py's `as_before`): equal, kept by
    the cache's encoding byte for byte alike; the economy's evaluator version is 12. Computed now it holds, beside its
    value, the activities whose probes it could not verify: none (`{}`); as before, nothing. With each `most` probe of
    the vector made unverified (act.p and act.q are probed: both are worth 0 at pt.k's point, and no projection spares
    them; whichever an optimum runs is in no K either way), its value and encoding are the same, and beside them it
    lists act.p and act.q, each with `most`."""
    folder = flips_fixture(tmp_path)
    graph = graph_of(folder)
    vector = solved(graph, VECTOR)
    with monkeypatch.context() as patched:
        as_before_c1(patched)
        earlier = graph_of(folder)
        was = solved(earlier, VECTOR)
    probe = economy_flow._Solver.probe

    def unverified(solver: Any, program: Any, stage: str, **options: Any) -> Any:
        if stage == economy_flow.JUDGEMENT and solver.clock is not None and solver.clock.stages == economy_flow.STAGES:
            return economy_flow._Unverified({"reason": exact.PIVOT_CAP, "stage": stage})
        return probe(solver, program, stage, **options)

    with monkeypatch.context() as patched:
        patched.setattr(economy_flow._Solver, "probe", unverified)
        doubting = graph_of(folder)
        doubted = solved(doubting, VECTOR)

    assert not vector.blocked and not doubted.blocked
    assert vector == was == doubted
    assert canonical(graph.encode(vector)) == canonical(earlier.encode(was)) == canonical(doubting.encode(doubted))
    assert (vector.unverified, was.unverified) == ({}, None)
    assert doubted.unverified == {"act.p": ("most",), "act.q": ("most",)}
    assert EVALUATORS["economy"].version == 12


# N11: a pass's program that is blocked ----------------------------------------------------------------------------


def test_a_blocked_program_of_a_pass_is_its_refusal_and_nothing_is_written(tmp_path, acmodel, app_dir, monkeypatch):
    """dominated_fixture, with the program of each pass built blocked (`flow.build_program`, by an error of the model's
    S3): the first pass refuses the export (`export_refused`, `impact_blocked`, at pt.j's estimate, the pass named),
    with the error that blocks it, needed by pt.j. Nothing is written: no out folder, no shared tree."""
    folder = dominated_fixture(tmp_path)
    failed = error(LP_SOLVE_FAILED, {"path": model_path(folder), "solve": "steady", "stage": "optimum",
                                     "reason": "no_solution"})
    monkeypatch.setattr(
        economy_flow, "build_program", lambda graph: economy_flow.Built(None, None, graph.register([failed]))
    )

    status, doc = export(acmodel, folder)

    assert status == 5
    refusal = {"path": rules_path(folder), "output": "rule:pt.j", "reason": "impact_blocked", "value": "pt.j=low"}
    assert doc["errors"] == [
        {"code": "export_refused", "detail": refusal, "ref": "/rules/2/logic/estimate"},
        {**failed, "needed_by": ["rule:pt.j"]},
    ]
    assert not (base(folder) / "out").exists()
    assert snapshot(app_dir / "state" / "provenance") == {}


# The six words ----------------------------------------------------------------------------------------------------


def test_the_conditions_the_assessment_writes_are_those_the_reader_takes():
    """What refused a pass is one of the six conditions, the same words `core.flow` decides, `core.impact` writes and
    `readers.check_assessment` reads back."""
    assert list(economy_flow.CONDITIONS) == list(impact.CONDITIONS) == list(readers.REFUSED_BY) == SIX
    assert impact.REFUSED_BY == REFUSED_BY and impact.CERTIFICATE == economy_flow.CERTIFICATE == "certificate"
