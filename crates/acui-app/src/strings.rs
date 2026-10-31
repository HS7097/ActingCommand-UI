// SPDX-License-Identifier: GPL-3.0-only
//! The two language tables. Every word a person reads in this program is here
//! and nowhere else; the wire spellings a row also shows — raw event types,
//! module names, ids, schema strings, sha256 — are never translated. The one
//! exception is the words for an outcome code, which are in acui-rows'
//! `outcome-text.json` (`acui_rows::outcome`), next to the UI's code table.
//!
//! The table of the language shown is taken again at every paint (`Ui::labels`):
//! into each window's `Strings` global for the labels the window draws itself,
//! filled again in every window when the language changes, and directly for
//! the lines the console fills. A worker thread never holds a table; it hands back a `Phrase`.

use acui_rows::outcome::{explain, Tongue};

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

    /// The outcome words' language for this table.
    pub const fn outcome_tongue(self) -> Tongue {
        match self {
            Self::Zh => Tongue::ZhHans,
            Self::En => Tongue::En,
        }
    }
}

/// One language. Arrays are indexed by the contract's own ordering; the pair
/// tables are keyed by the wire string or by a stable field key.
pub struct Labels {
    /// Which of the two tables this is, for the dictionary lookups in `acui-rows`.
    pub tongue: Language,
    /// The workbench's title; the ledger window's.
    pub window_title: &'static str,
    pub ledger_window_title: &'static str,
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
    /// Lowest and highest position the loaded windows cover.
    pub read_range: &'static str,
    /// Hidden performance-monitor events; appended to the loaded-rows line.
    pub performance_hidden: &'static str,
    pub show_performance: &'static str,
    pub jump_latest: &'static str,
    pub follow_latest: &'static str,
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
    /// The theme box, in `Theme` index order. On a system other than Windows
    /// the second option says it has no glass there.
    pub theme: &'static str,
    pub theme_options: [&'static str; 2],
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
    /// debug, success, info, warning, error, fatal.
    pub levels: [&'static str; 6],
    /// public, internal, sensitive, secret.
    pub sensitivities: [&'static str; 4],
    /// `EventSource`, in the contract's declaration order.
    pub sources: [(&'static str, &'static str); 8],
    /// Field key to label, for the instance card and the detail pane.
    pub fields: [(&'static str, &'static str); 50],
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
    /// An offline SQLite root: since LN-OFF-1 the opening checks the head,
    /// the tail and the eviction proofs only, and earlier records are
    /// authenticated as they are read, so "intact" would overstate it.
    pub integrity_ok_sqlite: &'static str,
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
    /// The offline face the ledger refused to open: ledger code, operation,
    /// detail; the hint only follows `ledger_io` with io kind `not_found`.
    pub source_unopened: &'static str,
    pub source_unopened_hint: &'static str,
    /// Follows `ledger_read_budget_exceeded`: the offline opening's own 60 s deadline.
    pub source_unopened_budget_hint: &'static str,
    /// The ledger's io kind; appended to the unopened line.
    pub io_kind_text: &'static str,
    /// Unopened: the top bar and the two boxes, then the list and the frame pane.
    pub ledger_unopened: &'static str,
    pub list_unopened: &'static str,
    pub age_days: &'static str,
    pub age_hours: &'static str,
    pub age_minutes: &'static str,
    pub age_seconds: &'static str,
    pub frame_size: &'static str,
    pub frame_size_missing: &'static str,
    pub frame_reading: &'static str,
    pub frame_verified: &'static str,
    pub frame_no_artifact: &'static str,
    /// The frame an event was taken on or acted on: its page, the sentence
    /// under it, and why the pane has no frame to show.
    pub page_unmatched: &'static str,
    pub frame_step: &'static str,
    pub frame_matched: &'static str,
    pub frame_unmatched: &'static str,
    pub frame_targets: &'static str,
    pub frame_explain_full: &'static str,
    pub frame_explain_short: &'static str,
    pub frame_via_step: &'static str,
    pub frame_via_input: &'static str,
    pub frame_basis_missing: &'static str,
    pub frame_capture_missing: &'static str,
    pub frame_events_failed: &'static str,
    /// Input kinds as the ledger writes them, by prefix, and their names.
    pub action_kinds: &'static [(&'static str, &'static str)],
    pub frame_unavailable: &'static str,
    pub frame_evicted: &'static str,
    pub frame_decode_failed: &'static str,
    pub frame_too_large: &'static str,
    pub filter_id_error: &'static str,
    pub filter_instance_conflict: &'static str,
    pub filter_rejected: &'static str,
    pub read_failed: &'static str,
    /// An outcome code's sentence, then its handling.
    pub outcome_words: &'static str,
    /// The outcome words could not be read: the table's problem.
    pub outcome_table_unusable: &'static str,
    /// A code neither the UI's table nor the pinned Runtime's catalog holds,
    /// such as one a newer Runtime sends: said, never left blank.
    pub outcome_unknown: &'static str,
    pub outcome_unknown_short: &'static str,
    /// A follow tick's failure, shown beside the view's own.
    pub follow_failed: &'static str,
    /// The committed time span could not be read: the read's own words.
    pub span_failed: &'static str,
    /// The data source while the read face is opening or did not open.
    pub data_source_pending: &'static str,
    /// The ledger window's reader opens the read face: the top bar, the boxes
    /// and the frame pane, then the list.
    pub ledger_opening: &'static str,
    pub list_opening: &'static str,
    /// The reader stopped: the panic's message; the reader is gone.
    pub ledger_worker_panicked: &'static str,
    pub ledger_worker_gone: &'static str,
    /// A read reached a Runtime other than the session's; beside "reopen".
    pub ledger_owner_changed: &'static str,
    /// A Runtime is up while an `auto` window reads offline for want of one;
    /// beside "reopen".
    pub ledger_runtime_started: &'static str,
    /// The button that opens a new session of the read face.
    pub reopen_ledger: &'static str,
    /// The busy line: what the reader reads, the seconds it has waited, and
    /// about how long a read of that kind may take at most; per page for a
    /// read that follows a cursor.
    pub busy_waited: &'static str,
    pub busy_waited_page: &'static str,
    pub busy_bindings: &'static str,
    pub busy_span: &'static str,
    pub busy_probing: &'static str,
    /// window, windows.
    pub busy_reading: &'static str,
    pub busy_repinning: &'static str,
    pub busy_polling: &'static str,
    pub busy_instances: &'static str,
    pub busy_reconnecting: &'static str,
    /// The selected frame's own events are being read: the busy line and the
    /// frame pane.
    pub frame_events_reading: &'static str,
    /// The launcher's caption, on the workbench and in the ledger window.
    pub launcher_title: &'static str,
    /// The launcher in plain words, on the workbench: what the Runtime is
    /// doing, or what became of the last request. No key, code, pid or path
    /// goes in these; the ledger window has the technical lines.
    pub launcher_checking: &'static str,
    pub launcher_starting: &'static str,
    pub launcher_running: &'static str,
    pub launcher_not_running: &'static str,
    pub launcher_check_failed: &'static str,
    pub launcher_start_failed: &'static str,
    pub launcher_needs_unlock: &'static str,
    pub launcher_stopping: &'static str,
    pub launcher_stop_requested: &'static str,
    pub launcher_stop_failed: &'static str,
    pub launcher_not_configured: &'static str,
    pub launcher_busy: &'static str,
    /// A plain sentence, then the short words of the outcome code behind it.
    pub launcher_because: &'static str,
    /// After a plain line whose technical words alone say a failure.
    pub launcher_see_ledger: &'static str,
    pub start: &'static str,
    pub request_shutdown: &'static str,
    /// pid, owner epoch.
    pub runtime_running: &'static str,
    /// client error code, client operation.
    pub runtime_not_running: &'static str,
    pub already_running: &'static str,
    /// The OS error of a worker thread that could not be started.
    pub thread_failed: &'static str,
    pub start_busy: &'static str,
    pub shutdown_busy: &'static str,
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
    /// exit code, the developer line of the log's terminal outcome (its last
    /// `OUTCOME actingd:` envelope) or why there is none, log path.
    pub start_exited: &'static str,
    /// pid, owner epoch.
    pub start_ready: &'static str,
    /// attempts, client error code, client operation.
    pub start_not_ready: &'static str,
    /// The start press as a client action: its sequence.
    pub start_recorded: &'static str,
    /// runtime refusal code, client error code, client operation.
    pub start_record_refused: &'static str,
    /// client error code, client operation.
    pub start_record_failed: &'static str,
    pub log_no_outcome: &'static str,
    /// error.
    pub log_unreadable: &'static str,
    /// The unlock entry, offered after a start whose terminal outcome is
    /// `owner_resource_unconfirmed`: its button, the statement the second
    /// press makes, and the button that makes it.
    pub unlock_owner: &'static str,
    pub unlock_statement: &'static str,
    pub unlock_confirm: &'static str,
    pub unlock_busy: &'static str,
    /// actor.
    pub unlock_running: &'static str,
    /// owner epoch, previous resource disposition, revision.
    pub unlock_ok: &'static str,
    /// exit code, owner epoch, previous resource disposition, revision.
    pub unlock_ok_nonzero: &'static str,
    /// outcome code, stage, journal_appended, exit code.
    pub unlock_failed: &'static str,
    /// exit code, the line `last_stderr_line` found on stderr.
    pub unlock_stderr: &'static str,
    /// why, exit code.
    pub unlock_unparseable: &'static str,
    /// JSON pointer.
    pub unlock_field_invalid: &'static str,
    pub unlock_no_output: &'static str,
    /// seconds.
    pub unlock_timeout: &'static str,
    pub unlock_killed: &'static str,
    /// error.
    pub unlock_kill_failed: &'static str,
    /// The wait's error, after a kill that worked.
    pub unlock_unreaped: &'static str,
    /// error.
    pub unlock_output_failed: &'static str,
    pub unlock_spawn_failed: &'static str,
    /// A scheduling pause's line in the ledger window. Its scope: global, or
    /// an instance's (alias). Then scope, reason, revision, since: a pause
    /// another tool set, with the stage of an instance's that cannot be resumed
    /// yet; the one Alice lifts herself; a deployment's, which the installer
    /// lifts.
    pub pause_global: &'static str,
    pub pause_instance: &'static str,
    pub pause_other_tool: &'static str,
    pub pause_draining: &'static str,
    pub pause_releasing: &'static str,
    pub pause_alice: &'static str,
    pub pause_deploy: &'static str,
    /// In place of the pause lines when the status read failed: code (with
    /// its short words), client operation.
    pub pauses_unread: &'static str,
    /// The resume entry of a pause another tool set: its button, the statement
    /// the second press makes, and the button that makes it.
    pub resume_scheduling: &'static str,
    pub resume_statement: &'static str,
    pub resume_confirm: &'static str,
    /// The last resume. scope, owner epoch, revision: out. scope, new
    /// revision: lifted; then the device code of a reconnect that failed, with
    /// its words. scope, runtime refusal code, client error code, client
    /// operation: refused. scope, client error code, client operation: did not
    /// succeed without a refusal code, not sent, not confirmed. scope, why: no
    /// thread for it.
    pub resume_sending: &'static str,
    pub resume_done: &'static str,
    pub resume_device_failed: &'static str,
    pub resume_refused: &'static str,
    pub resume_failed: &'static str,
    pub resume_not_sent: &'static str,
    pub resume_unconfirmed: &'static str,
    pub resume_no_thread: &'static str,
    /// After a resume's line: the status is being read again on a new
    /// connection, or it was. Code (with its short words), client operation:
    /// that read failed. The face reads offline: no status. The face does not
    /// read: nothing was read.
    pub resume_rereading: &'static str,
    pub resume_reread: &'static str,
    pub resume_reread_failed: &'static str,
    pub resume_reread_offline: &'static str,
    pub resume_not_reread: &'static str,
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
    pub instances_none: &'static str,
    pub offline_not_provided: &'static str,
    pub facts_none: &'static str,
    pub facts_no_events: &'static str,
    /// The read face's reason.
    pub facts_not_available: &'static str,
    /// The instance lines' first line when a read failed; the lines below it
    /// say how.
    pub instances_unread: &'static str,
    pub lease_active: &'static str,
    pub lease_cooldown: &'static str,
    pub lease_idle: &'static str,
    /// Lease text, queued request count.
    pub lease_queued: &'static str,
    pub not_registered: &'static str,
    pub fact_unrecorded: &'static str,
    /// Host code, host operation; appended to a client failure.
    pub host_failure: &'static str,
    /// The startup-failure window: its heading, its heading for the usage, its
    /// button, and why a window could not be made (error).
    pub startup_failed: &'static str,
    pub usage_title: &'static str,
    pub close: &'static str,
    pub window_failed: &'static str,
    /// Where the paths come from: error; requested root, selected root; root.
    pub install_selection_failed: &'static str,
    pub state_root_differs: &'static str,
    pub state_root_not_absolute: &'static str,
    /// The read face refused to open at the start: error.
    pub ledger_open_failed: &'static str,
    /// The workbench's red bar when this run has no state root: the original
    /// words of why.
    pub install_issue: &'static str,
    /// The workbench: the way into the ledger window; the instance entry that
    /// waits for the Runtime and what to do meanwhile; the caption of the
    /// configuration file to edit, and what it says when none is set.
    pub open_ledger: &'static str,
    pub add_instance: &'static str,
    pub add_instance_note: &'static str,
    pub config_file: &'static str,
    pub config_path_missing: &'static str,
    /// The workbench refused to close while an unlock runs.
    pub close_unlock_busy: &'static str,
    /// The Aero frame's caption buttons, by the names a screen reader says.
    pub caption_minimize: &'static str,
    pub caption_maximize: &'static str,
    pub caption_restore: &'static str,
    pub caption_close: &'static str,
    /// At the top of an Aero window whose own frame, or whose glass, did not
    /// come up: what the window is drawn with instead. Not said yet: no window
    /// draws a frame of its own so far.
    #[allow(dead_code)]
    pub chrome_failed: &'static str,
    #[allow(dead_code)]
    pub glass_failed: &'static str,
    /// The settings file: the variables its directory comes from; file, error;
    /// file, line, key, value; file, line; file; file, error.
    pub settings_no_dir: &'static str,
    pub settings_unreadable: &'static str,
    pub settings_unknown_value: &'static str,
    pub settings_malformed: &'static str,
    pub settings_save_blocked: &'static str,
    pub settings_save_failed: &'static str,
    /// The technical log: its path; the variables its directory comes from;
    /// its path, error.
    pub log_written: &'static str,
    pub log_no_dir: &'static str,
    pub log_failed: &'static str,
}

pub const ZH: Labels = Labels {
    tongue: Language::Zh,
    window_title: "ActingCommand 监控台",
    ledger_window_title: "高级排障与历史账本",
    usage: "用法：acui [--state-root <状态根>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]\n不给 --state-root 时取 acui.toml 的 state_root；两处都没有就不开台。",
    data_source: "数据来源：本机账本（只读）",
    data_source_online: "数据来源：运行中的 Runtime（只读）",
    ledger_dir: "账本目录",
    storage_format: "存储格式",
    storage_segments: "分段文件",
    storage_sqlite: "SQLite",
    storage_runtime: "由 Runtime 判定",
    read_range: "已读序号 {}–{}",
    performance_hidden: " · 另有 {} 条性能监视例行事件已隐藏",
    show_performance: "显示性能监视",
    jump_latest: "跳到最新",
    follow_latest: "跟随最新",
    source_incomplete: "源不完整",
    read_up_to_unknown: "读取范围未给",
    loaded_rows_top: "已载入 {} 条",
    loaded_badge: "已载入 {}",
    show_up_to: "只看到",
    all: "全部",
    text_size: "字号",
    text_sizes: ["标准", "大", "特大"],
    language: "语言",
    languages: ["中文", "English"],
    theme: "主题",
    theme_options: [
        "现代扁平",
        if cfg!(windows) { "Win7 Aero" } else { "Win7 Aero（实心）" },
    ],
    severity_min: ["严重度不限", "≥ 警告", "≥ 错误", "≥ 致命"],
    severity_max: ["上限不限", "≤ 信息", "≤ 警告", "≤ 错误"],
    all_modules: "全部模块",
    all_instances: "全部实例",
    port_option: "端口 {} ／ {}",
    no_binding_records: "无绑定记录",
    port_online_unsupported: "在线态不支持",
    host: "宿主",
    id_placeholder: "完整的 correlation_ / request_ / run_ / task_ / instance_ 编号",
    continue_reading: "读更早",
    card_title: "实例卡",
    card_note: "账本事实取自读面；条数与级别计数只算已载入的行，不含隐藏的性能监视事件",
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
    levels: ["调试", "成功", "信息", "警告", "错误", "致命"],
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
        ("loaded_rows", "已载入条数"),
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
        ("runtime_instances", "运行时实例"),
        ("instances_status_at", "状态读于序号"),
        ("instances_facts_at", "事实读于序号"),
        ("lease", "租约"),
        ("task_game", "游戏"),
        ("task_server", "区服"),
        ("task_page", "最近识别页面"),
        ("runtime_refusal", "Runtime 拒绝码"),
        ("client_error", "客户端错误"),
        ("host_failure", "宿主失败"),
        ("instance_facts", "实例事实·钉点序号"),
        ("facts_error", "事实读取错误"),
        ("facts_detail", "详情"),
        ("io_kind", "IO 类别"),
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
    integrity_ok_sqlite: "打开时的检查通过（较早的记录读到时再校验）",
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
    source_unopened: "账本未能打开：{}（{}）· {}",
    source_unopened_hint: "状态根里还没有账本，多半是 Runtime 从未在这里启动过；可用启动器的「启动」",
    source_unopened_budget_hint: "离线打开没能在 60 秒内完成（详情里的 phase 指出停在哪一步）：可按「重新打开」再试，或在 Runtime 运行时在线读",
    io_kind_text: " · IO 类别 {}",
    ledger_unopened: "账本未打开",
    list_unopened: "账本未打开：没有读取任何事件，原因见实例卡",
    age_days: "{} 天前",
    age_hours: "{} 小时前",
    age_minutes: "{} 分钟前",
    age_seconds: "{} 秒前",
    frame_size: "画面 {}×{}",
    frame_size_missing: "未记录画面尺寸",
    frame_reading: "正在读取并全量校验 {} 字节…",
    frame_verified: "已校验帧 {}×{} · 整份 sha256 校验通过",
    frame_no_artifact: "本条没有截图帧，只画几何叠加",
    page_unmatched: "未匹配页面",
    frame_step: "步骤 {} {}",
    frame_matched: "识别到 {}",
    frame_unmatched: "未匹配页面（候选 {} 个）",
    frame_targets: "{}（目标 {}/{} 通过）",
    frame_explain_full: "{}：{}，{}",
    frame_explain_short: "{}：{}",
    frame_via_step: "帧取自同一步骤的操作意图 #{}",
    frame_via_input: "帧取自同一运行中前一条操作意图 #{}",
    frame_basis_missing: "已载入的行里找不到它所基于的帧",
    frame_capture_missing: "账本里没有这一帧的截图产物",
    frame_events_failed: "读取这一帧的事件失败：{}",
    action_kinds: &[
        ("tap", "点击"),
        ("long_tap", "长按"),
        ("swipe", "滑动"),
        ("single_touch_drag", "拖动"),
        ("key", "按键"),
        ("text", "输入文字"),
        ("reset", "重置输入"),
    ],
    frame_unavailable: "素材未提供：{}",
    frame_evicted: "材料已淘汰（{}）：意图 #{}{} · 观察至 #{}",
    frame_decode_failed: "PNG 解码失败：{}",
    frame_too_large: "素材 {} 字节超出监控台上限 {} 字节",
    filter_id_error: "编号需填完整的 correlation_ / request_ / run_ / task_ / instance_ 标识",
    filter_instance_conflict: "端口与实例编号只能选一",
    filter_rejected: "过滤条件无效：{}",
    read_failed: "读取失败：{}",
    outcome_words: "{}{}",
    outcome_table_unusable: "结果码说明表不可用：{}",
    outcome_unknown: "这个码不在本界面的说明表里，可能来自更新版本的 Runtime；以上是原码，详情见排障窗口",
    outcome_unknown_short: "未收录的码",
    follow_failed: "跟随最新：{}",
    span_failed: "时间跨度：{}",
    data_source_pending: "数据来源：读面未打开（只读）",
    ledger_opening: "正在打开账本…",
    list_opening: "正在打开账本：还没有读取任何事件",
    ledger_worker_panicked: "读取线程意外终止：{}",
    ledger_worker_gone: "读取线程已经退出",
    ledger_owner_changed: "Runtime 已换成另一个进程：这个窗口读的是已经退出的那一个；按「重新打开」改读现在运行的 Runtime",
    ledger_runtime_started: "Runtime 已经在运行：这个窗口读的是离线账本；按「重新打开」改为在线读取",
    reopen_ledger: "重新打开",
    busy_waited: "{} 已等待 {} 秒（这类读取最长约 {} 秒）",
    busy_waited_page: "{} 已等待 {} 秒（每页最长约 {} 秒）",
    busy_bindings: "正在读取端口映射…",
    busy_span: "正在读取时间跨度…",
    busy_probing: "正在估算时间上界的位置…",
    busy_reading: "正在读取第 {}/{} 窗…",
    busy_repinning: "正在移到账本最新位置…",
    busy_polling: "正在询问账本最新位置…",
    busy_instances: "正在读取实例状态…",
    busy_reconnecting: "正在重建素材连接…",
    frame_events_reading: "正在读取这一帧的事件…",
    launcher_title: "启动器",
    launcher_checking: "正在检查 Runtime…",
    launcher_starting: "正在启动 Runtime…",
    launcher_running: "Runtime 正在运行",
    launcher_not_running: "Runtime 没有运行",
    launcher_check_failed: "没能检查 Runtime 是否在运行",
    launcher_start_failed: "Runtime 没能启动",
    launcher_needs_unlock: "需要解锁：上一个 Runtime 没有正常退出，请在「排障与账本」里处理",
    launcher_stopping: "正在请求 Runtime 停止…",
    launcher_stop_requested: "已请求 Runtime 停止",
    launcher_stop_failed: "停止请求没有成功",
    launcher_not_configured: "启动所需的路径没有配置，请按安装说明填写设置文件",
    launcher_busy: "上一次请求还没完成",
    launcher_because: "{}：{}",
    launcher_see_ledger: "详情见「排障与账本」",
    start: "启动",
    request_shutdown: "请求关闭",
    runtime_running: "Runtime 运行中 · PID {} · owner epoch {}",
    runtime_not_running: "Runtime 未运行 · {}（{}）",
    already_running: "已在运行，未拉起",
    thread_failed: "无法启动后台线程：{}",
    start_busy: "上一次启动还没结束",
    shutdown_busy: "上一次关闭请求还在进行",
    key_not_configured: "acui.toml 未配置 {}",
    key_not_absolute: "acui.toml 的 {} 不是绝对路径：{}",
    log_dir_failed: "无法建立日志目录 {}：{}",
    spawn_failed: "拉起 {} 失败：{}",
    child_status_failed: "读取子进程状态失败：{}",
    start_waiting: "已拉起 PID {} · 等待就绪 {}/{} · 日志 {}",
    start_exited: "actingd 已退出，退出码 {} · {} · 日志 {}",
    start_ready: "Runtime 已就绪 · PID {} · owner epoch {}",
    start_not_ready: "{} 次尝试后仍未就绪 · 最后错误 {}（{}）",
    start_recorded: "动作已记账 #{}",
    start_record_refused: "动作记账被拒：{} · 客户端 {}（{}）",
    start_record_failed: "动作记账失败：{}（{}）",
    log_no_outcome: "日志里没有 OUTCOME actingd: 结果行",
    log_unreadable: "读取日志失败：{}",
    unlock_owner: "解锁 owner…",
    unlock_statement: "确认：上一个 Runtime 的设备资源已经释放",
    unlock_confirm: "确认并解锁",
    unlock_busy: "解锁仍在进行",
    unlock_running: "正在运行 actingd unlock-owner --actor {} --confirm-resources-released…",
    unlock_ok: "已解锁 owner epoch {} · 解锁前处置 {} · owner.lock 修订 {} · 已自动重试启动一次（结果见上一行）",
    unlock_ok_nonzero: "unlock-owner 报 unlock_owner_completed 却以退出码 {} 结束：不算解锁成功，未重试启动 · owner epoch {} · 解锁前处置 {} · owner.lock 修订 {}",
    unlock_failed: "解锁失败：{}（阶段 {}）· journal_appended={} · 退出码 {}",
    unlock_stderr: "unlock-owner 退出码 {} · {}",
    unlock_unparseable: "unlock-owner 输出无法解析：{} · 退出码 {}",
    unlock_field_invalid: "字段 {} 缺失或不合约定",
    unlock_no_output: "stdout 没有 JSON，stderr 也没有输出",
    unlock_timeout: "unlock-owner {} 秒内未结束，结果未知",
    unlock_killed: "已终止并回收",
    unlock_kill_failed: "终止失败：{}",
    unlock_unreaped: "已终止，但回收失败：{}",
    unlock_output_failed: "读取 unlock-owner 输出失败：{} · 退出码 {}",
    unlock_spawn_failed: "运行 unlock-owner（{}）失败：{}",
    pause_global: "全局调度",
    pause_instance: "实例 {} 的调度",
    pause_other_tool: "{}：被其他工具暂停（理由 {}，修订 {}，自 {}）",
    pause_draining: " · 正在等进行中的任务收尾，收尾后才能恢复",
    pause_releasing: " · 正在交还设备，交还后才能恢复",
    pause_alice: "{}：已暂停（理由 {}，修订 {}，自 {}）· 此暂停由 Alice 决定恢复",
    pause_deploy: "{}：部署暂停（理由 {}，修订 {}，自 {}）· 这是安装器加的部署暂停，请用安装器恢复",
    pauses_unread: "状态没读到，暂停情况未知：{}（{}）",
    resume_scheduling: "恢复自动调度…",
    resume_statement: "确认：只恢复这一份暂停；它已有变动时 Runtime 会拒绝",
    resume_confirm: "确认并恢复",
    resume_sending: "正在恢复{}（owner epoch {}，修订 {}）…",
    resume_done: "已恢复{}（新修订 {}）",
    resume_device_failed: " · 设备还没连上：{}",
    resume_refused: "恢复{}没有成功：Runtime 答 {} · 客户端 {}（{}）",
    resume_failed: "恢复{}没有成功：{}（{}）",
    resume_not_sent: "恢复{}的请求没有发出：{}（{}）",
    resume_unconfirmed: "恢复{}的结果未确认：{}（{}）· 没有重发",
    resume_no_thread: "恢复{}的请求没有发出：{}",
    resume_rereading: " · 正在换新连接重读状态…",
    resume_reread: " · 已换新连接重读状态，以上方的暂停行为准",
    resume_reread_failed: " · 换新连接重读状态失败：{}（{}），暂停情况未知，详见实例卡片",
    resume_reread_offline: " · 本窗口现在离线读取，读不到状态，暂停情况未知",
    resume_not_reread: " · 窗口没有在读取，状态没有重读，暂停情况未知",
    shutdown_busy_retry: "请求关闭 · 忙碌重试 {}/{}",
    shutdown_attempts: "第 {}/{} 次",
    shutdown_accepted: "关闭请求已受理 · 回执 {} · 请求 {} · 动作已记账 #{}",
    shutdown_refused: "关闭请求被拒：{} · 客户端 {}（{}）",
    shutdown_failed: "关闭请求失败：{}（{}）",
    instances_none: "无登记实例",
    offline_not_provided: "离线读面不提供",
    facts_none: "无任务事实",
    facts_no_events: "账本里还没有事件",
    facts_not_available: "不可用：{}",
    instances_unread: "读取失败，原因见下",
    lease_active: "占用中",
    lease_cooldown: "接管冷却中",
    lease_idle: "空闲",
    lease_queued: "{} · 排队 {}",
    not_registered: "未登记（仅有任务事实）",
    fact_unrecorded: "未记录",
    host_failure: " · 宿主 {}（{}）",
    startup_failed: "监控台没能启动",
    usage_title: "用法",
    close: "关闭",
    window_failed: "窗口建不出来：{}",
    install_selection_failed: "读取本安装的选择失败：{}",
    state_root_differs: "--state-root 与本安装选定的配置不一致：给的是 {}，安装选定的是 {}",
    state_root_not_absolute: "acui.toml 的 state_root 不是绝对路径：{}",
    ledger_open_failed: "账本读面打不开：{}",
    install_issue: "这次运行没有可用的状态根：「启动」和「排障与账本」暂时不能用，语言和字号仍可修改。\n原因：{}",
    open_ledger: "排障与账本",
    add_instance: "添加实例",
    add_instance_note: "等 Runtime 支持；现在请按安装说明手工编辑配置文件",
    config_file: "配置文件",
    config_path_missing: "还没有配置 Runtime 的配置文件：请按安装说明填写设置文件",
    close_unlock_busy: "解锁还在进行：等它结束再关闭监控台",
    caption_minimize: "最小化",
    caption_maximize: "最大化",
    caption_restore: "还原",
    caption_close: "关闭",
    chrome_failed: "Aero 外框没有生效，这个窗口改用系统标题栏。详情见技术日志。",
    glass_failed: "Aero 玻璃没有生效，这个窗口改用实心外观。详情见技术日志。",
    settings_no_dir: "找不到存放设置文件的目录（{} 未设置）：本次按默认设置运行，改了的设置也存不下来",
    settings_unreadable: "设置文件 {} 读不出：{}。本次按默认设置运行；为免覆盖原文件，本次不保存设置",
    settings_unknown_value: "设置文件 {} 第 {} 行的取值认不出（{} = {}），本次按默认处理；这一行原样保留",
    settings_malformed: "设置文件 {} 第 {} 行不是「键 = 值」的写法，已跳过；这一行原样保留",
    settings_save_blocked: "设置没有保存：设置文件 {} 开台时读不出，为免覆盖原文件，本次不写",
    settings_save_failed: "设置没能保存进 {}：{}",
    log_written: "技术日志：{}",
    log_no_dir: "技术日志写不进去：找不到日志目录（{} 未设置）",
    log_failed: "技术日志写不进去：{}：{}",
};

pub const EN: Labels = Labels {
    tongue: Language::En,
    window_title: "ActingCommand Console",
    ledger_window_title: "Advanced Troubleshooting and Ledger History",
    usage: "Usage: acui [--state-root <state_root>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]\nWithout --state-root the state_root key of acui.toml is used; with neither, the console does not open.",
    data_source: "Data Source: Local Ledger (Read-Only)",
    data_source_online: "Data Source: Running Runtime (Read-Only)",
    ledger_dir: "Ledger Directory",
    storage_format: "Storage Format",
    storage_segments: "Segments",
    storage_sqlite: "SQLite",
    storage_runtime: "Chosen by the Runtime",
    read_range: "Read Positions {}–{}",
    performance_hidden: " · {} Routine Performance-Monitor Events Hidden",
    show_performance: "Show Performance Monitor",
    jump_latest: "Jump to Latest",
    follow_latest: "Follow Latest",
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
    theme: "Theme",
    theme_options: [
        "Modern Flat",
        if cfg!(windows) { "Win7 Aero" } else { "Win7 Aero (Solid)" },
    ],
    severity_min: ["Any Level", "≥ Warning", "≥ Error", "≥ Fatal"],
    severity_max: ["Any Upper Bound", "≤ Info", "≤ Warning", "≤ Error"],
    all_modules: "All Modules",
    all_instances: "All Instances",
    port_option: "Port {} ／ {}",
    no_binding_records: "No Binding Records",
    port_online_unsupported: "Not in Online Mode",
    host: "Host",
    id_placeholder: "Whole correlation_ / request_ / run_ / task_ / instance_ id",
    continue_reading: "Read Earlier",
    card_title: "Instance Card",
    card_note: "Ledger facts come from the read face; row and level counts cover the loaded rows only, without hidden performance-monitor events",
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
    levels: ["Debug", "Success", "Info", "Warning", "Error", "Fatal"],
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
        ("runtime_instances", "Runtime Instances"),
        ("instances_status_at", "Status at Sequence"),
        ("instances_facts_at", "Facts at Sequence"),
        ("lease", "Lease"),
        ("task_game", "Game"),
        ("task_server", "Server"),
        ("task_page", "Last Matched Page"),
        ("runtime_refusal", "Runtime Refusal"),
        ("client_error", "Client Error"),
        ("host_failure", "Host Failure"),
        ("instance_facts", "Instance Facts at Pin"),
        ("facts_error", "Fact Read Error"),
        ("facts_detail", "Detail"),
        ("io_kind", "IO Kind"),
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
    integrity_ok_sqlite: "Passed the opening check (earlier records are verified as they are read)",
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
    source_unopened: "ledger could not be opened: {} ({}) · {}",
    source_unopened_hint: "the state root has no ledger yet, most likely because the Runtime has never been started here; use Start in the launcher",
    source_unopened_budget_hint: "the offline opening did not finish within 60 s (the detail's phase says where it stopped): press Reopen to try again, or read online while the Runtime runs",
    io_kind_text: " · io kind {}",
    ledger_unopened: "Ledger Not Opened",
    list_unopened: "Ledger not opened: no events were read; the instance card says why",
    age_days: "{} days ago",
    age_hours: "{} hours ago",
    age_minutes: "{} minutes ago",
    age_seconds: "{} seconds ago",
    frame_size: "Frame {}×{}",
    frame_size_missing: "Frame Size Not Recorded",
    frame_reading: "Reading and verifying all {} bytes…",
    frame_verified: "Verified frame {}×{} · whole sha256 checked",
    frame_no_artifact: "No capture frame on this event; geometry only",
    page_unmatched: "No page matched",
    frame_step: "Step {} {}",
    frame_matched: "recognized {}",
    frame_unmatched: "no page matched ({} candidates)",
    frame_targets: "{} ({} of {} targets passed)",
    frame_explain_full: "{}: {}, {}",
    frame_explain_short: "{}: {}",
    frame_via_step: "frame taken from the same step's effect intent #{}",
    frame_via_input: "frame taken from the run's previous effect intent #{}",
    frame_basis_missing: "The frame it acted on is not found among the loaded rows",
    frame_capture_missing: "The ledger holds no capture of this frame",
    frame_events_failed: "Reading this frame's events failed: {}",
    action_kinds: &[
        ("tap", "tap"),
        ("long_tap", "long tap"),
        ("swipe", "swipe"),
        ("single_touch_drag", "drag"),
        ("key", "key"),
        ("text", "text"),
        ("reset", "reset"),
    ],
    frame_unavailable: "Material not given: {}",
    frame_evicted: "Material evicted ({}): intent #{}{} · observed through #{}",
    frame_decode_failed: "PNG decode failed: {}",
    frame_too_large: "Material of {} bytes is over the console's limit of {} bytes",
    filter_id_error: "The id must be a whole correlation_ / request_ / run_ / task_ / instance_ identifier",
    filter_instance_conflict: "Pick a port or type an instance id, not both",
    filter_rejected: "Filter rejected: {}",
    read_failed: "Read failed: {}",
    outcome_words: "{} {}",
    outcome_table_unusable: "Outcome words unavailable: {}",
    outcome_unknown: "This code is not in this console's table; it may come from a newer Runtime. The code is shown as is; see the troubleshooting window for details",
    outcome_unknown_short: "code not in this console's table",
    follow_failed: "Following latest: {}",
    span_failed: "Time span: {}",
    data_source_pending: "Data Source: Read Face Not Open (Read-Only)",
    ledger_opening: "Opening the Ledger…",
    list_opening: "Opening the ledger: no events read yet",
    ledger_worker_panicked: "The reader thread stopped unexpectedly: {}",
    ledger_worker_gone: "The reader thread has exited",
    ledger_owner_changed: "The Runtime is another process now: this window reads the one that exited; press Reopen to read the Runtime that runs now",
    ledger_runtime_started: "The Runtime is running now: this window reads the ledger offline; press Reopen to read it online",
    reopen_ledger: "Reopen",
    busy_waited: "{} Waiting {} s (a read of this kind takes at most about {} s)",
    busy_waited_page: "{} Waiting {} s (at most about {} s per page)",
    busy_bindings: "Reading the Port Map…",
    busy_span: "Reading the Time Span…",
    busy_probing: "Estimating Where the Time Bound Falls…",
    busy_reading: "Reading Window {} of {}…",
    busy_repinning: "Moving to the Ledger's Latest Position…",
    busy_polling: "Asking for the Ledger's Latest Position…",
    busy_instances: "Reading the Instances' Status…",
    busy_reconnecting: "Rebuilding the Material Connection…",
    frame_events_reading: "Reading This Frame's Events…",
    launcher_title: "Launcher",
    launcher_checking: "Checking the Runtime…",
    launcher_starting: "Starting the Runtime…",
    launcher_running: "The Runtime Is Running",
    launcher_not_running: "The Runtime Is Not Running",
    launcher_check_failed: "Could Not Check Whether the Runtime Is Running",
    launcher_start_failed: "The Runtime Did Not Start",
    launcher_needs_unlock: "Unlock Needed: the Last Runtime Did Not Exit Cleanly; Handle It in Troubleshooting and Ledger",
    launcher_stopping: "Asking the Runtime to Stop…",
    launcher_stop_requested: "The Runtime Was Asked to Stop",
    launcher_stop_failed: "The Stop Request Did Not Go Through",
    launcher_not_configured: "A Path the Start Needs Is Not Set; Fill In the Settings File as the Installation Guide Says",
    launcher_busy: "The Previous Request Has Not Finished",
    launcher_because: "{}: {}",
    launcher_see_ledger: "Details in Troubleshooting and Ledger",
    start: "Start",
    request_shutdown: "Request Shutdown",
    runtime_running: "Runtime Running · PID {} · Owner Epoch {}",
    runtime_not_running: "Runtime Not Running · {} ({})",
    already_running: "Already Running, Nothing Started",
    thread_failed: "Starting a Background Thread Failed: {}",
    start_busy: "The Previous Start Has Not Finished",
    shutdown_busy: "The Previous Shutdown Request Is Still Running",
    key_not_configured: "{} Not Configured in acui.toml",
    key_not_absolute: "{} in acui.toml Is Not an Absolute Path: {}",
    log_dir_failed: "Cannot Create Log Directory {}: {}",
    spawn_failed: "Starting {} Failed: {}",
    child_status_failed: "Reading the Child's Status Failed: {}",
    start_waiting: "Started PID {} · Waiting for Readiness {}/{} · Log {}",
    start_exited: "actingd Exited with Code {} · {} · Log {}",
    start_ready: "Runtime Ready · PID {} · Owner Epoch {}",
    start_not_ready: "Not Ready After {} Attempts · Last Error {} ({})",
    start_recorded: "Action Recorded at #{}",
    start_record_refused: "Action Record Refused: {} · Client {} ({})",
    start_record_failed: "Action Record Failed: {} ({})",
    log_no_outcome: "No OUTCOME actingd: Line in the Log",
    log_unreadable: "Reading the Log Failed: {}",
    unlock_owner: "Unlock Owner…",
    unlock_statement: "Confirm: the Device Resources of the Last Runtime Are Released",
    unlock_confirm: "Confirm and Unlock",
    unlock_busy: "The Unlock Is Still Running",
    unlock_running: "Running actingd unlock-owner --actor {} --confirm-resources-released…",
    unlock_ok: "Unlocked Owner Epoch {} · Disposition Before {} · owner.lock Revision {} · Start Retried Once (See the Line Above)",
    unlock_ok_nonzero: "unlock-owner Said unlock_owner_completed but Exited with Code {}: Not Taken as Unlocked, Start Not Retried · Owner Epoch {} · Disposition Before {} · owner.lock Revision {}",
    unlock_failed: "Unlock Failed: {} (Stage {}) · journal_appended={} · Exit Code {}",
    unlock_stderr: "unlock-owner Exited with Code {} · {}",
    unlock_unparseable: "unlock-owner Output Unparseable: {} · Exit Code {}",
    unlock_field_invalid: "Field {} Missing or Off-Contract",
    unlock_no_output: "No JSON on stdout, No Output on stderr",
    unlock_timeout: "unlock-owner Not Finished Within {} s, Outcome Unknown",
    unlock_killed: "Killed and Reaped",
    unlock_kill_failed: "Killing It Failed: {}",
    unlock_unreaped: "Killed, but Reaping It Failed: {}",
    unlock_output_failed: "Reading unlock-owner Output Failed: {} · Exit Code {}",
    unlock_spawn_failed: "Running unlock-owner ({}) Failed: {}",
    pause_global: "Global Scheduling",
    pause_instance: "Scheduling of Instance {}",
    pause_other_tool: "{}: Paused by Another Tool (Reason {}, Revision {}, Since {})",
    pause_draining: " · Waiting for the Running Tasks to Finish; It Can Be Resumed After That",
    pause_releasing: " · Handing the Device Back; It Can Be Resumed After That",
    pause_alice: "{}: Paused (Reason {}, Revision {}, Since {}) · Alice Decides When This Pause Is Lifted",
    pause_deploy: "{}: Deployment Pause (Reason {}, Revision {}, Since {}) · The Installer Set It; Lift It with the Installer",
    pauses_unread: "The Status Was Not Read; Whether Scheduling Is Paused Is Unknown: {} ({})",
    resume_scheduling: "Resume Scheduling…",
    resume_statement: "Confirm: Resume This One Pause; the Runtime Refuses If It Has Changed",
    resume_confirm: "Confirm and Resume",
    resume_sending: "Resuming {} (Owner Epoch {}, Revision {})…",
    resume_done: "Resumed {} (New Revision {})",
    resume_device_failed: " · The Device Is Not Connected Yet: {}",
    resume_refused: "Resuming {} Did Not Succeed: the Runtime Answered {} · Client {} ({})",
    resume_failed: "Resuming {} Did Not Succeed: {} ({})",
    resume_not_sent: "The Resume of {} Was Not Sent: {} ({})",
    resume_unconfirmed: "Whether {} Was Resumed Is Not Confirmed: {} ({}) · Not Sent Again",
    resume_no_thread: "The Resume of {} Was Not Sent: {}",
    resume_rereading: " · Reading the Status Again on a New Connection…",
    resume_reread: " · Status Read Again on a New Connection; the Pause Lines Above Are Current",
    resume_reread_failed: " · Reading the Status Again on a New Connection Failed: {} ({}); Whether It Is Paused Is Unknown, See the Instance Card",
    resume_reread_offline: " · This Window Now Reads Offline: No Status Can Be Read, and Whether It Is Paused Is Unknown",
    resume_not_reread: " · The Window Is Not Reading: the Status Was Not Read Again, and Whether It Is Paused Is Unknown",
    shutdown_busy_retry: "Request Shutdown · Busy, Retry {}/{}",
    shutdown_attempts: "Attempt {}/{}",
    shutdown_accepted: "Shutdown Accepted · Receipt {} · Request {} · Action Recorded at #{}",
    shutdown_refused: "Shutdown Refused: {} · Client {} ({})",
    shutdown_failed: "Shutdown Request Failed: {} ({})",
    instances_none: "No Registered Instance",
    offline_not_provided: "Not Provided Offline",
    facts_none: "No Task Fact",
    facts_no_events: "No Event in the Ledger Yet",
    facts_not_available: "Not Available: {}",
    instances_unread: "Not Read, See Below",
    lease_active: "Leased",
    lease_cooldown: "Takeover Cooldown",
    lease_idle: "Idle",
    lease_queued: "{} · {} Queued",
    not_registered: "Not Registered (Task Facts Only)",
    fact_unrecorded: "Not Recorded",
    host_failure: " · Host {} ({})",
    startup_failed: "The Console Could Not Start",
    usage_title: "Usage",
    close: "Close",
    window_failed: "A window could not be created: {}",
    install_selection_failed: "Reading this installation's selection failed: {}",
    state_root_differs: "--state-root differs from this installation's selected configuration: given {}, selected {}",
    state_root_not_absolute: "state_root in acui.toml is not an absolute path: {}",
    ledger_open_failed: "The ledger's read face could not be opened: {}",
    install_issue: "This run has no state root to use: Start and Troubleshooting and Ledger are off for now, and the language and the text size still work.\nWhy: {}",
    open_ledger: "Troubleshooting and Ledger",
    add_instance: "Add Instance",
    add_instance_note: "Waits for Runtime support; for now, edit the configuration file by hand as the installation guide says",
    config_file: "Configuration File",
    config_path_missing: "No Runtime configuration file is set: fill in the settings file as the installation guide says",
    close_unlock_busy: "The unlock is still running: close the console once it has ended",
    caption_minimize: "Minimize",
    caption_maximize: "Maximize",
    caption_restore: "Restore",
    caption_close: "Close",
    chrome_failed: "The Aero frame did not take effect: this window uses the system title bar instead. The technical log has the details.",
    glass_failed: "The Aero glass did not take effect: this window uses a solid look instead. The technical log has the details.",
    settings_no_dir: "No directory for the settings file ({} not set): this run uses the default settings, and a changed setting cannot be saved",
    settings_unreadable: "The settings file {} could not be read: {}. This run uses the default settings and saves none, so the file is never written over",
    settings_unknown_value: "The settings file {}, line {}: the value is not recognized ({} = {}); the default is used, and the line is kept as it is",
    settings_malformed: "The settings file {}, line {}: not of the form key = value; skipped, and the line is kept as it is",
    settings_save_blocked: "Settings not saved: the settings file {} could not be read at the start, so this run does not write it",
    settings_save_failed: "Saving the settings to {} failed: {}",
    log_written: "Technical log: {}",
    log_no_dir: "The technical log cannot be written: no log directory ({} not set)",
    log_failed: "The technical log cannot be written: {}: {}",
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

/// What a person reads for an outcome code, in the table's language: its
/// sentence, then its handling when it has one. A code neither the UI's table
/// nor the pinned Runtime's catalog holds (a newer Runtime's, say) gets the
/// unknown-code sentence, never nothing; an outcome table that does not parse
/// is said, not dropped.
pub fn outcome_sentence(labels: &Labels, code: &str) -> Option<String> {
    match explain(code, labels.tongue.outcome_tongue()) {
        Ok(Some(words)) => Some(match words.handling {
            Some(handling) => fill(labels.outcome_words, &[words.sentence, handling]),
            None => words.sentence.to_owned(),
        }),
        Ok(None) => Some(labels.outcome_unknown.to_owned()),
        Err(error) => Some(fill(labels.outcome_table_unusable, &[&error.to_string()])),
    }
}

/// An outcome code's short form, for a line that does not wrap; unknown codes
/// and errors as `outcome_sentence`.
pub fn outcome_short(labels: &Labels, code: &str) -> Option<String> {
    match explain(code, labels.tongue.outcome_tongue()) {
        Ok(Some(words)) => Some(words.short.to_owned()),
        Ok(None) => Some(labels.outcome_unknown_short.to_owned()),
        Err(error) => Some(fill(labels.outcome_table_unusable, &[&error.to_string()])),
    }
}
