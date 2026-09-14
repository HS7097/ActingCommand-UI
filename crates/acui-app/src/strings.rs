// SPDX-License-Identifier: AGPL-3.0-only
//! The two language tables. Every word a person reads in this program is here
//! and nowhere else; the wire spellings a row also shows — raw event types,
//! module names, ids, schema strings, sha256 — are never translated.
//!
//! The chosen table is read once at startup, into the `Strings` global for the
//! labels the window draws itself and directly for the lines the console fills.

/// `zh` or `en`, as the settings file and `--lang` spell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Zh,
    En,
}

impl Language {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "zh" => Some(Self::Zh),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub const fn labels(self) -> &'static Labels {
        match self {
            Self::Zh => &ZH,
            Self::En => &EN,
        }
    }
}

/// One language. Arrays are indexed by the contract's own ordering; the pair
/// tables are keyed by the wire string or by a stable field key.
pub struct Labels {
    /// Which of the two tables this is, for the dictionary lookups in `acui-rows`.
    pub tongue: Language,
    pub window_title: &'static str,
    pub usage: &'static str,
    pub data_source: &'static str,
    pub ledger_dir: &'static str,
    pub storage_format: &'static str,
    /// The two media the ledger picks between, in `backend()` spelling order.
    pub storage_segments: &'static str,
    pub storage_sqlite: &'static str,
    pub read_up_to: &'static str,
    pub source_incomplete: &'static str,
    pub read_up_to_unknown: &'static str,
    pub loaded_rows_top: &'static str,
    pub loaded_badge: &'static str,
    pub show_up_to: &'static str,
    pub all: &'static str,
    pub text_size: &'static str,
    pub text_sizes: [&'static str; 3],
    pub language: &'static str,
    pub languages: [&'static str; 2],
    pub restart_note: &'static str,
    pub severity_min: [&'static str; 4],
    pub severity_max: [&'static str; 4],
    pub all_modules: &'static str,
    pub id_placeholder: &'static str,
    pub continue_reading: &'static str,
    pub card_title: &'static str,
    pub card_note: &'static str,
    pub detail_title: &'static str,
    pub artifacts_title: &'static str,
    pub overlays_title: &'static str,
    pub raw_data_title: &'static str,
    pub no_event_selected: &'static str,
    pub not_selected: &'static str,
    /// seq, time, level, module, event, link.
    pub columns: [&'static str; 6],
    /// The #109 view names, in `LedgerView::ALL` order.
    pub tabs: [&'static str; 6],
    /// debug, info, warning, error, fatal.
    pub levels: [&'static str; 5],
    /// public, internal, sensitive, secret.
    pub sensitivities: [&'static str; 4],
    /// `EventSource`, in the contract's declaration order.
    pub sources: [(&'static str, &'static str); 8],
    /// Field key to label, for the instance card and the detail pane.
    pub fields: [(&'static str, &'static str); 28],
    pub artifact_kinds: [(&'static str, &'static str); 6],
    pub media_types: [(&'static str, &'static str); 4],
    pub checksum: &'static str,
    pub none: &'static str,
    pub event_count_missing: &'static str,
    /// recovered, unresolved, unknown.
    pub recovery_states: [&'static str; 3],
    pub recovery_gaps: [(&'static str, &'static str); 4],
    pub run_prefix: &'static str,
    pub failed_recovered: &'static str,
    pub failed_only: &'static str,
    pub gap_prefix: &'static str,
    pub join: &'static str,
    pub recovered_badge: &'static str,
    pub integrity_ok: &'static str,
    pub integrity_repairs: &'static str,
    pub integrity_bad: &'static str,
    pub writer_absent: &'static str,
    pub writer_locked: &'static str,
    pub writer_running: &'static str,
    pub writer_stopped: &'static str,
    pub age_days: &'static str,
    pub age_hours: &'static str,
    pub age_minutes: &'static str,
    pub age_seconds: &'static str,
    pub frame_size: &'static str,
    pub frame_size_missing: &'static str,
    pub frame_reading: &'static str,
    pub frame_verified: &'static str,
    pub frame_no_artifact: &'static str,
    pub frame_unavailable: &'static str,
    pub frame_evicted: &'static str,
    pub frame_decode_failed: &'static str,
    pub frame_too_large: &'static str,
    pub filter_id_error: &'static str,
    pub filter_rejected: &'static str,
    pub read_failed: &'static str,
}

pub const ZH: Labels = Labels {
    tongue: Language::Zh,
    window_title: "ActingCommand 监控台",
    usage: "用法：acui --state-root <状态根> [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]",
    data_source: "数据来源：本机账本（只读）",
    ledger_dir: "账本目录",
    storage_format: "存储格式",
    storage_segments: "分段文件",
    storage_sqlite: "SQLite",
    read_up_to: "读到第 {} 条",
    source_incomplete: "源不完整",
    read_up_to_unknown: "读取范围未给",
    loaded_rows_top: "本页已载入 {} 条",
    loaded_badge: "已载入 {}",
    show_up_to: "只看到",
    all: "全部",
    text_size: "字号",
    text_sizes: ["标准", "大", "特大"],
    language: "语言",
    languages: ["中文", "English"],
    restart_note: "重启后生效",
    severity_min: ["严重度不限", "≥ 警告", "≥ 错误", "≥ 致命"],
    severity_max: ["上限不限", "≤ 信息", "≤ 警告", "≤ 错误"],
    all_modules: "全部模块",
    id_placeholder: "完整的 correlation_ / request_ / run_ / task_ 编号",
    continue_reading: "继续读",
    card_title: "实例卡",
    card_note: "账本事实取自读面；只有「本页条数」算的是已载入的页",
    detail_title: "详情",
    artifacts_title: "附带材料",
    overlays_title: "识别框与点击点",
    raw_data_title: "原始数据",
    no_event_selected: "未选中事件",
    not_selected: "未选中",
    columns: ["序号", "时间（本地）", "级别", "来源模块", "事件", "关联：请求／任务／运行"],
    tabs: ["事件流", "观察与操作", "变更", "错误", "运行状况", "Lab"],
    levels: ["调试", "信息", "警告", "错误", "致命"],
    sensitivities: ["公开", "内部", "敏感", "机密"],
    sources: [
        ("runtime", "运行时"),
        ("scheduler", "调度器"),
        ("device", "设备"),
        ("cli", "命令行"),
        ("ui", "界面"),
        ("lab", "实验台"),
        ("system", "系统"),
        ("adapter", "适配器"),
    ],
    fields: [
        ("latest_sequence", "账本最新序号"),
        ("event_count", "账本事件总数"),
        ("loaded_rows", "本页条数"),
        ("first_event", "最早一条时间"),
        ("latest_event", "最新一条时间"),
        ("age", "距今"),
        ("integrity", "账本是否完整"),
        ("writer", "写入进程：编号／PID／是否仍在运行"),
        ("modules", "出现过的模块"),
        ("sequence", "序号"),
        ("event_id", "事件编号"),
        ("occurred_at", "发生时间"),
        ("event_type", "事件"),
        ("severity", "级别"),
        ("sensitivity", "敏感度"),
        ("origin", "来源：系统／模块／操作者"),
        ("views", "所属视图"),
        ("payload_schema", "载荷格式"),
        ("request_id", "请求编号"),
        ("correlation_id", "关联编号"),
        ("causation_id", "因果编号"),
        ("action_id", "动作编号"),
        ("run_id", "运行编号"),
        ("task_id", "任务编号"),
        ("instance_id", "实例编号"),
        ("recognition_id", "识别编号"),
        ("frame_id", "帧编号"),
        ("lease_id", "租约编号"),
    ],
    artifact_kinds: [
        ("capture.frame", "截图帧"),
        ("diagnostic.json", "诊断数据"),
        ("evidence.archive", "证据包"),
        ("evidence.manifest", "证据清单"),
        ("report.text", "文字报告"),
        ("report.strategy", "策略报告"),
    ],
    media_types: [
        ("image/png", "PNG"),
        ("application/json", "JSON"),
        ("application/zip", "ZIP"),
        ("text/plain", "纯文本"),
    ],
    checksum: "校验值",
    none: "—",
    event_count_missing: "—（读面未给，见 README）",
    recovery_states: ["已恢复", "未解决", "未知"],
    recovery_gaps: [
        ("missing_relation", "缺关系"),
        ("conflicting_outcome", "结果冲突"),
        ("source_incomplete", "源不完整"),
        ("context_limit", "上下文受限"),
    ],
    run_prefix: "一次运行：",
    failed_recovered: "第 {} 条失败，第 {} 条已恢复",
    failed_only: "第 {} 条失败，未见恢复",
    gap_prefix: "；缺口 ",
    join: "；",
    recovered_badge: "已恢复",
    integrity_ok: "完整",
    integrity_repairs: "完整 · 修复记录 {}",
    integrity_bad: "不完整：读全={} · 损坏尾部={}",
    writer_absent: "无写入进程记录",
    writer_locked: "记录被占用（{} 字节）",
    writer_running: "{} · PID {} · 仍在运行 · 起 {}",
    writer_stopped: "{} · PID {} · 已停止 · 起 {}",
    age_days: "{} 天前",
    age_hours: "{} 小时前",
    age_minutes: "{} 分钟前",
    age_seconds: "{} 秒前",
    frame_size: "画面 {}×{}",
    frame_size_missing: "未记录画面尺寸",
    frame_reading: "正在分段读取并全量校验 {} 字节…",
    frame_verified: "已校验帧 {}×{} · 分段读取，整份 sha256 校验通过",
    frame_no_artifact: "本条没有截图帧，只画几何叠加",
    frame_unavailable: "素材未提供：{}",
    frame_evicted: "材料已淘汰（{}）：意图 #{}{} · 观察至 #{}",
    frame_decode_failed: "PNG 解码失败：{}",
    frame_too_large: "素材 {} 字节超出监控台上限 {} 字节",
    filter_id_error: "编号需填完整的 correlation_ / request_ / run_ / task_ 标识",
    filter_rejected: "过滤条件无效：{}",
    read_failed: "读取失败：{}",
};

pub const EN: Labels = Labels {
    tongue: Language::En,
    window_title: "ActingCommand Console",
    usage: "Usage: acui --state-root <state_root> [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]",
    data_source: "Data Source: Local Ledger (Read-Only)",
    ledger_dir: "Ledger Directory",
    storage_format: "Storage Format",
    storage_segments: "Segments",
    storage_sqlite: "SQLite",
    read_up_to: "Read Up To #{}",
    source_incomplete: "Source Incomplete",
    read_up_to_unknown: "Read Range Not Given",
    loaded_rows_top: "Loaded Rows: {}",
    loaded_badge: "Loaded {}",
    show_up_to: "Show Up To",
    all: "All",
    text_size: "Text Size",
    text_sizes: ["Standard", "Large", "Extra Large"],
    language: "Language",
    languages: ["中文", "English"],
    restart_note: "Takes effect after restart",
    severity_min: ["Any Level", "≥ Warning", "≥ Error", "≥ Fatal"],
    severity_max: ["Any Upper Bound", "≤ Info", "≤ Warning", "≤ Error"],
    all_modules: "All Modules",
    id_placeholder: "Whole correlation_ / request_ / run_ / task_ id",
    continue_reading: "Continue Reading",
    card_title: "Instance Card",
    card_note: "Ledger facts come from the read face; only Loaded Rows counts the loaded page",
    detail_title: "Detail",
    artifacts_title: "Attached Materials",
    overlays_title: "Recognition Boxes and Tap Points",
    raw_data_title: "Raw Data",
    no_event_selected: "No event selected",
    not_selected: "Not Selected",
    columns: ["Seq", "Time (Local)", "Level", "Source Module", "Event", "Link: Request／Task／Run"],
    tabs: ["Event Stream", "Observe and Act", "Changes", "Errors", "Health", "Lab"],
    levels: ["Debug", "Info", "Warning", "Error", "Fatal"],
    sensitivities: ["Public", "Internal", "Sensitive", "Secret"],
    sources: [
        ("runtime", "Runtime"),
        ("scheduler", "Scheduler"),
        ("device", "Device"),
        ("cli", "CLI"),
        ("ui", "UI"),
        ("lab", "Lab"),
        ("system", "System"),
        ("adapter", "Adapter"),
    ],
    fields: [
        ("latest_sequence", "Latest Sequence"),
        ("event_count", "Total Events"),
        ("loaded_rows", "Loaded Rows"),
        ("first_event", "First Event"),
        ("latest_event", "Latest Event"),
        ("age", "Age"),
        ("integrity", "Ledger Integrity"),
        ("writer", "Writer: ID／PID／Running"),
        ("modules", "Modules Seen"),
        ("sequence", "Sequence"),
        ("event_id", "Event ID"),
        ("occurred_at", "Occurred At"),
        ("event_type", "Event"),
        ("severity", "Level"),
        ("sensitivity", "Sensitivity"),
        ("origin", "Origin: Source／Module／Actor"),
        ("views", "Views"),
        ("payload_schema", "Payload Schema"),
        ("request_id", "Request ID"),
        ("correlation_id", "Correlation ID"),
        ("causation_id", "Causation ID"),
        ("action_id", "Action ID"),
        ("run_id", "Run ID"),
        ("task_id", "Task ID"),
        ("instance_id", "Instance ID"),
        ("recognition_id", "Recognition ID"),
        ("frame_id", "Frame ID"),
        ("lease_id", "Lease ID"),
    ],
    artifact_kinds: [
        ("capture.frame", "Capture Frame"),
        ("diagnostic.json", "Diagnostic Data"),
        ("evidence.archive", "Evidence Archive"),
        ("evidence.manifest", "Evidence Manifest"),
        ("report.text", "Text Report"),
        ("report.strategy", "Strategy Report"),
    ],
    media_types: [
        ("image/png", "PNG"),
        ("application/json", "JSON"),
        ("application/zip", "ZIP"),
        ("text/plain", "Plain Text"),
    ],
    checksum: "Checksum",
    none: "—",
    event_count_missing: "—(not given by the read face, see README)",
    recovery_states: ["Recovered", "Unresolved", "Unknown"],
    recovery_gaps: [
        ("missing_relation", "Missing Relation"),
        ("conflicting_outcome", "Conflicting Outcome"),
        ("source_incomplete", "Source Incomplete"),
        ("context_limit", "Context Limit"),
    ],
    run_prefix: "Run: ",
    failed_recovered: "failed at #{}, recovered at #{}",
    failed_only: "failed at #{}, no recovery seen",
    gap_prefix: "; gap ",
    join: "; ",
    recovered_badge: "Recovered",
    integrity_ok: "Intact",
    integrity_repairs: "Intact · {} repairs recorded",
    integrity_bad: "Not intact: read_complete={} · corrupt tail={}",
    writer_absent: "No writer recorded",
    writer_locked: "Record held open ({} bytes)",
    writer_running: "{} · PID {} · still running · since {}",
    writer_stopped: "{} · PID {} · stopped · since {}",
    age_days: "{} days ago",
    age_hours: "{} hours ago",
    age_minutes: "{} minutes ago",
    age_seconds: "{} seconds ago",
    frame_size: "Frame {}×{}",
    frame_size_missing: "Frame Size Not Recorded",
    frame_reading: "Reading in ranges and verifying all {} bytes…",
    frame_verified: "Verified frame {}×{} · read in ranges, whole sha256 checked",
    frame_no_artifact: "No capture frame on this event; geometry only",
    frame_unavailable: "Material not given: {}",
    frame_evicted: "Material evicted ({}): intent #{}{} · observed through #{}",
    frame_decode_failed: "PNG decode failed: {}",
    frame_too_large: "Material of {} bytes is over the console's limit of {} bytes",
    filter_id_error: "The id must be a whole correlation_ / request_ / run_ / task_ identifier",
    filter_rejected: "Filter rejected: {}",
    read_failed: "Read failed: {}",
};

impl Labels {
    /// The label for a field key; the key itself when the table has no entry.
    pub fn field<'a>(&self, key: &'a str) -> &'a str {
        pick(&self.fields, key).unwrap_or(key)
    }

    /// The name for a wire code, or the wire code itself.
    pub fn artifact_kind<'a>(&self, wire: &'a str) -> &'a str {
        pick(&self.artifact_kinds, wire).unwrap_or(wire)
    }

    pub fn media_type<'a>(&self, wire: &'a str) -> &'a str {
        pick(&self.media_types, wire).unwrap_or(wire)
    }

    pub fn source<'a>(&self, wire: &'a str) -> &'a str {
        pick(&self.sources, wire).unwrap_or(wire)
    }

    pub fn recovery_gap<'a>(&self, wire: &'a str) -> &'a str {
        pick(&self.recovery_gaps, wire).unwrap_or(wire)
    }
}

fn pick(table: &[(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(wire, _)| *wire == key)
        .map(|(_, label)| *label)
}

/// Substitutes `{}` in a table string, left to right. The tables are the only
/// source of these templates, so a missing value just leaves the text short.
pub fn fill(template: &str, values: &[&str]) -> String {
    let mut text = String::with_capacity(template.len());
    let mut rest = template;
    for value in values {
        match rest.split_once("{}") {
            Some((head, tail)) => {
                text.push_str(head);
                text.push_str(value);
                rest = tail;
            }
            None => break,
        }
    }
    text.push_str(rest);
    text
}
