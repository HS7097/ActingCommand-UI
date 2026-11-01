// SPDX-License-Identifier: GPL-3.0-only
//! The theme on screen: the skin each window's own `Look` is drawn in, set
//! before the window shows and again in every window whenever a theme is
//! picked. The theme itself is the shared state's (`Ui::theme`); what a skin
//! looks like is the kit's (`ui/kit/tokens.slint`).

use crate::settings::Theme;
use crate::ui::Ui;
use crate::{Look, Skin};

/// The windows' looks. It holds nothing of its own yet: a window's skin is
/// held by that window's `Look`.
pub struct Skins;

impl Skins {
    /// A theme picked while the window is open: its look changes on the spot.
    pub fn switch(&self, look: Look<'_>, theme: Theme) {
        look.set_skin(skin(theme));
    }
}

/// A new window's look, before it shows: the theme of this run as it stands.
pub fn apply_before_show(look: Look<'_>, ui: &Ui) {
    look.set_skin(skin(ui.theme.get()));
}

/// The skin a theme is drawn in.
fn skin(theme: Theme) -> Skin {
    match theme {
        Theme::Flat => Skin::Flat,
        Theme::Aero => Skin::Aero,
    }
}
