"""The computation core (#424 design section 4).

PY-4: `run_model` (`run`) runs the static check, the run's model checks (`types`: dimensions,
preference keys, what the evaluation needs) and the derivation graph (`graph`) for one account state.
PY-5: `provenance` assembles the derivation tree of an output from what the graph read (`explain`).
PY-8: `unknown` evaluates the unknowns (discrete ones at their points, continuous ones by interval arithmetic,
`expr.interval`), and gives each output's sensitivity, the no-effect rule on the sound range, and the Monte
Carlo sampling of distributions.
PY-9: with a cache session (`acmodel.cache`), the graph computes incrementally (`Graph.cached`).
PY-10: `flow` is the economy (`economy`): the steady state's values and an account's plan, LP and MILP solved
by scipy's HiGHS and checked exactly (`run_economy`).
PY-13: `search` is the generic search for teams: numpy screening, evaluation through the engine host, local
swaps, the proof mode, node budgets, the warm start and the approximation-loss report.
"""

from acmodel.core.run import run_economy, run_model

__all__ = ["run_economy", "run_model"]
