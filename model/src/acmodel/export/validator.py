"""The Runtime's own validator of delivered files (#424 design-py 6.1): `actinglab`, an external command.

`paths.actinglab` names it: an `actinglab` from a Runtime remote build or release, never compiled here. For
each file an export wrote, it runs

    <paths.actinglab> --json resource validate --repo <repo> --changed-path <file relative to repo>

and reads the one JSON envelope `actinglab` prints on stdout (its stderr passes through to ours):
- **valid:** exit 0, `ok` true, and the answer's entry for the file has `status` `valid`;
- **refused:** `ok` false with its `error`. The validator's problem and JSON pointer are its
  `details.standard_pack_problem` and `details.json_pointer`, or, for a table, the `standard_pack_problem=…
  json_pointer=…` of its message; a refusal that names no problem gives its error code instead. An entry with
  another status (`excluded`) is a refusal too: the file was not validated.
  Either is `export_table_invalid`, with the problem in `problem` and the pointer as the error's `ref`.
- **No usable answer** (not configured, not started, past `export.validator_timeout_ms`, or an answer of
  another shape) is `export_validator_missing` with its `reason`. The tool keeps no copy of the Runtime's limits,
  so an export never stands without the validator.
"""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from acmodel.codes import EXPORT_TABLE_INVALID, EXPORT_VALIDATOR_MISSING
from acmodel.envelope import Failure, error

KEY = "paths.actinglab"
RAW_TEXT_MAX = 1024
# How a table refusal names its problem and pointer in the message (`StandardPackTableIssue`'s display).
PROBLEM_TEXT = re.compile(r"standard_pack_problem=(\S+) json_pointer=(\S*)")


@dataclass(frozen=True)
class Validator:
    """`path` (`paths.actinglab`, None when not configured), `timeout_ms` (`export.validator_timeout_ms`) and
    `run`, which runs a command and returns its exit code and stdout (`System.run_captured`)."""

    path: Path | None
    timeout_ms: int
    run: Callable[..., tuple[int, bytes]]

    def require(self) -> None:
        """`export_validator_missing` (`not_configured`) when no validator is configured."""
        if self.path is None:
            raise Failure(error(EXPORT_VALIDATOR_MISSING, {"key": KEY, "reason": "not_configured"}))


def _missing(validator: Validator, reason: str, **values: Any) -> dict[str, Any]:
    return error(EXPORT_VALIDATOR_MISSING, {"key": KEY, "reason": reason, "path": str(validator.path), **values})


def _text(value: Any) -> str | None:
    return value if isinstance(value, str) else None


def validate(validator: Validator, repo: Path, relative: str, target: Path, name: str) -> list[dict[str, Any]]:
    """The errors of validating the file `relative` (`target`, written for `name`, its table id) in `repo`;
    none when the validator accepts it."""
    command = [
        str(validator.path), "--json", "resource", "validate", "--repo", str(repo), "--changed-path", relative,
    ]
    try:
        exit_code, stdout = validator.run(command, timeout_ms=validator.timeout_ms)
    except subprocess.TimeoutExpired:
        return [_missing(validator, "timeout", timeout_ms=validator.timeout_ms)]
    except OSError as exc:
        return [_missing(validator, "start_failed", exception_type=type(exc).__name__, raw_text=str(exc))]
    text = stdout.decode("utf-8", "replace")
    try:
        answer = json.loads(text)
    except ValueError:
        answer = None
    invalid = _missing(validator, "answer_invalid", exit_code=exit_code, raw_text=text[:RAW_TEXT_MAX])
    if not isinstance(answer, dict):
        return [invalid]
    table = {"path": str(target), "table_id": name}
    if answer.get("ok") is True and exit_code == 0:
        data = answer.get("data")
        entries = data.get("entries") if isinstance(data, dict) else None
        entry = next(
            (item for item in entries if isinstance(item, dict) and item.get("path") == relative), None
        ) if isinstance(entries, list) else None
        status = _text(entry.get("status")) if entry is not None else None
        if status is None:
            return [invalid]
        if status == "valid":
            return []
        detail = {**table, "problem": status}
        reason = _text(entry.get("reason"))
        return [error(EXPORT_TABLE_INVALID, detail if reason is None else {**detail, "raw_text": reason})]
    failure = answer.get("error")
    if answer.get("ok") is not False or exit_code == 0 or not isinstance(failure, dict):
        return [invalid]
    code, message = _text(failure.get("code")), _text(failure.get("message"))
    if code is None or message is None:
        return [invalid]
    details = failure.get("details") if isinstance(failure.get("details"), dict) else {}
    problem, pointer = _text(details.get("standard_pack_problem")), _text(details.get("json_pointer"))
    if problem is None:
        found = PROBLEM_TEXT.search(message)
        problem, pointer = (found.group(1), found.group(2)) if found else (code, None)
    detail = {**table, "problem": problem, "validator_code": code, "raw_text": message[:RAW_TEXT_MAX]}
    return [error(EXPORT_TABLE_INVALID, detail, pointer)]
