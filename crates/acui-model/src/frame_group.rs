// SPDX-License-Identifier: GPL-3.0-only
//! One frame and what the ledger says happened on it: the capture's own
//! artifact, the recognition made on it, and the effect intents whose
//! coordinates were taken on it. Every event of a step names its frame through
//! `links.frame_id`, except the physical input, whose frame the `Ui` profile
//! does not carry; that one is found through the last effect intent of its run.

use acui_rows::{code, ArtifactKind, LedgerEventPosition, ProjectedEvent, TaskSemanticFact};
use serde_json::Value;

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

/// An input the step meant on the frame: its kind as the ledger writes it and
/// the points it names, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub step_index: u32,
    pub operation_label: String,
    pub kind: String,
    pub points: Vec<(f32, f32)>,
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
    pub marks: Vec<Mark>,
}

/// The group of frame `frame` (as `code` writes its id) over `events`; each
/// event counts once, by sequence.
pub fn frame_group<'a>(
    frame: &str,
    events: impl IntoIterator<Item = &'a ProjectedEvent>,
) -> FrameGroup {
    let mut group = FrameGroup { target: None, extent: None, recognition: None, marks: Vec::new() };
    let mut seen = Vec::new();
    let mut verified = false;
    for event in events {
        if seen.contains(&event.sequence)
            || event.links.frame_id().map(code).as_deref() != Some(frame)
        {
            continue;
        }
        seen.push(event.sequence);
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
            }) => {
                group.recognition = Some((matched_page.clone(), candidate_pages.len()));
                group.extent = Some((*frame_width as f32, *frame_height as f32));
            }
            Some(TaskSemanticFact::EffectIntent {
                step_index,
                operation_label,
                action,
                frame_extent,
            }) => {
                if let Some(extent) = frame_extent {
                    group.extent.get_or_insert((extent.width() as f32, extent.height() as f32));
                }
                let action = serde_json::to_value(action).unwrap_or(Value::Null);
                group.marks.push(Mark {
                    step_index: *step_index,
                    operation_label: operation_label.clone(),
                    kind: action["kind"].as_str().unwrap_or_default().to_string(),
                    points: points(&action),
                });
            }
            _ => {}
        }
    }
    group
}

fn is_effect_intent(event: &ProjectedEvent) -> bool {
    matches!(task_fact(event), Some(TaskSemanticFact::EffectIntent { .. }))
}

/// `x`/`y`, then `x1`/`y1` through `x3`/`y3`, as the action names them.
fn points(action: &Value) -> Vec<(f32, f32)> {
    let pair = |x: &str, y: &str| {
        Some((action.get(x)?.as_f64()? as f32, action.get(y)?.as_f64()? as f32))
    };
    std::iter::once(pair("x", "y"))
        .chain((1..=3).map(|index| pair(&format!("x{index}"), &format!("y{index}"))))
        .flatten()
        .collect()
}
