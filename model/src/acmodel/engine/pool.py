"""The engine host's workers (#424 design-py section 7.3; contracts/cli.md, "Engine host").

**Start.** Each worker is `<python> -P -s -B <model>/src/acmodel/engine/worker.py` with an explicit
environment that inherits nothing: `PYTHONHASHSEED` (`engine.hash_seed`, or `engine.verify_hash_seed` for
the verify worker), the three BLAS variables, `ACMODEL_SCRATCH`, `TEMP`, `TMP` and `TMPDIR` inside the
scratch directory, and `SYSTEMROOT` where the host has it. Without `-I` and `-E` the seed is honoured;
because the environment is explicit, no other `PYTHON*` variable enters. The working directory is the
worker's scratch directory, `<scratch root>/<engine id>/<slot>`; its stdout and stderr go to its log,
`<log dir>/<engine id>-<slot>.log`.
- The interpreter is the base interpreter of the running one: in a venv on Windows, `Scripts\\python.exe`
  is a launcher that runs the real interpreter as its child, so its process would not be the worker's
  (its id, its memory, its handle). The worker adds the venv's site directories itself.
- Before the worker runs any engine code, `os_layer.verify_child(pid)` checks that it is in the tool's Job
  and below normal (PY-7); the tool joined the Job before, so its workers are inside it from their start.

**Channel.** The worker listens on a fresh address with a fresh random authkey, both handed over on its
stdin (a named pipe on Windows, an abstract Unix socket on Linux); the host connects to it
(`multiprocessing.connection`). stdin stays open as the worker's lifeline. The start and every batch have
a deadline (`engine.start_timeout_ms`; `engine.request_timeout_ms` per request of the batch).

**Batches.** A call's requests go in batches of the manifest's `max_batch` (`search`: one at a time) to
at most `compute.workers.max` workers, the verify worker included. Results come back in request order.

**Outcomes.** Per request: a result, or an error entry. An adapter's domain failure is that request's
`engine_failed` (`adapter_error`, with its code). An exception, a result that is not JSON or not of its
shape, a timeout or the memory limit fail every request of the batch, and the call goes on. A guard
violation, a failed import, a `describe` that disagrees with the manifest and an exhausted restart budget
end the call (`Failure`); every worker is then stopped.

**Restarts and recycling.** A worker that exits without answering (a crash) is replaced and its batch run
again; so is one that timed out or hit the memory limit, without running the batch again. Each counts
against `engine.restart_budget` for the life of the pool; past it, `engine_failed`
(`restart_budget_exhausted`). A worker is recycled (stopped, and a new one started when needed; no budget)
after `engine.worker_max_requests` requests (0: never), when its private bytes exceed
`engine.worker_soft_memory_mib` after a batch, and after an exception.

**Verify run** (`verify`): the given requests run again one by one, each after `reset()`, in a fresh worker
with `engine.verify_hash_seed`; any difference from the expected result is `engine_nondeterministic`.

The pool stops only workers it started, by their process handle.
"""

from __future__ import annotations

import collections
import itertools
import json
import os
import secrets
import site
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from multiprocessing.connection import Client, wait
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Protocol

from acmodel.cache.keys import canonical, canonical_text
from acmodel.codes import (
    ENGINE_FAILED,
    ENGINE_GUARD_VIOLATION,
    ENGINE_IMPORT_FAILED,
    ENGINE_NONDETERMINISTIC,
    ENGINE_PROTOCOL_ERROR,
)
from acmodel.engine import protocol
from acmodel.engine.manifest import Engine
from acmodel.engine.settings import EngineSettings
from acmodel.envelope import Failure, error

WORKER = Path(__file__).resolve().with_name("worker.py")
BLAS_VARIABLES = ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "MKL_NUM_THREADS")
STARTING, IDLE, BUSY = "starting", "idle", "busy"
VERIFY_SLOT = "v"


class WorkerOs(Protocol):
    """What the pool needs of the host: the real one is `host.windows.WindowsHost` after `apply`."""

    def verify_child(self, pid: int) -> None:
        """Raise `Failure` unless process `pid` is in the tool's Job and at its priority class."""

    def private_mib(self, pid: int) -> int:
        """The private bytes of process `pid`, in MiB."""


@dataclass
class Outcome:
    """One request's outcome: its result (canonical JSON order), or its error entry."""

    result: dict[str, Any] | None = None
    error: dict[str, Any] | None = None


def worker_python() -> tuple[str, list[str]]:
    """The interpreter the workers run, and the venv's site directories they add (none outside a venv)."""
    base = getattr(sys, "_base_executable", None) or sys.executable
    if sys.prefix == sys.base_prefix:
        return base, []
    return base, [folder for folder in site.getsitepackages() if Path(folder).name == "site-packages"]


@dataclass(eq=False)
class _Worker:
    slot: str
    proc: subprocess.Popen
    log: Any
    conn: Any = None
    state: str = STARTING
    batch: int | None = None
    deadline: float = 0.0
    served: int = 0


@dataclass
class _Work:
    op: str
    requests: list[dict[str, Any]]
    indices: list[int]  # each request's index as the caller counts them, for the error entries
    batches: list[list[int]]  # positions in `requests`
    reset_before: bool
    outcomes: list[Outcome | None] = field(default_factory=list)


def difference(expected: Any, found: Any, at: str = "") -> str:
    """The JSON pointer of the first place where two JSON values differ (keys in sorted order)."""
    if type(expected) is not type(found):
        return at
    if type(expected) is dict:
        for key in sorted(set(expected) | set(found)):
            if key not in expected or key not in found:
                return protocol.pointer(at, key)
            if canonical(expected[key]) != canonical(found[key]):
                return difference(expected[key], found[key], protocol.pointer(at, key))
        return at
    if type(expected) is list:
        for index, (left, right) in enumerate(zip(expected, found)):
            if canonical(left) != canonical(right):
                return difference(left, right, protocol.pointer(at, index))
        return protocol.pointer(at, min(len(expected), len(found)))
    return at


def _close(handle: Any) -> None:
    """Close a pipe or file of a worker being stopped. A pipe to a process that is gone may refuse to close
    (its last bytes cannot be delivered); nothing is left to deliver, so that is not a failure."""
    if handle is None:
        return
    try:
        handle.close()
    except OSError:
        return


class Pool:
    def __init__(
        self,
        engine: Engine,
        settings: EngineSettings,
        *,
        scratch_root: Path,
        log_dir: Path,
        os_layer: WorkerOs,
        blas_threads: int,
        python: str | None = None,
        site_dirs: list[str] | None = None,
        environ: dict[str, str] | None = None,
        launch: Callable[..., subprocess.Popen] = subprocess.Popen,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        base, sites = worker_python()
        self.engine = engine
        self.settings = settings
        self._scratch = Path(scratch_root) / engine.engine_id
        self._logs = Path(log_dir)
        self._os = os_layer
        self._blas = str(blas_threads)
        self._python = python or base
        self._sites = list(sites if site_dirs is None else site_dirs)
        self._environ = dict(os.environ if environ is None else environ)
        self._launch = launch
        self._clock = clock
        self._main: list[_Worker] = []
        self._verify: list[_Worker] = []
        self.counters = {
            "started": 0, "restarts": 0, "recycled": 0, "batches": 0, "requests": 0, "peak_workers": 0,
            "peak_in_flight": 0,
        }

    def __enter__(self) -> Pool:
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def pids(self) -> list[int]:
        """The process ids of the workers alive now."""
        return [worker.proc.pid for worker in self._main + self._verify]

    def log_path(self, slot: str) -> Path:
        return self._logs / f"{self.engine.engine_id}-{slot}.log"

    def close(self) -> None:
        """Stop every worker."""
        for group in (self._main, self._verify):
            for worker in list(group):
                self._stop(worker, group)

    def warm(self, count: int) -> int:
        """Start workers ahead of the calls until `count` are alive (never more than `compute.workers.max`), and
        wait until each is ready: the engine imported, `make_engine` and `describe` done, so that the calls find
        its context built (`watch`, `watch.keep_workers_warm`). Returns how many it started. A failure stops every
        worker and raises, as a call's does; a worker that crashes while starting uses the restart budget and is
        not replaced."""
        target = min(count, self.settings.workers_max)
        work = _Work(op="start", requests=[], indices=[], batches=[], reset_before=False)
        started = 0
        try:
            while len(self._main) < target and self._live() < self.settings.workers_max:
                self._start(self._main, self.settings.hash_seed)
                started += 1
            self._run(work, self._main, self.settings.hash_seed, target)
        except BaseException:
            self.close()
            raise
        return started

    # Calls ------------------------------------------------------------------------------------------

    def call(self, op: str, requests: list[dict[str, Any]], indices: list[int] | None = None) -> list[Outcome]:
        """Each request's outcome, in order. `indices` number the requests in the error entries (default:
        their positions)."""
        work = self._work(op, requests, indices, reset_before=False)
        try:
            self._run(work, self._main, self.settings.hash_seed, self.settings.workers_max)
        except BaseException:
            self.close()
            raise
        return work.outcomes

    def verify(
        self, op: str, requests: list[dict[str, Any]], expected: list[dict[str, Any]], indices: list[int] | None = None
    ) -> list[dict[str, Any]]:
        """Run each request again, after `reset()`, in a fresh worker with `engine.verify_hash_seed`. Return
        one `engine_nondeterministic` entry per request whose outcome is not its expected result."""
        work = self._work(op, requests, indices, reset_before=True, batch=1)
        if not requests:
            return []
        while self._main and self._live() >= self.settings.workers_max:
            self._stop(self._main[-1], self._main)  # every main worker is idle between calls
        try:
            self._run(work, self._verify, self.settings.verify_hash_seed, 1)
        except BaseException:
            self.close()
            raise
        finally:
            for worker in list(self._verify):
                self._stop(worker, self._verify)
        problems = []
        for position, outcome in enumerate(work.outcomes):
            if outcome.error is None and canonical(outcome.result) == canonical(expected[position]):
                continue
            ref = "" if outcome.error is not None else difference(expected[position], outcome.result)
            problems.append(error(ENGINE_NONDETERMINISTIC, {
                "engine_id": self.engine.engine_id,
                "op": op,
                "request": work.indices[position],
                "hash_seed": self.settings.hash_seed,
                "verify_hash_seed": self.settings.verify_hash_seed,
            }, ref))
        return problems

    def _work(
        self, op: str, requests: list[dict[str, Any]], indices: list[int] | None, *, reset_before: bool,
        batch: int | None = None,
    ) -> _Work:
        if op not in protocol.CALLS or op not in self.engine.ops:
            raise ValueError(f"{op!r} is not a call op of engine {self.engine.engine_id!r}")
        for request in requests:  # what the core sends is its own: a wrong one is a defect, not an input
            try:
                protocol.check_request(request)
            except (protocol.ShapeError, protocol.NotJson) as exc:
                raise ValueError(f"an engine request that is not a JSON object with object entities: {exc}") from exc
        size = batch or (1 if op == "search" else self.engine.max_batch)
        count = len(requests)
        return _Work(
            op=op,
            requests=list(requests),
            indices=list(range(count)) if indices is None else list(indices),
            batches=[list(range(start, min(start + size, count))) for start in range(0, count, size)],
            reset_before=reset_before,
            outcomes=[None] * count,
        )

    # The dispatch loop --------------------------------------------------------------------------------

    def _live(self) -> int:
        return len(self._main) + len(self._verify)

    def _run(self, work: _Work, group: list[_Worker], seed: int, limit: int) -> None:
        pending = collections.deque(range(len(work.batches)))
        while pending or any(worker.state != IDLE for worker in group):
            idle = [worker for worker in group if worker.state == IDLE]
            starting = sum(worker.state == STARTING for worker in group)
            while len(pending) > len(idle) + starting and len(group) < limit and self._live() < self.settings.workers_max:
                self._start(group, seed)
                starting += 1
            for worker in idle:
                if not pending:
                    break
                self._send(worker, group, work, pending)
            waiting = [worker for worker in group if worker.state != IDLE]
            if not waiting:
                if pending:  # every slot is taken by another group: a defect of the caller
                    raise RuntimeError("no engine worker can be started for the pending batches")
                continue
            deadline = min(worker.deadline for worker in waiting)
            ready = wait([worker.conn for worker in waiting], max(0.0, deadline - self._clock()))
            if not ready:
                now = self._clock()
                for worker in waiting:
                    if now >= worker.deadline:
                        self._timed_out(worker, group, work)
                continue
            by_conn = {worker.conn: worker for worker in waiting}
            for conn in ready:
                self._receive(by_conn[conn], group, work, pending)

    def _send(self, worker: _Worker, group: list[_Worker], work: _Work, pending: collections.deque[int]) -> None:
        batch = pending.popleft()
        positions = work.batches[batch]
        payload = protocol.dumps([work.requests[position] for position in positions])
        worker.state, worker.batch = BUSY, batch
        worker.deadline = self._clock() + self.settings.request_timeout_ms * len(positions) / 1000
        self.counters["peak_in_flight"] = max(
            self.counters["peak_in_flight"], sum(w.state == BUSY for w in self._main + self._verify)
        )
        try:
            worker.conn.send((protocol.CALL, work.op, payload, work.reset_before))
        except (OSError, EOFError):  # the worker is gone: a crash
            self._crashed(worker, group, work, pending)

    def _receive(self, worker: _Worker, group: list[_Worker], work: _Work, pending: collections.deque[int]) -> None:
        op = work.op if worker.state == BUSY else "start"
        try:
            frame = worker.conn.recv()
        except (EOFError, OSError):  # the worker exited without answering
            self._crashed(worker, group, work, pending)
            return
        except Exception as exc:  # noqa: BLE001 - a frame that does not unpickle: reported, never skipped
            raise Failure(self._entry(ENGINE_PROTOCOL_ERROR, op, reason="frame_invalid", found=type(exc).__name__)) from exc
        if type(frame) is not tuple or len(frame) != 2:
            raise Failure(self._entry(ENGINE_PROTOCOL_ERROR, op, reason="frame_invalid", found=type(frame).__name__))
        tag, body = frame
        if tag == protocol.GUARD:
            raise Failure(*(error(ENGINE_GUARD_VIOLATION, {"engine_id": self.engine.engine_id, **v}) for v in body))
        if worker.state == STARTING:
            self._started(worker, tag, body)
        else:
            self._answered(worker, group, work, tag, body)

    def _started(self, worker: _Worker, tag: str, body: Any) -> None:
        if tag == protocol.READY:
            try:
                protocol.check_describe(json.loads(body), self.engine.doc)
            except protocol.ShapeError as exc:
                raise Failure(self._entry(
                    ENGINE_PROTOCOL_ERROR, "describe", ref=exc.pointer, reason="describe_invalid",
                    expected=exc.expected, found=exc.found,
                )) from None
            worker.state = IDLE
            return
        if tag == protocol.IMPORT_FAILED:
            raise Failure(error(ENGINE_IMPORT_FAILED, {
                "engine_id": self.engine.engine_id, "entry": self.engine.doc["entry"],
                "exception_type": body["exception_type"], "raw_text": body["raw_text"],
            }))
        if tag == protocol.FAILED:
            raise Failure(self._failed(body))
        raise Failure(self._entry(ENGINE_PROTOCOL_ERROR, "start", reason="frame_invalid", found=str(tag)))

    def _answered(self, worker: _Worker, group: list[_Worker], work: _Work, tag: str, body: Any) -> None:
        positions = work.batches[worker.batch]
        worker.state, worker.batch = IDLE, None
        if tag == protocol.FAILED:
            for position in positions:
                work.outcomes[position] = Outcome(error=self._failed(body, work.indices[position]))
            if body["reason"] == "worker_memory_limit":  # the worker exits after answering
                self._restarted(work.op, self._reap(worker, group))
            else:  # its state after an exception is unknown: it is not used again
                self._stop(worker, group)
                self.counters["recycled"] += 1
            return
        if tag != protocol.RESULT:
            raise Failure(self._entry(ENGINE_PROTOCOL_ERROR, work.op, reason="frame_invalid", found=str(tag)))
        for index, position in enumerate(positions):
            if type(body) is dict:  # not a list of one result per request
                found = body["found"] if body["count"] is None else f"{body['count']}_results"
                entry = self._entry(
                    ENGINE_PROTOCOL_ERROR, work.op, request=work.indices[position], reason="result_count",
                    expected=[f"{len(positions)}_results"], found=found,
                )
                work.outcomes[position] = Outcome(error=entry)
            else:
                work.outcomes[position] = self._outcome(work, position, body[index])
        worker.served += len(positions)
        self.counters["batches"] += 1
        self.counters["requests"] += len(positions)
        limit = self.settings.worker_max_requests
        if (limit and worker.served >= limit) or self._os.private_mib(worker.proc.pid) > self.settings.worker_soft_memory_mib:
            self._stop(worker, group)
            self.counters["recycled"] += 1

    def _outcome(self, work: _Work, position: int, item: Any) -> Outcome:
        index = work.indices[position]
        if type(item) is tuple:  # (NOT_JSON, pointer, found)
            return Outcome(error=self._entry(
                ENGINE_PROTOCOL_ERROR, work.op, ref=item[1], request=index, reason="not_json", found=item[2]
            ))
        value = json.loads(item)
        try:
            failure = protocol.check_result(work.op, value, work.requests[position])
        except protocol.ShapeError as exc:
            return Outcome(error=self._entry(
                ENGINE_PROTOCOL_ERROR, work.op, ref=exc.pointer, request=index, reason="result_invalid",
                expected=exc.expected, found=exc.found,
            ))
        if failure is not None:
            return Outcome(error=self._entry(
                ENGINE_FAILED, work.op, request=index, reason="adapter_error", engine_code=failure["code"],
                engine_detail=canonical_text(failure["detail"]),
            ))
        return Outcome(result=json.loads(canonical(value)))

    # Failures ---------------------------------------------------------------------------------------

    def _entry(self, code: str, op: str, *, ref: str | None = None, **values: Any) -> dict[str, Any]:
        detail = {"engine_id": self.engine.engine_id, "op": op}
        detail.update((key, value) for key, value in values.items() if value is not None)
        return error(code, detail, ref)

    def _failed(self, body: dict[str, Any], request: int | None = None) -> dict[str, Any]:
        """The entry of a FAILED frame: an exception, the memory limit, or a description that is not JSON."""
        if body["reason"] == protocol.NOT_JSON:
            return self._entry(
                ENGINE_PROTOCOL_ERROR, body["op"], ref=body["pointer"], request=request, reason="not_json",
                found=body["found"],
            )
        return self._entry(
            ENGINE_FAILED, body["op"], request=request, reason=body["reason"],
            exception_type=body.get("exception_type"), raw_text=body.get("raw_text"),
        )

    def _restarted(self, op: str, exit_code: int | None) -> None:
        """Count one replacement of a worker that ended abnormally; past the budget, the call fails."""
        if self.counters["restarts"] >= self.settings.restart_budget:
            raise Failure(self._entry(
                ENGINE_FAILED, op, reason="restart_budget_exhausted", exit_code=exit_code,
                restarts=self.counters["restarts"], budget=self.settings.restart_budget,
            ))
        self.counters["restarts"] += 1

    def _crashed(self, worker: _Worker, group: list[_Worker], work: _Work, pending: collections.deque[int]) -> None:
        busy, batch = worker.state == BUSY, worker.batch
        self._restarted(work.op if busy else "start", self._reap(worker, group))
        if busy:
            pending.appendleft(batch)  # run again on a new worker

    def _timed_out(self, worker: _Worker, group: list[_Worker], work: _Work) -> None:
        state, batch = worker.state, worker.batch
        self._kill(worker, group)
        if state == STARTING:
            raise Failure(self._entry(
                ENGINE_FAILED, "start", reason="timeout", timeout_ms=self.settings.start_timeout_ms
            ))
        positions = work.batches[batch]
        for position in positions:
            work.outcomes[position] = Outcome(error=self._entry(
                ENGINE_FAILED, work.op, request=work.indices[position], reason="timeout",
                timeout_ms=self.settings.request_timeout_ms * len(positions),
            ))
        self._restarted(work.op, None)

    # Starting and stopping ----------------------------------------------------------------------------

    def _start(self, group: list[_Worker], seed: int) -> None:
        if group is self._verify:
            slot = VERIFY_SLOT
        else:
            used = {worker.slot for worker in group}
            slot = next(name for name in (f"w{n}" for n in itertools.count()) if name not in used)
        log_path = self.log_path(slot)
        try:
            (self._scratch / slot / "tmp").mkdir(parents=True, exist_ok=True)
            scratch = Path(os.path.realpath(self._scratch / slot))
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log = open(log_path, "ab")  # noqa: SIM115 - it lives as long as the worker
        except OSError as exc:
            raise Failure(self._entry(
                ENGINE_FAILED, "start", reason="start_failed", path=str(exc.filename or log_path),
                exception_type=type(exc).__name__, raw_text=str(exc),
            )) from exc
        temp = str(scratch / "tmp")
        env = {
            "PYTHONHASHSEED": str(seed),
            **{name: self._blas for name in BLAS_VARIABLES},
            "ACMODEL_SCRATCH": str(scratch),
            "TEMP": temp,
            "TMP": temp,
            "TMPDIR": temp,
        }
        if "SYSTEMROOT" in self._environ:
            env["SYSTEMROOT"] = self._environ["SYSTEMROOT"]
        # On Windows, no console window per worker when the tool runs without one (a scheduled task).
        options = {"creationflags": subprocess.CREATE_NO_WINDOW} if sys.platform == "win32" else {}
        try:
            proc = self._launch(
                [self._python, "-P", "-s", "-B", str(WORKER)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, env=env, cwd=str(scratch), **options,
            )
        except OSError as exc:
            log.close()
            raise Failure(self._entry(
                ENGINE_FAILED, "start", reason="start_failed", path=self._python,
                exception_type=type(exc).__name__, raw_text=str(exc),
            )) from exc
        worker = _Worker(slot, proc, log)
        group.append(worker)
        self.counters["started"] += 1
        self.counters["peak_workers"] = max(self.counters["peak_workers"], self._live())
        try:
            self._os.verify_child(proc.pid)
            self._connect(worker, scratch)
        except BaseException:
            self._kill(worker, group)
            raise
        worker.deadline = self._clock() + self.settings.start_timeout_ms / 1000
        try:
            worker.conn.send((protocol.INIT, self._setup(scratch)))
        except (OSError, EOFError):  # it died at once: the wait sees its channel closed, a crash
            return

    def _setup(self, scratch: Path) -> dict[str, Any]:
        engine = self.engine
        return {
            "root": str(engine.root),
            "manifest_path": str(engine.manifest_path),
            "manifest": engine.doc,
            "entry": engine.doc["entry"],
            "sys_path": [str(engine.root / PurePosixPath(folder)) for folder in engine.doc["sys_path"]],
            "code": list(engine.code),
            "data": list(engine.data),
            "external": dict(engine.external),
            "scratch": str(scratch),
            "site_dirs": list(self._sites),
            "reset_each_batch": self.settings.reset_each_batch,
        }

    def _address(self, scratch: Path) -> tuple[str, str]:
        token = secrets.token_hex(16)
        if sys.platform == "win32":
            return rf"\\.\pipe\acmodel-{os.getpid()}-{token}", "AF_PIPE"
        if sys.platform.startswith("linux"):
            return "\0acmodel-" + token, "AF_UNIX"  # abstract: no file to leave behind
        return str(scratch / f"channel-{token[:8]}"), "AF_UNIX"

    def _connect(self, worker: _Worker, scratch: Path) -> None:
        """Hand the channel to the worker, wait for its listener, connect and authenticate: within
        `engine.start_timeout_ms`, or the worker is stopped."""
        address, family = self._address(scratch)
        key = secrets.token_bytes(32)
        start = json.dumps({"address": address, "family": family, "authkey": key.hex()}).encode("ascii") + b"\n"
        box: dict[str, Any] = {}

        def handshake() -> None:
            try:
                worker.proc.stdin.write(start)
                worker.proc.stdin.flush()
                box["line"] = worker.proc.stdout.readline()
                if box["line"] == protocol.READY_LINE:
                    box["conn"] = Client(address, family, authkey=key)
            except BaseException as exc:  # noqa: BLE001 - handed to the caller below
                box["error"] = exc

        thread = threading.Thread(target=handshake, daemon=True)
        thread.start()
        thread.join(self.settings.start_timeout_ms / 1000)
        if thread.is_alive():
            worker.proc.kill()  # unblocks the handshake: its pipes close
            thread.join()
            raise Failure(self._entry(ENGINE_FAILED, "start", reason="timeout", timeout_ms=self.settings.start_timeout_ms))
        _close(worker.proc.stdout)
        if "conn" in box:
            worker.conn = box["conn"]
            return
        exc, line = box.get("error"), box.get("line", b"")
        if exc is not None or line:  # it is alive and will not connect: stopped now
            worker.proc.kill()
        code = self._wait(worker.proc)  # otherwise it closed its stdout: it is ending by itself
        if exc is None and line:
            raise Failure(self._entry(ENGINE_PROTOCOL_ERROR, "start", reason="frame_invalid", found="start_line"))
        values: dict[str, Any] = {"exit_code": code}
        if exc is not None:
            values.update(exception_type=type(exc).__name__, raw_text=str(exc))
        raise Failure(self._entry(ENGINE_FAILED, "start", reason="worker_exited" if exc is None else "start_failed",
                                  **values))

    def _wait(self, proc: subprocess.Popen) -> int:
        """The exit status of a worker that is ending; killed past `engine.start_timeout_ms`."""
        try:
            return proc.wait(timeout=self.settings.start_timeout_ms / 1000)
        except subprocess.TimeoutExpired:
            proc.kill()
            return proc.wait()

    def _stop(self, worker: _Worker, group: list[_Worker]) -> None:
        """Ask the worker to end, close its lifeline and reap it."""
        if worker.conn is not None:
            try:
                worker.conn.send((protocol.STOP, None))
            except (OSError, EOFError):  # already gone: it is reaped below all the same
                pass
        self._reap(worker, group)

    def _reap(self, worker: _Worker, group: list[_Worker]) -> int | None:
        """Wait for the worker to end (killing it past `engine.start_timeout_ms`), close its handles and drop
        it from its group; its exit status."""
        _close(worker.proc.stdin)
        code = self._wait(worker.proc)
        self._release(worker, group)
        return code

    def _kill(self, worker: _Worker, group: list[_Worker]) -> int | None:
        if worker.proc.poll() is None:
            worker.proc.kill()
        _close(worker.proc.stdin)
        code = worker.proc.wait()
        self._release(worker, group)
        return code

    def _release(self, worker: _Worker, group: list[_Worker]) -> None:
        _close(worker.conn)
        _close(worker.proc.stdout)
        _close(worker.log)
        if worker in group:
            group.remove(worker)
