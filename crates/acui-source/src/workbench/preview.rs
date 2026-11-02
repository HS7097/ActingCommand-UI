// SPDX-License-Identifier: GPL-3.0-only
//! What the workbench's picture reads (UI-3c): the newest frame of one instance, what its
//! task recognized and meant on it, and that frame's material, on a connection of its own
//! ([`PreviewLane`]). Nothing out of here names a wire type: a frame is a [`FrameSeen`],
//! whose reference into the ledger only this file reads, and what was recognized and meant
//! on it is an [`Overlay`] of plain numbers, which [`FrameMarks`] folds out of the records
//! by the view model's own reading of a frame (`acui_model::frame_group`).
//!
//! The lane follows the Runtime across restarts as a [`ReadLane`] does, on its own
//! connection (actor `ui`, source `ui`, channel `ui`) at [`PREVIEW_IO`]. It asks two
//! things and writes nothing:
//!
//! - ledger pages (`QueryEvents`), which the Runtime's reader pool answers off the
//!   ledger's writer and which take no record: a poll after a known position, a lookup
//!   down from the head, the marks of one frame found by a lookup, and the head itself
//!   when a status did not state it;
//! - one frame's material whole (`ReadMaterial`, through [`MaterialReader`]), which takes
//!   no record when it is read or found gone, and one for any other failure.
//!
//! It never subscribes (`SubscribeEvents`): at the pinned Runtime a subscription that
//! falls behind halts the whole Runtime, while a page asks nothing of the writer. How
//! often it reads is the caller's; how much one page or one lookup may read is this
//! file's, in the table below, the read side of the picture's one table (its pace is the
//! console's).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use acui_rows::{
    code, page_source_complete, ArtifactKind, EventQuery, LedgerEventPosition,
    ProjectedArtifactReference, ProjectedEvent, ProjectionPayload, ProjectionProfile,
    PublicEventPayload, RuntimeEventQueryCursor, RuntimeEventQueryPage,
    RuntimeEventQueryPageRequest, RuntimeMaterialReadLimit as Limit,
    RuntimeMaterialReadState as State, MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use actingcommand_contract::outcome::Code;
use actingcommand_contract::{EventType, FrameId};
use actingcommand_runtime_client::RuntimeClientError;

use super::{connection_ended, InstanceKey, ReadLane, Unreached};
use crate::{ClientFailure, MaterialOutcome, MaterialReader, Spelling, MAX_FRAME_BYTES};

/// The lane's io timeout: how long a page or a material range waits for its receipt,
/// and the bound of its connect, as for the ledger window's page connection.
pub const PREVIEW_IO: Duration = Duration::from_secs(20);
/// The rows of one poll. The four kinds of record it asks for come nowhere near this many
/// in the seconds between two polls; a page that holds more is not used
/// ([`Poll::complete`]).
pub const POLL_ROWS: u16 = 64;
/// The rows of the one page of a frame's marks: its recognition and the inputs meant on
/// it come nowhere near this many.
pub const MARK_ROWS: u16 = 16;
/// The windows a lookup reads down from the head, each just below the one before: 512
/// positions, then four times as many each time.
pub const LOOKUP_WINDOWS: [u64; 5] = [512, 2_048, 8_192, 32_768, 131_072];
/// The most pages one lookup reads, a window's later pages included.
pub const LOOKUP_PAGES: u32 = 8;
/// The rows of one lookup page: the most a page holds.
const LOOKUP_ROWS: u16 = MAX_RUNTIME_EVENT_QUERY_EVENTS;

/// The contract's own spelling of a page's operation (`ContractLocation::QueryRuntimeEvents`).
const QUERY_RUNTIME_EVENTS: &str = "query_runtime_events";

/// One read, the owner epoch of the Runtime that answered it as the wire spells it, and
/// how many ledger pages it took.
#[derive(Debug, Clone)]
pub struct Reach<T> {
    pub epoch: String,
    pub value: T,
    pub pages: u32,
}

/// One poll's page: the frames and capture failures of the instance after the position
/// asked from, in ledger order, and what its tasks recognized and meant on frames.
#[derive(Debug, Clone)]
pub struct Poll {
    /// The position the Runtime answered at: where the next poll asks from, once this
    /// one is complete.
    pub snapshot: u64,
    /// The page holds every record asked for up to `snapshot`: it was cut neither by its
    /// rows, its bytes nor the Runtime's read budget, and its source is whole. One that
    /// is not is dropped, and the newest frame looked up instead (`PreviewLane::latest`).
    pub complete: bool,
    pub frames: Vec<FrameSeen>,
    pub failures: Vec<CaptureFailure>,
    pub marks: FrameMarks,
}

/// The records of what tasks recognized on frames of the instance and the inputs they meant
/// on them, as they came, which only [`FrameMarks::overlay`] reads. A frame's come after its
/// own `artifact.verified` record: a task recognizes a frame once it is captured and kept.
#[derive(Debug, Clone, Default)]
pub struct FrameMarks {
    events: Vec<ProjectedEvent>,
    /// The page they came on was not whole: a frame's may lack some.
    cut: bool,
}

impl FrameMarks {
    /// Whether the page they came on was not whole.
    pub fn cut(&self) -> bool {
        self.cut
    }

    /// Takes in `other`'s records, each once.
    pub fn absorb(&mut self, other: FrameMarks) {
        for event in other.events {
            if !self.events.iter().any(|held| held.sequence == event.sequence) {
                self.events.push(event);
            }
        }
        self.cut |= other.cut;
    }

    /// Keeps only the records of the frames `keys` names.
    pub fn keep(&mut self, keys: &[&FrameKey]) {
        self.events.retain(|event| {
            let key = event.links.frame_id().map(|frame| FrameKey::Frame(code(frame)));
            key.is_some_and(|key| keys.contains(&&key))
        });
    }

    /// What the records say was recognized and meant on the frame `key` names; none when
    /// they say nothing of it, and always for a frame that names no frame id. The targets'
    /// names are left out: the picture draws no words on a frame.
    pub fn overlay(&self, key: &FrameKey) -> Option<Overlay> {
        let FrameKey::Frame(frame) = key else {
            return None;
        };
        let group = acui_model::frame_group(frame, &self.events);
        if group.recognition.is_none() && group.marks.is_empty() {
            return None;
        }
        let boxes = group
            .boxes
            .iter()
            .map(|target| MarkBox {
                x: target.x,
                y: target.y,
                width: target.width,
                height: target.height,
                passed: target.passed,
            })
            .collect();
        let points = group.marks.iter().flat_map(|mark| mark.points.iter().copied()).collect();
        Some(Overlay { extent: group.extent, boxes, points })
    }
}

/// What a task recognized and meant on one frame, in the frame's pixels: the frame's size
/// as the recognition or an input's intent states it, if either does; the targets of the
/// latest recognition that have a region, each with whether it passed; and every point an
/// input was meant at.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub extent: Option<(f32, f32)>,
    pub boxes: Vec<MarkBox>,
    pub points: Vec<(f32, f32)>,
}

/// A target's region on a frame, and whether the recognition found it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub passed: bool,
}

/// What a lookup found.
#[derive(Debug, Clone)]
pub enum Latest {
    /// The newest frame it read. `capped`: it ran out of pages inside the window it
    /// found this in, so a newer one may be in the rest of that window.
    Found { frame: FrameSeen, capped: bool },
    /// No frame from `from` up to the head. Past the head when the lookup ran out of
    /// pages before it read even its first window whole, or the ledger is empty.
    NoneWithin { from: u64 },
}

/// A lookup that stopped short: why, and how many pages it asked by then, the one that
/// failed included, so that its reads count as those of one that did not.
#[derive(Debug, Clone)]
pub struct LookupStopped {
    pub failure: Unreached,
    pub pages: u32,
}

/// One captured frame, as its `artifact.verified` record states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSeen {
    pub key: FrameKey,
    /// The record's position, which the material read takes as its snapshot.
    pub sequence: u64,
    /// When the frame was captured: its artifact's creation, not the record's.
    pub captured_at_ms: u64,
    pub byte_count: u64,
    pub origin: FrameOrigin,
    /// The request the record names, as the wire spells it: only to tell a frame this
    /// console asked for.
    pub request: Option<String>,
    /// An eviction proof on the record names the frame's artifact (a ledger from before
    /// the frame cleaner): its material is not asked for.
    pub evicted: bool,
    reference: FrameRef,
}

/// Which frame it is: its frame id as the wire spells it, else, with none, its record's
/// position. What the recognition and taps on it are matched by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FrameKey {
    Frame(String),
    Sequence(u64),
}

/// What took a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameOrigin {
    /// A task's run, as the wire spells its id.
    Task { run_id: String },
    /// An observation, which names no run: a person's, a tool's, or the recovery ladder's.
    Observation,
}

/// Where a frame's material is: its record and its artifact; and the frame's id, which its
/// marks are asked by.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameRef {
    event: LedgerEventPosition,
    artifact: ProjectedArtifactReference,
    frame: Option<FrameId>,
}

/// A capture of the instance that failed. `code` is the outcome its record states, for
/// the words of a code table and the log only, never shown; absent when the record
/// states none, as some at the pinned Runtime do not (an observation's invalid frame or
/// unsaved material), and the `Ui` profile carries no diagnostic code instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFailure {
    pub sequence: u64,
    pub at_ms: u64,
    pub code: Option<String>,
}

/// What one material read came to.
pub enum FrameRead {
    /// The material whole, verified against its committed hash.
    Bytes(Vec<u8>),
    /// No longer wanted while it ran: nothing to show.
    Superseded,
    /// Why not, and for the log only, what the read face said: its state, limit and
    /// failure as the wire spells them.
    Failed { why: FrameFailure, detail: String },
}

/// Why a frame's material did not come, in the classes the picture has words for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFailure {
    /// The file is not there, by the frame cleaner or a hand: no record is taken.
    Missing,
    /// An eviction proof names it (an older ledger): not asked for, or refused so.
    Evicted,
    /// The Runtime's read budget or the console's deadline ran out.
    TimedOut,
    /// It does not verify: by the Runtime, which records it, or by the client's assembly.
    Integrity,
    /// The material store could not read it now, a lock taken among others.
    Busy,
    /// Over the console's cap, so not asked for, or a range over the reply's.
    TooLarge,
    /// The Runtime refused the read, or does not hold its source whole.
    Refused,
    /// The connection failed under the read, and is let go.
    Connection,
}

/// The picture's connection to the running Runtime. No connection when made: the first
/// read makes it.
pub struct PreviewLane {
    lane: ReadLane,
}

impl PreviewLane {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { lane: ReadLane::new(root, PREVIEW_IO) }
    }

    /// The instance's frames, capture failures, recognitions and inputs meant recorded after
    /// `after`, at the Runtime's latest position: one page of at most [`POLL_ROWS`] rows,
    /// asked without a snapshot.
    pub fn poll(&mut self, instance: InstanceKey, after: u64) -> Result<Reach<Poll>, Unreached> {
        let query = EventQuery {
            instance_id: Some(instance.0),
            event_types: vec![
                EventType::ArtifactVerified,
                EventType::CaptureFailed,
                EventType::TaskRecognitionCompleted,
                EventType::TaskEffectIntent,
            ],
            from_sequence: Some(after.saturating_add(1)),
            ..EventQuery::default()
        };
        let request = page_request(POLL_ROWS, None)?.without_run_recovery();
        let (epoch, page) = self.page(&query, &request)?;
        let complete = !page.has_more() && page_source_complete(&page);
        let mut frames = Vec::new();
        let mut failures = Vec::new();
        let mut marks = FrameMarks { events: Vec::new(), cut: !complete };
        for event in page.events() {
            if let Some(frame) = frame_seen(event) {
                frames.push(frame);
            } else if event.event_type == EventType::CaptureFailed {
                failures.push(CaptureFailure {
                    sequence: event.sequence,
                    at_ms: event.timestamp_unix_ms,
                    code: failure_code(event),
                });
            } else if is_mark(event) {
                marks.events.push(event.clone());
            }
        }
        let poll =
            Poll { snapshot: page.snapshot_ledger_position(), complete, frames, failures, marks };
        Ok(Reach { epoch, value: poll, pages: 1 })
    }

    /// What tasks recognized on `frame`, a frame a lookup found, and the inputs meant on it,
    /// recorded after it up to `head`, the position the lookup read at: one page of at most
    /// [`MARK_ROWS`] rows at that snapshot, as polls after `head` bring the rest. `None` for
    /// a frame that names no frame id, which nothing names: no page is asked.
    pub fn marks(
        &mut self,
        instance: InstanceKey,
        frame: &FrameSeen,
        head: u64,
    ) -> Result<Option<Reach<FrameMarks>>, Unreached> {
        let Some(frame_id) = frame.reference.frame else {
            return Ok(None);
        };
        let query = EventQuery {
            instance_id: Some(instance.0),
            frame_id: Some(frame_id),
            event_types: vec![EventType::TaskRecognitionCompleted, EventType::TaskEffectIntent],
            from_sequence: Some(frame.sequence),
            ..EventQuery::default()
        };
        let request = page_request(MARK_ROWS, None)?
            .at_snapshot(head)
            .map_err(|error| request_refused(error.code()))?
            .without_run_recovery();
        let (epoch, page) = self.page(&query, &request)?;
        let complete = !page.has_more() && page_source_complete(&page);
        let events = page.events().iter().filter(|event| is_mark(event)).cloned().collect();
        let marks = FrameMarks { events, cut: !complete };
        Ok(Some(Reach { epoch, value: marks, pages: 1 }))
    }

    /// The instance's newest frame at or below `head`, a position the Runtime stated.
    /// Pages run in ascending order, so this reads down by windows ([`LOOKUP_WINDOWS`]),
    /// each at the snapshot `head` and to its end through its cursor, and stops at the
    /// first window that holds a frame, at the last window, at the ledger's first
    /// position, or after [`LOOKUP_PAGES`] pages, with the newest frame it read by then.
    /// The reach is the first page's: the ledger's positions run on across a restart, so a
    /// page on either side of one tells the same history, and a caller already at the new
    /// owner drops the reach and looks again. One that stops short states the pages it
    /// asked by then.
    pub fn latest(
        &mut self,
        instance: InstanceKey,
        head: u64,
    ) -> Result<Reach<Latest>, LookupStopped> {
        let mut pages = 0;
        let looked = self.look_down(instance, head, &mut pages);
        looked.map_err(|failure| LookupStopped { failure, pages })
    }

    /// [`Self::latest`], counting in `pages` every page it asks, the one that fails
    /// included.
    fn look_down(
        &mut self,
        instance: InstanceKey,
        head: u64,
        pages: &mut u32,
    ) -> Result<Reach<Latest>, Unreached> {
        let mut answered: Option<String> = None;
        let mut newest: Option<FrameSeen> = None;
        let mut capped = false;
        // Every position from here up to the head has been read.
        let mut read_from = head.saturating_add(1);
        let mut to = head;
        'windows: for width in LOOKUP_WINDOWS {
            if to == 0 {
                break;
            }
            let from = to.saturating_sub(width - 1).max(1);
            let query = EventQuery {
                instance_id: Some(instance.0),
                event_type: Some(EventType::ArtifactVerified),
                from_sequence: Some(from),
                to_sequence: Some(to),
                ..EventQuery::default()
            };
            let mut cursor = None;
            loop {
                if *pages == LOOKUP_PAGES {
                    capped = true;
                    break 'windows;
                }
                let request = page_request(LOOKUP_ROWS, cursor)?
                    .at_snapshot(head)
                    .map_err(|error| request_refused(error.code()))?
                    .without_run_recovery();
                *pages += 1;
                let (epoch, page) = self.page(&query, &request)?;
                if answered.is_none() {
                    answered = Some(epoch);
                }
                if let Some(frame) = page.events().iter().rev().find_map(frame_seen) {
                    newest = Some(frame);
                }
                match page.next_cursor() {
                    Some(next) => cursor = Some(next.clone()),
                    None => break,
                }
            }
            read_from = from;
            if newest.is_some() {
                break;
            }
            to = from - 1;
        }
        let epoch = match answered {
            Some(epoch) => epoch,
            // An empty ledger: no page was asked.
            None => self.epoch()?,
        };
        let value = match newest {
            Some(frame) => Latest::Found { frame, capped },
            None => Latest::NoneWithin { from: read_from },
        };
        Ok(Reach { epoch, value, pages: *pages })
    }

    /// `frame`'s material whole, through the client's complete read under the console's
    /// byte cap and deadline (`MAX_FRAME_BYTES`, 30 s), at the frame's own position. The
    /// client asks `still_wanted` between ranges; a read no longer wanted comes to
    /// nothing. A frame an eviction proof names, or one over the cap, is not asked for.
    /// After a read the client latched its connection on, the lane lets the connection
    /// go and its next read connects anew.
    pub fn read(
        &mut self,
        frame: &FrameSeen,
        still_wanted: &dyn Fn() -> bool,
    ) -> Result<Reach<FrameRead>, Unreached> {
        let client = self.lane.connection().map_err(Unreached::of)?;
        let epoch = code(&client.runtime_info().owner_epoch());
        let value = if frame.evicted {
            FrameRead::Failed {
                why: FrameFailure::Evicted,
                detail: "an eviction proof on its record names it; not asked for".to_owned(),
            }
        } else if frame.byte_count > MAX_FRAME_BYTES {
            FrameRead::Failed {
                why: FrameFailure::TooLarge,
                detail: format!(
                    "{} bytes, over the cap of {MAX_FRAME_BYTES}; not asked for",
                    frame.byte_count
                ),
            }
        } else {
            let latched = Arc::new(AtomicBool::new(false));
            let reader = MaterialReader::Online { client, latched: Arc::clone(&latched) };
            let read = reader.read(
                frame.reference.event,
                &frame.reference.artifact,
                frame.sequence,
                still_wanted,
            );
            let fatal = latched.load(Ordering::SeqCst);
            if fatal {
                self.lane.client = None;
            }
            match read {
                Ok(None) => FrameRead::Superseded,
                Ok(Some(outcome)) => frame_read(outcome, fatal),
                // Only an offline reader or one whose rebuild failed fails so; this is
                // neither.
                Err(error) => FrameRead::Failed {
                    why: FrameFailure::Connection,
                    detail: format!("{error:#}"),
                },
            }
        };
        Ok(Reach { epoch, value, pages: 0 })
    }

    /// One page on this lane's connection, and the owner epoch of the Runtime that
    /// answered it. A connection kept from before that turns out to have ended is asked
    /// again once, on a new one, as `ReadLane::asked_once_more` does: the request reached
    /// nothing.
    fn page(
        &mut self,
        query: &EventQuery,
        request: &RuntimeEventQueryPageRequest,
    ) -> Result<(String, RuntimeEventQueryPage), Unreached> {
        let kept = self.lane.client.is_some();
        let asked = match self.ask_page(query, request) {
            Err(error) if kept && connection_ended(&error) => self.ask_page(query, request),
            asked => asked,
        };
        asked.map_err(Unreached::of)
    }

    fn ask_page(
        &mut self,
        query: &EventQuery,
        request: &RuntimeEventQueryPageRequest,
    ) -> Result<(String, RuntimeEventQueryPage), RuntimeClientError> {
        let client = self.lane.connection()?;
        let epoch = code(&client.runtime_info().owner_epoch());
        match client.query_event_page(query.clone(), ProjectionProfile::Ui, request.clone()) {
            Ok(page) => Ok((epoch, page)),
            Err(error) => Err(self.lane.settled(error)),
        }
    }

    /// The owner epoch of the Runtime the lane's connection reaches, made first when there
    /// is none.
    fn epoch(&mut self) -> Result<String, Unreached> {
        let client = self.lane.connection().map_err(Unreached::of)?;
        Ok(code(&client.runtime_info().owner_epoch()))
    }
}

/// A page request of `limit` rows on from `cursor`. The contract's refusal of one, which
/// these limits never meet, is trouble, with its own code.
fn page_request(
    limit: u16,
    cursor: Option<RuntimeEventQueryCursor>,
) -> Result<RuntimeEventQueryPageRequest, Unreached> {
    RuntimeEventQueryPageRequest::new(limit, cursor).map_err(|error| request_refused(error.code()))
}

fn request_refused(code: Code) -> Unreached {
    Unreached::Trouble(ClientFailure {
        code: code.spelling(),
        operation: QUERY_RUNTIME_EVENTS,
        runtime_code: None,
        host: None,
    })
}

/// The frame an `artifact.verified` record states, if it states one: a capture's
/// artifact, not a diagnostic file's or an evidence package's.
fn frame_seen(event: &ProjectedEvent) -> Option<FrameSeen> {
    if event.event_type != EventType::ArtifactVerified {
        return None;
    }
    let artifact =
        event.artifacts.iter().find(|artifact| artifact.kind == ArtifactKind::CaptureFrame)?;
    let key = match event.links.frame_id() {
        Some(frame) => FrameKey::Frame(code(frame)),
        None => FrameKey::Sequence(event.sequence),
    };
    let origin = match event.links.run_id() {
        Some(run) => FrameOrigin::Task { run_id: code(run) },
        None => FrameOrigin::Observation,
    };
    Some(FrameSeen {
        key,
        sequence: event.sequence,
        captured_at_ms: artifact.created_at_unix_ms,
        byte_count: artifact.byte_count,
        origin,
        request: event.links.request_id().map(code),
        evicted: event
            .artifact_evictions
            .iter()
            .any(|eviction| eviction.artifact_id == artifact.artifact_id),
        reference: FrameRef {
            event: LedgerEventPosition { event_id: event.event_id, sequence: event.sequence },
            artifact: artifact.clone(),
            frame: event.links.frame_id().copied(),
        },
    })
}

/// Whether a record is one of what a task recognized on a frame, or an input it meant on one.
fn is_mark(event: &ProjectedEvent) -> bool {
    matches!(event.event_type, EventType::TaskRecognitionCompleted | EventType::TaskEffectIntent)
}

/// The outcome code a `capture.failed` record states, codes only in the `Ui` profile.
fn failure_code(event: &ProjectedEvent) -> Option<String> {
    let ProjectionPayload::Public(payload) = &event.payload else {
        return None;
    };
    let PublicEventPayload::Capture(capture) = payload.as_ref() else {
        return None;
    };
    capture.outcome().map(|outcome| outcome.code().as_str().to_owned())
}

/// What a material read that came back comes to. `fatal`: the client latched the
/// connection on it, which a read the store could not make does not, and a connection
/// that failed under the read does.
fn frame_read(outcome: MaterialOutcome, fatal: bool) -> FrameRead {
    let MaterialOutcome { bytes, state, limit, eviction, failure } = outcome;
    if let Some(bytes) = bytes {
        return FrameRead::Bytes(bytes);
    }
    let why = match (state, limit) {
        (State::Missing, _) => FrameFailure::Missing,
        (
            State::NotProvided,
            Some(Limit::PendingEviction | Limit::Evicted | Limit::EvictionFailed),
        ) => FrameFailure::Evicted,
        (State::NotProvided, Some(Limit::BudgetExceeded)) => FrameFailure::TimedOut,
        (State::NotProvided, Some(Limit::ReplyBytes)) => FrameFailure::TooLarge,
        (State::IntegrityFailed, _) => FrameFailure::Integrity,
        (State::ReadFailed, _) if fatal => FrameFailure::Connection,
        (State::ReadFailed, _) => FrameFailure::Busy,
        // A refusal, a source the Runtime does not hold whole; and what neither face gives,
        // a read not provided without its limit or a verified one without its bytes.
        (
            State::RequestDenied | State::SourceIncomplete | State::NotProvided | State::Verified,
            _,
        ) => FrameFailure::Refused,
    };
    let mut detail = code(&state);
    if let Some(limit) = &limit {
        detail.push_str(&format!(" · {}", code(limit)));
    }
    if let Some(eviction) = &eviction {
        detail.push_str(&format!(" · evicted through {}", eviction.through_sequence));
    }
    if let Some(failure) = &failure {
        detail.push_str(&format!(" · {failure}"));
    }
    if fatal {
        detail.push_str(" · connection latched");
    }
    FrameRead::Failed { why, detail }
}
