<p align="right">🌐 <b>English</b> · <a href="./README.zh-CN.md">简体中文</a></p>

# ActingCommand Console

**⚠️ This program is still iterating rapidly; expect it to be complete within 2–5 weeks.**

This is ActingCommand's **human console**, a read-only native program. On one Runtime state root it opens
the ledger's **official read face**, and renders the view pages the ledger itself hands out into a
three-column interface: instance card, timeline, detail pane.

It is not part of the Runtime; it is the Runtime's **external, detachable client**.

## Data source: the read face, not files

The program accepts only one argument: the state root. **All file IO for reading ledger data belongs to
the read face**; the console never assembles paths inside the state root itself, never opens `ledger/`,
`artifacts/` or `runtime-state.sqlite`, and never launches a CLI to read data. The only processes it
starts are actingd itself (**Start**, see "Launcher") and `actingd check-config` when an
instance-configuration save is checked (see "Instance configuration").

There are two read faces, with the same query, page and cursor semantics; they differ only in where the
answers come from:

**Offline** (the original path):

- `GlobalLedger::open_metadata` opens the state root; the ledger itself determines the medium
  (segment / sqlite), authenticates the snapshot, and gives `latest_sequence` and the completeness
  observation. The whole session reads fixed at this one snapshot position.
- `actingcommand_ledger_forensics::query_view_page` produces the official page `RuntimeEventQueryPage`:
  events, the view membership each event carries, the read range, the page cursor, run recovery
  grouping, and artifact eviction facts.
- `actingcommand_ledger_forensics::read_material_complete` reads material as one whole object: the read
  face resolves the reference, takes the shared read protection, re-checks the reference and the
  retention state, and hands over the bytes only after the whole-file sha256 check passes; no prefix is
  ever handed over.

**Online** (via `actingcommand-runtime-client`, the client's only typed IPC path):

- `RuntimeClient::connect(RuntimeClientConfig::new(state_root, Ui, Ui))`: `runtime-info.json` is read by
  the client itself, the loopback address is taken by it, and the owner epoch is checked by it at connect
  time. The read face is only a client: it does not kill or wait for the Runtime, does not touch
  `owner.lock`, and writes nothing into the state root; launching and requesting shutdown belong to the
  top bar's launcher (see the "Launcher" section), and shutdown too goes only through this same typed
  client.
- The first page at startup asks without a snapshot position; the `snapshot_ledger_position` the Runtime
  states on that page is the position this session reads at — the pin. A person's jump to the latest,
  and turning following on, move it by the same kind of fresh first page; while following, each poll
  moves it to the position the Runtime's fact snapshot states (below). Offline it never moves. Every page
  after that is
  `RuntimeClient::query_event_page(query, ProjectionProfile::Ui, page.at_snapshot(pos))`,
  the same `EventQuery` bounded to one window, the same page limit, the same `next_cursor`.
- Material goes through `RuntimeClient::read_material_complete` on the same connection: the client
  reads the ranges itself (192 KiB each, 64 KiB against a Runtime that refuses larger ones), the Runtime
  verifies each against the whole file, and the client checks the assembled length and sha256 — the same
  result shape as the offline whole-object read. The console no longer assembles ranges itself.
- Medium, corrupt tail, writer-process record and the repair count are the offline read face's
  observations of files; the Runtime does not state these on the page. Online, the instance card's
  "storage format" says "determined by the Runtime", the repair count says the Runtime does not state it,
  the event count is the pinned position (the contract states sequences are gap-free from 1, so the count
  at a position is that position), and "writer process" states the connected Runtime itself
  (PID, owner epoch, start time, all out of its own `runtime-info.json`).
- Right after the pin, one `status()` and one `runtime_fact_snapshot()` on the same connection give the
  instance card its "runtime instances" lines. Status is read again only on a person's jump to the
  latest; while following, the task facts alone are read again whenever the pin moves. The first two
  lines state the sequence each read was taken at; both may be past the pinned snapshot, so this is state
  as last read, not state at the pin. By contract the Runtime records the status read itself as one
  observation event (`command.validated`), after the pin and so outside this session's snapshot. Then,
  per instance the status registers: its alias (the instance id in grey), port, lease (leased / takeover
  cooldown / idle, plus the queued request count when there is one), and the instance facts
  `task.game`, `task.server` and `task.page` (the page label the last recognition matched) verbatim, or
  "not recorded". A fact whose id the status does not register gets a row that says so. If either read
  fails, the lines state the Runtime's refusal, the client error and any host failure, one per line;
  the session still opens. Offline, the read face's `runtime_facts_at` replays the fact store at the
  pinned position itself, the same position every page reads at: the lines state that position, that
  there is no lease offline, and each instance's id (short; the full id in grey) with its three facts. A
  ledger with no event yet says so; a replay the read face declines states its reason, and a failed one
  its code, operation, io kind (for an io error) and detail, one per line. The replay runs once as the
  session opens, before the window shows, bounded by a 30-second deadline.

`--source <auto|offline|online>`, default `auto`: if the client can connect to the Runtime the state root
points at, online; otherwise offline. The instance card's first line, "read face", states which one was
chosen and why (`runtime-info.json` absent, or the client error code of the failed connection). With
`online`, a failed connection **exits directly carrying the client's error code**; it does not quietly
switch to offline. Across both read faces, a Runtime that connects but cannot answer the first page is an
error in any mode, with no fallback.

When the offline read face cannot be opened — with `offline`, or with `auto` falling back to it — the
console does not exit: the window opens **unopened**, which is what a fresh install looks like, since the
Runtime creates the ledger on its first start. The instance card then has only its first line, "read
face", which says the ledger could not be opened and states the read face's own `code`, `operation` and
`detail` verbatim, and for an io error its io kind (`LedgerIoKind`: `not_found`, `permission_denied`,
…). The kind, never the localized `detail`, tells "no ledger yet" from "a ledger that cannot be read":
only for `ledger_io` with io kind `not_found` does the line add that the state root has no ledger yet,
most likely because the Runtime has never been started here, and point at the launcher's Start. No ledger
fact is shown, not even a zero: the list says the ledger is not opened instead of standing empty; the
tabs, the filter boxes, the id box and the time slider are off; the module and port boxes and the frame
pane say "ledger not opened". Nothing is asked of the ledger and no material is read. The launcher works
as usual.

Dependencies are pinned to the Runtime's **main** (`Cargo.toml`):

```
rev = "a1e40e091f400d7cde038c777756aac473cffa75"
```

The four crates (contract / ledger / ledger-forensics / runtime-client) share this one rev.
`Cargo.lock` is checked in; CI runs `cargo build --locked --release --workspace` on windows-latest and
ubuntu-latest. The closure contains `rusqlite` (bundled), so both need a C compiler.

## What changed

- **The contract decides the classification**: the six tabs are the contract's six `LedgerView`s. Which
  tab a row belongs to is read from the `views` that event on the page carries. The number on a tab
  **appears only on the current tab** and is that view's **number of loaded rows**; the read face does not
  give a total per view, so no numbers are placed on the other views' tabs.
- **Filtering is a ledger query**: view, severity lower and upper bounds, source module,
  `correlation_`/`request_`/`run_`/`task_`/`instance_` id and time upper bound are assembled into one
  `EventQuery`, and the ledger is queried again at the **same snapshot position**; it does not filter the
  rows already held locally and then call itself a ledger query (the one stated exception is the
  performance monitor, below). An id must be a complete canonical id,
  otherwise it states that a complete identifier is required. The "source module" options are **rebuilt
  from the loaded rows on every load**, and the selected item is resolved by **module name** — the list
  changes, and an index is not a statement that keeps.
- **Instance filtering by port (offline read face only)**: an ADB port is the identity of an emulator
  instance. At startup, at the same snapshot position, the `runtime.instance_bound` facts are read
  **once** via `actingcommand_ledger_forensics::instance_bindings`, yielding **every** `instance_id`
  **ever bound** under each port (in first-binding order). Selecting a port in the "port" dropdown queries
  the ledger again at the same snapshot with that whole group of ids as `EventQuery.instance_ids` — the
  whole group together, never split, never only part of it; "all instances" clears it. The dropdown items
  are in ascending port order and state the port number and the abbreviated instance id of the most recent
  binding, with ` +n` appended when one port has more than one id. Port and `instance_` id are mutually
  exclusive: if both are given, the filter error line states it outright, with no default precedence.
  When the ledger holds no binding facts, the box has only the item "no binding records" and is disabled;
  when there are binding facts but not one of them gives a port (serial-port configurations and the like),
  the box has only "all instances" and is disabled — in both cases the id box can still take an
  `instance_` id directly. When reading the bindings fails, the box holds only the read face's error code,
  and the instance card states that code too; under the online read face the box is disabled and says
  "not supported online". With a port selected, the instance card additionally states the instance alias,
  the port, the source (`physical_device` / `fixture_simulation` verbatim on the lower layer), the number
  of bound ids and the sequence number of the most recent binding; the severity counts and the row count
  still come from the re-queried rows. Rows gain a "port" column: an event with no instance
  association says "host"; one with an association whose id's most recent binding gave HOST:PORT says the
  port number; one with an association but absent from the port table (fixtures, serial-port
  configurations, no binding seen) says the abbreviated instance id — no port is invented.
- **Reading from the latest end**: the ledger query has no descending order, so the timeline reads
  backward windows down from the pinned position, starting at 256 positions. A window that size holds no
  more events than one page's event limit, so one query reads it, every filter applied by the ledger. A
  window that comes back with fewer than 64 events doubles the next one's span, up to 4096 positions; one
  with 128 or more puts it back to 256. A wider window, and any the reply's byte limit splits, is read by
  following the cursor, whole — so one fill can add more rows than it aims for. Online, every page query
  is served on the Runtime's ledger writer (since Runtime `71db072d` it re-verifies only the head, the
  boundary and a new tail, about 15 ms a page at ten thousand events, where it used to read and verify
  the whole ledger), so one fill reads at least one window
  and starts no further window once it has 256 more rows, has read position 1, has read 16 windows or has
  run for one second. Rows are listed
  **newest first**; "read earlier" at the bottom continues below what is loaded. The top bar permanently
  states the positions the loaded windows cover ("read positions A–B"), with "source incomplete" appended
  when a window's page said its read was incomplete.
- **Jump to latest, follow latest (online only)**: "jump to latest" moves the pin to the Runtime's latest
  position (one fresh first page), reads status and facts again (the status read leaves one observation
  event in the ledger), drops any time bound and starts the view over from the new pin. "Follow latest"
  first catches up the same way without the status read, then every 5 seconds reads the Runtime's fact
  snapshot — its in-memory fact store, stated at the ledger's latest position, which costs no ledger read
  and writes nothing. Only when that position has moved does the pin move to it and are the windows
  between the old and new pin read onto the top of the view, with page queries; the loaded rows, the
  selection and a frame being read stay, and the task facts come from that same snapshot. Setting a time
  bound while following stops it. The span's end follows the pin: taken from the newer windows when they
  hold the event at the pin, otherwise one more one-event read. A failed tick shows as "following latest:
  …" beside the view's own error; a failed poll stays until a later tick gets past it, and a failed page
  read also stops the following — a Runtime refusing the query is not asked again every five seconds —
  until it is turned on again.
  Offline there is no running Runtime writing newer events, and both controls are off.
- **The performance monitor's routine events are hidden by default**: they would bury everything else.
  Unless "show performance monitor" is ticked, the performance monitor is picked as the module, or the
  Health tab (made of these events) is open, the query asks the ledger to leave out the two types that are
  `Info` from every writer — `perf.pressure_ended` and `perf.monitor_recovered` (`exclude_event_types`)
  — and the console drops the performance monitor's other events **below Warning**, `perf.summary`
  among them, from each window it reads. `perf.summary` stays with the console because the capacity
  monitor writes it as a warning or an error under disk pressure. The top bar says how many were dropped;
  the two types the ledger leaves out are not counted. The performance monitor stays on the module list
  while any are dropped, so it can still be picked. Its warnings and errors (disk pressure, high
  pressure, stutter, degraded monitoring, a summary under pressure) always show. The row and level
  counts on the card cover the loaded rows only. A Runtime before `51ba5565` does not know `exclude_event_types` and refuses the query while
  events are hidden; ticking "show performance monitor" reads it.
- **Recovery grouping comes from the ledger**: the `run_recovery` carried on the pages inserts a group row
  above each run's newest row, showing the state the ledger gives (recovered / unresolved / unknown),
  the grounds (row N failed, row M recovered) and the gaps; failure rows the ledger judges recovered are
  collapsed under the group row with a "recovered" mark. This is read-time grouping, not a rewrite of
  failure events. Across pages it **merges by run id**: a later page only adds evidence into it, positions
  given by earlier pages are all kept, and "recovered" rows already collapsed are not re-expanded by a
  later page.
- **Time upper bound**: the slider's starting position is the state it represents — the far right is
  "all", and the first drag narrows. The bound and its label follow the handle at once; the view reloads
  once the handle has rested for 0.4 s (a tab switch or "read earlier" meanwhile applies the new bound
  first). Reading only goes down, so with a bound set it must start above every matching event: the
  start is estimated from the committed time span, as if events were even in time, plus two windows of
  slack, and one probe with the view's own query asks for the first matching event above it. None:
  reading starts there. One found: the start moves above it, the step doubling each time, for at most
  three probes within the one-second budget, and otherwise falls back to the pin. The query's own time bound still decides what
  shows; the top bar states the positions actually read.
- **Row types are no longer mirrored**: `acui-rows` re-exports the contract types directly, and only adds
  display functions such as local time, id abbreviation, wire codes and the display-name dictionary.

## Frame material: read, but only what is verified

The ruling has changed: the console **does** load frame bytes, but only through the material read face,
and only within this rule:

- Only the `capture.frame` artifact of **the frame the selected event was taken on or acted on** is read
  (see "Every row on its frame" below), on demand, one at a time.
- Offline, one `read_material_complete` call reads the whole object and verifies its length and sha256
  before any byte is handed over, within a 30-second deadline. Online,
  `RuntimeClient::read_material_complete` does the same over the Runtime's verified ranges, with the same
  deadline (checked between ranges); if any range is not `verified` it stops, and the unfinished assembly
  is dropped.
- The whole-file limit is 8 MiB on both faces, the read's own `max_material_bytes` (the bound the console
  kept while it still assembled 64 KiB ranges itself, 128 of them); an artifact beyond that is refused
  outright, with a statement.
- When an artifact has been evicted, only the eviction facts are shown (disposition, intent/outcome
  position, the position observed up to); the file is not touched.
- On a read failure, the state and safe error code the read face gives are shown.
- Reading is done on a background thread, and every request carries a token; if a slow read returns after
  the selection has changed, it is discarded. **A stale or unverified image is never displayed, at any
  time.**
- Reading is done by one background worker thread, **only one at a time**. Offline the whole-object
  read cannot be stopped midway: a displaced one runs to its end, bounded by the 8 MiB limit and the
  30-second deadline, and its result is discarded. Online the client asks between ranges whether the
  read is still wanted, so a displaced one stops at the next range. A request displaced while it still
  waits for the worker never starts.
- Decoding uses only `image` (with only the `png` feature enabled, version pinned in the workspace's
  `[workspace.dependencies]`). The program decodes only two kinds of image: frames read back this way,
  and its own application icon.

The geometry overlay shares one coordinate system with the frame, sized in this order: the frame extent
the ledger formally states (`frame_extent` of `task.effect_intent`, the frame extent of
`task.geometry_observed`), else `frame_width`/`frame_height` in the payload, else the size the frame's
recognition or effect intent states (see below), else the pixel size the
verified frame decoded to, else the overlays' own extent. The decoded size belongs to the frame request
it came from: reselecting the event or reading earlier keeps it; switching events, clearing, or a
failed read drops it.

### Every row on its frame

A step's events name their frame in `links.frame_id`: the capture, the frame's `artifact.created` /
`artifact.verified`, the recognition, the effect intent. The frame pane shows that frame for any of
them, and for the step's other events through the effect intent that shares their `action_id`. A
physical input (`input.*`) does not carry its frame under the `Ui` profile — its `before_frame_id` is
projected away — so its frame is the one of the last effect intent of its run before it; the sentence
under the frame says which event the frame was taken from. The frame is found among the loaded rows
first; only when they hold no capture of it are the frame's own events read, with one query by frame
id, kept while the pane stays on that frame. A failed or incomplete read is stated in the frame note
under the frame, and the next reload tries again.

On the frame, beside the event's own geometry:

- **The page label** above the frame, beside its title: the page the recognition on this frame matched,
  or that it matched none. On the frame it would cover the page's own header targets.
- **The tap mark**: a ringed dot centred on each point of the step's effect intent (`action.x, y`; a
  swipe or drag has one per point). It replaces the event's own `action` geometry, which would draw the
  same input twice.
- **One sentence** under the frame, per input meant on it: "Step 0 notice_close: recognized
  bluearchive/news, tap (1142, 102)".
- **The recognition target boxes**: the targets the frame's latest recognition evaluated
  (`task.recognition_completed.targets`, those of the matched page or, with no match, of the first
  candidate), each in its `region`: solid green when it passed, dashed amber when it did not, labelled
  with its `target_id` and role. A keyword-only target has no region and is not drawn; the sentence
  counts all of them ("… (3 of 4 targets passed)").

## Running

```
acui [--state-root <state_root>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]
acui --help
```

`--state-root` applies to this run only and overrides `state_root` in the settings file; if neither is
present it prints the usage and exits, guessing no default. `--source` selects the read face (see above).
`--tab` specifies the startup tab (for screenshots and review), and its values are the views' own wire
names. `--lang` applies to **this run only**, overrides the language in the settings file, and is not
written back to the settings file.

The window is resizable: 1400×900 by default, 1100×700 minimum; the middle column stretches with the
window and the two side columns keep a fixed width. The window follows the system DPI; the program sets no
scaling of its own.

## Interface language and text size

Two dropdowns at the right of the top bar; a choice is written into the settings file as soon as it is
made:

- **Text size**: standard / large / extra-large = 1.0 / 1.25 / 1.5. **Takes effect on the spot**. Every
  font size and line height in the interface is one base size meant for a person at normal DPI (list row
  14px, detail pane 13px, subheading 16px, line height 26px) multiplied by this one factor; `app.slint`
  has only this one knob, `Scale.factor`.
- **Language**: 中文 / English. **Takes effect after a restart**, and that sentence is written right next
  to the box. The two language tables are in `crates/acui-app/src/strings.rs` (`ZH` and `EN`); at startup
  the selected one fills the `Strings` global once, and there is no re-layout afterwards — there is no
  gettext, and no swapping of words at runtime.

Two-layer labels: the upper layer is the name for a person to read, and the grey lower layer is the
verbatim form used in the program — the raw `event_type`, module names, ids of every kind,
`payload_schema` and sha256 are never translated. The dictionary is in `crates/acui-rows/src/display.rs`
and covers all 120 `event_type`s and 20 `origin.module`s in the contract; **anything not in the table is
displayed verbatim, not guessed**.

### Settings file

Language and text size are stored under the per-user configuration directory:

```
Windows:  %APPDATA%\ActingCommand\acui.toml
Linux:    $XDG_CONFIG_HOME/ActingCommand/acui.toml (falls back to $HOME/.config/…)
```

```toml
lang = "zh"          # zh | en
text_size = "standard"   # standard | large | extra-large
state_root = 'D:\ActingCommand\state'                    # optional, absolute path
actingd_config = 'D:\ActingCommand\actingd.config.json'  # optional, absolute path
actingd_exe = 'D:\ActingCommand\actingcommand-actingd.exe'   # optional, absolute path
```

It is read once at startup and written once on every dropdown change; on write-back the three path keys
are kept verbatim. **This is the only file the console itself reads and writes**, besides the `instances`
the instance-configuration window saves into `actingd_config` and the actingd start logs the launcher
creates and, after an early exit, reads back (see those sections): it is not inside any
state root, and the state root still belongs entirely to the read face. If the file is absent, unreadable,
or holds an unrecognized value, the defaults (Chinese, standard) are used. The parser is hand-written: one
`key = value` per line, one matching pair of quotes stripped, no escape handling — write Windows paths in
single quotes (TOML literal strings), not as `"D:\\…"`.

## Launcher

The third line of the top bar. On the left is the Runtime state from the last probe, then two buttons (and
last the instance-configuration button, see the next section), and the line below it is the result of the
last Start or Request shutdown press, or why the instance-configuration window did not open.

- **Runtime state**: probed once at startup, and probed again after every Start or Request shutdown
  press. A probe is one `RuntimeClient::connect`: on a connection it says "running · PID · owner epoch"
  (taken from its own `runtime-info.json`, checked against the client's owner epoch); on no connection
  it says "not running" plus the client's error code and operation name, guessing no cause.
- **Start**: probe first; if it is already running it launches nothing, says "already running, not
  launched" and records the press (see below). Otherwise it launches
  `actingcommand-actingd --config <actingd_config>` detached, from `actingd_exe` —
  that is the whole command line. If either of the two keys is missing or is not an absolute path, it
  states which key it is and launches nothing. stdout / stderr both go into a log under the console's
  **own** directory: `%LOCALAPPDATA%\ActingCommand\logs\actingd-<unix_ms>.log` (Linux: the same path under
  `$XDG_STATE_HOME` or `$HOME/.local/state`); the directory is created by the console and is **never
  inside the state root**. On Windows it is launched with `DETACHED_PROCESS`: the daemon does not inherit
  the console program's console, and does not receive its Ctrl+C.
- **Readiness decision**: at most 60 attempts, 500 ms each. Each attempt does `try_wait()` first: if the
  child process has exited it stops and states the exit code, the last line of that log starting with
  `FATAL actingd:` **verbatim** — or that the log could not be read, with the error, or holds no such
  line — and the log path; if it has not exited it
  does one more `connect`, and a connection means ready, stating the PID and owner epoch, and then the
  press is recorded (see below). If this console is reading offline, or its offline face is unopened, it
  appends the sentence "to read online, restart with `--source online`" — it **never quietly switches
  read face mid-session**. If all 60 attempts fail to connect, it says "still not ready" and the last
  client error code. Readiness is **never read from the daemon's output**: the log is read back only
  after an early exit, for that one line, and nothing in it is interpreted but whether it names
  `owner_resource_unconfirmed` (below).
- **Recording the start press**: the press can only be recorded once a Runtime exists, so the order is
  probe → (if needed) launch → readiness decision → record. A press is a person's act, so recording
  opens a new connection as actor `user`, source `ui` (the console's own reads use `ui` / `ui`), opens an
  interaction with `begin_interaction()` and records one `client_action` with
  `record_client_action_receipt` (surface `acui.launcher`, kind `button` with no value, control
  `launcher.start` when this press launched a process, `launcher.start.skipped_running` when it found the
  Runtime already running), whose receipt must bear a terminal; it runs on a worker thread, never on the
  window's event loop. Whether a process was launched is an outcome the ledger records on its own
  (`runtime.started`), not a value of the press. The line appends the sequence number at which it landed, or, if recording
  failed, the Runtime's refusal code (if any) **verbatim** plus the client error code and operation name,
  and the host code and operation when the refusal names one
  — the Runtime is still reported ready / running. If readiness fails there is no connection and **nothing is
  recorded**; the failure is shown only on the line — one of the two launcher actions that can take
  effect without being recorded; the other is an unlock that fails at stage `ledger` or is killed (on
  timeout, or when reading its status fails; below).
- **Unlock owner**: offered only when the fatal line of the last start names `owner_resource_unconfirmed`
  — actingd refused the state root because its last Runtime owner exited with device resources in use or
  unconfirmed. A line below the result line then shows an "Unlock Owner…" button. The first press runs
  nothing: it only shows the statement "the device resources of the last Runtime are released" and a
  "Confirm and Unlock" button. The second press runs, from `actingd_exe` (absolute, as for Start), exactly
  `unlock-owner --config <actingd_config> --actor acui --confirm-resources-released`, on a worker thread,
  with no console window (`CREATE_NO_WINDOW` on Windows: its output is captured, not detached), stdout
  and stderr captured, for at most 180 s — well above the Runtime's own 120 s budget for the ledger stage,
  so a run that would finish is never cut off; past that it is killed and reaped and the line says the
  outcome is unknown. The actor `acui` names the console, never a person: no OS user name reaches the ledger.
  The whole trimmed stdout must be one JSON object with `schema_version`
  `actingcommand.actingd.unlock-owner.v1`. `ok` with exit code 0 states the unlocked owner epoch, the
  disposition before (`in_use` / `unconfirmed`) and the `owner.lock` revision, withdraws the entry and
  presses Start once more, through the same path. `failed` states `error.code`, `error.stage` and
  `journal_appended` **verbatim** (`true` only at stage `ledger`: the unlock is durable and the next
  start takes the epoch over, but its ledger fact is missing). With no JSON, the last `FATAL actingd:`
  line on stderr is shown **verbatim** — an argument error, or an actingd too old to know the command. A
  spawn failure, a timeout, output that does not parse or does not match the contract, and an `ok` with
  a non-zero exit code each have their own line, and none retries the start. After any outcome but an
  unlock the entry is back at its first step; a new start withdraws it, and Start is refused while the
  unlock runs. The console records no client action for it: there is no running Runtime to record
  through, and `unlock-owner` appends its own `cli.command` fact (action `owner.unlock`). It never
  deletes `owner.lock` (Runtime `contracts/actingd-unlock-owner.md`).
- **Request shutdown**: only through the typed client, never killing a process. It opens a new connection
  as actor `user`, source `ui` (the Runtime admits a shutdown request only from a person at the console
  or an operator's CLI — from Runtime `75ed4b3f` on; before, only from the CLI, so the button could never
  succeed), opens an interaction with `begin_interaction()`, first records this button press as a
  `client_action` with `record_client_action_receipt` (surface `acui.launcher`, control
  `request_shutdown`), and only
  after obtaining a receipt bearing terminal does it send `request_shutdown()` — the action lands in the
  ledger first, then the request. If it is refused as `runtime_busy` — the Runtime holds its lifecycle
  admission briefly after another request, such as a status read; it is also refused as busy while a
  lease is active or requests are queued, which these retries will not outlast — it sends `request_shutdown()` again
  on the same interaction a second later, at most 5 attempts in all, the line saying "busy, retry n/5"
  meanwhile; the press is still recorded only once. If accepted, it states the receipt state, the
  request id and the sequence number at which the action landed in the ledger; if refused otherwise
  (owner / governance and the like), or still busy after the 5th attempt, it states the Runtime's
  refusal code **verbatim**, plus the client error code and operation name (and the host code and
  operation when the refusal names one); any other refusal or error
  stops at once. The final line also states how many attempts were sent. Afterwards it probes the state
  once more — the Runtime stops at its own pace, so this glance may still say running.
- **Never kill**: the `Child` handle is used only for `try_wait()`, to see whether it exited early — no
  `kill`, no blocking `wait`, no job object attached; the handle is dropped once the readiness decision
  ends, and the daemon outlives the console.

Pause/resume, start-at-boot, the installer and network downloads are all outside this slice.

## Instance configuration

The 实例配置 / Instance Configuration button opens a second window over the `instances` of
`actingd_config`, the file Start hands to actingd. Each entry is listed with its alias, `instance_id`,
binding as the file states it (`fixture_backend`, which the Runtime takes before any binding key / MuMu
index / MuMu name / ADB serial, shown instead of `host` + `port` when an entry has both / ADB host:port /
no binding key in the file — the Runtime's default address is not restated), `application_id`, capture
and touch backend, and what the session's port map says about that id — bound on a port, bound with the
latest binding outside the port map (serial-configured or with no port; the map does not tell which), or
not bound; read online, with the ledger not opened, or when reading the bindings failed, the row says
that instead, and an entry without a string `instance_id` says there that it cannot be edited. A key
holding JSON `null` reads as absent, in the list and in the form. The file is read again whenever the
window opens and after every save; a reason it cannot be listed takes the count's place, never an empty
list.

- **Discover**: Discover Instances asks the running Runtime, through the typed client
  (`discover_instances()`) on a worker thread, to re-run its provider's MuMu instance discovery; the
  Runtime runs `MuMuManager` on the host, and the client waits up to 25 seconds. The contract admits
  this query only from a person at the console or an operator's CLI, so it goes out on its own
  connection as actor `user`, source `ui` (the enum, not an OS user name), like the launcher's
  presses. It binds nothing and touches no device; by contract the Runtime records an answered query as
  one observation event
  (`command.validated`), and a refusal as `command.rejected` plus `runtime.failed`. The box beside it then
  lists every instance reported: its MuMu index, whether it runs, the alias the running Runtime binds
  to it (if any), its ADB address, the Android version, and its name last; the line states how many,
  the provider version and the query's sequence. Choosing in the box only selects; Use Selected applies:
  an instance neither the file (an entry with its index as `instance_index`, its name as
  `instance_name`, or its ADB port as `port`) nor the running Runtime binds starts a new entry bound by
  that index (the alias and the rest still to fill; its address is left to discovery at startup), while
  one already bound is pointed at and nothing changes. With no Runtime
  running, or a refusal (`instance_discovery_unavailable`, `mumu_manager_version_unsupported`, …), the
  line states the Runtime's code, the client error and any host failure, and no earlier result stays
  pickable.
- **The form**: Add Instance starts a new entry and a click on a row loads that entry. An entry without a
  string `instance_id`, or one whose binding no kind of the form represents — with `fixture_backend`,
  with `serial` set, or with no binding key at all — is listed, but a click on it says why and it cannot
  be saved. There is no delete. `alias` is required; a new entry's `instance_id` is `instance_` + 32
  lowercase hex characters from the OS RNG, and every `instance_id` is shown read-only; the binding is
  exactly one of `instance_index` (MuMu index), `instance_name` (MuMu name), or `host` + `port` (an
  explicit ADB address). `adb_path` is required with `host` + `port` and optional with a MuMu binding,
  whose discovery reports adb; `nemu_app_index` is an optional whole number. The form does not check
  `application_id`, `capture_backend` or `touch_backend`: whether they are needed and valid is decided by
  check-config, which also checks the `nemu_app_index` pairing. Only what needs the MuMu discovery
  result waits until the Runtime starts: the `MuMuManager` version and capabilities, exactly one
  discovered match, a declared `adb_path`, `host` or `port` against the discovered values, and the ADB
  endpoint (Runtime `contracts/actingd-check-config.md`, from `3d5398d6`). Text is trimmed and an empty
  box writes no key; a missing required value or a number that does not parse is stated before anything
  is written.
- **Save**: the file is read again as plain JSON — a missing or relative `actingd_config`, a missing or
  unreadable file, JSON that does not parse, no `instances` array, or an entry that is not an object is
  each stated as such. A new entry is appended; an existing one is found again by its `instance_id`
  (gone from the file, or changed there into one the form cannot save, the save stops and says why) and
  only the keys the form manages change, a changed binding kind removing the other kinds' keys. Every
  other key of the entry and of the file is kept, in its order. The result goes to
  `<config name>.candidate-<pid>` beside it (relative paths inside resolve against that directory) and
  `<actingd_exe> check-config --config <candidate>` runs off the event loop (30 s bound, no console
  window, stdout parsed whole as one `actingcommand.actingd.check-config.v1` report). Only `status: ok`
  with a successful exit renames it over the file; otherwise the candidate is removed, the file stays as
  it was, and the window says why: `error.code` and `stage` verbatim, or a missing or relative
  `actingd_exe`, writing the candidate failing, a spawn failure, no output reader thread, reading the
  child's status failing, the timeout (these three also say whether check-config could be terminated
  and then reaped),
  unreadable or unrecognized output, ok with a non-zero exit, or the rename failing.
- **Effect**: there is no hot reload; a saved entry takes effect when the Runtime restarts. After a save,
  one probe off the event loop says whether a Runtime is running now and points at the launcher's own
  buttons: Request Shutdown, then Start once it has stopped — or, with none running, just Start. The
  window restarts nothing itself.

## Setup wizard acsetup

`crates/acui-setup` is a standalone binary `acsetup.exe` (a Slint window, the same styling and icon as the
console) that installs the release files from the umbrella repository's
[Releases](https://github.com/HS7097/ActingCommand/releases) into a **per-user** installation. It ships
together with the UI repository's Windows build artifact (`acui-windows-<sha>.zip` gains one more file,
`acsetup.exe`). The same program comes in two editions, and a person downloads only one of them: the
online `acsetup.exe`, which fetches the release itself or takes a folder a person filled by hand, and
the offline `acsetup-full-<tag>.exe`, which carries one whole release (see "Offline edition" below). One window,
next only (the instances step can also be skipped), five steps; each page shows where its work stands
the way an installer does, and the full account goes to the install log (see "Progress and the install
log" below):

0. **Location**: the install root only (changeable, default `%LOCALAPPDATA%\Programs\ActingCommand`, no
   administrator needed), the free space on that volume, and whether an installation is already here
   (looking at `runtime\BUILD-MANIFEST.json`, whose commit and `ui\`'s are shown; if there is one, the
   run is an **upgrade**, see below). Next creates the root and the install log. A wizard running from
   `runtime\`, `ui\`, `tools\` or `previous\` of the installation it would upgrade stops here, naming its
   own file: that directory could not move aside; from the root itself or `downloads\` it upgrades as
   usual. The offline edition adds to the free-space line what extracting its release takes.
1. **Install** (**Upgrade** on an installed root, see below), one page from the download to the
   layout; on success the next page follows by itself. By default online. On entering, the umbrella
   [Releases](https://github.com/HS7097/ActingCommand/releases) are asked over HTTPS for one release: the
   newest stable release when there is one (GitHub's `releases/latest`), else the newest pre-release (the
   daily builds); never a draft. Its tag, name, date, kind and size are shown and logged. "安装 / Install" fetches `SHA256SUMS`,
   `MEMBERS.json` and then every other file `SHA256SUMS` lists — nothing else — into
   `<install root>\downloads\<tag>\`, each through a `.part` file renamed once its length is the length
   the release states, with a progress line per tenth for a file of a MiB or more; a file already there is
   fetched again, never trusted, and a failed one's `.part` file is removed. The tag and every file name
   are used only if they are letters, digits, `.`, `-` and `_`, do not start with a dot and are no Windows
   device name. HTTPS only, redirects included; a connection quiet for a minute fails. Ticking **Offline**
   instead takes a folder that already holds one release's files (default `%USERPROFILE%\Downloads`,
   typed in). A failed lookup is stated on the page and in the log, with "重新查询 / Look up again" and
   Offline both open; a failed fetch stops the run. This and the instances step's fetch of a package URL
   are the program's only network code (`ureq`, blocking, rustls with the Mozilla root set compiled in);
   the Runtime has none.
   The offline edition has neither the lookup nor the Offline tick: the page names the release it
   carries — its tag and the two commits of its `MEMBERS.json`, checked at start — and "安装 / Install"
   extracts it into `<install root>\downloads\<tag>\`, each file through a `.part` file whose length and
   sha256 must match before it is renamed (a rename a scanner holds up is retried a few times).
   The folder — fetched, offline or extracted — is then verified and laid out on the same page. It must hold `SHA256SUMS`, `MEMBERS.json`,
   `actingcommand-runtime-<sha>.zip`, `actingcommand-tools-<sha>.zip` and `acui-windows-<sha>.zip`
   (`<sha>` taken from `MEMBERS.json`'s `runtime_sha` / `ui_sha`; the three zips must appear in
   `SHA256SUMS`). `SHA256SUMS` is checked entry by entry; the archives are extracted into the temporary
   directory `.staging-<unix_ms>` under the install root; then, against the `BUILD-MANIFEST.json` each zip
   carries, the source repository, the commit id (equal to the MEMBERS sha), the Runtime's
   `runtime_payload_layout` (`distribution-v1`) and the size and sha256 of every entry in `files[]` are
   checked; a file in the zip that the manifest does not list also counts as a mismatch. Any mismatch
   stops it, worded as "the content differs from what it was at creation" — this is an integrity
   statement, not an authorization tone. Nothing inside the zips is run during verification. Then the
   layout: `runtime\` (the Runtime's entire payload + manifest, with `actingd.config.example.json`
   byte-for-byte verbatim), `ui\` (the console payload + manifest), `tools\` (**only** `actinglab.exe`,
   `actingledger.exe` and `ac_fastdeploy_ppocr.dll`; the other two exes in the tools pack are neither
   installed nor shown). The temporary directory is deleted afterwards.
2. **Options**, the configuration already written. Right after the layout, on the install page and
   inside its do-not-close span, the fresh install is configured with no question asked: the state root
   is `<install root>\state` (it must not exist or must be an empty directory — checked on step 0,
   before anything is fetched; **a state root that already holds content is never taken over**);
   `secret_fingerprint_salt` is generated as the hex of 32 bytes from the system random source
   (`getrandom`; **not displayed, not logged**); the console settings are written first (below), then
   — last, so that its presence marks a finished configuration, and never over an existing file —
   `<install root>\actingd.config.json`, with only the fields `schema_version`, `state_root`,
   `bind_host` (127.0.0.1), `bind_port` (0), `secret_fingerprint_salt` and `instances` (empty) — the
   Runtime's parser is `deny_unknown_fields`, so not one extra field is written. The console settings
   `%APPDATA%\ActingCommand\acui.toml` get `state_root`, `actingd_config` and `actingd_exe` (the format
   of the "Settings file" section, single-quoted literals; existing `lang` / `text_size` kept verbatim).
   The way it writes matches `crates/acui-app/src/settings.rs`, but `acui-setup` does not depend on
   `acui-app` and is a small duplicate writer. `instances` stays empty here; the instances step fills it.
   The options page then offers four ticks, written together off the event loop (a failure is said on
   the page, which can be used again):
   - **Start at boot** (unchecked by default): only when checked does it write `ActingCommand.cmd` in
     the per-user Startup folder (the shell's `FOLDERID_Startup`), whose content is
     `start "" "<install root>\runtime\actingcommand-actingd.exe" --config "<install root>\actingd.config.json"`;
     only when "also launch the console" is checked as well does it add one more line,
     `start "" "<install root>\ui\acui.exe"`. acsetup does not appear in the batch file. Unchecked, it
     writes nothing; a file of the same name already there is left alone, and mentioned in the log.
   - **Start menu shortcut** (checked by default) and **desktop shortcut** (unchecked by default): an
     `ActingCommand.lnk` to `<install root>\ui\acui.exe`, working in `ui\`, in the per-user Start
     menu's Programs (`FOLDERID_Programs`) or on the desktop (`FOLDERID_Desktop`, so a redirected or
     OneDrive desktop is found where Explorer finds it), written through the shell's own `IShellLink`.
     Unchecked writes nothing; the finish page lists the shortcuts written.
3. **Instances**, optional, looked for as soon as the page opens: "跳过 / Skip" leaves `instances`
   empty, to be filled later with the console's top-bar 实例配置 / Instance Configuration button; the
   finish page says so. Where MuMu is comes from the Runtime's own `check-config` (`mumu_root`: its path
   and source — `ACTINGCOMMAND_NEMU_FOLDER`, a running MuMu, the per-user or machine uninstall registry,
   the vendor folders; see the
   Runtime's `contracts/actingd-check-config.md`), or from the folder a person names when none is found;
   either way it is pinned into the configuration's `mumu_root` through the same candidate and
   `check-config` as below, so a second MuMu install cannot take the instances over later. A Runtime too
   old to say where MuMu is leaves it unpinned, as a note. A Runtime is then started from the install
   root as on an upgrade below (one that answers is shut down and started again: it has no instance yet),
   and `actingctl emulator discover` lists the instances from MuMu's own inventory — no emulator is
   started or stopped; a single instance is ticked for the person. "重新查找 / Find again" repeats it.
   Each ticked instance takes an alias (default `mumu-<index>`). The resources are one field for all of
   them: a resource repository's **bundle** or a **single sealed pack**, as the absolute path of an
   existing file or an `https://` URL fetched through a `.part` file into `<install root>\packages\`; a
   sha256 given is compared; "读取 / Read" opens it. A bundle holds `applications.json` (each server's
   Android package name) and `bundle.json` (every pack's path, package id, server, sha256 and size, and
   optionally `default_packs` per server); the page shows the game, its servers and package names, and a
   pack list starting from the bundle's own default when it names exactly one, else empty for the
   person to pick; the resources read are the ones written (changing the field or its sha256 asks for
   another Read); an empty MuMu field finds MuMu afresh — the wizard
   knows no game and guesses none. A single pack has no package name: the person gives it. "写入实例 /
   Apply" lays a bundle's packs out byte for byte under `<install root>\packages\<game>\`, each checked
   against `bundle.json` first, then writes one entry per ticked instance — alias, a new `instance_id`
   (`instance_` + 32 hex from the OS RNG), `instance_index`, the package name of the chosen pack's
   server, `touch_backend` `adb_shell_input`, `capture_backend` `adb` (until a first `nemu_ipc` frame
   is confirmed on the real machine), and the chosen pack's absolute path as `resource_package` — into
   `actingd.config.candidate-<pid>.json` next to the configuration. The Runtime's `check-config` checks
   it (a package that does not load is named with its alias, path and the loader's message); only an
   accepted candidate replaces the configuration, and the Runtime is restarted on it and `actingctl
   status` must answer. A failure before the configuration is replaced is said on the page and in the log
   and leaves the page usable; one after it stops the run, as does any failed log write. Leaving this step
   asks again, for the summary, what `mumu_root` is and whether a Runtime answers.
4. **Finish**: the summary — what was installed (runtime and ui commits, and the release tag, the
   offline folder or the carried release), the paths, and every note from the steps. "启动监控台 / Open console" launches
   `<install root>\ui\acui.exe` detached and closes the wizard; "finish" only closes. A Runtime the
   instances step started keeps running; when none runs, the console's launcher starts one.

**Upgrade**, on a root that already holds an installation. The install step reads the release's `MEMBERS.json`
first (online without saving it, offline from the folder, the offline edition from what it carries) and compares its two commits with the ones the
installed manifests name: the same two stop there, "already at this release's version", with nothing
fetched. Otherwise, after verifying as above:

1. the new Runtime runs `check-config` on the existing `actingd.config.json` (its `state_root` must be
   absolute); refused, nothing is changed;
2. a `previous\` kept from the upgrade before is set aside as `previous.older-<unix_ms>\` (any such
   directory an earlier upgrade could not remove goes first), and a new `<install root>\previous\` is
   made;
3. when `<state root>\runtime-info.json` exists and the new `actingctl status` is answered, the new
   `actingctl request-shutdown --state-root <state root> --wait 60` asks the Runtime to shut down and
   waits until its ownership record is closed and the process gone — first, because a Runtime the
   console started works in `ui\`. A file left by a Runtime that ended without shutting down, with no
   answer, is said and taken as not running;
4. `ui\`, `tools\` and `runtime\` move aside — an open console makes the first fail — and the verified
   payload is laid out as on a fresh install.

Until the payload is laid out, any failure removes what was half laid out, puts every moved directory
and the older `previous\` back, and starts a Runtime that was asked to shut down — and is gone — again
on the version still installed (said, with its log, or why it could not be); one whose shutdown was not
confirmed is left alone, and said so. Once laid out, the older `previous\` is
removed, and a Runtime that was running is started again on the new version: detached, from the install
root, its output in `<install root>\actingd-<unix_ms>.log`, up once its own `runtime-info.json` names
its pid within 30 seconds. An exit before that is said with its `FATAL` line; no answer in time is said
as possibly still starting. Either way the new version stays laid out and the version replaced in
`previous\`.

Newer or older is judged by publication time. Every install and upgrade keeps the release's
`MEMBERS.json` as `<install root>\installed-members.json`. A release whose `published_at_utc` is earlier
than the one recorded is said as "按发布时间判断，看起来是降级 / by publication time this looks like a
downgrade"; an installation without the record, or a time either one lacks, as "无法判断新旧 / cannot
tell which is newer" — either way, with "the Runtime has no state migration and no rollback", and
"确认降级 / Confirm downgrade" must be ticked before Install goes on. This holds for all three sources.
Publication time is a heuristic (a stable line's dates may one day run against the code's age), and is
said as one; the tick is the person's word, not the wizard's judgement.

State, `actingd.config.json`, the console's `acui.toml`, the Startup launcher and `downloads\` are left as
they are, and the options step is skipped. The version replaced is kept whole in `previous\`, one
version deep: the Runtime ships no state migration and no rollback of its own, so going back stays a
person's choice.

**Progress and the install log**: from leaving step 0 onward every line of work goes to
`<install root>\acsetup-<unix_ms>.log`, and a log write that fails stops the run. The pages show only the
phase (for example "下载 / Downloading · 41.2/74.0 MiB" or "安装文件 / Installing files · 118/260"), a
progress bar — moving without a size where none is known, such as while the Runtime shuts down — and
the latest log line as the one thing being done now. What the person must see — an older `previous\`
or a leftover that could not be removed, a `runtime-info.json` no Runtime answers for — stays under the
bar and is repeated in the summary. On failure the page shows the reason, what was left on disk (the
staging directory removed or not; on a fresh install, which of `runtime\`, `ui\` and `tools\` this run
laid out and must be removed before trying again) and the log path; a worker that panics stops the run
too, with its message. While files are laid out and configured, an upgrade swaps versions, the options are written or the instances step writes, the
window does not close; a lookup or a download may be closed, and a staging directory left that way is
removed on the next run, said in the log. The first close after the instances step started a Runtime
says that it keeps running. A root with program files but no `actingd.config.json`, or the
configuration without program files (an interrupted upgrade), is named on step 0 and neither installed
over nor upgraded. Apart from the installed payload, the configuration, the settings, the fetched release
files under `downloads\`, `installed-members.json`, (when checked) the start-at-boot batch file and the Start menu and desktop shortcuts, the resources fetched into
`packages\` and a bundle's packs placed under `packages\<game>\`, and the log of a Runtime it started, this is the only file the wizard writes (with
`previous\` on an upgrade).

**Offline edition**: `acsetup-full-<tag>.exe` is `acsetup.exe` byte for byte, then `SHA256SUMS` and
every file it lists in its order — `MEMBERS.json` among them, and the resource repositories' bundles —
then an index, then a trailer of exactly 125 bytes:
`ACSETUP-PAYLOAD-1 <payload start, %020d> <index length, %020d> <index sha256>\n`. The index's first
line is `acsetup-payload v1 <tag>`; each further line is `<sha256> <length> <name>` for one file, with the
same names, order and sha256 as `SHA256SUMS` and a `SHA256SUMS` line first. Which edition runs is read
from the executable itself before any window: the end of the PE image — the furthest raw data of any
section, from a minimal hand-written reading of the DOS header, PE signature, COFF header, optional
header, data directories and section table, all in checked arithmetic — against the file's effective
end: its length, or, once signed, the certificate table's offset less at most seven NUL bytes of
padding. Equal is the online edition; more must be a whole payload: trailer shape, payload start equal
to the image end, index at most 1 MiB and matching its sha256, lengths adding up exactly to the effective
end, names as the online fetch allows and also not starting with `-`, not ending in `.` or `.part`, and
unique without regard to case, `SHA256SUMS` and `MEMBERS.json` read from the payload and matching the
index, and a `build-r<7>-u<7>` tag matching `MEMBERS.json`'s commits. Anything else — a truncated file,
a damaged trailer, index or header — stops the wizard on its failure page before it writes anything,
naming what was expected and what was found, saying that nothing has been written and there is no log
yet, and pointing at downloading it again (checked against its `.sha256`) or the online `acsetup.exe`; it
never falls back to the network. The file stays open from that check to the extraction. The index and
`SHA256SUMS` guard against damage, not tampering: trust comes from downloading over HTTPS from the
Releases page and the `.sha256` beside each installer, as for the online edition. Windows SmartScreen
and some virus scanners may warn about a new, unsigned installer with data after its image; that is
expected, not a defect of the wizard. The umbrella release job assembles both installers and reads the
offline one back independently.

**Things it never does**: it does not install a service, does not create a scheduled task, does not change
PATH, does not write the registry; does not modify the configuration template; does not touch a state root
that already holds content; goes on the network only to list and fetch the umbrella release and a
package URL given in the instances step — the offline edition's install step not at all; stops or starts the Runtime only on an upgrade and in the
instances step, as above;
does not unpack a sealed resource pack — the Runtime loads it (a bundle's packs are taken out whole). On Linux the crate compiles as usual (CI runs `--workspace` on both legs), and running it exits
immediately with `acsetup v1 is Windows-only`.

Five dependencies are added (the offline edition's reading of its own executable is hand-written and
adds none), each with its purpose noted in `[workspace.dependencies]`: `sha2`
(verification), `zip` (`default-features = false`, only `deflate` enabled, the same version line the
Runtime locks), `getrandom` (the salt and `instance_id`), `ureq` (the fetch; `default-features = false` with only `tls`:
rustls, its `ring` provider and the compiled-in `webpki-roots`, so no system TLS library) and, on
Windows only, `windows` 0.62 (`Win32_Foundation`, `Win32_System_Com`, `Win32_UI_Shell`: the Startup,
Start menu and desktop folders through `SHGetKnownFolderPath`, and shortcuts through `IShellLinkW` +
`IPersistFile`; the version Slint already locks, so the lock gains no crate).

## Four layers, four crates

One Cargo workspace, dependency direction app → model → rows ← source:

- `acui-rows`: the only place that names contract types for the view model; it re-exports the contract
  types, and adds display functions, the display-name dictionary, and two flattened structures filled by
  `acui-source` and read by `acui-model`.
- `acui-source`: the read face, the only place that touches the state root. The offline face is
  `EvidenceSource::open` / `query` / `open_report` / `read_material`;
  `Session::open(root, mode)` picks one of it and the online `OnlineSource` according to `--source`, or
  gives back `Session::Unopened` with the ledger's own error (`LedgerOpenFailure`: code, operation,
  detail, io kind) when the ledger refuses the offline face, and `material_reader()` hands material
  reading to the background thread. It also reads the instances' task facts once per session
  (`instance_facts()`): offline replayed at the pinned position, online with their status right after
  the pin; for the fact snapshot's scope and value types it names the contract crate directly.
- `acui-model`: a pure Rust view model (tabs, filtering, paging, recovery collapsing, selection), with no
  dependency on slint and **no plain language either** — it gives structured facts only, and all wording
  is chosen by `acui-app` from the language tables.
- `acui-app`: the only crate that depends on slint; the `.slint` files are in `crates/acui-app/ui/`, the
  two language tables in `strings.rs`, and settings-file reading and writing in `settings.rs`.

`slint` 1.17.x, `default-features = false`; the ledger is read-only to the console (what the Runtime
records of the console's own requests — the start press, shutdown requests, the status read at an online
open, instance discovery queries — it records itself), the only control entry points are the
launcher's two buttons (start / request shutdown, see above), its owner-unlock entry (a confirmed
`actingd unlock-owner`), and the instance-configuration window's check-config-gated save, and there is no
approval entry point; no tests are written. The launcher is in
`crates/acui-app/src/launcher.rs`, the instance-configuration window in `instances.rs`, and the client
operations — probe, request shutdown, recording the start press, the online open's status and fact
reads, and instance discovery — are in `acui-source` (`probe_runtime` / `request_shutdown` /
`record_start` / `instance_facts` / `discover_instances`).

Every background worker — the start's readiness poll and its record, request shutdown, unlock-owner, a
save's check-config, discovery, a frame read, and every acsetup worker (release lookup, install or
upgrade, writing the options, discovery, reading resources, writing instances, settling) — is started through
`std::thread::Builder`. A thread the system refuses is stated, with the OS error, where that action
reports, and what the worker would have cleared (the start or unlock in flight, the save in progress,
the frame request) is reset: never a panic on the event loop. A launched actingd whose readiness poll
cannot start keeps running, and the line says so and that pressing Start again probes it. A child that was killed but could not be reaped is
said as that, not as a kill that failed.

A fifth crate, `acui-setup` (binary `acsetup`), sits outside these four layers: the setup wizard,
depending only on slint, serde, sha2, zip, getrandom, ureq and (Windows only) windows, and on none of the layers above; see the previous
section, "Setup wizard acsetup".

## Icon

The application icon is the black single-figure "commander" mark Alice ruled on; the assets are in
`crates/acui-app/assets/`: `acui-256.png` (256×256 transparent PNG) and `acui.ico` (multi-size, 16..256).

- **Window and taskbar icon**: `Window.icon: @image-url("../assets/acui-256.png")` in `app.slint`.
- **Executable icon**: in `build.rs`, `#[cfg(windows)]` calls `winresource` to compile `acui.ico` into the
  exe's resource section; this dependency hangs under `[target.'cfg(windows)'.build-dependencies]` and is
  not compiled on Linux.
- **acsetup**: the same assets, not copied: `crates/acui-setup/build.rs` and `ui/setup.slint` point at
  those two files in `crates/acui-app/assets/` with relative paths.

## What the read face blocks

These are not worked around; they are displayed as they are, and booked here. Line references are at the
pinned rev.

- **Event and repair counts: resolved offline; online, the event count only**. `GlobalLedgerMetadata`
  (`crates/ledger/src/global/evidence.rs:257`) now states `event_count()` (`:319`) and `repair_count()`
  (`:326`) from the authenticated metadata, without verifying any material, so the console no longer
  needs `GlobalLedger::open_evidence` (same file, `:434`) for them. The instance card shows both, and the
  number of rows **loaded in this view** separately; the two are never mixed. Over an incomplete read the
  event count covers only the verified prefix, and the card says so. The SQLite medium has no repair log
  (`None`), and the card says so instead of showing 0; the repair log does not share the event snapshot's
  sequence boundary. Online, the event count is the pinned position, as
  `contracts/runtime-state-observation.md` states (sequences are gap-free from 1); neither the page
  (`LedgerReadScope`) nor `runtime-info.json` states the repair count, so that line says the Runtime does
  not state it.
- **Whole-material read: resolved on both faces**. `read_material_complete`
  (`crates/ledger-forensics/src/material.rs:74`) reads one whole object: fresh ledger metadata twice, one
  reader, one whole-file hash, bounded by `max_material_bytes` and a deadline. The offline face calls it
  with the 8 MiB frame limit and a 30-second deadline (no Runtime caller of it sets one yet; the
  contract's 4-second `RUNTIME_MATERIAL_READ_BUDGET_MS` bounds a single range read, not a whole
  object). Online, the typed client's `RuntimeClient::read_material_complete`
  (`crates/runtime-client/src/client.rs:2089`) gives the same result shape over verified ranges, and the
  console calls it with the same limit and deadline; the Runtime still verifies the whole material for
  every range (a 3.6 MB frame is 19 ranges of 192 KiB).
- **Instance facts: resolved on both faces, at different positions**. Online, the fact store is read
  through `RuntimeClient::runtime_fact_snapshot()` (`crates/runtime-client/src/client.rs:848`), which
  answers at the Runtime's latest position, past the pin. Offline, `runtime_facts_at`
  (`crates/ledger-forensics/src/runtime_facts.rs:61`) replays the store at the pinned position itself,
  under the Runtime's own replay rules; the console never folds `runtime.fact_*` events itself. Lease
  state comes only from the online status read, so offline has none.
- **Geometry and frames cannot be brought together on these two roots**. In the 0828 and v5 roots, the
  only events carrying a `capture.frame` artifact are `artifact.created` / `artifact.verified`, and their
  payloads hold no geometry; the only events carrying geometry are `task.effect_intent` (six on 0828,
  five on v5), whose payload is a single tap coordinate and whose `links` hold **no** `frame_id`. The
  ledger gives no relation joining the two, so the console does not join them — the real frame is drawn as
  it is, and the overlay is empty. At the pin, `task.effect_intent` can state the frame extent its
  coordinates are in (`frame_extent`, `crates/actingcommand-contract/src/event/payload.rs:3315`) and
  `task.geometry_observed` its frame's extent (`:3041`); the overlay canvas uses that extent when an event
  states one. The effect intents on these two roots state none, so their size stays "not recorded".
- **Neither root holds artifact eviction facts**, so the eviction placeholder does not appear on these two
  roots; the code path is written to the contract.

## License

`GPL-3.0-only` (Alice ruled on 2026-09-17). The repository includes the full LICENSE text; the workspace
`license` field and the SPDX header of every `.rs` / `.slint` file agree with it. The interface is
rendered by [Slint](https://slint.dev), used under its GPLv3 licensing option. The Runtime crates depended
on (contract / ledger / ledger-forensics / runtime-client) are `AGPL-3.0-only`, and the two combine under
GPLv3 section 13.
