// SPDX-License-Identifier: GPL-3.0-only
//! The read face, and the only place that touches a state root.
//!
//! The console never joins a path into the state root, never opens `ledger/`,
//! `artifacts/` or `runtime-state.sqlite`, and never spawns a CLI. Everything
//! below goes through the Runtime's own offline entries: `GlobalLedger` picks
//! the medium and authenticates the snapshot, `query_view_page` projects the
//! formal page, and `read_material_to` resolves, guards and verifies material.

use std::path::{Path, PathBuf};

use acui_rows::{
    ArtifactEvictionObservation, EventQuery, LedgerEventPosition, MAX_RUNTIME_MATERIAL_CHUNK_BYTES,
    MAX_RUNTIME_MATERIAL_REPLY_BYTES, OpenReport, ProjectedArtifactReference, ProjectionProfile,
    RuntimeEventQueryCursor, RuntimeEventQueryPage, RuntimeEventQueryPageRequest,
    RuntimeMaterialReadLimit, RuntimeMaterialReadRequest, RuntimeMaterialReadResult,
    RuntimeMaterialReadState, WriterFacts,
};
use actingcommand_ledger::{
    GlobalLedger, GlobalLedgerEvidenceConfig, GlobalLedgerMetadata,
    GlobalLedgerWriterMetadataObservation,
};
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
        let result = read_range(state_root, request)?;
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
