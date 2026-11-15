"""The canonical serialization of a pack's JSON file (#466 V-a), to compare with only.

The Runtime's `actinglab package fmt` writes it (its `contracts/package-reference.md`, "The canonical serialization";
`apps/actinglab/src/package_tool.rs`, `canonical_bytes`: serde_json's pretty printer with a two-space indent and a final
line end), and `package fmt --check` (which a resource repository's CI may run) and the `format` step of
`resource verify` refuse a pack file that is not in it. It is UTF-8 without a byte-order mark; two-space indentation, one object member or array element per line, `": "`
after a member name, an empty object or array as `{}` or `[]`; the members in their document order; only what JSON
requires escaped (`"`, `\\` and the control characters, `\\n` and the like as their short escapes, the others as
`\\u00xx` in lower case); every line ending in LF, the last one included; an integer as its digits.

The strict reader (`model.jsonfile`) refuses a byte-order mark, a repeated key, a float and a lone surrogate, so for a
document it reads, Python's `json.dumps(doc, ensure_ascii=False, indent=2)` and a final LF are exactly those bytes.
`bindings check|apply` only compare a file with it (`pack_file_not_canonical`, and the check of the bytes `apply`
makes): the tool never rewrites a file into it, since that would change bytes no binding names.
"""

from __future__ import annotations

import json
from typing import Any


def canonical(doc: Any) -> bytes:
    """The canonical bytes of a document the strict reader read."""
    return (json.dumps(doc, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def is_canonical(raw: bytes, doc: Any) -> bool:
    """Whether `raw`, the bytes the strict reader read as `doc`, are its canonical serialization."""
    return raw == canonical(doc)
