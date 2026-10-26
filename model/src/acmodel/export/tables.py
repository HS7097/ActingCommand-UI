"""`acmodel export tables` (#424 design 6.1, 6.6; design-py 6.1): the tables of exports.json, as the Runtime's
shared tables (`actingcommand.table.v1`), into the working tree of a resource repository.

1. **The validator first.** Without `paths.actinglab` the export is `export_validator_missing` and writes
   nothing; the tool keeps no copy of the Runtime's limits.
2. **The run.** The model is evaluated for the state as `run` does, without the cache.
3. **The cells.** A `key` or `enum_string` cell is its literal. An `integer` or `boolean` cell is the output
   that its `$model` names, by the export policy (`export.policy`): a number scaled and rounded as its column
   declares, `null` for a provisional output that is not accepted, and `export_refused` for a blocked or
   per-account one. A refused cell refuses the export, and nothing is written.
4. **The files.** `<repo>/tables/<table_id>.json` holds each table as canonical JSON (`export.jcs`), and
   `<model folder>/out/provenance/<kind>/<id>.json` the provenance document of each output a cell delivers, as
   `explain` prints it.
5. **Validation.** For each table written: `<paths.actinglab> --json resource validate --repo <repo>
   --changed-path tables/<table_id>.json` (`export.validator`).

Every file is written as one change: when a write fails, the validator refuses a table or gives no usable
answer, each file written gets back its previous bytes (a file that was not there is removed again, and so is a
folder this export created), and the export fails with every error. It never commits, and it deletes nothing
else: not even a table that exports.json no longer lists.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from acmodel.check.walk import at
from acmodel.codes import EXPORT_WRITE_FAILED, MODEL_ID_DUPLICATE, RULE_REFERENCE_MISSING
from acmodel.config import Unknowns
from acmodel.core.run import Run, evaluate_model
from acmodel.envelope import Failure, error
from acmodel.export import jcs
from acmodel.export.policy import Policy, Refused, blocking, delivered, provenance_name, refused
from acmodel.export.validator import Validator, validate
from acmodel.model.readers import FILLED_COLUMNS

SCHEMA_VERSION = "actingcommand.table.v1"
TABLES = "tables"
# The members of a declared column that the table keeps as they are; `valid_until` becomes `valid_until_unix_s`.
COLUMN_MEMBERS = ("min", "max", "allowed", "nullable")


def _check(run: Run, policy: Policy) -> list[dict[str, Any]]:
    """Each table id once, each `valid_until` a window, and each filled cell an output of its column's type."""
    path, errors = run.exports.path, []
    first: dict[str, str] = {}
    for index, table in enumerate(run.exports.doc["tables"]):
        ref = at("", "tables", index, "table_id")
        if table["table_id"] in first:
            errors.append(error(MODEL_ID_DUPLICATE, {
                "path": path, "namespace": "export_table", "id": table["table_id"], "first_path": path,
                "first_ref": first[table["table_id"]],
            }, ref))
        first.setdefault(table["table_id"], ref)
        for position, column in enumerate(table["columns"]):
            window = column.get("valid_until")
            entry = run.checker.entries.get(window) if window is not None else None
            if window is not None and (entry is None or entry.kind != "window"):
                values: dict[str, Any] = {"path": path, "target": window, "expected": ["window"]}
                if entry is not None:
                    values["found"] = entry.token
                ref = at("", "tables", index, "columns", position, "valid_until")
                errors.append(error(RULE_REFERENCE_MISSING, values, ref))
        for number, row in enumerate(table["rows"]):
            for column in table["columns"]:
                if column["type"] in FILLED_COLUMNS:
                    ref = at("", "tables", index, "rows", number, column["name"], "$model")
                    output = row[column["name"]]["$model"]
                    found = policy.reference(output, column["type"] == "boolean", "table", path, ref)
                    if found is not None:
                        errors.append(found)
    return errors


def _column(run: Run, column: dict[str, Any]) -> dict[str, Any]:
    """A declared column as the table declares it: `valid_until` (a window) is its end, `valid_until_unix_s`."""
    out = {"name": column["name"], "type": column["type"]}
    out.update({member: column[member] for member in COLUMN_MEMBERS if member in column})
    if "valid_until" in column:
        out["valid_until_unix_s"] = run.checker.entries[column["valid_until"]].doc["ends_at_unix_s"]
    return out


@dataclass
class _Tables:
    """The files of the export: each table's bytes and each provenance document, with what `data` reports."""

    files: dict[Path, bytes] = field(default_factory=dict)
    tables: list[dict[str, Any]] = field(default_factory=list)
    refusals: list[dict[str, Any]] = field(default_factory=list)


def _build(run: Run, policy: Policy, repo: Path) -> _Tables:
    out, path = _Tables(), run.exports.path
    documents: dict[str, dict[str, Any]] = {}
    for index, table in enumerate(run.exports.doc["tables"]):
        rows, nulled, accepted = [], [], set()
        for number, row in enumerate(table["rows"]):
            cells: list[Any] = []
            for column in table["columns"]:
                cell = row[column["name"]]
                if column["type"] not in FILLED_COLUMNS:
                    cells.append(cell)
                    continue
                output, ref = cell["$model"], at("", "tables", index, "rows", number, column["name"])
                rounding = (column["scale"], column["round"]) if column["type"] == "integer" else None
                found = policy.resolve(output, rounding)
                if isinstance(found, Refused):
                    out.refusals.append(refused(path, output, ref, found))
                    cells.append(None)
                    continue
                cells.append(found.value)
                if found.value is None:
                    nulled.append({"row": number, "column": column["name"], "output": output})
                    continue
                documents[provenance_name(output)] = found.document
                if found.accepted:
                    accepted.add(output)
            rows.append(cells)
        doc = {
            "schema_version": SCHEMA_VERSION, "game": table["game"], "table_id": table["table_id"],
            "row_key": table["row_key"], "columns": [_column(run, column) for column in table["columns"]],
            "rows": rows,
        }
        target = repo / TABLES / f"{table['table_id']}.json"
        out.files[target] = jcs.dumps(doc)
        out.tables.append({
            "table_id": table["table_id"], "file": str(target), "nulled": nulled, "accepted": sorted(accepted),
        })
    for name in sorted(documents):
        out.files[run.folder / name] = delivered(documents[name])
    return out


def _write_failed(target: Path, exc: OSError) -> dict[str, Any]:
    detail = {"path": str(target), "exception_type": type(exc).__name__, "raw_text": str(exc)}
    return error(EXPORT_WRITE_FAILED, detail)


class _Change:
    """Files written as one change: each one's previous bytes (None when it was not there) and the folders it
    created, so that `restore` puts everything back."""

    def __init__(self) -> None:
        self.previous: dict[Path, bytes | None] = {}
        self.folders: list[Path] = []

    def write(self, target: Path, data: bytes) -> None:
        try:
            before = target.read_bytes()
        except FileNotFoundError:
            before = None
        missing = []
        folder = target.parent
        while not folder.exists():
            missing.append(folder)
            folder = folder.parent
        for folder in reversed(missing):
            folder.mkdir()
            self.folders.append(folder)
        self.previous[target] = before
        target.write_bytes(data)

    def restore(self) -> list[dict[str, Any]]:
        """Put back every file and remove the folders created; each one that cannot be is `export_write_failed`."""
        errors = []
        for target, before in reversed(list(self.previous.items())):
            try:
                if before is None:
                    target.unlink(missing_ok=True)
                else:
                    target.write_bytes(before)
            except OSError as exc:
                errors.append(_write_failed(target, exc))
        for folder in reversed(self.folders):
            try:
                folder.rmdir()
            except OSError as exc:
                errors.append(_write_failed(folder, exc))
        return errors


def export_tables(
    path: Path | str,
    state_path: Path | str,
    repo: Path | str,
    *,
    datasets: Path | str,
    max_bits: int,
    validator: Validator,
    unknowns: Unknowns = Unknowns(),
) -> dict[str, Any]:
    """Evaluate the model for the state, then write, validate and keep the tables and their provenance files,
    or refuse with nothing changed."""
    validator.require()
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    policy = Policy(run, refuse_provisional=False)
    errors = _check(run, policy)
    if errors:
        raise Failure(*errors)
    repo = Path(repo)
    built = _build(run, policy, repo)
    if built.refusals:
        raise Failure(*built.refusals, *blocking(run, built.refusals))
    change = _Change()
    for target, data in built.files.items():
        try:
            change.write(target, data)
        except OSError as exc:
            raise Failure(_write_failed(target, exc), *change.restore()) from exc
    problems = []
    try:
        for table in built.tables:
            relative = f"{TABLES}/{table['table_id']}.json"
            problems.extend(validate(validator, repo, relative, Path(table["file"]), table["table_id"]))
    except Exception:  # a defect: the files go back all the same, then the command crashes loudly
        change.restore()
        raise
    if problems:
        raise Failure(*problems, *change.restore())
    written = sorted(str(target) for target in built.files)
    return {**run.head, "repo": str(repo), "tables": built.tables, "written": written}
