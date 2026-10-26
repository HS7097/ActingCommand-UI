"""acmodel launcher, standard library only.

    py -3 -I -B acmodel.py <command>

Under -I neither this folder nor the cwd is on sys.path, so this file puts
model/src there and hands over to acmodel.cli. That sets the BLAS thread
variables and, unless this process already runs in the configured venv,
re-runs this file under the venv's Python through subprocess and exits with
the child's exit code. `setup` runs here, on the Python that starts it.
See contracts/cli.md and README.md.
"""

import sys
from pathlib import Path


def main() -> int:
    launcher = Path(__file__).resolve()
    sys.path.insert(0, str(launcher.parent / "src"))
    from acmodel import cli, system

    return cli.main(sys.argv[1:], system.current(launcher))


if __name__ == "__main__":
    sys.exit(main())
