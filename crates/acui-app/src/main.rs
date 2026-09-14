// SPDX-License-Identifier: AGPL-3.0-only
//! ActingCommand 监控台: a read-only window over one Runtime state root,
//! opened through the formal ledger read face.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use acui_model::{
    extract_frame_size, recovery_state_text, tab_from_name, tab_label, DisplayRow, FrameTarget,
    ViewModel, ALL_TABS,
};
use acui_rows::{
    code, format_clock, short_id, ArtifactEvictionObservation, EventQuery, EventSeverity,
    LedgerView, MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use acui_source::{read_material, EvidenceSource, MaterialOutcome};
use anyhow::{bail, Result};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

slint::include_modules!();

const USAGE: &str = "用法：acui --state-root <state_root> \
[--tab <events|observation|changes|errors|health|lab>]";

struct Args {
    state_root: PathBuf,
    tab: LedgerView,
}

/// `Ok(None)` means the usage line was printed and the process should stop.
fn parse_args() -> Result<Option<Args>> {
    let mut state_root = None;
    let mut tab = LedgerView::Events;
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--state-root" => match argv.next() {
                Some(path) => state_root = Some(PathBuf::from(path)),
                None => bail!("--state-root 缺少路径"),
            },
            "--tab" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("缺少 --tab 取值"),
                };
                tab = match tab_from_name(&name) {
                    Some(view) => view,
                    None => bail!("未知的 --tab 取值：{name}"),
                };
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => bail!("未知参数：{other}\n{USAGE}"),
        }
    }
    match state_root {
        Some(state_root) => Ok(Some(Args { state_root, tab })),
        None => bail!("缺少 --state-root\n{USAGE}"),
    }
}

/// One open state root, its view model, and the frame read in flight.
struct App {
    source: EvidenceSource,
    model: RefCell<ViewModel>,
    /// Which event's frame the pane has already asked for.
    pending: Cell<Option<u64>>,
    /// Bumped per request so a slow read can never paint a stale frame.
    generation: Arc<AtomicU64>,
}

fn main() -> Result<()> {
    let args = match parse_args()? {
        Some(args) => args,
        None => return Ok(()),
    };
    let source = EvidenceSource::open(&args.state_root)?;
    let label = args.state_root.display().to_string();
    let span = time_span(&source);
    let mut model = ViewModel::new(source.open_report(), source.snapshot_position(), label, span);
    model.tab = args.tab;
    let app = Rc::new(App {
        source,
        model: RefCell::new(model),
        pending: Cell::new(None),
        generation: Arc::new(AtomicU64::new(0)),
    });
    reload(&app, false);

    let window = AppWindow::new()?;
    window.set_module_options(module_options(&app));
    install_callbacks(&window, &app);
    refresh(&window, &app);
    window.run()?;
    Ok(())
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
        Err(text) => model.filter_error = Some(text),
        Ok(query) => match app.source.query(&query, MAX_RUNTIME_EVENT_QUERY_EVENTS, cursor) {
            Ok(page) => {
                model.filter_error = None;
                model.apply(page, append);
            }
            Err(error) => model.filter_error = Some(error.to_string()),
        },
    }
    if !append {
        app.pending.set(None);
    }
}

fn module_options(app: &Rc<App>) -> ModelRc<SharedString> {
    let mut options = vec![SharedString::from("全部模块")];
    options.extend(
        app.model
            .borrow()
            .modules()
            .into_iter()
            .map(|module| SharedString::from(module.as_str())),
    );
    ModelRc::new(VecModel::from(options))
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
    on!(on_module_changed, |app, index| {
        let module = (index > 0)
            .then(|| app.model.borrow().modules().get(index as usize - 1).copied())
            .flatten();
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
}

fn refresh(window: &AppWindow, app: &Rc<App>) {
    let model = app.model.borrow();
    let card = model.instance_card();
    window.set_mode_text("模式：离线·读面".into());
    window.set_source_label(format!("状态根：{}", card.source_label).into());
    window.set_counter_text(
        format!(
            "{} · 快照 #{} · {}",
            card.backend,
            card.snapshot_position,
            model.scope_text()
        )
        .into(),
    );
    window.set_cursor_label(
        model
            .filters
            .to_timestamp_unix_ms
            .map(|bound| format!("≤ {}", acui_rows::format_full(bound)))
            .unwrap_or_else(|| "全部".to_string())
            .into(),
    );

    let mut lines = vec![
        line("存储后端", card.backend.clone()),
        line("快照位置", format!("#{}", card.snapshot_position)),
        line("latest_sequence", card.latest_sequence.to_string()),
        line(
            "event_count",
            card.event_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "—（读面未给，见 README）".to_string()),
        ),
        line("账本完整性", card.integrity.clone()),
        line("写入方", card.writer.clone()),
        line("本视图已载入", card.loaded_count.to_string()),
        line("最早（已载入）", card.first_timestamp.unwrap_or_else(dash)),
        line("最新（已载入）", card.last_timestamp.unwrap_or_else(dash)),
    ];
    for (severity, count) in &card.severity_counts {
        lines.push(line(severity.as_str(), count.to_string()));
    }
    lines.push(line(
        "模块",
        card.modules
            .iter()
            .map(|module| module.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    window.set_card_lines(models(lines));

    let counts = model.membership_counts();
    window.set_tab_labels(models(
        ALL_TABS
            .iter()
            .enumerate()
            .map(|(index, tab)| SharedString::from(format!("{} {}", tab_label(*tab), counts[index])))
            .collect::<Vec<_>>(),
    ));
    window.set_active_tab(
        ALL_TABS
            .iter()
            .position(|tab| *tab == model.tab)
            .unwrap_or(0) as i32,
    );

    let display = model.display_rows();
    window.set_list_summary(format!("本页 {} 行", display.len()).into());
    window.set_filter_error(model.filter_error.clone().unwrap_or_default().into());
    window.set_has_more(model.has_more());
    let rows: Vec<RowItem> = display
        .iter()
        .map(|row| match row {
            DisplayRow::Recovery(group) => RowItem {
                recovery: true,
                note: format!(
                    "运行 {} · {} · {}{}",
                    short_id(&group.run_id),
                    recovery_state_text(group.state),
                    group.basis,
                    if group.gaps.is_empty() {
                        String::new()
                    } else {
                        format!(" · 缺口 {}", group.gaps)
                    }
                )
                .into(),
                ..RowItem::default()
            },
            DisplayRow::Event { event, folded } => RowItem {
                recovery: false,
                folded: *folded,
                sequence_text: event.sequence.to_string().into(),
                clock: format_clock(event.timestamp_unix_ms).into(),
                severity: event.severity.as_str().into(),
                module: event.origin.module().as_str().into(),
                event_type: code(&event.event_type).into(),
                note: if *folded { "已恢复" } else { "" }.into(),
                link_id: acui_rows::links_named(&event.links)
                    .first()
                    .map(|(_, value)| short_id(value))
                    .unwrap_or_default()
                    .into(),
            },
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
            window.set_detail_lines(models(
                detail
                    .lines
                    .iter()
                    .map(|(label, value)| line(label, value.clone()))
                    .collect::<Vec<_>>(),
            ));
            window.set_artifacts(models(
                detail
                    .event
                    .artifacts
                    .iter()
                    .map(|artifact| ArtifactItem {
                        artifact_id: code(&artifact.artifact_id).into(),
                        kind: code(&artifact.kind).into(),
                        media_type: code(&artifact.media_type).into(),
                        byte_count: format!("{} 字节", artifact.byte_count).into(),
                        sha256: artifact.sha256.as_str().into(),
                    })
                    .collect::<Vec<_>>(),
            ));
            let (canvas_width, canvas_height) = canvas_size(&detail.overlays, detail.frame_size);
            window.set_canvas_width(canvas_width);
            window.set_canvas_height(canvas_height);
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
            let frame_size = extract_frame_size(&acui_rows::payload_value(detail.event));
            update_frame(window, app, model.frame_target(), frame_size);
        }
        None => {
            window.set_detail_lines(models(vec![line("提示", "未选中事件".to_string())]));
            window.set_artifacts(models(Vec::<ArtifactItem>::new()));
            window.set_overlays(models(Vec::<OverlayItem>::new()));
            window.set_canvas_width(1.0);
            window.set_canvas_height(1.0);
            window.set_payload_json("".into());
            update_frame(window, app, None, None);
        }
    }
}

/// The frame pane. A frame is read on demand for the selected event only, and
/// only after the ledger says the material was not evicted.
fn update_frame(
    window: &AppWindow,
    app: &Rc<App>,
    target: Option<FrameTarget>,
    payload_frame_size: Option<(f32, f32)>,
) {
    let Some(target) = target else {
        clear_frame(window, app, "无 capture.frame 产物，仅画几何叠加");
        return;
    };
    if let Some(eviction) = &target.eviction {
        clear_frame(window, app, &eviction_text(eviction));
        return;
    }
    if app.pending.get() == Some(target.event.sequence) {
        return;
    }
    app.pending.set(Some(target.event.sequence));
    window.set_frame_ready(false);
    window.set_frame_note(
        format!("正在分段读取并全量校验 {} 字节…", target.artifact.byte_count).into(),
    );

    let generation = app.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let shared = Arc::clone(&app.generation);
    let root = app.source.state_root().to_path_buf();
    let snapshot = app.source.snapshot_position();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let read = read_material(&root, target.event, &target.artifact, snapshot);
        let painted = match read {
            Err(error) => Err(error.to_string()),
            Ok(outcome) => match &outcome.bytes {
                Some(bytes) => decode_png(bytes),
                None => Err(outcome_text(&outcome)),
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
                        format!("已校验帧 {width}×{height} · 分段读取，整份 sha256 校验通过").into(),
                    );
                    if payload_frame_size.is_none() {
                        window.set_canvas_width(width as f32);
                        window.set_canvas_height(height as f32);
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

fn clear_frame(window: &AppWindow, app: &Rc<App>, note: &str) {
    app.pending.set(None);
    app.generation.fetch_add(1, Ordering::SeqCst);
    window.set_frame_ready(false);
    window.set_frame_note(note.into());
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| format!("PNG 解码失败：{error}"))?
        .to_rgba8();
    Ok((decoded.width(), decoded.height(), decoded.into_raw()))
}

fn outcome_text(outcome: &MaterialOutcome) -> String {
    let mut text = format!("素材未提供：{}", code(&outcome.state));
    if let Some(limit) = &outcome.limit {
        text.push_str(&format!(" · {}", code(limit)));
    }
    if let Some(eviction) = &outcome.eviction {
        text.push_str(&format!(" · {}", eviction_text(eviction)));
    }
    if let Some(failure) = &outcome.failure {
        text.push_str(&format!(" · {failure}"));
    }
    text
}

fn eviction_text(eviction: &ArtifactEvictionObservation) -> String {
    format!(
        "产物已淘汰（{}）：意图 #{}{} · 观察至 #{}",
        eviction
            .disposition
            .as_ref()
            .map(code)
            .unwrap_or_else(|| "待定".to_string()),
        eviction.intent.sequence,
        eviction
            .outcome
            .map(|outcome| format!(" → 结果 #{}", outcome.sequence))
            .unwrap_or_default(),
        eviction.through_sequence
    )
}

/// The coordinate space the overlays and the frame share.
fn canvas_size(overlays: &[acui_model::Overlay], frame_size: Option<(f32, f32)>) -> (f32, f32) {
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

fn line(label: &str, value: String) -> FieldLine {
    FieldLine { label: label.into(), value: value.into() }
}

fn dash() -> String {
    "—".to_string()
}

fn models<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}
