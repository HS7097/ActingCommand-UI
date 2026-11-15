"""Bindings (#424 design 6.3): `bindings.json` ties literals of `schedule.json` and pack files to thresholds.

PY-6: `check_bindings` (`acmodel bindings check [--strict] [--strict-values]`) and `apply_bindings` (`acmodel bindings
apply`), with a position-tracking scanner that changes only a literal's bytes. #464 W1 adds the rules' values and their
origin, #466 W3 the pools' valuations, their origin and the limited mark, #466 W4a the values written by hand
(`--strict-values`) and the fixed values the Runtime refuses beside another member of their rule.
"""

from acmodel.bindings.commands import apply_bindings, check_bindings

__all__ = ["apply_bindings", "check_bindings"]
