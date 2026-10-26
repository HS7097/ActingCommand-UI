# acmodel input files and how they are read

acmodel reads six kinds of input file. Each has one strict reader in `src/acmodel/model/`:

| File | `schema_version` | Reader |
|---|---|---|
| `ours/model/model.json` | `acmodel.model.v1` | `read_model` |
| `ours/model/rules/<topic>.json` | `acmodel.rules.v1` | `read_rules` |
| `ours/model/data/<table>.json` | `acmodel.data.v1` | `read_data` |
| `ours/model/sources.json` | `acmodel.sources.v1` | `read_sources` |
| an account state snapshot (local only) | `acmodel.state.v1` | `read_state` |
| `ours/prefs/defaults.json` (the preference layer, read as it is) | `1` | `read_prefs_defaults` |

A reader returns the file's JSON document only after checking all of it. Otherwise it fails with one
`model_file_invalid` error per problem (`codes.json`):
- `ref` is the JSON pointer (RFC 6901) of the value; the empty pointer means the whole file;
- `detail.path` is the file, and `detail.format` is the reader (`acmodel.rules.v1`, …, `prefs.defaults`);
- `detail.reason` names the problem, and `key`, `expected` or `literal` say more.

**What the readers check:** the JSON level, `schema_version`, every member's type, the id grammar, and that
no member is unknown or missing. **What they do not check:** meaning across members and files, which is the
static check's (`acmodel check`, see "The static check" below). That covers references that resolve,
duplicate ids, status values, non-empty provenance, conflicts and the expressions themselves.

## Reading JSON

| Rule | Refused as `reason` | `ref` |
|---|---|---|
| The file can be read. | `unreadable` (`exception_type`, `raw_text`) | `""` |
| UTF-8 without a byte order mark. | `bom_present`; `utf8_invalid` (`offset` of the first bad byte) | `""` |
| JSON syntax (RFC 8259). | `syntax_invalid` (`line`, `column`) | `""` |
| No object repeats a key. Python's `json` would keep the last one silently. | `key_duplicate` (`key`) | the repeated key |
| No float literal: non-integer numbers are decimal strings (`"0.1"`). `1.0` and `1e3` are floats too. | `float_literal` (`literal`) | the value |
| No `NaN`, `Infinity` or `-Infinity`. | `constant_invalid` (`literal`) | the value |
| Integers lie within ±(2<sup>53</sup> − 1), the Runtime's canonical range. | `integer_out_of_range` (`literal`) | the value |
| Strings are valid Unicode: no lone surrogate from a `\u` escape. | `string_invalid` | the value or key |

## Shapes

| Rule | Refused as `reason` |
|---|---|
| A member must have its type: `string` (never empty), `integer`, `boolean`, `object` or `array`. | `type_invalid` (`expected`) |
| An object holds only the members listed below. | `member_unknown` (`key`) |
| A required member is present. Nothing has a default. | `member_missing` (`key`) |
| Of members that exclude each other, exactly one is present. | `member_conflict` or `member_missing` (`key`, `expected` lists them) |
| A value is one of the listed values, or in its range, or matches its grammar. | `value_invalid` (`expected`, `literal`) |
| An id matches its grammar. | `id_invalid` (`literal`) |

`schema_version` is checked first. If it is missing or wrong, that is the only error reported.

An optional member that is absent has exactly the meaning given here. Where none is given (for example
`counts_on`), a computation that needs it refuses; there is no default.

### Ids

| Grammar | Used for |
|---|---|
| **id**: `[a-z][a-z0-9_.-]{0,95}` | entity, rule, period, table, column, source, model, engine, variant and preference ids; state labels and entity classes |
| **unknown id**: `unk.` followed by `[a-z0-9_.-]{1,92}` | unknowns |
| **fact key**: `[a-z0-9][a-z0-9._:-]{0,127}`, the Runtime's identifier grammar | readings (`reading`, `live.fact`, `live.reference_fact`, the state's `readings`) |

The expression language uses the same grammars (`src/acmodel/ids.py`): the target of `read:` is a fact key,
every other reference target is an id.

### Common values

- **expr**: a non-empty string in the expression language (#424 design §3.4). The reader does not parse it;
  the static check does.
- **date**: `YYYY-MM-DD`, a real calendar date.
- **dimension**: `none`, `time`, `count`, `value` or `amount:<id>`.
- **note**: a non-empty string, never read by the tool.

### Provenance entries

`provenance` is a list of entries; `kind` selects the members. Required members are listed first, then
optional ones in brackets.

| `kind` | Members |
|---|---|
| `source` | `source` (id from `sources.json`), `locator` [`revision`, `quote`] |
| `ruling` | `ref`, `where`, `date` |
| `measurement` | `ref`, `date` [`by`, `evidence`] |
| `guide` | `ref` |
| `logic` | `ref` |
| `pending_check` | `ref` [`by`] |

## `model.json` (`acmodel.model.v1`)

| Member | Value |
|---|---|
| `model_id` | id |
| `files` | `{"rules": [...], "data": [...]}`: the model's files, relative to the folder of `model.json`. Each path is `rules/…json` or `data/…json`, with segments `[a-z0-9][a-z0-9_.-]*`. |
| `periods` | the period entities (resets), see below |
| `numeraire` | the id of the resource that values are counted in |
| `engines` | a list of `{engine_id, interface, min_version}`. `interface` is `acmodel.engine.v2`; `min_version` is digits separated by dots. Where an engine's code lives is local configuration, never this file (#424 design D-4). |
| [`note`] | |

A period is `{id, unit, reset_minute, utc_offset_minutes}` [`note`]:
- `reset_minute` is 0–1439 (the minute of the day of the reset);
- `utc_offset_minutes` is −720 to 840;
- `unit` is `day`, `week` (adds `reset_weekday`: `mon` … `sun`) or `month` (adds `reset_day`: 1–28).

## `rules/<topic>.json` (`acmodel.rules.v1`)

Members: `entities`, `rules` and `unknowns`, all required lists (they may be empty).

**Entities** (`kind` selects the members):

| `kind` | Members |
|---|---|
| `resource` | `id` [`reading`, `note`] |
| `store` | `id`, `holds` (resource id) [`reading`, `note`] |
| `activity` | `id` [`note`] |
| `entity_class` | `id`, `attributes` (attribute id to a column type, as in data files) [`note`] |
| `window` | `id`, `starts_at_unix_s`, `ends_at_unix_s` (integers ≥ 0) [`note`] |

Periods are entities too; they are declared in `model.json`.

**Rules:** `id`, `kind`, `subject` (entity id), `logic`, `provenance`, `status` [`when` (expr; absent means
always), `unknowns` (unknown ids), `supersedes` (rule ids), `note`]. `kind` selects the members of `logic`:

| `kind` | `logic` members |
|---|---|
| `cap` | `cap_kind` selects the rest. `hard` and `storage`: `on_reach` and exactly one of `value` (expr) and `reading` (fact key). `soft_read`: `reading` and `on_reach`. `on_reach` is `block_all_income`, `stop_production`, `discard`, `void_at_reset` or `{"block_income_from": [activity ids], "other_income": "notice_only"}`. |
| `regen` | `amount`, `every` [`while` (absent means always)], all expr |
| `flow` | [`consumes`, `produces` (resource id to expr; a product may instead be `random:<id>`, naming a `random` rule), `moves` (`from`, `to`, `amount`), `counts_on` (`success` or `attempt`), `batch` (`min`, `max`), `requires` (gate rule ids)] |
| `exchange` | `gives`, `takes` (resource id to expr) [`limit` (expr)] |
| `limit` | `max` (expr), `per` (period id) [`shared_with` (activity ids; absent means not shared)] |
| `cooldown` | `min_interval` (expr) |
| `gate` | `irreversible` (boolean), `origin` (`any` or `person`) [`requires` (expr; absent means no state condition), `prefs` (preference keys)] |
| `random` | exactly one of `pity_chain` (`rate`, `hard_pity`, `carry_over` (boolean) [`p_up`, `guarantee_after_miss` (boolean), `spark`]), `drop` (`rate`, or `samples` ≥ 1 with `successes` ≤ `samples`) and `outcomes` (a non-empty list of `{p, then}`, `then` being resource id to expr) |
| `derive` | `value` [`round` (`floor`, `ceil` or `half_even`; absent means exact), `general_logic` (expr), `live` (`fact`, `comparison`, `reference_fact`, `offset`)]. `comparison` is `eq`, `not_eq`, `less_than`, `less_than_or_equal`, `greater_than` or `greater_than_or_equal`, as in the Runtime's fact references. |
| `engine` | `engine` (engine id), `op` (`evaluate`, `predict` or `search`), `inputs` (id to expr), `metrics` (ids) |
| `objective` | `numeraire` (resource id), `terms` (list of expr), `horizon` (expr) |
| `weight_ref` | `file`, `pointer` (a non-empty JSON pointer), `compares_to` (output id) |

The members of `pity_chain`, `drop`, `outcomes` and `distribution` are this reader's reading of the design's
outline. The slices that compute them (random processes, unknowns) may revise them.

**Unknowns:** `id` (unknown id), `kind`, `question`, `status`, `provenance` [`note`, and the range form named by
`kind`]:
- `interval`: `{low, high}` (expr);
- `variants`: variant id to `{member id: expr}`;
- `distribution`: `family` selects the rest. `uniform`: `low`, `high`; `triangular`: `low`, `mode`, `high`;
  `beta_from_samples`: `samples` ≥ 1, `successes` ≥ 0; `empirical`: `values` (a non-empty list of expr).

The range form may be absent here. An unknown without one is a missing input, and a computation that needs
it fails.

## `data/<table>.json` (`acmodel.data.v1`)

| Member | Value |
|---|---|
| `table` | id |
| `key` | a column id, or a non-empty list of distinct column ids |
| `columns` | column id to `{type, …}`: `integer` and `decimal` add `dimension`; `enum` adds `values` (a non-empty list of ids); `boolean` and `id` add nothing |
| exactly one of `rows` and `external` | inline rows, or `{"kind": "external_dataset", "source", "sha256" (64 lowercase hex), "fetched_at"}` |
| [`provenance`] | table-level provenance, for rows without their own |
| [`note`] | |

- No column may be called `status` or `provenance` (`value_invalid`).
- **A row** holds exactly the key columns, the value columns and `status` [`provenance`]. A key column that
  `columns` does not list holds ids.
- **Cells:**
  - `integer`: a JSON integer, or a string `-?(0|[1-9][0-9]*)`;
  - `decimal`: a JSON integer, or a string `-?(0|[1-9][0-9]*)(\.[0-9]+)?`;
  - `boolean`: true or false;
  - `enum`: one of `values`;
  - `id`: an id.

## `sources.json` (`acmodel.sources.v1`)

`sources`: a list of `{id, title}` [`url`, `note`]. The ids mirror the guide's source list, so that one page
has one id.

## State snapshot (`acmodel.state.v1`)

| Member | Value |
|---|---|
| `model_id` | id |
| `label` | id, neutral (`acct-1`); no device identifier |
| `readings` | fact key to `{value, observed_at_unix_ms, source}`: an integer, an integer ≥ 0 and a string |
| `entities` | class id to entity id to attribute id to an integer, string or boolean |
| `cleared` | a list of ids |

## `ours/prefs/defaults.json` (preference layer, `schema_version` 1)

The preference layer's own file, read as it is:
- `defaults`: preference key (id) to any JSON value;
- `kinds`: preference key to `safety` or `strategy`;
- `neutral`: preference key to any JSON value;
- [`game`, `schema`, `note`]: strings, not read by the tool.

Every key in `defaults` has a kind and every kind has a default (`member_missing`). Exactly the `strategy`
keys have a neutral value: a missing one is `member_missing`, and one for any other key is `member_unknown`.

## The static check (`acmodel check`)

`acmodel check <model>` takes the model folder or its `model.json`. It reads `model.json`, `sources.json` in the
same folder, and every file that `model.json` lists. If a reader refuses a file, the check reads the remaining
files, reports every reader's errors and stops. Otherwise it checks the rules below.
- It computes nothing, and it reports every problem at once, with exit status 1.
- Each error holds the file in `detail.path` and the JSON pointer in `ref`. In an expression, `position` is the
  character offset of the reference.
- On success `data` holds `path`, `model_id` and these counts: `entities` (periods included), `rules` (the
  active ones), `superseded`, `unknowns` and `tables`.

### Duplicate ids

- **One namespace for the model:** the entities, periods, rules and unknowns of all files share one, so every
  id names one thing.
- **Separate namespaces:** tables, the row keys of each table, sources and engines each have their own.
- A second declaration is `model_id_duplicate`, with the `namespace` and the first declaration's place.
  References resolve to the first declaration.

### Status and provenance

| Entry | `status` | `provenance` |
|---|---|---|
| rule | `confirmed`, `pending` or `provisional` | a non-empty list |
| unknown | `pending` or `provisional` | a non-empty list |
| data row | `confirmed`, `pending` or `provisional` | its own non-empty list, or else the table's |
| data table | (none) | when present, a non-empty list |

- **Refusals:** another status is `rule_status_invalid`; an empty or absent provenance is
  `rule_provenance_missing`.
- **Sources:** every `source` that a provenance entry cites is in `sources.json` (`rule_reference_missing`, with
  `expected` `source`). So is the `source` of an external dataset.

### `supersedes`

- **A superseded rule** is one that any rule lists in `supersedes`. It stays in its file for history and is
  never read. The check reads only its `status`, `provenance` and `supersedes`.
- **References to it:** a reference to a superseded rule is `rule_reference_missing`, with `superseded_by`.
- **The target** of `supersedes` must be a rule.
- **Loops:** rules that supersede each other in a loop are `rule_cycle` with `via` `supersedes`.

### References

A reference that does not resolve to something of the expected kind is `rule_reference_missing`, with
`expected`, and `found` when the id names something of another kind. References to the data and to the account
state have their own codes.

| Where | Must name |
|---|---|
| a rule's `subject`; `cleared:<id>` | an entity of any kind (periods included) |
| a store's `holds`; the keys of `consumes`, `produces`, `gives`, `takes` and an outcome's `then`; `numeraire` (model and objective); `amount:<id>`; the `amount:<id>` dimension of a column or an attribute | a `resource` |
| `stock:<id>` | a `store` |
| `moves.from`, `moves.to` | a `resource` or a `store` |
| `block_income_from`, `shared_with` | `activity` entities |
| `limit.per` | a period |
| `rule:<id>`; `weight_ref.compares_to` | an active rule |
| `cap:<id>`; `count:<id>`; flow `requires`; a `random:<id>` product | an active `cap`, `limit`, `gate` or `random` rule |
| `metric:<rule>.<name>` | an active `engine` rule and one of its `metrics`; the longest rule id that leaves a metric name is taken |
| `cost(<a>, <r>)`, `yield(<a>, <r>)`; `max_single_income(<r>, <cap>)` | an `activity` and a `resource`; a `resource` and an active `cap` rule |
| `unknown:<id>`; `unknown:<id>.<member>` | an `interval` or `distribution` unknown; a member that every variant of a `variants` unknown has. The longest unknown id that the target begins with is taken. |
| an engine rule's `engine` | an engine of `model.json` |
| a rule's `unknowns` | unknowns |
| `param:<id>` | nothing yet: the format declares no parameters, so a `param:` reference is always refused |

**Data references** (`data_reference_missing`, with `missing` `table`, `column` or `row`):
- `data:<table>[<key>,…].<column>`, a set `data:<table>`, and `lookup(<table>, <column>, …)` name a table of
  the model and a column it declares.
- The key has as many parts as the table's key, and for inline rows it matches a row. The key `$` in a `sum` or
  `count` matches every row.

**State references** (`state_reference_missing`, with `missing` `reading`, `entity_class` or `attribute`):
- **Fact keys.** A fact key is declared as the `reading` of an entity or of an active `cap` rule. `read:<fact>`,
  `live.fact` and `live.reference_fact` name a declared fact key. A mistyped key is found here, before a run
  looks for it in a state snapshot.
- **Entities.** In `entity:<class>[<id>].<attr>`, `<class>` is an `entity_class` entity, and `<attr>` one of its
  `attributes`.

**Not checked here:**
- preference keys (`pref:<key>`, a gate's `prefs`), which the run reads with the preferences;
- the entity ids of the state (`entity:<class>[<id>]`);
- the dimensions of expressions (`expr_dimension_mismatch`). They need the types of `read:`, `pref:` and
  `metric:` values, which the evaluation gives.

### Pending rules and unknowns without a range

- **A pending rule** is usable only through an unknown with a range. So it lists in `unknowns` at least one
  unknown that has its range form (`interval`, `variants` or `distribution`).
  - If it lists none, that is `rule_pending_unbounded` with `reason` `no_unknown`.
  - If it lists only unknowns without one, that is `reason` `unknown_unbounded`.
  - Both are reported at the rule's `status`.
- **An expression** that references an unknown without its range form is `rule_pending_unbounded`
  (`unknown_unbounded`), at the expression.

### Where `pref:` may appear

A preference is read only in the expressions of a `gate` rule and in the `terms` of an `objective` rule (#424
design D-2). Anywhere else, unknowns included, `pref:` is `rule_pref_misplaced`.

### Conflicts

- **The rule.** Two active rules of the same kind hold the same subject and slot. That is `rule_conflict`,
  reported at the later rule.
- **`supersedes`.** When one rule lists the other in `supersedes`, the superseded one is not active, so they do
  not conflict.
- **`when`.** A `when` does not separate two rules. A value that changes with a condition is one rule with
  `if(…)`.

| Kind | Slot |
|---|---|
| `cap` | `cap_kind` |
| `limit` | `per` |
| `regen`, `flow`, `exchange`, `cooldown`, `random`, `objective` | the subject alone |
| `gate`, `derive`, `engine`, `weight_ref` | none: their rules never conflict |

### Loops

- **Nodes:** every active rule, every `interval` or `distribution` unknown, and every member of a `variants`
  unknown.
- **Links:**
  - A node links to the rules and unknowns that its expressions name.
  - `cost(<a>, <r>)` and `yield(<a>, <r>)` link to the flow rules of activity `<a>`.
  - `max_single_income(<r>, <cap>)` links to the cap and to the flow rules that produce `<r>` or move into it.
  - A flow's `requires` links to its gates, and a `random:<id>` product to its random rule.
  - A derive's `general_logic` is only compared with its value, and nothing reads it, so it links nothing.
- **Refusal:** a loop is `rule_cycle` with `via` `reference`. `members` are its nodes, sorted (`rule:<id>`,
  `unknown:<id>` or `unknown:<id>.<member>`). The error points at the first member's first link into the loop.

### External datasets

- **The local copy** of an external dataset is the file named by its `sha256` in the folder `paths.datasets`.
  That folder is set in the configuration (`contracts/cli.md`).
- **Refusals:** a copy that is absent or cannot be read is `data_external_missing`, at `/external`. A copy with
  another hash is `data_external_hash_mismatch`, at `/external/sha256`.
- The check never fetches anything.
