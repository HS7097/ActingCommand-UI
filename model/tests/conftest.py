"""Fakes for the PY-0 tests: the process surroundings, a runner, and the envelope check."""

from __future__ import annotations

import io
import json
import sys
from pathlib import Path
from typing import Any, Callable, Sequence

import pytest

from acmodel.system import System

MODEL_DIR = Path(__file__).resolve().parents[1]
CODES = json.loads((MODEL_DIR / "contracts" / "codes.json").read_text(encoding="utf-8"))["codes"]
PINNED = {"numpy": "2.5.4", "scipy": "1.18.1"}


class FakeRunner:
    """Records each child command; returns the given exit codes in turn (then 0)."""

    def __init__(self, exits: list[int], on_run: Callable[[list[str]], None] | None) -> None:
        self.exits = list(exits)
        self.on_run = on_run
        self.calls: list[dict[str, Any]] = []

    def __call__(self, cmd: list[str], *, env: dict[str, str], stdout_to_stderr: bool) -> int:
        self.calls.append({"cmd": list(cmd), "env": dict(env), "stdout_to_stderr": stdout_to_stderr})
        if self.on_run is not None:
            self.on_run(list(cmd))
        return self.exits.pop(0) if self.exits else 0


@pytest.fixture
def launcher() -> Path:
    return MODEL_DIR / "acmodel.py"


@pytest.fixture
def local_app_data(tmp_path: Path) -> Path:
    return tmp_path / "localappdata"


@pytest.fixture
def app_dir(local_app_data: Path) -> Path:
    """Where the configuration and the default venv live."""
    return local_app_data / "ActingCommand" / "acmodel"


@pytest.fixture
def write_config(app_dir: Path) -> Callable[[dict[str, Any]], None]:
    def write(doc: dict[str, Any]) -> None:
        app_dir.mkdir(parents=True, exist_ok=True)
        (app_dir / "config.json").write_text(json.dumps(doc), encoding="utf-8")

    return write


@pytest.fixture
def make_system(tmp_path: Path, local_app_data: Path, launcher: Path) -> Callable[..., System]:
    def make(
        *,
        prefix: Path | None = None,
        python_version: tuple[int, int, int] = (3, 13, 15),
        installed: dict[str, str] = PINNED,
        exits: Sequence[int] = (),
        on_run: Callable[[list[str]], None] | None = None,
    ) -> System:
        return System(
            launcher=launcher,
            environ={"LOCALAPPDATA": str(local_app_data)},
            executable="host-python",
            prefix=str(prefix if prefix is not None else tmp_path / "host-prefix"),
            platform=sys.platform,
            python_version=python_version,
            installed=dict(installed).get,
            run=FakeRunner(list(exits), on_run),
            stdout=io.StringIO(),
            stderr=io.StringIO(),
        )

    return make


@pytest.fixture
def check_envelope() -> Callable[[str, int], dict[str, Any]]:
    """Parse stdout as exactly one acmodel.cli.v1 envelope whose codes, values and
    exit status agree with contracts/codes.json; return it."""

    def check(stdout: str, status: int) -> dict[str, Any]:
        lines = stdout.splitlines()
        assert len(lines) == 1, stdout
        doc = json.loads(lines[0])
        assert set(doc) == {"schema_version", "command", "ok", "data", "errors", "warnings"}
        assert doc["schema_version"] == "acmodel.cli.v1"
        assert doc["ok"] is (doc["errors"] == [])
        assert isinstance(doc["data"], dict)
        assert doc["warnings"] == []
        for entry in doc["errors"]:
            assert set(entry) <= {"code", "detail", "ref", "needed_by"}
            values = CODES[entry["code"]]["values"]
            assert set(entry["detail"]) <= set(values), entry
            for key, spec in values.items():
                if spec["required"]:
                    assert key in entry["detail"], (entry, key)
                if "members" in spec and key in entry["detail"]:
                    assert entry["detail"][key] in spec["members"], (entry, key)
        expected = CODES[doc["errors"][0]["code"]]["exit"] if doc["errors"] else 0
        assert status == expected
        return doc

    return check
