"""Registered codes and their exit statuses.

The catalog with the values of each code is model/contracts/codes.json; the
two must list the same codes with the same exit statuses.
"""

USAGE_INVALID = "usage_invalid"
CONFIG_INVALID = "config_invalid"
DEPS_INVALID = "deps_invalid"
VENV_FAILED = "venv_failed"
COMMAND_CRASHED = "command_crashed"

EXIT_OK = 0

# 1 model or input error, 2 usage (contracts/cli.md lists all of 0-6).
EXIT_STATUS = {
    USAGE_INVALID: 2,
    CONFIG_INVALID: 1,
    DEPS_INVALID: 2,
    VENV_FAILED: 1,
    COMMAND_CRASHED: 1,
}
