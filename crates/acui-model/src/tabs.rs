// SPDX-License-Identifier: AGPL-3.0-only
//! The six view tabs. 事件流 and 错误 are ruling-defined; the other four are
//! provisional classifications by event_type prefix and origin.module, kept
//! until the row contract lands.

use acui_rows::{EventRow, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTab {
    EventStream,
    Errors,
    ObserveAndAct,
    Changes,
    Health,
    Lab,
}

pub const ALL_TABS: [ViewTab; 6] = [
    ViewTab::EventStream,
    ViewTab::Errors,
    ViewTab::ObserveAndAct,
    ViewTab::Changes,
    ViewTab::Health,
    ViewTab::Lab,
];

impl ViewTab {
    pub const fn label(self) -> &'static str {
        match self {
            Self::EventStream => "事件流",
            Self::Errors => "错误",
            Self::ObserveAndAct => "观察与操作",
            Self::Changes => "变更",
            Self::Health => "运行状况",
            Self::Lab => "Lab",
        }
    }

    /// Provisional tabs carry a visible marker; the two ruling-defined tabs do not.
    pub const fn is_provisional(self) -> bool {
        !matches!(self, Self::EventStream | Self::Errors)
    }

    pub fn membership(self, row: &EventRow) -> bool {
        let event_type = row.event_type.as_str();
        let module = row.origin.module.as_str();
        match self {
            Self::EventStream => true,
            Self::Errors => row.severity >= Severity::Warning,
            Self::Lab => {
                matches!(module, "actinglab" | "actingctl")
                    || starts_with_any(event_type, &["lab.", "cli."])
            }
            Self::ObserveAndAct => {
                matches!(module, "capture" | "capture-pipeline" | "recognition" | "device-proxy")
                    || starts_with_any(event_type, &["capture.", "recognition.", "input."])
            }
            Self::Changes => {
                matches!(module, "artifact-store" | "scheduler")
                    || starts_with_any(
                        event_type,
                        &["task.", "artifact.", "command.", "lease.", "scheduler."],
                    )
            }
            Self::Health => {
                matches!(module, "performance-monitor")
                    || starts_with_any(event_type, &["perf.", "runtime."])
            }
        }
    }
}

fn starts_with_any(event_type: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| event_type.starts_with(prefix))
}
