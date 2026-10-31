"""One run: parse, read the configuration, then `setup`, a re-dispatch into the venv,
or the dependency check and the command. Every run ends in one envelope, except a
re-dispatch, whose child prints it."""

from __future__ import annotations

import traceback
from pathlib import Path
from typing import Any, Sequence

from acmodel import TOOL, __version__, cache, config, deps, environment
from acmodel.bindings import apply_bindings, check_bindings
from acmodel.check import check_model
from acmodel.codes import COMMAND_CRASHED, USAGE_INVALID
from acmodel.core import run_economy, run_model
from acmodel.core.provenance import explain
from acmodel.envelope import Failure, build, error, write
from acmodel.export import Validator, export_scoring, export_tables, export_thresholds
from acmodel.host import windows
from acmodel.system import System
from acmodel.watch.watcher import Watcher

# Each command (one word, or a group and a word): the names of its positional arguments, then its options;
# all are required, and each option takes one value.
COMMANDS = {
    "setup": ((), ()),
    "version": ((), ()),
    "check": (("model",), ()),
    "run": (("model",), ("--state",)),
    "economy": (("model",), ("--state",)),
    "explain": (("model", "output"), ("--state",)),
    "export thresholds": (("model",), ("--state",)),
    "export tables": (("model",), ("--state", "--repo")),
    "export scoring": (("model",), ("--state",)),
    "bindings check": (("model",), ("--state",)),
    "bindings apply": (("model",), ("--state",)),
    "cache stats": ((), ()),
    "cache verify": ((), ()),
    "cache gc": ((), ()),
    "watch": (("model",), ("--state",)),
}
GROUPS = {name.split(" ")[0] for name in COMMANDS if " " in name}
# Flags: options that take no value, each optional and given at most once.
FLAGS = {
    "check": ("--strict-provenance", "--report-supplements"),
    "economy": ("--steady-only",),
    "bindings check": ("--strict",),
    "cache gc": ("--apply",),
}
USAGE = (
    "usage: acmodel.py <command> [<argument>] [--<option> <value>]\n"
    "commands: setup, version,\n"
    "          check <model folder or model.json> [--strict-provenance] [--report-supplements],\n"
    "          run <model folder or model.json> --state <state file>,\n"
    "          economy <model folder or model.json> --state <state file> [--steady-only],\n"
    "          explain <model folder or model.json> <output id> --state <state file>,\n"
    "          export thresholds <model folder or model.json> --state <state file>,\n"
    "          export tables <model folder or model.json> --state <state file> --repo <repository>,\n"
    "          export scoring <model folder or model.json> --state <state file>,\n"
    "          bindings check <model folder or model.json> --state <state file> [--strict],\n"
    "          bindings apply <model folder or model.json> --state <state file>,\n"
    "          cache stats, cache verify, cache gc [--apply],\n"
    "          watch <model folder or model.json> --state <state file>\n"
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


def _check(args: list[str], cfg: config.Config, system: System) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """The static check and its provenance warnings (#424 ACM-PROV): errors only under `--strict-provenance`;
    `--report-supplements` lists the supplemented values."""
    _dependencies(system)
    return check_model(
        Path(args[0]), datasets=cfg.datasets, strict_provenance="--strict-provenance" in args[1:],
        report_supplements="--report-supplements" in args[1:],
    )


def _run(
    args: list[str], cfg: config.Config, system: System
) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]]]:
    """A run returns its outputs together with the errors of the blocked ones (a partial run), and the
    warnings of the cache. It computes through the cache under `paths.state_root`."""
    packages = _dependencies(system)
    with cache.open_store(cfg.state_root, max_bytes=cfg.cache_max_bytes) as store:
        session = cache.Session(store, libraries=cache.libraries(system.python_version, packages))
        data, errors = run_model(
            Path(args[0]), Path(args[1]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits,
            unknowns=cfg.unknowns, cache=session,
        )
    _report_eviction(data, system)
    return data, errors, session.warnings


def _economy(
    args: list[str], cfg: config.Config, system: System
) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]]]:
    """The economy's steady state and plan, with the errors that block them, and the warnings of the cache and
    of the solves; with `--steady-only`, the steady state alone. It computes through the cache under
    `paths.state_root`, as `run` does."""
    packages = _dependencies(system)
    with cache.open_store(cfg.state_root, max_bytes=cfg.cache_max_bytes) as store:
        session = cache.Session(store, libraries=cache.libraries(system.python_version, packages))
        data, errors, warnings = run_economy(
            Path(args[0]), Path(args[1]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits,
            unknowns=cfg.unknowns, economy=cfg.economy, cache=session, steady_only="--steady-only" in args[2:],
        )
    _report_eviction(data, system)
    return data, errors, session.warnings + warnings


def _report_eviction(data: dict[str, Any], system: System) -> None:
    evicted = data["work"]["evicted"]
    if evicted["entries"]:
        system.stderr.write(
            f"acmodel: cache over cache.max_bytes: evicted {evicted['entries']} entries "
            f"({evicted['bytes']} bytes), least recently used first\n"
        )


def _cache(command: str, args: list[str], cfg: config.Config, system: System) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """`cache stats`, `cache verify` (its errors are the corrupt entries) and `cache gc [--apply]`."""
    _dependencies(system)
    if command == "cache stats":
        return cache.stats(cfg.state_root, max_bytes=cfg.cache_max_bytes), []
    if command == "cache verify":
        return cache.verify(cfg.state_root)
    data = cache.gc(cfg.state_root, max_bytes=cfg.cache_max_bytes, apply="--apply" in args)
    if data["apply"]:
        system.stderr.write(
            f"acmodel: cache gc removed {data['entries_removed']} entries, {data['blobs_removed']} blobs "
            f"({data['bytes_removed']} bytes) and {data['orphans_removed']} orphaned files "
            f"({data['orphan_bytes_removed']} bytes)\n"
        )
    return data, []


def _explain(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    _dependencies(system)
    return explain(
        Path(args[0]), args[1], Path(args[2]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits,
        unknowns=cfg.unknowns, economy=cfg.economy,
    )


def _export_thresholds(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    _dependencies(system)
    return export_thresholds(
        Path(args[0]), Path(args[1]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits, unknowns=cfg.unknowns
    )


def _export_tables(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    """The tables into the repository `--repo`, each checked by the validator `paths.actinglab`."""
    _dependencies(system)
    validator = Validator(cfg.actinglab, cfg.validator_timeout_ms, system.run_captured)
    return export_tables(
        Path(args[0]), Path(args[1]), Path(args[2]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits,
        validator=validator, unknowns=cfg.unknowns,
    )


def _export_scoring(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    _dependencies(system)
    return export_scoring(
        Path(args[0]), Path(args[1]), datasets=cfg.datasets, max_bits=cfg.expr_max_bits, unknowns=cfg.unknowns
    )


def _bindings(command: str, args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    """`bindings check [--strict]` compares the bound literals with their thresholds; `bindings apply`
    writes the ones that differ."""
    _dependencies(system)
    model, state = Path(args[0]), Path(args[1])
    if command == "bindings check":
        strict = "--strict" in args[2:]
        return check_bindings(
            model, state, datasets=cfg.datasets, max_bits=cfg.expr_max_bits, strict=strict, unknowns=cfg.unknowns
        )
    return apply_bindings(model, state, datasets=cfg.datasets, max_bits=cfg.expr_max_bits, unknowns=cfg.unknowns)


def _watch(args: list[str], cfg: config.Config, system: System) -> dict[str, Any]:
    """Automatic refresh until interrupted. The tool takes its host caps first (priority and Job object), so that
    every engine worker it starts is inside them."""
    packages = _dependencies(system)
    host = windows.current(system.platform)
    host.apply(cfg.host)
    watcher = Watcher(
        Path(args[0]), Path(args[1]), cfg=cfg, libraries=cache.libraries(system.python_version, packages), host=host,
        workers_os=host, stderr=system.stderr,
    )
    return watcher.run()


def main(argv: Sequence[str], system: System) -> int:
    command = None
    errors: list[dict[str, Any]] = []
    warnings: list[dict[str, Any]] = []
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
            data, warnings = _check(args, cfg, system)
        elif command == "run":
            data, errors, warnings = _run(args, cfg, system)
        elif command == "economy":
            data, errors, warnings = _economy(args, cfg, system)
        elif command == "explain":
            data = _explain(args, cfg, system)
        elif command in ("bindings check", "bindings apply"):
            data = _bindings(command, args, cfg, system)
        elif command == "export thresholds":
            data = _export_thresholds(args, cfg, system)
        elif command == "export tables":
            data = _export_tables(args, cfg, system)
        elif command == "export scoring":
            data = _export_scoring(args, cfg, system)
        elif command == "watch":
            data = _watch(args, cfg, system)
        else:
            data, errors = _cache(command, args, cfg, system)
    except Failure as failure:
        if failure.errors[0]["code"] == USAGE_INVALID:
            system.stderr.write(USAGE)
        return write(build(command, {}, failure.errors, failure.warnings), system.stdout, system.stderr)
    except Exception as exc:  # a defect: the traceback goes to stderr, the envelope still holds
        traceback.print_exc(file=system.stderr)
        crashed = error(COMMAND_CRASHED, {"exception_type": type(exc).__name__})
        return write(build(command, {}, [crashed]), system.stdout, system.stderr)
    return write(build(command, data, errors, warnings), system.stdout, system.stderr)
