"""PY-12: the engine host (#424 design-py section 7), on the neutral fake engine of tests/fakes/engine_home.

- Round trip and batching.
- An adapter error passes its code through; an exception is engine_failed with its type.
- A non-JSON result is engine_protocol_error.
- A fake engine that iterates a set of strings differs under the two hash seeds: engine_nondeterministic.
- A write outside scratch, an unlisted file read, an unlisted module and an argv change are each
  engine_guard_violation with its value.
- Never more than compute.workers.max workers at once, by a counter.
- A crash restarts within the budget, then fails loud.
- A changed engine file gives a new fingerprint and a cache miss.
- Windows leg: the workers inherit the Job and below-normal priority.
- Engine prints do not disturb the channel.

The workers are real processes; the host's side of the OS is a fake (`FakeOs`) except in the Windows leg,
and nothing sleeps. Each test works on its own copy of the fake engine.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

from acmodel import config
from acmodel.cache import Session, open_store
from acmodel.engine import client, manifest
from acmodel.engine.pool import Pool
from acmodel.envelope import Failure

TESTS = Path(__file__).resolve().parent
MODEL_DIR = TESTS.parent
FAKE_HOME = TESTS / "fakes" / "engine_home"
PROBE = TESTS / "engine_probe.py"
CODES = json.loads((MODEL_DIR / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
LIBRARIES = {"python": "3.13", "numpy": "2.5.4", "scipy": "1.18.1"}
BELOW_NORMAL = 0x00004000
WORDS = [f"word{index:02d}" for index in range(48)]


def registered(entry: dict[str, Any]) -> dict[str, Any]:
    """The entry, after checking that its code is registered and its values are that code's (codes.json)."""
    assert set(entry) <= {"code", "detail", "ref"}, entry
    values = CODES[entry["code"]]["values"]
    assert set(entry["detail"]) <= set(values), entry
    for key, spec in values.items():
        if spec["required"]:
            assert key in entry["detail"], (entry, key)
        if "members" in spec and key in entry["detail"]:
            assert entry["detail"][key] in spec["members"], (entry, key)
    assert CODES[entry["code"]]["exit"] == 3
    return entry


class FakeOs:
    """The host's side of each worker: every worker passes the Job check, and uses little memory."""

    def __init__(self) -> None:
        self.verified: list[int] = []

    def verify_child(self, pid: int) -> None:
        self.verified.append(pid)

    def private_mib(self, pid: int) -> int:
        return 64


@pytest.fixture
def home(tmp_path: Path) -> Path:
    """A copy of the fake engine's home."""
    target = tmp_path / "engines" / "fake"
    shutil.copytree(FAKE_HOME, target)
    return target


@pytest.fixture
def make_pool(tmp_path, home, write_config, local_app_data):
    """A pool on the fake engine, configured through config.json: `compute.workers.max` and the `engine`
    section as given, defaults otherwise."""
    pools: list[Pool] = []

    def make(*, workers: int, engine: dict[str, Any] | None = None, scratch: str = "scratch", launch=None) -> Pool:
        doc: dict[str, Any] = {
            "schema_version": "acmodel.config.v1",
            "compute": {"workers": {"max": workers}},
            "engines": {"roots": {"fake": str(home)}},
        }
        if engine is not None:
            doc["engine"] = engine
        write_config(doc)
        cfg = config.load({"LOCALAPPDATA": str(local_app_data)})
        loaded = manifest.load("fake", roots=cfg.engine.roots, datasets=cfg.datasets, libraries=LIBRARIES)
        options = {} if launch is None else {"launch": launch}
        pool = Pool(
            loaded, cfg.engine, scratch_root=tmp_path / scratch, log_dir=tmp_path / "logs", os_layer=FakeOs(),
            blas_threads=cfg.blas_threads, **options,
        )
        pools.append(pool)
        return pool

    yield make
    for pool in pools:
        pool.close()


def test_round_trip_and_batching(make_pool):
    pool = make_pool(workers=1)

    outcomes = pool.call("evaluate", [{"do": "double", "x": x} for x in (1, 2, 3, 4, 5)])

    assert [outcome.error for outcome in outcomes] == [None] * 5
    assert [outcome.result["metrics"] for outcome in outcomes] == [{"value": 2 * x} for x in (1, 2, 3, 4, 5)]
    # max_batch 2: the batches hold requests 0-1, 2-3 and 4, and the results come back in request order.
    assert [outcome.result["flags"] for outcome in outcomes] == [["batch:2"]] * 4 + [["batch:1"]]
    assert (pool.counters["batches"], pool.counters["requests"]) == (3, 5)

    payload = {
        "text": "aé中\U0001f600 \"quoted\" \\ /",
        "integers": [0, -7, 2**53 - 1],
        "float": 0.1,
        "others": [True, False, None, [], {}],
        "nested": {"a": {"b": ["c", {"d": 1}]}},
    }
    [echo] = pool.call("search", [{"do": "echo", "payload": payload}])
    assert echo.error is None
    assert echo.result == {"echo": payload, "depends_on": []}
    assert pool.counters["started"] == 1  # one worker served both calls


def test_an_adapter_error_passes_its_code_through_and_an_exception_is_engine_failed_with_its_type(make_pool):
    pool = make_pool(workers=1)

    adapter, plain, raised, beside = pool.call("evaluate", [
        {"do": "adapter_error", "code": "fake.not_found", "x": 1},
        {"do": "double", "x": 1},
        {"do": "raise"},
        {"do": "double", "x": 2},
    ])

    assert registered(adapter.error) == {"code": "engine_failed", "detail": {
        "engine_id": "fake", "op": "evaluate", "request": 0, "reason": "adapter_error",
        "engine_code": "fake.not_found", "engine_detail": '{"x":1}',
    }}
    assert plain.error is None and plain.result["metrics"] == {"value": 2}
    # The exception fails its batch, requests 2 and 3.
    for index, outcome in ((2, raised), (3, beside)):
        assert outcome.result is None
        assert registered(outcome.error) == {"code": "engine_failed", "detail": {
            "engine_id": "fake", "op": "evaluate", "request": index, "reason": "exception",
            "exception_type": "ValueError", "raw_text": "a fake failure",
        }}


def test_a_non_json_result_is_engine_protocol_error(make_pool):
    pool = make_pool(workers=1)

    bad, good = pool.call("evaluate", [{"do": "not_json"}, {"do": "double", "x": 3}])

    assert bad.result is None
    assert registered(bad.error) == {
        "code": "engine_protocol_error",
        "detail": {"engine_id": "fake", "op": "evaluate", "request": 0, "reason": "not_json", "found": "set"},
        "ref": "/metrics/value",
    }
    assert good.error is None and good.result["metrics"] == {"value": 6}


def test_a_fake_engine_that_iterates_a_set_of_strings_is_engine_nondeterministic_under_the_two_seeds(make_pool):
    request = {"do": "set_order", "words": WORDS}
    # Under one seed, two fresh workers iterate the set alike: the seed is set, not random.
    orders = []
    for scratch in ("first", "second"):
        pool = make_pool(workers=1, engine={"verify_sample_milli": 0}, scratch=scratch)
        [outcome] = client.call(pool, "evaluate", [request])
        orders.append(outcome.result["flags"])
        pool.close()
    assert orders[0] == orders[1]
    assert sorted(orders[0]) == WORDS
    pool = make_pool(workers=1, engine={"verify_sample_milli": 1000})

    with pytest.raises(Failure) as caught:
        client.call(pool, "evaluate", [request])

    [entry] = caught.value.errors
    assert registered(entry)["code"] == "engine_nondeterministic"
    assert entry["detail"] == {"engine_id": "fake", "op": "evaluate", "request": 0, "hash_seed": 0, "verify_hash_seed": 1}
    assert entry["ref"].startswith("/flags/")


@pytest.mark.parametrize(
    ("do", "expected"),
    [
        ("write_outside", {"reason": "write_outside_scratch", "path": "out.txt"}),
        ("read_unlisted", {"reason": "unlisted_file", "path": "notes.txt"}),
        ("import_unlisted", {"reason": "unlisted_module", "path": "extra/extra_mod.py"}),
        ("argv", {"reason": "global_state_changed", "state": "argv"}),
    ],
    ids=["write_outside_scratch", "unlisted_file", "unlisted_module", "argv_changed"],
)
def test_each_guard_gives_engine_guard_violation_with_its_value(make_pool, home, do, expected):
    pool = make_pool(workers=1)

    with pytest.raises(Failure) as caught:
        pool.call("evaluate", [{"do": do, "x": 1}])

    [entry] = caught.value.errors
    detail = dict(registered(entry)["detail"])
    assert entry["code"] == "engine_guard_violation"
    if "path" in expected:
        found = detail.pop("path")
        relative = os.path.join(os.path.realpath(home), *expected["path"].split("/"))
        assert os.path.normcase(found) == os.path.normcase(relative)
    assert detail == {"engine_id": "fake", "stage": "batch", **{k: v for k, v in expected.items() if k != "path"}}


def test_never_more_than_workers_max_at_once(make_pool):
    launched: list[subprocess.Popen] = []
    alive_at_launch: list[int] = []

    def launch(args, **options):
        alive_at_launch.append(sum(proc.poll() is None for proc in launched))
        proc = subprocess.Popen(args, **options)
        launched.append(proc)
        return proc

    # Recycled after every batch (2 requests), so six batches need six workers in turn.
    pool = make_pool(workers=2, engine={"worker_max_requests": 2}, launch=launch)

    outcomes = pool.call("evaluate", [{"do": "double", "x": x} for x in range(12)])

    assert [outcome.result["metrics"] for outcome in outcomes] == [{"value": 2 * x} for x in range(12)]
    assert pool.counters["peak_workers"] == 2  # the counter: the bound reached, never passed
    assert len(launched) == pool.counters["started"] == 6
    assert max(alive_at_launch) <= 1  # no worker was started while two were alive


def test_a_crash_restarts_within_the_budget_then_fails_loud(make_pool):
    within = make_pool(workers=1, engine={"restart_budget": 2}, scratch="within")
    [outcome] = within.call("evaluate", [{"do": "crash", "times": 2, "x": 4}])
    assert outcome.error is None and outcome.result["metrics"] == {"value": 8}
    assert within.counters["restarts"] == 2

    past = make_pool(workers=1, engine={"restart_budget": 2}, scratch="past")
    with pytest.raises(Failure) as caught:
        past.call("evaluate", [{"do": "crash", "times": 3, "x": 4}])

    [entry] = caught.value.errors
    assert registered(entry) == {"code": "engine_failed", "detail": {
        "engine_id": "fake", "op": "evaluate", "reason": "restart_budget_exhausted", "exit_code": 17,
        "restarts": 2, "budget": 2,
    }}
    assert past.pids() == []  # every worker is stopped


def test_a_changed_engine_file_gives_a_new_fingerprint_and_a_cache_miss(make_pool, home, tmp_path):
    state_root = tmp_path / "state"
    request = {"do": "double", "x": 5, "entities": {"u1": {"level": 1}}}

    def cached_call(pool: Pool) -> tuple[Any, dict[str, int]]:
        with open_store(state_root, max_bytes=2**31) as store:
            session = Session(store, libraries=LIBRARIES)
            [outcome] = client.call(pool, "evaluate", [request], session=session)
            store.commit()
            return outcome, session.work()["by_kind"]["engine"]

    first = make_pool(workers=1, engine={"verify_sample_milli": 0})
    outcome, work = cached_call(first)
    assert outcome.result["metrics"] == {"value": 10}
    assert work == {"computed": 1, "reused": 0, "cutoff": 0}
    outcome, work = cached_call(first)
    assert outcome.result["metrics"] == {"value": 10}
    assert work == {"computed": 0, "reused": 1, "cutoff": 0}

    adapter = home / "adapters" / "fake.py"
    adapter.write_bytes(adapter.read_bytes() + b"\n# a changed engine file\n")
    second = make_pool(workers=1, engine={"verify_sample_milli": 0})

    assert second.engine.fingerprint != first.engine.fingerprint
    outcome, work = cached_call(second)
    assert outcome.result["metrics"] == {"value": 10}
    assert work == {"computed": 1, "reused": 0, "cutoff": 0}


@pytest.mark.skipif(sys.platform != "win32", reason="priority classes and Job objects exist only on Windows")
def test_windows_leg_workers_inherit_the_job_and_below_normal_priority(tmp_path, home, write_config, local_app_data):
    write_config({
        "schema_version": "acmodel.config.v1",
        "compute": {"workers": {"max": 2}},
        "engine": {"verify_sample_milli": 0},
        "engines": {"roots": {"fake": str(home)}},
    })
    proc = subprocess.run(
        [sys.executable, "-I", "-B", str(PROBE), str(MODEL_DIR / "src"), str(local_app_data), str(tmp_path)],
        capture_output=True, text=True, timeout=300, check=False,
    )

    assert proc.returncode == 0, (proc.stdout, proc.stderr)
    report = json.loads(proc.stdout)
    assert report["errors"] == []
    assert report["values"] == [2, 4, 6, 8]
    # Two batches, two workers: each started inside the probe's Job, below normal, and checked by the pool
    # (verify_child) before it ran engine code.
    assert report["workers"] == [
        {"opened": True, "priority": BELOW_NORMAL, "in_job_queried": True, "in_job": True, "closed": True},
    ] * 2


def test_engine_prints_do_not_disturb_the_channel(make_pool):
    pool = make_pool(workers=1)

    printed, after = pool.call("evaluate", [{"do": "print", "x": 3}, {"do": "double", "x": 4}])
    [again] = pool.call("evaluate", [{"do": "double", "x": 5}])

    assert printed.error is None and printed.result["metrics"] == {"value": 6}
    assert after.error is None and after.result["metrics"] == {"value": 8}
    assert again.error is None and again.result["metrics"] == {"value": 10}
    assert pool.log_path("w0").read_bytes().count(b"fake-engine-print-marker") == 400  # all of it in the log
