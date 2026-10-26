"""The watch folder's own files (contracts/cli.md, "Automatic refresh").

- `events.jsonl`: the event log, one line of canonical JSON per event (`acmodel.watch.event.v1`: `at`, the local
  time, `event` and its values). It is only ever appended to. Each event also goes to stderr as one line.
- `watch.json` (`acmodel.watch.v1`): what a restart resumes from. It holds every watched file as the last refresh
  saw it (stamp and sha256), that refresh's run id and imported snapshot, and the last delivery candidate. It is
  written after every refresh, as a temporary file beside it and then a rename, so it is never half written.
  One that does not read is `watch_failed` (`resume`), and it is never replaced.
"""

from __future__ import annotations

import json
import os
import re
import tempfile
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Callable, TextIO

from acmodel.cache.keys import canonical, canonical_text
from acmodel.watch.files import Seen, Stamp, failed

SCHEMA = "acmodel.watch.v1"
EVENT_SCHEMA = "acmodel.watch.event.v1"
STATE = "watch.json"
EVENTS = "events.jsonl"
_HASH = re.compile(r"[0-9a-f]{64}")


def write_atomic(path: Path, data: bytes, stage: str) -> None:
    """Write `data` at `path` as a temporary file beside it, then a rename; a failure is `watch_failed` (`stage`)."""
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        handle, temporary = tempfile.mkstemp(dir=path.parent, prefix=path.name + ".", suffix=".tmp")
        with os.fdopen(handle, "wb") as file:
            file.write(data)
        os.replace(temporary, path)
    except OSError as exc:
        raise failed(stage, path, exc) from exc


@dataclass
class Saved:
    """What watch.json holds besides the model and the state it is for."""

    files: dict[str, Seen]
    last: dict[str, str] | None  # the last refresh: `run_id` and the imported snapshot (`state`)
    candidate: dict[str, Any] | None  # the last delivery candidate: `number` and `sha256`


def _is_hash(value: Any) -> bool:
    return isinstance(value, str) and _HASH.fullmatch(value) is not None


def _seen(value: Any) -> bool:
    return (
        isinstance(value, dict) and set(value) == {"size", "mtime_ns", "sha256"}
        and type(value["size"]) is int and value["size"] >= 0 and type(value["mtime_ns"]) is int
        and (value["sha256"] is None or _is_hash(value["sha256"]))
    )


def _problem(doc: Any, model: str, state: str) -> str | None:
    """The first member of watch.json that does not read, or None."""
    if not isinstance(doc, dict) or set(doc) != {"schema_version", "model", "state", "files", "last", "candidate"}:
        return "members"
    if doc["schema_version"] != SCHEMA:
        return "schema_version"
    if doc["model"] != model:
        return "model"
    if doc["state"] != state:
        return "state"
    files = doc["files"]
    if not isinstance(files, dict) or not all(_seen(entry) for entry in files.values()):
        return "files"
    last = doc["last"]
    if last is not None and not (
        isinstance(last, dict) and set(last) == {"run_id", "state"} and _is_hash(last["run_id"])
        and isinstance(last["state"], str)
    ):
        return "last"
    candidate = doc["candidate"]
    if candidate is not None and not (
        isinstance(candidate, dict) and set(candidate) == {"number", "sha256"}
        and type(candidate["number"]) is int and candidate["number"] >= 1 and _is_hash(candidate["sha256"])
    ):
        return "candidate"
    return None


class Journal:
    def __init__(self, folder: Path, now: Callable[[], datetime], stderr: TextIO) -> None:
        self.folder = folder
        self._now = now
        self._stderr = stderr

    def event(self, name: str, **values: Any) -> None:
        """Append one event to the log, and its line to stderr."""
        at = self._now().isoformat(timespec="milliseconds")
        line = canonical_text({"schema_version": EVENT_SCHEMA, "at": at, "event": name, **values})
        path = self.folder / EVENTS
        try:
            self.folder.mkdir(parents=True, exist_ok=True)
            with open(path, "a", encoding="utf-8", newline="\n") as log:
                log.write(line + "\n")
        except OSError as exc:
            raise failed("journal", path, exc) from exc
        # ASCII only, so that the line survives any console encoding.
        shown = json.dumps(values, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
        self._stderr.write(f"acmodel: watch {at} {name} {shown}\n")
        self._stderr.flush()

    def load(self, model: str, state: str) -> Saved | None:
        """watch.json, or None when there is none."""
        path = self.folder / STATE
        try:
            raw = path.read_bytes()
        except FileNotFoundError:
            return None
        except OSError as exc:
            raise failed("resume", path, exc) from exc
        try:
            doc = json.loads(raw.decode("utf-8"))
        except ValueError as exc:  # bad UTF-8, or not JSON
            raise failed("resume", path, exc, found="not_json") from exc
        problem = _problem(doc, model, state)
        if problem is not None:
            raise failed("resume", path, found=problem)
        files = {
            key: Seen(Stamp(entry["size"], entry["mtime_ns"]), entry["sha256"]) for key, entry in doc["files"].items()
        }
        return Saved(files, doc["last"], doc["candidate"])

    def save(self, model: str, state: str, saved: Saved) -> None:
        doc = {
            "schema_version": SCHEMA,
            "model": model,
            "state": state,
            "files": {
                key: {"size": seen.stamp.size, "mtime_ns": seen.stamp.mtime_ns, "sha256": seen.sha256}
                for key, seen in sorted(saved.files.items())
            },
            "last": saved.last,
            "candidate": saved.candidate,
        }
        write_atomic(self.folder / STATE, canonical(doc), "journal")
