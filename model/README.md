# acmodel

`acmodel` is the game-neutral modelling tool of ActingCommand. It is an offline Python 3.13 program that
runs on its own: there is nothing to build, and it does not depend on the console, the MCP server or the
Runtime.

This folder holds the launcher, `setup`, `version`, the output envelope, the dependency check, the strict
readers of the model files, the expression language, the static check (`check`), the derivation graph
(`run`) and its cache with incremental recompute (`cache`). The other computation commands come in later
slices.

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
- **Cache.** `run` keeps every computed node under the state root and computes again only what a change
  reaches: a node is reused while everything it read still hashes the same, and a node that comes out the
  same stops the change there. Each run gets a run id from its inputs, its outputs are kept under
  `runs/<run id>/`, and `data.work` counts what was computed and what was reused. `cache stats` counts
  the cache, `cache verify` checks every value against its hash, and `cache gc` trims it to
  `cache.max_bytes` (a dry run without `--apply`). A corrupt value is never used: `run` warns
  (`cache_entry_corrupt`) and computes it again (`contracts/cli.md`, "Cache").
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
                      formats.md (the input files, their strict reading and the static check)
src/acmodel/          the package: model/ the strict readers, expr/ the expression language,
                      check/ the static check, ids.py the id grammars they share, core/ the
                      run (its model checks and the derivation graph), cache/ the cache
                      (keys, sqlite3 index and blobs, verifying traces, run ids, stats/verify/gc),
                      host/ the host caps (priority, Job object, memory gates, quiet windows,
                      PAUSE file)
tests/                pytest tests with fakes
```
