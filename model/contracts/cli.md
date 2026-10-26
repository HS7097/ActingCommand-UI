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
| `command` | string or null | the command that ran; null when the command line names no known command |
| `ok` | bool | true exactly when `errors` is empty |
| `data` | object | the command's result; `{}` when it failed |
| `errors` | list | entries `{code, detail, ref?, needed_by?}` |
| `warnings` | list | entries of the same shape |

**Error entries.**
- `code` is a code registered in `codes.json`.
- `detail` holds that code's typed values only, never prose.
- `ref` is a reference into the input when there is one: for `config_invalid`, the JSON pointer of the
  member, with the file in `detail.path`.
- `needed_by` (the outputs an error blocks) is reserved for later commands.

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
| `compute.blas_threads` | 1 | the value of the BLAS thread variables; an integer of at least 1 |

```json
{"schema_version": "acmodel.config.v1", "paths": {"venv": "<absolute path>"}, "compute": {"blas_threads": 1}}
```
