"""PY-15: `watch` (#424 design 5.6, design-py 5.6), with a fake clock and fake file events, and no sleeps.

- Fake file events give one recompute per debounce window.
- A rewrite with the same hash gives none.
- A staged candidate and its diff are written, and nothing is committed: the repository is as the test left it.
- A fake memory pause stops the warm workers and resumes them later (real workers of the neutral fake engine).
- A restart resumes from the cache with no recompute (through the command line).

The host's clock and readings (`FakeHost`) and the files' modification times (`FakeStamps`) are fakes. A test ends
at the interrupt its fake clock raises past its end, as Ctrl+C ends `watch`.
"""

from __future__ import annotations

import io
import json
import os
import shutil
import sqlite3
from contextlib import closing
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli, config
from acmodel.host import windows
from acmodel.watch.files import Stamp
from acmodel.watch.watcher import Watcher

TESTS = Path(__file__).resolve().parent
EXAMPLE = TESTS / "fixtures" / "example"
TABLES = TESTS / "fixtures" / "tables"
FAKE_HOME = TESTS / "fakes" / "engine_home"
MODEL = "ours/model"
RULES = "ours/model/rules/a.json"
STATE = "state/acct-1.json"
LIBRARIES = {"python": "3.13", "numpy": "2.5.4", "scipy": "1.18.1"}
MONDAY = datetime(2030, 1, 7, 12, 0)
MARGIN = '"logic": {"value": "250",'


class FakeHost:
    """The host's clock and readings, and its side of the caps and the workers. `sleep_ms` calls `observe` (once
    per poll), then advances the clock and makes the changes scheduled up to then; past `until` it raises
    KeyboardInterrupt instead. The CPU is idle throughout."""

    def __init__(self, until: int, available_mib: int = 8192) -> None:
        self.until = until
        self.available = available_mib
        self.ms = 0
        self.scheduled: list[tuple[int, Callable[[], Any]]] = []
        self.observe: Callable[[], Any] | None = None
        self.applied: list[Any] = []

    def at(self, ms: int, change: Callable[[], Any]) -> None:
        self.scheduled.append((ms, change))

    def available_mib(self) -> int:
        return self.available

    def cpu_times(self) -> tuple[int, int]:
        return self.ms, self.ms

    def now(self) -> datetime:
        return MONDAY + timedelta(milliseconds=self.ms)

    def monotonic_ms(self) -> int:
        return self.ms

    def sleep_ms(self, ms: int) -> None:
        if self.observe is not None:
            self.observe()
        if self.ms + ms > self.until:
            raise KeyboardInterrupt
        self.ms += ms
        due = sorted((item for item in self.scheduled if item[0] <= self.ms), key=lambda item: item[0])
        self.scheduled = [item for item in self.scheduled if item[0] > self.ms]
        for _, change in due:
            change()

    def apply(self, caps: Any) -> None:
        self.applied.append(caps)

    def verify_child(self, pid: int) -> None:
        return None

    def private_mib(self, pid: int) -> int:
        return 64


class FakeStamps:
    """Fake file events: a file's stamp is its real size and a modification time that only `event` advances, so
    that the watcher sees exactly the events a test makes."""

    def __init__(self) -> None:
        self.times: dict[str, int] = {}

    def __call__(self, path: Path) -> Stamp | None:
        if not path.is_file():
            return None
        return Stamp(path.stat().st_size, self.times.get(os.path.realpath(path), 0))

    def event(self, path: Path, data: bytes) -> None:
        path.write_bytes(data)
        key = os.path.realpath(path)
        self.times[key] = self.times.get(key, 0) + 1


@pytest.fixture
def example(tmp_path: Path) -> Path:
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


@pytest.fixture
def make_watcher(tmp_path, write_config, local_app_data):
    """A watcher of the model and state of `folder`, configured through config.json: a poll every second, a window
    of five, and the sections given."""

    def make(folder: Path, host: FakeHost, stamps: FakeStamps, **sections: Any) -> Watcher:
        write_config({
            "schema_version": "acmodel.config.v1",
            "paths": {"state_root": str(tmp_path / "state_root"), "datasets": str(tmp_path / "datasets")},
            "watch": {"poll_interval_ms": 1000, "debounce_ms": 5000},
            **sections,
        })
        cfg = config.load({"LOCALAPPDATA": str(local_app_data)})
        return Watcher(
            folder / MODEL, folder / STATE, cfg=cfg, libraries=LIBRARIES, host=host, workers_os=host,
            stderr=io.StringIO(), stamp=stamps,
        )

    return make


def events(folder: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in (folder / "events.jsonl").read_text(encoding="utf-8").splitlines()]


def at_ms(event: dict[str, Any]) -> int:
    return (datetime.fromisoformat(event["at"]) - MONDAY) // timedelta(milliseconds=1)


def tree(folder: Path) -> dict[str, bytes | None]:
    """Every file (its bytes) and folder (None) under `folder`."""
    return {
        path.relative_to(folder).as_posix(): path.read_bytes() if path.is_file() else None
        for path in sorted(folder.rglob("*"))
    }


def test_fake_file_events_give_one_recompute_per_debounce_window(example, make_watcher):
    host, stamps = FakeHost(until=20_000), FakeStamps()
    rules = example / RULES
    text = rules.read_text(encoding="utf-8")
    assert text.count(MARGIN) == 1
    for at, margin in ((1_000, "260"), (2_000, "270"), (3_000, "280"), (13_000, "290")):
        data = text.replace(MARGIN, f'"logic": {{"value": "{margin}",').encode("utf-8")
        host.at(at, lambda data=data: stamps.event(rules, data))
    watcher = make_watcher(example, host, stamps)

    summary = watcher.run()

    log = events(watcher.folder)
    assert [at_ms(event) for event in log if event["event"] == "changed"] == [1_000, 2_000, 3_000, 13_000]
    refreshed = [event for event in log if event["event"] == "refreshed"]
    # The first at the start; then one per window: three events in the window from 1 000 to 6 000, one in the
    # window from 13 000 to 18 000.
    assert [at_ms(event) for event in refreshed] == [0, 6_000, 18_000]
    assert summary["refreshes"] == 3
    assert all(event["work"]["computed"] > 0 for event in refreshed)


def test_a_same_hash_rewrite_gives_no_recompute(example, make_watcher):
    host, stamps = FakeHost(until=12_000), FakeStamps()
    for at, name in ((1_000, RULES), (2_000, STATE), (7_000, RULES)):
        path = example / name
        host.at(at, lambda path=path: stamps.event(path, path.read_bytes()))
    watcher = make_watcher(example, host, stamps)

    summary = watcher.run()

    assert [event["event"] for event in events(watcher.folder)] == ["started", "refreshed", "staged", "stopped"]
    assert summary["refreshes"] == 1
    # The scan saw each rewrite, and hashed it the same.
    assert watcher.seen[os.path.realpath(example / RULES)].stamp.mtime_ns == 2
    assert watcher.seen[os.path.realpath(example / STATE)].stamp.mtime_ns == 1


def test_a_staged_candidate_and_its_diff_are_written_and_nothing_is_committed(tmp_path, make_watcher):
    folder = tmp_path / "repo"
    shutil.copytree(TABLES, folder)
    rules = folder / MODEL / "rules" / "values.json"
    text = rules.read_text(encoding="utf-8")
    old = '"logic": {"value": "1200"}'
    assert text.count(old) == 1
    edited = text.replace(old, '"logic": {"value": "1300"}').encode("utf-8")
    host, stamps = FakeHost(until=8_000), FakeStamps()
    host.at(1_000, lambda: stamps.event(rules, edited))
    before = tree(folder)
    watcher = make_watcher(folder, host, stamps)

    summary = watcher.run()

    # Nothing reached the repository: no export file, no commit; only the test's own edit.
    assert tree(folder) == {**before, rules.relative_to(folder).as_posix(): edited}
    assert summary["staged"] == [1, 2]
    staged = [event for event in events(watcher.folder) if event["event"] == "staged"]
    assert [(at_ms(event), event["candidate"], event["from"]) for event in staged] == [(0, 1, None), (6_000, 2, 1)]

    second = watcher.folder / "candidates" / "000002"
    diff = json.loads((second / "diff.json").read_text(encoding="utf-8"))
    assert (diff["schema_version"], diff["from"], diff["to"]) == ("acmodel.watch.diff.v1", 1, 2)
    changed = {entry["name"]: entry for entry in diff["files"]}
    table = changed["repo/tables/unit_values.json"]
    assert (table["change"], table["pointer"]) == ("changed", "/rows/0/2")
    assert changed["model/out/provenance/rules/v.whole.json"]["change"] == "changed"
    assert "repo/tables/rounding.json" not in changed  # the same bytes as in candidate 1
    written = json.loads((second / "files" / "repo" / "tables" / "unit_values.json").read_bytes())
    assert written["rows"][0][2] == 1_300_000

    summary_doc = json.loads((second / "summary.json").read_text(encoding="utf-8"))
    assert (summary_doc["number"], summary_doc["previous"], summary_doc["model_id"]) == (2, 1, "tabular")
    files = {path.relative_to(second / "files").as_posix() for path in (second / "files").rglob("*") if path.is_file()}
    assert {entry["name"] for entry in summary_doc["files"]} == files
    assert {"model/out/thresholds.json", "repo/tables/rounding.json", "repo/tables/unit_values.json"} <= files


def test_a_fake_memory_pause_stops_the_workers_and_resumes_them_later(example, tmp_path, make_watcher):
    home = tmp_path / "engines" / "fake"
    shutil.copytree(FAKE_HOME, home)
    manifest = example / MODEL / "model.json"
    doc = json.loads(manifest.read_text(encoding="utf-8"))
    doc["engines"] = [{"engine_id": "fake", "interface": "acmodel.engine.v2", "min_version": "1"}]
    manifest.write_text(json.dumps(doc), encoding="utf-8")
    host, stamps = FakeHost(until=6_000), FakeStamps()
    host.at(2_000, lambda: setattr(host, "available", 2_000))  # below 2560: the running watch pauses
    host.at(3_000, lambda: setattr(host, "available", 3_500))  # paused, it opens again only at 4096
    host.at(4_000, lambda: setattr(host, "available", 8_192))
    watcher = make_watcher(
        example, host, stamps, compute={"workers": {"max": 2}}, engines={"roots": {"fake": str(home)}}
    )
    alive: list[int] = []
    host.observe = lambda: alive.append(len(watcher.workers.pids()))

    watcher.run()

    assert alive == [2, 2, 0, 0, 2, 2, 2]  # after each poll, from 0 to 6 000
    assert watcher.workers.pids() == []  # stopped at the end
    log = events(watcher.folder)
    assert [event["event"] for event in log] == [
        "started", "workers_started", "refreshed", "staged", "paused", "workers_stopped", "resumed",
        "workers_started", "stopped",
    ]
    by_name = {event["event"]: event for event in log}
    assert [(entry["code"], entry["detail"]["reason"]) for entry in by_name["paused"]["reasons"]] == [
        ("host_gate_paused", "memory_below_pause")
    ]
    assert (at_ms(by_name["paused"]), by_name["workers_stopped"]["workers"], at_ms(by_name["resumed"])) == (
        2_000, 2, 4_000
    )
    assert [(at_ms(event), event["workers"]) for event in log if event["event"] == "workers_started"] == [
        (0, 2), (4_000, 2)
    ]


def test_a_restart_resumes_from_the_cache_with_no_recompute(
    example, tmp_path, make_system, app_dir, write_config, check_envelope, monkeypatch
):
    state_root = tmp_path / "state_root"
    write_config({
        "schema_version": "acmodel.config.v1", "paths": {"state_root": str(state_root)},
        "watch": {"poll_interval_ms": 1000},
    })
    hosts: list[FakeHost] = []
    monkeypatch.setattr(windows, "current", lambda platform: hosts[-1])

    def watch() -> dict[str, Any]:
        hosts.append(FakeHost(until=3_000))
        system = make_system(prefix=app_dir / "venv")
        status = cli.main(["watch", str(example / MODEL), "--state", str(example / STATE)], system)
        doc = check_envelope(system.stdout.getvalue(), status)
        assert status == 0 and doc["command"] == "watch"
        assert len(hosts[-1].applied) == 1  # the host caps first
        return doc["data"]

    def used() -> str:
        """The cache's use counter: every refresh that opens the cache advances it."""
        with closing(sqlite3.connect(state_root / "cache" / "index.sqlite")) as index:
            return index.execute("SELECT value FROM meta WHERE name = 'used'").fetchone()[0]

    first = watch()
    counter = used()
    second = watch()

    assert (first["resumed"], first["refreshes"], first["staged"], first["candidate"]) == (False, 1, [1], 1)
    assert (second["resumed"], second["refreshes"], second["staged"]) == (True, 0, [])
    assert (second["watch"], second["run_id"], second["candidate"]) == (first["watch"], first["run_id"], 1)
    assert second["polls"] == 4
    assert used() == counter  # nothing was computed: the cache was not even opened again
    assert [event["event"] for event in events(Path(first["watch"]))] == [
        "started", "refreshed", "staged", "stopped", "started", "stopped"
    ]
