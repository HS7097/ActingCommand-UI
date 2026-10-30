"""The static check of a model (`acmodel check`, contracts/formats.md "The static check").

`check_model(path, datasets=...)` reads the model with the strict readers, then checks what they
leave: references, duplicate ids, statuses, provenance, `supersedes` and conflicts, pending rules
and unknowns without a range, where `pref:` may appear, loops, and the hashes of external datasets.
It computes nothing. It returns a summary and the provenance warnings (#424 ACM-PROV), or raises
`Failure` with every problem found and those warnings.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from acmodel.check.checker import Checker
from acmodel.check.files import load
from acmodel.envelope import Failure


def check_model(
    path: Path | str, *, datasets: Path | str, strict_provenance: bool = False, report_supplements: bool = False
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """`path` is a model folder or its `model.json`; `datasets` holds the external datasets' copies. The
    provenance classes are warnings, or errors with `strict_provenance`; `report_supplements` adds the
    supplemented values to the summary (`supplements`)."""
    checker = Checker(load(Path(path)), Path(datasets), strict_provenance=strict_provenance)
    try:
        data = checker.run()
    except Failure as failure:
        failure.warnings = list(checker.warnings)
        raise
    if report_supplements:
        data["supplements"] = list(checker.supplements)
    return data, list(checker.warnings)


__all__ = ["check_model"]
