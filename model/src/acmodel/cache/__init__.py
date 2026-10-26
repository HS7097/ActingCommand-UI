"""The cache and incremental recompute (#424 design section 5; contracts/cli.md, "Cache").

`run` opens the store under `paths.state_root` (`open_store`) and a `Session` on it, and the derivation
graph computes through it (`core.graph`): each node is reused while its verifying trace holds, and computed
again otherwise. `cache stats`, `cache verify` and `cache gc` inspect and trim the store (`maintenance`).
"""

from acmodel.cache.keys import EVALUATORS, Evaluator, canonical, canonical_text, digest, libraries
from acmodel.cache.maintenance import gc, stats, verify
from acmodel.cache.session import Session
from acmodel.cache.store import Store, existing_store, open_store

__all__ = [
    "EVALUATORS", "Evaluator", "Session", "Store", "canonical", "canonical_text", "digest", "existing_store",
    "gc", "libraries", "open_store", "stats", "verify",
]
