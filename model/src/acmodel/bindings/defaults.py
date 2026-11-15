"""Variable defaults (#466 V-a): the default of an integer variable of the packs' `control.json`, written from a
constant threshold of exports.json, with who wrote it, `"default_origin": "model"`, right after it.

A variable's name is a name of the game root (the Runtime's `contracts/package-variables.md`, "Shared names and the root
check"): several packages may declare one name, and then only with byte-identical declarations (each entry in compact
JSON, member order included), or none of them applies (`variable_conflict`). So a default binding of bindings.json
(`defaults`, `{threshold, variable}`) names the variable, never a file or a place: the tool reads every
`packs/<folder>/control.json` of the repository (a folder whose name ends in `.part`, which an interrupted adoption leaves,
is skipped; the Runtime's `contracts/standard-pack-authoring.md`, "If a call was interrupted") and writes the same bytes
into every declaration of that name.

Of a bound declaration it judges what the Runtime's reader judges of an integer variable (`parse_package_variables`,
`integer_variable` in the contract crate's `package_variables.rs`), and what the Runtime and the Lab would refuse once the
number is written, each reason one code:
- the name is declared by some pack (`variable_unknown`), once in a file (`variable_declared_twice`; the Runtime's
  `variable_name_taken`), as an `integer` (`variable_not_integer`), with a range that holds a value (`min` and `max`
  integers, `step` absent or an integer, `min` not above `max`, `step` at least 1: `variable_range_invalid`, at the first
  member that breaks it, in the reader's order);
- the number lies in `min..=max` (`default_out_of_range`) on a step from `min` (`default_off_step`): the tool never rounds
  it or clamps it;
- the declaration holds an integer `default` (`binding_mismatch` `literal_missing`; the tool never adds the member) and
  an origin that is absent or `"model"` (`origin_invalid` otherwise, never rewritten; absent is `origin_missing`, which
  `apply` inserts right after the default);
- written, every declaration of the name is the same in compact JSON (`variable_declarations_differ`): what differs only
  in the default and an absent origin is made one by the write, anything else is a conflict the write would leave;
- every control.json that declares a bound variable is in the canonical serialization (`pack_file_not_canonical`;
  `canonical`): the tool changes only the bytes it must, so a file that is not would stay so, and the Lab's
  `package fmt --check` refuses it.

`default_origin: "model"` that no default binding names is `origin_unbound` (the origin guard, over every control.json).
A threshold binding or a fixed entry that names a literal under `/variables` of a pack's control.json is
`variable_literal_bound`: a variable's default goes through a default binding, which judges its range and its step,
writes its origin and keeps the declarations of the name one.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from fractions import Fraction
from pathlib import Path
from typing import Any, Iterable, Iterator

from acmodel.bindings import splice
from acmodel.bindings.canonical import is_canonical
from acmodel.bindings.scanner import MISSING, resolve
from acmodel.bindings.targets import POLICY_FORMAT, Target, integer, shown
from acmodel.check.walk import at
from acmodel.codes import (
    BINDING_MISMATCH,
    DEFAULT_OFF_STEP,
    DEFAULT_OUT_OF_RANGE,
    INSERT_STYLE_UNKNOWN,
    MODEL_ID_DUPLICATE,
    ORIGIN_INVALID,
    ORIGIN_MISSING,
    ORIGIN_UNBOUND,
    PACK_FILE_NOT_CANONICAL,
    VARIABLE_DECLARATIONS_DIFFER,
    VARIABLE_DECLARED_TWICE,
    VARIABLE_LITERAL_BOUND,
    VARIABLE_NOT_INTEGER,
    VARIABLE_RANGE_INVALID,
    VARIABLE_UNKNOWN,
)
from acmodel.envelope import Failure, error
from acmodel.model.shape import Context, json_type

DEFAULTS = "defaults"  # the member of bindings.json
PACKS, CONTROL, PART = "packs", "control.json", ".part"
VARIABLES = "variables"
NAME, TYPE, MIN, MAX, STEP, DEFAULT, ORIGIN = "name", "type", "min", "max", "step", "default", "default_origin"
INTEGER, MODEL = "integer", "model"
# A pack's control.json, as bindings.json names a file: relative to the repository, `/`-separated.
CONTROL_FILE = re.compile(r"packs/[^/]+/control\.json")
RATIO_LOW, RATIO_HIGH = Fraction(1, 2), Fraction(2)  # a changed default outside these times its old one is flagged


def entries(doc: dict[str, Any]) -> Iterator[tuple[str, dict[str, Any]]]:
    """(pointer in bindings.json, entry) of every default binding (absent means none)."""
    for index, entry in enumerate(doc.get(DEFAULTS, [])):
        yield at("", DEFAULTS, index), entry


def bound(doc: dict[str, Any]) -> frozenset[str]:
    """The names of the variables the default bindings name."""
    return frozenset(entry["variable"] for _, entry in entries(doc))


def references(path: str, doc: dict[str, Any], literals: Iterable[tuple[str, dict[str, Any]]]) -> list[dict[str, Any]]:
    """What bindings.json alone refuses of the defaults, before any file of the repository is read: a threshold binding
    or a fixed entry (`literals`, each with its pointer) that names a literal under `/variables` of a pack's control.json
    is `variable_literal_bound`, at the entry; a second default binding of one variable is `model_id_duplicate`
    (`namespace` `literal`, `id` `variable:<name>`): one literal named twice."""
    errors = []
    for ref, entry in literals:
        pointer = entry["path"]
        if CONTROL_FILE.fullmatch(entry["file"]) and (pointer == "/" + VARIABLES or pointer.startswith(f"/{VARIABLES}/")):
            detail = {"path": path, "entry": ref, "file": entry["file"], "pointer": pointer}
            errors.append(error(VARIABLE_LITERAL_BOUND, detail, ref))
    seen: dict[str, str] = {}
    for ref, entry in entries(doc):
        name = entry["variable"]
        if name in seen:
            detail = {"path": path, "namespace": "literal", "id": "variable:" + name, "first_path": path,
                      "first_ref": seen[name]}
            errors.append(error(MODEL_ID_DUPLICATE, detail, ref))
        else:
            seen[name] = ref
    return errors


def control_names(repo: Path) -> list[str]:
    """`packs/<folder>/control.json` of each folder of the repository's `packs` that holds one, in the order of the folder
    names. A folder whose name ends in `.part` is skipped: an interrupted adoption leaves it, and it is no pack."""
    packs = repo / PACKS
    if not packs.is_dir():
        return []
    return [
        f"{PACKS}/{folder.name}/{CONTROL}" for folder in sorted(packs.iterdir(), key=lambda found: found.name)
        if folder.is_dir() and not folder.name.endswith(PART) and (folder / CONTROL).is_file()
    ]


@dataclass(frozen=True)
class Control:
    """A pack's control.json, read by the strict reader, and the places of its variables by name (in order; a name
    declared twice has two)."""

    target: Target
    names: dict[str, list[int]]


def read_control(target: Target) -> Control:
    """What the tool needs of a control.json: an object whose `variables` (absent means none) is a list of objects, each
    with a string `name`; otherwise `Failure` (`model_file_invalid`, `format` `json`, at the member). Every other member
    is the Runtime's reader's to judge, and of a bound declaration `DefaultPlan`'s."""
    ctx = Context(target.path, POLICY_FORMAT)
    names: dict[str, list[int]] = {}
    doc = target.doc
    if not isinstance(doc, dict):
        ctx.refuse("type_invalid", "", expected=["object"])
    elif VARIABLES in doc and not isinstance(doc[VARIABLES], list):
        ctx.refuse("type_invalid", "/" + VARIABLES, expected=["array"])
    else:
        for index, item in enumerate(doc.get(VARIABLES, [])):
            ref = at("", VARIABLES, index)
            if not isinstance(item, dict):
                ctx.refuse("type_invalid", ref, expected=["object"])
            elif NAME not in item:
                ctx.refuse("member_missing", at(ref, NAME), key=NAME)
            elif not isinstance(item[NAME], str):
                ctx.refuse("type_invalid", at(ref, NAME), expected=["string"])
            else:
                names.setdefault(item[NAME], []).append(index)
    if ctx.errors:
        raise Failure(*ctx.errors)
    return Control(target, names)


def compact(item: Any) -> str:
    """A declaration as the Runtime compares two of them: compact JSON, its members in their order."""
    return json.dumps(item, ensure_ascii=False, separators=(",", ":"))


def unbound_origins(controls: list[Control], names: frozenset[str]) -> list[dict[str, Any]]:
    """The origin guard over the packs (#466 V-a), always on: `origin_unbound` (`member` `default_origin`, with
    `variable`, at the member) for each declaration whose `default_origin` is `"model"` and whose name no default
    binding names (`names`): only the model's export writes model. Any other origin is the Runtime's to refuse."""
    errors = []
    for control in controls:
        for index, item in enumerate(control.target.doc.get(VARIABLES, [])):
            if item.get(ORIGIN) == MODEL and item[NAME] not in names:
                detail = {"path": str(control.target.path), "member": ORIGIN, "variable": item[NAME]}
                errors.append(error(ORIGIN_UNBOUND, detail, at("", VARIABLES, index, ORIGIN)))
    return errors


def canonical_files(controls: list[Control], names: frozenset[str]) -> list[Control]:
    """The control.json files that declare a variable a default binding names: each must be canonical."""
    return [control for control in controls if names.intersection(control.names)]


def not_canonical(controls: list[Control], names: frozenset[str]) -> list[dict[str, Any]]:
    """`pack_file_not_canonical` (at the document) for each control.json that declares a bound variable and is not in
    the canonical serialization (`canonical`)."""
    return [
        error(PACK_FILE_NOT_CANONICAL, {"path": str(control.target.path)}, "")
        for control in canonical_files(controls, names) if not is_canonical(control.target.raw, control.target.doc)
    ]


def _declared(docs: dict[str, Any]) -> dict[str, dict[tuple[str, int], str]]:
    """Each name's declarations in the control files `docs` (by file name), by their place, in compact JSON."""
    found: dict[str, dict[tuple[str, int], str]] = {}
    for name, doc in docs.items():
        items = doc.get(VARIABLES, []) if isinstance(doc, dict) else []
        for index, item in enumerate(items if isinstance(items, list) else []):
            if isinstance(item, dict) and isinstance(item.get(NAME), str):
                found.setdefault(item[NAME], {})[(name, index)] = compact(item)
    return found


def check_written(controls: list[Control], names: frozenset[str], written: dict[str, tuple[bytes, Any]]) -> None:
    """The edited bytes of the control files (`written`, each file's bytes and the document they read back as), checked
    in memory before anything is written; a check that fails is a defect (`ValueError`), and nothing is written:
    - each written file that declares a bound variable is in the canonical serialization;
    - every declaration is where it was, and two declarations of a name that were the same in compact JSON before are
      the same after;
    - every declaration of a bound variable is the same in compact JSON after (fix1): a backstop to the plan's
      agreement (`DefaultPlan._agree`, `variable_declarations_differ`), also where they differed before."""
    for control in canonical_files(controls, names):
        name = control.target.name
        if name in written and not is_canonical(*written[name]):
            raise ValueError(f"the edited bytes of {control.target.path} are not canonical")
    before = _declared({control.target.name: control.target.doc for control in controls})
    after = _declared({
        control.target.name: written[control.target.name][1] if control.target.name in written else control.target.doc
        for control in controls
    })
    if {name: set(places) for name, places in before.items()} != {name: set(places) for name, places in after.items()}:
        raise ValueError("the edits moved a variable's declaration")
    for name, places in before.items():
        found = list(places.items())
        for position, (place, text) in enumerate(found):
            for other, other_text in found[position + 1:]:
                if text == other_text and after[name][place] != after[name][other]:
                    raise ValueError(f"the edits made two declarations of {name} differ")
    for name in sorted(names):
        if len(set(after.get(name, {}).values())) > 1:
            raise ValueError(f"the declarations of {name} differ after the edits")


class _Declaration:
    """One declaration of a bound variable: its control.json, its place, the entry as the file holds it, and what `apply`
    writes into it."""

    def __init__(self, control: Control, index: int) -> None:
        self.target = control.target
        self.ref = at("", VARIABLES, index)
        self.item: dict[str, Any] = control.target.doc[VARIABLES][index]
        self.default: Any = self.item.get(DEFAULT, MISSING)
        self.origin: Any = self.item.get(ORIGIN, MISSING)
        self.differs = False  # a difference `apply` writes: the default, or an absent origin
        self.edits: list[splice.Edit] = []
        self.inserted: list[str] = []


class DefaultPlan:
    """What a default binding needs of every declaration of its variable (#466 V-a), once its threshold's number is
    known: the problems both `check` and `apply` refuse, the differences `check` reports and `apply` writes (the default,
    an absent origin), the edits that write them, by file, and what `check` and `apply` show."""

    def __init__(
        self, bindings_path: str, ref: str, entry: dict[str, Any], value: int, status: str,
        acceptances: list[dict[str, str]], controls: list[Control], layouts: Any,
    ) -> None:
        self.ref, self.threshold, self.variable, self.value = ref, entry["threshold"], entry["variable"], value
        self.status, self.acceptances = status, acceptances
        self.problems: list[dict[str, Any]] = []
        self.differences: list[dict[str, Any]] = []
        self.declarations: list[_Declaration] = []
        self.range: tuple[int, int, int] | None = None  # (min, max, step), as the declarations hold it
        declared = False
        for control in controls:
            places = control.names.get(self.variable, [])
            declared = declared or bool(places)
            if len(places) > 1:  # the Runtime refuses the package (`variable_name_taken`): no declaration of it is judged
                detail = self._declared(control.target, first=at("", VARIABLES, places[0]))
                self.problems.extend(
                    error(VARIABLE_DECLARED_TWICE, detail, at("", VARIABLES, index, NAME)) for index in places[1:]
                )
            elif places:
                self.declarations.append(_Declaration(control, places[0]))
        if not declared:
            detail = {"path": bindings_path, "entry": ref, "threshold": self.threshold, "variable": self.variable}
            self.problems.append(error(VARIABLE_UNKNOWN, detail, at(ref, "variable")))
            return
        for declaration in self.declarations:
            self._judge(declaration)
        if not self.problems:
            self._agree()
        if not self.problems:
            self._edits(layouts)

    def _declared(self, target: Target, **values: Any) -> dict[str, Any]:
        return {"path": str(target.path), "entry": self.ref, "variable": self.variable, **values}

    def _detail(self, target: Target, **values: Any) -> dict[str, Any]:
        return {"path": str(target.path), "entry": self.ref, "threshold": self.threshold, "variable": self.variable,
                **values}

    def _bounds(self, declaration: _Declaration) -> tuple[int, int, int] | None:
        """The declaration's range, judged as the Runtime's reader judges it, in its order: `min`, `max` and `step` (absent:
        1) integers, then `min` not above `max`, then `step` at least 1; else `variable_range_invalid` at the first member
        that breaks it (at the declaration when `min` or `max` is absent), and None."""
        item, ref = declaration.item, declaration.ref

        def invalid(member: str, found: Any) -> None:
            values = {} if found is MISSING else {"found": shown(found)}
            detail = self._declared(declaration.target, member=member, **values)
            self.problems.append(error(VARIABLE_RANGE_INVALID, detail, ref if found is MISSING else at(ref, member)))

        for member in (MIN, MAX):
            if not integer(item.get(member, MISSING)):
                invalid(member, item.get(member, MISSING))
                return None
        step = item.get(STEP, 1)
        if not integer(step):
            invalid(STEP, step)
            return None
        low, high = item[MIN], item[MAX]
        if low > high:
            invalid(MIN, low)
            return None
        if step < 1:
            invalid(STEP, step)
            return None
        return low, high, step

    def _judge(self, declaration: _Declaration) -> None:
        """One declaration against the number: its type and range, its default and origin, and whether the number fits."""
        item, ref, target = declaration.item, declaration.ref, declaration.target
        kind = item.get(TYPE, MISSING)
        if kind != INTEGER:
            found = {} if kind is MISSING else {"found": shown(kind)}
            self.problems.append(error(VARIABLE_NOT_INTEGER, self._declared(target, **found),
                                       ref if kind is MISSING else at(ref, TYPE)))
            return
        bounds = self._bounds(declaration)
        default, origin = declaration.default, declaration.origin
        where = ref if default is MISSING else at(ref, DEFAULT)
        if not integer(default):
            found = {} if default is MISSING else {"found": json_type(default)}
            self.problems.append(self._mismatch(target, "literal_missing", where, **found))
        # The origin, as the Runtime reads it: absent is the pack's, `"model"` the model's export's, anything else refused.
        if origin is MISSING:
            self.differences.append(error(ORIGIN_MISSING, self._detail(target), ref))
            declaration.differs = True
        elif origin != MODEL:
            self.problems.append(error(ORIGIN_INVALID, self._detail(target, found=shown(origin)), at(ref, ORIGIN)))
        if bounds is None:
            return
        low, high, step = bounds
        self.range = self.range or bounds
        expected = str(self.value)
        if not low <= self.value <= high:
            detail = self._detail(target, min=str(low), max=str(high), expected=expected)
            self.problems.append(error(DEFAULT_OUT_OF_RANGE, detail, where))
        elif (self.value - low) % step:
            detail = self._detail(target, min=str(low), step=str(step), expected=expected)
            self.problems.append(error(DEFAULT_OFF_STEP, detail, where))
        elif integer(default) and default != self.value:
            self.differences.append(self._mismatch(target, "value", where, expected=expected, found=str(default)))
            declaration.differs = True

    def _mismatch(self, target: Target, reason: str, ref: str, **values: Any) -> dict[str, Any]:
        detail = {"path": str(target.path), "entry": self.ref, "reason": reason, "threshold": self.threshold,
                  "variable": self.variable, **values}
        return error(BINDING_MISMATCH, detail, ref)

    def _written(self, declaration: _Declaration) -> dict[str, Any]:
        """The declaration as `apply` leaves it: the default the number, in its place, and the origin `"model"` in its
        place or right after the default."""
        return splice.placed({**declaration.item, DEFAULT: self.value}, DEFAULT, ORIGIN, MODEL)

    def _agree(self) -> None:
        """Written, every declaration of the name must be the same in compact JSON, as the Runtime compares them
        (`variable_conflict`): `variable_declarations_differ` at each that is not the first's, which the write would
        leave so."""
        first, *others = self.declarations
        text = compact(self._written(first))
        for declaration in others:
            if compact(self._written(declaration)) != text:
                detail = self._declared(declaration.target, first_path=str(first.target.path), first_ref=first.ref)
                self.problems.append(error(VARIABLE_DECLARATIONS_DIFFER, detail, declaration.ref))

    def _edits(self, layouts: Any) -> None:
        """For each declaration with a difference: the default replaced in place, and `"default_origin": "model"`
        inserted right after it in the file's style (the separator after the default, or before it when it is the last
        member; `insert_style_unknown` when it is the only one, and then nothing is written)."""
        for declaration in self.declarations:
            if not declaration.differs:
                continue
            layout = layouts.of(declaration.target)
            default_ref = at(declaration.ref, DEFAULT)
            if declaration.default != self.value:
                declaration.edits.append(layout.integer(declaration.target.doc, default_ref, self.value))
            if declaration.origin is not MISSING:
                continue
            edit = layout.insert_after(default_ref, [(ORIGIN, json.dumps(MODEL).encode("ascii"))])
            if edit is None:
                detail = self._detail(declaration.target, anchor=DEFAULT, member=ORIGIN)
                self.problems.append(error(INSERT_STYLE_UNKNOWN, detail, declaration.ref))
                continue
            declaration.edits.append(edit)
            declaration.inserted.append(ORIGIN)
        if self.problems:
            for declaration in self.declarations:
                declaration.edits, declaration.inserted = [], []

    @property
    def differing(self) -> list[str]:
        """The control files with a difference `apply` writes, in order."""
        return list(dict.fromkeys(declaration.target.name for declaration in self.declarations if declaration.differs))

    @property
    def edits(self) -> dict[str, list[splice.Edit]]:
        """The edits, by the file's name as bindings.json would name it."""
        found: dict[str, list[splice.Edit]] = {}
        for declaration in self.declarations:
            if declaration.edits:
                found.setdefault(declaration.target.name, []).extend(declaration.edits)
        return found

    def expect(self, name: str, doc: Any) -> None:
        """The document the edits make of the file `name`: each declaration's default the number, its origin `"model"`
        right after the default."""
        for declaration in self.declarations:
            if declaration.target.name == name and declaration.edits:
                splice.assign(doc, at(declaration.ref, DEFAULT), self.value)
                splice.assign(doc, declaration.ref, splice.placed(resolve(doc, declaration.ref), DEFAULT, ORIGIN, MODEL))

    def described(self) -> dict[str, Any]:
        """What `check` shows of the binding: the number, its status and the acceptance it rests on, the range the
        declarations hold, and each declaration's file, place, default and origin as they are (the origin null when
        absent)."""
        low, high, step = self.range if self.range is not None else (None, None, None)
        return {
            "entry": self.ref, "threshold": self.threshold, "variable": self.variable, "value": self.value,
            "status": self.status, "acceptances": self.acceptances, "min": low, "max": high, "step": step,
            "declarations": [
                {"file": declaration.target.name, "pointer": declaration.ref, "literal": declaration.default,
                 "origin": None if declaration.origin is MISSING else declaration.origin}
                for declaration in self.declarations
            ],
        }

    def changed(self) -> list[dict[str, Any]]:
        """What `apply` shows of each declaration it writes: the default from and to, their ratio (null when it was 0)
        flagged outside one half to two, the origin from and to, and the member inserted."""
        found = []
        for declaration in self.declarations:
            if not declaration.edits:
                continue
            ratio = None if declaration.default == 0 else Fraction(self.value, declaration.default)
            found.append({
                "entry": self.ref, "threshold": self.threshold, "variable": self.variable,
                "file": declaration.target.name, "pointer": at(declaration.ref, DEFAULT),
                "from": declaration.default, "to": self.value,
                "ratio": None if ratio is None else str(ratio),
                "ratio_flagged": ratio is not None and not RATIO_LOW <= ratio <= RATIO_HIGH,
                "origin": {"from": None if declaration.origin is MISSING else declaration.origin, "to": MODEL},
                "inserted": declaration.inserted,
            })
        return found
