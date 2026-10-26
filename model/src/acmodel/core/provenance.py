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
  `threshold:<id>`, `flow:<id>:consumes:<resource>` (or `produces`), `flow:<id>:moves`,
  `member:<unknown>:<member>` and `point:<unknown>` (an interval or a distribution), each with `general:` in
  front for the node from the rules and the data only;
- `row:<table>[<key parts joined with commas>]`, and `entry:<rule id>` for a rule consulted for its logic;
- the account state: `reading:<fact>`, `cleared:<id>`, `entity:<class>[<id>].<attribute>` and
  `entities:<class>`.

A provisional output that `exports.json` accepts (#424 design 6.6) records its acceptance in `acceptance`, so
that every file the exports write says why a provisional value was delivered.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from acmodel.check.checker import Entry
from acmodel.check.walk import at
from acmodel.codes import MODEL_ID_DUPLICATE, RULE_REFERENCE_MISSING, USAGE_INVALID
from acmodel.config import Unknowns
from acmodel.core.common import PROVISIONAL, value_member
from acmodel.core.run import Run, evaluate_model
from acmodel.core.versions import relative as _relative
from acmodel.core.versions import sha256, versions
from acmodel.envelope import Failure, error

SCHEMA_VERSION = "acmodel.provenance.v1"
GRAPH_NODES = ("rule", "general_logic", "offset", "flow", "member", "point", "threshold", "general")
STATE_NODES = ("reading", "cleared", "entity", "entities")


def relative(run: Run, path: str) -> str:
    return _relative(run.folder, path)


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
    elif entry.kind in ("limit", "random"):
        member = value_member(entry)
    elif entry.kind == "cap" and "value" not in logic:
        member = ("reading",)
    else:
        member = ("value",)
    return at(entry.ref, "logic", *member)


def _describe(run: Run, item: tuple[str, ...]) -> dict[str, Any]:
    kind, checker, graph = item[0], run.checker, run.graph
    if kind == "general":
        node = {key: value for key, value in _describe(run, item[1:]).items() if key != "exact_values"}
        return {**node, "general": True, **_exact_values(run, item)}
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
    if kind in ("member", "point", "entry"):
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
    return {**_entry(run, entry, _expression(entry, item)), **_exact_values(run, item)}


def _exact_values(run: Run, item: tuple[str, ...]) -> dict[str, Any]:
    """`exact_values` of a random rule whose value the run carries as its enclosure: per scenario, the exact
    value it encloses."""
    result = run.graph.results.get(item)
    found = [
        {"unknowns": dict(scenario.unknowns), "value": scenario.exact_value}
        for scenario in (result.scenarios if result is not None else ()) if scenario.exact_value is not None
    ]
    return {"exact_values": found} if found else {}


def output_result(run: Run, output: str) -> dict[str, Any]:
    """The output as `run` shows it (a rule without its general logic, which is an output of its own)."""
    kind, name = output.split(":", 1)
    if kind == "threshold":
        return run.data["thresholds"][name]
    rule = run.data["rules"][name]
    if kind == "general_logic":
        return rule["general_logic"]
    return {key: value for key, value in rule.items() if key != "general_logic"}


def acceptances(run: Run) -> dict[str, int]:
    """Each output that `acceptances` of exports.json accepts while provisional, and the index of its entry.

    An entry that names no output of the model is `rule_reference_missing` (`expected` `output`), and a second
    entry of one output `model_id_duplicate` (`namespace` `acceptance`); both refuse whatever reads them.
    """
    path, known = run.exports.path, outputs(run)
    found: dict[str, int] = {}
    errors = []
    for index, entry in enumerate(run.exports.doc.get("acceptances", [])):
        ref, output = at("", "acceptances", index, "output"), entry["output"]
        if output in found:
            detail = {"path": path, "namespace": "acceptance", "id": output, "first_path": path,
                      "first_ref": at("", "acceptances", found[output], "output")}
            errors.append(error(MODEL_ID_DUPLICATE, detail, ref))
        elif output not in known:
            detail = {"path": path, "target": output, "expected": ["output"]}
            errors.append(error(RULE_REFERENCE_MISSING, detail, ref))
        else:
            found[output] = index
    if errors:
        raise Failure(*errors)
    return found


def acceptance(run: Run, output: str) -> dict[str, Any] | None:
    """The acceptance that a provisional `output` is delivered under: where it is in exports.json, its hash and
    its `accept_provisional`; None when the output is not provisional or not accepted."""
    index = acceptances(run).get(output)
    if index is None or output_result(run, output)["status"] != PROVISIONAL:
        return None
    entry = run.exports.doc["acceptances"][index]
    return {
        "file": relative(run, run.exports.path), "ref": at("", "acceptances", index), "sha256": sha256(entry),
        "accept_provisional": entry["accept_provisional"],
    }


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
    doc = {
        "schema_version": SCHEMA_VERSION,
        "model_id": run.head["model_id"],
        "output": output,
        "result": output_result(run, output),
        "versions": versions(run.checker, run.folder),
        "nodes": dict(sorted(nodes.items())),
    }
    accepted = acceptance(run, output)
    if accepted is not None:
        doc["acceptance"] = accepted
    return doc


def explain(
    path: Path | str,
    output: str,
    state_path: Path | str,
    *,
    datasets: Path | str,
    max_bits: int,
    unknowns: Unknowns = Unknowns(),
) -> dict[str, Any]:
    """`acmodel explain`: the provenance document of one output for one account state.

    An id that is not an output of the model is `usage_invalid` (`output_unknown`). A blocked output has no
    tree: the errors that block it are raised.
    """
    run = evaluate_model(path, state_path, datasets=datasets, max_bits=max_bits, unknowns=unknowns)
    key = outputs(run).get(output)
    if key is None:
        raise Failure(error(USAGE_INVALID, {"reason": "output_unknown", "argument": output}))
    if run.graph.blocked_by(key):
        raise Failure(*(entry for entry in run.errors if output in entry["needed_by"]))
    return {**run.head, "output": output, "provenance": assemble(run, output)}
