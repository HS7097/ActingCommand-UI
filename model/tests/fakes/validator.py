"""A fake of the Runtime's validator for the PY-14 tests, never the real binary: it answers
`--json resource validate --repo <repo> --changed-path <path>` with one JSON envelope on stdout, in the shape
`actinglab` prints (an `ok` answer with the path's entry, or an `error` with its code, message and details).

The environment variable ACMODEL_FAKE_VALIDATOR holds its instructions as JSON:
- `log`: a file it appends one line to per call: its arguments and the bytes of the file it validated, so a
  test sees what was validated, and that the new bytes were in place then;
- `refuse`: path to [problem, pointer]: it refuses that path as `actinglab` refuses a table, with the problem
  and the pointer in its message (exit 2); every other path is valid (exit 0).
"""

import json
import os
import sys
from pathlib import Path


def envelope(**members):
    return {"schema_version": "0.2", "cli_version": "0.0.0", "runtime_version": "fake",
            "command": "resource validate", **members}


def main(argv):
    control = json.loads(os.environ["ACMODEL_FAKE_VALIDATOR"])
    if argv[:3] != ["--json", "resource", "validate"] or len(argv) != 7:
        print(json.dumps(envelope(ok=False, error={"code": "usage", "message": "fake: unexpected arguments",
                                                   "blocked_by": []})))
        return 2
    flags = dict(zip(argv[3::2], argv[4::2]))
    repo, path = flags["--repo"], flags["--changed-path"]
    data = (Path(repo) / path).read_bytes()
    with open(control["log"], "a", encoding="utf-8") as log:
        log.write(json.dumps({"argv": argv, "bytes": data.decode("utf-8")}) + "\n")
    refusal = control.get("refuse", {}).get(path)
    if refusal is None:
        entry = {"path": path, "status": "valid", "family": "standard_pack_tables"}
        data = {"schema_version": "actinglab.resource-declarations.v1", "scope": "declarations_only", "repo": repo,
                "status": "valid", "entries": [entry], "warnings": [], "read_files": [path], "read_bytes": len(data)}
        print(json.dumps(envelope(ok=True, data=data)))
        return 0
    problem, pointer = refusal
    error = {"code": "resource_declaration_invalid",
             "message": f"{path}: standard_pack_problem={problem} json_pointer={pointer}",
             "blocked_by": [], "details": {"declaration_file": path}}
    print(json.dumps(envelope(ok=False, error=error)))
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
