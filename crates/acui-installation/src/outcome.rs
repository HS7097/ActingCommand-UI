// SPDX-License-Identifier: GPL-3.0-only
//! The UI's own outcome codes, and the words a person reads for them.
//!
//! Two tables are compiled in and parsed once. `outcome-codes.json` is the
//! UI's code table in the Runtime catalog's format
//! (`actingcommand.outcome-codes.v1`): only the `setup_` and `console_` codes,
//! locations and keys the UI owns, with their maintainer text.
//! `outcome-text.json` holds what a person reads, in English (`en`) and
//! Simplified Chinese (`zh-Hans`): one generic sentence per category, and each
//! UI code's own sentence, short form and handling.
//!
//! The Runtime's catalog is not compiled in, so a Runtime code gets no words
//! here and its caller shows the code as it is. CI checks this table against
//! the Runtime's merged catalog at the commit `runtime-outcome-catalog.rev`
//! names (Workflow #378 A-4): no name of one side is the other's, the keys a
//! UI code uses exist, and every UI code has its words and its constant here.
//!
//! Rust code names a UI code through the constants below, never as a literal.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use serde_json::{Map, Value};

/// The offline read face listed a port whose member instances have no latest
/// binding fact.
pub const CONSOLE_INSTANCE_BINDING_MALFORMED: &str = "console_instance_binding_malformed";
/// The Runtime answered the status read without its observation source.
pub const CONSOLE_STATUS_SOURCE_MISSING: &str = "console_status_source_missing";
/// A launcher press's client-action receipt carried no terminal event.
pub const CONSOLE_CLIENT_ACTION_TERMINAL_MISSING: &str = "console_client_action_terminal_missing";
/// The console's routine performance-monitor event types did not parse as
/// event types of the pinned contract.
pub const CONSOLE_ROUTINE_PERFORMANCE_TYPE_UNKNOWN: &str =
    "console_routine_performance_type_unknown";
/// The session's page connection reached a Runtime of another owner epoch than
/// the session's. The Runtime client's `runtime_owner_epoch_changed` means the
/// same; the console detects it itself, so it states its own code.
pub const CONSOLE_RUNTIME_OWNER_CHANGED: &str = "console_runtime_owner_changed";

/// Every code constant above: the code table must hold each of them.
const UI_CODES: [&str; 5] = [
    CONSOLE_INSTANCE_BINDING_MALFORMED,
    CONSOLE_STATUS_SOURCE_MISSING,
    CONSOLE_CLIENT_ACTION_TERMINAL_MISSING,
    CONSOLE_ROUTINE_PERFORMANCE_TYPE_UNKNOWN,
    CONSOLE_RUNTIME_OWNER_CHANGED,
];

const CODES_FILE: &str = "outcome-codes.json";
const TEXT_FILE: &str = "outcome-text.json";
const CODES_SOURCE: &str = include_str!("../outcome-codes.json");
const TEXT_SOURCE: &str = include_str!("../outcome-text.json");
const CODES_SCHEMA: &str = "actingcommand.outcome-codes.v1";

/// The language the words are in, by the tags the Runtime's `language`
/// vocabulary registers (Workflow #378 A-8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tongue {
    En,
    ZhHans,
}

impl Tongue {
    pub const ALL: [Self; 2] = [Self::En, Self::ZhHans];

    /// `en` or `zh-Hans`, as the text table spells it.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhHans => "zh-Hans",
        }
    }
}

/// An outcome's category, one per code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Success,
    Info,
    Warning,
    Error,
    Fatal,
}

impl Category {
    pub const ALL: [Self; 5] = [Self::Success, Self::Info, Self::Warning, Self::Error, Self::Fatal];

    /// The catalog's spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.as_str() == text)
    }
}

/// What a person reads for one UI code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Explanation {
    pub category: Category,
    pub sentence: &'static str,
    pub short: &'static str,
    /// What the person can do: the code's own handling, or `None` when the
    /// words are its category's generic sentence, which gives no advice.
    pub handling: Option<&'static str>,
}

/// A compiled-in table that does not parse into the shape this module reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeTableError {
    table: &'static str,
    problem: String,
}

impl OutcomeTableError {
    fn new(table: &'static str, problem: impl Into<String>) -> Self {
        Self { table, problem: problem.into() }
    }
}

impl fmt::Display for OutcomeTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} is unusable: {}", self.table, self.problem)
    }
}

impl std::error::Error for OutcomeTableError {}

/// The words for one UI code: the code's own, or, when it has none, its
/// category's generic sentence. `Ok(None)` when the code is not in the UI's
/// table, as every Runtime code is: the caller then shows the code as it is.
/// A compiled-in table that does not parse is an error, every time it is
/// asked, never a panic.
pub fn explain(code: &str, tongue: Tongue) -> Result<Option<Explanation>, OutcomeTableError> {
    let tables = tables()?;
    let Some(&category) = tables.categories.get(code) else {
        return Ok(None);
    };
    if let Some(text) = tables.codes.get(code) {
        let words = text.words(tongue);
        return Ok(Some(Explanation {
            category,
            sentence: words.sentence.as_str(),
            short: words.short.as_str(),
            handling: Some(words.handling.as_str()),
        }));
    }
    let words = tables.generic.get(&(category, tongue)).ok_or_else(|| {
        OutcomeTableError::new(
            TEXT_FILE,
            format!("generic.{}.{} is missing", category.as_str(), tongue.tag()),
        )
    })?;
    Ok(Some(Explanation {
        category,
        sentence: words.sentence.as_str(),
        short: words.short.as_str(),
        handling: None,
    }))
}

/// The two tables, parsed at the first ask; a parse failure is kept and
/// returned to every ask after it.
fn tables() -> Result<&'static Tables, OutcomeTableError> {
    static TABLES: OnceLock<Result<Tables, OutcomeTableError>> = OnceLock::new();
    TABLES
        .get_or_init(|| Tables::parse(CODES_SOURCE, TEXT_SOURCE))
        .as_ref()
        .map_err(|error| error.clone())
}

struct Tables {
    /// Every code of the UI table, with its category.
    categories: BTreeMap<String, Category>,
    generic: BTreeMap<(Category, Tongue), GenericWords>,
    /// The codes that have words of their own.
    codes: BTreeMap<String, CodeText>,
}

struct GenericWords {
    sentence: String,
    short: String,
}

struct CodeWords {
    sentence: String,
    short: String,
    handling: String,
}

struct CodeText {
    en: CodeWords,
    zh_hans: CodeWords,
}

impl CodeText {
    fn words(&self, tongue: Tongue) -> &CodeWords {
        match tongue {
            Tongue::En => &self.en,
            Tongue::ZhHans => &self.zh_hans,
        }
    }
}

impl Tables {
    fn parse(codes_source: &str, text_source: &str) -> Result<Self, OutcomeTableError> {
        let categories = parse_categories(codes_source)?;
        if let Some(code) = UI_CODES.into_iter().find(|code| !categories.contains_key(*code)) {
            return Err(OutcomeTableError::new(
                CODES_FILE,
                format!("codes.{code} is missing"),
            ));
        }
        let text: Value = serde_json::from_str(text_source)
            .map_err(|error| OutcomeTableError::new(TEXT_FILE, error.to_string()))?;

        let generic_table = object(&text, "generic", TEXT_FILE)?;
        let mut generic = BTreeMap::new();
        for category in Category::ALL {
            let place = format!("generic.{}", category.as_str());
            let entry = generic_table
                .get(category.as_str())
                .ok_or_else(|| OutcomeTableError::new(TEXT_FILE, format!("{place} is missing")))?;
            for tongue in Tongue::ALL {
                let words = tongue_entry(entry, tongue, &place)?;
                let place = format!("{place}.{}", tongue.tag());
                generic.insert(
                    (category, tongue),
                    GenericWords {
                        sentence: string(words, "sentence", &place)?,
                        short: string(words, "short", &place)?,
                    },
                );
            }
        }

        let mut codes = BTreeMap::new();
        for (code, entry) in object(&text, "codes", TEXT_FILE)? {
            let place = format!("codes.{code}");
            if !categories.contains_key(code) {
                return Err(OutcomeTableError::new(
                    TEXT_FILE,
                    format!("{place} is not a code of {CODES_FILE}"),
                ));
            }
            let read = |tongue: Tongue| -> Result<CodeWords, OutcomeTableError> {
                let words = tongue_entry(entry, tongue, &place)?;
                let place = format!("{place}.{}", tongue.tag());
                Ok(CodeWords {
                    sentence: string(words, "sentence", &place)?,
                    short: string(words, "short", &place)?,
                    handling: string(words, "handling", &place)?,
                })
            };
            let own = CodeText { en: read(Tongue::En)?, zh_hans: read(Tongue::ZhHans)? };
            codes.insert(code.clone(), own);
        }
        Ok(Self { categories, generic, codes })
    }
}

/// Every code of the code table with its category.
fn parse_categories(source: &str) -> Result<BTreeMap<String, Category>, OutcomeTableError> {
    let table: Value = serde_json::from_str(source)
        .map_err(|error| OutcomeTableError::new(CODES_FILE, error.to_string()))?;
    if table.get("schema_version").and_then(Value::as_str) != Some(CODES_SCHEMA) {
        return Err(OutcomeTableError::new(
            CODES_FILE,
            format!("schema_version is not {CODES_SCHEMA}"),
        ));
    }
    let mut categories = BTreeMap::new();
    for (code, entry) in object(&table, "codes", CODES_FILE)? {
        let category = entry
            .get("category")
            .and_then(Value::as_str)
            .and_then(Category::parse)
            .ok_or_else(|| {
                OutcomeTableError::new(
                    CODES_FILE,
                    format!(
                        "codes.{code}.category is not one of success, info, warning, error, fatal"
                    ),
                )
            })?;
        categories.insert(code.clone(), category);
    }
    Ok(categories)
}

/// The member `name` of `value`, which must be an object.
fn object<'a>(
    value: &'a Value,
    name: &str,
    table: &'static str,
) -> Result<&'a Map<String, Value>, OutcomeTableError> {
    value
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| OutcomeTableError::new(table, format!("{name} is not an object")))
}

/// One tongue's words in a text entry at `place`, which must be an object.
fn tongue_entry<'a>(
    entry: &'a Value,
    tongue: Tongue,
    place: &str,
) -> Result<&'a Value, OutcomeTableError> {
    entry.get(tongue.tag()).filter(|words| words.is_object()).ok_or_else(|| {
        OutcomeTableError::new(
            TEXT_FILE,
            format!("{place}.{} is not an object", tongue.tag()),
        )
    })
}

/// The member `name` of the text entry at `place`, which must be a string
/// with something other than white space in it.
fn string(words: &Value, name: &str, place: &str) -> Result<String, OutcomeTableError> {
    match words.get(name).and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_owned()),
        _ => Err(OutcomeTableError::new(
            TEXT_FILE,
            format!("{place}.{name} is not a non-empty string"),
        )),
    }
}
