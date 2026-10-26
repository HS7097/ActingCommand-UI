"""One run's use of the cache (#424 design sections 5.1-5.5).

- **Verifying traces.** A node is reused only when one of its traces (the most recently used first) still
  holds: every input it read, in the order it read them, hashes as recorded. The inputs are checked one
  by one and the check stops at the first difference, so an input is only ever brought up to date when
  the node, computed again, would read it too.
- **Early cutoff.** A node computed again whose value hashes as its previous one changes nothing for the
  nodes that read it: their traces still hold.
- **Corrupt entries** are never served: the node is computed again and `cache_entry_corrupt` is a warning.
- **Work counters** per node kind: computed, reused, and cut off (computed again to the same value).
"""

from __future__ import annotations

from typing import Any, Callable, Mapping

from acmodel.cache.keys import (
    EVALUATORS,
    Evaluator,
    canonical_text,
    digest,
    evaluator_tokens,
    node_key,
    run_id,
    spec_hash,
)
from acmodel.cache.store import CorruptEntry, Store, Trace
from acmodel.codes import CACHE_ENTRY_CORRUPT
from acmodel.envelope import error


class Session:
    def __init__(
        self, store: Store, *, libraries: Mapping[str, str], evaluators: Mapping[str, Evaluator] = EVALUATORS
    ) -> None:
        self.store = store
        self.tokens = evaluator_tokens(evaluators, libraries)
        self.used = store.begin()
        self.warnings: list[dict[str, Any]] = []
        self.evicted = {"entries": 0, "bytes": 0}
        self._traces: dict[str, list[Trace]] = {}
        self._corrupt: set[str] = set()  # node keys met corrupt in this run
        self._counts: dict[str, dict[str, int]] = {}

    def spec(self, kind: str, spec: Any) -> str:
        """The spec hash of a node of `kind`: with its evaluator version, so another version never matches."""
        return spec_hash(kind, self.tokens[kind], spec)

    def traces(self, spec: str) -> list[Trace]:
        if spec not in self._traces:
            self._traces[spec] = self.store.traces(spec)
        return self._traces[spec]

    def lookup(self, kind: str, spec: str, current: Callable[[tuple[str, ...]], str]) -> tuple[str, bytes] | None:
        """The value hash and value of a trace of the node that still holds, or None.

        `current(input)` is the input's value hash in this run; for a graph node not computed yet it raises
        `Need`, and the lookup is asked again once it is.
        """
        for trace in self.traces(spec):
            if trace.key in self._corrupt:
                continue
            if trace.reads is None:
                self.corrupt(kind, trace.key, CorruptEntry("index_invalid", self.store.index))
                self.store.drop(trace.key)
                continue
            if not all(current(item) == value for item, value in trace.reads):
                continue
            try:
                blob = self.store.read_blob(trace.value)
            except CorruptEntry as corrupt:
                self.corrupt(kind, trace.key, corrupt)
                return None
            self.store.touch(trace.key, self.used)
            self.count(kind, "reused")
            return trace.value, blob
        return None

    def save(
        self, kind: str, spec: str, node: list[str], trace: list[list[Any]], blob: bytes, compute_us: int
    ) -> str:
        """Keep a computed node: its trace (each input with its value hash, in the order read) and its value.
        Return the value hash."""
        value = digest(blob)
        previous = self.traces(spec)
        if previous and previous[0].value == value:
            self.count(kind, "cutoff")
        self.count(kind, "computed")
        self.store.put(
            key=node_key(spec, trace), spec=spec, kind=kind, node=canonical_text(node), value=value,
            deps=canonical_text(trace), blob=blob, used=self.used, compute_us=compute_us,
        )
        return value

    def corrupt(self, kind: str, key: str, corrupt: CorruptEntry) -> None:
        self._corrupt.add(key)
        self.warnings.append(error(CACHE_ENTRY_CORRUPT, corrupt.detail(key=key, kind=kind)))

    def count(self, kind: str, what: str) -> None:
        counts = self._counts.setdefault(kind, {"computed": 0, "reused": 0, "cutoff": 0})
        counts[what] += 1

    def run_id(self, inputs: Mapping[str, Any]) -> str:
        return run_id(self.tokens, inputs)

    def finish(self, run: str, outputs: bytes) -> None:
        """Keep the run's outputs under its run id, evict down to the size cap (never this run's entries)
        and commit."""
        if self.store.write_run(run, outputs):
            path = self.store.run_path(run)
            self.warnings.append(error(CACHE_ENTRY_CORRUPT, {"reason": "run_outputs_mismatch", "path": str(path)}))
        self.evicted = self.store.evict(keep=self.used)
        self.store.commit()

    def work(self) -> dict[str, Any]:
        totals = {what: sum(counts[what] for counts in self._counts.values()) for what in ("computed", "reused", "cutoff")}
        return {
            "nodes": totals["computed"] + totals["reused"],
            **totals,
            "corrupt": len(self._corrupt),
            "evicted": dict(self.evicted),
            "by_kind": {kind: dict(self._counts[kind]) for kind in sorted(self._counts)},
        }
