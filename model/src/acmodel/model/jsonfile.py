"""Strict JSON reading (contracts/formats.md, "Reading JSON").

UTF-8 without a BOM; no repeated keys; no float literals; no NaN or Infinity;
integers within +-(2**53 - 1); strings without lone surrogates. Every refusal is
a `model_file_invalid` error whose `ref` is the JSON pointer of the value.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from acmodel.codes import MODEL_FILE_INVALID
from acmodel.envelope import Failure, error

BOM = b"\xef\xbb\xbf"
MAX_SAFE_INTEGER = 2**53 - 1


def pointer(parent: str, token: str | int) -> str:
    """The JSON pointer (RFC 6901) of member or index `token` under `parent`."""
    return parent + "/" + str(token).replace("~", "~0").replace("/", "~1")


def refusal(reason: str, path: Path | str, fmt: str, ref: str, **values: Any) -> dict[str, Any]:
    return error(MODEL_FILE_INVALID, {"reason": reason, "path": str(path), "format": fmt, **values}, ref)


class _Refused:
    """A literal the reader refuses, left in place so that its pointer can be found."""

    __slots__ = ("reason", "literal")

    def __init__(self, reason: str, literal: str) -> None:
        self.reason = reason
        self.literal = literal


class _Repeated(dict):
    """An object that repeated a key. The first value is kept; `repeated` names the repeats."""

    repeated: list[str]


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    doc: dict[str, Any] = {}
    repeated = []
    for key, value in pairs:
        if key in doc:
            repeated.append(key)
        else:
            doc[key] = value
    if not repeated:
        return doc
    marked = _Repeated(doc)
    marked.repeated = repeated
    return marked


def _integer(text: str) -> int | _Refused:
    # 2**53 - 1 has 16 digits; a longer literal is out of range before int() sees it.
    if len(text.lstrip("-")) > 16 or abs(int(text)) > MAX_SAFE_INTEGER:
        return _Refused("integer_out_of_range", text)
    return int(text)


def _float(text: str) -> _Refused:
    return _Refused("float_literal", text)


def _constant(text: str) -> _Refused:
    return _Refused("constant_invalid", text)


def _unicode(text: str) -> bool:
    try:
        text.encode("utf-8")
    except UnicodeEncodeError:  # a lone surrogate from a \\u escape
        return False
    return True


def _scan(doc: Any) -> list[tuple[str, str, dict[str, Any]]]:
    """Every refused literal, repeated key and invalid string, in a fixed order."""
    found: list[tuple[str, str, dict[str, Any]]] = []
    stack: list[tuple[Any, str]] = [(doc, "")]
    while stack:
        value, ref = stack.pop()
        if isinstance(value, _Refused):
            found.append((value.reason, ref, {"literal": value.literal}))
        elif isinstance(value, str):
            if not _unicode(value):
                found.append(("string_invalid", ref, {}))
        elif isinstance(value, dict):
            children = []
            for key, child in value.items():
                at = pointer(ref, key)
                if not _unicode(key):
                    found.append(("string_invalid", at, {}))
                children.append((child, at))
            for key in getattr(value, "repeated", ()):
                found.append(("key_duplicate", pointer(ref, key), {"key": key}))
            stack.extend(reversed(children))
        elif isinstance(value, list):
            stack.extend(reversed([(child, pointer(ref, index)) for index, child in enumerate(value)]))
    return found


def read(path: Path, fmt: str) -> bytes:
    """The bytes of `path`, or `Failure` (`unreadable`)."""
    try:
        return path.read_bytes()
    except OSError as exc:
        detail = {"exception_type": type(exc).__name__, "raw_text": str(exc)}
        raise Failure(refusal("unreadable", path, fmt, "", **detail)) from exc


def load(path: Path, fmt: str) -> Any:
    """The JSON document in `path`, or `Failure` with every problem found."""
    return parse(read(path, fmt), path, fmt)


def parse(raw: bytes, path: Path, fmt: str) -> Any:
    """The JSON document in `raw`, the bytes of `path`, or `Failure` with every problem found."""

    def refused(reason: str, **values: Any) -> Failure:
        return Failure(refusal(reason, path, fmt, "", **values))

    if raw.startswith(BOM):
        raise refused("bom_present")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise refused("utf8_invalid", offset=exc.start, raw_text=exc.reason) from exc
    try:
        doc = json.loads(
            text,
            object_pairs_hook=_object,
            parse_float=_float,
            parse_int=_integer,
            parse_constant=_constant,
        )
    except json.JSONDecodeError as exc:
        raise refused("syntax_invalid", line=exc.lineno, column=exc.colno, raw_text=exc.msg) from exc
    except RecursionError as exc:
        raise refused("syntax_invalid", exception_type=type(exc).__name__) from exc
    found = _scan(doc)
    if found:
        raise Failure(*(refusal(reason, path, fmt, ref, **values) for reason, ref, values in found))
    return doc
