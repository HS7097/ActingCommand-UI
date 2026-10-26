"""Registered codes and their exit statuses.

The catalog with the values of each code is model/contracts/codes.json; the
two must list the same codes with the same exit statuses.
"""

USAGE_INVALID = "usage_invalid"
CONFIG_INVALID = "config_invalid"
DEPS_INVALID = "deps_invalid"
VENV_FAILED = "venv_failed"
COMMAND_CRASHED = "command_crashed"
MODEL_FILE_INVALID = "model_file_invalid"
EXPR_PARSE_ERROR = "expr_parse_error"
EXPR_DIMENSION_MISMATCH = "expr_dimension_mismatch"
EXPR_OVERFLOW = "expr_overflow"
EXPR_EVAL_FAILED = "expr_eval_failed"
MODEL_ID_DUPLICATE = "model_id_duplicate"
RULE_REFERENCE_MISSING = "rule_reference_missing"
DATA_REFERENCE_MISSING = "data_reference_missing"
STATE_REFERENCE_MISSING = "state_reference_missing"
RULE_PROVENANCE_MISSING = "rule_provenance_missing"
RULE_STATUS_INVALID = "rule_status_invalid"
RULE_CONFLICT = "rule_conflict"
RULE_CYCLE = "rule_cycle"
RULE_PENDING_UNBOUNDED = "rule_pending_unbounded"
RULE_PREF_MISPLACED = "rule_pref_misplaced"
DATA_EXTERNAL_MISSING = "data_external_missing"
DATA_EXTERNAL_HASH_MISMATCH = "data_external_hash_mismatch"
HOST_GATE_PAUSED = "host_gate_paused"
HOST_JOB_NOT_APPLIED = "host_job_not_applied"
HOST_PRIORITY_NOT_APPLIED = "host_priority_not_applied"
HOST_READING_FAILED = "host_reading_failed"
STATE_READING_MISSING = "state_reading_missing"
STATE_ENTITY_MISSING = "state_entity_missing"
RULING_OVERRIDES_GENERAL_LOGIC = "ruling_overrides_general_logic"  # a note (category info), never an error
EXPORT_REFUSED = "export_refused"
EXPORT_WRITE_FAILED = "export_write_failed"

EXIT_OK = 0

# 1 model or input error, 2 usage, 4 host gate, 5 export refused (contracts/cli.md lists all of 0-6).
EXIT_STATUS = {
    USAGE_INVALID: 2,
    CONFIG_INVALID: 1,
    DEPS_INVALID: 2,
    VENV_FAILED: 1,
    COMMAND_CRASHED: 1,
    MODEL_FILE_INVALID: 1,
    EXPR_PARSE_ERROR: 1,
    EXPR_DIMENSION_MISMATCH: 1,
    EXPR_OVERFLOW: 1,
    EXPR_EVAL_FAILED: 1,
    MODEL_ID_DUPLICATE: 1,
    RULE_REFERENCE_MISSING: 1,
    DATA_REFERENCE_MISSING: 1,
    STATE_REFERENCE_MISSING: 1,
    RULE_PROVENANCE_MISSING: 1,
    RULE_STATUS_INVALID: 1,
    RULE_CONFLICT: 1,
    RULE_CYCLE: 1,
    RULE_PENDING_UNBOUNDED: 1,
    RULE_PREF_MISPLACED: 1,
    DATA_EXTERNAL_MISSING: 1,
    DATA_EXTERNAL_HASH_MISMATCH: 1,
    HOST_GATE_PAUSED: 4,
    HOST_JOB_NOT_APPLIED: 4,
    HOST_PRIORITY_NOT_APPLIED: 4,
    HOST_READING_FAILED: 4,
    STATE_READING_MISSING: 1,
    STATE_ENTITY_MISSING: 1,
    RULING_OVERRIDES_GENERAL_LOGIC: 0,
    EXPORT_REFUSED: 5,
    EXPORT_WRITE_FAILED: 5,
}
