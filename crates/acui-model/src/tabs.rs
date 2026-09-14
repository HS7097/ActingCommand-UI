// SPDX-License-Identifier: AGPL-3.0-only
//! The six tabs are the contract's six `LedgerView`s. The console does not
//! classify anything: a row belongs to a tab when the page says it does.

use acui_rows::LedgerView;

pub const ALL_TABS: [LedgerView; 6] = LedgerView::ALL;

pub const fn tab_label(view: LedgerView) -> &'static str {
    match view {
        LedgerView::Events => "事件流",
        LedgerView::Observation => "观察",
        LedgerView::Changes => "变更",
        LedgerView::Errors => "错误",
        LedgerView::Health => "运行状况",
        LedgerView::Lab => "Lab",
    }
}

/// The `--tab` spelling, which is the view's own wire name.
pub const fn tab_name(view: LedgerView) -> &'static str {
    match view {
        LedgerView::Events => "events",
        LedgerView::Observation => "observation",
        LedgerView::Changes => "changes",
        LedgerView::Errors => "errors",
        LedgerView::Health => "health",
        LedgerView::Lab => "lab",
    }
}

pub fn tab_from_name(name: &str) -> Option<LedgerView> {
    ALL_TABS.into_iter().find(|view| tab_name(*view) == name)
}
