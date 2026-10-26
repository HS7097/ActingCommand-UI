"""The id grammars of the model files (contracts/formats.md, "Ids").

One definition serves the strict readers (`model/readers.py`) and the expression lexer
(`expr/syntax.py`), so that every id and fact key a reader accepts is exactly one an expression
can name, and the reverse.
"""

ID_MAX = 96
FACT_KEY_MAX = 128
UNKNOWN_PREFIX = "unk."

_ID_FIRST = "a-z"
_ID_REST = "a-z0-9_.-"
_FACT_FIRST = "a-z0-9"
_FACT_REST = "a-z0-9._:-"

# An id: entities, rules, periods, tables, columns, sources, engines, variants, preference keys.
ID = f"[{_ID_FIRST}][{_ID_REST}]{{0,{ID_MAX - 1}}}"
# An unknown id is an id that starts with `unk.`.
UNKNOWN_ID = rf"unk\.[{_ID_REST}]{{1,{ID_MAX - len(UNKNOWN_PREFIX)}}}"
# A fact key, the Runtime's identifier grammar: readings and `read:` references.
FACT_KEY = f"[{_FACT_FIRST}][{_FACT_REST}]{{0,{FACT_KEY_MAX - 1}}}"

# The lexer takes the whole run of these characters, then refuses a run longer than the maximum.
ID_RUN = f"[{_ID_FIRST}][{_ID_REST}]*"
FACT_KEY_RUN = f"[{_FACT_FIRST}][{_FACT_REST}]*"
