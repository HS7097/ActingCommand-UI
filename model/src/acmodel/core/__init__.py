"""The computation core (#424 design section 4).

PY-4: `run_model` (`run`) runs the static check, the run's model checks (`types`: dimensions,
preference keys, what the evaluation needs) and the derivation graph (`graph`) for one account state.
"""

from acmodel.core.run import run_model

__all__ = ["run_model"]
