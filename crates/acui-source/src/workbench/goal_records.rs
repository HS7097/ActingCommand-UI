// SPDX-License-Identifier: GPL-3.0-only
//! The goal page's `records` entries (Workflow #455 RR4b): whom to raise, and to which grade. What
//! the Runtime says of them is the answer of the read `GoalPageView` (`runtime-client` 15), which
//! `GoalView` never carries, being frozen at revision 14 (`contracts/runtime-interface.md`
//! 811-831 at aa6765dd): its part `records`, read with every `GoalView` the Goals tab reads, on
//! that read's clock and on its lane ([`read_records`]); and its parts `options` and `positions`,
//! read only as a person opens the picker or chooses a character in it ([`read_options`],
//! [`read_positions`]), on a person's connection with the full receipt, as a write's, so that the
//! pointer of a refusal
//! `goal_page_view_refused` is kept (`runtime-interface.md` 895-899). Nothing out of here names a
//! wire type: an answer is kept whole ([`RecordsLoaded`], [`OptionsLoaded`], [`PositionsLoaded`])
//! until it is shown, since its words depend on the language shown.
//!
//! The console computes nothing of its own. Which records apply, the key's held state, each lane
//! the walk takes in the table's order, the current position or why it is not known, where a
//! prerequisite raises a lane, and every name are the Runtime's (`runtime-interface.md` 833-862);
//! a record's row shows no amount of what it lacks, which the Runtime does not answer either (855).
//! A position is said by the pack's own words, else by its numbers ([`position_text`]).
//!
//! Reading a part is one page and then each `next` until none (`runtime-interface.md` 867-870), and
//! nothing more: every page holds the Runtime's evaluation locks (899-903). What a refusal of a
//! later page leads to is [`after_refusal`]'s: the records or the declaration moved, the part is
//! read again from its first page, at most once in one read, and after a second move what was
//! shown before is kept until the next read (871-878); the declaration moved also lets go of what
//! is kept under the old tables' sha256 (893-895); a page not in memory keeps it and shows nothing
//! of the part (879-884). A first page whose declaration is `not_checked_yet`, `root_failing` or
//! `no_schedule` holds nothing of its part ([`holds_part`], 821-824). A Runtime of `runtime-client`
//! 14 or lower drops the connection on the request, which the client says as
//! `runtime_operation_unsupported` (890-892): the lane then asks no more in that owner epoch, and
//! the page is shown as before, without records.
//!
//! The blocks of the records entries go on the page right after the `GoalView` entry their
//! `after_entry` names, which may be another records entry's block ([`placed`]). A write is one
//! `EditGoal` `set` of the entry's name with its whole array: the records as the answer gave them,
//! in their order, each `to` exactly as answered, the chosen key's record replaced in place or
//! added at the end, or left out ([`records_after`], [`records_json`]); an empty array removes the
//! last (`contracts/standard-pack-goals.md` 139-165 at aa6765dd). Its `to` is made of the picker's
//! choices ([`to_of`]), each lane starting at its record's target, else at the ladder's last
//! position ([`default_target`]). The names under which the stored records hold no declared entry,
//! and every declared entry while the stored record does not read, are cleared by name
//! (`runtime-interface.md` 754-786).

use std::collections::BTreeMap;
use std::path::Path;

use acui_rows::outcome::Tongue;
use actingcommand_contract::{
    GoalDeclarationRead, GoalHeldState, GoalKeyHeld, GoalLadderRef, GoalLadderSource,
    GoalLanePosition, GoalLaneUnknown, GoalPageCondition, GoalPageCursor, GoalPagePart,
    GoalPageRecordsState, GoalPageRequirement, GoalPageRow, GoalPageSub, GoalPageTables,
    GoalPageText, GoalPageView, GoalTablesRead, MAX_GOAL_PAGE_VIEW_LIMIT,
};
use actingcommand_runtime_client::{RuntimeClient, RuntimeClientError};
use serde_json::{Map, Value};

use super::goals::{
    in_language, set_by, GoalAct, GoalChange, GoalLine, GoalWrite, GoalsPage, SetBy, WriteProblem,
    EDIT_IO,
};
use super::{configure_config, pack};
use crate::{runtime_connection, ClientFailure, RUNTIME_RESULT_UNEXPECTED};

/// The refusals of a page with a cursor (`contracts/outcome-codes/runtime-host.json` 22209-22296 at
/// aa6765dd): the stored records moved; the tables or the records entries moved; the page is not in
/// memory; and the part names what the root does not have (of `options` and `positions` only).
const RECORDS_MOVED: &str = "goal_page_view_records_moved";
const DECLARATION_MOVED: &str = "goal_page_view_declaration_moved";
const DECLARATION_ABSENT: &str = "goal_page_view_declaration_absent";
const PART_REFUSED: &str = "goal_page_view_refused";
/// What `goal_page_view_refused` says beyond its code.
const GOAL_PROBLEM: &str = "goal_problem";
const JSON_POINTER: &str = "json_pointer";
/// The client's own spelling of the read's operation (runtime-client `codes.rs` 275 at aa6765dd),
/// under which a page that goes nowhere is said.
const OPERATION: &str = "runtime_goal_page_view";

/// The most records an application holds over all its records entries (`runtime-interface.md`
/// 770 at aa6765dd: more is `record_out_of_limits`, 0 to 128).
pub const APPLICATION_RECORDS: u64 = 128;
/// How many characters the picker lists at most ([`key_matches`]).
pub const KEYS_SHOWN: usize = 100;
/// What the key of a records entry's line and of a record's line begin with ([`records_key`],
/// [`record_key`]).
pub const RECORDS_KEY: &str = "records:";
pub const RECORD_KEY: &str = "record:";

/// The key of the line of the records entry `entry`.
pub fn records_key(entry: &str) -> String {
    format!("{RECORDS_KEY}{entry}")
}

/// The key of the line of the record of `key` in the records entry `entry`. An entry's name has no
/// `:` (`goal_name`), so the first one after the prefix ends it.
pub fn record_key(entry: &str, key: &str) -> String {
    format!("{RECORD_KEY}{entry}:{key}")
}

/// Whether a page whose declaration is `state` holds anything of its part: not one whose page is
/// not in memory (`not_checked_yet`, `root_failing`, `no_schedule`), whatever its `tables` says,
/// which no part is ended on and nothing is kept of (`runtime-interface.md` 821-824 at aa6765dd).
pub fn holds_part(state: GoalDeclarationRead) -> bool {
    !matches!(
        state,
        GoalDeclarationRead::NotCheckedYet
            | GoalDeclarationRead::RootFailing
            | GoalDeclarationRead::NoSchedule
    )
}

/// What a refusal of a page leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRefusal {
    /// The part is read again from its first page; `drop`: and what is kept under the tables'
    /// sha256 its cursor named is let go.
    Restart { drop: bool },
    /// It moved again after one start again in this read: what was shown before stays, and the
    /// part is read at the next read; `drop` as above.
    GiveUp { drop: bool },
    /// The page is not in memory: what is kept stays, nothing of the part is shown, and the part
    /// is read again from its first page at the next read, which says when it is back.
    KeepCached,
    /// Any other refusal, or none: a failure, said with its codes.
    Fail,
}

/// What a refusal of `code` of a page with a cursor leads to, `restarted` whether this read began
/// the part again already, the one place that decides it (`runtime-interface.md` 871-884, 893-895
/// at aa6765dd).
pub fn after_refusal(code: &str, restarted: bool) -> AfterRefusal {
    let drop = match code {
        RECORDS_MOVED => false,
        DECLARATION_MOVED => true,
        DECLARATION_ABSENT => return AfterRefusal::KeepCached,
        _ => return AfterRefusal::Fail,
    };
    if restarted {
        AfterRefusal::GiveUp { drop }
    } else {
        AfterRefusal::Restart { drop }
    }
}

/// What reading one part came to, and the tables' sha256 under which what is kept is let go, when
/// the declaration moved during it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartAnswer<T> {
    pub dropped: Option<String>,
    pub came: Came<T>,
}

/// What reading one part came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Came<T> {
    Read(T),
    /// The page is not in memory, on its first page or a later one: nothing of the part.
    NotInMemory,
    /// The part moved again after one start again: what was shown before stays.
    Moved,
    /// `goal_page_view_refused`: the part names what the root does not have now, by the edit's
    /// word, at its pointer when the receipt keeps it.
    Refused { problem: WriteProblem, pointer: Option<String> },
    /// This Runtime has no such read, by the code that says so; also what a lane answers without
    /// asking once its Runtime of the owner epoch said so.
    Unsupported(&'static str),
    Failed(ClientFailure),
}

/// A failure of a part's read that is no answer of it.
pub(super) enum PartFailure {
    /// The client's: a refusal no step here is for, or no answer at all.
    Client(RuntimeClientError),
    /// A page that has rows left and answers none, or begins before the one before it: a fault
    /// of the Runtime, which nothing reads on past.
    Stuck,
}

/// A part read page after page.
enum Paged {
    /// Its first page, without its rows; every row, in order; the last page's version.
    Rows { first: Box<GoalPageView>, rows: Vec<GoalPageRow>, version: u64 },
    NotInMemory,
    Moved,
}

/// The part `part` of `alias`, page after page on `client`, as [`after_refusal`] says, with the
/// sha256 to let go of when the declaration moved.
fn paged(
    client: &RuntimeClient,
    alias: &str,
    part: &GoalPagePart,
) -> Result<(Paged, Option<String>), PartFailure> {
    let mut dropped: Option<String> = None;
    let mut restarted = false;
    'part: loop {
        let mut first: Option<GoalPageView> = None;
        let mut rows: Vec<GoalPageRow> = Vec::new();
        let mut cursor: Option<GoalPageCursor> = None;
        // The last page's version.
        let version = loop {
            let sent = cursor.take();
            let answer = client.goal_page_view(
                alias,
                part.clone(),
                sent.clone(),
                Some(MAX_GOAL_PAGE_VIEW_LIMIT),
            );
            let mut page = match answer {
                Ok(page) => page,
                Err(error) => {
                    let code = refusal_code(&error).unwrap_or_default();
                    let moved_sha = sent.and_then(|sent| sent.tables_sha256);
                    match after_refusal(&code, restarted) {
                        AfterRefusal::Restart { drop } => {
                            if drop {
                                dropped = moved_sha;
                            }
                            restarted = true;
                            continue 'part;
                        }
                        AfterRefusal::GiveUp { drop } => {
                            if drop {
                                dropped = moved_sha;
                            }
                            return Ok((Paged::Moved, dropped));
                        }
                        AfterRefusal::KeepCached => return Ok((Paged::NotInMemory, dropped)),
                        AfterRefusal::Fail => return Err(PartFailure::Client(error)),
                    }
                }
            };
            if sent.is_none() && !holds_part(page.declaration.state) {
                return Ok((Paged::NotInMemory, dropped));
            }
            let version = page.version;
            let next = page.next.take();
            let got = page.rows.len();
            rows.append(&mut page.rows);
            let total = match &first {
                Some(first) => first.rows_total,
                None => page.rows_total,
            };
            if first.is_none() {
                first = Some(page);
            }
            let Some(next) = next else {
                break version;
            };
            let onward = sent.as_ref().map_or(true, |sent| next.after > sent.after);
            let within = u64::try_from(rows.len()).unwrap_or(u64::MAX) < u64::from(total);
            if got == 0 || !onward || !within {
                return Err(PartFailure::Stuck);
            }
            cursor = Some(next);
        };
        let Some(first) = first else {
            return Err(PartFailure::Stuck);
        };
        return Ok((Paged::Rows { first: Box::new(first), rows, version }, dropped));
    }
}

/// The Runtime's code of a refusal it answered and went on after; none of anything else.
fn refusal_code(error: &RuntimeClientError) -> Option<String> {
    if !pack::answered(error) {
        return None;
    }
    error.receipt_outcome().map(|outcome| outcome.code().as_str().to_owned())
}

/// What a failed read that is no answer of its part comes to, the one place that tells it: a
/// Runtime without the read; the part refused, with its word and pointer; any other failure, with
/// its codes. A lane lets go of its connection first, as after any read.
pub(super) fn came_of<T>(error: RuntimeClientError) -> Came<T> {
    if let Some(code) = pack::view_unsupported(&error) {
        return Came::Unsupported(code);
    }
    let refused = pack::answered(&error)
        && error.receipt_outcome().is_some_and(|outcome| outcome.code().as_str() == PART_REFUSED);
    if refused {
        let outcome = error.receipt_outcome();
        let text = |key: &str| {
            let value = outcome.and_then(|outcome| outcome.value(key));
            value.and_then(Value::as_str).map(str::to_owned)
        };
        let problem = WriteProblem::of(&text(GOAL_PROBLEM).unwrap_or_default());
        return Came::Refused { problem, pointer: text(JSON_POINTER) };
    }
    Came::Failed(ClientFailure::from(error))
}

/// The failure of a page that goes nowhere ([`PartFailure::Stuck`]).
pub(super) fn stuck() -> ClientFailure {
    ClientFailure {
        code: RUNTIME_RESULT_UNEXPECTED,
        operation: OPERATION,
        runtime_code: None,
        host: None,
    }
}

/// The sha256 of the tables a page names, while they are accepted: the only tables a records
/// entry applies, and what is kept is kept under.
fn accepted_sha(tables: &GoalPageTables) -> Option<String> {
    match tables.state {
        GoalTablesRead::Accepted => tables.sha256.clone(),
        GoalTablesRead::Absent
        | GoalTablesRead::Refused
        | GoalTablesRead::Unreadable
        | GoalTablesRead::NotRead => None,
    }
}

/// A part's read as its answer, its rows made into `T` by `make`.
fn answer_of<T>(
    read: Result<(Paged, Option<String>), PartFailure>,
    make: impl FnOnce(&GoalPageView, Vec<GoalPageRow>, u64) -> T,
) -> PartAnswer<T> {
    match read {
        Ok((paged, dropped)) => {
            let came = match paged {
                Paged::Rows { first, rows, version } => Came::Read(make(&*first, rows, version)),
                Paged::NotInMemory => Came::NotInMemory,
                Paged::Moved => Came::Moved,
            };
            PartAnswer { dropped, came }
        }
        Err(PartFailure::Stuck) => PartAnswer { dropped: None, came: Came::Failed(stuck()) },
        Err(PartFailure::Client(error)) => PartAnswer { dropped: None, came: came_of(error) },
    }
}

/// The part `records` of `alias` on the lane's `client`, read whole ([`paged`]); a failure the
/// lane settles before it says it ([`came_of`]).
pub(super) fn read_records(
    client: &RuntimeClient,
    alias: &str,
) -> Result<PartAnswer<RecordsLoaded>, PartFailure> {
    let (paged, dropped) = paged(client, alias, &GoalPagePart::Records)?;
    let came = match paged {
        Paged::Rows { first, rows, version } => Came::Read(RecordsLoaded {
            version,
            tables: accepted_sha(&first.tables),
            state: first.records_state,
            rows,
        }),
        Paged::NotInMemory => Came::NotInMemory,
        Paged::Moved => Came::Moved,
    };
    Ok(PartAnswer { dropped, came })
}

/// The connection of the picker's reads: a person's, with the full receipt, at a write's io
/// timeout, since each page takes the Runtime's evaluation locks.
fn pick_connection(root: &Path) -> Result<RuntimeClient, ClientFailure> {
    let config = configure_config(root).with_io_timeout(EDIT_IO);
    runtime_connection(config).map_err(ClientFailure::from)
}

/// The part `options` of the records entry `entry` of `alias`, read whole on a connection of its
/// own, which is let go: its table's columns, classes and keys, each key with its held state and
/// its current position on each of its ladders (`runtime-interface.md` 856-860 at aa6765dd).
pub fn read_options(root: &Path, alias: &str, entry: &str) -> PartAnswer<OptionsLoaded> {
    let client = match pick_connection(root) {
        Ok(client) => client,
        Err(failure) => return PartAnswer { dropped: None, came: Came::Failed(failure) },
    };
    let part = GoalPagePart::Options { entry: entry.to_owned() };
    answer_of(paged(&client, alias, &part), |first, rows, _| OptionsLoaded {
        tables: accepted_sha(&first.tables),
        fresh_until: first.fresh_until_unix_ms,
        rows,
    })
}

/// The part `positions` of each of `ladders` of the records entry `entry` of `alias`, in turn on
/// one connection of their own, which is let go (`runtime-interface.md` 861-862 at aa6765dd).
pub fn read_positions(
    root: &Path,
    alias: &str,
    entry: &str,
    ladders: &[Ladder],
) -> Vec<(Ladder, PartAnswer<PositionsLoaded>)> {
    let client = match pick_connection(root) {
        Ok(client) => client,
        Err(failure) => {
            return ladders
                .iter()
                .map(|ladder| {
                    let came = Came::Failed(failure.clone());
                    (ladder.clone(), PartAnswer { dropped: None, came })
                })
                .collect();
        }
    };
    ladders
        .iter()
        .map(|ladder| {
            let part =
                GoalPagePart::Positions { entry: entry.to_owned(), ladder: ladder.of.wire() };
            let answer = answer_of(paged(&client, alias, &part), |first, rows, _| {
                PositionsLoaded { tables: accepted_sha(&first.tables), rows }
            });
            (ladder.clone(), answer)
        })
        .collect()
}

/// The part `records` as the Runtime answered it, kept whole until it is shown
/// ([`RecordsLoaded::shown`]): the page's version (the last page's, the next `expected_version`,
/// `runtime-interface.md` 825 and 787-795 at aa6765dd), the accepted tables' sha256, the stored
/// record's state, and every row in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordsLoaded {
    version: u64,
    tables: Option<String>,
    state: Option<GoalPageRecordsState>,
    rows: Vec<GoalPageRow>,
}

impl RecordsLoaded {
    /// The version of this read, which a write of what it shows names.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The sha256 of the accepted tables it was read over.
    pub fn tables_sha256(&self) -> Option<&str> {
        self.tables.as_deref()
    }

    /// The stored record is one its reader does not read.
    pub fn unreadable(&self) -> bool {
        self.state == Some(GoalPageRecordsState::Unreadable)
    }

    /// Its blocks in `tongue` ([`shown`]).
    pub fn shown(&self, tongue: Tongue) -> RecordsShown {
        shown(self, tongue)
    }

    /// A read of `rows` at `version`, over tables of one sha256, for the tests of the page that
    /// places them.
    #[cfg(test)]
    pub(crate) fn of_rows(
        version: u64,
        state: Option<GoalPageRecordsState>,
        rows: Vec<GoalPageRow>,
    ) -> Self {
        Self { version, tables: Some(format!("sha256:{}", "c".repeat(64))), state, rows }
    }
}

/// The records part's blocks, as the page shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordsShown {
    /// Each records entry of `goals.json`, in file order, with the entry it follows.
    pub declared: Vec<RecordsBlock>,
    /// Each name the stored records hold that no records entry declares, in stored order.
    pub undeclared: Vec<RecordsBlock>,
    /// The stored record is one its reader does not read.
    pub unreadable: bool,
    /// Records answered before any row of their entry, which no block shows.
    pub loose: usize,
}

impl RecordsShown {
    /// Whether it shows anything at all.
    pub fn any(&self) -> bool {
        !self.declared.is_empty() || !self.undeclared.is_empty()
    }
}

/// One records entry's line and its records' lines, and the name of the entry it follows
/// (`after_entry`: none, the first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordsBlock {
    pub line: RecordsLine,
    pub after: Option<String>,
    pub records: Vec<RecordLine>,
}

/// A records entry: its name, which a write names (shown only of an entry no records entry
/// declares, which has no other); its title and help; whether `goals.json` declares it, whether
/// its table applies, how many records it holds and of how many at most; its table's name, which
/// what is kept of its positions goes by (never shown); its stored records as answered, in their
/// order, which a write sends back; whether a character can be added; and whether the stored record
/// reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordsLine {
    pub name: String,
    pub title: Option<String>,
    pub help: Option<String>,
    pub declared: bool,
    pub applies: bool,
    pub total: u16,
    pub max: Option<u64>,
    pub table: Option<String>,
    pub stored: Vec<StoredRecord>,
    pub room: Room,
    pub unreadable: bool,
}

impl RecordsLine {
    /// Whether a clear of its name is offered: an entry no records entry declares, which a `set`
    /// does not reach; and every declared one while the stored record does not read, which only a
    /// clear ends (`runtime-interface.md` 754-786 at aa6765dd).
    pub fn clearable(&self) -> bool {
        !self.declared || self.unreadable
    }

    /// Its stored record of `key`, when it has one.
    pub fn recorded(&self, key: &str) -> Option<&StoredRecord> {
        self.stored.iter().find(|record| record.key == key)
    }

    /// The name what is kept of its table's ladders goes by: its table's, else its own.
    pub fn table_name(&self) -> &str {
        self.table.as_deref().unwrap_or(self.name.as_str())
    }
}

/// One stored record as the answer gave it: its key and its target, the stored canonical object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecord {
    pub key: String,
    pub to: Map<String, Value>,
}

/// Whether a character can be added to a records entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Room {
    Open,
    /// It holds `max_records` already.
    EntryFull,
    /// The application holds [`APPLICATION_RECORDS`] over all its entries.
    AllFull,
    /// It takes none here: not declared, its table not applied, or the stored record unreadable,
    /// each said otherwise.
    Closed,
}

/// Whether a character can be added to an entry that is `declared`, whose table `applies`, while
/// the stored record is `unreadable` or not, holding `total` of at most `max` records, the
/// application holding `all` over every entry.
pub fn room(
    declared: bool,
    applies: bool,
    unreadable: bool,
    total: u64,
    max: Option<u64>,
    all: u64,
) -> Room {
    if !declared || !applies || unreadable {
        return Room::Closed;
    }
    if max.map_or(true, |max| total >= max) {
        return Room::EntryFull;
    }
    if all >= APPLICATION_RECORDS {
        return Room::AllFull;
    }
    Room::Open
}

/// One stored record's line: its entry and key (which a write names), the key's name in the
/// language shown; whether the needs apply it, else why, by the edit's word; whether its character
/// is held; each lane the walk takes, in the table's order; who set it; whether its texts were left
/// out (ids only); and whether it can be changed and removed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordLine {
    pub entry: String,
    pub key: String,
    pub name: Option<String>,
    pub applied: bool,
    pub problem: Option<WriteProblem>,
    pub held: Option<KeyHeld>,
    pub lanes: Vec<LaneLine>,
    pub set: SetBy,
    pub ids_only: bool,
    pub changeable: bool,
}

/// A character's held state, as the table's `held_by` list says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyHeld {
    Held,
    /// The list read whole holds no row of it: the record counts for nothing until it is held.
    NotHeld,
    /// The list gives no reading, and why when it says.
    Unknown(Option<ListUnknown>),
}

/// Why a list gives no reading: the vocabulary `unknown_reason` (policy `record_needs.rs` 113-143
/// at aa6765dd), one word each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListUnknown {
    /// `fact_missing`: the instance holds no such list yet.
    Missing,
    /// `fact_expired`.
    Expired,
    /// `fact_low_confidence`.
    LowConfidence,
    /// `type_mismatch`: the list does not read as a list.
    TypeMismatch,
    /// A word of a newer Runtime, by itself.
    Other(String),
}

impl ListUnknown {
    fn of(word: &str) -> Self {
        match word {
            "fact_missing" => Self::Missing,
            "fact_expired" => Self::Expired,
            "fact_low_confidence" => Self::LowConfidence,
            "type_mismatch" => Self::TypeMismatch,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// One lane of a record: its column and sub, by their names in the language shown or their ids;
/// the record's own target (none: only a prerequisite raised it); where a prerequisite of another
/// column takes it beyond that; the key's position now, else why it is not known; and the
/// conditions met in the game a step it takes names, each by its name or id. Each position as
/// [`position_text`] says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneLine {
    pub column: String,
    pub sub: Option<String>,
    pub target: Option<String>,
    pub raised_to: Option<String>,
    pub current: Option<String>,
    pub unknown: Option<LaneUnknown>,
    pub conditions: Vec<String>,
}

/// Why a key's position on a ladder is not known, by the walk's note (`runtime-interface.md`
/// 849-852 at aa6765dd).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneUnknown {
    /// `goal_records_progress_unknown`: the progress list gives no reading, and why when it says.
    Progress(Option<ListUnknown>),
    /// `goal_records_row_missing`: the list has no row of the character yet.
    RowMissing,
    /// `goal_records_reading_missing`: the list's row lacks the reading column the tables name,
    /// which only the resource side mends; that column when it is named.
    ReadingMissing(Option<String>),
    /// `goal_records_position_unknown`: the readings match no position of the ladder.
    PositionUnknown,
    /// A code of a newer Runtime, by itself.
    Other(String),
}

impl LaneUnknown {
    fn of(unknown: &GoalLaneUnknown) -> Self {
        match unknown.code.as_str() {
            "goal_records_progress_unknown" => {
                Self::Progress(unknown.unknown_word.as_deref().map(ListUnknown::of))
            }
            "goal_records_row_missing" => Self::RowMissing,
            "goal_records_reading_missing" => Self::ReadingMissing(unknown.reading.clone()),
            "goal_records_position_unknown" => Self::PositionUnknown,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// How a position is shown: the pack's words of it in `tongue` when it says it (the console's one
/// fallback of texts, [`in_language`]), else its numbers joined by "·"; the Runtime says a position
/// only through the pack's own lists and invents none (`runtime-interface.md` 864-867 at aa6765dd).
pub fn position_text(position: &[i64], said: Option<&GoalPageText>, tongue: Tongue) -> String {
    if let Some(words) = said.and_then(|said| in_language(&said.0, tongue)) {
        return words.to_owned();
    }
    position.iter().map(i64::to_string).collect::<Vec<String>>().join("·")
}

/// A text of the pack in `tongue`, none when it writes none.
fn named(text: Option<&GoalPageText>, tongue: Tongue) -> Option<String> {
    text.and_then(|text| in_language(&text.0, tongue)).map(str::to_owned)
}

/// A key's held state as the Runtime says it.
fn key_held(held: &GoalKeyHeld) -> KeyHeld {
    match held.state {
        GoalHeldState::Held => KeyHeld::Held,
        GoalHeldState::NotHeld => KeyHeld::NotHeld,
        GoalHeldState::Unknown => {
            KeyHeld::Unknown(held.unknown_word.as_deref().map(ListUnknown::of))
        }
    }
}

/// A text of `member` of `value` per locale, in `tongue`.
fn text_of(value: Option<&Value>, member: &str, tongue: Tongue) -> Option<String> {
    let texts: Vec<(String, String)> = value?
        .get(member)?
        .as_object()?
        .iter()
        .filter_map(|(locale, text)| Some((locale.clone(), text.as_str()?.to_owned())))
        .collect();
    in_language(&texts, tongue).map(str::to_owned)
}

/// The blocks of `loaded` in `tongue`: each entry row with the records that follow it (the
/// Runtime answers an entry's row before its records, `runtime-interface.md` 833-838 at aa6765dd),
/// whether a character can be added to it ([`room`]: its count and the application's over every
/// entry), and whether its records can be changed here (its entry declared and applied, the
/// stored record readable). With `texts_left_out`, a record shows ids only.
pub fn shown(loaded: &RecordsLoaded, tongue: Tongue) -> RecordsShown {
    let unreadable = loaded.unreadable();
    let mut blocks: Vec<RecordsBlock> = Vec::new();
    let mut loose = 0;
    for row in &loaded.rows {
        match row {
            GoalPageRow::Entry {
                name, declared, after_entry, entry, applies, records_total, ..
            } => {
                let entry = entry.as_ref();
                let max = entry.and_then(|entry| entry.get("max_records")).and_then(Value::as_u64);
                blocks.push(RecordsBlock {
                    line: RecordsLine {
                        name: name.clone(),
                        title: text_of(entry, "title", tongue),
                        help: text_of(entry, "help", tongue),
                        declared: *declared,
                        applies: *applies,
                        total: *records_total,
                        max,
                        table: entry
                            .and_then(|entry| entry.get("table"))
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        stored: Vec::new(),
                        room: Room::Closed,
                        unreadable,
                    },
                    after: after_entry.clone(),
                    records: Vec::new(),
                });
            }
            GoalPageRow::Record {
                entry,
                key,
                key_name,
                to,
                set,
                applied,
                problem,
                held,
                lanes,
                texts_left_out,
            } => {
                let Some(block) = blocks.iter_mut().rev().find(|block| block.line.name == *entry)
                else {
                    loose += 1;
                    continue;
                };
                let ids_only = *texts_left_out;
                let tongued = |text: Option<&GoalPageText>| {
                    if ids_only {
                        None
                    } else {
                        named(text, tongue)
                    }
                };
                let said = |at: &GoalLanePosition| {
                    let words = if ids_only { None } else { at.said.as_ref() };
                    position_text(&at.position, words, tongue)
                };
                let lanes = lanes
                    .iter()
                    .map(|lane| LaneLine {
                        column: tongued(lane.column_name.as_ref())
                            .unwrap_or_else(|| lane.column.clone()),
                        sub: lane.sub.as_ref().map(|sub| {
                            tongued(lane.sub_name.as_ref()).unwrap_or_else(|| sub.clone())
                        }),
                        target: lane.target.as_ref().map(said),
                        raised_to: lane.raised_to.as_ref().map(said),
                        current: lane.current.as_ref().map(said),
                        unknown: lane.unknown.as_ref().map(LaneUnknown::of),
                        conditions: lane
                            .conditions
                            .iter()
                            .map(|condition| condition_text(condition, ids_only, tongue))
                            .collect(),
                    })
                    .collect();
                let line = &block.line;
                let changeable = line.declared && line.applies && !unreadable;
                block.records.push(RecordLine {
                    entry: entry.clone(),
                    key: key.clone(),
                    name: tongued(key_name.as_ref()),
                    applied: *applied,
                    problem: problem.as_deref().map(WriteProblem::of),
                    held: held.as_ref().map(key_held),
                    lanes,
                    set: set_by(set),
                    ids_only,
                    changeable,
                });
                block.line.stored.push(StoredRecord { key: key.clone(), to: to.clone() });
            }
            GoalPageRow::Column { .. }
            | GoalPageRow::Class { .. }
            | GoalPageRow::Key { .. }
            | GoalPageRow::Position { .. } => {}
        }
    }
    let all: u64 = blocks.iter().map(|block| u64::from(block.line.total)).sum();
    for block in &mut blocks {
        let line = &mut block.line;
        let total = u64::from(line.total);
        line.room = room(line.declared, line.applies, unreadable, total, line.max, all);
    }
    let (declared, undeclared): (Vec<RecordsBlock>, Vec<RecordsBlock>) =
        blocks.into_iter().partition(|block| block.line.declared);
    RecordsShown { declared, undeclared, unreadable, loose }
}

/// A condition met in the game, by its name in `tongue`, else its id.
fn condition_text(condition: &GoalPageCondition, ids_only: bool, tongue: Tongue) -> String {
    let name = if ids_only { None } else { named(condition.name.as_ref(), tongue) };
    name.unwrap_or_else(|| condition.condition.clone())
}

/// One place of the page's merged order: the `GoalView` entry at that place among those placed, or
/// the records entry's block at that place among the declared ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Entry(usize),
    Block(usize),
}

/// The order of the page's placed entries, named `entries` in their order, and of the records
/// entries' blocks, each `(name, after_entry)` in file order: each block right after the entry or
/// the block its `after_entry` names (the entry before it in `goals.json`, any type,
/// `runtime-interface.md` 833-836 at aa6765dd), and after the blocks that already follow that one;
/// first of all without one; last of all when the name is none of those placed before it.
pub fn placed(entries: &[&str], blocks: &[(&str, Option<&str>)]) -> Vec<Slot> {
    let mut order: Vec<Slot> = (0..entries.len()).map(Slot::Entry).collect();
    for (at, (_, after)) in blocks.iter().enumerate() {
        let target = match after {
            None => Some(0),
            Some(name) => order
                .iter()
                .position(|slot| match slot {
                    Slot::Entry(entry) => entries[*entry] == *name,
                    Slot::Block(block) => blocks[*block].0 == *name,
                })
                .map(|place| place + 1),
        };
        let mut place = target.unwrap_or(order.len());
        while matches!(order.get(place), Some(Slot::Block(_))) {
            place += 1;
        }
        order.insert(place, Slot::Block(at));
    }
    order
}

/// One change of a records entry's array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordChange {
    /// The record of `key`, with `to`: in place of the one of `key`, or at the end.
    Put { key: String, to: Map<String, Value> },
    /// The record of `key` left out.
    Remove(String),
}

/// The records of an entry, `stored` as the answer gave them in their order, after `change`: every
/// other record as it was, its `to` exactly as answered. None of a removal of a key it does not
/// hold.
pub fn records_after(stored: &[StoredRecord], change: &RecordChange) -> Option<Vec<StoredRecord>> {
    match change {
        RecordChange::Put { key, to } => {
            let mut records = stored.to_vec();
            match records.iter_mut().find(|record| record.key == *key) {
                Some(record) => record.to = to.clone(),
                None => records.push(StoredRecord { key: key.clone(), to: to.clone() }),
            }
            Some(records)
        }
        RecordChange::Remove(key) => {
            if !stored.iter().any(|record| record.key == *key) {
                return None;
            }
            Some(stored.iter().filter(|record| record.key != *key).cloned().collect())
        }
    }
}

/// A records entry's value: the JSON array of `records`, each `{key, to}`, in their order; `[]` of
/// none (`contracts/standard-pack-goals.md` 139-147 at aa6765dd).
pub fn records_json(records: &[StoredRecord]) -> String {
    let items = records
        .iter()
        .map(|record| {
            let mut item = Map::new();
            item.insert("key".to_owned(), Value::String(record.key.clone()));
            item.insert("to".to_owned(), Value::Object(record.to.clone()));
            Value::Object(item)
        })
        .collect();
    Value::Array(items).to_string()
}

/// One lane's target as the picker leaves it: its column, its sub, and the position chosen (none:
/// no target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneTarget {
    pub column: String,
    pub sub: Option<String>,
    pub position: Option<Vec<i64>>,
}

/// A record's `to` of `targets`: each lane with a position, a column's as `{column: position}` and
/// a sub's as `{column: {sub: position}}`, each position the ladder's array; nothing of a lane
/// without one (`contracts/standard-pack-goals.md` 139-147 at aa6765dd). Empty: nothing to save.
pub fn to_of(targets: &[LaneTarget]) -> Map<String, Value> {
    let mut to = Map::new();
    for target in targets {
        let Some(position) = &target.position else {
            continue;
        };
        let position = Value::Array(position.iter().copied().map(Value::from).collect());
        match &target.sub {
            None => {
                to.insert(target.column.clone(), position);
            }
            Some(sub) => {
                let subs = to
                    .entry(target.column.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(subs) = subs {
                    subs.insert(sub.clone(), position);
                }
            }
        }
    }
    to
}

/// The position `to` names of the lane of `column` and `sub`: an array of whole numbers; none when
/// it names none, or one of another form.
pub fn lane_target(to: &Map<String, Value>, column: &str, sub: Option<&str>) -> Option<Vec<i64>> {
    let at = to.get(column)?;
    let at = match sub {
        Some(sub) => at.as_object()?.get(sub)?,
        None => at,
    };
    at.as_array()?.iter().map(Value::as_i64).collect()
}

/// The place among `positions` a lane of `column` and `sub` starts at in the picker: of a character
/// recorded already (`recorded`, its `to`), its record's target, or no target where the record
/// names none on this lane, or the last position where it names one its ladder no longer holds; of
/// a new one, the ladder's last position. None of a ladder without positions.
pub fn default_target(
    positions: &[Vec<i64>],
    recorded: Option<&Map<String, Value>>,
    column: &str,
    sub: Option<&str>,
) -> Option<usize> {
    let last = positions.len().checked_sub(1)?;
    let Some(to) = recorded else {
        return Some(last);
    };
    let stored = lane_target(to, column, sub)?;
    Some(positions.iter().position(|position| *position == stored).unwrap_or(last))
}

/// Whether the position at `index` of a ladder is at or before the key's position now (`current`,
/// by its index), and so greyed in the picker, though it may still be chosen: a target there asks
/// nothing (`records-page-needs`, section 2).
pub fn at_or_before(index: u16, current: Option<u16>) -> bool {
    current.is_some_and(|current| index <= current)
}

/// What a press `act` on the records line `line` of `page` writes, the one place that decides it:
/// none when it writes nothing. A character saved on an entry's line or on one of its records'
/// lines (`Put`, its record replaced in place or added where the entry has room, never a `to` that
/// names nothing or the one stored already); a record removed (`Remove`, on its line); a clear of
/// an entry's name where it is offered ([`RecordsLine::clearable`]). Only of an entry declared and
/// applied, while the stored record reads, but for the clear. It never asks first: every one of
/// them can be set back here.
pub(super) fn records_press(page: &GoalsPage, line: &GoalLine, act: &GoalAct) -> Option<GoalWrite> {
    match (line, act) {
        (GoalLine::Records(block), GoalAct::Put { key, to }) => put(block, key, to),
        (GoalLine::Records(block), GoalAct::Clear) if block.clearable() => {
            Some(GoalWrite { name: block.name.clone(), change: GoalChange::Clear })
        }
        (GoalLine::Record(record), GoalAct::Put { key, to }) if record.changeable => {
            put(block_of(page, &record.entry)?, key, to)
        }
        (GoalLine::Record(record), GoalAct::Remove) if record.changeable => {
            let block = block_of(page, &record.entry)?;
            let records = records_after(&block.stored, &RecordChange::Remove(record.key.clone()))?;
            Some(set(&block.name, &records))
        }
        _ => None,
    }
}

/// The records entry `entry`'s line on `page`.
fn block_of<'a>(page: &'a GoalsPage, entry: &str) -> Option<&'a RecordsLine> {
    match page.line(&records_key(entry))? {
        GoalLine::Records(block) => Some(block),
        _ => None,
    }
}

/// The record of `key` with `to` saved on `block` ([`records_press`]).
fn put(block: &RecordsLine, key: &str, to: &Map<String, Value>) -> Option<GoalWrite> {
    if !block.declared || !block.applies || block.unreadable || to.is_empty() {
        return None;
    }
    match block.recorded(key) {
        Some(record) if record.to == *to => return None,
        Some(_) => {}
        None if block.room != Room::Open => return None,
        None => {}
    }
    let change = RecordChange::Put { key: key.to_owned(), to: to.clone() };
    Some(set(&block.name, &records_after(&block.stored, &change)?))
}

/// The `set` of `name` to `records`.
fn set(name: &str, records: &[StoredRecord]) -> GoalWrite {
    GoalWrite { name: name.to_owned(), change: GoalChange::Set(records_json(records)) }
}

/// The key of the record a refusal's JSON pointer `pointer` names in the array `value_json` that
/// was sent: a records value's part at fault is named `/i/...` (`contracts/standard-pack-goals.md`
/// 152-165 at aa6765dd); the `key` of the item at `i`, none of another pointer or item.
pub fn pointed_record(value_json: &str, pointer: &str) -> Option<String> {
    let place = pointer.strip_prefix('/')?.split('/').next()?;
    if place.is_empty() || !place.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let place: usize = place.parse().ok()?;
    let value: Value = serde_json::from_str(value_json).ok()?;
    Some(value.as_array()?.get(place)?.get("key")?.as_str()?.to_owned())
}

/// One ladder of a table, by the table's name: a class's of a column, read once for every key of
/// the class, or a key's own of a column (and sub) (`runtime-interface.md` 861-862 at aa6765dd).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ladder {
    pub table: String,
    pub of: LadderOf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LadderOf {
    Class { class: String, column: String },
    Key { key: String, column: String, sub: Option<String> },
}

impl LadderOf {
    /// As the wire spells it.
    fn wire(&self) -> GoalLadderRef {
        match self {
            Self::Class { class, column } => {
                GoalLadderRef::Class { class: class.clone(), column: column.clone() }
            }
            Self::Key { key, column, sub } => {
                GoalLadderRef::Key { key: key.clone(), column: column.clone(), sub: sub.clone() }
            }
        }
    }
}

/// The part `options` as the Runtime answered it, kept whole until it is shown
/// ([`OptionsLoaded::table`]): the accepted tables' sha256 it was read over, until when the lists
/// it read hold, and every row in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsLoaded {
    tables: Option<String>,
    fresh_until: Option<u64>,
    rows: Vec<GoalPageRow>,
}

impl OptionsLoaded {
    /// The sha256 of the accepted tables it was read over.
    pub fn tables_sha256(&self) -> Option<&str> {
        self.tables.as_deref()
    }

    /// Whether the lists it read (each key's held state and position now) still hold at `now_ms`:
    /// until the earliest expiry among them, which the page names (`runtime-interface.md` 828-829
    /// at aa6765dd); always, when it names none.
    pub fn fresh_at(&self, now_ms: u64) -> bool {
        self.fresh_until.map_or(true, |until| now_ms < until)
    }

    /// The table of the entry it is of, named `table`, in `tongue` ([`table_options`]).
    pub fn table(&self, table: &str, tongue: Tongue) -> TableOptions {
        table_options(&self.rows, table, tongue)
    }
}

/// A table as the picker shows it: its columns, then its keys, each in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableOptions {
    pub columns: Vec<ColumnOption>,
    pub keys: Vec<KeyOption>,
}

impl TableOptions {
    /// The column of `column`, when the table has it.
    pub fn column(&self, column: &str) -> Option<&ColumnOption> {
        self.columns.iter().find(|option| option.column == column)
    }

    /// The key of `key`, when the table has it.
    pub fn key(&self, key: &str) -> Option<&KeyOption> {
        self.keys.iter().find(|option| option.key == key)
    }
}

/// A column: its id, its name in the language shown (its id when the pack names none), its subs
/// by id with their names likewise, and what it requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnOption {
    pub column: String,
    pub name: String,
    pub subs: Vec<(String, String)>,
    pub requires: Vec<Requirement>,
}

/// One prerequisite of a column, as the file writes it, in words: another column (its name or id)
/// at a position, before any step or, with `from`, before a step that arrives at that position of
/// the requiring column's own ladder; or a condition met in the game (its name or id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requirement {
    Column { column: String, at_least: String, from: Option<String> },
    Condition(String),
}

/// A key of the table: its id, its name in the language shown, its class's name (its id when the
/// pack names none), its held state, its ladders, and the words a search finds it by (its names in
/// every language and its id, in lower case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyOption {
    pub key: String,
    pub name: Option<String>,
    pub class: String,
    pub held: KeyHeld,
    pub lanes: Vec<KeyLaneOption>,
    words: Vec<String>,
}

/// One ladder a key has: its column and sub by id and by name, its size, the key's position now by
/// its index (for a key held) or why it is not known, and the ladder it walks there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLaneOption {
    pub column: String,
    pub sub: Option<String>,
    pub column_name: String,
    pub sub_name: Option<String>,
    pub positions: u16,
    pub current: Option<u16>,
    pub unknown: Option<LaneUnknown>,
    pub ladder: Ladder,
}

/// The table of `rows`, the part `options` of an entry whose table is `table`, in `tongue`: the
/// column rows, the class rows and the key rows in file order (`runtime-interface.md` 856-860 at
/// aa6765dd); a key's ladder is its class's of a column of `per: "class"` (`source: class`), else
/// its own, of its sub for a column of `per: "sub"`.
pub fn table_options(rows: &[GoalPageRow], table: &str, tongue: Tongue) -> TableOptions {
    type ColumnRow<'a> =
        (&'a String, Option<&'a GoalPageText>, &'a Vec<GoalPageSub>, &'a Vec<GoalPageRequirement>);
    let mut column_rows: Vec<ColumnRow<'_>> = Vec::new();
    let mut classes: BTreeMap<&str, Option<String>> = BTreeMap::new();
    for row in rows {
        match row {
            GoalPageRow::Column { column, name, subs, requires, .. } => {
                column_rows.push((column, name.as_ref(), subs, requires));
            }
            GoalPageRow::Class { class, name } => {
                classes.insert(class.as_str(), named(name.as_ref(), tongue));
            }
            _ => {}
        }
    }
    let column_name = |id: &str| -> String {
        column_rows
            .iter()
            .find(|(column, ..)| column.as_str() == id)
            .and_then(|(_, name, ..)| named(*name, tongue))
            .unwrap_or_else(|| id.to_owned())
    };
    let columns: Vec<ColumnOption> = column_rows
        .iter()
        .map(|(column, name, subs, requires)| ColumnOption {
            column: (*column).clone(),
            name: named(*name, tongue).unwrap_or_else(|| (*column).clone()),
            subs: subs
                .iter()
                .map(|sub| {
                    let name = named(sub.name.as_ref(), tongue);
                    (sub.sub.clone(), name.unwrap_or_else(|| sub.sub.clone()))
                })
                .collect(),
            requires: requires
                .iter()
                .map(|requirement| match requirement {
                    GoalPageRequirement::Column(required) => Requirement::Column {
                        column: column_name(&required.column),
                        at_least: position_text(
                            &required.at_least,
                            required.at_least_said.as_ref(),
                            tongue,
                        ),
                        from: required
                            .from
                            .as_ref()
                            .map(|from| position_text(from, required.from_said.as_ref(), tongue)),
                    },
                    GoalPageRequirement::Condition(condition) => {
                        Requirement::Condition(condition_text(condition, false, tongue))
                    }
                })
                .collect(),
        })
        .collect();
    let keys = rows
        .iter()
        .filter_map(|row| {
            let GoalPageRow::Key { key, name, class, held, lanes } = row else {
                return None;
            };
            let mut words: Vec<String> = name
                .as_ref()
                .map(|name| name.0.iter().map(|(_, text)| text.to_lowercase()).collect::<Vec<_>>())
                .unwrap_or_default();
            words.push(key.to_lowercase());
            let lanes = lanes
                .iter()
                .map(|lane| {
                    let of = match lane.source {
                        GoalLadderSource::Class => {
                            LadderOf::Class { class: class.clone(), column: lane.column.clone() }
                        }
                        GoalLadderSource::Own => LadderOf::Key {
                            key: key.clone(),
                            column: lane.column.clone(),
                            sub: lane.sub.clone(),
                        },
                    };
                    let column = columns.iter().find(|column| column.column == lane.column);
                    let column_name =
                        column.map_or_else(|| lane.column.clone(), |column| column.name.clone());
                    KeyLaneOption {
                        column: lane.column.clone(),
                        sub: lane.sub.clone(),
                        column_name,
                        sub_name: lane.sub.as_ref().map(|sub| {
                            column
                                .and_then(|column| column.subs.iter().find(|(id, _)| id == sub))
                                .map_or_else(|| sub.clone(), |(_, name)| name.clone())
                        }),
                        positions: lane.positions,
                        current: lane.current.as_ref().map(|current| current.index),
                        unknown: lane.unknown.as_ref().map(LaneUnknown::of),
                        ladder: Ladder { table: table.to_owned(), of },
                    }
                })
                .collect();
            let class_name = classes.get(class.as_str()).cloned().flatten();
            Some(KeyOption {
                key: key.clone(),
                name: named(name.as_ref(), tongue),
                class: class_name.unwrap_or_else(|| class.clone()),
                held: key_held(held),
                lanes,
                words,
            })
        })
        .collect();
    TableOptions { columns, keys }
}

/// The keys of `keys` that `query` finds, around spaces and in any case, in a name of any language
/// or in the id, all of them for none; of the keys held only, but with `all`; in file order, the
/// first [`KEYS_SHOWN`] by their places among `keys`, and how many more it finds.
pub fn key_matches(keys: &[KeyOption], query: &str, all: bool) -> (Vec<usize>, usize) {
    let query = query.trim().to_lowercase();
    let mut found: Vec<usize> = Vec::new();
    let mut more = 0;
    for (at, key) in keys.iter().enumerate() {
        let shown = all || key.held == KeyHeld::Held;
        let finds = query.is_empty() || key.words.iter().any(|word| word.contains(query.as_str()));
        if !(shown && finds) {
            continue;
        }
        if found.len() == KEYS_SHOWN {
            more += 1;
        } else {
            found.push(at);
        }
    }
    (found, more)
}

/// The part `positions` of one ladder as the Runtime answered it: the accepted tables' sha256 it
/// was read over, and its rows in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionsLoaded {
    tables: Option<String>,
    rows: Vec<GoalPageRow>,
}

/// One position of a ladder: its index, its numbers and how it is shown ([`position_text`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionOption {
    pub index: u16,
    pub position: Vec<i64>,
    pub text: String,
}

impl PositionsLoaded {
    /// The sha256 of the accepted tables it was read over.
    pub fn tables_sha256(&self) -> Option<&str> {
        self.tables.as_deref()
    }

    /// Its positions in order, each shown in `tongue`.
    pub fn shown(&self, tongue: Tongue) -> Vec<PositionOption> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                GoalPageRow::Position { index, position, said } => Some(PositionOption {
                    index: *index,
                    position: position.clone(),
                    text: position_text(position, said.as_ref(), tongue),
                }),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        after_refusal, at_or_before, default_target, holds_part, key_matches, lane_target, placed,
        pointed_record, position_text, record_key, records_after, records_json, records_key, room,
        shown, table_options, to_of, AfterRefusal, KeyHeld, LadderOf, LaneTarget, LaneUnknown,
        ListUnknown, RecordChange, RecordsLoaded, Requirement, Room, Slot, StoredRecord,
        APPLICATION_RECORDS, KEYS_SHOWN,
    };
    use crate::workbench::goals::WriteProblem;
    use acui_rows::outcome::Tongue;
    use actingcommand_contract::{
        GoalDeclarationRead, GoalPageRecordsState, GoalPageRow, GoalPageText,
    };
    use serde_json::{json, Map, Value};

    /// A row as the wire carries it, read through the contract's own type, so that every member
    /// named here is one the pinned contract has.
    fn row(value: Value) -> GoalPageRow {
        serde_json::from_value(value).expect("a row the pinned contract reads")
    }

    fn map(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn stored(key: &str, to: Value) -> StoredRecord {
        StoredRecord { key: key.to_owned(), to: map(to) }
    }

    fn text(pairs: &[(&str, &str)]) -> GoalPageText {
        let texts = pairs.iter().map(|(locale, text)| ((*locale).to_owned(), (*text).to_owned()));
        GoalPageText(texts.collect())
    }

    fn set() -> Value {
        json!({"unix_ms": 1_794_500_000_000_u64, "channel": "ui"})
    }

    fn entry(name: &str, after: Option<&str>, applies: bool, total: u16, max: u64) -> Value {
        let mut row = json!({
            "row": "entry",
            "name": name,
            "declared": true,
            "entry": {
                "name": name,
                "type": "records",
                "title": {"en": "Raise", "zh-Hans": "培养"},
                "help": {"en": "Whom to raise"},
                "table": "heroes",
                "max_records": max,
            },
            "applies": applies,
            "records_total": total,
        });
        if let Some(after) = after {
            row["after_entry"] = json!(after);
        }
        if !applies {
            row["problem"] = json!("name_unknown");
        }
        row
    }

    fn loaded(state: Option<GoalPageRecordsState>, rows: Vec<Value>) -> RecordsLoaded {
        RecordsLoaded {
            version: 9,
            tables: Some(format!("sha256:{}", "c".repeat(64))),
            state,
            rows: rows.into_iter().map(row).collect(),
        }
    }

    #[test]
    fn a_page_not_in_memory_holds_nothing_of_its_part() {
        for state in [
            GoalDeclarationRead::NotCheckedYet,
            GoalDeclarationRead::RootFailing,
            GoalDeclarationRead::NoSchedule,
        ] {
            assert!(!holds_part(state), "{state:?}");
        }
        for state in [
            GoalDeclarationRead::Absent,
            GoalDeclarationRead::Accepted,
            GoalDeclarationRead::Refused,
            GoalDeclarationRead::Unreadable,
            GoalDeclarationRead::NotRead,
        ] {
            assert!(holds_part(state), "{state:?}");
        }
    }

    #[test]
    fn a_moved_part_starts_again_once_then_gives_up_and_only_a_moved_declaration_drops_caches() {
        assert_eq!(
            after_refusal("goal_page_view_records_moved", false),
            AfterRefusal::Restart { drop: false }
        );
        assert_eq!(
            after_refusal("goal_page_view_records_moved", true),
            AfterRefusal::GiveUp { drop: false }
        );
        assert_eq!(
            after_refusal("goal_page_view_declaration_moved", false),
            AfterRefusal::Restart { drop: true }
        );
        assert_eq!(
            after_refusal("goal_page_view_declaration_moved", true),
            AfterRefusal::GiveUp { drop: true }
        );
        for restarted in [false, true] {
            assert_eq!(
                after_refusal("goal_page_view_declaration_absent", restarted),
                AfterRefusal::KeepCached
            );
            for other in [
                "goal_page_view_refused",
                "invalid_goal_page_view",
                "runtime_unavailable",
                "",
            ] {
                assert_eq!(after_refusal(other, restarted), AfterRefusal::Fail, "{other}");
            }
        }
    }

    #[test]
    fn blocks_go_after_the_entry_they_name_in_file_order_first_without_one_last_when_not_found() {
        let entries = ["heading", "gold", "switch"];
        // After an entry; after another block; first; a name not on the page; after the block
        // placed last.
        let blocks = [
            ("raise", Some("gold")),
            ("raise_more", Some("raise")),
            ("first", None),
            ("lost", Some("gone")),
            ("after_lost", Some("lost")),
        ];
        assert_eq!(
            placed(&entries, &blocks),
            vec![
                Slot::Block(2),
                Slot::Entry(0),
                Slot::Entry(1),
                Slot::Block(0),
                Slot::Block(1),
                Slot::Entry(2),
                Slot::Block(3),
                Slot::Block(4),
            ]
        );
        // Two blocks naming one entry keep their file order.
        assert_eq!(
            placed(&["gold"], &[("a", Some("gold")), ("b", Some("gold"))]),
            vec![Slot::Entry(0), Slot::Block(0), Slot::Block(1)]
        );
        // A page with no entry placed: every block in file order.
        assert_eq!(
            placed(&[], &[("a", Some("heading")), ("b", Some("a"))]),
            vec![Slot::Block(0), Slot::Block(1)]
        );
        assert_eq!(placed(&["gold"], &[]), vec![Slot::Entry(0)]);
    }

    #[test]
    fn a_put_replaces_its_record_in_place_and_keeps_every_other_to_as_answered() {
        let records = [
            stored("hero_a", json!({"phase": [2]})),
            stored("hero_b", json!({"mastery": {"1": [2]}, "level": [1, 2]})),
            stored("hero_c", json!({"phase": [1]})),
        ];
        let change = RecordChange::Put { key: "hero_b".to_owned(), to: map(json!({"phase": [0]})) };
        let after = records_after(&records, &change).expect("a put");
        assert_eq!(
            after,
            vec![
                stored("hero_a", json!({"phase": [2]})),
                stored("hero_b", json!({"phase": [0]})),
                stored("hero_c", json!({"phase": [1]})),
            ]
        );
        let sent: Value = serde_json::from_str(&records_json(&after)).expect("JSON");
        assert_eq!(
            sent,
            json!([
                {"key": "hero_a", "to": {"phase": [2]}},
                {"key": "hero_b", "to": {"phase": [0]}},
                {"key": "hero_c", "to": {"phase": [1]}},
            ])
        );
    }

    #[test]
    fn a_put_of_a_key_not_recorded_goes_at_the_end() {
        let records = [stored("hero_a", json!({"phase": [2]}))];
        let to = map(json!({"level": [2, 2]}));
        let change = RecordChange::Put { key: "hero_z".to_owned(), to };
        let after = records_after(&records, &change).expect("a put");
        let sent: Value = serde_json::from_str(&records_json(&after)).expect("JSON");
        assert_eq!(
            sent,
            json!([
                {"key": "hero_a", "to": {"phase": [2]}},
                {"key": "hero_z", "to": {"level": [2, 2]}},
            ])
        );
    }

    #[test]
    fn removing_a_record_keeps_the_others_in_order_and_the_last_one_sends_an_empty_array() {
        let records = [
            stored("hero_a", json!({"phase": [2]})),
            stored("hero_b", json!({"phase": [1]})),
        ];
        let after = records_after(&records, &RecordChange::Remove("hero_a".to_owned()))
            .expect("a removal");
        assert_eq!(after, vec![stored("hero_b", json!({"phase": [1]}))]);
        let last = records_after(&after, &RecordChange::Remove("hero_b".to_owned()))
            .expect("a removal");
        assert_eq!(records_json(&last), "[]");
        // A key it does not hold removes nothing.
        assert_eq!(records_after(&after, &RecordChange::Remove("hero_x".to_owned())), None);
    }

    #[test]
    fn a_to_names_a_column_by_its_position_and_a_sub_column_by_its_subs() {
        let target = |column: &str, sub: Option<&str>, position: Option<Vec<i64>>| LaneTarget {
            column: column.to_owned(),
            sub: sub.map(str::to_owned),
            position,
        };
        let to = to_of(&[
            target("phase", None, Some(vec![2])),
            target("level", None, Some(vec![2, 2])),
            target("skill", None, None),
            target("mastery", Some("1"), Some(vec![2])),
            target("mastery", Some("2"), None),
            target("mastery", Some("3"), Some(vec![1])),
            target("bond", Some("1"), None),
        ]);
        assert_eq!(
            Value::Object(to),
            json!({"phase": [2], "level": [2, 2], "mastery": {"1": [2], "3": [1]}})
        );
        assert!(to_of(&[target("phase", None, None)]).is_empty());
    }

    #[test]
    fn a_lane_starts_at_its_last_position_or_at_what_its_record_holds() {
        let positions = vec![vec![0], vec![1], vec![2]];
        // A new character: the last position.
        assert_eq!(default_target(&positions, None, "phase", None), Some(2));
        // A character recorded: its target, no target where its record names none, the last
        // where its record names one the ladder no longer holds.
        let to = map(json!({"phase": [1], "mastery": {"1": [0]}, "level": [9]}));
        assert_eq!(default_target(&positions, Some(&to), "phase", None), Some(1));
        assert_eq!(default_target(&positions, Some(&to), "mastery", Some("1")), Some(0));
        assert_eq!(default_target(&positions, Some(&to), "mastery", Some("2")), None);
        assert_eq!(default_target(&positions, Some(&to), "skill", None), None);
        assert_eq!(default_target(&positions, Some(&to), "level", None), Some(2));
        // A ladder without positions has none.
        assert_eq!(default_target(&[], None, "phase", None), None);
        assert_eq!(lane_target(&to, "mastery", Some("1")), Some(vec![0]));
        assert_eq!(lane_target(&to, "mastery", None), None);
    }

    #[test]
    fn positions_at_or_before_the_current_one_are_greyed_and_none_without_a_current() {
        assert!(at_or_before(0, Some(1)));
        assert!(at_or_before(1, Some(1)));
        assert!(!at_or_before(2, Some(1)));
        assert!(!at_or_before(0, None));
    }

    #[test]
    fn a_position_is_said_by_the_pack_else_by_its_numbers() {
        let said = text(&[("en", "Phase 2"), ("zh-Hans", "第二阶段")]);
        assert_eq!(position_text(&[2], Some(&said), Tongue::En), "Phase 2");
        assert_eq!(position_text(&[2], Some(&said), Tongue::ZhHans), "第二阶段");
        // Only another language written: that one, the console's one fallback.
        let french = text(&[("fr", "Niveau 2")]);
        assert_eq!(position_text(&[0, 2], Some(&french), Tongue::En), "Niveau 2");
        assert_eq!(position_text(&[1, 2], None, Tongue::En), "1·2");
        assert_eq!(position_text(&[3], None, Tongue::ZhHans), "3");
    }

    #[test]
    fn an_entry_has_room_below_its_bound_and_the_applications_128() {
        assert_eq!(room(true, true, false, 2, Some(3), 10), Room::Open);
        assert_eq!(room(true, true, false, 3, Some(3), 10), Room::EntryFull);
        assert_eq!(room(true, true, false, 2, None, 10), Room::EntryFull);
        assert_eq!(room(true, true, false, 2, Some(3), APPLICATION_RECORDS), Room::AllFull);
        assert_eq!(room(true, true, false, 2, Some(3), APPLICATION_RECORDS - 1), Room::Open);
        assert_eq!(room(false, true, false, 0, Some(3), 0), Room::Closed);
        assert_eq!(room(true, false, false, 0, Some(3), 0), Room::Closed);
        assert_eq!(room(true, true, true, 0, Some(3), 0), Room::Closed);
    }

    #[test]
    fn a_refusals_pointer_names_the_record_sent_at_its_place() {
        let sent = r#"[{"key":"hero_a","to":{"phase":[2]}},{"key":"hero_b","to":{"level":[9]}}]"#;
        assert_eq!(pointed_record(sent, "/1/to/level"), Some("hero_b".to_owned()));
        assert_eq!(pointed_record(sent, "/0/key"), Some("hero_a".to_owned()));
        assert_eq!(pointed_record(sent, "/0"), Some("hero_a".to_owned()));
        assert_eq!(pointed_record(sent, "/2/key"), None);
        assert_eq!(pointed_record(sent, ""), None);
        assert_eq!(pointed_record(sent, "/x/key"), None);
        assert_eq!(pointed_record("{}", "/0"), None);
    }

    #[test]
    fn the_keys_of_the_line_and_of_its_records_are_told_apart() {
        assert_eq!(records_key("raise"), "records:raise");
        assert_eq!(record_key("raise", "hero:a"), "record:raise:hero:a");
        assert_ne!(records_key("raise"), record_key("raise", ""));
    }

    #[test]
    fn records_are_shown_under_their_entry_with_the_runtimes_lanes_in_its_order() {
        let read = loaded(
            Some(GoalPageRecordsState::Read),
            vec![
                entry("raise", Some("gold"), true, 2, 3),
                json!({
                    "row": "record",
                    "entry": "raise",
                    "key": "hero_a",
                    "key_name": {"en": "First hero", "zh-Hans": "一号"},
                    "to": {"phase": [2], "mastery": {"1": [2]}},
                    "set": set(),
                    "applied": true,
                    "held": {"state": "held", "fact_key": "list.heroes"},
                    "lanes": [
                        {
                            "column": "phase",
                            "column_name": {"en": "Phase"},
                            "source": "class",
                            "target": {"index": 2, "position": [2], "said": {"en": "Phase 2"}},
                            "current": {"index": 0, "position": [0]},
                        },
                        {
                            "column": "skill",
                            "source": "own",
                            "raised_to": {"index": 3, "position": [4]},
                            "unknown": {
                                "code": "goal_records_progress_unknown",
                                "fact_key": "list.skills",
                                "unknown_reason": "fact_expired",
                            },
                        },
                        {
                            "column": "mastery",
                            "sub": "1",
                            "column_name": {"en": "Mastery"},
                            "sub_name": {"en": "First mastery"},
                            "source": "own",
                            "target": {"index": 2, "position": [2]},
                            "unknown": {"code": "goal_records_row_missing", "fact_key": "list.m"},
                            "conditions": [
                                {"condition": "side_quest", "name": {"en": "The side quest"}},
                            ],
                        },
                    ],
                }),
                json!({
                    "row": "record",
                    "entry": "raise",
                    "key": "hero_b",
                    "to": {"phase": [9]},
                    "set": set(),
                    "applied": false,
                    "problem": "out_of_range",
                    "lanes": [],
                }),
                json!({
                    "row": "entry",
                    "name": "old_raise",
                    "declared": false,
                    "applies": false,
                    "problem": "name_unknown",
                    "records_total": 1,
                }),
                json!({
                    "row": "record",
                    "entry": "old_raise",
                    "key": "hero_c",
                    "key_name": {"en": "Third hero"},
                    "to": {"phase": [1]},
                    "set": set(),
                    "applied": false,
                    "problem": "name_unknown",
                    "held": {"state": "unknown", "unknown_reason": "fact_missing"},
                    "lanes": [],
                    "texts_left_out": true,
                }),
            ],
        );
        let shown = shown(&read, Tongue::En);
        assert!(!shown.unreadable);
        assert_eq!(shown.loose, 0);
        assert_eq!(shown.declared.len(), 1);
        let block = &shown.declared[0];
        assert_eq!(block.after.as_deref(), Some("gold"));
        assert_eq!(block.line.title.as_deref(), Some("Raise"));
        assert_eq!(block.line.help.as_deref(), Some("Whom to raise"));
        assert_eq!((block.line.total, block.line.max), (2, Some(3)));
        assert_eq!(block.line.table.as_deref(), Some("heroes"));
        // Three records over the application, two of at most three in the entry.
        assert_eq!(block.line.room, Room::Open);
        assert_eq!(
            block.line.stored,
            vec![
                stored("hero_a", json!({"phase": [2], "mastery": {"1": [2]}})),
                stored("hero_b", json!({"phase": [9]})),
            ]
        );
        let first = &block.records[0];
        assert_eq!(first.name.as_deref(), Some("First hero"));
        assert!(first.applied && first.changeable);
        assert_eq!(first.held, Some(KeyHeld::Held));
        let columns: Vec<&str> = first.lanes.iter().map(|lane| lane.column.as_str()).collect();
        assert_eq!(columns, vec!["Phase", "skill", "Mastery"]);
        assert_eq!(first.lanes[0].target.as_deref(), Some("Phase 2"));
        assert_eq!(first.lanes[0].current.as_deref(), Some("0"));
        assert_eq!(first.lanes[1].target, None);
        assert_eq!(first.lanes[1].raised_to.as_deref(), Some("4"));
        assert_eq!(
            first.lanes[1].unknown,
            Some(LaneUnknown::Progress(Some(ListUnknown::Expired)))
        );
        assert_eq!(first.lanes[2].sub.as_deref(), Some("First mastery"));
        assert_eq!(first.lanes[2].unknown, Some(LaneUnknown::RowMissing));
        assert_eq!(first.lanes[2].conditions, vec!["The side quest".to_owned()]);
        let second = &block.records[1];
        assert_eq!(second.name, None);
        assert!(!second.applied);
        assert_eq!(second.problem, Some(WriteProblem::OutOfRange));
        // The undeclared group: its name only, its records ids only, nothing changed here.
        assert_eq!(shown.undeclared.len(), 1);
        let gone = &shown.undeclared[0];
        assert!(gone.line.clearable());
        assert_eq!(gone.line.room, Room::Closed);
        let third = &gone.records[0];
        assert!(third.ids_only && !third.changeable);
        assert_eq!(third.name, None);
        assert_eq!(third.held, Some(KeyHeld::Unknown(Some(ListUnknown::Missing))));
    }

    #[test]
    fn an_unreadable_record_closes_every_entry_and_offers_each_declared_ones_clear() {
        let read = loaded(
            Some(GoalPageRecordsState::Unreadable),
            vec![entry("raise", None, true, 0, 3), entry("other", Some("raise"), false, 0, 3)],
        );
        let shown = shown(&read, Tongue::En);
        assert!(shown.unreadable);
        for block in &shown.declared {
            assert_eq!(block.line.room, Room::Closed, "{}", block.line.name);
            assert!(block.line.clearable(), "{}", block.line.name);
        }
        // Readable, a declared entry has no clear of its own: removing its last record empties it.
        let read = Some(GoalPageRecordsState::Read);
        let readable = loaded(read, vec![entry("raise", None, true, 0, 3)]);
        let shown = super::shown(&readable, Tongue::En);
        assert!(!shown.declared[0].line.clearable());
        // Not applied: no room, said apart.
        let closed = loaded(read, vec![entry("raise", None, false, 0, 3)]);
        assert_eq!(super::shown(&closed, Tongue::En).declared[0].line.room, Room::Closed);
    }

    #[test]
    fn the_application_bound_counts_every_entrys_records() {
        let read = loaded(
            Some(GoalPageRecordsState::Read),
            vec![entry("raise", None, true, 100, 128), entry("more", Some("raise"), true, 28, 128)],
        );
        // The records rows themselves are not needed for the count: `records_total` is.
        let shown = shown(&read, Tongue::En);
        assert_eq!(shown.declared[0].line.room, Room::AllFull);
        assert_eq!(shown.declared[1].line.room, Room::AllFull);
    }

    fn options_rows() -> Vec<GoalPageRow> {
        [
            json!({
                "row": "column",
                "column": "phase",
                "name": {"en": "Phase", "zh-Hans": "阶段"},
                "per": "class",
                "requires": [],
            }),
            json!({
                "row": "column",
                "column": "mastery",
                "per": "sub",
                "subs": [{"sub": "1", "name": {"en": "First mastery"}}, {"sub": "2"}],
                "requires": [
                    {"column": "phase", "at_least": [2], "at_least_said": {"en": "Phase 2"}},
                    {"column": "skill", "at_least": [4], "from": [1]},
                    {"condition": "side_quest", "name": {"en": "The side quest"}},
                ],
            }),
            json!({"row": "class", "class": "rank_6", "name": {"en": "Class six"}}),
            json!({"row": "class", "class": "rank_5"}),
            json!({
                "row": "key",
                "key": "hero_a",
                "name": {"en": "First hero", "zh-Hans": "一号"},
                "class": "rank_6",
                "held": {"state": "held"},
                "lanes": [
                    {
                        "column": "phase",
                        "source": "class",
                        "positions": 3,
                        "current": {"index": 1, "position": [1]},
                    },
                    {"column": "mastery", "sub": "1", "source": "own", "positions": 3},
                    {"column": "mastery", "sub": "2", "source": "own", "positions": 2,
                     "unknown": {"code": "goal_records_position_unknown"}},
                ],
            }),
            json!({
                "row": "key",
                "key": "hero_b",
                "class": "rank_5",
                "held": {"state": "not_held", "fact_key": "list.heroes"},
                "lanes": [{"column": "phase", "source": "class", "positions": 2}],
            }),
            json!({
                "row": "key",
                "key": "hero_c",
                "name": {"en": "Third hero"},
                "class": "rank_6",
                "held": {"state": "unknown", "unknown_reason": "fact_low_confidence"},
                "lanes": [],
            }),
        ]
        .into_iter()
        .map(row)
        .collect()
    }

    #[test]
    fn a_tables_options_are_its_columns_and_keys_with_their_names_and_ladders() {
        let table = table_options(&options_rows(), "heroes", Tongue::En);
        assert_eq!(table.columns.len(), 2);
        let mastery = table.column("mastery").expect("a column");
        assert_eq!(mastery.name, "mastery");
        assert_eq!(
            mastery.subs,
            vec![("1".to_owned(), "First mastery".to_owned()), ("2".to_owned(), "2".to_owned())]
        );
        assert_eq!(
            mastery.requires,
            vec![
                Requirement::Column {
                    column: "Phase".to_owned(),
                    at_least: "Phase 2".to_owned(),
                    from: None,
                },
                Requirement::Column {
                    column: "skill".to_owned(),
                    at_least: "4".to_owned(),
                    from: Some("1".to_owned()),
                },
                Requirement::Condition("The side quest".to_owned()),
            ]
        );
        let hero = table.key("hero_a").expect("a key");
        assert_eq!(hero.name.as_deref(), Some("First hero"));
        assert_eq!(hero.class, "Class six");
        assert_eq!(hero.lanes[0].column_name, "Phase");
        assert_eq!(hero.lanes[0].current, Some(1));
        // A class's ladder for a column of its class, the key's own for the others.
        assert_eq!(
            hero.lanes[0].ladder.of,
            LadderOf::Class { class: "rank_6".to_owned(), column: "phase".to_owned() }
        );
        assert_eq!(
            hero.lanes[1].ladder.of,
            LadderOf::Key {
                key: "hero_a".to_owned(),
                column: "mastery".to_owned(),
                sub: Some("1".to_owned()),
            }
        );
        assert_eq!(hero.lanes[1].sub_name.as_deref(), Some("First mastery"));
        assert_eq!(hero.lanes[2].unknown, Some(LaneUnknown::PositionUnknown));
        assert_eq!(hero.lanes[0].ladder.table, "heroes");
        let other = table.key("hero_b").expect("a key");
        assert_eq!(other.class, "rank_5");
        assert_eq!(other.held, KeyHeld::NotHeld);
        // The same column of two keys of one class is one ladder.
        assert_eq!(other.lanes[0].ladder.of, LadderOf::Class {
            class: "rank_5".to_owned(),
            column: "phase".to_owned(),
        });
    }

    #[test]
    fn keys_are_found_by_any_name_or_id_and_held_ones_only_by_default() {
        let table = table_options(&options_rows(), "heroes", Tongue::En);
        assert_eq!(key_matches(&table.keys, "", false), (vec![0], 0));
        assert_eq!(key_matches(&table.keys, "", true), (vec![0, 1, 2], 0));
        // A name in another language than the one shown, and an id.
        assert_eq!(key_matches(&table.keys, " 一号 ", false), (vec![0], 0));
        assert_eq!(key_matches(&table.keys, "HERO_B", true), (vec![1], 0));
        assert_eq!(key_matches(&table.keys, "hero_b", false), (Vec::new(), 0));
        assert_eq!(key_matches(&table.keys, "third", true), (vec![2], 0));
        // At most so many, and how many more.
        let many: Vec<_> = (0..KEYS_SHOWN + 5).map(|_| table.keys[0].clone()).collect();
        let (found, more) = key_matches(&many, "", false);
        assert_eq!((found.len(), more), (KEYS_SHOWN, 5));
    }
}
