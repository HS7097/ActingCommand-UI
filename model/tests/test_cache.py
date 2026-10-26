"""PY-9: the cache and incremental recompute (#424 design section 5) on the worked example (design section
3.8): an incremental run equals a cold one byte for byte; only the dependents of a change recompute; a
same-value change stops at the early cutoff; a corrupt blob is reported and recomputed; equal inputs give
equal run ids; an evaluator bump or a library version reaches only its kinds; the outputs do not depend
on the string-hash seed; and the provenance record (PY-5) comes out of a warm cache unchanged."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
from contextlib import closing
from dataclasses import replace
from pathlib import Path

import pytest

from acmodel import cli, environment
from acmodel.cache import EVALUATORS, Session, canonical, open_store
from acmodel.core import run_model
from acmodel.core.provenance import assemble, outputs as output_nodes
from acmodel.core.run import evaluate_model

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
CODES = json.loads((Path(__file__).resolve().parents[1] / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
MODEL = "ours/model"
RULES = "ours/model/rules/a.json"
COSTS = "ours/model/data/costs.json"
STATE = "state/acct-1.json"
LIBRARIES = {"python": "3.13", "numpy": "2.5.4", "scipy": "1.18.1"}

# The example with an entity class, one unit of it in the state and a rule that sums over the units, so
# that a new unit is a new entity.
UNITS = [
    (
        RULES,
        '{"id": "act.daily", "kind": "activity"}\n  ],',
        '{"id": "act.daily", "kind": "activity"},\n'
        '    {"id": "unit", "kind": "entity_class", "attributes": {"level": {"type": "integer", "dimension": "count"}}}\n'
        "  ],",
    ),
    (
        RULES,
        '"status": "confirmed"}\n  ],\n  "unknowns"',
        '"status": "confirmed"},\n'
        '    {"id": "level.total", "kind": "derive", "subject": "unit",\n'
        '     "logic": {"value": "sum(entity:unit: entity:unit[$].level)"},\n'
        '     "provenance": [{"kind": "logic", "ref": "#425 7"}], "status": "confirmed"}\n'
        '  ],\n  "unknowns"',
    ),
    (STATE, '"entities": {}', '"entities": {"unit": {"u1": {"level": 2}}}'),
]
DATA_ROW = [(COSTS, '"a": "15", "status": "confirmed"', '"a": "16", "status": "confirmed"')]
READING = [(STATE, '"resource.a_capacity": {"value": 3470,', '"resource.a_capacity": {"value": 3471,')]
RULE = [(RULES, '"logic": {"value": "250",', '"logic": {"value": "260",')]
NEW_ENTITY = [(STATE, '"u1": {"level": 2}', '"u1": {"level": 2}, "u2": {"level": 3}')]


@pytest.fixture
def example(tmp_path: Path) -> Path:
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


def apply(folder: Path, edits: list[tuple[str, str, str]]) -> None:
    """Replace one unique piece of text in a file of the example, for each edit."""
    for name, old, new in edits:
        path = folder / name
        text = path.read_text(encoding="utf-8")
        assert text.count(old) == 1, old
        path.write_bytes(text.replace(old, new).encode("utf-8"))


def valid(doc: dict, status: int) -> None:
    """One `acmodel.cli.v1` envelope whose errors and warnings are registered codes with their values, and
    whose exit status is the first error's."""
    assert set(doc) == {"schema_version", "command", "ok", "data", "errors", "warnings"}
    assert doc["ok"] is (doc["errors"] == [])
    for entry in doc["errors"] + doc["warnings"]:
        assert set(entry) <= {"code", "detail", "ref", "needed_by"}
        values = CODES[entry["code"]]["values"]
        assert set(entry["detail"]) <= set(values), entry
        for key, spec in values.items():
            if spec["required"]:
                assert key in entry["detail"], (entry, key)
            if "members" in spec and key in entry["detail"]:
                assert entry["detail"][key] in spec["members"], (entry, key)
    assert status == (CODES[doc["errors"][0]["code"]]["exit"] if doc["errors"] else 0)


@pytest.fixture
def acmodel(make_system, app_dir, write_config):
    """`acmodel <argv>` in process with `paths.state_root` at `state_root`: (exit status, envelope)."""

    def command(*argv: object, state_root: Path) -> tuple[int, dict]:
        write_config({"schema_version": "acmodel.config.v1", "paths": {"state_root": str(state_root)}})
        system = make_system(prefix=app_dir / "venv")
        status = cli.main([str(item) for item in argv], system)
        lines = system.stdout.getvalue().splitlines()
        assert len(lines) == 1, system.stdout.getvalue()
        doc = json.loads(lines[0])
        valid(doc, status)
        return status, doc

    return command


@pytest.fixture
def run(acmodel, example):
    """`acmodel run` on the example with the cache under `state_root`."""

    def run_example(state_root: Path) -> tuple[int, dict]:
        status, doc = acmodel("run", example / MODEL, "--state", example / STATE, state_root=state_root)
        assert doc["command"] == "run"
        return status, doc

    return run_example


def outputs(state_root: Path, doc: dict) -> bytes:
    return (state_root / "runs" / doc["data"]["run_id"] / "outputs.json").read_bytes()


def results(doc: dict, *drop: str) -> dict:
    """The run's data without its work counters (and whatever else `drop` names)."""
    return {key: value for key, value in doc["data"].items() if key not in ("work", *drop)}


def computed(work: dict) -> dict[str, int]:
    """The nodes computed, by kind."""
    return {kind: counts["computed"] for kind, counts in work["by_kind"].items() if counts["computed"]}


def core_run(example: Path, state_root: Path, *, evaluators=EVALUATORS, libraries=LIBRARIES) -> dict:
    """`run_model` with a cache session of the given evaluator and library versions."""
    with open_store(state_root, max_bytes=2**31) as store:
        session = Session(store, libraries=libraries, evaluators=evaluators)
        data, errors = run_model(
            example / MODEL, example / STATE, datasets=state_root / "datasets", max_bits=127, cache=session
        )
    assert errors == [] and session.warnings == []
    return data


@pytest.mark.parametrize(
    "change",
    [
        pytest.param(DATA_ROW, id="data_row"),
        pytest.param(READING, id="reading"),
        pytest.param(RULE, id="rule"),
        pytest.param(NEW_ENTITY, id="new_entity"),
    ],
)
def test_an_incremental_run_equals_a_cold_one_byte_for_byte(example, run, tmp_path, change):
    apply(example, UNITS)
    warm, cold = tmp_path / "warm", tmp_path / "cold"
    _, before = run(warm)
    apply(example, change)

    status, incremental = run(warm)
    _, fresh = run(cold)

    assert status == 0
    assert incremental["data"]["work"]["reused"] > 0
    assert fresh["data"]["work"]["reused"] == 0
    assert incremental["data"]["rules"] != before["data"]["rules"]  # the change reached an output
    assert results(incremental) == results(fresh)
    assert outputs(warm, incremental) == outputs(cold, fresh)


@pytest.mark.parametrize(
    ("change", "dependents"),
    [
        # the row's flow amount, the floor that adds it up, and the threshold of the floor
        pytest.param(DATA_ROW, {"flow": 1, "rule": 1, "threshold": 1}, id="data_row"),
        # the read cap and the red line that reads it
        pytest.param(READING, {"rule": 2}, id="reading"),
    ],
)
def test_only_the_dependents_recompute(example, run, tmp_path, change, dependents):
    state_root = tmp_path / "state"
    _, cold = run(state_root)
    apply(example, change)

    _, warm = run(state_root)

    work = warm["data"]["work"]
    assert computed(work) == dependents
    assert work["nodes"] == cold["data"]["work"]["nodes"]
    assert work["reused"] == work["nodes"] - sum(dependents.values())


def test_a_same_value_change_stops_at_the_early_cutoff(example, run, tmp_path):
    state_root = tmp_path / "state"
    _, before = run(state_root)
    apply(example, [(COSTS, '"a": "15", "status": "confirmed"', '"a": 15, "status": "confirmed"')])

    _, after = run(state_root)

    work = after["data"]["work"]
    assert work["by_kind"]["flow"] == {"computed": 1, "reused": 0, "cutoff": 1}
    assert (work["computed"], work["cutoff"], work["reused"]) == (1, 1, work["nodes"] - 1)
    assert results(after, "run_id") == results(before, "run_id")


def test_a_corrupt_blob_gives_cache_entry_corrupt_and_a_recompute(example, run, acmodel, tmp_path):
    state_root = tmp_path / "state"
    _, first = run(state_root)
    with closing(sqlite3.connect(state_root / "cache" / "index.sqlite")) as index:
        [(key, value)] = index.execute("SELECT key, value FROM nodes WHERE node = ?", ('["rule","reserve.a"]',)).fetchall()
        [(sharing,)] = index.execute("SELECT count(*) FROM nodes WHERE value = ?", (value,)).fetchall()
    assert sharing == 1  # no other node shares the blob
    blob = state_root / "cache" / "blobs" / value[:2] / value
    garbage = b'{"corrupt":true}'
    blob.write_bytes(garbage)
    corrupt = {
        "reason": "hash_mismatch", "path": str(blob), "key": key, "kind": "rule",
        "found": hashlib.sha256(garbage).hexdigest(),
    }

    status, verified = acmodel("cache", "verify", state_root=state_root)
    assert status == 1
    assert [(entry["code"], entry["detail"]) for entry in verified["errors"]] == [("cache_entry_corrupt", corrupt)]

    status, second = run(state_root)

    assert status == 0
    assert [(entry["code"], entry["detail"]) for entry in second["warnings"]] == [("cache_entry_corrupt", corrupt)]
    work = second["data"]["work"]
    assert (work["corrupt"], work["computed"], computed(work)) == (1, 1, {"rule": 1})
    assert results(second) == results(first)
    assert hashlib.sha256(blob.read_bytes()).hexdigest() == value  # recomputed and kept again
    status, verified = acmodel("cache", "verify", state_root=state_root)
    assert (status, verified["errors"]) == (0, [])


def test_equal_inputs_give_equal_run_ids(example, run, tmp_path):
    _, first = run(tmp_path / "a")
    _, again = run(tmp_path / "a")
    _, elsewhere = run(tmp_path / "b")
    apply(example, READING)
    _, changed = run(tmp_path / "a")
    apply(example, [(STATE, '"resource.a_capacity": {"value": 3471,', '"resource.a_capacity": {"value": 3470,')])
    _, back = run(tmp_path / "a")

    run_id = first["data"]["run_id"]
    assert len(run_id) == 64
    assert again["data"]["run_id"] == elsewhere["data"]["run_id"] == back["data"]["run_id"] == run_id
    assert changed["data"]["run_id"] != run_id
    assert outputs(tmp_path / "a", first) == outputs(tmp_path / "b", elsewhere)


def test_one_kinds_evaluator_bump_leaves_the_other_kinds_entries_in_use(example, tmp_path):
    state_root = tmp_path / "state"
    first = core_run(example, state_root)
    offset = EVALUATORS["offset"]
    bumped = {**EVALUATORS, "offset": replace(offset, version=offset.version + 1)}

    second = core_run(example, state_root, evaluators=bumped)

    work = second["work"]
    assert computed(work) == {"offset": 1}
    assert work["reused"] == work["nodes"] - 1
    assert results({"data": second}, "run_id") == results({"data": first}, "run_id")


def test_a_library_version_change_misses_only_the_kinds_that_declare_it(example, tmp_path):
    state_root = tmp_path / "state"
    declared = {
        **EVALUATORS,
        "flow": replace(EVALUATORS["flow"], libraries=("numpy",)),
        "member": replace(EVALUATORS["member"], libraries=("scipy",)),
    }
    first = core_run(example, state_root, evaluators=declared)

    second = core_run(example, state_root, evaluators=declared, libraries={**LIBRARIES, "numpy": "2.5.5"})

    work = second["work"]
    assert computed(work) == {"flow": 1}
    assert work["reused"] == work["nodes"] - 1
    assert results({"data": second}, "run_id") == results({"data": first}, "run_id")


def test_provenance_and_outputs_are_byte_identical_with_and_without_a_warm_cache(example, tmp_path):
    """Every output's provenance document, and the run's data and errors, are the same from a run without
    the cache and from a run that reuses every node (the value kept in the cache carries `Scenario.reads`)."""
    apply(example, UNITS)
    state_root = tmp_path / "state"
    with open_store(state_root, max_bytes=2**31) as store:  # fills the cache
        run_model(example / MODEL, example / STATE, datasets=state_root / "datasets", max_bits=127,
                  cache=Session(store, libraries=LIBRARIES))
    plain = evaluate_model(example / MODEL, example / STATE, datasets=state_root / "datasets", max_bits=127)

    with open_store(state_root, max_bytes=2**31) as store:
        session = Session(store, libraries=LIBRARIES)
        warm = evaluate_model(example / MODEL, example / STATE, datasets=state_root / "datasets", max_bits=127,
                              cache=session)
        work = session.work()

    assert work["computed"] == 0 and work["reused"] == work["nodes"] > 0
    assert canonical([warm.data, warm.errors]) == canonical([plain.data, plain.errors])
    for output in output_nodes(plain):
        assert canonical(assemble(warm, output)) == canonical(assemble(plain, output)), output


def test_run_outputs_are_byte_identical_under_two_hash_seeds(example, tmp_path, launcher):
    apply(example, UNITS)
    seen = []
    for seed in ("0", "1"):
        local = tmp_path / f"localappdata-{seed}"
        app = local / "ActingCommand" / "acmodel"
        app.mkdir(parents=True)
        # This interpreter stands in for the venv: CI installed requirements-dev.txt into it.
        config = {"schema_version": "acmodel.config.v1", "paths": {"venv": sys.prefix}}
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
        valid(doc, proc.returncode)
        kept = (app / "state" / "runs" / doc["data"]["run_id"] / "outputs.json").read_bytes()
        seen.append((hashed.stdout, proc.stdout, kept))

    (hash_0, stdout_0, kept_0), (hash_1, stdout_1, kept_1) = seen
    assert hash_0 != hash_1  # the two seeds took effect
    assert stdout_0 == stdout_1
    assert kept_0 == kept_1
