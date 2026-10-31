<p align="right">🌐 <b>English</b> · <a href="./README.zh-CN.md">简体中文</a></p>

# ActingCommand Console

**⚠️ Pre-release, debug phase: interfaces, configuration and file formats still change, and only the newest version is supported. See "Development status".**

This is ActingCommand's **human console**, a read-only native program. On one Runtime state root it opens
the ledger's **official read face**, and renders the view pages the ledger itself hands out into a
three-column interface: instance card, timeline, detail pane.

It is not part of the Runtime; it is the Runtime's **external, detachable client**.

> **acsetup is not shipped with the builds for now; UI-2a brings it back. Until then, install with `actingctl install`.**

This repository also holds the **setup wizard** `acsetup` (see "Setup wizard acsetup") and the fixed
entry `acforward`, through which an A/B installation starts its programs. Like the Runtime, these
programs know no game: they hold no game logic and no game data. A game is supported through the
resource packs (one standard pack per game) the Runtime loads; the wizard reads what a pack declares
and guesses nothing.

## Current release

- **UI v0.11.4**, a pre-release like every release so far: `acui-windows-<sha>.zip` with `SHA256SUMS`
  on this repository's [Releases](https://github.com/HS7097/ActingCommand-UI/releases). The zip holds
  `acui.exe` (the console), `acsetup.exe` (the setup wizard), `acforward.exe` (the fixed entry),
  `LICENSE`, `README.md` and the `BUILD-MANIFEST.json` that binds them to their commit. It is built
  against the Runtime crates of a Runtime commit after v0.11.6 that still speaks the 0.11 protocols
  (the pinned rev under "Data source").
- **New in v0.11.4**: the Runtime crates are pinned to that commit, and this README was updated. The
  interfaces it declares are those of v0.11.3.
- **From v0.11.3**: whether components fit together is judged from the interfaces each one declares,
  not from version numbers (exit code 6 when they do not fit; see "Component interfaces"); Tools layout 3,
  with the watchdog launcher `actingwatch.exe`, is recognized; once the ledger is at revision 2, a
  rollback to a Runtime v0.11.2 slot is refused (exit code 1, nothing changed); resource-only update
  `acsetup --resources <zip>`, also a tick on step 0; actingd is started through WMI with a hidden
  window, and a start that exits at once is reported with the log's `FATAL` line; an upgrade refreshes
  the fixed entries whose bytes changed.
- **Works with the 0.11 Runtime**: by the interfaces it declares (`actingd-config` [1, 2], `ledger`
  [1, 2], `runtime-client` [1, 1], `package` [1, 2]) it fits Runtime v0.11.3 to v0.11.6; acsetup v0.11.3
  has upgraded an A/B installation to Runtime v0.11.6.
- **Does not work with Runtime 0.12**: the Runtime's 0.12 series (newest v0.12.10) declares
  `actingd-config` [5, 9], `ledger` [3, 4], `runtime-client` [2, 2] and `package` [3, 5]. acsetup refuses
  such a Runtime with exit code 6 before it changes anything, and the console can neither connect to it
  (a 0.12 Runtime refuses requests v1 to v3) nor read its ledger. There is no UI release for the 0.12
  series.
- **Where to get it**: the umbrella repository's
  [Releases](https://github.com/HS7097/ActingCommand/releases) carry the two installers, the online
  `acsetup.exe` and the offline `acsetup-full-<tag>.exe`, each with its `.sha256`. The newest umbrella
  release, v0.11.4, carries Runtime v0.11.4, UI v0.11.3 and the available standard packs. UI v0.11.4 and
  the Runtime releases after v0.11.4 are not in an umbrella release, so the online wizard installs
  Runtime v0.11.4 and UI v0.11.3 today.

## Development status

- **Debug phase**: the main loop is find a problem → fix it → check against what was expected → change
  again. Deployment and availability come second.
- **Every release is a pre-release**: interfaces, configuration and file formats still change. **Only the
  newest version is supported**; older series get no fixes (the 0.11 series is not maintained).
- **Runtime 0.12**: the Runtime's 0.12 series brought a new ledger (a 0.11 state root is not carried
  over), new command-line output and exit codes and a new pack format. This UI does not work with it (see
  "Current release").
- **On real devices**: since early October, routine batches run every day on real MuMu instances,
  scheduled by catalog with the available standard packs. The standard packs' coverage is still
  incomplete, and multi-day unattended runs are still being validated.
- **Version numbers**: X for major or incompatible changes, Y for new features or new coverage, Z for
  fixes (including features that serve a fix). The Runtime, the UI and each standard pack are released on
  their own, only when they change; whether they fit together is decided by their declared interfaces,
  not by matching version numbers.

## Data source: the read face, not files

The program accepts only one argument: the state root. **All file IO for reading ledger data belongs to
the read face**; the console never assembles paths inside the state root itself, never opens `ledger/`,
`artifacts/` or `runtime-state.sqlite`, and never launches a CLI to read data. It starts two
kinds of process, both named in "Launcher": actingd itself (**Start**) and the twice-confirmed
`actingd unlock-owner` (the unlock).

There are two read faces, with the same query, page and cursor semantics; they differ only in where the
answers come from:

**Offline** (the original path):

- `GlobalLedger::open_metadata` opens the state root; the ledger itself determines the medium
  (segment / sqlite), opens a bounded snapshot at one pinned point (on sqlite: the writer open's checks,
  the newest records and the eviction proofs; each page authenticates the rows it reads, at most 256
  records or 768 KiB a page, with a cursor to the rest), and gives `latest_sequence` and the completeness
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
  workbench's launcher (see the "Launcher" section), and shutdown too goes only through this same typed
  client.
- The first page at startup asks without a snapshot position; the `snapshot_ledger_position` the Runtime
  states on that page is the position this session reads at — the pin. A person's jump to the latest,
  and turning following on, move it by the same kind of fresh first page; while following, each poll
  moves it to the position the Runtime's fact snapshot states (below). Offline it never moves. Every page
  after that is
  `RuntimeClient::query_event_page(query, ProjectionProfile::Ui, page.at_snapshot(pos))`,
  the same `EventQuery` bounded to one window, the same page limit, the same `next_cursor`.
- Material goes through `RuntimeClient::read_material_complete` on a connection of its own, made at
  the pin with the page connection, to the same owner epoch, and rebuilt after a read latched it; so a
  material read (up to 30 s) never holds a follow poll or a status read behind it. The client
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
  latest; while following, every poll reads the task facts again, whether or not the pin moved. The first two
  lines state the sequence each read was taken at; both may be past the pinned snapshot, so this is state
  as last read, not state at the pin. By contract the status read takes the Runtime's memory only and
  appends no ledger record (AO-4q); its position is the ledger's committed head when it was sampled. Then,
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
  session opens (after the window shows, on the window's reader thread), bounded by a 30-second deadline.

`--source <auto|offline|online>`, default `auto`: if the client can connect to the Runtime the state root
points at, online; otherwise offline. The instance card's first line, "read face", states which one was
chosen and why (`runtime-info.json` absent, or the client error code of the failed connection). With
`online`, a failed connection **is an error carrying the client's error code**; it does not quietly
switch to offline. Across both read faces, a Runtime that connects but cannot answer the first page is an
error in any mode, with no fallback. Neither error ends the console: the window stays, the list and the
instance card state in red why the read face did not open, and "Reopen" is offered.

When the offline read face cannot be opened — with `offline`, or with `auto` falling back to it — the
console does not exit: the ledger window opens **unopened**, which is what a fresh install looks like, since the
Runtime creates the ledger on its first start. The instance card then has only its first line, "read
face", which says the ledger could not be opened and states the read face's own `code`, `operation` and
`detail` verbatim, and for an io error its io kind (`LedgerIoKind`: `not_found`, `permission_denied`,
…). The kind, never the localized `detail`, tells "no ledger yet" from "a ledger that cannot be read":
only for `ledger_io` with io kind `not_found` does the line add that the state root has no ledger yet,
most likely because the Runtime has never been started here, and point at the launcher's Start. No ledger
fact is shown, not even a zero: the list says the ledger is not opened instead of standing empty; the
tabs, the filter boxes, the id box and the time slider are off; the module and port boxes and the frame
pane say "ledger not opened". Nothing is asked of the ledger and no material is read. The launcher works
as usual; once the Runtime has started and made the ledger, "Reopen" opens the read face again. A window
of the default source (`auto`) that reads offline, or whose offline face is unopened, because no Runtime
was there offers "Reopen" as soon as the launcher's probe finds one running, with a line saying it can
read online now; a window `--source offline` asked for keeps reading offline. Neither switches read face
mid-session on its own.

**The reader thread**: the window comes up first, and the read face is opened afterwards on the window's
own reader thread (`acui-ledger-<window id>`). Opening, page reads, reading earlier, following, the jump
to the latest, the time bound's probes and the frame's own events are all read there, one request after
another; the window's event loop never waits on a read, and the read face never leaves that thread. While
it opens, the list says the ledger is being opened and the controls are off. While a read is in flight,
the top bar has one more line, "… waiting N s (a read of this kind takes at most about M s)", or "at most
about M s per page" for a read that follows a cursor. It shows once a request has read for more than a
second, so a quick read never makes the rows below jump; then its clock moves once a second, and only
while a read is in flight. A tab or filter change, the time slider at rest, the jump to the latest, following turned
on or off, and a reopen supersede the reads in flight: what they read is not left in the window, and a read
that follows a cursor stops before its next page; a follow poll is only sent once the last one has
finished. The window's state is locked only for the few milliseconds it takes to leave a result there or to
paint, never while reading. A reader thread that panics, or that the system refuses, is stated in red in
the window and in the technical log, and "Reopen" is offered too. "Reopen" starts a new session: the old
reader stops after the page in hand, and the new session opens the read face again with the tab and the
filters the boxes show now, without a time bound. When a read reaches a Runtime that is another process now
(`console_runtime_owner_changed`), the top bar says so beside "Reopen", which reads the Runtime that runs
now. A time span read follows the cursor for at most 64 pages; with still no event it stops, as
`console_time_span_page_limit`, rather than keep the window reading.

Dependencies are pinned to one exact Runtime source (`Cargo.toml`), the head of Runtime
PR #1469 (Workflow #429 114-A: the not-run history shapes and ledger revision 5, on top of R-7a; it keeps
v0.12.32's bounded offline opening), which speaks the 0.12 protocols (`runtime-client` revision 4, ledger
revision 5). The Runtime merges pull requests with a merge commit, so this head stays on its main:

```
rev = "4d3523f78240b2b97ef57c876ec2bb63d403b1eb"
```

The five crates (contract / ledger / ledger-forensics / runtime-client / execution-kernel) share this one rev.
The embedded typed client reads optional `resource_dispositions` groups in online lifecycle-failure
pages; the offline ledger reader accepts them in full failure and `ResourceQuiescence` records.
Historical records without the field keep their meaning. Strict validation and explicit read errors
remain. The UI still uses `ProjectionProfile::Ui`: supplied failure groups remain visible under its privacy rules;
observed-phase groups are available through full events/Forensic, not the UI projection.
The embedded ledger reader also uses the shared recovery lifecycle contract: preparation triggers
link their preparation event, and task triggers retain their task/run links. `environment_ready`
and package `recovered` remain distinct outcomes. These details belong to full lifecycle records;
the public UI projection and the page's `run_recovery` groups keep their existing shapes.
Producer, Tools and UI releases must use this contract together.
`Cargo.lock` is checked in; CI runs `cargo build --locked --release --workspace` on windows-latest and
ubuntu-latest. The closure contains `rusqlite` (bundled), so both need a C compiler. CI builds pull
requests and pushes to branches other than `main`; a push to `main` builds nothing. A release is on
demand only: Actions → release → Run workflow (or `gh workflow run release.yml -f bump=patch|minor|major
[-f version=X.Y.Z] [-f source_sha=<sha>] [-f prerelease=true] [-f dry_run=true]`) builds the chosen `main` commit and publishes
`acui-windows-<sha>.zip` with `SHA256SUMS` as this repository's Release `vX.Y.Z`, its tag on that exact
commit; with `prerelease` (always for an `-rc.N` version) it is a pre-release, never marked Latest; the
version exists only in the tag.

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
  when a window's page said its read was incomplete — except a page that only the Runtime's read budget
  stopped (#411 A51): its cursor goes on, and the window follows it.
- **Jump to latest, follow latest (online only)**: "jump to latest" moves the pin to the Runtime's latest
  position (one fresh first page), reads status and facts again (the status read appends no ledger
  record), drops any time bound and starts the view over from the new pin. "Follow latest"
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
  reading starts there — unless the Runtime's read budget stopped that page before it decided a row
  (#411 A51), which decides nothing, so the pin. One found: the start moves above it, the step doubling each time, for at most
  three probes within the one-second budget, and otherwise falls back to the pin. The query's own time bound still decides what
  shows; the top bar states the positions actually read.
- **Row types are no longer mirrored**: `acui-rows` re-exports the contract types directly, and only adds
  display functions such as local time, id abbreviation, wire codes and the display-name dictionary.

## Frame material: read, but only what is verified

The console **does** load frame bytes, but only through the material read face, and only within this
rule:

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
first; only when they hold no capture of it are the frame's own events read, by the reader thread, with
one query by frame id (the note under the frame says so while it runs), kept while the pane stays on that
frame. A failed or incomplete read is stated in the frame note under the frame, and the next reload tries
again. The material reader is the one the reader thread hands the frame pane; when a frame read latches
the material connection, the reader thread makes a new one at once, and the next frame read uses it.

On the frame, beside the event's own geometry:

- **The page label** above the frame, beside its title: the page the recognition on this frame matched,
  or that it matched none. On the frame it would cover the page's own header targets.
- **The tap mark**: a white-ringed dot centred on each point of the step's effect intent (`action.x, y`; a
  swipe or drag has one per point). It replaces the event's own `action` geometry, which would draw the
  same input twice.
- **One sentence** under the frame, per input meant on it: "Step 0 notice_close: recognized
  `<game>/news`, tap (1142, 102)".
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

The main window is the **workbench**: the Runtime in plain words with Start and Request Shutdown, the
plain-words result of the last press, the language and the text size, a "Troubleshooting and Ledger"
button, and an "Add Instance" button that is off (see "Instance configuration"). The ledger window,
"Advanced Troubleshooting and Ledger History", opens only when asked: from that button, or at the start
when `--tab` or `--source` is given. There is at most one per source (the default `auto`, `online`,
`offline`); asking again restores the one open (Slint 1.17 has no call that brings a window to the
front). `--state-root` alone opens none, since the fixed entry passes it on every start. A closed ledger
window's reader stops after the page in hand; opening it again is a new window and a new session. Closing
the workbench ends the program, except while an unlock runs (see "Launcher").

`--state-root` applies to this run only and overrides `state_root` in the settings file; if neither is
present the workbench still opens, with a red bar at its top that says there is no state root to use and
gives the usage, and Start, Request Shutdown and "Troubleshooting and Ledger" are off; no default is
guessed. Started through an A/B installation's fixed
entry `<install root>\ui\acui.exe`, the console takes the state root, the configuration and the Runtime
program from the installation's selection instead, and refuses a `--state-root` that differs from it (in
the same red bar). `--source` selects the ledger window's read face (see above).
`--tab` specifies the ledger window's startup tab (for screenshots and review), and its values are the
views' own wire names. `--lang` applies to **this run only**, overrides the language in the settings
file, and is not written back to the settings file.

Started directly — `acui.exe`, or a shortcut that points at it — the console opens no black console
window; started from a command line, `--help` and the reason for a failed start still go to that
console. Started through the fixed entry `acforward.exe`, a console window still shows for now.

The windows are resizable: the workbench 820×560 by default, 600×420 minimum; the ledger window 1400×900
by default, 1100×700 minimum, its middle column stretching with the window and the two side columns
keeping a fixed width. The windows follow the system DPI; the program sets no scaling of its own.

Offline acceptance material: `tools/fixture-state-root` is a small generator, its own package outside the
workspace and its build, that starts the pinned Runtime host on two fake devices and leaves a small v0.12
state root (instance bindings with ports, two capture frames with material, a few hundred events, more
than one offline page). The manual workflow `fixture` (`.github/workflows/fixture.yml`, Windows) builds and
runs it and uploads the zipped root as the artifact `fixture-state-root-<commit>`; unzip it to a writable
directory and open it with `acui --state-root <root> --source offline`. It is material, not a test.

### When the start fails

A start that cannot go on — a command line the console does not understand, a workbench window that
cannot be made — is shown in a
window of its own with the reason, written to the console the program was started from (if there is one),
and appended to the technical log. The exit code is 2 for a command line the console does not understand
and 1 otherwise. When not even that window can be made, a system message box says it. `--help` prints the
usage on that console, or shows it in the same window when there is none, and exits 0. An installation
selection it cannot read, a `--state-root` that differs from it, a `state_root` that is not absolute, or no
state root at all is no failed start: the workbench opens with a red bar, plain words first and then the
original, and the technical log has it too.

The technical log is `%LOCALAPPDATA%\ActingCommand\logs\acui-<start unix ms>.log` (elsewhere under
`$XDG_STATE_HOME`, else `$HOME/.local/state`), beside actingd's start logs. It is created by its first
entry and holds failures only: a failed start or no state root to use, a problem of the settings file, a
result a background thread could not hand back to the window, a ledger window that could not be made or
shown, a ledger reader thread that could not start or that panicked. A
read face that does not open is no failed start: the ledger window stays and says why in its ledger area
(see above). Once it exists, the top of every window says where it is; when it
cannot be written, the windows say that instead.

## Interface language and text size

Two dropdowns at the right of the workbench's top and of every ledger window's top bar; a choice made in
any window applies at once to every open window and is written into the settings file as soon as it is
made, and a ledger window opened later comes up with the current language and text size:

- **Text size**: standard / large / extra-large = 1.0 / 1.25 / 1.5. **Takes effect on the spot**. Every
  font size and line height in the interface is one base size meant for a person at normal DPI (list row
  14px, detail pane 13px, subheading 16px, line height 26px) multiplied by this one factor; `app.slint`
  has only this one knob, `Scale.factor`. The minimum widths of the controls that hold text (the boxes,
  the id box, the time slider, the notice lines) and the minimum heights of text areas are multiplied by
  it too; a window's minimum size and the three columns' floor widths are not.
- **Language**: 中文 / English. **Takes effect on the spot**: the choice fills every window's `Strings`
  global again and paints every line of every window again — the launcher's plain and technical lines,
  the frame pane's note, the port box and the words after a read error included. A background thread never holds a language table; it
  hands back phrases that are put into words when painted, so a start or a frame read still in flight
  reports in the new language. The two language tables are in `crates/acui-app/src/strings.rs` (`ZH` and
  `EN`); there is no gettext. A read face's own error text is shown verbatim, untranslated.

Two-layer labels: the upper layer is the name for a person to read, and the grey lower layer is the
verbatim form used in the program — the raw `event_type`, module names, ids of every kind,
`payload_schema` and sha256 are never translated. The dictionary is in `crates/acui-rows/src/display.rs`
and covers all 121 `event_type`s and 20 `origin.module`s in the contract; **anything not in the table is
displayed verbatim, not guessed**.

### Settings file

Language and text size are stored under the per-user configuration directory:

```
Windows:  %APPDATA%\ActingCommand\acui.toml
Linux:    $XDG_CONFIG_HOME/ActingCommand/acui.toml (falls back to $HOME/.config/…)
```

```toml
# zh | en
lang = "zh"
# standard | large | extra-large
text_size = "standard"
# flat | aero; read and checked only, nothing is drawn by it yet
theme = "flat"
# optional, absolute paths
state_root = '<install root>\state'
actingd_config = '<install root>\actingd.config.json'
actingd_exe = '<install root>\runtime\actingcommand-actingd.exe'
```

It is read once at startup, and a dropdown change writes its one key: the file is read again, the last
line of that key is replaced (or a line appended), and every other line — comments, the path keys, keys
the console does not know such as `install_root`, lines it cannot parse — is kept byte for byte; the
result goes to `acui.toml.tmp` beside it, which then replaces the file. **This is the only file the
console itself reads and writes**, besides the actingd start logs the launcher creates and, after an
early exit, reads back, and its own technical log (see those sections): it is not inside any state root,
and the state root still belongs entirely to the read face. A missing file is a first run: the defaults
(Chinese, standard, flat) are used and nothing is said. Anything else is said at the top of every window,
with the file and the line: a value the console does not recognize, or a line that is not `key = value`,
in amber, the default used and the line kept; a file that is there but cannot be read (permissions, not
UTF-8), in red, the defaults used and **no save at all this run**, so the file is never written over; no
config directory, or a save that failed, in red, the latter until a save succeeds. Each also goes to the
technical log. The parser is hand-written: one `key = value` per line, a line starting with `#` is a
comment, one matching pair of quotes is stripped, there is no escape handling and no comment after a
value — write Windows paths in single quotes (TOML literal strings), not in double quotes with doubled
backslashes.

## Launcher

On the workbench: on the left the Runtime state from the last probe, in plain words, then the Start and
Request Shutdown buttons, and the line below them is the result of the last press, in plain words only (no
key, code, PID or path; an outcome code shows only as its short words; a failure only the technical text
states adds "details in Troubleshooting and Ledger"). Every ledger window's top bar has the technical
lines: the state from the last probe with its PID and owner epoch, or the client's code, and below it the
technical text of the last press; the unlock entry is there too (below). A line turns red on a failure and
wraps instead of being cut. The probe, the whole start (probe, launch,
readiness, record) and the shutdown request each run on a background thread of their own, and the window
never waits on them; one request of each kind runs at a time, and a second press while one has not
finished only says so. Of probes in flight, only the newest one's answer is shown.

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
  child process has exited it stops and states the exit code, actingd's terminal outcome in that log —
  its last `OUTCOME actingd:` line holding an `actingcommand.outcome.v1` envelope with a code
  (runtime-outcomes 1), shown as the developer line written just before it **verbatim**, else as that
  outcome line — or that the log could not be read, with the error, or holds no such line — and the log
  path; if it has not exited it
  does one more `connect`, and a connection means ready, stating the PID and owner epoch, and then the
  press is recorded (see below). A ledger window of the default source that reads offline, or whose
  offline face is unopened, then offers "Reopen", saying it can read online now — it **never quietly
  switches read face mid-session**. If all 60 attempts fail to connect, it says "still not ready" and the last
  client error code. Readiness is **never read from the daemon's output**: the log is read back only
  after an early exit, for that one outcome, and nothing in it is interpreted but whether its code is
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
  effect without being recorded; the other is an unlock that fails at stage `unlock_owner_ledger` or is
  killed (on timeout, or when reading its status fails; below).
- **Unlock owner**: offered only when the terminal outcome of the last start has the code
  `owner_resource_unconfirmed`
  — actingd refused the state root because its last Runtime owner exited with device resources in use or
  unconfirmed. The workbench then says an unlock is needed and points at "Troubleshooting and Ledger", and
  every ledger window shows an "Unlock Owner…" button below its technical lines. The first press runs
  nothing: it only shows the statement "the device resources of the last Runtime are released" and a
  "Confirm and Unlock" button. The second press runs, from `actingd_exe` (absolute, as for Start), exactly
  `unlock-owner --config <actingd_config> --actor acui --confirm-resources-released`, on a worker thread,
  with no console window (`CREATE_NO_WINDOW` on Windows: its output is captured, not detached), stdout
  and stderr captured, for at most 180 s — well above the Runtime's own 120 s budget for the ledger stage,
  so a run that would finish is never cut off; past that it is killed and reaped and the line says the
  outcome is unknown. The actor `acui` names the console, never a person: no OS user name reaches the ledger.
  The whole trimmed stdout must be one JSON object with `schema_version`
  `actingcommand.actingd.unlock-owner.v2`, judged by its `outcome.code`. `unlock_owner_completed` with
  exit code 0 states the unlocked owner epoch, the disposition before (`in_use` / `unconfirmed`) and the
  `owner.lock` revision, withdraws the entry and presses Start once more, through the same path. Any
  other code is a failure: it states the code, the outcome's `stage` value and `journal_appended`
  **verbatim** (`true` only at stage `unlock_owner_ledger`: the unlock is durable and the next start
  takes the epoch over, but its ledger fact is missing), with a dash for either one a failure refused
  before the unlock itself (usage, options, the installation check) does not carry. With no JSON, the
  last developer line (`<LEVEL> actingd: <code> <text>`) on stderr, else its last non-blank line, is
  shown **verbatim**. A spawn failure, a timeout, output that does not parse or does not match the
  contract, and an `unlock_owner_completed` with a non-zero exit code each have their own line, and none
  retries the start. After any outcome but an
  unlock the entry is back at its first step; a new start withdraws it. Start is refused while the
  unlock runs, and so is closing the workbench, which says to close it once the unlock has ended;
  pressing Start again while the previous start has not finished is
  refused outright — "The Previous Start Has Not Finished" — launching nothing. The console records no client action for it: there is no running Runtime to record
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

The console has no pause control: pause scheduling with
`actingctl pause --state-root <state root>` (add `--instance <alias>` for one instance) and lift it with
`actingctl resume` and the same flags, or use the MCP tools `ac_pause` / `ac_resume` (see "MCP for
agents"; in Runtime v0.11.6 these two fail with `client_action_invalid` for an alias
containing upper-case letters, a known issue, so use `actingctl pause` / `resume` there). The ledger window
can only resume, conditionally, a pause another tool set (below). Start at boot is an option of the setup wizard; see "Setup wizard acsetup".

- **Conditional resume (ledger window)**: below the technical lines the window lists the global pause and
  each instance's pause of its last status read (reason, revision, since), each by its own scope. One
  another tool set — the global one, or an instance's whose device is handed back — carries "Resume
  Scheduling…": the first press only shows the statement "resume this one pause" and "Confirm and
  Resume"; the confirm sends one conditional `ResumeScheduling` on a worker thread, over a fresh person's
  connection (actor `user`), its condition the owner epoch of that status read and the scope's revision,
  so the Runtime refuses it and lifts nothing once that pause has changed or another Runtime runs; only the
  pause of that line is resumed. Lifted, refused (shown by its code, "code not in this console's table" for
  one the table lacks) or not confirmed (a receipt timeout, a dropped connection,
  `runtime_scheduling_resume_unconfirmed`), nothing is sent again: the status is read again on a new
  connection and painted. The line says what that read found: one that failed, or one the window could not
  make (it reads offline, or no longer reads), says in red, with its code, that whether scheduling is
  paused is unknown, and a failed status read shows one red line in place of the pause lines; only a read
  that was made says the pause lines are current. Two kinds of pause get no button, only why: reason
  `alice-116-scheduler-stop` ("Alice decides when this pause is lifted"), and a deployment's — `acsetup_update`, `install_restart`
  and any reason starting with `deploy-` — which the installer lifts (the console does not run
  `release-hold`). An instance pause still draining or handing its device back gets none either: the
  Runtime refuses its resume then.

## Instance configuration

The console no longer has an instance-configuration window. Until the Runtime's `install instance`
lands, an instance can only be added by hand: edit the `instances` of the actingd configuration
(`actingd_config`) as the Runtime's installation notes describe, then restart the Runtime. The
workbench's "Add Instance" button is off for that reason, with this note beside it and the path of that
configuration file below, selectable to copy. Finding instances comes back with the console's own
instance page in a later version.

## Setup wizard acsetup

> **acsetup is not shipped with the builds for now; UI-2a brings it back. Until then, install with `actingctl install`.**

`crates/acui-setup` is a standalone binary `acsetup.exe` (a Slint window, the same styling and icon as the
console) that installs the release files from the umbrella repository's
[Releases](https://github.com/HS7097/ActingCommand/releases) into a **per-user** installation. It ships
together with the UI repository's Windows build (`acui-windows-<sha>.zip`, an Actions artifact of a
branch or pull-request build or an asset of a UI Release `vX.Y.Z`, contains `acui.exe`, `acsetup.exe` and the stable entry `acforward.exe`). The same program comes in two editions, and a person downloads only one of them: the
online `acsetup.exe`, which fetches the release itself or takes a folder a person filled by hand, and
the offline `acsetup-full-<tag>.exe`, which carries one whole release (see "Offline edition" below). One window,
next only (the instances step can also be skipped), five steps; each page shows where its work stands
the way an installer does, and the full account goes to the install log (see "Progress and the install
log" below):

0. **Location**: the install root only (changeable, default `%LOCALAPPDATA%\Programs\ActingCommand`, no
   administrator needed), the free space on that volume, and whether an installation is already here
   (reading selected MEMBERS or the original root manifests before migration; if there is one, the
   run is an **upgrade**, see below). Next creates the root and the install log. On a fresh install this step also requires a usable state root and an install root free of single quotes (the console's settings hold those paths as TOML literal strings), and stops here otherwise. A wizard running from a business slot or an original program directory stops here. The verified fixed
   management entry at `<root>/ui/acsetup.exe` can upgrade the business slots. The offline edition adds to the free-space line what extracting its release takes.
1. **Install** (**Upgrade** on an installed root, see below), one page from the download to the
   layout; on success the next page follows by itself. By default online. On entering, the umbrella
   [Releases](https://github.com/HS7097/ActingCommand/releases) are asked over HTTPS for one release: the
   newest stable release when there is one (GitHub's `releases/latest`), else the newest pre-release by
   publication time (today the umbrella publishes `vX.Y.Z` pre-releases; the old daily `build-*`
   pre-releases are discontinued and stay on the page for reference only); never a draft. Its tag, name, date, kind and size are shown and logged. "安装 / Install" fetches `SHA256SUMS`,
   `MEMBERS.json` and then every other file `SHA256SUMS` lists — nothing else — into
   `<install root>\downloads\<tag>\`, each through a `.part` file renamed once its length is the length
   the release states, with a progress line per tenth for a file of a MiB or more; a file already there is
   fetched again, never trusted, and a failed one's `.part` file is removed. The tag and every file name
   are used only if they are letters, digits, `.`, `-` and `_`, do not start with a dot and are no Windows
   device name. HTTPS only, redirects included; a connection quiet for a minute fails. Ticking **Offline**
   instead takes a folder that already holds one release's files (default `%USERPROFILE%\Downloads`,
   typed in). A failed lookup is stated on the page and in the log, with "重新查询 / Look up again" and
   Offline both open; a failed fetch stops the run. This is the program's only network code (`ureq`,
   blocking, rustls with the Mozilla root set compiled in); the Runtime has none.
   The offline edition has neither the lookup nor the Offline tick: the page names the release it
   carries — its tag and the two commits of its `MEMBERS.json`, checked at start — and "安装 / Install"
   extracts it into `<install root>\downloads\<tag>\`, each file through a `.part` file whose length and
   sha256 must match before it is renamed (a rename a scanner holds up is retried a few times).
   The folder — fetched, offline or extracted — is then verified and laid out on the same page. It must hold `SHA256SUMS`, `MEMBERS.json`,
   `actingcommand-runtime-<sha>.zip`, `actingcommand-tools-<sha>.zip` and `acui-windows-<sha>.zip`
   (`<sha>` taken from `MEMBERS.json`'s `runtime_sha` / `ui_sha`; the three zips must appear in
   `SHA256SUMS`). `SHA256SUMS` is checked entry by entry; the archives are extracted into the temporary
   directory `.staging-<unix_ms>` under the install root, split into `runtime\`, `ui\` and `tools\`;
   nothing ever runs from it. Then, against the `BUILD-MANIFEST.json` each zip
   carries, the source repository, the commit id (equal to the MEMBERS sha), the Runtime's
   `runtime_payload_layout` (`distribution-v1`) and the size and sha256 of every entry in `files[]` are
   checked; a file in the zip that the manifest does not list also counts as a mismatch. Any mismatch
   stops it, worded as "the content differs from what it was at creation" — this is an integrity
   statement, not an authorization tone. Nothing inside the zips is run during verification. The program core (the Runtime and UI payloads) is prepared in the fresh A slot
   or the spare slot; the Tools files go to the install root's `tools\`, where only files whose content
   changed are replaced. A replaced file is kept, nothing there is deleted, and a file the release does
   not know is left as it is — such as `actingcommand-device-test.exe`, which Runtime v0.11.6 no longer
   ships; move it away by hand. Each slot retains its original manifests and MEMBERS; candidate and download
   materials remain available. The finish page reads `tools\platform-tools\source.properties` at the
   install root.
2. **Options**, the configuration already written. Right after the layout, on the install page and
   inside its do-not-close span, the fresh install is configured with no question asked: the state root
   is `<install root>\state` (it must not exist or must be an empty directory — checked on step 0,
   before anything is fetched; **a state root that already holds content is never taken over**);
   `secret_fingerprint_salt` is generated as the hex of 32 bytes from the system random source
   (`getrandom`; **not displayed, not logged**). The private configuration under
   `install/generations/<generation>/actingd.config.json` is qualified, console settings are written,
   and `install/active.json` is committed atomically. The configuration contains only `schema_version`, `state_root`,
   `bind_host` (127.0.0.1), `bind_port` (0), `secret_fingerprint_salt` and `instances` (empty) — the
   Runtime's parser is `deny_unknown_fields`, so not one extra field is written. The console settings
   `%APPDATA%\ActingCommand\acui.toml` get `state_root`, `actingd_config` and `actingd_exe` (the format
   of the "Settings file" section, single-quoted literals; existing `lang` / `text_size` kept verbatim).
   Configuration and Runtime settings name the fixed root argument alias and executable entry.
   The way it writes matches `crates/acui-app/src/settings.rs`, but `acui-setup` does not depend on
   `acui-app` and is a small duplicate writer. `instances` stays empty here; the instances step fills it.
   The options page then offers four ticks, written together off the event loop (a failure is said on
   the page, which can be used again):
   - **Start at boot** (unchecked by default): only when checked does it write `ActingCommand.cmd` in
     the per-user Startup folder (the shell's `FOLDERID_Startup`), whose content is `@echo off` followed by
     `start "" "<install root>\runtime\actingcommand-actingd.exe" --config "<install root>\actingd.config.json"`;
     only when "also launch the console" is checked as well does it add one more line,
     `start "" "<install root>\ui\acui.exe"`. acsetup does not appear in the batch file. Unchecked, it
     writes nothing; a file of the same name already there is left alone, and mentioned in the log. An
     install root containing `%` cannot be quoted verbatim in a .cmd, so with the tick on this step
     fails and says so; the page stays usable.
   - **Start menu shortcut** (checked by default) and **desktop shortcut** (unchecked by default): an
     `ActingCommand.lnk` to `<install root>\ui\acui.exe`, working in `ui\`, in the per-user Start
     menu's Programs (`FOLDERID_Programs`) or on the desktop (`FOLDERID_Desktop`, so a redirected or
     OneDrive desktop is found where Explorer finds it), written through the shell's own `IShellLink`.
     Unchecked writes nothing; the finish page lists the shortcuts written.
3. **Instances**, optional, looked for as soon as the page opens: "跳过 / Skip" leaves `instances`
   empty, to be filled in by hand later (see "Instance configuration"); the finish page says so. Where MuMu is comes from the Runtime's own `check-config` (`mumu_root`: its path
   and source — `ACTINGCOMMAND_NEMU_FOLDER`, a running MuMu, the per-user or machine uninstall registry,
   the vendor folders; see the
   Runtime's `contracts/actingd-check-config.md`), or from the folder a person names when none is found;
   either way it is pinned into the configuration's `mumu_root` through the same candidate and
   `check-config` as below, so a second MuMu install cannot take the instances over later. A Runtime too
   old to say where MuMu is leaves it unpinned, as a note. A Runtime is then started from the install
   root as on an upgrade below (one that answers is shut down and started again: it has no instance yet),
   and `actingctl emulator discover` lists the instances from MuMu's own inventory — no emulator is
   started or stopped; a single instance is ticked for the person. "重新查找 / Find again" repeats it.
   An empty MuMu field finds MuMu afresh. The resources are the resource repositories' **bundles** the
   release carries: its `MEMBERS.json` names each in `bundles[]` with its sha256, and the file is in the
   download folder, already checked against `SHA256SUMS`; on the first discovery the wizard holds each
   to be listed in `SHA256SUMS` under that same sha256, checks the file's sha256 once more and reads it.
   A bundle that fails any of this is named in the notes; the configuration plan cannot be
   submitted while a required release bundle is unreadable.
   **No URL is asked for, and no hash is typed by hand.** A bundle holds
   `applications.json` (the game, its display name `label` when given, and each server's label and
   Android package name) and `bundle.json` (every pack's path, package id, server, sha256 and size, and
   each server's default pack in `default_packs`). A bundle whose `bundle.json` is
   `actingcommand.bundle.v2` (the contract's `BundleIndexV2`) maps each package id to a
   content directory `packs/<digest>/` named by its `content-directory.v1` digest, and a server's
   default pack is the one `applications.json` names in `servers.<server>.default_package_id`; v1 is
   read as before. `actingcommand.bundle.v3` adds the required `maintenance` array, with
   `package_id`, `server` and `uses` (`startup`, `prerequisite`, `return_home`) per entry.
   The shared contract strictly decodes and validates v2/v3. Every actual v3 pack passes
   hash/Containment and `PreparedContainedTask::describe_path`, then the shared
   `validate_bundle_maintenance` checks identity, use qualifications and the bundle's chains.
   Admission has a 120-second deadline and runs no task or provider. Unknown versions, fields,
   uses, references and invalid qualifications fail explicitly. The page lists the programs and package names they
   support — in the shape "`<label>`：`<server label>` `<Android package name>`", as the bundle's
   `applications.json` states them (the game id when a bundle gives no `label`) — whether each came with the release, and two kinds of server as not offered:
   one that names a default pack but no package name, and one with a package name but no default pack.
   A local bundle file can be added at any time — it is the way in while the release carries none: its
   absolute path, "加入 / Add", no hash; a second bundle for a game already
   offered (whatever the case of its name) is refused. Each ticked instance takes an alias (default `mumu-<index>`) and
   **its own choice** of "program · server · package name", one per server that has both a package
   name and a default pack;
   when there is only one, it is chosen for every instance. The wizard knows no game and guesses none.
   "写入实例 / Apply" first stages and verifies the bundles, builds the configuration plan and
   resolves conflicts, then confirms Runtime shutdown before placing installed material under
   `<install root>\packages\<game>\`. ZIP bundles go under `bundles\<bundle-sha256>\`, retaining
   older files even when their pack names are reused. A v2/v3 pack is
   streamed file by file into `<digest>.part\`, each file hashed as it is written, and renamed to
   `<digest>\` only once the files, bytes and `content-directory.v1` digest are the index's; a
   `<digest>\` already there is read whole and reused when its digest matches, and otherwise renamed
   `<digest>.broken-<unix>` (kept, said in the notes) and unpacked again; a failed unpack is kept under
   that name too. Older directories and zips are left in place, and neither declaration is written into
   the install root. Logs, per instance,
   the program, the pack's package id, path and sha256; then writes one entry per ticked instance —
   alias, a new `instance_id` (`instance_` + 32 hex from the OS RNG), `instance_index`, the chosen
   server's package name, `touch_backend` `adb_shell_input`, `capture_backend` `adb` (until a first
   `nemu_ipc` frame is confirmed on the real machine), and the absolute path of that server's default
   pack (for v2 its `<digest>\` directory, which a Runtime with content-directory support admits in
   full) as `resource_package`. The selected game/server's maintenance uses supply the existing
   `startup_package`, `prerequisite_packages` and `return_home_packages`; a return-home use also
   registers its prerequisite reference. No business control or trigger is added. Equal complete
   bindings reuse the current value. Conflicts show current/proposed values and their sources for
   an explicit Keep/Use choice; Cancel stops the plan, and an unanswered choice expires after
   30 minutes. Missing declarations do not delete existing bindings. The merged candidate's actual
   old and new references are admitted again and checked through descriptor qualifications and
   `PrerequisiteChain`, including every retained binding; unresolved material rejects the plan.
   acsetup prepares a private successor generation and preserves relative-path meaning. The target's
   `check-config` checks it; current generation and exact baseline bytes must match before atomic
   selection commit. Prior generations remain intact. A pre-start failure restores
   the original configuration and leaves Runtime stopped; incomplete restoration stops setup.
   Once committed, Runtime is restarted on it and `actingctl
   status` must answer. A failure before the configuration is replaced is said on the page and in the log
   and leaves the page usable; one after it stops the run, as does any failed log write. Leaving this step
   asks again, for the summary, what `mumu_root` is and whether a Runtime answers.
4. **Finish**: the summary — what was installed (runtime and ui commits, and the release tag, the
   offline folder or the carried release), the paths, and every note from the steps. "启动监控台 / Open console" launches
   `<install root>\ui\acui.exe` detached and closes the wizard; "finish" only closes. A Runtime the
   instances step started keeps running; when none runs, the console's launcher starts one.

**A/B installation and upgrade**: each slot, `<root>/A` and `<root>/B`, keeps the program core:
the Runtime and UI payloads, their original build manifests and exact MEMBERS (a retained v0.11.1
slot also keeps its own Tools). The Tools, state, content-addressed resources, models, downloads,
logs and private configuration generations stay outside the slots.
acsetup owns the sole `install/active.json` selection: slot, generation, MEMBERS identity and
hash-bound config/provider inputs are committed together. Configuration edits also use its
writer lock and an exact active-generation/byte comparison.

Downloads, full verification, spare-slot materialization, resource admission and configuration
planning precede shutdown. Existing instances retain their identities and business settings;
associations and maintenance conflicts use the same explicit choices as the instances page.
Private generations preserve relative-path meaning and retain shared model paths; only a v0.11.1
slot's generation keeps a provider manifest bound to that slot's tools. The target's own `check-config` checks the unselected candidate.

Permanent empty `install/slot-A.lock` and `slot-B.lock` files locate shared OS occupancy.
Consumers hold shared locks; materialization requires exclusive occupancy and native exclusion
of older MCP/Tools/ADB consumers. An occupied spare slot stops materialization while downloads
and the current selection remain available. Shared ADB services are not ended to free a slot.
Replaced spare material is retained under `install/retained-<slot>-<time>`.

The Host performs natural drain under its lifecycle admission and commits formal atomic shutdown.
acsetup requires the exact accepted owner, closed owner journal and process exit before changing
selection. Failed status alone does not prove closure: a stopped installation must pass the
formal cold owner/writer gate. Each switch also runs the target's `ledger-maintenance verify`
against current quiescent data. This updates owner.lock epoch/revision; material or budget
failures leave compatibility unproved.

When Runtime was running, acsetup starts the new slot held before Provider, verifies its exact
ticket/process, then releases it through the same Host. Drain, held and release each use a
60-second Host deadline. Controls have a 75-second client limit; cold verification has 150
seconds around the Runtime's existing limits. Unknown submissions are not repeated: acsetup
queries the original transition. Unresolved closure stops the switch. It does not kill an
unresolved control/maintenance process. The Host's released result confirms preparation and
this transition's original user-pause restoration. A previously stopped Runtime stays stopped.
acsetup starts every Runtime in one way: WMI `Win32_Process.Create`,
with `Win32_ProcessStartup.ShowWindow = 0`, starts `cmd.exe`, which runs actingd and appends its
output to `<root>\actingd-<time>.log`. The Runtime therefore has a hidden window, belongs to no
app job and not to the job of whatever ran acsetup, and no window can be closed under it. The
started actingd reads the selection just committed. When it ends at once, acsetup's error
quotes the log's FATAL line.

**Component interfaces**: whether a Runtime, a UI, acsetup itself and the
resource bundles of a run can work together is judged from what each declares, not from a list
of release pairs. A build manifest's `interfaces` object (a bundle's optional zip-root
`interfaces.json`) gives, per interface, the revisions the component reads, `[min, max]`; a
writer writes `max`. The vocabulary, the bump rule and the two checks are the Runtime's
`contracts/component-interfaces.md`: data one component writes and others read (`ledger`,
`install-selection`, `package`) passes when the writer's `max` lies in each reader's range; a
live exchange (`install-control`, `runtime-client`) passes when the two ranges meet, and uses
the highest common revision. The Runtime's configuration (`actingd-config`) has three parties:
acsetup writes it, the console edits it and the Runtime reads it, so the three need one
revision they all speak; two pairwise checks are not enough. acsetup's own declaration is
`crates/acui-setup/component-interfaces.json`, compiled into acsetup and written into the UI
manifest by the build; it follows the Runtime crates this UI is built on. Releases built
before declarations (Runtime and UI v0.11.0 to v0.11.2) are read from a built-in table, and any
other undeclared program is refused; a bundle without `interfaces.json` counts as `package`
[1, 1]. Before any question and before anything
changes, a fresh install, a first migration and an A/B upgrade check the release's Runtime and
UI against each other, against the Runtime they take over and against acsetup; a Runtime laid
into a new slot must also have Tools layout 2 or 3 (a slot holds only the program core). The
release's bundles, a bundle the wizard adds and the instances step's bundles are checked
against the Runtime that will run them and against acsetup. A rollback checks the retained
slot the same way. Every unmet edge is listed, naming the interface, both components and their
ranges; the run stops with nothing changed (exit code 6 on the command line, 1 for
`--rollback`). The negotiated `install-control` revision says how acsetup closes and starts a
Runtime: 0 is the cold protocol, 1 the Host installation transition. The fixed entries in
`<root>\runtime\` and `<root>\ui\` read `install/active.json` as well, so an A/B upgrade
replaces every one whose bytes differ from the release's `acforward.exe`; the previous one is
kept under `install\entries-<generation>\`, and a running entry keeps running from there.

The first migration accepts old-layout programs whose interfaces it can drive (declared, or
v0.11.0 and v0.11.1 from the built-in table), prepares A and closes the old Runtime through the
`install-control` revision both speak — the atomic idle shutdown of the cold protocol for
v0.11.0; Busy leaves the switch uncommitted. Original root programs,
configuration and console settings are retained under `install/initial-backup-<generation>`.
Fixed root entries use `acforward.exe` to pin a complete selection, inherit stdio and return the
actual exit code. Root `actingd.config.json` is an argument alias for the selected private input,
with no writable file at that path. Boot entries, shortcuts and MCP retain fixed root paths.
MCP gets explicit root/state-root arguments; conflicting locations are rejected.

Running UI/MCP/Tools processes retain their own generations and materials. Reopen the process to
use a successor. The v0.11.0 UI is an observation entry explicitly bound to shared state. Its
settings contain the root Runtime entry and config alias; reading the absent alias in its old
editor fails visibly. Effective editing belongs to the fixed acsetup manager. acsetup controls
a Runtime that speaks only `install-control` 0 (v0.11.0) through its actual cold startup
behavior.

**Explicit rollback**: run `<root>/ui/acsetup.exe --rollback` to select the retained other slot.
Its complete manifests are rechecked, and candidate inputs are replanned from current business
settings. After confirmed owner closure, its `check-config` and `ledger-maintenance verify`
must pass against the latest data. These gates do not prove Provider/device readiness. An earlier
release can also be provided through the normal installer with the page's confirmation.
Publication time is only a prompt heuristic; the cold gate remains mandatory. A Runtime that
speaks only `install-control` 0 (v0.11.0) uses cold startup.

Rollback has limits, because a newer Runtime may change the state root for good. Once a Runtime
v0.11.3 or later has written the ledger at revision 2, a slot with Runtime v0.11.2 or earlier is
refused (exit code 1, nothing changed). Once Runtime v0.11.6's frame retention cleaner has run,
Runtime v0.11.5 and earlier refuse that state root. With a large state root, an upgrade from
v0.11.1–v0.11.3 to v0.11.4 is one-way in practice.

acsetup is a windowed program, so a console prompt returns at once when `--rollback` or
`--replace-manager` is typed directly. The work runs without the console (closing the window does
not stop it), and when it ends its result line (or `失败 / FAILED: …`) and the log path appear in
that console. Every run writes its result as the last line of `<root>/acsetup-<time>.log`. To wait
for the result and see the exit code (0 success, 1 failure) in an interactive PowerShell window:

```powershell
$p = Start-Process -FilePath '<install root>\ui\acsetup.exe' -ArgumentList '--rollback' -PassThru -NoNewWindow
$null = $p.Handle; $p.WaitForExit(); "exit code: $($p.ExitCode)"
```

In a script, pass the exit code on instead (do not paste this into an interactive window: `exit`
closes it, and the result line with it):

```powershell
$p = Start-Process -FilePath '<install root>\ui\acsetup.exe' -ArgumentList '--rollback' -PassThru -NoNewWindow
$null = $p.Handle; $p.WaitForExit(); exit $p.ExitCode
```

Before a new Runtime start attempt, only the complete original transaction can restore a
committed selection. First-migration recovery also restores root programs/configuration/settings.
Incomplete restoration retains recovery materials and leaves Runtime stopped. After the first
start attempt, the selected generation and entire ledger remain in place. Any explicit later
rollback rechecks the latest data. Unconfirmed closure never starts a second Runtime.

**Command line**: any argument but `--commit-config`, `--rollback` and
`--replace-manager` runs acsetup without its window. `acsetup --root <root> (--plan | --yes)
[--conflicts new|old] [--associate <alias>=<bundle>/<server>]… [--allow-downgrade] [--online |
--from <folder>]` installs or upgrades; every question the wizard would ask is answered by a flag,
and a question without its flag stops before anything changes. `--plan` lists every change and
difference and writes nothing under the root: its log and its scratch copy go to `%TEMP%`. Exit
codes: 0 done, 1 failed, 2 usage, 3 binding differences without `--conflicts`, 4 a downgrade without
`--allow-downgrade`, 5 an association without `--associate`, 6 incompatible interfaces (see
"Component interfaces"); 2 to 6 stop before the installation changes. `acsetup --help` prints the
whole list.

**Resource-only update**: `acsetup --root <root> --resources <bundle.zip> [--sums
<SHA256SUMS>] (--plan | --yes) [--conflicts new|old] [--associate …]` puts one resource repository
bundle (v2 or v3; a v1 bundle needs a full upgrade) into an existing A/B installation and changes
neither the programs nor the slot. Neither `--online`, `--from` nor `--allow-downgrade` goes with it;
both editions take it, and the offline edition's carried release is then not used. Under the writer
lock it checks the zip against its line in the `SHA256SUMS` beside it (or `--sums`), reads the
bundle, checks the interfaces of the selected slot's programs, acsetup and the bundle, stages and
admits every pack (and qualifies a v3 maintenance declaration), and compares every pack with
`packages\<game>\<digest>\`: present and equal is reused, absent is new, and present but different
stops the run with nothing changed — such a directory is never moved or deleted; rename it aside by
hand or run a full upgrade, which sets it aside. It then plans the bindings the bundle declares
(startup, prerequisite, return-home) with the same association and conflict rules as an upgrade
(exit codes 5 and 3 without their flags), and lists every binding that changes. With no new pack and
no binding change it stops: nothing to do. Otherwise it places the new packs, each through
`<digest>.part`; when no binding changes, that is all — no new generation, no restart. When bindings
change, it prepares a new generation, qualifies the whole chain, runs the selected slot's
`check-config`, closes the Runtime (drain and atomic shutdown when it runs, the cold gate when not),
commits, and starts it again when it was running. A difference in path spelling alone is no change.
The summary names the zip, the packs placed and reused, every binding changed, the generation (or
"configuration unchanged"), the Runtime and the unchanged programs and slot. `--plan` runs the same
checks and writes nothing under the root. A typical run is
`acsetup.exe --root <root> --resources <dir>\<bundle>.zip --plan`, then `--yes --conflicts new`.
In the wizard, step 0 offers the tick "只更新资源（程序不变）/ Update resources only (programs
unchanged)" on an A/B installation (on an old layout it says a full upgrade comes first). Next then
makes the same checks as for an upgrade (the root, the rule against running from a program folder
of it, the log) and opens step 7, "只更新资源 / Update resources": the bundle zip's absolute
path, optionally another `SHA256SUMS`, and "更新资源 / Update resources". The run is the command
line's: it takes the writer lock before it clears leftover staging, and the association page
(step 5) and the conflict page (step 6) answer its
questions; Cancel on either changes nothing. The window cannot be closed until the run has ended.
Success leads to the finish page with the summary above and "启动监控台 / Open console", which
starts the console through the installation's fixed entry; a failure page says what stands.

**Temporary directories**: each run removes its own `.staging-<time>` (the instances step's
`.staging-resources-<time>`, `--replace-manager`'s `install\manager-source-<time>`) once it has
ended, whether it succeeded or not, and logs it; one it cannot remove is a note in the summary and
is removed when the next run starts, as is one a killed run left behind.

**Fixed management entry**: `<root>/ui/acsetup.exe` retains its own original build manifest and
MEMBERS in `install/manager`; business-slot rollback keeps it. To replace it, close the fixed
manager (every `<root>/ui/acsetup.exe` window) and run any verified, unmodified copy of the
release's acsetup except the fixed manager itself, for example the new slot's
`<root>/<slot>/ui/acsetup.exe` or the release's own acsetup:
`& "<root>\<slot>\ui\acsetup.exe" --replace-manager "<absolute-install-root>" "<absolute-release-directory>"`
(in PowerShell a quoted program path needs `&`). An A/B upgrade replaces an older fixed manager
by itself; when it was started from the fixed manager, its summary gives this exact command.
The directory contains normal release files and SHA256SUMS. Native occupancy must prove the old
manager exited; the running executable must match the verified UI payload. Prior management
material and identity are retained. Slot-contained acsetup originals remain intact.

**Progress and installation log**: bounded installation operations go to `<root>/acsetup-<time>.log`.
Runtime results come from formal Host/CLI and GlobalLedger; spawned Runtime process logs retain
startup/fatal lines. Log failures stop the operation. Pages show phase, progress and explicit
errors; failure materials, generations, program backups and hashed resources remain available.
The wizard cannot close while committing installation/configuration inputs. Preparation aims to
reduce downtime; cold verification and startup duration require actual measurement.

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
padding — a certificate table that does not end exactly at the end of the file is damage. Equal is the
online edition; more must be a whole payload: trailer shape, payload start equal
to the image end, index at most 1 MiB and matching its sha256, lengths adding up exactly to the effective
end, names as the online fetch allows and also not starting with `-`, not ending in `.` or `.part`, and
unique without regard to case, `SHA256SUMS` and `MEMBERS.json` read from the payload and matching the
index, and a tag of the old daily shape `build-r<7>-u<7>` matching `MEMBERS.json`'s commits (a
`vX.Y.Z` tag names no commit, so there is nothing to match). Anything else — a truncated file,
a damaged trailer, index or header — stops the wizard on its failure page before it writes anything,
naming what was expected and what was found, saying that nothing has been written and there is no log
yet, and pointing at downloading it again (checked against its `.sha256`) or the online `acsetup.exe`; it
never falls back to the network. The file stays open from that check to the extraction. The index and
`SHA256SUMS` guard against damage, not tampering: trust comes from downloading over HTTPS from the
Releases page and the `.sha256` beside each installer, as for the online edition. Windows SmartScreen
and some virus scanners may warn about a new, unsigned installer with data after its image; that is
expected, not a defect of the wizard. Both installers are assembled for each umbrella release, and the
offline one is read back independently before it is published.

**Things it never does**: it does not install a service, does not create a scheduled task, does not change
PATH, does not write the registry; does not modify the configuration template; does not touch a state root
that already holds content; goes on the network only to list and fetch the umbrella release — the
offline edition not at all; stops or starts the Runtime only on an upgrade, in the
instances step and in a resource-only update whose bindings change, as above;
does not unpack a sealed resource pack — the Runtime loads it (a v1 bundle's packs are taken out whole; a v2 bundle's packs are content directories, laid out file by file as above). On Linux the crate compiles as usual (CI runs `--workspace` on both legs), and running it exits
immediately with `acsetup v1 is Windows-only`.

The Runtime's watchdog is a scheduled task, so acsetup does not set it up. To have a Runtime that is
gone without a formal close (a crash, a closed window, a reboot) started again, run
`<install root>\runtime\actingctl.exe watchdog install --root <install root>` once from a normal (not
elevated) PowerShell of the user the Runtime runs for (Runtime v0.11.3 and later; `watchdog status`
shows its state). Run `watchdog uninstall` before removing an installation. Its rules are in the
Runtime's `distribution/windows/INSTALL.md`.

Seven dependencies are added for the wizard (the offline edition's reading of its own executable is
hand-written and adds none), each with its purpose noted in `[workspace.dependencies]` or the crate's
`Cargo.toml`: `actingcommand-contract` (bundle index v2/v3 and the `content-directory.v1` digest, one
implementation; the console's rev, and it brings only `serde`, `serde_json` and `sha2`),
`actingcommand-execution-kernel` (the offline, hash-admitted pack qualifications, at the same rev),
`sha2` (verification), `zip` (`default-features = false`, only `deflate` enabled, the same version line
the Runtime locks), `getrandom` (the salt and `instance_id`), `ureq` (the fetch; `default-features = false` with only `tls`:
rustls, its `ring` provider and the compiled-in `webpki-roots`, so no system TLS library) and, on
Windows only, `windows` 0.62 (`Win32_Foundation`, `Win32_System_Com`, `Win32_System_RestartManager`,
`Win32_UI_Shell`: the Startup, Start menu and desktop folders through `SHGetKnownFolderPath`, shortcuts
through `IShellLinkW` + `IPersistFile`, and the Restart Manager to find the processes that still hold a
slot's files; the version Slint already locks, so the lock gains no crate).

## MCP for agents

The console has no MCP server of its own. The Runtime's command line serves one: `actingctl mcp-serve`
(Runtime v0.11.0 and later) is a local MCP server over stdio, tools only, for agents such as Claude Code
and Codex. `actingctl mcp-config` prints the registration for a client and writes no client file:

```powershell
& '<install root>\runtime\actingctl.exe' mcp-config --client claude --tier observer
& '<install root>\runtime\actingctl.exe' mcp-config --client codex --tier observer,operator
```

- **Claude Code**: it prints one command,
  `claude mcp add --scope user actingcommand -- "<install root>\runtime\actingctl.exe" mcp-serve --tier …`;
  run it once.
- **Codex**: it prints a `[mcp_servers.actingcommand]` section (the command, its arguments, two timeouts
  and a commented `enabled_tools` example) to put into Codex's `config.toml`.
- On an A/B installation it prints the fixed entry `<install root>\runtime\actingctl.exe`, never a slot
  path, so the registration keeps working after an upgrade or a rollback (Runtime v0.11.2 and later).
  Started through that entry, the server takes the installation's state root from its selection; a
  `--root` or `--state-root` that differs from it is refused.
- **Tiers** (0.11 series): `observer` (reading; always on, and the default), `operator` (devices and
  scheduling), `author` (Lab recording). A tool of a tier that is not enabled answers
  `tier_not_enabled`. `actingctl mcp-serve --list-tools [--format json|markdown]` lists all 22 tools.
- Approvals, edits of the actingd configuration and Runtime restarts stay with the person.
- A running server keeps the installation generation it started with (see "A/B installation and
  upgrade"); restart it in the client after an upgrade.

What each tool takes and answers: `actingctl mcp-serve --list-tools --format markdown`. The manual for
agents is `skills/actingcommand/` in the
[umbrella repository](https://github.com/HS7097/ActingCommand/tree/main/skills/actingcommand).

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
  two language tables in `strings.rs`, and settings-file reading and writing in `settings.rs`. What the
  windows share — the language, the text size, the launcher's view, and the registry of the workbench and
  the ledger windows — is in `ui.rs`, on the event loop only. The
  ledger window is painted from its own state alone (`ledger/state.rs`, `ledger/paint.rs`), never from
  the read face; the read face is opened and held by the window's own reader thread
  (`ledger/worker.rs`), the reads leave their results in the state first, a frame is read and
  decoded on the window's own worker (`ledger/frame.rs`), and the conditional resume is in
  `ledger/resume.rs`.

`slint` 1.17.x, `default-features = false`; the ledger is read-only to the console (what the Runtime
records of the console's own requests — the start press, shutdown and resume requests — it records itself; the
status read appends nothing), the only control entry points are the launcher's two buttons (start /
request shutdown, see above), its owner-unlock entry (a confirmed `actingd unlock-owner`) and the ledger
window's confirmed conditional resume (end of "Launcher"), and there
is no approval entry point; the console crates carry no tests, and CI builds the workspace without running
tests. The launcher is in
`crates/acui-app/src/launcher.rs`, where the paths come from in `install.rs`, the technical log and the
startup-failure window in `report.rs`, and the client operations — probe, request shutdown, recording
the start press, the online open's status and fact reads, and the conditional resume — are in `acui-source` (`probe_runtime` /
`request_shutdown` / `record_start` / `instance_facts` / `resume_paused`).

Every background worker — the probe at startup, the whole start (probe, launch, readiness poll and its
record), request shutdown, unlock-owner, a conditional resume, a frame read, and every acsetup worker (release lookup, install or
upgrade, writing the options, discovery, reading resources, writing instances, settling) — is started through
`std::thread::Builder`. A thread the system refuses is stated, with the OS error, where that action
reports, and what the worker would have cleared (the probe, start, shutdown, unlock or resume in flight, the save
in progress, the frame request) is reset: never a panic on the event loop. A start whose thread cannot
start launches nothing, and the line says so. A child that was killed but could not be reaped is
said as that, not as a kill that failed.

Two more crates sit outside these four layers. `acui-setup` (binary `acsetup`) is the setup wizard; it
depends on none of the four layers; besides `acui-installation` it uses Slint, `serde`, `serde_json`,
`anyhow` and the dependencies listed under "Setup wizard acsetup", the shared contract and
execution-kernel's offline package qualifications among them.
`acui-installation` provides the shared installation-selection consumer for the console and the wizard,
and the binary `acforward`, the fixed entry.

## Icon

The application icon is the black single-figure "commander" mark; the assets are in
`crates/acui-app/assets/`: `acui-256.png` (256×256 transparent PNG) and `acui.ico` (multi-size, 16..256).

- **Window and taskbar icon**: `Window.icon: @image-url("../assets/acui-256.png")` in `app.slint`.
- **Executable icon**: in `build.rs`, `#[cfg(windows)]` calls `winresource` to compile `acui.ico` into the
  exe's resource section; this dependency hangs under `[target.'cfg(windows)'.build-dependencies]` and is
  not compiled on Linux.
- **acsetup**: the same assets, not copied: `crates/acui-setup/build.rs` and `ui/setup.slint` point at
  those two files in `crates/acui-app/assets/` with relative paths.

## What the read face blocks

These are not worked around; they are displayed as they are, and recorded here. File references are
to the Runtime source at the pinned rev.

- **Event and repair counts: resolved offline; online, the event count only**. `GlobalLedgerMetadata`
  (`crates/ledger/src/global/evidence.rs`) now states `event_count()` and `repair_count()` from the
  authenticated metadata, without verifying any material, so the console no longer needs
  `GlobalLedger::open_evidence` (same file) for them. The instance card shows both, and the
  number of rows **loaded in this view** separately; the two are never mixed. Over an incomplete read the
  event count covers only the verified prefix, and the card says so. The SQLite medium has no repair log
  (`None`), and the card says so instead of showing 0; the repair log does not share the event snapshot's
  sequence boundary. Online, the event count is the pinned position, as
  `contracts/runtime-state-observation.md` states (sequences are gap-free from 1); neither the page
  (`LedgerReadScope`) nor `runtime-info.json` states the repair count, so that line says the Runtime does
  not state it.
- **Whole-material read: resolved on both faces**. `read_material_complete`
  (`crates/ledger-forensics/src/material.rs`) reads one whole object: fresh ledger metadata twice, one
  reader, one whole-file hash, bounded by `max_material_bytes` and a deadline. The offline face calls it
  with the 8 MiB frame limit and a 30-second deadline (no Runtime caller of it sets one yet; the
  contract's 4-second `RUNTIME_MATERIAL_READ_BUDGET_MS` bounds a single range read, not a whole
  object). Online, the typed client's `RuntimeClient::read_material_complete`
  (`crates/runtime-client/src/client.rs`) gives the same result shape over verified ranges, and the
  console calls it with the same limit and deadline; the Runtime still verifies the whole material for
  every range (a 3.6 MB frame is 19 ranges of 192 KiB).
- **Instance facts: resolved on both faces, at different positions**. Online, the fact store is read
  through `RuntimeClient::runtime_fact_snapshot()` (`crates/runtime-client/src/client.rs`), which
  answers at the Runtime's latest position, past the pin. Offline, `runtime_facts_at`
  (`crates/ledger-forensics/src/runtime_facts.rs`) replays the store at the pinned position itself,
  under the Runtime's own replay rules; the console never folds `runtime.fact_*` events itself. Lease
  state comes only from the online status read, so offline has none.
- **Geometry and frames cannot be brought together on two early state roots**. In two early real-device
  state roots the console was first checked against, the only events carrying a `capture.frame`
  artifact are `artifact.created` / `artifact.verified`, and their payloads hold no geometry; the only
  events carrying geometry are a handful of `task.effect_intent`, whose payload is a single tap
  coordinate and whose `links` hold **no** `frame_id`. The ledger gives no relation joining the two, so
  the console does not join them — the real frame is drawn as it is, and the overlay is empty. At the
  pin, `task.effect_intent` can state the frame extent its coordinates are in (`frame_extent`,
  `crates/actingcommand-contract/src/event/payload.rs`) and `task.geometry_observed` its frame's extent
  (`TaskGeometryFrame::extent`, same file); the overlay canvas uses that extent when an event states
  one. The effect intents on those two roots state none, so their size stays "not recorded".
- **Neither of those roots holds artifact eviction facts**, so the eviction placeholder does not appear
  on them; the code path is written to the contract.

## License

`GPL-3.0-only`. The repository includes the full LICENSE text; the workspace `license` field and the SPDX
header of every `.rs` / `.slint` file agree with it. The interface is rendered by
[Slint](https://slint.dev), used under its GPLv3 licensing option. The Runtime crates depended on
(contract / ledger / ledger-forensics / runtime-client / execution-kernel) are `AGPL-3.0-only`, and the
two combine under GPLv3 section 13.
