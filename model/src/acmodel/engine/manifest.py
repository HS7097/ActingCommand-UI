"""An engine's manifest `engine.json` (`acmodel.engine.v2`), its files and its fingerprint (#424 design-py 7.2;
contracts/formats.md, "Engines").

`load(engine_id, roots=…, datasets=…, libraries=…)` finds the engine's home in `engines.roots`
(`engine_not_configured` if absent), reads `engine.json` strictly (`model_file_invalid`), lists the files its
`code` and `data` patterns match, verifies its external datasets (`data_external_missing`,
`data_external_hash_mismatch`) and computes the fingerprint.

**Fingerprint** = sha256 over canonical JSON of: the manifest's bytes' sha256; every listed file as
[path relative to the home, sha256 of its bytes], sorted by path; the external datasets as [source, sha256];
and the library versions (the Python minor version, numpy, scipy; design-py D-P5). The tool computes it;
the manifest never supplies it. Any changed byte in a listed file is a new fingerprint.
"""

from __future__ import annotations

import hashlib
import os
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, Mapping

from acmodel.cache.keys import canonical, digest
from acmodel.check.external import dataset_path, verify
from acmodel.codes import ENGINE_NOT_CONFIGURED
from acmodel.engine.protocol import INTERFACE, OPS
from acmodel.envelope import Failure, error
from acmodel.model import readers
from acmodel.model.jsonfile import parse, read, refusal
from acmodel.model.shape import Choice, Context, Integer, ListOf, Object, Text

FILE = "engine.json"
FINGERPRINT_SCHEMA = "acmodel.engine.fingerprint.v1"
_IDENTIFIER = r"[A-Za-z_][A-Za-z0-9_]*"
ENTRY = Text(rf"{_IDENTIFIER}(\.{_IDENTIFIER})*:{_IDENTIFIER}")
# Relative paths below the home with `/`: no `.` or `..` segment, no drive, no backslash. A folder may be
# `.`, the home itself; a pattern may use `*`, `?` and `**` (pathlib's glob).
_SEGMENT = r"(?!\.{1,2}(/|$))[A-Za-z0-9_.-]+"
FOLDER = Text(rf"\.|{_SEGMENT}(/{_SEGMENT})*")
_GLOB_SEGMENT = r"(?!\.{1,2}(/|$))[A-Za-z0-9_.*?-]+"
PATTERN = Text(rf"{_GLOB_SEGMENT}(/{_GLOB_SEGMENT})*")

SHAPE = Object(
    {
        "schema_version": readers.ANY,
        "engine_id": readers.ID,
        "version": readers.VERSION,
        "python": Text(r"[0-9]+\.[0-9]+"),
        "entry": ENTRY,
        "sys_path": ListOf(FOLDER),
        "code": ListOf(PATTERN, min_items=1),
        "data": ListOf(PATTERN),
        "external": ListOf(Object({"source": readers.ID, "sha256": readers.SHA256})),
        "ops": ListOf(Choice(*OPS), min_items=1),
        "deterministic": Choice(True),
        "max_batch": Integer(1, readers.MAX),
        "memory_mib_estimate": Integer(1, readers.MAX),
    },
    {"config": readers.ANY, "note": readers.TEXT},
)


@dataclass(frozen=True)
class Engine:
    """One engine as the host knows it: its home, manifest, listed files and fingerprint."""

    engine_id: str
    root: Path  # the real path of the home
    manifest_path: Path
    doc: dict[str, Any]
    code: tuple[str, ...]  # the absolute paths of the files `code` lists, sorted
    data: tuple[str, ...]  # the same for `data`
    external: dict[str, str]  # source to the dataset's local path
    fingerprint: str

    @property
    def ops(self) -> tuple[str, ...]:
        return tuple(self.doc["ops"])

    @property
    def max_batch(self) -> int:
        return self.doc["max_batch"]

    @property
    def memory_mib_estimate(self) -> int:
        return self.doc["memory_mib_estimate"]


def python_minor() -> str:
    return f"{sys.version_info[0]}.{sys.version_info[1]}"


def matched(root: Path, patterns: list[str]) -> list[str]:
    """The files under `root` that the patterns match, as paths relative to it with `/`, sorted."""
    found: set[str] = set()
    for pattern in patterns:
        for path in root.glob(pattern):
            if path.is_file():
                found.add(path.relative_to(root).as_posix())
    return sorted(found)


def _sha256(path: Path, ctx: Context, ref: str) -> str | None:
    try:
        with path.open("rb") as handle:
            return hashlib.file_digest(handle, "sha256").hexdigest()
    except OSError as exc:
        ctx.errors.append(refusal(
            "unreadable", path, INTERFACE, ref, exception_type=type(exc).__name__, raw_text=str(exc)
        ))
        return None


def load(
    engine_id: str, *, roots: Mapping[str, Path], datasets: Path, libraries: Mapping[str, str]
) -> Engine:
    if engine_id not in roots:
        raise Failure(error(ENGINE_NOT_CONFIGURED, {"engine_id": engine_id}))
    root = Path(os.path.realpath(roots[engine_id]))
    path = root / FILE
    raw = read(path, INTERFACE)
    doc = parse(raw, path, INTERFACE)
    ctx = Context(path, INTERFACE)
    if not isinstance(doc, dict):
        ctx.refuse("type_invalid", "", expected=["object"])
    elif "schema_version" not in doc:
        ctx.refuse("member_missing", "/schema_version", key="schema_version")
    elif Choice(INTERFACE).check(doc["schema_version"], "/schema_version", ctx) and SHAPE.check(doc, "", ctx):
        _meaning(doc, engine_id, root, ctx)
    if ctx.errors:
        raise Failure(*ctx.errors)
    code = matched(root, doc["code"])
    data = matched(root, doc["data"])
    files = []
    for relative in sorted(set(code) | set(data)):
        sha256 = _sha256(root / relative, ctx, "")
        files.append([relative, sha256])
    problems = []
    external = {}
    for index, item in enumerate(doc["external"]):
        for entry in verify(str(path), item, datasets):
            problems.append({**entry, "ref": f"/external/{index}" + entry["ref"].removeprefix("/external")})
        external[item["source"]] = str(dataset_path(datasets, item["sha256"]))
    if ctx.errors or problems:
        raise Failure(*ctx.errors, *problems)
    token = {
        "schema_version": FINGERPRINT_SCHEMA,
        "manifest": hashlib.sha256(raw).hexdigest(),
        "files": files,
        "external": sorted([item["source"], item["sha256"]] for item in doc["external"]),
        "libraries": dict(libraries),
    }
    return Engine(
        engine_id=engine_id,
        root=root,
        manifest_path=path,
        doc=doc,
        code=tuple(str(root / PurePosixPath(relative)) for relative in code),
        data=tuple(str(root / PurePosixPath(relative)) for relative in data),
        external=external,
        fingerprint=digest(canonical(token)),
    )


def _meaning(doc: dict[str, Any], engine_id: str, root: Path, ctx: Context) -> None:
    """What the shape leaves: the engine is the one asked for, on this Python; the ops include `describe`
    and repeat none; every `sys_path` folder exists; every pattern matches a file."""
    if doc["engine_id"] != engine_id:
        ctx.refuse("value_invalid", "/engine_id", expected=[engine_id], literal=doc["engine_id"])
    if doc["python"] != python_minor():
        ctx.refuse("value_invalid", "/python", expected=[python_minor()], literal=doc["python"])
    ops = doc["ops"]
    if "describe" not in ops or len(set(ops)) != len(ops):
        ctx.refuse("value_invalid", "/ops", expected=list(OPS))
    for index, folder in enumerate(doc["sys_path"]):
        if not (root / PurePosixPath(folder)).is_dir():
            ctx.refuse("value_invalid", f"/sys_path/{index}", literal=folder)
    for member in ("code", "data"):
        for index, pattern in enumerate(doc[member]):
            if not matched(root, [pattern]):
                ctx.refuse("value_invalid", f"/{member}/{index}", literal=pattern)

