"""PY-0: `version`, an unknown command and the dependency check, in process."""

import pytest

from acmodel import __version__, cli


def test_version_prints_one_envelope_with_the_versions(make_system, app_dir, check_envelope):
    # No config file: the default venv location applies, and this process runs in it.
    system = make_system(prefix=app_dir / "venv")

    status = cli.main(["version"], system)

    doc = check_envelope(system.stdout.getvalue(), status)
    assert status == 0
    assert doc["command"] == "version"
    assert doc["data"] == {
        "tool": "acmodel",
        "tool_version": __version__,
        "python": "3.13.15",
        "packages": {"numpy": "2.5.4", "scipy": "1.18.1"},
    }
    assert system.run.calls == []


def test_unknown_command_exits_2_with_usage(make_system, check_envelope):
    system = make_system()

    status = cli.main(["frobnicate"], system)

    doc = check_envelope(system.stdout.getvalue(), status)
    assert status == 2
    assert doc["command"] is None
    assert [(e["code"], e["detail"]) for e in doc["errors"]] == [
        ("usage_invalid", {"reason": "command_unknown", "argument": "frobnicate"}),
    ]
    assert system.run.calls == []


@pytest.mark.parametrize(
    ("python_version", "installed", "expected"),
    [
        ((3, 13, 15), {"numpy": "2.5.4"}, {"reason": "missing", "package": "scipy"}),
        (
            (3, 13, 15),
            {"numpy": "2.5.3", "scipy": "1.18.1"},
            {"reason": "version_mismatch", "package": "numpy", "expected": "2.5.4", "found": "2.5.3"},
        ),
        (
            (3, 12, 7),
            {"numpy": "2.5.4", "scipy": "1.18.1"},
            {"reason": "python_version", "expected": "3.13", "found": "3.12.7"},
        ),
    ],
    ids=["scipy_missing", "numpy_not_at_pin", "python_not_3_13"],
)
def test_dependency_check_fails_with_deps_invalid(
    make_system, app_dir, launcher, check_envelope, python_version, installed, expected
):
    system = make_system(prefix=app_dir / "venv", python_version=python_version, installed=installed)

    status = cli.main(["version"], system)

    doc = check_envelope(system.stdout.getvalue(), status)
    assert status == 2
    assert doc["data"] == {}
    [entry] = doc["errors"]
    assert entry["code"] == "deps_invalid"
    assert {key: entry["detail"].get(key) for key in expected} == expected
    assert entry["detail"]["setup_command"][-2:] == [str(launcher), "setup"]
