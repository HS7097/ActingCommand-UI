"""Reading a model's files: `model.json`, `sources.json` beside it, and the files it lists."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from acmodel.envelope import Failure
from acmodel.model import read_data, read_model, read_rules, read_sources

MANIFEST = "model.json"
SOURCES = "sources.json"


@dataclass(frozen=True)
class File:
    path: str  # as the errors name it
    doc: dict[str, Any]


@dataclass(frozen=True)
class Model:
    manifest: File
    sources: File
    rules: tuple[File, ...]
    data: tuple[File, ...]


def manifest_path(path: Path) -> Path:
    """`path` is a model folder or its `model.json`."""
    return path / MANIFEST if path.is_dir() else path


def load(path: Path) -> Model:
    """Read every file of the model, or raise `Failure` with every reader's errors.

    The manifest comes first: when it cannot be read, nothing else can be found.
    """
    manifest_file = manifest_path(path)
    manifest = File(str(manifest_file), read_model(manifest_file))
    folder = manifest_file.parent
    errors: list[dict[str, Any]] = []

    def read(reader: Callable[[Path], dict[str, Any]], target: Path) -> File | None:
        try:
            return File(str(target), reader(target))
        except Failure as failure:
            errors.extend(failure.errors)
            return None

    sources = read(read_sources, folder / SOURCES)
    rules = [read(read_rules, folder / name) for name in manifest.doc["files"]["rules"]]
    data = [read(read_data, folder / name) for name in manifest.doc["files"]["data"]]
    if errors:
        raise Failure(*errors)
    return Model(manifest, sources, tuple(rules), tuple(data))
