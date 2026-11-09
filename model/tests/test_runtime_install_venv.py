"""`paths.venv` is never a Runtime install root nor inside one either (#424, 2026-11-09; contracts/cli.md, "Never in a
Runtime install root").

`setup` creates the venv there and installs into it, and every other command runs the venv's Python. The venv is checked
with the state root when the configuration is read: one in an install root is `config_invalid` (`runtime_install_root`,
ref `/paths/venv`), so no command runs, no child is started and no venv is made. Each fake install root is a folder
under `tmp_path` that holds one mark (`test_runtime_install`); no path of a machine is used, and the runner is a fake
that makes a venv as `python -m venv` would, so nothing is installed.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from acmodel import cli, environment
from test_runtime_install import DECOYS, MARKS, fake_install_root, snapshot

SCHEMA = "acmodel.config.v1"


def fake_venv_module(cmd: list[str]) -> None:
    """What `<python> -I -B -m venv <path>` leaves: the venv's Python and its pyvenv.cfg."""
    if cmd[3:5] == ["-m", "venv"]:
        target = Path(cmd[5])
        python = environment.venv_python(target, sys.platform)
        python.parent.mkdir(parents=True)
        python.write_bytes(b"")
        (target / "pyvenv.cfg").write_text("version = 3.13.15\n", encoding="utf-8")


def refused(app_dir: Path, key: str, folder: Path, root: Path, mark: str) -> dict:
    return {
        "code": "config_invalid",
        "ref": f"/paths/{key}",
        "detail": {
            "reason": "runtime_install_root", "path": str(app_dir / "config.json"), "key": key, "folder": str(folder),
            "root": str(root), "marker": mark,
        },
    }


@pytest.fixture
def acmodel(make_system, write_config):
    """`acmodel <argv>` outside the venv, with the given `paths`: (exit status, the system)."""

    def call(*argv: str, paths: dict[str, Path]):
        write_config({"schema_version": SCHEMA, "paths": {key: str(value) for key, value in paths.items()}})
        system = make_system(on_run=fake_venv_module)
        return cli.main(list(argv), system), system

    return call


@pytest.mark.parametrize("mark", MARKS)
def test_setup_refuses_a_venv_in_an_install_root_and_makes_no_venv(tmp_path, app_dir, acmodel, check_envelope, mark):
    """`paths.venv` two folders below the fake root: `setup` is refused when the configuration is read, before the venv
    would be made; no child runs, and nothing is written into the fake root."""
    root = fake_install_root(tmp_path, mark)
    venv = root / "acmodel" / "venv"
    before = snapshot(root)

    status, system = acmodel("setup", paths={"venv": venv})

    assert system.run.calls == []
    doc = check_envelope(system.stdout.getvalue(), status)
    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [refused(app_dir, "venv", venv, root, mark)]
    assert not venv.exists()
    assert snapshot(root) == before


def test_a_venv_already_in_an_install_root_is_not_run_either(tmp_path, app_dir, acmodel, check_envelope):
    """A venv that already lies in the fake root: a command other than `setup` is refused the same way, before the
    re-dispatch would start the venv's Python."""
    mark = "runtime/BUILD-MANIFEST.json"
    root = fake_install_root(tmp_path, mark)
    venv = root / "acmodel" / "venv"
    fake_venv_module(["host-python", "-I", "-B", "-m", "venv", str(venv)])
    before = snapshot(root)

    status, system = acmodel("version", paths={"venv": venv})

    assert system.run.calls == []
    doc = check_envelope(system.stdout.getvalue(), status)
    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [refused(app_dir, "venv", venv, root, mark)]
    assert snapshot(root) == before


def test_a_state_root_and_a_venv_in_an_install_root_are_one_error_each(tmp_path, app_dir, acmodel, check_envelope):
    """Both in the fake root: one `config_invalid` each, the state root first, and nothing is made."""
    mark = "install/active.json"
    root = fake_install_root(tmp_path, mark)
    state_root = root / "acmodel" / "state"
    venv = root / "acmodel" / "venv"
    before = snapshot(root)

    status, system = acmodel("setup", paths={"state_root": state_root, "venv": venv})

    assert system.run.calls == []
    doc = check_envelope(system.stdout.getvalue(), status)
    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [
        refused(app_dir, "state_root", state_root, root, mark),
        refused(app_dir, "venv", venv, root, mark),
    ]
    assert snapshot(root) == before


def test_a_venv_level_that_cannot_be_looked_at_refuses_setup(tmp_path, app_dir, acmodel, check_envelope, monkeypatch):
    """A level above the venv whose marks cannot be looked at: `setup` is refused (`runtime_install_unchecked`, with the
    exception), and no venv is made."""
    from acmodel import location

    blocked = tmp_path / "blocked"
    looked = location._present

    def present(path: Path) -> bool:
        if Path(path).parent == blocked:
            raise PermissionError(13, "denied", str(path))
        return looked(path)

    monkeypatch.setattr(location, "_present", present)
    venv = blocked / "venv"

    status, system = acmodel("setup", paths={"venv": venv})

    assert system.run.calls == []
    doc = check_envelope(system.stdout.getvalue(), status)
    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [{
        "code": "config_invalid",
        "ref": "/paths/venv",
        "detail": {
            "reason": "runtime_install_unchecked", "path": str(app_dir / "config.json"), "key": "venv",
            "folder": str(venv), "root": str(blocked), "marker": "actingd.config.json",
            "exception_type": "PermissionError",
            "raw_text": str(PermissionError(13, "denied", str(blocked / "actingd.config.json"))),
        },
    }]
    assert not blocked.exists()


def test_a_venv_with_no_mark_on_the_way_up_is_set_up_as_before(tmp_path, acmodel, check_envelope, launcher):
    """Names that are no mark, on the way up or beside it, refuse nothing: `setup` makes the venv and installs."""
    base = tmp_path / "plain"
    for name in DECOYS:
        target = base.joinpath(*name.split("/"))
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(b"{}\n")
    venv = base / "acmodel" / "venv"

    status, system = acmodel("setup", paths={"venv": venv})

    doc = check_envelope(system.stdout.getvalue(), status)
    assert (status, doc["errors"]) == (0, [])
    requirements = launcher.parent / "requirements.txt"
    assert doc["data"] == {"venv": str(venv), "created": True, "requirements": str(requirements)}
    assert [call["cmd"][:6] for call in system.run.calls] == [
        ["host-python", "-I", "-B", "-m", "venv", str(venv)],
        [str(environment.venv_python(venv, sys.platform)), "-I", "-B", "-m", "pip", "install"],
    ]
    assert (venv / "pyvenv.cfg").is_file()
