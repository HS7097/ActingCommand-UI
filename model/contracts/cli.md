# acmodel command line (`acmodel.cli.v1`)

## Start

```text
py -3 -I -B <model>\acmodel.py <command>      (Windows; or <model>\acmodel.cmd <command>)
python3 -I -B <model>/acmodel.py <command>    (elsewhere)
```

1. The launcher puts `<model>/src` on `sys.path`. Under `-I`, neither the script's folder nor the cwd is
   on it, so the start does not depend on the cwd.
2. It parses the command line and reads the configuration.
3. It sets `OPENBLAS_NUM_THREADS`, `OMP_NUM_THREADS` and `MKL_NUM_THREADS` to `compute.blas_threads`, in
   its own environment and so in every child's.
4. `setup` runs on the Python that started the launcher. Every other command needs the venv:
   - If `sys.prefix` is not the configured venv, the launcher re-runs itself as
     `<venv python> -I -B <model>/acmodel.py <same arguments>` through `subprocess`. Its exit status is
     the child's, and the child prints the envelope.
     - `<venv python>` is `Scripts\python.exe` on Windows and `bin/python` elsewhere.
     - The child's environment carries `ACMODEL_DISPATCHED=1`; a child that still does not run in the
       venv fails with `venv_failed` (`stage: dispatch`) instead of dispatching again.
   - In the venv, the dependency check runs first, then the command.

## Commands

| Command | Does | `data` |
|---|---|---|
| `setup` | Creates the venv at `paths.venv` from the running Python 3.13 (`python -I -B -m venv <path>`) if the path is absent. Then it installs `requirements.txt` with `python -I -B -m pip install --require-hashes --only-binary=:all: --no-deps -r requirements.txt` using the venv's Python. It reuses a 3.13 venv, refuses a venv of another minor version (`deps_invalid`, `python_version`) and a path that holds no venv (`venv_failed`, `stage: inspect`), and never deletes anything. The children's output goes to stderr. | `venv`, `created`, `requirements` |
| `version` | Runs the dependency check and reports the versions. | `tool`, `tool_version`, `python`, `packages` (name to version) |
| `check <model>` | Runs the dependency check, then the static check of the model: `<model>` is the model folder or its `model.json` (`formats.md`, "The static check"). It reads and computes, never writes. Every problem is one error; with any, the exit status is 1. | `path`, `model_id`, `entities`, `rules`, `superseded`, `unknowns`, `tables` |
| `run <model> --state <file>` | Runs the dependency check, the static check, the run's model checks and the derivation graph for the account state `<file>` (`formats.md`, "The run"), incrementally through the cache (see Cache). It writes only under `paths.state_root`: the cache and the run's outputs. A model or input that does not check is refused whole (exit 1, `data` `{}`). Otherwise every output is computed or blocked: a blocked output's error lists the outputs it blocks in `needed_by`, the other outputs are still in `data`, and the exit status is 1. Provisional outputs alone exit 0. A corrupt cache entry is a warning (`cache_entry_corrupt`) and the node is computed again. | `path`, `model_id`, `state`, `run_id`, `rules`, `thresholds`, `notes`, `work` |
| `explain <model> <output> --state <file>` | Runs as `run` does, without the cache, and prints the provenance document of one output (`rule:<id>`, `general_logic:<id>` or `threshold:<id>`; `formats.md`, "Outputs"). It writes nothing. An id that is not an output is `usage_invalid` (`output_unknown`); a blocked output reports the errors that block it (exit 1). | `path`, `model_id`, `state`, `output`, `provenance` |
| `export thresholds <model> --state <file>` | Runs as `run` does, without the cache, then writes `out/thresholds.json` and one provenance file per threshold into the model folder (`formats.md`, "Outputs"). Only account-independent, confirmed, integer thresholds are exported: any other is `export_refused` (exit 5), and then nothing is written. A per-account value never enters these shared files. | `path`, `model_id`, `state`, `written` (the files written) |
| `bindings check <model> --state <file> [--strict]` | Runs as `run` does, without the cache, and compares each literal that `bindings.json` binds, in `schedule.json` or a pack file, with its threshold's value as `export thresholds` would write it (`formats.md`, "Bindings"). A threshold the export refuses is refused the same way (`export_refused`, exit 5); a `fact_offset` threshold whose live offset read the account state is `binding_unsupported` (exit 5); a literal that differs, or sits where its threshold's form cannot, is `binding_mismatch` (exit 6). With `--strict`, every integer literal of a `fact` predicate of `schedule.json` that is neither bound nor fixed is `binding_unbound_literal` (exit 6). It writes nothing. | `path`, `model_id`, `state`, `repo`, `strict`, `bindings`, `fixed`, and `literals` with `--strict` |
| `bindings apply <model> --state <file>` | Runs as `bindings check` does without `--strict`, then writes each bound literal that differs from its threshold. Only the literal's bytes change: line ends, spacing and every other byte stay. Anything else `check` refuses is refused, and then nothing is written. | `path`, `model_id`, `state`, `repo`, `changed`, `written` |
| `cache stats` | Counts what the cache holds. It writes nothing. | `path`, `exists`, `max_bytes`, `entries`, `blobs`, `bytes`, `by_kind` (entries per node kind), `runs` |
| `cache verify` | Reads every blob the index names and checks that it hashes to its name. Each entry that does not is one `cache_entry_corrupt` error (exit 1). It writes and repairs nothing. | `path`, `exists`, `entries`, `blobs`, `problems` |
| `cache gc [--apply]` | Plans the eviction down to `cache.max_bytes`, least recently used first, and the removal of the blob files no entry names (orphans, and temporary files of interrupted writes). Without `--apply` it only reports the plan; with it, it removes exactly that, inside the cache folder. Run it while no run is in progress. | `path`, `exists`, `apply`, `max_bytes`, `entries`, `bytes`, `entries_removed`, `blobs_removed`, `bytes_removed`, `orphans_removed`, `orphan_bytes_removed`, `other_files` |

A command is one word, or a group and a word: `export`, `bindings` and `cache` are groups
(`export thresholds`, `bindings check`, `cache stats`), and the envelope's `command` is both words. Every
argument and option a command names is required, and an option takes one value. A missing one is
`usage_invalid` (`argument_missing`, with the argument's or option's name), and an extra or repeated one is
`usage_invalid` (`argument_unexpected`). A flag (`--strict` of `bindings check`, `--apply` of `cache gc`)
takes no value, is optional and may be given once. A state root without a cache is reported by the `cache`
commands as `exists: false`; they create nothing.

**Dependency check.** It runs before every command except `setup`:
- the interpreter must be Python 3.13;
- numpy and scipy must be installed at the pins of `requirements.txt`.

Each problem is one `deps_invalid` error (exit 2), with `reason` `python_version`, `missing` or
`version_mismatch`, and `setup_command`. A venv whose Python is absent is `deps_invalid` `missing` with
its `path`.

## Envelope

Every run prints exactly one line of ASCII JSON on stdout. A re-dispatching parent prints nothing; its
child prints the line.

```json
{"schema_version":"acmodel.cli.v1","command":"version","ok":true,"data":{},"errors":[],"warnings":[]}
```

| Member | Type | Meaning |
|---|---|---|
| `schema_version` | string | `acmodel.cli.v1` |
| `command` | string or null | the command that ran; null when the command line is refused (`usage_invalid`) |
| `ok` | bool | true exactly when `errors` is empty |
| `data` | object | the command's result; `{}` when it failed, except a partial `run`, which keeps its outputs |
| `errors` | list | entries `{code, detail, ref?, needed_by?}` |
| `warnings` | list | entries of the same shape: problems the command recovered from (`run`: `cache_entry_corrupt`); they never change the exit status |

**Error entries.**
- `code` is a code registered in `codes.json`.
- `detail` holds that code's typed values only, never prose.
- `ref` is a reference into the input when there is one: for `config_invalid`, the JSON pointer of the
  member, with the file in `detail.path`; for the model's codes, the JSON pointer into the file in
  `detail.path`.
- `needed_by` lists the outputs an error blocks (`run`, `explain`, `export thresholds`, `bindings`: `rule:<id>`,
  `general_logic:<id>`, `threshold:<id>`), sorted.

Human text, one line per error and per warning and the usage text, goes to stderr.

## Exit statuses

The exit status is that of the first error's code (`codes.json`), or 0.

| Status | Meaning |
|---|---|
| 0 | ok, possibly with provisional outputs |
| 1 | model or input error (something blocked) |
| 2 | usage |
| 3 | engine failure |
| 4 | host gate (paused or stopped; resumable) |
| 5 | export refused |
| 6 | regression or binding mismatch |

## Configuration (`acmodel.config.v1`)

- **Location.** `%LOCALAPPDATA%\ActingCommand\acmodel\config.json`. If `LOCALAPPDATA` is unset or not
  an absolute path, the run fails with `config_invalid` (`location_invalid`); it never guesses.
- **Defaults.** The file is optional, and every key has a default.
- **Reading.** A present file is read strictly: UTF-8 JSON without BOM, no duplicate keys, no unknown
  members, typed values.

| Key | Default | Meaning |
|---|---|---|
| `schema_version` | (required in a file) | `acmodel.config.v1` |
| `paths.venv` | `%LOCALAPPDATA%\ActingCommand\acmodel\venv` | the venv; an absolute path |
| `paths.state_root` | `%LOCALAPPDATA%\ActingCommand\acmodel\state` | the state root: the cache and the runs' outputs (see Cache); an absolute path, never inside a repository |
| `paths.datasets` | `%LOCALAPPDATA%\ActingCommand\acmodel\datasets` | the local copies of external datasets, each the file named by its sha256 (`check` verifies them); an absolute path |
| `compute.blas_threads` | 1 | the value of the BLAS thread variables; an integer of at least 1 |
| `expr.max_bits` | 127 | the most bits a numerator or denominator may have in expression evaluation (durations in seconds); beyond it is `expr_overflow`; an integer of at least 1 |
| `cache.max_bytes` | 2147483648 (2 GiB) | the size cap of the cache's values; above it, the least recently used entries are evicted after a run (never that run's) and by `cache gc --apply` |
| `compute.priority` | `below_normal` | the priority class of the tool and of every process it starts: `below_normal` or `idle` |
| `compute.job.cpu_rate_milli` | 250 | the hard CPU cap of the tool's Job object, in thousandths of all logical processors together; 1 to 1000 |
| `compute.job.worker_memory_mib` | 1024 | the memory limit of every process in the Job (the tool and its workers), in MiB; at least 1 |
| `host.memory.start_min_mib` | 4096 | a batch starts, and a paused gate reopens, only at or above this much available host memory (MiB) |
| `host.memory.pause_below_mib` | 2560 | inside a batch, a checkpoint pauses below this (MiB); at most `start_min_mib` |
| `host.memory.report_plan_above_mib` | 3072 | a run whose planned memory is above this (MiB) is reported before it starts |
| `host.cpu.pause_load_milli` | 900 | the gate holds while the host CPU load is above this (thousandths); 1 to 1000 |
| `host.cpu.window_ms` | 5000 | the CPU load is averaged over at least this many milliseconds |
| `host.quiet_windows` | `[]` | local-time windows without compute (see Host caps) |
| `host.pause_file` | `<paths.state_root>\PAUSE` | while this file exists, the gate holds; an absolute path |
| `host.pause_poll_ms` | 5000 | how often a waiting gate checks again, in milliseconds |
| `compute.workers.max` | 2 | the most engine worker processes alive at once, the verify worker included (see Engine host) |
| `engine.hash_seed` | 0 | `PYTHONHASHSEED` of the workers; 0 to 4294967295 |
| `engine.verify_hash_seed` | 1 | `PYTHONHASHSEED` of the verify worker; 0 to 4294967295, and not `engine.hash_seed` |
| `engine.verify_sample_milli` | 10 | the share of an engine call's requests run again in the verify worker, in thousandths; 0 (none) to 1000 (all) |
| `engine.reset_each_batch` | `true` | call the engine's `reset()` after every batch; a boolean |
| `engine.worker_soft_memory_mib` | 896 | a worker whose private bytes exceed this after a batch is recycled (MiB); below `compute.job.worker_memory_mib` |
| `engine.worker_max_requests` | 0 | a worker is recycled after this many requests; 0: never |
| `engine.request_timeout_ms` | 600000 | the most time per request of a batch; a batch has its number of requests times this |
| `engine.start_timeout_ms` | 120000 | the most time a worker may take to start (connect, import, `make_engine`, `describe`), and to stop before it is killed |
| `engine.restart_budget` | 3 | how many workers that ended abnormally (a crash, a timeout, the memory limit) a pool replaces; 0 or more |
| `engines.roots` | `{}` | where each engine's home is on this host: engine id to an absolute path (#424 design D-4) |

| `sensitivity.top_n` | 5 | how many unknowns each output's `sensitivity` lists (`formats.md`, "Unknowns") |
| `unknowns.scale` | 1000 | the export precision that the no-effect rule (on the sound range) and the impacts are measured at: a value counts as value × scale rounded half to even (1000: milli) |
| `unknowns.samples` | 0 | the Monte Carlo samples of the distributions per output; 0, the default, samples nothing; at most 2<sup>63</sup> − 1 |
| `unknowns.seed` | 0 | the first word of each Philox key (the second is the unknown's stream id); 0 to 2<sup>64</sup> − 1 |
| `unknowns.chunk_size` | 1024 | the samples drawn per Philox call; it never changes a result |
| `unknowns.resolution` | 1000000 | the equal cells of a draw on (0, 1); at most 2<sup>64</sup> |
| `unknowns.wilson_digits` | 6 | the decimals that the Wilson bounds of a rate from samples are rounded to, the low bound down and the high bound up (`formats.md`, "Random rules"); at most 100 |
| `unknowns.drop_z` | `"1.959964"` | the z of the Wilson bounds (95 %): a decimal string above 0, digits with an optional fraction, never a JSON number |
| `random.max_transitions` | 1000000 | the most transitions (pulls × pity counts × guarantee states) a pity chain may need; beyond it is `expr_eval_failed` (`budget_exceeded`) |

All the numbers are integers of at least 1, unless their row says otherwise (`unknowns.samples` and
`unknowns.seed` may be 0).

```json
{"schema_version": "acmodel.config.v1", "paths": {"venv": "<absolute path>"}, "compute": {"blas_threads": 1}, "expr": {"max_bits": 127}}
```

## Cache

`run` computes the derivation graph through a cache under `paths.state_root` (`src/acmodel/cache/`, #424
design section 5): a node is computed again only when something it read has changed.

**Layout** (`<state_root>` is `paths.state_root`):

| Path | Holds |
|---|---|
| `cache/index.sqlite` | the index (sqlite3, WAL): one row per computation of a node, its trace |
| `cache/blobs/<first two hex digits>/<value hash>` | each node value as canonical JSON, named by its sha256 and shared by every trace with that value |
| `runs/<run id>/outputs.json` | a run's outputs: `schema_version` `acmodel.run.v1`, `run_id`, `data` (the run's `data` without `run_id` and `work`) and `errors`, as canonical JSON |

Only the main process writes. Canonical JSON here means sorted keys, no whitespace and UTF-8; it is used for
hashing and never depends on Python's string-hash randomisation.

**A node's trace** is everything its computation read, in the order it read it, each with its value hash:
- the graph nodes it read;
- the leaves, hashed from this run's inputs: each declaration it consulted (with its file, its JSON pointer
  and whether it is active); each data row; each state reading's **value** (not `observed_at_unix_ms` or
  `source`, so a fresh reading of the same value recomputes nothing); `cleared`; each state entity
  attribute and the set of entities of a class; and the parts of the model's structure the evaluation
  consults: the typed expression, the threshold's entry in `exports.json`, a table's columns and the key
  cells of its rows, the flows behind `cost`, `yield` and `max_single_income`, a store's storage cap, and
  which unknown an `unknown:` reference names.

A state value is part of the trace whether it is present or not, so adding a missing reading recomputes
what needed it.

**Keys.**
- A node's **spec hash** is the sha256 of `acmodel.node.v1`, the node kind, that kind's evaluator version,
  and the node spec: the node, `expr.max_bits`, and the fields a value is kept with. Each kind has its own
  evaluator version (`EVALUATORS` in `cache/keys.py`), with the versions of the libraries it declares
  (`python` minor version, `numpy`, `scipy`; none of the derivation kinds declares one). So bumping one
  kind, or a library one kind declares, recomputes only the nodes of that kind; their dependents are reused
  when the value comes out the same.
- An **engine call** is a node of kind `engine` (see Engine host): its spec is the engine id, the engine's
  fingerprint (which covers the library versions), the op and the request without its `entities`; its
  trace is the hash of each entity the result's `depends_on` names.
- **Sampled and impacts nodes.** The Monte Carlo tallies of an output (kind `sampled`, `formats.md`,
  "Unknowns") declare `numpy` and the `python` minor version, and their spec adds the keys under `unknowns`
  and `sensitivity` but `chunk_size`, which changes no result. The impacts of the continuous unknowns of an
  output that computes with more than one (kind `impacts`) add `unknowns.scale`. The trace of either is
  everything its computations read: the output itself, and each node and leaf that the computation at the
  samples or at the held ends reads, so a change inside a distribution that leaves its enclosure as it is still
  computes the samples again.
- **Random processes** (`formats.md`, "Random rules"). A node that computes one itself adds to its spec the keys
  it reads: `unknowns.wilson_digits` and `unknowns.drop_z` for the Wilson bounds of a `beta_from_samples` unknown
  or of a drop from samples, `random.max_transitions` for a pity chain. They declare no library.
- A **node key** is the sha256 of the spec hash and the sorted (input, value hash) pairs of its trace.

**Verifying traces.** A node is reused when one of its traces, the most recently used first, still holds:
every input, in the order read, hashes as recorded. The check stops at the first input that differs, so an
input is only brought up to date when the node computed again would read it too. Otherwise the node is
computed again and its trace and value are kept.

**Early cutoff.** A node computed again whose value hashes as before leaves the traces of the nodes that read
it intact, so they are reused.

**Run id** = the sha256 of `acmodel.run.v1`, the tool (name, version, the evaluator versions) and the run's
inputs: the rules version (`model.json`, `sources.json` and the rule files) and the data version (the data
files), each over [path relative to the model folder, document] sorted by path; `exports.json`; the whole
state snapshot; the preference defaults when the model reads them; the engine fingerprints (none yet);
`expr.max_bits`, `sensitivity.top_n`, the keys under `unknowns` but `chunk_size`, and `random.max_transitions`; and the paths of the model and the state,
which the outputs name. Equal inputs give equal run ids.

**Work counters** (`data.work` of `run`): `nodes` (computed plus reused), `computed`, `reused`, `cutoff`
(computed again to the same value), `corrupt` (corrupt entries met), `evicted` (`entries`, `bytes`), and
`by_kind`: `computed`, `reused` and `cutoff` per node kind.

**Corrupt entries.** A blob is used only after its bytes hash to its name. A blob that does not, is absent or
cannot be read, and an index row whose reads do not read, is `cache_entry_corrupt`: `run` reports it in
`warnings`, never serves it, computes the node again and keeps the new value. `cache verify` reports each
such entry as an error.

**Size.** After a run, while the values the index names exceed `cache.max_bytes`, the least recently used
entries are evicted (never those the run used), and so are the blobs no remaining entry names; the run
reports it in `work.evicted` and on stderr. `cache gc` plans the same without a run, plus the orphaned blob
files, and removes them only with `--apply`. Nothing outside the cache folder is ever removed.

**Failures.** A cache that cannot be opened, read or written is `cache_unavailable` (exit 1); the command
stops and never computes without its cache.

## Host caps

The host layer is `src/acmodel/host/`. The commands that compute call it before and during their work;
`setup` and `version` do not. `run`, `explain`, `export thresholds` and `bindings` in this version evaluate
only the derivation graph, in their own process and with no workers, and do not call it yet.

**The caps of the tool's own process** (Windows; `host.windows`):
1. `SetPriorityClass` to `compute.priority`, then `GetPriorityClass` must read it back. Otherwise the
   result is `host_priority_not_applied` (exit 4). Every kernel32 function is called with typed
   `argtypes` and `restype`.
2. A new Job object gets the limits `compute.job.worker_memory_mib` per process and the priority class
   (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION`), and the hard CPU cap `compute.job.cpu_rate_milli`
   (`JOBOBJECT_CPU_RATE_CONTROL_INFORMATION`).
3. The tool joins the Job itself. It then reads the limits and its membership back
   (`QueryInformationJobObject`, `IsProcessInJob`), and its priority once more. Any failure is
   `host_job_not_applied` (exit 4).
4. Every process started afterwards is in the Job from its start and inherits the priority class. One
   such process can be checked by its process id (`verify_child`).

On another platform there is no real host layer: `host_priority_not_applied` with `stage`
`platform_unsupported`.

**The gate** (`host.gate`) is checked before every batch (`start`) and at checkpoints inside one
(`checkpoint`). Each reason that holds it is one `host_gate_paused` entry (exit 4):

| Reason | Holds when |
|---|---|
| `memory_below_start` | before a batch, or while the gate is paused: available memory below `host.memory.start_min_mib` |
| `memory_below_pause` | at a checkpoint inside a batch: available memory below `host.memory.pause_below_mib` |
| `quiet_window` | the local time is inside a window of `host.quiet_windows` (`window` is its index) |
| `pause_file` | `host.pause_file` exists |
| `cpu_load` | the host CPU load over the last span of at least `host.cpu.window_ms` is above `host.cpu.pause_load_milli`; the first span starts when the gate is created, and until it completes there is no CPU reading |
| `plan_above_report` | before a run: workers × (the recorded peak or the engine's estimate, at most `compute.job.worker_memory_mib`) + the main process is above `host.memory.report_plan_above_mib`; `estimate_mib` is that sum |

- A one-shot command ends with exit 4 when the gate holds (`require`). A later run resumes.
- A long-lived command waits instead (`wait`): it checks again every `host.pause_poll_ms` until no reason
  holds.
- A reading that fails (available memory, CPU times, the pause file other than by its absence) is
  `host_reading_failed` (exit 4). The gate never guesses.

**Quiet windows.** Each window has `start` and `end` as local `HH:MM` and exactly one of `days` (from
`mon`, `tue`, `wed`, `thu`, `fri`, `sat`, `sun`) or `dates` (`YYYY-MM-DD`):
- A window holds from `start` (inclusive) to `end` (exclusive) on its days.
- An `end` at or before `start` ends the next day; that part belongs to the day the window started.
- `start` and `end` must differ.

```json
{"host": {"quiet_windows": [{"days": ["mon", "thu"], "start": "04:40", "end": "05:40"},
                            {"dates": ["2030-01-02"], "start": "21:20", "end": "00:00"}]}}
```

Host facts such as the restart window or a deploy evening belong in the local configuration, not in
code. The `PAUSE` file covers one-off pauses.

## Engine host

The engine host is `src/acmodel/engine/` (#424 design-py section 7). It runs the battle and team engines,
which live outside this repository, in the tool's own worker processes; the main process never imports
engine code. The manifest `engine.json` and the interface `acmodel.engine.v2` are in `formats.md`
("Engines"). No command uses engines yet; the search (PY-13) and `watch` (PY-15) will.

**Where an engine is.** `engines.roots` maps the engine id to its home, the folder with `engine.json`. An
engine the configuration does not name is `engine_not_configured` (exit 3); the tool never guesses.

**Workers.**
- **Start.** `<python> -P -s -B <model>/src/acmodel/engine/worker.py`, where `<python>` is the base
  interpreter of the running Python (in a venv on Windows, `Scripts\python.exe` is a launcher that runs it
  as a child), which adds the venv's site directories itself.
- **Environment.** Explicit; nothing else is inherited: `PYTHONHASHSEED` (`engine.hash_seed`; the verify
  worker `engine.verify_hash_seed`), `OPENBLAS_NUM_THREADS`, `OMP_NUM_THREADS` and `MKL_NUM_THREADS`
  (`compute.blas_threads`), `ACMODEL_SCRATCH`, `TEMP`, `TMP` and `TMPDIR` inside the scratch directory, and
  `SYSTEMROOT` where the host has it. The worker is started without `-I` and `-E`, so the seed holds.
- **Scratch and log.** Each worker's working directory is `<scratch root>/<engine id>/<slot>`, the only place
  it may write; its stdout and stderr go to `<log dir>/<engine id>-<slot>.log`, with the tracebacks of the
  engine's exceptions. Slots are `w0`, `w1`, … and `v` for the verify worker.
- **Caps.** The tool joins its Job before it starts workers (Host caps), so each worker is inside the Job and
  below normal from its start. The host checks that for every worker before it runs engine code
  (`verify_child`; `host_job_not_applied` or `host_priority_not_applied`, exit 4). At most
  `compute.workers.max` workers are alive at once.
- **Channel.** `multiprocessing.connection`: the worker listens on an address and with an authkey that are
  new for each worker (a named pipe on Windows, an abstract Unix socket on Linux), handed over on its stdin;
  the host connects. Frames are pickles of builtin values between the tool's own processes; results cross as
  JSON text. Nothing an engine prints reaches the channel. stdin stays open as the worker's lifeline: a worker
  whose tool is gone exits.
- **Init.** The worker puts the manifest's `sys_path` first, keeps bytecode caches in its scratch directory,
  installs the guards, imports `entry` and calls `make_engine(config, scratch_dir)` and `describe()`. An
  entry that does not import, or a `make_engine` that raises, is `engine_import_failed` (exit 3). A
  `describe` that disagrees with `engine.json` is `engine_protocol_error` (`describe_invalid`).

**Calls.** A call's requests go to the workers in batches of `max_batch` (`search`: one at a time); the
outcomes come back in request order. With `engine.reset_each_batch`, `reset()` follows every batch.
- A domain failure of one request is its `engine_failed` (`adapter_error`), with the adapter's code in
  `engine_code`.
- An exception fails every request of the batch (`engine_failed`, `exception`, with its type), and that
  worker is not used again. A result that is not JSON or not of its shape is that request's
  `engine_protocol_error`. The call goes on with the other batches.
- A batch has `engine.request_timeout_ms` per request; past it the worker is stopped and the batch's requests
  are `engine_failed` (`timeout`). A worker out of memory at the Job's limit gives `worker_memory_limit`.
- **Guards** (`engine_guard_violation`, exit 3; the call fails and every worker is stopped): a write outside
  the scratch directory, a read of a file under the home that `code` and `data` do not list, a module under
  the home that `code` does not list, and a change of `sys.argv`, the working directory, `sys.path` or
  `os.environ` by a stage. An audit hook sees the file operations and refuses them; the module closure and
  the global state are checked after the import and after every batch.

**Restarts and recycling.** A worker that exits without answering has crashed: it is replaced and its batch
runs again. A crash, a timeout and the memory limit each use one of `engine.restart_budget` restarts of the
pool; past them, `engine_failed` (`restart_budget_exhausted`, exit 3), and every worker is stopped. A worker
is recycled, without using the budget, after `engine.worker_max_requests` requests, when its private bytes
exceed `engine.worker_soft_memory_mib` after a batch, and after an exception. The host stops only workers it
started, by their process handle.

**Cache.** An engine call goes through the cache as a node of kind `engine` (see Cache): it is reused while
the engine's fingerprint, the op, the request without `entities`, and every entity its result's `depends_on`
names hash the same. Errors are never kept.

**Verify sample.** `engine.verify_sample_milli` of a call's requests, chosen by the sha256 of [fingerprint,
op, request], run again with the full request, after `reset()`, in a fresh worker with
`engine.verify_hash_seed`, cached results included. Any other outcome is `engine_nondeterministic` (exit 3):
hash or set order, state carried between calls, or an untruthful `depends_on`. The call fails, and nothing
of it is kept in the cache.
