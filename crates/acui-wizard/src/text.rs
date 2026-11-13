// SPDX-License-Identifier: GPL-3.0-only
//! What the wizard says to people: every sentence in both of its languages, so the window can
//! show the one the person chose and the log and the command line can carry both.

use std::fmt;

/// The wizard's two languages; the console's `lang` values (`acui-app` `Language::from_wire`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// `zh` or `en`, as the console's settings file writes them.
    pub fn from_wire(text: &str) -> Option<Self> {
        match text {
            "zh" => Some(Self::Zh),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub const fn wire(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

/// One sentence for people, in Chinese and in English.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub zh: String,
    pub en: String,
}

impl Said {
    pub fn new(zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self {
            zh: zh.into(),
            en: en.into(),
        }
    }

    /// The sentence in `lang`.
    pub fn in_lang(&self, lang: Lang) -> &str {
        match lang {
            Lang::Zh => &self.zh,
            Lang::En => &self.en,
        }
    }

    /// This sentence with `detail` (a path, a code, a program's own words) after both halves.
    pub fn with(mut self, detail: impl fmt::Display) -> Self {
        let detail = detail.to_string();
        if !detail.is_empty() {
            self.zh = format!("{}：{detail}", self.zh);
            self.en = format!("{}: {detail}", self.en);
        }
        self
    }
}

/// Both halves on one line, as the log and stderr carry them.
impl fmt::Display for Said {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} / {}", self.zh, self.en)
    }
}
