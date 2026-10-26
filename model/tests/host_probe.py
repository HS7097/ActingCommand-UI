"""The Windows leg of the PY-7 tests, run as a process of its own by test_host.py.

It applies the default host caps to itself through acmodel.host, as the tool does when it starts. Then
it asks the OS, through its own typed prototypes, about itself and about a child it starts afterwards,
and prints one JSON line. pytest's own process is never capped, so no later test runs inside the Job.

argv: <model/src> <a LOCALAPPDATA without a configuration file>
"""

import ctypes
import json
import subprocess
import sys

sys.path.insert(0, sys.argv[1])

from acmodel import config  # noqa: E402
from acmodel.envelope import Failure  # noqa: E402
from acmodel.host import windows  # noqa: E402


def kernel32():
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    prototypes = {
        "GetCurrentProcess": (windows.HANDLE, []),
        "GetPriorityClass": (windows.DWORD, [windows.HANDLE]),
        "QueryInformationJobObject": (
            windows.BOOL,
            [windows.HANDLE, ctypes.c_int, ctypes.c_void_p, windows.DWORD, ctypes.POINTER(windows.DWORD)],
        ),
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
    caps = config.load({"LOCALAPPDATA": sys.argv[2]}).host
    host = windows.current(sys.platform)
    try:
        host.apply(caps)
    except Failure as failure:
        print(json.dumps({"apply_errors": failure.errors}))
        return 1
    k32 = kernel32()
    limits = windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
    rate = windows.JOBOBJECT_CPU_RATE_CONTROL_INFORMATION()
    # A null Job handle asks about the Job of the calling process (the innermost one when nested).
    own_job = [
        bool(k32.QueryInformationJobObject(
            None, windows.JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS, ctypes.byref(limits), ctypes.sizeof(limits), None
        )),
        bool(k32.QueryInformationJobObject(
            None, windows.JOB_OBJECT_CPU_RATE_CONTROL_INFORMATION_CLASS, ctypes.byref(rate), ctypes.sizeof(rate), None
        )),
    ]
    report = {
        "priority": k32.GetPriorityClass(k32.GetCurrentProcess()),
        "own_job_queried": own_job,
        "limit_flags": limits.BasicLimitInformation.LimitFlags,
        "job_priority_class": limits.BasicLimitInformation.PriorityClass,
        "process_memory_limit": limits.ProcessMemoryLimit,
        "cpu_rate_flags": rate.ControlFlags,
        "cpu_rate": rate.CpuRate,
    }
    # The child waits on its stdin until the checks are done: no sleep, no race.
    child = subprocess.Popen([sys.executable, "-I", "-B", "-c", "import sys; sys.stdin.read()"], stdin=subprocess.PIPE)
    try:
        handle = k32.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, False, child.pid)
        in_job = windows.BOOL()
        report["child"] = {
            "opened": bool(handle),
            "priority": k32.GetPriorityClass(handle),
            "in_job_queried": bool(k32.IsProcessInJob(handle, host.job, ctypes.byref(in_job))),
            "in_job": bool(in_job.value),
            "closed": bool(k32.CloseHandle(handle)),
        }
        try:
            host.verify_child(child.pid)
            report["child"]["verify_errors"] = []
        except Failure as failure:
            report["child"]["verify_errors"] = failure.errors
    finally:
        child.stdin.close()
        report["child_exit"] = child.wait(timeout=60)
    print(json.dumps(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
