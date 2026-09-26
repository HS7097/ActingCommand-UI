// SPDX-License-Identifier: GPL-3.0-only
//! Display-only names for the two closed vocabularies a row shows: the event
//! type and the origin module. These are names for a human, never a fact: a
//! wire string this table does not know keeps its raw spelling, and the raw
//! spelling is shown next to the name either way.

/// `(wire, 中文, English)` for every `EventType` the contract declares.
const EVENT_TYPES: [(&str, &str, &str); 121] = [
    ("provider.startup_observed", "厂商启动已观察", "Provider Startup Observed"),
    ("runtime.started", "运行时已启动", "Runtime Started"),
    ("runtime.takeover", "运行时已接管", "Runtime Takeover"),
    ("runtime.failed", "运行时失败", "Runtime Failed"),
    ("runtime.lifecycle_observed", "运行时生命周期已观察", "Runtime Lifecycle Observed"),
    ("runtime.instance_bound", "运行时实例已绑定", "Runtime Instance Bound"),
    ("runtime.fact_recorded", "运行时事实已记录", "Runtime Fact Recorded"),
    ("runtime.fact_invalidated", "运行时事实已作废", "Runtime Fact Invalidated"),
    ("runtime.fact_snapshot", "运行时事实快照", "Runtime Fact Snapshot"),
    ("monitor.probe_requested", "探针已请求", "Monitor Probe Requested"),
    ("monitor.probe_started", "探针已开始", "Monitor Probe Started"),
    ("monitor.probe_completed", "探针已完成", "Monitor Probe Completed"),
    ("monitor.probe_failed", "探针失败", "Monitor Probe Failed"),
    ("monitor.recovery_admitted", "恢复已准入", "Monitor Recovery Admitted"),
    ("monitor.recovery_deferred", "恢复已推迟", "Monitor Recovery Deferred"),
    ("perf.pressure_started", "性能压力开始", "Performance Pressure Started"),
    ("perf.pressure_ended", "性能压力结束", "Performance Pressure Ended"),
    ("perf.stutter_detected", "检测到卡顿", "Stutter Detected"),
    ("perf.summary", "性能小结", "Performance Summary"),
    ("perf.monitor_degraded", "性能监视已降级", "Performance Monitor Degraded"),
    ("perf.monitor_recovered", "性能监视已恢复", "Performance Monitor Recovered"),
    ("perf.balance_changed", "性能取舍已调整", "Performance Balance Changed"),
    ("fact.published", "事实已发布", "Fact Published"),
    ("fact.invalidated", "事实已作废", "Fact Invalidated"),
    ("approval.decision", "审批裁定", "Approval Decision"),
    ("command.received", "命令已收到", "Command Received"),
    ("command.validated", "命令已通过校验", "Command Validated"),
    ("command.rejected", "命令被拒", "Command Rejected"),
    ("scheduler.admitted", "调度已准入", "Scheduler Admitted"),
    ("scheduler.queued", "调度已排队", "Scheduler Queued"),
    ("scheduler.denied", "调度被拒", "Scheduler Denied"),
    ("scheduler.preempted", "调度被抢占", "Scheduler Preempted"),
    ("policy.dispatch_intent", "策略派发意图", "Policy Dispatch Intent"),
    ("policy.dispatch_admitted", "策略派发已准入", "Policy Dispatch Admitted"),
    ("policy.dispatch_rejected", "策略派发被拒", "Policy Dispatch Rejected"),
    ("policy.dispatch_completed", "策略派发已完成", "Policy Dispatch Completed"),
    ("policy.execution_recorded", "策略执行已记录", "Policy Execution Recorded"),
    ("policy.planning_signal_observed", "策略规划信号已观察", "Policy Planning Signal Observed"),
    ("catalog.transition_intent", "目录切换意图", "Catalog Transition Intent"),
    ("catalog.activated", "目录已启用", "Catalog Activated"),
    ("catalog.rolled_back", "目录已回滚", "Catalog Rolled Back"),
    ("catalog.transition_failed", "目录切换失败", "Catalog Transition Failed"),
    ("lease.requested", "租约已申请", "Lease Requested"),
    ("lease.granted", "租约已授予", "Lease Granted"),
    ("lease.transferred", "租约已转移", "Lease Transferred"),
    ("lease.renewed", "租约已续期", "Lease Renewed"),
    ("lease.released", "租约已释放", "Lease Released"),
    ("lease.expired", "租约已过期", "Lease Expired"),
    ("lease.transition_intent", "租约变更意图", "Lease Transition Intent"),
    ("lease.transition_failed", "租约变更失败", "Lease Transition Failed"),
    ("task.requested", "任务已申请", "Task Requested"),
    ("task.started", "任务已开始", "Task Started"),
    ("task.step_started", "步骤已开始", "Task Step Started"),
    ("task.evidence_indexed", "证据已入索引", "Task Evidence Indexed"),
    ("task.geometry_observed", "画面几何已观察", "Task Geometry Observed"),
    ("task.recognition_started", "识别已开始", "Task Recognition Started"),
    ("task.recognition_completed", "识别已完成", "Task Recognition Completed"),
    ("task.entry_preflight", "入口预检", "Task Entry Preflight"),
    ("task.effect_intent", "准备执行操作", "Effect Intent"),
    ("task.effect_completed", "操作已完成", "Effect Completed"),
    ("task.step_finished", "步骤已结束", "Task Step Finished"),
    ("task.completed", "任务已完成", "Task Completed"),
    ("task.failed", "任务失败", "Task Failed"),
    ("task.cancelled", "任务已取消", "Task Cancelled"),
    ("task.terminal_intent", "收尾意图", "Task Terminal Intent"),
    ("task.terminal_commit_failed", "收尾提交失败", "Task Terminal Commit Failed"),
    ("task.terminal_rejected", "收尾被拒", "Task Terminal Rejected"),
    ("application.intent", "应用操作意图", "Application Intent"),
    ("application.completed", "应用操作已完成", "Application Completed"),
    ("application.failed", "应用操作失败", "Application Failed"),
    ("input.intent", "输入意图", "Input Intent"),
    ("input.committed", "输入已提交", "Input Committed"),
    ("input.completed", "输入已完成", "Input Completed"),
    ("input.failed", "输入失败", "Input Failed"),
    ("capture.requested", "截图已请求", "Capture Requested"),
    ("capture.completed", "截图已完成", "Capture Completed"),
    ("capture.failed", "截图失败", "Capture Failed"),
    ("capture.pressure_changed", "截图压力已变化", "Capture Pressure Changed"),
    ("capture.dedup_window", "截图去重窗口", "Capture Dedup Window"),
    ("capture.policy_changed", "截图策略已变化", "Capture Policy Changed"),
    ("capture.summary_committed", "截图小结已提交", "Capture Summary Committed"),
    ("recognition.requested", "识别已请求", "Recognition Requested"),
    ("recognition.completed", "识别已完成", "Recognition Completed"),
    ("recognition.failed", "识别失败", "Recognition Failed"),
    ("artifact.pin_recorded", "材料已钉住", "Artifact Pin Recorded"),
    ("artifact.pin_released", "材料钉住已解除", "Artifact Pin Released"),
    ("artifact.eviction_intent", "材料淘汰意图", "Artifact Eviction Intent"),
    ("artifact.eviction_outcome", "材料淘汰结果", "Artifact Eviction Outcome"),
    ("artifact.created", "材料已写入", "Artifact Created"),
    ("artifact.verified", "材料已校验", "Artifact Verified"),
    ("artifact.store_failed", "材料写入失败", "Artifact Store Failed"),
    ("artifact.verification_failed", "材料校验失败", "Artifact Verification Failed"),
    ("artifact.export_completed", "材料导出已完成", "Artifact Export Completed"),
    ("artifact.export_failed", "材料导出失败", "Artifact Export Failed"),
    ("resource.authoring_started", "资源编写已开始", "Resource Authoring Started"),
    ("resource.draft_built", "资源草稿已构建", "Resource Draft Built"),
    ("resource.validation_completed", "资源校验已完成", "Resource Validation Completed"),
    ("resource.promote_intent", "资源发布意图", "Resource Promote Intent"),
    ("resource.promoted", "资源已发布", "Resource Promoted"),
    ("resource.promote_failed", "资源发布失败", "Resource Promote Failed"),
    ("ui.action", "界面操作", "UI Action"),
    ("client.action", "客户端操作", "Client Action"),
    ("governance.identity_declared", "治理身份已声明", "Governance Identity Declared"),
    ("cli.command", "命令行命令", "CLI Command"),
    ("lab.request", "Lab 请求", "Lab Request"),
    ("state.migrated", "状态已迁移", "State Migrated"),
    ("release.staged", "版本已预备", "Release Staged"),
    ("release.transition_intent", "版本切换意图", "Release Transition Intent"),
    ("release.activated", "版本已启用", "Release Activated"),
    ("release.rolled_back", "版本已回滚", "Release Rolled Back"),
    ("release.transition_failed", "版本切换失败", "Release Transition Failed"),
    ("agent.wake_requested", "代理唤醒已请求", "Agent Wake Requested"),
    ("agent.session_started", "代理会话已开始", "Agent Session Started"),
    ("agent.session_resumed", "代理会话已续接", "Agent Session Resumed"),
    ("agent.response_recorded", "代理回复已记录", "Agent Response Recorded"),
    ("agent.session_completed", "代理会话已完成", "Agent Session Completed"),
    ("agent.session_escalated", "代理会话已上报", "Agent Session Escalated"),
    ("ledger.recovered", "账本已恢复", "Ledger Recovered"),
    ("signature.registered", "特征已登记", "Signature Registered"),
    ("signature.matched", "特征已匹配", "Signature Matched"),
    ("signature.retired", "特征已退役", "Signature Retired"),
];

/// `(wire, 中文, English)` for every `OriginModule` the contract declares.
const MODULES: [(&str, &str, &str); 20] = [
    ("provider", "厂商", "Provider"),
    ("actingctl", "命令行工具", "Actingctl"),
    ("actinglab", "实验台", "Actinglab"),
    ("runtime", "运行时", "Runtime"),
    ("scheduler", "调度器", "Scheduler"),
    ("policy", "策略", "Policy"),
    ("device-proxy", "设备代理", "Device Proxy"),
    ("capture", "截图", "Capture"),
    ("capture-pipeline", "截图流水线", "Capture Pipeline"),
    ("recognition", "识别", "Recognition"),
    ("resource-tooling", "资源工具", "Resource Tooling"),
    ("artifact-store", "材料库", "Artifact Store"),
    ("evidence-exporter", "证据导出", "Evidence Exporter"),
    ("global-ledger", "全局账本", "Global Ledger"),
    ("performance-monitor", "性能监视", "Performance Monitor"),
    ("fact-store", "事实库", "Fact Store"),
    ("runtime-facts", "运行时事实", "Runtime Facts"),
    ("governance", "治理", "Governance"),
    ("agent-dispatcher", "代理调度", "Agent Dispatcher"),
    ("process-test", "流程测试", "Process Test"),
];

/// `(中文, English)`, or `None` when the table does not know this wire string.
pub fn event_type_names(wire: &str) -> Option<(&'static str, &'static str)> {
    lookup(&EVENT_TYPES, wire)
}

/// `(中文, English)`, or `None` when the table does not know this wire string.
pub fn module_names(wire: &str) -> Option<(&'static str, &'static str)> {
    lookup(&MODULES, wire)
}

fn lookup(table: &[(&str, &'static str, &'static str)], wire: &str) -> Option<(&'static str, &'static str)> {
    table
        .iter()
        .find(|(raw, _, _)| *raw == wire)
        .map(|(_, zh, en)| (*zh, *en))
}

/// `3.5 MB` — the size band a human reads, next to the exact byte count.
pub fn format_bytes(byte_count: u64) -> String {
    let value = byte_count as f64;
    if value >= 1_000_000.0 {
        format!("{:.1} MB", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1} KB", value / 1_000.0)
    } else {
        format!("{byte_count} B")
    }
}
