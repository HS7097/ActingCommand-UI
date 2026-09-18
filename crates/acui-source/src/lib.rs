// SPDX-License-Identifier: GPL-3.0-only
//! The read face, and the only place that touches a state root.
//!
//! The console never joins a path into the state root, never opens `ledger/`,
//! `artifacts/` or `runtime-state.sqlite`, and never spawns a CLI. Everything
//! below goes through the Runtime's own entries. Offline, `GlobalLedger` picks
//! the medium and authenticates the snapshot, `query_view_page` projects the
//! formal page, and `read_material_to` resolves, guards and verifies material.
//! Online, the same page and material operations are asked of the running
//! Runtime through `actingcommand_runtime_client`, the one typed IPC path a
//! client has. This crate is a client only: it never starts, kills or waits
//! on the Runtime, and never writes into the state root. The launcher's two
//! questions — is a Runtime running here, and will it accept a shutdown
//! request — are asked through that same typed client, at the end of this file.

use std::path::{Path, PathBuf};

use acui_rows::{
    code, ArtifactEvictionObservation, ClientActionKind, ClientActionRecord, EventActor,
    EventQuery, EventSource, LedgerEventPosition, LedgerView, MAX_RUNTIME_MATERIAL_CHUNK_BYTES,
    MAX_RUNTIME_MATERIAL_REPLY_BYTES, OpenReport,
    ProjectedArtifactReference, ProjectionProfile, RuntimeEventQueryCursor, RuntimeEventQueryPage,
    RuntimeEventQueryPageRequest, RuntimeMaterialReadLimit, RuntimeMaterialReadRequest,
    RuntimeMaterialReadResult, RuntimeMaterialReadState, WriterFacts,
};
use actingcommand_ledger::{
    GlobalLedger, GlobalLedgerEvidenceConfig, GlobalLedgerMetadata,
    GlobalLedgerWriterMetadataObservation,
};
use actingcommand_runtime_client::{RuntimeClient, RuntimeClientConfig, RuntimeClientError};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

/// Ranges one frame may cost. The contract bounds a single range
/// (`MAX_RUNTIME_MATERIAL_CHUNK_BYTES`) and a single reply; the range budget is
/// what bounds the whole material the console is willing to assemble.
pub const MAX_FRAME_RANGES: u64 = 128;
pub const MAX_FRAME_BYTES: u64 = MAX_RUNTIME_MATERIAL_CHUNK_BYTES as u64 * MAX_FRAME_RANGES;

/// One authenticated snapshot of one state root.
pub struct EvidenceSource {
    root: PathBuf,
    snapshot: GlobalLedgerMetadata,
}

impl EvidenceSource {
    pub fn open(state_root: impl Into<PathBuf>) -> Result<Self> {
        let root = state_root.into();
        let snapshot = GlobalLedger::open_metadata(GlobalLedgerEvidenceConfig::new(&root))
            .with_context(|| format!("打开状态根 {} 失败", root.display()))?;
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

    pub fn open_report(&self) -> OpenReport {
        OpenReport {
            backend: self.snapshot.backend().to_string(),
            latest_sequence: self.snapshot.latest_sequence(),
            // Only `GlobalLedger::open_evidence` states these, and it drops every
            // event whose material the caller cannot verify, so asking for them
            // would mean hashing every artifact in the root at startup.
            event_count: None,
            read_complete: self.snapshot.read_complete(),
            corrupt_tail: self.snapshot.corrupt_tail().map(|tail| {
                format!(
                    "{} 段{} 偏移{} 悬挂{}字节",
                    tail.code, tail.segment_index, tail.byte_offset, tail.dangling_byte_count
                )
            }),
            repair_count: None,
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
    snapshot: u64,
    read_complete: bool,
}

impl OnlineSource {
    /// Discovery is the client's: it reads `runtime-info.json`, takes the
    /// loopback address from it and checks the owner epoch on connect.
    fn connect(root: &Path) -> Result<RuntimeClient, RuntimeClientError> {
        RuntimeClient::connect(RuntimeClientConfig::new(root, EventActor::Ui, EventSource::Ui))
    }

    /// The first page, asked without a snapshot, is where the Runtime states
    /// the position this session then reads at.
    fn pin(root: PathBuf, client: RuntimeClient) -> Result<Self> {
        let request = RuntimeEventQueryPageRequest::new(1, None)
            .map_err(|error| anyhow!("页请求无效：{}", error.code()))?;
        let page = client
            .query_event_page(
                EventQuery { view: Some(LedgerView::Events), ..EventQuery::default() },
                ProjectionProfile::Ui,
                request,
            )
            .map_err(|error| anyhow!("Runtime 查询失败：{error}"))?;
        Ok(Self {
            root,
            client,
            snapshot: page.snapshot_ledger_position(),
            read_complete: page.read_scope().is_none_or(|scope| scope.read_complete),
        })
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot_position(&self) -> u64 {
        self.snapshot
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
            .at_snapshot(self.snapshot)
            .map_err(|error| anyhow!("快照位置无效：{}", error.code()))?;
        self.client
            .query_event_page(query.clone(), ProjectionProfile::Ui, request)
            .map_err(|error| anyhow!("Runtime 查询失败：{error}"))
    }

    /// The medium, the corrupt tail and the writer record are the offline
    /// face's observations of the files; the Runtime does not state them
    /// through its page. Online, the writer is the Runtime this session is
    /// connected to, as its own `runtime-info.json` describes it.
    pub fn open_report(&self) -> OpenReport {
        let info = self.client.runtime_info();
        OpenReport {
            backend: "runtime".to_string(),
            latest_sequence: self.snapshot,
            event_count: None,
            read_complete: self.read_complete,
            corrupt_tail: None,
            repair_count: None,
            writer: WriterFacts::Runtime {
                pid: info.pid(),
                owner_epoch: code(&info.owner_epoch()),
                started_at_unix_ms: info.started_at_unix_ms(),
            },
        }
    }
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
    Offline { source: EvidenceSource, reason: OfflineReason },
    Online(OnlineSource),
}

impl ReadSource {
    /// `Online` fails loud with the client's own error. `Auto` reads offline
    /// only when the client could not reach a Runtime, and keeps the reason;
    /// a Runtime that was reached but could not answer the first page is an
    /// error in every mode.
    pub fn open(state_root: impl Into<PathBuf>, mode: SourceMode) -> Result<Self> {
        let root = state_root.into();
        let reason = match mode {
            SourceMode::Offline => OfflineReason::Requested,
            SourceMode::Auto | SourceMode::Online => match OnlineSource::connect(&root) {
                Ok(client) => return OnlineSource::pin(root, client).map(Self::Online),
                Err(error) if mode == SourceMode::Auto => match error.code() {
                    "runtime_info_unavailable" => OfflineReason::RuntimeInfoAbsent,
                    code => OfflineReason::ConnectFailed { code, operation: error.operation() },
                },
                Err(error) => bail!("在线读面不可用 / online source unavailable: {error}"),
            },
        };
        Ok(Self::Offline { source: EvidenceSource::open(root)?, reason })
    }

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
/// code verbatim.
#[derive(Debug, Clone)]
pub struct ClientFailure {
    pub code: &'static str,
    pub operation: &'static str,
    pub runtime_code: Option<String>,
}

impl From<RuntimeClientError> for ClientFailure {
    fn from(error: RuntimeClientError) -> Self {
        Self {
            code: error.code(),
            operation: error.operation(),
            runtime_code: error.projection().map(|projection| code(&projection.code)),
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

/// Asks the running Runtime to shut down, through the typed client only:
/// one fresh connection, one interaction on it, the console's button press
/// recorded as a client action first (its receipt must carry a terminal
/// event), then `request_shutdown` to the owner frozen at connect time. A
/// refusal at either step comes back as a [`ClientFailure`] with the
/// Runtime's code; nothing here retries, kills or waits.
pub fn request_shutdown(state_root: &Path) -> Result<ShutdownAccepted, ClientFailure> {
    let client = OnlineSource::connect(state_root)?;
    let interaction = client.begin_interaction()?;
    let action = ClientActionRecord::new(
        "acui.launcher",
        "request_shutdown",
        ClientActionKind::Button,
        None,
        None,
    )
    .map_err(|_| ClientFailure {
        code: "client_action_invalid",
        operation: "record_client_action",
        runtime_code: None,
    })?;
    let recorded = interaction.record_client_action_receipt(action)?;
    let action_sequence = recorded
        .terminal()
        .ok_or(ClientFailure {
            code: "client_action_terminal_missing",
            operation: "record_client_action",
            runtime_code: None,
        })?
        .sequence;
    let receipt = interaction.request_shutdown()?;
    Ok(ShutdownAccepted {
        receipt_state: code(&receipt.state()),
        request_id: code(&receipt.request_id()),
        action_sequence,
    })
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
    /// See [`read_material`]; the online face assembles the same ranges, each
    /// verified by the Runtime against the whole committed material.
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
            Self::Online(client) => assemble(
                |request| {
                    client
                        .read_material(request)
                        .map_err(|error| anyhow!("Runtime 素材读取失败：{error}"))
                },
                event,
                artifact,
                snapshot_position,
                still_wanted,
            ),
        }
    }
}

/// What one material read produced. `bytes` is present only for a fully
/// verified assembly; everything else is the read face's own typed outcome.
pub struct MaterialOutcome {
    pub bytes: Option<Vec<u8>>,
    pub state: RuntimeMaterialReadState,
    pub limit: Option<RuntimeMaterialReadLimit>,
    pub eviction: Option<ArtifactEvictionObservation>,
    pub failure: Option<String>,
}

/// Assembles one committed material from verified ranges. Every range is
/// resolved, guarded and hash-verified against the whole material by the read
/// face before its bytes are handed back; a range that is not `verified` ends
/// the assembly with that range's own outcome.
///
/// `still_wanted` is asked before every range. A read whose answer has stopped
/// mattering stops issuing ranges and gives back `None`: each range re-hashes
/// the whole material, so a superseded read is expensive to let run on.
pub fn read_material(
    state_root: &Path,
    event: LedgerEventPosition,
    artifact: &ProjectedArtifactReference,
    snapshot_position: u64,
    still_wanted: &dyn Fn() -> bool,
) -> Result<Option<MaterialOutcome>> {
    assemble(
        |request| read_range(state_root, request),
        event,
        artifact,
        snapshot_position,
        still_wanted,
    )
}

/// The range loop both faces share; `read_range` is the one face-specific step.
fn assemble(
    read_range: impl Fn(RuntimeMaterialReadRequest) -> Result<RuntimeMaterialReadResult>,
    event: LedgerEventPosition,
    artifact: &ProjectedArtifactReference,
    snapshot_position: u64,
    still_wanted: &dyn Fn() -> bool,
) -> Result<Option<MaterialOutcome>> {
    if artifact.byte_count > MAX_FRAME_BYTES {
        bail!(
            "素材 {} 字节超出监控台上限 {} 字节（{} 段 × {} 字节）",
            artifact.byte_count,
            MAX_FRAME_BYTES,
            MAX_FRAME_RANGES,
            MAX_RUNTIME_MATERIAL_CHUNK_BYTES
        );
    }
    let mut bytes = Vec::with_capacity(artifact.byte_count as usize);
    while (bytes.len() as u64) < artifact.byte_count {
        if !still_wanted() {
            return Ok(None);
        }
        let offset = bytes.len() as u64;
        let remaining = artifact.byte_count - offset;
        let request = RuntimeMaterialReadRequest {
            event,
            artifact_id: artifact.artifact_id,
            snapshot_position,
            byte_count: artifact.byte_count,
            sha256: artifact.sha256.clone(),
            expected_run_id: artifact.run_id,
            expected_frame_id: artifact.frame_id,
            expected_request_id: None,
            expected_correlation_id: artifact.correlation_id,
            offset,
            requested_length: remaining.min(MAX_RUNTIME_MATERIAL_CHUNK_BYTES as u64) as u32,
            max_reply_bytes: MAX_RUNTIME_MATERIAL_REPLY_BYTES,
        };
        let result = read_range(request)?;
        match result.chunk {
            Some(chunk) if result.state == RuntimeMaterialReadState::Verified => {
                bytes.extend_from_slice(&chunk.bytes)
            }
            _ => {
                return Ok(Some(MaterialOutcome {
                    bytes: None,
                    state: result.state,
                    limit: result.limit,
                    eviction: result.source.and_then(|source| source.eviction),
                    failure: result.failure.map(|failure| failure.code),
                }))
            }
        }
    }
    Ok(Some(MaterialOutcome {
        bytes: Some(bytes),
        state: RuntimeMaterialReadState::Verified,
        limit: None,
        eviction: None,
        failure: None,
    }))
}

/// `read_material_to` writes its structured result and then reports a non-verified
/// outcome as an error; the typed result is the answer, so it is parsed either way.
fn read_range(
    state_root: &Path,
    request: RuntimeMaterialReadRequest,
) -> Result<RuntimeMaterialReadResult> {
    let request = actingcommand_ledger_forensics::ForensicMaterialRequest::new(state_root, request)
        .map_err(|error| anyhow!("素材请求无效：{error}"))?;
    let mut body = Vec::new();
    let reported = actingcommand_ledger_forensics::read_material_to(request, &mut body);
    match serde_json::from_slice::<MaterialEnvelope>(&body) {
        Ok(envelope) => Ok(envelope.data),
        Err(_) => Err(match reported {
            Err(error) => anyhow!("素材读取失败：{error}"),
            Ok(()) => anyhow!("素材读取未给出可解析的结果"),
        }),
    }
}

#[derive(Deserialize)]
struct MaterialEnvelope {
    #[allow(dead_code)]
    command: String,
    data: RuntimeMaterialReadResult,
}
