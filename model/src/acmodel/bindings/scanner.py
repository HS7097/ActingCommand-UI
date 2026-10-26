"""JSON pointers and a position-tracking scanner (#424 design-py 6.3).

`spans(raw)` gives the byte span of every value of a JSON document by its JSON pointer, so that
`bindings apply` replaces exactly a literal's bytes: line ends (CRLF files stay CRLF), spacing, key order and
every other byte stay as they are. It reads only bytes the strict reader (`model.jsonfile.parse`) has already
accepted, so it trusts the grammar; a document it cannot follow is a defect, raised as `ValueError`.
"""

from __future__ import annotations

import json
import re
from typing import Any

from acmodel.model.jsonfile import pointer

MISSING = object()  # what `resolve` gives for a pointer that names nothing

_INDEX = re.compile(r"0|[1-9][0-9]*")
_SPACE = re.compile(rb"[ \t\n\r]*")
_STRING = re.compile(rb'"(?:[^"\\]|\\.)*"', re.DOTALL)
_SCALAR = re.compile(rb"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?|true|false|null")


def tokens(ref: str) -> list[str]:
    """The reference tokens of a JSON pointer (RFC 6901), unescaped."""
    return [token.replace("~1", "/").replace("~0", "~") for token in ref.split("/")[1:]]


def parent(ref: str, levels: int = 1) -> str:
    """The pointer `levels` members above `ref`."""
    return "/".join(ref.split("/")[:-levels])


def resolve(doc: Any, ref: str) -> Any:
    """The value at `ref` in `doc`, or `MISSING`. An array index is `0` or has no leading zero."""
    value = doc
    for token in tokens(ref):
        if isinstance(value, dict) and token in value:
            value = value[token]
        elif isinstance(value, list) and _INDEX.fullmatch(token) and int(token) < len(value):
            value = value[int(token)]
        else:
            return MISSING
    return value


class _Scanner:
    def __init__(self, raw: bytes) -> None:
        self.raw = raw
        self.pos = 0
        self.spans: dict[str, tuple[int, int]] = {}

    def skip(self) -> None:
        self.pos = _SPACE.match(self.raw, self.pos).end()

    def peek(self) -> bytes:
        return self.raw[self.pos:self.pos + 1]

    def step(self, char: bytes) -> None:
        if self.peek() != char:
            raise ValueError(f"expected {char!r} at byte {self.pos}")
        self.pos += 1
        self.skip()

    def match(self, pattern: re.Pattern[bytes]) -> re.Match[bytes]:
        found = pattern.match(self.raw, self.pos)
        if found is None:
            raise ValueError(f"no JSON value at byte {self.pos}")
        self.pos = found.end()
        self.skip()
        return found

    def member(self, frame: list[Any]) -> str:
        """The pointer of the next member of the open container `frame`; an object's key is read first."""
        if frame[2] == b"]":
            return pointer(frame[0], frame[3])
        key = json.loads(self.match(_STRING).group().decode("utf-8"))
        self.step(b":")
        return pointer(frame[0], key)


def spans(raw: bytes) -> dict[str, tuple[int, int]]:
    """The byte span `(start, end)` of every value of the JSON document `raw`, by its JSON pointer (the empty
    pointer for the whole document). It works without recursion, so depth is no limit."""
    scan = _Scanner(raw)
    scan.skip()
    stack: list[list[Any]] = []  # each open container: [its pointer, its first byte, its closing byte, members read]
    ref = ""
    while True:
        char = scan.peek()
        if char in (b"{", b"["):
            frame = [ref, scan.pos, b"}" if char == b"{" else b"]", 0]
            stack.append(frame)
            scan.step(char)
            if scan.peek() != frame[2]:
                ref = scan.member(frame)
                continue
        else:
            found = scan.match(_STRING if char == b'"' else _SCALAR)
            scan.spans[ref] = found.span()
        while stack:  # the value is complete: go on to the next member, or close containers
            frame = stack[-1]
            if scan.peek() == b",":
                scan.step(b",")
                frame[3] += 1
                ref = scan.member(frame)
                break
            end = scan.pos + 1
            scan.step(frame[2])
            scan.spans[frame[0]] = (frame[1], end)
            stack.pop()
        else:
            if scan.pos != len(raw):
                raise ValueError(f"bytes after the document at byte {scan.pos}")
            return scan.spans
