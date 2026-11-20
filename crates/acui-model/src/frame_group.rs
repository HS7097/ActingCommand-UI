// SPDX-License-Identifier: GPL-3.0-only
//! One frame and what the ledger says happened on it: the capture's own
//! artifact, the recognition made on it, and the effect intents whose
//! coordinates were taken on it. Every event of a step names its frame through
//! `links.frame_id`, except the physical input, whose frame the `Ui` profile
//! does not carry; that one is found through the last effect intent of its run.
//!
//! An effect intent's input is in base pixels and its frame is the device's: its
//! points are put on the frame by its run's display map, as the records read of
//! the run (`RunMaps`) know it, through the product's own conversion
//! (`effect_points`).

use acui_rows::{
    code, ArtifactKind, InputAction, LedgerEventPosition, ProjectedEvent, TaskSemanticFact,
};
use serde_json::Value;

use crate::effect_points::{drawn_input, DrawnInput, InEffect, RunMaps};
use crate::{task_fact, FrameTarget};

/// How the frame of an event was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasisVia {
    /// The event names the frame itself.
    Own,
    /// Another event of the same step, its effect intent, names it.
    Step,
    /// A physical input: the last effect intent of its run before it names it.
    Input,
}

/// The frame an event was taken on or acted on, found among `rows`: the event
/// that names it, and how.
pub fn basis<'a>(
    event: &'a ProjectedEvent,
    rows: &'a [ProjectedEvent],
) -> Option<(&'a ProjectedEvent, BasisVia)> {
    if event.links.frame_id().is_some() {
        return Some((event, BasisVia::Own));
    }
    if let Some(action) = event.links.action_id() {
        let step = rows.iter().find(|row| {
            row.links.action_id() == Some(action)
                && row.links.frame_id().is_some()
                && is_effect_intent(row)
        });
        if let Some(step) = step {
            return Some((step, BasisVia::Step));
        }
    }
    let run = event.links.run_id()?;
    if !code(&event.event_type).starts_with("input.") {
        return None;
    }
    rows.iter()
        .filter(|row| {
            row.sequence < event.sequence
                && row.links.run_id() == Some(run)
                && row.links.frame_id().is_some()
                && is_effect_intent(row)
        })
        .max_by_key(|row| row.sequence)
        .map(|row| (row, BasisVia::Input))
}

/// An input the step meant on the frame: its kind as the effect intent writes
/// it, the package's; the run it was meant in, by its id as `code` writes it;
/// how its points were put on the device frame; and those points, in order, in
/// device-frame pixels, read from the input as converted (`x`/`y`, then `x1`/`y1`
/// through `x3`/`y3`, then a path drag's `points`). No point is given of an input
/// that was not converted, or whose run's map is not known yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub step_index: u32,
    pub operation_label: String,
    pub kind: String,
    pub run: Option<String>,
    pub conversion: MarkConversion,
    pub points: Vec<(f32, f32)>,
}

/// How a mark's points were put on the device frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkConversion {
    /// As asked: its run has no map in effect, or the map is the plain identity,
    /// or the effect intent states no frame size.
    AsAsked,
    /// Converted by its run's map, as the product sends it.
    Converted,
    /// The product refuses this input under its run's map: no point is drawn.
    NotConverted,
    /// Its run's map in effect is not known: its records are not read up to the
    /// effect intent, or their read failed or was not whole. No point is drawn.
    Unknown,
}

/// One target the recognition evaluated on the frame, where it has a region:
/// the matched page's (or, with no match, the first candidate's), in frame
/// pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetBox {
    pub target_id: String,
    /// `required`, `any_of`, `optional` or `forbidden`, as the ledger writes it.
    pub role: String,
    pub passed: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// What the ledger says about one frame.
pub struct FrameGroup {
    /// The capture's verified artifact, else the one created, when read.
    pub target: Option<FrameTarget>,
    /// The frame's pixel size as the recognition or an effect intent states it.
    pub extent: Option<(f32, f32)>,
    /// `Some` once a recognition on the frame is read: its matched page, and
    /// how many pages it tried.
    pub recognition: Option<(Option<String>, usize)>,
    /// The latest recognition's targets: how many it evaluated and how many
    /// passed, and the boxes of those with a region.
    pub targets: (usize, usize),
    pub boxes: Vec<TargetBox>,
    pub marks: Vec<Mark>,
}

/// The group of frame `frame` (as `code` writes its id) over `events`, taken
/// in ledger order, each event once: a later recognition states the page. Each
/// effect intent's mark is put on the frame by its run's map in effect as `runs`
/// know it.
pub fn frame_group<'a>(
    frame: &str,
    events: impl IntoIterator<Item = &'a ProjectedEvent>,
    runs: &RunMaps,
) -> FrameGroup {
    let mut group = FrameGroup {
        target: None,
        extent: None,
        recognition: None,
        targets: (0, 0),
        boxes: Vec::new(),
        marks: Vec::new(),
    };
    let mut events: Vec<&ProjectedEvent> = events
        .into_iter()
        .filter(|event| event.links.frame_id().map(code).as_deref() == Some(frame))
        .collect();
    events.sort_by_key(|event| event.sequence);
    events.dedup_by_key(|event| event.sequence);
    let mut verified = false;
    for event in events {
        let capture = event
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == ArtifactKind::CaptureFrame);
        if let Some(artifact) = capture {
            let this_verified = code(&event.event_type) == "artifact.verified";
            if group.target.is_none() || (this_verified && !verified) {
                verified = this_verified;
                group.target = Some(FrameTarget {
                    event: LedgerEventPosition { event_id: event.event_id, sequence: event.sequence },
                    eviction: event
                        .artifact_evictions
                        .iter()
                        .find(|eviction| eviction.artifact_id == artifact.artifact_id)
                        .cloned(),
                    artifact: artifact.clone(),
                });
            }
        }
        match task_fact(event) {
            Some(TaskSemanticFact::RecognitionCompleted {
                candidate_pages,
                matched_page,
                frame_width,
                frame_height,
                targets,
                ..
            }) => {
                group.recognition = Some((matched_page.clone(), candidate_pages.len()));
                group.extent = Some((*frame_width as f32, *frame_height as f32));
                let passed = targets.iter().filter(|target| target.passed).count();
                group.targets = (passed, targets.len());
                group.boxes = targets
                    .iter()
                    .filter_map(|target| {
                        let region = target.region.as_ref()?;
                        Some(TargetBox {
                            target_id: target.target_id.clone(),
                            role: code(&target.role),
                            passed: target.passed,
                            x: region.x as f32,
                            y: region.y as f32,
                            width: region.width as f32,
                            height: region.height as f32,
                        })
                    })
                    .collect();
            }
            Some(TaskSemanticFact::EffectIntent { frame_extent, .. }) => {
                if let Some(extent) = frame_extent {
                    group.extent.get_or_insert((extent.width() as f32, extent.height() as f32));
                }
                group.marks.extend(effect_mark(event, runs));
            }
            _ => {}
        }
    }
    group
}

/// The mark of `event` when it is an effect intent: its input put on its device
/// frame under its run's map in effect as `runs` know it (`drawn_input`). An
/// effect intent of no run has no map in effect, as the Runtime's own lookup
/// takes it (945727d4 `crates/ledger-forensics/src/input_pairing.rs:60-61`).
pub fn effect_mark(event: &ProjectedEvent, runs: &RunMaps) -> Option<Mark> {
    let Some(TaskSemanticFact::EffectIntent { step_index, operation_label, action, frame_extent }) =
        task_fact(event)
    else {
        return None;
    };
    let run = event.links.run_id().map(code);
    let in_effect = match &run {
        Some(run) => runs.in_effect(run, event.sequence),
        None => InEffect::Identity,
    };
    let drawn = match in_effect {
        InEffect::Unknown => None,
        InEffect::Identity => Some(drawn_input(action, *frame_extent, None)),
        InEffect::Map(map) => Some(drawn_input(action, *frame_extent, Some(map))),
    };
    let (conversion, at) = match drawn {
        None => (MarkConversion::Unknown, Vec::new()),
        Some(DrawnInput::AsAsked) => (MarkConversion::AsAsked, points(action)),
        Some(DrawnInput::Converted(device)) => (MarkConversion::Converted, points(&device)),
        Some(DrawnInput::NotConverted) => (MarkConversion::NotConverted, Vec::new()),
    };
    let asked = serde_json::to_value(action).unwrap_or(Value::Null);
    Some(Mark {
        step_index: *step_index,
        operation_label: operation_label.clone(),
        kind: asked["kind"].as_str().unwrap_or_default().to_string(),
        run,
        conversion,
        points: at,
    })
}

/// The effect intents on frame `frame` (as `code` writes its id) among
/// `events`, in ledger order, each once: those whose marks `frame_group` makes.
pub fn frame_effects<'a>(
    frame: &str,
    events: impl IntoIterator<Item = &'a ProjectedEvent>,
) -> Vec<&'a ProjectedEvent> {
    let mut effects: Vec<&ProjectedEvent> = events
        .into_iter()
        .filter(|event| {
            event.links.frame_id().map(code).as_deref() == Some(frame) && is_effect_intent(event)
        })
        .collect();
    effects.sort_by_key(|event| event.sequence);
    effects.dedup_by_key(|event| event.sequence);
    effects
}

pub(crate) fn is_effect_intent(event: &ProjectedEvent) -> bool {
    matches!(task_fact(event), Some(TaskSemanticFact::EffectIntent { .. }))
}

/// `x`/`y`, then `x1`/`y1` through `x3`/`y3`, then each `{x, y}` of `points` in
/// order, as the input names them. A path drag names only `points`, the points
/// it passes through, beside its box (Runtime 945727d4
/// `crates/actingcommand-contract/src/runtime.rs:795-803, 834-837`); converted,
/// they are the device points the product sends (`crates/display-adapter/src/lib.rs:1204-1249`).
pub(crate) fn points(action: &InputAction) -> Vec<(f32, f32)> {
    let action = serde_json::to_value(action).unwrap_or(Value::Null);
    let pair = |at: &Value, x: &str, y: &str| {
        Some((at.get(x)?.as_f64()? as f32, at.get(y)?.as_f64()? as f32))
    };
    let path = action.get("points").and_then(Value::as_array).into_iter().flatten();
    std::iter::once(pair(&action, "x", "y"))
        .chain((1..=3).map(|index| pair(&action, &format!("x{index}"), &format!("y{index}"))))
        .chain(path.map(|point| pair(point, "x", "y")))
        .flatten()
        .collect()
}
