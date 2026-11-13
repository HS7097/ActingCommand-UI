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

    /// The language for a Windows language identifier (what `GetUserDefaultUILanguage`
    /// answers): Chinese when its primary language is Chinese (`LANG_CHINESE`, in any script or
    /// region), English for every other.
    pub const fn from_windows_language(id: u16) -> Self {
        const PRIMARY_LANGUAGE_MASK: u16 = 0x03ff;
        const LANG_CHINESE: u16 = 0x04;
        if (id & PRIMARY_LANGUAGE_MASK) == LANG_CHINESE {
            Self::Zh
        } else {
            Self::En
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_in_any_script_or_region_is_chinese_and_the_rest_english() {
        // zh-CN, zh-TW, zh-HK, zh-SG, zh-MO, zh-Hans, zh-Hant.
        for id in [0x0804, 0x0404, 0x0c04, 0x1004, 0x1404, 0x0004, 0x7c04] {
            assert_eq!(Lang::from_windows_language(id), Lang::Zh, "{id:#06x}");
        }
        // en-US, en-GB, ja-JP, ko-KR, de-DE, and no language at all.
        for id in [0x0409, 0x0809, 0x0411, 0x0412, 0x0407, 0x0000] {
            assert_eq!(Lang::from_windows_language(id), Lang::En, "{id:#06x}");
        }
    }
}
