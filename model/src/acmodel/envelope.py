"""The `acmodel.cli.v1` envelope: exactly one JSON line on stdout per run.

Human text goes to stderr only (contracts/cli.md).
"""

from __future__ import annotations

import json
from typing import Any, TextIO

from acmodel.codes import EXIT_OK, EXIT_STATUS

SCHEMA_VERSION = "acmodel.cli.v1"


def error(code: str, detail: dict[str, Any] | None = None, ref: str | None = None) -> dict[str, Any]:
    """One `errors` entry. `detail` holds typed values only, never prose."""
    if code not in EXIT_STATUS:
        raise ValueError(f"unregistered code: {code!r}")
    entry: dict[str, Any] = {"code": code, "detail": dict(detail or {})}
    if ref is not None:
        entry["ref"] = ref
    return entry


class Failure(Exception):
    """Registered errors that end the command."""

    def __init__(self, *errors: dict[str, Any]) -> None:
        super().__init__(", ".join(entry["code"] for entry in errors))
        self.errors = list(errors)


def build(command: str | None, data: dict[str, Any], errors: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "command": command,
        "ok": not errors,
        "data": data,
        "errors": errors,
        "warnings": [],
    }


def exit_status(envelope: dict[str, Any]) -> int:
    """The exit status of the first error's code, or 0."""
    errors = envelope["errors"]
    return EXIT_STATUS[errors[0]["code"]] if errors else EXIT_OK


def write(envelope: dict[str, Any], stdout: TextIO, stderr: TextIO) -> int:
    """Print one line per error to stderr, then the envelope to stdout; return the exit status."""
    for entry in envelope["errors"]:
        values = " ".join(f"{key}={json.dumps(value)}" for key, value in entry["detail"].items())
        ref = f" ref={entry['ref']}" if "ref" in entry else ""
        stderr.write(f"acmodel: {entry['code']} {values}{ref}\n")
    stderr.flush()
    # ASCII only, so the line survives any console or pipe encoding.
    stdout.write(json.dumps(envelope, ensure_ascii=True, separators=(",", ":")) + "\n")
    stdout.flush()
    return exit_status(envelope)
