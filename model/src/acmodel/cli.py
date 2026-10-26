"""One run: parse, read the configuration, then `setup`, a re-dispatch into the venv,
or the dependency check and the command. Every run ends in one envelope, except a
re-dispatch, whose child prints it."""

from __future__ import annotations

import traceback
from typing import Any, Sequence

from acmodel import TOOL, __version__, config, deps, environment
from acmodel.codes import COMMAND_CRASHED, USAGE_INVALID
from acmodel.envelope import Failure, build, error, write
from acmodel.system import System

COMMANDS = ("setup", "version")
USAGE = "usage: acmodel.py <command>\ncommands: " + ", ".join(COMMANDS) + "\n"


def parse(argv: Sequence[str]) -> str:
    if not argv:
        raise Failure(error(USAGE_INVALID, {"reason": "command_missing"}))
    command, *rest = argv
    if command not in COMMANDS:
        raise Failure(error(USAGE_INVALID, {"reason": "command_unknown", "argument": command}))
    if rest:
        raise Failure(error(USAGE_INVALID, {"reason": "argument_unexpected", "argument": rest[0]}))
    return command


def _version(system: System) -> dict[str, Any]:
    packages = deps.check(
        system.python_version,
        system.installed,
        deps.setup_command(system.launcher, system.platform),
    )
    return {
        "tool": TOOL,
        "tool_version": __version__,
        "python": deps.label(system.python_version),
        "packages": packages,
    }


def main(argv: Sequence[str], system: System) -> int:
    command = None
    try:
        command = parse(argv)
        cfg = config.load(system.environ)
        environment.set_blas_threads(system.environ, cfg.blas_threads)
        if command == "setup":
            data = environment.setup(cfg, system)
        elif not environment.under_venv(cfg.venv, system.prefix):
            return environment.dispatch(argv, cfg, system)
        else:
            data = _version(system)
    except Failure as failure:
        if failure.errors[0]["code"] == USAGE_INVALID:
            system.stderr.write(USAGE)
        return write(build(command, {}, failure.errors), system.stdout, system.stderr)
    except Exception as exc:  # a defect: the traceback goes to stderr, the envelope still holds
        traceback.print_exc(file=system.stderr)
        crashed = error(COMMAND_CRASHED, {"exception_type": type(exc).__name__})
        return write(build(command, {}, [crashed]), system.stdout, system.stderr)
    return write(build(command, data, []), system.stdout, system.stderr)
