# acmodel input files and how they are read

acmodel reads eight kinds of input file. Each has one strict reader in `src/acmodel/model/`:

| File | `schema_version` | Reader |
|---|---|---|
| `ours/model/model.json` | `acmodel.model.v1` | `read_model` |
| `ours/model/rules/<topic>.json` | `acmodel.rules.v1` | `read_rules` |
| `ours/model/data/<table>.json` | `acmodel.data.v1` | `read_data` |
| `ours/model/sources.json` | `acmodel.sources.v1` | `read_sources` |
| `ours/model/exports.json` | `acmodel.exports.v1` | `read_exports` |
| `ours/model/bindings.json` | `acmodel.bindings.v1` | `read_bindings` |
| an account state snapshot (local only) | `acmodel.state.v1` | `read_state` |
| `ours/prefs/defaults.json` (the preference layer, read as it is) | `1` | `read_prefs_defaults` |

The engine host also reads each engine's `engine.json` (`acmodel.engine.v2`), in the engine's home outside
any repository, with the same rules (see "Engines" at the end).

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

Absent members of `pity_chain` mean: `p_up` 1 (every top-tier pull is the target), `guarantee_after_miss` false
and no `spark`. The run computes a `pity_chain` and a `drop` (see "Random rules" under "The run"); `outcomes`
and `random:<id>` products are read and type-checked, and not evaluated yet. The run computes `distribution` as
written here (see "Unknowns" under "The run").

**Unknowns:** `id` (unknown id), `kind`, `question`, `status`, `provenance` [`note`, and the range form named by
`kind`]:
- `interval`: `{low, high}` (expr);
- `variants`: variant id to `{member id: expr}`;
- `distribution`: `family` selects the rest. `uniform`: `low`, `high`; `triangular`: `low`, `mode`, `high`;
  `beta_from_samples`: `samples` ≥ 1, `successes` from 0 to `samples`; `empirical`: `values` (a non-empty list
  of expr).

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

## `exports.json` (`acmodel.exports.v1`)

| Member | Value |
|---|---|
| `thresholds` | a list of `{id, from, form}` [`round`, `note`]: `id` is an id, unique among the thresholds; `from` is the id of a `derive` rule; `form` is `constant` (the rule's value) or `fact_offset` (the offset of the rule's `live`); `round` is `floor`, `ceil` or `half_even` (absent means exact) |
| `tables`, `scoring` | lists. Their entries are defined with the table and scoring exports; until then an entry is refused (`value_invalid`). |
| [`note`] | |

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

**Not checked here:** three checks belong to the run (see "The run" below):
- preference keys (`pref:<key>`, a gate's `prefs`), which the run reads with the preferences;
- the entity ids of the state (`entity:<class>[<id>]`);
- the dimensions of expressions (`expr_dimension_mismatch`). They need the types of `read:`, `pref:` and
  `metric:` values, which the run gives.

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
  - `max_single_income(<r>, <cap>)` links to the cap, to the flow rules that produce `<r>` or move into it, and
    to the `storage` cap of each store that such a flow moves out of.
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

## The run (`acmodel run`)

`acmodel run <model> --state <file>` computes the model's outputs for one account state. It reads:
- the model, and runs the static check. A model that does not check is refused with its errors, and nothing
  is computed;
- `exports.json` beside `model.json`, and the state snapshot, whose `model_id` must be the model's
  (`model_file_invalid`, `value_invalid`, at `/model_id`);
- the preference defaults `ours/prefs/defaults.json` of the same repository (`<model folder>/../prefs/`),
  only when the model reads a preference.

It then runs the model checks below, which also refuse the whole run, and evaluates the derivation graph.

### Types

Every number has a dimension (`amount:<resource>`, `time`, `count`, `value`, `none`), and a number literal is
`number`, which fits any dimension but `time`. The dimension rules of the operators and functions are those
of the expression language (`src/acmodel/expr/dimensions.py`). The references have these types:

| Reference | Type |
|---|---|
| `amount:<r>` | `amount:<r>` |
| `stock:<s>` | the amount of the resource the store holds |
| `cap:<c>`, `rule:<cap>` | the amount of what the cap's subject holds: the resource itself, or a store's resource |
| `rule:<derive>` | the type of its `value` |
| `rule:<random>` | `count` for a pity chain (its expected pulls), `none` for a drop (its chance) |
| `rule:<limit>`, `count:<limit>` | `count` |
| `read:<fact>` | the dimension of what declares the fact key: an entity's reading is the amount it holds, a cap's reading the cap's |
| `unknown:<id>.<member>` | the one type of the member in every variant |
| `unknown:<id>` | the one type of its bounds (`interval`, `uniform`, `triangular`, `empirical`); `none` for `beta_from_samples` |
| `data:<table>[…].<column>`, `entity:<class>[…].<attr>`, `lookup(…)` | an `integer` or `decimal` column has its `dimension`; a `boolean` column is a boolean |
| `pref:<key>` | from its value in the preference defaults: an integer is `number`, a boolean a boolean |
| `metric:<rule>.<name>` | `value` |
| `cleared:<id>` | a boolean |

### Model checks of the run

Each problem is one error, with the file in `detail.path` and the JSON pointer in `ref`; all are reported
together, with exit status 1.

**Dimensions** (`expr_dimension_mismatch`):
- **Inside each expression** of an active rule and of an unknown, by the rules of the expression language.
- **A reference without a type**: a preference whose value is a string, array, object or null, and a column
  or attribute of type `enum` or `id` (`operator` `reference`). So is a fact key declared with two
  dimensions (`operator` `read`).
- **Each member that expects a dimension**:

  | Member | Expects |
  |---|---|
  | a rule's `when`; regen `while`; gate `requires` | a boolean |
  | cap `value` | the cap's dimension |
  | limit `max`; flow `batch.min`, `batch.max`; exchange `limit` | `count` |
  | regen `amount` | the amount of what its subject holds (a resource or a store) |
  | regen `every`, cooldown `min_interval`, objective `horizon` | `time` |
  | flow `consumes`, `produces`; exchange `gives`, `takes` | the amount of the resource named by the key |
  | flow `moves.amount` | the amount that `from` holds, and the amount that `to` holds |
  | derive `general_logic` | the dimension of its `value` |
  | derive `live.offset` | the dimension of `live.reference_fact`, which is that of `live.fact` (`operator` `live`) |
  | random `pity_chain.rate`, `pity_chain.p_up`, `drop.rate`, an outcome's `p` | `none` (a chance) |
  | random `pity_chain.hard_pity`, `pity_chain.spark` | `count` |
  | random: an outcome's `then` | the amount of the resource named by the key |
  | derive `value` with `round` | a number that is not `time` (`operator` `round`) |
  | the members of the variants of an unknown, the bounds of an interval or a distribution | one dimension (`operator` `variants`, `interval` or `distribution`) |
  | each key of `lookup(<table>, …)` | its key column's type; a key column of type `id` or `enum` (or none declared) cannot be looked up |

  `operator` names the member, and `operands` are the expected and the found type. The engine members are not
  checked here; their slice checks them.

**References** (`rule_reference_missing`):
- **Preference keys.** Every `pref:<key>` and every key of a gate's `prefs` is in the preference defaults'
  `defaults` (`expected` `pref`).
- **Rule values.** `rule:<id>` names a rule that has a value: a `derive`, `cap` or `limit` rule, or a `random`
  rule with a `pity_chain` or a `drop` (`expected` lists `rule.random.pity_chain` and `rule.random.drop`; one
  with `outcomes` is `found` `rule.random.outcomes`).
- **Caps.** A cap's subject is a resource or a store (`expected` `resource`, `store`), at its `subject`.
- **Flows.** `cost(<a>, <r>)` needs an active flow of `<a>` that consumes `<r>` or moves out of it;
  `yield(<a>, <r>)` one that produces `<r>` or moves into it; `max_single_income(<r>, <cap>)` a flow the cap
  blocks that brings `<r>`. Otherwise `expected` is `rule.flow`, and `target` the activity or the cap. An
  empty sum would be a number without a rule behind it.
- **Thresholds.** The `from` of a threshold names an active derive (`expected` `rule.derive`); in the
  `fact_offset` form it must have `live` (`expected` `rule.derive.live`). A threshold id given twice is
  `model_id_duplicate` (`namespace` `threshold`). A threshold of a boolean is `expr_dimension_mismatch`
  (`operator` `threshold`).

### Evaluation

**Nodes.** The outputs are:
- `rule:<id>`: the value of every active `derive` (its `value`, then its `round`), `cap` (its `value`, or its
  reading), `limit` (its `max`) and `random` rule with a `pity_chain` or a `drop` (see "Random rules");
- `general_logic:<id>`: a derive's `general_logic`, computed beside its value and read by nothing;
- `threshold:<id>`: each threshold of `exports.json`. The `constant` form is the derive's value, the
  `fact_offset` form the value of its `live.offset`; then the threshold's `round`. The `fact_offset` form
  needs no reading.

Inside the graph, the amounts of flows and the members of unknowns are nodes too. Each node is computed after
what it reads, without recursion.

**Values** are exact. References read:
- `read:<fact>` and a cap's reading: the state's reading; `amount:<r>` and `stock:<s>`: the reading of the
  entity. A reading the state lacks is `state_reading_missing`, and so is an entity that declares no
  reading. There is no fallback to a constant.
- `entity:<class>[<id>].<attr>` and the members of `entity:<class>`: the state's entities. A class, entity or
  attribute the state lacks is `state_entity_missing`. A class in the state with no entities is an empty set.
- `cleared:<id>`: whether the state's `cleared` lists it.
- `data:` cells and `lookup`: the rows of the table; a cell of dimension `time` counts seconds.
  `lookup` finds the row whose key cells equal its keys (`data_reference_missing`, `row`, when none does).
- `cost(<a>, <r>)`: the sum of what the active flows of `<a>` consume of `<r>` and move out of it;
  `yield(<a>, <r>)`: the sum of what they produce of `<r>` and move into it.
- `max_single_income(<r>, <cap>)`: the largest single income that the cap blocks, over the flows it blocks
  and over the variants the node leaves open. It is general logic: computed from the rules and the data
  only, never from the account state, so no reading changes it. An income counts with its amount, and a move
  out of a store with at most the store's `storage` cap (one move takes at most a full store); an income that
  would need a state value and has no such cap blocks the output (`expr_eval_failed`,
  `evaluation_unsupported`, `found` `state`).

**Not evaluated yet** (`expr_eval_failed`, `evaluation_unsupported`; `found` names it): `metric:`, `count:` and
`pref:` references, `random:<id>` products, the rows of an external table, and a value rule's `when`. Every
output that needs one is blocked. The other rule kinds (regen, exchange, cooldown, gate, engine, objective,
weight_ref), and random rules with `outcomes`, are type-checked but not evaluated.

**Unknowns.** A node that reads a discrete unknown is computed once per point of it, and once per point of each
further one that it reads under that choice. A continuous unknown is one enclosure, and what reads it is computed
by interval arithmetic (see "Unknowns" below). An output that read a discrete unknown lists `variants`
(`[{"unknowns": {<id>: <point>}, "value": …}]`), and a number that read any unknown its `range`.

**Status** of an output: the worst of what it read.

| Input | Counts as |
|---|---|
| a `confirmed` rule or data row; a reading | `confirmed` |
| a `provisional` rule or data row | `provisional` |
| an unknown it read; an unknown that a `pending` rule it read names; an unknown whose points a maximum ran over (`max_single_income`) | `provisional`, unless the output read that unknown and its sound range has one value at export precision (the no-effect rule, below) |
| a `pending` data row (a row cannot name an unknown) | blocked: `rule_pending_unbounded` (`no_unknown`, `id` `<table>[<key>]`) |

**Blocked outputs and partial runs.** An output that cannot be computed is `blocked` with its errors, and so
is every output that reads it. Outputs that do not read it are still computed. Each error is reported once,
at its own expression, with `needed_by`, the outputs it blocks; the run then exits with status 1.

**Notes** (`data.notes`; never errors): `ruling_overrides_general_logic`, for a derive whose value differs from
its `general_logic`. The value stands by its provenance, and the difference is reported.

### Unknowns

The keys are under `unknowns`, and `sensitivity.top_n`, in the configuration (`contracts/cli.md`); the code is
`src/acmodel/core/unknown.py` and `src/acmodel/expr/interval.py`.

**Discrete unknowns** are evaluated at their points, each a scenario: a `variants` unknown at each variant (by its
id), an `empirical` distribution at each of its values (by position: `0`, `1`, …).

**Continuous unknowns** (`interval`, and the `uniform`, `triangular` and `beta_from_samples` distributions) are one
enclosure each: from the lowest value of the low bound to the highest value of the high bound. Every bound is
evaluated, and bounds that may be out of order (low above high, a triangular mode outside them) are
`expr_eval_failed` (`range_invalid`) at the range form; they block what needs the unknown. The bounds of a
`beta_from_samples` rate are its Wilson bounds (see "Random rules"): it reads no expression.

**Ranges are sound enclosures.** Everything a continuous unknown reaches is computed by interval arithmetic with
exact rational bounds, so the `range` of an output is never narrower than its true range.
- **Exact where proven.** The structural monotonic check gives each continuous unknown a direction through every
  operator: `+`, `-` (its right side turned around), unary `-`, `min`, `max`, `clamp`, `floor`, `ceil`,
  `round_half_even`, `sum` and the flow sums are monotone in each operand; `*` and `/` are monotone in one operand
  when the other has one sign over its enclosure. When every unknown reaches the value only through such operators
  and in one and the same direction, the enclosure is attained, and `range` says `exact: true`.
- **Wider elsewhere.** Anything else is `exact: false`, and the enclosure may be wider than the true range:
  - an unknown that reaches the value in both directions (`unknown:<id> - unknown:<id>` gives [−w, w], not 0);
  - a product or quotient whose other operand may be of either sign (`x * x` across 0);
  - a condition the unknowns leave undecided: a comparison that holds for some of their values only. `if` then
    gives the enclosure of both branches, `and`, `or` and `not` stay undecided, `count` counts it as 0 or 1, and
    a boolean output that stays undecided shows `null`.
- **Refused, never guessed** (`expr_eval_failed`): a division by an enclosure that holds 0 (`division_by_zero`);
  `clamp` bounds that may cross (`clamp_bounds_invalid`); a `lookup` key that the unknowns leave open
  (`evaluation_unsupported`, `found` `interval`).

**The no-effect rule** decides on the sound range, by computation and never by a flag. An output that read
unknowns is provisional when its range holds more than one value at export precision: the value times
`unknowns.scale` (1000, milli), rounded half to even; a boolean counts as 0 or 1, an undecided one as both. It is
also provisional when it depends on an unknown it did not read (unresolved): one that a pending rule it read names
(the pending rule is uncertain only through it), or one whose points a maximum ran over.

**Sensitivity.** For each unknown an output read, its impact is the largest change of the output at export
precision while that unknown moves and the others are held at one of their points (each point of a discrete one,
the low and the high of a continuous one).
- **A discrete unknown:** two scenarios that chose different points of it, and the same point of every other
  discrete unknown both chose, are neighbours in it. The change between them is the largest difference of their
  values (of their enclosures, so the continuous unknowns count with their whole range).
- **A continuous unknown:** the width of the output's enclosure with that unknown free. When the output computes
  with other continuous unknowns, each of them is held at each of its ends in turn (the `impacts` node).
- **Listing.** An output that read an unknown lists in `sensitivity` its `sensitivity.top_n` (5) largest impacts,
  as `{"unknown", "impact"}` sorted by impact and then by id. An impact is an exact rational in the output's unit
  (time in seconds). It ranks the unknowns; the status does not rest on it but on the sound range.

**Sampling** (off by default: `unknowns.samples` is 0).
- **What is drawn.** With `unknowns.samples` = N, every output that read a distribution is computed N more times,
  with every distribution at one sample. The variants stay at their points, and the intervals and the
  `beta_from_samples` rates stay free, so a sampled value can be an enclosure.
- **Streams.** Each distribution has its own stream of `numpy.random.Philox`, keyed by `unknowns.seed` and the
  unknown's stream id (the first 8 bytes of the sha256 of `acmodel.stream.v1:<unknown id>`).
- **Chunks.** Sample s is the block the stream gives from counter s; the chunk k of `unknowns.chunk_size` samples
  starts at counter k × chunk size. So the chunk size never changes a sample.
- **Values.** A draw on (0, 1) is the middle of one of `unknowns.resolution` equal cells.
  - A `uniform` value is low + (high − low) × draw.
  - A `triangular` value comes from its inverse distribution function, with the square root rounded down to a
    multiple of 1 / resolution.
  - An `empirical` value is the one at position floor(word × n / 2<sup>64</sup>).

  All of it is integer and exact, the same on every platform and under every string-hash seed. A distribution
  whose bounds other unknowns leave open is not sampled (`evaluation_unsupported`, `found` `interval`).
- **Tallies.** At export precision, per choice of the output's discrete unknowns: the count, the totals of the low
  and the high ends, the lowest low and the highest high, each held to the int64 range. Beyond it is
  `expr_overflow` (`operator` `tally`, `max_bits` 63). A sample that cannot be computed blocks the output with its
  error.

### Random rules

The code is `src/acmodel/core/random.py`; the keys are `unknowns.drop_z`, `unknowns.wilson_digits` and
`random.max_transitions` (`contracts/cli.md`). Everything is exact, with no float, so a value is the same on
every platform.

**A pity chain** is the pulls toward one target:
- each pull is top-tier with chance `rate`, and surely when it is the `hard_pity`-th pull since the last top-tier
  one;
- a top-tier pull is the target with chance `p_up`, and surely when the last top-tier pull missed the target and
  `guarantee_after_miss` holds;
- with `spark`, the target is taken by exchange at pull `spark` of the chain when it has not come before;
- the pity count (the pulls since the last top-tier pull) and the guarantee that a chain leaves for the next
  chain, or for the next period when the target has not come, carry when `carry_over` holds; otherwise the next
  one starts fresh. A target from a top-tier pull leaves a fresh state, one taken by exchange the state of that
  pull. Spark points never carry.

Its value (`rule:<id>`, type `count`) is the **expected pulls to the target from a fresh start**, an exact
rational. The chances are integers over a common denominator: with `rate` = a/b and `p_up` = u/v, one pull
multiplies every mass by an integer over b·v. Without a spark the expectation has a closed form over the cycles
between top-tier pulls; with one, the exact DP runs to the spark. A large chain has large numerators and
denominators: a derive that reads it is held to `expr.max_bits` like any value it reads.
- Its members are single values (`evaluation_unsupported`, `found` `interval`, when continuous unknowns leave
  one open) in their domains: `rate` from 0 to 1, `p_up` above 0 and at most 1, `hard_pity` and `spark` whole
  numbers of at least 1 (`value_invalid`, `operator` the member, `expected` `probability`,
  `positive_probability` or `positive_integer`, `found` the value).
- Its work, pulls × pity counts × guarantee states (the closed form counting as one pull, a spark as `spark`
  pulls; two guarantee states when a miss can be followed by the guarantee), is at most `random.max_transitions`
  (`budget_exceeded`, `expected` the limit, `found` the work, at the `pity_chain`).

**A drop** has the value (`rule:<id>`, type `none`) of its chance per run: its `rate`, which must lie in [0, 1]
for every value its unknowns allow (`value_invalid`, `expected` `probability`), or the **Wilson bounds** of its
`samples` and `successes`. A drop from samples counts as a continuous unknown of its own, named by the rule's
id: it shows its `range` and is listed in `sensitivity` under that id.

**Wilson bounds** of n samples with k successes at z = `unknowns.drop_z` (a decimal string, 1.959964 for 95 %):
the centre (k + z²/2) / (n + z²) less and plus the half width z·√(k(n − k)/n + z²/4) / (n + z²). The square
root is taken with `math.isqrt` on the bounds scaled by 10<sup>d</sup>, d = `unknowns.wilson_digits` (6): the
low bound is rounded down and the high bound up to a multiple of 10<sup>−d</sup>, both exactly, so the bounds
hold the true interval and are rationals, the same on every platform.

### Output

`data` holds `path`, `model_id`, `state` (`path`, `label`), `run_id` and `work` (the run id and the work
counters of the cache, `cli.md`, "Cache"), and:
- `rules`: each value rule's output by id: `kind`, `status` (`confirmed`, `provisional` or `blocked`), `type`
  and, unless blocked, `per_account` (whether it read the account state) with `value`, or `variants` and
  `range`. A value that continuous unknowns leave open shows `range` instead of `value`, in a variant too:
  `{"low", "high", "exact"}`. An output that read an unknown adds `sensitivity`, and a sampled one `sampled`: a
  list, one entry per choice of its discrete unknowns, with `unknowns`, `samples`, `mean` (`low`, `high`: the
  means of the ends), and `low` and `high`, at export precision. A derive with `general_logic` holds it as
  `general_logic`, in the same shape.
- `thresholds`: by id, `form` and the same members. The number of the `fact_offset` form is `offset`, beside
  `fact`, `comparison` and `reference_fact`.
- `notes`.

A number is a string holding an exact rational (`"-250"`, `"7/2"`; time in seconds); a boolean is a JSON
boolean, or null when continuous unknowns leave it undecided.

The errors are ordered by file, pointer, code and then their whole canonical text, so that the order never
depends on which nodes the cache reused. The outputs, without `work`, are kept as
`runs/<run id>/outputs.json` under `paths.state_root`.

## Outputs: provenance, `explain` and `thresholds.json`

### Provenance (`acmodel.provenance.v1`)

The provenance document of one output holds what its value rests on (#424 design 6.4). It is assembled from
what each node of the graph read while it was evaluated. So it names exactly what the value used: one branch
of an `if`, the row a `lookup` found, every row of a set.

| Member | Value |
|---|---|
| `schema_version` | `acmodel.provenance.v1` |
| `model_id` | the model's id |
| `output` | `rule:<id>`, `general_logic:<id>` or `threshold:<id>` |
| `result` | the output as `run` shows it (a rule without its `general_logic`, which is an output of its own) |
| `versions` | `rules`: the sha256 over `model.json`, `sources.json` and the rule files; `data`: over the data files; `tool`: the tool version. Each file counts as `[path, document]`, sorted by path. A hash is over canonical JSON: sorted keys, no whitespace, UTF-8. |
| `nodes` | node id to node; the tree is rooted at `output` |

**Nodes.**
- Every node has `node` (what it is) and `reads`, the sorted ids of the nodes its value read. A leaf reads
  nothing, and a node read twice is listed once.
- Paths are relative to the model folder, with `/`.
- A node of the general logic (`max_single_income`, from the rules and the data only) has `general:` before
  its id and the member `general: true`.

| `node` | Ids | Members |
|---|---|---|
| `threshold` | `threshold:<id>` | `id`, `form`, [`round`], `file` (`exports.json`), `ref`, `sha256` of its entry |
| `rule` | `rule:<id>` (its value), `general_logic:<id>`, `offset:<id>` (`live.offset`); the flow amounts `flow:<id>:consumes:<resource>`, `flow:<id>:produces:<resource>` and `flow:<id>:moves`; `entry:<id>`, a rule consulted for its logic only (the cap of `max_single_income`, and an income whose amount needs the account state, which its store's storage cap bounds) | `kind`, `id`, `file`, `ref` (the pointer of the expression, or of a cap's `reading`; for `entry:` the rule's), `sha256` of the rule entry, and its `status` and `provenance` as written |
| `unknown` | `member:<unknown>:<member>`; `point:<unknown>` (an interval or a distribution) | as a rule, with `member` for a member; `ref` is the unknown's |
| `row` | `row:<table>[<key parts joined with commas>]` | `table`, `key`, `file`, `ref`, `sha256` of the row, `status`, `provenance` (the row's, else the table's) |
| `reading` | `reading:<fact>` | `fact`, `value`, `observed_at_unix_ms`, `source` |
| `cleared` | `cleared:<id>` | `id`, `value` |
| `entity` | `entity:<class>[<id>].<attribute>` | `entity_class`, `entity`, `attribute`, `value` |
| `entities` | `entities:<class>` | `entity_class`, `entities` |

The last four come from the account state, so an output whose tree holds one is per-account.

### `acmodel explain <model> <output> --state <file>`

It runs the model as `run` does and prints the provenance document of `<output>` in `data.provenance`, beside
`path`, `model_id`, `state` and `output`. It writes nothing.
- An id that is not an output of the model is `usage_invalid` (`output_unknown`).
- A blocked output has no tree. The errors that block it are reported, with exit status 1.

### `acmodel export thresholds <model> --state <file>`

It runs the model, then writes into the model folder:
1. `out/provenance/thresholds/<id>.json`: the provenance document of each threshold, as `explain` prints it;
2. `out/thresholds.json` (`acmodel.thresholds.v1`), last.

| Member of `thresholds.json` | Value |
|---|---|
| `schema_version` | `acmodel.thresholds.v1` |
| `model_id`, `versions` | as in a provenance document |
| `thresholds` | a list sorted by `id`. Each entry has `id`, `form`, `dimension` (the type of its number), `status` and `provenance` (the path of its provenance file). A `constant` adds `value`; a `fact_offset` adds `fact`, `comparison`, `reference_fact` and `offset`. `value` and `offset` are JSON integers. |

**Only account-independent values are exported** (#424, ruling 66). These files are delivered with the
resource repository. So a threshold is refused (`export_refused`, exit 5) when it is:
- `blocked`. The errors that block it are reported too.
- `per_account`: it read the account state. Such a value is never written into a shared output; `run` and
  `explain` show it locally.
- `provisional`: not confirmed. Accepting a provisional value comes with the export policy.
- `not_integer` or `out_of_range`: its exact value is not an integer (`exports.json` can declare its `round`),
  or lies beyond ±(2<sup>53</sup> − 1). A confirmed threshold that read unknowns has a value or an
  enclosure per scenario, all within one value at export precision: one integer throughout is exported, and
  anything else is refused as `not_integer`, with a value that is not an integer, or an enclosure as
  `<low>..<high>`.

**Writing.**
- When any threshold is refused, nothing is written.
- A write that fails is `export_write_failed`.
- The files have sorted keys, a two-space indent, UTF-8 and LF line ends.
- Nothing is deleted, not even the provenance file of a threshold that `exports.json` no longer lists.

## Bindings: `bindings.json`, `bindings check` and `bindings apply`

A binding ties a literal of the resource repository to a threshold of `exports.json` (#424 design 6.3), so that
a number in `schedule.json` or in a pack file has the threshold's provenance. `bindings.json` sits beside
`model.json`. Its file names are relative to the repository, the folder two above the model folder
(`<repo>/ours/model`).

### `bindings.json` (`acmodel.bindings.v1`)

| Member | Value |
|---|---|
| `bindings` | a list of `{threshold, file, path}` [`rule_id`, `note`]: `threshold` is the id of a threshold of `exports.json`, `file` the file that holds the literal and `path` its JSON pointer |
| `fixed` | a list of `{file, path, reason, ref}` [`rule_id`, `note`]: a literal that is no quantity, with the reason and a reference (a ruling, a board item) |
| [`note`] | |

- **`file`** is relative to the repository: `/`-separated segments of `[A-Za-z0-9_-][A-Za-z0-9_.-]*`, ending
  in `.json`.
- **`rule_id`** is there exactly when `file` is `schedule.json` (the game root's schedule). It is the `id` of a
  rule, and `path` is then relative to that rule. For any other file, `path` points into the whole file.
- **One literal**, by its file, rule and pointer, is named once among the bindings and the fixed entries
  (`model_id_duplicate`, `namespace` `literal`).
- **A threshold** that `exports.json` does not list is `rule_reference_missing` (`expected` `threshold`).

### The values

`bindings check` and `bindings apply` run the model for the account state `--state`, as `run` does. They take
each bound threshold's value exactly as `export thresholds` writes it, so a binding holds only an exported
value:
- **What the export refuses** is refused the same way (`export_refused`, exit 5): a threshold that is blocked
  (with the errors that block it), provisional, a constant that read the account state (`per_account`), not an
  integer, or out of range.
- **A `fact_offset` threshold whose live offset read the account state** is `binding_unsupported` (exit 5),
  with the state nodes it read. A fact reference holds one fact plus a constant (#386 CF-4), and the tool never
  writes an account's value as a constant. The scheduler side chooses: a pack publishes the derived fact, or a
  selection document computes it.

### Reading the files

- Every file an entry names is read once, as strict JSON (see "Reading JSON"). A refusal is
  `model_file_invalid`, with `format` `json` for a pack file and `actingcommand.schedule.v1` for
  `schedule.json`.
- The tool needs only pointers and literals. Whether `schedule.json` is valid as a whole is the validator's
  job (`actinglab`, resource CI).
- Of `schedule.json` it needs `schema_version` `actingcommand.schedule.v1`, and `rules` (absent means none)
  that are objects with distinct string `id`s.

### Where a literal may sit

| Threshold | Its literal |
|---|---|
| `constant` | any JSON integer, except the offset of a fact reference |
| `fact_offset` | the `offset` of a fact reference `{"type": "fact", "fact_key": B, "offset": N}` that is the `value` of a `fact` predicate `{"kind": "fact", "fact_key": A, "comparison": …, "value": …}`. A must be the threshold's `fact`, and B its `reference_fact`. |

The comparison is not compared: a rule may test the other side of a threshold (`less_than` where the threshold
says `greater_than_or_equal`).

### `acmodel bindings check <model> --state <file> [--strict]`

- **Each bound literal** equals its threshold's value (`constant`) or offset (`fact_offset`). Otherwise it is
  `binding_mismatch` (exit 6), with `reason`:
  - `value`, with `expected` and `found`, at the literal;
  - `fact_key` or `reference_fact`, with `expected` and `found`, at the fact key;
  - `form`, at a literal that cannot hold the threshold's form;
  - `literal_missing`, when the rule or the pointer names no integer. Fixed entries are checked for this too.
- **`--strict`** also reads `schedule.json`. It refuses every integer literal that a `fact` predicate of one of
  its rules compares with, when the literal is neither bound nor fixed (`binding_unbound_literal`, exit 6).
  These literals are a typed value (`value/value`) or a fact reference's offset (`value/offset`). So no number
  without provenance reaches the schedule.
  - Other literals are not scanned: `max_age_ms`, counts and budgets, pools, and the pack files. They are bound
    when a threshold covers them.
- **It writes nothing.** Every problem is reported at once, and the exit status is the first error's.
- **`data`** holds `path`, `model_id`, `state`, `repo` and `strict`, and:
  - `bindings`: each with `entry` (its pointer in `bindings.json`), `threshold`, `form`, `file`, [`rule_id`],
    `path` and `literal`;
  - `fixed`: each with `entry`, `file`, [`rule_id`], `path`, `literal`, `reason` and `ref`;
  - with `--strict`, `literals`: how many literals the fact predicates of `schedule.json` compare with.

### `acmodel bindings apply <model> --state <file>`

- **It writes each bound literal that differs from its threshold.** Only the literal's bytes change. A
  position-tracking scanner finds them, so line ends (CRLF stays CRLF), spacing and every other byte stay as
  they are.
- **It refuses everything `check` refuses but a differing value**, and then writes nothing. It does not scan
  for unbound literals; `check --strict` does that afterwards.
- **A file that cannot be written** is `export_write_failed`. The files written before it stay written, and
  `apply` can run again.
- **`data`** holds `path`, `model_id`, `state`, `repo`, and:
  - `changed`: each with `entry`, `threshold`, `file`, [`rule_id`], `path`, `from` and `to`;
  - `written`: the files whose bytes changed.

## Engines: `engine.json` and the interface `acmodel.engine.v2`

An engine is Python code in an engine's home outside this repository: an adapter, and the calculator code
and data it uses (#424 design-py section 7). `engines.roots` of the local configuration says where each home
is. The engine host (`src/acmodel/engine/`, `cli.md` "Engine host") runs engines only in its own worker
processes.

### `engine.json` (`acmodel.engine.v2`)

It sits in the home and is read strictly, as the files above are (`detail.format` is `acmodel.engine.v2`).

| Member | Value |
|---|---|
| `schema_version` | `acmodel.engine.v2` |
| `engine_id` | an id: the one `model.json` and `engines.roots` name |
| `version` | digits separated by dots |
| `python` | the Python minor version it runs on; another than the running one (`3.13`) is refused |
| `entry` | `module:callable`, the function the worker calls: `make_engine(config, scratch_dir)` |
| `sys_path` | folders of the home that the worker puts first on `sys.path` (`.` is the home); each must exist |
| `code` | glob patterns (`*`, `?`, `**`) of the engine's Python files; at least one |
| `data` | glob patterns of the other files the engine reads |
| `external` | `{source, sha256}` datasets, verified as a data file's are (`data_external_missing`, `data_external_hash_mismatch`); their local paths reach `make_engine` |
| `ops` | from `describe`, `evaluate`, `predict`, `search`: `describe` among them, none twice |
| `deterministic` | `true` |
| `max_batch` | the most requests per `evaluate` or `predict` call; at least 1 |
| `memory_mib_estimate` | the expected memory of one worker, in MiB, for the planned-memory report |
| `config` (optional) | any JSON value, for the adapter |
| `note` (optional) | text |

- **Paths and patterns** are relative to the home, with `/`, and have no `.` or `..` segment. Every pattern
  must match at least one file.
- **The listed files** are those that `code` or `data` match. They are everything under the home the
  engine may read (besides `engine.json`), and every module of the engine must be one of the `code` files.
- **The fingerprint** is the sha256 over canonical JSON of: the sha256 of `engine.json`'s bytes; every listed
  file as [path, sha256 of its bytes], sorted by path; the external datasets as [source, sha256]; and the
  library versions (the Python minor version, numpy, scipy). The tool computes it; `engine.json` never
  supplies it. Any changed byte of a listed file, of `engine.json` or of a library version is a new
  fingerprint, so every cached result of the engine is computed again.

### The interface

```text
make_engine(config, scratch_dir) -> engine     config: {"root", "manifest", "external"}
engine.describe() -> {engine_id, version, ops, metrics, entity_classes, max_batch}
engine.evaluate(requests) -> [result, ...]     one per request, in order
engine.predict(requests) -> [result, ...]      one per request, in order
engine.search(request) -> result
engine.reset() -> None                         drops derived caches, never prepared inputs
```

- **Values.** Requests and results are JSON values only: dict with string keys, list, str, int, finite
  float, bool and None, exactly those types. A tuple, a set, a subclass or NaN in a result is
  `engine_protocol_error` (`not_json`), with `ref` the pointer of the value in that request's result.
- **A request** is an object. Its member `entities`, if present, maps opaque entity ids to what the engine
  may read about each; every other member is read whole.
- **A result** holds `depends_on`, the ids of the request's `entities` that the result read, each once. An
  `evaluate` result also holds `metrics` (metric name to number) and optionally `flags`, `unmodelled` and
  `pending_readings_used` (lists of strings), and nothing else. A `predict` or `search` result's other
  members are the engine's. A result of another shape is `engine_protocol_error` (`result_invalid`).
- **A domain failure** of one request is the result `{"error": {"code": <string>, "detail": <JSON value>}}`:
  that request's `engine_failed` (`adapter_error`), with `engine_code` the code and `engine_detail` the
  detail as canonical JSON.
- **`describe`** agrees with `engine.json` on `engine_id`, `version`, `max_batch` and the set of `ops`;
  `metrics` and `entity_classes` are lists of strings. Otherwise `engine_protocol_error`
  (`describe_invalid`).
- **What an engine must not do** is in `cli.md` ("Engine host", Guards): write outside its scratch
  directory, read unlisted files or modules of its home, or change `sys.argv`, the working directory,
  `sys.path` or `os.environ`.
