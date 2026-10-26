"""External datasets: the local copy of each is the file named by its sha256 under `paths.datasets`.

Fetching stays on the data side; the check only verifies the copy by its hash.
"""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from acmodel.codes import DATA_EXTERNAL_HASH_MISMATCH, DATA_EXTERNAL_MISSING
from acmodel.envelope import error


def dataset_path(datasets: Path, sha256: str) -> Path:
    return datasets / sha256


def verify(path: str, external: dict[str, Any], datasets: Path) -> list[dict[str, Any]]:
    """The errors of one data file's `external` declaration: its copy is absent, unreadable or different."""
    sha256 = external["sha256"]
    dataset = dataset_path(datasets, sha256)
    missing = {"path": path, "sha256": sha256, "source": external["source"], "dataset": str(dataset)}
    try:
        with dataset.open("rb") as handle:
            found = hashlib.file_digest(handle, "sha256").hexdigest()
    except FileNotFoundError:
        return [error(DATA_EXTERNAL_MISSING, missing, "/external")]
    except OSError as exc:  # a folder in its place, or no permission
        detail = {**missing, "exception_type": type(exc).__name__, "raw_text": str(exc)}
        return [error(DATA_EXTERNAL_MISSING, detail, "/external")]
    if found != sha256:
        detail = {"path": path, "sha256": sha256, "found": found, "dataset": str(dataset)}
        return [error(DATA_EXTERNAL_HASH_MISMATCH, detail, "/external/sha256")]
    return []
