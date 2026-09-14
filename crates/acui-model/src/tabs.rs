// SPDX-License-Identifier: AGPL-3.0-only
//! The six tabs are the contract's six `LedgerView`s. The console does not
//! classify anything: a row belongs to a tab when the page says it does. The
//! names a person reads live in `acui-app`; only the wire names are here.

use acui_rows::LedgerView;

pub const ALL_TABS: [LedgerView; 6] = LedgerView::ALL;

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
