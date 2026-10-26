"""PY-8: unknowns (#424 design 3.5, 4.5 and 4.8) on the worked example (design section 3.8) and neutral
additions to it: the range and the top unknown of `loss.collect_at_redline`; an unknown without effect at
milli leaves its output confirmed; the exact interval cases; and sampled results that are the same for
chunk sizes 1 and 4 and under two string-hash seeds."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from fractions import Fraction
from pathlib import Path
from typing import Any

import pytest

from acmodel import cli, environment

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
MODEL = "ours/model"
RULES = "ours/model/rules/a.json"
EXPORTS = "ours/model/exports.json"
STATE = "state/acct-1.json"


def derive(rule_id: str, value: str, *unknowns: str) -> dict[str, Any]:
    return {
        "id": rule_id, "kind": "derive", "subject": "res.a", "logic": {"value": value}, "unknowns": list(unknowns),
        "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "confirmed",
    }


def unknown(unknown_id: str, kind: str, form: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": unknown_id, "kind": kind, "question": "fixture", kind: form, "status": "provisional",
        "provenance": [{"kind": "pending_check", "ref": "fixture"}],
    }


INTERVALS = [
    unknown("unk.i", "interval", {"low": "0.1", "high": "0.2"}),
    unknown("unk.j", "interval", {"low": "-1", "high": "2"}),
    unknown("unk.k", "interval", {"low": "3", "high": "4"}),
    unknown("unk.t", "interval", {"low": "1h", "high": "2h"}),
    unknown("unk.reversed", "interval", {"low": "2", "high": "1"}),
]
DISTRIBUTIONS = [
    unknown("unk.u", "distribution", {"family": "uniform", "low": "1", "high": "3"}),
    unknown("unk.tri", "distribution", {"family": "triangular", "low": "0", "mode": "1", "high": "4"}),
    unknown("unk.e", "distribution", {"family": "empirical", "values": ["5", "2", "9"]}),
]
SAMPLED = [
    derive("mc.mix", "unknown:unk.u + unknown:unk.tri * unknown:unk.e", "unk.u", "unk.tri", "unk.e"),
    derive("mc.pair", "unknown:unk.u + unknown:unk.i", "unk.u", "unk.i"),
]


@pytest.fixture
def example(tmp_path: Path) -> Path:
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


def add(folder: Path, *, unknowns=(), rules=(), thresholds=()) -> None:
    """Add unknowns and rules to the example's rule file, and thresholds to its exports.json."""
    for name, members in ((RULES, {"unknowns": unknowns, "rules": rules}), (EXPORTS, {"thresholds": thresholds})):
        path = folder / name
        doc = json.loads(path.read_text(encoding="utf-8"))
        for member, entries in members.items():
            doc[member].extend(entries)
        path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


@pytest.fixture
def acmodel(make_system, app_dir, write_config, check_envelope, tmp_path):
    """One command in the venv, with `paths.state_root` at `state_root` and the keys under `unknowns`:
    (exit status, envelope)."""

    def command(*argv: object, state_root: Path | None = None, unknowns: dict[str, int] | None = None):
        config: dict[str, Any] = {
            "schema_version": "acmodel.config.v1", "paths": {"state_root": str(state_root or tmp_path / "state")},
        }
        if unknowns is not None:
            config["unknowns"] = unknowns
        write_config(config)
        system = make_system(prefix=app_dir / "venv")
        status = cli.main([str(item) for item in argv], system)
        return status, check_envelope(system.stdout.getvalue(), status)

    return command


def run(acmodel, folder: Path, **options) -> tuple[int, dict]:
    status, doc = acmodel("run", folder / MODEL, "--state", folder / STATE, **options)
    assert doc["command"] == "run"
    return status, doc


def test_the_loss_at_the_red_line_ranges_from_0_to_550_with_its_top_unknown(example, acmodel):
    status, doc = run(acmodel, example)

    assert status == 0
    assert doc["data"]["rules"]["loss.collect_at_redline"] == {
        "kind": "derive", "status": "provisional", "type": "amount:res.a", "per_account": False,
        "variants": [
            {"unknowns": {"unk.collect": "up_to_cap"}, "value": "550"},
            {"unknowns": {"unk.collect": "whole"}, "value": "0"},
        ],
        "range": {"low": "0", "high": "550"},
        "sensitivity": [{"unknown": "unk.collect", "impact": "550"}],
    }


@pytest.mark.parametrize(
    ("high", "impact", "expected"),
    [
        pytest.param("0.0004", "0", "confirmed", id="below_milli"),
        pytest.param("0.0006", "1/1000", "provisional", id="one_milli"),
    ],
)
def test_an_unknown_without_effect_at_milli_leaves_its_output_confirmed(example, acmodel, high, impact, expected):
    """The floor plus a small unknown: at milli (round half even) 215.0004 is still 215, and 215.0006 is
    not. Rounded down, the threshold of it is 215 at both points, so it is confirmed and exported either
    way: the rule is decided per output, by computation."""
    add(
        example,
        unknowns=[unknown("unk.small", "interval", {"low": "0", "high": high})],
        rules=[derive("floor.small", "cost(act.daily, res.a) + rule:reserve.a + unknown:unk.small", "unk.small")],
        thresholds=[{"id": "thr.small", "from": "floor.small", "form": "constant", "round": "floor"}],
    )

    status, doc = run(acmodel, example)

    assert status == 0
    rule = doc["data"]["rules"]["floor.small"]
    assert rule["status"] == expected
    assert rule["sensitivity"] == [{"unknown": "unk.small", "impact": impact}]
    assert rule["range"] == {"low": "215", "high": str(215 + Fraction(high))}
    threshold = doc["data"]["thresholds"]["thr.small"]
    assert (threshold["status"], threshold["range"]) == ("confirmed", {"low": "215", "high": "215"})
    assert threshold["sensitivity"] == [{"unknown": "unk.small", "impact": "0"}]
    status, exported = acmodel("export", "thresholds", example / MODEL, "--state", example / STATE)
    assert (status, exported["errors"]) == (0, [])
    written = json.loads((example / MODEL / "out" / "thresholds.json").read_text(encoding="utf-8"))
    assert {entry["id"]: entry.get("value") for entry in written["thresholds"]}["thr.small"] == 215


@pytest.mark.parametrize(
    ("value", "status", "expected"),
    [
        pytest.param("unknown:unk.i + 0.1", "provisional", {"low": "1/5", "high": "3/10"}, id="sum"),
        # one unknown is one value in a scenario, so it cancels exactly
        pytest.param("unknown:unk.i - unknown:unk.i", "confirmed", {"low": "0", "high": "0"}, id="one_unknown"),
        pytest.param("unknown:unk.j * unknown:unk.k", "provisional", {"low": "-4", "high": "8"}, id="product"),
        pytest.param("min(unknown:unk.i, 0.15) * 3", "provisional", {"low": "3/10", "high": "9/20"}, id="min"),
        pytest.param("floor(unknown:unk.i * 10)", "provisional", {"low": "1", "high": "2"}, id="floor"),
        pytest.param("unknown:unk.t * 2", "provisional", {"low": "7200", "high": "14400"}, id="time_in_seconds"),
        pytest.param("unknown:unk.u - 1", "provisional", {"low": "0", "high": "2"}, id="uniform"),
        pytest.param("unknown:unk.tri / 8", "provisional", {"low": "0", "high": "1/2"}, id="triangular"),
        pytest.param("unknown:unk.e * 2", "provisional", {"low": "4", "high": "18"}, id="empirical"),
        pytest.param("unknown:unk.reversed", "blocked", None, id="low_above_high"),
    ],
)
def test_exact_interval_cases(example, acmodel, value, status, expected):
    add(example, unknowns=INTERVALS + DISTRIBUTIONS, rules=[derive("case.x", value)])

    exit_status, doc = run(acmodel, example)

    rule = doc["data"]["rules"]["case.x"]
    assert rule["status"] == status
    if expected is not None:
        assert exit_status == 0
        assert rule["range"] == expected
        return
    assert exit_status == 1
    assert [(e["code"], e["ref"], e["detail"], e["needed_by"]) for e in doc["errors"]] == [(
        "expr_eval_failed", "/unknowns/5/interval",
        {"path": str(example / RULES), "reason": "range_invalid", "operator": "interval", "position": 0},
        ["rule:case.x"],
    )]


def test_sampled_results_are_identical_for_chunk_sizes_1_and_4(example, acmodel, tmp_path):
    add(example, unknowns=INTERVALS[:1] + DISTRIBUTIONS, rules=SAMPLED)
    seen = []
    for chunk in (1, 4):
        unknowns = {"samples": 9, "seed": 11, "chunk_size": chunk}
        status, doc = run(acmodel, example, state_root=tmp_path / f"state-{chunk}", unknowns=unknowns)
        assert status == 0
        assert doc["data"]["work"]["by_kind"]["sampled"] == {"computed": 2, "reused": 0, "cutoff": 0}
        seen.append({key: value for key, value in doc["data"].items() if key != "work"})

    one, four = seen
    assert one == four
    rules = one["rules"]
    assert [(entry["unknowns"], entry["samples"]) for entry in rules["mc.mix"]["sampled"]] == [({}, 9)]
    assert [(entry["unknowns"], entry["samples"]) for entry in rules["mc.pair"]["sampled"]] == [
        ({"unk.i": "high"}, 9), ({"unk.i": "low"}, 9),
    ]
    for name in ("mc.mix", "mc.pair"):  # every sample lies inside the exact range
        low, high = (Fraction(rules[name]["range"][end]) for end in ("low", "high"))
        for entry in rules[name]["sampled"]:
            assert low <= Fraction(entry["low"]) <= Fraction(entry["mean"]) <= Fraction(entry["high"]) <= high


def test_sampled_results_are_identical_under_two_hash_seeds(example, tmp_path, launcher):
    add(example, unknowns=INTERVALS[:1] + DISTRIBUTIONS, rules=SAMPLED)
    seen = []
    for seed in ("0", "1"):
        local = tmp_path / f"localappdata-{seed}"
        app = local / "ActingCommand" / "acmodel"
        app.mkdir(parents=True)
        # This interpreter stands in for the venv: CI installed requirements-dev.txt into it.
        config = {"schema_version": "acmodel.config.v1", "paths": {"venv": sys.prefix},
                  "unknowns": {"samples": 5, "seed": 3, "chunk_size": 2}}
        (app / "config.json").write_text(json.dumps(config), encoding="utf-8")
        env = {key: value for key, value in os.environ.items() if key != environment.DISPATCH_MARK}
        env.update(LOCALAPPDATA=str(local), PYTHONHASHSEED=seed)
        # Not -I, which ignores PYTHONHASHSEED; -s and -P keep the user site and the script folder off sys.path.
        python = [sys.executable, "-s", "-P", "-B"]
        hashed = subprocess.run(
            [*python, "-c", "print(hash('acmodel'))"], env=env, capture_output=True, timeout=120, check=True
        )
        proc = subprocess.run(
            [*python, str(launcher), "run", str(example / MODEL), "--state", str(example / STATE)],
            cwd=tmp_path, env=env, capture_output=True, timeout=120, check=False,
        )
        assert proc.returncode == 0, proc.stderr
        doc = json.loads(proc.stdout)
        assert [entry["samples"] for entry in doc["data"]["rules"]["mc.mix"]["sampled"]] == [5]
        seen.append((hashed.stdout, proc.stdout))

    (hash_0, stdout_0), (hash_1, stdout_1) = seen
    assert hash_0 != hash_1  # the two seeds took effect
    assert stdout_0 == stdout_1
