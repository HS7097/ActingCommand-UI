"""Delivery candidates (#424 design 5.6; contracts/cli.md, "Automatic refresh").

A candidate is what a delivery would write for one refresh: the files of `export thresholds` (`thresholds.json` and
the provenance files) and of `export tables` (each table and the provenance files its cells need), built by the
exports themselves from the refresh's run, with the same refusals. Their names start with `model/` (relative to
the model folder) or `repo/` (relative to the resource repository: `repo/tables/<table_id>.json`). The scoring
documents are not part of it while `export scoring` writes nothing (`validator_path_unknown`).

A candidate is staged only when its files differ from the last candidate's: under
`candidates/<number>/` of the watch folder, its files under `files/`, `diff.json` (`acmodel.watch.diff.v1`: each
file added, removed or changed since the last candidate, with the sha256 before and after and, for a changed JSON
file, the pointer of its first difference) and `summary.json` (`acmodel.watch.candidate.v1`). It is written in a
temporary folder beside it and then renamed, so a candidate folder is always whole.

**Nothing is committed, pushed or written into a repository:** a person or an agent delivers a candidate by running
the exports (which validate the tables with the Runtime's validator) and opening the pull request.
"""

from __future__ import annotations

import json
import os
import re
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any, Mapping

from acmodel.cache.keys import canonical, canonical_text, digest
from acmodel.core.run import Run
from acmodel.engine.pool import difference
from acmodel.envelope import Failure
from acmodel.export import tables, thresholds
from acmodel.watch.files import failed

CANDIDATES = "candidates"
FILES = "files"
SUMMARY = "summary.json"
DIFF = "diff.json"
SUMMARY_SCHEMA = "acmodel.watch.candidate.v1"
DIFF_SCHEMA = "acmodel.watch.diff.v1"
MODEL = "model"  # names of the files of the model folder start with this
REPO = "repo"  # names of the files of the resource repository start with this
_NUMBER = re.compile(r"[0-9]{6}")


def _dump(doc: Any) -> bytes:
    """summary.json and diff.json: sorted keys, two-space indent, UTF-8, LF line ends, one final newline."""
    return (json.dumps(doc, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def build(run: Run, base: Path) -> dict[str, bytes]:
    """The candidate's files by name, sorted. Every refusal of either export raises `Failure` together. `base` only
    names the repository's files, and is never written."""
    files: dict[str, bytes] = {}
    errors: dict[str, dict[str, Any]] = {}
    try:
        files.update((f"{MODEL}/{name}", data) for name, data in thresholds.build(run).items())
    except Failure as failure:
        errors.update((canonical_text(entry), entry) for entry in failure.errors)
    repo = base / REPO
    try:
        built = tables.build(run, repo)
    except Failure as failure:
        errors.update((canonical_text(entry), entry) for entry in failure.errors)
    else:
        for target, data in built.files.items():
            if target.is_relative_to(repo):
                files[f"{REPO}/{target.relative_to(repo).as_posix()}"] = data
            else:
                files[f"{MODEL}/{target.relative_to(run.folder).as_posix()}"] = data
    if errors:
        raise Failure(*errors.values())
    return dict(sorted(files.items()))


def fingerprint(files: Mapping[str, bytes]) -> str:
    """The sha256 of a candidate: over its files' names and sha256s."""
    return digest(canonical([[name, digest(data)] for name, data in sorted(files.items())]))


def folder_name(number: int) -> str:
    return f"{number:06d}"


def _read(folder: Path) -> dict[str, bytes]:
    """The files of a staged candidate, by the names its summary lists."""
    path = folder / SUMMARY
    try:
        summary = json.loads(path.read_bytes().decode("utf-8"))
        names = [entry["name"] for entry in summary["files"]]
        found = {}
        for name in names:
            path = folder / FILES / PurePosixPath(name)
            found[name] = path.read_bytes()
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise failed("stage", path, exc) from exc
    return found


def _pointer(before: bytes, after: bytes) -> str | None:
    """The JSON pointer of the first difference of two JSON files, or None when either is not JSON."""
    try:
        return difference(json.loads(before), json.loads(after))
    except ValueError:
        return None


def diff(before: Mapping[str, bytes], after: Mapping[str, bytes]) -> list[dict[str, Any]]:
    """Each file added, removed or changed, by name."""
    entries = []
    for name in sorted(set(before) | set(after)):
        old, new = before.get(name), after.get(name)
        if old == new:
            continue
        entry: dict[str, Any] = {
            "name": name,
            "change": "added" if old is None else "removed" if new is None else "changed",
            "before": None if old is None else digest(old),
            "after": None if new is None else digest(new),
        }
        if old is not None and new is not None and (pointer := _pointer(old, new)) is not None:
            entry["pointer"] = pointer
        entries.append(entry)
    return entries


def _next(base: Path) -> int:
    """One above the highest candidate number under `base`."""
    try:
        numbers = [int(entry.name) for entry in base.iterdir() if _NUMBER.fullmatch(entry.name)] if base.is_dir() else []
    except OSError as exc:
        raise failed("stage", base, exc) from exc
    return max(numbers, default=0) + 1


def stage(
    folder: Path, files: Mapping[str, bytes], *, previous: int | None, head: Mapping[str, Any]
) -> tuple[int, Path, dict[str, Any]]:
    """Stage a candidate under `folder/candidates/`, diffed against candidate `previous` (none when it is None or
    its folder is gone). Returns its number, its folder and its diff document."""
    base = folder / CANDIDATES
    before_folder = None if previous is None else base / folder_name(previous)
    compared = before_folder is not None and before_folder.exists()
    before = _read(before_folder) if compared else {}
    number = _next(base)
    target = base / folder_name(number)
    changes = {
        "schema_version": DIFF_SCHEMA, "from": previous if compared else None, "to": number,
        "files": diff(before, files),
    }
    summary = {
        "schema_version": SUMMARY_SCHEMA, **head, "number": number, "previous": previous, "sha256": fingerprint(files),
        "files": [{"name": name, "sha256": digest(data), "bytes": len(data)} for name, data in sorted(files.items())],
    }
    try:
        base.mkdir(parents=True, exist_ok=True)
        staging = Path(tempfile.mkdtemp(prefix=".staging-", dir=base))
        for name, data in files.items():
            path = staging / FILES / PurePosixPath(name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (staging / DIFF).write_bytes(_dump(changes))
        (staging / SUMMARY).write_bytes(_dump(summary))
        os.rename(staging, target)
    except OSError as exc:
        raise failed("stage", exc.filename or target, exc) from exc
    return number, target, changes
