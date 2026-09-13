// SPDX-License-Identifier: AGPL-3.0-only
//! ActingCommand 监控台 v0-standalone: a read-only window over exported ledger pages.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use acui_model::{format_clock, format_full, ViewModel, ViewTab, ALL_TABS};
use acui_rows::Severity;
use acui_source::{EventSource, FileSource};
use anyhow::{bail, Result};
use slint::{ModelRc, SharedString, VecModel};

slint::include_modules!();

const USAGE: &str = "用法：acui [--events <events.json>]... [--open <open.json>] \
[--tab <stream|errors|observe|changes|health|lab>]";

struct Args {
    events: Vec<PathBuf>,
    open: Option<PathBuf>,
    tab: ViewTab,
}

/// `Ok(None)` means the usage line was printed and the process should stop.
fn parse_args() -> Result<Option<Args>> {
    let mut events = Vec::new();
    let mut open = None;
    let mut tab = ViewTab::EventStream;
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--events" => match argv.next() {
                Some(path) => events.push(PathBuf::from(path)),
                None => bail!("--events 缺少文件路径"),
            },
            "--open" => match argv.next() {
                Some(path) => open = Some(PathBuf::from(path)),
                None => bail!("--open 缺少文件路径"),
            },
            "--tab" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("缺少 --tab 取值"),
                };
                tab = match name.as_str() {
                    "stream" => ViewTab::EventStream,
                    "errors" => ViewTab::Errors,
                    "observe" => ViewTab::ObserveAndAct,
                    "changes" => ViewTab::Changes,
                    "health" => ViewTab::Health,
                    "lab" => ViewTab::Lab,
                    other => bail!("未知的 --tab 取值：{other}"),
                };
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => bail!("未知参数：{other}\n{USAGE}"),
        }
    }
    Ok(Some(Args { events, open, tab }))
}

fn main() -> Result<()> {
    let args = match parse_args()? {
        Some(args) => args,
        None => return Ok(()),
    };
    let source = if args.events.is_empty() {
        FileSource::empty()
    } else {
        FileSource::load(&args.events, args.open.as_deref())?
    };
    let label = source.source_label();
    let (rows, open) = source.into_parts();
    let mut model = ViewModel::new(rows, open, label);
    model.tab = args.tab;
    let model = Rc::new(RefCell::new(model));

    let window = AppWindow::new()?;
    window.set_tab_labels(ModelRc::new(VecModel::from(
        ALL_TABS
            .iter()
            .map(|tab| {
                SharedString::from(if tab.is_provisional() {
                    format!("{}（临）", tab.label())
                } else {
                    tab.label().to_string()
                })
            })
            .collect::<Vec<_>>(),
    )));
    let mut module_options = vec![SharedString::from("全部模块")];
    module_options.extend(
        model
            .borrow()
            .modules()
            .into_iter()
            .map(SharedString::from),
    );
    window.set_module_options(ModelRc::new(VecModel::from(module_options)));

    install_callbacks(&window, &model);
    refresh(&window, &model.borrow());
    window.run()?;
    Ok(())
}

fn install_callbacks(window: &AppWindow, model: &Rc<RefCell<ViewModel>>) {
    macro_rules! on {
        ($setter:ident, |$view:ident, $value:ident| $body:expr) => {{
            let weak = window.as_weak();
            let shared = Rc::clone(model);
            window.$setter(move |$value| {
                {
                    let mut $view = shared.borrow_mut();
                    $body;
                }
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &shared.borrow());
                }
            });
        }};
    }

    on!(on_tab_changed, |view, index| {
        view.tab = ALL_TABS[index.clamp(0, ALL_TABS.len() as i32 - 1) as usize]
    });
    on!(on_severity_changed, |view, index| {
        view.filters.min_severity = match index {
            1 => Some(Severity::Warning),
            2 => Some(Severity::Error),
            _ => None,
        }
    });
    on!(on_module_changed, |view, index| {
        let modules = view.modules();
        view.filters.module = if index <= 0 {
            None
        } else {
            modules.get(index as usize - 1).cloned()
        }
    });
    on!(on_id_changed, |view, text| {
        view.filters.id_text = if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    });
    // The slider carries the sequence as f32: exact only below 2^24, so past that
    // the cursor quantises to the nearest representable sequence. v0 accepts this.
    on!(on_cursor_changed, |view, value| {
        let max = view.max_sequence();
        let cursor = value.round().max(0.0) as u64;
        view.filters.through_sequence = if cursor >= max { None } else { Some(cursor) }
    });
    on!(on_row_clicked, |view, index| {
        let sequence = view
            .visible()
            .get(index.max(0) as usize)
            .map(|row| row.sequence);
        view.selected_sequence = sequence;
    });
}

fn refresh(window: &AppWindow, model: &ViewModel) {
    let card = model.instance_card();
    window.set_mode_text("模式：离线·文件".into());
    window.set_source_label(format!("来源：{}", card.source_label).into());
    // Ledger facts come from open.json only; the loaded count is shown separately.
    window.set_counter_text(
        format!(
            "账本 latest_sequence {} / event_count {}",
            optional(card.latest_sequence),
            optional(card.ledger_event_count)
        )
        .into(),
    );
    window.set_loaded_text(format!("事件条数（已载入）{}", card.loaded_count).into());

    let max_sequence = model.max_sequence();
    window.set_cursor_max(max_sequence as f32);
    window.set_cursor_value(model.filters.through_sequence.unwrap_or(max_sequence) as f32);

    let mut card_lines = vec![
        line("事件条数（已载入）", card.loaded_count.to_string()),
        line("最早事件", card.first_timestamp.unwrap_or_else(dash)),
        line("最新事件", card.last_timestamp.unwrap_or_else(dash)),
        line("最新事件距今", card.last_event_age_text),
    ];
    if let Some(integrity) = card.ledger_integrity {
        card_lines.push(line("账本完整性", integrity));
    }
    for (severity, count) in &card.severity_counts {
        card_lines.push(line(severity.as_str(), count.to_string()));
    }
    card_lines.push(line("模块", card.modules.join("\n")));
    if let Some(owner) = card.writer_owner_id {
        card_lines.push(line("写入方 owner_id", owner));
    }
    if let Some(pid) = card.writer_pid {
        card_lines.push(line("写入方 pid", pid.to_string()));
    }
    if let Some(active) = card.writer_active {
        card_lines.push(line("写入方在线", if active { "是" } else { "否" }.to_string()));
    }
    window.set_card_lines(models(card_lines));

    window.set_active_tab(
        ALL_TABS
            .iter()
            .position(|tab| *tab == model.tab)
            .unwrap_or(0) as i32,
    );
    window.set_provisional_note(
        if model.tab.is_provisional() {
            "临时归类，待行契约"
        } else {
            ""
        }
        .into(),
    );

    let visible = model.visible();
    window.set_list_summary(format!("本视图 {} 条", visible.len()).into());
    let rows: Vec<RowItem> = visible
        .iter()
        .map(|row| RowItem {
            sequence_text: row.sequence.to_string().into(),
            clock: format_clock(row.timestamp_unix_ms).into(),
            severity: row.severity.as_str().into(),
            module: row.origin.module.as_str().into(),
            event_type: row.event_type.as_str().into(),
            link_id: row.links.first().unwrap_or("").into(),
        })
        .collect();
    window.set_rows(models(rows));
    window.set_selected_index(
        visible
            .iter()
            .position(|row| Some(row.sequence) == model.selected_sequence)
            .map(|index| index as i32)
            .unwrap_or(-1),
    );

    match model.detail() {
        Some(detail) => {
            let row = detail.row;
            let mut lines = vec![
                line("sequence", row.sequence.to_string()),
                line("event_id", row.event_id.clone()),
                line("时间", format_full(row.timestamp_unix_ms)),
                line("event_type", row.event_type.clone()),
                line("severity", row.severity.as_str().to_string()),
                line("sensitivity", row.sensitivity.clone()),
                line(
                    "origin",
                    format!(
                        "{} / {} / {}",
                        row.origin.source, row.origin.module, row.origin.actor
                    ),
                ),
                line("schema", row.schema_version.clone()),
                line("payload_schema", row.payload_schema.clone()),
            ];
            for (name, value) in row.links.named() {
                lines.push(line(name, value.to_string()));
            }
            window.set_detail_lines(models(lines));

            let artifacts: Vec<ArtifactItem> = detail
                .artifacts
                .iter()
                .map(|artifact| ArtifactItem {
                    artifact_id: artifact.artifact_id.as_str().into(),
                    kind: artifact.kind.as_str().into(),
                    media_type: artifact.media_type.as_str().into(),
                    byte_count: format!("{} 字节", artifact.byte_count).into(),
                    sha256: artifact.sha256.as_str().into(),
                })
                .collect();
            window.set_artifacts(models(artifacts));

            let (canvas_width, canvas_height, size_note) = match detail.frame_size {
                Some((width, height)) => (width, height, format!("画面 {width:.0}×{height:.0}")),
                None => {
                    let width = detail
                        .overlays
                        .iter()
                        .map(|overlay| overlay.x + overlay.width)
                        .fold(0.0_f32, f32::max)
                        .max(1.0);
                    let height = detail
                        .overlays
                        .iter()
                        .map(|overlay| overlay.y + overlay.height)
                        .fold(0.0_f32, f32::max)
                        .max(1.0);
                    (width * 1.1, height * 1.1, "画面尺寸未知，按几何范围铺排".to_string())
                }
            };
            window.set_canvas_width(canvas_width);
            window.set_canvas_height(canvas_height);
            window.set_canvas_note(
                format!(
                    "几何叠加（不载入图像）：{} 个 · {}",
                    detail.overlays.len(),
                    size_note
                )
                .into(),
            );
            let overlays: Vec<OverlayItem> = detail
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
                .collect();
            window.set_overlays(models(overlays));
            window.set_payload_json(detail.pretty_payload_json.into());
        }
        None => {
            window.set_detail_lines(models(vec![line("提示", "未选中事件".to_string())]));
            window.set_artifacts(models(Vec::<ArtifactItem>::new()));
            window.set_overlays(models(Vec::<OverlayItem>::new()));
            window.set_canvas_width(1.0);
            window.set_canvas_height(1.0);
            window.set_canvas_note("几何叠加：未选中事件".into());
            window.set_payload_json("".into());
        }
    }
}

fn line(label: &str, value: String) -> FieldLine {
    FieldLine {
        label: label.into(),
        value: value.into(),
    }
}

fn dash() -> String {
    "—".to_string()
}

fn optional(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_else(dash)
}

fn models<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}
