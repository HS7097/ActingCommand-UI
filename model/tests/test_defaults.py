"""#466 V-a, the model's writer of the packs' variable defaults: the default bindings of `bindings check` and `bindings
apply` (`defaults` of bindings.json: a constant threshold's number written into the `default` of every declaration of an
integer variable in the packs' `control.json`, with `"default_origin": "model"` inserted right after it), the checks of
the declarations, the canonical serialization, the origin guard over the packs, the literal bindings refused under
`/variables`, and a part another write left beside a control.json (contracts/formats.md, "A variable's default").

Every fixture is the worked example of test_bindings, neutral, with packs added under `packs/`: its constant threshold
`thr.daily.floor` is 215 (`floor.daily`: the daily cost 15 plus the reserve 200), `thr.run.trigger` is of the
`fact_offset` form. The variable `floor` is declared in two packs, 0 to 1000 in steps of 5, its default 0."""

from __future__ import annotations

import json
import shutil
from pathlib import Path
from typing import Any, Callable

import pytest

from acmodel import cli
from acmodel.bindings import canonical, defaults
from acmodel.bindings.targets import read_target
from acmodel.codes import EXIT_STATUS
from test_economy import CODES

EXAMPLE = Path(__file__).resolve().parent / "fixtures" / "example"
MODEL = "ours/model"
STATE = "state/acct-1.json"
ONE, TWO, THREE = "packs/pkg.one/control.json", "packs/pkg.two/control.json", "packs/pkg.three/control.json"
FLOOR = 215  # thr.daily.floor
BINDING = {"threshold": "thr.daily.floor", "variable": "floor"}


@pytest.fixture
def acmodel(make_system, app_dir):
    """One command in the venv: (exit status, envelope). The envelope is one acmodel.cli.v1 line whose errors and
    warnings agree with contracts/codes.json (each code of its category, with its values), and the exit status the
    first error's."""

    def run(*argv: Any) -> tuple[int, dict[str, Any]]:
        system = make_system(prefix=app_dir / "venv")
        status = cli.main([str(arg) for arg in argv], system)
        [line] = system.stdout.getvalue().splitlines()
        doc = json.loads(line)
        assert doc["ok"] is (doc["errors"] == [])
        for kind, entries in (("error", doc["errors"]), ("warning", doc["warnings"])):
            for entry in entries:
                spec = CODES[entry["code"]]
                assert spec["category"] == kind, entry
                assert set(entry["detail"]) <= set(spec["values"]), entry
                for key, value in spec["values"].items():
                    assert not value["required"] or key in entry["detail"], (entry, key)
                    assert "members" not in value or key not in entry["detail"] or entry["detail"][key] in value["members"]
        assert status == (CODES[doc["errors"][0]["code"]]["exit"] if doc["errors"] else 0)
        return status, doc

    return run


@pytest.fixture
def example(tmp_path: Path) -> Path:
    """A copy of the worked example; its packs hold no control.json yet."""
    target = tmp_path / "example"
    shutil.copytree(EXAMPLE, target)
    return target


def bindings(acmodel, folder: Path, command: str, *flags: str) -> tuple[int, dict[str, Any]]:
    return acmodel("bindings", command, folder / MODEL, "--state", folder / STATE, *flags)


def edit(path: Path, change: Callable[[Any], None]) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")


def bind_defaults(folder: Path, *entries: dict[str, Any]) -> None:
    edit(folder / MODEL / "bindings.json", lambda doc: doc.update(defaults=list(entries)))


def floor(**members: Any) -> dict[str, Any]:
    """The integer variable `floor` as a pack declares it: 0 to 1000 in steps of 5, its default 0, the default its last
    member; `members` replace a member in its place, or follow the default."""
    item: dict[str, Any] = {"name": "floor", "type": "integer", "title": {"en": "Keep at least"}, "min": 0,
                            "max": 1000, "step": 5, "default": 0}
    item.update(members)
    return item


def without(item: dict[str, Any], *keys: str) -> dict[str, Any]:
    return {key: value for key, value in item.items() if key not in keys}


# Another pack's variable of another name and type, which no binding names: never read further, never written.
PICK = {"name": "pick", "type": "choice", "title": {"en": "Item"},
        "options": [{"value": "item_a", "title": {"en": "Item A"}}, {"value": "item_b", "title": {"en": "Item B"}}],
        "default": "item_b"}


def control(package_id: str, *variables: dict[str, Any]) -> dict[str, Any]:
    doc: dict[str, Any] = {"package_id": package_id, "provides": []}
    if variables:
        doc["variables"] = list(variables)
    return doc


def pretty(doc: Any) -> bytes:
    """A document as `actinglab package fmt` writes it (two-space indent, LF, a final LF)."""
    return (json.dumps(doc, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def write_control(folder: Path, name: str, doc: Any, raw: bytes | None = None) -> Path:
    path = folder / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(pretty(doc) if raw is None else raw)
    return path


def two_packs(folder: Path, one: dict[str, Any], two: dict[str, Any], *others: dict[str, Any]) -> tuple[Path, Path]:
    """pkg.one declares `one`; pkg.two declares `pick`, then `two`, then `others`."""
    return (write_control(folder, ONE, control("pkg.one", one)),
            write_control(folder, TWO, control("pkg.two", PICK, two, *others)))


def codes(doc: dict[str, Any]) -> list[tuple[str, str | None]]:
    return [(entry["code"], entry.get("ref")) for entry in doc["errors"]]


# The default, written --------------------------------------------------------------------------------------------------

# pkg.one's control.json as `apply` leaves it, written out by hand: the default replaced, the origin on a line of its
# own after it, every other byte as it was, LF throughout.
ONE_DONE = b"""{
  "package_id": "pkg.one",
  "provides": [],
  "variables": [
    {
      "name": "floor",
      "type": "integer",
      "title": {
        "en": "Keep at least"
      },
      "min": 0,
      "max": 1000,
      "step": 5,
      "default": 215,
      "default_origin": "model"
    }
  ]
}
"""


def test_apply_writes_the_number_into_every_declaration_alike_and_changes_no_other_byte(example, acmodel):
    """`floor` is declared in pkg.one and pkg.two (beside an unbound `pick`). `check` names each difference, the default
    and the absent origin of each declaration; `apply` writes 215 and inserts the origin right after the default, the
    declaration's last member, with the separator before it, so both files stay canonical and the two declarations are
    byte-identical; `pick` and every other byte stay. Then `check` passes and shows the binding, and `apply` again
    writes nothing."""
    one, two = two_packs(example, floor(), floor())
    bind_defaults(example, BINDING)
    before_two = two.read_bytes()

    status, doc = bindings(acmodel, example, "check")

    assert status == 6

    def detail(path: Path, **values: str) -> dict[str, Any]:
        return {"path": str(path), "entry": "/defaults/0", "threshold": "thr.daily.floor", "variable": "floor", **values}

    assert doc["errors"] == [
        {"code": "origin_missing", "detail": detail(one), "ref": "/variables/0"},
        {"code": "binding_mismatch", "detail": detail(one, reason="value", expected="215", found="0"),
         "ref": "/variables/0/default"},
        {"code": "origin_missing", "detail": detail(two), "ref": "/variables/1"},
        {"code": "binding_mismatch", "detail": detail(two, reason="value", expected="215", found="0"),
         "ref": "/variables/1/default"},
    ]

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert one.read_bytes() == ONE_DONE
    written = json.loads(two.read_text(encoding="utf-8"))
    assert written["variables"][0] == PICK
    assert defaults.compact(written["variables"][1]) == defaults.compact(json.loads(ONE_DONE)["variables"][0])
    assert two.read_bytes() == before_two.replace(b'"default": 0\n', b'"default": 215,\n      "default_origin": "model"\n')
    assert b"\r" not in one.read_bytes() + two.read_bytes()
    assert doc["data"]["written"] == sorted([str(one), str(two)])
    change = {"entry": "/defaults/0", "threshold": "thr.daily.floor", "variable": "floor", "from": 0, "to": FLOOR,
              "ratio": None, "ratio_flagged": False, "origin": {"from": None, "to": "model"},
              "inserted": ["default_origin"]}
    assert doc["data"]["changed"] == [
        {**change, "file": ONE, "pointer": "/variables/0/default"},
        {**change, "file": TWO, "pointer": "/variables/1/default"},
    ]
    assert not (one.parent / "control.json.part").exists() and not (two.parent / "control.json.part").exists()

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"], doc["warnings"]) == (0, [], [])
    assert doc["data"]["control_files"] == 2
    assert doc["data"]["defaults"] == [{
        "entry": "/defaults/0", "threshold": "thr.daily.floor", "variable": "floor", "value": FLOOR,
        "status": "confirmed", "acceptances": [], "min": 0, "max": 1000, "step": 5,
        "declarations": [
            {"file": ONE, "pointer": "/variables/0", "literal": FLOOR, "origin": "model"},
            {"file": TWO, "pointer": "/variables/1", "literal": FLOOR, "origin": "model"},
        ],
    }]

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["data"]["changed"], doc["data"]["written"]) == (0, [], [])


def test_the_origin_goes_in_after_a_default_in_the_middle_with_the_separator_after_it(example, acmodel):
    """The default is followed by `help`: the origin goes in between, with the separator that follows the default, and
    the file stays canonical (a text of the file in UTF-8 stays as it is). No `step` means steps of 1."""
    item = without(floor(help={"fr": "Réserve gardée"}), "step")
    path = write_control(example, ONE, control("pkg.one", item))
    bind_defaults(example, BINDING)

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    expected = {key: value for key, value in item.items() if key != "help"}
    expected.update(default=FLOOR, default_origin="model", help=item["help"])
    assert list(json.loads(path.read_text(encoding="utf-8"))["variables"][0]) == list(expected)
    assert path.read_bytes() == pretty(control("pkg.one", expected))
    assert "Réserve".encode("utf-8") in path.read_bytes()

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["defaults"][0]["step"], doc["data"]["defaults"][0]["min"]) == (1, 0)


def test_an_origin_model_stays_and_another_origin_is_refused_and_never_rewritten(example, acmodel):
    """Three cases of `default_origin`: absent (the first test: inserted), `"model"` (kept where it is; only the default
    is written, nothing inserted), and anything else, `"pack"` or `true` (`origin_invalid`: `check` reports it, `apply`
    refuses and writes nothing, not even the other declaration)."""
    path = write_control(example, ONE, control("pkg.one", floor(default_origin="model")))
    bind_defaults(example, BINDING)

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert json.loads(path.read_text(encoding="utf-8"))["variables"][0] == floor(default=FLOOR, default_origin="model")
    assert doc["data"]["changed"][0]["inserted"] == []
    assert doc["data"]["changed"][0]["origin"] == {"from": "model", "to": "model"}

    for found, origin in (("pack", "pack"), ("boolean", True)):
        one, two = two_packs(example, floor(default_origin=origin), floor())
        before = (one.read_bytes(), two.read_bytes())
        invalid = {"code": "origin_invalid",
                   "detail": {"path": str(one), "entry": "/defaults/0", "threshold": "thr.daily.floor",
                              "variable": "floor", "found": found},
                   "ref": "/variables/0/default_origin"}

        status, doc = bindings(acmodel, example, "check")

        assert status == 6
        assert doc["errors"][0] == invalid

        status, doc = bindings(acmodel, example, "apply")

        assert (status, doc["data"], doc["errors"]) == (6, {}, [invalid])
        assert (one.read_bytes(), two.read_bytes()) == before


def test_declarations_that_differ_only_in_what_apply_writes_are_made_alike(example, acmodel):
    """pkg.one was written already (as an apply that stopped after its first rename leaves it), pkg.two not: they
    differ only in the default and the absent origin, which `apply` writes, so it writes pkg.two and the two are alike."""
    one, two = two_packs(example, floor(default=FLOOR, default_origin="model"), floor())
    bind_defaults(example, BINDING)
    before = one.read_bytes()

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert one.read_bytes() == before
    assert doc["data"]["written"] == [str(two)]
    assert json.loads(two.read_text(encoding="utf-8"))["variables"][1] == json.loads(before)["variables"][0]


@pytest.mark.parametrize(
    ("other", "case"),
    [(floor(max=2000), "another member"), (floor(help={"en": "x"}, default_origin="model"), "the origin elsewhere")],
    ids=["another_member", "origin_elsewhere"],
)
def test_declarations_the_write_would_leave_unlike_are_refused(example, acmodel, other, case):
    """pkg.two's declaration differs in another member than the default, or holds its origin after `help`, not right after
    the default: written, the two would still differ, which the Runtime reports `variable_conflict`. `check` and `apply`
    refuse it at pkg.two's declaration, and nothing is written."""
    one, two = two_packs(example, floor(), other)
    bind_defaults(example, BINDING)
    before = (one.read_bytes(), two.read_bytes())
    expected = [{
        "code": "variable_declarations_differ",
        "detail": {"path": str(two), "entry": "/defaults/0", "variable": "floor", "first_path": str(one),
                   "first_ref": "/variables/0"},
        "ref": "/variables/1",
    }]

    status, doc = bindings(acmodel, example, "check")

    assert status == 6, case
    assert [entry for entry in doc["errors"] if entry["code"] == "variable_declarations_differ"] == expected

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (6, expected)
    assert (one.read_bytes(), two.read_bytes()) == before


# The declaration and the number -----------------------------------------------------------------------------------------


def test_a_variable_no_pack_declares_is_refused(example, acmodel):
    two_packs(example, floor(), floor())
    bind_defaults(example, {"threshold": "thr.daily.floor", "variable": "ceiling"})
    expected = [{
        "code": "variable_unknown",
        "detail": {"path": str(example / MODEL / "bindings.json"), "entry": "/defaults/0", "threshold": "thr.daily.floor",
                   "variable": "ceiling"},
        "ref": "/defaults/0/variable",
    }]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, example, command)

        assert (status, doc["errors"]) == (6, expected)


def test_a_name_declared_twice_in_one_file_is_refused_and_that_file_not_judged(example, acmodel):
    """pkg.one declares `floor` twice (the Runtime refuses the package, `variable_name_taken`): which one to write is not
    the tool's to guess, so none of that file's is judged; pkg.two's is, and its differences are reported."""
    one = write_control(example, ONE, control("pkg.one", floor(), floor(max=10)))
    two = write_control(example, TWO, control("pkg.two", floor()))
    bind_defaults(example, BINDING)
    before = (one.read_bytes(), two.read_bytes())

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert doc["errors"][0] == {
        "code": "variable_declared_twice",
        "detail": {"path": str(one), "entry": "/defaults/0", "variable": "floor", "first": "/variables/0"},
        "ref": "/variables/1/name",
    }
    assert codes(doc)[1:] == [("origin_missing", "/variables/0"), ("binding_mismatch", "/variables/0/default")]
    assert all(entry["detail"]["path"] == str(two) for entry in doc["errors"][1:])

    status, doc = bindings(acmodel, example, "apply")

    assert (status, codes(doc)) == (6, [("variable_declared_twice", "/variables/1/name")])
    assert (one.read_bytes(), two.read_bytes()) == before


@pytest.mark.parametrize(
    ("item", "values", "ref"),
    [
        (floor(type="choice"), {"found": "choice"}, "/variables/0/type"),
        (without(floor(), "type"), {}, "/variables/0"),
    ],
    ids=["choice", "no_type"],
)
def test_a_declaration_that_is_no_integer_is_refused(example, acmodel, item, values, ref):
    path = write_control(example, ONE, control("pkg.one", item))
    bind_defaults(example, BINDING)
    expected = [{"code": "variable_not_integer",
                 "detail": {"path": str(path), "entry": "/defaults/0", "variable": "floor", **values}, "ref": ref}]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, example, command)

        assert (status, doc["errors"]) == (6, expected)


@pytest.mark.parametrize(
    ("item", "member", "found", "ref"),
    [
        (without(floor(), "min"), "min", None, "/variables/0"),
        (floor(max="1000"), "max", "1000", "/variables/0/max"),
        (floor(step=True), "step", "boolean", "/variables/0/step"),
        (floor(min=10, max=5), "min", "10", "/variables/0/min"),
        (floor(step=0), "step", "0", "/variables/0/step"),
    ],
    ids=["min_absent", "max_a_text", "step_a_boolean", "min_above_max", "step_zero"],
)
def test_a_range_that_holds_no_value_is_refused(example, acmodel, item, member, found, ref):
    """Judged in the Runtime's reader's order: `min` and `max` integers, `step` absent or an integer, then `min` not above
    `max`, then `step` at least 1; the first that breaks is reported."""
    path = write_control(example, ONE, control("pkg.one", item))
    bind_defaults(example, BINDING)
    detail = {"path": str(path), "entry": "/defaults/0", "variable": "floor", "member": member}
    if found is not None:
        detail["found"] = found

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert doc["errors"][0] == {"code": "variable_range_invalid", "detail": detail, "ref": ref}
    assert bindings(acmodel, example, "apply")[0] == 6


@pytest.mark.parametrize(
    ("item", "code", "values"),
    [
        (floor(max=100), "default_out_of_range", {"min": "0", "max": "100"}),
        (floor(min=300), "default_out_of_range", {"min": "300", "max": "1000"}),
        (floor(step=10), "default_off_step", {"min": "0", "step": "10"}),
    ],
    ids=["above_max", "below_min", "off_step"],
)
def test_a_number_the_declaration_cannot_hold_is_refused_never_rounded_or_clamped(example, acmodel, item, code, values):
    """215 above a `max` of 100, below a `min` of 300, or between the steps 210 and 220 of a `step` of 10: refused (exit
    5), at the default, and nothing is written."""
    path = write_control(example, ONE, control("pkg.one", item))
    bind_defaults(example, BINDING)
    before = path.read_bytes()
    expected = {"code": code, "detail": {"path": str(path), "entry": "/defaults/0", "threshold": "thr.daily.floor",
                                         "variable": "floor", **values, "expected": "215"},
                "ref": "/variables/0/default"}

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"][0]) == (5, expected)

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (5, [expected])
    assert path.read_bytes() == before


def test_a_step_counts_from_min(example, acmodel):
    """With `min` 5 and `step` 10, 215 lies on a step (5 + 21 × 10): it is written."""
    path = write_control(example, ONE, control("pkg.one", floor(min=5, step=10, default=5)))
    bind_defaults(example, BINDING)

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert json.loads(path.read_text(encoding="utf-8"))["variables"][0]["default"] == FLOOR


@pytest.mark.parametrize(
    ("item", "values", "ref"),
    [(without(floor(), "default"), {}, "/variables/0"), (floor(default="0"), {"found": "string"}, "/variables/0/default")],
    ids=["absent", "a_text"],
)
def test_a_declaration_without_an_integer_default_is_refused_and_none_is_added(example, acmodel, item, values, ref):
    path = write_control(example, ONE, control("pkg.one", item))
    bind_defaults(example, BINDING)
    before = path.read_bytes()
    expected = {"code": "binding_mismatch",
                "detail": {"path": str(path), "entry": "/defaults/0", "reason": "literal_missing",
                           "threshold": "thr.daily.floor", "variable": "floor", **values},
                "ref": ref}

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"][0]) == (6, expected)
    assert path.read_bytes() == before


# The threshold ----------------------------------------------------------------------------------------------------------


def test_a_fact_offset_threshold_is_the_wrong_form(example, acmodel):
    two_packs(example, floor(), floor())
    bind_defaults(example, {"threshold": "thr.run.trigger", "variable": "floor"})
    expected = [{
        "code": "binding_mismatch",
        "detail": {"path": str(example / MODEL / "bindings.json"), "entry": "/defaults/0", "reason": "form",
                   "threshold": "thr.run.trigger", "variable": "floor"},
        "ref": "/defaults/0/threshold",
    }]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, example, command)

        assert (status, doc["errors"]) == (6, expected)


# reserve.a as a provisional derive of its own, and a constant threshold of it.
PENDING_RULE = {"id": "reserve.b", "kind": "derive", "subject": "res.a", "logic": {"value": "200"},
                "provenance": [{"kind": "logic", "ref": "fixture"}], "status": "provisional"}


def test_a_pending_threshold_refuses_the_whole_apply_and_once_accepted_is_written_provisional(example, acmodel):
    """`ceiling` takes `thr.reserve.b`, provisional and not accepted: `apply` refuses everything, and does not write
    `floor` either. Accepted in exports.json, it is written, and `check` shows it provisional with its acceptance."""
    model = example / MODEL
    edit(model / "rules" / "a.json", lambda doc: doc["rules"].append(PENDING_RULE))
    edit(model / "exports.json",
         lambda doc: doc["thresholds"].append({"id": "thr.reserve.b", "from": "reserve.b", "form": "constant"}))
    ceiling = {**floor(), "name": "ceiling"}
    one = write_control(example, ONE, control("pkg.one", floor(), ceiling))
    bind_defaults(example, BINDING, {"threshold": "thr.reserve.b", "variable": "ceiling"})
    before = one.read_bytes()
    refused = {"code": "export_refused",
               "detail": {"path": str(model / "exports.json"), "output": "threshold:thr.reserve.b",
                          "reason": "provisional"},
               "ref": "/thresholds/2"}

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["data"], doc["errors"]) == (5, {}, [refused])
    assert one.read_bytes() == before

    accepted = {"output": "threshold:thr.reserve.b", "accept_provisional": {"ref": "R-9", "until": "a device check"}}
    edit(model / "exports.json", lambda doc: doc.update(acceptances=[accepted]))

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (0, [])
    assert [item["default"] for item in json.loads(one.read_text(encoding="utf-8"))["variables"]] == [FLOOR, 200]

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (0, [])
    assert [(entry["status"], entry["acceptances"]) for entry in doc["data"]["defaults"]] == [
        ("confirmed", []),
        ("provisional", [{"output": "threshold:thr.reserve.b", "ref": "R-9", "until": "a device check"}]),
    ]


# bindings.json ----------------------------------------------------------------------------------------------------------


def test_a_variable_bound_twice_or_to_no_threshold_is_refused_before_the_packs_are_read(example, acmodel):
    two_packs(example, floor(), floor())
    path = str(example / MODEL / "bindings.json")
    bind_defaults(example, BINDING, BINDING, {"threshold": "thr.none", "variable": "floor"})

    status, doc = bindings(acmodel, example, "check")

    assert status == 1
    assert doc["errors"] == [
        {"code": "rule_reference_missing", "detail": {"path": path, "target": "thr.none", "expected": ["threshold"]},
         "ref": "/defaults/2/threshold"},
        {"code": "model_id_duplicate",
         "detail": {"path": path, "namespace": "literal", "id": "variable:floor", "first_path": path,
                    "first_ref": "/defaults/0"},
         "ref": "/defaults/1"},
        {"code": "model_id_duplicate",
         "detail": {"path": path, "namespace": "literal", "id": "variable:floor", "first_path": path,
                    "first_ref": "/defaults/0"},
         "ref": "/defaults/2"},
    ]


@pytest.mark.parametrize(
    ("entry", "reason", "ref", "values"),
    [
        ({**BINDING, "file": ONE}, "member_unknown", "/defaults/0/file", {"key": "file"}),
        ({"threshold": "thr.daily.floor"}, "member_missing", "/defaults/0/variable", {"key": "variable"}),
        ({"threshold": "thr.daily.floor", "variable": "Floor"}, "value_invalid", "/defaults/0/variable",
         {"literal": "Floor"}),
    ],
    ids=["a_file", "no_variable", "a_name_out_of_form"],
)
def test_the_reader_holds_a_default_binding_to_its_shape(example, acmodel, entry, reason, ref, values):
    bind_defaults(example, entry)
    expected = {"code": "model_file_invalid",
                "detail": {"reason": reason, "path": str(example / MODEL / "bindings.json"),
                           "format": "acmodel.bindings.v1", **values},
                "ref": ref}

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (1, [expected])


@pytest.mark.parametrize(
    "literal",
    [
        {"threshold": "thr.daily.floor", "file": ONE, "path": "/variables/0/default"},
        {"file": ONE, "path": "/variables", "reason": "not a quantity", "ref": "fixture"},
    ],
    ids=["threshold_binding", "fixed_entry"],
)
def test_a_literal_under_variables_is_bound_only_by_a_default_binding(example, acmodel, literal):
    """A threshold binding or a fixed entry under `/variables` of a pack's control.json would go round the range, the
    step, the origin and the shared name: refused (exit 1) before any file of the repository is read."""
    path = write_control(example, ONE, control("pkg.one", floor()))
    before = path.read_bytes()
    member = "bindings" if "threshold" in literal else "fixed"
    edit(example / MODEL / "bindings.json", lambda doc: doc[member].append(literal))
    index = 3 if member == "bindings" else 1
    expected = [{
        "code": "variable_literal_bound",
        "detail": {"path": str(example / MODEL / "bindings.json"), "entry": f"/{member}/{index}", "file": ONE,
                   "pointer": literal["path"]},
        "ref": f"/{member}/{index}",
    }]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, example, command)

        assert (status, doc["errors"]) == (1, expected)
    assert path.read_bytes() == before


def test_a_threshold_literal_elsewhere_in_a_control_json_is_written_with_the_default(example, acmodel):
    """A threshold binding may still name another literal of a control.json: both edits go into the one file, which stays
    canonical."""
    doc = {**control("pkg.one", floor()), "limits": {"floor": 0}}
    path = write_control(example, ONE, doc)
    bind_defaults(example, BINDING)
    edit(example / MODEL / "bindings.json", lambda bound: bound["bindings"].append(
        {"threshold": "thr.daily.floor", "file": ONE, "path": "/limits/floor"}))

    status, out = bindings(acmodel, example, "apply")

    assert (status, out["errors"]) == (0, [])
    expected = {**control("pkg.one", floor(default=FLOOR, default_origin="model")), "limits": {"floor": FLOOR}}
    assert path.read_bytes() == pretty(expected)
    assert out["data"]["written"] == [str(path)]


# The packs: the guard, the canonical form, what is read ------------------------------------------------------------------


def test_a_model_origin_no_default_binding_names_is_refused(example, acmodel):
    """The origin guard covers every pack's control.json: `"model"` there that no default binding names is
    `origin_unbound`, in `check` and `apply`; bound, it passes."""
    path = write_control(example, THREE, control("pkg.three", floor(default=FLOOR, default_origin="model")))
    expected = [{"code": "origin_unbound",
                 "detail": {"path": str(path), "member": "default_origin", "variable": "floor"},
                 "ref": "/variables/0/default_origin"}]

    for command in ("check", "apply"):
        status, doc = bindings(acmodel, example, command)

        assert (status, doc["errors"]) == (6, expected)

    bind_defaults(example, BINDING)
    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (0, [])


def test_a_control_json_that_declares_a_bound_variable_must_be_canonical(example, acmodel):
    """pkg.one's control.json in one line, pkg.two's in CRLF: each is `pack_file_not_canonical` (fixed apart, with
    `actinglab package fmt`), and nothing is written. pkg.three, compact too, declares no bound variable: not read for
    it."""
    one = write_control(example, ONE, None, json.dumps(control("pkg.one", floor())).encode("utf-8"))
    two = write_control(example, TWO, None, pretty(control("pkg.two", floor())).replace(b"\n", b"\r\n"))
    write_control(example, THREE, None, json.dumps(control("pkg.three", PICK)).encode("utf-8"))
    bind_defaults(example, BINDING)
    before = (one.read_bytes(), two.read_bytes())
    expected = [{"code": "pack_file_not_canonical", "detail": {"path": str(path)}, "ref": ""} for path in (one, two)]

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert [entry for entry in doc["errors"] if entry["code"] == "pack_file_not_canonical"] == expected

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (6, expected)
    assert (one.read_bytes(), two.read_bytes()) == before


def test_every_control_json_is_read_and_an_interrupted_adoptions_folder_is_skipped(example, acmodel):
    """Without default bindings, `check` reads every pack's control.json for the guard (`control_files`) and shows no
    defaults; a folder `<package>.part`, which an interrupted adoption leaves, is no pack and is skipped (its `"model"`
    origin is not reported), and so is a folder without a control.json (the example's pack of a policy file)."""
    write_control(example, ONE, control("pkg.one", PICK))
    write_control(example, "packs/pkg.two.part/control.json", control("pkg.two", floor(default_origin="model")))

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (0, [])
    assert (doc["data"]["defaults"], doc["data"]["control_files"]) == ([], 1)


@pytest.mark.parametrize(
    ("raw", "reason", "ref", "values"),
    [
        (b'{"package_id": "pkg.one", "rate": 0.5}\n', "float_literal", "/rate", {"literal": "0.5"}),
        (pretty({"package_id": "pkg.one", "variables": {}}), "type_invalid", "/variables", {"expected": ["array"]}),
        (pretty({"package_id": "pkg.one", "variables": [{"type": "integer"}]}), "member_missing", "/variables/0/name",
         {"key": "name"}),
    ],
    ids=["a_float", "variables_no_list", "a_variable_without_a_name"],
)
def test_a_control_json_the_tool_cannot_read_refuses_the_check(example, acmodel, raw, reason, ref, values):
    """Every control.json is read, so one the strict reader refuses, or whose variables have no names, refuses `check`
    (exit 1), with or without default bindings."""
    path = write_control(example, ONE, None, raw)

    status, doc = bindings(acmodel, example, "check")

    assert (status, doc["errors"]) == (1, [{
        "code": "model_file_invalid", "detail": {"reason": reason, "path": str(path), "format": "json", **values},
        "ref": ref}])


def test_a_part_left_beside_a_control_json_is_reported_refused_and_kept(example, acmodel):
    """pkg.two's control.json.part is there (a folder, as anything another write left): `check` reports it beside the
    differences, `apply` refuses everything (pkg.one is not written either), and the part and every file keep their
    bytes."""
    one, two = two_packs(example, floor(), floor())
    bind_defaults(example, BINDING)
    part = two.parent / "control.json.part"
    part.mkdir()
    (part / "left.txt").write_bytes(b"another write\n")
    before = (one.read_bytes(), two.read_bytes())
    present = {"code": "part_file_present", "detail": {"path": str(two), "part": str(part)}}

    status, doc = bindings(acmodel, example, "check")

    assert status == 6
    assert doc["errors"][-1] == present
    assert [entry for entry in doc["errors"] if entry["code"] == "part_file_present"] == [present]

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["data"], doc["errors"]) == (5, {}, [present])
    assert (one.read_bytes(), two.read_bytes()) == before
    assert (part / "left.txt").read_bytes() == b"another write\n"
    assert not (one.parent / "control.json.part").exists()


# The checks in memory, and the canonical serialization -------------------------------------------------------------------


def test_edited_bytes_that_fail_a_check_in_memory_are_a_defect_and_nothing_is_written(example, acmodel, monkeypatch):
    """When the edited bytes of a control.json did not read as canonical, `apply` stops (`command_crashed`) before it
    writes anything: no part is written, no file changed."""
    one, two = two_packs(example, floor(), floor())
    bind_defaults(example, BINDING)
    before = (one.read_bytes(), two.read_bytes())
    originals = set(before)
    real = defaults.is_canonical
    monkeypatch.setattr(defaults, "is_canonical", lambda raw, doc: raw in originals and real(raw, doc))

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (1, [{"code": "command_crashed", "detail": {"exception_type": "ValueError"}}])
    assert (one.read_bytes(), two.read_bytes()) == before
    assert not (one.parent / "control.json.part").exists() and not (two.parent / "control.json.part").exists()


def test_check_written_holds_the_canonical_form_and_the_declarations_alike(tmp_path):
    """`defaults.check_written` on its own: edited bytes that are not canonical, a declaration moved, and two declarations
    that were alike and are not after, are each a defect; edits that keep them alike pass."""
    write_control(tmp_path, ONE, control("pkg.one", floor()))
    write_control(tmp_path, TWO, control("pkg.two", floor(), PICK))
    controls = [defaults.read_control(read_target(tmp_path, name)) for name in (ONE, TWO)]
    names = frozenset({"floor"})

    def made(doc: Any) -> tuple[bytes, Any]:
        return pretty(doc), doc

    alike = floor(default=FLOOR, default_origin="model")
    both = {ONE: made(control("pkg.one", alike)), TWO: made(control("pkg.two", alike, PICK))}
    defaults.check_written(controls, names, both)
    for written in (
        {ONE: (json.dumps(control("pkg.one", alike)).encode("utf-8"), control("pkg.one", alike))},
        {ONE: made(control("pkg.one", alike))},
        {TWO: made(control("pkg.two", PICK, floor()))},
    ):
        with pytest.raises(ValueError):
            defaults.check_written(controls, names, written)


def test_check_written_holds_every_declaration_of_a_bound_name_alike_after_the_edits(tmp_path):
    """fix1, a backstop to the plan's agreement: written, every declaration of a bound variable is the same in compact
    JSON, also where they differed before (defaults 5 and 10 here, which the check above does not hold); one left unlike
    is a defect. A variable no binding names is not judged: `pick` differs between the files and passes."""
    other = {**PICK, "default": "item_a"}
    write_control(tmp_path, ONE, control("pkg.one", floor(default=5), other))
    write_control(tmp_path, TWO, control("pkg.two", PICK, floor(default=10)))
    controls = [defaults.read_control(read_target(tmp_path, name)) for name in (ONE, TWO)]
    names = frozenset({"floor"})

    def made(doc: Any) -> tuple[bytes, Any]:
        return pretty(doc), doc

    alike = floor(default=FLOOR, default_origin="model")
    defaults.check_written(controls, names, {ONE: made(control("pkg.one", alike, other)),
                                             TWO: made(control("pkg.two", PICK, alike))})
    for written in (
        {ONE: made(control("pkg.one", alike, other))},
        {ONE: made(control("pkg.one", alike, other)), TWO: made(control("pkg.two", PICK, floor(default=FLOOR)))},
    ):
        with pytest.raises(ValueError, match="differ after the edits"):
            defaults.check_written(controls, names, written)


def test_declarations_the_plan_let_through_unlike_are_a_defect_and_nothing_is_written(example, acmodel, monkeypatch):
    """fix1: were the plan's agreement to miss declarations that stay unlike once written (another title here), the
    check in memory stops `apply` (`command_crashed`) before it writes anything."""
    one, two = two_packs(example, floor(), floor(title={"en": "Keep at most"}))
    bind_defaults(example, BINDING)
    before = (one.read_bytes(), two.read_bytes())
    monkeypatch.setattr(defaults.DefaultPlan, "_agree", lambda self: None)

    status, doc = bindings(acmodel, example, "apply")

    assert (status, doc["errors"]) == (1, [{"code": "command_crashed", "detail": {"exception_type": "ValueError"}}])
    assert (one.read_bytes(), two.read_bytes()) == before
    assert not (one.parent / "control.json.part").exists() and not (two.parent / "control.json.part").exists()


@pytest.mark.parametrize(
    ("raw", "holds"),
    [
        (b'{\n  "a": [],\n  "b": {},\n  "c": "\xc3\xa9",\n  "d": [\n    1,\n    true\n  ]\n}\n', True),
        (b'{\r\n  "a": 1\r\n}\r\n', False),
        (b'{\n  "a": 1\n}', False),
        (b'{\n    "a": 1\n}\n', False),
        (b'{\n  "a": "\\u00e9"\n}\n', False),
        (b'{\n  "a": [\n  ]\n}\n', False),
        (b'{\n  "a" : 1\n}\n', False),
        (b'{\n  "a": "\\n\\u0001"\n}\n', True),
    ],
    ids=["canonical", "crlf", "no_final_lf", "four_spaces", "escaped_text", "empty_list_open", "space_before_colon",
         "control_characters"],
)
def test_the_canonical_serialization_is_the_one_package_fmt_writes(tmp_path, raw, holds):
    path = tmp_path / "file.json"
    path.write_bytes(raw)
    target = read_target(tmp_path, "file.json")

    assert canonical.is_canonical(target.raw, target.doc) is holds


def test_the_codes_of_v_a_are_registered_each_of_its_category():
    assert {code: CODES[code]["exit"] for code in CODES} == EXIT_STATUS
    new = {
        "variable_literal_bound": ("error", 1), "variable_unknown": ("error", 6), "variable_declared_twice": ("error", 6),
        "variable_not_integer": ("error", 6), "variable_range_invalid": ("error", 6),
        "variable_declarations_differ": ("error", 6), "default_out_of_range": ("error", 5),
        "default_off_step": ("error", 5), "pack_file_not_canonical": ("error", 6), "part_file_present": ("error", 5),
    }
    assert {code: (CODES[code]["category"], CODES[code]["exit"]) for code in new} == new
    assert "default_origin" in CODES["origin_unbound"]["values"]["member"]["members"]
    assert {"variable"} <= set(CODES["binding_mismatch"]["values"]) & set(CODES["origin_missing"]["values"])


def test_every_warning_says_its_kind():
    """fix1, the code rule (contracts/formats.md, "Errors and warnings"): the description of every warning says which of
    the two kinds it is, once; the held ones are of the first kind (refused once the hold is lifted), every other of the
    second (refused by neither the Runtime nor the Lab)."""
    phrase = "A warning of the {} kind"
    kinds = {
        code: [kind for kind in ("first", "second") for _ in range(spec["description"].count(phrase.format(kind)))]
        for code, spec in CODES.items() if spec["category"] == "warning"
    }
    assert kinds == {code: ["first" if code.startswith("held_") else "second"] for code in kinds}
    assert {"prices_stale", "values_stale", "valuations_stale", "precision_low", "limited_without_consumes",
            "limited_unmodelled"} <= set(kinds)
