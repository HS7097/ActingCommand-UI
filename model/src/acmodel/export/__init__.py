"""Exports into the model folder (#424 design section 6).

PY-5: `export_thresholds` (`acmodel export thresholds`), `thresholds.json` and its provenance files.
"""

from acmodel.export.thresholds import export_thresholds

__all__ = ["export_thresholds"]
