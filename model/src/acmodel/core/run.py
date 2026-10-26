"""`acmodel run`: the static check, the run's model checks, then the derivation graph for one account state.

The model folder holds `exports.json` beside `model.json`. The preference defaults are
`ours/prefs/defaults.json` of the same repository, `<model folder>/../prefs/defaults.json` (#424 design
3.1); they are read only when the model reads a preference.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from acmodel.check.checker import Checker
from acmodel.check.files import File, load
from acmodel.core import types
from acmodel.core.graph import Graph
from acmodel.envelope import Failure
from acmodel.model import read_exports, read_prefs_defaults, read_state, readers
from acmodel.model.jsonfile import refusal

EXPORTS = "exports.json"


@dataclass(frozen=True)
class Run:
    """One evaluated run: what `run` prints (`head`, `data`, `errors`), and the graph it came from, which
    `explain` and the exports read."""

    checker: Checker
    graph: Graph
    folder: Path  # the model folder, which the paths of shared outputs are relative to
    exports: File
    head: dict[str, Any]
    data: dict[str, Any]
    errors: list[dict[str, Any]]


def _read(reader: Callable[[Path], dict[str, Any]], path: Path, errors: list[dict[str, Any]]) -> dict[str, Any] | None:
    try:
        return reader(path)
    except Failure as failure:
        errors.extend(failure.errors)
        return None


def evaluate_model(path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int) -> Run:
    """Check the model at `path` (its folder or `model.json`) and evaluate it for the state at `state_path`.

    A model that does not check, or inputs that do not read, raise `Failure` and compute nothing.
    Otherwise every output is computed or blocked: `errors` are those of the blocked outputs.
    """
    checker = Checker(load(Path(path)), Path(datasets))
    checker.run()
    manifest = checker.model.manifest
    folder = Path(manifest.path).parent
    state_path, exports_path = Path(state_path), folder / EXPORTS
    errors: list[dict[str, Any]] = []
    state = _read(read_state, state_path, errors)
    exports = _read(read_exports, exports_path, errors)
    if errors:
        raise Failure(*errors)
    model_id = manifest.doc["model_id"]
    if state["model_id"] != model_id:
        raise Failure(refusal(
            "value_invalid", state_path, readers.STATE.name, "/model_id", expected=[model_id], literal=state["model_id"]
        ))
    prefs_path = folder.parent / "prefs" / "defaults.json"
    exports_file = File(str(exports_path), exports)
    typed = types.check(checker, exports_file, lambda: read_prefs_defaults(prefs_path))
    graph = Graph(checker, typed, state, exports, max_bits=max_bits)
    data, blocked = graph.run()
    head = {
        "path": manifest.path,
        "model_id": model_id,
        "state": {"path": str(state_path), "label": state["label"]},
    }
    return Run(checker, graph, folder, exports_file, head, data, blocked)


def run_model(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """`acmodel run`: the run's `data` and the errors of the blocked outputs (see `evaluate_model`)."""
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits)
    return {**run.head, **run.data}, run.errors
