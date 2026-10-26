"""Provenance (#424 design 6.4) and `acmodel explain`.

`assemble(run, output)` gives the provenance document (`acmodel.provenance.v1`) of one output of a run that
is not blocked. `output` is `rule:<id>`, `general_logic:<id>` or `threshold:<id>`, as `needed_by` names
them. The tree is assembled from what each graph node's value read (`graph`, "Reads"), so it holds exactly
what the value rests on.

- `nodes` maps a node id to the node, and the tree is rooted at `output`. Each node lists in `reads` the
  ids of what it read, sorted; a leaf reads nothing. A node read twice is listed once.
- Every path in it is relative to the model folder, with `/`, so that the document can be delivered.
- `explain` prints the document, and `export thresholds` writes it for each threshold.

Node ids:
- a graph node: its key joined with `:`. These are `rule:<id>`, `general_logic:<id>`, `offset:<id>`,
  `threshold:<id>`, `flow:<id>:consumes:<resource>` (or `produces`), `flow:<id>:moves` and
  `member:<unknown>:<member>`, each with `general:` in front for the node from the rules and the data only;
- `row:<table>[<key parts joined with commas>]`, and `entry:<rule id>` for a rule consulted for its logic;
- the account state: `reading:<fact>`, `cleared:<id>`, `entity:<class>[<id>].<attribute>` and
  `entities:<class>`.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from acmodel import __version__
from acmodel.check.checker import Entry
from acmodel.check.files import File
from acmodel.check.walk import at
from acmodel.codes import USAGE_INVALID
from acmodel.core.run import Run, evaluate_model
from acmodel.envelope import Failure, error

SCHEMA_VERSION = "acmodel.provenance.v1"
GRAPH_NODES = ("rule", "general_logic", "offset", "flow", "member", "threshold", "general")
STATE_NODES = ("reading", "cleared", "entity", "entities")


def sha256(doc: Any) -> str:
    """The sha256 of a document's canonical JSON: sorted keys, no whitespace, UTF-8."""
    text = json.dumps(doc, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def relative(run: Run, path: str) -> str:
    return Path(path).relative_to(run.folder).as_posix()


def versions(run: Run) -> dict[str, str]:
    """`rules`: the sha256 over `model.json`, `sources.json` and the rule files; `data`: over the data files.
    Each file counts as [its path relative to the model folder, its document], sorted by path. `tool`: the
    tool version."""
    model = run.checker.model

    def digest(files: list[File]) -> str:
        return sha256(sorted([relative(run, file.path), file.doc] for file in files))

    return {
        "rules": digest([model.manifest, model.sources, *model.rules]),
        "data": digest(list(model.data)),
        "tool": __version__,
    }


def node_id(item: tuple[str, ...]) -> str:
    if item[0] == "row":
        return f"row:{item[1]}[{','.join(item[2:])}]"
    if item[0] == "entity":
        return f"entity:{item[1]}[{item[2]}].{item[3]}"
    return ":".join(item)


def outputs(run: Run) -> dict[str, tuple[str, str]]:
    """Each output id of the run and its graph node."""
    return dict(run.graph.outputs())


def _reads(run: Run, item: tuple[str, ...]) -> list[tuple[str, ...]]:
    if item[0] not in GRAPH_NODES:
        return []
    found: dict[tuple[str, ...], None] = {}
    for scenario in run.graph.results[item].scenarios:  # every variant's reads
        found.update(dict.fromkeys(scenario.reads))
    return sorted(found, key=node_id)


def _entry(run: Run, entry: Entry, ref: str) -> dict[str, Any]:
    """A rule or an unknown: where it is, its hash, its own status and its provenance as written."""
    return {
        "node": "unknown" if entry.category == "unknown" else "rule",
        "kind": entry.kind,
        "id": entry.id,
        "file": relative(run, entry.path),
        "ref": ref,
        "sha256": sha256(entry.doc),
        "status": entry.doc["status"],
        "provenance": entry.doc["provenance"],
    }


def _expression(entry: Entry, key: tuple[str, ...]) -> str:
    """The JSON pointer of the expression (or a cap's reading) that graph node `key` evaluates."""
    logic = entry.doc["logic"]
    if key[0] == "general_logic":
        member: tuple[str, ...] = ("general_logic",)
    elif key[0] == "offset":
        member = ("live", "offset")
    elif key[0] == "flow":
        member = ("moves", "amount") if key[2] == "moves" else key[2:4]
    elif entry.kind == "limit":
        member = ("max",)
    elif entry.kind == "cap" and "value" not in logic:
        member = ("reading",)
    else:
        member = ("value",)
    return at(entry.ref, "logic", *member)


def _describe(run: Run, item: tuple[str, ...]) -> dict[str, Any]:
    kind, checker, graph = item[0], run.checker, run.graph
    if kind == "general":
        return {**_describe(run, item[1:]), "general": True}
    if kind == "threshold":
        index, threshold = next(
            (index, entry) for index, entry in enumerate(run.exports.doc["thresholds"]) if entry["id"] == item[1]
        )
        node = {
            "node": "threshold", "id": item[1], "form": threshold["form"], "file": relative(run, run.exports.path),
            "ref": at("", "thresholds", index), "sha256": sha256(threshold),
        }
        if "round" in threshold:
            node["round"] = threshold["round"]
        return node
    if kind in ("member", "entry"):
        entry = checker.entries[item[1]]
        node = _entry(run, entry, entry.ref)
        return {**node, "member": item[2]} if kind == "member" else node
    if kind == "row":
        table = checker.tables[item[1]]
        index = table.rows[item[2:]]
        row = table.file.doc["rows"][index]
        return {
            "node": "row", "table": item[1], "key": list(item[2:]), "file": relative(run, table.file.path),
            "ref": at("", "rows", index), "sha256": sha256(row), "status": row["status"],
            "provenance": row["provenance"] if "provenance" in row else table.file.doc["provenance"],
        }
    if kind == "reading":
        return {"node": "reading", "fact": item[1], **graph.state["readings"][item[1]]}
    if kind == "cleared":
        return {"node": "cleared", "id": item[1], "value": item[1] in graph.cleared}
    if kind == "entity":
        value = graph.state["entities"][item[1]][item[2]][item[3]]
        return {"node": "entity", "entity_class": item[1], "entity": item[2], "attribute": item[3], "value": value}
    if kind == "entities":
        return {"node": "entities", "entity_class": item[1], "entities": sorted(graph.state["entities"][item[1]])}
    entry = checker.entries[item[1]]  # rule, general_logic, offset, flow
    return _entry(run, entry, _expression(entry, item))


def _result(run: Run, output: str) -> dict[str, Any]:
    """The output as `run` shows it (a rule without its general logic, which is an output of its own)."""
    kind, name = output.split(":", 1)
    if kind == "threshold":
        return run.data["thresholds"][name]
    rule = run.data["rules"][name]
    if kind == "general_logic":
        return rule["general_logic"]
    return {key: value for key, value in rule.items() if key != "general_logic"}


def assemble(run: Run, output: str) -> dict[str, Any]:
    """The provenance document of `output`, an output of `run` that is not blocked."""
    nodes: dict[str, dict[str, Any]] = {}
    todo = [outputs(run)[output]]
    while todo:
        item = todo.pop()
        name = node_id(item)
        if name in nodes:
            continue
        found = _reads(run, item)
        nodes[name] = {**_describe(run, item), "reads": [node_id(read) for read in found]}
        todo.extend(found)
    return {
        "schema_version": SCHEMA_VERSION,
        "model_id": run.head["model_id"],
        "output": output,
        "result": _result(run, output),
        "versions": versions(run),
        "nodes": dict(sorted(nodes.items())),
    }


def explain(
    path: Path | str, output: str, state_path: Path | str, *, datasets: Path | str, max_bits: int
) -> dict[str, Any]:
    """`acmodel explain`: the provenance document of one output for one account state.

    An id that is not an output of the model is `usage_invalid` (`output_unknown`). A blocked output has no
    tree: the errors that block it are raised.
    """
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits)
    key = outputs(run).get(output)
    if key is None:
        raise Failure(error(USAGE_INVALID, {"reason": "output_unknown", "argument": output}))
    if run.graph.results[key].blocked:
        raise Failure(*(entry for entry in run.errors if output in entry["needed_by"]))
    return {**run.head, "output": output, "provenance": assemble(run, output)}
