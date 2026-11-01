// SPDX-License-Identifier: GPL-3.0-only
//! The theme on screen: the skin each window's own `Look` is drawn in, set
//! before the window shows and again in every window whenever a theme is
//! picked. The theme itself is the shared state's (`Ui::theme`); what a skin
//! looks like is the kit's (`ui/kit/tokens.slint`); what this platform can draw
//! of the Aero frame is the native layer's (`acui_glass`).

use acui_glass::Capability;

use crate::settings::Theme;
use crate::ui::Ui;
use crate::{Look, Skin};

/// The windows' looks. A window's skin is held by that window's `Look`; this
/// holds what is the same for all of them.
pub struct Skins {
    capability: Capability,
}

impl Skins {
    /// Asks the native layer what this platform can draw, once for the run.
    pub fn detect() -> Self {
        Self { capability: acui_glass::capability() }
    }

    /// A theme picked while the window is open: its look changes on the spot.
    pub fn switch(&self, look: Look<'_>, theme: Theme) {
        look.set_skin(skin(theme));
        look.set_chrome(theme == Theme::Aero && self.capability == Capability::Glass);
    }
}

/// A new window's look, before it shows: the theme of this run as it stands.
pub fn apply_before_show(look: Look<'_>, ui: &Ui) {
    ui.skins.switch(look, ui.theme.get());
}

/// The skin a theme is drawn in.
fn skin(theme: Theme) -> Skin {
    match theme {
        Theme::Flat => Skin::Flat,
        Theme::Aero => Skin::Aero,
    }
}
