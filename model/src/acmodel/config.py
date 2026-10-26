"""Configuration: `%LOCALAPPDATA%\\ActingCommand\\acmodel\\config.json` (`acmodel.config.v1`).

The file is optional and every key has a default. A file that exists is read
strictly: UTF-8 JSON, no duplicate keys, no unknown members, typed values.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from acmodel.codes import CONFIG_INVALID
from acmodel.envelope import Failure, error

SCHEMA_VERSION = "acmodel.config.v1"
LOCATION_VARIABLE = "LOCALAPPDATA"
APP_DIR = ("ActingCommand", "acmodel")
DEFAULT_BLAS_THREADS = 1

_MEMBERS = {"schema_version": None, "paths": {"venv", "state_root"}, "compute": {"blas_threads"}}


@dataclass(frozen=True)
class Config:
    path: Path
    venv: Path
    state_root: Path
    blas_threads: int


class _DuplicateKey(ValueError):
    def __init__(self, key: str) -> None:
        super().__init__(key)
        self.key = key


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    doc: dict[str, Any] = {}
    for key, value in pairs:
        if key in doc:
            raise _DuplicateKey(key)
        doc[key] = value
    return doc


def _pointer(*parts: str) -> str:
    return "".join("/" + part.replace("~", "~0").replace("/", "~1") for part in parts)


def _invalid(reason: str, path: Path, ref: str | None = None, **values: Any) -> Failure:
    return Failure(error(CONFIG_INVALID, {"reason": reason, "path": str(path), **values}, ref))


def load(environ: Mapping[str, str]) -> Config:
    base = environ.get(LOCATION_VARIABLE, "")
    if not base or not Path(base).is_absolute():
        raise Failure(error(CONFIG_INVALID, {"reason": "location_invalid", "variable": LOCATION_VARIABLE}))
    root = Path(base, *APP_DIR)
    path = root / "config.json"
    try:
        raw = path.read_bytes()
    except FileNotFoundError:
        return _build({"schema_version": SCHEMA_VERSION}, path, root)
    except OSError as exc:
        raise _invalid("unreadable", path, exception_type=type(exc).__name__, raw_text=str(exc)) from exc
    try:
        doc = json.loads(raw.decode("utf-8"), object_pairs_hook=_no_duplicates)
    except _DuplicateKey as exc:
        raise _invalid("key_duplicate", path, key=exc.key) from exc
    except ValueError as exc:  # bad UTF-8, a BOM, or not JSON
        raise _invalid("malformed", path, exception_type=type(exc).__name__, raw_text=str(exc)) from exc
    return _build(doc, path, root)


def _section(doc: dict[str, Any], name: str, path: Path) -> dict[str, Any]:
    section = doc.get(name, {})
    if not isinstance(section, dict):
        raise _invalid("value_invalid", path, _pointer(name))
    for key in sorted(section):
        if key not in _MEMBERS[name]:
            raise _invalid("member_unknown", path, _pointer(name, key), key=key)
    return section


def _absolute(section: dict[str, Any], key: str, default: Path, path: Path) -> Path:
    if key not in section:
        return default
    value = section[key]
    if not isinstance(value, str) or not value or not Path(value).is_absolute():
        raise _invalid("value_invalid", path, _pointer("paths", key))
    return Path(value)


def _build(doc: Any, path: Path, root: Path) -> Config:
    if not isinstance(doc, dict):
        raise _invalid("value_invalid", path, "")
    for key in sorted(doc):
        if key not in _MEMBERS:
            raise _invalid("member_unknown", path, _pointer(key), key=key)
    if "schema_version" not in doc:
        raise _invalid("member_missing", path, _pointer("schema_version"), key="schema_version")
    if doc["schema_version"] != SCHEMA_VERSION:
        raise _invalid("value_invalid", path, _pointer("schema_version"))
    paths = _section(doc, "paths", path)
    compute = _section(doc, "compute", path)
    threads = compute.get("blas_threads", DEFAULT_BLAS_THREADS)
    if type(threads) is not int or threads < 1:
        raise _invalid("value_invalid", path, _pointer("compute", "blas_threads"))
    return Config(
        path=path,
        venv=_absolute(paths, "venv", root / "venv", path),
        state_root=_absolute(paths, "state_root", root / "state", path),
        blas_threads=threads,
    )
