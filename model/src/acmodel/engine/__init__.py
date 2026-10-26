"""The engine host (#424 design-py section 7; contracts/formats.md "Engines", contracts/cli.md "Engine host").

An engine is Python code outside this repository, found through `engines.roots` and its manifest
`engine.json` (`acmodel.engine.v2`). It runs only inside the tool's own worker processes; the main
process never imports it.

- `manifest`: reads `engine.json`, lists the engine's files and computes its fingerprint (`load`).
- `settings`: the configuration keys `compute.workers.max`, `engine.*` and `engines.roots`.
- `pool`: starts the workers (explicit environment, hash seed), talks to them over
  `multiprocessing.connection`, and handles batching, timeouts, restarts, recycling and the verify run.
- `client`: one engine call through the cache (keyed by the fingerprint, the op and the request, traced
  by `depends_on`) and the verify sample.
- `protocol`, `guards`, `worker`: the payload rules, the worker's guards and the worker itself. These
  three use the standard library only, because the worker imports them.

This file imports nothing, so that the worker loads only what it needs.
"""
