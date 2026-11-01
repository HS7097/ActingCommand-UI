// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's reading threads, and the one table of what they read and how
//! often. The Runtime states no frequency for any of this: the numbers are
//! estimates, and only here.
//!
//! `acui-live` reads `Status` alone, on a connection of its own: the client holds
//! its lock for a whole exchange, and `ScheduleExplanation` waits on the policy
//! lock, so the Runtime's lamp must not share a connection with it.
//!
//! A thread reads what is due, hands each result to the event loop, and sleeps
//! until the next is due or it is poked. It ends once the event loop lets its
//! sender go; nothing joins it, since it may be stuck in a read. A panic in a read
//! costs that read and the connection, is said in red, and the next round goes on.
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
use std::sync::Arc;
use std::time::{Duration, Instant};

use acui_source::workbench::{ExplainRead, LiveRead, ReadLane};
use acui_source::ClientFailure;

use crate::launcher::spawn_worker;
use crate::report;

/// `READS` has one row for each `Read`, in its order: `Tally` counts by it.
const READ_KINDS: usize = 2;

#[rustfmt::skip]
static READS: [Cadence; READ_KINDS] = [
    Cadence { read: Read::Status,  lane: Lane::Live, shown: secs(3),  minimized: Some(secs(30)), failed: secs(5) },
    Cadence { read: Read::Explain, lane: Lane::Slow, shown: secs(15), minimized: None,           failed: secs(30) },
];

/// A read longer than this leaves a line in the technical log.
const LONG_READ: Duration = secs(2);

/// A schedule explanation not read again for this long no longer counts as what
/// the schedule says now. Far above the slow thread's own times, so a read that
/// waits behind a slow one is not called unreadable.
pub const EXPLAIN_TOO_OLD: Duration = secs(60);

/// How long a thread with nothing due sleeps: a window coming back pokes it.
const NOTHING_DUE: Duration = secs(3600);

const fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// One read's place in the table.
struct Cadence {
    read: Read,
    lane: Lane,
    /// How often while the workbench is not minimized.
    shown: Duration,
    /// How often while it is; `None`: not at all, until a window coming back
    /// pokes the thread.
    minimized: Option<Duration>,
    /// How soon after a failed read, while the workbench is not minimized.
    failed: Duration,
}

/// One kind of read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    Status,
    Explain,
}

impl Read {
    fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Explain => "schedule explanation",
        }
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

/// Reads what the thread reads now, without waiting for its time.
pub struct Poke;

/// What a read came to, for the event loop.
pub enum Landing {
    Live(LiveRead),
    Explain(Result<ExplainRead, ClientFailure>),
    /// The read panicked: the panic's message, for the log.
    Panicked { lane: Lane, message: String },
}

impl Landing {
    fn failed(&self) -> bool {
        !matches!(self, Self::Live(LiveRead::Read(_)) | Self::Explain(Ok(_)))
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
    /// out a stop: the window coming back reads it anyway.
    fn at(&self, shown: bool, now: Instant) -> Option<Instant> {
        let every = match (shown, self.failed) {
            (true, false) => self.cadence.shown,
            (true, true) => self.cadence.failed,
            (false, _) => self.cadence.minimized?,
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
    tally: Arc<Tally>,
) -> io::Result<Sender<Poke>> {
    let (sender, commands) = mpsc::channel();
    spawn_worker(lane.name(), move || run(lane, root, &commands, &shown, &tally))?;
    Ok(sender)
}

fn run(lane: Lane, root: PathBuf, commands: &Receiver<Poke>, shown: &AtomicBool, tally: &Tally) {
    let mut connection = ReadLane::new(root.clone(), lane.io());
    let mut plan: Vec<Due> = READS
        .iter()
        .filter(|cadence| cadence.lane == lane)
        .map(|cadence| Due { cadence, last: None, failed: false, poked: false })
        .collect();
    loop {
        let now = Instant::now();
        let is_shown = shown.load(Ordering::SeqCst);
        let next = plan.iter().filter_map(|due| due.at(is_shown, now)).min();
        let wait = next.map_or(NOTHING_DUE, |at| at.saturating_duration_since(now));
        let mut poked = match commands.recv_timeout(wait) {
            Ok(Poke) => true,
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Pokes queued behind that one ask for the same reads.
        loop {
            match commands.try_recv() {
                Ok(Poke) => poked = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for due in &mut plan {
            due.poked |= poked;
            let now = Instant::now();
            let is_due = due.at(shown.load(Ordering::SeqCst), now).is_some_and(|at| at <= now);
            if !is_due {
                continue;
            }
            let read = due.cadence.read;
            let landing = match catch_unwind(AssertUnwindSafe(|| read_on(read, &mut connection))) {
                Ok(landing) => landing,
                Err(panic) => {
                    connection = ReadLane::new(root.clone(), lane.io());
                    Landing::Panicked { lane, message: panic_message(&*panic) }
                }
            };
            let took = now.elapsed();
            tally.note(read, took);
            if took >= LONG_READ {
                report::log(lane.name(), &format!("{} took {} ms", read.name(), took.as_millis()));
            }
            due.last = Some(Instant::now());
            due.failed = landing.failed();
            due.poked = false;
            report::post(lane.name(), move || super::landed(landing));
        }
    }
}

fn read_on(read: Read, connection: &mut ReadLane) -> Landing {
    match read {
        Read::Status => Landing::Live(connection.read_live()),
        Read::Explain => Landing::Explain(connection.read_explain()),
    }
}

/// A panic's message when it gave one as text.
fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "-".to_owned())
}
