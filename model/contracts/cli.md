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
| `run <model> --state <file>` | Runs the dependency check, the static check, the run's model checks and the derivation graph for the account state `<file>` (`formats.md`, "The run"). It reads and computes, never writes. A model or input that does not check is refused whole (exit 1, `data` `{}`). Otherwise every output is computed or blocked: a blocked output's error lists the outputs it blocks in `needed_by`, the other outputs are still in `data`, and the exit status is 1. Provisional outputs alone exit 0. | `path`, `model_id`, `state`, `rules`, `thresholds`, `notes` |
| `explain <model> <output> --state <file>` | Runs as `run` does and prints the provenance document of one output (`rule:<id>`, `general_logic:<id>` or `threshold:<id>`; `formats.md`, "Outputs"). It writes nothing. An id that is not an output is `usage_invalid` (`output_unknown`); a blocked output reports the errors that block it (exit 1). | `path`, `model_id`, `state`, `output`, `provenance` |
| `export thresholds <model> --state <file>` | Runs as `run` does, then writes `out/thresholds.json` and one provenance file per threshold into the model folder (`formats.md`, "Outputs"). Only account-independent, confirmed, integer thresholds are exported: any other is `export_refused` (exit 5), and then nothing is written. A per-account value never enters these shared files. | `path`, `model_id`, `state`, `written` (the files written) |

`export` is a group: its command is two words (`export thresholds`), and the envelope's `command` is both.
Every argument and option a command names is required, and an option takes one value. A missing one is
`usage_invalid` (`argument_missing`, with the argument's or option's name), and an extra or repeated one is
`usage_invalid` (`argument_unexpected`).

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
| `warnings` | list | entries of the same shape |

**Error entries.**
- `code` is a code registered in `codes.json`.
- `detail` holds that code's typed values only, never prose.
- `ref` is a reference into the input when there is one: for `config_invalid`, the JSON pointer of the
  member, with the file in `detail.path`; for the model's codes, the JSON pointer into the file in
  `detail.path`.
- `needed_by` lists the outputs an error blocks (`run`, `explain`, `export thresholds`: `rule:<id>`,
  `general_logic:<id>`, `threshold:<id>`), sorted.

Human text, one line per error and the usage text, goes to stderr.

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
| `paths.state_root` | `%LOCALAPPDATA%\ActingCommand\acmodel\state` | the state root (used by later commands); an absolute path |
| `paths.datasets` | `%LOCALAPPDATA%\ActingCommand\acmodel\datasets` | the local copies of external datasets, each the file named by its sha256 (`check` verifies them); an absolute path |
| `compute.blas_threads` | 1 | the value of the BLAS thread variables; an integer of at least 1 |
| `expr.max_bits` | 127 | the most bits a numerator or denominator may have in expression evaluation (durations in seconds); beyond it is `expr_overflow`; an integer of at least 1 |
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

All the numbers are integers of at least 1.

```json
{"schema_version": "acmodel.config.v1", "paths": {"venv": "<absolute path>"}, "compute": {"blas_threads": 1}, "expr": {"max_bits": 127}}
```

## Host caps

The host layer is `src/acmodel/host/`. The commands that compute call it before and during their work;
`setup` and `version` do not. `run`, `explain` and `export thresholds` in this version evaluate only the
derivation graph, in their own process and with no workers, and do not call it yet.

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
