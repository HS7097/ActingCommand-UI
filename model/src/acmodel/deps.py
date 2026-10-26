"""The dependency check (`deps_invalid`): Python 3.13 and the pinned packages."""

from __future__ import annotations

from pathlib import Path
from typing import Callable, Sequence

from acmodel.codes import DEPS_INVALID
from acmodel.envelope import Failure, error

REQUIRED_PYTHON = (3, 13)
# The pins of model/requirements.txt; change both together. The launcher test in
# CI reads the versions installed from that file against these.
PINS = (("numpy", "2.5.4"), ("scipy", "1.18.1"))


def label(version: Sequence[int]) -> str:
    return ".".join(str(part) for part in version)


def setup_command(launcher: Path, platform: str) -> list[str]:
    """The command that creates the venv, named by every `deps_invalid`."""
    python = ["py", "-" + label(REQUIRED_PYTHON)] if platform == "win32" else ["python" + label(REQUIRED_PYTHON)]
    return [*python, "-I", "-B", str(launcher), "setup"]


def check(
    python_version: Sequence[int],
    installed: Callable[[str], str | None],
    setup: list[str],
) -> dict[str, str]:
    """Return the pinned packages' versions, or raise every problem at once."""
    problems = []
    if tuple(python_version[:2]) != REQUIRED_PYTHON:
        problems.append({"reason": "python_version", "expected": label(REQUIRED_PYTHON), "found": label(python_version)})
    found = {}
    for package, pin in PINS:
        version = installed(package)
        if version is None:
            problems.append({"reason": "missing", "package": package, "expected": pin})
        elif version != pin:
            problems.append({"reason": "version_mismatch", "package": package, "expected": pin, "found": version})
        else:
            found[package] = version
    if problems:
        raise Failure(*(error(DEPS_INVALID, {**problem, "setup_command": setup}) for problem in problems))
    return found
