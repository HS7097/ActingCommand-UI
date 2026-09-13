// SPDX-License-Identifier: AGPL-3.0-only
//! Pure view model: tabs, filters, time cursor, selection. No toolkit here.

mod overlay;
mod tabs;

pub use overlay::{extract_frame_size, extract_overlays, Overlay};
pub use tabs::{ViewTab, ALL_TABS};

use acui_rows::{ArtifactRef, EventRow, OpenReport, Severity};
use chrono::{Local, TimeZone, Utc};

#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub min_severity: Option<Severity>,
    pub module: Option<String>,
    pub id_text: Option<String>,
    /// Time cursor: only events at or before this sequence are shown.
    pub through_sequence: Option<u64>,
}

impl Filters {
    fn accepts(&self, row: &EventRow) -> bool {
        if let Some(min) = self.min_severity {
            if row.severity < min {
                return false;
            }
        }
        if let Some(module) = &self.module {
            if &row.origin.module != module {
                return false;
            }
        }
        if let Some(cursor) = self.through_sequence {
            if row.sequence > cursor {
                return false;
            }
        }
        if let Some(text) = &self.id_text {
            if !text.is_empty() && !(row.event_id.contains(text) || row.links.contains_text(text)) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
pub struct InstanceCard {
    pub source_label: String,
    /// Rows actually loaded from the given page files.
    pub loaded_count: usize,
    /// `event_count` as stated by open.json; `None` without `--open`.
    pub ledger_event_count: Option<u64>,
    /// `read_complete` / `corrupt_tail` / `repair_count` from open.json, rendered.
    pub ledger_integrity: Option<String>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub severity_counts: Vec<(Severity, usize)>,
    pub modules: Vec<String>,
    pub writer_owner_id: Option<String>,
    pub writer_pid: Option<u64>,
    pub writer_active: Option<bool>,
    pub latest_sequence: Option<u64>,
    pub last_event_age_text: String,
}

pub struct DetailView<'a> {
    pub row: &'a EventRow,
    pub pretty_payload_json: String,
    pub artifacts: &'a [ArtifactRef],
    pub overlays: Vec<Overlay>,
    /// Frame size in device pixels when the payload states it.
    pub frame_size: Option<(f32, f32)>,
}

pub struct ViewModel {
    pub rows: Vec<EventRow>,
    pub tab: ViewTab,
    pub filters: Filters,
    pub selected_sequence: Option<u64>,
    open: Option<OpenReport>,
    source_label: String,
}

impl ViewModel {
    pub fn new(rows: Vec<EventRow>, open: Option<OpenReport>, source_label: String) -> Self {
        Self {
            rows,
            tab: ViewTab::EventStream,
            filters: Filters::default(),
            selected_sequence: None,
            open,
            source_label,
        }
    }

    pub fn visible(&self) -> Vec<&EventRow> {
        self.rows
            .iter()
            .filter(|row| self.tab.membership(row) && self.filters.accepts(row))
            .collect()
    }

    pub fn modules(&self) -> Vec<String> {
        let mut modules: Vec<String> = self
            .rows
            .iter()
            .map(|row| row.origin.module.clone())
            .collect();
        modules.sort();
        modules.dedup();
        modules
    }

    pub fn max_sequence(&self) -> u64 {
        self.rows.last().map(|row| row.sequence).unwrap_or(0)
    }

    pub fn detail(&self) -> Option<DetailView<'_>> {
        let sequence = self.selected_sequence?;
        let row = self
            .rows
            .iter()
            .find(|row| row.sequence == sequence)
            .filter(|row| self.tab.membership(row) && self.filters.accepts(row))?;
        Some(DetailView {
            row,
            pretty_payload_json: serde_json::to_string_pretty(&row.payload)
                .unwrap_or_else(|_| row.payload.to_string()),
            artifacts: &row.artifacts,
            overlays: extract_overlays(&row.payload),
            frame_size: extract_frame_size(&row.payload),
        })
    }

    pub fn instance_card(&self) -> InstanceCard {
        let mut severity_counts: Vec<(Severity, usize)> = Vec::new();
        for severity in [
            Severity::Debug,
            Severity::Info,
            Severity::Warning,
            Severity::Error,
            Severity::Fatal,
        ] {
            let count = self
                .rows
                .iter()
                .filter(|row| row.severity == severity)
                .count();
            if count > 0 {
                severity_counts.push((severity, count));
            }
        }
        let writer = self.open.as_ref().and_then(|open| open.writer.as_ref());
        let last = self.rows.last().map(|row| row.timestamp_unix_ms);
        InstanceCard {
            source_label: self.source_label.clone(),
            loaded_count: self.rows.len(),
            ledger_event_count: self.open.as_ref().and_then(|open| open.event_count),
            ledger_integrity: self.open.as_ref().map(integrity_text),
            first_timestamp: self.rows.first().map(|row| format_full(row.timestamp_unix_ms)),
            last_timestamp: last.map(format_full),
            severity_counts,
            modules: self.modules(),
            writer_owner_id: writer.and_then(|writer| writer.owner_id.clone()),
            writer_pid: writer.and_then(|writer| writer.pid),
            writer_active: writer.and_then(|writer| writer.active),
            latest_sequence: self.open.as_ref().and_then(|open| open.latest_sequence),
            last_event_age_text: last.map(age_text).unwrap_or_else(|| "—".to_string()),
        }
    }
}

/// Ledger integrity as stated by open.json: 正常, or the raw values when not.
fn integrity_text(open: &OpenReport) -> String {
    let corrupt = open
        .corrupt_tail
        .as_ref()
        .filter(|value| !value.is_null());
    if open.read_complete == Some(true) && corrupt.is_none() && open.repair_count.unwrap_or(0) == 0 {
        return "正常".to_string();
    }
    format!(
        "read_complete={} / corrupt_tail={} / repair_count={}",
        open.read_complete
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
        corrupt
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        open.repair_count
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
    )
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

fn local(timestamp_unix_ms: u64) -> Option<chrono::DateTime<Local>> {
    Utc.timestamp_millis_opt(timestamp_unix_ms as i64)
        .single()
        .map(|time| time.with_timezone(&Local))
}

fn age_text(timestamp_unix_ms: u64) -> String {
    let now = Utc::now().timestamp_millis();
    let delta = now - timestamp_unix_ms as i64;
    if delta < 0 {
        return "来自未来".to_string();
    }
    let seconds = delta / 1000;
    if seconds < 60 {
        format!("{seconds} 秒前")
    } else if seconds < 3600 {
        format!("{} 分钟前", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} 小时 {} 分前", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{} 天 {} 小时前", seconds / 86_400, (seconds % 86_400) / 3600)
    }
}
