"""The versions of a model's inputs: one definition, used by the provenance documents and the exports
(`provenance`, `export.thresholds`) and by the run id (`run`, #424 design 5.1 and 6.4).

It sits apart from both so that `run` can use it without importing `provenance`, which imports `run`.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from acmodel import __version__
from acmodel.cache.keys import canonical, digest
from acmodel.check.checker import Checker
from acmodel.check.files import File


def sha256(doc: Any) -> str:
    """The sha256 of a document's canonical JSON: sorted keys, no whitespace, UTF-8."""
    return digest(canonical(doc))


def relative(folder: Path, path: str) -> str:
    """`path` relative to the model folder, with `/`."""
    return Path(path).relative_to(folder).as_posix()


def versions(checker: Checker, folder: Path) -> dict[str, str]:
    """`rules`: the sha256 over `model.json`, `sources.json` and the rule files; `data`: over the data files.
    Each file counts as [its path relative to the model folder, its document], sorted by path. `tool`: the
    tool version."""
    model = checker.model

    def version(files: list[File]) -> str:
        return sha256(sorted(([relative(folder, file.path), file.doc] for file in files), key=lambda pair: pair[0]))

    return {
        "rules": version([model.manifest, model.sources, *model.rules]),
        "data": version(list(model.data)),
        "tool": __version__,
    }
