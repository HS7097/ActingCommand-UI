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
| `measurement` | `ref`, `date` [`by`, `evidence`, `role` and its members] |
| `guide` | `ref` [`source` (id from `sources.json`), or `role` and its members] |
| `logic` | `ref` |
| `pending_check` | `ref` [`by`] |

**Roles** (#424 ACM-PROV). A measurement, and a guide row that is a live reading, says what it is for with `role`:

| `role` | Meaning | Its members |
|---|---|---|
| `supplement` | a neutral constant that no public source gives, supplemented from observation | `searched` (a list of ids from `sources.json`: the sources checked that have no value) [`samples` ≥ 1 and `successes` ≤ `samples`, together] |
| `behaviour` | a confirmation of how the game behaves; never the provenance of a number | none |

- **A drop rate**: in the provenance of a `random` rule whose `drop` is a `rate`, every `supplement` carries
  `samples` and `successes` (`member_missing`). A drop from `samples` holds them already.
- **Refusals** (`model_file_invalid`): another `role` is `value_invalid`; a `supplement` without `searched`, or
  with only one of `samples` and `successes`, is `member_missing`; `searched`, `samples` or `successes` beside
  `behaviour`, or beside a guide row's `source`, is `member_unknown`, and without any `role` `member_missing`
  (`role`). A guide row names `source` or `role`, not both (`member_conflict`).
- **`role` may be absent for now:** the static check warns of a measurement without it (see "Provenance classes"
  below).
- **`by`** names a role, such as the team that measured, never an instance, a device or an account.
- **Account data has no kind here.** Inventories, levels, formations, what an account farms and its ledger income
  are account state, never a rule or a data row; an output that reads the account state is per-account and is
  refused from every shared output (`per_account`).

## `model.json` (`acmodel.model.v1`)

| Member | Value |
|---|---|
| `model_id` | id |
| `files` | `{"rules": [...], "data": [...]}`: the model's files, relative to the folder of `model.json`. Each path is `rules/…json` or `data/…json`, with segments `[a-z0-9][a-z0-9_.-]*`. |
| `periods` | the period entities (resets), see below |
| `numeraire` | the id of the `value` entity that values are counted in (#424 ACM-PRICE 2.2 (1); see "The objective" under "The economy") |
| `engines` | a list of `{engine_id, interface, min_version}`. `interface` is `acmodel.engine.v2`; `min_version` is digits separated by dots. Where an engine's code lives is local configuration, never this file (#424 design D-4). |
| [`note`] | |

A period is `{id, unit, reset_minute, utc_offset_minutes}` [`note`]:
- `reset_minute` is 0–1439 (the minute of the day of the reset);
- `utc_offset_minutes` is −720 to 840;
- `unit` is `day`, `week` (adds `reset_weekday`: `mon` … `sun`), `month` (adds `reset_day`: 1–28) or `days`
  (#424 ACM-PRICE 3.4), a period of a fixed number of days such as a 14-day season: it adds `length` (2–366, its
  days) and `anchor_date` (`YYYY-MM-DD`, the date of one of its resets).

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
| `value` | `id` [`note`]: the unit values are counted in, which `numeraire` names (#424 ACM-PRICE 2.2 (1)). It holds no stock and has no reading: it is no pool, no flow consumes or produces it, and nothing binds it. |

Periods are entities too; they are declared in `model.json`.

**Rules:** `id`, `kind`, `subject` (entity id), `logic`, `provenance`, `status` [`when` (expr; absent means
always), `unknowns` (unknown ids), `supersedes` (rule ids), `note`]. `kind` selects the members of `logic`:

| `kind` | `logic` members |
|---|---|
| `cap` | `cap_kind` selects the rest. `hard` and `storage`: `on_reach` and exactly one of `value` (expr) and `reading` (fact key). `soft_read`: `reading` and `on_reach`. `income` (#424 ACM-PRICE 3.3), what a period may earn: `value` (expr), `per` (period id) and `on_reach` [`from` (activity ids: the activities whose income it counts; absent means every activity that produces the resource or moves into it)]; it takes neither `reading` nor `reference`. `on_reach` is `block_all_income`, `stop_production`, `discard`, `void_at_reset` or `{"block_income_from": [activity ids], "other_income": "notice_only"}`. `hard`, `storage` and `soft_read` may add [`reference` (expr): with a `reading` only, the cap's value in the reference state, which the economy's steady state reads instead of the reading (#424 ACM-PRICE G-A); it must not read the account state. The reference state is neutral (ruling 126 A): the value is the same for every account, never this account's level] and [`per` (period id): with `on_reach` `void_at_reset` only, the period at whose resets the cap voids what lies above it; absent means every day (G-C)]. A `reference` beside a `value` is `member_conflict`, a `per` with another `on_reach` `member_unknown`. |
| `regen` | `amount`, `every` [`while` (absent means always)], all expr |
| `flow` | [`consumes`, `produces` (resource id to expr; a product may instead be `random:<id>`, naming a `random` rule), `moves` (`from`, `to`, `amount`), `counts_on` (`success` or `attempt`), `batch` (`min`, `max`), `requires` (gate rule ids)] |
| `exchange` | `gives`, `takes` (resource id to expr) [`limit` (expr)] |
| `limit` | `max` (expr), `per` (period id) [`shared_with` (activity ids; absent means not shared)] |
| `cooldown` | `min_interval` (expr) |
| `gate` | `irreversible` (boolean), `origin` (`any` or `person`) [`requires` (expr; absent means no state condition), `prefs` (preference keys)] |
| `random` | exactly one of `pity_chain` (`rate`, `hard_pity`, `carry_over` (boolean) [`p_up`, `guarantee_after_miss` (boolean), `spark`]), `drop` (`rate`, or `samples` ≥ 1 with `successes` ≤ `samples`) and `outcomes` (a non-empty list of `{p, then}`, `then` being resource id to expr) |
| `derive` | `value` [`round` (`floor`, `ceil` or `half_even`; absent means exact), `general_logic` (expr), `live` (`fact`, `comparison`, `reference_fact`, `offset`), `estimate` (below)]. `comparison` is `eq`, `not_eq`, `less_than`, `less_than_or_equal`, `greater_than` or `greater_than_or_equal`, as in the Runtime's fact references. |
| `engine` | `engine` (engine id), `op` (`evaluate`, `predict` or `search`), `inputs` (id to expr), `metrics` (ids) |
| `objective` | a goal sink of the economy (#424 ACM-PRICE 2.2, PRICE-2a; see "The objective" under "The economy"). `tier` selects the rest. `main`, a main goal: `value` (expr), what one unit of the subject, a resource, is worth in value units. `secondary`, the reference demand, whose subject is the numeraire: `demand` and `costs` (table ids), `value` (expr), what one training point is worth in value units, and `horizon` (expr), the time the demand's counts are spread over. It reads no preference. |
| `weight_ref` | `file`, `pointer` (a non-empty JSON pointer), `compares_to` (output id) |
| `no_use` | a declared zero (#424 ACM-PRICE addendum 2.1 §3.2, PRICE-2b-2): the subject, a pool (a resource), has no use. `reason` (text, why) [`holds_when` (text, the account condition it holds under; absent means always) and `facts` (a non-empty list of fact keys, the neutral facts the resource side checks that condition with on the instances), which go together (`member_missing`)]. It has no value and no expression. Its provenance names the public pages the reason rests on. |

**The estimate of a provisional derive** (`estimate`, #424 ACM-PRICE addendum 2.1 §3.7, PRICE-2b-2) says what range
the derive's point value stands for, so that the impact assessment can move it (PRICE-4). `value` stays the point value
everywhere: the steady state, `run` and the economy read only it, and nothing reads the estimate but the coverage section
and the checks below.
- It is allowed only on a rule of status `provisional`; on any other it is `member_unknown`.
- Its members: `basis` (`variants`, a choice the pack makes at run time, by variant; `samples`, a supplement from
  observation; `statistics`, a public statistics site; `design`, a design value), `covers` (`activities`, a non-empty list
  of the activities whose amounts per run the estimate describes [`holds_when`, text, the account condition the point value
  holds under, never which account; `facts`, fact keys that check it]), and exactly one of `of` (an expression: the quantity
  the point value averages, written with the unknowns it averages over, `unknown:<id>.<member>`; the range is its value
  over every scenario of them, each point of a discrete one and the enclosure of a continuous one) and the pair `low` and
  `high` (expressions, the two ends). `of` with `low` or `high` is `member_conflict`, neither `member_missing` (both with
  `expected` `["of", "low"]`), and `low` without `high` or the reverse `member_missing`.
- Which variants it covers is given by the unknowns `of` reads; which stages, shops or projects by `covers.activities`;
  which accounts by `covers.holds_when`.

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

`sources`: a list of `{id, title}` [`url`, `class`, `note`]. The ids mirror the guide's source list, so that one
page has one id.

`class` (#424 ACM-PROV) is what the source is: `wiki`, `drop_stats` (a drop statistics site), `datamine` (data
read from the game's files) or `official` (the publisher's own). Another value is `value_invalid`. It may be
absent for now: the static check warns of a source without it (see "Provenance classes" below).

## `exports.json` (`acmodel.exports.v1`)

| Member | Value |
|---|---|
| `thresholds` | a list of `{id, from, form}` [`round`, `note`]: `id` is an id, unique among the thresholds; `from` is the id of a `derive` rule; `form` is `constant` (the rule's value) or `fact_offset` (the offset of the rule's `live`); `round` is `floor`, `ceil` or `half_even` (absent means exact) |
| `tables` | a list of tables, each `{table_id, game, row_key, columns, rows}` [`note`] (below) |
| `scoring` | a list of `{id, template}` [`note`]: `id` is an id; `template` a file relative to the model folder, `/`-separated segments `[a-z0-9][a-z0-9_.-]*` ending in `.json` |
| [`acceptances`] | a list of `{output, accept_provisional: {ref, until}}` [`note`]: `output` is an output id (`rule:<id>`, `general_logic:<id>`, `threshold:<id>`); `ref` names the ruling or board comment, `until` the pending check. Absent means none. |
| [`note`] | |

**A table** becomes the Runtime's shared table `tables/<table_id>.json` (`actingcommand.table.v1`) of the
resource repository; see "`acmodel export tables`" below.
- `table_id` and every column `name` are ids; `game` is the table's game identifier and `row_key` a list of
  column names, both passed through to the table.
- `columns`: a list of columns, each with `name` and `type`; `type` selects the other members:

  | `type` | Members | Its cells in `rows` |
  |---|---|---|
  | `key` | none | a string |
  | `enum_string` | `allowed` (strings) [`nullable`, `valid_until`] | a string |
  | `integer` | `min`, `max` (integers), `scale` (an integer ≥ 1), `round` (`floor`, `ceil` or `half_even`) [`nullable`, `valid_until`] | `{"$model": "<output id>"}` |
  | `boolean` | [`nullable`, `valid_until`] | `{"$model": "<output id>"}` |

  `nullable` is a boolean, `valid_until` the id of a `window` entity (each column may add `note`). The scale
  and the rounding of a number are declared, never implicit.
- `rows`: a list of objects, each with exactly one member per column (by its name).
- A column name used twice is `value_invalid`. Nothing else of the Runtime's table rules is checked here: the
  Runtime's own validator checks the written table (the tool keeps no copy of its limits).

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

`acmodel check <model> [--strict-provenance] [--report-supplements]` takes the model folder or its `model.json`.
It reads `model.json`, `sources.json` in the same folder, and every file that `model.json` lists. If a reader
refuses a file, the check reads the remaining files, reports every reader's errors and stops. Otherwise it checks
the rules below.
- It computes nothing, and it reports every problem at once, with exit status 1.
- Each error holds the file in `detail.path` and the JSON pointer in `ref`. In an expression, `position` is the
  character offset of the reference.
- The provenance classes are warnings, and errors only with `--strict-provenance` (see "Provenance classes"
  below). The warnings are reported with the errors, if there are any.
- On success `data` holds `path`, `model_id` and these counts: `entities` (periods included), `rules` (the
  active ones), `superseded`, `unknowns` and `tables`. With `--report-supplements` it also holds `supplements`.

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
- **Sources:** every `source` that a provenance entry cites (a guide row's too), and every id of a `searched`, is
  in `sources.json` (`rule_reference_missing`, with `expected` `source`). So is the `source` of an external
  dataset.

### Provenance classes

The modelling data comes from public sources: a wiki, a drop statistics site, the game's data, the publisher. A
neutral constant that none of them gives may be supplemented from observation, marked as such; account data
never enters the model (#424 ACM-PROV). So the check tells these apart:

- **Every number of a confirmed rule or data row** has at least one provenance entry that can be a number's:
  - a `source`;
  - a `measurement` of `role` `supplement` with a non-empty `searched`;
  - a `ruling` or a `logic` entry (a design value, such as a floor);
  - a `guide` row counts as what it names underneath it: on a `source`, as a source; with a `role`, as a
    measurement of that role.

  A rule's numbers are the numbers and durations of its expressions (its `general_logic` and `when` too) and a
  drop's `samples` and `successes`; a row's are its cells of `integer` and `decimal` columns that are not key
  columns. A rule or row with only `behaviour` confirmations, pending checks, measurements without a role or
  guide rows without a class is `rule_provenance_unsourced` (`number_unsourced`, `found` its entries), at its
  provenance list (at the row, when the table's provenance is the row's). An empty provenance list is
  `rule_provenance_missing` already. Pending and provisional entries, unknowns and superseded rules are not
  checked: their values are not confirmed, or never read.
- **The classes are named:** a measurement without `role` is `rule_provenance_unsourced` (`role_missing`), a
  source of `sources.json` without `class` is `class_missing` (`entry` `source`), and a guide row that names
  neither a `source` nor a `role` is `guide_class_missing`, each at its entry.
- **Warnings for now.** Today's rule files were written before the classes, so each of these is a warning, which
  never changes the exit status. With `--strict-provenance` they are errors (exit 1). `bindings check --strict`
  and every command that runs the model do not read them: they never stop a binding, a run or an export.
- **`--report-supplements`** lists every supplemented value, so that it can be replaced when a public source
  appears: `data.supplements` holds, for each provenance entry of `role` `supplement` (a superseded rule's
  excepted), `path`, `ref` (the entry), `entry` (`rule`, `unknown`, `row` or `table`), `id`, and the entry
  itself as `provenance`. They are in the order the check reads them: the rules of the rule files, then their
  unknowns, then the data files.

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
| a store's `holds`; the keys of `consumes`, `produces`, `gives`, `takes` and an outcome's `then`; `amount:<id>`; the `amount:<id>` dimension of a column or an attribute; a main objective's `subject`; each column of a secondary objective's costs table but its key (the error at that column of the data file) | a `resource` |
| `numeraire` of `model.json` | a `value` entity |
| a secondary objective's `subject` | the numeraire (`expected` `numeraire`) |
| `stock:<id>` | a `store` |
| `moves.from`, `moves.to` | a `resource` or a `store` |
| `block_income_from`, `shared_with`, an income cap's `from`, an estimate's `covers.activities` | `activity` entities |
| a `no_use` rule's `subject` | a `resource` (`expected` `resource`, with `found`) |
| `limit.per`, a cap's `per` | a period |
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
- **A secondary objective** (#424 ACM-PRICE 2.3): `demand` and `costs` name tables of the model (`table`, at the
  member); the demand table declares the columns `count`, `points` and `main` (`column`, at `/logic/demand`); and
  the two are joined by key, so the key of each row of an inline demand table is the key of a row of the costs
  table (`row`, with `table` the costs table and `key` the row's key, at that row of the demand table's file).

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

A preference is read only in the expressions of a `gate` rule (#424 design D-2, as ACM-PRICE 2.2 (7) changes it).
Anywhere else, unknowns and every member of an `objective` rule included, `pref:` is `rule_pref_misplaced`: the
objective gives values that are the same for every account, and the preferences are a layer of their own above
them (ruling R18).

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
| `regen`, `flow`, `exchange`, `cooldown`, `random`, `objective`, `no_use` | the subject alone |
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
  - A derive's `general_logic` is only compared with its value, and nothing reads it, so it links nothing; nor does
    its `estimate`.
- **Refusal:** a loop is `rule_cycle` with `via` `reference`. `members` are its nodes, sorted (`rule:<id>`,
  `unknown:<id>` or `unknown:<id>.<member>`). The error points at the first member's first link into the loop.

### Declared zeros and estimates (#424 ACM-PRICE PRICE-2b-2)

These read the static judgement of coverage (see "Coverage" under "The economy"), which needs every reference
resolved: the check makes them only when it found nothing else.
- **`no_use_contradicted`** (addendum 2.1 §3.2): a pool declared zero is covered: it has a main objective (`goal`), a
  demand row takes it (`costs`), or an activity takes it and brings in a covered pool that is not declared zero (`use`,
  with the `activities`). A use that leads only to other declared zeros is no use, so it never contradicts. The error is
  at the declaration, and nothing of the model is computed while it stands.
- **`no_use_condition_incomplete`**: a pool's uses lead directly to another declared zero (an activity takes the pool and
  brings that zero in), whose `facts` the pool's declaration does not all hold (`downstream`, `missing`): the pool is
  zero only where its downstream zero is, so its condition is its own and theirs together. Each declaration answers for
  the zeros its uses lead to directly; those answer for theirs. A pool that is neither covered nor declared zero is
  unknown, never 0.
- **An estimate's checks** (addendum 2.1 §3.7). Check 1, one of `of` and `low`/`high`, is the reader's. Check 2, the
  point value within the range, needs the values: the run makes it (`estimate_outside_range`, see "Evaluation" under
  "The run"). Here:
  - check 3, **`estimate_basis_invalid`**: `basis` `variants` needs an `of` that reads a member of a `variants` unknown
    (`missing` `variants_unknown`); `samples` needs a `measurement` of role `supplement` in the rule's provenance
    (`supplement`), one with `samples` and `successes` when `of` reads a `beta_from_samples` distribution (`samples`);
  - check 4, **`estimate_scope_exceeded`**: each activity one of whose rules (a flow, a limit: a rule whose subject is the
    activity) reads the derive, directly or through other derives, is in `covers.activities`; `activities` are those that
    are not, and `read_by` their rules that read it. An estimate is never used for an activity it does not cover.

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
  | cap `value`, `reference` | the cap's dimension |
  | limit `max`; flow `batch.min`, `batch.max`; exchange `limit` | `count` |
  | regen `amount` | the amount of what its subject holds (a resource or a store) |
  | regen `every`, cooldown `min_interval`, objective `horizon` | `time` |
  | objective `value` | `value` (a number literal fits) |
  | flow `consumes`, `produces`; exchange `gives`, `takes` | the amount of the resource named by the key |
  | flow `moves.amount` | the amount that `from` holds, and the amount that `to` holds |
  | derive `general_logic` | the dimension of its `value` |
  | derive `estimate`: `of`, `low`, `high` | a number of the dimension of its `value`, never a boolean (`operator` `estimate`) |
  | derive `live.offset` | the dimension of `live.reference_fact`, which is that of `live.fact` (`operator` `live`) |
  | random `pity_chain.rate`, `pity_chain.p_up`, `drop.rate`, an outcome's `p` | `none` (a chance) |
  | random `pity_chain.hard_pity`, `pity_chain.spark` | `count` |
  | random: an outcome's `then` | the amount of the resource named by the key |
  | derive `value` with `round` | a number that is not `time` (`operator` `round`) |
  | the members of the variants of an unknown, the bounds of an interval or a distribution | one dimension (`operator` `variants`, `interval` or `distribution`) |
  | each key of `lookup(<table>, …)` | its key column's type; a key column of type `id` or `enum` (or none declared) cannot be looked up |

  `operator` names the member, and `operands` are the expected and the found type. The engine members are not
  checked here; their slice checks them.
- **A secondary objective's tables** (#424 ACM-PRICE 2.3): each column the economy reads is an `integer` or
  `decimal` column of its dimension. In the demand table `count` is a `count`, `points` and `main` are `none`; in
  the costs table the column of each resource is the amount of that resource. Another type or dimension is
  `operator` `demand` or `costs`, position 0, at the column (`/columns/<id>`, or its `/dimension`) of the data file.

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
  reading; never its `reference`, which only the economy's steady state reads), `limit` (its `max`) and `random`
  rule with a `pity_chain` or a `drop` (see "Random rules");
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
weight_ref, no_use), and random rules with `outcomes`, are type-checked but not evaluated. `acmodel economy` reads the
flow, regen, cap, limit, gate and objective rules, and its coverage section the `no_use` rules (see "The economy").

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

**An estimate's range** (#424 ACM-PRICE addendum 2.1 §3.7, check 2; PRICE-2b-2). A derive with an `estimate` computes
it beside its value, in the same state, and nothing reads it: `of` over every scenario of the unknowns it reads (from its
lowest to its highest value, an enclosure counting with both ends), or the lowest value of `low` to the highest of
`high`. Every scenario of the value, both ends of an enclosure, lies in that range; otherwise the rule is blocked
(`estimate_outside_range`, with `value`, `low` and `high`, at the estimate), and so is every output that reads it. An
estimate that cannot be computed blocks the rule with its own errors. In the reference state of the economy's steady
state, an estimate that needs the account state is not checked: `run`, which reads the state, checks it.

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
between top-tier pulls; with one, the exact DP runs to the spark.
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

**A value too large to carry exactly.** A random rule's value whose numerator or denominator has more than
`expr.max_bits` bits (a long chain's expected pulls, say) is carried as its sound enclosure: the low end rounded
down and the high end up to `random.enclosure_digits` (12) decimal digits, with `exact: false`. So it is never
rounded silently: it shows a `range`, flows through the interval arithmetic and the no-effect rule like any
enclosure, and what reads it computes with bounds that fit. The exact value stays in the rule's provenance node
(`exact_values`, and so in `explain`). A value that fits stays exact.

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

## The economy (`acmodel economy`)

`acmodel economy <model> --state <file>` checks and reads the model as `run` does, then solves the economy (#424
design 4.3 and 4.4, design-py 4.3; `src/acmodel/core/flow.py`): two linear programs generated from the rules,
solved by scipy's HiGHS and checked exactly. It computes only the graph nodes they read, through the cache as `run`
does. The configuration keys are under `lp`, `search` and `economy` (`contracts/cli.md`).

### What the programs model

- **Time.** One step is one day. Reset minutes and offsets are not read: the steady state looks the same from every
  step, and a plan starts at a reset of every day and week period.
  - **The steady cycle** (#424 ACM-PRICE 3.4) is the least common multiple of the lengths of the periods that have a
    fixed number of days: a `day` 1, a `week` 7, a `days` period its `length`. They are taken shortest first (by id
    among equals), each while the multiple stays at most `economy.max_cycle_steps` (28); one that would make it
    longer is relaxed (below). With a day and a week the cycle is 7 steps, with a 14-day period as well 14; with no
    period it is 1.
  - **Relaxed periods** (3.3). A `month` has no fixed number of days: it counts as 146097/4800 days (the mean of the
    Gregorian calendar's 400 years, exact). In the steady state a month and every period the cycle does not take are
    relaxed by rate; so are, in the plan, a month and every `days` period (the plan has no start date that would place
    its `anchor_date`). A limit or income cap of a relaxed period has one window, the whole program, and its value
    counts times the program's days over the period's (`value × steps ÷ days`). This is exact for the long-run
    average and drops only the order within the period. Each is listed in `relaxed`, by rule: `{rule, per, days,
    value}`, `days` the period's and `value` what it counts over the program. A `void_at_reset` cap of a relaxed
    period is refused (`void_at_reset`, at its `per`): its resets have no place in the cycle.
- **Stocks.** Each resource and store has a stock at every step boundary, never below 0. A store's stock is apart
  from the stock of the resource it holds.
- **Activities.** Each activity that is the subject of a flow has a count per step. One execution performs all its
  flows: what they consume and produce, and their moves. `counts_on` and `batch` are not read: in the programs every
  execution succeeds, and a batch is that many executions.
- **Moves.** A move whose amount comes from the rules and the data moves that amount per execution. A move whose
  amount needs the account state, such as a collection of what the store holds (`stock:<store>`), moves a variable
  amount, at most its store's storage cap per execution; its store must have a storage cap with a value, or in the
  steady state with a `reference` (`state`).
- **Regens.** A regen with an `every` of a day or less brings its amount times a day over `every` in each step. A
  longer `every` must be a whole number of days, and its amount comes whole in the first step and every that many
  steps after; in the steady state it must divide the cycle (`every`). An `every` that is not positive is
  `value_invalid`. A store's regen is its production. A `while` is read only when it says where production stops,
  in one of these two forms; any other is refused (`while`):
  - a store's regen, what the store's storage cap does (`stock:<store> < cap:<its storage cap>`);
  - in the steady state, a resource's regen that stops at a cap of that resource (#424 ACM-PRICE G-B):
    `amount:<resource> < cap:<cap>` (or `rule:<cap>`), the cap having a `value` or a `reference`, or
    `amount:<resource> < read:<fact>`, the fact being the reading of the one cap of that resource that has a
    `reference`. The cap's value (its `reference`, for a reading) is where the regen stops. In each step what the step
    consumes of the resource comes first, then what the regen brings, up to that value; the rest of the regen is lost
    (`lost_production`, at most what the regen brings in the step). Other incomes are not stopped. The cap is priced
    like any cap with a value, and its stock ends the cycle with exactly the stock it began with. The plan does not
    model it yet (`while`).
- **Caps:**
  - `hard` on a resource, with `block_all_income` or `discard`: its stock stays at or below the cap at every step
    boundary. What the cap blocks overflows: at most the step's income, never stock already held.
  - `hard` on a resource with `void_at_reset`, in the steady state (G-C): read as `discard`, but only at the resets
    of its `per` (every step boundary for a day, or without `per`; every seventh, from the first, for a week; every
    `length`-th for a `days` period the cycle takes). Between
    them the stock may exceed the cap; what lies above it at a reset is voided: it overflows in the step that ends at
    the reset, at most the income since the last reset. The plan refuses it (`void_at_reset`).
  - `income` on a resource, with `discard` or `block_all_income` (#424 ACM-PRICE 3.3): per window of its `per` (as
    a limit's, below; a relaxed period's at its rate), what the activities it counts bring into the resource (their
    products and their moves into it) less what the cap voids is at most its `value`. It voids, in each step, at most
    what they bring in that step, and the rest reaches the stock as usual; what it voids is never stock already held,
    and with a hard cap on the same resource the two together never remove more than the step's income. Other
    incomes (a regen, other activities) are not counted. Both programs read it, and it is priced like any cap with a
    value (`rule_prices`). The overflow among the losses is the least with what the cap voids aside.
  - `storage` on a store, with `stop_production`, `discard` or `block_all_income`: its stock stays at or below the
    cap, and production beyond it is lost (at most the step's production).
  - `soft_read` on a resource, with `block_income_from`: the plan reads it (see below). The steady state reads no
    account value, so it does not, but where a resource's regen stops at it (above).
  - A cap with a `reading` counts, in the steady state, at its `reference`, and so do `cap:<id>` and `rule:<id>` in
    what the steady state reads (G-A); `read:<fact>` still reads the account state. A `reference` that needs the
    account state is refused (`state`), as is a reading without a `reference`. The plan and `run` read the reading.
  - Any other subject or `on_reach` is refused, with `found` the subject's kind or the `on_reach`.
- **Limits.** Each window of the limit's period (one step for a day, seven for a week, `length` for a `days` period;
  the whole program, at its rate, for a relaxed one) counts the executions of its subject and of its `shared_with`
  activities: at most `max`.
- **Slots: concurrency as slot-hours** (#424 ACM-PRICE 3.1). A bound on how many executions run at once (a number
  of slots, such as four expeditions at a time) needs no format of its own. It is written as a resource of
  slot-hours:
  - a `resource` entity `slot.<name>`;
  - a `regen` of it: `amount` the number of slots, `every` `1h`, no `while`;
  - a `hard` cap of it: `value` `"0"`, `on_reach` `discard`;
  - each activity that holds a slot consumes its hours: `consumes` `slot.<name>` the hours it runs (an expression,
    such as `rule:<a derive of the hours>`).

  The cap of 0 voids every slot-hour not used in the step it comes in, so slots never carry over: a step gives at
  most slots × 24 hours of running. The cap's price is `{"low": "0", "high": null}`, and the slot's price is what one
  more hour of running is worth: an activity bound by the slots is worth, before its slot-hours, its slot price times
  its hours. Such a resource (any resource with an active hard cap whose `value` is the number 0) is a slot: it is no
  Runtime pool and its price is a diagnosis only, so `bindings check` and `bindings apply` refuse any entry that names
  it as a pool (`binding_mismatch`, `slot`; see "Bindings").
- **Gates.** A flow behind a gate of origin `person` (real payment) takes its activity out of both programs; the
  activity is listed in `excluded` (#424 design 4.7). A gate with no `requires`, no `prefs` and not irreversible lets
  the flow run. Any other gate is refused (`gate`).
- **Objectives.** The steady state's objective comes from the `objective` rules, its goal sinks (see "The
  objective" below). The plan reads none of them: its objective values its final stocks.
- **Not modelled yet**, each refused: `exchange` and `cooldown` rules; a `random:` product; a `when`; and an amount,
  cap, limit or objective value that reads an unknown (`unknown`), because each program solves single values. In the
  steady state, a pending flow is left out instead (below).
- **Pending flows** (the ACM-PRICE addendum, part 1). In the steady state, a flow whose `status` is `pending` and one
  of whose amounts reads an unknown is not refused: its activity is left out of the program and listed in
  `excluded_pending`, with the flow, the activity and the unknowns its amounts read. It is a structural gap: every
  price of its static component is then provisional, computed without that activity, and a component with no edge to
  it keeps its own status (`price_status`, see "Coverage" below; addendum 2.1 §3.3, PRICE-2b-2). Once the flow no longer
  reads an unknown (a measurement filled in), it is in the program again, and nothing is provisional for that reason. A flow of another status that reads
  an unknown, any other rule that does, and every such flow in the plan are refused (`unknown`).

Every refusal is `expr_eval_failed` (`evaluation_unsupported`, operator `economy`, `found` as named above) at the
rule, member or period, and blocks the output; nothing is approximated.

**Status** of each output: the worst of the rules, rows and readings it read. It is provisional when it depends on
an unknown it does not read: one that a pending rule it read names. Each amount, regen member, cap, limit, gate,
objective member and demand or costs row keeps its own status, so that each price has the status of its static
component (#424 ACM-PRICE 4.6, PRICE-2b-2; see "Coverage" below). The steady state's status is the worst of all of them
but those of the pending flows it leaves out, provisional when it leaves one out, and never better than any price's.

### The objective (`objective` rules; #424 ACM-PRICE 2.2, 2.3 and addendum 2.1 §4, PRICE-2a)

The steady state values everything in the unit of the numeraire, a `value` entity. That entity holds no stock, so
nothing can be kept in it: what counts is what the **goal sinks** bring in per cycle. Each sink is a column of the
steady state's program, per step, that takes something and brings value units.

**A main goal** is one `objective` rule of `tier` `main` per resource that is a main goal item (one that the game
sells for real money and that play can earn again and again, such as a premium currency):
```json
{"id": "goal.<item>", "kind": "objective", "subject": "<resource>",
 "logic": {"tier": "main", "value": "rule:w.<item>"}, "provenance": […], "status": "…"}
```
- `value` is what one unit of the resource is worth, in value units (dimension `value`; a number fits). It comes from
  the public exchange rates, as a rule with its provenance (`w.<item>`, of class `official` or `wiki`); it is data,
  never a preference.
- Its sink takes one unit of the resource for `value` value units, in any number.

**The secondary goal** is at most one `objective` rule of `tier` `secondary`, whose `subject` is the numeraire: the
reference demand, the training a neutral reference state still needs (ruling 108; the reference state is neutral,
ruling 126 A, below):
```json
{"id": "goal.training", "kind": "objective", "subject": "<the numeraire>",
 "logic": {"tier": "secondary", "demand": "refdemand", "costs": "training_costs",
           "value": "rule:w.point", "horizon": "365d"}, "provenance": […], "status": "…"}
```
- `demand` names the demand table, `costs` the costs table, both of the model (`files.data`, inline rows). The costs
  are written once, in the costs table; the demand table names them by key.
- The **costs table** has a key (a step of training, such as `<class>.lv100_105`) and one column per resource that
  some step costs, the column's id being the resource's id, `integer` or `decimal`, of dimension `amount:<resource>`;
  a step that does not use a resource has 0 there. Each row's provenance is the public cost table it comes from.
- The **demand table** has the same key and three columns: `count` (dimension `count`), how many times the reference
  state still does the step over `horizon`; `points` (`none`), the training points one step is worth; and `main`
  (`none`), the value units the step adds to the main goal later (a level that raises a repeatable main income;
  usually 0: the neutral reference state has every mode at its top tier already). The key of each row must be a row
  of the costs table (`data_reference_missing`, `row`). A row may stand for a class of steps; the grain is the
  research channel's.
- `value` is what one training point is worth, in value units (λ, from public exchange rates, with its provenance:
  data, never a preference); `horizon` (time, positive, else `value_invalid`) is the time the counts are spread over
  (365 days for a year's model).
- Each demand row is a sink that takes the row's costs for `points × value + main` value units, at most `count ×
  cycle days ÷ horizon days` times a cycle (its bound is a group of the infeasibility analysis, named by the
  objective). A pending row blocks the steady state (`rule_pending_unbounded`, `no_unknown`), as a run does; an
  external table is refused (`external_dataset`).

**Two functions**, each defined once and used both to optimise and to check (`core/flow.py`: `total_value`,
`acquisition`, `acquirers`):
- **The value V**: what the goal sinks bring over the cycle, each main goal item taken at its `value` and each
  demand row at `points × value + main`.
- **The protected acquisition A**: what the protected acquisitions (P, below) bring in of the main goal items over
  the cycle, net of what they take of them, at their values; less what an income cap on such an item that counts an
  activity of P voids of it, at that value; plus the `main` of each demand row's executions. A use of a main goal item
  (a sink, a conversion that spends it) never lowers A: the use side is compared inside V.
- **P, the protected acquisitions** (§4.1, decided mechanically): an activity is in P when all three hold:
  1. it acquires a main goal item: it produces one (more than 0 an execution) or moves into one;
  2. a limit counts it (its subject or `shared_with`), or an income cap does (its `from` lists it, or omits `from`
     while it brings the cap's resource in);
  3. each of its inputs is time. Its inputs are the resources it consumes a non-zero amount of (an amount of 0 is no
     input) and what its moves take from. An input is time when it is a resource or store with a regen that brings
     something and is not a slot (stamina, a facility's store), or a slot (see "Slots") that only activities of the
     first kind take from, so that its capacity earns main goal income alone.
  Every other activity of the first kind is a **conversion**: it spends another pool (a stock without regen, such as
  a currency or a material) or capacity it shares with other uses, and is compared inside V like any activity. It
  is never protected; a minimum of it is an instruction (#429 R-13, a player goal), not the model's.
- **The three solves** (§4.2), each exact:
  - S3 maximises V alone. Its optimum V0 is the steady state's `objective`, and its program is the one every price
    re-solve changes. An infeasible S3 is `lp_infeasible`.
  - S1 maximises A alone: A* (`acquisition`). When A has no term (no P, no `main`), A* is 0 and S2 is S3.
  - S2 maximises V with A ≥ A* (the row `floor:acquisition:0`, checked exactly as every other row): V*. V* above V0
    is `lp_numeric_check_failed` (stage `value`).
  - V0 above V* is **`objective_tier_conflict`**: by value alone the secondary goal would take what the protected
    acquisition needs, and pool prices cannot hold that priority, so the steady state is blocked (and so is the plan).
    It lists the protected activities whose part of A is smaller in the optimum of S3 than in that of S2
    (`activities`; `steps` and `caps` for a demand row's `main` and an income cap's void), and P (`protected`). The
    data (the weights and their provenance) or an instruction resolves it, never the engine.
  - V0 = V*: the lexicographic optimum is an optimum of V, and the steady state is priced from S3. An activity that
    some optimum of S3 uses and no optimum of S2 does is the warning **`objective_tier_tie`**, one per activity: by
    value it is as good as the acquisition it would take from (probed on each face, at most one execution a cycle).
    - **Complementary slackness** (PRICE-2c) spares most probes. S3's dual (see "The joint price vector") is solved and
      checked exactly first, and gives an exact optimal dual point (y, w). An activity whose dual slack there,
      s_j = (Aᵀy + w − c)_j, is above 0 in every step runs in no optimum of S3, so in none of S2 either (V* = V0, so
      S2's optima are optima of S3): it is no tie, and it is not probed. The argument: for any feasible x and dual
      feasible (y, w), c·x ≤ (Aᵀy + w)·x = y·(Ax) + w·x ≤ b·y + u·w (x ≥ 0 and s ≥ 0; y ≥ 0 where Ax ≤ b; w ≥ 0 and
      x ≤ u). At an optimum x* both ends equal V0 (strong duality, checked exactly), so Σ_j s_j·x*_j = 0, each term at
      least 0 and so 0: s_j > 0 gives x*_j = 0 in every optimum. A slack of 0 proves nothing, so only those activities
      are probed.
    - **A probe that cannot be verified** (PRICE-2c). A probe's answer above 0 holds for any exactly feasible point; an
      answer of 0 only for a certified optimum ("Exactness and failures"). A tie that its probes cannot decide exactly
      (no certified 0, or a probe whose LP fails its exact check) is the warning **`lp_probe_unverified`** (`probe`
      `tie`), never "no tie": its activity counts as a structural gap of its component (`probe_unverified`, see
      "Coverage") and the steady state stands.
    - **The faces the probes read** (PRICE-2e). The optima of S3 are probed on S3's optimal face in its
      complementary-slackness form ("S3's optimal face", below), with no row V ≥ V0; those of S2 on that form with
      A ≥ A*: V* = V0 here, so S2's optima are S3's that meet A ≥ A*, and the row V ≥ V* is not needed either. The
      same sets as before, so the same answers.
  - The least losses are taken among the lexicographic optima (A ≥ A*, V ≥ V*). That LP keeps the row V ≥ V* (PRICE-2e
    poses the probes' faces without it): `losses` reads its solution, not only its optimal value, and one LP of
    another form could stop at another of the least-loss vertices.
- **The reference state is neutral** (ruling 126 A). The steady state reads the rules and the data only: a cap's
  `reference` is its value in the neutral reference state (facility levels, stage stars), the demand and costs are
  the neutral reference demand from public cost tables, and an objective's `value` and `horizon` never read the
  account state (`state`, as every member). So the values are the same for every account; an account's line-up,
  unlocks, saturation and reward tiers belong to the layer above the model (admission facts, in-step choices, #429
  R-13, player goals, a conditional zero's acceptance scope).
- A model without an objective rule has V = 0: every price is 0.

### The steady state (`steady`)

- **Game-wide.** It reads the rules and the data only, in the reference state (a cap with a reading at its
  `reference`), never the account state. An amount, cap or limit that needs the account state is refused (`state`).
- **Its objective** is the value of its goal sinks over one cycle (V, "The objective" above), after the
  protected acquisition A is checked (S1, S2 and S3). Every stock ends the cycle with at least the stock it began
  with, and a stock with a hard or storage cap, or whose regen stops at a cap, with exactly that stock, so the cycle
  can repeat.
- **Prices** (#424 ACM-PRICE addendum 2.1 §2.2 step 3; PRICE-2b-1). A resource's price in a step is the projection
  of the dual of its balance row in that step on D*, the optimal face of S3's dual (see "The joint price vector"):
  its least value there, the exact value of one more unit, and its greatest, the exact value of one less.
  - When the two agree, the price is exact (`"1/10"`).
  - Otherwise it is the interval `{"low", "high"}` (a degenerate basis); `high` is null when one less is
    infeasible, and `low` when one more is: the cycle cannot hold it. That happens at a regen stop, whose stock
    ends the cycle as it began and loses at most what the regen brings, when the program can use no more of the
    resource and already loses all that its regen brings (its users left out as pending, say). Neither is an error.
  - **Cross-checks** (`lp_dual_disagrees`, a warning; the price stands). The change of the optimum per grain
    (`lp.grain`, 1) when that step brings one grain more of income, and one grain less, is kept in `grain`, in the
    same shape: its chords enclose the projection's exact derivatives, so the projection lies inside it (check
    `grain`, exact); it is wider where a kink lies within a grain (with income 10.5 and a use bound at 10, the
    re-solves give `[1, 3/2]` and the projection `1`). They can differ only where one grain more of income can
    overflow at a hard cap while one more unit of stock has no room. HiGHS's marginal lies inside the projection,
    within the tolerance τ (check `marginal`; see "Marginals" below).
  - **How the re-solves are found** (PRICE-2d; each is the exact optimum of S3's program with that income, the value
    a new LP of it gives, so `grain` and the check are as before). One grain more or less of income in a step moves
    only right sides of S3's program: the balance row of that step, and for a resource with a hard cap, one grain
    more also the overflow row of the step that closes the cap's window (the income it may block). So:
    1. *Proved by S3's basis.* Where S3's certified basis stays primal feasible with the moved right sides, it stays
       optimal, and the optimum moves by exactly the moves times their rows' duals: no LP (below, "How the values on
       D* are found").
    2. *Otherwise from S3.* HiGHS's instance of S3 gets only the moved row ends and goes on from S3's basis, which is
       still dual feasible (no cost changed): its dual simplex, without presolve. The basis it ends at is solved
       exactly by updating S3's basis, which is factorised once: in product form, each variable that enters replacing
       one that leaves (an eta matrix per exchange), so no new elimination is made; then certified exactly as every
       LP's basis is ("Exactness and failures"). A basis this does not certify gets the exact pivots of any LP, from a
       new elimination; so does a basis so far from S3's that the update would cost more than that. The update solves
       once with S3's factorisation for each variable that enters and applies every eta so far, so its cost grows
       with the square of their number: it stops once it has taken as long as a new rebuild takes (the mean of this
       steady state's rebuilds so far, at first twice the time S3's basis took to factorise), and no later update
       then lets more variables enter than it had. The time picks the way only: what comes of a basis is always what
       the new elimination gives at it (a vertex the update certifies is that basis's own, which the elimination
       certifies with no pivot).
    3. *A new LP*, as before PRICE-2d, where HiGHS does not end optimal there (one grain more or less is infeasible,
       say: a new LP decides that, as before), where step 2 gives no certified vertex, or where the income would move
       more than right sides (a store's bound on lost production, or what a move under a read cap takes; no resource
       has one).
    The vertex of step 2 is checked exactly against every bound and row it changes (those holding a variable whose
    value differs from S3's, and the moved rows); every other bound and row is S3's, with S3's values, which S3's own
    exact check passed. A cap's or a limit's price (`rule_prices`) is a new LP for each re-solve.
  - Each cap with a value (a `reference` counts as one; an income cap too) that the program reads and each limit
    (`rule_prices`) is priced by re-solves, with its value one grain higher and one lower: exact when the two agree, an
    interval otherwise. A relaxed rule's value is its value per period, so its price is the cycle's gain per unit of
    it. Every re-solve is of S3, V alone; a price is in value units, and the numeraire, which holds nothing, has none.
- **The status of each price** (`price_status`): resource to `{status, reasons}`, the status and the reasons of its
  static component (see "Coverage" below): `excluded_pending`, `no_steady_supply`, `objective_tier_tie` and
  `probe_unverified` (PRICE-2c), the structural gaps the component holds, and `inputs`, a rule, row or reading of the component is provisional, depends on
  an unknown it leaves unresolved, or names an unknown. A confirmed price has no reason. The graph output
  `price:<resource>` shows its entry (see "The joint price vector" below).
- **Activity values** per step: what one execution produces less what it consumes, at that step's prices, in value
  units. Where a price is an interval, so is the value, and an unbounded end is null. An activity that moves out
  of or into a store has no value (null): what it moves depends on what the store holds.
- **The joint price vector and the activity judgements** (`joint`): see "The joint price vector" below.
- **Losses** over one cycle: per resource with a hard cap its `overflow`, and per store with a storage cap and per
  resource whose regen stops at a cap its `lost_production`. They are the least among the lexicographic optima (A ≥
  A*, V ≥ V*), from a further solve.
- **The tiers**: `objective` is V0 (= V*), `acquisition` A*, and `protected` and `conversions` the activities that
  acquire a main goal item, in P and not, by id, each `{activity, goals, bounds, inputs, spends}`: the main goal items
  it brings in, the limits and income caps that count it, its inputs and of them those that are not time (empty for
  P). A protected activity has a bound and spends nothing. The research channel checks this list against the guide.
- **Marginals.** HiGHS's marginals of the same rows are kept in `marginals`, as a cross-check only, each made a
  fraction by `limit_denominator(lp.max_denominator)`. A marginal m outside its price's interval by more than the
  tolerance τ is the warning `lp_dual_disagrees` (check `marginal`); the price stands. m agrees with an end p of the
  interval when |m − p| ≤ τ · max(1, |p|), with τ = 1e-7 (`MARGINAL_TOLERANCE` in `core/flow.py`), HiGHS's default
  dual feasibility tolerance: a marginal is a float, and an exact price can have a denominator far above
  `lp.max_denominator` (numbers with four decimals give some near 6e28), so the two never agree exactly. Only this
  comparison of a float with an exact price has a tolerance: the check `grain`, of two exact fractions, and every
  exactness check stay exact. A storage cap's marginal also counts the moves it bounds, whose coefficient it is, and a
  cap's the regen stops at it.

### The joint price vector (`joint`; #424 ACM-PRICE addendum 2.1 §2, ruling 127 iii; PRICE-2b-1)

The Runtime adds the pools a task touches before it rounds once, so the prices it is given must be the coordinates of
one dual optimal point, never a pick per pool: with V = min(A, B, C), each pool's interval is [0, 1], their midpoints
(1/2, 1/2, 1/2) value a set at 3/2, and the joint vector (1/3, 1/3, 1/3) at its 1.

- **The dual**, generated from the rows of S3's program, never written by hand: a variable per row (non-negative for
  an at-most; free for an equality, held as the difference of two non-negative ones, `y+` less `y-`), a non-negative
  one per variable with an upper bound, a row per column (Σ_i A_ij·y_i + w_j ≥ c_j), and the cost Σ rhs·y + Σ u·w,
  minimised. D*, its optimal face, is the dual's points at the strong-duality row `duality:face:0`: the cost equals V0.
  The least cost, solved and checked exactly, must be V0 exactly. Every LP on D* is solved on its
  complementary-slackness form, which needs no strong-duality row (below; the selection too since PRICE-2e), and every
  solution there is checked exactly against every row and bound of that form; the joint vector also at the
  strong-duality row, exactly. Any of these failing is `lp_numeric_check_failed` (stage `dual`); it blocks the steady
  state, so that no price is exportable without the whole vector (§2.6), and with it the plan.
- **The projection.** A pool's coordinate π_r is the mean over the cycle of its balance rows' duals (the average
  price: an activity's value is linear in the prices, so its value at the mean is the mean of its values per step).
  Its `interval` is π_r's least and greatest value on D*, an unbounded end null: these replace the re-solves as the
  price's interval.
- **How the values on D* are found** (PRICE-2c; every result is the same, exactly, only fewer and smaller LPs are
  solved):
  - *Proved by S3's basis.* One solve with S3's certified basis, factorised once for all of them (PRICE-2d), gives how
    its basic variables move when the step brings more or less income. Where one grain more (or less) leaves them
    within their bounds, the basis stays optimal (its dual feasibility does not read the right sides), so the optimum
    moves by exactly one grain times the balance row's dual in that basis (and for a resource with a hard cap, whose
    one grain more also moves an overflow row, times the sum of the two rows' duals; PRICE-2d): that chord needs no
    re-solve. For a resource without a hard cap, where a little more (or less) does, the optimum is linear on that
    side with that slope, its derivative there: the projection's `low` (or `high`) needs no LP.
  - *Pinned by the chords.* With f(δ) the optimum when a step brings δ more of a resource (its balance row's right
    side raised by δ, nothing else), f is concave; its right and left derivatives are the projection's `low` and
    `high`, and its chords enclose them: (f(g) − f(0))/g ≤ `low` ≤ `high` ≤ (f(0) − f(−g))/g. So when the re-solves at
    plus and minus one grain (`grain`) both exist and agree, the projection is exactly that value and no LP on D* is
    solved for it. A resource with a hard cap never takes this way (one grain more also raises what may overflow, a
    second right side).
  - *Constant on D*.* A pool whose projection is exact in every step has its one mean as its `interval`, and an
    activity whose value reads only such pools has its one value as its `range`: no LP.
  - *D* in the complementary-slackness form.* The other projections, intervals and ranges, and the forcing costs, are
    solved on the optimal face written without the strong-duality row: the dual's rows and variables with, for an
    exact optimum x* of the primal, the dual of each at-most row x* leaves slack fixed at 0, the `w` of each variable
    below its upper bound fixed at 0, and the dual row of each variable above 0 an equality. A dual feasible point is
    optimal exactly when it is complementary to one primal optimum, so this is the face itself; HiGHS solves on it far
    faster than on the face with its dense cost row (on a large model, one such LP took minutes). x* is S3's optimum
    (exactly optimal once the strong-duality check has found a dual point of its cost), and for a forcing cost an
    optimum of S3 that runs the activity its `most` (its `most` probe's solution, or S3's when complementary
    slackness gave `most` 0). Since PRICE-2e the selection and its exact check are solved on it too, with the rows that
    fix the coordinates so far: on a large model, the face with its strong-duality row held one withheld pool's LP of
    the selection for over 30 minutes. In the dual's own terms these are its complementary-slackness conditions with
    x*, the dual of its dual: a dual variable whose reduced cost (its primal row's slack b_i − a_i·x*, or for a `w` its
    bound's slack u_j − x*_j) is above 0 sits at 0, and a dual row whose own dual x*_j is above 0 is tight.
  - *S3's optimal face* (PRICE-2e), where `most` and the ties are probed, is written the same way from the dual side,
    without the row V ≥ V0 (the dense row of every term of the objective): with the exact optimal dual point (y*, w*)
    of the strong-duality check, each column whose dual slack s_j = (Aᵀy* + w* − c)_j is above 0 fixed at its lower
    bound 0, each variable with an upper bound whose `w`, that bound's dual, is above 0 fixed at its upper bound, and
    each at-most row whose dual y_i is above 0 an equality. For any feasible x, c·x = b·y* + u·w* − Σ_i y_i·(b_i −
    a_i·x) − Σ_j w_j·(u_j − x_j) − Σ_j s_j·x_j (the sum over the at-most rows; an equality's slack is 0 whatever its
    free dual), and b·y* + u·w* = V0; each product is of two factors at least 0, so x is optimal exactly when every
    one is 0: the fixed bounds and the equalities above. In reduced-cost terms (d_j = c_j − (Aᵀy*)_j = w_j − s_j, a
    maximum), a variable whose reduced cost is not 0 sits at the bound it names: below 0 at its lower bound, above 0 at
    its upper. A bounded variable with both duals above 0 would be asked to sit at 0 and at its upper bound; S3's
    optimum meets every condition, so that happens only when the upper bound is 0. The same set as with the row V ≥ V0,
    so every probe's value is the same.
  - *Warm starts.* Consecutive LPs on the same rows that differ only in their objective (those above) are given to
    the HiGHS instance that solved the previous one, with the objective changed: HiGHS's primal simplex goes on from
    the basis it ended at, without presolve. Since PRICE-2e each face has its instance (D*, and S3's optimal face for
    the `most` probes, whose LPs alternate with the ranges); the selection adds a row per coordinate it fixes, given to
    the instance as a new basic row, and fixes each pool at the extreme the last LP found, which its solution meets;
    and the first `most` probe, and each tie probe that S3's optimum meets, starts from S3's basis (S3's vertex lies on
    the face). Only where HiGHS starts differs; each LP's exact vertex and certificate are its own ("Exactness and
    failures").
- **The selection**, each step an exact LP on D* and what is fixed so far, in this order:
  1. a pool bounded below only (`high` null: one less is infeasible) that a main goal values (its own `objective` of
     tier `main`): fixed at its least value, the value of one more (its goal's weight), by id; `by` `one_more`;
  2. the lexicographic max-min over K: each pool of a bounded interval (`low` < `high`), at the normalised distance
     min(π − low, high − π) ÷ (high − low) from its ends; and each activity that no optimum of S3 runs and whose value
     f (what it produces less what it consumes, at π) reaches 0 on D* and no higher, from a finite least value m, at
     (0 − f) ÷ (0 − m). A stage maximises the level t that every free coordinate reaches; each coordinate that is at t
     in every optimum of the stage (the most it reaches while the others reach t is t) is fixed at its value there,
     and the next stage goes on with the rest. Each stage fixes at least one. The stages are `max_min`, each `{level,
     fixed}`, the fixed coordinates by id: `price:<pool>` and `activity_value:<activity>`; a pool fixed here has `by`
     `max_min`. So a pool's coordinate is its interval's midpoint when that point is jointly feasible and makes no
     activity outside the optima read 0, and an activity that no optimum runs reads strictly below 0: a value of 0 is
     then only an optimum's marginal activity;
  3. a pool whose two ends agree has that value; `by` `exact`;

  and last, each pool still free, which the selection withholds (`withheld`, below), by id: at its least value on
  the face left (`one_more`), else at its greatest (`one_less`), else, unbounded both ways, at 0 (`zero`). It is free
  through the max-min, so a coordinate that is not exported never moves one that is (the design leaves a pool bounded
  below only and valued by no goal free in the same way). The vector is then checked exactly as a point of D*: an
  exactly checked solution of D*'s complementary-slackness form with every coordinate fixed, whose cost is V0 exactly
  at the strong-duality row. It is independent of the dual point HiGHS returns: each step is an LP's optimal value.
- **`withheld`**: null, or why the selection gives no exportable price:
  - `{"reason": "no_steady_supply"}`: one less unit is infeasible (`high` null) and no main goal values the pool: it
    has no steady supply, a structural gap (§3.1);
  - `{"reason": "no_room", ["regen", "stop"], ["cap"], "excluded"}`: one more unit is infeasible (`low` null): nothing
    in the cycle can take it. Its coordinate is fixed last, at its greatest value on the face left, the value of one
    less unit (finite: one less only loses less), mirroring step 1. It names what bounds the pool: its regen stop (`regen`, the regen; `stop`,
    the cap it stops at) and its hard cap (`cap`), and the pending flows left out that take from it (`excluded`, by
    id). This is the regen stop whose only users are left out as pending (`low` null, `high` 0); the plan still
    refuses a regen stop (`while`), so it never reads a price without a low end.
- **The activity judgements** (`activities`), per activity of the program:
  - `value`: f at the vector; `range`: f's least and greatest value on D* (exact, or `{low, high}` with a null for
    an unbounded end); both null for an activity that moves out of or into a store (what it moves depends on what the
    store holds);
  - `most`: the most an optimum of S3 runs it over the cycle (null when unbounded); `in_optimum`: whether that is
    above 0;
  - `forcing_cost`: the loss per execution of running it more than `most`, the greatest dual of the row "its
    executions over the cycle at least `most`" on the optimal face of S3 with that row; `input_bound`: true when that
    dual is unbounded, so nothing is left in the cycle to run it again (`forcing_cost` null; a matter of timing only).
    An unbounded `most` costs nothing more: 0. A positive cost means one more execution crowds out a better use or
    makes what is already saturated: such an activity is no zero-band realization (addendum 2.1 §7).
  - `most` needs no probe for an activity whose dual slack at the exact optimal dual point of S3 is above 0 in every
    step: by complementary slackness (see "The objective", the three solves) no optimum runs it, so `most` is 0 and
    `in_optimum` false (PRICE-2c);
  - `unverified` (PRICE-2c): the judgements whose probe could not be verified exactly, in this order: `most`, `range`,
    `forcing_cost` (empty when every one is). `most` needs a certified optimum of its LP, and `range` and `forcing_cost`
    a certified optimum or HiGHS's unbounded status of each of theirs ("Exactness and failures"). An unverified
    judgement is null, never guessed: `most` with `in_optimum`, `range`, and `forcing_cost` with `input_bound`;
    `forcing_cost` reads `most`, so it is unverified with it. Each unverified probe is the warning
    `lp_probe_unverified` (`probe` `most`, `range` or `forcing_cost`; none of its own for a `forcing_cost` that only
    `most` leaves unverified). An activity whose `most` or `range` is unverified is left out of K (whether an optimum
    runs it is not known); its component counts as a structural gap (`probe_unverified`, see "Coverage"). The joint
    price vector itself is never relaxed: the dual, the projections and the selection still block when they fail.
- **The graph outputs** `price:<resource>` and the diagnostic `activity_value:<activity>` (#424 ACM-PRICE 4.1): each
  resource's and each activity's entry of `joint`, a price with its own `status` and `reasons` (its entry of
  `price_status`: its static component's, see "Coverage" below), an activity's value with the steady state's
  `status`, and `per_account` false. `explain` prints their provenance; nothing binds or exports an activity's value.
  They are blocked exactly when the steady state is. `out/prices.json` and the acceptances of `price:` are PRICE-4's.

### Coverage (`coverage`; #424 ACM-PRICE addendum 2.1 §3.1–§3.3 and §3.7, the design's §4.6; PRICE-2b-2)

The coverage section of the steady state lets the research channel and Alice read whether a game's business coverage is
complete (addendum 2.1 §3.6): no structural gap, every pool a business rule touches covered or verified zero, and every
estimate accepted (that last part, and the report in `out/prices.json` and `bindings check`'s `data.coverage`, are
PRICE-4's). The code is `src/acmodel/check/coverage.py` (the static judgement, which `acmodel check` reads too) and
`src/acmodel/core/coverage.py` (the section).

**The static judgement.** From the rules and the data only: no value is computed and nothing is solved. An amount counts
unless it is written as the number 0 (`"0"`); what any other expression comes to only the run knows, so it counts.
- **Uses.** An activity, the subject of an active flow that no gate of origin `person` keeps out (real payment, never in
  the economy), takes from what it consumes and what its moves take from, and brings in what it produces and what its
  moves bring into.
- **Covered** (§3.1): a pool reaches a goal sink through activities when any one holds: it has its own goal term (a main
  objective; `goal`); it has a place in the costs table (a demand row of the secondary objective takes a non-zero amount
  of it; `costs`); or an activity takes it and brings in a covered pool other than itself (`use`). A pool declared zero
  (`no_use`) never covers what leads to it: a use that leads only to declared zeros is no use (§3.2).
- **No steady supply** (§3.3): a pool that an activity or a demand row takes, that no regen (of an amount not written as
  0 or less) and no activity brings in, and that has no goal term of its own: one grain less of it would be infeasible.
- **Static components** (the design's §4.6): the connected components of the flow graph. Its nodes are the resources,
  stores and activities, and the goal sinks: each main objective, and the secondary objective. Its edges are the amounts
  of the flows (each activity to what it takes from and brings in), the activities a cap blocks (`block_income_from`) or
  counts (an income cap's `from`), a limit's `shared_with`, each main objective to its resource and the secondary
  objective to the resources its demand rows take. Each rule the economy reads belongs to the node it is about: a flow
  and its gates to the activity, a regen, cap or limit to its subject, an objective and the rows of its tables to its
  sink. A component is named by its first member in id order.

**Structural gaps** (§3.3). A component that holds one is a structural gap as a whole, for each it holds:
`excluded_pending` (an activity the steady state left out, see "Pending flows"), `no_steady_supply` (a pool without a
steady supply), `objective_tier_tie` (an activity of the warning `objective_tier_tie`) and `probe_unverified` (an
activity of the warning `lp_probe_unverified`: whether it is a tie, or a judgement of it, could not be verified exactly,
PRICE-2c; conservative, never read as no tie). Its prices are computed and
shown, provisional, and are not exportable (PRICE-4 writes `pending`). A component with no edge to a gap keeps its own
class and status. Two gaps of §3.3 have no place here: an `objective_tier_conflict` blocks the whole steady state (and
with it this section), and a pool brought only within an activity window cannot be written yet (no flow is tied to a
window before ACM-PRICE-E), so it is not in the model.

**The status of a price** is that of its static component: the worst status of the items of the component the steady
state read (amounts, regen members, caps and limits, objective members, demand and costs rows, gates), each with the
status of everything its value read. It is provisional (`inputs`) when one of them is provisional or depends on an
unknown it leaves unresolved, or when a rule of the component, or a rule its expressions reach, names an unknown in its
`unknowns`, whatever its own status; and when the component is a structural gap. A pending flow the steady state left
out counts as `excluded_pending` only. So two components with no edge between them have their own statuses: one may be
confirmed while the other is provisional.

**The class of a pool** (§3.1), each resource that is not a slot:

| `class` | When | `reasons` |
|---|---|---|
| `verified_zero` | declared zero, and the static check found the declaration sound (`no_use_contradicted`, `no_use_condition_incomplete`) | `no_use`; `no_use` names the rule, and `status` is its status |
| `structural_gap` | otherwise, when its component is a structural gap | the component's gaps |
| `covered` | otherwise, when it is covered; `status` provisional means covered with estimates | its grounds: `goal`, `costs`, `use` |
| `unknown` | anything else | `not_covered`, and `zero_uses` when uses of it lead only to declared zeros |

**The section** (`coverage`):
- `pools`: pool to `{class, status, reasons, component}` (and `no_use` for a verified zero);
- `components`: component to `{members, status, gap, gaps, reasons}`, for each component that holds a resource: its
  members by id; `gap`, whether it is a structural gap; `gaps`, each gap it holds to its activities or pools, by id; and
  the reasons of its status, the gaps in the order above, then `inputs`;
- `counts`: the pools of each class, and `with_estimates`, the covered ones whose status is provisional;
- `estimates`: each provisional derive the steady state read, through any node, by id: `{rule, value, estimate}`, its
  point value and its `estimate` as written (null when it has none), so that the research channel can verify the
  estimates of its step 3.

What it leaves to PRICE-4: a covered pool whose price is 0 names the constraint that makes it 0 (an LP fact); a verified
zero is exported only with its acceptance and `scope`; a pool of `schedule.json` that the model lacks is unknown; and a
provisional derive under `estimates` without an `estimate` is `estimate_missing` there.

### The plan (`plan`)

- **One account.** Each resource and store opens at its reading in the state. One that declares no reading needs
  none when nothing can lower or cap its stock: it opens at 0 and is listed in `unread`. Otherwise a missing reading
  is `state_reading_missing`.
- **Its horizon** is `economy.plan_horizon` steps (7). It starts at a reset of every day and week period and in the
  first step of every regen with a longer `every`; a month and a `days` period are relaxed over the horizon (see
  "Time"). A limit's and an income cap's first window is whole, because the state holds no counts yet.
- **Counts are integers**; moved amounts and stocks need not be.
- **Read caps.** A `soft_read` cap blocks, in each step, the collections (moves that take what a store holds) of the
  activities it lists, unless the step opens at least one grain below the cap's reading. So a collection is blocked at
  or above the read cap.
  - An indicator per step does this; the hard cap of the resource bounds it, and a resource without one is refused
    (`unbounded_stock`).
  - Any other income of the listed activities is refused (`income`).
- **Its objective** is the value of what the stocks gain over the horizon, maximised. Each resource counts at the
  steady state's re-solved price (`grain`) for the step the plan ends in: the low end of an interval, the value of one
  more unit of income. A store's stock counts nothing at the end.
- **The MILP** is solved within `search.milp_node_limit` nodes and a relative gap of 0, and never against a time limit.
  - A plan that the limit stops before it is proven optimal is shown with `proven: false`, with the warning
    `search_budget_exhausted`.
  - With no feasible plan at all, the plan is blocked (`lp_solve_failed`, `no_solution`).
- **It needs the steady state.** What blocks the steady state blocks the plan.

### Exactness and failures

- **Exact solutions.** Every solution is made exact before it is used.
  - **An LP's, from HiGHS's basis** (PRICE-2c; `src/acmodel/core/exact.py`). HiGHS is called through the bindings
    scipy bundles (`scipy.optimize._highspy`, the module `linprog` calls, with `linprog`'s model and options), so that
    its column and row basis statuses are read with its solution. The basis is defined by those statuses, never by
    values close to a bound: each nonbasic column is fixed at the bound its status names, each nonbasic row is tight at
    the end its status names, and the basic variables (columns, and the rows' logical variables r = a·x) are solved
    exactly in fractions from the basis system B·z_B = −N·z_N (a sparse elimination), and the row duals from
    Bᵀ·y = c_B (since PRICE-2e from the same elimination, recorded and applied transposed: one system's solution each,
    the same as two eliminations give). The basis is **certified** when it is exactly optimal: every basic variable within its bounds, and the
    reduced cost of every nonbasic one of the sign of a maximum (at most 0 where it could rise, at least 0 where it
    could fall). Its vertex is then the exact optimum, whatever HiGHS's tolerances (1e-7) let pass.
  - **The pivoting fallback.** A basis that is not exactly optimal gets at most `lp.max_pivots` (20) exact simplex
    pivots, each with Bland's rule (the least index enters, and leaves among ties) and every quantity re-solved from the
    new basis: a primal step when the basis is primal feasible, a dual step when it is dual feasible. A basis that is
    neither, singular, invalid (not one basic variable per row, or a nonbasic one at an end that is unbounded), or not
    optimal after the cap is not certified.
  - **A re-solve of S3 at ±grain** (PRICE-2d; "How the re-solves are found"). Its program differs from S3's only in
    some right sides, so its basis is solved by updating S3's factorised basis (product form of the inverse: B'⁻¹ =
    E_k ⋯ E_1·B⁻¹, one eta matrix per variable that enters, each replacing the least leaving one whose entry in its
    column's solve is not 0; none is singular): the basic values E·(z_B + B⁻¹·Δv) and the duals y + B⁻ᵀ·Δu, from the
    change Δv of the nonbasic part of the right side and the change Δu of the basic costs. The certificate is the
    same, exactly. A basis it does not certify, or one too far from S3's for the update to cost less than a new
    elimination (the update stops once it has taken as long as one), takes the pivoting fallback from a new
    elimination. A re-solve HiGHS does not end optimal is a new LP, as before.
  - **Otherwise** (no valid basis, no certificate, and every MILP): the variables HiGHS leaves at a bound, and the
    integer ones rounded, are fixed, and the rows they leave tight are solved exactly for the others; when those rows
    do not determine them, each value is converted with `limit_denominator(lp.max_denominator)` instead. Such a solution
    is feasible once checked, but not certified optimal.
  - The result is checked exactly against every bound, integrality and row; what breaks one is
    `lp_numeric_check_failed`. Objectives, prices and losses come only from these fractions.
  - So is every solution on the steady state's dual, against every row and bound of D*'s complementary-slackness form
    (which holds the dual rows), the joint vector also at the strong-duality row, and the dual's least cost must be V0
    exactly (stage `dual`; "The joint price vector").
  - **Probes** (PRICE-2c): a per-activity probe (a tie, `most`, `range`, `forcing_cost`) whose answer needs an optimum
    reads it only from a certified one; an answer above 0 of a tie probe holds for any exactly feasible point. A probe
    that is not verified, or whose LP fails (`lp_numeric_check_failed`, `lp_solve_failed`), is `lp_probe_unverified`, a
    warning, and never blocks the steady state. Every other solve (S3, S1, S2, the least losses, the price re-solves,
    the dual and every step of the joint price vector) blocks as before when its check fails.
- **Infeasible.** An infeasible program is `lp_infeasible`. It names a minimal set of conflicting groups: each regen,
  cap, limit and read cap is a group, and in a plan each opening stock. They are dropped in turn, in id order, and
  each stays dropped while the rest is still infeasible. A group whose check cannot be decided is kept and listed in
  `unchecked`.
- **Other failures.** An unbounded program, or another status HiGHS ends with after a second try without presolve, is
  `lp_solve_failed`.

### Output

`data` holds `path`, `model_id`, `state`, `run_id` and `work`, as `run` does, and `steady` and `plan`. Each of the
two has `status` and `per_account`; a blocked one has only `status`.
- **`steady`:** `cycle` (`steps`, `step_seconds`), `objective`, `acquisition`, `protected`, `conversions`, `prices`
  (resource to its price per step), `rule_prices`, `activity_values` (activity to its value per step, or null),
  `joint` (`prices`: resource to `{value, interval, by, withheld}`; `max_min`: a list of `{level, fixed}`;
  `activities`: activity to `{value, range, in_optimum, most, forcing_cost, input_bound, unverified}`), `losses` (`overflow`,
  `lost_production`), `grain` (`prices`: resource to its re-solved price per step), `marginals` (`prices`,
  `rule_prices`), `excluded`, `relaxed` (a list of `{rule, per, days, value}`, by rule), `excluded_pending` (a list
  of `{flow, activity, unknowns}`, by flow), `price_status` and `coverage` (see "Coverage").
- **`plan`:** `steps`, `proven`, `objective`, `counts` (activity to its count per step), `moved` (collection to its
  amount per step), `end` (the final stocks), `unread`, `excluded` and `relaxed`.
- With `--steady-only` (`contracts/cli.md`), `data` holds `steady` and no `plan`: the plan is not computed, and none
  of its errors is reported.
- With `--steady-only --quick`, `data` holds `quick`, and neither `steady` nor `plan` (see "The quick look").

A number is a string holding an exact rational. The errors name the outputs they block in `needed_by`:
`economy:steady` and `economy:plan` (the graph outputs `price:` and `activity_value:` are blocked with the steady
state), and with `--quick` `economy:quick`. The warnings are those of the solves.

### The quick look

`economy --steady-only --quick` is **for iteration only**, while a model is being written: it answers what an edit did to
the coverage, V0, A\*, conflicts and ties in a fraction of the time. **Acceptance reads the full run, never this.**

It builds the steady state's program as `--steady-only` does, runs the three solves (S3 with its checked dual, S1, S2 and
the tie probes) and the least losses, and stops before the re-solves at ±grain, the projections, the prices of the
rules, the activities' values and the joint vector.

`data.quick` holds:
- `status`: `quick`. A blocked one holds only `status`, `blocked`, as `steady` does, with the same errors, needed by
  `economy:quick`;
- `objective`, `acquisition`, `protected`, `conversions`, `losses`, `excluded` and `relaxed`, as `steady` writes them;
- `dual_prices`, resource to one exact rational per step: the dual on its balance row at the exact optimal dual point of
  S3 that the strong-duality check found, the same point `steady` reads as `dual`. **This is one optimal dual, neither
  the joint selection (`joint.prices`) nor a price interval (`prices`).** It lies inside that step's interval of `prices`;
  where the interval is wide, which point it is depends on the solver's path, so a change in it alone says nothing;
- `coverage`, as in `steady` (see "Coverage"), from what these solves know;
- `coverage_unknown`, the reasons they cannot rule out: `["probe_unverified"]`. A `probe_unverified` in `coverage` holds
  (from a tie probe); one that is absent may still come from the judgements' probes (`most`, the range, the forcing
  cost) in the full run, and turn a component into a structural gap with its prices provisional.

The warnings are `objective_tier_tie` and the tie probes' `lp_probe_unverified`; the later stages' are not run.

**It can never be taken for a steady state.** It is its own cache node (`economy:quick`) under its own key and its own
run id. It neither fills nor reuses the node `economy:steady`, so a later full run computes that node as if it had never
run. No other node, `explain`, export, binding or acceptance reads it. It has the `economy` kind, so it shares that
kind's evaluator version, and it adds no version: the full run's output is unchanged.

## Outputs: provenance, `explain`, `thresholds.json`, the tables and the scoring documents

### Provenance (`acmodel.provenance.v1`)

The provenance document of one output holds what its value rests on (#424 design 6.4). It is assembled from
what each node of the graph read while it was evaluated. So it names exactly what the value used: one branch
of an `if`, the row a `lookup` found, every row of a set.

| Member | Value |
|---|---|
| `schema_version` | `acmodel.provenance.v1` |
| `model_id` | the model's id |
| `output` | `rule:<id>`, `general_logic:<id>` or `threshold:<id>`; or an output of the economy's steady state, `price:<resource>` or `activity_value:<activity>` (PRICE-2b-1) |
| `result` | the output as `run` shows it (a rule without its `general_logic`, which is an output of its own) |
| `versions` | `rules`: the sha256 over `model.json`, `sources.json` and the rule files; `data`: over the data files; `tool`: the tool version. Each file counts as `[path, document]`, sorted by path. A hash is over canonical JSON: sorted keys, no whitespace, UTF-8. |
| `nodes` | node id to node; the tree is rooted at `output` |
| [`acceptance`] | only for a provisional output that `acceptances` of exports.json accepts: `file` (`exports.json`), `ref` (its entry, `/acceptances/<index>`), `sha256` of the entry and its `accept_provisional` |

An acceptance that names no output of the model is `rule_reference_missing` (`expected` `output`), and a second
one of the same output `model_id_duplicate` (`namespace` `acceptance`): `explain` and every export refuse them.

**Nodes.**
- Every node has `node` (what it is) and `reads`, the sorted ids of the nodes its value read. A leaf reads
  nothing, and a node read twice is listed once.
- Paths are relative to the model folder, with `/`.
- A node of the general logic (`max_single_income`, from the rules and the data only) has `general:` before
  its id and the member `general: true`.
- A node in the reference state, which the economy's steady state reads, has `reference:` before its id and the
  member `reference: true`; a cap with a reading points at its `reference`. Its regens and objectives are such nodes
  too: `reference:regen:<id>:<amount or every>` and `reference:objective:<id>:<value or horizon>`, whose `ref` is that
  member.
- An economy output (`price:<resource>`, `activity_value:<activity>`) reads `economy:steady`, which reads what the
  steady state's program was built from: its reference-state nodes, its data rows and its gates (`entry:<id>`). In
  this version an economy output's tree is the whole steady state's.

| `node` | Ids | Members |
|---|---|---|
| `threshold` | `threshold:<id>` | `id`, `form`, [`round`], `file` (`exports.json`), `ref`, `sha256` of its entry |
| `price`, `activity_value` | `price:<resource>`, `activity_value:<activity>` | `resource` or `activity` |
| `economy` | `economy:steady` | `solve` (`steady`), `file` (`model.json`) |
| `rule` | `rule:<id>` (its value), `general_logic:<id>`, `offset:<id>` (`live.offset`); the flow amounts `flow:<id>:consumes:<resource>`, `flow:<id>:produces:<resource>` and `flow:<id>:moves`; `entry:<id>`, a rule consulted for its logic only (the cap of `max_single_income`, and an income whose amount needs the account state, which its store's storage cap bounds) | `kind`, `id`, `file`, `ref` (the pointer of the expression, or of a cap's `reading`, or a random rule's form; for `entry:` the rule's), `sha256` of the rule entry, and its `status` and `provenance` as written; a random rule whose value the run carries as its enclosure adds `exact_values`, `[{"unknowns", "value"}]`, the exact value per scenario |
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
- For `price:<resource>` and `activity_value:<activity>` it also computes the economy's steady state, without the
  cache, with the keys under `lp` and `economy`; its `result` is the output as "The joint price vector" shows it.
- An id that is not an output of the model is `usage_invalid` (`output_unknown`): for an economy output, an id that
  names no resource (no activity) of the model, or none of the steady state (an activity it leaves out).
- A blocked output has no tree. The errors that block it are reported, with exit status 1 (for an economy output,
  those of the steady state, each needed by the output).

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
- `provisional`: not confirmed, and not accepted (see "The export policy"). An accepted provisional threshold is
  written with its `status` `provisional`, and its provenance file records the acceptance.
- `not_integer` or `out_of_range`: its exact value is not an integer (`exports.json` can declare its `round`),
  or lies beyond ±(2<sup>53</sup> − 1). A confirmed threshold that read unknowns has a value or an
  enclosure per scenario, all within one value at export precision: one integer throughout is exported, and
  anything else is refused as `not_integer`, with a value that is not an integer, or an enclosure as
  `<low>..<high>`.
- `not_single`: an accepted provisional threshold whose scenarios are several integers (`<lowest>..<highest>`);
  none of them is picked.

**Writing.**
- When any threshold is refused, nothing is written.
- A write that fails is `export_write_failed`.
- The files have sorted keys, a two-space indent, UTF-8 and LF line ends.
- Nothing is deleted, not even the provenance file of a threshold that `exports.json` no longer lists.

### The export policy

What a delivered file takes from an output (#424 design 6.6; `src/acmodel/export/policy.py`):

| Output | Threshold | Table cell | Scoring placeholder |
|---|---|---|---|
| confirmed | its value | its value | its value |
| provisional | refused unless accepted | `null` (an unresolved estimate) unless accepted | refused unless accepted |
| blocked | refused | refused | refused |
| per-account (it read the account state) | refused | refused | refused |

- **Acceptances.** An entry of `acceptances` in exports.json accepts one provisional output (for example a value
  marked as provisional until a device check): it is delivered, and its provenance file records the acceptance.
- **Numbers of the tables and the scoring documents.** The output's exact value times the declared `scale`,
  rounded by the declared `round`: `floor`, `ceil` or `half_even` (to the nearest, a tie to the even integer).
  Every scenario of the output, each variant and both ends of an enclosure, must give the same integer, and then
  the true value gives it too (rounding never decreases). Otherwise the value is refused (`not_single`, with
  `<lowest>..<highest>`); an integer beyond ±(2<sup>53</sup> − 1) is `out_of_range`.
- **Booleans** must be one value in every scenario; an undecided one is `not_single` (`false..true`).
- **Provenance files.** Each output a file delivers has its provenance document, as `explain` prints it, in
  `out/provenance/<kind>/<id>.json` of the model folder: `rules/` for `rule:<id>`, `general_logic/` and
  `thresholds/`.

A refusal is `export_refused` at the threshold, cell or placeholder, with the errors that block a blocked
output; the export then writes nothing.

### `acmodel export tables <model> --state <file> --repo <repository>`

It writes the tables of exports.json into the working tree of the resource repository `--repo`, and the
Runtime's own validator checks each of them (#424 design 6.1, design-py 6.1; `src/acmodel/export/tables.py`).
1. **The validator.** `paths.actinglab` (`contracts/cli.md`) must name it; otherwise the export is
   `export_validator_missing` (`not_configured`, exit 5), and nothing is computed or written.
2. **The run.** The model is evaluated for the state as `run` does, without the cache. A table id given twice
   (`model_id_duplicate`, `namespace` `export_table`), a `valid_until` that names no window
   (`rule_reference_missing`, `expected` `window`), a cell that names no output (`expected` `output`) or an
   output of the wrong type for its column (`expr_dimension_mismatch`, `operator` `table`) refuse the export.
3. **The cells** follow the export policy. A refused cell refuses the whole export, and nothing is written.
4. **The files.**
   - `<repository>/tables/<table_id>.json`: `{schema_version: "actingcommand.table.v1", game, table_id,
     row_key, columns, rows}`. Each column keeps `name`, `type`, `min`, `max`, `allowed` and `nullable` as
     declared, and a `valid_until` becomes `valid_until_unix_s`, the end of its window. Each row is the list of
     its cells in the order of the columns.
   - The provenance file of each output a cell delivers (not of a `null`).

   A table is canonical JSON (JCS, `src/acmodel/export/jcs.py`): object keys sorted by their UTF-16 code units
   (not by code point: the two differ above U+FFFF), no whitespace, minimal escapes, UTF-8, no final newline;
   numbers only as integers within ±(2<sup>53</sup> − 1), never a float.
5. **Validation.** For each table written:
   `<paths.actinglab> --json resource validate --repo <repository> --changed-path tables/<table_id>.json`. It
   passes with exit 0 and the answer's entry for the file `valid`. A refusal is `export_table_invalid`, with
   the validator's `standard_pack_problem` in `problem` and its JSON pointer in the table as the error's `ref`.
   A validator that cannot be started, runs past `export.validator_timeout_ms` or answers otherwise is
   `export_validator_missing` (`start_failed`, `timeout`, `answer_invalid`).

**One change.** When a write fails (`export_write_failed`) or a table is not accepted, every file the export
wrote gets its previous bytes back: a file that was not there is removed again, and so is a folder the export
created. The export never commits, and deletes nothing else, not even a table that exports.json no longer lists.

**`data`** holds `path`, `model_id`, `state`, `repo`, `tables` (each `table_id`, `file`, `nulled`: the cells
written as `null`, each `{row, column, output}`, and `accepted`: the outputs delivered under an acceptance) and
`written`, every file written.

### `acmodel export scoring <model> --state <file>`

It fills the scoring templates of exports.json (#424 design 6.2; `src/acmodel/export/scoring.py`).
- **A template** (`scoring.template.json`, say) is the resource side's: gates, terms, `on_unknown`, tie-breaks.
  Each number the model gives it is a placeholder, `{"$model": "<output id>", "scale": <integer ≥ 1>,
  "round": "floor" | "ceil" | "half_even"}`; nothing else of the template changes.
- **Reading.** A template is strict JSON ("Reading JSON"), and a placeholder holds exactly those members
  (`model_file_invalid`, `format` `scoring.template`). A scoring id given twice is `model_id_duplicate`
  (`namespace` `scoring`); a placeholder that names no output, or a boolean one, is `rule_reference_missing`
  (`expected` `output`) or `expr_dimension_mismatch` (`operator` `scoring`).
- **Filling.** Each placeholder becomes its output's number by the export policy. One refused placeholder
  (`export_refused` at the placeholder: blocked, per-account, provisional and not accepted, `not_single`,
  `out_of_range`) refuses the whole document: a scoring document is never partly filled.
- **Writing.** Where `scoring.json` lives and which validator family reads it are not decided yet (367-D1). Until
  they are, a template that fills is refused as `export_refused` (`validator_path_unknown`, `output`
  `scoring:<id>`), and nothing is written (exit 5).

## Bindings: `bindings.json`, `bindings check` and `bindings apply`

A binding ties a literal of the resource repository to a threshold of `exports.json` (#424 design 6.3), so that
a number in `schedule.json` or in a pack file has the threshold's provenance. `bindings.json` sits beside
`model.json`. Its file names are relative to the repository, the folder two above the model folder
(`<repo>/ours/model`).

### `bindings.json` (`acmodel.bindings.v1`)

| Member | Value |
|---|---|
| `bindings` | a list of `{threshold, file, path}` [`rule_id` or `pool_id`, `note`]: `threshold` is the id of a threshold of `exports.json`, `file` the file that holds the literal and `path` its JSON pointer |
| `fixed` | a list of `{file, path, reason, ref}` [`rule_id` or `pool_id`, `note`]: a literal that is no quantity, with the reason and a reference (a ruling, a board item) |
| [`note`] | |

- **`file`** is relative to the repository: `/`-separated segments of `[A-Za-z0-9_-][A-Za-z0-9_.-]*`, ending
  in `.json`.
- **`rule_id` or `pool_id`**: exactly one of them is there when `file` is `schedule.json` (the game root's
  schedule; otherwise `member_missing` or `member_conflict`, with `expected` naming both), and neither for any
  other file (`member_unknown`).
  - `rule_id` is the `id` of a rule of its `rules`, and `path` is then relative to that rule.
  - `pool_id` is the `id` of a pool of its `pools`, and `path` is then relative to that pool, for example
    `/projection/per_ms` or `/capacity`.
  - For any other file, `path` points into the whole file.
- **One literal**, by its file, rule or pool, and pointer, is named once among the bindings and the fixed
  entries (`model_id_duplicate`, `namespace` `literal`).
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
- Of `schedule.json` it needs `schema_version` `actingcommand.schedule.v1`, and `rules` and `pools` (each
  absent means none) that are objects with distinct string `id`s.

### Where a literal may sit

| Threshold | Its literal |
|---|---|
| `constant` | any JSON integer, except the offset of a fact reference that is the `value` of a `fact` predicate. The offset of a fact reference elsewhere (a `resource_projection` predicate's `value`, a pool's `capacity`, #386 CF-5) takes a constant. |
| `fact_offset` | the `offset` of a fact reference `{"type": "fact", "fact_key": B, "offset": N}` that is the `value` of a `fact` predicate `{"kind": "fact", "fact_key": A, "comparison": …, "value": …}`. A must be the threshold's `fact`, and B its `reference_fact`. |

The comparison is not compared: a rule may test the other side of a threshold (`less_than` where the threshold
says `greater_than_or_equal`).

### `acmodel bindings check <model> --state <file> [--strict]`

- **Each bound literal** equals its threshold's value (`constant`) or offset (`fact_offset`). Otherwise it is
  `binding_mismatch` (exit 6), with `reason`:
  - `value`, with `expected` and `found`, at the literal;
  - `fact_key` or `reference_fact`, with `expected` and `found`, at the fact key;
  - `form`, at a literal that cannot hold the threshold's form;
  - `literal_missing`, when the rule, the pool or the pointer names no integer. Fixed entries are checked for
    this too.
  - `slot`, when `pool_id` is a slot resource of the model (a resource with an active hard cap whose `value` is
    0; see "Slots" under "The economy", #424 ACM-PRICE 3.1), at the pool (`/pools`, when no pool has that id). A
    slot is no Runtime pool and its price is a diagnosis only, so neither a binding nor a fixed entry may name it.
    `bindings apply` refuses it the same way.
- **`--strict`** also reads `schedule.json`. It refuses every integer literal that a `fact` or
  `resource_projection` predicate of one of its rules compares with, and every quantity of one of its pools,
  when the literal is neither bound nor fixed (`binding_unbound_literal`, exit 6, with `rule_id` or `pool_id`).
  So no number without provenance reaches the schedule.
  - These literals are, of a `fact` predicate, its typed value (`value/value`) or its fact reference's offset
    (`value/offset`); of a `resource_projection` predicate, its `value` or its fact reference's offset
    (`value/offset`, #386 CF-5).
  - The quantities of a pool (#424 ACM-POOL) are its `capacity`, or its fact reference's offset
    (`capacity/offset`, #386 CF-5), and its projection's `projection/amount` and `projection/per_ms` (a
    projection of kind `none` has neither). A binding or a fixed entry names them by `pool_id`.
  - Other literals are not scanned: `max_age_ms`, counts and budgets, a pool's `value_source`, `group_delay` and
    `valuation`, and the pack files. They are bound when a threshold covers them.
  - The errors come in document order, the rules' before the pools'.
- **It writes nothing.** Every problem is reported at once, and the exit status is the first error's.
- **`data`** holds `path`, `model_id`, `state`, `repo` and `strict`, and:
  - `bindings`: each with `entry` (its pointer in `bindings.json`), `threshold`, `form`, `file`, [`rule_id` or
    `pool_id`], `path` and `literal`;
  - `fixed`: each with `entry`, `file`, [`rule_id` or `pool_id`], `path`, `literal`, `reason` and `ref`;
  - with `--strict`, `literals`: how many literals it scanned, those of the rules' predicates and the pools'
    quantities.
- **The provenance classes** of `acmodel check` (warnings, or errors under `check --strict-provenance`) are not
  read here: `--strict` concerns the literals of the schedule only.

### `acmodel bindings apply <model> --state <file>`

- **It writes each bound literal that differs from its threshold.** Only the literal's bytes change. A
  position-tracking scanner finds them, so line ends (CRLF stays CRLF), spacing and every other byte stay as
  they are.
- **It refuses everything `check` refuses but a differing value**, and then writes nothing. It does not scan
  for unbound literals; `check --strict` does that afterwards.
- **A file that cannot be written** is `export_write_failed`. The files written before it stay written, and
  `apply` can run again.
- **`data`** holds `path`, `model_id`, `state`, `repo`, and:
  - `changed`: each with `entry`, `threshold`, `file`, [`rule_id` or `pool_id`], `path`, `from` and `to`;
  - `written`: the files whose bytes changed.

## The search

The generic search (`src/acmodel/core/search.py`; #424 design 4.6, design-py 4.6) fills teams from a roster of
entities, with an engine through the engine host (see "Engines" below). No command runs it yet: a command builds
its problem and calls it. Its configuration keys are under `search` (`contracts/cli.md`).

### The problem

- **Entities:** opaque ids, each with tags, which only the legality rules read, and data (a JSON object), which
  only the engine reads.
- **Teams**, in order, each with a `context` (a JSON object for the engine), and:
  - **slots**, in order: a slot takes `count` distinct entities, each with every tag of its `requires` and none of
    its `excludes`;
  - **quotas:** at least `low` and at most `high` (if given) of the team's members have the quota's tag.
- **Disjoint:** an entity is a member of at most one team.
- **The metric:** the `evaluate` metric to maximise. A team's value is that metric of its members, exact (the exact
  value of the number, never rounded); an assignment's value is the sum of its teams' values.

A team's **row** is its members slot by slot, each slot's members in id order. A problem is built by the core, so a
malformed one is a defect of the tool, not an input.

### What the search asks the engine

| Op | Requests | Result |
|---|---|---|
| `predict` (optional) | one per team: `team`, `context`, `slots` (each `slot`, `count`, and `eligible`: the ids that may fill it) and `entities` (every eligible entity) | `additive` (every entity of the request to a number), `pairwise` (optional: an id to an object of later ids, in id order, to a number; an absent pair is 0) and `depends_on`, and nothing else |
| `evaluate` | one per team row: `team`, `context`, `slots` (each `slot` and its `members`) and `entities` (the members) | an `evaluate` result whose `metrics` hold the metric |

Another predict result, or an evaluate result without the metric, is `engine_protocol_error` (`result_invalid`, op
`predict` or `evaluate`, `request` its index in the call), with `ref` the pointer of the problem. The engine host's
errors pass through. Any of them ends the search.

### Steps

1. **Screen.** Every legal row of each team is enumerated with numpy: the product of the slots' combinations, in
   chunks of `search.screen_chunk_rows` rows, less the rows that repeat an entity or break a quota. Each row is
   scored with the predict terms, each rounded half to even at `search.screen_scale` to a 64-bit integer: the sum of
   its members' additive terms and of its pairs' pairwise terms. The screen only decides which rows are evaluated
   first; every value shown comes from `evaluate`.
2. **Proof mode** (`method` `exact`). When all teams have at most `search.exact_max` legal rows together, every row
   is evaluated, and the best assignment is proven (`proven: true`).
3. **Otherwise** (`method` `heuristic`, `proven: false`; the engine must have `predict`):
   - the first `search.top_k` rows of each team by screen score are evaluated;
   - the best assignment of the evaluated rows is taken. The rows that its local swaps give (a member replaced by an
     entity no team uses; two members of two slots, or of two teams, exchanged) and that are legal and not evaluated
     yet are evaluated, and the best assignment is taken again, until its swaps give no new row;
   - while the evaluated rows hold no assignment of all the teams, the next `search.top_k` rows of each team are
     evaluated.

The **best assignment** of one team is its best evaluated row; of two or more, a depth-first search over their
evaluated rows, best first, bounded by the best values of the teams left.

**Ties** break by id, then index. Of two rows or assignments of equal value, the first is the one whose members,
team by team and each team's in id order, come first in id order (ids compare by code point: `a10` before `a9`);
then the one whose members, slot by slot, come first. The screen orders equal scores the same way. So no result
depends on the enumeration, the chunks or the number of workers.

**Budget.** `search.node_limit` bounds the team rows one search evaluates, and the nodes of each assignment of two
or more teams; never a time. A search that reaches it ends with the best assignment it found, `proven: false`, and
the warning `search_budget_exhausted` (`solve` `search`, `phase` `evaluate` or `assign`).

### Warm start

A search leaves a state: the roster, the rounded predict terms, and every screened row with its scores. A later
search of the same teams (the same slots and quotas, and the same `search.screen_scale`), given that state, screens
only the rows that hold an entity that is new or gone, or whose tags or terms changed; a changed pairwise term
changes both its entities. The other rows keep their scores. Its result is the cold search's: the same rows with the
same scores, evaluated alike. `start` is `warm`, and `work.screened` counts only the rows it screened.

### The approximation loss

The additive pick is what the additive terms alone would choose: each team in order takes its legal row with the
largest sum of additive terms among the entities the earlier teams left (ties as above). Its rows are evaluated
first. The loss is the best value less the pick's value, and `relative` is the loss over the best value, when that
is above 0. The loss is `exact` when the search is proven; otherwise the optimum may be higher, and the loss is a
lower bound.

### Output

- `method`, `proven`, `start` (`cold` or `warm`);
- `value`: the best assignment's value;
- `teams`: for each team, `team`, `slots` (each `slot` and its `members`) and `value`;
- `space`: the legal rows of all teams;
- `approximation`: null when the engine has no `predict`. Otherwise `additive` (as `teams`; a team's `value` is null
  when the budget left it unevaluated), `value`, `loss`, `relative` and `exact`; all but `exact` are null when the
  additive pick cannot fill every team;
- `work`: `screened` (the rows this search screened) and `evaluated` (the team rows it evaluated).

A value is a string holding an exact rational.

### Failures

`search_failed` (exit 1), by `reason`:
- `no_legal_assignment`: a team has no legal row (`team`), or the disjoint teams have no assignment that fills them
  all;
- `op_missing`: the engine has no `evaluate`, or no `predict` while the legal rows (`space`) are more than
  `search.exact_max`;
- `space_too_large`: the product of a team's slot combinations has more rows (`rows`) than `search.screen_max_rows`;
  nothing is enumerated;
- `screen_overflow`: a team's rounded predict terms could sum beyond the 64-bit integers of the screen;
- `budget_exhausted`: `search.node_limit` was reached (`phase`) before any assignment of all the teams was evaluated.

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
