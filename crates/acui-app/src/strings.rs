// SPDX-License-Identifier: GPL-3.0-only
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
    pub data_source_online: &'static str,
    pub ledger_dir: &'static str,
    pub storage_format: &'static str,
    /// The two media the ledger picks between, in `backend()` spelling order.
    pub storage_segments: &'static str,
    pub storage_sqlite: &'static str,
    /// Online: the Runtime answers pages without stating its medium.
    pub storage_runtime: &'static str,
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
    /// The port box: its first item, one item per port (port, latest member
    /// id), and the one item it shows when it is disabled and why.
    pub all_instances: &'static str,
    pub port_option: &'static str,
    pub no_binding_records: &'static str,
    pub port_online_unsupported: &'static str,
    /// The port column of a row that links no instance.
    pub host: &'static str,
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
    /// seq, time, level, module, event, link, port.
    pub columns: [&'static str; 7],
    /// The #109 view names, in `LedgerView::ALL` order.
    pub tabs: [&'static str; 6],
    /// debug, info, warning, error, fatal.
    pub levels: [&'static str; 5],
    /// public, internal, sensitive, secret.
    pub sensitivities: [&'static str; 4],
    /// `EventSource`, in the contract's declaration order.
    pub sources: [(&'static str, &'static str); 8],
    /// Field key to label, for the instance card and the detail pane.
    pub fields: [(&'static str, &'static str); 36],
    /// `ExecutionBackendProvenance`, by wire value.
    pub provenances: [(&'static str, &'static str); 2],
    pub artifact_kinds: [(&'static str, &'static str); 6],
    pub media_types: [(&'static str, &'static str); 4],
    pub checksum: &'static str,
    pub none: &'static str,
    /// A ledger count over an incomplete read, a medium without a repair log,
    /// and a count the online Runtime does not state.
    pub count_verified_prefix: &'static str,
    pub count_no_repair_log: &'static str,
    pub count_not_stated: &'static str,
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
    pub integrity_bad: &'static str,
    pub writer_absent: &'static str,
    pub writer_locked: &'static str,
    pub writer_running: &'static str,
    pub writer_stopped: &'static str,
    pub writer_runtime: &'static str,
    /// The instance card's read-face line: online, or offline and why.
    pub source_online: &'static str,
    pub source_offline_requested: &'static str,
    pub source_offline_absent: &'static str,
    pub source_offline_failed: &'static str,
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
    pub filter_instance_conflict: &'static str,
    pub filter_rejected: &'static str,
    pub read_failed: &'static str,
    /// The launcher block in the top bar: status, two buttons, one line.
    pub launcher_title: &'static str,
    pub start: &'static str,
    pub request_shutdown: &'static str,
    /// pid, owner epoch.
    pub runtime_running: &'static str,
    /// client error code, client operation.
    pub runtime_not_running: &'static str,
    pub already_running: &'static str,
    pub start_busy: &'static str,
    /// settings key.
    pub key_not_configured: &'static str,
    /// settings key, value.
    pub key_not_absolute: &'static str,
    /// directory, error.
    pub log_dir_failed: &'static str,
    /// executable, error.
    pub spawn_failed: &'static str,
    /// error.
    pub child_status_failed: &'static str,
    /// pid, attempt, attempts, log path.
    pub start_waiting: &'static str,
    /// exit code, log path.
    pub start_exited: &'static str,
    /// pid, owner epoch.
    pub start_ready: &'static str,
    pub restart_online: &'static str,
    /// attempts, client error code, client operation.
    pub start_not_ready: &'static str,
    /// The start press as a client action: its sequence.
    pub start_recorded: &'static str,
    /// runtime refusal code, client error code, client operation.
    pub start_record_refused: &'static str,
    /// client error code, client operation.
    pub start_record_failed: &'static str,
    /// attempt, attempts.
    pub shutdown_busy_retry: &'static str,
    /// attempts sent, attempts.
    pub shutdown_attempts: &'static str,
    /// receipt state, request id, action sequence.
    pub shutdown_accepted: &'static str,
    /// runtime refusal code, client error code, client operation.
    pub shutdown_refused: &'static str,
    /// client error code, client operation.
    pub shutdown_failed: &'static str,
    /// The instance-configuration window, and the top-bar button that opens it.
    pub instance_config: &'static str,
    pub config_intro: &'static str,
    pub add_instance: &'static str,
    /// alias, instance_id, binding, adb_path, nemu_app_index, application_id,
    /// capture_backend, touch_backend.
    pub form_fields: [&'static str; 8],
    pub binding_kinds: [&'static str; 3],
    /// instance_index, instance_name, host, port; adb_path optional, then
    /// required; nemu_app_index.
    pub form_placeholders: [&'static str; 7],
    pub optional_note: &'static str,
    pub check_and_save: &'static str,
    /// error.
    pub id_failed: &'static str,
    /// field; field, largest value, value.
    pub value_missing: &'static str,
    pub value_not_number: &'static str,
    /// config path; config path, error; config path, error; config path.
    pub config_missing: &'static str,
    pub config_unreadable: &'static str,
    pub config_not_json: &'static str,
    pub config_no_instances: &'static str,
    pub checking: &'static str,
    /// candidate path, error.
    pub candidate_failed: &'static str,
    /// seconds; exit code, output; error.code, stage; exit code.
    pub check_timeout: &'static str,
    pub check_unparsed: &'static str,
    pub check_failed: &'static str,
    pub check_ok_nonzero: &'static str,
    /// config path, error; candidate path, error.
    pub replace_failed: &'static str,
    pub candidate_left: &'static str,
    /// Ends every failed save.
    pub config_unchanged: &'static str,
    /// config path.
    pub saved: &'static str,
    /// Runtime status as the launcher words it.
    pub saved_running: &'static str,
    pub saved_stopped: &'static str,
    /// config path, instance count; config path, entry index.
    pub config_listed: &'static str,
    pub config_bad_entry: &'static str,
    /// Online; error; port; bound with no port; not bound.
    pub ledger_online: &'static str,
    pub ledger_failed: &'static str,
    pub ledger_port: &'static str,
    pub ledger_no_port: &'static str,
    pub ledger_none: &'static str,
    /// index; name; host, port.
    pub binding_index: &'static str,
    pub binding_name: &'static str,
    pub binding_adb: &'static str,
    pub binding_none: &'static str,
    /// instance_id, application_id, capture_backend, touch_backend.
    pub instance_detail: &'static str,
    /// instance_id.
    pub entry_gone: &'static str,
    pub entry_no_id: &'static str,
}

pub const ZH: Labels = Labels {
    tongue: Language::Zh,
    window_title: "ActingCommand 监控台",
    usage: "用法：acui [--state-root <状态根>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]\n不给 --state-root 时取 acui.toml 的 state_root；两处都没有就不开台。",
    data_source: "数据来源：本机账本（只读）",
    data_source_online: "数据来源：运行中的 Runtime（只读）",
    ledger_dir: "账本目录",
    storage_format: "存储格式",
    storage_segments: "分段文件",
    storage_sqlite: "SQLite",
    storage_runtime: "由 Runtime 判定",
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
    all_instances: "全部实例",
    port_option: "端口 {} ／ {}",
    no_binding_records: "无绑定记录",
    port_online_unsupported: "在线态不支持",
    host: "宿主",
    id_placeholder: "完整的 correlation_ / request_ / run_ / task_ / instance_ 编号",
    continue_reading: "继续读",
    card_title: "实例卡",
    card_note: "账本事实取自读面；只有「本页条数」算的是已载入的页",
    detail_title: "详情",
    artifacts_title: "附带材料",
    overlays_title: "识别框与点击点",
    raw_data_title: "原始数据",
    no_event_selected: "未选中事件",
    not_selected: "未选中",
    columns: [
        "序号",
        "时间（本地）",
        "级别",
        "来源模块",
        "事件",
        "关联：请求／任务／运行",
        "端口",
    ],
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
        ("source", "读面"),
        ("latest_sequence", "账本最新序号"),
        ("event_count", "账本事件总数"),
        ("loaded_rows", "本页条数"),
        ("first_event", "最早一条时间"),
        ("latest_event", "最新一条时间"),
        ("age", "距今"),
        ("integrity", "账本是否完整"),
        ("repair_count", "修复记录数"),
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
        ("instance_alias", "实例别名"),
        ("adb_port", "端口"),
        ("provenance", "来源"),
        ("bound_ids", "绑定编号数"),
        ("latest_binding", "最新绑定序号"),
        ("port_bindings", "实例绑定"),
    ],
    provenances: [
        ("physical_device", "实机"),
        ("fixture_simulation", "夹具"),
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
    count_verified_prefix: "{}（读取不完整，只计已校验的前缀）",
    count_no_repair_log: "—（此存储格式没有修复日志）",
    count_not_stated: "—（Runtime 不提供此项）",
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
    integrity_bad: "不完整：读全={} · 损坏尾部={}",
    writer_absent: "无写入进程记录",
    writer_locked: "记录被占用（{} 字节）",
    writer_running: "{} · PID {} · 仍在运行 · 起 {}",
    writer_stopped: "{} · PID {} · 已停止 · 起 {}",
    writer_runtime: "运行中的 Runtime · PID {} · {} · 起 {}",
    source_online: "在线 · 经 runtime-client 直连运行中的 Runtime",
    source_offline_requested: "离线 · 按 --source offline",
    source_offline_absent: "离线 · 自动：状态根里没有 runtime-info.json",
    source_offline_failed: "离线 · 自动：连接 Runtime 失败 {}（{}）",
    age_days: "{} 天前",
    age_hours: "{} 小时前",
    age_minutes: "{} 分钟前",
    age_seconds: "{} 秒前",
    frame_size: "画面 {}×{}",
    frame_size_missing: "未记录画面尺寸",
    frame_reading: "正在读取并全量校验 {} 字节…",
    frame_verified: "已校验帧 {}×{} · 整份 sha256 校验通过",
    frame_no_artifact: "本条没有截图帧，只画几何叠加",
    frame_unavailable: "素材未提供：{}",
    frame_evicted: "材料已淘汰（{}）：意图 #{}{} · 观察至 #{}",
    frame_decode_failed: "PNG 解码失败：{}",
    frame_too_large: "素材 {} 字节超出监控台上限 {} 字节",
    filter_id_error: "编号需填完整的 correlation_ / request_ / run_ / task_ / instance_ 标识",
    filter_instance_conflict: "端口与实例编号只能选一",
    filter_rejected: "过滤条件无效：{}",
    read_failed: "读取失败：{}",
    launcher_title: "启动器",
    start: "启动",
    request_shutdown: "请求关闭",
    runtime_running: "Runtime 运行中 · PID {} · owner epoch {}",
    runtime_not_running: "Runtime 未运行 · {}（{}）",
    already_running: "已在运行，未拉起",
    start_busy: "上一次启动仍在等待就绪",
    key_not_configured: "acui.toml 未配置 {}",
    key_not_absolute: "acui.toml 的 {} 不是绝对路径：{}",
    log_dir_failed: "无法建立日志目录 {}：{}",
    spawn_failed: "拉起 {} 失败：{}",
    child_status_failed: "读取子进程状态失败：{}",
    start_waiting: "已拉起 PID {} · 等待就绪 {}/{} · 日志 {}",
    start_exited: "actingd 已退出，退出码 {}，见 {}",
    start_ready: "Runtime 已就绪 · PID {} · owner epoch {}",
    restart_online: "本台按离线读；要在线读请带 --source online 重启",
    start_not_ready: "{} 次尝试后仍未就绪 · 最后错误 {}（{}）",
    start_recorded: "动作已记账 #{}",
    start_record_refused: "动作记账被拒：{} · 客户端 {}（{}）",
    start_record_failed: "动作记账失败：{}（{}）",
    shutdown_busy_retry: "请求关闭 · 忙碌重试 {}/{}",
    shutdown_attempts: "第 {}/{} 次",
    shutdown_accepted: "关闭请求已受理 · 回执 {} · 请求 {} · 动作已记账 #{}",
    shutdown_refused: "关闭请求被拒：{} · 客户端 {}（{}）",
    shutdown_failed: "关闭请求失败：{}（{}）",
    instance_config: "实例配置",
    config_intro: "列出 actingd_config 的 instances。点一行载入下面的表单编辑，或「新增实例」另起一项，然后「校验并保存」：先在同一目录写临时文件，交 actingd check-config 校验，通过才替换原文件；表单不管的字段原样保留。改动在 Runtime 重启后生效。",
    add_instance: "新增实例",
    form_fields: ["alias（必填）", "instance_id", "绑定方式", "adb_path", "nemu_app_index", "application_id", "capture_backend", "touch_backend"],
    binding_kinds: ["MuMu 序号 instance_index", "MuMu 名称 instance_name", "ADB 地址 host + port"],
    form_placeholders: ["instance_index（0–65535）", "instance_name", "host", "port（0–65535）", "选填：留空则用 MuMu 发现报告的 adb", "必填：adb 可执行文件的路径", "选填（0–4294967295）"],
    optional_note: "adb_path 在 ADB 地址方式下必填、MuMu 方式下选填；nemu_app_index、application_id 与两个后端选填，取值由 check-config 判定。留空的选填项不写进文件。",
    check_and_save: "校验并保存",
    id_failed: "生成 instance_id 失败：{}",
    value_missing: "{} 不能为空",
    value_not_number: "{} 须为 0–{} 的整数：{}",
    config_missing: "{} 不存在",
    config_unreadable: "读取 {} 失败：{}",
    config_not_json: "{} 不是合法的 JSON：{}",
    config_no_instances: "{} 里没有 instances 数组",
    checking: "正在用 check-config 校验…",
    candidate_failed: "写临时文件 {} 失败：{}",
    check_timeout: "check-config {} 秒内未结束，已终止",
    check_unparsed: "check-config 的输出无法识别（退出码 {}）：{}",
    check_failed: "check-config 未通过 · error.code {} · stage {}",
    check_ok_nonzero: "check-config 报 ok 但退出码为 {}，不保存",
    replace_failed: "替换 {} 失败：{}",
    candidate_left: "；临时文件 {} 未能删除：{}",
    config_unchanged: " · 原配置未改动",
    saved: "已保存进 {}（check-config 通过）· Runtime 重启后生效",
    saved_running: "现在：{} · 在主窗口先「请求关闭」，停下后再「启动」",
    saved_stopped: "现在：{} · 在主窗口点「启动」即按新配置运行",
    config_listed: "{} · 共 {} 个实例",
    config_bad_entry: "{} 的 instances 第 {} 项不是对象",
    ledger_online: "账本绑定：在线读面不给",
    ledger_failed: "账本绑定读取失败：{}",
    ledger_port: "账本已绑定 · 端口 {}",
    ledger_no_port: "账本已绑定 · 最新一次没有端口",
    ledger_none: "账本里没有此实例的绑定",
    binding_index: "MuMu 序号 {}",
    binding_name: "MuMu 名称 {}",
    binding_adb: "ADB {}:{}",
    binding_none: "未绑定",
    instance_detail: "{} · 应用 {} · 截图 {} · 触控 {}",
    entry_gone: "配置文件里已没有 {}（别处改过）",
    entry_no_id: "这一项没有字符串 instance_id，不能在这里编辑",
};

pub const EN: Labels = Labels {
    tongue: Language::En,
    window_title: "ActingCommand Console",
    usage: "Usage: acui [--state-root <state_root>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]\nWithout --state-root the state_root key of acui.toml is used; with neither, the console does not open.",
    data_source: "Data Source: Local Ledger (Read-Only)",
    data_source_online: "Data Source: Running Runtime (Read-Only)",
    ledger_dir: "Ledger Directory",
    storage_format: "Storage Format",
    storage_segments: "Segments",
    storage_sqlite: "SQLite",
    storage_runtime: "Chosen by the Runtime",
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
    all_instances: "All Instances",
    port_option: "Port {} ／ {}",
    no_binding_records: "No Binding Records",
    port_online_unsupported: "Not in Online Mode",
    host: "Host",
    id_placeholder: "Whole correlation_ / request_ / run_ / task_ / instance_ id",
    continue_reading: "Continue Reading",
    card_title: "Instance Card",
    card_note: "Ledger facts come from the read face; only Loaded Rows counts the loaded page",
    detail_title: "Detail",
    artifacts_title: "Attached Materials",
    overlays_title: "Recognition Boxes and Tap Points",
    raw_data_title: "Raw Data",
    no_event_selected: "No event selected",
    not_selected: "Not Selected",
    columns: [
        "Seq",
        "Time (Local)",
        "Level",
        "Source Module",
        "Event",
        "Link: Request／Task／Run",
        "Port",
    ],
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
        ("source", "Read Face"),
        ("latest_sequence", "Latest Sequence"),
        ("event_count", "Total Events"),
        ("loaded_rows", "Loaded Rows"),
        ("first_event", "First Event"),
        ("latest_event", "Latest Event"),
        ("age", "Age"),
        ("integrity", "Ledger Integrity"),
        ("repair_count", "Repair Records"),
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
        ("instance_alias", "Alias"),
        ("adb_port", "Port"),
        ("provenance", "Provenance"),
        ("bound_ids", "Bound IDs"),
        ("latest_binding", "Latest Binding"),
        ("port_bindings", "Instance Bindings"),
    ],
    provenances: [
        ("physical_device", "Physical Device"),
        ("fixture_simulation", "Fixture"),
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
    count_verified_prefix: "{} (incomplete read: verified prefix only)",
    count_no_repair_log: "—(this storage format keeps no repair log)",
    count_not_stated: "—(not stated by the Runtime)",
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
    integrity_bad: "Not intact: read_complete={} · corrupt tail={}",
    writer_absent: "No writer recorded",
    writer_locked: "Record held open ({} bytes)",
    writer_running: "{} · PID {} · still running · since {}",
    writer_stopped: "{} · PID {} · stopped · since {}",
    writer_runtime: "Running Runtime · PID {} · {} · since {}",
    source_online: "Online · runtime-client to the running Runtime",
    source_offline_requested: "Offline · by --source offline",
    source_offline_absent: "Offline · auto: no runtime-info.json in the state root",
    source_offline_failed: "Offline · auto: connecting to the Runtime failed with {} ({})",
    age_days: "{} days ago",
    age_hours: "{} hours ago",
    age_minutes: "{} minutes ago",
    age_seconds: "{} seconds ago",
    frame_size: "Frame {}×{}",
    frame_size_missing: "Frame Size Not Recorded",
    frame_reading: "Reading and verifying all {} bytes…",
    frame_verified: "Verified frame {}×{} · whole sha256 checked",
    frame_no_artifact: "No capture frame on this event; geometry only",
    frame_unavailable: "Material not given: {}",
    frame_evicted: "Material evicted ({}): intent #{}{} · observed through #{}",
    frame_decode_failed: "PNG decode failed: {}",
    frame_too_large: "Material of {} bytes is over the console's limit of {} bytes",
    filter_id_error: "The id must be a whole correlation_ / request_ / run_ / task_ / instance_ identifier",
    filter_instance_conflict: "Pick a port or type an instance id, not both",
    filter_rejected: "Filter rejected: {}",
    read_failed: "Read failed: {}",
    launcher_title: "Launcher",
    start: "Start",
    request_shutdown: "Request Shutdown",
    runtime_running: "Runtime Running · PID {} · Owner Epoch {}",
    runtime_not_running: "Runtime Not Running · {} ({})",
    already_running: "Already Running, Nothing Started",
    start_busy: "The Previous Start Is Still Waiting for Readiness",
    key_not_configured: "{} Not Configured in acui.toml",
    key_not_absolute: "{} in acui.toml Is Not an Absolute Path: {}",
    log_dir_failed: "Cannot Create Log Directory {}: {}",
    spawn_failed: "Starting {} Failed: {}",
    child_status_failed: "Reading the Child's Status Failed: {}",
    start_waiting: "Started PID {} · Waiting for Readiness {}/{} · Log {}",
    start_exited: "actingd Exited with Code {}, See {}",
    start_ready: "Runtime Ready · PID {} · Owner Epoch {}",
    restart_online: "This Console Reads Offline; Restart with --source online to Read It",
    start_not_ready: "Not Ready After {} Attempts · Last Error {} ({})",
    start_recorded: "Action Recorded at #{}",
    start_record_refused: "Action Record Refused: {} · Client {} ({})",
    start_record_failed: "Action Record Failed: {} ({})",
    shutdown_busy_retry: "Request Shutdown · Busy, Retry {}/{}",
    shutdown_attempts: "Attempt {}/{}",
    shutdown_accepted: "Shutdown Accepted · Receipt {} · Request {} · Action Recorded at #{}",
    shutdown_refused: "Shutdown Refused: {} · Client {} ({})",
    shutdown_failed: "Shutdown Request Failed: {} ({})",
    instance_config: "Instance Configuration",
    config_intro: "Lists the instances of actingd_config. Click a row to edit it in the form below, or Add Instance to start a new one, then Check and Save: a temporary file is written in the same directory and checked by actingd check-config, and only an ok replaces the file; fields the form does not manage are kept. Changes take effect when the Runtime restarts.",
    add_instance: "Add Instance",
    form_fields: ["alias (required)", "instance_id", "Binding", "adb_path", "nemu_app_index", "application_id", "capture_backend", "touch_backend"],
    binding_kinds: ["MuMu Index instance_index", "MuMu Name instance_name", "ADB Address host + port"],
    form_placeholders: ["instance_index (0–65535)", "instance_name", "host", "port (0–65535)", "Optional: empty uses the adb MuMu discovery reports", "Required: path to the adb executable", "Optional (0–4294967295)"],
    optional_note: "adb_path is required with an ADB address and optional with a MuMu binding; nemu_app_index, application_id and the two backends are optional and check-config decides their valid values. An empty optional box writes nothing.",
    check_and_save: "Check and Save",
    id_failed: "Generating an instance_id Failed: {}",
    value_missing: "{} Must Not Be Empty",
    value_not_number: "{} Must Be a Whole Number 0–{}: {}",
    config_missing: "{} Does Not Exist",
    config_unreadable: "Reading {} Failed: {}",
    config_not_json: "{} Is Not Valid JSON: {}",
    config_no_instances: "{} Has No instances Array",
    checking: "Checking with check-config…",
    candidate_failed: "Writing the Temporary File {} Failed: {}",
    check_timeout: "check-config Did Not Finish Within {} s and Was Stopped",
    check_unparsed: "check-config Output Not Recognized (Exit Code {}): {}",
    check_failed: "check-config Rejected It · error.code {} · stage {}",
    check_ok_nonzero: "check-config Said ok but Exited with {}; Not Saved",
    replace_failed: "Replacing {} Failed: {}",
    candidate_left: "; Temporary File {} Not Removed: {}",
    config_unchanged: " · The Config File Was Not Changed",
    saved: "Saved to {} (check-config ok) · Takes Effect When the Runtime Restarts",
    saved_running: "Now: {} · In the Main Window, Request Shutdown, Then Start Once It Has Stopped",
    saved_stopped: "Now: {} · Start in the Main Window Runs the New Configuration",
    config_listed: "{} · {} Instances",
    config_bad_entry: "In {}, instances Entry {} Is Not an Object",
    ledger_online: "Ledger Binding: Not Given Online",
    ledger_failed: "Reading Ledger Bindings Failed: {}",
    ledger_port: "Bound in the Ledger · Port {}",
    ledger_no_port: "Bound in the Ledger · Latest Binding Has No Port",
    ledger_none: "No Ledger Binding for This Instance",
    binding_index: "MuMu Index {}",
    binding_name: "MuMu Name {}",
    binding_adb: "ADB {}:{}",
    binding_none: "No Binding",
    instance_detail: "{} · App {} · Capture {} · Touch {}",
    entry_gone: "{} Is No Longer in the Config File (Changed Elsewhere)",
    entry_no_id: "This Entry Has No String instance_id and Cannot Be Edited Here",
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

    pub fn provenance<'a>(&self, wire: &'a str) -> &'a str {
        pick(&self.provenances, wire).unwrap_or(wire)
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
