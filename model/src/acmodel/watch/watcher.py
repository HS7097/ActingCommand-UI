"""`acmodel watch <model> --state <file>`: automatic refresh (#424 design 5.6, design-py 5.6; contracts/cli.md,
"Automatic refresh").

**Its folder** is `<paths.state_root>/watch/<id>/`, the id the first 16 hex digits of the sha256 of the model folder
and the state file, so that a restart with the same command line finds it again.

**Each poll**, every `watch.poll_interval_ms`:
1. *Scan* the watched files and the state snapshot (`files`). A change opens a window of `watch.debounce_ms`; at its
   end one refresh covers every change seen in it, so a burst of changes gives one recompute per window. A rewrite
   of the same bytes is no change.
2. *Check the host gate* (`host.gate`): the first poll as a batch's start, every later one as a checkpoint, since
   the watch is one long batch. While it holds, every engine worker is stopped and nothing is computed; the window
   waits. It opens again only at the start level.
3. *Keep the engine workers warm* (`watch.keep_workers_warm`; `workers`): started, cold, when none are alive.
4. *Refresh* when the window has ended: import the snapshot, run the model through the cache (only what a change
   reaches is computed again), keep the run under `runs/<run id>/`, and stage a delivery candidate when the export
   files differ from the last one's (`candidate`). A refresh that the model, the state or the exports refuse is
   logged with its errors (`refresh_failed`, or `refreshed` with `export_errors`), and the watch goes on until the
   next change; the watch's own failures and the host's end it.

**Resume.** After every refresh, `watch.json` records the files as that refresh saw them, its run id and the last
candidate (`journal`). A restart reads it back: when nothing changed meanwhile, nothing is computed; otherwise the
changes open a window as usual, and the refresh reuses every cached node that they do not reach.

**It never commits, pushes or publishes anything, and never reads or writes the live Runtime.** It is long-lived,
so it is started by Task Scheduler or WMI, never from an agent session, and it ends at an interrupt (`stopped`).
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Mapping, TextIO

from acmodel import cache
from acmodel.cache.keys import canonical, digest
from acmodel.check.files import manifest_path
from acmodel.codes import (
    CACHE_UNAVAILABLE,
    HOST_JOB_NOT_APPLIED,
    HOST_PRIORITY_NOT_APPLIED,
    HOST_READING_FAILED,
    WATCH_FAILED,
)
from acmodel.config import Config
from acmodel.core.run import Run, evaluate_model, finish_run
from acmodel.engine import manifest
from acmodel.engine.manifest import Engine
from acmodel.engine.pool import Pool, WorkerOs
from acmodel.envelope import Failure
from acmodel.host.gate import CHECKPOINT, START, Gate, Host
from acmodel.model import read_model
from acmodel.watch import candidate
from acmodel.watch.files import FileSnapshot, Item, Seen, Stamper, disk_stamp, failed, file_item, listing, roots, scan
from acmodel.watch.journal import Journal, Saved, write_atomic
from acmodel.watch.workers import Workers

WATCH = "watch"
SNAPSHOTS = "snapshots"
WORKERS = "workers"
ID_DIGITS = 16
# What ends the watch: the cache, the host layer and the watch's own files. A later change of the model or the
# state cannot mend them.
FATAL = {CACHE_UNAVAILABLE, HOST_JOB_NOT_APPLIED, HOST_PRIORITY_NOT_APPLIED, HOST_READING_FAILED}


def fatal(failure: Failure) -> bool:
    return any(
        entry["code"] in FATAL or (entry["code"] == WATCH_FAILED and entry["detail"]["stage"] != "import")
        for entry in failure.errors
    )


def watch_folder(state_root: Path, model: Path, state: Path) -> Path:
    return state_root / WATCH / digest(canonical({"model": str(model), "state": str(state)}))[:ID_DIGITS]


class Watcher:
    def __init__(
        self,
        model: Path,
        state: Path,
        *,
        cfg: Config,
        libraries: Mapping[str, str],
        host: Host,
        workers_os: WorkerOs,
        stderr: TextIO,
        stamp: Stamper = disk_stamp,
    ) -> None:
        self.model = Path(os.path.realpath(manifest_path(Path(model)))).parent  # the model folder
        self.source = FileSnapshot(Path(os.path.realpath(state)), stamp)
        self.cfg = cfg
        self.settings = cfg.watch
        self.libraries = dict(libraries)
        self.host = host
        self.stamp = stamp
        self.folder = watch_folder(cfg.state_root, self.model, self.source.path)
        self.journal = Journal(self.folder, host.now, stderr)
        self.gate = Gate(cfg.host, host)
        self.workers = Workers(self._pool, workers_max=cfg.engine.workers_max)
        self._os = workers_os
        self.seen: dict[str, Seen] = {}
        self.last: dict[str, str] | None = None  # the last refresh: `run_id` and the imported snapshot
        self.candidate: dict[str, Any] | None = None  # the last candidate: `number` and `sha256`
        self.due_at: int | None = None  # when the open window ends (monotonic ms)
        self.resumed = False
        self.paused = False
        self.warm_failed = False
        self._first_check = True
        self._quiet_scan = False  # the first scan of a fresh start: every file is new, and the refresh is due
        self.polls = 0
        self.refreshes = 0
        self.staged: list[int] = []

    def _pool(self, engine: Engine) -> Pool:
        return Pool(
            engine, self.cfg.engine, scratch_root=self.folder / WORKERS / "scratch",
            log_dir=self.folder / WORKERS / "logs", os_layer=self._os, blas_threads=self.cfg.blas_threads,
        )

    # The loop -------------------------------------------------------------------------------------------

    def run(self) -> dict[str, Any]:
        """Poll every `watch.poll_interval_ms` until interrupted; the summary. A fatal failure is logged, every
        worker is stopped, and it is raised."""
        try:
            self.start()
            while True:
                self.poll()
                self.host.sleep_ms(self.settings.poll_interval_ms)
        except KeyboardInterrupt:
            self.journal.event("stopped")
        except Failure as failure:
            try:
                self.journal.event("failed", errors=failure.errors)
            except Failure as unlogged:  # both are reported
                failure.errors.extend(unlogged.errors)
            raise
        finally:
            self.workers.stop()
        return self.summary()

    def start(self) -> None:
        saved = self.journal.load(str(self.model), str(self.source.path))
        if saved is None:
            self.due_at = self.host.monotonic_ms()  # a first refresh at once
            self._quiet_scan = True
        else:
            self.resumed = True
            self.seen, self.last, self.candidate = saved.files, saved.last, saved.candidate
        self.journal.event(
            "started", model=str(self.model), state=str(self.source.path), folder=str(self.folder),
            resumed=self.resumed, run_id=None if self.last is None else self.last["run_id"],
            candidate=None if self.candidate is None else self.candidate["number"],
        )
        try:
            ids = [entry["engine_id"] for entry in read_model(manifest_path(self.model))["engines"]]
        except Failure as failure:  # the workers wait for a refresh that reads the model
            self.journal.event("workers_failed", errors=failure.errors)
            return
        self._engines(ids)

    def poll(self) -> None:
        self.polls += 1
        now = self.host.monotonic_ms()
        changed = self._scan()
        if changed and not self._quiet_scan:
            self.journal.event("changed", files=changed)
            if self.due_at is None:
                self.due_at = now + self.settings.debounce_ms
        self._quiet_scan = False
        held = self.gate.check(START if self._first_check else CHECKPOINT)
        self._first_check = False
        if held:
            if not self.paused:
                self.paused = True
                self.journal.event("paused", reasons=held)
            stopped = self.workers.stop()
            if stopped:
                self.journal.event("workers_stopped", workers=stopped)
            return
        if self.paused:
            self.paused = False
            self.journal.event("resumed")
        if self.settings.keep_workers_warm and not self.warm_failed and not self.workers.pids():
            self._warm()
        if self.due_at is not None and now >= self.due_at:
            self._refresh()

    def summary(self) -> dict[str, Any]:
        return {
            "path": str(self.model),
            "state": str(self.source.path),
            "watch": str(self.folder),
            "resumed": self.resumed,
            "polls": self.polls,
            "refreshes": self.refreshes,
            "staged": list(self.staged),
            "run_id": None if self.last is None else self.last["run_id"],
            "candidate": None if self.candidate is None else self.candidate["number"],
        }

    # The parts ------------------------------------------------------------------------------------------

    def _scan(self) -> list[str]:
        items = [file_item(path, self.stamp) for path in listing(roots(self.model, self.cfg.datasets))]
        items.append(Item(str(self.source.path), self.source.stamp, self.source.read))
        self.seen, changed = scan(items, self.seen)
        return changed

    def _engines(self, ids: list[str]) -> None:
        """Keep a pool for each engine `model.json` lists, as its files are now."""
        engines, errors = [], []
        for engine_id in ids:
            try:
                engines.append(manifest.load(
                    engine_id, roots=self.cfg.engine.roots, datasets=self.cfg.datasets, libraries=self.libraries
                ))
            except Failure as failure:
                errors.extend(failure.errors)
        self.workers.update(engines)
        self.warm_failed = False
        if errors:
            self.journal.event("workers_failed", errors=errors)

    def _warm(self) -> None:
        if not self.workers.engines():
            return
        try:
            started = self.workers.warm()
        except Failure as failure:
            if fatal(failure):
                raise
            self.warm_failed = True  # tried again after the next refresh
            self.journal.event("workers_failed", errors=failure.errors)
            return
        self.journal.event("workers_started", engines=self.workers.engines(), workers=started)

    def _import(self) -> Path:
        """The snapshot's bytes, copied under the watch folder once, named by their sha256."""
        try:
            data = self.source.read()
        except OSError as exc:
            raise failed("import", self.source.path, exc) from exc
        target = self.folder / SNAPSHOTS / f"{digest(data)}.json"
        try:
            present = target.read_bytes() == data
        except FileNotFoundError:
            present = False
        except OSError as exc:
            raise failed("import", target, exc) from exc
        if not present:
            write_atomic(target, data, "import")
        return target

    def _refresh(self) -> None:
        self.due_at = None
        self.refreshes += 1
        files = dict(self.seen)  # what this refresh saw
        cfg = self.cfg
        try:
            imported = self._import()
            with cache.open_store(cfg.state_root, max_bytes=cfg.cache_max_bytes, wait_ms=cfg.cache_wait_ms) as store:
                session = cache.Session(store, libraries=self.libraries)
                run = evaluate_model(
                    self.model, imported, datasets=cfg.datasets, max_bits=cfg.expr_max_bits, unknowns=cfg.unknowns,
                    cache=session,
                )
                data = finish_run(run, session, max_bits=cfg.expr_max_bits, unknowns=cfg.unknowns)
                outputs = store.run_path(data["run_id"])
        except Failure as failure:
            if fatal(failure):
                raise
            self.journal.save(str(self.model), str(self.source.path), Saved(files, self.last, self.candidate))
            self.journal.event("refresh_failed", errors=failure.errors)
            return
        self._engines([entry["engine_id"] for entry in run.checker.model.manifest.doc["engines"]])
        self.last = {"run_id": data["run_id"], "state": str(imported)}
        exports, staged = self._candidate(run, data["run_id"], imported)
        self.journal.save(str(self.model), str(self.source.path), Saved(files, self.last, self.candidate))
        self.journal.event(
            "refreshed", run_id=data["run_id"], outputs=str(outputs), work=data["work"], errors=run.errors,
            warnings=session.warnings, **exports,
        )
        if staged is not None:
            self.journal.event("staged", **staged)

    def _candidate(self, run: Run, run_id: str, imported: Path) -> tuple[dict[str, Any], dict[str, Any] | None]:
        """Stage a delivery candidate when the export files differ from the last candidate's. Returns what the
        `refreshed` event says of the exports, and the `staged` event's values when one was staged."""
        try:
            files = candidate.build(run, self.folder)
        except Failure as failure:
            return {"exports": "refused", "candidate": None, "export_errors": failure.errors}, None
        sha256 = candidate.fingerprint(files)
        if self.candidate is not None and self.candidate["sha256"] == sha256:
            return {"exports": "unchanged", "candidate": self.candidate["number"]}, None
        head = {
            "staged_at": self.host.now().isoformat(timespec="milliseconds"),
            "model": str(self.model),
            "model_id": run.head["model_id"],
            "state": {"source": str(self.source.path), "imported": str(imported), "label": run.head["state"]["label"]},
            "run_id": run_id,
        }
        previous = None if self.candidate is None else self.candidate["number"]
        number, path, changes = candidate.stage(self.folder, files, previous=previous, head=head)
        self.candidate = {"number": number, "sha256": sha256}
        self.staged.append(number)
        staged = {
            "candidate": number, "path": str(path), "from": changes["from"],
            "changed": [[entry["name"], entry["change"]] for entry in changes["files"]],
        }
        return {"exports": "staged", "candidate": number}, staged
