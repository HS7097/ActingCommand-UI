**🌐 Language / 语言:** English · [简体中文](./README.zh-CN.md)

# ActingCommand Console

**⚠️ This program is still iterating rapidly; expect it to be complete within 2–5 weeks.**

This is ActingCommand's **human console**, a read-only native program. On one Runtime state root it opens
the ledger's **official read face**, and renders the view pages the ledger itself hands out into a
three-column interface: instance card, timeline, detail pane.

It is not part of the Runtime; it is the Runtime's **external, detachable client**.

## Data source: the read face, not files

The program accepts only one argument: the state root. **All file IO belongs to the read face**; the
console never assembles paths inside the state root itself, never opens `ledger/`, `artifacts/` or
`runtime-state.sqlite`, and never launches any CLI.

There are two read faces, with the same query, page and cursor semantics; they differ only in where the
answers come from:

**Offline** (the original path, not a line changed):

- `GlobalLedger::open_metadata` opens the state root; the ledger itself determines the medium
  (segment / sqlite), authenticates the snapshot, and gives `latest_sequence` and the completeness
  observation. The whole session reads fixed at this one snapshot position.
- `actingcommand_ledger_forensics::query_view_page` produces the official page `RuntimeEventQueryPage`:
  events, the view membership each event carries, the read range, the page cursor, run recovery
  grouping, and artifact eviction facts.
- `actingcommand_ledger_forensics::read_material_to` reads material: for each chunk the read face
  resolves the reference, takes the shared read protection, re-checks the reference and the retention
  state, and hands over that chunk's bytes only after the whole-file sha256 check passes.

**Online** (via `actingcommand-runtime-client`, the client's only typed IPC path):

- `RuntimeClient::connect(RuntimeClientConfig::new(state_root, Ui, Ui))`: `runtime-info.json` is read by
  the client itself, the loopback address is taken by it, and the owner epoch is checked by it at connect
  time. The read face is only a client: it does not kill or wait for the Runtime, does not touch
  `owner.lock`, and writes nothing into the state root; launching and requesting shutdown belong to the
  top bar's launcher (see the "Launcher" section), and shutdown too goes only through this same typed
  client.
- The first page at startup asks without a snapshot position; the `snapshot_ledger_position` the Runtime
  states on that page is the position this session reads at, fixed — as offline, one snapshot for the
  whole session. Every page after that is
  `RuntimeClient::query_event_page(query, ProjectionProfile::Ui, page.at_snapshot(pos))`,
  the same `EventQuery`, the same page limit, the same `next_cursor`.
- Material goes through `RuntimeClient::read_material`: the same `RuntimeMaterialReadRequest`, the same
  chunking and whole-file verification, only the verification is done by the Runtime, on the same connection.
- Medium, corrupt tail and writer-process record are the offline read face's observations of files; the
  Runtime does not state these on the page. Online, the instance card's "storage format" says "determined
  by the Runtime", and "writer process" states the connected Runtime itself
  (PID, owner epoch, start time, all out of its own `runtime-info.json`).

`--source <auto|offline|online>`, default `auto`: if the client can connect to the Runtime the state root
points at, online; otherwise offline. The instance card's first line, "read face", states which one was
chosen and why (`runtime-info.json` absent, or the client error code of the failed connection). With
`online`, a failed connection **exits directly carrying the client's error code**; it does not quietly
switch to offline. Across both read faces, a Runtime that connects but cannot answer the first page is an
error in any mode, with no fallback.

Dependencies are pinned to the Runtime's **main** (`Cargo.toml`):

```
rev = "7f3df214ed613dcae20780bce26e384eee95310a"
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
  rows already held locally and then call itself a ledger query. An id must be a complete canonical id,
  otherwise it states that a complete identifier is required. The "source module" options are **rebuilt
  from the current page on every load**, and the selected item is resolved by **module name** — the list
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
  of bound ids and the sequence number of the most recent binding; the severity counts and this page's row
  count still come from the re-queried page. Rows gain a "port" column: an event with no instance
  association says "host"; one with an association whose id's most recent binding gave HOST:PORT says the
  port number; one with an association but absent from the port table (fixtures, serial-port
  configurations, no binding seen) says the abbreviated instance id — no port is invented.
- **Paging is the page cursor**: "continue reading" takes the `next_cursor` the page gave, fetches the
  next page and appends it. The top bar permanently shows "read up to row N", with "source incomplete"
  appended when the source is incomplete.
- **Recovery grouping comes from the ledger**: the `run_recovery` carried on the page inserts a group row
  before the first row of each run, showing the state the ledger gives (recovered / unresolved / unknown),
  the grounds (row N failed, row M recovered) and the gaps; failure rows the ledger judges recovered are
  collapsed under the group row with a "recovered" mark. This is read-time grouping, not a rewrite of
  failure events. Across pages it **merges by run id**: a later page only adds evidence into it, positions
  given by earlier pages are all kept, and "recovered" rows already collapsed are not re-expanded by a
  later page.
- **Time upper bound**: the slider's starting position is the state it represents — the far right is
  "all", and the first drag narrows.
- **Row types are no longer mirrored**: `acui-rows` re-exports the contract types directly, and only adds
  display functions such as local time, id abbreviation, wire codes and the display-name dictionary.

## Frame material: read, but only what is verified

The ruling has changed: the console **does** load frame bytes, but only through the material read face,
and only within this rule:

- Only the `capture.frame` artifact **the selected event itself** carries is read, on demand, one at a time.
- Requests are chunked by `MAX_RUNTIME_MATERIAL_CHUNK_BYTES` (64 KiB), and for each chunk the read face
  verifies the whole file's length and sha256; if any chunk is not `verified` it aborts, and all bytes
  already obtained are discarded.
- The whole-file limit is the contract's chunk limit × 128 chunks (8 MiB); an artifact beyond that is
  refused outright, with a statement.
- When an artifact has been evicted, only the eviction facts are shown (disposition, intent/outcome
  position, the position observed up to); the file is not touched.
- On a read failure, the state and safe error code the read face gives are shown.
- Reading is done on a background thread, and every request carries a token; if a slow read returns after
  the selection has changed, it is discarded. **A stale or unverified image is never displayed, at any
  time.**
- Reading is done by one background worker thread, **only one at a time**: the one whose request has been
  displaced stops before the next chunk and sends no further chunk — every chunk re-hashes the whole
  material, so letting a read nobody is waiting for keep running is the most expensive mistake.
- Decoding uses only `image` (with only the `png` feature enabled, version pinned in the workspace's
  `[workspace.dependencies]`). The program decodes only two kinds of image: frames read back this way,
  and its own application icon.

The geometry overlay shares one coordinate system with the frame: if the payload gives the screen size it
is used, otherwise the decoded pixel size is used.

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
and covers all 115 `event_type`s and 19 `origin.module`s in the contract; **anything not in the table is
displayed verbatim, not guessed**.

### Settings file

Language and text size are stored under the per-user configuration directory:

```
Windows:  %APPDATA%\ActingCommand\acui.toml
Linux:    $XDG_CONFIG_HOME/ActingCommand/acui.toml（没有就用 $HOME/.config/…）
```

```toml
lang = "zh"          # zh | en
text_size = "standard"   # standard | large | extra-large
state_root = 'D:\ActingCommand\state'                    # 可选，绝对路径
actingd_config = 'D:\ActingCommand\actingd.toml'         # 可选，绝对路径
actingd_exe = 'D:\ActingCommand\actingcommand-actingd.exe'   # 可选，绝对路径
```

It is read once at startup and written once on every dropdown change; on write-back the three path keys
are kept verbatim. **This is the only file the console itself reads and writes**: it is not inside any
state root, and the state root still belongs entirely to the read face. If the file is absent, unreadable,
or holds an unrecognized value, the defaults (Chinese, standard) are used. The parser is hand-written: one
`key = value` per line, one matching pair of quotes stripped, no escape handling — write Windows paths in
single quotes (TOML literal strings), not as `"D:\\…"`.

## Launcher

The third line of the top bar. On the left is the Runtime state from the last probe, then two buttons, and
the line below it is the result of the last button press.

- **Runtime state**: probed once at startup, and probed again after every button press. A probe is one
  `RuntimeClient::connect`: on a connection it says "running · PID · owner epoch" (taken from its own
  `runtime-info.json`, checked against the client's owner epoch); on no connection it says "not running"
  plus the client's error code and operation name, guessing no cause.
- **Start**: probe first; if it is already running it only says "already running, not launched".
  Otherwise it launches `actingcommand-actingd --config <actingd_config>` detached, from `actingd_exe` —
  that is the whole command line. If either of the two keys is missing or is not an absolute path, it
  states which key it is and launches nothing. stdout / stderr both go into a log under the console's
  **own** directory: `%LOCALAPPDATA%\ActingCommand\logs\actingd-<unix_ms>.log` (Linux: the same path under
  `$XDG_STATE_HOME` or `$HOME/.local/state`); the directory is created by the console and is **never
  inside the state root**. On Windows it is launched with `DETACHED_PROCESS`: the daemon does not inherit
  the console program's console, and does not receive its Ctrl+C.
- **Readiness decision**: at most 60 attempts, 500 ms each. Each attempt does `try_wait()` first: if the
  child process has exited it stops and states the exit code and the log path; if it has not exited it
  does one more `connect`, and a connection means ready, stating the PID and owner epoch. If this console
  is reading offline, it appends the sentence "to read online, restart with `--source online`" — it
  **never quietly switches read face mid-session**. If all 60 attempts fail to connect, it says "still not
  ready" and the last client error code. It **does not parse the daemon's output**.
- **Request shutdown**: only through the typed client, never killing a process. It opens a new connection,
  opens an interaction with `begin_interaction()`, first records this button press as a `client_action`
  with `record_client_action_receipt` (surface `acui.launcher`, control `request_shutdown`), and only
  after obtaining a receipt bearing terminal does it send `request_shutdown()` — the action lands in the
  ledger first, then the request. If accepted, it states the receipt state, the request id and the
  sequence number at which the action landed in the ledger; if refused (owner / governance and the like)
  it states the Runtime's refusal code **verbatim**, plus the client error code and operation name; it
  **does not retry**. Afterwards it probes the state once more — the Runtime stops at its own pace, so
  this glance may still say running.
- **Never kill**: the `Child` handle is used only for `try_wait()`, to see whether it exited early — no
  `kill`, no blocking `wait`, no job object attached; the handle is dropped once the readiness decision
  ends, and the daemon outlives the console.

Pause/resume, unlocking the owner, start-at-boot, the installer and network downloads are all outside this
slice.

## Setup wizard acsetup

`crates/acui-setup` is a standalone binary `acsetup.exe` (a Slint window, the same styling and icon as the
console) that installs the release files from the umbrella repository's
[Releases](https://github.com/HS7097/ActingCommand/releases) into a **per-user** installation. It ships
together with the UI repository's Windows build artifact (`acui-windows-<sha>.zip` gains one more file,
`acsetup.exe`). **v1 is offline**: there is no networking code in the program at all, and a person
downloads the release files into a folder first. One window, back / next, six steps:

0. **Prepare**: the install root (changeable, default `%LOCALAPPDATA%\Programs\ActingCommand`, no
   administrator needed), the free space on that volume, whether an installation is already here (looking
   at `runtime\BUILD-MANIFEST.json`; if there is one it stops at this step — v1 has no upgrade flow, so
   pick another root), and the folder holding the release files (default `%USERPROFILE%\Downloads`,
   changeable; v1 has no native directory dialog, so the path is typed in directly).
1. **Verify**: the folder is required to hold `SHA256SUMS`, `MEMBERS.json`,
   `actingcommand-runtime-<sha>.zip`, `actingcommand-tools-<sha>.zip` and `acui-windows-<sha>.zip`
   (`<sha>` taken from `MEMBERS.json`'s `runtime_sha` / `ui_sha`; the three zips must appear in
   `SHA256SUMS`). `SHA256SUMS` is checked entry by entry; the archives are extracted into the temporary
   directory `.staging-<unix_ms>` under the install root; then, against the `BUILD-MANIFEST.json` each zip
   carries, the source repository, the commit id (equal to the MEMBERS sha), the Runtime's
   `runtime_payload_layout` (`distribution-v1`) and the size and sha256 of every entry in `files[]` are
   checked; a file in the zip that the manifest does not list also counts as a mismatch. Any mismatch
   stops it, worded as "the content differs from what it was at creation" — this is an integrity
   statement, not an authorization tone. Nothing inside the zips is run during verification.
2. **Lay out**: `runtime\` (the Runtime's entire payload + manifest, with `actingd.config.example.json`
   byte-for-byte verbatim), `ui\` (the console payload + manifest), `tools\` (**only** `actinglab.exe`,
   `actingledger.exe` and `ac_fastdeploy_ppocr.dll`; the other two exes in the tools pack are neither
   installed nor shown). The temporary directory is deleted afterwards.
3. **Configure**: the state root defaults to `<install root>\state` (it must not exist or must be an empty
   directory; **a state root that already holds content is never taken over**); `secret_fingerprint_salt`
   is generated as the hex of 32 bytes from the system random source (`getrandom`; **not displayed, not
   logged**); `<install root>\actingd.config.json` is written, with only the fields `schema_version`,
   `state_root`, `bind_host` (127.0.0.1), `bind_port` (0), `secret_fingerprint_salt` and `instances`
   (empty) — the Runtime's parser is `deny_unknown_fields`, so not one extra field is written. Then the
   console settings `%APPDATA%\ActingCommand\acui.toml` are written with `state_root`, `actingd_config`
   and `actingd_exe` (the format of the "Settings file" section, single-quoted literals; existing `lang` /
   `text_size` kept verbatim). The way it writes matches `crates/acui-app/src/settings.rs`, but
   `acui-setup` does not depend on `acui-app` and is a small duplicate writer. **Instances (emulators /
   devices) are not configured in the wizard**; `instances` is left empty and they are added in the
   console afterwards.
4. **Start at boot** (optional, unchecked by default): only when checked does it write
   `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\ActingCommand.cmd` in the per-user startup
   folder, whose content is
   `start "" "<install root>\runtime\actingcommand-actingd.exe" --config "<install root>\actingd.config.json"`;
   only when "also launch the console" is checked as well does it add one more line,
   `start "" "<install root>\ui\acui.exe"`. acsetup does not appear in the batch file. Unchecked, it
   writes nothing; a file of the same name already in the startup folder is left alone, and merely
   mentioned in one sentence on the finish page.
5. **Finish**: "启动监控台 / Open console" launches `<install root>\ui\acui.exe` detached (it never
   launches actingd directly; the Runtime is launched by the console's launcher) and closes the wizard;
   "finish" only closes.

**Install log**: from step 1 onward every step appends a plain-language line to
`<install root>\acsetup-<unix_ms>.log`; on failure the last line states the reason, and the window shows
the log path. Apart from the installed payload, the configuration, the settings and (when checked) the
start-at-boot batch file, this is the only file the wizard writes.

**Things it never does**: it does not install a service, does not create a scheduled task, does not change
PATH, does not write the registry; does not modify the configuration template; does not touch a state root
that already holds content; does not configure instances; does not go on the network; does not upgrade and
does not install resource packs. On Linux the crate compiles as usual (CI runs `--workspace` on both
legs), and running it exits immediately with `acsetup v1 is Windows-only`.

Only three dependencies are added, each with its purpose noted in `[workspace.dependencies]`: `sha2`
(verification), `zip` (`default-features = false`, only `deflate` enabled, the same version line the
Runtime locks) and `getrandom` (the salt).

## Four layers, four crates

One Cargo workspace, dependency direction app → model → rows ← source:

- `acui-rows`: the only place that names contract types for the view model; it re-exports the contract
  types, and adds display functions, the display-name dictionary, and two flattened structures filled by
  `acui-source` and read by `acui-model`.
- `acui-source`: the read face, the only place that touches the state root. The offline
  `EvidenceSource::open` / `query` / `open_report` / `read_material` are kept verbatim;
  `ReadSource::open(root, mode)` picks one of it and the online `OnlineSource` according to `--source`,
  and `material_reader()` hands material reading to the background thread.
- `acui-model`: a pure Rust view model (tabs, filtering, paging, recovery collapsing, selection), with no
  dependency on slint and **no plain language either** — it gives structured facts only, and all wording
  is chosen by `acui-app` from the language tables.
- `acui-app`: the only crate that depends on slint; the `.slint` files are in `crates/acui-app/ui/`, the
  two language tables in `strings.rs`, and settings-file reading and writing in `settings.rs`.

`slint` 1.17.x, `default-features = false`; the ledger is read-only, the only control entry points are the
launcher's two buttons (start / request shutdown, see above), and there is no approval entry point; no
tests are written. The launcher is in `crates/acui-app/src/launcher.rs`, and the two client operations,
probe and request shutdown, are in `acui-source` (`probe_runtime` / `request_shutdown`).

A fifth crate, `acui-setup` (binary `acsetup`), sits outside these four layers: the setup wizard,
depending only on slint, serde, sha2, zip and getrandom, and on none of the layers above; see the previous
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

These are not worked around; they are displayed as they are, and booked here:

- **There is no `event_count` or `repair_count`**. `GlobalLedgerMetadata`
  (`crates/ledger/src/global/evidence.rs:257`) gives only `latest_sequence` / `read_complete` /
  `backend` / `writer_metadata` / `corrupt_tail`, with no accessor for the event count or the repair
  count; the only thing that gives these two, `GlobalLedger::open_evidence` (same file, `:417`), requires
  the caller to hand over a `VerifiedArtifactReference` for every artifact reference, and events that
  cannot be verified are dropped (measured on the 0828 root: of 2585 rows only 10 were left), which
  amounts to hashing the entire artifacts directory (457 MB) at startup. The instance card therefore
  displays `event_count` as "—(not given by the read face, see README)", and separately marks the number
  of rows **loaded in this view**; the two are never mixed.
- **There is no whole-material entry point, and reading one frame is expensive**. `read_material_to` in
  `crates/ledger-forensics/src/material.rs:51` does one chunk only, and every chunk has to reopen the
  ledger metadata twice and re-hash the whole material; reading one 3.6 MB frame takes 57 chunks, measured
  at about 5 seconds (release). There is no whole-file read entry point, and no reader reused across
  chunks, so the console puts reading on a background thread rather than assembling a simplified read
  path of its own.
- **Geometry and frames cannot be brought together on these two roots**. In the 0828 and v5 roots, the
  only events carrying a `capture.frame` artifact are `artifact.created` / `artifact.verified`, and their
  payloads hold no geometry; the only events carrying geometry are `task.effect_intent` (six on 0828,
  five on v5), whose payload is a single tap coordinate and whose `links` hold **no** `frame_id`. The
  ledger gives no relation joining the two, so the console does not join them — the real frame is drawn as
  it is, and the overlay is empty.
- **Neither root holds artifact eviction facts**, so the eviction placeholder does not appear on these two
  roots; the code path is written to the contract.

## License

`GPL-3.0-only` (Alice ruled on 2026-09-17). The repository includes the full LICENSE text; the workspace
`license` field and the SPDX header of every `.rs` / `.slint` file agree with it. The interface is
rendered by [Slint](https://slint.dev), used under its GPLv3 licensing option. The Runtime crates depended
on (contract / ledger / ledger-forensics / runtime-client) are `AGPL-3.0-only`, and the two combine under
GPLv3 section 13.
