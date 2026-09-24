// SPDX-License-Identifier: GPL-3.0-only
//! Pure view model over the formal page: tabs, filters, paging, recovery
//! folding, selection. No toolkit here, no classification of its own, and no
//! human-facing wording — every name a person reads is chosen in `acui-app`,
//! which is the only crate that holds the two language tables.
//!
//! The view reads from the latest end: backward windows of positions, newest
//! first on screen. The ledger query has no descending order, but a window of
//! `WINDOW` positions holds at most one page's worth of events, so one query
//! reads it. A window that comes back sparse widens the next, up to
//! `MAX_WIDTH`; a wider one may take more than one page, read by following the
//! cursor, as the reply's byte limit may split any window.

mod frame_group;
mod overlay;
mod tabs;

pub use frame_group::{BasisVia, FrameGroup, Mark, TargetBox};
pub use overlay::{extract_frame_size, extract_overlays, Overlay};
pub use tabs::{tab_from_name, tab_name, ALL_TABS};

use acui_rows::{
    code, payload_value, ArtifactEvictionObservation, ArtifactKind, EventQuery, EventSeverity,
    InstanceId, LedgerCount, LedgerEventPosition, LedgerRecoveryGap, LedgerRecoveryState,
    LedgerRunRecovery, LedgerView, OpenReport, OriginModule, PortBindings, PortEntry,
    ProjectedArtifactReference, ProjectedEvent, ProjectionPayload, PublicEventPayload,
    RuntimeEventQueryPage, TaskSemanticFact, WriterFacts, MAX_RUNTIME_EVENT_QUERY_EVENTS,
};

/// Positions one backward window spans: never more events than one page's
/// event limit.
pub const WINDOW: u64 = MAX_RUNTIME_EVENT_QUERY_EVENTS as u64;
/// The widest a backward window grows while the windows come back sparse.
pub const MAX_WIDTH: u64 = WINDOW * 16;
/// The performance monitor's event types that are `Info` from every writer,
/// as the ledger names them: the ledger itself leaves them out while the view
/// hides routine performance events. `perf.summary` is not among them: the
/// capacity monitor writes it as a warning or an error under disk pressure.
const ROUTINE_PERFORMANCE: [&str; 2] = ["perf.pressure_ended", "perf.monitor_recovered"];
/// Rows one fill aims to add, and the most windows it ever reads to get them;
/// the console also stops a fill at a time budget, since every query is served
/// on the Runtime's ledger writer.
pub const FILL_ROWS: usize = MAX_RUNTIME_EVENT_QUERY_EVENTS as usize;
pub const FILL_WINDOWS: usize = 16;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Why this view has no rows to show: the filters as written, or the read itself.
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
/// Nothing here filters rows the console already holds, with one exception
/// the view states: the performance monitor's routine events are hidden —
/// the two types that are always `Info` left out by the ledger, its other
/// events below Warning dropped from each window read and counted — unless
/// shown here, picked as the module, or on the Health tab, which is made of
/// them. Its warnings and errors always show.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub show_performance: bool,
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

/// The positions the loaded windows cover, lowest to highest, and whether
/// every window's page said it read its range completely.
pub struct ReadScope {
    pub range: Option<(u64, u64)>,
    pub read_complete: bool,
}

/// The ledger facts about the open source, plus the counts of the loaded rows.
#[derive(Debug, Clone)]
pub struct InstanceCard {
    pub state_root: String,
    pub backend: String,
    pub latest_sequence: u64,
    pub event_count: LedgerCount,
    pub read_complete: bool,
    pub corrupt_tail: Option<String>,
    pub repair_count: LedgerCount,
    pub writer: WriterFacts,
    /// Rows this view has loaded so far, hidden performance-monitor events not
    /// among them.
    pub loaded_count: usize,
    /// The committed span of the whole snapshot, not of the loaded rows.
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
    /// The frame size the event states: its formal extent, else a
    /// `frame_width`/`frame_height` pair in the payload.
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
    /// What the last follow tick failed at, shown beside `query_error`. Only a
    /// follow tick sets or clears it, so following never wipes a failure of
    /// another read.
    pub follow_error: Option<QueryError>,
    /// Why the committed span could not be read, which leaves the time cursor
    /// without a range; stays until a span read succeeds.
    pub span_error: Option<QueryError>,
    /// Loaded rows in ledger order, oldest first; shown newest first.
    rows: Vec<ProjectedEvent>,
    recovery: Vec<RecoveryGroup>,
    /// The position this view reads down from, and the lowest one read so far.
    upper: u64,
    lowest_read: Option<u64>,
    /// The span of the next backward window: `WINDOW`, widened while sparse.
    width: u64,
    hidden_performance: usize,
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
            follow_error: None,
            span_error: None,
            rows: Vec::new(),
            recovery: Vec::new(),
            upper: snapshot_position,
            lowest_read: None,
            width: WINDOW,
            hidden_performance: 0,
            scope: ReadScope { range: None, read_complete: true },
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

    /// The filters' query, the routine performance types left out by the
    /// ledger while they are hidden.
    pub fn query(&self) -> Result<EventQuery, QueryError> {
        let mut query = self.filters.query(self.tab)?;
        if self.hides_performance() {
            query.exclude_event_types = ROUTINE_PERFORMANCE
                .iter()
                .map(|name| parse_id(name))
                .collect::<Option<_>>()
                .ok_or_else(|| QueryError::Rejected("routine_performance_type_unknown".into()))?;
            query
                .validate()
                .map_err(|error| QueryError::Rejected(error.code().to_string()))?;
        }
        Ok(query)
    }

    /// Starts the view over, reading down from `upper`: the pinned position,
    /// or the last one a time bound allows.
    pub fn begin(&mut self, upper: u64) {
        self.rows.clear();
        self.recovery.clear();
        self.selected_sequence = None;
        self.upper = upper;
        self.lowest_read = None;
        self.width = WINDOW;
        self.hidden_performance = 0;
        self.scope = ReadScope { range: None, read_complete: true };
    }

    /// The next window down, `(from, to)`, or `None` once position 1 is read.
    pub fn next_window(&self) -> Option<(u64, u64)> {
        let to = self.lowest_read.map_or(self.upper, |lowest| lowest - 1);
        (to > 0).then(|| (to.saturating_sub(self.width - 1).max(1), to))
    }

    pub fn has_earlier(&self) -> bool {
        self.next_window().is_some()
    }

    /// The filters' query bounded to one window.
    pub fn window_query(&self, (from, to): (u64, u64)) -> Result<EventQuery, QueryError> {
        let mut query = self.query()?;
        query.from_sequence = Some(from);
        query.to_sequence = Some(to);
        query
            .validate()
            .map_err(|error| QueryError::Rejected(error.code().to_string()))?;
        Ok(query)
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Routine performance-monitor events the ledger did return, dropped from
    /// the windows read so far; the types it leaves out are not counted.
    pub fn hidden_performance(&self) -> usize {
        self.hidden_performance
    }

    /// Whether the performance monitor's routine events are hidden: not asked
    /// for, not the module picked, not the Health tab.
    pub fn hides_performance(&self) -> bool {
        !self.filters.show_performance
            && self.filters.origin_module != Some(OriginModule::PerformanceMonitor)
            && self.tab != LedgerView::Health
    }

    /// One window read whole — its pages in order — put below the rows already
    /// loaded. The performance monitor's routine events are dropped and counted
    /// when hidden. A window that brought back fewer than a quarter page of
    /// events doubles the next one's span, up to `MAX_WIDTH`; one that brought
    /// half a page or more puts it back to `WINDOW`.
    pub fn apply_window(&mut self, (from, to): (u64, u64), pages: &[RuntimeEventQueryPage]) {
        let (mut events, complete, read) = self.take_window(pages);
        self.width = if read < WINDOW / 4 {
            (self.width * 2).min(MAX_WIDTH)
        } else if read >= WINDOW / 2 {
            WINDOW
        } else {
            self.width
        };
        events.append(&mut self.rows);
        self.rows = events;
        self.lowest_read = Some(from);
        self.scope = ReadScope {
            range: Some((from, self.scope.range.map_or(to, |(_, high)| high))),
            read_complete: self.scope.read_complete && complete,
        };
    }

    /// The pin moved: what the source now states about itself. Rows already
    /// read stay; the newer windows read what came after them.
    pub fn set_pin(&mut self, open: OpenReport, snapshot_position: u64) {
        self.open = open;
        self.snapshot_position = snapshot_position;
    }

    /// The committed span the time cursor runs over, read again.
    pub fn set_span(&mut self, span: Option<(u64, u64)>) {
        self.span = span;
    }

    /// The next window above what is loaded, up to the pin; `None` once the
    /// view reaches the pin, and always under a time bound, where the view does
    /// not read down from the pin.
    pub fn next_newer_window(&self) -> Option<(u64, u64)> {
        let from = self.upper + 1;
        (self.filters.to_timestamp_unix_ms.is_none() && from <= self.snapshot_position)
            .then(|| (from, (from + WINDOW - 1).min(self.snapshot_position)))
    }

    /// One newer window read whole, put above the rows already loaded. When
    /// no window below was read yet — the reload's first one failed — the
    /// view's lowest read position becomes this window's, so "read earlier"
    /// reads what lies below it, once.
    pub fn apply_newer_window(&mut self, (from, to): (u64, u64), pages: &[RuntimeEventQueryPage]) {
        let (mut events, complete, _) = self.take_window(pages);
        self.rows.append(&mut events);
        self.upper = to;
        self.lowest_read.get_or_insert(from);
        self.scope = ReadScope {
            range: Some((self.scope.range.map_or(from, |(low, _)| low), to)),
            read_complete: self.scope.read_complete && complete,
        };
    }

    /// A window's events in ledger order, performance-monitor ones dropped and
    /// counted when hidden, its recovery statements merged; whether every page
    /// read its range completely; and how many events the ledger returned.
    fn take_window(&mut self, pages: &[RuntimeEventQueryPage]) -> (Vec<ProjectedEvent>, bool, u64) {
        let hide = self.hides_performance();
        let mut events = Vec::new();
        let mut complete = true;
        let mut read = 0;
        for page in pages {
            read += page.events().len() as u64;
            for event in page.events() {
                let routine = matches!(event.severity, EventSeverity::Debug | EventSeverity::Info);
                if hide && routine && event.origin.module() == OriginModule::PerformanceMonitor {
                    self.hidden_performance += 1;
                } else {
                    events.push(event.clone());
                }
            }
            complete &= page.read_scope().is_none_or(|scope| scope.read_complete);
            for group in page.run_recovery() {
                self.absorb_recovery(group);
            }
        }
        (events, complete, read)
    }

    fn absorb_recovery(&mut self, group: &LedgerRunRecovery) {
        let run_id = code(&group.run_id);
        match self.recovery.iter_mut().find(|existing| existing.run_id == run_id) {
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

    pub fn read_scope(&self) -> &ReadScope {
        &self.scope
    }

    /// The modules of the loaded rows, and the performance monitor while any
    /// of its events are hidden, so it can still be picked.
    pub fn modules(&self) -> Vec<OriginModule> {
        let mut modules: Vec<OriginModule> =
            self.rows.iter().map(|event| event.origin.module()).collect();
        if self.hidden_performance > 0 {
            modules.push(OriginModule::PerformanceMonitor);
        }
        modules.sort();
        modules.dedup();
        modules
    }

    /// Rows newest first. Each run the pages carry a recovery statement for
    /// gets a header at its newest row, and every failure the ledger resolved
    /// is folded under it.
    pub fn display_rows(&self) -> Vec<DisplayRow<'_>> {
        let folded: Vec<Vec<u64>> = self.recovery.iter().map(RecoveryGroup::folded).collect();
        let mut header_shown = vec![false; self.recovery.len()];
        let mut emitted: Vec<u64> = Vec::new();
        let mut display = Vec::new();
        for event in self.rows.iter().rev() {
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
                for sequence in folded[index].iter().rev() {
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
            frame_size: frame_extent(event).or_else(|| extract_frame_size(&payload)),
        })
    }

    /// The frame the selected event was taken on or acted on, found among the
    /// loaded rows: the event that names it, and how.
    pub fn selected_basis(&self) -> Option<(&ProjectedEvent, BasisVia)> {
        frame_group::basis(self.selected()?, &self.rows)
    }

    /// What the loaded rows and `extra`, events read for it, say about frame
    /// `frame` (its id as `code` writes it).
    pub fn frame_group(&self, frame: &str, extra: &[ProjectedEvent]) -> FrameGroup {
        frame_group::frame_group(frame, self.rows.iter().chain(extra))
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
    /// re-queried rows, never from the map.
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

    /// The pin every page reads at; it moves only by `set_pin`.
    pub fn snapshot_position(&self) -> u64 {
        self.snapshot_position
    }
}

/// The frame extent the ledger formally states: an effect intent's
/// `frame_extent`, or the frame a geometry observation was made on. The
/// console reads the `Ui` profile, which the ledger projects as `Public`.
fn frame_extent(event: &ProjectedEvent) -> Option<(f32, f32)> {
    let extent = match task_fact(event)? {
        TaskSemanticFact::EffectIntent { frame_extent, .. } => (*frame_extent)?,
        TaskSemanticFact::GeometryObserved { observation } => observation.frame.as_ref()?.extent,
        _ => return None,
    };
    Some((extent.width() as f32, extent.height() as f32))
}

/// The task fact an event states, as the `Ui` profile projects it: `Public`.
fn task_fact(event: &ProjectedEvent) -> Option<&TaskSemanticFact> {
    let ProjectionPayload::Public(payload) = &event.payload else {
        return None;
    };
    let PublicEventPayload::Task(task) = payload.as_ref() else {
        return None;
    };
    task.task_semantic_fact()
}
