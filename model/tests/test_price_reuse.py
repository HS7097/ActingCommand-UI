"""#424 PRICE-REUSE: the evaluator digest that `acmodel export prices` records, and `--reuse-assessment`, which takes the
impact assessment of an earlier export over when its versions, its evaluator digest and its shared tree are this export's
(contracts/cli.md, "Cache", "The evaluator digest"; contracts/formats.md, "Taking an assessment over").

The fixtures are test_prices.py's hand-solvable neutral models; flips_fixture's assessment moves s's price over [1, 2] and
flips act.p and act.q, so an assessment taken over is visible in what it decides."""

from __future__ import annotations

import hashlib
import json
import shutil
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest

from acmodel import cache, config
from acmodel.cache.keys import EVALUATORS, evaluator_tokens
from acmodel.config import EconomySettings
from acmodel.core import evaluator, flow, impact
from acmodel.core.graph import Graph
from acmodel.export import policy
from test_prices import (  # noqa: F401  (acmodel is a fixture)
    HEX,
    STEADY_ROOT,
    accept,
    acmodel,
    base,
    edit,
    entries,
    export,
    flips_fixture,
    redescribed,
    untimed_doc,
)

PINNED = {"numpy": "2.5.4", "scipy": "1.18.1"}  # the pins the fake system reports (conftest.py)
LIBS = {"numpy": "2.5.4", "python": "3.13", "scipy": "1.18.1"}
# The digest's configuration by default: every key but those that never change a result and the plan's.
DEFAULT_CONFIG = {
    "compute.blas_threads": 1, "compute.workers.max": 2, "economy.max_cycle_steps": 28, "engine.hash_seed": 0,
    "engine.reset_each_batch": True, "engine.worker_max_requests": 0, "engine.worker_soft_memory_mib": 896,
    "expr.max_bits": 127, "lp.grain": 1, "lp.max_denominator": 1_000_000, "lp.max_pivots": 20,
    "random.enclosure_digits": 12, "random.max_transitions": 1_000_000, "search.exact_max": 5000,
    "search.node_limit": 100_000, "search.screen_max_rows": 2**23, "search.screen_scale": 1_000_000, "search.top_k": 32,
    "sensitivity.top_n": 5, "unknowns.drop_z": "1.959964", "unknowns.resolution": 1_000_000, "unknowns.samples": 0,
    "unknowns.scale": 1000, "unknowns.seed": 0, "unknowns.wilson_digits": 6, "watch.keep_workers_warm": True,
}
TAKEN = ["estimates.assessed", "estimates.skipped", "impact", "prices.flips", "prices.impact"]


def defaults(tmp_path: Path) -> config.Config:
    """The configuration without a file: every key by default."""
    return config.load({"LOCALAPPDATA": str(tmp_path / "localappdata")})


def changed(settings: config.Config, key: str, value: Any) -> config.Config:
    """`settings` with the configuration key `key` (`config.FIELDS`) at `value`."""

    def put(holder: Any, names: tuple[str, ...]) -> Any:
        inner = value if len(names) == 1 else put(getattr(holder, names[0]), names[1:])
        return replace(holder, **{names[0]: inner})

    return put(settings, config.FIELDS[key])


def reuse(acmodel, folder: Path, earlier: Path) -> tuple[int, dict[str, Any]]:
    return acmodel(
        "export", "prices", str(base(folder)), "--state", str(folder / "state.json"), "--reuse-assessment", str(earlier)
    )


def snapshot(folder: Path) -> dict[str, bytes]:
    """Every file under `folder`, by its path relative to it, with its bytes; nothing when it is not there."""
    if not folder.exists():
        return {}
    return {path.relative_to(folder).as_posix(): path.read_bytes() for path in sorted(folder.rglob("*")) if path.is_file()}


@pytest.fixture
def assessed(monkeypatch) -> list[Any]:
    """The impact assessments run, one graph each: `core.impact.assess`, watched."""
    calls: list[Any] = []
    assess = impact.assess

    def watched(graph: Any, *args: Any) -> Any:
        calls.append(graph)
        return assess(graph, *args)

    monkeypatch.setattr(impact, "assess", watched)
    return calls


def package(tmp_path: Path) -> Path:
    """A package of two source files, one in a subfolder."""
    root = tmp_path / "pkg"
    (root / "sub").mkdir(parents=True)
    (root / "__init__.py").write_bytes(b"NAME = 'neutral'\n")
    (root / "sub" / "b.py").write_bytes(b"VALUE = 1\n")
    return root


# The evaluator digest ----------------------------------------------------------------------------------------------------


def test_the_evaluator_digest_is_the_same_twice_and_moves_with_each_of_its_inputs(tmp_path):
    """The digest of the same package, libraries and configuration is the same each time. Compiled files, a
    `__pycache__` folder (whatever it holds) and other files are not the source: adding them changes nothing. One byte of
    a source file changes `source`; each library the steady state's node declares changes `libraries` (the Python patch
    version is not one); each configuration key of the steady state's spec changes `config` (expr.max_bits too); the
    plan's keys are not the steady state's and change nothing. Each change moves `sha256`."""
    root = package(tmp_path)
    settings = defaults(tmp_path)
    first = evaluator.record(LIBS, settings, root)

    assert evaluator.record(LIBS, settings, root) == first
    assert set(first) == {"sha256", "source", "libraries", "config"}
    assert HEX.fullmatch(first["sha256"]) and HEX.fullmatch(first["source"])
    assert (first["libraries"], first["config"]) == (LIBS, DEFAULT_CONFIG)
    assert evaluator.source_files(root) == ["__init__.py", "sub/b.py"]

    (root / "__pycache__").mkdir()
    (root / "__pycache__" / "b.cpython-313.pyc").write_bytes(b"\x00compiled")
    (root / "sub" / "__pycache__").mkdir()
    (root / "sub" / "__pycache__" / "left.py").write_bytes(b"NOT = 'read'\n")
    (root / "c.pyc").write_bytes(b"\x00compiled")
    (root / "notes.txt").write_text("not code", encoding="utf-8")
    assert evaluator.source_files(root) == ["__init__.py", "sub/b.py"]
    assert evaluator.record(LIBS, settings, root) == first

    (root / "sub" / "b.py").write_bytes(b"VALUE = 2\n")
    moved = evaluator.record(LIBS, settings, root)
    assert moved["source"] != first["source"] and moved["sha256"] != first["sha256"]
    assert (moved["libraries"], moved["config"]) == (LIBS, DEFAULT_CONFIG)
    (root / "sub" / "b.py").write_bytes(b"VALUE = 1\n")
    assert evaluator.record(LIBS, settings, root) == first

    for name in LIBS:
        moved = evaluator.record({**LIBS, name: "0.0.1"}, settings, root)
        assert moved["libraries"] == {**LIBS, name: "0.0.1"} and moved["sha256"] != first["sha256"]
        assert (moved["source"], moved["config"]) == (first["source"], DEFAULT_CONFIG)
    patch_15, patch_16 = cache.libraries((3, 13, 15), PINNED), cache.libraries((3, 13, 16), PINNED)
    assert evaluator.record(patch_16, settings, root) == evaluator.record(patch_15, settings, root)

    for key in ("lp.max_denominator", "lp.grain", "economy.max_cycle_steps", "lp.max_pivots"):
        moved = evaluator.record(LIBS, changed(settings, key, DEFAULT_CONFIG[key] + 1), root)
        assert moved["config"] == {**DEFAULT_CONFIG, key: DEFAULT_CONFIG[key] + 1} and moved["sha256"] != first["sha256"]
    moved = evaluator.record(LIBS, changed(settings, "expr.max_bits", 128), root)
    assert moved["config"] == {**DEFAULT_CONFIG, "expr.max_bits": 128} and moved["sha256"] != first["sha256"]
    for key in ("search.milp_node_limit", "economy.plan_horizon"):
        assert evaluator.record(LIBS, changed(settings, key, config.value(settings, key) + 1), root) == first


def test_the_tools_own_source_is_every_py_file_of_its_package():
    """The digest the export records reads the acmodel package itself: its `.py` files, by their paths relative to it,
    sorted, none in a `__pycache__`."""
    found = evaluator.source_files()

    assert {"__init__.py", "core/evaluator.py", "export/prices.py", "cache/keys.py"} <= set(found)
    assert found == sorted(found)
    assert all(name.endswith(".py") and "__pycache__" not in name.split("/") for name in found)


class _Asked(Exception):
    """The node's spec was asked for: the recording session stops the computation there."""


class _Recorder:
    """A cache session that records the spec of the node asked for, and stops there."""

    def __init__(self) -> None:
        self.specs: list[tuple[str, Any]] = []

    def spec(self, kind: str, spec: Any) -> str:
        self.specs.append((kind, spec))
        raise _Asked


def test_the_evaluator_digest_holds_what_the_steady_states_cache_key_holds(tmp_path):
    """The digest holds what the cache key of the steady state's node holds (cli.md, "Cache", "Keys"): every member of the
    spec `Graph.cached` builds for it but the node and its value's format is configuration, and each is in the digest's
    `config` with its value, by its configuration key (a member added to that spec fails here until the digest holds it
    too); the libraries in the digest are those of the node's token (`EVALUATORS`, `evaluator_tokens`). The settings
    differ from the defaults, so each value is the one given; the plan's two are not in it."""
    settings = EconomySettings(
        max_denominator=999, grain=3, node_limit=7, plan_horizon=5, max_cycle_steps=14, max_pivots=9,
    )
    recorder = _Recorder()
    checker = SimpleNamespace(unknowns=[], entries={})
    graph = Graph(checker, None, {"cleared": []}, {"thresholds": []}, max_bits=61, economy=settings, cache=recorder)

    with pytest.raises(_Asked):
        graph.cached(flow.NODES[flow.STEADY])

    [(kind, spec)] = recorder.specs
    assert (kind, spec["node"]) == (evaluator.KIND, [evaluator.KIND, evaluator.SOLVE])
    assert set(spec) == {"node", "format", "max_bits", "economy"}, "a new member of the steady state's spec"
    configured = replace(defaults(tmp_path), expr_max_bits=61, economy=settings)
    named = {field: key for key, field in config.FIELDS.items()}
    held = {named[("expr_max_bits",)]: spec["max_bits"]}
    held.update({named[("economy", field)]: value for field, value in spec["economy"].items()})
    assert held.items() <= evaluator.config(configured).items()
    libraries = cache.libraries((3, 13, 15), PINNED)
    assert evaluator.libraries(libraries) == evaluator_tokens(EVALUATORS, libraries)[kind]["libraries"]
    assert evaluator.record(libraries, configured)["config"] == {
        **DEFAULT_CONFIG, "expr.max_bits": 61, "lp.max_denominator": 999, "lp.grain": 3, "economy.max_cycle_steps": 14,
        "lp.max_pivots": 9,
    }


def test_every_configuration_key_is_in_the_digest_or_listed_as_left_out_with_why(tmp_path, monkeypatch):
    """Every configuration key (`config.keys`, from the reader's table) is in exactly one of three: the digest's
    configuration, the keys that never change a result (`NEUTRAL`) and the plan's (`PLAN_ONLY`), each listed with why;
    `config.FIELDS` reads every key and nothing else; the digest lists its keys sorted. A key added to the reader's table
    fails the digest (and so every export) until `FIELDS` reads it, and is then in the digest unless a list names it; a
    list that names a key that is not one, or two lists naming one key, fail it too."""
    every = config.keys()
    taken, neutral, plan = set(evaluator.keys()), set(evaluator.NEUTRAL), set(evaluator.PLAN_ONLY)
    root, settings = package(tmp_path), defaults(tmp_path)

    assert len(every) == len(set(every)) and set(every) == taken | neutral | plan
    assert not taken & neutral and not taken & plan and not neutral & plan
    assert set(config.FIELDS) == set(every)
    assert all(evaluator.NEUTRAL.values()) and all(evaluator.PLAN_ONLY.values())
    assert list(evaluator.config(settings)) == evaluator.keys() == sorted(taken) == sorted(DEFAULT_CONFIG)
    assert plan == {"economy.plan_horizon", "search.milp_node_limit"}

    monkeypatch.setitem(config._MEMBERS, "lp", {*config._MEMBERS["lp"], "added"})
    assert "lp.added" in config.keys()
    with pytest.raises(ValueError, match="lp.added"):
        evaluator.keys()
    with pytest.raises(ValueError, match="lp.added"):
        evaluator.record(LIBS, settings, root)
    monkeypatch.setitem(config.FIELDS, "lp.added", ("economy", "grain"))
    assert evaluator.keys() == sorted(taken | {"lp.added"})
    monkeypatch.setitem(evaluator.NEUTRAL, "lp.added", "a key this test adds")
    assert evaluator.keys() == sorted(taken)
    monkeypatch.setitem(evaluator.PLAN_ONLY, "lp.added", "a key this test adds")
    with pytest.raises(ValueError, match="lp.added"):
        evaluator.keys()
    monkeypatch.delitem(evaluator.PLAN_ONLY, "lp.added")
    monkeypatch.setitem(evaluator.NEUTRAL, "lp.gone", "a key the configuration does not have")
    with pytest.raises(ValueError, match="lp.gone"):
        evaluator.keys()


def test_each_key_of_the_digest_moves_it_and_a_key_left_out_does_not(tmp_path):
    """Each key of the digest's configuration, changed, changes that key alone in `config` and moves `sha256`: among them
    one under `unknowns` (also `drop_z`, a decimal string, and a seed beyond the safe range, which is recorded as its
    decimal text), `sensitivity.top_n` and the keys under `random`. A change of a key that never changes a result (the
    cache, a timeout, a chunk size) or of the plan's leaves the digest as it is."""
    root, settings = package(tmp_path), defaults(tmp_path)
    first = evaluator.record(LIBS, settings, root)

    for key in evaluator.keys():
        found = config.value(settings, key)
        other = (not found) if type(found) is bool else found + 1 if type(found) is int else found + "1"
        moved = evaluator.record(LIBS, changed(settings, key, other), root)
        assert moved["config"] == {**DEFAULT_CONFIG, key: other} and moved["sha256"] != first["sha256"], key
    for key, value, recorded in (
        ("unknowns.wilson_digits", 7, 7),
        ("unknowns.drop_z", "2.5", "2.5"),
        ("unknowns.seed", 2**64 - 1, "18446744073709551615"),
        ("sensitivity.top_n", 6, 6),
        ("random.max_transitions", 99, 99),
        ("random.enclosure_digits", 13, 13),
    ):
        moved = evaluator.record(LIBS, changed(settings, key, value), root)
        assert moved["config"] == {**DEFAULT_CONFIG, key: recorded} and moved["sha256"] != first["sha256"], key
    for key, value in (
        ("cache.max_bytes", 1), ("cache.wait_ms", 1), ("export.validator_timeout_ms", 1),
        ("engine.request_timeout_ms", 1), ("unknowns.chunk_size", 7), ("search.screen_chunk_rows", 7),
        ("search.milp_node_limit", 7), ("economy.plan_horizon", 3),
    ):
        assert evaluator.record(LIBS, changed(settings, key, value), root) == first, key


# The export records it ---------------------------------------------------------------------------------------------------


def test_prices_json_records_the_evaluator_digest_and_nothing_else_moves(tmp_path, acmodel, app_dir):
    """Without the option, prices.json gains one member, `evaluator`: the tool's source, the libraries it runs with
    (Python 3.13, the pinned numpy and scipy) and the default configuration (every key but those that never change a
    result and the plan's). The versions keep their three members, and no price's provenance file holds it, so no
    `binds` moves with it; `verify prices` reads the file."""
    folder = flips_fixture(tmp_path)

    status, doc = export(acmodel, folder)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert "reused_assessment" not in doc["data"]
    prices = json.loads((base(folder) / "out" / "prices.json").read_text(encoding="utf-8"))
    recorded = prices["evaluator"]
    assert (recorded["libraries"], recorded["config"]) == (LIBS, DEFAULT_CONFIG)
    assert recorded["source"] == evaluator.source() and HEX.fullmatch(recorded["sha256"])
    assert set(prices["versions"]) == {"rules", "data", "tool"} and "reused_assessment" not in prices
    for entry in prices["prices"]:
        written = json.loads((base(folder) / entry["provenance"]).read_text(encoding="utf-8"))
        assert "evaluator" not in written and "evaluator" not in written["versions"]

    status, verified = acmodel("verify", "prices", str(base(folder)), "--state", str(folder / "state.json"))

    assert (status, verified["errors"]) == (0, [])


# Taking the assessment over ----------------------------------------------------------------------------------------------


def test_an_export_that_takes_the_assessment_over_writes_the_full_exports_files(tmp_path, acmodel, assessed):
    """The same inputs, exported in full, then again taking the assessment over from the model's own out folder: the
    assessment does not run, every price's provenance file is the full export's byte for byte, and prices.json is too once
    `reused_assessment` is taken out. That member records the earlier prices.json's sha256, its versions, its shared
    tree's digest, its evaluator's digest and the members taken over; the envelope adds the earlier file. The flips and the
    impact range taken over are flips_fixture's (s over [1, 2], act.p and act.q). `verify prices` reads the file."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    status, doc = export(acmodel, folder)
    assert (status, doc["errors"], doc["warnings"], len(assessed)) == (0, [], [], 1)
    full = snapshot(out)
    earlier = json.loads(full["prices.json"])

    status, doc = reuse(acmodel, folder, out)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert len(assessed) == 1  # only the full export's
    taken = snapshot(out)
    assert sorted(taken) == sorted(full)
    for name in full:
        if name != "prices.json":
            assert taken[name] == full[name], name
    prices = json.loads(taken["prices.json"])
    marker = prices.pop("reused_assessment")
    assert policy.delivered(prices) == full["prices.json"]
    assert marker == {
        "prices_sha256": hashlib.sha256(full["prices.json"]).hexdigest(),
        "versions": earlier["versions"],
        "shared": {STEADY_ROOT: earlier["shared"][STEADY_ROOT]["sha256"]},
        "evaluator": earlier["evaluator"]["sha256"],
        "members": TAKEN,
    }
    assert doc["data"]["reused_assessment"] == {"file": str(out / "prices.json"), **marker}
    found = entries(folder)
    assert (found["res.s"]["impact"], found["res.s"]["flips"]) == ({"low": "1", "high": "2"}, ["act.p", "act.q"])

    status, verified = acmodel("verify", "prices", str(base(folder)), "--state", str(folder / "state.json"))

    assert (status, verified["errors"]) == (0, [])


def test_an_acceptance_added_after_the_export_reads_the_flips_taken_over(tmp_path, acmodel, assessed):
    """exports.json is neither in the versions nor read by the assessment, so an export after an acceptance is added may
    take the assessment over. flips_fixture, exported in full and its out folder kept aside; then s is accepted listing
    act.p only (incomplete: `acceptance_incomplete`, naming act.q) and v listing both flips (exported). The full export
    and the one that takes the earlier assessment over answer the same and write the same files, prices.json but for
    `reused_assessment` and its passes' `timing_ms` (PASS-TIMING: taken over, it is the earlier run's, not this full
    export's)."""
    folder = flips_fixture(tmp_path)
    out, kept = base(folder) / "out", tmp_path / "earlier"
    export(acmodel, folder)
    shutil.copytree(out, kept)
    before = entries(folder)
    accept(
        folder,
        {"output": "price:res.s", "accept_provisional": {
            "ref": "R-1", "until": "item", "binds": before["res.s"]["binds"], "scope": "game", "flips": ["act.p"]}},
        {"output": "price:res.v", "accept_provisional": {
            "ref": "R-2", "until": "item", "binds": before["res.v"]["binds"], "scope": "game", "flips": ["act.p", "act.q"]}},
    )

    status, full_doc = export(acmodel, folder)

    assert (status, full_doc["errors"]) == (0, [])
    assert [entry["code"] for entry in full_doc["warnings"]] == ["acceptance_incomplete"]
    full = snapshot(out)
    found = entries(folder)
    assert (found["res.s"]["pending"], found["res.v"]["pending"]) == (["acceptance_incomplete"], [])
    assert len(assessed) == 2

    status, doc = reuse(acmodel, folder, kept)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], full_doc["warnings"])
    assert len(assessed) == 2
    taken = snapshot(out)
    assert sorted(taken) == sorted(full)
    for name in full:
        if name != "prices.json":
            assert taken[name] == full[name], name
    prices = json.loads(taken["prices.json"])
    marker = prices.pop("reused_assessment")
    assert policy.delivered(untimed_doc(prices)) == policy.delivered(untimed_doc(json.loads(full["prices.json"])))
    assert marker["prices_sha256"] == hashlib.sha256((kept / "prices.json").read_bytes()).hexdigest()
    assert doc["data"]["reused_assessment"]["file"] == str(kept / "prices.json")
    assert {key: doc["data"][key] for key in ("counts", "unit_milli")} == {
        key: full_doc["data"][key] for key in ("counts", "unit_milli")
    }


def _drop_evaluator(doc: dict[str, Any]) -> None:
    doc.pop("evaluator")


def _grain_two(doc: dict[str, Any]) -> None:
    doc["evaluator"]["config"]["lp.grain"] = 2


def _numpy_older(doc: dict[str, Any]) -> None:
    doc["evaluator"]["libraries"]["numpy"] = "2.5.3"


def _other_source(doc: dict[str, Any]) -> None:
    doc["evaluator"]["source"] = "0" * 64


@pytest.mark.parametrize(
    ("change", "member", "reason", "ref", "expected", "found"),
    [
        pytest.param(_drop_evaluator, "evaluator", "not_recorded", "/evaluator", "none", "sha256", id="no_evaluator"),
        pytest.param(_grain_two, "evaluator.config.lp.grain", "differs", "/evaluator/config/lp.grain", "2", "1",
                     id="evaluator_config"),
        pytest.param(_numpy_older, "evaluator.libraries.numpy", "differs", "/evaluator/libraries/numpy", "2.5.3",
                     "2.5.4", id="evaluator_library"),
        pytest.param(_other_source, "evaluator.source", "differs", "/evaluator/source", "0" * 64, "source",
                     id="evaluator_source"),
    ],
)
def test_an_earlier_evaluator_digest_that_is_not_this_ones_is_refused(
    tmp_path, acmodel, assessed, change, member, reason, ref, expected, found
):
    """The earlier prices.json without an evaluator digest (as one written before PRICE-REUSE) is never reused
    (`not_recorded`, expected none); one whose digest differs is refused member by member, with both values. Each is
    `reuse_refused` (exit 5): nothing is assessed, and neither the model's out folder nor the state root changes."""
    folder = flips_fixture(tmp_path)
    out, kept = base(folder) / "out", tmp_path / "earlier"
    export(acmodel, folder)
    shutil.copytree(out, kept)
    recorded = json.loads((out / "prices.json").read_text(encoding="utf-8"))["evaluator"]
    edit(kept / "prices.json", change)
    before = snapshot(out), snapshot(tmp_path / "localappdata")

    status, doc = reuse(acmodel, folder, kept)

    assert (status, doc["data"]) == (5, {})
    assert doc["errors"] == [{
        "code": "reuse_refused",
        "detail": {"path": str(kept / "prices.json"), "member": member, "reason": reason, "expected": expected,
                   "found": recorded[found] if found in recorded else found},
        "ref": ref,
    }]
    assert len(assessed) == 1  # the full export's only
    assert (snapshot(out), snapshot(tmp_path / "localappdata")) == before


def test_a_change_of_the_model_or_of_the_configuration_is_refused(tmp_path, acmodel, assessed, write_config):
    """A rule's note changes the rules version: `versions.rules`, with both digests; the model's own out folder named as
    the earlier export, whose prices.json is then of other versions. A configuration key of the steady state's spec
    changed since the export (lp.grain 2): `evaluator.config.lp.grain`, 1 then 2. Neither runs the assessment or writes."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    export(acmodel, folder)
    earlier = json.loads((out / "prices.json").read_text(encoding="utf-8"))
    edit(base(folder) / "rules" / "economy.json", lambda rules: rules["rules"][0].update(note="edited"))
    before = snapshot(out), snapshot(tmp_path / "localappdata")

    status, doc = reuse(acmodel, folder, out)

    assert status == 5
    [refused] = doc["errors"]
    assert (refused["code"], refused["ref"]) == ("reuse_refused", "/versions/rules")
    assert {key: refused["detail"][key] for key in ("path", "member", "reason", "expected")} == {
        "path": str(out / "prices.json"), "member": "versions.rules", "reason": "differs",
        "expected": earlier["versions"]["rules"],
    }
    assert HEX.fullmatch(refused["detail"]["found"]) and refused["detail"]["found"] != earlier["versions"]["rules"]
    assert (snapshot(out), snapshot(tmp_path / "localappdata")) == before

    edit(base(folder) / "rules" / "economy.json", lambda rules: rules["rules"][0].pop("note"))
    status, doc = reuse(acmodel, folder, out)
    assert (status, doc["errors"]) == (0, [])  # the model as it was: taken over
    write_config({"schema_version": "acmodel.config.v1", "lp": {"grain": 2}})
    before = snapshot(out), snapshot(tmp_path / "localappdata")

    status, doc = reuse(acmodel, folder, out)

    assert status == 5
    assert doc["errors"] == [{
        "code": "reuse_refused",
        "detail": {"path": str(out / "prices.json"), "member": "evaluator.config.lp.grain", "reason": "differs",
                   "expected": "1", "found": "2"},
        "ref": "/evaluator/config/lp.grain",
    }]
    assert len(assessed) == 1
    assert (snapshot(out), snapshot(tmp_path / "localappdata")) == before


@pytest.mark.parametrize(
    ("section", "key", "value", "expected"),
    [
        pytest.param("unknowns", "wilson_digits", 7, "6", id="unknowns"),
        pytest.param("sensitivity", "top_n", 6, "5", id="sensitivity"),
        pytest.param("random", "enclosure_digits", 13, "12", id="random"),
    ],
)
def test_a_change_of_a_key_under_unknowns_sensitivity_or_random_is_refused(
    tmp_path, acmodel, assessed, write_config, section, key, value, expected
):
    """The keys under `unknowns` (but `chunk_size`), `sensitivity` and `random` are not in the steady state's spec, but
    what the steady state or the assessment reads may depend on them (a Wilson bound, a pity chain, an enclosure's
    digits) while the shared tree stays the same: they are in the digest. One changed since the export is
    `evaluator.config.<key>`, the earlier value then this one; nothing is assessed or written."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    export(acmodel, folder)
    write_config({"schema_version": "acmodel.config.v1", section: {key: value}})
    before = snapshot(out), snapshot(tmp_path / "localappdata")

    status, doc = reuse(acmodel, folder, out)

    assert status == 5
    assert doc["errors"] == [{
        "code": "reuse_refused",
        "detail": {"path": str(out / "prices.json"), "member": f"evaluator.config.{section}.{key}", "reason": "differs",
                   "expected": expected, "found": str(value)},
        "ref": f"/evaluator/config/{section}.{key}",
    }]
    assert len(assessed) == 1
    assert (snapshot(out), snapshot(tmp_path / "localappdata")) == before


def test_a_change_of_keys_that_never_change_a_result_takes_the_assessment_over(
    tmp_path, acmodel, assessed, write_config
):
    """`cache.wait_ms`, `export.validator_timeout_ms` and `unknowns.chunk_size` are listed as never changing a result:
    changed since the export, the digest is the same, and the assessment is taken over."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    export(acmodel, folder)
    earlier = json.loads((out / "prices.json").read_text(encoding="utf-8"))["evaluator"]
    write_config({
        "schema_version": "acmodel.config.v1", "cache": {"wait_ms": 1234}, "export": {"validator_timeout_ms": 999},
        "unknowns": {"chunk_size": 7},
    })

    status, doc = reuse(acmodel, folder, out)

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert len(assessed) == 1
    prices = json.loads((out / "prices.json").read_text(encoding="utf-8"))
    assert prices["evaluator"] == earlier and prices["reused_assessment"]["evaluator"] == earlier["sha256"]


def test_a_shared_tree_that_is_not_this_steady_states_is_refused(tmp_path, acmodel, assessed, monkeypatch):
    """The versions and the evaluator digest agree, but the steady state's tree is described otherwise than at the
    earlier export (test_prices.py's `redescribed`: the tree alone changes): once the steady state is computed, its
    tree's digest is not the recorded one, `shared.economy:steady`, with both digests. The assessment does not run and
    nothing is written, the tree under the state root neither."""
    folder = flips_fixture(tmp_path)
    out = base(folder) / "out"
    export(acmodel, folder)
    recorded = json.loads((out / "prices.json").read_text(encoding="utf-8"))["shared"][STEADY_ROOT]["sha256"]
    before = snapshot(out), snapshot(tmp_path / "localappdata")

    with monkeypatch.context() as patched:
        redescribed(patched)
        status, doc = reuse(acmodel, folder, out)

    assert status == 5
    [refused] = doc["errors"]
    assert (refused["code"], refused["ref"]) == ("reuse_refused", f"/shared/{STEADY_ROOT}")
    assert {key: refused["detail"][key] for key in ("path", "member", "reason", "expected")} == {
        "path": str(out / "prices.json"), "member": f"shared.{STEADY_ROOT}", "reason": "differs", "expected": recorded,
    }
    assert HEX.fullmatch(refused["detail"]["found"]) and refused["detail"]["found"] != recorded
    assert len(assessed) == 1
    assert (snapshot(out), snapshot(tmp_path / "localappdata")) == before


def test_a_folder_without_an_earlier_prices_json_computes_nothing(tmp_path, acmodel, assessed):
    """The folder named holds no prices.json: `model_file_invalid` (`unreadable`, of the v2 format), before the model is
    run; nothing is written."""
    folder = flips_fixture(tmp_path)
    nowhere = tmp_path / "nowhere"

    status, doc = reuse(acmodel, folder, nowhere)

    assert status == 1
    [refused] = doc["errors"]
    assert (refused["code"], refused["detail"]["reason"], refused["detail"]["path"], refused["detail"]["format"]) == (
        "model_file_invalid", "unreadable", str(nowhere / "prices.json"), "acmodel.prices.v2",
    )
    assert assessed == [] and not (base(folder) / "out").exists()


def test_the_option_takes_one_value_once_and_only_on_export_prices(tmp_path, acmodel):
    """`--reuse-assessment` without its value is `argument_missing`; given twice, or on another command, it is
    `argument_unexpected` (exit 2)."""
    folder = flips_fixture(tmp_path)
    model, state, kept = str(base(folder)), str(folder / "state.json"), str(tmp_path / "earlier")

    for argv, reason in (
        (("export", "prices", model, "--state", state, "--reuse-assessment"), "argument_missing"),
        (("export", "prices", model, "--state", state, "--reuse-assessment", kept, "--reuse-assessment", kept),
         "argument_unexpected"),
        (("verify", "prices", model, "--state", state, "--reuse-assessment", kept), "argument_unexpected"),
    ):
        status, doc = acmodel(*argv)

        assert status == 2
        assert doc["errors"] == [{
            "code": "usage_invalid", "detail": {"reason": reason, "argument": "--reuse-assessment"},
        }]
