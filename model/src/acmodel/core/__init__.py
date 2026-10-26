"""The computation core (#424 design section 4).

PY-4: `run_model` (`run`) runs the static check, the run's model checks (`types`: dimensions,
preference keys, what the evaluation needs) and the derivation graph (`graph`) for one account state.
PY-5: `provenance` assembles the derivation tree of an output from what the graph read (`explain`).
PY-8: `unknown` gives the points of the unknowns, each output's sensitivity with the no-effect rule, and the
Monte Carlo sampling of distributions.
PY-9: with a cache session (`acmodel.cache`), the graph computes incrementally (`Graph.cached`).
"""

from acmodel.core.run import run_model

__all__ = ["run_model"]
