"""The host caps: the configuration keys under `compute` and `host` (contracts/cli.md, Host caps).

Every key has a default; the defaults are the 10-24 rules (4096 MiB to start, pause below 2560,
1024 MiB per process, report a plan above 3072, below-normal priority). Times of day are the host's
local wall-clock time.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import date, datetime, timedelta
from pathlib import Path
from typing import Any, Callable, Sequence

# The Windows priority classes (GetPriorityClass values) the tool may run at.
PRIORITY_CLASSES = {"below_normal": 0x00004000, "idle": 0x00000040}
DAYS = ("mon", "tue", "wed", "thu", "fri", "sat", "sun")
PAUSE_FILE_NAME = "PAUSE"

DEFAULT_PRIORITY = "below_normal"
DEFAULT_CPU_RATE_MILLI = 250
DEFAULT_WORKER_MEMORY_MIB = 1024
DEFAULT_START_MIN_MIB = 4096
DEFAULT_PAUSE_BELOW_MIB = 2560
DEFAULT_REPORT_PLAN_ABOVE_MIB = 3072
DEFAULT_PAUSE_LOAD_MILLI = 900
DEFAULT_CPU_WINDOW_MS = 5000
DEFAULT_PAUSE_POLL_MS = 5000

_TIME = re.compile(r"([01][0-9]|2[0-3]):([0-5][0-9])")
_DATE = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}")
_NESTED = {
    ("compute", "job"): {"cpu_rate_milli", "worker_memory_mib"},
    ("host", "memory"): {"start_min_mib", "pause_below_mib", "report_plan_above_mib"},
    ("host", "cpu"): {"pause_load_milli", "window_ms"},
}
_WINDOW_MEMBERS = {"days", "dates", "start", "end"}

# invalid(reason, pointer parts, **values) -> the config_invalid Failure to raise (config.py).
Invalid = Callable[..., Exception]


@dataclass(frozen=True)
class QuietWindow:
    """No compute from `start` to `end` (minutes after local midnight) on the listed weekdays
    (Monday 0) or dates. An `end` at or before `start` ends the next day; that part belongs to the
    day the window started."""

    days: frozenset[int]
    dates: frozenset[date]
    start: int
    end: int

    def active(self, now: datetime) -> bool:
        minute = now.hour * 60 + now.minute
        today = now.date()
        if self.start < self.end:
            return self._on(today) and self.start <= minute < self.end
        return (self._on(today) and minute >= self.start) or (
            self._on(today - timedelta(days=1)) and minute < self.end
        )

    def _on(self, day: date) -> bool:
        return day in self.dates if self.dates else day.weekday() in self.days


@dataclass(frozen=True)
class HostCaps:
    priority: str
    cpu_rate_milli: int
    worker_memory_mib: int
    start_min_mib: int
    pause_below_mib: int
    report_plan_above_mib: int
    pause_load_milli: int
    cpu_window_ms: int
    pause_poll_ms: int
    quiet_windows: tuple[QuietWindow, ...]
    pause_file: Path

    @property
    def priority_class(self) -> int:
        return PRIORITY_CLASSES[self.priority]


def read(compute: dict[str, Any], host: dict[str, Any], state_root: Path, invalid: Invalid) -> HostCaps:
    """The caps from the `compute` and `host` sections (their own members are checked by config.py)."""
    job = _object(compute, ("compute", "job"), invalid)
    memory = _object(host, ("host", "memory"), invalid)
    cpu = _object(host, ("host", "cpu"), invalid)
    priority = compute.get("priority", DEFAULT_PRIORITY)
    if not isinstance(priority, str) or priority not in PRIORITY_CLASSES:
        raise invalid("value_invalid", ("compute", "priority"))
    caps = HostCaps(
        priority=priority,
        cpu_rate_milli=_integer(job, ("compute", "job", "cpu_rate_milli"), DEFAULT_CPU_RATE_MILLI, 1000, invalid),
        worker_memory_mib=_integer(
            job, ("compute", "job", "worker_memory_mib"), DEFAULT_WORKER_MEMORY_MIB, None, invalid
        ),
        start_min_mib=_integer(memory, ("host", "memory", "start_min_mib"), DEFAULT_START_MIN_MIB, None, invalid),
        pause_below_mib=_integer(
            memory, ("host", "memory", "pause_below_mib"), DEFAULT_PAUSE_BELOW_MIB, None, invalid
        ),
        report_plan_above_mib=_integer(
            memory, ("host", "memory", "report_plan_above_mib"), DEFAULT_REPORT_PLAN_ABOVE_MIB, None, invalid
        ),
        pause_load_milli=_integer(cpu, ("host", "cpu", "pause_load_milli"), DEFAULT_PAUSE_LOAD_MILLI, 1000, invalid),
        cpu_window_ms=_integer(cpu, ("host", "cpu", "window_ms"), DEFAULT_CPU_WINDOW_MS, None, invalid),
        pause_poll_ms=_integer(host, ("host", "pause_poll_ms"), DEFAULT_PAUSE_POLL_MS, None, invalid),
        quiet_windows=_windows(host.get("quiet_windows", []), invalid),
        pause_file=_pause_file(host, state_root, invalid),
    )
    if caps.pause_below_mib > caps.start_min_mib:  # the gate would reopen below the line it pauses at
        raise invalid("value_invalid", ("host", "memory", "pause_below_mib"))
    return caps


def _object(parent: dict[str, Any], parts: tuple[str, str], invalid: Invalid) -> dict[str, Any]:
    value = parent.get(parts[1], {})
    if not isinstance(value, dict):
        raise invalid("value_invalid", parts)
    for key in sorted(value):
        if key not in _NESTED[parts]:
            raise invalid("member_unknown", (*parts, key), key=key)
    return value


def _integer(section: dict[str, Any], parts: Sequence[str], default: int, high: int | None, invalid: Invalid) -> int:
    """An integer of at least 1 (and at most `high`)."""
    value = section.get(parts[-1], default)
    if type(value) is not int or value < 1 or (high is not None and value > high):
        raise invalid("value_invalid", tuple(parts))
    return value


def _pause_file(host: dict[str, Any], state_root: Path, invalid: Invalid) -> Path:
    if "pause_file" not in host:
        return state_root / PAUSE_FILE_NAME
    value = host["pause_file"]
    if not isinstance(value, str) or not value or not Path(value).is_absolute():
        raise invalid("value_invalid", ("host", "pause_file"))
    return Path(value)


def _windows(value: Any, invalid: Invalid) -> tuple[QuietWindow, ...]:
    if not isinstance(value, list):
        raise invalid("value_invalid", ("host", "quiet_windows"))
    return tuple(_window(doc, ("host", "quiet_windows", str(index)), invalid) for index, doc in enumerate(value))


def _window(doc: Any, at: tuple[str, ...], invalid: Invalid) -> QuietWindow:
    if not isinstance(doc, dict):
        raise invalid("value_invalid", at)
    for key in sorted(doc):
        if key not in _WINDOW_MEMBERS:
            raise invalid("member_unknown", (*at, key), key=key)
    for key in ("start", "end"):
        if key not in doc:
            raise invalid("member_missing", (*at, key), key=key)
    start = _minutes(doc["start"], (*at, "start"), invalid)
    end = _minutes(doc["end"], (*at, "end"), invalid)
    if start == end:
        raise invalid("value_invalid", (*at, "end"))
    if "days" in doc and "dates" in doc:  # exactly one of the two
        raise invalid("value_invalid", (*at, "dates"))
    if "days" in doc:
        days = _items(doc["days"], (*at, "days"), lambda item: item if item in DAYS else None, invalid)
        return QuietWindow(frozenset(DAYS.index(day) for day in days), frozenset(), start, end)
    if "dates" in doc:
        dates = _items(doc["dates"], (*at, "dates"), _date, invalid)
        return QuietWindow(frozenset(), frozenset(dates), start, end)
    raise invalid("member_missing", (*at, "days"), key="days")


def _minutes(value: Any, at: tuple[str, ...], invalid: Invalid) -> int:
    match = _TIME.fullmatch(value) if isinstance(value, str) else None
    if match is None:
        raise invalid("value_invalid", at)
    return int(match.group(1)) * 60 + int(match.group(2))


def _date(text: str) -> date | None:
    if not _DATE.fullmatch(text):
        return None
    try:
        return date.fromisoformat(text)
    except ValueError:  # e.g. a 30th of February: refused below, with its pointer
        return None


def _items(value: Any, at: tuple[str, ...], parse: Callable[[str], Any], invalid: Invalid) -> list[Any]:
    """A non-empty list of distinct strings, each of which `parse` accepts."""
    if not isinstance(value, list) or not value:
        raise invalid("value_invalid", at)
    items: list[Any] = []
    for index, item in enumerate(value):
        parsed = parse(item) if isinstance(item, str) else None
        if parsed is None or parsed in items:
            raise invalid("value_invalid", (*at, str(index)))
        items.append(parsed)
    return items
