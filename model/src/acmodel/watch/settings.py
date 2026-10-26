"""The keys of `watch` (contracts/cli.md, "Configuration" and "Automatic refresh"; #424 design-py 5.6).

- `watch.poll_interval_ms`: how often the watched files are scanned (size and modification time, then the
  sha256 of the files whose size or time changed);
- `watch.debounce_ms`: a change opens a window of this length; at its end one refresh covers every change seen in
  it;
- `watch.keep_workers_warm`: the engine workers stay started between refreshes, so that an engine's context is
  built once and not per change; a held host gate stops them.

Every key has a default; config.py checks the section's members and hands it here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable

DEFAULT_POLL_INTERVAL_MS = 5000
DEFAULT_DEBOUNCE_MS = 5000
DEFAULT_KEEP_WORKERS_WARM = True
MAX_MS = 2**53 - 1

MEMBERS = {"poll_interval_ms", "debounce_ms", "keep_workers_warm"}

# invalid(reason, pointer parts, **values) -> the config_invalid Failure to raise (config.py).
Invalid = Callable[..., Exception]


@dataclass(frozen=True)
class WatchSettings:
    poll_interval_ms: int = DEFAULT_POLL_INTERVAL_MS
    debounce_ms: int = DEFAULT_DEBOUNCE_MS
    keep_workers_warm: bool = DEFAULT_KEEP_WORKERS_WARM


def read(section: dict[str, Any], invalid: Invalid) -> WatchSettings:
    """The settings from the `watch` section: the times are integers from 1 to 2^53 - 1, the switch a boolean."""
    values: dict[str, Any] = {}
    for key in ("poll_interval_ms", "debounce_ms"):
        if key in section:
            value = section[key]
            if type(value) is not int or not 1 <= value <= MAX_MS:
                raise invalid("value_invalid", ("watch", key))
            values[key] = value
    if "keep_workers_warm" in section:
        if type(section["keep_workers_warm"]) is not bool:
            raise invalid("value_invalid", ("watch", "keep_workers_warm"))
        values["keep_workers_warm"] = section["keep_workers_warm"]
    return WatchSettings(**values)
