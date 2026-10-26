"""PY-0: the launcher as a real process, and the re-dispatch into the venv."""

import os
import subprocess
import sys

import pytest

from acmodel import cli, environment


def test_launcher_under_isolated_mode_from_an_unrelated_cwd_prints_the_envelope(
    tmp_path, local_app_data, write_config, launcher, check_envelope
):
    # This interpreter stands in for the venv: CI installed requirements-dev.txt into it.
    write_config({"schema_version": "acmodel.config.v1", "paths": {"venv": sys.prefix}})
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    env = {key: value for key, value in os.environ.items() if key != environment.DISPATCH_MARK}
    env["LOCALAPPDATA"] = str(local_app_data)

    proc = subprocess.run(
        [sys.executable, "-I", "-B", str(launcher), "version"],
        cwd=elsewhere,
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )

    assert proc.returncode == 0, proc.stderr
    doc = check_envelope(proc.stdout, proc.returncode)
    assert doc["command"] == "version"
    assert doc["data"]["tool"] == "acmodel"
    assert doc["data"]["python"].startswith("3.13.")
    assert doc["data"]["packages"] == {"numpy": "2.5.4", "scipy": "1.18.1"}


@pytest.mark.parametrize("child_status", [0, 5])
def test_redispatch_passes_the_child_exit_code_through(
    tmp_path, write_config, make_system, launcher, child_status
):
    venv = tmp_path / "fake-venv"
    python = environment.venv_python(venv, sys.platform)
    python.parent.mkdir(parents=True)
    python.write_bytes(b"")
    write_config({"schema_version": "acmodel.config.v1", "paths": {"venv": str(venv)}})
    system = make_system(exits=[child_status])  # this process runs outside the venv

    status = cli.main(["version"], system)

    assert status == child_status
    assert system.stdout.getvalue() == ""  # the child prints the envelope
    [call] = system.run.calls
    assert call["cmd"] == [str(python), "-I", "-B", str(launcher), "version"]
    assert call["stdout_to_stderr"] is False
    assert call["env"][environment.DISPATCH_MARK] == "1"
    assert {name: call["env"][name] for name in environment.BLAS_VARIABLES} == {
        "OPENBLAS_NUM_THREADS": "1",
        "OMP_NUM_THREADS": "1",
        "MKL_NUM_THREADS": "1",
    }
