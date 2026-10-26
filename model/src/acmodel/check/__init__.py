"""The static check of a model (`acmodel check`, contracts/formats.md "The static check").

`check_model(path, datasets=...)` reads the model with the strict readers, then checks what they
leave: references, duplicate ids, statuses, provenance, `supersedes` and conflicts, pending rules
and unknowns without a range, where `pref:` may appear, loops, and the hashes of external datasets.
It computes nothing. It returns a summary, or raises `Failure` with every problem found.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from acmodel.check.checker import Checker
from acmodel.check.files import load


def check_model(path: Path | str, *, datasets: Path | str) -> dict[str, Any]:
    """`path` is a model folder or its `model.json`; `datasets` holds the external datasets' copies."""
    return Checker(load(Path(path)), Path(datasets)).run()


__all__ = ["check_model"]
