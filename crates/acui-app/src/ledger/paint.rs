// SPDX-License-Identifier: GPL-3.0-only
//! The ledger window's paint, and every word of it: the card, the list, the
//! detail, the frame pane, the boxes and the busy line, made from the window's
//! state alone (`LedgerState`) in the language shown at that moment, while the
//! caller holds its lock. Nothing here reads the read face; a new frame read
//! takes the material reader the window's reader published, runs on the
//! window's frame slot and comes back here to be painted (`frame_read`).

use std::sync::Arc;
use std::time::Duration;

use acui_model::{
    BasisVia, DisplayRow, FrameGroup, FrameTarget, InstanceCard, Mark, Overlay, QueryError,
    RecoveryGroup, ViewModel, ALL_TABS,
};
use acui_rows::outcome::CONSOLE_RUNTIME_OWNER_CHANGED;
use acui_rows::{
    code, event_type_names, format_bytes, format_clock, format_full, links_named, module_names,
    seconds_since, short_id, ArtifactEvictionObservation, EventSeverity, LedgerCount,
    OriginModule, ProjectedEvent, Sensitivity, WriterFacts,
};
use acui_source::{MaterialOutcome, OfflineFacts, OfflineReason, RuntimeInstance, MAX_FRAME_BYTES};
use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};

use crate::ledger::frame::{FrameRequest, FrameSlot};
use crate::ledger::resume;
use crate::ledger::state::{
    BusyKind, FrameLookupError, InstancesSnapshot, LedgerFailure, LedgerState, Phase, PortMap,
    SourceFacts,
};
use crate::ledger::LedgerHandle;
use crate::phrase::{phrase, Phrase};
use crate::strings::{fill, outcome_sentence, outcome_short, Labels, Language};
use crate::ui::Ui;
use crate::{ArtifactItem, FieldLine, LedgerWindow, OverlayItem, RowItem, Strings, TargetItem};

/// How long a request runs before the busy line shows.
const BUSY_QUIET: Duration = Duration::from_secs(1);

/// The port box's items, and the port each item after the first one means: the
/// first means every instance, then one per port, ascending, naming the port
/// and the member its latest binding is for, with ` +n` for the further
/// members of the same port. A map with nothing to pick — no binding facts,
/// facts that name no port, online, a face still opening or unopened, or a
/// refused read — gives one item that says which, and no choices, so the box
/// is disabled. Made again at every paint and every pick, in the language
/// shown then; the map itself is fixed for the session.
pub fn port_options(labels: &Labels, port_map: &PortMap) -> (Vec<SharedString>, Vec<u16>) {
    let bindings = match port_map {
        PortMap::Read(bindings) if !bindings.ports.is_empty() => bindings,
        PortMap::Read(bindings) if !bindings.unported.is_empty() => {
            return (vec![labels.all_instances.into()], Vec::new())
        }
        PortMap::Read(_) => return (vec![labels.no_binding_records.into()], Vec::new()),
        PortMap::Opening => return (vec![labels.ledger_opening.into()], Vec::new()),
        PortMap::Online => return (vec![labels.port_online_unsupported.into()], Vec::new()),
        PortMap::Unopened => return (vec![labels.ledger_unopened.into()], Vec::new()),
        PortMap::Failed(error) => return (vec![error.to_string().into()], Vec::new()),
    };
    let mut options = vec![SharedString::from(labels.all_instances)];
    options.extend(bindings.ports.iter().map(|entry| {
        let mut text = fill(
            labels.port_option,
            &[&entry.port.to_string(), &short_id(&code(&entry.latest_instance_id))],
        );
        if entry.members.len() > 1 {
            text.push_str(&format!(" +{}", entry.members.len() - 1));
        }
        SharedString::from(text)
    }));
    (options, bindings.ports.iter().map(|entry| entry.port).collect())
}

/// Paints the ledger window from `state` alone, in the language of `labels`:
/// nothing here reads the read face, and the caller holds the state's lock for
/// the length of the paint. What a paint needs read, the window's reader read
/// before it. A new frame read takes a clone of the material reader the reader
/// published. `runtime_ready` is the launcher's newest probe: a Runtime is up.
pub fn refresh(
    window: &LedgerWindow,
    ledger: &LedgerHandle,
    state: &LedgerState,
    labels: &Labels,
    runtime_ready: bool,
) {
    let source = match &state.phase {
        Phase::Open(facts) if facts.offline_reason.is_none() => labels.data_source_online,
        Phase::Open(_) | Phase::Unopened { .. } => labels.data_source,
        Phase::Opening { .. } | Phase::Failed { .. } => labels.data_source_pending,
    };
    window.global::<Strings>().set_data_source(source.into());
    window.set_busy_text(busy_text(labels, state).into());
    paint_reopen(window, state, labels, runtime_ready);
    resume::paint(window, ledger, state, labels);
    let (Phase::Open(facts), Some(model)) = (&state.phase, &state.model) else {
        paint_closed(window, ledger, state, labels);
        return;
    };
    let card = model.instance_card(state.port_bindings());

    window.set_unopened_note(SharedString::new());
    window.set_unopened_failed(false);
    window.set_ledger_dir_value(card.state_root.as_str().into());
    window.set_storage_format_value(backend_text(labels, &card.backend).into());
    let scope = model.read_scope();
    window.set_read_up_to_text(
        match scope.range {
            Some((low, high)) => {
                let mut text = fill(labels.read_range, &[&low.to_string(), &high.to_string()]);
                if !scope.read_complete {
                    text.push_str(" · ");
                    text.push_str(labels.source_incomplete);
                }
                text
            }
            None => labels.read_up_to_unknown.to_string(),
        }
        .into(),
    );
    let mut loaded = fill(labels.loaded_rows_top, &[&card.loaded_count.to_string()]);
    if model.hidden_performance() > 0 {
        let hidden = model.hidden_performance().to_string();
        loaded.push_str(&fill(labels.performance_hidden, &[&hidden]));
    }
    window.set_loaded_rows_text(loaded.into());
    window.set_show_performance(model.filters.show_performance);
    window.set_follow_available(facts.follows);
    window.set_cursor_label(
        model
            .filters
            .to_timestamp_unix_ms
            .map(|bound| format!("≤ {}", format_full(bound)))
            .unwrap_or_else(|| labels.all.to_string())
            .into(),
    );

    let mut lines = vec![
        // Which face answered, and for an `auto` pick, why: never a silent choice.
        field(
            labels,
            "source",
            source_text(labels, state),
            if state.offline_reason().is_some() { "offline" } else { "online" },
        ),
        field(labels, "latest_sequence", card.latest_sequence.to_string(), "latest_sequence"),
        field(labels, "event_count", count_text(labels, card.event_count), "event_count"),
        field(labels, "loaded_rows", card.loaded_count.to_string(), ""),
        field(labels, "first_event", stamp(labels, card.first_timestamp_unix_ms), ""),
        field(labels, "latest_event", stamp(labels, card.last_timestamp_unix_ms), ""),
        field(
            labels,
            "age",
            card.last_timestamp_unix_ms
                .map(|stamp| age_text(labels, stamp))
                .unwrap_or_else(|| labels.none.to_string()),
            "",
        ),
        field(labels, "integrity", integrity_text(labels, &card), ""),
        field(labels, "repair_count", count_text(labels, card.repair_count), "repair_count"),
        field(labels, "writer", writer_text(labels, &card.writer), ""),
    ];
    // The port map could not be read: the code, and behind it the read
    // face's own words. The box beside the list carries the same code.
    if let PortMap::Failed(error) = &state.port_map {
        lines.push(field(
            labels,
            "port_bindings",
            fill(labels.read_failed, &[&error.to_string()]),
            error.root_cause().to_string(),
        ));
    }
    lines.extend(instance_lines(labels, facts));
    // The picked port: its latest binding's facts and the size of its set.
    // The counts below still come from the re-queried page.
    if let Some(entry) = &card.port {
        lines.push(field(
            labels,
            "instance_alias",
            entry.latest_alias.clone(),
            code(&entry.latest_instance_id),
        ));
        lines.push(field(labels, "adb_port", entry.port.to_string(), ""));
        lines.push(field(
            labels,
            "provenance",
            labels.provenance(&entry.latest_provenance).to_string(),
            entry.latest_provenance.clone(),
        ));
        lines.push(field(labels, "bound_ids", entry.members.len().to_string(), ""));
        lines.push(field(labels, "latest_binding", entry.latest_sequence.to_string(), ""));
    }
    for (severity, count) in &card.severity_counts {
        lines.push(FieldLine {
            label: labels.levels[severity_index(*severity)].into(),
            value: count.to_string().into(),
            raw: severity.as_str().into(),
            wrap: false,
        });
    }
    // Last line of the card, and the only one that may run to several lines.
    lines.push(FieldLine {
        label: labels.field("modules").into(),
        value: card
            .modules
            .iter()
            .map(|module| module_text(labels, *module))
            .collect::<Vec<_>>()
            .join(" · ")
            .into(),
        raw: SharedString::new(),
        wrap: true,
    });
    window.set_card_lines(models(lines));

    // Only the tab a person is looking at carries a number, and the number is
    // the rows this view has loaded — never a count taken from another view.
    let active = ALL_TABS.iter().position(|tab| *tab == model.tab).unwrap_or(0);
    sync_tab_labels(
        window,
        labels
            .tabs
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if index == active {
                    let badge = fill(labels.loaded_badge, &[&card.loaded_count.to_string()]);
                    SharedString::from(format!("{name} · {badge}"))
                } else {
                    SharedString::from(*name)
                }
            })
            .collect(),
    );
    window.set_active_tab(active as i32);

    sync_modules(window, ledger, labels, &model.filters.origin_module.clone(), card.modules);
    sync_ports(window, state, labels, model.filters.port);

    let display = model.display_rows();
    let mut errors = Vec::new();
    if let Some(error) = &model.query_error {
        errors.push(query_error_text(labels, Some(error)));
    }
    if let Some(error) = &model.follow_error {
        errors.push(fill(labels.follow_failed, &[&query_error_text(labels, Some(error))]));
    }
    if let Some(error) = &model.span_error {
        errors.push(fill(labels.span_failed, &[&query_error_text(labels, Some(error))]));
    }
    window.set_filter_error(errors.join(" · ").into());
    window.set_has_more(model.has_earlier());
    let rows: Vec<RowItem> = display
        .iter()
        .map(|row| match row {
            DisplayRow::Recovery(group) => RowItem {
                recovery: true,
                note: recovery_text(labels, group).into(),
                ..RowItem::default()
            },
            DisplayRow::Event { event, folded } => {
                let raw_type = code(&event.event_type);
                RowItem {
                    recovery: false,
                    folded: *folded,
                    sequence_text: event.sequence.to_string().into(),
                    clock: format_clock(event.timestamp_unix_ms).into(),
                    level: labels.levels[severity_index(event.severity)].into(),
                    level_tone: severity_tone(event.severity),
                    module: module_text(labels, event.origin.module()).into(),
                    event_name: event_text(labels, &raw_type).into(),
                    event_raw: raw_type.into(),
                    // The port the map gives the linked instance; the id itself
                    // when the map gives none — never a port the ledger did not state.
                    port: match event.links.instance_id() {
                        None => labels.host.to_string(),
                        Some(id) => match state.port_bindings().and_then(|map| map.port_of.get(id)) {
                            Some(port) => port.to_string(),
                            None => short_id(&code(id)),
                        },
                    }
                    .into(),
                    note: if *folded { labels.recovered_badge } else { "" }.into(),
                    link_id: links_named(&event.links)
                        .first()
                        .map(|(_, value)| short_id(value))
                        .unwrap_or_default()
                        .into(),
                }
            }
        })
        .collect();
    window.set_rows(models(rows));
    window.set_selected_index(
        display
            .iter()
            .position(|row| match row {
                DisplayRow::Event { event, .. } => {
                    Some(event.sequence) == model.selected_sequence
                }
                DisplayRow::Recovery(_) => false,
            })
            .map(|index| index as i32)
            .unwrap_or(-1),
    );

    match model.detail() {
        Some(detail) => {
            window.set_detail_lines(models(detail_lines(labels, detail.event)));
            window.set_artifacts(models(
                detail
                    .event
                    .artifacts
                    .iter()
                    .map(|artifact| {
                        let kind = code(&artifact.kind);
                        let media = code(&artifact.media_type);
                        ArtifactItem {
                            artifact_id: code(&artifact.artifact_id).into(),
                            kind: format!(
                                "{} · {} · {}",
                                labels.artifact_kind(&kind),
                                labels.media_type(&media),
                                format_bytes(artifact.byte_count)
                            )
                            .into(),
                            detail: format!("{kind} · {media} · {}", artifact.byte_count).into(),
                            sha256_label: labels.checksum.into(),
                            sha256: artifact.sha256.as_str().into(),
                        }
                    })
                    .collect::<Vec<_>>(),
            ));
            // The frame the event was taken on or acted on, with what the
            // ledger says about it; an event of no frame keeps its own capture,
            // if it has one.
            let mut frame = frame_view(state, model);
            let target = match frame.as_mut() {
                Some(frame) => frame.group.target.take(),
                None => model.frame_target(),
            };
            let none_note = match &frame {
                Some(FrameView { pending: true, .. }) => labels.frame_events_reading.to_string(),
                Some(FrameView { failure: Some(failure), .. }) => {
                    fill(labels.frame_events_failed, &[&failure.text(labels)])
                }
                Some(_) => labels.frame_capture_missing.to_string(),
                None if model.selected().is_some_and(is_input) => {
                    labels.frame_basis_missing.to_string()
                }
                None => labels.frame_no_artifact.to_string(),
            };
            let sequence = target.as_ref().map(|target| target.event.sequence);
            let stated_size =
                detail.frame_size.or(frame.as_ref().and_then(|frame| frame.group.extent));
            update_frame(
                window,
                ledger,
                state,
                labels,
                model.snapshot_position(),
                target,
                stated_size,
                none_note,
            );
            // The size the ledger states, else what the verified frame decoded
            // to, else the overlays' own extent.
            let frame_size = stated_size
                .or_else(|| sequence.and_then(|sequence| ledger.frame.decoded_size(sequence)));
            // The step's own marks replace the event's `action` geometry, which
            // would draw the same input twice.
            let marks = frame.as_ref().map_or(&[][..], |frame| frame.group.marks.as_slice());
            let (canvas_width, canvas_height) = canvas_size(&detail.overlays, marks, frame_size);
            window.set_canvas_width(canvas_width);
            window.set_canvas_height(canvas_height);
            window.set_frame_size_text(frame_size_text(labels, frame_size).into());
            let mut overlays: Vec<OverlayItem> = detail
                .overlays
                .iter()
                .filter(|overlay| marks.is_empty() || !overlay.kind.starts_with("action"))
                .map(|overlay| OverlayItem {
                    kind: overlay.kind.as_str().into(),
                    x: overlay.x,
                    y: overlay.y,
                    width: overlay.width,
                    height: overlay.height,
                    point: overlay.is_point(),
                    mark: false,
                })
                .collect();
            overlays.extend(marks.iter().flat_map(|mark| {
                mark.points.iter().map(|&(x, y)| OverlayItem {
                    kind: mark.kind.as_str().into(),
                    x,
                    y,
                    width: 0.0,
                    height: 0.0,
                    point: true,
                    mark: true,
                })
            }));
            window.set_overlays(models(overlays));
            let boxes = frame.as_ref().map_or(&[][..], |frame| frame.group.boxes.as_slice());
            window.set_targets(models(
                boxes
                    .iter()
                    .map(|target| TargetItem {
                        label: format!("{} · {}", target.target_id, target.role).into(),
                        x: target.x,
                        y: target.y,
                        width: target.width,
                        height: target.height,
                        passed: target.passed,
                    })
                    .collect::<Vec<_>>(),
            ));
            window.set_frame_page(
                frame.as_ref().map(|frame| page_text(labels, &frame.group)).unwrap_or_default().into(),
            );
            window.set_frame_explain(
                frame.as_ref().map(|frame| explain_text(labels, frame)).unwrap_or_default().into(),
            );
            window.set_payload_json(detail.pretty_payload_json.into());
        }
        None => clear_detail(window, ledger, labels, labels.no_event_selected.to_string()),
    }
}

/// The detail pane with no event: no lines but "not selected", no material,
/// no geometry, and the frame pane saying `note`.
fn clear_detail(window: &LedgerWindow, ledger: &LedgerHandle, labels: &Labels, note: String) {
    window.set_detail_lines(models(vec![FieldLine {
        label: labels.not_selected.into(),
        value: SharedString::new(),
        raw: SharedString::new(),
        wrap: false,
    }]));
    window.set_artifacts(models(Vec::<ArtifactItem>::new()));
    window.set_overlays(models(Vec::<OverlayItem>::new()));
    window.set_targets(models(Vec::<TargetItem>::new()));
    window.set_frame_page(SharedString::new());
    window.set_frame_explain(SharedString::new());
    window.set_canvas_width(1.0);
    window.set_canvas_height(1.0);
    window.set_frame_size_text(SharedString::new());
    window.set_payload_json(SharedString::new());
    clear_frame(window, &ledger.frame, note);
}

/// The window over a read face that is not open — still opening, unopened, or
/// failed: nothing is shown as read, so the card has the read-face line only,
/// and the list, the boxes and the frame pane say why instead of standing
/// empty, which would read as an empty ledger. The controls that would query
/// it are off. Whatever an earlier session of the window painted goes.
fn paint_closed(
    window: &LedgerWindow,
    ledger: &LedgerHandle,
    state: &LedgerState,
    labels: &Labels,
) {
    let (top, note, failed) = match &state.phase {
        Phase::Opening { .. } | Phase::Open(_) => {
            (labels.ledger_opening, labels.list_opening.to_owned(), false)
        }
        Phase::Unopened { .. } => (labels.ledger_unopened, labels.list_unopened.to_owned(), true),
        Phase::Failed { failure, .. } => (labels.ledger_unopened, failure_text(labels, failure), true),
    };
    let (value, raw) = match &state.phase {
        Phase::Unopened { .. } => (source_text(labels, state), "offline"),
        Phase::Failed { failure, .. } => (failure_text(labels, failure), ""),
        Phase::Opening { .. } | Phase::Open(_) => (labels.ledger_opening.to_owned(), ""),
    };
    window.set_ledger_dir_value(state.state_root().display().to_string().into());
    window.set_storage_format_value(labels.none.into());
    window.set_read_up_to_text(top.into());
    window.set_loaded_rows_text(SharedString::new());
    window.set_follow_available(false);
    window.set_cursor_label(labels.all.into());
    // The card's only line, and so the one that may wrap.
    window.set_card_lines(models(vec![FieldLine {
        label: labels.field("source").into(),
        value: value.into(),
        raw: raw.into(),
        wrap: true,
    }]));
    sync_tab_labels(window, labels.tabs.iter().map(|name| SharedString::from(*name)).collect());
    window.set_active_tab(-1);
    window.set_module_options(models(vec![SharedString::from(top)]));
    window.set_module_index(0);
    sync_ports(window, state, labels, None);
    window.set_filter_error(SharedString::new());
    window.set_has_more(false);
    window.set_rows(models(Vec::<RowItem>::new()));
    window.set_selected_index(-1);
    window.set_unopened_note(note.as_str().into());
    window.set_unopened_failed(failed);
    clear_detail(window, ledger, labels, note);
}

/// Why a session reads nothing more.
fn failure_text(labels: &Labels, failure: &LedgerFailure) -> String {
    match failure {
        LedgerFailure::Open { text, code } => {
            let reason = match (*code).and_then(|code| outcome_sentence(labels, code)) {
                Some(words) => format!("{text} · {words}"),
                None => text.clone(),
            };
            fill(labels.ledger_open_failed, &[&reason])
        }
        LedgerFailure::Panicked(message) => {
            fill(labels.ledger_worker_panicked, &[message.as_deref().unwrap_or(labels.none)])
        }
        LedgerFailure::NoThread(error) => fill(labels.thread_failed, &[error]),
        LedgerFailure::Gone => labels.ledger_worker_gone.to_owned(),
    }
}

/// The reopen offer alone: painted with the rest of the window, and again with
/// the launcher's lines (`LedgerHandle::paint_reopen`), since a Runtime the
/// launcher finds up is worth a new session to a window that reads offline
/// only for want of one.
pub fn paint_reopen(
    window: &LedgerWindow,
    state: &LedgerState,
    labels: &Labels,
    runtime_ready: bool,
) {
    let (offered, note, failed) = reopen_offer(labels, state, runtime_ready);
    window.set_reopen_offered(offered);
    window.set_reopen_note(note.into());
    window.set_reopen_failed(failed);
}

/// Whether the window offers a new session, why when the rest of the window
/// does not already say it, and whether that why is a failure: a face that did
/// not open, a reader that stopped, a read that reached a Runtime other than
/// the session's, or a Runtime that is up now while the window reads offline
/// because `auto` found none. A window `--source offline` asked for reads
/// offline as asked, Runtime or not.
fn reopen_offer(
    labels: &Labels,
    state: &LedgerState,
    runtime_ready: bool,
) -> (bool, String, bool) {
    let online_now = runtime_ready
        && matches!(
            state.offline_reason(),
            Some(OfflineReason::RuntimeInfoAbsent | OfflineReason::ConnectFailed { .. })
        );
    let started = || (true, labels.ledger_runtime_started.to_owned(), false);
    match &state.phase {
        Phase::Opening { .. } => (false, String::new(), false),
        Phase::Unopened { .. } if online_now => started(),
        Phase::Unopened { .. } | Phase::Failed { .. } => (true, String::new(), false),
        Phase::Open(facts) if owner_changed(state, facts) => {
            (true, labels.ledger_owner_changed.to_owned(), true)
        }
        Phase::Open(_) if online_now => started(),
        Phase::Open(_) => (false, String::new(), false),
    }
}

/// Whether a read of this session, or its last status read, reached a Runtime
/// of another owner epoch: every later read through that connection fails the
/// same way, and only a new session reads the Runtime that runs now.
fn owner_changed(state: &LedgerState, facts: &SourceFacts) -> bool {
    let changed = |error: &Option<QueryError>| {
        matches!(
            error,
            Some(QueryError::ReadFailed { code: Some(found), .. })
                if *found == CONSOLE_RUNTIME_OWNER_CHANGED
        )
    };
    let read = state.model.as_ref().is_some_and(|model| {
        changed(&model.query_error) || changed(&model.follow_error) || changed(&model.span_error)
    });
    let status = matches!(
        &facts.instances,
        InstancesSnapshot::Online(Err(failure)) if failure.code == CONSOLE_RUNTIME_OWNER_CHANGED
    );
    read || status
}

/// The busy line: what the window's reader is reading, how long it has
/// waited, and about how long a read of that kind may take; empty while it
/// waits for a request, and for a request's first `BUSY_QUIET`, so a quick one
/// never flashes a line that pushes the window's rows down and back.
pub fn busy_text(labels: &Labels, state: &LedgerState) -> String {
    let Some(busy) = state.busy.as_ref().filter(|busy| busy.started.elapsed() >= BUSY_QUIET)
    else {
        return String::new();
    };
    let what = match busy.kind {
        BusyKind::Opening => labels.ledger_opening.to_owned(),
        BusyKind::Bindings => labels.busy_bindings.to_owned(),
        BusyKind::Span => labels.busy_span.to_owned(),
        BusyKind::Probing => labels.busy_probing.to_owned(),
        BusyKind::Reading { read, of } => {
            fill(labels.busy_reading, &[&read.to_string(), &of.to_string()])
        }
        BusyKind::Repinning => labels.busy_repinning.to_owned(),
        BusyKind::Polling => labels.busy_polling.to_owned(),
        BusyKind::Instances => labels.busy_instances.to_owned(),
        BusyKind::FrameEvents => labels.frame_events_reading.to_owned(),
        BusyKind::Reconnecting => labels.busy_reconnecting.to_owned(),
    };
    let template = if busy.per_page { labels.busy_waited_page } else { labels.busy_waited };
    let waited = busy.since.elapsed().as_secs().to_string();
    fill(template, &[&what, &waited, &busy.bound.to_string()])
}

/// Rebuilds the module option list from the loaded rows and puts the box back
/// on the module it is filtering by, found by name in the list that now exists.
fn sync_modules(
    window: &LedgerWindow,
    ledger: &LedgerHandle,
    labels: &Labels,
    picked: &Option<OriginModule>,
    mut choices: Vec<OriginModule>,
) {
    // A picked module stays on the list even when the loaded rows show none of it,
    // so the box never displays a module the console is not filtering by.
    if let Some(module) = picked {
        if !choices.contains(module) {
            choices.push(*module);
            choices.sort();
        }
    }
    let mut options = vec![SharedString::from(labels.all_modules)];
    options.extend(
        choices
            .iter()
            .map(|module| SharedString::from(module_text(labels, *module))),
    );
    let index = picked
        .and_then(|module| choices.iter().position(|option| *option == module))
        .map(|index| index as i32 + 1)
        .unwrap_or(0);
    *ledger.module_choices.borrow_mut() = choices;
    *ledger.module_options.borrow_mut() = options.clone();
    window.set_module_options(models(options));
    window.set_module_index(index);
}

/// Puts the port box back on the port the console is filtering by, found in
/// the list the session's fixed map gives in the language shown; the first
/// item when there is none.
fn sync_ports(window: &LedgerWindow, state: &LedgerState, labels: &Labels, picked: Option<u16>) {
    let (options, choices) = port_options(labels, &state.port_map);
    let index = picked
        .and_then(|port| choices.iter().position(|choice| *choice == port))
        .map(|index| index as i32 + 1)
        .unwrap_or(0);
    window.set_port_options(models(options));
    window.set_port_enabled(!choices.is_empty());
    window.set_port_index(index);
}

fn detail_lines(labels: &Labels, event: &ProjectedEvent) -> Vec<FieldLine> {
    let raw_type = code(&event.event_type);
    let raw_source = code(&event.origin.source());
    let raw_module = event.origin.module().as_str();
    let mut lines = vec![
        field(labels, "sequence", event.sequence.to_string(), ""),
        field(labels, "event_id", code(&event.event_id), ""),
        field(labels, "occurred_at", format_full(event.timestamp_unix_ms), ""),
        field(labels, "event_type", event_text(labels, &raw_type), raw_type.clone()),
        field(
            labels,
            "severity",
            labels.levels[severity_index(event.severity)].to_string(),
            event.severity.as_str(),
        ),
        field(
            labels,
            "sensitivity",
            labels.sensitivities[sensitivity_index(event.sensitivity)].to_string(),
            code(&event.sensitivity),
        ),
        field(
            labels,
            "origin",
            format!(
                "{} · {} · {}",
                labels.source(&raw_source),
                module_text(labels, event.origin.module()),
                code(&event.origin.actor())
            ),
            format!("{raw_source} · {raw_module}"),
        ),
        field(labels, "views", views_text(labels, event), ""),
        field(labels, "payload_schema", event.payload_schema.clone(), ""),
    ];
    lines.extend(
        links_named(&event.links)
            .into_iter()
            .map(|(key, value)| field(labels, key, value, "")),
    );
    lines
}

/// The frame pane. A frame is read on demand for the selected event only, and
/// only after the ledger says the material was not evicted. The read runs on
/// the window's frame slot (`FrameSlot::read`), which hands back the bytes and
/// a `Phrase`; the pane's note is kept with the request, so a paint while the
/// read runs, or after it, says it again in the language shown then. A new
/// request takes a clone of the material reader the window's reader published;
/// a read that finds it latched, or unusable, asks the window's reader for a
/// new one, for the request after it.
fn update_frame(
    window: &LedgerWindow,
    ledger: &LedgerHandle,
    state: &LedgerState,
    labels: &Labels,
    snapshot: u64,
    target: Option<FrameTarget>,
    payload_frame_size: Option<(f32, f32)>,
    none_note: String,
) {
    let slot = &ledger.frame;
    let Some(target) = target else {
        clear_frame(window, slot, none_note);
        return;
    };
    if let Some(eviction) = &target.eviction {
        clear_frame(window, slot, eviction_text(labels, eviction));
        return;
    }
    if target.artifact.byte_count > MAX_FRAME_BYTES {
        clear_frame(
            window,
            slot,
            fill(
                labels.frame_too_large,
                &[&target.artifact.byte_count.to_string(), &MAX_FRAME_BYTES.to_string()],
            ),
        );
        return;
    }
    // Published together with the open face, so only a window still opening
    // has none; it paints no frame.
    let Some(reader) = state.reader.as_ref() else {
        clear_frame(window, slot, labels.ledger_opening.to_string());
        return;
    };
    let reading = {
        let bytes = target.artifact.byte_count.to_string();
        phrase(move |labels| fill(labels.frame_reading, &[&bytes]))
    };
    let generation = {
        let mut request = slot.request();
        if let Some(held) =
            request.as_ref().filter(|held| held.sequence == target.event.sequence)
        {
            window.set_frame_note((held.note)(labels).into());
            return;
        }
        let generation = slot.next_generation();
        *request = Some(FrameRequest {
            sequence: target.event.sequence,
            generation,
            decoded: None,
            note: Arc::clone(&reading),
        });
        generation
    };
    window.set_frame_ready(false);
    window.set_frame_note(reading(labels).into());
    let (id, session) = (ledger.id, state.tag.session);
    let rebuild = move || crate::ledger::refresh_reader(id, session);
    let started = slot.read(
        window,
        reader.clone(),
        target,
        snapshot,
        generation,
        payload_frame_size,
        rebuild,
    );
    // No read runs. The request is dropped, so choosing the event again retries.
    if let Err(error) = started {
        clear_frame(window, slot, fill(labels.thread_failed, &[&error.to_string()]));
    }
}

fn clear_frame(window: &LedgerWindow, slot: &FrameSlot, note: String) {
    slot.clear();
    window.set_frame_ready(false);
    window.set_frame_note(note.into());
}

/// A frame read come back for the request still current, painted in the
/// language shown now: the pane's note, and the frame once it is verified and
/// decoded.
pub fn frame_read(
    window: &LedgerWindow,
    ui: &Ui,
    note: Phrase,
    painted: Result<(u32, u32, Vec<u8>), Phrase>,
    payload_frame_size: Option<(f32, f32)>,
) {
    let labels = ui.labels();
    window.set_frame_note(note(labels).into());
    match painted {
        Ok((width, height, rgba)) => {
            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
            buffer.make_mut_bytes().copy_from_slice(&rgba);
            window.set_frame_image(Image::from_rgba8(buffer));
            window.set_frame_ready(true);
            // The payload did not state a size; the verified frame does.
            if payload_frame_size.is_none() {
                window.set_canvas_width(width as f32);
                window.set_canvas_height(height as f32);
                window.set_frame_size_text(
                    fill(labels.frame_size, &[&width.to_string(), &height.to_string()]).into(),
                );
            }
        }
        Err(_) => window.set_frame_ready(false),
    }
}

/// What the pane says of a verified frame: its decoded size.
pub fn verified_phrase(width: u32, height: u32) -> Phrase {
    let (width, height) = (width.to_string(), height.to_string());
    phrase(move |labels| fill(labels.frame_verified, &[&width, &height]))
}

/// What the pane says of a frame whose PNG does not decode: the decoder's words.
pub fn decode_failed(error: String) -> Phrase {
    phrase(move |labels| fill(labels.frame_decode_failed, &[&error]))
}

/// Why the read face gave no material, as the worker found it: the state and
/// limit codes verbatim, the eviction it observed, and its own failure text.
pub fn outcome_phrase(outcome: &MaterialOutcome) -> Phrase {
    let state = code(&outcome.state);
    let limit = outcome.limit.as_ref().map(code);
    let eviction = outcome.eviction.as_ref().map(Eviction::of);
    let failure = outcome.failure.clone();
    phrase(move |labels| {
        let mut text = fill(labels.frame_unavailable, &[&state]);
        if let Some(limit) = &limit {
            text.push_str(&format!(" · {limit}"));
        }
        if let Some(eviction) = &eviction {
            text.push_str(&format!(" · {}", eviction.text(labels)));
        }
        if let Some(failure) = &failure {
            text.push_str(&format!(" · {failure}"));
        }
        text
    })
}

fn eviction_text(labels: &Labels, eviction: &ArtifactEvictionObservation) -> String {
    Eviction::of(eviction).text(labels)
}

/// What an eviction observation says, as plain values a worker can hand back.
struct Eviction {
    disposition: Option<String>,
    intent: u64,
    terminal: Option<u64>,
    through: u64,
}

impl Eviction {
    fn of(eviction: &ArtifactEvictionObservation) -> Self {
        Self {
            disposition: eviction.disposition.as_ref().map(code),
            intent: eviction.intent.sequence,
            terminal: eviction.terminal_event.map(|terminal| terminal.sequence),
            through: eviction.through_sequence,
        }
    }

    fn text(&self, labels: &Labels) -> String {
        fill(
            labels.frame_evicted,
            &[
                self.disposition.as_deref().unwrap_or(labels.none),
                &self.intent.to_string(),
                &self.terminal.map(|terminal| format!(" → #{terminal}")).unwrap_or_default(),
                &self.through.to_string(),
            ],
        )
    }
}

fn recovery_text(labels: &Labels, group: &RecoveryGroup) -> String {
    let evidence = group
        .evidence
        .iter()
        .map(|(failure, success)| match success {
            Some(success) => fill(
                labels.failed_recovered,
                &[&failure.to_string(), &success.to_string()],
            ),
            None => fill(labels.failed_only, &[&failure.to_string()]),
        })
        .collect::<Vec<_>>()
        .join(labels.join);
    let mut text = format!(
        "{}{} · {} · {evidence}",
        labels.run_prefix,
        short_id(&group.run_id),
        labels.recovery_states[recovery_index(group.state)]
    );
    if !group.gaps.is_empty() {
        text.push_str(labels.gap_prefix);
        text.push_str(
            &group
                .gaps
                .iter()
                .map(|gap| labels.recovery_gap(&code(gap)).to_string())
                .collect::<Vec<_>>()
                .join(labels.join),
        );
    }
    text
}

fn integrity_text(labels: &Labels, card: &InstanceCard) -> String {
    match (&card.corrupt_tail, card.read_complete) {
        (None, true) if card.backend == "sqlite" => labels.integrity_ok_sqlite.to_string(),
        (None, true) => labels.integrity_ok.to_string(),
        (tail, complete) => fill(
            labels.integrity_bad,
            &[&complete.to_string(), tail.as_deref().unwrap_or(labels.none)],
        ),
    }
}

fn count_text(labels: &Labels, count: LedgerCount) -> String {
    match count {
        LedgerCount::Counted(count) => count.to_string(),
        LedgerCount::VerifiedPrefix(count) => {
            fill(labels.count_verified_prefix, &[&count.to_string()])
        }
        LedgerCount::NoRepairLog => labels.count_no_repair_log.to_string(),
        LedgerCount::NotStatedByRuntime => labels.count_not_stated.to_string(),
    }
}

fn writer_text(labels: &Labels, writer: &WriterFacts) -> String {
    match writer {
        WriterFacts::Absent => labels.writer_absent.to_string(),
        WriterFacts::Locked { byte_count } => {
            fill(labels.writer_locked, &[&byte_count.to_string()])
        }
        WriterFacts::Readable { owner_id, pid, active, started_at_unix_ms } => fill(
            if *active { labels.writer_running } else { labels.writer_stopped },
            &[owner_id, &pid.to_string(), &format_full(*started_at_unix_ms)],
        ),
        WriterFacts::Runtime { pid, owner_epoch, started_at_unix_ms } => fill(
            labels.writer_runtime,
            &[&pid.to_string(), owner_epoch, &format_full(*started_at_unix_ms)],
        ),
    }
}

fn source_text(labels: &Labels, state: &LedgerState) -> String {
    let mut text = match state.offline_reason() {
        None => labels.source_online.to_string(),
        Some(OfflineReason::Requested) => labels.source_offline_requested.to_string(),
        Some(OfflineReason::RuntimeInfoAbsent) => labels.source_offline_absent.to_string(),
        Some(OfflineReason::ConnectFailed { code, operation }) => {
            fill(labels.source_offline_failed, &[code, operation])
        }
    };
    // The ledger's own words, verbatim, and its io kind: only `not_found`
    // says the state root has no ledger yet.
    if let Phase::Unopened { failure, .. } = &state.phase {
        let detail = failure.detail.as_deref().unwrap_or(labels.none);
        text.push_str(" · ");
        text.push_str(&fill(labels.source_unopened, &[failure.code, failure.operation, detail]));
        if let Some(kind) = &failure.io_kind {
            text.push_str(&fill(labels.io_kind_text, &[kind]));
        }
        // A state root with no ledger yet is a first run, not a failure: it
        // gets the start hint instead of the error category's sentence.
        if failure.code == "ledger_io" && failure.io_kind.as_deref() == Some("not_found") {
            text.push_str(" · ");
            text.push_str(labels.source_unopened_hint);
        } else if let Some(words) = outcome_sentence(labels, failure.code) {
            text.push_str(" · ");
            text.push_str(&words);
        }
        if failure.code == "ledger_read_budget_exceeded" {
            text.push_str(" · ");
            text.push_str(labels.source_unopened_budget_hint);
        }
    }
    text
}

fn age_text(labels: &Labels, timestamp_unix_ms: u64) -> String {
    let seconds = seconds_since(timestamp_unix_ms).max(0);
    let (template, value) = if seconds >= 86_400 {
        (labels.age_days, seconds / 86_400)
    } else if seconds >= 3_600 {
        (labels.age_hours, seconds / 3_600)
    } else if seconds >= 60 {
        (labels.age_minutes, seconds / 60)
    } else {
        (labels.age_seconds, seconds)
    };
    fill(template, &[&value.to_string()])
}

fn frame_size_text(labels: &Labels, frame_size: Option<(f32, f32)>) -> String {
    match frame_size {
        Some((width, height)) => fill(
            labels.frame_size,
            &[&(width as u32).to_string(), &(height as u32).to_string()],
        ),
        None => labels.frame_size_missing.to_string(),
    }
}

/// A refused query names its code, the contract's or the console's own; the
/// code's words follow it.
fn query_error_text(labels: &Labels, error: Option<&QueryError>) -> String {
    match error {
        None => String::new(),
        Some(QueryError::IdNotCanonical) => labels.filter_id_error.to_string(),
        Some(QueryError::InstanceFilterConflict) => labels.filter_instance_conflict.to_string(),
        Some(QueryError::Rejected(reason)) => {
            let text = fill(labels.filter_rejected, &[reason]);
            match outcome_sentence(labels, reason) {
                Some(words) => format!("{text} · {words}"),
                None => text,
            }
        }
        // The read face's own words, and, when the error carries one of the
        // console's own codes, that code's words after them.
        Some(QueryError::ReadFailed { text, code }) => {
            let reason = match (*code).and_then(|code| outcome_sentence(labels, code)) {
                Some(words) => format!("{text} · {words}"),
                None => text.clone(),
            };
            fill(labels.read_failed, &[&reason])
        }
    }
}

/// A code on a card line, which does not wrap: the code, and its short words
/// when it has any. The resume line says a status read's failure the same way.
pub fn code_short(labels: &Labels, code: &str) -> String {
    match outcome_short(labels, code) {
        Some(short) => format!("{code} · {short}"),
        None => code.to_string(),
    }
}

fn backend_text(labels: &Labels, backend: &str) -> String {
    match backend {
        "segment" => labels.storage_segments.to_string(),
        "sqlite" => labels.storage_sqlite.to_string(),
        "runtime" => labels.storage_runtime.to_string(),
        other => other.to_string(),
    }
}

fn module_text(labels: &Labels, module: OriginModule) -> String {
    let raw = module.as_str();
    match module_names(raw) {
        Some((zh, en)) => pick(labels, zh, en).to_string(),
        None => raw.to_string(),
    }
}

fn event_text(labels: &Labels, raw: &str) -> String {
    match event_type_names(raw) {
        Some((zh, en)) => pick(labels, zh, en).to_string(),
        None => raw.to_string(),
    }
}

const fn pick(labels: &Labels, zh: &'static str, en: &'static str) -> &'static str {
    match labels.tongue {
        Language::Zh => zh,
        Language::En => en,
    }
}

fn views_text(labels: &Labels, event: &ProjectedEvent) -> String {
    event
        .views
        .iter()
        .map(|view| {
            ALL_TABS
                .iter()
                .position(|tab| tab == view)
                .map(|index| labels.tabs[index])
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn stamp(labels: &Labels, timestamp_unix_ms: Option<u64>) -> String {
    timestamp_unix_ms
        .map(format_full)
        .unwrap_or_else(|| labels.none.to_string())
}

const fn severity_index(severity: EventSeverity) -> usize {
    match severity {
        EventSeverity::Debug => 0,
        EventSeverity::Success => 1,
        EventSeverity::Info => 2,
        EventSeverity::Warning => 3,
        EventSeverity::Error => 4,
        EventSeverity::Fatal => 5,
    }
}

const fn severity_tone(severity: EventSeverity) -> i32 {
    match severity {
        EventSeverity::Debug => 0,
        EventSeverity::Success | EventSeverity::Info => 1,
        EventSeverity::Warning => 2,
        EventSeverity::Error | EventSeverity::Fatal => 3,
    }
}

const fn sensitivity_index(sensitivity: Sensitivity) -> usize {
    match sensitivity {
        Sensitivity::Public => 0,
        Sensitivity::Internal => 1,
        Sensitivity::Sensitive => 2,
        Sensitivity::Secret => 3,
    }
}

const fn recovery_index(state: acui_rows::LedgerRecoveryState) -> usize {
    match state {
        acui_rows::LedgerRecoveryState::Recovered => 0,
        acui_rows::LedgerRecoveryState::Unresolved => 1,
        acui_rows::LedgerRecoveryState::Unknown => 2,
    }
}

/// The frame pane's view of the selected event's frame: what the ledger says
/// about it, how it was found, whether the frame's own events are still being
/// read, and why they could not be read.
struct FrameView {
    group: FrameGroup,
    via: BasisVia,
    basis_sequence: u64,
    pending: bool,
    failure: Option<FrameLookupError>,
}

/// The selected event's frame, found among the loaded rows and, when they hold
/// no capture of it, among the frame's own events, which the window's reader
/// reads — one query by frame id, kept while the pane stays on that frame — so
/// an event still lands on its frame when the capture is filtered out or not
/// loaded.
fn frame_view(state: &LedgerState, model: &ViewModel) -> Option<FrameView> {
    let (basis, via) = model.selected_basis()?;
    let frame_id = *basis.links.frame_id()?;
    let frame = code(&frame_id);
    let basis_sequence = basis.sequence;
    let pending = state.frame_events.pending(&frame);
    let (extra, failure) = match state.frame_events.of(&frame) {
        Some(Ok(events)) => (events.as_slice(), None),
        Some(Err(error)) => (&[][..], Some(error.clone())),
        None => (&[][..], None),
    };
    let group = model.frame_group(&frame, extra);
    Some(FrameView { group, via, basis_sequence, pending, failure })
}

fn is_input(event: &ProjectedEvent) -> bool {
    code(&event.event_type).starts_with("input.")
}

/// The page label shown above the frame: the page the recognition matched, or
/// that it matched none; nothing before a recognition is read.
fn page_text(labels: &Labels, group: &FrameGroup) -> String {
    match &group.recognition {
        Some((Some(page), _)) => page.clone(),
        Some((None, _)) => labels.page_unmatched.to_string(),
        None => String::new(),
    }
}

/// One sentence per input meant on the frame — its step, what was recognized,
/// what was done where — then how the frame was found when the event does not
/// name it.
fn explain_text(labels: &Labels, frame: &FrameView) -> String {
    let recognition = frame.group.recognition.as_ref().map(|(page, candidates)| {
        let seen = match page {
            Some(page) => fill(labels.frame_matched, &[page]),
            None => fill(labels.frame_unmatched, &[&candidates.to_string()]),
        };
        match frame.group.targets {
            (_, 0) => seen,
            (passed, total) => {
                fill(labels.frame_targets, &[&seen, &passed.to_string(), &total.to_string()])
            }
        }
    });
    let mut parts: Vec<String> = frame
        .group
        .marks
        .iter()
        .map(|mark| {
            let step =
                fill(labels.frame_step, &[&mark.step_index.to_string(), &mark.operation_label]);
            match &recognition {
                Some(seen) => fill(labels.frame_explain_full, &[&step, seen, &mark_text(labels, mark)]),
                None => fill(labels.frame_explain_short, &[&step, &mark_text(labels, mark)]),
            }
        })
        .collect();
    if parts.is_empty() {
        parts.extend(recognition);
    }
    let sequence = frame.basis_sequence.to_string();
    match frame.via {
        BasisVia::Own => {}
        BasisVia::Step => parts.push(fill(labels.frame_via_step, &[&sequence])),
        BasisVia::Input => parts.push(fill(labels.frame_via_input, &[&sequence])),
    }
    parts.join(labels.join)
}

/// What an input did where: its name, and its points in order.
fn mark_text(labels: &Labels, mark: &Mark) -> String {
    let name = labels
        .action_kinds
        .iter()
        .find(|(kind, _)| mark.kind.starts_with(kind))
        .map_or(mark.kind.as_str(), |(_, name)| name);
    let points: Vec<String> =
        mark.points.iter().map(|(x, y)| format!("({}, {})", x.round(), y.round())).collect();
    if points.is_empty() {
        name.to_string()
    } else {
        format!("{name} {}", points.join(" → "))
    }
}

/// The coordinate space the overlays and the frame share: the frame's size,
/// else the extent of the event's overlays and the step's marks.
fn canvas_size(overlays: &[Overlay], marks: &[Mark], frame_size: Option<(f32, f32)>) -> (f32, f32) {
    match frame_size {
        Some(size) => size,
        None => {
            let points = marks.iter().flat_map(|mark| mark.points.iter().copied());
            let extents = overlays
                .iter()
                .map(|overlay| (overlay.x + overlay.width, overlay.y + overlay.height))
                .chain(points);
            let (width, height) = extents
                .fold((0.0_f32, 0.0_f32), |(width, height), (x, y)| (width.max(x), height.max(y)));
            let (width, height) = (width.max(1.0), height.max(1.0));
            (width * 1.1, height * 1.1)
        }
    }
}

/// The card's instance lines. Online they come from the status read and fact
/// snapshot as last read, at the positions the first two lines state: not state
/// at the pinned snapshot. Offline they are the facts the read face replays at the pinned
/// position itself; there is no status offline. One short value per line,
/// since a line in the middle of the card cannot wrap: a failure is split into
/// its parts.
fn instance_lines(labels: &Labels, source: &SourceFacts) -> Vec<FieldLine> {
    let online = match &source.instances {
        InstancesSnapshot::Offline(facts) => return offline_instance_lines(labels, facts),
        InstancesSnapshot::Online(online) => online,
    };
    let read = match online {
        Err(failure) => {
            let text = labels.instances_unread.to_string();
            let mut lines = vec![field(labels, "runtime_instances", text, "")];
            if let Some(runtime_code) = &failure.runtime_code {
                lines.push(field(labels, "runtime_refusal", code_short(labels, runtime_code), ""));
            }
            let code = code_short(labels, failure.code);
            lines.push(field(labels, "client_error", code, failure.operation));
            if let Some((host_code, operation)) = &failure.host {
                lines.push(field(labels, "host_failure", host_code.clone(), operation.as_str()));
            }
            return lines;
        }
        Ok(read) => read,
    };
    let mut lines = vec![
        field(labels, "instances_status_at", read.status_sequence.to_string(), ""),
        field(labels, "instances_facts_at", read.facts_position.to_string(), ""),
    ];
    if read.instances.is_empty() {
        lines.push(field(labels, "runtime_instances", labels.instances_none.to_string(), ""));
    }
    for instance in &read.instances {
        let id = instance.instance_id.as_str();
        match &instance.status {
            Some(live) => {
                lines.push(field(labels, "instance_alias", live.alias.clone(), id));
                let port =
                    live.adb_port.map_or_else(|| labels.none.to_string(), |port| port.to_string());
                lines.push(field(labels, "adb_port", port, ""));
                let lease = match (live.lease_active, live.takeover_cooldown_active) {
                    (true, _) => labels.lease_active,
                    (false, true) => labels.lease_cooldown,
                    (false, false) => labels.lease_idle,
                };
                let lease = match live.queued_request_count {
                    0 => lease.to_string(),
                    queued => fill(labels.lease_queued, &[lease, &queued.to_string()]),
                };
                lines.push(field(labels, "lease", lease, ""));
            }
            None => {
                let text = labels.not_registered.to_string();
                lines.push(field(labels, "instance_alias", text, id));
            }
        }
        lines.extend(task_fact_lines(labels, instance));
    }
    lines
}

/// The offline instance lines: the pinned position, that there is no lease
/// offline, then each instance's facts; or the read face's own reason or error.
fn offline_instance_lines(labels: &Labels, facts: &OfflineFacts) -> Vec<FieldLine> {
    match facts {
        OfflineFacts::NoEvents => {
            vec![field(labels, "instance_facts", labels.facts_no_events.to_string(), "0")]
        }
        OfflineFacts::Available { position, instances } => {
            let mut header = position.to_string();
            if instances.is_empty() {
                header.push_str(" · ");
                header.push_str(labels.facts_none);
            }
            let lease = labels.offline_not_provided.to_string();
            let mut lines = vec![
                field(labels, "instance_facts", header, "offline"),
                field(labels, "lease", lease, ""),
            ];
            for instance in instances {
                let id = instance.instance_id.as_str();
                lines.push(field(labels, "instance_id", short_id(id), id));
                lines.extend(task_fact_lines(labels, instance));
            }
            lines
        }
        OfflineFacts::NotAvailable { position, reason, .. } => {
            let text = fill(labels.facts_not_available, &[reason]);
            vec![field(labels, "instance_facts", text, position.to_string())]
        }
        OfflineFacts::Failed { position, code, operation, detail, io_kind } => {
            let unread = labels.instances_unread.to_string();
            let mut lines = vec![
                field(labels, "instance_facts", unread, position.to_string()),
                field(
                    labels,
                    "facts_error",
                    code_short(labels, code),
                    operation.as_deref().unwrap_or(labels.none),
                ),
            ];
            if let Some(kind) = io_kind {
                lines.push(field(labels, "io_kind", kind.clone(), ""));
            }
            lines.push(field(labels, "facts_detail", detail.clone(), ""));
            lines
        }
    }
}

/// An instance's three task facts, or "not recorded".
fn task_fact_lines(labels: &Labels, instance: &RuntimeInstance) -> [FieldLine; 3] {
    let fact = |value: &Option<String>| {
        value.clone().unwrap_or_else(|| labels.fact_unrecorded.to_string())
    };
    [
        field(labels, "task_game", fact(&instance.game), "task.game"),
        field(labels, "task_server", fact(&instance.server), "task.server"),
        field(labels, "task_page", fact(&instance.page), "task.page"),
    ]
}

fn field(
    labels: &Labels,
    key: &str,
    value: String,
    raw: impl Into<SharedString>,
) -> FieldLine {
    FieldLine {
        label: labels.field(key).into(),
        value: value.into(),
        raw: raw.into(),
        wrap: false,
    }
}

pub fn shared(items: &[&'static str]) -> ModelRc<SharedString> {
    models(items.iter().map(|item| SharedString::from(*item)).collect())
}

pub fn models<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// Puts the tab names into the list the window already shows, one row at a
/// time, so the tabs drawn from it stay — and with them the keyboard focus a
/// person left on one. A new list makes the window throw every tab away and
/// build them again, and the focus goes with the tab it sat on. Only when the
/// rows differ in number (the first paint) is the list itself replaced.
fn sync_tab_labels(window: &LedgerWindow, texts: Vec<SharedString>) {
    let current = window.get_tab_labels();
    match current.as_any().downcast_ref::<VecModel<SharedString>>() {
        Some(rows) if rows.row_count() == texts.len() => {
            for (index, text) in texts.into_iter().enumerate() {
                if rows.row_data(index).as_ref() != Some(&text) {
                    rows.set_row_data(index, text);
                }
            }
        }
        _ => window.set_tab_labels(models(texts)),
    }
}
