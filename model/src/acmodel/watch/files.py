"""What `watch` looks at, with the standard library only (#424 design-py 5.6).

- **The watched files:** the JSON files of the model folder (its `out/` aside, which the exports write), those of
  the preference folder beside it (`<model folder>/../prefs`) and every file of `paths.datasets`. A folder that is
  absent holds nothing; one that cannot be listed is `watch_failed` (`scan`).
- **The scan** reads each file's size and modification time (its `Stamp`); only a file whose stamp changed is read
  and hashed (sha256). A file is changed when it is new, gone, or its bytes hash otherwise, so a rewrite of the same
  bytes changes nothing. A file that cannot be read has no hash and counts as changed: the refresh that reads it
  reports why.
- **The state snapshot** comes through an import hook, a `SnapshotSource`. Today the source is a file a person or
  an agent writes (`FileSnapshot`); later a read-only export of the Runtime's observer interface can be one. It is
  scanned like the files, and each refresh imports it: its bytes are copied once, named by their sha256, under the
  watch folder, and the refresh reads that copy, so a source rewritten while the refresh runs cannot reach it half
  written.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Protocol

from acmodel.cache.keys import digest
from acmodel.codes import WATCH_FAILED
from acmodel.envelope import Failure, error

OUT = "out"  # the folder the exports write into the model folder: never watched
PREFS = "prefs"  # the preference folder, beside the model folder


def failed(stage: str, path: Path | str, exc: BaseException | None = None, **values: Any) -> Failure:
    detail: dict[str, Any] = {"stage": stage, "path": str(path), **values}
    if exc is not None:
        detail.update(exception_type=type(exc).__name__, raw_text=str(exc))
    return Failure(error(WATCH_FAILED, detail))


@dataclass(frozen=True)
class Stamp:
    size: int
    mtime_ns: int


@dataclass(frozen=True)
class Seen:
    """A watched file as the last scan saw it: its stamp, and the sha256 of its bytes (None: they could not be
    read)."""

    stamp: Stamp
    sha256: str | None


# A file's stamp, or None when it is absent.
Stamper = Callable[[Path], Stamp | None]


def disk_stamp(path: Path) -> Stamp | None:
    try:
        status = os.stat(path)
    except FileNotFoundError:
        return None
    except OSError as exc:  # not "absent": the scan cannot tell, so it does not guess
        raise failed("scan", path, exc) from exc
    return Stamp(status.st_size, status.st_mtime_ns)


class SnapshotSource(Protocol):
    """The import hook of the account state snapshots."""

    path: Path  # what the event log and watch.json name

    def stamp(self) -> Stamp | None:
        """The snapshot's stamp, or None when there is none."""

    def read(self) -> bytes:
        """The snapshot's bytes; raises OSError when it cannot."""


class FileSnapshot:
    """A state snapshot that a person or an agent writes to a file."""

    def __init__(self, path: Path, stamp: Stamper = disk_stamp) -> None:
        self.path = Path(path)
        self._stamp = stamp

    def stamp(self) -> Stamp | None:
        return self._stamp(self.path)

    def read(self) -> bytes:
        return self.path.read_bytes()


@dataclass(frozen=True)
class Root:
    folder: Path
    json_only: bool
    skip: tuple[str, ...] = ()  # names of folders directly under `folder` that are not watched


def roots(model_folder: Path, datasets: Path) -> list[Root]:
    return [
        Root(model_folder, True, (OUT,)),
        Root(model_folder.parent / PREFS, True),
        Root(datasets, False),
    ]


def listing(watched: Iterable[Root]) -> list[Path]:
    """Every watched file, sorted."""
    found: set[Path] = set()
    for root in watched:
        if not root.folder.is_dir():
            continue

        def refuse(exc: OSError) -> None:
            raise failed("scan", exc.filename or root.folder, exc) from exc

        for folder, folders, names in os.walk(root.folder, onerror=refuse):
            if Path(folder) == root.folder:
                folders[:] = [name for name in folders if name not in root.skip]
            found.update(
                Path(folder, name) for name in names if not root.json_only or name.endswith(".json")
            )
    return sorted(found)


@dataclass(frozen=True)
class Item:
    """One watched thing for the scan: its key in watch.json, its stamp and its bytes."""

    key: str
    stamp: Callable[[], Stamp | None]
    read: Callable[[], bytes]


def file_item(path: Path, stamp: Stamper) -> Item:
    return Item(str(path), lambda: stamp(path), path.read_bytes)


def scan(items: Iterable[Item], previous: Mapping[str, Seen]) -> tuple[dict[str, Seen], list[str]]:
    """The watched things as they are now, and the keys of those whose content changed (new, gone, other bytes,
    or bytes that could not be read), sorted."""
    now: dict[str, Seen] = {}
    changed: list[str] = []
    for item in items:
        stamp = item.stamp()
        if stamp is None:
            continue
        before = previous.get(item.key)
        if before is not None and before.stamp == stamp:
            now[item.key] = before
            continue
        try:
            sha256: str | None = digest(item.read())
        except FileNotFoundError:  # gone since its stamp was read: absent
            continue
        except OSError:  # it counts as changed; the refresh that reads it reports why
            sha256 = None
        now[item.key] = Seen(stamp, sha256)
        if before is None or sha256 is None or before.sha256 != sha256:
            changed.append(item.key)
    changed.extend(key for key in previous if key not in now)
    return now, sorted(set(changed))
