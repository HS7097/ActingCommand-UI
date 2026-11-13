<p align="right">🌐 <b>English</b> · <a href="./README.zh-CN.md">简体中文</a></p>

# ActingCommand Console

**⚠️ Pre-release, debug phase: interfaces, configuration and file formats still change, and only the newest version is supported. See "Development status".**

This is ActingCommand's **human console**, a read-only native program. On one Runtime state root it opens
the ledger's **official read face**, and renders the view pages the ledger itself hands out into a
three-column interface: instance card, timeline, detail pane.

It is not part of the Runtime; it is the Runtime's **external, detachable client**.

> The `acsetup.exe` in the builds is the newly written 0.12 setup wizard (UI-W2): it installs the Runtime,
> its tools and the console from an install set into one new folder; see "Setup wizard acsetup". The UI
> releases published so far do not carry it; until one does, install with `actingctl install`.

This repository also holds the **setup wizard** `acsetup` (see "Setup wizard acsetup") and the fixed
entry `acforward`, through which an A/B installation starts its programs. Like the Runtime, these
programs know no game: they hold no game logic and no game data. A game is supported through the
resource packs (one standard pack per game) the Runtime loads; the wizard reads what a pack declares
and guesses nothing.

## Current release

- **The 0.12 series (UI v0.12.0 and later)**, pre-releases like every release so far:
  `acui-windows-<sha>.zip` with `SHA256SUMS` on this repository's
  [Releases](https://github.com/HS7097/ActingCommand-UI/releases). Every release is a pre-release, so
  GitHub marks none as "Latest": take the newest tag. The zip holds `acui.exe` (the console),
  `acforward.exe` (the fixed entry), `LICENSE`, `README.md` and the `BUILD-MANIFEST.json` that binds them
  to their commit; the releases published so far hold no `acsetup.exe` (branch builds carry it, see the note above). Each release is built
  against the crates of the Runtime rev its commit pins (see "Data source"): UI v0.12.15 against those
  of Runtime v0.12.47; the pin is now v0.12.58.
- **Works with the 0.12 Runtime**: it declares `actingd-config` [5, 29], `ledger` [3, 8],
  `runtime-client` [3, 8] and `runtime-outcomes` [1, 1]. It fits Runtime v0.12.47 to v0.12.58: each of
  them declares a `ledger` maximum within [3, 8] and an `actingd-config` and a `runtime-client` range
  that meet the console's. The ledger of v0.12.55 and later (revision 8) is one way: once such a Runtime has run on
  a state root, no Runtime before v0.12.55 opens it, and a console whose `ledger` range ends at 7 may fail to
  read it, so switch the console first, then the Runtime. Whether another Runtime fits is judged from
  the interfaces each side declares, not from version numbers (see "Component interfaces").
- **The 0.11 series**: UI v0.11.4 and earlier speak the 0.11 protocols only and cannot connect to a
  0.12 Runtime. The 0.11 series is not maintained.
- **What changed**: each release's notes on its Releases page say what changed in it.
- **Umbrella repository**: its newest release, v0.11.4, still carries the 0.11 series (Runtime v0.11.4,
  UI v0.11.3) and its installers; no umbrella release carries the 0.12 series yet.

## Development status

- **Debug phase**: the main loop is find a problem → fix it → check against what was expected → change
  again. Deployment and availability come second.
- **Every release is a pre-release**: interfaces, configuration and file formats still change. **Only the
  newest version is supported**; older series get no fixes (the 0.11 series is not maintained).
- **Runtime 0.12**: the Runtime's 0.12 series brought a new ledger (a 0.11 state root is not carried
  over), new command-line output and exit codes and a new pack format. The console follows it from UI
  v0.12.0 on (see "Current release").
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
- The position this session reads at — the pin — is the ledger head the Runtime states in memory: the
  `ledger_committed_head` of its status read, or, when that read fails or states no source, the
  `ledger_position` of its fact snapshot (both read at open, below). No page is asked to learn it, and
  neither read appends a ledger record. A person's jump to the latest moves it by reading both again the
  same way; turning following on, and each poll while following, move it to the position the Runtime's
  fact snapshot states (below). Offline it never moves. Every page is
  `RuntimeClient::query_event_page(query, ProjectionProfile::Ui, page.at_snapshot(pos))`,
  the same `EventQuery` bounded to one window, the same page limit, the same `next_cursor`. Online the
  console sends no page that does not carry both `from_sequence` and `to_sequence` within 1 to the pin,
  that spans more than 4096 positions (a frame's lookup: 65,536 either side of its event), or that
  carries a time condition — with one, the Runtime may read through its time index, where the position
  bounds do not hold the scan. A read asking for such a page would be a defect of the console: it fails as
  `console_online_page_unbounded`, and nothing is sent.
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
  (PID, owner epoch, start time, all out of its own `runtime-info.json`). "Ledger integrity" says "not
  read yet" until a page at the current pin has come back — again each time the pin moves — and then
  whether every page read back at the current pin was whole; offline it is the opening's own observation.
- At open, one `status()` and then one `runtime_fact_snapshot()` on the same connection give the pin
  (above) and the instance card its "runtime instances" lines. Status is read again only on a person's
  jump to the latest and, on a connection of its own and without moving the pin, after a resume a person
  sent ("Conditional resume", below); while following, every poll reads the task facts again, whether or
  not the pin moved. The first two lines state the sequence each read was taken at; the fact snapshot may be past the
  pin taken from the status, and while following the status stays where it was read as the pin moves on,
  so this is state as last read, not state at the pin. By contract the status read takes the Runtime's memory only and
  appends no ledger record (AO-4q); its position is the ledger's committed head when it was sampled. Then,
  per instance the status registers: its alias (the instance id in grey), port, lease (leased / takeover
  cooldown / idle, plus the queued request count when there is one), and the instance facts
  `task.game`, `task.server` and `task.page` (the page label the last recognition matched) verbatim, or
  "not recorded". A fact whose id the status does not register gets a row that says so. If either read
  fails, the lines state the Runtime's refusal, the client error and any host failure, one per line;
  the session still opens as long as the other read states a head, and does not open when neither does
  (below). Offline, the read face's `runtime_facts_at` replays the fact store at the
  pinned position itself, the same position every page reads at: the lines state that position, that
  there is no lease offline, and each instance's id (short; the full id in grey) with its three facts. A
  ledger with no event yet says so; a replay the read face declines states its reason, and a failed one
  its code, operation, io kind (for an io error) and detail, one per line. The replay runs once as the
  session opens (after the window shows, on the window's reader thread), bounded by a 30-second deadline.

`--source <auto|offline|online>`, default `auto`: if the client can connect to the Runtime the state root
points at, online; otherwise **no ledger is opened**: `auto` never opens the offline face by itself, which
would open the local ledger (on a production machine, the big one under the state root) without anyone
asking. The window then reads nothing of the ledger — no page, no material, no port map, no fact
replay. The instance card's first line, "read face", says why (`runtime-info.json` absent, or the client
error code and operation of the failed connection); the list says no events were read, in red when the
connection failed, and the controls that would query the ledger are off, as for an unopened face (below);
the top bar offers "Reopen", which tries online again, and a button, "Open Offline (Read-Only, for a Copy
or a Test Root)", with the state root whose ledger it would open written beside it. Only that press opens
the offline face, in the same window, as a new session of `offline`; the card's "read face" line then says
it was opened by that press. `--source offline` stays the explicit choice and opens the offline face
directly. With
`online`, a failed connection **is an error carrying the client's error code**; it does not quietly
switch to offline. Across both read faces, a Runtime that connects but states no ledger head — neither its
status read nor its fact snapshot — is an error in any mode, with both reads' reasons and no fallback. Neither error ends the console: the window stays, the list and the
instance card state in red why the read face did not open, and "Reopen" is offered.

When the offline read face cannot be opened — with `offline`, or after the press of "Open Offline" — the
console does not exit: the window shows the face **unopened**, which is what a fresh state root looks like
to the offline face, since the Runtime creates the ledger on its first start. The instance card then has only its first line, "read
face", which says the ledger could not be opened and states the read face's own `code`, `operation` and
`detail` verbatim, and for an io error its io kind (`LedgerIoKind`: `not_found`, `permission_denied`,
…). The kind, never the localized `detail`, tells "no ledger yet" from "a ledger that cannot be read":
only for `ledger_io` with io kind `not_found` does the line add that the state root has no ledger yet,
most likely because the Runtime has never been started here, and point at the launcher's Start. No ledger
fact is shown, not even a zero: the list says the ledger is not opened instead of standing empty; the
tabs, the filter boxes, the id box and the time slider are off; the module and port boxes and the frame
pane say "ledger not opened". Nothing is asked of the ledger and no material is read. The launcher works
as usual; once the Runtime has started and made the ledger, "Reopen" opens the read face again. A window
of the default source (`auto`) that reads nothing because no Runtime was there, or that reads offline (or
whose offline face is unopened) after the press of "Open Offline", offers "Reopen" as soon as the
launcher's probe finds one running, with a line saying it can read online now, and that reopen reads
online; while no Runtime is found, "Reopen" opens the same face again (after the press, the offline one).
A window `--source offline` asked for keeps reading offline. Neither switches read face mid-session on its
own.

**The reader thread**: the window comes up first, and the read face is opened afterwards on the window's
own reader thread (`acui-ledger-<window id>`). Opening, page reads, reading earlier, following, the jump
to the latest, the time cursor's point reads and the frame's own events are all read there, one request after
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
filters the boxes show now, without the time cursor. When a read reaches a Runtime that is another process now
(`console_runtime_owner_changed`), the top bar says so beside "Reopen", which reads the Runtime that runs
now.

The committed time span — the time of the ledger's first event and of the event at the pin, which the
time slider runs over — is read after the first fill, not before it, and once per pin. The first event's
time is one point read of position 1, kept for the session; the pin's is taken from a window just read
when it holds the event at the pin, otherwise from one point read of the pin. Each is a page of limit 1
bounded to that one position, without run recovery. Only when position 1 holds no event is one more page
asked, of positions 1 to 4096 (or to the pin, if lower), still of limit 1. A span read follows no cursor
and reads no wider: a page that comes back without the event it must hold — cut short by the Runtime's
read budget, or with none in its range — stops it, as `console_time_span_page_limit`; the span stays as
it was, and following does not ask it again by itself. A ledger whose pin is 0 has no span, and no page
is asked for one. The span's two ends are also where locating the time cursor starts: position 1 with
the first event's time, and the position the span was last read at with its event's time (see "Time
cursor" below); while no span has been read, the slider stays at "all".

**A page the Runtime refuses** — its read budget ran out (`ledger_read_budget_exceeded`; right after it
starts, while it still builds its Lab relations, many pages are refused so, most often on the Lab tab) or
its writer did not answer in time (`writer_response_timeout`) — is stated with its code and why, and the
window never sends it again by itself. What else stops depends on the page: a timeline page that fails —
a window the fill reads, or one following reads onto the top — ends the fill there, stops following, and
no span page is asked after it; a point read locating the time cursor ends it the same way, before any
window is read; a span page or a frame's lookup that fails stops nothing else, and
following, if on, goes on without asking that page again. The line says how a person reads again, which
depends on where it failed:
on the timeline or the span, Read Earlier, a tab or filter change, Jump to Latest, or closing and
reopening the window; while following, ticking Follow Latest again, Jump to Latest, a tab or filter
change, or closing and reopening; for a frame's lookup, Jump to Latest, a tab or filter change, or closing
and reopening, then selecting the row again; for a point read locating the time cursor, where no window
was read and Read Earlier is not offered, moving "Show up to" again or a tab or filter change, which
locate it again, or its handle back at the far right ("all"), Jump to Latest (online) or closing and
reopening, which read without it.

Dependencies are pinned to one exact Runtime source (`Cargo.toml`), Runtime v0.12.58
(#434 LAB-6S1a, v0.12.58: `runtime-client` revision 8 adds an optional `initiator` to `BindChannel`, to
its answer `ChannelBound` and to every receipt of a connection that declared one, taken only on the `lab`
channel and never written on `ui`, so this console, which binds `ui` and declares none, sends and reads
every message byte for byte as under revision 7 and pairs with a Runtime of [3, 7] as before; #434
LAB-6S3, v0.12.58: on a connection bound to `ui` the Runtime takes only the request kinds of its UI list,
and a `ConfigureSettings` only of keys whose console tier is `daily` or `advanced` and that are settable,
refusing anything else `channel_request_refused`; every kind this console sends is on that list and every
key it sets is of those two tiers; #434 LAB-6S1b, v0.12.58: the contract crate no longer judges a request
by its envelope's origin beyond the set of client origins, and the ten `invalid_…_origin` codes of the
gates it dropped are retired; v0.12.56's configuration
revision 29 adds `scheduling.task_held` (`enabled`, `after_ms`), also per instance (#438 HELD-1), and an
`instances[]` entry's `device_group` (#437 MAPP-1, read at assembly, not settable), every new key `hidden`
in `ui_tier`, so the settings page shows no new row; ledger revision 8 does not move (no record carries the
initiator), and the ledger crate's run recovery no longer counts a record whose outcome code is of the
ladder class `planned_stop` (a stop on request, a pause) as a failure of its group for its severity alone
(#429 RG-STOP, v0.12.58); in the outcome-code catalog
it embeds twelve codes (among them `task_held`, `task_held_ended` and four `device_group_…` codes), seven
keys, one location, the vocabularies `device_group_problem`, `device_group_wait`, `held_end` and
`held_state` and new tokens of `argument_problem`, `install_caveat`, `install_refusal` and
`standard_pack_problem`, with those ten origin codes retired, none removed and no `console_` or `setup_`
name; `package` revision 10 (#444 NOTICES-2, a bundle's `notices/`) is acsetup's; policy, which
runtime-client links, gained device groups (MAPP-1); ledger-forensics, artifact-store, runtime-database
and selection-policy have the same source, and the defaults file runtime-config and ledger-forensics embed
only gained the `task_held` defaults and the ladder profile `without_emulator_restart`, which
ledger-forensics does not read; the Runtime's `Cargo.lock` only gained its own two new programs
`actinglab-mcp` and `mcp-entry-g2`, and no external crate moved;
v0.12.55's #428 S168-LB1a: a `fact.published` record's `record_list` may hold up to 1024 rows
(`MAX_FACT_RECORD_LIST_ROWS`, 256 before), which ledger revision 8 adds, one way (once a Runtime that
declares it has run on a state root, no earlier Runtime opens it), and `runtime-client` revision 7 lets an
`InstanceFactSnapshot` answer carry such a list, its request an optional `key_prefix` and its answer an
optional `records_total`; the console sends no `InstanceFactSnapshot`, and an online ledger page's public
projection of a `fact.published` carries no rows, so a long list never reaches it online, and offline the
pinned crates read one; #429 R-6b (R-6b1, R-6b2) joins those two revisions, raising nothing:
`policy.execution_recorded` gains a third result kind, `stopped` (`outcome`, `runtime_ms`), the
settlement of a scheduled run a `StopInstanceRun` stopped, beside its `task.failed` terminal with
`contained_task_cancelled`, which a console built on v0.12.53 cannot decode (offline the page that holds
it fails whole, online `runtime_receipt_decode_failed`) and this one decodes; from v0.12.55 the Runtime
stops a scheduled run too, asked by actinglab `task stop` or the Lab's tool, while MCP `ac_stop_run` and
actinglab `run stop` still refuse every scheduled run, and this console's button for a scheduled run is
Close This Task, which closes the task and then stops this run (#429 UI-3e3, UI-3e2b, see the rescue bar
under "Launcher"); v0.12.54's configuration revision 28 adds the value `auto` of `idle.action` and the keys
`idle.return_home_min_gap_ms` and `idle.stop_emulator_min_gap_ms`, each also per instance (#386 IDLE-2),
and `instance.<instance id>.scheduling.pool_lines` (#428 SCHED-C1e), every new key `hidden` in `ui_tier`,
so the settings page shows no new row; the settings page shows `auto` as "choose by the idle time", and
an account's rules panel says "It is now “choose by the idle time”, which is not offered here" and does
not offer it; the requests `PackScheduleView` and `EditPackSchedule` gained the success codes
`request_pack_schedule_view_completed` and `request_edit_pack_schedule_completed` (#429 R-8), and the
console reads an account's `PackScheduleView` and writes `EditPackSchedule` of the account's own scope
(the Daily Tasks switches, below, and the rescue bar's Close This Task, which writes the same row); `contained_task_state_invalid` says what was invalid with the optional values
`state_problem` (vocabulary `state_problem`: `held_state_unusable`, `no_move_from_page`,
`runtime_invariant`, `transition_refused`), `phase_id`, `package_id`, `page_id` and `pack_operation_id`
(#428 DIAG-SI), `runtime-outcomes` still 1; in the outcome-code catalog it embeds five codes (those two
success codes, `consumption_fact_unusable`, `policy_fact_rows_exceeded` and
`selection_decided_on_lower_bound`), nine keys, the vocabularies `consumption_source`, `lines_source`,
`list_max_bound` and `state_problem` and the token `list_bytes_exceeded` of `reading_problem`, with no
code retired, removed or re-categorised and no `console_` or `setup_` name; the ledger crate pairs a
stopped run's record with its `task.failed` terminal and recovers such a run as `stopped` (R-6b); policy
and selection-policy, which runtime-client links, gained the lines of a pool per instance (SCHED-C1e), a
run's consumption read from a fact (#428 SCHED-C1f1), a no-progress guard held only for its own cause
(#428 SS-4b) and the lower bound of a list maximum (#428 S168-LB1b); runtime-client gained the read by
key prefix and reads a stopped run's summary;
ledger-forensics, artifact-store and runtime-database have the same source, and the defaults file
runtime-config and ledger-forensics embed only gained the two idle defaults (600000 and 7200000 ms),
which ledger-forensics does not read; the Runtime's `Cargo.lock` is unchanged and no external crate moved;
v0.12.53's #429 R-6: the contract crate gives the request `StopInstanceRun` its success code
`request_stop_instance_run_completed`, so a console built on it checks the receipt of a stop a v0.12.53 Runtime
completes, which one built on v0.12.47, where the kind had no success code, would mis-check; a stop refused for a
run the scheduler or the Runtime itself started, `scheduled_contained_task_not_client_cancellable`, names who
started it with the key `run_origin` (vocabulary `run_origin`: `policy`, `host`); from this version the
console's Stop This Run on the rescue bar stops a run another tool started (see after Pause This
Account); since v0.12.47 the interfaces the console meets have not moved (`actingd-config` [5, 27],
`ledger` [3, 7], `runtime-client` [3, 6], `StopInstanceRun` being a revision-3 kind; `package` [3, 8],
since v0.12.50, is acsetup's), the contract crate otherwise only gained the runtime fact
`config.catalog_provenance` (#428 SCHED-C1b), the list readings under `inventory.list.` (#428 S168-4), a
`schedule.json` rule's fixed value (#428 SCHED-C1a) and the names a replacement in place leaves in `tables/`
(#428 TB-R1), none of which the console reads, and in the outcome-code catalog it embeds ten codes (among them
`request_stop_instance_run_completed` and ISO-START-1's `policy_settlement_facts_skipped`), seven keys, nine
locations, the vocabulary `run_origin` and new tokens of `install_caveat`, `install_operation_kind`,
`install_refusal`, `install_step_kind` and `reading_problem`, with no code retired or removed and no `console_` or
`setup_` name; the ledger crate no longer lets a time bound drive a page's statement (#422 QP-1: the same rows in
the same order), settles a run without a terminal whose one release is its own as interrupted also when it wrote
effect or input records (#428 UNSET-2), and adds `host.capped_episodes` to the default
`ledger_observer.tag.workbench_patterns` (#428 SCHED-C1d), at ledger revision 7 still; runtime-config's source
changed only in a test and the defaults file it embeds only by that same pattern, so the key table's keys and
`ui_tier` are v0.12.47's and the settings page shows no new row; policy and
selection-policy, which runtime-client links, gained the capped spending order's trigger line and its episodes
kept across a start (SCHED-C1c, SCHED-C1d), fixed values (SCHED-C1a) and a table maximum over a list
(#428 S168-5a); runtime-client, artifact-store and runtime-database have the same source, and so has
ledger-forensics, whose embedded defaults file differs only by that same pattern, which it does not read;
v0.12.44's #407 GPU-1: configuration revision 27 lets the `vision` section name DirectML
(`vision.execution_provider` `directml`, `vision.directml_device`, `vision.gpu_fallback`); the section takes
effect at the next Runtime start, is no key of the key table, and the console neither writes nor interprets it,
so the settings page shows no new row and a file that names them is within the console's `actingd-config`
[5, 27]; since v0.12.43 the contract crate otherwise only gained value keys, the runtime fact
`config.catalog_warnings` (#428 CW-1) and the OCR node placement report v2, none of which the console reads, and
in the outcome-code catalog it embeds the warning `vision_gpu_unavailable`, the scheduling codes `pool_line_held`,
`pool_lines_invalid`, `pool_lines_unvalued` and `capped_spend` (SS-2h), the Lab codes `lab_operation_check_failed`
and `lab_operation_completed`, the vocabularies `gpu_unavailable_reason`, `gpu_fallback` and `pool_line_hold` and
the provider `directml`, with `watchdog_launcher_spawn_failed` and `resource_check_release_retired` retired;
`runtime-client` reads official OCR evidence on DirectML and keeps the Runtime's answer to a Lab operation whose
check failed, neither a request the console makes; policy gained the business tier's turn across instances
(SS-2f) and pool lines with the capped spending order (SS-2h); ledger, ledger-forensics, runtime-config and the
other Runtime crates the console links are unchanged;
v0.12.43's #404 FS-3: ledger revision 7 adds the confirmations `tracked` and `track_lost` of a `task.selection_evaluated`
record, which only a select step of a package declaring `confirm: "track"` writes and which a console built on
revision 6 cannot read, so the ledger page that holds one fails whole; since v0.12.40 the contract crate
otherwise only gained the performance status's optional members `memory` and `vision`, #407 PERF-READ,
`runtime-client` revision 6, a status the console never asks for, and in the outcome-code catalog it embeds the
warning `perf_memory_unavailable`, the value keys `confirm` and `shift_px` and the new vocabulary `selection_confirm`; artifact-store's frame stores
count what they hold for that status, and the other Runtime crates the console links are unchanged;
v0.12.40's #402 HS-5: a switch to MuMu native touch no longer needs the game running, and a fence that the
device's own backend open caused no longer refuses a switch, so "Use File Value" on the settings page can lift
it once the file's backends pass their self-check; v0.12.39's ledger revision 6,
#402 HS-2b, lets the console read the lifecycle record
`device.backend_switched` the Runtime writes when it switches an instance's touch and capture backends while it
runs, which a console built on revision 5 cannot read; the settings page's "Device Backends" group of each
device switches its two backends while the Runtime runs, #402 HS-1 to HS-4, and waits for a switch's answer as
long as the contract's `BACKEND_SWITCH_WAIT_MS` and the client's IO margin say;
with v0.12.38's frame reads off the ledger writer, #422 RA-1, and Workflow #432 AGT-1b's optional status member
`agent_wake`, which the console reads and does not show, on top of #429 R-0b, R-4, R-11, R-7a and 114-A; it keeps
v0.12.32's bounded offline opening), which speaks the 0.12 protocols (`runtime-client` revision 8, ledger revision 8):

```
rev = "ef94d950a102a595fa0bde974b8d77ad54329b48"
```

The six crates (contract / ledger / ledger-forensics / runtime-client / runtime-config / execution-kernel) share this one rev.
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
  `correlation_`/`request_`/`run_`/`task_`/`instance_` id are assembled into one
  `EventQuery` — never a time condition, on either read face: the time cursor is where reading starts,
  not a filter (below) —
  and the ledger is queried again at the **same snapshot position**; it does not filter the
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
  and the instance card states that code too; under the online read face the same box picks an instance
  instead (next item). With a port selected, the instance card additionally states the instance alias,
  the port, the source (`physical_device` / `fixture_simulation` verbatim on the lower layer), the number
  of bound ids and the sequence number of the most recent binding; the severity counts and the row count
  still come from the re-queried rows. Rows gain a "port" column: an event with no instance
  association says "host"; one with an association whose id's most recent binding gave HOST:PORT says the
  port number; one with an association but absent from the port table (fixtures, serial-port
  configurations, no binding seen) says the abbreviated instance id — no port is invented.
- **Instance filtering online**: online there is no binding read, so the same box lists the instances
  the Runtime's status read reports now: "all instances" first, the default, then one item per instance
  in the status's order (by alias) — its alias, and " · port N" when the status states that instance's
  ADB port; a serial-configured instance, or one without an ADB target, gets its alias alone, and no port
  is invented. Only the ids that status reports are listed; the ids a port was bound to earlier belong to
  the offline read face. Picking one queries the ledger again with that one id as
  `EventQuery.instance_ids`, every page bounded as every online page is (`from_sequence` and
  `to_sequence`, limit 256, at the pin), for the timeline, "read earlier" and following alike; "all
  instances" clears it. The list is made again at every status read: at open, on a jump to the latest,
  and after a resume a person sent; following reads no status and leaves it as it is. When that read no
  longer reports the picked instance, or it or the fact snapshot read with it fails and so cannot confirm
  it, the pick goes back to "all instances", the filter error line names the instance, and the rows read
  under the pick go: the view is read again from the pin (after a jump to the latest that could not move
  the pin it stays empty, beside the jump's error, until a person reads again). A pick carried into a
  reopened window is checked the same way. A status read that reports no instance leaves the box saying
  so; a failed status or fact snapshot read leaves it saying the instances were not fully read (status or
  fact snapshot), see the instance card; either way the box is disabled, and the id box still takes an
  `instance_` id. A picked instance and an `instance_` id are mutually exclusive, as offline.
- **Reading from the latest end**: the ledger query has no descending order, so the timeline reads
  backward windows down from the pinned position, starting at 256 positions. A window that size holds no
  more events than one page's event limit, so one query reads it, every filter applied by the ledger. A
  window that comes back with fewer than 64 events doubles the next one's span, up to 4096 positions; one
  with 128 or more puts it back to 256. A wider window, and any the reply's byte limit or the Runtime's
  read budget cuts short, is read by following the cursor, whole — so one fill can add more rows than it
  aims for — in at most 64 pages; a window still going on past that stops, as `console_window_page_limit`,
  and is not shown as read. Online, every page query — the window's `from_sequence` and `to_sequence`,
  a limit of 256, at the pin — is answered by the Runtime's ledger reader pool, off its writer (#422 QR-1; a frame's artifact lookups
  too since v0.12.38, RA-1), and still costs the Runtime a read and a verification per page, so one fill
  reads at least one window
  and starts no further window once it has 256 more rows, has read position 1, has read 16 windows or has
  run for one second. Rows are listed
  **newest first**; "read earlier" at the bottom continues below what is loaded. The top bar permanently
  states the positions the loaded windows cover ("read positions A–B"), with "source incomplete" appended
  when a window's page said its read was incomplete — except a page that only the Runtime's read budget
  stopped (#411 A51): its cursor goes on, and the window follows it. At a pin of 0 the ledger holds no
  record: no page is asked, and the top bar says the ledger is empty. Two lines of the instance card count,
  over the loaded windows: "Pages Read", the windows and pages read, then ("locate N") the point reads,
  one page each, that located the time cursor this view starts from (0 to 8; 0 when it was located before
  at this pin, or no cursor is set); "Pages Cut", the pages the read budget
  cut short (`read_budget`; their cursor read the rest) and the pages stating a run whose recovery context
  the budget cut (`context_limit`): counted, never dropped. The span's own point reads are not counted on
  the card.
- **Jump to latest, follow latest (online only)**: "jump to latest" reads status and facts again —
  neither read appends a ledger record, and no page is asked — moves the pin to the head they state
  (above), clears the time cursor and starts the view over from the new pin; when neither states a head,
  the pin stays and the view's error says both reasons. "Follow latest"
  first catches up with one fact-snapshot poll, without the status read, then every 5 seconds reads the Runtime's fact
  snapshot — its in-memory fact store, stated at the ledger's latest position, which costs no ledger read
  and writes nothing. Only when that position has moved does the pin move to it and are the windows
  between the old and new pin read onto the top of the view, with page queries; the loaded rows, the
  selection and a frame being read stay, and the task facts come from that same snapshot. The span's end
  follows the pin: taken from the newer windows when they hold the event at the pin, otherwise one more
  point read of the pin; a span read that failed is not asked again by a tick. A failed tick shows as
  "following latest: …" beside the view's own error; a failed poll stays until a later tick gets past it,
  and a failed page read also stops the following — a Runtime refusing the query is not asked again every
  five seconds — until it is turned on again; so does a page that failed a fill.
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
- **Time cursor (both read faces)**: "Show up to" picks a ledger **position** to read down from, never a
  time condition; no page of the window, online or offline, carries one. The slider's starting position
  is the state it represents — the far right is "all", the view reads down from the pin — and the first
  drag narrows. The handle picks a time between the committed span's two ends (above); the label follows
  it at once as "≈ that time", and following stops. Once the handle has rested for 0.4 s (a tab switch or
  "read earlier" meanwhile goes first), the view locates that time as a position: at most 8 point reads,
  each one page of limit 1 bounded to one position (`from_sequence` = `to_sequence`) between position 1
  and the position the span was last read at, at the pin, without run recovery, every one within the
  online bounds above. Each read goes where the time would fall if the events between the two nearest
  times known on either side were even in time, or halfway when those two times are equal or out of
  order; the position whose event's time is nearest the time picked wins, and an exact match ends the
  reads early. The view then reads down from that position in the usual windows, and the label shows
  that event's own time, "≤ time". The same time at the same pin is not located again: a tab or filter
  change reads from the same position with no point read. While locating, the busy line says which page
  of 8 it is ("Locating Show Up To: Page n of 8"), and the card's "Pages Read" line counts them
  ("locate N"); the busy line and the error name the control by its caption, "Show Up To".
  **The position is approximate**: a
  record's time does not always follow its position (#411 A24 — a run's diagnostic artifact records are
  appended when the run ends but carry its start time), so a point read can land on such a record and
  pull the search aside; the label's time is what was actually read there. Offline this differs from the
  earlier time upper bound, a time condition on each query: events below the position with a time after
  the one picked are no longer hidden. A point read that comes back without the event its position must
  hold — cut short by the Runtime's read budget, or with none there — stops the locate, loudly, as
  `console_time_locate_failed`: the view reads no window from a guessed position and stays empty with
  that code until a person reads again; a point read the Runtime refuses (above) stops it the same way.
  Read Earlier is not offered then, as no window was read: the line says to move the handle again or
  change the tab or a filter, which locates it again, or to drag it back to the far right, Jump to Latest
  (online) or reopen, which read without it. The handle back at the far right, a jump to the latest, following
  turned on and a reopen all clear the cursor.
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
frame. Online that query also carries the instance the event naming the frame links, when it links one,
and reads only the positions within 65,536 either side of that event, within the pin — a capture's
diagnostic records are appended when its run ends, so they may lie well after it; when that window holds
no capture, the note says in which positions it looked ("no capture of this frame between positions A
and B"), not that the ledger holds none. Offline it is the frame id alone, as before. A failed or
incomplete read is stated in the frame note under the frame and is never asked again by itself: the next
reload (not "read earlier") clears it, and selecting the row again reads it again. The material reader is the one the reader thread hands the frame pane; when a frame read latches
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
acui [--root <install_folder>] [--state-root <state_root>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--page <main|settings>] [--account <alias>] [--lang <zh|en>]
acui --help
```

The main window is the **workbench**. Its top bar has a lamp with the Runtime in plain words, Start, a
"Troubleshooting and Ledger" button and, at its far end, Settings; under it is the plain-words result of
the last press. Then one of two pages. The main page has two columns: the left one is for the accounts,
with an "Add Account or Device" button that is off (see "Instance configuration"); the right one has the
settings file's problems, what falls short of the theme and where the technical log is. Settings switches
to the settings page, where the same button reads "Back to Workbench": first those same lines, then the
Runtime's settings, then "Runtime" (Request Shutdown, "Start the Runtime again automatically") and
"Appearance" (the text size, the language and the theme). The Runtime's settings come from the running Runtime (read again on going to the page), in two
groups, "Everyday" and "Advanced": which keys show and in which group is the Runtime's own tiering, and the
four settings of the performance preset are only on the top bar. Each says its value, where it comes
from, how it is changed (shown only for now, changed under that account's Auto Rules on the main page,
changed in the configuration file with a Runtime restart, or set by the installer) and when a change
takes effect; one the file does not name and that has no default says "not set" rather than being left
out. Everyday starts with the quiet times of all accounts (`scheduling.blackout.host`): one line per
time, each with Delete, then Add a Time…, Delete All (asked once) and Back to File Setting, the same
form as an account's own under its Auto Rules on the main page; each change sends the whole list online
once; a start equal to its end is not sent, and Whole Day makes 0:00 to 0:00 the next day. Of the other
rows, only "Days to keep unconfirmed evidence frames" is changed here: type a whole number and press
Save, or empty the field and save to go
back to the configuration file's value (or the default). Only the Runtime judges the range, and a refusal
says what it said; the result has a line of its own under the top bar, naming the setting. A save is
sent once: when its result is unknown, what holds is read again before anything is said, and nothing is
ever sent again by itself. After the rows, each device the Runtime drives (an emulator or a handset
entry, not a fixture) has a "Device Backends" group with its touch backend and its capture backend, each
in a drop-down of the backends that kind of device has (ADB input, MaaTouch, minitouch and, on an
emulator, MuMu native (Nemu IPC) for touch; ADB screenshot and, on an emulator, DroidCast and MuMu native,
or on a handset scrcpy, for capture), with where the value comes from (the configuration file, or set
online, when and through which way) and the device's latest self-check. Apply switches that device at
once if it is free (waiting at most 3 seconds, the contract's `CONNECTION_PREPARATION_CLAIM_WAIT_MS`): it
reconnects and self-checks on the new backends, and the console waits for the answer up to about 138 seconds
(the contract's `BACKEND_SWITCH_WAIT_MS`, 133 s at v0.12.58, plus the client's 5 s IO margin; the page takes
both numbers from the pinned crates), and the group says "Switching…" meanwhile. MuMu native touch goes only with
MuMu native capture: picking it brings that capture along, and picking another capture under it first asks
which touch goes with it, then sends both at once. A value set online offers "Use File Value", which is a
switch too (both rows together when clearing one alone could break that pair), unless the file's value is
the one in effect: then the device is left as it is, and the line says only that the file's value is back.
The result has a line of its own under the top bar, naming the account, and a refusal says why in words:
the device is busy (with what), played, fenced, not running, or the new backend did not open (and why: for
MuMu native touch, say, the game package configured for the device is not installed; since v0.12.40 a game
that is not running is no reason, and only a Runtime rolled back to v0.12.39 or earlier still gives it) and the
previous one is back or did not come back either. Nothing is sent again by itself: when a switch's result is unknown the
console waits out that whole wait and until the Runtime shows the device no longer switching, then says what holds now, with the
device's self-check. A Runtime before 0.12.39 has no such settings, and then no group shows. After every start of the Runtime its start report is read once: when an update
or a rollback reset settings changed online, the line under the top bar and the settings page say which;
"Got It" hides that line for this start. Online settings kept over a default that changed are said in
small print on the settings page only. A report that cannot be read is tried at most 3 times for a start,
then the settings page says so in red. While the Runtime does not answer, no older value is shown.
"Start the Runtime again automatically" is only shown: each time the settings page is opened,
`actingctl watchdog status` runs once in the background (read-only; `install`, `uninstall` and
`run-once` are never run), and the page says "On", "Off", "May not work" or "Cannot be read" of the
watchdog's scheduled task as it reports it, the last two with the reason in the technical log. After
the Runtime started again it is read again the next time the page is opened. The setup wizard's "Run
ActingCommand automatically after sign-in" (page 7) installs this very watchdog (see "Setup wizard acsetup"). Both pages are always there and only one shows at a
time, so what the boxes hold stays when you switch away and back.
The ledger window,
"Advanced Troubleshooting and Ledger History", opens only when asked: from that button, or at the start
when `--tab` or `--source` is given. There is at most one per source (the default `auto`, `online`,
`offline`); asking again restores the one open (Slint 1.17 has no call that brings a window to the
front). `--state-root` alone opens none, since the fixed entry passes it on every start. A closed ledger
window's reader stops after the page in hand; opening it again is a new window and a new session. Closing
the workbench ends the program, except while an unlock runs (see "Launcher").

The main page's right column has two tabs, **Live View** and **Daily Tasks**, before the alias of the
account picked on the left; the tab picked stays for the run, across accounts and pages, Live View being
the first. Daily Tasks is that account's task packages, those the Runtime's switch view lists with a
switch at the front of their row (see the next paragraph): one row per package, with its name (its
package id when the game's `ui.json` gives none), "Irreversible" when the Runtime derives it so and
"Risky" when its `ui.json` entry names a risk word (the word itself is not shown), why it runs or does
not run now in a sentence, and its `ui.json` summary in grey ("No description given" without one). The
sentence is "Running" (for a direct run, "not scheduled") while the status says the account runs that
package; where the dispatch stands while the task selected for the account is one of the package's;
else the words of the package's first candidate row in the Runtime's own order, chosen by its reason
code (a `benefit_unknown` by its `benefit_gap`, a `pool_line_held` by its `pool_line_hold`), with
"(earliest hh:mm)" when that row has a wake, and under it the other sentences of its tasks once each. The
account cards say a reason in the same words. No reason code, count or scheduler number shows: a code
the console has no words for says "Not scheduled right now", and the technical log hears of it once.
With a `ui.json` the rows go by its groups and its order, the rest under "Other"; without one, by name.
Which package a task runs comes from the `task_ids` of the same `actingctl packs` run the names come from:
a task no row of it lists, and every task when that run failed or had warnings, is listed by its task id
under "This account: some plans do not match a task pack", which opens. Above the rows: the account's
pause or fence in its card's words (another tool's pause without the reason it wrote), which round of the
evaluation the rows are of ("As evaluated in the round of hh:mm"), and whether the names cannot be read or
not every task is listed. The list reads one
`ScheduleExplanation` of that one account with its candidate rows, answered from memory without the
ledger, only while its tab shows on the main page of a window that is not minimized and the Runtime is
online: every 15 seconds, 30 after a failure, and at once when the tab, the account or the owner epoch
changes; the live view reads nothing meanwhile. While the Runtime does not answer, a read fails, nothing
new came for 60 seconds, the account is not in the evaluation or none was made yet, no row shows, only
why.

The switches (#429 R-8, from Runtime 0.12.55) are drawn from that account's `PackScheduleView` alone:
ticked means the Runtime schedules the package now. Under the name, in grey, where that comes from: "Set
for this account: turned off (since hh:mm)", "Set for this account: on", "Inherited from the server: off",
"Inherited from the game: on", "The standard pack's default: …" (not before R-9), with ", until hh:mm"
when its row ends; a package no row decides now says nothing (the note above the rows says that tasks
never set go by their standard pack's rules), or "Changes at hh:mm" when a row of it begins later. The
console writes the account's own layer only: off is one row from now on, without an end, that the package
is not scheduled; on goes back to normal, withdrawing the account's own rows of it when they decide it and
the layer below schedules it, and else one row from now on that it is. Either replaces every row of that
package for the account, rows that begin later included, and no other package's; a run of it already
going finishes as usual. Each write names the version of the account's own scope the view gave; when
something else (another tool, the Lab, an agent) changed it since, the Runtime refuses it as a conflict
and changes nothing, the row says in amber "This account's task switches were changed elsewhere and are
refreshed: check them, then press again", and nothing is sent again; `fact_observation_not_newer`
(another list landed in the same millisecond) is sent once more at once on the same connection. One write
of an account is out at a time: while it is out, and after an answer that may have changed the switches
or says something else did (it took, a conflict, the same millisecond twice, not confirmed) until a read
of the view begun after it has come, every switch of the account is greyed out, still showing what was
read before; after any other refusal, or a write not sent, they come back at once. The rescue bar's Close
This Task (see "Launcher") writes the same row as off and is one write of the account too: while it is
out every switch of the account is greyed out, and after one that may have written (it took, or is not
confirmed) they wait for a read of the view begun after it; while a switch's write is out, that button,
and Stop This Run of a run another tool started too, is greyed out and says "This account's task
switches are being changed; a moment". What each press came
to shows under its row, with when it landed (but for "Changing…", a conflict, a thread that could not
start and a restart), until that row is pressed again, the list shows another account, or the Runtime
restarts; the list hidden and shown again (another tab or page, the window minimized, the Runtime not
answering a while) keeps it, and one that took says nothing more. A package the view names that this
round's evaluation does not match still has a row ("This round's evaluation does not match it"); one the
list has that the view does not name has no switch and says "This task pack cannot be switched on or off
on its own here", and the technical log hears of it once. The view is read on the list's terms (the main
page of a window that is not minimized, this tab, the Runtime online, that account only), every 60
seconds, 60 after a failure, and at once when the tab, the account or the owner epoch changes, a write
lands, or after a conflict; nothing new for 180 seconds counts as unreadable, the note above the rows
says in amber that the task switches cannot be read right now, and no switch shows while the list's rows
still do. A Runtime before 0.12.55
(the view answers `not_implemented`, or before 0.12.30 is not known) has the note say that this version of
the Runtime cannot switch single tasks, and the view is not read on its clock again in that owner epoch;
but since what said so may have been a connection that ended just before its answer and was made again in
the same owner epoch, the list shown anew 60 seconds or more after it (the tab opened, another account
picked, the window restored from minimized, the main page shown again, the Runtime answering again after
a while) reads the view once more: an answer that says so again keeps it, one that shows the switches
lets it go and the view is read as usual; a list hidden again before that read lands drops it, and what
was said before holds until the list is shown anew again. Each read has
the Runtime bring its fact store up to the ledger under the fact write gate, so it is read seldom; the
technical log's read counts at the end of a run call it `pack schedule`. The Runtime's own reading of
account data (`_fact_refresh`) is not held by a row that turns a package off; a package only a row still
names, which no task of the account uses any more, is not in the view, so it is neither shown nor
withdrawn here. Once a package turned off reaches the deferral step, the card and the list say "This task
is turned off".

`--state-root` applies to this run only and overrides `state_root` in the settings file; if neither is
present the workbench still opens, with a red bar at its top that says in plain words that there is no
state root to use and what to do (the usage and the key names go to the technical log only), and Start,
Request Shutdown and "Troubleshooting and Ledger" are off; no default is guessed. Started through an A/B installation's fixed
entry `<install root>\ui\acui.exe`, the console takes the state root, the configuration and the Runtime
program from the installation's selection instead, and refuses a `--state-root` that differs from it (in
the same red bar).

**Installation folder (root mode)**: started directly as `<install folder>\ui\acui.exe` of a
single-location installation folder (one with `runtime\BUILD-MANIFEST.json` and no `install\active.json`,
the rule the Runtime itself finds a root by; `ui` in any case), the console takes all three paths from
that folder, as the Runtime's own tools find them: the configuration `<install folder>\actingd.config.json`
(read up to 1 MiB), the state root as its `state_root` names it, an absolute path used as written (not
assumed to be `<install folder>\state`; a folder not there yet is no error, only a line in the technical
log), and the Runtime program `<install folder>\runtime\actingcommand-actingd.exe`. The settings file's
three path keys are not used then, and a `--state-root` that differs from the configuration's is refused. A
console outside any installation folder may name one with `--root <install folder>` (absolute, with
`runtime\BUILD-MANIFEST.json` too, and with no `install\active.json` in it or in the folder above: one that
has is an A/B installation or one of its slots); a console inside one may only repeat that folder with `--root`, and on
an A/B installation `--root` may only repeat its root, or the red bar says so. A folder that lacks
something — the Runtime program, the configuration, a configuration that cannot be read or is not JSON,
no `state_root` or one that is not absolute, or a half-made folder with `actingd.config.json` or `runtime\`
but no manifest — is said in the red bar part by part, with the advice to run the installer again to
repair it, and is **never** stood in for by the settings file of somewhere else, so the console does not
quietly reach another installation; the technical log has each part with its path and the system's own
error text. The four tiers in order: an A/B installation's selection → `--root` → the installation folder
the program is in → `--state-root` and the settings file. A console outside any installation folder (such
as one installed under `%LOCALAPPDATA%\Programs`) takes the last tier as ever, and reads and writes as it
did. An installation folder must not be named `A` or `B`: the selection reader takes it for an A/B slot and
fails, and the red bar then says so in one more line. When the selection cannot be read (a folder named
`A` or `B`, or `ACTINGCOMMAND_INSTALL_ROOT` / `ACTINGCOMMAND_INSTALL_SELECTION` left behind by another
process), this run has no paths, but its settings are still the installation folder's own (see "Settings
file"), never the global settings file; a slot of an A/B installation (with `install\active.json` in the
folder above) is no installation folder and keeps the global settings file. On the workbench's settings page, a line under
"Appearance" names the installation folder this console uses and the file its settings are saved in.

Only `<install folder>\ui\acui.exe` counts: a console put anywhere else in an installation folder (right
in the root, say, or in `tools\`) does not take that folder and goes the last tier as ever. The program's
own path decides, as the system resolves it: when `ui` is a junction or a link, the folder is found by
where it leads, so the folder it leads to must itself be named `ui`, and the one above that is the
installation folder.

**Whether the Runtime at the state root is this installation folder's**: in root mode the state root is the
one the configuration names, as written, so a configuration copied from another installation leads the
console to that installation's Runtime (on this machine, perhaps the one in daily use). So in root mode the
console checks the Runtime its state root names before it connects to it at all, for its reads (the
workbench's status, schedule, settings and picture, the ledger windows, the launcher's probes) as for a
person's requests (Request Shutdown, recording a Start, Pause All and Put All on Duty, an account's pause and
resume, a resume in the ledger window, a change of settings, a device's operation, a refresh of the picture,
switching or closing a task, stopping a run). It takes the process id from the state root's
`runtime-info.json`, reading the file alone, and asks the system for that process's program file, which, as
the system resolves it (links followed, `\\?\` dropped), must lie in `<install folder>\runtime\`, or in a
kept build's `<install folder>\install\builds\<commit>\runtime\`. When it does not, or this cannot be told
(the program file cannot be had or resolved), the console connects to nothing at that state root: no
handshake, no read, no request. The workbench's lamp turns red with "The Running Runtime Is Not This
Installation Folder's", the red words at the top name the process, its program file (or why it cannot be
told) and the installation folder, the ledger windows show the same as the Runtime's status, and nothing
read online is shown; Start launches nothing and records nothing, and the settings page's
`actingctl watchdog status`, which asks a Runtime it finds for its health, is not run either (the page says
whether the Runtime is started again automatically cannot be read, and the technical log says why). A
button pressed meanwhile says under it that nothing was sent because the running Runtime is not this
installation folder's; the code
`console_runtime_not_this_root` is not shown there, only in the technical log's line of that press. What
the check found is kept for that process and its owner epoch and asked again once the Runtime changes; each
new finding goes into the technical log once. A `runtime-info.json` whose process no longer exists (a
Runtime that ended without removing it) names no running Runtime: nothing is connected to it either, the
lamp says the Runtime is not running, and Start is offered. Outside root mode (an A/B installation, the
settings file's paths) there is no such check.

Two cases the check cannot cover. `unlock-owner` runs this installation folder's own actingd with the
folder's own configuration and connects to no Runtime, so there is nothing to check. And when the
configuration names a state root copied from another installation and that installation's Runtime is not
running at the time, Start launches this folder's actingd, as its configuration says, on that state root:
with no running Runtime to check, this cannot be told.

Logs are not kept per installation folder yet: the technical log and actingd's start logs both stay in the
global `%LOCALAPPDATA%\ActingCommand\logs\` (see "When the start fails" and "Launcher"), the consoles of
every installation folder writing into the one directory, told apart only by the start time in each file's
name; the technical log's `place` line of each start names the installation folder that run used.

`--source` selects the ledger window's read face (see above).
`--tab` specifies the ledger window's startup tab (for screenshots and review), and its values are the
views' own wire names. `--page` is of the same kind: it only picks the workbench page the window opens on,
`main` (the default) or `settings` (for screenshots). `--account` is of the same kind too: the account card
selected when the window opens (the first card otherwise, and when no account of that alias is listed, which
the technical log says). `--lang` applies to **this run only**, overrides the language in the settings
file, and is not written back to the settings file.

Started directly — `acui.exe`, or a shortcut that points at it — the console opens no black console
window; started from a command line, `--help` and the reason for a failed start still go to that
console. Started through the fixed entry `acforward.exe`, a console window still shows for now.

The windows are resizable: the workbench 1200×780 by default, 1000×640 minimum; the ledger window 1400×900
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
selection it cannot read, a `--state-root` that differs from it or from the installation folder's
configuration, a `state_root` that is not absolute, an incomplete installation folder, a `--root` that is
no installation folder or not the one the console is in, or no state root at all is no failed start: the workbench opens with a red bar, plain words first and then the
original, and the technical log has it too; for no state root at all the red bar has the plain words
only, and the usage with its key names goes to the technical log.

The technical log is `%LOCALAPPDATA%\ActingCommand\logs\acui-<start unix ms>.log` (elsewhere under
`$XDG_STATE_HOME`, else `$HOME/.local/state`), beside actingd's start logs. It is created by its first
entry and holds failures: a failed start or no state root to use, a problem of the settings file, a
result a background thread could not hand back to the window, a ledger window that could not be made or
shown, a ledger reader thread that could not start or that panicked. Besides, every start writes one
`place` line: the tier the paths came from (`selection`, `root mode (exe|--root)`, `legacy`), what they
are and which settings file is used, and in root mode one more line when the global settings file is there
but not used, or used because it names this installation folder; these are no failures, and on their own
they put no word of the log on the windows. A
read face that does not open is no failed start: the ledger window stays and says why in its ledger area
(see above). Once it exists, the top of every window says where it is; when it
cannot be written, the windows say that instead.

## Interface language, text size and theme

Three dropdowns under "Appearance" on the workbench's settings page and at the right of every ledger window's top bar; a choice made in
any window applies at once to every open window and is written into the settings file as soon as it is
made, and a ledger window opened later comes up with the current language, text size and theme:

- **Text size**: standard / large / extra-large = 1.0 / 1.25 / 1.5. **Takes effect on the spot**. Every
  font size and line height in the interface is one base size meant for a person at normal DPI (list row
  14px, detail pane 13px, subheading 16px, line height 26px) multiplied by this one factor; the interface
  has only this one knob, `Scale.factor` (in `crates/acui-app/ui/kit/tokens.slint`). The minimum widths
  of the controls that hold text (the boxes, the id box, the time slider, the notice lines) and the
  minimum heights of text areas are multiplied by it too; a window's minimum size and the three columns'
  floor widths are not.
- **Language**: 中文 / English. **Takes effect on the spot**: the choice fills every window's `Strings`
  global again and paints every line of every window again — the launcher's plain and technical lines,
  the frame pane's note, the port box and the words after a read error included. A background thread never holds a language table; it
  hands back phrases that are put into words when painted, so a start or a frame read still in flight
  reports in the new language. The two language tables are in `crates/acui-app/src/strings.rs` (`ZH` and
  `EN`); there is no gettext. A read face's own error text is shown verbatim, untranslated.
- **Theme**: Modern Flat / Win7 Aero (`flat` / `aero` in the settings file). **Takes effect on the spot**,
  nothing is restarted: the choice sets the skin of every window's own `Look` global, which every page,
  card, text and control of the interface is drawn from (`crates/acui-app/ui/kit/`). Modern Flat follows
  the system's light or dark setting; inside a Win7 Aero window the page stays light whatever the system
  says. The startup failure window is always Modern Flat, as it comes before the theme does. On a system other than Windows the second option is named
  Win7 Aero (Solid).

  On Windows, Win7 Aero draws the window's own frame: a caption band and an 8px edge of see-through glass,
  Win7's caption buttons, and a window that fades in over 250 ms and out over 180 ms. Moving, snapping,
  resizing from all eight sides, double-click to maximize and the system menu stay the system's. The glass is
  a second window with no input, kept right under the first, on which Windows.UI.Composition draws what lies
  behind through a Gaussian blur of strength `AeroConst.sigma`; that is 0 today, so what lies behind shows
  plainly, and changing the blur means changing that one value. What cannot be drawn falls one step, says so
  in red in that window and in the technical log, and is tried again when a theme is picked: without the
  glass the frame stays over solid glass, and without the frame the Aero page sits in the system's frame. A
  window never moves to Modern Flat by itself. The technical log (kind `acui-glass`) has a line for each glass
  built and each gone, `垫底窗已建立` and `垫底窗已销毁`, with the main window's and the glass window's
  rectangles and the running count of both; a run that ends with the two counts equal left none behind.

Two-layer labels: the upper layer is the name for a person to read, and the grey lower layer is the
verbatim form used in the program — the raw `event_type`, module names, ids of every kind,
`payload_schema` and sha256 are never translated. The dictionary is in `crates/acui-rows/src/display.rs`
and covers all 121 `event_type`s and 20 `origin.module`s in the contract; **anything not in the table is
displayed verbatim, not guessed**.

### Settings file

Language, text size and theme are stored in a settings file, `acui.toml`. Which one is decided once at
startup, and the whole run reads and writes that one only:

- **A console outside any installation folder**, and an A/B installation: the one under the per-user
  configuration directory (the global settings file below), as ever:

  ```
  Windows:  %APPDATA%\ActingCommand\acui.toml
  Linux:    $XDG_CONFIG_HOME/ActingCommand/acui.toml (falls back to $HOME/.config/…)
  ```

- **Root mode** (see "Running"): the installation folder's own `<install folder>\acui.toml`, beside its
  `actingd.config.json` (not in `ui\`, the program folder, which an update replaces whole). In order:
  1. When `<install folder>\acui.toml` is there, it is used (one that cannot be looked at counts as
     there, so that what is wrong with it is said).
  2. When it is not, but the global settings file is, and its `actingd_config` names this installation
     folder's `actingd.config.json` (the same file), the global settings file is still used: an
     installation set up through the global settings file before the console was put in its `ui\` keeps
     its settings, `resume_elsewhere_reasons` among them. The technical log says so.
  3. Otherwise `<install folder>\acui.toml`: the defaults while it is not there, and it is made by the
     first change in a dropdown. In root mode the global settings file is used in no other case (when it is
     there and not used, the technical log says so).

  An installation folder's own settings file takes none of the three path keys: in root mode every path
  comes from the installation folder, so such a line has no effect, is said in amber, line by line, and is
  kept as it is; it never quietly takes effect. An installer that presets the language writes this file line
  by line (a single line `lang = "en"`, say), and never the global settings file.

On the workbench's settings page, a line under "Appearance" names the installation folder in use, if any,
and the file the settings are saved in.

```toml
# zh | en
lang = "zh"
# standard | large | extra-large
text_size = "standard"
# flat | aero; set from the theme dropdown, the workbench and the ledger windows are drawn by it
theme = "flat"
# optional, absolute paths; the global settings file's only, not used in root mode
state_root = '<install root>\state'
actingd_config = '<install root>\actingd.config.json'
actingd_exe = '<install root>\runtime\actingcommand-actingd.exe'
# optional, whole seconds 2–60 (missing, out of range or not a number: 5): the live picture
# reads a task frame at most this often. A frame read is answered by the Runtime's reader pool,
# off the ledger writer (v0.12.38); default 5; do not set it faster before the C7 joint test.
preview_frame_gap_seconds = 5
# optional, one line, default none, e.g. ["a-reason", "b.reason"]: the pause reasons the console
# offers no resume for, since only whoever set such a pause lifts it
resume_elsewhere_reasons = []
```

It is read once at startup, and a dropdown change writes its one key: the file is read again, the last
line of that key is replaced (or a line appended), and every other line — comments, the path keys, keys
the console does not know such as `install_root`, lines it cannot parse — is kept byte for byte; the
result goes to `acui.toml.tmp` beside it, which then replaces the file. **The file so chosen is the only
file the console itself reads and writes**, besides the actingd start logs the launcher creates and, after an
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

`resume_elsewhere_reasons` names the pauses the console leaves to whoever set them. The Runtime keeps
no actor with a pause, only its reason, so the pauses that must not be lifted from here are named by
their reasons: a pause with one of them gets neither Put All on Duty, Resume This Account nor "Resume
Scheduling…", only the words "this pause is to be lifted by whoever set it", on the top bar, on the
account cards and in the ledger window. No box writes the key, and a save of another key keeps its line byte for byte, like any
line; the 0.12 setup wizard never writes this global file (it writes only `<root>\acui.toml` in the root
it installs, with the one line `lang`; see "Setup wizard acsetup"), so it never drops the line. It is one line, its key bare or in quotes, a TOML array of
strings in double or single quotes, a trailing comma allowed; `[]`, or no line at all, is the default:
none. Each item must be a reason the Runtime itself takes for a pause (checked by the Runtime's own
`SchedulingPauseReason`: 1–64 characters of `a-z`, `0-9`, `_`, `.` and `-`), so there is nothing to
escape. A line of this key the console cannot use — not such an array (an array in quotes is a string),
an item not quoted or not such a reason, a comment after the value, a line of the key without `=`, the
key in quotes that do not pair — is said in red at the top of every window, with the file and the
line, and **fails closed**: until it is fixed, whatever another line of the key says, no pause except
the console's own two offers a resume anywhere in the console: Pause All, the global pause with reason
`console_pause_all`, and Pause This Account, an instance's pause with reason `console_pause_account`
(see "Launcher"); either reason on the other scope gets none. A settings file that is there but
cannot be read, or no configuration directory, fails closed the same way. A deployment's pause stays
the installer's in any case, and a pause the Lab's rescue module set stays its operation's (see
"Conditional resume" under "Launcher"). The reasons are the operator's own data: the console never
rewrites one, and the ledger window still shows each pause's reason verbatim on its line under the
technical lines.

## Launcher

On the workbench the top bar has a lamp and the Runtime state from the newest status read, in plain words,
then the Start button, on only while the Runtime is not running and no start is out; it is off while the
lamp says the Runtime is stopping or not answering, or that connecting to it went wrong, since one may still
be there. It is also on when the status reads themselves fail, since a start probes first. Request Shutdown
is under "Runtime" on the settings page. The line under the top bar is the result of the last press, in plain words only (no key,
code, PID or path; an outcome code shows only as its short words; a failure only the technical text states
adds "details in Troubleshooting and Ledger"). A start that launched a daemon and got no Runtime from it
also shows that daemon's log path there, selectable to copy. Every ledger window's top bar has the technical
lines: the state from the last probe with its PID and owner epoch, or the client's code, and below it the
technical text of the last press; the unlock entry is there too (below). A line turns red on a failure and
wraps instead of being cut; only the lamp's words elide when the top bar is short of room. The probe, the whole start (probe, launch,
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
  the console program's console, and does not receive its Ctrl+C. Without an installation selection (root
  mode, or the settings file's paths), both this launch and `unlock-owner` (below) first clear the two
  environment variables `ACTINGCOMMAND_INSTALL_ROOT` and `ACTINGCOMMAND_INSTALL_SELECTION`, as the
  Runtime contract install-cli's start rules have it; an A/B installation still hands its own selection to
  the programs of its slot.
- **Readiness decision**: at most 60 attempts, 500 ms each. Each attempt does `try_wait()` first: if the
  child process has exited it stops and states the exit code, actingd's terminal outcome in that log —
  its last `OUTCOME actingd:` line holding an `actingcommand.outcome.v1` envelope with a code
  (runtime-outcomes 1), shown as the developer line written just before it **verbatim**, else as that
  outcome line — or that the log could not be read, with the error, or holds no such line — and the log
  path; if it has not exited it
  does one more `connect`, and a connection means ready, stating the PID and owner epoch, and then the
  press is recorded (see below). A ledger window of the default source that reads nothing for want of a
  Runtime, or reads offline after the press of "Open Offline" (its offline face unopened included), then
  offers "Reopen", saying it can read online now — it **never quietly
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

The workbench's top bar has **Pause All** and **Put All on Duty**. Pause All asks the Runtime for a
global pause of scheduling: no new task is scheduled, and a task already running finishes on its own.
Put All on Duty lifts that global pause; a global pause another tool set is lifted only after the person
confirms it, and a pause whose reason the settings file lists in `resume_elsewhere_reasons` (see
"Settings file") is to be lifted by whoever set it, so it cannot be lifted there. The ledger window can
resume, conditionally, a pause another tool set (below). The global pause the setup wizard sets after a
new installation's first start (reason `first-start`) has words of its own: Put All on Duty's
confirmation, the account cards and the line under the task list say it is the pause after setup, to be
lifted once all looks right; who lifts it, and how, is as for any other tool's global pause. Those words go
by the reason string alone: a global pause another tool sets with the same reason `first-start` is shown
the same way. Start at boot is an option of the setup wizard; see "Setup wizard acsetup".

Each account's card has a button of its own. While the account has no pause of its own it is **Pause
This Account**: one `PauseScheduling` of that instance alone, with the console's own reason
`console_pause_account` and the same drain timeout as Pause All (the lowest the Runtime takes, 1 s), sent
once on a worker thread over a fresh person's connection (actor `user`) and never again by itself. No new
task of that account is scheduled. Unlike the global pause an instance pause drains: a task still running
on the account once that second has passed is asked to stop at its next checkpoint, and the Runtime
answers once the device is handed back, so a press can take about half a minute, which the card says
while it is out. Under the pause the console set there, once its device is handed back, the button is
**Resume This Account**: one conditional `ResumeScheduling` of that instance, its condition the owner
epoch and the revision of the status read the card was painted from, so the Runtime refuses it and lifts
nothing once that pause has changed. It goes at one press, without the confirmation Put All on Duty asks
of another tool's pause, since the console set this pause itself. The resume connects the device again;
a reconnect that fails is said in red with its code, and the resume stands. What a press came to is said
on the card: done (a resume done while a global pause holds adds that no task is scheduled for the
account until that one is lifted too); a pause that already holds, which keeps its reason; a refusal,
in red, its code verbatim and the code's words (a fixture instance, for one, is refused with
`fixture_execution_scope_forbidden`); or not confirmed, which is not sent again. While
`resume_elsewhere_reasons` cannot be used the card still lifts this pause, as the top bar still lifts its
own Pause All; a settings file that lists `console_pause_account` leaves it to "whoever set it", like
any reason listed. The card resumes only the console's own account pause: any other pause of the
account, another tool's or one set with `actingctl pause --instance`, is said on the card by who lifts it
as before, and the ledger window resumes those it may (below). From the command line, `actingctl pause
--state-root <state root> --instance <alias>` and `actingctl resume` with the same flags, or the MCP tools
`ac_pause` / `ac_resume` (see "MCP for agents"), still pause and resume one instance.

Over the live view, the rescue bar has one more row while the account does something: **Stop This Run**
for a run another tool started (#429 UI-3e2, R-6's first slice, Runtime v0.12.53 or later), and **Close
This Task** for a run the scheduler dispatched (UI-3e2b, R-8 and R-6b, Runtime v0.12.55 or later); a
scheduled run has no button that only stops it. It can be pressed while the account's activity in the
last status read names its `request_id`: a run another tool started (`direct_run`: a run of the Lab's
`run start`, `actingctl task-run` or the MCP tool `ac_run_pack`, say), and a scheduled run that names its
`package_id` too (`policy_run`, one finishing under the global pause included). The Runtime's own steps
(starting, recovering or winding down when idle) send nothing: the button stays off and says why. One
press sends it, with no second question, and no pause, quiet time or fence holds it back. Stop This Run
sends one `StopInstanceRun` on a worker thread over a fresh person's connection, `wait_ms` 5 s; the
Runtime stops the run at its next checkpoint, resets the device itself with the run's own lease, and
answers. What it came to is said under the row, with when; never a code or a request id on screen, and
the stopped run's request id only in the technical log. After an answer that the run still stops
(`settled: false`), or one not confirmed, the stop is sent once more, and only once, when a status read
of the same owner epoch, begun after that answer, shows the run gone, to read the reset's result;
nothing sends it on a timer, and a refusal or one not sent is not sent again. That the run stopped or
had ended, as the follow-up reads it, is said with the time the console learned it ("Stopped (learned
at hh:mm:ss)"), not the time the Runtime ended the run, which was earlier; an answer within the first
request's wait is said with its time alone. A Runtime that restarted,
an account no longer listed or a reading thread gone leaves the result unknown, and nothing more is
sent; a run that does not end is waited for as long as it runs, with no limit of the console's own. A
Runtime before v0.12.53 answers `not_implemented` (which records nothing), and both buttons then stay
off for that Runtime's owner epoch. Whether another console or tool already asked the run to stop is not
seen here: another press costs only three more ledger records (`ui.action`, `command.received`, then
`command.validated` or `command.rejected`), and still reads the result.

Close This Task closes the task first and then stops this run, on one fresh person's connection: it reads
the account's `PackScheduleView`; when the run's package is in it, it writes one row of the account's own
scope from now on, without an end, that the package is not scheduled (the very row the Daily Tasks
switch writes when turned off, naming no version, so that another package changed a moment ago does not
make it a conflict; `fact_observation_not_newer` is sent once more at once), and only once that is
Completed sends the stop above. A close row not written (refused, not sent, not confirmed) stops nothing,
and the line says "Not closed: …. This run was not stopped either" or that whether the close was written
is not known. A package the view does not name (the account's daily tasks do not run it: a reading's
maintenance, or the catalog changed since the dispatch) gets no row, which the list could neither show
nor withdraw, and its run is only stopped: once it stopped, "This task pack is not one of this account's
daily tasks, so only this run was stopped"; otherwise (not confirmed, still stopping, refused, a restart
and the like) "This task pack is not one of this account's daily tasks, so nothing was closed here",
followed by what the stop came to. A view that is not read does nothing ("Not closed: …"); a view whose
connection ended before its answer (the client's `runtime_operation_unsupported`) is a view not read too
("Not closed: the connection ended before this account's task switches were read …. This run was not
stopped either"), not a Runtime without task switches. A view or a close row answered `not_implemented`
(before 0.12.55) does nothing either, and Close This Task then stays off for that Runtime's owner epoch
("This Runtime cannot close a task from here yet"), as the Daily Tasks switches are not read then (and,
as when the view itself said so, Daily Tasks shown anew 60 seconds or more after reads the view once
more, and switches it finds bring the button back). A program error (its thread panicked) says that
whether the task was closed and whether this run stopped are both unknown, the card and Daily Tasks
saying what holds, and nothing is sent again. Once closed, the line begins "Closed this task
(hh:mm:ss); it is no longer scheduled", followed by what the stop came to, and once the run stopped, "To
bring the task back, turn it on under Daily Tasks": that is the way back, where its row then says "Set
for this account: turned off (since hh:mm)". The follow-up only stops; it writes no close row again.
While the close is out every switch of the account is greyed out; while a switch's write is out the button
is, and Stop This Run too ("This account's task switches are being changed; a moment"). In the ledger, the close is one
`fact.published`, the stop three records, a follow-up three more. Of a package that has daily tasks and
also reads account data, the close row closes its daily tasks, and the Runtime's reading of account data
(`_fact_refresh`) is dispatched as before. A scheduled run still queued or being prepared is not in the
status read and cannot be pressed; Pause This Account keeps such runs from going. A stop is no failure:
nothing retries, backs off, pauses the task or starts a ladder. A direct run's lines are as before, and
it writes no close row. In the ledger window, a stopped scheduled run's `task.failed` (`contained_task_cancelled`),
its `policy.execution_recorded` (result `stopped`) and the scheduler driver's `runtime.failed` say
Stopped beside them, not in a failure's red, their type and level as the Runtime recorded them; the
run's recovery header is the ledger's, and may still count them as failures with no recovery seen. On
the Daily Tasks tab a dispatch that is over this round says "Ended this round", whether its run ran
through or was stopped.

- **Conditional resume (ledger window)**: below the technical lines the window lists the global pause and
  each instance's pause of its last status read (reason, revision, since), each by its own scope. One
  another tool or the console itself set (the console's own line says "Paused by This Console", not
  "Paused by Another Tool") — the global one, or an instance's whose device is handed back — carries
  "Resume Scheduling…": the first press only shows the statement "resume this one pause" and "Confirm
  and Resume"; the confirm sends one conditional `ResumeScheduling` on a worker thread, over a fresh
  person's connection (actor `user`), its condition the owner epoch of that status read and the scope's revision,
  so the Runtime refuses it and lifts nothing once that pause has changed or another Runtime runs; only the
  pause of that line is resumed. Lifted, refused (shown by its code, "code not in this console's table" for
  one the table lacks) or not confirmed (a receipt timeout, a dropped connection,
  `runtime_scheduling_resume_unconfirmed`), nothing is sent again: the status is read again on a new
  connection and painted. The line says what that read found: one that failed, or one the window could not
  make (it reads offline, or no longer reads), says in red, with its code, that whether scheduling is
  paused is unknown, and a failed status read shows one red line in place of the pause lines; only a read
  that was made says the pause lines are current. Three kinds of pause get no button, only why: one whose
  reason `resume_elsewhere_reasons` lists ("this pause is to be lifted by whoever set it"); a
  deployment's — `acsetup_update`, `install_restart` and any reason starting with `deploy-` — which the
  installer lifts (the console does not run `release-hold`); and one the Lab's rescue module set (Runtime
  0.12.48 on), its reason exactly `lab-<operation>-<unix ms>` with the operation one of `config`,
  `deploy`, `down`, `instance` (the instance add, 0.12.52 on), `place`, `restore` and `rollback`, which
  that operation lifts as it finishes, except
  `down`, the stop, whose pause stays until it is lifted from the Lab ("a Lab rescue operation set this
  pause, and the operation lifts it when it finishes; one a stop set (reason starting lab-down-) is not
  lifted by itself: like one an operation that has stopped left behind, lift it from the Lab"), on the
  top bar and the account cards too. While
  that settings line cannot be used, every pause but a deployment's, a Lab rescue's and the console's
  own two (Pause All, the global pause with reason `console_pause_all`, and Pause This Account, an
  instance's with reason `console_pause_account`) is of the first kind. Two pauses set for a person to
  lift after looking are neither a deployment's nor a Lab rescue's, so they carry "Resume Scheduling…"
  like another tool's pause unless `resume_elsewhere_reasons` lists their reason or, as just said, cannot
  be used, when they are of the first kind: `instance.added`, which the Lab's rescue
  module's `instance add` (Runtime INST-5) sets on the account it has just added and which, unlike that
  module's `lab-…` pauses, no operation lifts; and `policy_settlement_facts_skipped`, the hold the
  Runtime's start sets on an instance whose settlement facts it could not record (Runtime ISO-START-1).
  No Runtime release up to 0.12.51 sets either; both come with 0.12.52, and the pinned 0.12.58 has
  both. An instance
  pause still draining or handing its device back gets none either: the Runtime refuses its resume
  then. Each line shows its pause's reason verbatim, whatever the operator wrote.

## Instance configuration

The console no longer has an instance-configuration window. Until the Runtime's `install instance`
lands, an instance can only be added by hand: edit the `instances` of the actingd configuration
(`actingd_config`) as the Runtime's installation notes describe, then restart the Runtime. The
workbench's "Add Account or Device" button is off for that reason, with this note under it and the path of that
configuration file below, selectable to copy. Finding instances comes back with the console's own
instance page in a later version.

## Setup wizard acsetup

`acsetup.exe` (`crates/acui-wizard`) is the 0.12 setup wizard: it installs ActingCommand's Runtime, its
tools and the console into **one new folder** (what the Runtime calls a single-location root) and
connects the signed-in games in the MuMu emulator. Opened from Explorer it is a window with the console's
look and icon. It opens in the language Windows shows its own interface in (Chinese on a Chinese Windows,
English otherwise) and switches between 中文 and English at the top right at any time; with a command it
is a command line (see below). It goes on no network, downloads nothing, and does not change any
ActingCommand already on the computer.

**The install set.** The wizard installs from an "install set" only: a folder holding `install-set.json`
(`actingcommand.install-set.v1`) and every file it lists — the Runtime zip, the Tools zip, the console
zip, one standard pack zip per game (with the licence and notice files that go beside it) and the
text-recognition (vision) files — each with its size and SHA-256. The wizard looks for `install-set.json`
next to itself first and asks for the folder when there is none. It checks every file's size and SHA-256,
each of the three program zips file by file against its own `BUILD-MANIFEST.json` (with the commit the set
names), the `interfaces` of the console's and the Runtime's manifests as the Runtime's
`contracts/component-interfaces.md` checks them (`runtime-client` and `actingd-config` must overlap, the
Runtime's highest `ledger` and `runtime-outcomes` revisions must lie in the console's range), and each
standard pack's `bundle.json`, `applications.json`, `schedule.json` and `interfaces.json` (its `package`
revision must lie within the range this Runtime declares). The vision files must carry ONNX Runtime's
`ort/licenses/onnxruntime/LICENSE` and `ThirdPartyNotices.txt`. Anything wrong stops it before anything is
written: a damaged file, a pack or console that does not match this version and a set it cannot read each
get a plain sentence, with the technical detail in small print below. Whoever gives you the package puts
the set together.

The files that go beside a standard pack are its game's `files`, laid at `<root>\licenses\<game>\<path>`;
`path` is a plain relative path, each once per game:

```json
"bundles": [
  {
    "game": "arknights", "file": "bundles/arknights-resources.zip", "size": 123, "sha256": "…",
    "files": [
      {"path": "pack-files/NOTICE.md", "file": "bundles/arknights/NOTICE.md", "size": 1, "sha256": "…"},
      {"path": "LICENSES/MaaAssistantArknights/LICENSE", "file": "bundles/arknights/LICENSE", "size": 1, "sha256": "…"}
    ]
  }
]
```

**Ten pages.** Pages 1 to 8 only gather choices and run read-only checks; nothing is written. Cancel or
closing the window on those pages asks first, and quitting leaves nothing behind.

1. **Welcome**: what will be installed and what to have ready (64-bit Windows 10 or 11, MuMu installed with
   the game signed in, about how much disk space). The games are named as their packs name them: before the
   set is checked, the wizard reads only the small `applications.json` of each standard pack zip for its
   name, and shows the folder name when that cannot be read.
2. **Files**: the install set is checked with a progress bar and a line in plain words under it (how many
   files are checked, which program or game was just checked); repositories and commits go to the setup
   log only. Then its versions and games are listed.
3. **Location**: by default `X:\ActingCommand` on the first local fixed drive other than the system's, else
   `%USERPROFILE%\ActingCommand`; when that one exists and is not empty, `-2`, `-3`, … are tried. Type a
   folder or pick one with Browse… (the folder picker belongs to the wizard's window, which takes no input
   while it is open); once typing pauses, each rule gets a ✓ or ✗: the folder does not exist
   or is empty, its last name is not `A` or `B`, the path holds no `'` or `%`, it is not inside another
   ActingCommand, it is none of ActingCommand's per-user folders `%APPDATA%\ActingCommand` (the global
   console settings `acui.toml` live there) and `%LOCALAPPDATA%\ActingCommand` (the console's and the
   Runtime's logs, which every watchdog reads), lies in none and holds none, and the disk has room. Any ✗
   keeps Next off. The window writes sizes in GB and MB (counted in 1024s, as Explorer does); the setup log
   and the command line write GiB and MiB.
4. **Games**: tick the games to install; every game of the set is listed, all ticked by default.
5. **Emulator**: MuMu is found read-only (as the Runtime finds it: a running MuMu, the registry's uninstall
   entries, the MuMu folders in Program Files) and `MuMuManager.exe info -v all` lists its emulators (no
   window, within 10 seconds); no emulator is opened or closed. No emulator is ticked to begin with: the
   person ticks the ones to hand over, and Next with none ticked asks first. The page says what the
   wizard calls an instance from here on: each emulator ticked becomes one instance in ActingCommand, one
   signed-in game in one emulator. An emulator another ActingCommand on this computer already uses is
   greyed, and ticking it asks first. The other installations are found two ways: a running
   `actingcommand-actingd.exe`, and a registered watchdog task (which finds a stopped one too); its
   configuration uses an emulator when it binds it by index, name, `serial` or adb port. Without MuMu the
   page can be skipped: the set is installed without instances, to be added later.
6. **Instances**: one row per ticked emulator: its name (by default "game-index"), game and server; under
   Advanced, the touch and capture methods (both `nemu_ipc` by default) and "Open this emulator when
   ActingCommand starts" (off by default). Names must differ, hold no space and not be too long: with two or
   more emulators on one game, the Runtime's schedule names each task copy `<id>@<name>`, at most 128 bytes,
   so a name's limit follows from the longest id in that game's `schedule.json`.
7. **Options**: desktop and Start menu shortcuts (on by default); run automatically after sign-in (the
   Runtime's watchdog scheduled task, **off by default**; leave it off when another ActingCommand on this
   computer already runs automatically); start when setup finishes (on); on the first start, keep every
   instance paused (on: ActingCommand starts playing only after you check each instance in the console and
   press "Put All on Duty"). The options set no rule on one another: the watchdog never starts a root that
   never started, so without "start when setup finishes" the installation stays down until you start it.
8. **Confirm**: the location, versions, games, instances, options, the folders created and the files written,
   all listed (with `licenses` when a game brings licence files, and `install-wizard.config.json` with
   instances, the start and the pause). Once Install is pressed there is no going back to the earlier pages.
9. **Install**: step by step, each done, skipped or failed, the step under way with a line of detail and a
   progress bar. Before writing, the install files, the emulators and the folder are checked once more. A
   failed step stops the install; what was written stays as it is, and the wizard deletes nothing. Closing
   the window while it installs is held off.
10. **Finish**: where it was installed, where the setup log is and which shortcuts were created. "Open
    console" (the new root's `ui\acui.exe`, started apart from the wizard and without any `ACTINGCOMMAND_…`
    or `ACTINGLAB_…` variable the wizard inherited) and "Open folder" are for the person to press; the wizard
    opens nothing by itself. The page also says how to add instances later, since the console cannot add
    them yet: run the wizard again into another folder (a second, separate ActingCommand), or wait for a
    later version. After a failure the page says which step and why, offers to copy the error and
    to open the setup log, and lists how to undo what the install actually did, item by item: a Runtime it
    started (whether it is paused, and how to stop it: open `<root>\ui\acui.exe` and press "Request
    Shutdown", or run `"<root>\runtime\actingctl.exe" request-shutdown --state-root "<root>\state"`), a
    watchdog it installed (`"<root>\runtime\actingctl.exe" watchdog uninstall --root "<root>"`), the
    shortcuts it created (their paths); deleting the folder comes last.

**What it lays down:**

```text
<root>\
  runtime\              the Runtime
  tools\                the tools
  ui\                   the console (acui.exe)
  packages\<game>\      standard packs, in the installed layout the Runtime reads
  licenses\<game>\      the licence and notice files beside a standard pack (when the set has them)
  vision\               text recognition: ort\ (with ONNX Runtime's licence files) and models\<model>\
  state\                the records (the state root)
  actingd.config.json   the Runtime's configuration
  install-wizard.config.json  the full configuration with the instances (with instances and the first
                        pause only; once in place, the same as the line above)
  acui.toml             this installation's console settings (the language only)
  install-wizard.log    the setup log
```

The fifteen steps of page 9, in order: create the folders and the setup log; lay out the Runtime, the Tools
and the console, each file checked against its `BUILD-MANIFEST.json`; place the vision files, each against
its hash; lay out the standard packs (each zip's SHA-256 computed once more right before, against the set)
and place the files beside each, each against its hash, in `licenses\<game>\`; check each game with
`actinglab standard-pack check`; write the configuration (as `actingd.config.json.part` first); put it in
place only once `actingd check-config` passes; hand the root to the Runtime's own installer with `actinglab
adopt`; write the console settings; create the shortcuts (named after the root folder, such as
"ActingCommand Console (AC2)", with a number when that name is taken: someone else's is never touched);
start (`actinglab up`); pause for the first start (`actinglab pause --reason first-start`); connect the
instances (`actinglab config replace`); install the watchdog (`actinglab watchdog install`); confirm the
Runtime is online with `actinglab status`. Steps not chosen show "Skipped". Every child program runs without
a window, its output in the setup log.

Why the instances come last: a global pause stops no run already under way, so a Runtime started with its
instances could dispatch between the start and the pause. With instances and the first pause, step 7
checks the full configuration, keeps it as `install-wizard.config.json` and puts in place the same
configuration without the instances (the same salt, state root and MuMu folder, through `check-config`
too); once the Runtime runs it and the pause is set, step 13 puts the full one in place with the Runtime's
own `config replace`: it checks the file with this build's `check-config`, then, the Runtime running, takes
its own hold, closes it formally, swaps the file (the replaced one kept as `actingd.config.json.prev-<ms>`)
and starts it again; it hands back only its own hold and leaves the global pause as it is. The global pause
is a persisted runtime fact that every start that is not installer-held restores before any instance is
prepared (Runtime `contracts/scheduling-pause.md`, "Persistence, no expiry"). The last step reads the
status once: online, the first pause still there, no instance running a task (a run is a failure, never a
success), and every configured instance present.

The watchdog comes last: it never starts a root that never started, one closed formally or one the
installer is working on, and starts only one whose last start ended in an error (Runtime
`contracts/runtime-watchdog.md`, "Decision", rows 3, 5, 6 and 7a). Last, it cannot start again a start the
wizard just reported failed, and a failure before it leaves no task.

**Other ActingCommand installations are left alone:** the wizard writes only the folder it creates, plus the
shortcuts and the watchdog task you tick; it never writes the global
`%APPDATA%\ActingCommand\acui.toml`, touches no other installation or state root, and starts or stops no
emulator. Every child program it runs (and "Open console") gets none of the `ACTINGCOMMAND_…` and
`ACTINGLAB_…` variables the wizard inherited, only the initiator it sets itself, so a wizard started where
another state root is named still acts on its own root only. It reads another ActingCommand for one thing
only: it finds each installation's root from the running `actingcommand-actingd.exe` and from the
registered watchdog tasks (`Get-ScheduledTask` in a hidden Windows PowerShell, read only, as the Runtime
reads its tasks), reads that root's `actingd.config.json` and nothing else, and marks on page 5 the
emulators it binds.

**Uninstalling:** there is no uninstaller. Stop the Runtime with "Request Shutdown" in this installation's
console; if automatic start is on, then run `"<root>\runtime\actingctl.exe" watchdog uninstall --root
"<root>"` (the console's settings page only shows the watchdog); then delete the whole `<root>` folder and
the shortcuts the wizard made (the finish page lists them).

**Command line:** with a command no window opens, and the output goes to the calling console (the release
build is a window program; its command line attaches to the parent's console).

```text
acsetup check-set --set <install set folder>
    checks the install set only and prints what it holds
acsetup discover [--mumu-root <MuMu folder>]
    lists MuMu's emulators and those another ActingCommand uses (read only)
acsetup install --set <install set folder> --root <new install folder> --plan <plan.json>
    installs as the plan says, with the steps of page 9
```

Exit codes: 0 done, 1 failed (stderr says which step and why, and lists how to undo what the install did,
as the failure page does), 2 usage. The plan file holds what the
window gathers on pages 1 to 8 (`actingcommand.install-wizard-plan.v1`): `language` (`zh` or `en`), `games`
(game folder names), an optional `mumu_root`, `instances` (each with `index`, `alias`, `game`, `server`, and
optionally `touch_backend`, `capture_backend`, `start_emulator`, `confirmed_in_use`) and `options`
(`desktop_shortcut`, `start_menu_shortcut`, `watchdog`, `start_after_install`, `pause_first_start`, with the
window's defaults). Its rules are the window's; a plan that breaks one stops before anything is written.

**Dependencies:** `crates/acui-wizard` uses Slint (the window, the same workspace dependency as the
console), `serde`, `serde_json`, `sha2` (verification), `zip` (`default-features = false`, only `deflate`,
the version line the Runtime locks), `getrandom` (the configuration's `secret_fingerprint_salt` and each
`instance_id`) and, on Windows only, `windows` 0.62 (`Win32_Foundation`, `Win32_System_Com`,
`Win32_System_RestartManager`, `Win32_UI_Shell`: the Start menu and desktop folders through
`SHGetKnownFolderPath`, shortcuts through `IShellLinkW` + `IPersistFile`, the folder picker through
`IFileOpenDialog`; the version Slint already locks, so the lock gains no crate). The exe carries the
console's icon, compiled in by `winresource`. The 0.11 wizard `crates/acui-setup` is out of the workspace,
its code left as it is; its A/B slots, online download, upgrades and resource-only updates are not part of
the new wizard.

The console's Win7 Aero frame adds this repository's `acui-glass` crate and no other crate to the lock. It
turns on Slint's `raw-window-handle-06` feature to reach the native window, and `acui-glass` uses
`raw-window-handle` 0.6 (the window as its face takes it, on every platform) and, on Windows only,
`windows` 0.62 (on top of the features above: `Foundation`, `Graphics_Effects`, `System`, `UI_Composition`,
`UI_Composition_Desktop`, `Win32_Graphics_Dwm`, `Win32_Graphics_Gdi`, `Win32_System_WinRT`,
`Win32_System_WinRT_Composition`, `Win32_System_WinRT_Graphics_Direct2D`, `Win32_UI_HiDpi` and
`Win32_UI_WindowsAndMessaging`: the subclass and hit test, the DWM attributes, the glass's window, Composition
and its Direct2D interop), `windows-core` (which the `#[implement]` macro names) and `windows-numerics`
(Composition's vectors), all at the versions Slint already locks.

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

One Cargo workspace, dependency direction app → model → rows ← source, and source → model for the
one frame-grouping function the live picture shares with the ledger window:

- `acui-rows`: the only place that names contract types for the view model; it re-exports the contract
  types, and adds display functions, the display-name dictionary, and two flattened structures filled by
  `acui-source` and read by `acui-model`.
- `acui-source`: the read face, the only place that touches the state root. The offline face is
  `EvidenceSource::open` / `query` / `open_report` / `read_material`;
  `Session::open(root, mode)` picks one of it and the online `OnlineSource` according to `--source`, or
  gives back `Session::Unopened` with the ledger's own error (`LedgerOpenFailure`: code, operation,
  detail, io kind) when the ledger refuses the offline face, or, for `auto` that cannot connect,
  `Session::OfflineOffered` with why, opening no face at all; `material_reader()` hands material
  reading to the background thread. It also reads the instances' task facts once per session
  (`instance_facts()`): offline replayed at the pinned position, online with their status, the two
  reads the pin is taken from; for the fact snapshot's scope and value types it names the contract crate directly.
- `acui-model`: a pure Rust view model (tabs, filtering, paging, recovery collapsing, selection), with no
  dependency on slint and **no plain language either** — it gives structured facts only, and all wording
  is chosen by `acui-app` from the language tables.
- `acui-app`: the only crate that depends on slint; the `.slint` files are in `crates/acui-app/ui/`, the
  two language tables in `strings.rs`, and settings-file reading and writing in `settings.rs`. What the
  windows share — the language, the text size, the theme, the launcher's view, and the registry of the
  workbench and the ledger windows — is in `ui.rs`, on the event loop only; each window's skin by the
  theme is set in `skin.rs`. The
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
is no approval entry point; of the console crates only two carry tests: `acui-model`, four unit tests of
`stopped`, the pure decision inside `stop_marked` (UI-3e3), and `acui-source`, six unit tests of
`pack::switch`, the pure decision of what a press of a task switch writes (UI-3f2). CI builds the workspace
and then runs just the crates that hold unit tests of pure functions, now
`cargo test --locked -p acui-model -p acui-source -p acui-wizard` (`acui-wizard` is the setup wizard, with
tests of the pure functions that only compute: the install set, the folder rules, the layout mapping, the
configuration, the name rules, the console's interface edges, the variables a child does not inherit,
what the Runtime's answers say, which emulators another installation binds and what a failure left to
undo); the UI and the workspace as a whole are not tested. The
launcher is in
`crates/acui-app/src/launcher.rs`, where the paths come from in `install.rs`, the technical log and the
startup-failure window in `report.rs`, and the client operations — probe, request shutdown, recording
the start press, the online open's status and fact reads, and the conditional resume — are in `acui-source` (`probe_runtime` /
`request_shutdown` / `record_start` / `instance_facts` / `resume_paused`).

Every background worker — the probe at startup, the whole start (probe, launch, readiness poll and its
record), request shutdown, unlock-owner, a conditional resume, a frame read, and every worker of the setup
wizard (checking the install set, checking the folder, the emulator search, the folder picker, the install) — is started through
`std::thread::Builder`. A thread the system refuses is stated, with the OS error, where that action
reports, and what the worker would have cleared (the probe, start, shutdown, unlock or resume in flight, the save
in progress, the frame request) is reset: never a panic on the event loop. A start whose thread cannot
start launches nothing, and the line says so. A child that was killed but could not be reaped is
said as that, not as a kill that failed.

Three more crates sit outside these four layers. `acui-glass` is the native half of the Win7 Aero frame,
used by `acui-app` through a safe face that knows nothing of Slint: all the console's Win32 and WinRT window
code and its unsafe code for them are there, and off Windows it only says it cannot. `acui-wizard` (binary
`acsetup`) is the 0.12 setup wizard: a back-end library with the window and the command line that share
it; it depends on none of the four layers (its dependencies are under "Setup wizard acsetup"). The 0.11
wizard `acui-setup` is out of the workspace.
`acui-installation` provides the shared installation-selection consumer for the console,
and the binary `acforward`, the fixed entry.

## Icon

The application icon is the black single-figure "commander" mark; the assets are in
`crates/acui-app/assets/`: `acui-256.png` (256×256 transparent PNG) and `acui.ico` (multi-size, 16..256).

- **Window and taskbar icon**: `Window.icon: @image-url("../assets/acui-256.png")` in `app.slint`.
- **Executable icon**: in `build.rs`, `#[cfg(windows)]` calls `winresource` to compile `acui.ico` into the
  exe's resource section; this dependency hangs under `[target.'cfg(windows)'.build-dependencies]` and is
  not compiled on Linux.
- **acsetup**: the same assets, not copied: `crates/acui-wizard/build.rs` and `ui/wizard.slint` point at
  those two files in `crates/acui-app/assets/` with relative paths; `ui/wizard.slint` imports the
  console's kit `crates/acui-app/ui/kit/` the same way.

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
  answers at the Runtime's latest position, at the pin or past it. Offline, `runtime_facts_at`
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
(contract / ledger / ledger-forensics / runtime-client / runtime-config / execution-kernel) are `AGPL-3.0-only`, and the
two combine under GPLv3 section 13.

No source code of another automation project is in this repository. The upstream license matrix of the
whole family is in the umbrella repository's
[LICENSE_POLICY.md](https://github.com/HS7097/ActingCommand/blob/main/LICENSE_POLICY.md).
