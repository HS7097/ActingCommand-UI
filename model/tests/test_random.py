"""PY-11: random processes (#424 design 4.5, design-py 4.5). The exact pity chain on the closed-form cases (hard
pity 2 and 3), its carry-over transitions and its spark; the expected pulls as exact rationals, also as a random
rule's value in a run, and carried as its enclosure when it exceeds `expr.max_bits`; and the Wilson bounds of a
rate from samples, equal to hand-computed rationals for given (n, k, z), from the function and in a run (a
`beta_from_samples` unknown and a drop from samples)."""

from __future__ import annotations

import json
import math
import shutil
from fractions import Fraction as F
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli
from acmodel.core import random as chains
from acmodel.core.random import FRESH, State

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
RULES = "ours/model/rules/a.json"
R = F(1, 10)


def test_hard_pity_2_and_3_have_their_closed_forms():
    two, three = chains.chain(R, 2), chains.chain(R, 3)

    assert chains.pulls(two, 2).target == (R, 1 - R)
    assert chains.expected(two) == 2 - R == F(19, 10)
    assert chains.pulls(three, 3).target == (R, (1 - R) * R, (1 - R) ** 2)
    assert chains.expected(three) == 1 + (1 - R) + (1 - R) ** 2 == F(271, 100)
    assert chains.pulls(three, 3).left == ()  # the hard pity ends every chain


def test_carry_over_transitions():
    carried = chains.chain(F(1, 2), 3, p_up=F(1, 2), guarantee_after_miss=True, carry_over=True)
    fresh = chains.chain(F(1, 2), 3, p_up=F(1, 2), guarantee_after_miss=True, carry_over=False)

    # One pull: the target (top-tier, then the target), a miss that guarantees the next top-tier, or no top-tier.
    first = chains.pulls(carried, 1)
    assert first.target == (F(1, 4),)
    assert first.left == ((State(0, True), F(1, 4)), (State(1, False), F(1, 2)))
    assert first.carried == ((FRESH, F(1, 4)), (State(0, True), F(1, 4)), (State(1, False), F(1, 2)))
    assert chains.pulls(fresh, 1).carried == ((FRESH, F(1)),)
    # From a carried pity count of 2, the next pull is surely top-tier; after its miss, the guarantee holds.
    later = chains.pulls(carried, 4, State(2, False))
    assert later.target == (F(1, 2), F(1, 4), F(1, 8), F(1, 8))
    assert chains.expected(carried, State(2, False)) == F(15, 8)
    assert chains.expected(carried, State(0, True)) == 1 + F(1, 2) + F(1, 4)


def test_spark_takes_the_target_by_exchange():
    chain = chains.chain(F(1, 2), 5, p_up=F(1, 2), spark=2, carry_over=True)

    found = chains.pulls(chain, 2)
    assert found.target == (F(1, 4), F(3, 4))  # pull 2: 3/16 from a top-tier pull, the other 9/16 by exchange
    assert found.left == ()
    # The exchange leaves the pity count of its pull; a top-tier pull leaves a fresh state.
    assert found.carried == ((FRESH, F(5, 8)), (State(1, False), F(1, 8)), (State(2, False), F(1, 4)))
    assert chains.expected(chain) == F(7, 4)
    never = chains.chain(F(0), 10, spark=3, carry_over=True)
    assert chains.pulls(never, 3).target == (0, 0, 1)
    assert chains.pulls(never, 3).carried == ((State(3, False), F(1)),)
    assert chains.expected(never) == 3


def test_expected_pulls_are_exact_rationals(tmp_path, make_system, app_dir, write_config, check_envelope):
    guaranteed = chains.chain(R, 2, p_up=F(1, 2), guarantee_after_miss=True)
    target = chains.pulls(guaranteed, 4).target
    assert target == (F(1, 20), F(91, 200), F(9, 100), F(81, 200))
    assert chains.expected(guaranteed) == sum(n * p for n, p in enumerate(target, 1)) == F(57, 20)
    unguaranteed = chains.chain(R, 2, p_up=F(1, 2))
    assert chains.expected(unguaranteed) == F(19, 5)  # 2 - R per top-tier pull, half of them the target
    partial = sum(n * p for n, p in enumerate(chains.pulls(unguaranteed, 80).target, 1))
    assert 0 < F(19, 5) - partial < F(1, 1000)

    folder = model(tmp_path, entities=["act.pull"], rules=[rule("pity.a", "act.pull", {"pity_chain": {
        "rate": "0.1", "hard_pity": "2", "p_up": "1 / 2", "guarantee_after_miss": True, "carry_over": True,
    }})])
    status, doc = run(folder, {}, make_system, app_dir, write_config, check_envelope)

    assert status == 0
    assert doc["data"]["rules"]["pity.a"] == {
        "kind": "random", "status": "confirmed", "type": "count", "per_account": False, "value": "57/20",
    }


def test_a_value_beyond_max_bits_is_carried_as_its_enclosure(
    tmp_path, make_system, app_dir, write_config, check_envelope
):
    # The 80-pity chain at rate 6/1000, 50/50 with a guarantee: 3/2 of a cycle of sum(0.994**i, i < 80) pulls.
    exact = F(3, 2) * (1 - F(994, 1000) ** 80) / F(6, 1000)
    assert exact.denominator.bit_length() > 127  # beyond the default expr.max_bits
    scale = 10**12  # random.enclosure_digits
    low, high = F(math.floor(exact * scale), scale), F(math.ceil(exact * scale), scale)
    assert low < exact < high

    folder = model(tmp_path, entities=["act.pull"], rules=[
        rule("pity.long", "act.pull", {"pity_chain": {
            "rate": "0.006", "hard_pity": "80", "p_up": "1 / 2", "guarantee_after_miss": True, "carry_over": True,
        }}),
        derive("pulls.two", "rule:pity.long * 2"),
    ])
    status, doc = run(folder, {}, make_system, app_dir, write_config, check_envelope)

    assert status == 0
    rules = doc["data"]["rules"]
    assert rules["pity.long"] == {
        "kind": "random", "status": "confirmed", "type": "count", "per_account": False,
        "range": {"low": str(low), "high": str(high), "exact": False},
    }
    assert rules["pulls.two"] == {  # computed from the enclosure, not refused as expr_overflow
        "kind": "derive", "status": "confirmed", "type": "count", "per_account": False,
        "range": {"low": str(2 * low), "high": str(2 * high), "exact": False},
    }
    status, doc = run(folder, {}, make_system, app_dir, write_config, check_envelope, output="rule:pity.long")
    assert status == 0
    assert doc["data"]["provenance"]["nodes"]["rule:pity.long"]["exact_values"] == [{"unknowns": {}, "value": str(exact)}]


@pytest.mark.parametrize(
    ("samples", "successes", "z", "digits", "low", "high"),
    [
        (10, 5, "2", 6, F(116369, 500000), F(383631, 500000)),  # 1/2 -+ sqrt(1/14)
        (4, 0, "2", 6, F(0), F(1, 2)),  # 1/4 -+ 1/4: exact, nothing to round
        (4, 4, "2", 6, F(1, 2), F(1)),
        (100, 10, "1.959964", 6, F(55229, 1000000), F(87183, 500000)),  # 0.0552291368..., 0.1743656622...
        (100, 10, "1.959964", 2, F(1, 20), F(9, 50)),
    ],
)
def test_wilson_bounds_equal_hand_computed_rationals(samples, successes, z, digits, low, high):
    assert chains.wilson(samples, successes, F(z), digits) == (low, high)


def test_wilson_bounds_of_rates_from_samples_in_a_run(tmp_path, make_system, app_dir, write_config, check_envelope):
    folder = model(
        tmp_path,
        entities=["act.drop"],
        rules=[derive("rate.b", "unknown:unk.b"), rule("drop.b", "act.drop", {"drop": {"samples": 10, "successes": 5}})],
        unknowns=[{
            "id": "unk.b", "kind": "distribution", "question": "fixture", "status": "provisional",
            "distribution": {"family": "beta_from_samples", "samples": 10, "successes": 5},
            "provenance": [{"kind": "pending_check", "ref": "fixture"}],
        }],
    )
    status, doc = run(folder, {"drop_z": "2", "wilson_digits": 6}, make_system, app_dir, write_config, check_envelope)

    assert status == 0
    enclosure = {"low": "116369/500000", "high": "383631/500000", "exact": True}
    rules = doc["data"]["rules"]
    assert rules["rate.b"] == {
        "kind": "derive", "status": "provisional", "type": "none", "per_account": False, "range": enclosure,
        "sensitivity": [{"unknown": "unk.b", "impact": "267/500"}],
    }
    assert rules["drop.b"] == {
        "kind": "random", "status": "provisional", "type": "none", "per_account": False, "range": enclosure,
        "sensitivity": [{"unknown": "drop.b", "impact": "267/500"}],
    }


# A model: the worked example with neutral additions --------------------------------------------------------


def rule(rule_id: str, subject: str, logic: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": rule_id, "kind": "random", "subject": subject, "logic": logic,
        "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed",
    }


def derive(rule_id: str, value: str) -> dict[str, Any]:
    return {
        "id": rule_id, "kind": "derive", "subject": "res.a", "logic": {"value": value},
        "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed",
    }


def model(tmp_path: Path, *, entities=(), rules=(), unknowns=()) -> Path:
    folder = tmp_path / "example"
    shutil.copytree(EXAMPLE, folder)
    path = folder / RULES
    doc = json.loads(path.read_text(encoding="utf-8"))
    doc["entities"].extend({"kind": "activity", "id": entity} for entity in entities)
    doc["rules"].extend(rules)
    doc["unknowns"].extend(unknowns)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")
    return folder


def run(folder: Path, unknowns: dict[str, Any], make_system, app_dir, write_config, check_envelope, output=None):
    """`acmodel run` on the model, or `explain` of `output`."""
    write_config({
        "schema_version": "acmodel.config.v1", "paths": {"state_root": str(folder.parent / "state")},
        "unknowns": unknowns,
    })
    system = make_system(prefix=app_dir / "venv")
    command = ["run"] if output is None else ["explain"]
    argv = [*command, str(folder / "ours" / "model"), *([output] if output else []), "--state",
            str(folder / "state" / "acct-1.json")]
    status = cli.main(argv, system)
    return status, check_envelope(system.stdout.getvalue(), status)
