"""A neutral fake engine for the engine host tests (`acmodel.engine.v2`).

Each request names in `do` what the engine does with it:
- `double`: metric `value` is `x` times the scale of data/values.json; flag `batch:<size of the batch>`;
- `echo` (search): gives back `payload`;
- `adapter_error`: the domain failure `{"error": {"code": <code>, "detail": ...}}`;
- `raise`: raises ValueError;
- `not_json`: a set where a number belongs;
- `set_order`: the flags are the words in the iteration order of a set of them;
- `pid`: flag `pid:<process id>`;
- `crash`: exits the process at once, the first `times` times (counted by marker files in the scratch
  directory), then answers as `double`;
- `print`: prints a lot to stdout and stderr, then answers as `double`;
- `write_outside`, `read_unlisted`, `import_unlisted`, `argv`: what the guards must catch.
"""

import importlib
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.dirname(HERE)
PRINT_MARKER = "fake-engine-print-marker"


class FakeEngine:
    def __init__(self, config, scratch):
        self.scratch = scratch
        with open(os.path.join(config["root"], "data", "values.json"), encoding="utf-8") as handle:
            self.scale = json.load(handle)["scale"]

    def describe(self):
        return {
            "engine_id": "fake",
            "version": "1",
            "ops": ["describe", "evaluate", "search"],
            "metrics": ["value"],
            "entity_classes": ["unit"],
            "max_batch": 2,
        }

    def evaluate(self, requests):
        return [self.one(request, len(requests)) for request in requests]

    def search(self, request):
        if request["do"] == "echo":
            return {"echo": request["payload"], "depends_on": []}
        return self.one(request, 1)

    def reset(self):
        pass

    def double(self, request, batch):
        return {"metrics": {"value": request["x"] * self.scale}, "flags": [f"batch:{batch}"], "depends_on": []}

    def one(self, request, batch):
        do = request["do"]
        if do == "double":
            return self.double(request, batch)
        if do == "adapter_error":
            return {"error": {"code": request["code"], "detail": {"x": request["x"]}}}
        if do == "raise":
            raise ValueError("a fake failure")
        if do == "not_json":
            return {"metrics": {"value": {1, 2}}, "depends_on": []}
        if do == "set_order":
            words = set(request["words"])
            return {"metrics": {"value": len(words)}, "flags": list(words), "depends_on": []}
        if do == "pid":
            return {"metrics": {"value": 0}, "flags": [f"pid:{os.getpid()}"], "depends_on": []}
        if do == "crash":
            done = len([name for name in os.listdir(self.scratch) if name.startswith("crash-")])
            if done < request["times"]:
                with open(os.path.join(self.scratch, f"crash-{done}"), "w", encoding="utf-8"):
                    pass
                os._exit(17)
            return self.double(request, batch)
        if do == "print":
            line = PRINT_MARKER + " " + "x" * 1000 + "\n"
            for _ in range(200):  # more than a pipe's buffer, on both streams
                sys.stdout.write(line)
                sys.stderr.write(line)
            print(json.dumps({"tag": "result", "looks_like": "a frame"}))
            sys.stdout.flush()
            os.write(1, b"\x80\x04raw bytes on fd 1\n")
            return self.double(request, batch)
        if do == "write_outside":
            with open(os.path.join(HOME, "out.txt"), "w", encoding="utf-8") as handle:
                handle.write("outside")
            return self.double(request, batch)
        if do == "read_unlisted":
            with open(os.path.join(HOME, "notes.txt"), encoding="utf-8") as handle:
                handle.read()
            return self.double(request, batch)
        if do == "import_unlisted":
            importlib.import_module("extra_mod")
            return self.double(request, batch)
        if do == "argv":
            sys.argv.append("changed")
            return self.double(request, batch)
        raise KeyError(do)


def make_engine(config, scratch):
    return FakeEngine(config, scratch)
