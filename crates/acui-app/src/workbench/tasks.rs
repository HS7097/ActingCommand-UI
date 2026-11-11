// SPDX-License-Identifier: GPL-3.0-only
//! What an account's daily task list says (UI-3f1), as a pure function of what the reads left: no
//! words and no Slint here, `paint` makes both of a view. Only words: no switch (`SwitchSlot`).
//!
//! A row is a task package. Which packages an account has is read off its own evaluation: each
//! task of it decides for every instance it matches, so each package the Runtime can schedule on
//! the account has a row, and so has the one it runs now. Which package a task runs is the one
//! thing a candidate does not say until SS-7: `package_of` says it, from the `task_ids` of the
//! `packs` answer the names come from, and is the one place that does. A task it finds no package
//! for is listed under the account itself, by its task id; the Runtime's own reading task by
//! words of its own.
//!
//! What a row says is the first of these that holds: `Status` says the account runs the package,
//! for the scheduler or not; the task selected for the account is one of the package's, and then
//! where that dispatch stands, never why; else the package's first candidate row in the Runtime's
//! own order (`ReasonClass`), with the other sentences of its tasks after it, each once. That
//! order is the Runtime's (due rows first, then by wake), not a reason of the package's: nothing
//! says one task ranks after another. No count and no number of the scheduler's is in a view, a
//! code only for the log, and a task id only under the account itself.
//!
//! The rows keep one order, so that a read every 15 seconds moves none: by the groups and the
//! order of the game's `ui.json`, the rest after them under "other"; without the file, by name.

use acui_source::workbench::{
    ActivityKind, DispatchStage, InstanceLive, TaskRow, TasksEntry, TasksRead,
};
use acui_source::ClientFailure;

use super::card::{Down, Line, ReasonClass};
use super::tools::{NameTable, PackTable, PackUi};
use crate::strings::Language;

/// The Runtime's own task of reading an account's data (`_fact_refresh`, a reserved task id).
const FACT_REFRESH: &str = "_fact_refresh";
/// How a read of the list is refused when its answer is larger than one frame, and when the
/// Runtime has no such account.
const TOO_LARGE: &str = "runtime_schedule_explanation_too_large_for_frame";
const INSTANCE_UNKNOWN: &str = "instance_unknown";
/// The most other sentences under a row's; more are said to be more, never how many.
const ALSO_MOST: usize = 2;

/// What the list shows now: why no row does, or its rows.
pub enum TasksView {
    /// No Runtime answers now.
    Down(Down),
    NoAccount,
    /// The list shows anew, of another account or another owner epoch: no answer yet.
    Reading,
    Unreadable(Unreadable),
    /// The Runtime has not evaluated since it started.
    NotEvaluated,
    /// Its latest evaluation decided nothing for the account.
    NotInTable,
    Listed(TaskList),
}

/// Why the list cannot be read now: no row of an earlier answer stands in for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// The read failed, panicked or has no thread, or nothing new came for too long.
    Failed,
    /// The answer would not fit in one frame.
    TooLarge,
    /// The Runtime has no such account.
    UnknownInstance,
}

/// One answer's list.
pub struct TaskList {
    /// The account's pause or fence, as its card says it: none of its tasks starts meanwhile.
    pub held: Option<Line>,
    /// When the evaluation the rows are of ran, in unix ms.
    pub evaluated_at: u64,
    /// The `packs` answer gave no names: every task is under the account itself.
    pub names_unreadable: bool,
    /// The Runtime counted more candidate rows than it answered.
    pub cut_short: bool,
    pub groups: Vec<PackGroup>,
    /// The tasks no package was found for, first the selected one's.
    pub unmatched: Vec<TaskLine>,
    /// Whether their line is open.
    pub fold_open: bool,
}

impl TaskList {
    /// Whether no row offers a switch, which the list's note says until UI-3f2: R-8 says each
    /// package's own source instead.
    pub fn switches_closed(&self) -> bool {
        self.groups
            .iter()
            .flat_map(|group| &group.rows)
            .all(|row| row.switch == SwitchSlot::NotOffered)
    }
}

/// The packages of one group, in the list's order.
pub struct PackGroup {
    pub title: GroupTitle,
    pub rows: Vec<PackRow>,
}

/// A group's title: none when no package has an entry of its game's `ui.json`, which groups
/// nothing; one of the file's groups, by its game and its id; or the packages no group holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupTitle {
    None,
    Group { game: String, id: String },
    Other,
}

/// One package.
pub struct PackRow {
    pub package_id: String,
    pub said: TaskSaid,
    /// The other sentences of its tasks, each once and none its own, in the Runtime's order: at
    /// most `ALSO_MOST`, and whether there were more.
    pub also: Vec<TaskSaid>,
    pub more: bool,
    /// The Runtime derives it irreversible.
    pub irreversible: bool,
    /// Its `ui.json` entry names a risk word.
    pub risky: bool,
    pub switch: SwitchSlot,
}

/// A row's switch: none is offered before UI-3f2, which reads each package's schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchSlot {
    NotOffered,
}

/// A task under the account itself: by its id, or the Runtime's reading task by words.
pub struct TaskLine {
    pub task: TaskName,
    pub said: TaskSaid,
}

pub enum TaskName {
    Id(String),
    FactRefresh,
}

/// What a row says of why its package runs or does not run now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskSaid {
    /// The account runs it for the scheduler, or not.
    Running,
    RunningDirect,
    /// Selected, and its dispatch is under way; while a pause holds, it does not start.
    Scheduled { paused: bool },
    /// Selected, and its dispatch is over: it ran, or it did not start.
    Completed,
    Rejected,
    /// A candidate row's reason: `why` its code with the value that chose its words, for the
    /// log of an `Unlisted` one, and `wake` the row's own; none on an other sentence.
    Reason { class: ReasonClass, why: String, wake: Option<u64> },
}

/// What the list's read left for the account shown: no answer of this account and epoch yet,
/// a failure (none when the read cannot even be had), or the answer.
#[derive(Clone, Copy)]
pub enum TasksNow<'a> {
    Reading,
    Failed(Option<&'a ClientFailure>),
    Read(&'a TasksRead),
}

/// What the list is made of: the account as the newest `Status` lists it, whether a pause holds
/// it, its state above the rows (`held`), its read, the names and the packages' table of the
/// newest `packs` answer while it gave them, whether its line of tasks without a package is open,
/// and the language whose names order rows no `ui.json` orders.
pub struct Listing<'a> {
    pub instance: &'a InstanceLive,
    pub paused: bool,
    pub held: Option<Line>,
    pub read: TasksNow<'a>,
    pub names: Option<&'a NameTable>,
    pub packs: Option<&'a PackTable>,
    pub names_unreadable: bool,
    pub fold_open: bool,
    pub language: Language,
}

/// The package `task` runs, by the installation rows of the newest `packs` answer: the one place
/// a task is put to a package. SS-7 gives every candidate its own, and then only this changes.
pub fn package_of<'a>(packs: Option<&'a PackTable>, task: &str) -> Option<&'a str> {
    packs?.package_of(task)
}

/// What of a card's lines goes above the rows: its main line when that is the account's pause
/// or fence, else a pause it says in its small print (the global one, over a task finishing).
pub fn held(main: Line, notes: Vec<Line>) -> Option<Line> {
    match main {
        Line::Draining { .. }
        | Line::HandingBack
        | Line::Paused { .. }
        | Line::AwaitingPerson { .. }
        | Line::Recovering => Some(main),
        _ => notes.into_iter().find(|line| matches!(line, Line::Paused { .. })),
    }
}

/// The list as its read leaves it.
pub fn view(listing: Listing<'_>) -> TasksView {
    let read = match listing.read {
        TasksNow::Reading => return TasksView::Reading,
        TasksNow::Failed(failure) => return TasksView::Unreadable(unreadable(failure)),
        TasksNow::Read(read) => read,
    };
    let Some(evaluated_at) = read.evaluated_at else {
        return TasksView::NotEvaluated;
    };
    let Some(entry) = &read.entry else {
        return TasksView::NotInTable;
    };
    let (rows, unmatched) = pack_rows(&listing, entry);
    TasksView::Listed(TaskList {
        held: listing.held,
        evaluated_at,
        names_unreadable: listing.names_unreadable,
        cut_short: entry.cut_short,
        groups: grouped(rows, listing.packs, listing.names, listing.language),
        unmatched,
        fold_open: listing.fold_open,
    })
}

/// A failure's own words where it has them.
fn unreadable(failure: Option<&ClientFailure>) -> Unreadable {
    let Some(failure) = failure else {
        return Unreadable::Failed;
    };
    if failure.host.as_ref().is_some_and(|(code, _)| code == TOO_LARGE) {
        Unreadable::TooLarge
    } else if failure.runtime_code.as_deref() == Some(INSTANCE_UNKNOWN) {
        Unreadable::UnknownInstance
    } else {
        Unreadable::Failed
    }
}

/// The packages' rows, in the order each first comes (the package run now, the selected task's,
/// then the candidates'), and the tasks no package was found for.
fn pack_rows(listing: &Listing<'_>, entry: &TasksEntry) -> (Vec<PackRow>, Vec<TaskLine>) {
    let packs = listing.packs;
    // By package, its candidate rows in the Runtime's order.
    let mut found: Vec<(String, Vec<&TaskRow>)> = Vec::new();
    let mut unmatched = Vec::new();
    let running: Option<(&str, TaskSaid)> =
        listing.instance.activity.as_ref().and_then(|activity| {
            let said = match activity.kind {
                ActivityKind::PolicyRun => TaskSaid::Running,
                ActivityKind::DirectRun => TaskSaid::RunningDirect,
                _ => return None,
            };
            Some((activity.package_id.as_deref()?, said))
        });
    if let Some(package) = running.as_ref().map(|(package, _)| *package) {
        place(&mut found, package);
    }
    let selected: Option<(Option<&str>, TaskSaid)> = entry.selected.as_ref().map(|selected| {
        let said = stage_said(selected.stage, listing.paused);
        match package_of(packs, &selected.task_id) {
            Some(package) => (Some(package), said),
            None => {
                let task = task_name(&selected.task_id);
                unmatched.push(TaskLine { task, said: said.clone() });
                (None, said)
            }
        }
    });
    if let Some(package) = selected.as_ref().and_then(|(package, _)| *package) {
        place(&mut found, package);
    }
    for row in &entry.rows {
        match package_of(packs, &row.task_id) {
            Some(package) => {
                let at = place(&mut found, package);
                found[at].1.push(row);
            }
            None => {
                let task = task_name(&row.task_id);
                unmatched.push(TaskLine { task, said: reason_said(row) });
            }
        }
    }
    let rows = found
        .into_iter()
        .map(|(package, tasks)| {
            let running = running
                .as_ref()
                .filter(|(of, _)| *of == package.as_str())
                .map(|(_, said)| said);
            let chosen = selected
                .as_ref()
                .filter(|(of, _)| *of == Some(package.as_str()))
                .map(|(_, said)| said);
            let (said, also, more) = match running.or(chosen) {
                Some(said) => (said.clone(), Vec::new(), false),
                None => reasons(&tasks),
            };
            let facts = packs.and_then(|packs| packs.pack(&package));
            PackRow {
                irreversible: facts.is_some_and(|facts| facts.irreversible),
                risky: facts.and_then(|facts| facts.ui.as_ref()).is_some_and(|ui| ui.risky),
                package_id: package,
                said,
                also,
                more,
                switch: SwitchSlot::NotOffered,
            }
        })
        .collect();
    (rows, unmatched)
}

/// Where `package` is in `found`, put at its end the first time.
fn place(found: &mut Vec<(String, Vec<&TaskRow>)>, package: &str) -> usize {
    if let Some(at) = found.iter().position(|(seen, _)| seen == package) {
        return at;
    }
    found.push((package.to_owned(), Vec::new()));
    found.len() - 1
}

/// A selected task's dispatch, as it stands: under way, or over.
fn stage_said(stage: DispatchStage, paused: bool) -> TaskSaid {
    match stage {
        DispatchStage::Pending | DispatchStage::Intent | DispatchStage::Admitted => {
            TaskSaid::Scheduled { paused }
        }
        DispatchStage::Completed => TaskSaid::Completed,
        DispatchStage::Rejected => TaskSaid::Rejected,
    }
}

fn task_name(task: &str) -> TaskName {
    if task == FACT_REFRESH {
        TaskName::FactRefresh
    } else {
        TaskName::Id(task.to_owned())
    }
}

fn reason_said(row: &TaskRow) -> TaskSaid {
    let (gap, hold) = (row.gap.as_deref(), row.hold.as_deref());
    TaskSaid::Reason {
        class: ReasonClass::of(&row.why, gap, hold),
        why: super::card::reason_logged(&row.why, gap, hold),
        wake: row.wake,
    }
}

/// A package's sentence from its candidate rows: its first one's, with its wake; then each other
/// sentence of them once, without a wake, at most `ALSO_MOST`, and whether there were more.
fn reasons(tasks: &[&TaskRow]) -> (TaskSaid, Vec<TaskSaid>, bool) {
    let Some((first, rest)) = tasks.split_first() else {
        // Not reached: a package has a row only for a task of its own.
        let said =
            TaskSaid::Reason { class: ReasonClass::Unlisted, why: String::new(), wake: None };
        return (said, Vec::new(), false);
    };
    let said = reason_said(first);
    let class_of = |said: &TaskSaid| match said {
        TaskSaid::Reason { class, .. } => Some(*class),
        _ => None,
    };
    let mut also: Vec<TaskSaid> = Vec::new();
    let mut more = false;
    for row in rest {
        let TaskSaid::Reason { class, why, .. } = reason_said(row) else {
            continue;
        };
        let seen = Some(class) == class_of(&said)
            || also.iter().any(|other| class_of(other) == Some(class));
        if seen {
            continue;
        }
        if also.len() == ALSO_MOST {
            more = true;
            break;
        }
        also.push(TaskSaid::Reason { class, why, wake: None });
    }
    (said, also, more)
}

/// The rows in their groups. With no package of a `ui.json` entry, one group without a title,
/// by name; else by the file's groups and order, and after them under "other" the packages no
/// group holds, those of no entry last, by name.
fn grouped(
    rows: Vec<PackRow>,
    packs: Option<&PackTable>,
    names: Option<&NameTable>,
    language: Language,
) -> Vec<PackGroup> {
    let mut rows = rows;
    let listed = |row: &PackRow| entry_of(packs, &row.package_id).is_some();
    let name = |row: &PackRow| {
        let title = names.and_then(|names| names.title(&row.package_id, language));
        title.unwrap_or(row.package_id.as_str()).to_owned()
    };
    if !rows.iter().any(listed) {
        rows.sort_by_cached_key(name);
        return vec![PackGroup { title: GroupTitle::None, rows }];
    }
    rows.sort_by_cached_key(|row| {
        let (group, order) = rank(packs, &row.package_id);
        (group.is_none(), group, order, name(row))
    });
    let mut groups: Vec<PackGroup> = Vec::new();
    for row in rows {
        let ranked = rank(packs, &row.package_id).0.is_some();
        let title = match entry_of(packs, &row.package_id) {
            Some(ui) if ranked => GroupTitle::Group {
                game: ui.game.clone(),
                id: ui.group.clone().unwrap_or_default(),
            },
            Some(_) | None => GroupTitle::Other,
        };
        match groups.last_mut() {
            Some(group) if group.title == title => group.rows.push(row),
            _ => groups.push(PackGroup { title, rows: vec![row] }),
        }
    }
    groups
}

/// `package`'s entry of its game's `ui.json`, when the table has one.
fn entry_of<'a>(packs: Option<&'a PackTable>, package: &str) -> Option<&'a PackUi> {
    packs?.pack(package)?.ui.as_ref()
}

/// Where `package`'s group stands among its game's, `None` for none of the file's; and its own
/// place in the file, after every entry when it has none.
fn rank(packs: Option<&PackTable>, package: &str) -> (Option<usize>, usize) {
    let Some(ui) = entry_of(packs, package) else {
        return (None, usize::MAX);
    };
    let group = ui.group.as_deref().and_then(|group| packs?.group_rank(&ui.game, group));
    (group, ui.order)
}
