"""One run: parse, read the configuration, then `setup`, a re-dispatch into the venv,
or the dependency check and the command. Every run ends in one envelope, except a
re-dispatch, whose child prints it."""

from __future__ import annotations

import traceback
from pathlib import Path
from typing import Any, Sequence

from acmodel import TOOL, __version__, config, deps, environment
from acmodel.check import check_model
from acmodel.codes import COMMAND_CRASHED, USAGE_INVALID
from acmodel.envelope import Failure, build, error, write
from acmodel.system import System

# Each command and the names of its arguments, all required.
COMMANDS = {"setup": (), "version": (), "check": ("model",)}
USAGE = (
    "usage: acmodel.py <command> [<argument>]\n"
    "commands: setup, version, check <model folder or model.json>\n"
)


def parse(argv: Sequence[str]) -> tuple[str, list[str]]:
    if not argv:
        raise Failure(error(USAGE_INVALID, {"reason": "command_missing"}))
    command, *rest = argv
    if command not in COMMANDS:
        raise Failure(error(USAGE_INVALID, {"reason": "command_unknown", "argument": command}))
    names = COMMANDS[command]
    if len(rest) < len(names):
        raise Failure(error(USAGE_INVALID, {"reason": "argument_missing", "argument": names[len(rest)]}))
    if len(rest) > len(names):
        raise Failure(error(USAGE_INVALID, {"reason": "argument_unexpected", "argument": rest[len(names)]}))
    return command, rest


def _dependencies(system: System) -> dict[str, str]:
    return deps.check(
        system.python_version,
        system.installed,
        deps.setup_command(system.launcher, system.platform),
    )


def _version(system: System) -> dict[str, Any]:
    return {
        "tool": TOOL,
        "tool_version": __version__,
        "python": deps.label(system.python_version),
        "packages": _dependencies(system),
    }


def _check(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    _dependencies(system)
    return check_model(Path(args[0]), datasets=cfg.datasets)


def main(argv: Sequence[str], system: System) -> int:
    command = None
    try:
        command, args = parse(argv)
        cfg = config.load(system.environ)
        environment.set_blas_threads(system.environ, cfg.blas_threads)
        if command == "setup":
            data = environment.setup(cfg, system)
        elif not environment.under_venv(cfg.venv, system.prefix):
            return environment.dispatch(argv, cfg, system)
        elif command == "version":
            data = _version(system)
        else:
            data = _check(args, cfg, system)
    except Failure as failure:
        if failure.errors[0]["code"] == USAGE_INVALID:
            system.stderr.write(USAGE)
        return write(build(command, {}, failure.errors), system.stdout, system.stderr)
    except Exception as exc:  # a defect: the traceback goes to stderr, the envelope still holds
        traceback.print_exc(file=system.stderr)
        crashed = error(COMMAND_CRASHED, {"exception_type": type(exc).__name__})
        return write(build(command, {}, [crashed]), system.stdout, system.stderr)
    return write(build(command, data, []), system.stdout, system.stderr)
