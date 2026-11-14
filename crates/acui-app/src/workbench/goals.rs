// SPDX-License-Identifier: GPL-3.0-only
//! What an account's Goals tab says (Workflow #455 T1-C, step 1, read only), as the reads leave
//! it: no words and no Slint here, `paint` makes both of a view. Its rows are
//! `acui_source::workbench::goals::page`'s, a pure function of the account's `ResourceTargetView`
//! and the one place that decides them; this says only why there are none.
//!
//! The tab writes nothing: setting a goal here is the write half's (T1-C's second part), and the
//! Lab (its command line and its MCP) sets them meanwhile, which the tab says in its own words.

use acui_source::workbench::goals::GoalsPage;
use acui_source::ClientFailure;

use super::card::Down;
use super::model::Fault;

/// What the Goals tab shows now: why no row does, or its rows. A failure is said with its codes,
/// never as an empty page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalsView {
    /// No Runtime answers now.
    Down(Down),
    NoAccount,
    /// The tab shows anew, of another account or another owner epoch: no answer yet.
    Reading,
    /// The read failed: its failure, whose codes the tab says.
    Failed(ClientFailure),
    /// The read panicked, or the reading thread is gone.
    Fault(Fault),
    /// Nothing new came for too long while the window shows.
    Late,
    /// This Runtime has no such read: the code that says so.
    Unsupported(&'static str),
    Read(GoalsPage),
}
