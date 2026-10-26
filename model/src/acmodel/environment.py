"""The venv: its layout, whether this process runs in it, re-dispatch into it, and `setup`.

Nothing here ever deletes a venv or any file in one; the person removes it.
"""

from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any, MutableMapping, Sequence

from acmodel import deps
from acmodel.codes import DEPS_INVALID, VENV_FAILED
from acmodel.config import Config
from acmodel.envelope import Failure, error
from acmodel.system import System

# Set in the child of a re-dispatch: a child that still is not in the venv fails
# instead of dispatching again.
DISPATCH_MARK = "ACMODEL_DISPATCHED"
BLAS_VARIABLES = ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "MKL_NUM_THREADS")
PIP_INSTALL = ("-m", "pip", "install", "--require-hashes", "--only-binary=:all:", "--no-deps", "-r")

# `version = 3.13.15` (venv) or `version_info = 3.13.15` (other venv tools).
_VERSION_LINE = re.compile(r"\s*version(?:_info)?\s*=\s*((\d+)\.(\d+)\S*)\s*")


def venv_python(venv: Path, platform: str) -> Path:
    return venv / "Scripts" / "python.exe" if platform == "win32" else venv / "bin" / "python"


def under_venv(venv: Path, prefix: str) -> bool:
    def norm(path: os.PathLike[str] | str) -> str:
        return os.path.normcase(os.path.realpath(path))

    return norm(venv) == norm(prefix)


def set_blas_threads(environ: MutableMapping[str, str], threads: int) -> None:
    """Before numpy or scipy load anywhere: in this process and in every child."""
    for name in BLAS_VARIABLES:
        environ[name] = str(threads)


def _failed(stage: str, venv: Path, **values: Any) -> Failure:
    return Failure(error(VENV_FAILED, {"stage": stage, "path": str(venv), **values}))


def dispatch(argv: Sequence[str], config: Config, system: System) -> int:
    """Re-run the launcher under the venv's Python; return the child's exit code."""
    if system.environ.get(DISPATCH_MARK):
        raise _failed("dispatch", config.venv, found=system.prefix)
    python = venv_python(config.venv, system.platform)
    if not python.is_file():
        raise Failure(error(DEPS_INVALID, {
            "reason": "missing",
            "path": str(config.venv),
            "setup_command": deps.setup_command(system.launcher, system.platform),
        }))
    env = dict(system.environ)
    env[DISPATCH_MARK] = "1"
    cmd = [str(python), "-I", "-B", str(system.launcher), *argv]
    try:
        return system.run(cmd, env=env, stdout_to_stderr=False)
    except OSError as exc:
        raise _failed("dispatch", config.venv, exception_type=type(exc).__name__, raw_text=str(exc)) from exc


def venv_version(venv: Path) -> tuple[str, tuple[int, int]] | None:
    """The Python version a venv was made from, as its pyvenv.cfg states it, and its minor."""
    try:
        text = (venv / "pyvenv.cfg").read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None
    for line in text.splitlines():
        match = _VERSION_LINE.fullmatch(line)
        if match:
            return match.group(1), (int(match.group(2)), int(match.group(3)))
    return None


def _run(system: System, stage: str, venv: Path, cmd: list[str]) -> None:
    try:
        status = system.run(cmd, env=dict(system.environ), stdout_to_stderr=True)
    except OSError as exc:
        raise _failed(stage, venv, exception_type=type(exc).__name__, raw_text=str(exc)) from exc
    if status != 0:
        raise _failed(stage, venv, exit_code=status)


def setup(config: Config, system: System) -> dict[str, Any]:
    """Create the venv from the running 3.13 if it is absent, then install the hashed wheels."""
    command = deps.setup_command(system.launcher, system.platform)
    if tuple(system.python_version[:2]) != deps.REQUIRED_PYTHON:
        raise Failure(error(DEPS_INVALID, {
            "reason": "python_version",
            "expected": deps.label(deps.REQUIRED_PYTHON),
            "found": deps.label(system.python_version),
            "setup_command": command,
        }))
    venv = config.venv
    created = False
    if os.path.lexists(venv):
        found = venv_version(venv) if venv.is_dir() else None
        if found is None:
            raise _failed("inspect", venv)  # not a venv: refused, never replaced
        if found[1] != deps.REQUIRED_PYTHON:
            raise Failure(error(DEPS_INVALID, {
                "reason": "python_version",
                "expected": deps.label(deps.REQUIRED_PYTHON),
                "found": found[0],
                "path": str(venv),
                "setup_command": command,
            }))
    else:
        _run(system, "create", venv, [system.executable, "-I", "-B", "-m", "venv", str(venv)])
        created = True
    requirements = system.launcher.parent / "requirements.txt"
    python = venv_python(venv, system.platform)
    _run(system, "install", venv, [str(python), "-I", "-B", *PIP_INSTALL, str(requirements)])
    return {"venv": str(venv), "created": created, "requirements": str(requirements)}
