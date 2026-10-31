// SPDX-License-Identifier: GPL-3.0-only
//! The read face, and the only place that touches a state root.
//!
//! The console never joins a path into the state root, never opens `ledger/`,
//! `artifacts/` or `runtime-state.sqlite`, and never spawns a CLI. Everything
//! below goes through the Runtime's own entries. Offline, `GlobalLedger` picks
//! the medium and opens a bounded snapshot at one pinned point,
//! `query_view_page` projects the formal page (at most 256 records or 768 KiB,
//! with a cursor to the rest), and `read_material_complete` resolves, guards
//! and verifies material.
//! Offline, `runtime_facts_at` replays the instances' task facts at the pinned
//! position. Online, the same pages, material whole (the client's own
//! `read_material_complete` assembles the verified ranges), the instances'
//! status and task facts, and instance discovery are asked of the running
//! Runtime through `actingcommand_runtime_client`, the one typed IPC path a
//! client has. This
//! crate is a client only: it never starts, kills or waits on the Runtime, and
//! never writes into the state root. The launcher's two
//! questions — is a Runtime running here, and will it accept a shutdown
//! request — are asked through that same typed client, and its start press is
//! recorded through it, at the end of this file.
//! Every connection a session keeps is rebuilt after the client latched it
//! (see `Rebuilding`); what a person does goes out on a fresh connection each
//! time.

use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use acui_rows::outcome::{
    CONSOLE_CLIENT_ACTION_TERMINAL_MISSING, CONSOLE_INSTANCE_BINDING_MISSING,
    CONSOLE_RUNTIME_OWNER_CHANGED, CONSOLE_STATUS_SOURCE_MISSING,
};
use acui_rows::{
    code, page_source_complete, ArtifactEvictionObservation, ClientActionKind, ClientActionRecord,
    EventActor, EventQuery, EventSource, LedgerCount, LedgerEventPosition, LedgerView, OpenReport,
    PortBindings, PortEntry, ProjectedArtifactReference, ProjectionProfile,
    RuntimeEventQueryCursor, RuntimeEventQueryPage, RuntimeEventQueryPageRequest,
    RuntimeMaterialReadLimit, RuntimeMaterialReadState, WriterFacts,
};
use actingcommand_contract::outcome::{Code, Location};
use actingcommand_contract::{
    FactValue, OwnerEpoch, RuntimeFactRecord, RuntimeFactScope, RuntimeFactSnapshot,
};
use actingcommand_ledger::{
    GlobalLedger, GlobalLedgerError, GlobalLedgerEvidenceConfig, GlobalLedgerMetadata,
    GlobalLedgerWriterMetadataObservation, LedgerArtifactSelection,
};
use actingcommand_ledger_forensics::{ForensicMaterialCompleteResult, ForensicRuntimeFactsResult};
use actingcommand_runtime_client::{
    RuntimeChannel, RuntimeClient, RuntimeClientConfig, RuntimeClientError,
    RuntimeMaterialCompleteResult, RuntimeMaterialSelection,
};
use anyhow::{anyhow, bail, Result};

/// The console's cap on one material, on either face: 8 MiB, the bound it kept
/// while it still assembled 64 KiB online ranges itself (128 of them). Both
/// faces' whole-object reads take it as their byte limit.
pub const MAX_FRAME_BYTES: u64 = 8 * 1024 * 1024;

/// Bound on one whole-object read, offline or online. The contract's 4 s
/// `RUNTIME_MATERIAL_READ_BUDGET_MS` bounds a single range read, not a whole
/// object; a verified read of at most `MAX_FRAME_BYTES` is expected to finish
/// well inside it. Online the deadline is cooperative: the client checks it
/// between ranges, not inside one exchange.
const MATERIAL_READ_DEADLINE: Duration = Duration::from_secs(30);

/// Bound on the offline face's one fact replay per session.
const FACTS_READ_DEADLINE: Duration = Duration::from_secs(30);
/// The offline face's own opening of the ledger. On an SQLite root the opening
/// is a bounded snapshot (Runtime 0.12.32, Workflow #422 LN-OFF-1): the writer
/// open's checks, `1..=head` contiguous, the newest records authenticated and
/// the eviction proof set read, no other history; each later read is one page
/// of at most `MAX_RUNTIME_EVENT_QUERY_EVENTS` records or 768 KiB with a cursor
/// to the rest (Alice, decision 115). So the opening takes a deadline only, no
/// byte or event limit, which would refuse a ledger for how much it holds.
/// 60 s, not the ledger's 600 s default: the session opens before the window
/// shows. Past it, `ledger_read_budget_exceeded` (detail `phase=open|proofs`)
/// and the session stays unopened, loudly. A Segment root (test builds only)
/// is read whole within it.
const OFFLINE_OPEN_DEADLINE: Duration = Duration::from_secs(60);
/// Bound on each later read of the offline snapshot: one page, or the port
/// map's binding read. On an SQLite root the ledger gives each read its own
/// 10 s from its start (`ledger_observer.writer_reply_timeout_ms`'s factory
/// value) and a caller can only tighten it, so for one page this changes
/// nothing; it caps the binding read, which pages under one call, at 30 s in
/// all. The deadline a snapshot holds is sticky, so each read sets its own
/// first (`set_read_deadline`). 30 s, as the facts replay and a material read,
/// each of which opens the ledger anew within that same bound.
const OFFLINE_READ_DEADLINE: Duration = Duration::from_secs(30);

/// The io timeout of the session's ledger page connection (see `LedgerPages`):
/// how long one page exchange waits for its receipt, and the bound of that
/// connection's connect and owner check. The client's default, kept by the
/// main connection, is 5 s; a page the Runtime answers from a busy host (CPU at
/// 100 %) can take longer, and one timeout latches the connection it happened
/// on. The client refuses anything above 60 s at connect
/// (`runtime_client_config_invalid`). A page query runs synchronously on the
/// UI event loop, so the longer this is, the longer one slow page can hang the
/// window: hence 20 s, not the 60 s cap. One page query can wait several
/// times this. A rebuild of the page connection waits for its TCP connect, for
/// its owner check's receipt (`Health`) and for its channel binding's receipt
/// (`BindChannel`), each bounded by this; a query whose last one latched the
/// connection rebuilds it first, then asks, and on a timeout waits
/// `LEDGER_PAGE_RETRY_BACKOFF`, rebuilds again and asks once more: at worst
/// eight times this plus the backoff, 161 s, and on loopback, where the connect
/// is answered at once, about six times, 121 s.
const LEDGER_PAGE_IO_TIMEOUT: Duration = Duration::from_secs(20);

/// The wait before the one retry of a page exchange that timed out.
const LEDGER_PAGE_RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// The spelling of a client error's `code()` or `operation()`, whichever type
/// the pinned client returns them as: `&'static str`, or the contract's
/// registered `Code` and `Location`, whose `as_str` is that same spelling.
/// Every comparison of a client error's code, and every `&'static str` field
/// filled from one, reads it through here.
trait Spelling {
    fn spelling(self) -> &'static str;
}

impl Spelling for &'static str {
    fn spelling(self) -> &'static str {
        self
    }
}

impl Spelling for Code {
    fn spelling(self) -> &'static str {
        self.as_str()
    }
}

impl Spelling for Location {
    fn spelling(self) -> &'static str {
        self.as_str()
    }
}

/// One bounded snapshot of one state root, pinned at its opening.
pub struct EvidenceSource {
    root: PathBuf,
    /// In a cell because each read resets its deadline (see `snapshot_for_read`)
    /// while the session reads through `&self`, on the event loop only.
    snapshot: RefCell<GlobalLedgerMetadata>,
}

/// The ledger's refusal to open a state root: its own code, operation and
/// detail, verbatim, and for an io error its kind as the ledger spells it
/// (`not_found`, `permission_denied`, …). The kind, not the localized
/// `detail`, tells a state root with no ledger yet from one whose ledger
/// cannot be read; `detail` is never parsed here.
#[derive(Debug, Clone)]
pub struct LedgerOpenFailure {
    pub code: &'static str,
    pub operation: &'static str,
    pub detail: Option<String>,
    pub io_kind: Option<String>,
}

impl From<GlobalLedgerError> for LedgerOpenFailure {
    fn from(error: GlobalLedgerError) -> Self {
        Self {
            code: error.code(),
            operation: error.operation(),
            detail: error.detail().map(str::to_string),
            io_kind: error.io_kind().map(|kind| code(&kind)),
        }
    }
}

impl EvidenceSource {
    pub fn open(state_root: impl Into<PathBuf>) -> Result<Self, LedgerOpenFailure> {
        let root = state_root.into();
        let config = GlobalLedgerEvidenceConfig::new(&root)
            .with_deadline(Instant::now() + OFFLINE_OPEN_DEADLINE);
        let snapshot = GlobalLedger::open_metadata(config)?;
        Ok(Self { root, snapshot: RefCell::new(snapshot) })
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }

    /// The committed position this whole session reads at.
    pub fn snapshot_position(&self) -> u64 {
        self.snapshot.borrow().latest_sequence()
    }

    /// The snapshot for one read, its deadline first set to
    /// `OFFLINE_READ_DEADLINE` from now, so no read runs under the deadline an
    /// earlier one set.
    fn snapshot_for_read(&self) -> Ref<'_, GlobalLedgerMetadata> {
        self.snapshot
            .borrow_mut()
            .set_read_deadline(Instant::now() + OFFLINE_READ_DEADLINE);
        self.snapshot.borrow()
    }

    /// One formal page: events, their view membership, read scope, cursor,
    /// run recovery groups and artifact eviction facts.
    pub fn query(
        &self,
        query: &EventQuery,
        limit: u16,
        cursor: Option<RuntimeEventQueryCursor>,
    ) -> Result<RuntimeEventQueryPage> {
        let request = RuntimeEventQueryPageRequest::new(limit, cursor)
            .map_err(|error| anyhow!("页请求无效：{}", error.code()))?
            .at_snapshot(self.snapshot_position())
            .map_err(|error| anyhow!("快照位置无效：{}", error.code()))?;
        actingcommand_ledger_forensics::query_view_page(
            &self.snapshot_for_read(),
            query,
            ProjectionProfile::Ui,
            &request,
        )
        .map_err(|error| anyhow!("账本查询失败：{}", error))
    }

    /// Instance identity by ADB port at this session's snapshot, as the read
    /// face derives it from every `runtime.instance_bound` fact. Read once per
    /// session: the snapshot is pinned, so the answer cannot change. A ledger
    /// with no binding facts is an empty map, not an error; a typed refusal
    /// (`instance_bindings_incomplete`, `instance_binding_malformed`, …) comes
    /// back with its code as the outermost context.
    pub fn instance_bindings(&self) -> Result<PortBindings> {
        let bindings = actingcommand_ledger_forensics::instance_bindings(
            &self.snapshot_for_read(),
            self.snapshot_position(),
        )
        .map_err(|error| {
            let code = error.code();
            anyhow::Error::new(error).context(code)
        })?;
        let mut ports = Vec::with_capacity(bindings.ports.len());
        for (port, members) in &bindings.ports {
            // The facts shown for a port are those of its latest binding.
            let latest = members
                .iter()
                .filter_map(|id| bindings.latest.get(id))
                .max_by_key(|binding| binding.sequence)
                .ok_or_else(|| {
                    anyhow!("{CONSOLE_INSTANCE_BINDING_MISSING}: 端口 {port} 没有绑定事实")
                })?;
            ports.push(PortEntry {
                port: *port,
                members: members.clone(),
                latest_instance_id: latest.instance_id,
                latest_alias: latest.instance_alias.clone(),
                latest_provenance: code(&latest.provenance),
                latest_sequence: latest.sequence,
            });
        }
        // An id's own port is the one its latest binding names as HOST:PORT —
        // the same rule the read face uses for port membership.
        let port_of: BTreeMap<_, _> = bindings
            .latest
            .values()
            .filter(|binding| !binding.serial_configured)
            .filter_map(|binding| binding.adb_port.map(|port| (binding.instance_id, port)))
            .collect();
        Ok(PortBindings {
            ports,
            unported: bindings.unported.into_iter().collect(),
            port_of,
        })
    }

    /// The runtime fact store replayed at this session's snapshot position by
    /// the read face's own `runtime_facts_at`: the same position every page
    /// reads at. Read once per session; the snapshot is pinned.
    pub fn instance_facts(&self) -> OfflineFacts {
        let position = self.snapshot_position();
        // Positions start at 1: at 0 the ledger holds no event, and the read
        // face would refuse the position itself rather than say so.
        if position == 0 {
            return OfflineFacts::NoEvents;
        }
        let deadline = Instant::now() + FACTS_READ_DEADLINE;
        let result =
            actingcommand_ledger_forensics::runtime_facts_at(&self.root, position, deadline);
        match result {
            ForensicRuntimeFactsResult::Available { position, facts, .. } => {
                let mut instances = Vec::new();
                fold_task_facts(&facts.records, &mut instances);
                OfflineFacts::Available { position, instances }
            }
            ForensicRuntimeFactsResult::NotAvailable { position, latest_sequence, reason } => {
                OfflineFacts::NotAvailable { position, latest_sequence, reason: code(&reason) }
            }
            ForensicRuntimeFactsResult::Failed { position, outcome, detail, io_kind } => {
                let io_kind = io_kind.map(|kind| acui_rows::code(&kind));
                let operation = outcome
                    .value("operation")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                let code = outcome.code().as_str().to_owned();
                OfflineFacts::Failed { position, code, operation, detail, io_kind }
            }
        }
    }

    pub fn open_report(&self) -> OpenReport {
        let snapshot = self.snapshot.borrow();
        let event_count = snapshot.event_count() as u64;
        let complete = snapshot.read_complete() && snapshot.corrupt_tail().is_none();
        OpenReport {
            backend: snapshot.backend().to_string(),
            latest_sequence: snapshot.latest_sequence(),
            event_count: if complete {
                LedgerCount::Counted(event_count)
            } else {
                LedgerCount::VerifiedPrefix(event_count)
            },
            read_complete: snapshot.read_complete(),
            corrupt_tail: snapshot.corrupt_tail().map(|tail| {
                format!(
                    "{} 段{} 偏移{} 悬挂{}字节",
                    tail.code, tail.segment_index, tail.byte_offset, tail.dangling_byte_count
                )
            }),
            repair_count: match snapshot.repair_count() {
                Some(count) => LedgerCount::Counted(count as u64),
                None => LedgerCount::NoRepairLog,
            },
            writer: match snapshot.writer_metadata() {
                GlobalLedgerWriterMetadataObservation::Absent => WriterFacts::Absent,
                GlobalLedgerWriterMetadataObservation::Locked { byte_count } => {
                    WriterFacts::Locked { byte_count: *byte_count }
                }
                GlobalLedgerWriterMetadataObservation::Readable(metadata) => {
                    WriterFacts::Readable {
                        owner_id: metadata.owner_id().to_string(),
                        pid: metadata.pid(),
                        active: metadata.active(),
                        started_at_unix_ms: metadata.started_at_unix_ms(),
                    }
                }
            },
        }
    }
}

/// One session against the running Runtime, read at the one snapshot the
/// Runtime stated on the session's first page — the same pinned-snapshot
/// session the offline face gives, so both faces page and cursor alike.
pub struct OnlineSource {
    root: PathBuf,
    /// The session's main connection, at the client's default io timeout:
    /// follow polls, the instances' status and facts, material reads and
    /// `open_report`. Rebuilt after the client latched it (see `Rebuilding`).
    main: Rebuilding,
    /// The connection every page, the first included, and every repin reads
    /// through.
    pages: LedgerPages,
    /// The pin every page reads at. `repin` moves it on a person's jump to the
    /// latest or when following is turned on; `poll` moves it while following.
    snapshot: Cell<u64>,
    read_complete: Cell<bool>,
    instances: RefCell<Result<RuntimeInstances, ClientFailure>>,
}

/// How a kept connection is made again: `OnlineSource::connect` for the main
/// connection, `LedgerPages::connect` for the page one.
type Connect = fn(&Path) -> Result<RuntimeClient, RuntimeClientError>;

/// A connection the session keeps — the main one or the page one — rebuilt
/// after the client latched it. R-0c (Runtime contracts/runtime-interface.md,
/// the client methods): a receipt timeout, and any exchange that fails fatally
/// (`is_fatal`: without the Runtime's error projection, or with a fatal one),
/// latches the connection, and every later exchange on it or on a clone of it
/// fails the same way; the Runtime is asked again only on a new connection. So
/// the first exchange after such a failure connects anew, to the session's
/// owner epoch, and goes out there. Nothing is sent again here: a failed
/// exchange comes back as it failed. A rebuild waits for its TCP connect,
/// `Health` and `BindChannel`, each within the connection's io timeout. (A fatal
/// failure before the send latches nothing; the rebuild then costs one connect.)
struct Rebuilding {
    connect: Connect,
    connection: RefCell<RuntimeClient>,
    /// The session's owner epoch, of its first connection: a rebuilt connection
    /// must reach that same Runtime, since every pin was stated by it.
    owner_epoch: OwnerEpoch,
    /// Set once an exchange on the connection, or on a clone of it, failed
    /// fatally; a material read on the frame worker sets it there (see
    /// `MaterialReader::Online`). Cleared by a rebuild.
    latched: Arc<AtomicBool>,
}

/// Why a rebuild left the latched connection in place.
enum RebuildFailed {
    Connect(RuntimeClientError),
    /// The new connection reached a Runtime of another owner epoch.
    OwnerEpochChanged,
}

impl fmt::Display for RebuildFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "{error}"),
            Self::OwnerEpochChanged => formatter.write_str(CONSOLE_RUNTIME_OWNER_CHANGED),
        }
    }
}

impl RebuildFailed {
    /// As the card shows a client failure: the client's own error, or the
    /// console's own `CONSOLE_RUNTIME_OWNER_CHANGED` at the client's connect
    /// operation.
    fn into_failure(self) -> ClientFailure {
        match self {
            Self::Connect(error) => ClientFailure::from(error),
            Self::OwnerEpochChanged => ClientFailure {
                code: CONSOLE_RUNTIME_OWNER_CHANGED,
                operation: CONNECT_RUNTIME,
                runtime_code: None,
                host: None,
            },
        }
    }
}

impl Rebuilding {
    fn new(connect: Connect, client: RuntimeClient, owner_epoch: OwnerEpoch) -> Self {
        Self {
            connect,
            connection: RefCell::new(client),
            owner_epoch,
            latched: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Replaces the connection with a fresh one to the session's Runtime and
    /// clears `latched`. When the connect fails, or reaches a Runtime of another
    /// owner epoch, the old connection and `latched` stay.
    fn reconnect(&self, root: &Path) -> Result<(), RebuildFailed> {
        let client = (self.connect)(root).map_err(RebuildFailed::Connect)?;
        if client.runtime_info().owner_epoch() != self.owner_epoch {
            return Err(RebuildFailed::OwnerEpochChanged);
        }
        *self.connection.borrow_mut() = client;
        self.latched.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// The connection to ask on: rebuilt first when an earlier exchange latched it.
    fn client(&self, root: &Path) -> Result<RuntimeClient, RebuildFailed> {
        if self.latched.load(Ordering::SeqCst) {
            self.reconnect(root)?;
        }
        Ok(self.current())
    }

    /// The connection as it stands, latched or not.
    fn current(&self) -> RuntimeClient {
        self.connection.borrow().clone()
    }

    /// Marks the connection for a rebuild when `error` is fatal, the failures
    /// the client latches a connection on.
    fn note(&self, error: &RuntimeClientError) {
        if error.is_fatal() {
            self.latched.store(true, Ordering::SeqCst);
        }
    }

    /// `note`, then the error as the card shows it.
    fn failure(&self, error: RuntimeClientError) -> ClientFailure {
        self.note(&error);
        ClientFailure::from(error)
    }
}

/// The session's own connection for ledger pages: actor `ui`, source `ui`,
/// channel `ui`, as the main connection, but at `LEDGER_PAGE_IO_TIMEOUT`, and
/// apart from it, so a slow page latches neither the follow polls nor the
/// material reads. Rebuilt after the client latched it, as the main connection
/// is (see `Rebuilding`).
struct LedgerPages {
    connection: Rebuilding,
}

impl LedgerPages {
    fn connect(root: &Path) -> Result<RuntimeClient, RuntimeClientError> {
        RuntimeClient::connect(
            RuntimeClientConfig::new(root, RuntimeChannel::Ui, EventActor::Ui, EventSource::Ui)
                .with_io_timeout(LEDGER_PAGE_IO_TIMEOUT),
        )
    }

    /// The page connection at open, to the Runtime of the session's owner
    /// epoch, the one the main connection reached. A failed connect fails the
    /// online face as its main connect does.
    fn open(root: &Path, owner_epoch: OwnerEpoch) -> Result<Self> {
        let client = Self::connect(root)
            .map_err(|error| anyhow!("在线读面不可用 / online source unavailable: {error}"))?;
        if client.runtime_info().owner_epoch() != owner_epoch {
            bail!("在线读面不可用 / online source unavailable: {CONSOLE_RUNTIME_OWNER_CHANGED}");
        }
        Ok(Self { connection: Rebuilding::new(Self::connect, client, owner_epoch) })
    }

    /// One page exchange, `call` on the page connection, rebuilt first when an
    /// earlier exchange latched it. A page query only reads; it writes nothing
    /// into the ledger, so asking it again is safe. But the query that timed
    /// out may still be running on the Runtime's ledger writer thread, so a
    /// timeout (see `page_timed_out`) is retried once only, after
    /// `LEDGER_PAGE_RETRY_BACKOFF`, on a rebuilt connection, and not at all
    /// when the rebuilt connection reaches a Runtime of another owner epoch:
    /// the pin was stated by the one that is gone. Any other failure, and the
    /// retry's, comes back as it is. Every failure reads `Runtime 查询失败：`
    /// and the client's error; a timeout whose rebuild failed carries the
    /// rebuild's reason after it.
    fn exchange<T>(
        &self,
        root: &Path,
        call: impl Fn(&RuntimeClient) -> Result<T, RuntimeClientError>,
    ) -> Result<T> {
        let client = self
            .connection
            .client(root)
            .map_err(|reason| anyhow!("Runtime 查询失败：{reason}"))?;
        let first = match call(&client) {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        drop(client);
        self.connection.note(&first);
        if !page_timed_out(&first) {
            return Err(anyhow!("Runtime 查询失败：{first}"));
        }
        std::thread::sleep(LEDGER_PAGE_RETRY_BACKOFF);
        if let Err(reason) = self.connection.reconnect(root) {
            return Err(anyhow!("Runtime 查询失败：{first}；{reason}"));
        }
        let client = self.connection.current();
        call(&client).map_err(|error| {
            self.connection.note(&error);
            anyhow!("Runtime 查询失败：{error}")
        })
    }
}

/// Whether a page exchange timed out waiting for its receipt: the client's
/// `runtime_receipt_header_failed` (or `runtime_receipt_timeout`, which a
/// receipt deadline gives) with the header read's io kind `TimedOut` or
/// `WouldBlock`, as Windows and Unix spell a socket read timeout. The code is
/// compared as its spelling (see `Spelling`).
fn page_timed_out(error: &RuntimeClientError) -> bool {
    matches!(error.code().spelling(), "runtime_receipt_header_failed" | "runtime_receipt_timeout")
        && error.receipt_header_io().is_some_and(|io| {
            matches!(io.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
        })
}

/// The client's own spelling of its connect operation (`ClientLocation::ConnectRuntime`).
const CONNECT_RUNTIME: &str = "connect_runtime";

/// The running Runtime's instances: one status read and one fact snapshot,
/// right after the session's page pin and again on a person's jump to the
/// latest; while following, each poll's fact snapshot refreshes the task facts
/// alone. Each read is at the ledger position the Runtime stated for it, which
/// may be past the pin: state as last read, not state at the pinned snapshot,
/// and the card says so. Neither read appends a ledger record (AO-4q).
#[derive(Debug, Clone)]
pub struct RuntimeInstances {
    /// The ledger's committed head when the Runtime sampled the status
    /// (`ledger_committed_head`; 0 for an empty ledger).
    pub status_sequence: u64,
    /// The ledger position the fact snapshot is sealed at.
    pub facts_position: u64,
    pub instances: Vec<RuntimeInstance>,
}

/// One instance as the status read registers it, with the `task.` facts the
/// snapshot holds for its id. A fact the snapshot holds for an id the status
/// does not register gets a row of its own with `status: None`; offline there
/// is no status read, and every row has `status: None`.
#[derive(Debug, Clone)]
pub struct RuntimeInstance {
    pub instance_id: String,
    pub status: Option<RuntimeInstanceLive>,
    pub game: Option<String>,
    pub server: Option<String>,
    pub page: Option<String>,
}

/// The instances' task facts as the offline face replays them at the pinned
/// position, or why it could not: the read face's own reason or error.
#[derive(Debug, Clone)]
pub enum OfflineFacts {
    /// The pinned position is 0: the ledger holds no event yet.
    NoEvents,
    Available { position: u64, instances: Vec<RuntimeInstance> },
    /// `reason` as the read face spells it (`ledger_empty`, `position_beyond_snapshot`).
    NotAvailable { position: u64, latest_sequence: u64, reason: String },
    /// The read face's failure as its outcome states it (ledger revision 3):
    /// the relayed code, and its `operation` value when the catalog lists that
    /// spelling (`None` otherwise).
    Failed {
        position: u64,
        code: String,
        operation: Option<String>,
        detail: String,
        /// The io error's kind, when the failure came from one.
        io_kind: Option<String>,
    },
}

/// What a session read about its instances: online, status and facts as last
/// read (see `RuntimeInstances`); offline, the facts at the pin.
pub enum InstanceFacts<'a> {
    Online(Ref<'a, Result<RuntimeInstances, ClientFailure>>),
    Offline(&'a OfflineFacts),
}

#[derive(Debug, Clone)]
pub struct RuntimeInstanceLive {
    pub alias: String,
    pub adb_port: Option<u16>,
    pub lease_active: bool,
    /// Never true together with `lease_active`, by contract.
    pub takeover_cooldown_active: bool,
    pub queued_request_count: u32,
}

impl OnlineSource {
    /// Discovery is the client's: it reads `runtime-info.json`, takes the
    /// loopback address from it and checks the owner epoch on connect. This is
    /// the console's own connection (actor `ui`, source `ui`), for what it asks
    /// on its own — material, the status and facts read at open, follow polls,
    /// probes; pages go through the session's page connection (`LedgerPages`).
    fn connect(root: &Path) -> Result<RuntimeClient, RuntimeClientError> {
        RuntimeClient::connect(RuntimeClientConfig::new(
            root,
            RuntimeChannel::Ui,
            EventActor::Ui,
            EventSource::Ui,
        ))
    }

    /// A connection for what a person does at the console — a button press, a
    /// shutdown request, a discovery query: actor `user`, source `ui`. The
    /// Runtime admits a shutdown request and an instance discovery only from
    /// this origin or an operator's CLI.
    ///
    /// Both console connections bind channel `ui` (Runtime
    /// contracts/runtime-interface.md, #389 U-2). Every press takes a fresh one
    /// and drops it after: a connection the client latched (R-0c: a receipt
    /// timeout, or any fatal failure) is never asked again, and nothing written
    /// is sent twice. The console keeps no write connection.
    fn connect_as_person(root: &Path) -> Result<RuntimeClient, RuntimeClientError> {
        RuntimeClient::connect(RuntimeClientConfig::new(
            root,
            RuntimeChannel::Ui,
            EventActor::User,
            EventSource::Ui,
        ))
    }

    /// The first page, asked without a snapshot on the session's page
    /// connection, is where the Runtime states the position this session then
    /// reads at.
    fn pin(root: PathBuf, client: RuntimeClient) -> Result<Self> {
        let owner_epoch = client.runtime_info().owner_epoch();
        let main = Rebuilding::new(Self::connect, client, owner_epoch);
        let pages = LedgerPages::open(&root, owner_epoch)?;
        let (snapshot, read_complete) = latest(&pages, &root)?;
        let instances = read_instances(&main, &root);
        Ok(Self {
            root,
            main,
            pages,
            snapshot: Cell::new(snapshot),
            read_complete: Cell::new(read_complete),
            instances: RefCell::new(instances),
        })
    }

    pub fn instances(&self) -> Ref<'_, Result<RuntimeInstances, ClientFailure>> {
        self.instances.borrow()
    }

    /// Moves the pin to the Runtime's latest position, as a fresh first page
    /// states it; `Ok(true)` when it moved. Pages read after this read at the
    /// new pin. A page query writes nothing into the ledger.
    pub fn repin(&self) -> Result<bool> {
        let (snapshot, read_complete) = latest(&self.pages, &self.root)?;
        let moved = snapshot != self.snapshot.get();
        self.snapshot.set(snapshot);
        self.read_complete.set(read_complete);
        Ok(moved)
    }

    /// One follow poll: a single fact snapshot. The Runtime keeps its fact
    /// store in memory and states it at the ledger's latest position, so this
    /// reads no ledger and writes nothing. When that position is past the pin,
    /// the pin moves to it (`Ok(true)`); the task facts are taken from the same
    /// snapshot, each instance keeping its status as last read — following
    /// reads no status; a person's jump to the latest does.
    pub fn poll(&self) -> Result<bool> {
        let client = self
            .main
            .client(&self.root)
            .map_err(|reason| anyhow!("Runtime 事实快照读取失败：{reason}"))?;
        let snapshot = client.runtime_fact_snapshot().map_err(|error| {
            self.main.note(&error);
            anyhow!("Runtime 事实快照读取失败：{error}")
        })?;
        let moved = snapshot.ledger_position > self.snapshot.get();
        if moved {
            self.snapshot.set(snapshot.ledger_position);
        }
        self.refold(&snapshot);
        Ok(moved)
    }

    /// The task facts of one snapshot over the instances as last read; a
    /// failed status read at open stays as it was.
    fn refold(&self, snapshot: &RuntimeFactSnapshot) {
        let refolded = match &*self.instances.borrow() {
            Err(_) => return,
            Ok(current) => {
                let mut instances: Vec<RuntimeInstance> = current
                    .instances
                    .iter()
                    .filter(|instance| instance.status.is_some())
                    .map(|instance| RuntimeInstance {
                        game: None,
                        server: None,
                        page: None,
                        ..instance.clone()
                    })
                    .collect();
                fold_task_facts(&snapshot.records, &mut instances);
                RuntimeInstances {
                    status_sequence: current.status_sequence,
                    facts_position: snapshot.ledger_position,
                    instances,
                }
            }
        };
        *self.instances.borrow_mut() = Ok(refolded);
    }

    /// Re-reads status and facts, as at open: for a person's jump to the latest.
    pub fn refresh_instances(&self) {
        let refreshed = read_instances(&self.main, &self.root);
        *self.instances.borrow_mut() = refreshed;
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot_position(&self) -> u64 {
        self.snapshot.get()
    }

    /// The same page request as offline, answered by the Runtime on the page
    /// connection.
    pub fn query(
        &self,
        query: &EventQuery,
        limit: u16,
        cursor: Option<RuntimeEventQueryCursor>,
    ) -> Result<RuntimeEventQueryPage> {
        let request = RuntimeEventQueryPageRequest::new(limit, cursor)
            .map_err(|error| anyhow!("页请求无效：{}", error.code()))?
            .at_snapshot(self.snapshot.get())
            .map_err(|error| anyhow!("快照位置无效：{}", error.code()))?;
        self.pages.exchange(&self.root, |client| {
            client.query_event_page(query.clone(), ProjectionProfile::Ui, request.clone())
        })
    }

    /// The medium, the corrupt tail, the writer record and the repair count are
    /// the offline face's observations of the files; the Runtime does not
    /// state them through its page or its runtime info. Online, the writer is
    /// the Runtime this session is connected to, as its own
    /// `runtime-info.json` describes it. The event count is the pinned position:
    /// the contract states sequences are gap-free from 1, so the count at a
    /// position is that position (`runtime-state-observation.md`).
    pub fn open_report(&self) -> OpenReport {
        let client = self.main.current();
        let info = client.runtime_info();
        OpenReport {
            backend: "runtime".to_string(),
            latest_sequence: self.snapshot.get(),
            event_count: LedgerCount::Counted(self.snapshot.get()),
            read_complete: self.read_complete.get(),
            corrupt_tail: None,
            repair_count: LedgerCount::NotStatedByRuntime,
            writer: WriterFacts::Runtime {
                pid: info.pid(),
                owner_epoch: code(&info.owner_epoch()),
                started_at_unix_ms: info.started_at_unix_ms(),
            },
        }
    }

    /// The material handle on the main connection, rebuilt first when an
    /// earlier exchange latched it; when that rebuild fails, a handle whose
    /// read says why.
    fn material_reader(&self) -> MaterialReader {
        match self.main.client(&self.root) {
            Ok(client) => {
                MaterialReader::Online { client, latched: Arc::clone(&self.main.latched) }
            }
            Err(reason) => MaterialReader::Unavailable(reason.to_string()),
        }
    }
}

/// The Runtime's latest position and whether its first page read completely:
/// one first page, asked without a snapshot on the page connection.
fn latest(pages: &LedgerPages, root: &Path) -> Result<(u64, bool)> {
    let request = RuntimeEventQueryPageRequest::new(1, None)
        .map_err(|error| anyhow!("页请求无效：{}", error.code()))?;
    let page = pages.exchange(root, |client| {
        client.query_event_page(
            EventQuery { view: Some(LedgerView::Events), ..EventQuery::default() },
            ProjectionProfile::Ui,
            request.clone(),
        )
    })?;
    Ok((page.snapshot_ledger_position(), page_source_complete(&page)))
}

/// Which read face `--source` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceMode {
    /// Online when the client can reach a Runtime through the state root,
    /// offline otherwise; the instance card says which and why.
    #[default]
    Auto,
    Offline,
    Online,
}

impl SourceMode {
    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "offline" => Some(Self::Offline),
            "online" => Some(Self::Online),
            _ => None,
        }
    }
}

/// Why a session reads offline. Stated on the instance card, so an `auto`
/// pick is never silent.
#[derive(Debug, Clone)]
pub enum OfflineReason {
    /// `--source offline`.
    Requested,
    /// `auto`: the client found no `runtime-info.json` in the state root.
    RuntimeInfoAbsent,
    /// `auto`: the state root names a Runtime, but the client could not
    /// connect to it. Carries the client's own error code and operation.
    ConnectFailed { code: &'static str, operation: &'static str },
}

/// The read face this session uses. Both faces answer the same query with the
/// same page, cursor and snapshot semantics; only where the answer comes from
/// differs.
pub enum ReadSource {
    Offline { source: Box<EvidenceSource>, reason: OfflineReason, facts: OfflineFacts },
    Online(OnlineSource),
}

/// What this session reads through: an open read face, or the offline face
/// the ledger refused to open. Unopened, nothing is read — no page, no port
/// map, no material — and the launcher still works: the Runtime it starts is
/// what creates the ledger.
pub enum Session {
    Open(ReadSource),
    Unopened { root: PathBuf, reason: OfflineReason, failure: LedgerOpenFailure },
}

impl Session {
    /// `Online` fails loud with the client's own error. `Auto` reads offline
    /// only when the client could not reach a Runtime, and keeps the reason;
    /// a Runtime that was reached but could not answer the first page is an
    /// error in every mode. An offline face the ledger refuses to open is not
    /// an error of the session: it is `Unopened`, with the ledger's own error.
    pub fn open(state_root: impl Into<PathBuf>, mode: SourceMode) -> Result<Self> {
        let root = state_root.into();
        let reason = match mode {
            SourceMode::Offline => OfflineReason::Requested,
            SourceMode::Auto | SourceMode::Online => match OnlineSource::connect(&root) {
                Ok(client) => {
                    return OnlineSource::pin(root, client)
                        .map(|source| Self::Open(ReadSource::Online(source)))
                }
                Err(error) if mode == SourceMode::Auto => match error.code().spelling() {
                    "runtime_info_unavailable" => OfflineReason::RuntimeInfoAbsent,
                    code => OfflineReason::ConnectFailed {
                        code,
                        operation: error.operation().spelling(),
                    },
                },
                Err(error) => bail!("在线读面不可用 / online source unavailable: {error}"),
            },
        };
        Ok(match EvidenceSource::open(&root) {
            Ok(source) => {
                let facts = source.instance_facts();
                Self::Open(ReadSource::Offline { source: Box::new(source), reason, facts })
            }
            Err(failure) => Self::Unopened { root, reason, failure },
        })
    }

    pub fn state_root(&self) -> &Path {
        match self {
            Self::Open(source) => source.state_root(),
            Self::Unopened { root, .. } => root,
        }
    }

    pub fn offline_reason(&self) -> Option<&OfflineReason> {
        match self {
            Self::Open(source) => source.offline_reason(),
            Self::Unopened { reason, .. } => Some(reason),
        }
    }
}

impl ReadSource {
    pub fn state_root(&self) -> &Path {
        match self {
            Self::Offline { source, .. } => source.state_root(),
            Self::Online(source) => source.state_root(),
        }
    }

    /// The committed position pages read at: offline, fixed for the whole
    /// session; online, the pin as last moved.
    pub fn snapshot_position(&self) -> u64 {
        match self {
            Self::Offline { source, .. } => source.snapshot_position(),
            Self::Online(source) => source.snapshot_position(),
        }
    }

    pub fn query(
        &self,
        query: &EventQuery,
        limit: u16,
        cursor: Option<RuntimeEventQueryCursor>,
    ) -> Result<RuntimeEventQueryPage> {
        match self {
            Self::Offline { source, .. } => source.query(query, limit, cursor),
            Self::Online(source) => source.query(query, limit, cursor),
        }
    }

    pub fn open_report(&self) -> OpenReport {
        match self {
            Self::Offline { source, .. } => source.open_report(),
            Self::Online(source) => source.open_report(),
        }
    }

    pub fn offline_reason(&self) -> Option<&OfflineReason> {
        match self {
            Self::Offline { reason, .. } => Some(reason),
            Self::Online(_) => None,
        }
    }

    /// The port map, offline only: the Runtime does not derive bindings
    /// through its page, so online is `Ok(None)`, not a read.
    pub fn instance_bindings(&self) -> Result<Option<PortBindings>> {
        match self {
            Self::Offline { source, .. } => source.instance_bindings().map(Some),
            Self::Online(_) => Ok(None),
        }
    }

    /// The instances' task facts: online with their status, as last read;
    /// offline replayed at the pinned position, once per session.
    pub fn instance_facts(&self) -> InstanceFacts<'_> {
        match self {
            Self::Offline { facts, .. } => InstanceFacts::Offline(facts),
            Self::Online(source) => InstanceFacts::Online(source.instances()),
        }
    }

    /// Whether this face can follow the ledger: online only. Offline there
    /// is no running Runtime to write newer events.
    pub fn follows(&self) -> bool {
        matches!(self, Self::Online(_))
    }

    /// Online, moves the pin to the Runtime's latest position; offline,
    /// `Ok(false)`.
    pub fn repin(&self) -> Result<bool> {
        match self {
            Self::Offline { .. } => Ok(false),
            Self::Online(source) => source.repin(),
        }
    }

    /// Online, one follow poll (see `OnlineSource::poll`); offline, `Ok(false)`.
    pub fn poll(&self) -> Result<bool> {
        match self {
            Self::Offline { .. } => Ok(false),
            Self::Online(source) => source.poll(),
        }
    }

    /// Online, re-reads status and facts; offline, nothing.
    pub fn refresh_instances(&self) {
        if let Self::Online(source) = self {
            source.refresh_instances();
        }
    }

    /// A handle a background thread reads material through.
    pub fn material_reader(&self) -> MaterialReader {
        match self {
            Self::Offline { source, .. } => MaterialReader::Offline(source.root.clone()),
            Self::Online(source) => source.material_reader(),
        }
    }
}

/// The Runtime a state root names, as one fresh connect learned it: the pid
/// and owner epoch from its own `runtime-info.json`, after the client's
/// owner-epoch check on connect.
#[derive(Debug, Clone)]
pub struct RuntimeFacts {
    pub pid: u32,
    pub owner_epoch: String,
}

/// A client operation that did not succeed: the client's own error code and
/// operation, and, when the Runtime answered with a refusal, that refusal's
/// code verbatim, with the host failure behind it when the refusal names one.
#[derive(Debug, Clone)]
pub struct ClientFailure {
    pub code: &'static str,
    pub operation: &'static str,
    pub runtime_code: Option<String>,
    /// The host's own code and operation (`host_code`, `host_operation`).
    pub host: Option<(String, String)>,
}

impl From<RuntimeClientError> for ClientFailure {
    fn from(error: RuntimeClientError) -> Self {
        Self {
            code: error.code().spelling(),
            operation: error.operation().spelling(),
            runtime_code: error.projection().map(|projection| code(&projection.code)),
            host: error
                .host_failure()
                .map(|(host_code, operation)| (host_code.to_string(), operation.to_string())),
        }
    }
}

/// One connect, nothing else: the launcher's "is a Runtime running here"
/// question, answered by the client's own discovery and owner-epoch check.
pub fn probe_runtime(state_root: &Path) -> Result<RuntimeFacts, ClientFailure> {
    let client = OnlineSource::connect(state_root)?;
    let info = client.runtime_info();
    Ok(RuntimeFacts { pid: info.pid(), owner_epoch: code(&info.owner_epoch()) })
}

/// One status read, then one fact snapshot, on the session's main connection,
/// rebuilt first when an earlier exchange latched it. Neither appends a ledger
/// record (Runtime contracts/runtime-state-observation.md, AO-4q). Either
/// failing fails the pair with its own error; a status without its source is
/// the console's own `CONSOLE_STATUS_SOURCE_MISSING`, since the card would
/// otherwise state a position the Runtime did not give. Only the three `task.`
/// keys are taken.
fn read_instances(main: &Rebuilding, root: &Path) -> Result<RuntimeInstances, ClientFailure> {
    let client = main.client(root).map_err(RebuildFailed::into_failure)?;
    let status = client.status().map_err(|error| main.failure(error))?;
    let status_sequence = status
        .source()
        .ok_or(ClientFailure {
            code: CONSOLE_STATUS_SOURCE_MISSING,
            operation: "runtime_status",
            runtime_code: None,
            host: None,
        })?
        .ledger_committed_head;
    let snapshot = client.runtime_fact_snapshot().map_err(|error| main.failure(error))?;
    let mut instances: Vec<RuntimeInstance> = status
        .instances()
        .iter()
        .map(|instance| RuntimeInstance {
            instance_id: code(&instance.instance_id()),
            status: Some(RuntimeInstanceLive {
                alias: instance.instance_alias().to_string(),
                adb_port: instance.adb_port(),
                lease_active: instance.lease_active(),
                takeover_cooldown_active: instance.takeover_cooldown_active(),
                queued_request_count: instance.queued_request_count(),
            }),
            game: None,
            server: None,
            page: None,
        })
        .collect();
    fold_task_facts(&snapshot.records, &mut instances);
    Ok(RuntimeInstances { status_sequence, facts_position: snapshot.ledger_position, instances })
}

/// Folds a snapshot's instance-scoped `task.game` / `task.server` / `task.page`
/// records into `instances`, adding a row with `status: None` for an id not
/// there yet. The task keys are strings by contract; any other value is shown
/// as the wire states it, never dropped.
fn fold_task_facts(records: &[RuntimeFactRecord], instances: &mut Vec<RuntimeInstance>) {
    for record in records {
        let RuntimeFactScope::Instance { instance_id } = &record.scope else {
            continue;
        };
        if !matches!(record.key.as_str(), "task.game" | "task.server" | "task.page") {
            continue;
        }
        let id = code(instance_id);
        let index = match instances.iter().position(|instance| instance.instance_id == id) {
            Some(index) => index,
            None => {
                instances.push(RuntimeInstance {
                    instance_id: id,
                    status: None,
                    game: None,
                    server: None,
                    page: None,
                });
                instances.len() - 1
            }
        };
        let text = match &record.value {
            FactValue::String(text) => text.clone(),
            other => code(other),
        };
        let instance = &mut instances[index];
        match record.key.as_str() {
            "task.game" => instance.game = Some(text),
            "task.server" => instance.server = Some(text),
            _ => instance.page = Some(text),
        }
    }
}

/// One instance discovery query as the Runtime answered it: the provider's
/// version, the ledger's committed head when the Runtime sampled the answer
/// (`ledger_committed_head`), and every instance the provider reported, in
/// index order.
#[derive(Debug, Clone)]
pub struct Discovery {
    pub provider_version: String,
    pub sequence: u64,
    pub instances: Vec<DiscoveredInstance>,
}

/// One reported instance. `bound_alias` is the registered instance the
/// Runtime has bound to it, if any.
#[derive(Debug, Clone)]
pub struct DiscoveredInstance {
    pub index: u16,
    pub name: String,
    pub adb_host: Option<String>,
    pub adb_port: Option<u16>,
    pub running: bool,
    pub bound_alias: Option<String>,
    pub android_version: Option<String>,
}

/// Asks the running Runtime to re-run its provider's instance discovery: one
/// fresh person's connection (see `OnlineSource::connect_as_person`), one
/// `discover_instances()`. It binds nothing and touches no device; an answered
/// query appends no ledger record (AO-4q). A refusal carries the Runtime's
/// code and, when the provider's tool failed, the host failure.
pub fn discover_instances(state_root: &Path) -> Result<Discovery, ClientFailure> {
    let client = OnlineSource::connect_as_person(state_root)?;
    let discovery = client.discover_instances()?;
    let instances = discovery
        .instances()
        .iter()
        .map(|instance| DiscoveredInstance {
            index: instance.instance_index,
            name: instance.instance_name.clone(),
            adb_host: instance.adb_host.clone(),
            adb_port: instance.adb_port,
            running: instance.running,
            bound_alias: instance.bound_alias.clone(),
            android_version: instance.android_version.clone(),
        })
        .collect();
    Ok(Discovery {
        provider_version: discovery.provider_version().to_string(),
        sequence: discovery.source().ledger_committed_head,
        instances,
    })
}

/// What an accepted shutdown request came back with.
#[derive(Debug, Clone)]
pub struct ShutdownAccepted {
    /// The receipt's state, as the contract spells it.
    pub receipt_state: String,
    pub request_id: String,
    /// The ledger position of the console's own `client_action` record,
    /// committed before the shutdown request was sent.
    pub action_sequence: u64,
}

/// One press of request shutdown: the shutdown requests it sent, 0 when it
/// failed before the first, and how the last one was answered.
pub struct ShutdownOutcome {
    pub attempts: u32,
    pub result: Result<ShutdownAccepted, ClientFailure>,
}

/// Shutdown requests one press sends at most. The Runtime refuses one as
/// `runtime_busy` while another request still holds its lifecycle admission,
/// as one briefly does after its reply; each refusal is its own ledgered request.
/// It is also refused as busy while a lease is active or requests are queued,
/// which these retries will not outlast.
pub const SHUTDOWN_ATTEMPTS: u32 = 5;
const SHUTDOWN_BUSY_WAIT: Duration = Duration::from_secs(1);

/// Asks the running Runtime to shut down, through the typed client only: the
/// console's button press recorded once, first (see `record_press`), then
/// `request_shutdown` to the owner frozen at connect time, on that same
/// interaction, sent again a second later while it is refused as
/// `runtime_busy`, `retrying` told each new attempt's number first. Any other
/// refusal or error, or the last busy refusal, comes back as a
/// [`ClientFailure`] with the Runtime's code. Nothing here kills the Runtime or
/// waits for it to stop.
pub fn request_shutdown(state_root: &Path, mut retrying: impl FnMut(u32)) -> ShutdownOutcome {
    let (interaction, action_sequence) =
        match record_press(state_root, "request_shutdown") {
            Ok(recorded) => recorded,
            Err(failure) => return ShutdownOutcome { attempts: 0, result: Err(failure) },
        };
    let mut attempts = 1;
    let result = loop {
        let error = match interaction.request_shutdown() {
            Ok(receipt) => {
                break Ok(ShutdownAccepted {
                    receipt_state: code(&receipt.state()),
                    request_id: code(&receipt.request_id()),
                    action_sequence,
                })
            }
            Err(error) => error,
        };
        // A fatal failure latched this person's connection, and one whose
        // receipt did not come back leaves the request unconfirmed: neither is
        // sent again (R-0c). The press's last probe reads the Runtime on a fresh
        // connection.
        let fatal = error.is_fatal();
        let failure = ClientFailure::from(error);
        let busy = failure.runtime_code.as_deref() == Some("runtime_busy");
        if fatal || !busy || attempts == SHUTDOWN_ATTEMPTS {
            break Err(failure);
        }
        attempts += 1;
        retrying(attempts);
        std::thread::sleep(SHUTDOWN_BUSY_WAIT);
    };
    ShutdownOutcome { attempts, result }
}

/// Records the launcher's start press, once the Runtime it started, or found
/// already running, takes a connection. Before that there is no Runtime to
/// record it, so a start that never got ready records nothing. Whether a
/// process was launched is an outcome the ledger states on its own, not a
/// value of the press; a press that found the Runtime running is its own control.
pub fn record_start(state_root: &Path, spawned: bool) -> Result<u64, ClientFailure> {
    let control_id = if spawned { "launcher.start" } else { "launcher.start.skipped_running" };
    let (_, sequence) = record_press(state_root, control_id)?;
    Ok(sequence)
}

/// One fresh person's connection, one interaction on it, and one launcher
/// press recorded there as a value-less button client action whose receipt
/// must carry a terminal event: a press is a person's act, so the record says
/// actor `user`, source `ui`. Gives back the interaction, on which a shutdown
/// request goes out as the same person, and the record's ledger position. A
/// record the contract refuses to build fails with the contract's own code; a
/// receipt without its terminal event is the console's own
/// `CONSOLE_CLIENT_ACTION_TERMINAL_MISSING`.
fn record_press(state_root: &Path, control_id: &str) -> Result<(RuntimeClient, u64), ClientFailure> {
    let client = OnlineSource::connect_as_person(state_root)?;
    let interaction = client.begin_interaction()?;
    let action =
        ClientActionRecord::new("acui.launcher", control_id, ClientActionKind::Button, None, None)
            .map_err(|error| ClientFailure {
                code: error.code().spelling(),
                operation: "record_client_action",
                runtime_code: None,
                host: None,
            })?;
    let recorded = interaction.record_client_action_receipt(action)?;
    let sequence = recorded
        .terminal()
        .ok_or(ClientFailure {
            code: CONSOLE_CLIENT_ACTION_TERMINAL_MISSING,
            operation: "record_client_action",
            runtime_code: None,
            host: None,
        })?
        .sequence;
    Ok((interaction, sequence))
}

/// Where a material read goes. Offline it is the forensic read face over the
/// state root; online it is the Runtime's own typed material read, on the
/// session's main connection, not the page connection.
#[derive(Clone)]
pub enum MaterialReader {
    Offline(PathBuf),
    /// The main connection as last rebuilt, with its latch: a read that fails
    /// fatally sets it, so the session's next exchange on the main connection
    /// connects anew (see `Rebuilding`).
    Online { client: RuntimeClient, latched: Arc<AtomicBool> },
    /// The main connection was latched and its rebuild failed: the read fails
    /// with why.
    Unavailable(String),
}

impl MaterialReader {
    /// See [`read_material`]; online, the client's own complete read assembles
    /// the ranges, each verified by the Runtime against the whole committed
    /// material, with the offline result semantics.
    pub fn read(
        &self,
        event: LedgerEventPosition,
        artifact: &ProjectedArtifactReference,
        snapshot_position: u64,
        still_wanted: &dyn Fn() -> bool,
    ) -> Result<Option<MaterialOutcome>> {
        match self {
            Self::Offline(root) => {
                read_material(root, event, artifact, snapshot_position, still_wanted)
            }
            Self::Online { client, latched } => Ok(read_material_online(
                client,
                latched,
                event,
                artifact,
                snapshot_position,
                still_wanted,
            )),
            Self::Unavailable(reason) => {
                if !still_wanted() {
                    return Ok(None);
                }
                Err(anyhow!("Runtime 材料读取失败：{reason}"))
            }
        }
    }
}

/// What one material read produced. `bytes` is present only for a fully
/// verified material; everything else is the read face's own typed outcome.
pub struct MaterialOutcome {
    pub bytes: Option<Vec<u8>>,
    pub state: RuntimeMaterialReadState,
    pub limit: Option<RuntimeMaterialReadLimit>,
    pub eviction: Option<ArtifactEvictionObservation>,
    pub failure: Option<String>,
}

/// Reads one committed material whole: the read face resolves and guards the
/// reference, re-checks it and its retention, and hands bytes back only once
/// the whole object is hash-verified; no prefix is ever exposed.
///
/// `still_wanted` is asked before the read and after it. The read itself
/// cannot be stopped midway, so a read superseded while it ran gives back
/// `None` and its result is dropped.
pub fn read_material(
    state_root: &Path,
    event: LedgerEventPosition,
    artifact: &ProjectedArtifactReference,
    snapshot_position: u64,
    still_wanted: &dyn Fn() -> bool,
) -> Result<Option<MaterialOutcome>> {
    if !still_wanted() {
        return Ok(None);
    }
    let selection = LedgerArtifactSelection {
        event,
        artifact_id: artifact.artifact_id,
        snapshot_position,
        sha256: artifact.sha256.clone(),
        byte_count: artifact.byte_count,
        run_id: artifact.run_id,
        frame_id: artifact.frame_id,
        request_id: None,
        correlation_id: artifact.correlation_id,
    };
    let result = actingcommand_ledger_forensics::read_material_complete(
        state_root,
        selection,
        MAX_FRAME_BYTES as usize,
        Instant::now() + MATERIAL_READ_DEADLINE,
    );
    if !still_wanted() {
        return Ok(None);
    }
    Ok(Some(match result {
        ForensicMaterialCompleteResult::Verified { bytes, .. } => MaterialOutcome {
            bytes: Some(bytes),
            state: RuntimeMaterialReadState::Verified,
            limit: None,
            eviction: None,
            failure: None,
        },
        // A retention limit carries no failure record; any other limit does,
        // or at least the read face's own error.
        ForensicMaterialCompleteResult::NotProvided { source, limit, failure, error } => {
            MaterialOutcome {
                bytes: None,
                state: RuntimeMaterialReadState::NotProvided,
                limit: Some(limit),
                eviction: source.and_then(|source| source.eviction),
                failure: failure
                    .map(|failure| failure.code)
                    .or_else(|| error.map(|error| error.code().to_string())),
            }
        }
        ForensicMaterialCompleteResult::Failed { source, state, failure, .. } => MaterialOutcome {
            bytes: None,
            state,
            limit: None,
            eviction: source.and_then(|source| source.eviction),
            failure: Some(failure.code),
        },
    }))
}

/// The online counterpart of [`read_material`]: `RuntimeClient::read_material_complete`
/// on the session's main connection, under the same byte cap and deadline. The
/// client asks `still_wanted` between ranges and drops an unfinished assembly
/// itself; a read stopped that way is superseded, and nothing is shown. A client error is kept whole (code, operation,
/// the Runtime's refusal and host failure), not reduced to its code.
fn read_material_online(
    client: &RuntimeClient,
    latched: &AtomicBool,
    event: LedgerEventPosition,
    artifact: &ProjectedArtifactReference,
    snapshot_position: u64,
    still_wanted: &dyn Fn() -> bool,
) -> Option<MaterialOutcome> {
    if !still_wanted() {
        return None;
    }
    let selection = RuntimeMaterialSelection {
        event,
        artifact_id: artifact.artifact_id,
        snapshot_position,
        sha256: artifact.sha256.clone(),
        byte_count: artifact.byte_count,
        run_id: artifact.run_id,
        frame_id: artifact.frame_id,
        request_id: None,
        correlation_id: artifact.correlation_id,
    };
    let result = client.read_material_complete(
        selection,
        MAX_FRAME_BYTES as usize,
        Instant::now() + MATERIAL_READ_DEADLINE,
        still_wanted,
    );
    // A fatal client error latched the main connection this clone shares; the
    // session's next exchange on it connects anew (see `Rebuilding`). Marked
    // even for a read no longer wanted.
    let fatal = match &result {
        RuntimeMaterialCompleteResult::NotProvided { error: Some(error), .. }
        | RuntimeMaterialCompleteResult::Failed { error, .. } => error.is_fatal(),
        _ => false,
    };
    if fatal {
        latched.store(true, Ordering::SeqCst);
    }
    if !still_wanted() {
        return None;
    }
    Some(match result {
        RuntimeMaterialCompleteResult::Verified { bytes, .. } => MaterialOutcome {
            bytes: Some(bytes),
            state: RuntimeMaterialReadState::Verified,
            limit: None,
            eviction: None,
            failure: None,
        },
        RuntimeMaterialCompleteResult::NotProvided { source, limit, failure, error } => {
            MaterialOutcome {
                bytes: None,
                state: RuntimeMaterialReadState::NotProvided,
                limit: Some(limit),
                eviction: source.and_then(|source| source.eviction),
                failure: match (failure, error) {
                    (Some(failure), Some(error)) => Some(format!("{} · {error}", failure.code)),
                    (Some(failure), None) => Some(failure.code),
                    (None, error) => error.map(|error| error.to_string()),
                },
            }
        }
        RuntimeMaterialCompleteResult::Failed { source, state, failure, error } => {
            MaterialOutcome {
                bytes: None,
                state,
                limit: None,
                eviction: source.and_then(|source| source.eviction),
                failure: Some(match failure {
                    Some(failure) => format!("{} · {error}", failure.code),
                    None => error.to_string(),
                }),
            }
        }
    })
}
