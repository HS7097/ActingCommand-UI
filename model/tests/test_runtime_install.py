"""acmodel never writes into a Runtime install root (#424, 2026-11-09).

`paths.state_root`, and every folder a command writes into (the model folder of the exports and of `bindings apply`, the
repository of `export tables`), must be neither a Runtime install root nor inside one. An install root is recognised by
its marks only (`acmodel.location`), so each fake root here is a folder under `tmp_path` that holds one mark; no path of a
machine is used. A refused command computes nothing and writes nothing.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

import pytest

from acmodel import cli

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
MODEL = "ours/model"
STATE = "state/acct-1.json"
# The marks of a Runtime install root, relative to it (the Runtime's install layout).
MARKS = ("actingd.config.json", "runtime/BUILD-MANIFEST.json", "install/active.json")
# Names that are no mark: a file of a Runtime or UI build at another place than a mark's, and a mark in a folder that is
# not on the way up.
DECOYS = ("BUILD-MANIFEST.json", "active.json", "ui/BUILD-MANIFEST.json", "other/actingd.config.json")
# The commands that write into the model folder; `export tables` writes into its `--repo` too.
WRITERS = (
    "export thresholds", "export prices", "export values", "export valuations", "export tables", "export scoring",
    "bindings apply",
)


def fake_install_root(base: Path, mark: str) -> Path:
    """A folder that holds one mark of a Runtime install root."""
    root = base / "fake-install"
    target = root.joinpath(*mark.split("/"))
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(b"{}\n")
    return root


def snapshot(folder: Path) -> list[tuple[str, bytes | None]]:
    """Every path under `folder`, with the bytes of each file (None for a folder)."""
    return sorted(
        (str(item.relative_to(folder)), None if item.is_dir() else item.read_bytes()) for item in folder.rglob("*")
    )


def example_in(folder: Path) -> Path:
    """A copy of the worked example at `folder`."""
    shutil.copytree(EXAMPLE, folder)
    return folder


def refused(argument: str, folder: Path, root: Path, mark: str) -> dict:
    return {
        "code": "location_refused",
        "detail": {
            "reason": "runtime_install_root", "argument": argument, "folder": str(folder), "root": str(root),
            "marker": mark,
        },
    }


@pytest.fixture
def acmodel(make_system, app_dir, write_config, check_envelope):
    """`acmodel <argv>` in the venv with `paths.state_root` at `state_root`: (exit status, envelope)."""

    def call(*argv: object, state_root: Path) -> tuple[int, dict]:
        write_config({"schema_version": "acmodel.config.v1", "paths": {"state_root": str(state_root)}})
        system = make_system(prefix=app_dir / "venv")
        status = cli.main([str(item) for item in argv], system)
        return status, check_envelope(system.stdout.getvalue(), status)

    return call


@pytest.mark.parametrize("depth", [0, 2])
@pytest.mark.parametrize("mark", MARKS)
def test_a_state_root_in_an_install_root_is_config_invalid_and_nothing_is_written(
    tmp_path, app_dir, acmodel, mark, depth
):
    """The state root is the fake root itself, or two folders below it: `run` is refused when the configuration is
    read, before the cache under the state root is opened."""
    root = fake_install_root(tmp_path, mark)
    state_root = root / "acmodel" / "state" if depth else root
    example = example_in(tmp_path / "example")
    before = snapshot(root), snapshot(example)

    status, doc = acmodel("run", example / MODEL, "--state", example / STATE, state_root=state_root)

    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [{
        "code": "config_invalid",
        "ref": "/paths/state_root",
        "detail": {
            "reason": "runtime_install_root", "path": str(app_dir / "config.json"), "key": "state_root",
            "folder": str(state_root), "root": str(root), "marker": mark,
        },
    }]
    assert (snapshot(root), snapshot(example)) == before


@pytest.mark.parametrize("command", WRITERS)
@pytest.mark.parametrize("mark", MARKS)
def test_a_model_folder_in_an_install_root_refuses_every_command_that_writes_into_it(tmp_path, acmodel, mark, command):
    """The worked example lies in the fake root, its model folder four folders below it: nothing is computed, `out/` is
    not made, the state root is not made, and the repository of `export tables` stays as it was."""
    root = fake_install_root(tmp_path, mark)
    example = example_in(root / "repos" / "example")
    repo = tmp_path / "repo"
    repo.mkdir()
    argv: list[object] = [*command.split(" "), example / MODEL, "--state", example / STATE]
    if command == "export tables":
        argv += ["--repo", repo]
    before = snapshot(root), snapshot(repo)

    status, doc = acmodel(*argv, state_root=tmp_path / "state")

    assert (status, doc["data"]) == (5, {})
    assert doc["errors"] == [refused("model", example / MODEL, root, mark)]
    assert (snapshot(root), snapshot(repo)) == before
    assert not (example / MODEL / "out").exists()
    assert not (tmp_path / "state").exists()


@pytest.mark.parametrize("inside", [("--repo",), ("model", "--repo")])
def test_a_repository_in_an_install_root_refuses_export_tables(tmp_path, acmodel, inside):
    """`export tables` writes into `--repo` too: each folder in the fake root is one error, in the order of the
    command line, and nothing is written."""
    mark = "install/active.json"
    root = fake_install_root(tmp_path, mark)
    example = example_in((root if "model" in inside else tmp_path) / "example")
    repo = root / "repo"
    repo.mkdir()
    before = snapshot(root), snapshot(example)

    status, doc = acmodel(
        "export", "tables", example / MODEL, "--state", example / STATE, "--repo", repo, state_root=tmp_path / "state"
    )

    assert (status, doc["data"]) == (5, {})
    folders = {"model": example / MODEL, "--repo": repo}
    assert doc["errors"] == [refused(argument, folders[argument], root, mark) for argument in inside]
    assert (snapshot(root), snapshot(example)) == before


def test_folders_with_no_mark_on_the_way_up_work_as_before(tmp_path, acmodel):
    """Names that are no mark, on the way up or beside it, refuse nothing: `run` keeps its outputs under the state root
    and `export thresholds` writes into the model folder."""
    base = tmp_path / "plain"
    for name in DECOYS:
        target = base.joinpath(*name.split("/"))
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(b"{}\n")
    example = example_in(base / "repos" / "example")
    state_root = base / "acmodel" / "state"

    status, doc = acmodel("run", example / MODEL, "--state", example / STATE, state_root=state_root)

    assert (status, doc["errors"]) == (0, [])
    assert (state_root / "runs" / doc["data"]["run_id"] / "outputs.json").is_file()

    status, doc = acmodel("export", "thresholds", example / MODEL, "--state", example / STATE, state_root=state_root)

    assert (status, doc["errors"]) == (0, [])
    assert (example / MODEL / "out" / "thresholds.json").is_file()


def test_a_command_that_only_reads_the_model_folder_is_not_refused(tmp_path, acmodel):
    """`run` and `explain` read a model in the fake root and write only under the state root, which is outside it."""
    root = fake_install_root(tmp_path, "actingd.config.json")
    example = example_in(root / "example")
    state_root = tmp_path / "state"
    before = snapshot(root)

    status, doc = acmodel("run", example / MODEL, "--state", example / STATE, state_root=state_root)
    assert (status, doc["errors"]) == (0, [])
    status, doc = acmodel(
        "explain", example / MODEL, "threshold:thr.daily.floor", "--state", example / STATE, state_root=state_root
    )
    assert (status, doc["errors"]) == (0, [])
    assert snapshot(root) == before


def test_a_link_into_an_install_root_is_followed(tmp_path, app_dir, acmodel):
    """A state root reached through a symbolic link is walked up from where the link leads too."""
    mark = "actingd.config.json"
    root = fake_install_root(tmp_path, mark)
    (root / "inner").mkdir()
    link = tmp_path / "link"
    try:
        os.symlink(root / "inner", link, target_is_directory=True)
    except OSError as exc:  # a host that does not let this user make links
        pytest.skip(f"no symbolic link here: {type(exc).__name__}")
    state_root = link / "state"
    example = example_in(tmp_path / "example")
    before = snapshot(root)

    status, doc = acmodel("run", example / MODEL, "--state", example / STATE, state_root=state_root)

    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [{
        "code": "config_invalid",
        "ref": "/paths/state_root",
        "detail": {
            "reason": "runtime_install_root", "path": str(app_dir / "config.json"), "key": "state_root",
            "folder": str(state_root), "root": str(root.resolve()), "marker": mark,
        },
    }]
    assert snapshot(root) == before


def test_a_level_that_cannot_be_looked_at_refuses_too(tmp_path, app_dir, acmodel, monkeypatch):
    """A level whose marks cannot be looked at (anything but "not there") cannot be shown not to be an install root:
    it refuses, with the exception, and nothing is written."""
    from acmodel import location

    blocked = tmp_path / "blocked"
    looked = location._present

    def present(path: Path) -> bool:
        if Path(path).parent == blocked:
            raise PermissionError(13, "denied", str(path))
        return looked(path)

    monkeypatch.setattr(location, "_present", present)
    state_root = blocked / "state"
    example = example_in(tmp_path / "example")

    status, doc = acmodel("run", example / MODEL, "--state", example / STATE, state_root=state_root)

    assert (status, doc["data"]) == (1, {})
    assert doc["errors"] == [{
        "code": "config_invalid",
        "ref": "/paths/state_root",
        "detail": {
            "reason": "runtime_install_unchecked", "path": str(app_dir / "config.json"), "key": "state_root",
            "folder": str(state_root), "root": str(blocked), "marker": "actingd.config.json",
            "exception_type": "PermissionError",
            "raw_text": str(PermissionError(13, "denied", str(blocked / "actingd.config.json"))),
        },
    }]
    assert not blocked.exists()
