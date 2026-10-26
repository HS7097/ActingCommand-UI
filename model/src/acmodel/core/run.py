"""`acmodel run`: the static check, the run's model checks, then the derivation graph for one account state.

The model folder holds `exports.json` beside `model.json`. The preference defaults are
`ours/prefs/defaults.json` of the same repository, `<model folder>/../prefs/defaults.json` (#424 design
3.1); they are read only when the model reads a preference.

With a cache session (`acmodel.cache`), the graph computes incrementally, the run gets its run id, and its
outputs are kept under `runs/<run id>/` (contracts/cli.md, "Cache").
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any, Callable

from acmodel.cache.keys import RUN_SCHEMA, canonical, digest
from acmodel.check.checker import Checker
from acmodel.check.files import File, load
from acmodel.core import types
from acmodel.core.graph import Graph
from acmodel.envelope import Failure
from acmodel.model import read_exports, read_prefs_defaults, read_state, readers
from acmodel.model.jsonfile import refusal

if TYPE_CHECKING:
    from acmodel.cache.session import Session

EXPORTS = "exports.json"


def _read(reader: Callable[[Path], dict[str, Any]], path: Path, errors: list[dict[str, Any]]) -> dict[str, Any] | None:
    try:
        return reader(path)
    except Failure as failure:
        errors.extend(failure.errors)
        return None


def versions(checker: Checker, folder: Path) -> dict[str, str]:
    """`rules`: the sha256 over `model.json`, `sources.json` and the rule files; `data`: over the data files.
    Each file counts as [its path relative to the model folder, its document], sorted by path."""
    model = checker.model

    def version(files: list[File]) -> str:
        pairs = [[Path(file.path).relative_to(folder).as_posix(), file.doc] for file in files]
        return digest(canonical(sorted(pairs, key=lambda pair: pair[0])))

    return {"rules": version([model.manifest, model.sources, *model.rules]), "data": version(list(model.data))}


def run_model(
    path: Path | str, state_path: Path | str, *, datasets: Path | str, max_bits: int, cache: Session | None = None
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Run the model at `path` (its folder or `model.json`) for the state at `state_path`.

    A model that does not check, or inputs that do not read, raise `Failure` and compute nothing.
    Otherwise every output is computed or blocked: the result is the run's `data` and the errors of
    the blocked outputs. With `cache`, `data` also holds `run_id` and `work`, and the outputs are kept.
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
    prefs: list[dict[str, Any]] = []  # the preference defaults, once read

    def load_prefs() -> dict[str, Any]:
        prefs.append(read_prefs_defaults(prefs_path))
        return prefs[-1]

    typed = types.check(checker, File(str(exports_path), exports), load_prefs)
    data, blocked = Graph(checker, typed, state, exports, max_bits=max_bits, cache=cache).run()
    head = {
        "path": manifest.path,
        "model_id": model_id,
        "state": {"path": str(state_path), "label": state["label"]},
    }
    if cache is None:
        return {**head, **data}, blocked
    run_id = cache.run_id({
        **versions(checker, folder),
        "exports": digest(canonical(exports)),
        "state": digest(canonical(state)),
        "prefs": digest(canonical(prefs[-1])) if prefs else None,
        "engines": {},  # engine fingerprints come with the engines
        "config": {"max_bits": max_bits},
        "paths": {"model": manifest.path, "state": str(state_path)},
    })
    outputs = {"schema_version": RUN_SCHEMA, "run_id": run_id, "data": {**head, **data}, "errors": blocked}
    cache.finish(run_id, canonical(outputs))
    return {**head, "run_id": run_id, **data, "work": cache.work()}, blocked
