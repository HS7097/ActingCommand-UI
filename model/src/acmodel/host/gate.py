"""The host gate: whether compute may start or go on, decided at checkpoints (contracts/cli.md).

- **Memory.** A batch starts only at or above `host.memory.start_min_mib` of available host memory.
  Inside a batch, a checkpoint pauses below `host.memory.pause_below_mib`. A paused gate reopens
  only at the start level again.
- **Quiet windows, the PAUSE file and the CPU load** hold the gate in both phases: a local time
  inside `host.quiet_windows`; the file `host.pause_file` existing; the host CPU load, averaged over
  at least `host.cpu.window_ms`, above `host.cpu.pause_load_milli`.
- **The planned memory** of a run above `host.memory.report_plan_above_mib` is reported before the
  run starts (exit 4 with the estimate).

Every hold is one `host_gate_paused` entry. The OS is reached only through a `Host`: the real one
is `windows.WindowsHost`, and the tests pass a fake.
"""

from __future__ import annotations

import os
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Protocol

from acmodel.codes import HOST_GATE_PAUSED, HOST_READING_FAILED
from acmodel.envelope import Failure, error
from acmodel.host.caps import HostCaps

START = "start"  # before a batch
CHECKPOINT = "checkpoint"  # inside a batch


class Host(Protocol):
    def available_mib(self) -> int:
        """Available physical memory of the host, in MiB."""

    def cpu_times(self) -> tuple[int, int]:
        """Cumulative (idle, total) CPU time of the host over all processors, in one unit."""

    def now(self) -> datetime:
        """Local wall-clock time."""

    def monotonic_ms(self) -> int: ...

    def sleep_ms(self, ms: int) -> None: ...


class SystemClock:
    """The real clock of a Host."""

    def now(self) -> datetime:
        return datetime.now()

    def monotonic_ms(self) -> int:
        return time.monotonic_ns() // 1_000_000

    def sleep_ms(self, ms: int) -> None:
        time.sleep(ms / 1000)


@dataclass(frozen=True)
class Waited:
    waited_ms: int
    reasons: tuple[str, ...]  # every reason that held the gate meanwhile, sorted


def held(reason: str, **values: Any) -> dict[str, Any]:
    return error(HOST_GATE_PAUSED, {"reason": reason, **values})


def pause_file_present(path: Path) -> bool:
    try:
        os.stat(path)
    except FileNotFoundError:
        return False
    except OSError as exc:  # not "absent": the gate cannot tell, so it does not guess
        raise Failure(error(HOST_READING_FAILED, {
            "reading": "pause_file",
            "path": str(path),
            "exception_type": type(exc).__name__,
            "raw_text": str(exc),
        })) from exc
    return True


def check_plan(caps: HostCaps, *, workers: int, per_worker_mib: int, main_mib: int) -> int:
    """The planned memory of a run: workers x (the recorded peak or the engine's estimate, at most
    the Job's per-process limit) + the main process. Above the report line the run does not start:
    `host_gate_paused` (`plan_above_report`) with the estimate, exit 4."""
    estimate = workers * min(per_worker_mib, caps.worker_memory_mib) + main_mib
    if estimate > caps.report_plan_above_mib:
        raise Failure(held("plan_above_report", estimate_mib=estimate, limit_mib=caps.report_plan_above_mib))
    return estimate


class Gate:
    def __init__(self, caps: HostCaps, host: Host) -> None:
        self._caps = caps
        self._host = host
        self.paused = False
        # The CPU load needs two readings; the first one is taken here.
        self._sample = (host.monotonic_ms(), *host.cpu_times())
        self._load: int | None = None

    def check(self, phase: str) -> list[dict[str, Any]]:
        """The `host_gate_paused` entries that hold the gate now; empty when it is open."""
        if phase not in (START, CHECKPOINT):
            raise ValueError(f"unknown gate phase: {phase!r}")
        caps = self._caps
        reasons = []
        available = self._host.available_mib()
        if phase == START or self.paused:
            if available < caps.start_min_mib:
                reasons.append(held("memory_below_start", available_mib=available, limit_mib=caps.start_min_mib))
        elif available < caps.pause_below_mib:
            reasons.append(held("memory_below_pause", available_mib=available, limit_mib=caps.pause_below_mib))
        now = self._host.now()
        reasons.extend(
            held("quiet_window", window=index) for index, window in enumerate(caps.quiet_windows) if window.active(now)
        )
        if pause_file_present(caps.pause_file):
            reasons.append(held("pause_file", path=str(caps.pause_file)))
        load = self._cpu_load()
        if load is not None and load > caps.pause_load_milli:
            reasons.append(held("cpu_load", load_milli=load, limit_milli=caps.pause_load_milli))
        self.paused = bool(reasons)
        return reasons

    def require(self, phase: str) -> None:
        """For a one-shot command: a held gate ends it with exit 4; a later run resumes."""
        reasons = self.check(phase)
        if reasons:
            raise Failure(*reasons)

    def wait(self, phase: str) -> Waited:
        """Check every `host.pause_poll_ms` until the gate is open."""
        began = self._host.monotonic_ms()
        seen: set[str] = set()
        while reasons := self.check(phase):
            seen.update(entry["detail"]["reason"] for entry in reasons)
            self._host.sleep_ms(self._caps.pause_poll_ms)
        return Waited(self._host.monotonic_ms() - began, tuple(sorted(seen)))

    def _cpu_load(self) -> int | None:
        """The host CPU load in milli over the last span of at least `host.cpu.window_ms`.
        None until the first span is complete: one reading gives no load."""
        at = self._host.monotonic_ms()
        idle, total = self._host.cpu_times()
        since, idle_before, total_before = self._sample
        if at - since < self._caps.cpu_window_ms:
            return self._load
        idle_delta, total_delta = idle - idle_before, total - total_before
        if total_delta <= 0 or not 0 <= idle_delta <= total_delta:
            raise Failure(error(HOST_READING_FAILED, {"reading": "cpu_times"}))
        self._load = (total_delta - idle_delta) * 1000 // total_delta
        self._sample = (at, idle, total)
        return self._load
