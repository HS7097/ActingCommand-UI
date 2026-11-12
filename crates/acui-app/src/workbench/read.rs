// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's reading threads, and the one table of what they read and how
//! often. The Runtime states no frequency for any of this: the numbers are
//! estimates, and only here.
//!
//! `acui-live` reads `Status` alone, on a connection of its own: the client holds
//! its lock for a whole exchange, `ScheduleExplanation` waits on the policy lock and
//! `RuntimeFactSnapshot` on the fact write gate and the settings' gate, so the
//! Runtime's lamp must not share a connection with them. `acui-slow` reads those two
//! one after the other, and a poke reads both, or the one it names: going to the settings
//! page reads the fact snapshot alone.
//!
//! `acui-slow` also reads the `StartReport`, which only a new start changes: once for each
//! owner epoch, when the event loop names it, and never on a clock of its own. One that
//! fails is named again `START_RETRY` later, `START_TRIES` times in all for the epoch.
//!
//! And while an account's daily task list shows (UI-3f1), the `ScheduleExplanation` of that
//! one account with its candidate rows, on the explanation's clock: the event loop names the
//! account in a slot of its own (`TasksFor`), which the thread reads when that read comes due,
//! and with the slot empty it never comes due. It waits on the policy lock as the cards' does.
//! Beside it, of the same account named in the same slot, its task switches (UI-3f2,
//! `PackScheduleView`), on a slower clock of their own: they change only as a person presses one,
//! as another tool writes them, or as a row of them ends, and each read holds the fact write gate
//! a moment. Not at all in an owner epoch whose Runtime said it has none: the slot says so.
//!
//! A thread reads what is due, hands each result to the event loop, and sleeps
//! until the next is due or it is poked. It ends once the event loop lets its
//! sender go; nothing joins it, since it may be stuck in a read. A panic in a read
//! costs that read and the connection, is said in red, and the next round goes on.
//!
//! Every read states how many writes had landed when it began (`super::wrote`): one
//! begun before a write landed may say what that write changed, and does not count.
//!
//! Two lines of the technical log are what a limit on reads is checked by: a read
//! that took long, as it happens, and every read's count and longest time at the
//! end of the run (`Tally`).

use std::any::Any;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acui_source::workbench::pack::PacksRead;
use acui_source::workbench::{ConfigRead, ExplainRead, LiveRead, ReadLane, StartRead, TasksRead};
use acui_source::ClientFailure;

use crate::launcher::spawn_worker;
use crate::report;

/// `READS` has one row for each `Read`, in its order: `Tally` counts by it.
const READ_KINDS: usize = 6;

#[rustfmt::skip]
static READS: [Cadence; READ_KINDS] = [
    Cadence { read: Read::Status,  lane: Lane::Live, every: Some(Every { shown: secs(3),  minimized: Some(secs(30)), failed: secs(5) }) },
    Cadence { read: Read::Explain, lane: Lane::Slow, every: Some(Every { shown: secs(15), minimized: None,           failed: secs(30) }) },
    Cadence { read: Read::Config,  lane: Lane::Slow, every: Some(Every { shown: secs(30), minimized: None,           failed: secs(30) }) },
    // Once for each owner epoch, when the event loop names it.
    Cadence { read: Read::Start,   lane: Lane::Slow, every: None },
    // Only while an account's task list shows: the slot names it (`TasksFor`).
    Cadence { read: Read::Tasks,   lane: Lane::Slow, every: Some(Every { shown: secs(15), minimized: None,           failed: secs(30) }) },
    // The same list's switches, while the slot names them too (`Listed::packs`).
    Cadence { read: Read::Packs,   lane: Lane::Slow, every: Some(Every { shown: secs(60), minimized: None,           failed: secs(60) }) },
];

/// The account whose daily task list shows, while it does (UI-3f1), and whether its switches are
/// read (UI-3f2): the event loop writes it, and the slow thread reads it as either read comes due.
/// Empty, neither is ever due.
pub type TasksFor = Arc<Mutex<Option<Listed>>>;

/// What the slot names: the account, and whether its switches are read beside its list, which they
/// are not in an owner epoch whose Runtime said it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub alias: String,
    pub packs: bool,
}

/// How many times in one owner epoch the start report is asked for, and how long after a
/// failure it is asked again.
pub const START_TRIES: u8 = 3;
pub const START_RETRY: Duration = secs(30);

/// A read longer than this leaves a line in the technical log.
const LONG_READ: Duration = secs(2);

/// A schedule explanation not read again for this long, the cards' or an account's task
/// list's, no longer counts as what the schedule says now. Far above the slow thread's own
/// times, so a read that waits behind a slow one is not called unreadable.
pub const EXPLAIN_TOO_OLD: Duration = secs(60);

/// An account's switches not read again for this long no longer count as what holds now: three
/// of their reads' times, so a read that waits behind a slow one is not called unreadable.
pub const PACKS_TOO_OLD: Duration = secs(180);

/// How long a thread with nothing due sleeps: a window coming back pokes it.
const NOTHING_DUE: Duration = secs(3600);

const fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// One read's place in the table.
struct Cadence {
    read: Read,
    lane: Lane,
    /// How often; `None`: only when a poke names it.
    every: Option<Every>,
}

/// How often a read is made by its thread's own clock.
struct Every {
    /// While the workbench is not minimized.
    shown: Duration,
    /// While it is; `None`: not at all, until a window coming back pokes the thread.
    minimized: Option<Duration>,
    /// How soon after a failed read, while the workbench is not minimized.
    failed: Duration,
}

/// One kind of read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    Status,
    Explain,
    Config,
    Start,
    /// An account's daily task list (UI-3f1).
    Tasks,
    /// That list's switches (UI-3f2).
    Packs,
}

impl Read {
    fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Explain => "schedule explanation",
            Self::Config => "fact snapshot",
            Self::Start => "start report",
            Self::Tasks => "task list",
            Self::Packs => "pack schedule",
        }
    }

    /// The thread that makes this read.
    pub fn lane(self) -> Lane {
        READS[self as usize].lane
    }
}

/// One reading thread, with its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Live,
    Slow,
}

impl Lane {
    /// The thread's name, also what its lines in the technical log are under.
    pub fn name(self) -> &'static str {
        match self {
            Self::Live => "acui-live",
            Self::Slow => "acui-slow",
        }
    }

    /// The connection's io timeout.
    fn io(self) -> Duration {
        match self {
            Self::Live => secs(5),
            Self::Slow => secs(20),
        }
    }
}

/// Reads now, without waiting for their time: every read of the thread its clock makes, only
/// the one named, or an account's task list and its switches, both of a list shown anew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poke {
    All,
    Only(Read),
    List,
}

impl Poke {
    /// Whether it asks for `read`. The start report is asked for by name only.
    pub fn asks(self, read: Read) -> bool {
        match self {
            Self::All => READS[read as usize].every.is_some(),
            Self::Only(only) => only == read,
            Self::List => matches!(read, Read::Tasks | Read::Packs),
        }
    }
}

/// What a read came to, for the event loop.
pub enum Landing {
    Live(LiveRead),
    Explain(Result<ExplainRead, ClientFailure>),
    Config(Result<ConfigRead, ClientFailure>),
    Start(Result<StartRead, ClientFailure>),
    /// Of the account the slot named: a failure says which, as an answer does.
    Tasks { alias: String, read: Result<TasksRead, ClientFailure> },
    /// The same account's switches, likewise.
    Packs { alias: String, read: Result<PacksRead, ClientFailure> },
    /// The read panicked: the panic's message, for the log.
    Panicked { read: Read, message: String },
}

impl Landing {
    fn failed(&self) -> bool {
        !matches!(
            self,
            Self::Live(LiveRead::Read(_))
                | Self::Explain(Ok(_))
                | Self::Config(Ok(_))
                | Self::Start(Ok(_))
                | Self::Tasks { read: Ok(_), .. }
                | Self::Packs { read: Ok(_), .. }
        )
    }
}

/// How many of each read this run made, and the longest each took.
#[derive(Default)]
pub struct Tally {
    reads: [ReadTally; READ_KINDS],
}

#[derive(Default)]
struct ReadTally {
    count: AtomicU64,
    longest_ms: AtomicU64,
}

impl Tally {
    fn note(&self, read: Read, took: Duration) {
        let tally = &self.reads[read as usize];
        tally.count.fetch_add(1, Ordering::SeqCst);
        let took = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        tally.longest_ms.fetch_max(took, Ordering::SeqCst);
    }

    /// The line the technical log gets at the end of the run.
    pub fn line(&self) -> String {
        let reads: Vec<String> = READS
            .iter()
            .map(|cadence| {
                let tally = &self.reads[cadence.read as usize];
                format!(
                    "{}: {} reads, longest {} ms",
                    cadence.read.name(),
                    tally.count.load(Ordering::SeqCst),
                    tally.longest_ms.load(Ordering::SeqCst),
                )
            })
            .collect();
        reads.join("; ")
    }
}

/// One read's clock on its thread.
struct Due {
    cadence: &'static Cadence,
    last: Option<Instant>,
    failed: bool,
    poked: bool,
}

impl Due {
    /// When this read is next due; `None` while the table stops it. A poke waits
    /// out a stop: the window coming back reads it anyway. One without a clock is due
    /// only when poked, minimized or not. The task list's is never due while no account's
    /// list shows (`listed`), poked or not: it waits for one; nor are its switches' while the
    /// slot does not name them.
    fn at(&self, shown: bool, listed: Option<&Listed>, now: Instant) -> Option<Instant> {
        let waits = match self.cadence.read {
            Read::Tasks => listed.is_none(),
            Read::Packs => !listed.is_some_and(|listed| listed.packs),
            Read::Status | Read::Explain | Read::Config | Read::Start => false,
        };
        if waits {
            return None;
        }
        let Some(clock) = &self.cadence.every else {
            return self.poked.then_some(now);
        };
        let every = match (shown, self.failed) {
            (true, false) => clock.shown,
            (true, true) => clock.failed,
            (false, _) => clock.minimized?,
        };
        match self.last {
            Some(last) if !self.poked => Some(last + every),
            _ => Some(now),
        }
    }
}

/// `lane`'s thread: the sender is what keeps it reading.
pub fn spawn(
    lane: Lane,
    root: PathBuf,
    shown: Arc<AtomicBool>,
    tasks_for: TasksFor,
    tally: Arc<Tally>,
    write_gen: Arc<AtomicU64>,
) -> io::Result<Sender<Poke>> {
    let (sender, commands) = mpsc::channel();
    spawn_worker(lane.name(), move || {
        run(lane, root, &commands, &shown, &tasks_for, &tally, &write_gen)
    })?;
    Ok(sender)
}

fn run(
    lane: Lane,
    root: PathBuf,
    commands: &Receiver<Poke>,
    shown: &AtomicBool,
    tasks_for: &TasksFor,
    tally: &Tally,
    write_gen: &AtomicU64,
) {
    let mut connection = ReadLane::new(root.clone(), lane.io());
    let mut plan: Vec<Due> = READS
        .iter()
        .filter(|cadence| cadence.lane == lane)
        .map(|cadence| Due { cadence, last: None, failed: false, poked: false })
        .collect();
    // The account the task list's and its switches' reads are of as they come due; none, they do
    // not come due.
    let listed = || tasks_for.lock().unwrap_or_else(|held| held.into_inner()).clone();
    loop {
        let now = Instant::now();
        let is_shown = shown.load(Ordering::SeqCst);
        let is_listed = listed();
        let next = plan.iter().filter_map(|due| due.at(is_shown, is_listed.as_ref(), now)).min();
        let wait = next.map_or(NOTHING_DUE, |at| at.saturating_duration_since(now));
        match commands.recv_timeout(wait) {
            Ok(poke) => mark(&mut plan, poke),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Pokes queued behind that one ask for their reads in the same round.
        loop {
            match commands.try_recv() {
                Ok(poke) => mark(&mut plan, poke),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for due in &mut plan {
            let now = Instant::now();
            let listed_now = listed();
            let is_due = due
                .at(shown.load(Ordering::SeqCst), listed_now.as_ref(), now)
                .is_some_and(|at| at <= now);
            if !is_due {
                continue;
            }
            let read = due.cadence.read;
            // Before the read: the event loop lets it go if a write lands meanwhile.
            let generation = write_gen.load(Ordering::SeqCst);
            let asked = listed_now.as_ref().map(|listed| listed.alias.as_str());
            let landing = match catch_unwind(AssertUnwindSafe(|| {
                read_on(read, &mut connection, asked)
            })) {
                Ok(landing) => landing,
                Err(panic) => {
                    connection = ReadLane::new(root.clone(), lane.io());
                    Landing::Panicked { read, message: panic_message(&*panic) }
                }
            };
            let took = now.elapsed();
            tally.note(read, took);
            if took >= LONG_READ {
                report::log_quiet(lane.name(), &format!("{} took {} ms", read.name(), took.as_millis()));
            }
            due.last = Some(Instant::now());
            due.failed = landing.failed();
            due.poked = false;
            report::post(lane.name(), move || super::landed(landing, generation));
        }
    }
}

/// The reads of `plan` that `poke` asks for are due now.
fn mark(plan: &mut [Due], poke: Poke) {
    for due in plan {
        due.poked |= poke.asks(due.cadence.read);
    }
}

/// `read` on `connection`; the task list's and its switches' of the account `listed` names, which
/// it always does when either read is due (`Due::at`): an empty alias would be refused as no alias
/// at all.
fn read_on(read: Read, connection: &mut ReadLane, listed: Option<&str>) -> Landing {
    match read {
        Read::Status => Landing::Live(connection.read_live()),
        Read::Explain => Landing::Explain(connection.read_explain()),
        Read::Config => Landing::Config(connection.read_config()),
        Read::Start => Landing::Start(connection.read_start()),
        Read::Tasks => {
            let alias = listed.unwrap_or_default();
            Landing::Tasks { alias: alias.to_owned(), read: connection.read_tasks(alias) }
        }
        Read::Packs => {
            let alias = listed.unwrap_or_default();
            Landing::Packs { alias: alias.to_owned(), read: connection.read_packs(alias) }
        }
    }
}

/// A panic's message when it gave one as text.
pub fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "-".to_owned())
}
