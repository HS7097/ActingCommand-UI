# acmodel

`acmodel` is the game-neutral modelling tool of ActingCommand. It is an offline Python 3.13 program that
runs on its own: there is nothing to build, and it does not depend on the console, the MCP server or the
Runtime.

This folder holds the launcher, `setup`, `version`, the output envelope, the dependency check, the strict
readers of the model files, the expression language, the static check (`check`), the derivation graph
(`run`) with its unknowns and their sensitivity and its random processes (exact pity chains, Wilson bounds of
rates from samples), its cache with incremental recompute (`cache`), provenance
(`explain`), the exports (`export thresholds`, `export tables`, `export scoring`) with their export policy, the
bindings of thresholds to the schedule
(`bindings check`, `bindings apply`), the economy's LP and MILP (`economy`) and the engine host, which runs
engines kept outside this repository in worker processes of its own. The other computation commands come in later slices.

## Needs

- Python 3.13. On Windows that is the `py` launcher with a 3.13 installed.
- Network access to PyPI, once, for `setup`.

## Set up (once)

```text
py -3.13 -I -B model\acmodel.py setup
```

`setup` creates a virtual environment from that Python 3.13 and installs only the hash-pinned wheels of
`requirements.txt` (numpy, scipy), with no source builds and no dependency resolution.
- **Default venv location:** `%LOCALAPPDATA%\ActingCommand\acmodel\venv` (about 150 MB). It can be
  changed with `paths.venv` (see Configuration).
- **Running it again** reuses a 3.13 venv and runs the install again.
- **It never deletes anything.** It refuses a venv of another Python minor version, and a path that is
  not a venv. To start over, remove the venv folder yourself and run `setup` again.

## Use

```text
py -3 -I -B model\acmodel.py version
model\acmodel.cmd version
model\acmodel.cmd check <resource repo>\ours\model
model\acmodel.cmd run <resource repo>\ours\model --state <state snapshot>
model\acmodel.cmd economy <resource repo>\ours\model --state <state snapshot>
model\acmodel.cmd explain <resource repo>\ours\model threshold:<id> --state <state snapshot>
model\acmodel.cmd export thresholds <resource repo>\ours\model --state <state snapshot>
model\acmodel.cmd export tables <resource repo>\ours\model --state <state snapshot> --repo <resource repo>
model\acmodel.cmd export scoring <resource repo>\ours\model --state <state snapshot>
model\acmodel.cmd bindings check <resource repo>\ours\model --state <state snapshot> --strict
model\acmodel.cmd bindings apply <resource repo>\ours\model --state <state snapshot>
model\acmodel.cmd cache stats
model\acmodel.cmd cache verify
model\acmodel.cmd cache gc [--apply]
```

- **`check`** reads a model (its folder or its `model.json`) and checks it without computing anything:
  references, duplicate ids, statuses, provenance, `supersedes` and conflicts, pending rules, where
  `pref:` may appear, loops, and the hashes of external datasets (`contracts/formats.md`).
- **`run`** checks the model, then the dimensions of its expressions and its preference keys, and
  computes its derived values and thresholds for one account state, exactly. An output that cannot be
  computed (a reading the state lacks, say) is blocked with its error; the others are still computed, and
  the exit status is 1 (`contracts/formats.md`, "The run").
- **Unknowns.** An output that reads a variants or empirical unknown is computed at each of its points; an
  interval or a continuous distribution is computed by exact interval arithmetic. Its range is a sound
  enclosure, never narrower than the true one: exact where a structural monotonic check proves it, and
  marked otherwise. Only an output whose range moves at milli is provisional for its unknowns, and it lists
  the unknowns with the largest impact. Distributions can also be sampled (`unknowns.samples`),
  deterministically (`contracts/formats.md`, "Unknowns").
- **Cache.** `run` keeps every computed node under the state root and computes again only what a change
  reaches: a node is reused while everything it read still hashes the same, and a node that comes out the
  same stops the change there. Each run gets a run id from its inputs, its outputs are kept under
  `runs/<run id>/`, and `data.work` counts what was computed and what was reused. `cache stats` counts
  the cache, `cache verify` checks every value against its hash, and `cache gc` trims it to
  `cache.max_bytes` (a dry run without `--apply`). A corrupt value is never used: `run` warns
  (`cache_entry_corrupt`) and computes it again (`contracts/cli.md`, "Cache").
- **`economy`** solves the economy with scipy's HiGHS (`contracts/formats.md`, "The economy"):
  - **the steady state**, game-wide: an LP over one cycle (a week, or a day), with the price of every resource
    per step and of every cap and limit, exact or an interval, by re-solves one grain up and down; the value of
    each activity; and the least overflow and lost production;
  - **the plan** of one account: a MILP from the state's readings over `economy.plan_horizon` days, with integer
    counts and read caps that block collections at or above them, its final stocks priced by the steady state,
    stopped only by `search.milp_node_limit` (then `proven: false`).
  Every solution is made exact and checked exactly (`lp_numeric_check_failed`); contradictory rules are named
  (`lp_infeasible`); what the programs do not model yet is refused, never approximated.
- **`explain`** prints the provenance tree of one output (`rule:<id>`, `general_logic:<id>` or
  `threshold:<id>`): every rule, data row, ruling and reading its value rests on, with their hashes and
  the model's versions. It writes nothing; like `export thresholds`, it computes without the cache.
- **`export thresholds`** writes `out/thresholds.json` and one provenance file per threshold into the
  model folder, for delivery with the resource repository. It never commits. Only account-independent
  values go into these shared files: a threshold that read the account state, is not confirmed, is not an
  integer, or is blocked is refused (`export_refused`, exit 5), and then nothing is written
  (`contracts/formats.md`, "Outputs").
- **The export policy** decides what a delivered file takes from an output: a confirmed value as it is; a
  provisional one only when `acceptances` of exports.json accept it (a table cell is `null` otherwise, the
  others are refused); a blocked or per-account one never. A number is scaled and rounded as declared
  (`floor`, `ceil`, `half_even`), and each output a file delivers gets its provenance file in the model folder,
  equal to what `explain` prints.
- **`export tables`** writes the tables of exports.json into the repository's `tables/` as the Runtime's shared
  tables, in canonical JSON (keys in UTF-16 order, integers only), and runs the Runtime's own validator,
  `actinglab --json resource validate`, on each (`paths.actinglab`). Without a validator nothing is written; when
  it refuses a table, every file gets its previous bytes back (`export_table_invalid`, exit 5, with the
  validator's problem and JSON pointer). It never commits.
- **`export scoring`** fills each scoring template with the model's numbers; one placeholder it cannot fill
  refuses the whole document. Until the place of `scoring.json` and its validator are decided, it writes nothing
  (`export_refused`, `validator_path_unknown`).
- **`bindings check`** compares each literal that `ours/model/bindings.json` ties to a threshold, in the
  repository's `schedule.json` or a pack file, with the value `export thresholds` would write. With
  `--strict` it also refuses every number of a `fact` predicate of `schedule.json` that is neither bound
  nor listed as fixed with a reason, so no number without provenance reaches the schedule. It writes
  nothing.
- **`bindings apply`** writes the bound literals that differ, and changes only their bytes: line ends
  (CRLF files stay CRLF) and formatting are kept. It refuses what `check` refuses, other than a differing
  value, and then writes nothing (`contracts/formats.md`, "Bindings").
- **Re-dispatch.** The launcher re-runs itself under the venv's Python, and the exit status is passed
  through.
- **Dependency check.** Before every command except `setup`, the launcher checks that the interpreter
  is 3.13 and that numpy and scipy are at their pins. If not, it fails with `deps_invalid` and names the
  `setup` command.
- **Output.** Every run prints one JSON envelope (`acmodel.cli.v1`) on stdout; human text goes to
  stderr.
- **Exit statuses:** 0 ok, 1 model or input error, 2 usage, 3 engine failure, 4 host gate, 5 export
  refused, 6 regression or binding mismatch.

See `contracts/cli.md` for the envelope and the start sequence, and `contracts/codes.json` for the
codes.

## Configuration

The file is `%LOCALAPPDATA%\ActingCommand\acmodel\config.json`. It is optional, and every key has a
default:

```json
{
  "schema_version": "acmodel.config.v1",
  "paths": {"venv": "<absolute path>", "state_root": "<absolute path>", "datasets": "<absolute path>"},
  "compute": {"blas_threads": 1},
  "expr": {"max_bits": 127}
}
```

- **Strict reading.** Unknown members, duplicate keys and values of the wrong type are refused
  (`config_invalid`), never ignored.
- **State root.** The default for `paths.state_root` is `%LOCALAPPDATA%\ActingCommand\acmodel\state`. It
  holds the cache (`cache/`) and the runs' outputs (`runs/`), never inside a repository.
- **Cache size.** `cache.max_bytes` (default 2 GiB) caps the cache; the least recently used entries are
  evicted after a run.
- **External datasets.** `paths.datasets` (default `%LOCALAPPDATA%\ActingCommand\acmodel\datasets`) holds
  the local copies of large datasets, each the file named by its sha256. `check` verifies them and never
  fetches.
- **Exact arithmetic.** Expressions compute with exact fractions. `expr.max_bits` (default 127) bounds
  every numerator and denominator; a larger one is `expr_overflow`, never wrapped or rounded.
- **Host caps.** The keys under `compute` (`priority`, `job`) and `host` (`memory`, `cpu`,
  `quiet_windows`, `pause_file`, `pause_poll_ms`) cap the tool on the host. Their defaults:
  - below-normal priority, set and read back;
  - a Job object with 1024 MiB per process and a 25 % CPU cap;
  - a batch starts at 4096 MiB of available memory or more, pauses below 2560 MiB and resumes at 4096 MiB;
  - a run planned above 3072 MiB is reported first (exit 4);
  - no compute in a quiet window or while the `PAUSE` file exists (default `<state root>\PAUSE`);
  - a pause while the host CPU load is above 90 %.

  `contracts/cli.md` (Host caps) has the details.
- **Engines.** An engine is Python code in its own home outside this repository, with a manifest
  `engine.json` (`contracts/formats.md`, "Engines"). `engines.roots` maps each engine id to its home on this
  host; nothing in this repository names one. The engine host runs an engine only in worker processes of the
  tool, which inherit the Job and the priority:
  - at most `compute.workers.max` (default 2) at once;
  - with an explicit environment and a fixed hash seed (`engine.hash_seed`);
  - with guards against writes outside their scratch directory, files and modules the manifest does not
    list, and changes of `sys.argv`, the working directory, `sys.path` or the environment;
  - with timeouts, a restart budget, recycling, and a verify sample run again under another hash seed.

  Engine results are cached by the engine's fingerprint (its manifest, its files and the library versions).
  The keys are under `engine` and `engines`; `contracts/cli.md` (Engine host) has the details.
- **Unknowns.** `sensitivity.top_n` (5) unknowns listed per output, and the keys under `unknowns`: `scale`
  (1000, milli) the export precision of the no-effect rule and the impacts, and the Monte Carlo sampling of
  distributions: `samples` (0, off), `seed` (0), `chunk_size` (1024) and `resolution` (1000000).
- **Random processes.** The Wilson bounds of a rate from samples use `unknowns.drop_z` (`"1.959964"`, a decimal
  string: 95 %) and `unknowns.wilson_digits` (6), the low bound rounded down and the high bound up; a pity chain
  may need at most `random.max_transitions` (1000000) transitions; a random rule's value beyond `expr.max_bits`
  is carried as its enclosure to `random.enclosure_digits` (12) decimals (`contracts/formats.md`, "Random
  rules").
- **Economy.** `lp.max_denominator` (1000000) bounds the fractions a solver's floats become, `lp.grain` (1) is the
  step of the price re-solves, `search.milp_node_limit` (100000) the plan's only budget, and `economy.plan_horizon`
  (7) its days.
- **Exports.** `paths.actinglab` (no default) is the Runtime's validator that `export tables` runs, an
  `actinglab` from a Runtime build or release; `export.validator_timeout_ms` (120000) bounds each run of it.

## Tests

CI (`.github/workflows/model.yml`) runs pytest on Windows and Ubuntu with Python 3.13. It installs the
hash-pinned `requirements-dev.txt` and runs from this folder:

```text
python -I -B -m pip install --require-hashes --only-binary=:all: --no-deps -r requirements-dev.txt
python -I -B -m pytest -c pytest.ini -p no:cacheprovider
```

## Layout

```text
acmodel.py            launcher (standard library only)
acmodel.cmd           Windows wrapper for the launcher
requirements.txt      numpy, scipy: exact pins with sha256 hashes
requirements-dev.txt  the above plus pytest and its dependencies, hashed
pytest.ini            test paths; src on the import path
contracts/            cli.md (envelope, start, configuration), codes.json (the codes),
                      formats.md (the input files, their strict reading, the static check,
                      the run, the economy and the outputs)
src/acmodel/          the package: model/ the strict readers, expr/ the expression language,
                      check/ the static check, ids.py the id grammars they share, core/ the
                      run (its model checks, the derivation graph, the unknowns, provenance and the
                      versions of the inputs, and flow.py the economy's LP and MILP), cache/ the
                      cache (keys, sqlite3 index and blobs, verifying traces, run ids,
                      stats/verify/gc), export/ the exports (thresholds, tables with their JCS
                      writer and validator call, scoring templates, the export policy), bindings/
                      the bindings check and apply with their position-tracking scanner, host/ the
                      host caps (priority, Job object, memory gates, quiet windows, PAUSE file),
                      engine/ the engine host (manifest and fingerprint, workers, guards, the cached
                      call and the verify sample)
tests/                pytest tests with fakes; fakes/ holds the neutral fake engine and the fake
                      validator script
```
