"""`acmodel watch`: automatic refresh (#424 design 5.6, design-py 5.6; contracts/cli.md, "Automatic refresh").

- `settings`: the keys `watch.poll_interval_ms`, `watch.debounce_ms` and `watch.keep_workers_warm`.
- `files`: the scan of the watched files by size and modification time, the sha256 of those that changed, and the
  import hook of the state snapshot (today a file a person or an agent writes).
- `journal`: the event log `events.jsonl` and `watch.json`, what a restart resumes from.
- `candidate`: the delivery candidates, the files `export thresholds` and `export tables` would write, staged with
  a summary and a diff under the watch folder, never committed.
- `workers`: the engine workers kept warm between refreshes, stopped while the host gate holds.
- `watcher`: polling, debounce, the gate, the refresh through the cache, and resume after a restart.

`watch` never commits, pushes or publishes anything, and never reads or writes the live Runtime. This file imports
nothing, so that config.py can read `settings` without loading the rest.
"""
