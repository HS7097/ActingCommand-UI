"""One engine worker process (#424 design-py section 7.3; contracts/cli.md, "Engine host").

The host starts it as `<python> -P -s -B <model>/src/acmodel/engine/worker.py`, with an explicit
environment (`PYTHONHASHSEED` among it) and the scratch directory as its working directory. Standard library
only, besides `acmodel.engine.guards` and `.protocol`.

1. **Channel.** The first line on stdin is JSON: the address, family and authkey (hex) of the channel. The
   worker listens there (`multiprocessing.connection.Listener`), writes `READY_LINE` to stdout, then points
   its stdout at its stderr, the run log, so that nothing an engine prints can reach the host's pipes. The
   host connects and authenticates; the frames are pickles of builtin values, results as JSON bytes.
2. **Lifeline.** stdin stays open while the host lives. When it closes, the worker exits, so a worker never
   outlives the tool.
3. **Init.** The `INIT` frame names the engine. The worker adds the venv's site directories, puts the
   manifest's `sys_path` first on `sys.path`, keeps bytecode caches in the scratch directory, installs the
   guards, imports `entry` and calls `make_engine(config, scratch)` and `describe()`. It answers `READY` with
   the description, `IMPORT_FAILED`, `FAILED` (`describe` raised) or `GUARD`.
4. **Calls.** Each `CALL` frame runs one batch: `reset()` first if asked (the verify run), the op, then
   `reset()` again when `engine.reset_each_batch` is on. The answer is `RESULT` (each result as JSON bytes,
   or `NOT_JSON` with where and what; or `{found, count}` when the op did not return a list of one result
   per request), `FAILED` (the exception's type and message; its traceback goes to the log) or `GUARD`
   (every violation, which comes before anything else). After a `MemoryError` the worker answers `FAILED`
   (`worker_memory_limit`) and exits. `STOP` ends it.
"""

from __future__ import annotations

import importlib
import json
import os
import site
import sys
import threading
import traceback
from typing import Any

_SRC = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, _SRC)
from acmodel.engine import guards, protocol  # noqa: E402

sys.path.remove(_SRC)  # engine code never imports the tool

from multiprocessing.connection import Listener  # noqa: E402

EXIT_HOST_GONE = 0
EXIT_MEMORY = 75


def _lifeline() -> None:
    """Exit when the host closes stdin (or is gone)."""
    try:
        sys.stdin.buffer.read()
    finally:
        os._exit(EXIT_HOST_GONE)


def _exception(exc: BaseException, op: str) -> dict[str, Any]:
    traceback.print_exception(exc, file=sys.stderr)
    sys.stderr.flush()
    return {"reason": "exception", "op": op, "exception_type": type(exc).__name__, "raw_text": str(exc)}


class Worker:
    def __init__(self, conn: Any) -> None:
        self.conn = conn
        self.engine: Any = None
        self.guards: guards.Guards | None = None
        self.reset_after = True

    def run(self) -> None:
        tag, setup = self.conn.recv()
        if tag != protocol.INIT:
            raise RuntimeError(f"expected {protocol.INIT}, got {tag!r}")
        if not self.init(setup):
            return
        while True:
            frame = self.conn.recv()
            if frame[0] == protocol.STOP:
                return
            if frame[0] != protocol.CALL:
                raise RuntimeError(f"unknown frame {frame[0]!r}")
            _, op, requests, reset_before = frame
            self.conn.send(self.call(op, json.loads(requests), reset_before))

    def init(self, setup: dict[str, Any]) -> bool:
        for folder in setup["site_dirs"]:
            site.addsitedir(folder)
        sys.path[0:0] = setup["sys_path"]
        sys.pycache_prefix = os.path.join(setup["scratch"], "pycache")
        self.reset_after = setup["reset_each_batch"]
        self.guards = checks = guards.Guards(
            root=setup["root"], scratch=setup["scratch"], code=setup["code"], data=setup["data"],
            manifest=setup["manifest_path"],
        )
        checks.install()
        before = guards.snapshot()
        module_name, _, attribute = setup["entry"].partition(":")
        config = {"root": setup["root"], "manifest": setup["manifest"], "external": setup["external"]}
        failure: dict[str, Any] | None = None
        described: Any = None
        stage = "import"
        try:
            checks.active = True
            factory = getattr(importlib.import_module(module_name), attribute)
            self.engine = factory(config, setup["scratch"])
            checks.stage = stage = "describe"
            described = self.engine.describe()
        except MemoryError:
            checks.active = False
            self.conn.send((protocol.FAILED, {"reason": "worker_memory_limit", "op": "describe"}))
            os._exit(EXIT_MEMORY)
        except BaseException as exc:  # noqa: BLE001 - every exception is reported, none is swallowed
            checks.active = False
            failure = _exception(exc, "describe")
        finally:
            checks.active = False
        checks.check_modules()
        checks.check_state(before)
        violations = checks.take()
        if violations:
            self.conn.send((protocol.GUARD, violations))
            return False
        if failure is not None:
            if stage == "import":
                self.conn.send((protocol.IMPORT_FAILED, failure))
            else:
                self.conn.send((protocol.FAILED, failure))
            return False
        try:
            payload = protocol.dumps(described)
        except protocol.NotJson as exc:
            self.conn.send((protocol.FAILED, {"reason": protocol.NOT_JSON, "op": "describe", "pointer": exc.pointer,
                                              "found": exc.found}))
            return False
        self.conn.send((protocol.READY, payload))
        return True

    def call(self, op: str, requests: list[Any], reset_before: bool) -> tuple[Any, ...]:
        checks = self.guards
        before = guards.snapshot()
        results: Any = None
        failure: dict[str, Any] | None = None
        stage = "reset" if reset_before else "batch"
        try:
            checks.active = True
            if reset_before:
                checks.stage = "reset"
                self.engine.reset()
            checks.stage = stage = "batch"
            if op == "search":
                results = [self.engine.search(request) for request in requests]
            else:
                results = getattr(self.engine, op)(requests)
            checks.active = False
            items = self.encode(results, len(requests))
            if self.reset_after:
                checks.active = True
                checks.stage = stage = "reset"
                self.engine.reset()
        except MemoryError:
            checks.active = False
            self.conn.send((protocol.FAILED, {"reason": "worker_memory_limit", "op": op}))
            os._exit(EXIT_MEMORY)
        except BaseException as exc:  # noqa: BLE001 - every exception is reported, none is swallowed
            checks.active = False
            failure = _exception(exc, "reset" if stage == "reset" else op)
        finally:
            checks.active = False
        checks.check_modules()
        checks.check_state(before)
        violations = checks.take()
        if violations:
            return (protocol.GUARD, violations)
        if failure is not None:
            return (protocol.FAILED, failure)
        return (protocol.RESULT, items)

    @staticmethod
    def encode(results: Any, count: int) -> list[Any] | dict[str, Any]:
        """Each result as JSON bytes, or (NOT_JSON, pointer, found). An answer that is not a list of `count`
        results is one `{"found": <type>, "count": <length or None>}` for the whole batch."""
        if type(results) is not list:
            return {"found": type(results).__name__, "count": None}
        if len(results) != count:
            return {"found": "list", "count": len(results)}
        items: list[Any] = []
        for result in results:
            try:
                items.append(protocol.dumps(result))
            except protocol.NotJson as exc:
                items.append((protocol.NOT_JSON, exc.pointer, exc.found))
        return items


def main() -> int:
    start = json.loads(sys.stdin.buffer.readline())
    listener = Listener(start["address"], start["family"], authkey=bytes.fromhex(start["authkey"]))
    sys.stdout.flush()
    os.write(1, protocol.READY_LINE)
    os.dup2(2, 1)  # from here on, stdout is the run log
    threading.Thread(target=_lifeline, daemon=True).start()
    conn = listener.accept()
    listener.close()
    try:
        Worker(conn).run()
    except EOFError:  # the host closed the channel: nothing is left to answer
        return EXIT_HOST_GONE
    finally:
        conn.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
