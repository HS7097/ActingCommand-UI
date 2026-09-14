// SPDX-License-Identifier: AGPL-3.0-only
//! ActingCommand 监控台 / ActingCommand Console: a read-only window over one
//! Runtime state root, opened through the formal ledger read face.

mod settings;
mod strings;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use acui_model::{
    tab_from_name, DisplayRow, FrameTarget, InstanceCard, Overlay, QueryError, RecoveryGroup,
    ViewModel, ALL_TABS,
};
use acui_rows::{
    code, event_type_names, format_bytes, format_clock, format_full, links_named, module_names,
    seconds_since, short_id, ArtifactEvictionObservation, EventQuery, EventSeverity, LedgerView,
    OriginModule, ProjectedEvent, Sensitivity, WriterFacts, MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use acui_source::{read_material, EvidenceSource, MaterialOutcome, MAX_FRAME_BYTES};
use anyhow::{bail, Result};
use settings::{Settings, TextSize};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use strings::{fill, Labels, Language};

slint::include_modules!();

#[derive(Default)]
struct Args {
    state_root: Option<PathBuf>,
    tab: Option<LedgerView>,
    /// `--lang` overrides the settings file for this run only.
    language: Option<Language>,
    help: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--state-root" => match argv.next() {
                Some(path) => args.state_root = Some(PathBuf::from(path)),
                None => bail!("--state-root 缺少路径 / --state-root needs a path"),
            },
            "--tab" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--tab 缺少取值 / --tab needs a value"),
                };
                args.tab = match tab_from_name(&name) {
                    Some(view) => Some(view),
                    None => bail!("未知的 --tab 取值 / unknown --tab value: {name}"),
                };
            }
            "--lang" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--lang 缺少取值 / --lang needs a value"),
                };
                args.language = match Language::from_wire(&name) {
                    Some(language) => Some(language),
                    None => bail!("未知的 --lang 取值 / unknown --lang value: {name}"),
                };
            }
            "--help" | "-h" => args.help = true,
            other => bail!("未知参数 / unknown argument: {other}"),
        }
    }
    Ok(args)
}

/// One open state root, its view model, the chosen language, and the frame read
/// in flight.
struct App {
    source: EvidenceSource,
    model: RefCell<ViewModel>,
    labels: &'static Labels,
    language: Language,
    text_size: Cell<TextSize>,
    /// The module option list as the box currently shows it, and what each
    /// entry after the first one means. Rebuilt from every page.
    module_options: RefCell<Vec<SharedString>>,
    module_choices: RefCell<Vec<OriginModule>>,
    /// Which event's frame the pane has already asked for.
    pending: Cell<Option<u64>>,
    /// Bumped per request so a slow read can never paint a stale frame, and so
    /// a superseded read can see that it has been superseded.
    generation: Arc<AtomicU64>,
    /// One material worker at a time.
    worker: Arc<Mutex<()>>,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let stored = settings::load();
    let language = args.language.unwrap_or(stored.language);
    let labels = language.labels();
    if args.help {
        println!("{}", labels.usage);
        return Ok(());
    }
    let Some(state_root) = args.state_root else {
        bail!("{}", labels.usage);
    };

    let source = EvidenceSource::open(&state_root)?;
    let span = time_span(&source);
    let mut model = ViewModel::new(
        source.open_report(),
        source.snapshot_position(),
        state_root.display().to_string(),
        span,
    );
    model.tab = args.tab.unwrap_or(LedgerView::Events);
    let app = Rc::new(App {
        source,
        model: RefCell::new(model),
        labels,
        language,
        text_size: Cell::new(stored.text_size),
        module_options: RefCell::new(Vec::new()),
        module_choices: RefCell::new(Vec::new()),
        pending: Cell::new(None),
        generation: Arc::new(AtomicU64::new(0)),
        worker: Arc::new(Mutex::new(())),
    });
    reload(&app, false);

    let window = AppWindow::new()?;
    install_strings(&window, labels);
    window.global::<Scale>().set_factor(stored.text_size.factor());
    window.set_text_size_index(stored.text_size.index());
    window.set_language_index(if language == Language::Zh { 0 } else { 1 });
    install_callbacks(&window, &app);
    refresh(&window, &app);
    window.run()?;
    Ok(())
}

/// Fills the `Strings` global once. There is no second call: the language
/// switch takes effect when the program is started again.
fn install_strings(window: &AppWindow, labels: &'static Labels) {
    let global = window.global::<Strings>();
    global.set_window_title(labels.window_title.into());
    global.set_data_source(labels.data_source.into());
    global.set_ledger_dir(labels.ledger_dir.into());
    global.set_storage_format(labels.storage_format.into());
    global.set_show_up_to(labels.show_up_to.into());
    global.set_text_size(labels.text_size.into());
    global.set_language(labels.language.into());
    global.set_restart_note(labels.restart_note.into());
    global.set_text_size_options(shared(&labels.text_sizes));
    global.set_language_options(shared(&labels.languages));
    global.set_severity_min_options(shared(&labels.severity_min));
    global.set_severity_max_options(shared(&labels.severity_max));
    global.set_id_placeholder(labels.id_placeholder.into());
    global.set_continue_reading(labels.continue_reading.into());
    global.set_card_title(labels.card_title.into());
    global.set_card_note(labels.card_note.into());
    global.set_detail_title(labels.detail_title.into());
    global.set_artifacts_title(labels.artifacts_title.into());
    global.set_overlays_title(labels.overlays_title.into());
    global.set_raw_data_title(labels.raw_data_title.into());
    global.set_col_seq(labels.columns[0].into());
    global.set_col_time(labels.columns[1].into());
    global.set_col_level(labels.columns[2].into());
    global.set_col_module(labels.columns[3].into());
    global.set_col_event(labels.columns[4].into());
    global.set_col_link(labels.columns[5].into());
}

/// The committed time span, read as the first and the last event of the snapshot.
fn time_span(source: &EvidenceSource) -> Option<(u64, u64)> {
    let events = EventQuery { view: Some(LedgerView::Events), ..EventQuery::default() };
    let first = source.query(&events, 1, None).ok()?;
    let last = source
        .query(
            &EventQuery { from_sequence: Some(source.snapshot_position()), ..events },
            1,
            None,
        )
        .ok()?;
    Some((
        first.events().first()?.timestamp_unix_ms,
        last.events().first()?.timestamp_unix_ms,
    ))
}

/// Re-runs the current filters as one ledger query at the pinned snapshot.
fn reload(app: &Rc<App>, append: bool) {
    let mut model = app.model.borrow_mut();
    let cursor = if append { model.cursor() } else { None };
    match model.query() {
        Err(error) => model.query_error = Some(error),
        Ok(query) => match app.source.query(&query, MAX_RUNTIME_EVENT_QUERY_EVENTS, cursor) {
            Ok(page) => {
                model.query_error = None;
                model.apply(page, append);
            }
            Err(error) => model.query_error = Some(QueryError::ReadFailed(error.to_string())),
        },
    }
    if !append {
        app.pending.set(None);
    }
}

fn install_callbacks(window: &AppWindow, app: &Rc<App>) {
    macro_rules! on {
        ($setter:ident, |$app:ident, $value:ident| $body:expr) => {{
            let weak = window.as_weak();
            let $app = Rc::clone(app);
            window.$setter(move |$value| {
                $body;
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &$app);
                }
            });
        }};
    }

    on!(on_tab_changed, |app, index| {
        app.model.borrow_mut().tab = ALL_TABS[index.clamp(0, ALL_TABS.len() as i32 - 1) as usize];
        reload(&app, false)
    });
    on!(on_minimum_severity_changed, |app, index| {
        app.model.borrow_mut().filters.minimum_severity = match index {
            1 => Some(EventSeverity::Warning),
            2 => Some(EventSeverity::Error),
            3 => Some(EventSeverity::Fatal),
            _ => None,
        };
        reload(&app, false)
    });
    on!(on_maximum_severity_changed, |app, index| {
        app.model.borrow_mut().filters.maximum_severity = match index {
            1 => Some(EventSeverity::Info),
            2 => Some(EventSeverity::Warning),
            3 => Some(EventSeverity::Error),
            _ => None,
        };
        reload(&app, false)
    });
    // By name against the list as it stands now, never by a position taken from
    // a list that the next page may have rebuilt.
    on!(on_module_changed, |app, name| {
        let module = app
            .module_options
            .borrow()
            .iter()
            .position(|option| *option == name)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| app.module_choices.borrow().get(index).copied());
        app.model.borrow_mut().filters.origin_module = module;
        reload(&app, false)
    });
    on!(on_id_changed, |app, text| {
        app.model.borrow_mut().filters.id_text = text.to_string();
        reload(&app, false)
    });
    on!(on_cursor_changed, |app, value| {
        let bound = app.model.borrow().span().and_then(|(from, to)| {
            let ratio = (value as f64 / 1000.0).clamp(0.0, 1.0);
            (ratio < 1.0).then(|| from + ((to - from) as f64 * ratio) as u64)
        });
        app.model.borrow_mut().filters.to_timestamp_unix_ms = bound;
        reload(&app, false)
    });
    on!(on_row_clicked, |app, index| {
        let mut model = app.model.borrow_mut();
        let selected = match model.display_rows().get(index.max(0) as usize) {
            Some(DisplayRow::Event { event, .. }) => Some(event.sequence),
            _ => None,
        };
        if selected.is_some() {
            model.selected_sequence = selected;
        }
    });
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_load_more(move || {
            reload(&app, true);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &app);
            }
        });
    }
    {
        // Text size applies now, through the one scale property.
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_text_size_changed(move |index| {
            let size = TextSize::from_index(index);
            app.text_size.set(size);
            settings::save(Settings { language: app.language, text_size: size });
            if let Some(window) = weak.upgrade() {
                window.global::<Scale>().set_factor(size.factor());
            }
        });
    }
    {
        // Language is written now and read at the next start; the note beside
        // the box says so.
        let app = Rc::clone(app);
        window.on_language_changed(move |index| {
            let language = if index == 1 { Language::En } else { Language::Zh };
            settings::save(Settings { language, text_size: app.text_size.get() });
        });
    }
}

fn refresh(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    let model = app.model.borrow();
    let card = model.instance_card();

    window.set_ledger_dir_value(card.state_root.as_str().into());
    window.set_storage_format_value(backend_text(labels, &card.backend).into());
    let scope = model.read_scope();
    window.set_read_up_to_text(
        match scope.scanned_through_position {
            Some(position) => {
                let mut text = fill(labels.read_up_to, &[&position.to_string()]);
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
    window.set_loaded_rows_text(
        fill(labels.loaded_rows_top, &[&card.loaded_count.to_string()]).into(),
    );
    window.set_cursor_label(
        model
            .filters
            .to_timestamp_unix_ms
            .map(|bound| format!("≤ {}", format_full(bound)))
            .unwrap_or_else(|| labels.all.to_string())
            .into(),
    );

    let mut lines = vec![
        field(labels, "latest_sequence", card.latest_sequence.to_string(), "latest_sequence"),
        field(
            labels,
            "event_count",
            card.event_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| labels.event_count_missing.to_string()),
            "event_count",
        ),
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
        field(labels, "writer", writer_text(labels, &card.writer), ""),
    ];
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
    window.set_tab_labels(models(
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
            .collect::<Vec<_>>(),
    ));
    window.set_active_tab(active as i32);

    sync_modules(window, app, &model.filters.origin_module.clone(), card.modules);

    let display = model.display_rows();
    window.set_filter_error(query_error_text(labels, model.query_error.as_ref()).into());
    window.set_has_more(model.has_more());
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
            let (canvas_width, canvas_height) = canvas_size(&detail.overlays, detail.frame_size);
            window.set_canvas_width(canvas_width);
            window.set_canvas_height(canvas_height);
            window.set_frame_size_text(frame_size_text(labels, detail.frame_size).into());
            window.set_overlays(models(
                detail
                    .overlays
                    .iter()
                    .map(|overlay| OverlayItem {
                        kind: overlay.kind.as_str().into(),
                        x: overlay.x,
                        y: overlay.y,
                        width: overlay.width,
                        height: overlay.height,
                        point: overlay.is_point(),
                    })
                    .collect::<Vec<_>>(),
            ));
            window.set_payload_json(detail.pretty_payload_json.into());
            // The size the payload states, as the detail already resolved it.
            update_frame(window, app, model.frame_target(), detail.frame_size);
        }
        None => {
            window.set_detail_lines(models(vec![FieldLine {
                label: labels.not_selected.into(),
                value: SharedString::new(),
                raw: SharedString::new(),
                wrap: false,
            }]));
            window.set_artifacts(models(Vec::<ArtifactItem>::new()));
            window.set_overlays(models(Vec::<OverlayItem>::new()));
            window.set_canvas_width(1.0);
            window.set_canvas_height(1.0);
            window.set_frame_size_text(SharedString::new());
            window.set_payload_json(SharedString::new());
            clear_frame(window, app, labels.no_event_selected.to_string());
        }
    }
}

/// Rebuilds the module option list from this page and puts the box back on the
/// module it is filtering by, found by name in the list that now exists.
fn sync_modules(
    window: &AppWindow,
    app: &Rc<App>,
    picked: &Option<OriginModule>,
    mut choices: Vec<OriginModule>,
) {
    // A picked module stays on the list even when this page shows none of it,
    // so the box never displays a module the console is not filtering by.
    if let Some(module) = picked {
        if !choices.contains(module) {
            choices.push(*module);
            choices.sort();
        }
    }
    let mut options = vec![SharedString::from(app.labels.all_modules)];
    options.extend(
        choices
            .iter()
            .map(|module| SharedString::from(module_text(app.labels, *module))),
    );
    let index = picked
        .and_then(|module| choices.iter().position(|option| *option == module))
        .map(|index| index as i32 + 1)
        .unwrap_or(0);
    *app.module_choices.borrow_mut() = choices;
    *app.module_options.borrow_mut() = options.clone();
    window.set_module_options(models(options));
    window.set_module_index(index);
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
/// only after the ledger says the material was not evicted.
fn update_frame(
    window: &AppWindow,
    app: &Rc<App>,
    target: Option<FrameTarget>,
    payload_frame_size: Option<(f32, f32)>,
) {
    let labels = app.labels;
    let Some(target) = target else {
        clear_frame(window, app, labels.frame_no_artifact.to_string());
        return;
    };
    if let Some(eviction) = &target.eviction {
        clear_frame(window, app, eviction_text(labels, eviction));
        return;
    }
    if target.artifact.byte_count > MAX_FRAME_BYTES {
        clear_frame(
            window,
            app,
            fill(
                labels.frame_too_large,
                &[&target.artifact.byte_count.to_string(), &MAX_FRAME_BYTES.to_string()],
            ),
        );
        return;
    }
    if app.pending.get() == Some(target.event.sequence) {
        return;
    }
    app.pending.set(Some(target.event.sequence));
    window.set_frame_ready(false);
    window.set_frame_note(
        fill(labels.frame_reading, &[&target.artifact.byte_count.to_string()]).into(),
    );

    let generation = app.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let shared = Arc::clone(&app.generation);
    let worker = Arc::clone(&app.worker);
    let root = app.source.state_root().to_path_buf();
    let snapshot = app.source.snapshot_position();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        // One worker at a time. A superseded read stops at its next range and
        // lets the slot go; every range re-hashes the whole material, so
        // letting a read nobody waits for run on is the expensive mistake.
        let _slot = worker.lock().unwrap_or_else(|held| held.into_inner());
        let still_wanted = || shared.load(Ordering::SeqCst) == generation;
        if !still_wanted() {
            return;
        }
        let read = read_material(&root, target.event, &target.artifact, snapshot, &still_wanted);
        let painted = match read {
            Err(error) => Err(error.to_string()),
            Ok(None) => return,
            Ok(Some(outcome)) => match &outcome.bytes {
                Some(bytes) => decode_png(labels, bytes),
                None => Err(outcome_text(labels, &outcome)),
            },
        };
        let _ = slint::invoke_from_event_loop(move || {
            if shared.load(Ordering::SeqCst) != generation {
                return;
            }
            let Some(window) = weak.upgrade() else {
                return;
            };
            match painted {
                Ok((width, height, rgba)) => {
                    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
                    buffer.make_mut_bytes().copy_from_slice(&rgba);
                    window.set_frame_image(Image::from_rgba8(buffer));
                    window.set_frame_ready(true);
                    window.set_frame_note(
                        fill(labels.frame_verified, &[&width.to_string(), &height.to_string()])
                            .into(),
                    );
                    // The payload did not state a size; the verified frame does.
                    if payload_frame_size.is_none() {
                        window.set_canvas_width(width as f32);
                        window.set_canvas_height(height as f32);
                        window.set_frame_size_text(
                            fill(labels.frame_size, &[&width.to_string(), &height.to_string()])
                                .into(),
                        );
                    }
                }
                Err(text) => {
                    window.set_frame_ready(false);
                    window.set_frame_note(text.into());
                }
            }
        });
    });
}

fn clear_frame(window: &AppWindow, app: &Rc<App>, note: String) {
    app.pending.set(None);
    app.generation.fetch_add(1, Ordering::SeqCst);
    window.set_frame_ready(false);
    window.set_frame_note(note.into());
}

fn decode_png(labels: &Labels, bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| fill(labels.frame_decode_failed, &[&error.to_string()]))?
        .to_rgba8();
    Ok((decoded.width(), decoded.height(), decoded.into_raw()))
}

fn outcome_text(labels: &Labels, outcome: &MaterialOutcome) -> String {
    let mut text = fill(labels.frame_unavailable, &[&code(&outcome.state)]);
    if let Some(limit) = &outcome.limit {
        text.push_str(&format!(" · {}", code(limit)));
    }
    if let Some(eviction) = &outcome.eviction {
        text.push_str(&format!(" · {}", eviction_text(labels, eviction)));
    }
    if let Some(failure) = &outcome.failure {
        text.push_str(&format!(" · {failure}"));
    }
    text
}

fn eviction_text(labels: &Labels, eviction: &ArtifactEvictionObservation) -> String {
    fill(
        labels.frame_evicted,
        &[
            &eviction
                .disposition
                .as_ref()
                .map(code)
                .unwrap_or_else(|| labels.none.to_string()),
            &eviction.intent.sequence.to_string(),
            &eviction
                .outcome
                .map(|outcome| format!(" → #{}", outcome.sequence))
                .unwrap_or_default(),
            &eviction.through_sequence.to_string(),
        ],
    )
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
        (None, true) => match card.repair_count {
            Some(0) | None => labels.integrity_ok.to_string(),
            Some(count) => fill(labels.integrity_repairs, &[&count.to_string()]),
        },
        (tail, complete) => fill(
            labels.integrity_bad,
            &[&complete.to_string(), tail.as_deref().unwrap_or(labels.none)],
        ),
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
    }
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

fn query_error_text(labels: &Labels, error: Option<&QueryError>) -> String {
    match error {
        None => String::new(),
        Some(QueryError::IdNotCanonical) => labels.filter_id_error.to_string(),
        Some(QueryError::Rejected(reason)) => fill(labels.filter_rejected, &[reason]),
        Some(QueryError::ReadFailed(reason)) => fill(labels.read_failed, &[reason]),
    }
}

fn backend_text(labels: &Labels, backend: &str) -> String {
    match backend {
        "segment" => labels.storage_segments.to_string(),
        "sqlite" => labels.storage_sqlite.to_string(),
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
        EventSeverity::Info => 1,
        EventSeverity::Warning => 2,
        EventSeverity::Error => 3,
        EventSeverity::Fatal => 4,
    }
}

const fn severity_tone(severity: EventSeverity) -> i32 {
    match severity {
        EventSeverity::Debug => 0,
        EventSeverity::Info => 1,
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

/// The coordinate space the overlays and the frame share.
fn canvas_size(overlays: &[Overlay], frame_size: Option<(f32, f32)>) -> (f32, f32) {
    match frame_size {
        Some(size) => size,
        None => {
            let width = overlays
                .iter()
                .map(|overlay| overlay.x + overlay.width)
                .fold(0.0_f32, f32::max)
                .max(1.0);
            let height = overlays
                .iter()
                .map(|overlay| overlay.y + overlay.height)
                .fold(0.0_f32, f32::max)
                .max(1.0);
            (width * 1.1, height * 1.1)
        }
    }
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

fn shared(items: &[&'static str]) -> ModelRc<SharedString> {
    models(items.iter().map(|item| SharedString::from(*item)).collect())
}

fn models<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}
