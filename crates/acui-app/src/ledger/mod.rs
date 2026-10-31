// SPDX-License-Identifier: GPL-3.0-only
//! The ledger window's own side, apart from the read face: what it shows
//! (`state`), how that is painted, with every word of it (`paint`), and its
//! frame reads (`frame`). The read face and the reads stay with the caller,
//! which leaves their results in the state before the window is painted.

pub mod frame;
pub mod paint;
pub mod state;

use std::cell::RefCell;

use acui_rows::OriginModule;
use slint::SharedString;

use frame::FrameSlot;
use state::LedgerState;

/// One ledger window's view of its read face: the state it is painted from,
/// its frame reads, and the module option list as the box currently shows
/// it, with what each entry after the first one means, rebuilt from every
/// page.
pub struct LedgerHandle {
    pub state: RefCell<LedgerState>,
    pub frame: FrameSlot,
    pub module_options: RefCell<Vec<SharedString>>,
    pub module_choices: RefCell<Vec<OriginModule>>,
}

impl LedgerHandle {
    pub fn new(state: LedgerState) -> Self {
        Self {
            state: RefCell::new(state),
            frame: FrameSlot::default(),
            module_options: RefCell::new(Vec::new()),
            module_choices: RefCell::new(Vec::new()),
        }
    }
}
