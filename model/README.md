# acmodel

`acmodel` is the game-neutral modelling tool of ActingCommand. It is an offline Python 3.13 program that
runs on its own: there is nothing to build, and it does not depend on the console, the MCP server or the
Runtime.

This folder holds the skeleton: the launcher, `setup`, `version`, the output envelope and the dependency
check. The computation commands come in later slices.

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
```

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
  "paths": {"venv": "<absolute path>", "state_root": "<absolute path>"},
  "compute": {"blas_threads": 1},
  "expr": {"max_bits": 127}
}
```

- **Strict reading.** Unknown members, duplicate keys and values of the wrong type are refused
  (`config_invalid`), never ignored.
- **State root.** The default for `paths.state_root` is `%LOCALAPPDATA%\ActingCommand\acmodel\state`.
- **Exact arithmetic.** Expressions compute with exact fractions. `expr.max_bits` (default 127) bounds
  every numerator and denominator; a larger one is `expr_overflow`, never wrapped or rounded.

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
                      formats.md (the input files and their strict reading)
src/acmodel/          the package; model/ holds the strict readers of the input files
tests/                pytest tests with fakes
```
