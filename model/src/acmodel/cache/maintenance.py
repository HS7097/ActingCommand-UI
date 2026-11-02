"""`acmodel cache stats | verify | gc [--apply]`: the cache under `paths.state_root`, outside a run.

- `stats` counts what the index holds. It writes nothing.
- `verify` reads every blob the index names and checks that its bytes hash to its name: each entry that
  does not is one `cache_entry_corrupt` error. It writes nothing and repairs nothing (a run recomputes).
- `gc` plans the eviction down to `cache.max_bytes` (least recently used first) and the removal of the
  blob files no trace names (orphans, and temporary files of interrupted writes). Without `--apply` it
  only reports the plan; with it, it removes exactly that, inside the cache folder, and nothing else.
  Run it when no run is in progress.
A state root without a cache is reported as `exists: false`; nothing is created.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from acmodel.cache.store import (
    CACHE,
    DEFAULT_WAIT_MS,
    HASH,
    RUNS,
    TEMP,
    CorruptEntry,
    existing_store,
    reads,
    unavailable,
)
from acmodel.codes import CACHE_ENTRY_CORRUPT
from acmodel.envelope import error


def _runs(state_root: Path) -> int:
    folder = state_root / RUNS
    if not folder.is_dir():
        return 0
    try:
        with os.scandir(folder) as entries:
            return sum(1 for entry in entries if entry.is_dir() and HASH.fullmatch(entry.name))
    except OSError as exc:
        raise unavailable("read", folder, exc) from exc


def stats(state_root: Path, *, max_bytes: int, wait_ms: int = DEFAULT_WAIT_MS) -> dict[str, Any]:
    store = existing_store(state_root, max_bytes=max_bytes, wait_ms=wait_ms)
    head = {"path": str(state_root / CACHE), "exists": store is not None, "max_bytes": max_bytes}
    if store is None:
        return {**head, "entries": 0, "blobs": 0, "bytes": 0, "by_kind": {}, "runs": _runs(state_root)}
    with store:
        rows = store.rows()
    sizes = {value: size for _, _, value, _, size in rows}
    kinds: dict[str, int] = {}
    for _, kind, _, _, _ in rows:
        kinds[kind] = kinds.get(kind, 0) + 1
    return {
        **head,
        "entries": len(rows),
        "blobs": len(sizes),
        "bytes": sum(sizes.values()),
        "by_kind": {kind: kinds[kind] for kind in sorted(kinds)},
        "runs": _runs(state_root),
    }


def verify(state_root: Path, *, wait_ms: int = DEFAULT_WAIT_MS) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    store = existing_store(state_root, max_bytes=0, wait_ms=wait_ms)  # verify never evicts
    head = {"path": str(state_root / CACHE), "exists": store is not None}
    if store is None:
        return {**head, "entries": 0, "blobs": 0, "problems": 0}, []
    errors: list[dict[str, Any]] = []
    checked: dict[str, CorruptEntry | None] = {}
    with store:
        rows = store.rows()
        for key, kind, value, deps, _ in rows:
            if not (isinstance(value, str) and HASH.fullmatch(value)) or reads(deps) is None:
                detail = {"reason": "index_invalid", "path": str(store.index), "key": str(key), "kind": str(kind)}
                errors.append(error(CACHE_ENTRY_CORRUPT, detail))
                continue
            if value not in checked:
                try:
                    store.read_blob(value)
                    checked[value] = None
                except CorruptEntry as corrupt:
                    checked[value] = corrupt
            problem = checked[value]
            if problem is not None:
                errors.append(error(CACHE_ENTRY_CORRUPT, problem.detail(key=key, kind=kind)))
    return {**head, "entries": len(rows), "blobs": len(checked), "problems": len(errors)}, errors


def gc(state_root: Path, *, max_bytes: int, apply: bool, wait_ms: int = DEFAULT_WAIT_MS) -> dict[str, Any]:
    store = existing_store(state_root, max_bytes=max_bytes, wait_ms=wait_ms)
    head = {"path": str(state_root / CACHE), "exists": store is not None, "apply": apply, "max_bytes": max_bytes}
    if store is None:
        return {
            **head, "entries": 0, "bytes": 0, "entries_removed": 0, "blobs_removed": 0, "bytes_removed": 0,
            "orphans_removed": 0, "orphan_bytes_removed": 0, "other_files": 0,
        }
    with store:
        plan = store.plan(keep=None)
        files, other = store.files()
        orphans = [path for name, path in files if TEMP.fullmatch(name) or name not in plan.values]
        try:
            orphan_bytes = sum(path.stat().st_size for path in orphans)
        except OSError as exc:
            raise unavailable("read", store.blobs, exc) from exc
        if apply:
            store.remove(plan.keys, [store.blob_path(value) for value in plan.blobs] + orphans)
    return {
        **head,
        "entries": plan.entries,
        "bytes": plan.bytes,
        "entries_removed": len(plan.keys),
        "blobs_removed": len(plan.blobs),
        "bytes_removed": plan.freed,
        "orphans_removed": len(orphans),
        "orphan_bytes_removed": orphan_bytes,
        "other_files": other,
    }
