"""Where the tool never writes: a Runtime install root (#424, 2026-11-09; contracts/cli.md, "Never in a Runtime install
root").

`paths.state_root` (checked when the configuration is read, `config.load`) and every folder a command writes into (the
model folder of the exports and of `bindings apply`, the repository of `export tables`; `cli.WRITES`) must be neither a
Runtime install root nor inside one. A folder is inside one when the folder itself, or any folder above it up to its
anchor, holds one of the marks the Runtime's install layout puts at an install root (the Runtime repository,
contracts/install-cli.md, "Layout" and "The A/B layout"; contracts/runtime-watchdog.md, "Layout"):

- `actingd.config.json`: the root's one configuration file, in either layout;
- `runtime/BUILD-MANIFEST.json`: the mark of a single-location root;
- `install/active.json`: the mark of an A/B root, its install selection.

Only whether something is at a mark's name is looked at, never what it holds; no path of a machine is known here.

**How a folder is walked** (`install_root`):
1. It is made absolute with `os.path.abspath`, which also folds `.` and `..` by their text. That path is walked first:
   the folder, then each folder above it, up to the anchor `pathlib` gives it (a drive's root, the share of a UNC
   path, or `/`).
2. Then its resolved form, `Path.resolve()`: symbolic links and junctions are followed as far as they exist, so a link
   that leads into an install root is refused as well as a path inside one that a link leads out of. A level already
   looked at in step 1 is not looked at again.
3. At each level, each mark is looked for with `os.lstat`: anything at its name counts, a dangling link too. A level
   that does not exist (not found, or not a folder) holds no mark.
4. Names are compared by the file system: on Windows a mark is found whatever its case; elsewhere the name is the one
   the Runtime writes.
5. A level that cannot be looked at (any other error of `os.lstat`, or a path that cannot be resolved) cannot be shown
   not to be an install root, so it refuses too (`runtime_install_unchecked`, with the exception).
"""

from __future__ import annotations

import os
from pathlib import Path

# The marks of a Runtime install root, relative to it, in the order they are looked for at each level.
MARKS = (("actingd.config.json",), ("runtime", "BUILD-MANIFEST.json"), ("install", "active.json"))
INSTALL_ROOT = "runtime_install_root"  # a level holds a mark
UNCHECKED = "runtime_install_unchecked"  # a level could not be looked at


def _present(path: Path) -> bool:
    """Whether anything is at `path`, a dangling link too. Not found, or a level that is no folder, is False; any other
    error is raised."""
    try:
        os.lstat(path)
    except (FileNotFoundError, NotADirectoryError):
        return False
    return True


def _unchecked(level: Path, exc: BaseException, marker: str | None) -> dict[str, str]:
    found = {"reason": UNCHECKED, "root": str(level)}
    if marker is not None:
        found["marker"] = marker
    return {**found, "exception_type": type(exc).__name__, "raw_text": str(exc)}


def _walk(start: Path, seen: set[str]) -> dict[str, str] | None:
    """The first level from `start` up to its anchor that holds a mark, or that cannot be looked at; the levels in
    `seen` are skipped, and each one looked at is added."""
    for level in (start, *start.parents):
        key = os.path.normcase(str(level))
        if key in seen:
            continue
        seen.add(key)
        for parts in MARKS:
            marker = "/".join(parts)
            try:
                if _present(level.joinpath(*parts)):
                    return {"reason": INSTALL_ROOT, "root": str(level), "marker": marker}
            except OSError as exc:
                return _unchecked(level, exc, marker)
    return None


def install_root(folder: Path | str) -> dict[str, str] | None:
    """Why nothing may be written into `folder`, or None when no level of it holds a mark.

    - `{"reason": "runtime_install_root", "root", "marker"}`: the first level (the folder itself, then each folder
      above it) that holds a mark; `root` is that level and `marker` the mark, with `/`;
    - `{"reason": "runtime_install_unchecked", "root", ["marker"], "exception_type", "raw_text"}`: the first level that
      could not be looked at, with the mark looked for (none when the folder could not be resolved).
    """
    lexical = Path(os.path.abspath(folder))
    seen: set[str] = set()
    found = _walk(lexical, seen)
    if found is not None:
        return found
    try:
        resolved = lexical.resolve()
    except (OSError, RuntimeError) as exc:  # RuntimeError: a loop of links, on a Python that raises it
        return _unchecked(lexical, exc, None)
    return _walk(resolved, seen)
