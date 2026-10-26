"""What the run sees of its process: tests pass fakes of the same shape."""

from __future__ import annotations

import importlib.metadata
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, MutableMapping, Sequence, TextIO


@dataclass
class System:
    launcher: Path
    environ: MutableMapping[str, str]
    executable: str
    prefix: str
    platform: str
    python_version: tuple[int, ...]
    installed: Callable[[str], str | None]
    # run(cmd, *, env, stdout_to_stderr) -> exit code; raises OSError if it cannot start.
    run: Callable[..., int]
    stdout: TextIO
    stderr: TextIO


def installed_version(package: str) -> str | None:
    try:
        return importlib.metadata.version(package)
    except importlib.metadata.PackageNotFoundError:
        return None


def run_process(cmd: Sequence[str], *, env: dict[str, str], stdout_to_stderr: bool) -> int:
    """Run a child to completion. With stdout_to_stderr its output joins our stderr,
    so that our stdout keeps only the envelope."""
    sys.stdout.flush()
    sys.stderr.flush()
    stdout = sys.stderr if stdout_to_stderr else None
    return subprocess.run(list(cmd), env=env, stdout=stdout, check=False).returncode


def current(launcher: Path) -> System:
    return System(
        launcher=launcher,
        environ=os.environ,
        executable=sys.executable,
        prefix=sys.prefix,
        platform=sys.platform,
        python_version=tuple(sys.version_info[:3]),
        installed=installed_version,
        run=run_process,
        stdout=sys.stdout,
        stderr=sys.stderr,
    )
