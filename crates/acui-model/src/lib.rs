// SPDX-License-Identifier: GPL-3.0-only
//! Pure view model over the formal page: tabs, filters, paging, recovery
//! folding, selection. No toolkit here, no classification of its own, and no
//! human-facing wording — every name a person reads is chosen in `acui-app`,
//! which is the only crate that holds the two language tables.

mod overlay;
mod tabs;

pub use overlay::{extract_frame_size, extract_overlays, Overlay};
pub use tabs::{tab_from_name, tab_name, ALL_TABS};

use acui_rows::{
    code, payload_value, ArtifactEvictionObservation, ArtifactKind, EventQuery, EventSeverity,
    InstanceId, LedgerEventPosition, LedgerRecoveryGap, LedgerRecoveryState, LedgerRunRecovery,
    LedgerView, OpenReport, OriginModule, PortBindings, PortEntry, ProjectedArtifactReference,
    ProjectedEvent, RuntimeEventQueryCursor, RuntimeEventQueryPage, WriterFacts,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Why this view has no page: the filters as written, or the read itself.
#[derive(Debug, Clone)]
pub enum QueryError {
    /// The id box holds something that is not a whole canonical id.
    IdNotCanonical,
    /// A port is picked and an `instance_` id is typed: the ledger takes one
    /// instance filter, and the console gives neither precedence.
    InstanceFilterConflict,
    /// The ledger refused the assembled query; carries the ledger's own code.
    Rejected(String),
    /// The read face could not answer; carries what it reported.
    ReadFailed(String),
}

/// Filter state, turned into one `EventQuery` and re-run against the ledger.
/// Nothing here filters rows the console already holds.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub minimum_severity: Option<EventSeverity>,
    pub maximum_severity: Option<EventSeverity>,
    pub origin_module: Option<OriginModule>,
    /// One canonical `correlation_` / `request_` / `run_` / `task_` /
    /// `instance_` id.
    pub id_text: String,
    /// Every instance id ever bound to the picked port: one instance, queried
    /// as the whole set or not at all.
    pub instance_ids: Vec<InstanceId>,
    /// The port that set stands for; display only, the query carries the set.
    pub port: Option<u16>,
    pub from_timestamp_unix_ms: Option<u64>,
    pub to_timestamp_unix_ms: Option<u64>,
}

impl Filters {
    pub fn query(&self, view: LedgerView) -> Result<EventQuery, QueryError> {
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
            } else if text.starts_with("instance_") {
                parse_id(text).map(|id| query.instance_id = Some(id))
            } else {
                None
            };
            if assigned.is_none() {
                return Err(QueryError::IdNotCanonical);
            }
        }
        if query.instance_id.is_some() && !self.instance_ids.is_empty() {
            return Err(QueryError::InstanceFilterConflict);
        }
        query.instance_ids = self.instance_ids.clone();
        query
            .validate()
            .map_err(|error| QueryError::Rejected(error.code().to_string()))?;
        Ok(query)
    }
}

fn parse_id<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(Value::String(text.to_string())).ok()
}

/// A run whose failures the ledger itself resolved at this snapshot. Merged
/// across pages by run id: a later page adds evidence, it never takes any away.
#[derive(Debug, Clone)]
pub struct RecoveryGroup {
    pub run_id: String,
    pub state: LedgerRecoveryState,
    /// `(failure position, the success the ledger says answered it)`.
    pub evidence: Vec<(u64, Option<u64>)>,
    pub gaps: Vec<LedgerRecoveryGap>,
}

impl RecoveryGroup {
    /// The failures this group folds: every failure the ledger answered.
    pub fn folded(&self) -> Vec<u64> {
        self.evidence
            .iter()
            .filter_map(|(failure, success)| success.map(|_| *failure))
            .collect()
    }

    /// Takes in what a later page states about the same run. Positions already
    /// held are kept, and a folded row is never unfolded: a failure that this
    /// group already answered keeps its success.
    fn absorb(&mut self, group: &LedgerRunRecovery) {
        // How resolved a state is; a page that states less never pulls the
        // group back, so a run folded by an earlier page stays folded.
        const fn rank(state: LedgerRecoveryState) -> u8 {
            match state {
                LedgerRecoveryState::Unknown => 0,
                LedgerRecoveryState::Unresolved => 1,
                LedgerRecoveryState::Recovered => 2,
            }
        }
        if rank(group.state) > rank(self.state) {
            self.state = group.state;
        }
        for item in &group.evidence {
            let success = item.success.as_ref().map(|success| success.sequence);
            match self
                .evidence
                .iter_mut()
                .find(|(failure, _)| *failure == item.failure.sequence)
            {
                Some((_, held)) => *held = held.or(success),
                None => self.evidence.push((item.failure.sequence, success)),
            }
        }
        self.evidence.sort_by_key(|(failure, _)| *failure);
        for gap in &group.gaps {
            if !self.gaps.contains(gap) {
                self.gaps.push(*gap);
            }
        }
    }
}

/// One line of the middle list: either a run's recovery header, or an event.
pub enum DisplayRow<'a> {
    Recovery(&'a RecoveryGroup),
    Event { event: &'a ProjectedEvent, folded: bool },
}

/// How far into the snapshot the page read, as the page itself states it.
pub struct ReadScope {
    pub scanned_through_position: Option<u64>,
    pub read_complete: bool,
}

/// The ledger facts about the open source, plus the two page-scoped counts.
#[derive(Debug, Clone)]
pub struct InstanceCard {
    pub state_root: String,
    pub backend: String,
    pub latest_sequence: u64,
    pub event_count: Option<u64>,
    pub read_complete: bool,
    pub corrupt_tail: Option<String>,
    pub repair_count: Option<u64>,
    pub writer: WriterFacts,
    /// Rows this view has loaded so far — the one page-scoped count here.
    pub loaded_count: usize,
    /// The committed span of the whole snapshot, not of the loaded page.
    pub first_timestamp_unix_ms: Option<u64>,
    pub last_timestamp_unix_ms: Option<u64>,
    pub severity_counts: Vec<(EventSeverity, usize)>,
    pub modules: Vec<OriginModule>,
    /// The picked port's set and latest binding facts; `None` while every
    /// instance is shown.
    pub port: Option<PortEntry>,
}

pub struct DetailView<'a> {
    pub event: &'a ProjectedEvent,
    pub pretty_payload_json: String,
    pub overlays: Vec<Overlay>,
    /// The frame size the payload states, when it states one.
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
    pub query_error: Option<QueryError>,
    rows: Vec<ProjectedEvent>,
    recovery: Vec<RecoveryGroup>,
    next_cursor: Option<RuntimeEventQueryCursor>,
    scope: ReadScope,
    snapshot_position: u64,
    open: OpenReport,
    state_root: String,
    span: Option<(u64, u64)>,
}

impl ViewModel {
    pub fn new(
        open: OpenReport,
        snapshot_position: u64,
        state_root: String,
        span: Option<(u64, u64)>,
    ) -> Self {
        Self {
            tab: LedgerView::Events,
            filters: Filters::default(),
            selected_sequence: None,
            query_error: None,
            rows: Vec::new(),
            recovery: Vec::new(),
            next_cursor: None,
            scope: ReadScope { scanned_through_position: None, read_complete: true },
            snapshot_position,
            open,
            state_root,
            span,
        }
    }

    /// The whole committed time span, for the time-range cursor.
    pub fn span(&self) -> Option<(u64, u64)> {
        self.span
    }

    pub fn query(&self) -> Result<EventQuery, QueryError> {
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
        self.scope = ReadScope {
            scanned_through_position: page
                .read_scope()
                .map(|scope| scope.scanned_through_position),
            read_complete: page.read_scope().is_none_or(|scope| scope.read_complete),
        };
        for group in page.run_recovery() {
            let run_id = code(&group.run_id);
            match self
                .recovery
                .iter_mut()
                .find(|existing| existing.run_id == run_id)
            {
                Some(existing) => existing.absorb(group),
                None => {
                    let mut fresh = RecoveryGroup {
                        run_id,
                        state: group.state,
                        evidence: Vec::new(),
                        gaps: Vec::new(),
                    };
                    fresh.absorb(group);
                    self.recovery.push(fresh);
                }
            }
        }
    }

    pub fn read_scope(&self) -> &ReadScope {
        &self.scope
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
        let folded: Vec<Vec<u64>> = self.recovery.iter().map(RecoveryGroup::folded).collect();
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
                    && (folded[index].contains(&event.sequence)
                        || run.as_deref() == Some(group.run_id.as_str()))
            });
            if let Some(index) = anchor {
                header_shown[index] = true;
                display.push(DisplayRow::Recovery(&self.recovery[index]));
                for sequence in &folded[index] {
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
        Some(DetailView {
            event,
            pretty_payload_json: serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| payload.to_string()),
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

    /// `bindings` is the port map the session read once; the card takes the
    /// picked port's entry from it. Severity and loaded counts come from the
    /// re-queried page, never from the map.
    pub fn instance_card(&self, bindings: Option<&PortBindings>) -> InstanceCard {
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
            state_root: self.state_root.clone(),
            backend: self.open.backend.clone(),
            latest_sequence: self.open.latest_sequence,
            event_count: self.open.event_count,
            read_complete: self.open.read_complete,
            corrupt_tail: self.open.corrupt_tail.clone(),
            repair_count: self.open.repair_count,
            writer: self.open.writer.clone(),
            loaded_count: self.rows.len(),
            first_timestamp_unix_ms: self.span.map(|(first, _)| first),
            last_timestamp_unix_ms: self.span.map(|(_, last)| last),
            severity_counts,
            modules: self.modules(),
            port: self.filters.port.and_then(|port| {
                bindings?.ports.iter().find(|entry| entry.port == port).cloned()
            }),
        }
    }

    /// The position this whole session reads at; the same for every page.
    pub fn snapshot_position(&self) -> u64 {
        self.snapshot_position
    }
}
