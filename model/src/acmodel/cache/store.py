"""The cache's store (#424 design section 5.5), under `paths.state_root`:

- `cache/index.sqlite`: the index (sqlite3, WAL). One row per computation of a node, its trace: the node
  key, the spec hash, the kind, the node, its value hash, what it read in order with each value hash, the
  value's size, its last use (a counter that every run advances) and the time it took.
- `cache/blobs/<first two hex digits>/<value hash>`: each value as canonical JSON, named by its sha256 and
  shared by every trace with that value.
- `runs/<run id>/outputs.json`: the outputs of a run.

Only the main process writes. A blob is used only after its bytes hash to its name; otherwise it is a
`CorruptEntry`. Eviction removes the least recently used traces, then the blobs that no trace names any
more, inside the cache folder only. Every other failure is `cache_unavailable`.

**The index is written at the ends of a run, never across a computation** (#424 PRICE-2f). SQLite has one writer, and
Python's sqlite3 opens a transaction at the first INSERT, UPDATE or DELETE that lasts until the commit. So what a run
keeps as it goes (`put`, `touch`, `drop`) is queued here, and `commit` (or `evict`) writes it in one short transaction:
another process's run waits for that write at most, not for the whole of this run. What is never committed is lost, as
a rolled-back transaction was: a run that fails leaves no trace of itself in the index. A connection that finds the
index locked waits `cache.wait_ms`; past it the statement fails, and that is `cache_unavailable` with `wait_ms`.
"""

from __future__ import annotations

import json
import os
import re
import sqlite3
import tempfile
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

from acmodel.cache.keys import digest
from acmodel.codes import CACHE_UNAVAILABLE
from acmodel.envelope import Failure, error

SCHEMA = "acmodel.cache.v1"
CACHE = "cache"
INDEX = "index.sqlite"
BLOBS = "blobs"
RUNS = "runs"
OUTPUTS = "outputs.json"
HASH = re.compile(r"[0-9a-f]{64}")
SHARD = re.compile(r"[0-9a-f]{2}")
# What `tempfile.mkstemp` names when a blob is written: `<value hash>.<random>.tmp`.
TEMP = re.compile(r"[0-9a-f]{64}\.[a-z0-9_]+\.tmp")
DEFAULT_WAIT_MS = 60_000  # `cache.wait_ms`: how long a connection waits for a lock on the index

_TABLES = (
    "CREATE TABLE IF NOT EXISTS meta (name TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS nodes ("
    "key TEXT PRIMARY KEY, spec TEXT NOT NULL, kind TEXT NOT NULL, node TEXT NOT NULL, value TEXT NOT NULL, "
    "deps TEXT NOT NULL, size INTEGER NOT NULL, used INTEGER NOT NULL, compute_us INTEGER NOT NULL)",
    "CREATE INDEX IF NOT EXISTS nodes_spec ON nodes (spec)",
    "CREATE INDEX IF NOT EXISTS nodes_used ON nodes (used)",
)


def unavailable(stage: str, path: Path, exc: BaseException | None = None, **values: Any) -> Failure:
    detail: dict[str, Any] = {"stage": stage, "path": str(path), **values}
    if exc is not None:
        detail.update(exception_type=type(exc).__name__, raw_text=str(exc))
    return Failure(error(CACHE_UNAVAILABLE, detail))


class CorruptEntry(Exception):
    """A cache entry that does not hold what the index names (`cache_entry_corrupt`)."""

    def __init__(self, reason: str, path: Path, **values: Any) -> None:
        super().__init__(reason)
        self.reason = reason
        self.path = path
        self.values = values

    def detail(self, **more: Any) -> dict[str, Any]:
        return {"reason": self.reason, "path": str(self.path), **more, **self.values}


@dataclass(frozen=True)
class Trace:
    key: str
    value: str
    reads: tuple[tuple[tuple[str, ...], str], ...] | None  # (input, value hash) as read; None: the row does not read


@dataclass(frozen=True)
class Plan:
    """What an eviction removes: traces, least recently used first, until the values that the remaining
    traces name fit in the size cap; then the blobs no remaining trace names."""

    keys: tuple[str, ...]
    blobs: tuple[str, ...]
    values: frozenset[str]  # every value hash the index names before the eviction
    entries: int
    bytes: int  # the size of the distinct values the index names
    freed: int  # the size of `blobs`


def reads(text: Any) -> tuple[tuple[tuple[str, ...], str], ...] | None:
    """The reads of an index row, or None when they are not a list of [input, value hash] pairs, each input a
    non-empty list of strings."""
    try:
        doc = json.loads(text)
    except (TypeError, ValueError):
        return None
    if not isinstance(doc, list):
        return None
    found = []
    for pair in doc:
        if not (
            isinstance(pair, list) and len(pair) == 2 and isinstance(pair[0], list) and pair[0]
            and all(isinstance(part, str) for part in pair[0])
            and isinstance(pair[1], str) and HASH.fullmatch(pair[1])
        ):
            return None
        found.append((tuple(pair[0]), pair[1]))
    return tuple(found)


def _write_file(path: Path, data: bytes) -> None:
    """Write `data` at `path` atomically: a temporary file beside it, then a rename. A temporary blob file
    left by a failed write is removed by `cache gc`."""
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        handle, temporary = tempfile.mkstemp(dir=path.parent, prefix=path.name + ".", suffix=".tmp")
        with os.fdopen(handle, "wb") as file:
            file.write(data)
        os.replace(temporary, path)
    except OSError as exc:
        raise unavailable("write", path, exc) from exc


def _connect(index: Path, wait_ms: int) -> sqlite3.Connection:
    try:
        return sqlite3.connect(index, timeout=wait_ms / 1000)
    except sqlite3.Error as exc:
        raise unavailable("open", index, exc) from exc


class Store:
    def __init__(self, state_root: Path, connection: sqlite3.Connection, max_bytes: int, wait_ms: int) -> None:
        self.root = state_root / CACHE
        self.index = self.root / INDEX
        self.blobs = self.root / BLOBS
        self.runs = state_root / RUNS
        self.max_bytes = max_bytes
        self.wait_ms = wait_ms
        self._connection = connection
        self._valid: set[str] = set()  # blobs this process read or wrote whose bytes hash to their name
        self._queued: list[tuple[str, tuple[Any, ...]]] = []  # what the next `commit` writes, in order

    def __enter__(self) -> Store:
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def close(self) -> None:
        """Close the index; what was not committed is lost: the queue, and a transaction still open is rolled back."""
        try:
            self._connection.close()
        except sqlite3.Error as exc:
            raise self._failed("write", exc) from exc

    def _failed(self, stage: str, exc: sqlite3.Error) -> Failure:
        """`cache_unavailable` of the index; for a lock that outlasted the wait, with the wait it used."""
        busy = getattr(exc, "sqlite_errorcode", 0) & 0xFF == sqlite3.SQLITE_BUSY
        return unavailable(stage, self.index, exc, **({"wait_ms": self.wait_ms} if busy else {}))

    def sql(self, stage: str, statement: str, parameters: Iterable[Any] = ()) -> list[tuple[Any, ...]]:
        try:
            return self._connection.execute(statement, tuple(parameters)).fetchall()
        except sqlite3.Error as exc:
            raise self._failed(stage, exc) from exc

    def flush(self) -> None:
        """Write what is queued in this connection's transaction, which takes the index's write lock until the commit
        (module docstring): call it only at the end of a run's work, right before a commit."""
        queued, self._queued = self._queued, []
        for statement, parameters in queued:
            self.sql("write", statement, parameters)

    def commit(self, stage: str = "write") -> None:
        self.flush()
        try:
            self._connection.commit()
        except sqlite3.Error as exc:
            raise self._failed(stage, exc) from exc

    def check_schema(self) -> None:
        rows = self.sql("open", "SELECT value FROM meta WHERE name = 'schema'")
        found = rows[0][0] if rows else None
        if found != SCHEMA:
            raise unavailable("open", self.index, found=str(found))

    # A run ------------------------------------------------------------------------------------------

    def begin(self) -> int:
        """Start a run: advance the use counter, whose new value the run's traces carry."""
        rows = self.sql("read", "SELECT value FROM meta WHERE name = 'used'")
        if len(rows) != 1 or not str(rows[0][0]).isdigit():
            raise unavailable("read", self.index, found="used")
        used = int(rows[0][0]) + 1
        self.sql("write", "UPDATE meta SET value = ? WHERE name = 'used'", (str(used),))
        self.commit()
        return used

    def traces(self, spec: str) -> list[Trace]:
        """The traces of one node spec, the most recently used first."""
        rows = self.sql("read", "SELECT key, value, deps FROM nodes WHERE spec = ? ORDER BY used DESC, key", (spec,))
        return [
            Trace(key, value, reads(deps) if isinstance(value, str) and HASH.fullmatch(value) else None)
            for key, value, deps in rows
        ]

    def blob_path(self, value: str) -> Path:
        return self.blobs / value[:2] / value

    def read_blob(self, value: str) -> bytes:
        path = self.blob_path(value)
        try:
            data = path.read_bytes()
        except FileNotFoundError:
            raise CorruptEntry("blob_missing", path) from None
        except OSError as exc:
            raise CorruptEntry("blob_unreadable", path, exception_type=type(exc).__name__, raw_text=str(exc)) from exc
        found = digest(data)
        if found != value:
            raise CorruptEntry("hash_mismatch", path, found=found)
        self._valid.add(value)
        return data

    def put(
        self, *, key: str, spec: str, kind: str, node: str, value: str, deps: str, blob: bytes, used: int,
        compute_us: int,
    ) -> None:
        """Keep a trace and its value; a blob already there with the right bytes is kept as it is. The blob is written
        now, its trace at the next `commit`."""
        if value not in self._valid:
            path = self.blob_path(value)
            try:
                current = path.read_bytes()
            except FileNotFoundError:
                current = None
            except OSError as exc:
                raise unavailable("read", path, exc) from exc
            if current is None or digest(current) != value:
                _write_file(path, blob)
            self._valid.add(value)
        self._queued.append((
            "INSERT OR REPLACE INTO nodes (key, spec, kind, node, value, deps, size, used, compute_us) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (key, spec, kind, node, value, deps, len(blob), used, compute_us),
        ))

    def touch(self, key: str, used: int) -> None:
        self._queued.append(("UPDATE nodes SET used = ? WHERE key = ?", (used, key)))

    def drop(self, key: str) -> None:
        """Remove one trace whose row does not read; the node is recomputed."""
        self._queued.append(("DELETE FROM nodes WHERE key = ?", (key,)))

    def run_path(self, run_id: str) -> Path:
        return self.runs / run_id / OUTPUTS

    def write_run(self, run_id: str, outputs: bytes) -> bool:
        """Keep a run's outputs. True when the file held other bytes for the same run id; it is replaced."""
        path = self.run_path(run_id)
        try:
            current = path.read_bytes()
        except FileNotFoundError:
            current = None
        except OSError as exc:
            raise unavailable("read", path, exc) from exc
        if current == outputs:
            return False
        _write_file(path, outputs)
        return current is not None

    # Size --------------------------------------------------------------------------------------------

    def rows(self) -> list[tuple[Any, ...]]:
        """(key, kind, value, deps, size) of every trace, by key."""
        return self.sql("read", "SELECT key, kind, value, deps, size FROM nodes ORDER BY key")

    def plan(self, keep: int | None) -> Plan:
        """The eviction down to `max_bytes`; traces used at or after `keep` (the running run's) stay."""
        rows = self.sql("read", "SELECT key, value, size, used FROM nodes ORDER BY used, key")
        names = Counter(value for _, value, _, _ in rows)
        sizes = {value: size for _, value, size, _ in rows}
        total = sum(sizes.values())
        left, keys, blobs = total, [], []
        for key, value, _, used in rows:
            if left <= self.max_bytes or (keep is not None and used >= keep):
                break
            keys.append(key)
            names[value] -= 1
            if names[value] == 0:
                blobs.append(value)
                left -= sizes[value]
        return Plan(tuple(keys), tuple(blobs), frozenset(sizes), len(rows), total, total - left)

    def remove(self, keys: Iterable[str], files: Iterable[Path]) -> None:
        """Remove traces from the index, then files of the cache folder."""
        try:
            self._connection.executemany("DELETE FROM nodes WHERE key = ?", [(key,) for key in keys])
        except sqlite3.Error as exc:
            raise self._failed("evict", exc) from exc
        self.commit("evict")
        for path in files:
            self._valid.discard(path.name)
            try:
                os.remove(path)
            except FileNotFoundError:
                continue  # already absent: nothing left to remove
            except OSError as exc:
                raise unavailable("evict", path, exc) from exc

    def evict(self, keep: int) -> dict[str, int]:
        self.flush()  # the run's own traces count, and the run's commit follows
        plan = self.plan(keep)
        if plan.keys:
            self.remove(plan.keys, [self.blob_path(value) for value in plan.blobs])
        return {"entries": len(plan.keys), "bytes": plan.freed}

    def files(self) -> tuple[list[tuple[str, Path]], int]:
        """(name, path) of every blob and temporary blob file in the blob folders, by path, and the number of
        other entries there, which the cache did not write and leaves alone."""
        found: list[tuple[str, Path]] = []
        other = 0
        if not self.blobs.is_dir():
            return found, other
        try:
            with os.scandir(self.blobs) as shards:
                listed = sorted(shards, key=lambda entry: entry.name)
            for shard in listed:
                if not (shard.is_dir() and SHARD.fullmatch(shard.name)):
                    other += 1
                    continue
                with os.scandir(shard.path) as entries:
                    for entry in sorted(entries, key=lambda item: item.name):
                        if entry.is_file() and (HASH.fullmatch(entry.name) or TEMP.fullmatch(entry.name)):
                            found.append((entry.name, Path(entry.path)))
                        else:
                            other += 1
        except OSError as exc:
            raise unavailable("read", self.blobs, exc) from exc
        return found, other


def open_store(state_root: Path, *, max_bytes: int, wait_ms: int = DEFAULT_WAIT_MS) -> Store:
    """The cache under `state_root` for a run; it is created when absent."""
    index = state_root / CACHE / INDEX
    try:
        (state_root / CACHE / BLOBS).mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise unavailable("open", index, exc) from exc
    store = Store(state_root, _connect(index, wait_ms), max_bytes, wait_ms)
    try:
        [(mode,)] = store.sql("open", "PRAGMA journal_mode=WAL")
        if mode != "wal":
            raise unavailable("open", index, found=str(mode))
        for statement in _TABLES:
            store.sql("open", statement)
        store.sql("open", "INSERT OR IGNORE INTO meta (name, value) VALUES ('schema', ?)", (SCHEMA,))
        store.sql("open", "INSERT OR IGNORE INTO meta (name, value) VALUES ('used', '0')")
        store.commit("open")
        store.check_schema()
    except BaseException:
        store.close()
        raise
    return store


def existing_store(state_root: Path, *, max_bytes: int, wait_ms: int = DEFAULT_WAIT_MS) -> Store | None:
    """The cache under `state_root` for inspection, or None when there is none. Nothing is created."""
    index = state_root / CACHE / INDEX
    if not index.is_file():
        return None
    store = Store(state_root, _connect(index, wait_ms), max_bytes, wait_ms)
    try:
        store.check_schema()
    except BaseException:
        store.close()
        raise
    return store
