// SPDX-License-Identifier: AGPL-3.0-only
//! Pure view model over the formal page: tabs, filters, paging, recovery
//! folding, selection. No toolkit here, and no classification of its own.

mod overlay;
mod tabs;

pub use overlay::{extract_frame_size, extract_overlays, Overlay};
pub use tabs::{tab_from_name, tab_label, tab_name, ALL_TABS};

use acui_rows::{
    code, format_full, links_named, payload_value, ArtifactEvictionObservation, ArtifactKind,
    EventQuery, EventSeverity, LedgerEventPosition, LedgerRecoveryState, LedgerRunRecovery,
    LedgerView, OpenReport, OriginModule, ProjectedArtifactReference, ProjectedEvent,
    RuntimeEventQueryCursor, RuntimeEventQueryPage,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Filter state, turned into one `EventQuery` and re-run against the ledger.
/// Nothing here filters rows the console already holds.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub minimum_severity: Option<EventSeverity>,
    pub maximum_severity: Option<EventSeverity>,
    pub origin_module: Option<OriginModule>,
    /// One canonical `correlation_` / `request_` / `run_` / `task_` id.
    pub id_text: String,
    pub from_timestamp_unix_ms: Option<u64>,
    pub to_timestamp_unix_ms: Option<u64>,
}

impl Filters {
    pub fn query(&self, view: LedgerView) -> Result<EventQuery, String> {
        let mut query = EventQuery {
            view: Some(view),
            minimum_severity: self.minimum_severity,
            maximum_severity: self.maximum_severity,
            origin_module: self.origin_module,
            from_timestamp_unix_ms: self.from_timestamp_unix_ms,
            to_timestamp_unix_ms: self.to_timestamp_unix_ms,
            ..EventQuery::default()
        };
        let text = self.id_text.trim();
        if !text.is_empty() {
            let assigned = if text.starts_with("correlation_") {
                parse_id(text).map(|id| query.correlation_id = Some(id))
            } else if text.starts_with("request_") {
                parse_id(text).map(|id| query.request_id = Some(id))
            } else if text.starts_with("run_") {
                parse_id(text).map(|id| query.run_id = Some(id))
            } else if text.starts_with("task_") {
                parse_id(text).map(|id| query.task_id = Some(id))
            } else {
                None
            };
            if assigned.is_none() {
                return Err("id 需为完整的 correlation_ / request_ / run_ / task_ 标识".to_string());
            }
        }
        query
            .validate()
            .map_err(|error| format!("过滤条件无效：{}", error.code()))?;
        Ok(query)
    }
}

fn parse_id<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(Value::String(text.to_string())).ok()
}

/// A run whose failures the ledger itself resolved at this snapshot.
#[derive(Debug, Clone)]
pub struct RecoveryGroup {
    pub run_id: String,
    pub state: LedgerRecoveryState,
    /// The ledger-given basis: which failure was answered by which success.
    pub basis: String,
    pub gaps: String,
    pub folded: Vec<u64>,
}

/// One line of the middle list: either a run's recovery header, or an event.
pub enum DisplayRow<'a> {
    Recovery(&'a RecoveryGroup),
    Event { event: &'a ProjectedEvent, folded: bool },
}

#[derive(Debug, Clone)]
pub struct InstanceCard {
    pub source_label: String,
    pub backend: String,
    pub snapshot_position: u64,
    pub latest_sequence: u64,
    pub event_count: Option<u64>,
    pub integrity: String,
    pub writer: String,
    pub loaded_count: usize,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub severity_counts: Vec<(EventSeverity, usize)>,
    pub modules: Vec<OriginModule>,
}

pub struct DetailView<'a> {
    pub event: &'a ProjectedEvent,
    pub pretty_payload_json: String,
    pub lines: Vec<(String, String)>,
    pub overlays: Vec<Overlay>,
    pub frame_size: Option<(f32, f32)>,
}

/// What the frame pane should read, and what the ledger already says about it.
pub struct FrameTarget {
    pub event: LedgerEventPosition,
    pub artifact: ProjectedArtifactReference,
    pub eviction: Option<ArtifactEvictionObservation>,
}

pub struct ViewModel {
    pub tab: LedgerView,
    pub filters: Filters,
    pub selected_sequence: Option<u64>,
    pub filter_error: Option<String>,
    rows: Vec<ProjectedEvent>,
    recovery: Vec<RecoveryGroup>,
    next_cursor: Option<RuntimeEventQueryCursor>,
    scope_text: String,
    source_complete: bool,
    snapshot_position: u64,
    open: OpenReport,
    source_label: String,
    span: Option<(u64, u64)>,
}

impl ViewModel {
    pub fn new(
        open: OpenReport,
        snapshot_position: u64,
        source_label: String,
        span: Option<(u64, u64)>,
    ) -> Self {
        Self {
            tab: LedgerView::Events,
            filters: Filters::default(),
            selected_sequence: None,
            filter_error: None,
            rows: Vec::new(),
            recovery: Vec::new(),
            next_cursor: None,
            scope_text: String::new(),
            source_complete: true,
            snapshot_position,
            open,
            source_label,
            span,
        }
    }

    /// The whole committed time span, for the time-range cursor.
    pub fn span(&self) -> Option<(u64, u64)> {
        self.span
    }

    pub fn query(&self) -> Result<EventQuery, String> {
        self.filters.query(self.tab)
    }

    pub fn cursor(&self) -> Option<RuntimeEventQueryCursor> {
        self.next_cursor.clone()
    }

    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }

    /// Replaces the rows, or appends the next page of the same query.
    pub fn apply(&mut self, page: RuntimeEventQueryPage, append: bool) {
        if !append {
            self.rows.clear();
            self.recovery.clear();
            self.selected_sequence = None;
        }
        self.rows.extend(page.events().iter().cloned());
        self.next_cursor = page.next_cursor().cloned();
        self.source_complete = page.read_scope().is_none_or(|scope| scope.read_complete);
        self.scope_text = match page.read_scope() {
            Some(scope) => format!(
                "已读到 #{}{}",
                scope.scanned_through_position,
                if scope.read_complete { "" } else { " · 源不完整" }
            ),
            None => "读取范围未给".to_string(),
        };
        for group in page.run_recovery() {
            let run_id = code(&group.run_id);
            self.recovery.retain(|existing| existing.run_id != run_id);
            self.recovery.push(recovery_group(run_id, group));
        }
    }

    pub fn scope_text(&self) -> &str {
        &self.scope_text
    }

    pub fn source_complete(&self) -> bool {
        self.source_complete
    }

    /// How many loaded rows each view claims, from the membership the page carries.
    pub fn membership_counts(&self) -> [usize; 6] {
        let mut counts = [0_usize; 6];
        for event in &self.rows {
            for (index, view) in ALL_TABS.into_iter().enumerate() {
                if event.views.contains(&view) {
                    counts[index] += 1;
                }
            }
        }
        counts
    }

    pub fn modules(&self) -> Vec<OriginModule> {
        let mut modules: Vec<OriginModule> =
            self.rows.iter().map(|event| event.origin.module()).collect();
        modules.sort();
        modules.dedup();
        modules
    }

    /// Rows in ledger order. Each run the page carries a recovery statement for
    /// gets a header at its first row, and every failure the ledger resolved is
    /// folded under it.
    pub fn display_rows(&self) -> Vec<DisplayRow<'_>> {
        let mut header_shown = vec![false; self.recovery.len()];
        let mut emitted: Vec<u64> = Vec::new();
        let mut display = Vec::new();
        for event in &self.rows {
            if emitted.contains(&event.sequence) {
                continue;
            }
            let run = event.links.run_id().map(code);
            let anchor = self.recovery.iter().enumerate().position(|(index, group)| {
                !header_shown[index]
                    && (group.folded.contains(&event.sequence)
                        || run.as_deref() == Some(group.run_id.as_str()))
            });
            if let Some(index) = anchor {
                header_shown[index] = true;
                let group = &self.recovery[index];
                display.push(DisplayRow::Recovery(group));
                for sequence in &group.folded {
                    if let Some(member) = self.rows.iter().find(|row| row.sequence == *sequence) {
                        emitted.push(*sequence);
                        display.push(DisplayRow::Event { event: member, folded: true });
                    }
                }
            }
            if !emitted.contains(&event.sequence) {
                display.push(DisplayRow::Event { event, folded: false });
            }
        }
        display
    }

    pub fn selected(&self) -> Option<&ProjectedEvent> {
        let sequence = self.selected_sequence?;
        self.rows.iter().find(|event| event.sequence == sequence)
    }

    pub fn detail(&self) -> Option<DetailView<'_>> {
        let event = self.selected()?;
        let payload = payload_value(event);
        let mut lines = vec![
            ("sequence".to_string(), event.sequence.to_string()),
            ("event_id".to_string(), code(&event.event_id)),
            ("时间".to_string(), format_full(event.timestamp_unix_ms)),
            ("event_type".to_string(), code(&event.event_type)),
            ("severity".to_string(), event.severity.as_str().to_string()),
            ("sensitivity".to_string(), code(&event.sensitivity)),
            (
                "origin".to_string(),
                format!(
                    "{} / {} / {}",
                    code(&event.origin.source()),
                    event.origin.module(),
                    code(&event.origin.actor())
                ),
            ),
            ("所属视图".to_string(), views_text(event)),
            ("payload_schema".to_string(), event.payload_schema.clone()),
        ];
        lines.extend(
            links_named(&event.links)
                .into_iter()
                .map(|(name, value)| (name.to_string(), value)),
        );
        Some(DetailView {
            event,
            pretty_payload_json: serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| payload.to_string()),
            lines,
            overlays: extract_overlays(&payload),
            frame_size: extract_frame_size(&payload),
        })
    }

    pub fn frame_target(&self) -> Option<FrameTarget> {
        let event = self.selected()?;
        let artifact = event
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == ArtifactKind::CaptureFrame)?;
        Some(FrameTarget {
            event: LedgerEventPosition { event_id: event.event_id, sequence: event.sequence },
            eviction: event
                .artifact_evictions
                .iter()
                .find(|eviction| eviction.artifact_id == artifact.artifact_id)
                .cloned(),
            artifact: artifact.clone(),
        })
    }

    pub fn instance_card(&self) -> InstanceCard {
        let mut severity_counts = Vec::new();
        for severity in [
            EventSeverity::Debug,
            EventSeverity::Info,
            EventSeverity::Warning,
            EventSeverity::Error,
            EventSeverity::Fatal,
        ] {
            let count = self
                .rows
                .iter()
                .filter(|event| event.severity == severity)
                .count();
            if count > 0 {
                severity_counts.push((severity, count));
            }
        }
        InstanceCard {
            source_label: self.source_label.clone(),
            backend: self.open.backend.clone(),
            snapshot_position: self.snapshot_position,
            latest_sequence: self.open.latest_sequence,
            event_count: self.open.event_count,
            integrity: integrity_text(&self.open),
            writer: writer_text(&self.open),
            loaded_count: self.rows.len(),
            first_timestamp: self
                .rows
                .first()
                .map(|event| format_full(event.timestamp_unix_ms)),
            last_timestamp: self
                .rows
                .last()
                .map(|event| format_full(event.timestamp_unix_ms)),
            severity_counts,
            modules: self.modules(),
        }
    }
}

fn recovery_group(run_id: String, group: &LedgerRunRecovery) -> RecoveryGroup {
    let basis = group
        .evidence
        .iter()
        .map(|item| match &item.success {
            Some(success) => format!("失败 #{} → 成功 #{}", item.failure.sequence, success.sequence),
            None => format!("失败 #{} 未见成功", item.failure.sequence),
        })
        .collect::<Vec<_>>()
        .join("；");
    RecoveryGroup {
        run_id,
        state: group.state,
        basis,
        gaps: group
            .gaps
            .iter()
            .map(code)
            .collect::<Vec<_>>()
            .join("、"),
        folded: group
            .evidence
            .iter()
            .filter(|item| item.success.is_some())
            .map(|item| item.failure.sequence)
            .collect(),
    }
}

pub const fn recovery_state_text(state: LedgerRecoveryState) -> &'static str {
    match state {
        LedgerRecoveryState::Recovered => "已恢复",
        LedgerRecoveryState::Unresolved => "未解决",
        LedgerRecoveryState::Unknown => "未知",
    }
}

fn views_text(event: &ProjectedEvent) -> String {
    event
        .views
        .iter()
        .map(|view| tab_label(*view))
        .collect::<Vec<_>>()
        .join("、")
}

fn integrity_text(open: &OpenReport) -> String {
    match (&open.corrupt_tail, open.read_complete) {
        (None, true) => match open.repair_count {
            Some(0) | None => "正常".to_string(),
            Some(count) => format!("正常 · 修复记录 {count}"),
        },
        (tail, complete) => format!(
            "read_complete={complete} / corrupt_tail={}",
            tail.as_deref().unwrap_or("无")
        ),
    }
}

fn writer_text(open: &OpenReport) -> String {
    match &open.writer {
        acui_rows::WriterFacts::Absent => "无写入方记录".to_string(),
        acui_rows::WriterFacts::Locked { byte_count } => format!("被占用（{byte_count} 字节）"),
        acui_rows::WriterFacts::Readable { owner_id, pid, active, started_at_unix_ms } => {
            format!(
                "{owner_id} · pid {pid} · {} · 起 {}",
                if *active { "在线" } else { "离线" },
                format_full(*started_at_unix_ms)
            )
        }
    }
}
