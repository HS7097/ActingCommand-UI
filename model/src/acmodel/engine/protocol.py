"""The engine interface `acmodel.engine.v2`: what may cross the channel, and the shapes of the results
(contracts/formats.md, "Engines").

The worker uses it to check that an engine's results are JSON values before it sends them, and the host
to check their shapes. Standard library only, because the worker imports it.

- **JSON values** are exactly dict (string keys), list, str (valid Unicode), int, finite float, bool and
  None. Anything else, a tuple, a set, a subclass or NaN among them, is not one.
- **A request** is a JSON object. Its optional member `entities` maps opaque entity ids to what the engine
  may read about them; every other member is read whole.
- **A result.** `evaluate` gives one result per request: `metrics` (name to number) and `depends_on` (the
  ids of the request's `entities` the result read), and optionally `flags`, `unmodelled` and
  `pending_readings_used` (lists of strings). A `predict` or `search` result is an object with
  `depends_on`; its other members are the engine's.
- **A domain failure** of one request is the result `{"error": {"code": <string>, "detail": <JSON value>}}`.
- **`describe`** gives `engine_id`, `version`, `ops`, `metrics`, `entity_classes` and `max_batch`.
"""

from __future__ import annotations

import json
import math
from typing import Any

INTERFACE = "acmodel.engine.v2"
OPS = ("describe", "evaluate", "predict", "search")
CALLS = ("evaluate", "predict", "search")  # the ops a call names; `search` takes one request at a time
ENTITIES = "entities"
EVALUATE_LISTS = ("flags", "unmodelled", "pending_readings_used")
DESCRIBE = ("engine_id", "version", "ops", "metrics", "entity_classes", "max_batch")

# The frames between the host and a worker: tuples of a tag and builtin values, results as JSON bytes.
READY_LINE = b"acmodel-worker-ready\n"  # on the worker's stdout once its listener is up
INIT, READY, IMPORT_FAILED = "init", "ready", "import_failed"
CALL, RESULT, FAILED, GUARD, STOP = "call", "result", "failed", "guard", "stop"
NOT_JSON = "not_json"  # an item of a RESULT frame whose value is not a JSON value


def pointer(parent: str, token: str | int) -> str:
    """The JSON pointer (RFC 6901) of member or index `token` under `parent`."""
    return parent + "/" + str(token).replace("~", "~0").replace("/", "~1")


class NotJson(Exception):
    """A value that is not a JSON value, at `pointer`; `found` names what it is."""

    def __init__(self, at: str, found: str) -> None:
        super().__init__(at, found)
        self.pointer = at
        self.found = found


def check_json(value: Any, at: str = "") -> None:
    """Raise `NotJson` at the first value (in sorted key order) that is not a JSON value."""
    kind = type(value)
    if value is None or kind is bool or kind is int:
        return
    if kind is str:
        try:
            value.encode("utf-8")
        except UnicodeEncodeError:  # a lone surrogate
            raise NotJson(at, "string_invalid") from None
        return
    if kind is float:
        if not math.isfinite(value):
            raise NotJson(at, "float_not_finite")
        return
    if kind is list:
        for index, item in enumerate(value):
            check_json(item, pointer(at, index))
        return
    if kind is dict:
        for key in value:
            if type(key) is not str:
                raise NotJson(pointer(at, str(key)), type(key).__name__ + "_key")
        for key in sorted(value):
            check_json(key, pointer(at, key))
            check_json(value[key], pointer(at, key))
        return
    raise NotJson(at, kind.__name__)


def dumps(value: Any) -> bytes:
    """A JSON value as UTF-8 JSON text; `NotJson` if it is not one."""
    check_json(value)
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")


class ShapeError(Exception):
    """A JSON value of the wrong shape, at `pointer`; `expected` says what belongs there."""

    def __init__(self, at: str, expected: list[str], found: str | None = None) -> None:
        super().__init__(at, expected, found)
        self.pointer = at
        self.expected = expected
        self.found = found


def _strings(value: Any, at: str, *, distinct: bool = False) -> list[str]:
    if type(value) is not list or any(type(item) is not str for item in value):
        raise ShapeError(at, ["array of strings"], _type(value))
    if distinct and len(set(value)) != len(value):
        raise ShapeError(at, ["distinct strings"])
    return value


def _type(value: Any) -> str:
    if value is None:
        return "null"
    return {bool: "boolean", int: "integer", float: "number", str: "string", list: "array", dict: "object"}.get(
        type(value), type(value).__name__
    )


def check_request(request: Any) -> None:
    """A request the core sends: a JSON object whose `entities`, if present, is an object."""
    if type(request) is not dict:
        raise ShapeError("", ["object"], _type(request))
    check_json(request)
    if ENTITIES in request and type(request[ENTITIES]) is not dict:
        raise ShapeError(pointer("", ENTITIES), ["object"], _type(request[ENTITIES]))


def check_result(op: str, value: Any, request: dict[str, Any]) -> dict[str, Any] | None:
    """The domain failure `{code, detail}` of a result, or None for a result of the right shape;
    `ShapeError` otherwise. `value` is a JSON value."""
    if type(value) is not dict:
        raise ShapeError("", ["object"], _type(value))
    if "error" in value:
        extra = sorted(key for key in value if key != "error")
        if extra:
            raise ShapeError(pointer("", extra[0]), ["error"])
        failure = value["error"]
        if type(failure) is not dict or sorted(failure) != ["code", "detail"]:
            raise ShapeError("/error", ["code", "detail"], _type(failure))
        if type(failure["code"]) is not str or not failure["code"]:
            raise ShapeError("/error/code", ["string"], _type(failure["code"]))
        return failure
    if "depends_on" not in value:
        raise ShapeError("/depends_on", ["depends_on"])
    entities = request.get(ENTITIES, {})
    for index, entity in enumerate(_strings(value["depends_on"], "/depends_on", distinct=True)):
        if entity not in entities:
            raise ShapeError(pointer("/depends_on", index), ["an id of the request's entities"])
    if op != "evaluate":
        return None
    allowed = ["depends_on", "metrics", *EVALUATE_LISTS]
    for key in sorted(value):
        if key not in allowed:
            raise ShapeError(pointer("", key), sorted(allowed))
    metrics = value.get("metrics")
    if type(metrics) is not dict:
        raise ShapeError("/metrics", ["object"], _type(metrics))
    for name in sorted(metrics):
        if type(metrics[name]) not in (int, float):
            raise ShapeError(pointer("/metrics", name), ["number"], _type(metrics[name]))
    for key in EVALUATE_LISTS:
        if key in value:
            _strings(value[key], pointer("", key))
    return None


def check_describe(value: Any, manifest: dict[str, Any]) -> None:
    """`describe()`'s answer agrees with the manifest: `engine_id`, `version`, `max_batch` and the set of
    `ops` equal; `metrics` and `entity_classes` are lists of strings. `ShapeError` otherwise."""
    if type(value) is not dict:
        raise ShapeError("", ["object"], _type(value))
    for key in sorted(value):
        if key not in DESCRIBE:
            raise ShapeError(pointer("", key), list(DESCRIBE))
    for key in DESCRIBE:
        if key not in value:
            raise ShapeError(pointer("", key), [key])
    for key in ("engine_id", "version", "max_batch"):
        if type(value[key]) is not type(manifest[key]) or value[key] != manifest[key]:
            raise ShapeError(pointer("", key), [str(manifest[key])], _type(value[key]))
    ops = _strings(value["ops"], "/ops", distinct=True)
    if sorted(ops) != sorted(manifest["ops"]):
        raise ShapeError("/ops", sorted(manifest["ops"]))
    for key in ("metrics", "entity_classes"):
        _strings(value[key], pointer("", key))
