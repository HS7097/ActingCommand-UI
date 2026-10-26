"""The host layer: the caps of the tool's own processes and the gate that decides when compute
may run (contracts/cli.md, Host caps).

- `caps`: the configuration keys and their defaults (`Config.host`).
- `gate`: the memory gates, quiet windows, the PAUSE file, the CPU-load pause and the
  planned-memory report, over a `Host`, the OS layer (the tests pass a fake).
- `windows`: the real `Host` on Windows, with typed ctypes. `current(sys.platform).apply(caps)`
  sets and verifies below-normal priority and puts the process into its Job object.
"""

from acmodel.host.caps import HostCaps, QuietWindow
from acmodel.host.gate import CHECKPOINT, START, Gate, Host, SystemClock, Waited, check_plan, pause_file_present

__all__ = [
    "CHECKPOINT",
    "START",
    "Gate",
    "Host",
    "HostCaps",
    "QuietWindow",
    "SystemClock",
    "Waited",
    "check_plan",
    "pause_file_present",
]
