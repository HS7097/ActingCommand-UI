// SPDX-License-Identifier: GPL-3.0-only
//! The read face, and the only place that touches a state root.
//!
//! The console never joins a path into the state root, never opens `ledger/`,
//! `artifacts/` or `runtime-state.sqlite`, and never spawns a CLI. Everything
//! below goes through the Runtime's own entries. Offline, `GlobalLedger` picks
//! the medium and authenticates the snapshot, `query_view_page` projects the
//! formal page, and `read_material_complete` resolves, guards and verifies material.
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

use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use acui_rows::{
    code, ArtifactEvictionObservation, ClientActionKind, ClientActionRecord, EventActor,
    EventQuery, EventSource, LedgerCount, LedgerEventPosition, LedgerView, OpenReport,
    PortBindings, PortEntry, ProjectedArtifactReference, ProjectionProfile,
    RuntimeEventQueryCursor, RuntimeEventQueryPage, RuntimeEventQueryPageRequest,
    RuntimeMaterialReadLimit, RuntimeMaterialReadState, WriterFacts,
};
use actingcommand_contract::{FactValue, RuntimeFactRecord, RuntimeFactScope, RuntimeFactSnapshot};
use actingcommand_ledger::{
    GlobalLedger, GlobalLedgerError, GlobalLedgerEvidenceConfig, GlobalLedgerMetadata,
    GlobalLedgerWriterMetadataObservation, LedgerArtifactSelection,
};
use actingcommand_ledger_forensics::{ForensicMaterialCompleteResult, ForensicRuntimeFactsResult};
use actingcommand_runtime_client::{
    RuntimeClient, RuntimeClientConfig, RuntimeClientError, RuntimeMaterialCompleteResult,
    RuntimeMaterialSelection,
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

/// One authenticated snapshot of one state root.
pub struct EvidenceSource {
    root: PathBuf,
    snapshot: GlobalLedgerMetadata,
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
        let snapshot = GlobalLedger::open_metadata(GlobalLedgerEvidenceConfig::new(&root))?;
        Ok(Self { root, snapshot })
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }

    /// The committed position this whole session reads at.
    pub fn snapshot_position(&self) -> u64 {
        self.snapshot.latest_sequence()
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
            &self.snapshot,
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
            &self.snapshot,
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
                    anyhow!("instance_binding_malformed: 端口 {port} 没有绑定事实")
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
            ForensicRuntimeFactsResult::Failed { position, code, operation, detail, io_kind } => {
                let io_kind = io_kind.map(|kind| acui_rows::code(&kind));
                OfflineFacts::Failed { position, code, operation, detail, io_kind }
            }
        }
    }

    pub fn open_report(&self) -> OpenReport {
        let event_count = self.snapshot.event_count() as u64;
        let complete = self.snapshot.read_complete() && self.snapshot.corrupt_tail().is_none();
        OpenReport {
            backend: self.snapshot.backend().to_string(),
            latest_sequence: self.snapshot.latest_sequence(),
            event_count: if complete {
                LedgerCount::Counted(event_count)
            } else {
                LedgerCount::VerifiedPrefix(event_count)
            },
            read_complete: self.snapshot.read_complete(),
            corrupt_tail: self.snapshot.corrupt_tail().map(|tail| {
                format!(
                    "{} 段{} 偏移{} 悬挂{}字节",
                    tail.code, tail.segment_index, tail.byte_offset, tail.dangling_byte_count
                )
            }),
            repair_count: match self.snapshot.repair_count() {
                Some(count) => LedgerCount::Counted(count as u64),
                None => LedgerCount::NoRepairLog,
            },
            writer: match self.snapshot.writer_metadata() {
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
    client: RuntimeClient,
    /// The pin every page reads at; moved only by `repin`, on a person's jump
    /// to the latest or while the console follows it.
    snapshot: Cell<u64>,
    read_complete: Cell<bool>,
    instances: RefCell<Result<RuntimeInstances, ClientFailure>>,
}

/// The running Runtime's instances, read once right after the session's page
/// pin and never refreshed: one status read and one fact snapshot, each at the
/// ledger position the Runtime stated for it, which may be past the pin. State
/// at open, not state at the pinned snapshot, and the card says so. By contract
/// the Runtime records the status read as one observation event
/// (`command.validated`), after the pin and so outside this session's snapshot.
#[derive(Debug, Clone)]
pub struct RuntimeInstances {
    /// The sequence of the status read's own committed observation.
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
    Failed {
        position: u64,
        code: &'static str,
        operation: &'static str,
        detail: String,
        /// The io error's kind, when the failure came from one.
        io_kind: Option<String>,
    },
}

/// What a session read about its instances, once: online, status and facts
/// right after the pin; offline, the facts at the pin.
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
    /// on its own — pages, material, the status and facts read at open, probes.
    fn connect(root: &Path) -> Result<RuntimeClient, RuntimeClientError> {
        RuntimeClient::connect(RuntimeClientConfig::new(root, EventActor::Ui, EventSource::Ui))
    }

    /// A connection for what a person does at the console — a button press, a
    /// shutdown request, a discovery query: actor `user`, source `ui`. The
    /// Runtime admits a shutdown request and an instance discovery only from
    /// this origin or an operator's CLI.
    fn connect_as_person(root: &Path) -> Result<RuntimeClient, RuntimeClientError> {
        RuntimeClient::connect(RuntimeClientConfig::new(root, EventActor::User, EventSource::Ui))
    }

    /// The first page, asked without a snapshot, is where the Runtime states
    /// the position this session then reads at.
    fn pin(root: PathBuf, client: RuntimeClient) -> Result<Self> {
        let (snapshot, read_complete) = latest(&client)?;
        let instances = read_instances(&client);
        Ok(Self {
            root,
            client,
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
        let (snapshot, read_complete) = latest(&self.client)?;
        let moved = snapshot != self.snapshot.get();
        self.snapshot.set(snapshot);
        self.read_complete.set(read_complete);
        Ok(moved)
    }

    /// One follow poll: a single fact snapshot. The Runtime keeps its fact
    /// store in memory and states it at the ledger's latest position, so this
    /// reads no ledger and writes nothing. When that position is past the pin,
    /// the pin moves to it (`Ok(true)`); the task facts are taken from the same
    /// snapshot, each instance keeping its status as last read — a status read
    /// is recorded in the ledger, and following never repeats it.
    pub fn poll(&self) -> Result<bool> {
        let snapshot = self
            .client
            .runtime_fact_snapshot()
            .map_err(|error| anyhow!("Runtime 事实快照读取失败：{error}"))?;
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

    /// Re-reads status and facts, as at open: for a person's jump to the
    /// latest, which may leave one status observation event in the ledger.
    pub fn refresh_instances(&self) {
        let refreshed = read_instances(&self.client);
        *self.instances.borrow_mut() = refreshed;
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot_position(&self) -> u64 {
        self.snapshot.get()
    }

    /// The same page request as offline, answered by the Runtime.
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
        self.client
            .query_event_page(query.clone(), ProjectionProfile::Ui, request)
            .map_err(|error| anyhow!("Runtime 查询失败：{error}"))
    }

    /// The medium, the corrupt tail, the writer record and the repair count are
    /// the offline face's observations of the files; the Runtime does not
    /// state them through its page or its runtime info. Online, the writer is
    /// the Runtime this session is connected to, as its own
    /// `runtime-info.json` describes it. The event count is the pinned position:
    /// the contract states sequences are gap-free from 1, so the count at a
    /// position is that position (`runtime-state-observation.md`).
    pub fn open_report(&self) -> OpenReport {
        let info = self.client.runtime_info();
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
}

/// The Runtime's latest position and whether its first page read completely:
/// one first page, asked without a snapshot.
fn latest(client: &RuntimeClient) -> Result<(u64, bool)> {
    let request = RuntimeEventQueryPageRequest::new(1, None)
        .map_err(|error| anyhow!("页请求无效：{}", error.code()))?;
    let page = client
        .query_event_page(
            EventQuery { view: Some(LedgerView::Events), ..EventQuery::default() },
            ProjectionProfile::Ui,
            request,
        )
        .map_err(|error| anyhow!("Runtime 查询失败：{error}"))?;
    Ok((
        page.snapshot_ledger_position(),
        page.read_scope().is_none_or(|scope| scope.read_complete),
    ))
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
                Err(error) if mode == SourceMode::Auto => match error.code() {
                    "runtime_info_unavailable" => OfflineReason::RuntimeInfoAbsent,
                    code => OfflineReason::ConnectFailed { code, operation: error.operation() },
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

    /// The committed position this whole session reads at.
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

    /// The instances' task facts, read once per session: online with their
    /// status right after the pin, offline replayed at the pinned position.
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
            Self::Online(source) => MaterialReader::Online(source.client.clone()),
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
            code: error.code(),
            operation: error.operation(),
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

/// One status read, then one fact snapshot, on the session's connection; the
/// status read is the one that leaves an observation event. Either failing
/// fails the pair with its own error; a status without its observation
/// source is `status_source_missing`, since the card would otherwise state a
/// position the Runtime did not give. Only the three `task.` keys are taken.
fn read_instances(client: &RuntimeClient) -> Result<RuntimeInstances, ClientFailure> {
    let status = client.status()?;
    let status_sequence = status
        .source()
        .ok_or(ClientFailure {
            code: "status_source_missing",
            operation: "runtime_status",
            runtime_code: None,
            host: None,
        })?
        .sequence;
    let snapshot = client.runtime_fact_snapshot()?;
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
/// version, the sequence of its committed observation, and every instance the
/// provider reported, in index order.
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
/// `discover_instances()`. It binds nothing and touches no device; by contract the
/// Runtime records an answered query as one observation event
/// (`command.validated`), and a refusal as `command.rejected` plus
/// `runtime.failed`. A refusal carries the Runtime's code and, when the
/// provider's tool failed, the host failure.
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
        sequence: discovery.source().sequence,
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
        let failure = match interaction.request_shutdown() {
            Ok(receipt) => {
                break Ok(ShutdownAccepted {
                    receipt_state: code(&receipt.state()),
                    request_id: code(&receipt.request_id()),
                    action_sequence,
                })
            }
            Err(error) => ClientFailure::from(error),
        };
        let busy = failure.runtime_code.as_deref() == Some("runtime_busy");
        if !busy || attempts == SHUTDOWN_ATTEMPTS {
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
/// request goes out as the same person, and the record's ledger position.
fn record_press(state_root: &Path, control_id: &str) -> Result<(RuntimeClient, u64), ClientFailure> {
    let client = OnlineSource::connect_as_person(state_root)?;
    let interaction = client.begin_interaction()?;
    let action =
        ClientActionRecord::new("acui.launcher", control_id, ClientActionKind::Button, None, None)
            .map_err(|_| ClientFailure {
                code: "client_action_invalid",
                operation: "record_client_action",
                runtime_code: None,
                host: None,
            })?;
    let recorded = interaction.record_client_action_receipt(action)?;
    let sequence = recorded
        .terminal()
        .ok_or(ClientFailure {
            code: "client_action_terminal_missing",
            operation: "record_client_action",
            runtime_code: None,
            host: None,
        })?
        .sequence;
    Ok((interaction, sequence))
}

/// Where a material read goes. Offline it is the forensic read face over the
/// state root; online it is the Runtime's own typed material read, on the same
/// connection the pages come from.
#[derive(Clone)]
pub enum MaterialReader {
    Offline(PathBuf),
    Online(RuntimeClient),
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
            Self::Online(client) => {
                Ok(read_material_online(client, event, artifact, snapshot_position, still_wanted))
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
/// on the session's connection, under the same byte cap and deadline. It checks
/// `still_wanted` before and after, not between ranges; the client drops an
/// unfinished assembly itself. A client error is kept whole (code, operation,
/// the Runtime's refusal and host failure), not reduced to its code.
fn read_material_online(
    client: &RuntimeClient,
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
    );
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
