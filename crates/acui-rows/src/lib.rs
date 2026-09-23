// SPDX-License-Identifier: GPL-3.0-only
//! The only crate that names the Runtime contract types for the view model.
//!
//! Nothing is mirrored here any more: the page, its events, the view membership
//! each event carries, the retention facts and the material read result are the
//! contract's own types, re-exported unchanged. What this crate adds is display
//! only — local time, wire codes as text, shortened ids, the display-name
//! dictionary — plus the two plain structs `acui-source` fills from the
//! ledger's own observations.

mod display;

pub use display::{event_type_names, format_bytes, module_names};

pub use actingcommand_contract::{
    ArtifactEvictionObservation, ArtifactKind, ClientActionKind, ClientActionRecord,
    ClientActionValue, EventActor, EventLinks, EventQuery, EventSeverity, EventSource, InstanceId,
    LedgerEventPosition, LedgerRecoveryGap, LedgerRecoveryState,
    LedgerRunRecovery, LedgerView, MAX_RUNTIME_EVENT_QUERY_EVENTS,
    MAX_RUNTIME_MATERIAL_CHUNK_BYTES, MAX_RUNTIME_MATERIAL_REPLY_BYTES, OriginModule,
    ProjectedArtifactReference, ProjectedEvent, ProjectionPayload, ProjectionProfile,
    PublicEventPayload, RuntimeEventQueryCursor, RuntimeEventQueryPage,
    RuntimeEventQueryPageRequest, RuntimeMaterialReadLimit, RuntimeMaterialReadRequest,
    RuntimeMaterialReadResult, RuntimeMaterialReadState, Sensitivity, TaskSemanticFact,
};

use std::collections::BTreeMap;

use chrono::{Local, TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;

/// What the ledger states about the opened source. Plain fields so both
/// `acui-source` (which fills it) and `acui-model` (which shows it) can name it.
#[derive(Debug, Clone)]
pub struct OpenReport {
    /// `segment` or `sqlite`, chosen by the ledger from the state root; `runtime`
    /// when the session reads a running Runtime, which does not state its medium.
    pub backend: String,
    pub latest_sequence: u64,
    pub event_count: LedgerCount,
    pub read_complete: bool,
    pub corrupt_tail: Option<String>,
    /// The repair log's own count; it does not share the event snapshot's boundary.
    pub repair_count: LedgerCount,
    pub writer: WriterFacts,
}

/// A ledger-wide count as the read face states it, or why it states none.
#[derive(Debug, Clone, Copy)]
pub enum LedgerCount {
    Counted(u64),
    /// Over an incomplete read: the verified prefix only.
    VerifiedPrefix(u64),
    /// The medium keeps no such record: SQLite has no repair log.
    NoRepairLog,
    /// Online: neither the page nor `runtime-info.json` states it.
    NotStatedByRuntime,
}

#[derive(Debug, Clone)]
pub enum WriterFacts {
    Absent,
    Locked { byte_count: u64 },
    Readable { owner_id: String, pid: u32, active: bool, started_at_unix_ms: u64 },
    /// The running Runtime this session is connected to, as its `runtime-info.json` states it.
    Runtime { pid: u32, owner_epoch: String, started_at_unix_ms: u64 },
}

/// Instance identity by ADB port, as the offline read face derived it from
/// every `runtime.instance_bound` fact through the session's snapshot. Filled
/// by `acui-source` once per session, read by `acui-model` and the console.
#[derive(Debug, Clone)]
pub struct PortBindings {
    /// One entry per port, ascending.
    pub ports: Vec<PortEntry>,
    /// Instances outside the port map: serial-configured or never given an
    /// ADB port. Binding facts exist for them, so they are not "no records".
    pub unported: Vec<InstanceId>,
    /// The port an instance id's latest binding names as HOST:PORT, for the
    /// rows' port column. An id whose latest binding is serial-configured or
    /// has no port is absent here even if it belongs to a port's set.
    pub port_of: BTreeMap<InstanceId, u16>,
}

/// One ADB port: every instance id ever bound to it — one instance, always
/// queried together as the whole set — and the facts of its latest binding.
#[derive(Debug, Clone)]
pub struct PortEntry {
    pub port: u16,
    /// The whole set, in first-binding order.
    pub members: Vec<InstanceId>,
    /// The member whose binding has the highest sequence; `latest_*` are its.
    pub latest_instance_id: InstanceId,
    pub latest_alias: String,
    /// The provenance's wire value: `physical_device` or `fixture_simulation`.
    pub latest_provenance: String,
    pub latest_sequence: u64,
}

/// A schema-owned code as its wire text, e.g. `capture.completed`, `run_18cf…`.
pub fn code(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(text)) => text,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// `run_18cfd721…0013`: keeps the type prefix and both ends of the hex.
pub fn short_id(id: &str) -> String {
    match id.split_once('_') {
        Some((prefix, hex)) if hex.len() > 14 => {
            format!("{prefix}_{}…{}", &hex[..8], &hex[hex.len() - 4..])
        }
        _ => id.to_string(),
    }
}

/// Every id this event links to, in a stable display order.
pub fn links_named(links: &EventLinks) -> Vec<(&'static str, String)> {
    let mut named = Vec::new();
    let mut push = |name, value: Option<String>| {
        if let Some(value) = value {
            named.push((name, value));
        }
    };
    push("request_id", links.request_id().map(code));
    push("correlation_id", links.correlation_id().map(code));
    push("causation_id", links.causation_id().map(code));
    push("action_id", links.action_id().map(code));
    push("run_id", links.run_id().map(code));
    push("task_id", links.task_id().map(code));
    push("instance_id", links.instance_id().map(code));
    push("recognition_id", links.recognition_id().map(code));
    push("frame_id", links.frame_id().map(code));
    push("lease_id", links.lease_id().map(code));
    named
}

/// The projected payload as JSON, for the read-only payload pane and for the
/// geometry the console draws over a frame.
pub fn payload_value(event: &ProjectedEvent) -> Value {
    serde_json::to_value(&event.payload).unwrap_or(Value::Null)
}

/// `hh:mm:ss.mmm` in local time, for list rows.
pub fn format_clock(timestamp_unix_ms: u64) -> String {
    match local(timestamp_unix_ms) {
        Some(time) => time.format("%H:%M:%S%.3f").to_string(),
        None => "—".to_string(),
    }
}

/// Full local timestamp, for the instance card and the detail header.
pub fn format_full(timestamp_unix_ms: u64) -> String {
    match local(timestamp_unix_ms) {
        Some(time) => time.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        None => "—".to_string(),
    }
}

/// Whole seconds from a committed timestamp to now; negative if it is ahead.
pub fn seconds_since(timestamp_unix_ms: u64) -> i64 {
    Local::now().timestamp_millis().saturating_sub(timestamp_unix_ms as i64) / 1000
}

fn local(timestamp_unix_ms: u64) -> Option<chrono::DateTime<Local>> {
    Utc.timestamp_millis_opt(timestamp_unix_ms as i64)
        .single()
        .map(|time| time.with_timezone(&Local))
}
