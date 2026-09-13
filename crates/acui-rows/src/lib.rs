// SPDX-License-Identifier: AGPL-3.0-only
//! Row types mirroring the exported `actingcommand.event.v2` projection.
//!
//! These are read-only mirrors, not the contract itself: unknown fields are
//! ignored so a newer Runtime projection still loads.

use serde::Deserialize;
use serde_json::{Map, Value};

/// Ordered severity, matching `EventSeverity` in the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Origin {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub module: String,
    #[serde(default)]
    pub actor: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Links {
    pub instance_id: Option<String>,
    pub request_id: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub lease_id: Option<String>,
    pub frame_id: Option<String>,
    pub action_id: Option<String>,
    pub recognition_id: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Links {
    /// Named ids present on this event, in a stable display order.
    pub fn named(&self) -> Vec<(&'static str, &str)> {
        let known = [
            ("task_id", &self.task_id),
            ("run_id", &self.run_id),
            ("request_id", &self.request_id),
            ("correlation_id", &self.correlation_id),
            ("causation_id", &self.causation_id),
            ("action_id", &self.action_id),
            ("recognition_id", &self.recognition_id),
            ("frame_id", &self.frame_id),
            ("lease_id", &self.lease_id),
            ("instance_id", &self.instance_id),
        ];
        known
            .into_iter()
            .filter_map(|(name, value)| value.as_deref().map(|value| (name, value)))
            .collect()
    }

    pub fn first(&self) -> Option<&str> {
        self.named().first().map(|(_, value)| *value)
    }

    pub fn contains_text(&self, needle: &str) -> bool {
        self.named().iter().any(|(_, value)| value.contains(needle))
            || self
                .other
                .values()
                .filter_map(Value::as_str)
                .any(|value| value.contains(needle))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub kind: String,
    pub media_type: String,
    pub byte_count: u64,
    pub sha256: String,
    pub object_key: Option<String>,
    pub frame_id: Option<String>,
    pub run_id: Option<String>,
    pub correlation_id: Option<String>,
    pub created_at_unix_ms: Option<u64>,
    pub producer: Option<String>,
    pub retention_class: Option<String>,
    pub redaction_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventRow {
    #[serde(default)]
    pub schema_version: String,
    pub sequence: u64,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub timestamp_unix_ms: u64,
    #[serde(default)]
    pub event_type: String,
    pub severity: Severity,
    #[serde(default)]
    pub sensitivity: String,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub links: Links,
    #[serde(default)]
    pub payload_schema: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

/// One page of the offline reader's `events` output.
#[derive(Debug, Clone, Deserialize)]
pub struct Page {
    pub after_sequence: Option<u64>,
    pub through_sequence: Option<u64>,
    pub limit: Option<u64>,
    #[serde(default)]
    pub events: Vec<EventRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageEnvelope {
    pub command: String,
    pub data: Page,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Writer {
    pub owner_id: Option<String>,
    pub pid: Option<u64>,
    pub started_at_unix_ms: Option<u64>,
    pub active: Option<bool>,
}

/// The offline reader's `open` output.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenReport {
    pub latest_sequence: Option<u64>,
    pub event_count: Option<u64>,
    pub storage_backend: Option<String>,
    #[serde(default)]
    pub writer: Option<Writer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenEnvelope {
    pub command: String,
    pub data: OpenReport,
}
