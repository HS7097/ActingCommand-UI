// SPDX-License-Identifier: GPL-3.0-only
//! Words said later. A worker thread never holds a language table: what it has
//! to say goes back as a `Phrase`, a closure over the facts it found, and turns
//! into text only on the event loop, with the table of the language shown at
//! that moment. So a language switch reaches a line a thread wrote before it.

use std::sync::Arc;

use crate::strings::Labels;

/// Text still to be put into words: called with the table of the language
/// shown now.
pub type Phrase = Arc<dyn Fn(&Labels) -> String + Send + Sync>;

/// A `Phrase` from a closure over owned facts.
pub fn phrase(words: impl Fn(&Labels) -> String + Send + Sync + 'static) -> Phrase {
    Arc::new(words)
}

/// Words that are the same in every language: another program's own text,
/// shown verbatim.
pub fn verbatim(text: String) -> Phrase {
    phrase(move |_| text.clone())
}

/// One line of an outcome: its words, and whether they say a failure, which
/// the window draws in the error colour.
#[derive(Clone)]
pub struct Line {
    pub text: Phrase,
    pub failed: bool,
}

impl Line {
    pub fn said(text: Phrase) -> Self {
        Self { text, failed: false }
    }

    pub fn failed(text: Phrase) -> Self {
        Self { text, failed: true }
    }
}
