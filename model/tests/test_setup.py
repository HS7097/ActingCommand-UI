"""PY-0: `setup` with a fake runner."""

import sys
from pathlib import Path

from acmodel import cli, environment


def _snapshot(root: Path) -> dict[str, bytes | None]:
    return {
        path.relative_to(root).as_posix(): path.read_bytes() if path.is_file() else None
        for path in sorted(root.rglob("*"))
    }


def test_setup_creates_the_venv_at_the_configured_path_and_installs_hashed_wheels(
    tmp_path, app_dir, write_config, make_system, launcher, check_envelope
):
    venv = tmp_path / "configured" / "venv"
    write_config({"schema_version": "acmodel.config.v1", "paths": {"venv": str(venv)}})

    def fake_venv_module(cmd):
        if cmd[3:5] == ["-m", "venv"]:
            target = Path(cmd[5])
            python = environment.venv_python(target, sys.platform)
            python.parent.mkdir(parents=True)
            python.write_bytes(b"")
            (target / "pyvenv.cfg").write_text("version = 3.13.15\n", encoding="utf-8")

    system = make_system(on_run=fake_venv_module)

    status = cli.main(["setup"], system)

    doc = check_envelope(system.stdout.getvalue(), status)
    assert status == 0
    requirements = launcher.parent / "requirements.txt"
    assert [call["cmd"] for call in system.run.calls] == [
        ["host-python", "-I", "-B", "-m", "venv", str(venv)],
        [
            str(environment.venv_python(venv, sys.platform)), "-I", "-B", "-m", "pip", "install",
            "--require-hashes", "--only-binary=:all:", "--no-deps", "-r", str(requirements),
        ],
    ]
    assert all(call["stdout_to_stderr"] for call in system.run.calls)
    assert doc["data"] == {"venv": str(venv), "created": True, "requirements": str(requirements)}
    assert (venv / "pyvenv.cfg").is_file()
    assert not (app_dir / "venv").exists()


def test_setup_refuses_a_venv_of_another_minor_version_and_deletes_nothing(
    tmp_path, write_config, make_system, check_envelope
):
    venv = tmp_path / "configured" / "venv"
    (venv / "lib").mkdir(parents=True)
    (venv / "pyvenv.cfg").write_text("home = elsewhere\nversion = 3.12.9\n", encoding="utf-8")
    (venv / "lib" / "kept.txt").write_bytes(b"kept")
    before = _snapshot(venv)
    write_config({"schema_version": "acmodel.config.v1", "paths": {"venv": str(venv)}})
    system = make_system()

    status = cli.main(["setup"], system)

    doc = check_envelope(system.stdout.getvalue(), status)
    assert status == 2
    [entry] = doc["errors"]
    assert entry["code"] == "deps_invalid"
    assert entry["detail"]["reason"] == "python_version"
    assert (entry["detail"]["expected"], entry["detail"]["found"]) == ("3.13", "3.12.9")
    assert entry["detail"]["path"] == str(venv)
    assert system.run.calls == []
    assert _snapshot(venv) == before
