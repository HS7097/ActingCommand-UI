"""Changing a JSON file's bytes where they must change, and only there (#424 design-py 6.3; #464 W1).

`bindings apply` changes a file by edits on its bytes, every other byte kept (line ends, spacing, key order):
- **a replacement**: the bytes of one value, by its pointer (`Layout.replace`);
- **an insertion** (#464 W1): new members right after a member of an object, the anchor (`Layout.insert_after`). Each
  is written as the file writes the anchor: the separator that follows the anchor (its comma with the whitespace before
  and after it, which may hold a line end and an indent, CRLF or LF as the file has it), then the key, the anchor's own
  colon with its whitespace, then the value. An anchor that no member follows has no separator to copy: the insertion
  is refused (None), never guessed.

`expected` gives the document the edits must make: the original document with each changed value and each inserted
member in place (`assign`, `placed`), and `check` parses the edited bytes with the strict reader and compares them with
it, member order included. Bytes that do not read back as that document are a defect (`ValueError`), and then nothing
is written. It reads only the standard library.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from acmodel.bindings.scanner import layout, parent, tokens
from acmodel.envelope import Failure
from acmodel.model import jsonfile

_COLON = re.compile(rb"[ \t\r\n]*:[ \t\r\n]*")
_SEPARATOR = re.compile(rb"[ \t\r\n]*,[ \t\r\n]*")


@dataclass(frozen=True)
class Edit:
    """The bytes `start` to `end` of a file become `data` (an insertion has `start == end`)."""

    start: int
    end: int
    data: bytes


class Layout:
    """Where each value and each key of a file's bytes is (`scanner.layout`)."""

    def __init__(self, raw: bytes) -> None:
        self.raw = raw
        self.values, self.keys = layout(raw)

    def replace(self, ref: str, data: bytes) -> Edit:
        """The value at `ref` written as `data`."""
        start, end = self.values[ref]
        return Edit(start, end, data)

    def insert_after(self, anchor: str, members: list[tuple[str, bytes]]) -> Edit | None:
        """`members` (each a key and its value's bytes), in order, right after the member at the pointer `anchor`, each
        written with the separator that follows the anchor and the anchor's colon. None when no member follows the anchor
        (no separator to copy)."""
        key_end = self.keys[anchor][1]
        value_start, value_end = self.values[anchor]
        colon = self.raw[key_end:value_start]
        if _COLON.fullmatch(colon) is None:  # the scanner read a colon there, so this is a defect
            raise ValueError(f"no colon between the key and the value of {anchor}")
        found = _SEPARATOR.match(self.raw, value_end)
        if found is None or self.raw[found.end():found.end() + 1] != b'"':
            return None
        separator = found.group()
        data = b"".join(separator + json.dumps(key).encode("ascii") + colon + value for key, value in members)
        return Edit(value_end, value_end, data)


def apply(raw: bytes, edits: list[Edit]) -> bytes:
    """`raw` with every edit made. Edits never overlap, and two insertions are never at one place (a defect)."""
    ordered = sorted(edits, key=lambda edit: (edit.start, edit.end))
    for first, second in zip(ordered, ordered[1:]):
        if second.start < first.end or (first.start == second.start and first.start == first.end):
            raise ValueError(f"edits at bytes {first.start} and {second.start} overlap")
    for edit in reversed(ordered):
        raw = raw[:edit.start] + edit.data + raw[edit.end:]
    return raw


def assign(doc: Any, ref: str, value: Any) -> None:
    """The value at the pointer `ref` of `doc`, which names a member or an item that is there, set to `value` in place
    (a member keeps its place among the others)."""
    holder = doc
    for token in tokens(parent(ref)):
        holder = holder[int(token)] if isinstance(holder, list) else holder[token]
    last = tokens(ref)[-1]
    if isinstance(holder, list):
        holder[int(last)] = value
    else:
        holder[last] = value


def placed(holder: dict[str, Any], anchor: str, key: str, value: Any) -> dict[str, Any]:
    """`holder` with the member `key` holding `value`: in its place when it is there, else right after `anchor`."""
    if key in holder:
        return {**holder, key: value}
    found: dict[str, Any] = {}
    for name, member in holder.items():
        found[name] = member
        if name == anchor:
            found[key] = value
    if key not in found:
        raise ValueError(f"no member {anchor!r} to insert {key!r} after")
    return found


def check(raw: bytes, expected: Any, path: Path, fmt: str) -> None:
    """The edited bytes `raw` read back, by the strict reader, as exactly the document `expected`, member order
    included; otherwise a defect (`ValueError`), and the caller writes nothing."""
    try:
        doc = jsonfile.parse(raw, path, fmt)
    except Failure as failure:
        raise ValueError(f"the edited bytes of {path} do not read: {failure}") from None
    if json.dumps(doc) != json.dumps(expected):
        raise ValueError(f"the edited bytes of {path} are not the document the edits make")
