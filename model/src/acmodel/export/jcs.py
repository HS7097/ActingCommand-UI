"""The JCS writer of the delivered tables (#424 design-py 6.1): canonical JSON with the Runtime's rules.

- **Keys** are sorted by their UTF-16 code units (`key.encode("utf-16-be")`), as the Runtime's canonical JSON
  sorts them. Python's own order is by code point, and the two differ for characters above U+FFFF.
- **Numbers** are integers in the ECMAScript safe range, ±(2**53 − 1). A float is never written, whatever its
  value; neither is an integer outside the range, nor any other type.
- **No insignificant whitespace**; arrays keep their order.
- **Strings** have the minimal escapes: `"` and `\\`, the short forms `\\b`, `\\t`, `\\n`, `\\f` and `\\r`, and
  `\\u00xx` (lowercase hex) for the other control characters below U+0020. Everything else is written as it is,
  in UTF-8. A lone surrogate cannot be written.

`dumps` refuses what it cannot write with `NotCanonical`, which names the JSON pointer of the value. The exports
check every value before they write it, so a refusal here is a defect, never a silent change of the value.
"""

from __future__ import annotations

from typing import Any

from acmodel.model.jsonfile import MAX_SAFE_INTEGER, pointer

_SHORT = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\t": "\\t", "\n": "\\n", "\f": "\\f", "\r": "\\r"}


class NotCanonical(ValueError):
    """A value canonical JSON cannot hold: `reason` and the JSON pointer of the value (`ref`)."""

    def __init__(self, reason: str, ref: str) -> None:
        super().__init__(f"{reason} at {ref!r}")
        self.reason = reason
        self.ref = ref


def _string(text: str, ref: str) -> str:
    out = []
    for char in text:
        if char in _SHORT:
            out.append(_SHORT[char])
        elif char < " ":
            out.append(f"\\u{ord(char):04x}")
        elif "\ud800" <= char <= "\udfff":
            raise NotCanonical("string_invalid", ref)
        else:
            out.append(char)
    return '"' + "".join(out) + '"'


def _write(value: Any, ref: str, out: list[str]) -> None:
    if value is None:
        out.append("null")
    elif value is True or value is False:
        out.append("true" if value else "false")
    elif type(value) is int:
        if abs(value) > MAX_SAFE_INTEGER:
            raise NotCanonical("integer_out_of_range", ref)
        out.append(str(value))
    elif type(value) is str:
        out.append(_string(value, ref))
    elif type(value) is list:
        out.append("[")
        for index, item in enumerate(value):
            if index:
                out.append(",")
            _write(item, pointer(ref, index), out)
        out.append("]")
    elif type(value) is dict:
        for key in value:
            if type(key) is not str:
                raise NotCanonical("key_invalid", ref)
        out.append("{")
        for index, key in enumerate(sorted(value, key=lambda key: key.encode("utf-16-be", "surrogatepass"))):
            if index:
                out.append(",")
            out.append(_string(key, pointer(ref, key)))
            out.append(":")
            _write(value[key], pointer(ref, key), out)
        out.append("}")
    elif type(value) is float:
        raise NotCanonical("float", ref)
    else:
        raise NotCanonical("type_invalid", ref)


def dumps(value: Any) -> bytes:
    """The canonical bytes of `value`, UTF-8 with no final newline; or `NotCanonical`."""
    out: list[str] = []
    _write(value, "", out)
    return "".join(out).encode("utf-8")
