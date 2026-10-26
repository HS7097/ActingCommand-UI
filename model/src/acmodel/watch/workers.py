"""The engine workers that `watch` keeps warm (#424 design-py 5.6; contracts/cli.md, "Automatic refresh").

One pool (`engine.pool.Pool`) per engine that `model.json` lists, loaded through `engines.roots`. With
`watch.keep_workers_warm` the workers are started ahead of the calls and stay started between refreshes, so that an
engine's context is built once and not per change; together they are at most `compute.workers.max`, shared out in
engine-id order. When the host gate holds, every worker is stopped, which frees its memory; they start again, cold,
once it opens. A pool whose engine is no longer listed, or whose fingerprint changed (another byte of its code, data
or manifest), is closed and replaced, so that a worker never runs other code than its fingerprint names.
"""

from __future__ import annotations

from typing import Callable, Sequence

from acmodel.engine.manifest import Engine
from acmodel.engine.pool import Pool


class Workers:
    def __init__(self, make_pool: Callable[[Engine], Pool], *, workers_max: int) -> None:
        self._make = make_pool
        self._max = workers_max
        self._pools: dict[str, Pool] = {}

    def engines(self) -> list[str]:
        return sorted(self._pools)

    def update(self, engines: Sequence[Engine]) -> None:
        """Keep one pool per engine given: close the others, and those whose fingerprint changed."""
        wanted = {engine.engine_id: engine for engine in engines}
        for engine_id in sorted(self._pools):
            pool = self._pools[engine_id]
            if engine_id not in wanted or wanted[engine_id].fingerprint != pool.engine.fingerprint:
                pool.close()
                del self._pools[engine_id]
        for engine_id in sorted(wanted):
            if engine_id not in self._pools:
                self._pools[engine_id] = self._make(wanted[engine_id])

    def pids(self) -> list[int]:
        """The process ids of every worker alive."""
        return sorted(pid for engine_id in sorted(self._pools) for pid in self._pools[engine_id].pids())

    def warm(self) -> int:
        """Start each pool's share of `compute.workers.max` and wait until they are ready; how many started. A
        failure stops every worker and raises."""
        ids = sorted(self._pools)
        started = 0
        try:
            for index, engine_id in enumerate(ids):
                share = self._max // len(ids) + (1 if index < self._max % len(ids) else 0)
                if share:
                    started += self._pools[engine_id].warm(share)
        except BaseException:
            self.stop()
            raise
        return started

    def stop(self) -> int:
        """Stop every worker; how many were alive."""
        alive = len(self.pids())
        for engine_id in sorted(self._pools):
            self._pools[engine_id].close()
        return alive
