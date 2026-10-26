"""The Windows leg of the PY-12 tests, run as a process of its own by test_engine.py.

It applies the configured host caps to itself, as the tool does when it starts, then runs one call on the
fake engine through a pool whose host is the real one, so the pool checks every worker it starts
(`verify_child`) and reads its private bytes (`private_mib`). While the workers are alive, it asks the OS
about each of them through its own typed prototypes, and prints one JSON line. pytest's own process is
never capped.

argv: <model/src> <LOCALAPPDATA with the configuration> <a folder for the scratch and logs>
"""

import ctypes
import json
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])

from acmodel import config  # noqa: E402
from acmodel.engine import manifest  # noqa: E402
from acmodel.engine.pool import Pool  # noqa: E402
from acmodel.envelope import Failure  # noqa: E402
from acmodel.host import windows  # noqa: E402

LIBRARIES = {"python": "3.13", "numpy": "2.5.4", "scipy": "1.18.1"}


def kernel32():
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    prototypes = {
        "GetPriorityClass": (windows.DWORD, [windows.HANDLE]),
        "OpenProcess": (windows.HANDLE, [windows.DWORD, windows.BOOL, windows.DWORD]),
        "IsProcessInJob": (windows.BOOL, [windows.HANDLE, windows.HANDLE, ctypes.POINTER(windows.BOOL)]),
        "CloseHandle": (windows.BOOL, [windows.HANDLE]),
    }
    for name, (restype, argtypes) in prototypes.items():
        function = getattr(k32, name)
        function.restype = restype
        function.argtypes = argtypes
    return k32


def main() -> int:
    cfg = config.load({"LOCALAPPDATA": sys.argv[2]})
    work = Path(sys.argv[3])
    host = windows.current(sys.platform)
    report = {"errors": [], "values": [], "workers": []}
    try:
        host.apply(cfg.host)
        engine = manifest.load("fake", roots=cfg.engine.roots, datasets=cfg.datasets, libraries=LIBRARIES)
        with Pool(engine, cfg.engine, scratch_root=work / "scratch", log_dir=work / "logs", os_layer=host,
                  blas_threads=cfg.blas_threads) as pool:
            outcomes = pool.call("evaluate", [{"do": "double", "x": x} for x in (1, 2, 3, 4)])
            report["values"] = [outcome.result["metrics"]["value"] for outcome in outcomes if outcome.error is None]
            report["errors"] = [outcome.error for outcome in outcomes if outcome.error is not None]
            k32 = kernel32()
            for pid in pool.pids():
                handle = k32.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
                in_job = windows.BOOL()
                report["workers"].append({
                    "opened": bool(handle),
                    "priority": k32.GetPriorityClass(handle),
                    "in_job_queried": bool(k32.IsProcessInJob(handle, host.job, ctypes.byref(in_job))),
                    "in_job": bool(in_job.value),
                    "closed": bool(k32.CloseHandle(handle)),
                })
    except Failure as failure:
        report["errors"].extend(failure.errors)
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
