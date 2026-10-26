"""The real host on Windows: kernel32 through ctypes with typed prototypes.

Every function gets `argtypes` and `restype` before its first call. Untyped, `SetPriorityClass`
received a truncated process handle on 64-bit Windows, returned 0 and left the process at normal
priority (the 10-24 lesson). Every result is checked, and every cap is read back before it counts.

`WindowsHost.apply` lowers the priority of the tool's own process and puts the process into a new
Job object that caps CPU, per-process memory and priority. A process started afterwards is inside
the Job from its start and inherits both; `verify_child` checks that for one process.
"""

from __future__ import annotations

import ctypes
from typing import Any, Callable

from acmodel.codes import HOST_JOB_NOT_APPLIED, HOST_PRIORITY_NOT_APPLIED, HOST_READING_FAILED
from acmodel.envelope import Failure, error
from acmodel.host.caps import HostCaps
from acmodel.host.gate import SystemClock

# The Windows types, spelled out: this module also imports where ctypes.wintypes differs, because
# the tests run the logic against a fake kernel32 on Linux too.
HANDLE = ctypes.c_void_p
DWORD = ctypes.c_uint32
BOOL = ctypes.c_int32
SIZE_T = ctypes.c_size_t
ULONG_PTR = ctypes.c_size_t
LARGE_INTEGER = ctypes.c_int64
ULONGLONG = ctypes.c_uint64

MIB = 1 << 20
JOB_OBJECT_LIMIT_PRIORITY_CLASS = 0x00000020
JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x00000100
JOB_OBJECT_CPU_RATE_CONTROL_ENABLE = 0x00000001
JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP = 0x00000004
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS = 9  # JobObjectExtendedLimitInformation
JOB_OBJECT_CPU_RATE_CONTROL_INFORMATION_CLASS = 15  # JobObjectCpuRateControlInformation
PROCESS_QUERY_LIMITED_INFORMATION = 0x00001000
LIMIT_FLAGS = JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_PRIORITY_CLASS
CPU_RATE_FLAGS = JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP


class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", DWORD), ("dwHighDateTime", DWORD)]

    def ticks(self) -> int:
        return self.dwHighDateTime << 32 | self.dwLowDateTime


class MEMORYSTATUSEX(ctypes.Structure):
    _fields_ = [
        ("dwLength", DWORD),
        ("dwMemoryLoad", DWORD),
        ("ullTotalPhys", ULONGLONG),
        ("ullAvailPhys", ULONGLONG),
        ("ullTotalPageFile", ULONGLONG),
        ("ullAvailPageFile", ULONGLONG),
        ("ullTotalVirtual", ULONGLONG),
        ("ullAvailVirtual", ULONGLONG),
        ("ullAvailExtendedVirtual", ULONGLONG),
    ]


class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", LARGE_INTEGER),
        ("PerJobUserTimeLimit", LARGE_INTEGER),
        ("LimitFlags", DWORD),
        ("MinimumWorkingSetSize", SIZE_T),
        ("MaximumWorkingSetSize", SIZE_T),
        ("ActiveProcessLimit", DWORD),
        ("Affinity", ULONG_PTR),
        ("PriorityClass", DWORD),
        ("SchedulingClass", DWORD),
    ]


class IO_COUNTERS(ctypes.Structure):
    _fields_ = [
        (name, ULONGLONG)
        for name in (
            "ReadOperationCount",
            "WriteOperationCount",
            "OtherOperationCount",
            "ReadTransferCount",
            "WriteTransferCount",
            "OtherTransferCount",
        )
    ]


class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION),
        ("IoInfo", IO_COUNTERS),
        ("ProcessMemoryLimit", SIZE_T),
        ("JobMemoryLimit", SIZE_T),
        ("PeakProcessMemoryUsed", SIZE_T),
        ("PeakJobMemoryUsed", SIZE_T),
    ]


class JOBOBJECT_CPU_RATE_CONTROL_INFORMATION(ctypes.Structure):
    # The second member is a union of DWORD CpuRate, DWORD Weight and two WORD rates.
    _fields_ = [("ControlFlags", DWORD), ("CpuRate", DWORD)]


_PROTOTYPES: dict[str, tuple[Any, list[Any]]] = {
    "GetCurrentProcess": (HANDLE, []),
    "SetPriorityClass": (BOOL, [HANDLE, DWORD]),
    "GetPriorityClass": (DWORD, [HANDLE]),
    "CreateJobObjectW": (HANDLE, [ctypes.c_void_p, ctypes.c_wchar_p]),
    "SetInformationJobObject": (BOOL, [HANDLE, ctypes.c_int, ctypes.c_void_p, DWORD]),
    "QueryInformationJobObject": (BOOL, [HANDLE, ctypes.c_int, ctypes.c_void_p, DWORD, ctypes.POINTER(DWORD)]),
    "AssignProcessToJobObject": (BOOL, [HANDLE, HANDLE]),
    "IsProcessInJob": (BOOL, [HANDLE, HANDLE, ctypes.POINTER(BOOL)]),
    "OpenProcess": (HANDLE, [DWORD, BOOL, DWORD]),
    "CloseHandle": (BOOL, [HANDLE]),
    "GlobalMemoryStatusEx": (BOOL, [ctypes.POINTER(MEMORYSTATUSEX)]),
    "GetSystemTimes": (BOOL, [ctypes.POINTER(FILETIME)] * 3),
}


def load_kernel32() -> Any:
    """A private kernel32 handle whose functions are all typed (not the shared ctypes.windll)."""
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    for name, (restype, argtypes) in _PROTOTYPES.items():
        function = getattr(kernel32, name)
        function.restype = restype
        function.argtypes = argtypes
    return kernel32


def current(platform: str) -> WindowsHost:
    """The real host. Only Windows has one: elsewhere the caps cannot be applied, and that is
    `host_priority_not_applied` (`platform_unsupported`), never a run without them."""
    if platform != "win32":
        raise Failure(error(HOST_PRIORITY_NOT_APPLIED, {
            "stage": "platform_unsupported",
            "process": "current",
            "platform": platform,
        }))
    return WindowsHost(load_kernel32(), ctypes.get_last_error)


class WindowsHost(SystemClock):
    """The gate's readings, and the caps of the tool's own process."""

    def __init__(self, kernel32: Any, last_error: Callable[[], int]) -> None:
        self._k32 = kernel32
        self._last_error = last_error
        self._caps: HostCaps | None = None
        # The Job, open for the life of the process (never closed, so it lives as long as we do).
        self.job: int | None = None

    def available_mib(self) -> int:
        status = MEMORYSTATUSEX()
        status.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
        if not self._k32.GlobalMemoryStatusEx(ctypes.byref(status)):
            raise self._reading_failed("memory", "GlobalMemoryStatusEx")
        return status.ullAvailPhys // MIB

    def cpu_times(self) -> tuple[int, int]:
        idle, kernel, user = FILETIME(), FILETIME(), FILETIME()
        if not self._k32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user)):
            raise self._reading_failed("cpu_times", "GetSystemTimes")
        return idle.ticks(), kernel.ticks() + user.ticks()  # kernel time includes idle time

    def apply(self, caps: HostCaps) -> None:
        process = self._k32.GetCurrentProcess()
        if not self._k32.SetPriorityClass(process, caps.priority_class):
            raise _priority_failed("set", "current", caps, os_error=self._last_error())
        self._check_priority(process, "current", caps)
        job = self._k32.CreateJobObjectW(None, None)
        if not job:
            raise _job_failed("create", os_error=self._last_error())
        self.job = job
        limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
        limits.BasicLimitInformation.LimitFlags = LIMIT_FLAGS
        limits.BasicLimitInformation.PriorityClass = caps.priority_class
        limits.ProcessMemoryLimit = caps.worker_memory_mib * MIB
        self._set(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS, limits, "set_limits")
        rate = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION()
        rate.ControlFlags = CPU_RATE_FLAGS
        rate.CpuRate = caps.cpu_rate_milli * 10  # cycles per 10,000
        self._set(job, JOB_OBJECT_CPU_RATE_CONTROL_INFORMATION_CLASS, rate, "set_cpu_rate")
        if not self._k32.AssignProcessToJobObject(job, process):
            raise _job_failed("assign", os_error=self._last_error())
        self._caps = caps
        self._verify_limits(job, caps)
        self._check_membership(process, "current")
        self._check_priority(process, "current", caps)

    def verify_child(self, pid: int) -> None:
        """A process started after `apply` is in the same Job and at the same priority class."""
        if self._caps is None:
            raise RuntimeError("verify_child before apply")
        handle = self._k32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
        if not handle:
            raise self._reading_failed("process", "OpenProcess")
        try:
            self._check_priority(handle, "child", self._caps)
            self._check_membership(handle, "child")
        finally:
            closed = self._k32.CloseHandle(handle)
        if not closed:
            raise self._reading_failed("process", "CloseHandle")

    def _check_priority(self, handle: int, process: str, caps: HostCaps) -> None:
        found = self._k32.GetPriorityClass(handle)
        if found != caps.priority_class:
            values = {"found": found}
            if found == 0:  # the call itself failed
                values["os_error"] = self._last_error()
            raise _priority_failed("verify", process, caps, **values)

    def _check_membership(self, handle: int, process: str) -> None:
        result = BOOL()
        if not self._k32.IsProcessInJob(handle, self.job, ctypes.byref(result)):
            raise _job_failed("verify", limit="membership", process=process, os_error=self._last_error())
        if not result.value:
            raise _job_failed("verify", limit="membership", process=process, expected=1, found=0)

    def _set(self, job: int, info_class: int, info: ctypes.Structure, stage: str) -> None:
        if not self._k32.SetInformationJobObject(job, info_class, ctypes.byref(info), ctypes.sizeof(info)):
            raise _job_failed(stage, os_error=self._last_error())

    def _query(self, job: int, info_class: int, info: ctypes.Structure) -> None:
        if not self._k32.QueryInformationJobObject(job, info_class, ctypes.byref(info), ctypes.sizeof(info), None):
            raise _job_failed("verify", os_error=self._last_error())

    def _verify_limits(self, job: int, caps: HostCaps) -> None:
        limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
        self._query(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS, limits)
        rate = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION()
        self._query(job, JOB_OBJECT_CPU_RATE_CONTROL_INFORMATION_CLASS, rate)
        checks = [
            ("limit_flags", LIMIT_FLAGS, limits.BasicLimitInformation.LimitFlags & LIMIT_FLAGS),
            ("priority_class", caps.priority_class, limits.BasicLimitInformation.PriorityClass),
            ("process_memory", caps.worker_memory_mib * MIB, limits.ProcessMemoryLimit),
            ("cpu_rate_flags", CPU_RATE_FLAGS, rate.ControlFlags & CPU_RATE_FLAGS),
            ("cpu_rate", caps.cpu_rate_milli * 10, rate.CpuRate),
        ]
        for limit, expected, found in checks:
            if found != expected:
                raise _job_failed("verify", limit=limit, expected=expected, found=found)

    def _reading_failed(self, reading: str, call: str) -> Failure:
        return Failure(error(HOST_READING_FAILED, {"reading": reading, "call": call, "os_error": self._last_error()}))


def _priority_failed(stage: str, process: str, caps: HostCaps, **values: Any) -> Failure:
    return Failure(error(HOST_PRIORITY_NOT_APPLIED, {
        "stage": stage,
        "process": process,
        "priority": caps.priority,
        "expected": caps.priority_class,
        **values,
    }))


def _job_failed(stage: str, **values: Any) -> Failure:
    return Failure(error(HOST_JOB_NOT_APPLIED, {"stage": stage, **values}))
