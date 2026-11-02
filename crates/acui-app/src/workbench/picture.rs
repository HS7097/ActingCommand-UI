// SPDX-License-Identifier: GPL-3.0-only
//! The live view in the workbench's right column (UI-3c): which account's picture it
//! shows, and what the picture area says, as a pure function of that. No words and no
//! Slint here: `paint` makes both of a view.
//!
//! The account is the one a person picked on its card, or that `--account` named at the
//! start; until then, and once the one picked is no longer among the accounts a `Status`
//! lists, the first card's. Which one is picked is not kept past this run.

use std::mem;

/// Which account's picture shows.
#[derive(Default)]
pub struct Pictures {
    /// Picked on its card, or named by `--account`; else the first card's, once a
    /// `Status` lists any account.
    selected: Option<String>,
    /// `--account` named `selected`, and no `Status` has listed any account since.
    named: bool,
}

impl Pictures {
    /// `--account`, before the window is up.
    pub fn name(&mut self, alias: String) {
        self.selected = Some(alias);
        self.named = true;
    }

    /// `alias` picked on its card.
    pub fn pick(&mut self, alias: &str) {
        self.named = false;
        if self.selected.as_deref() != Some(alias) {
            self.selected = Some(alias.to_owned());
        }
    }

    /// The account shown of the cards' `aliases`, in their order: the one picked while it
    /// is among them, else the first; none while there is no card.
    pub fn shown<'a>(&self, aliases: &[&'a str]) -> Option<&'a str> {
        let picked = self.selected.as_deref();
        let found =
            aliases.iter().copied().find(|alias| picked.is_some_and(|picked| picked == *alias));
        found.or_else(|| aliases.first().copied())
    }

    /// A `Status` listed the accounts `aliases`: a pick that is none of them, or no pick,
    /// goes to the first, which stays picked from then on. With no account listed nothing
    /// changes. What `--account` named, when no account listed has that alias, for the log.
    pub fn settle(&mut self, aliases: &[&str]) -> Option<String> {
        let first = *aliases.first()?;
        let named = mem::take(&mut self.named);
        let picked = self.selected.as_deref();
        if picked.is_some_and(|picked| aliases.iter().any(|alias| *alias == picked)) {
            return None;
        }
        let missed = self.selected.replace(first.to_owned());
        missed.filter(|_| named)
    }
}

/// What the picture area shows, before its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureView {
    /// There is no card to show the picture of.
    NoAccount,
    /// No frame shows, and why.
    Empty { why: EmptyWhy },
}

/// Why no frame shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyWhy {
    /// The account's latest frame is being looked for.
    Looking,
}

/// The picture area of `account`, the one `Pictures::shown` gave, or of none.
pub fn picture_view(account: Option<&str>) -> PictureView {
    match account {
        None => PictureView::NoAccount,
        Some(_) => PictureView::Empty { why: EmptyWhy::Looking },
    }
}
