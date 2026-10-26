"""PY-7: the host layer.

- Gate decisions from fake readings: start at 4096 MiB, pause below 2560, resume at 4096; the CPU load.
- A planned run above 3072 MiB exits 4 with the estimate.
- Pause and resume under a fake clock: a quiet window and the PAUSE file.
- A fake kernel32 whose call returns 0: host_priority_not_applied, exit 4.
- Windows leg: a process that applied the caps is below normal and in a Job with the memory limit,
  and a child it starts afterwards is in the same Job and below normal.
"""

from __future__ import annotations

import io
import json
import subprocess
import sys
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import config
from acmodel.envelope import Failure, build, write
from acmodel.host import CHECKPOINT, START, Gate, Waited, check_plan, windows

MODEL_DIR = Path(__file__).resolve().parents[1]
PROBE = Path(__file__).resolve().parent / "host_probe.py"
MONDAY = datetime(2030, 1, 7, 4, 39)
BELOW_NORMAL = 0x00004000
NORMAL = 0x00000020
MINUTE = 60_000


class FakeHost:
    """Fake readings and a fake clock. sleep_ms advances the clock, then makes the changes the test
    scheduled up to the new time. With idle_cpu the CPU counters advance with the clock, all idle."""

    def __init__(self, available_mib: int, idle_cpu: bool = True) -> None:
        self.available = available_mib
        self.idle_cpu = idle_cpu
        self.idle = 0
        self.total = 0
        self.ms = 0
        self.scheduled: list[tuple[int, Callable[[], Any]]] = []
        self.sleeps = 0

    def available_mib(self) -> int:
        return self.available

    def cpu_times(self) -> tuple[int, int]:
        return self.idle, self.total

    def now(self) -> datetime:
        return MONDAY + timedelta(milliseconds=self.ms)

    def monotonic_ms(self) -> int:
        return self.ms

    def advance(self, ms: int) -> None:
        self.ms += ms
        if self.idle_cpu:
            self.idle += ms
            self.total += ms

    def at(self, ms: int, change: Callable[[], Any]) -> None:
        self.scheduled.append((ms, change))

    def sleep_ms(self, ms: int) -> None:
        self.sleeps += 1
        assert self.sleeps <= 10_000, "the gate never opened"
        self.advance(ms)
        due = sorted((item for item in self.scheduled if item[0] <= self.ms), key=lambda item: item[0])
        self.scheduled = [item for item in self.scheduled if item[0] > self.ms]
        for _, change in due:
            change()


@pytest.fixture
def load_caps(write_config, local_app_data) -> Callable[..., Any]:
    """The host caps through the configuration: its defaults, or the given `host` section."""

    def load(host: dict[str, Any] | None = None):
        if host is not None:
            write_config({"schema_version": "acmodel.config.v1", "host": host})
        return config.load({"LOCALAPPDATA": str(local_app_data)}).host

    return load


def held(entries: list[dict[str, Any]]) -> list[tuple[str, dict[str, Any]]]:
    """(reason, other values) of each host_gate_paused entry."""
    assert {entry["code"] for entry in entries} <= {"host_gate_paused"}
    return [
        (entry["detail"]["reason"], {key: value for key, value in entry["detail"].items() if key != "reason"})
        for entry in entries
    ]


def exit_status(errors: list[dict[str, Any]], check_envelope) -> int:
    """The exit status of a failed command with these errors, after checking its envelope
    against contracts/codes.json."""
    stdout = io.StringIO()
    status = write(build(None, {}, errors), stdout, io.StringIO())
    check_envelope(stdout.getvalue(), status)
    return status


def test_memory_gate_starts_at_4096_pauses_below_2560_and_resumes_at_4096(load_caps, check_envelope):
    caps = load_caps()
    assert (caps.start_min_mib, caps.pause_below_mib) == (4096, 2560)
    host = FakeHost(available_mib=4095)
    gate = Gate(caps, host)

    with pytest.raises(Failure) as caught:
        gate.require(START)
    assert held(caught.value.errors) == [("memory_below_start", {"available_mib": 4095, "limit_mib": 4096})]
    assert exit_status(caught.value.errors, check_envelope) == 4

    below_start = "memory_below_start", {"limit_mib": 4096}
    steps = [
        (4096, START, []),
        (2560, CHECKPOINT, []),  # a running batch goes on down to 2560
        (2559, CHECKPOINT, [("memory_below_pause", {"limit_mib": 2560})]),
        (4095, CHECKPOINT, [below_start]),  # paused: it reopens only at the start level
        (4096, CHECKPOINT, []),  # resumed
        (3000, CHECKPOINT, []),  # running again
        (3000, START, [below_start]),  # a new batch needs 4096
    ]
    for available, phase, expected in steps:
        host.available = available
        expected = [(reason, {"available_mib": available, **values}) for reason, values in expected]
        assert held(gate.check(phase)) == expected, (available, phase)
        assert gate.paused is bool(expected)


def test_cpu_load_above_900_over_a_window_holds_the_gate(load_caps):
    caps = load_caps()
    assert (caps.pause_load_milli, caps.cpu_window_ms) == (900, 5000)
    host = FakeHost(available_mib=8192, idle_cpu=False)
    gate = Gate(caps, host)  # the first CPU reading: (0, 0) at 0 ms

    above = [("cpu_load", {"load_milli": 901, "limit_milli": 900})]
    steps = [
        (0, 0, 0, []),  # one reading gives no load
        (4999, 0, 9999, []),  # all busy, but shorter than the window: still no load
        (5000, 1000, 10000, []),  # 900 over 0..5000 ms is not above 900
        (10000, 1990, 20000, above),  # 901 over 5000..10000 ms
        (12000, 1990, 24000, above),  # too short a span since 10000 ms: the last load stands
        (15000, 6990, 30000, []),  # 500 over 10000..15000 ms
    ]
    for ms, idle, total, expected in steps:
        host.ms, host.idle, host.total = ms, idle, total
        assert held(gate.check(CHECKPOINT)) == expected, ms


@pytest.mark.parametrize(
    ("workers", "per_worker_mib", "main_mib", "estimate"),
    [
        (2, 1024, 1024, 3072),
        (2, 1500, 1024, 3072),  # a worker counts at most the Job's 1024 per process
        (3, 1024, 1, 3073),
        (4, 600, 800, 3200),
    ],
    ids=["at_the_line", "worker_estimate_capped", "one_above", "above"],
)
def test_a_plan_above_3072_exits_4_with_the_estimate(
    load_caps, check_envelope, workers, per_worker_mib, main_mib, estimate
):
    caps = load_caps()
    assert (caps.report_plan_above_mib, caps.worker_memory_mib) == (3072, 1024)

    if estimate <= 3072:
        assert check_plan(caps, workers=workers, per_worker_mib=per_worker_mib, main_mib=main_mib) == estimate
        return
    with pytest.raises(Failure) as caught:
        check_plan(caps, workers=workers, per_worker_mib=per_worker_mib, main_mib=main_mib)
    assert held(caught.value.errors) == [("plan_above_report", {"estimate_mib": estimate, "limit_mib": 3072})]
    assert exit_status(caught.value.errors, check_envelope) == 4


def test_quiet_windows_and_the_pause_file_pause_and_resume_under_a_fake_clock(load_caps, tmp_path):
    pause = tmp_path / "PAUSE"
    caps = load_caps({
        "quiet_windows": [
            {"days": ["mon", "thu"], "start": "04:40", "end": "05:40"},
            {"dates": ["2030-01-08"], "start": "23:50", "end": "00:10"},
        ],
        "pause_file": str(pause),
        "pause_poll_ms": MINUTE,
    })
    assert MONDAY.weekday() == 0
    host = FakeHost(available_mib=8192)  # Monday 04:39
    gate = Gate(caps, host)
    assert gate.check(START) == []

    host.advance(MINUTE)  # 04:40: the Monday window
    assert held(gate.check(CHECKPOINT)) == [("quiet_window", {"window": 0})]

    # The PAUSE file appears at 05:00; the window ends at 05:40; the file goes at 06:00.
    host.at(host.ms + 20 * MINUTE, lambda: pause.write_bytes(b""))
    host.at(host.ms + 80 * MINUTE, pause.unlink)
    assert gate.wait(CHECKPOINT) == Waited(80 * MINUTE, ("pause_file", "quiet_window"))
    assert host.now() == datetime(2030, 1, 7, 6, 0)
    assert host.sleeps == 80
    assert not gate.paused

    window_1 = [("quiet_window", {"window": 1})]
    moments = [
        (datetime(2030, 1, 8, 4, 40), []),  # Tuesday is not a day of window 0
        (datetime(2030, 1, 8, 23, 49), []),
        (datetime(2030, 1, 8, 23, 50), window_1),
        (datetime(2030, 1, 9, 0, 9), window_1),  # after midnight it is still the 8th's window
        (datetime(2030, 1, 9, 0, 10), []),
        (datetime(2030, 1, 9, 23, 55), []),  # the 9th is not a date of window 1
        (datetime(2030, 1, 10, 4, 40), [("quiet_window", {"window": 0})]),  # Thursday
        (datetime(2030, 1, 10, 5, 40), []),  # the end is exclusive
    ]
    for moment, expected in moments:
        host.advance((moment - host.now()) // timedelta(milliseconds=1))
        assert held(gate.check(START)) == expected, moment


class FakeKernel32:
    """Only the priority calls: apply must stop before it creates a Job."""

    PSEUDO_HANDLE = 2**64 - 1  # (HANDLE)-1, as the typed GetCurrentProcess returns it

    def __init__(self, set_result: int, read_back: int) -> None:
        self.set_result = set_result
        self.read_back = read_back
        self.calls: list[tuple[Any, ...]] = []

    def GetCurrentProcess(self) -> int:
        return self.PSEUDO_HANDLE

    def SetPriorityClass(self, handle: int, priority_class: int) -> int:
        self.calls.append(("SetPriorityClass", handle, priority_class))
        return self.set_result

    def GetPriorityClass(self, handle: int) -> int:
        self.calls.append(("GetPriorityClass", handle))
        return self.read_back


@pytest.mark.parametrize(
    ("set_result", "read_back", "expected"),
    [
        (0, NORMAL, {"stage": "set", "os_error": 5}),
        (1, NORMAL, {"stage": "verify", "found": NORMAL}),
    ],
    ids=["set_returns_0", "process_stays_normal"],
)
def test_a_fake_kernel32_whose_call_returns_0_is_host_priority_not_applied_exit_4(
    load_caps, check_envelope, set_result, read_back, expected
):
    kernel32 = FakeKernel32(set_result, read_back)
    host = windows.WindowsHost(kernel32, last_error=lambda: 5)

    with pytest.raises(Failure) as caught:
        host.apply(load_caps())

    [entry] = caught.value.errors
    assert entry["code"] == "host_priority_not_applied"
    assert entry["detail"] == {"process": "current", "priority": "below_normal", "expected": BELOW_NORMAL, **expected}
    assert exit_status(caught.value.errors, check_envelope) == 4
    assert kernel32.calls[0] == ("SetPriorityClass", FakeKernel32.PSEUDO_HANDLE, BELOW_NORMAL)
    assert host.job is None


@pytest.mark.skipif(sys.platform != "win32", reason="priority classes and Job objects exist only on Windows")
def test_windows_leg_the_caps_are_applied_and_a_child_started_afterwards_inherits_them(tmp_path):
    proc = subprocess.run(
        [sys.executable, "-I", "-B", str(PROBE), str(MODEL_DIR / "src"), str(tmp_path)],
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )

    assert proc.returncode == 0, (proc.stdout, proc.stderr)
    report = json.loads(proc.stdout)
    # The probe itself: below normal, and its own Job carries the limits.
    assert report["priority"] == BELOW_NORMAL
    assert report["own_job_queried"] == [True, True]
    memory_and_priority = windows.JOB_OBJECT_LIMIT_PROCESS_MEMORY | windows.JOB_OBJECT_LIMIT_PRIORITY_CLASS
    assert report["limit_flags"] & memory_and_priority == memory_and_priority
    assert report["process_memory_limit"] == 1024 * 2**20
    assert report["job_priority_class"] == BELOW_NORMAL
    hard_cap = windows.JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | windows.JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP
    assert report["cpu_rate_flags"] & hard_cap == hard_cap
    assert report["cpu_rate"] == 2500  # 250 milli = 2500 cycles per 10,000
    # A child started afterwards: in the same Job and below normal, by the OS and by verify_child.
    assert report["child"] == {
        "opened": True,
        "priority": BELOW_NORMAL,
        "in_job_queried": True,
        "in_job": True,
        "closed": True,
        "verify_errors": [],
    }
    assert report["child_exit"] == 0
