"""One engine call through the cache and the verify sample (#424 design section 5.1, design-py 7.3;
contracts/cli.md, "Engine host").

**Cache keys.** An engine result is a cache node of kind `engine`:
- its spec is the engine id, the engine's **fingerprint**, the op and the request **without** its
  `entities`; a changed engine file, manifest or library version is a new fingerprint and so a miss;
- its trace is ("entity", id) with the hash of `entities[id]` for each id of the result's `depends_on`,
  so a cached result is reused for any request that differs only in entities the engine did not read.

Domain failures and every other error are never kept.

**Verify sample.** `engine.verify_sample_milli` of the requests, chosen by the sha256 of
[fingerprint, op, request] (so the same requests every time), run again with the full request in a fresh
worker with the other hash seed (`Pool.verify`), cached results included: a difference is
`engine_nondeterministic`, which catches hash or set order, state carried between calls and an untruthful
`depends_on`. When it fails, the call fails and nothing of it is kept.
"""

from __future__ import annotations

import json
import time
from typing import TYPE_CHECKING, Any

from acmodel.cache.keys import canonical, digest
from acmodel.engine.pool import Outcome, Pool
from acmodel.engine.protocol import ENTITIES
from acmodel.envelope import Failure

if TYPE_CHECKING:
    from acmodel.cache.session import Session

KIND = "engine"


def sampled(fingerprint: str, op: str, request: dict[str, Any], milli: int) -> bool:
    """Whether the verify run takes this request: the first 32 bits of the sha256 of [fingerprint, op,
    request], modulo 1000, below `milli`."""
    return int(digest(canonical([fingerprint, op, request]))[:8], 16) % 1000 < milli


def _entity_hash(request: dict[str, Any], entity: str) -> str:
    return digest(canonical(request.get(ENTITIES, {}).get(entity)))


def _spec(session: Session, pool: Pool, op: str, request: dict[str, Any]) -> str:
    engine = pool.engine
    context = {key: value for key, value in request.items() if key != ENTITIES}
    return session.spec(KIND, {"engine": engine.engine_id, "fingerprint": engine.fingerprint, "op": op, "request": context})


def call(pool: Pool, op: str, requests: list[dict[str, Any]], *, session: Session | None = None) -> list[Outcome]:
    """Each request's outcome, in order: from the cache where a trace still holds, from the workers
    otherwise; then the verify sample."""
    engine = pool.engine
    outcomes: list[Outcome | None] = [None] * len(requests)
    specs: dict[int, str] = {}
    if session is not None:
        for index, request in enumerate(requests):
            specs[index] = spec = _spec(session, pool, op, request)
            hit = session.lookup(KIND, spec, lambda item, request=request: _entity_hash(request, item[1]))
            if hit is not None:
                outcomes[index] = Outcome(result=json.loads(hit[1]))
    misses = [index for index, outcome in enumerate(outcomes) if outcome is None]
    started = time.perf_counter_ns()
    computed = pool.call(op, [requests[index] for index in misses], indices=misses)
    compute_us = (time.perf_counter_ns() - started) // 1000 // max(1, len(misses))
    for index, outcome in zip(misses, computed):
        outcomes[index] = outcome
    chosen = [
        index for index, outcome in enumerate(outcomes)
        if outcome.error is None and sampled(engine.fingerprint, op, requests[index], pool.settings.verify_sample_milli)
    ]
    problems = pool.verify(
        op, [requests[index] for index in chosen], [outcomes[index].result for index in chosen], indices=chosen
    )
    if problems:
        raise Failure(*problems)
    if session is not None:
        for index in misses:
            result = outcomes[index].result
            if result is None:
                continue
            trace = [[["entity", entity], _entity_hash(requests[index], entity)] for entity in sorted(result["depends_on"])]
            session.save(KIND, specs[index], [KIND, engine.engine_id, op], trace, canonical(result), compute_us)
    return outcomes
