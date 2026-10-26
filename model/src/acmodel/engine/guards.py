"""The worker's guards (#424 design-py section 7.3). Standard library only, because the worker imports it.

Each violation is reported as `engine_guard_violation` with its `reason`:
- `write_outside_scratch`: an `open` for writing, or a change to the file system (remove, rename, mkdir,
  rmdir, truncate, link, symlink, rmtree), outside the worker's scratch directory;
- `unlisted_file`: a read of a file under the engine root that neither `code` nor `data` lists, nor
  `engine.json` itself; the fingerprint would miss it;
- `unlisted_module`: a module under the engine root that `code` does not list. The audit hook sees the read
  of its file (a file with a module suffix, `.py` among them), and the module closure check finds it in
  `sys.modules` after the import and after every batch;
- `global_state_changed`: `sys.argv`, the working directory, `sys.path` or `os.environ` differ after a
  stage from what they were before it (`state` names which).

The audit hook (`sys.addaudithook`) records a violation and then raises `GuardViolation`, so the operation
does not happen. An engine that catches the exception is reported all the same: the worker reports every
recorded violation before anything else. The hook checks only while `active` is set, that is while engine
code runs; the worker's own work (frames, tracebacks) is not checked.

Paths are compared as `os.path.normcase(os.path.abspath(path))`, with no file-system call inside the hook.
The host hands over the engine root, the scratch directory and the listed files as real paths.
"""

from __future__ import annotations

import importlib.machinery
import os
import sys
from typing import Any

WRITE_FLAGS = os.O_WRONLY | os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_TRUNC
WRITE_MODES = "wax+"
MODULE_SUFFIXES = tuple(suffix.lower() for suffix in importlib.machinery.all_suffixes())
# The audit events that change the file system, and the positions of the paths they change.
CHANGES = {
    "os.remove": (0,),
    "os.rename": (0, 1),
    "os.mkdir": (0,),
    "os.rmdir": (0,),
    "os.truncate": (0,),
    "os.link": (1,),
    "os.symlink": (1,),
    "shutil.rmtree": (0,),
}
STATES = ("argv", "cwd", "sys_path", "environ")


class GuardViolation(Exception):
    """Raised by the audit hook into the engine code that tried the operation."""


def _key(path: str) -> str:
    return os.path.normcase(path)


def _inside(key: str, folder: str) -> bool:
    prefix = folder if folder.endswith(os.sep) else folder + os.sep
    return key == folder or key.startswith(prefix)


def _path(value: Any) -> str | None:
    """An absolute path from an audit argument; None for a file descriptor or anything else."""
    if isinstance(value, bytes):
        value = os.fsdecode(value)
    elif isinstance(value, os.PathLike):
        value = os.fspath(value)
        if isinstance(value, bytes):
            value = os.fsdecode(value)
    if not isinstance(value, str):
        return None
    return os.path.abspath(value)


def snapshot() -> dict[str, Any]:
    """The global state an engine must leave as it found it."""
    try:
        cwd: str | None = os.getcwd()
    except OSError:  # the working directory is gone: that is a change too
        cwd = None
    return {"argv": list(sys.argv), "cwd": cwd, "sys_path": list(sys.path), "environ": dict(os.environ)}


class Guards:
    def __init__(self, *, root: str, scratch: str, code: list[str], data: list[str], manifest: str) -> None:
        self.root = _key(os.path.abspath(root))
        self.scratch = _key(os.path.abspath(scratch))
        self.code = frozenset(_key(os.path.abspath(path)) for path in code)
        self.readable = self.code | {_key(os.path.abspath(path)) for path in [*data, manifest]}
        self.active = False
        self.stage = "import"
        self.violations: list[dict[str, Any]] = []
        self._seen: set[tuple[tuple[str, Any], ...]] = set()
        self._busy = False

    def install(self) -> None:
        sys.addaudithook(self._hook)

    def take(self) -> list[dict[str, Any]]:
        """The violations recorded since the last call."""
        found, self.violations = self.violations, []
        return found

    def _record(self, reason: str, **values: Any) -> dict[str, Any]:
        entry = {"reason": reason, "stage": self.stage, **values}
        # One violation is recorded once, under the first stage that found it (a module loaded by the op
        # is still loaded after the reset that follows).
        mark = tuple(sorted((key, value) for key, value in entry.items() if key != "stage"))
        if mark not in self._seen:
            self._seen.add(mark)
            self.violations.append(entry)
        return entry

    # The audit hook ----------------------------------------------------------------------------------

    def _hook(self, event: str, args: tuple[Any, ...]) -> None:
        if not self.active or self._busy or (event != "open" and event not in CHANGES):
            return
        self._busy = True  # nothing the check does may check itself
        try:
            found = self._check(event, args)
        finally:
            self._busy = False
        if found is not None:
            raise GuardViolation(found["reason"], found.get("path"))

    def _check(self, event: str, args: tuple[Any, ...]) -> dict[str, Any] | None:
        if event == "open":
            path = _path(args[0]) if args else None
            if path is None:
                return None
            mode = args[1] if len(args) > 1 else None
            flags = args[2] if len(args) > 2 else 0
            writing = (isinstance(mode, str) and any(letter in mode for letter in WRITE_MODES)) or (
                isinstance(flags, int) and bool(flags & WRITE_FLAGS)
            )
            return self._access(path, writing)
        for position in CHANGES[event]:
            path = _path(args[position]) if position < len(args) else None
            if path is not None and not _inside(_key(path), self.scratch):
                return self._record("write_outside_scratch", path=path)
        return None

    def _access(self, path: str, writing: bool) -> dict[str, Any] | None:
        key = _key(path)
        if _inside(key, self.scratch):
            return None
        if writing:
            return self._record("write_outside_scratch", path=path)
        if _inside(key, self.root) and key not in self.readable:
            reason = "unlisted_module" if key.endswith(MODULE_SUFFIXES) else "unlisted_file"
            return self._record(reason, path=path)
        return None

    # The checks after a stage ----------------------------------------------------------------------

    def check_modules(self) -> None:
        """Module closure: every loaded module whose file lies under the engine root is listed in `code`."""
        for name, module in sorted(list(sys.modules.items()), key=lambda item: item[0]):
            file = getattr(module, "__file__", None)
            if not isinstance(file, str):
                continue
            path = os.path.abspath(file)
            key = _key(path)
            if _inside(key, self.root) and key not in self.code:
                self._record("unlisted_module", module=name, path=path)

    def check_state(self, before: dict[str, Any]) -> None:
        after = snapshot()
        for state in STATES:
            if before[state] != after[state]:
                self._record("global_state_changed", state=state)
