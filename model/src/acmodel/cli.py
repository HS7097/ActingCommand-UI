"""One run: parse, read the configuration, then `setup`, a re-dispatch into the venv,
or the dependency check and the command. Every run ends in one envelope, except a
re-dispatch, whose child prints it."""

from __future__ import annotations

import traceback
from pathlib import Path
from typing import Any, Sequence

from acmodel import TOOL, __version__, config, deps, environment
from acmodel.bindings import apply_bindings, check_bindings
from acmodel.check import check_model
from acmodel.codes import COMMAND_CRASHED, USAGE_INVALID
from acmodel.core import run_model
from acmodel.core.provenance import explain
from acmodel.envelope import Failure, build, error, write
from acmodel.export import export_thresholds
from acmodel.system import System

# Each command (one word, or a group and a word): the names of its positional arguments, then its options;
# all are required, and each option takes one value.
COMMANDS = {
    "setup": ((), ()),
    "version": ((), ()),
    "check": (("model",), ()),
    "run": (("model",), ("--state",)),
    "explain": (("model", "output"), ("--state",)),
    "export thresholds": (("model",), ("--state",)),
    "bindings check": (("model",), ("--state",)),
    "bindings apply": (("model",), ("--state",)),
}
GROUPS = {name.split(" ")[0] for name in COMMANDS if " " in name}
# Flags: options that take no value, each optional and given at most once.
FLAGS = {"bindings check": ("--strict",)}
USAGE = (
    "usage: acmodel.py <command> [<argument>] [--<option> <value>]\n"
    "commands: setup, version, check <model folder or model.json>,\n"
    "          run <model folder or model.json> --state <state file>,\n"
    "          explain <model folder or model.json> <output id> --state <state file>,\n"
    "          export thresholds <model folder or model.json> --state <state file>,\n"
    "          bindings check <model folder or model.json> --state <state file> [--strict],\n"
    "          bindings apply <model folder or model.json> --state <state file>\n"
)


def _usage(reason: str, argument: str | None = None) -> Failure:
    detail = {"reason": reason} if argument is None else {"reason": reason, "argument": argument}
    return Failure(error(USAGE_INVALID, detail))


def parse(argv: Sequence[str]) -> tuple[str, list[str]]:
    """The command and its arguments: the positional ones, then the option values, in declared order."""
    if not argv:
        raise _usage("command_missing")
    command, *rest = argv
    if command in GROUPS and rest:
        command, rest = f"{command} {rest[0]}", rest[1:]
    if command not in COMMANDS:
        raise _usage("command_unknown", command)
    names, options = COMMANDS[command]
    flags = FLAGS.get(command, ())
    positional: list[str] = []
    given: dict[str, str] = {}
    set_flags: set[str] = set()
    items = iter(rest)
    for item in items:
        if not item.startswith("--"):
            positional.append(item)
            continue
        if item in flags and item not in set_flags:
            set_flags.add(item)
            continue
        if item not in options or item in given:
            raise _usage("argument_unexpected", item)
        value = next(items, None)
        if value is None:
            raise _usage("argument_missing", item)
        given[item] = value
    if len(positional) < len(names):
        raise _usage("argument_missing", names[len(positional)])
    if len(positional) > len(names):
        raise _usage("argument_unexpected", positional[len(names)])
    for option in options:
        if option not in given:
            raise _usage("argument_missing", option)
    return command, positional + [given[option] for option in options] + [flag for flag in flags if flag in set_flags]


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


def _run(args: list[str], cfg: config.Config, system: System) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """A run returns its outputs together with the errors of the blocked ones (a partial run)."""
    _dependencies(system)
    return run_model(Path(args[0]), Path(args[1]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits)


def _explain(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    _dependencies(system)
    return explain(Path(args[0]), args[1], Path(args[2]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits)


def _export_thresholds(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    _dependencies(system)
    return export_thresholds(Path(args[0]), Path(args[1]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits)


def _bindings(command: str, args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    """`bindings check [--strict]` compares the bound literals with their thresholds; `bindings apply`
    writes the ones that differ."""
    _dependencies(system)
    model, state = Path(args[0]), Path(args[1])
    if command == "bindings check":
        strict = "--strict" in args[2:]
        return check_bindings(model, state, datasets=cfg.datasets, max_bits=cfg.expr_max_bits, strict=strict)
    return apply_bindings(model, state, datasets=cfg.datasets, max_bits=cfg.expr_max_bits)


def main(argv: Sequence[str], system: System) -> int:
    command = None
    errors: list[dict[str, Any]] = []
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
        elif command == "check":
            data = _check(args, cfg, system)
        elif command == "run":
            data, errors = _run(args, cfg, system)
        elif command == "explain":
            data = _explain(args, cfg, system)
        elif command in ("bindings check", "bindings apply"):
            data = _bindings(command, args, cfg, system)
        else:
            data = _export_thresholds(args, cfg, system)
    except Failure as failure:
        if failure.errors[0]["code"] == USAGE_INVALID:
            system.stderr.write(USAGE)
        return write(build(command, {}, failure.errors), system.stdout, system.stderr)
    except Exception as exc:  # a defect: the traceback goes to stderr, the envelope still holds
        traceback.print_exc(file=system.stderr)
        crashed = error(COMMAND_CRASHED, {"exception_type": type(exc).__name__})
        return write(build(command, {}, [crashed]), system.stdout, system.stderr)
    return write(build(command, data, errors), system.stdout, system.stderr)
