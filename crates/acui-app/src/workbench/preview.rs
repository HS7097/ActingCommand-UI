// SPDX-License-Identifier: GPL-3.0-only
//! The workbench's picture thread (UI-3c), `acui-preview`, and the one table of how often it
//! reads: the frames the right column's live view shows come from here. How much one page
//! or one lookup may read is `acui-source`'s (`PREVIEW_IO`, `POLL_ROWS`, `MARK_ROWS`,
//! `LOOKUP_WINDOWS`, `LOOKUP_PAGES`), the read side of the same table; how often is this
//! file's, and only here. The Runtime states no frequency for any of it: the numbers are
//! estimates.
//!
//! The thread follows one account at a time, the one the event loop names, and nothing while
//! it names none (`sync`): not on the settings page, not minimized, not while no Runtime
//! answers, not while the account's gate is shut (`Model::preview_gate`). It looks the
//! account's newest frame up when it starts on it, polls the account's records after that at
//! its pace, reads a frame newer than the one shown at most every `MIN_FRAME_GAP`, the newest
//! only, decodes it, and hands it to the event loop with its pixels ready, so the event loop
//! does no I/O and no decoding. It never writes, and never subscribes. A frame it could not
//! read is not read again: a failed read takes a record in the Runtime's ledger.
//!
//! What a task recognized and meant on a frame comes with the polls, after the frame's own
//! record; for a task's frame a lookup found, or an order carried, one page more reads what
//! came of it before the position the polls go on after (`mark_page`). The thread keeps it
//! only of the frame shown and the one waiting to be read, hands it over with the frame, and
//! what comes of the frame shown later, as it comes.
//!
//! Every order to follow another account, or none, states a generation (`Pictures::sync`);
//! what the thread hands over states the one it was for, and one of another generation is
//! let go (`landed`), as is one of another owner epoch than the one online. A material read
//! asks between its ranges whether its generation is still the one, so stopping is quick.
//! No write concerns the picture (`super::wrote`): a frame is history.
//!
//! What a limit on its reads is checked by is in the technical log: a read that took long,
//! as it happens, and every read's count at the end of the run (`PreviewTally`).

use std::collections::VecDeque;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use acui_source::workbench::preview::{
    CaptureFailure, FrameKey, FrameMarks, FrameOrigin, FrameRead, FrameSeen, Latest, LookupStopped,
    Overlay, Poll, PreviewLane, LOOKUP_PAGES,
};
use acui_source::workbench::{InstanceKey, Unreached};
use acui_source::MAX_FRAME_BYTES;
use slint::{Rgba8Pixel, SharedPixelBuffer};

use super::paint;
use super::picture::{Mismatch, Unread};
use super::read::panic_message;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::Ui;

/// The thread's name, also what the picture's lines in the technical log are under.
pub const THREAD: &str = "acui-preview";

/// How often the account's records are polled: while it runs a task or its recovery ladder
/// climbs, or the last poll found a new frame; and otherwise, to catch an observation made
/// elsewhere, or a task in the seconds before `Status` says it runs.
const POLL_FAST: Duration = Duration::from_secs(2);
const POLL_IDLE: Duration = Duration::from_secs(10);
/// How long after a round that failed the next one is, by how many failed in a row; the
/// pace again once one goes well.
const POLL_FAILED: [Duration; 3] =
    [Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(30)];
/// The least time between two frames read, unless the settings file says otherwise
/// (`preview_frame_gap_seconds`, `frame_gap`), in whole seconds from `FRAME_GAP_LEAST` to
/// `FRAME_GAP_MOST`. Each range of a frame's material asks the ledger's writer twice, so this
/// is kept slow until a joint test says otherwise. Only while the window shows no frame of
/// the account followed does one a lookup found, or the order carried, read at once: following
/// the same account again, after the settings page, a minimized window or a Runtime that did
/// not answer for a moment, waits the gap like any other.
pub const MIN_FRAME_GAP: Duration = Duration::from_secs(10);
const FRAME_GAP_LEAST: u64 = 2;
const FRAME_GAP_MOST: u64 = 60;
/// An account looked up this recently, in the same owner epoch, is not looked up again when
/// it is followed again: it polls on from where it was.
pub const LOOKUP_AGAIN: Duration = Duration::from_secs(10);
/// How long the thread waits after an order to follow before it reads, for another: going
/// down the cards from the keyboard reads only the last.
const SELECT_SETTLE: Duration = Duration::from_millis(300);
/// A frame captured longer ago than this is an old picture.
pub const STALE_AFTER: Duration = Duration::from_secs(30);
/// How many frames that could not be read the thread keeps, so as not to read them again.
const FAILED_FRAMES: usize = 64;
/// The most pixels a decoded frame may have.
const MAX_DECODED_PIXELS: u64 = 16_777_216;
/// A read longer than this leaves a line in the technical log.
const LONG_READ: Duration = Duration::from_secs(2);
/// How long the thread sleeps with nothing to follow: an order wakes it.
const NOTHING_DUE: Duration = Duration::from_secs(3600);

/// The least time between two frames read that `value`, as the settings file has it, says;
/// `None` when it is not a whole number of seconds in range.
pub fn frame_gap(value: &str) -> Option<Duration> {
    let seconds: u64 = value.parse().ok()?;
    (FRAME_GAP_LEAST..=FRAME_GAP_MOST).contains(&seconds).then(|| Duration::from_secs(seconds))
}

/// The account the thread is to follow: who it is (`alias`, `instance_id`, `epoch`), and
/// how.
#[derive(Debug, Clone)]
pub struct Target {
    pub alias: String,
    pub key: InstanceKey,
    pub instance_id: String,
    pub epoch: String,
    /// The ledger's head as the last `Status` stated it: where a lookup starts down from.
    pub head: Option<u64>,
    /// It runs a task, or its recovery ladder climbs: poll fast.
    pub fast: bool,
}

impl Target {
    /// Whether `other` is the same account to follow: its head and its pace do not count.
    pub fn same(&self, other: &Self) -> bool {
        self.alias == other.alias
            && self.instance_id == other.instance_id
            && self.epoch == other.epoch
    }
}

/// Where following an account starts.
#[derive(Debug, Clone)]
pub enum Start {
    /// Look its newest frame up, down from the head.
    Lookup,
    /// It was looked up a moment ago in this owner epoch: poll on after `after`, and read
    /// `newest`, the newest frame known of it, if it is newer than the one shown.
    Known { after: u64, newest: Option<FrameSeen> },
}

/// What the event loop tells the thread.
pub enum Order {
    /// Follow `target` from now on, under `generation`. `shown`: the frame of it the window
    /// shows, by its position, which a frame read must be newer than, and its key, which what
    /// is recognized on it later goes by.
    Follow { generation: u64, target: Target, start: Start, shown: Option<(u64, FrameKey)> },
    /// Follow nothing.
    Stop,
    /// The account followed runs a task or climbs its ladder, or no longer.
    Pace(bool),
}

/// What the thread hands to the event loop, under the generation it was for.
pub enum Seen {
    /// A lookup's newest frame, if it found one; the next poll asks after `after`.
    Looked { alias: String, epoch: String, found: Option<FrameSeen>, after: u64 },
    /// A poll that found something: a new frame, a capture that failed, or that following
    /// goes on after rounds that failed.
    Polled {
        alias: String,
        epoch: String,
        after: u64,
        newest: Option<FrameSeen>,
        failure: Option<CaptureFailure>,
    },
    /// A frame read and decoded, and what its task recognized and meant on it, as far as that
    /// is known.
    Frame {
        alias: String,
        epoch: String,
        frame: FrameSeen,
        pixels: SharedPixelBuffer<Rgba8Pixel>,
        overlay: Option<Overlay>,
    },
    /// What a task recognized and meant on the frame `key`, the one shown, known now.
    Marks { alias: String, epoch: String, key: FrameKey, overlay: Option<Overlay> },
    /// A frame not shown, and why; it is not read again.
    Unreadable { alias: String, epoch: String, sequence: u64, why: Unread },
    /// A round did not reach the Runtime; it is tried again a while later.
    Interrupted { failure: Unreached },
    /// A round panicked: the panic's message, for the log. The connection goes with it.
    Panicked { message: String },
}

/// What one round of the thread asks for, for the log at the end of the run.
#[derive(Default)]
pub struct PreviewTally {
    polls: AtomicU64,
    polls_fast: AtomicU64,
    poll_longest_ms: AtomicU64,
    lookups: AtomicU64,
    lookup_pages: AtomicU64,
    lookups_capped: AtomicU64,
    mark_pages: AtomicU64,
    reads: AtomicU64,
    read_bytes: AtomicU64,
    reads_missing: AtomicU64,
    reads_failed: AtomicU64,
    read_longest_ms: AtomicU64,
    decodes: AtomicU64,
    decode_longest_ms: AtomicU64,
}

impl PreviewTally {
    fn poll(&self, fast: bool, took: Duration) {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if fast {
            self.polls_fast.fetch_add(1, Ordering::SeqCst);
        }
        self.poll_longest_ms.fetch_max(millis(took), Ordering::SeqCst);
    }

    /// A lookup of `pages` pages, its head's page included when it read one.
    fn lookup(&self, pages: u32, capped: bool) {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.lookup_pages.fetch_add(u64::from(pages), Ordering::SeqCst);
        if capped {
            self.lookups_capped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn head_page(&self) {
        self.lookup_pages.fetch_add(1, Ordering::SeqCst);
    }

    /// A page of a frame's marks.
    fn mark_page(&self) {
        self.mark_pages.fetch_add(1, Ordering::SeqCst);
    }

    /// A material read that went to the Runtime, and what it came to.
    fn read(&self, took: Duration, bytes: Option<usize>, missing: bool, failed: bool) {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.read_longest_ms.fetch_max(millis(took), Ordering::SeqCst);
        if let Some(bytes) = bytes {
            let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
            self.read_bytes.fetch_add(bytes, Ordering::SeqCst);
        }
        if missing {
            self.reads_missing.fetch_add(1, Ordering::SeqCst);
        }
        if failed {
            self.reads_failed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn decode(&self, took: Duration) {
        self.decodes.fetch_add(1, Ordering::SeqCst);
        self.decode_longest_ms.fetch_max(millis(took), Ordering::SeqCst);
    }

    /// The line the technical log gets at the end of the run.
    pub fn line(&self) -> String {
        let count = |counter: &AtomicU64| counter.load(Ordering::SeqCst);
        format!(
            "polls {} (fast {}, longest {} ms); lookups {} (pages {}, capped {}); \
             mark pages {}; frame reads {} (bytes {}, missing {}, failed {}, longest {} ms); \
             decodes {} (longest {} ms)",
            count(&self.polls),
            count(&self.polls_fast),
            count(&self.poll_longest_ms),
            count(&self.lookups),
            count(&self.lookup_pages),
            count(&self.lookups_capped),
            count(&self.mark_pages),
            count(&self.reads),
            count(&self.read_bytes),
            count(&self.reads_missing),
            count(&self.reads_failed),
            count(&self.read_longest_ms),
            count(&self.decodes),
            count(&self.decode_longest_ms),
        )
    }
}

fn millis(took: Duration) -> u64 {
    u64::try_from(took.as_millis()).unwrap_or(u64::MAX)
}

/// The picture thread: the sender is what keeps it reading. `generation` is the one
/// `Pictures` counts; `gap`, the least time between two frames read.
pub fn spawn(
    root: PathBuf,
    generation: Arc<AtomicU64>,
    tally: Arc<PreviewTally>,
    gap: Duration,
) -> io::Result<Sender<Order>> {
    let (sender, orders) = mpsc::channel();
    spawn_worker(THREAD, move || run(&root, &orders, &generation, &tally, gap))?;
    Ok(sender)
}

/// What the thread keeps from one account to the next: when it last read a frame, the
/// frames it does not read again, and the class of the last one that failed, for the log.
struct Reads {
    gap: Duration,
    last: Option<Instant>,
    failed: VecDeque<u64>,
    last_unread: Option<Unread>,
}

/// A frame read and decoded that the event loop has not taken yet: only the newest waits.
type Waiting = Arc<Mutex<Option<(u64, Seen)>>>;

fn run(
    root: &Path,
    orders: &Receiver<Order>,
    generation: &AtomicU64,
    tally: &PreviewTally,
    gap: Duration,
) {
    let mut lane = PreviewLane::new(root);
    let mut line: Option<Line> = None;
    let mut reads = Reads { gap, last: None, failed: VecDeque::new(), last_unread: None };
    let waiting: Waiting = Arc::new(Mutex::new(None));
    loop {
        let now = Instant::now();
        let wait = line
            .as_ref()
            .map_or(NOTHING_DUE, |line| line.due(&reads, now).saturating_duration_since(now));
        let mut followed = match orders.recv_timeout(wait) {
            Ok(order) => take(&mut line, order),
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Orders queued behind that one count in the same round.
        loop {
            match orders.try_recv() {
                Ok(order) => followed |= take(&mut line, order),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // A new account to follow waits until the orders stop for a moment.
        while followed {
            match orders.recv_timeout(SELECT_SETTLE) {
                Ok(order) => {
                    take(&mut line, order);
                }
                Err(RecvTimeoutError::Timeout) => followed = false,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let Some(current) = line.as_mut() else {
            continue;
        };
        if current.next_poll <= Instant::now() {
            let round = catch_unwind(AssertUnwindSafe(|| current.round(&mut lane, tally)));
            match round {
                Ok(Ok(())) => current.went_well(),
                Ok(Err(halt)) => current.failed(&mut lane, root, halt),
                Err(panic) => {
                    current.failed(&mut lane, root, Halt::Panicked(panic_message(&*panic)))
                }
            }
        }
        let now = Instant::now();
        if current.read_at(&reads, now).is_some_and(|at| at <= now) {
            let read = catch_unwind(AssertUnwindSafe(|| {
                current.read(&mut lane, &mut reads, generation, tally, &waiting)
            }));
            match read {
                Ok(Ok(())) => {}
                Ok(Err(halt)) => current.failed(&mut lane, root, halt),
                Err(panic) => {
                    current.failed(&mut lane, root, Halt::Panicked(panic_message(&*panic)))
                }
            }
        }
    }
}

/// `order` taken: whether it is one to follow an account.
fn take(line: &mut Option<Line>, order: Order) -> bool {
    match order {
        Order::Follow { generation, target, start, shown } => {
            *line = Some(Line::new(generation, target, start, shown));
            true
        }
        Order::Stop => {
            *line = None;
            false
        }
        Order::Pace(fast) => {
            if let Some(line) = line {
                // Running from now on: the next poll comes at the fast pace at the latest.
                if fast && !line.target.fast {
                    line.next_poll = line.next_poll.min(Instant::now() + POLL_FAST);
                }
                line.target.fast = fast;
            }
            false
        }
    }
}

/// Why a round stopped short.
enum Halt {
    Unreached(Unreached),
    /// The connection reaches another owner epoch than the one to follow, as the wire spells
    /// it.
    Epoch(String),
    /// The round panicked: the panic's message.
    Panicked(String),
}

impl From<Unreached> for Halt {
    fn from(unreached: Unreached) -> Self {
        Self::Unreached(unreached)
    }
}

/// Following one account.
struct Line {
    generation: u64,
    target: Target,
    /// The position the next poll asks after; none until a lookup put it at the head.
    after: Option<u64>,
    /// The newest frame known of the account: shown, waiting, or neither.
    newest: Option<u64>,
    /// The newest frame shown or read since: a frame read must be newer.
    shown: Option<u64>,
    /// The newest frame not read yet, and whether it reads now rather than after the gap: one
    /// a lookup found, or the order carried, does while no frame of the account is shown.
    waiting: Option<FrameSeen>,
    read_now: bool,
    next_poll: Instant,
    /// Rounds that failed in a row: how long until the next.
    failures: usize,
    /// The last poll found a frame newer than any known: poll fast.
    saw_new: bool,
    /// A round failed since the last that went well: the next that does says so.
    troubled: bool,
    /// What tasks recognized and meant, of the frame shown and the one waiting only.
    marks: FrameMarks,
    /// The key of the frame shown, and what the event loop was last told was recognized and
    /// meant on it.
    shown_key: Option<FrameKey>,
    told: Option<Overlay>,
    /// A task's frame the order carried: what came of it before `after` is still to be read.
    marks_owed: Option<FrameSeen>,
}

impl Line {
    fn new(generation: u64, target: Target, start: Start, shown: Option<(u64, FrameKey)>) -> Self {
        let (after, known) = match start {
            Start::Lookup => (None, None),
            Start::Known { after, newest } => (Some(after), newest),
        };
        let (shown, shown_key) = shown.unzip();
        let mut line = Self {
            generation,
            target,
            after,
            newest: shown,
            shown,
            waiting: None,
            read_now: false,
            next_poll: Instant::now(),
            failures: 0,
            saw_new: false,
            troubled: false,
            marks: FrameMarks::default(),
            shown_key,
            told: None,
            marks_owed: None,
        };
        if let Some(frame) = known {
            let owed = is_task(&frame).then(|| frame.clone());
            line.offer(frame, true);
            if line.waiting.is_some() {
                line.marks_owed = owed;
            }
        }
        line
    }

    fn fast(&self) -> bool {
        self.target.fast || self.saw_new
    }

    /// When the waiting frame is read: at once when it reads now or none was read before,
    /// else the gap after the last; never before the next round while rounds fail.
    fn read_at(&self, reads: &Reads, now: Instant) -> Option<Instant> {
        self.waiting.as_ref()?;
        let at = match reads.last {
            Some(last) if !self.read_now => last + reads.gap,
            Some(_) | None => now,
        };
        Some(if self.failures > 0 { at.max(self.next_poll) } else { at })
    }

    /// When the thread next has something to do for this account.
    fn due(&self, reads: &Reads, now: Instant) -> Instant {
        self.read_at(reads, now).map_or(self.next_poll, |at| at.min(self.next_poll))
    }

    /// `frame`, newer than the one shown and the one waiting, waits to be read: at once while
    /// no frame of the account is shown, when a lookup found it or the order carried it
    /// (`found`), or it takes the place of one that was to; else the gap after the last read.
    fn offer(&mut self, frame: FrameSeen, found: bool) {
        let sequence = frame.sequence;
        if self.newest.map_or(true, |newest| sequence > newest) {
            self.newest = Some(sequence);
        }
        let newer = self.shown.map_or(true, |shown| sequence > shown)
            && self.waiting.as_ref().map_or(true, |waiting| sequence > waiting.sequence);
        if newer {
            self.waiting = Some(frame);
            self.read_now = self.shown.is_none() && (found || self.read_now);
        }
    }

    fn same_epoch(&self, epoch: &str) -> Result<(), Halt> {
        if epoch == self.target.epoch {
            Ok(())
        } else {
            Err(Halt::Epoch(epoch.to_owned()))
        }
    }

    /// A lookup until one went well, then a poll; before the first poll, what came of a
    /// task's frame the order carried.
    fn round(&mut self, lane: &mut PreviewLane, tally: &PreviewTally) -> Result<(), Halt> {
        let Some(after) = self.after else {
            return self.look_up(lane, tally);
        };
        if let Some(frame) = self.marks_owed.clone() {
            self.mark_page(lane, tally, &frame, after)?;
            self.marks_owed = None;
            self.tell_marks();
        }
        self.poll(lane, tally, after)
    }

    fn went_well(&mut self) {
        self.failures = 0;
        self.troubled = false;
        let pace = if self.fast() { POLL_FAST } else { POLL_IDLE };
        self.next_poll = Instant::now() + pace;
    }

    /// A round that stopped short: the event loop hears why, and the next round is a while
    /// later. A connection that panicked, or reaches another owner epoch, is let go; one of
    /// another epoch says nothing to the event loop, whose next `Status` either names that
    /// epoch, with an order to follow it, or no Runtime.
    fn failed(&mut self, lane: &mut PreviewLane, root: &Path, halt: Halt) {
        let seen = match halt {
            Halt::Unreached(failure) => Some(Seen::Interrupted { failure }),
            Halt::Epoch(epoch) => {
                *lane = PreviewLane::new(root);
                let line = format!(
                    "{}: the connection reaches owner epoch {epoch}, not {}; connecting again",
                    self.target.alias, self.target.epoch
                );
                report::log_quiet(THREAD, &line);
                None
            }
            Halt::Panicked(message) => {
                *lane = PreviewLane::new(root);
                Some(Seen::Panicked { message })
            }
        };
        self.failures += 1;
        self.troubled = true;
        self.saw_new = false;
        let wait = POLL_FAILED[(self.failures - 1).min(POLL_FAILED.len() - 1)];
        self.next_poll = Instant::now() + wait;
        if let Some(seen) = seen {
            post(self.generation, seen);
        }
    }

    /// The account's newest frame, down from the head `Status` stated, or from the one a
    /// page states when it did not; polls go on after that head.
    fn look_up(&mut self, lane: &mut PreviewLane, tally: &PreviewTally) -> Result<(), Halt> {
        let head = match self.target.head {
            Some(head) => head,
            None => {
                // Counted and timed whether it came back or not.
                let started = Instant::now();
                let reached = lane.head();
                tally.head_page();
                slow(&format!("the ledger's head for {}", self.target.alias), started.elapsed());
                let reach = reached?;
                self.same_epoch(&reach.epoch)?;
                reach.value
            }
        };
        let found = self.latest(lane, tally, head)?;
        if let Some(frame) = &found {
            self.marks_of_found(lane, tally, frame, head)?;
        }
        self.after = Some(head);
        if let Some(frame) = &found {
            self.offer(frame.clone(), true);
        }
        let seen = Seen::Looked {
            alias: self.target.alias.clone(),
            epoch: self.target.epoch.clone(),
            found,
            after: head,
        };
        post(self.generation, seen);
        self.tell_marks();
        Ok(())
    }

    /// What came of `frame`, which a lookup found at or below `head`, before `head`: one page,
    /// for a task's frame that is shown, or is to be read, which one too old, cleaned up or too
    /// large to read is not.
    fn marks_of_found(
        &mut self,
        lane: &mut PreviewLane,
        tally: &PreviewTally,
        frame: &FrameSeen,
        head: u64,
    ) -> Result<(), Halt> {
        let readable = !frame.evicted && frame.byte_count <= MAX_FRAME_BYTES;
        let current = self.shown.map_or(true, |shown| frame.sequence >= shown);
        if is_task(frame) && readable && current {
            self.mark_page(lane, tally, frame, head)?;
        }
        Ok(())
    }

    /// What a task recognized on `frame` and meant on it, recorded up to `at`: one page,
    /// counted and timed whether it came back or not. None is asked of a frame that names no
    /// frame id, or of an observation's.
    fn mark_page(
        &mut self,
        lane: &mut PreviewLane,
        tally: &PreviewTally,
        frame: &FrameSeen,
        at: u64,
    ) -> Result<(), Halt> {
        if !is_task(frame) {
            return Ok(());
        }
        let started = Instant::now();
        let asked = lane.marks(self.target.key, frame, at);
        let took = started.elapsed();
        if matches!(asked, Ok(None)) {
            return Ok(());
        }
        tally.mark_page();
        slow(&format!("the marks of frame {} of {}", frame.sequence, self.target.alias), took);
        let Some(reach) = asked? else {
            return Ok(());
        };
        self.same_epoch(&reach.epoch)?;
        if reach.value.cut() {
            let line = format!(
                "the marks of frame {} of {} were not whole at {at}; some may not be drawn",
                frame.sequence, self.target.alias
            );
            report::log_quiet(THREAD, &line);
        }
        self.marks.absorb(reach.value);
        Ok(())
    }

    /// The event loop hears what is now known to be recognized and meant on the frame shown,
    /// when that changed; and the thread lets go of what it keeps of frames other than that
    /// one and the one waiting.
    fn tell_marks(&mut self) {
        if let Some(key) = &self.shown_key {
            let overlay = self.marks.overlay(key);
            if overlay != self.told {
                self.told = overlay.clone();
                let seen = Seen::Marks {
                    alias: self.target.alias.clone(),
                    epoch: self.target.epoch.clone(),
                    key: key.clone(),
                    overlay,
                };
                post(self.generation, seen);
            }
        }
        let waiting = self.waiting.as_ref().map(|frame| &frame.key);
        let keys: Vec<&FrameKey> = self.shown_key.iter().chain(waiting).collect();
        self.marks.keep(&keys);
    }

    /// The newest frame at or below `head`. One that ran out of pages says so in the log,
    /// and gives the newest frame it read, if any. One that stopped short counts the pages it
    /// asked, and its time, as one that did not.
    fn latest(
        &mut self,
        lane: &mut PreviewLane,
        tally: &PreviewTally,
        head: u64,
    ) -> Result<Option<FrameSeen>, Halt> {
        let started = Instant::now();
        let looked = lane.latest(self.target.key, head);
        let took = started.elapsed();
        let alias = &self.target.alias;
        let reach = match looked {
            Ok(reach) => reach,
            Err(LookupStopped { failure, pages }) => {
                tally.lookup(pages, false);
                slow(&format!("a lookup for {alias} ({pages} pages, stopped short)"), took);
                return Err(Halt::Unreached(failure));
            }
        };
        let (found, capped) = match reach.value {
            Latest::Found { frame, capped } => (Some(frame), capped),
            Latest::NoneWithin { .. } => (None, reach.pages >= LOOKUP_PAGES),
        };
        tally.lookup(reach.pages, capped);
        slow(&format!("a lookup for {alias} ({} pages)", reach.pages), took);
        self.same_epoch(&reach.epoch)?;
        if capped {
            let then =
                if found.is_some() { "the newest frame it read shows" } else { "no frame read" };
            let line = format!(
                "a lookup for {alias} stopped after {} pages below {head}: {then}",
                reach.pages
            );
            report::log_quiet(THREAD, &line);
        }
        Ok(found)
    }

    /// The account's frames and failed captures after `after`. A page that is not whole is
    /// let go, and the newest frame looked up at the position it states instead.
    fn poll(
        &mut self,
        lane: &mut PreviewLane,
        tally: &PreviewTally,
        after: u64,
    ) -> Result<(), Halt> {
        // Counted and timed whether it came back or not.
        let started = Instant::now();
        let polled = lane.poll(self.target.key, after);
        let took = started.elapsed();
        tally.poll(self.fast(), took);
        slow(&format!("a poll for {}", self.target.alias), took);
        let reach = polled?;
        self.same_epoch(&reach.epoch)?;
        let Poll { snapshot, complete, frames, failures, marks } = reach.value;
        let (newest, failure, marks) = if complete {
            (frames.into_iter().last(), failures.into_iter().last(), Some(marks))
        } else {
            let line = format!(
                "a poll for {} after {after} was not whole at {snapshot}; looking up its newest frame",
                self.target.alias
            );
            report::log_quiet(THREAD, &line);
            let found = self.latest(lane, tally, snapshot)?;
            if let Some(frame) = &found {
                self.marks_of_found(lane, tally, frame, snapshot)?;
            }
            (found, None, None)
        };
        self.after = Some(snapshot);
        let known = self.newest;
        let fresh = newest.filter(|frame| known.map_or(true, |known| frame.sequence > known));
        self.saw_new = fresh.is_some();
        if let Some(frame) = &fresh {
            self.offer(frame.clone(), false);
        }
        if let Some(marks) = marks {
            self.marks.absorb(marks);
        }
        self.tell_marks();
        if fresh.is_some() || failure.is_some() || self.troubled {
            let seen = Seen::Polled {
                alias: self.target.alias.clone(),
                epoch: self.target.epoch.clone(),
                after: snapshot,
                newest: fresh,
                failure,
            };
            post(self.generation, seen);
        }
        Ok(())
    }

    /// The waiting frame, read whole and decoded; one the thread could not read before is
    /// not asked for again. The read stops between its ranges once the event loop orders
    /// otherwise. A read that did not reach the Runtime leaves the frame waiting.
    fn read(
        &mut self,
        lane: &mut PreviewLane,
        reads: &mut Reads,
        generation: &AtomicU64,
        tally: &PreviewTally,
        waiting: &Waiting,
    ) -> Result<(), Halt> {
        let Some(frame) = self.waiting.take() else {
            return Ok(());
        };
        self.read_now = false;
        if reads.failed.contains(&frame.sequence) {
            return Ok(());
        }
        let ours = self.generation;
        let wanted = || generation.load(Ordering::SeqCst) == ours;
        // A frame an eviction proof names, or one over the cap, is not asked for.
        let sent = !frame.evicted && frame.byte_count <= MAX_FRAME_BYTES;
        let started = Instant::now();
        let reach = match lane.read(&frame, &wanted) {
            Ok(reach) => reach,
            Err(unreached) => {
                self.waiting = Some(frame);
                return Err(Halt::Unreached(unreached));
            }
        };
        let took = started.elapsed();
        if sent {
            reads.last = Some(Instant::now());
        }
        slow(&format!("frame {} of {} read", frame.sequence, self.target.alias), took);
        if let Err(halt) = self.same_epoch(&reach.epoch) {
            self.waiting = Some(frame);
            return Err(halt);
        }
        match reach.value {
            FrameRead::Bytes(bytes) => {
                tally.read(took, Some(bytes.len()), false, false);
                let decoding = Instant::now();
                let decoded = decode(&bytes);
                tally.decode(decoding.elapsed());
                match decoded {
                    Ok(pixels) => {
                        self.shown = Some(frame.sequence);
                        reads.last_unread = None;
                        let overlay = self.marks.overlay(&frame.key);
                        self.shown_key = Some(frame.key.clone());
                        self.told = overlay.clone();
                        let seen = Seen::Frame {
                            alias: self.target.alias.clone(),
                            epoch: self.target.epoch.clone(),
                            frame,
                            pixels,
                            overlay,
                        };
                        hand_over(waiting, ours, seen);
                        // Nothing of the frames before it is kept.
                        self.tell_marks();
                    }
                    Err((why, detail)) => self.unreadable(reads, &frame, why, &detail),
                }
            }
            // The event loop ordered otherwise meanwhile: nothing to show.
            FrameRead::Superseded => {
                if sent {
                    tally.read(took, None, false, false);
                }
            }
            FrameRead::Failed { why, detail } => {
                let why = Unread::of(why);
                if sent {
                    tally.read(took, None, why == Unread::Missing, why != Unread::Missing);
                }
                self.unreadable(reads, &frame, why, &detail);
            }
        }
        Ok(())
    }

    /// `frame` is not shown, and not read again. The log hears a class of failure as it
    /// changes, and the end-of-run count every one.
    fn unreadable(&self, reads: &mut Reads, frame: &FrameSeen, why: Unread, detail: &str) {
        if reads.failed.len() >= FAILED_FRAMES {
            reads.failed.pop_front();
        }
        reads.failed.push_back(frame.sequence);
        if reads.last_unread != Some(why) {
            reads.last_unread = Some(why);
            let line = format!(
                "frame {} of {} not shown ({}): {detail}",
                frame.sequence,
                self.target.alias,
                why.name()
            );
            if why.is_problem() {
                report::log(THREAD, &line);
            } else {
                report::log_quiet(THREAD, &line);
            }
        }
        let seen = Seen::Unreadable {
            alias: self.target.alias.clone(),
            epoch: self.target.epoch.clone(),
            sequence: frame.sequence,
            why,
        };
        post(self.generation, seen);
    }
}

/// Whether a task took `frame`: only on such a frame is anything recognized or meant.
fn is_task(frame: &FrameSeen) -> bool {
    matches!(frame.origin, FrameOrigin::Task { .. })
}

/// A read that took long, in the log as it happens.
fn slow(what: &str, took: Duration) {
    if took >= LONG_READ {
        report::log_quiet(THREAD, &format!("{what} took {} ms", took.as_millis()));
    }
}

/// A frame's PNG as pixels the window takes as they are: decoded here, off the event loop.
fn decode(bytes: &[u8]) -> Result<SharedPixelBuffer<Rgba8Pixel>, (Unread, String)> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| (Unread::Undecodable, error.to_string()))?;
    let (width, height) = (image.width(), image.height());
    if u64::from(width) * u64::from(height) > MAX_DECODED_PIXELS {
        let detail = format!("{width} x {height} pixels, over {MAX_DECODED_PIXELS}");
        return Err((Unread::TooLarge, detail));
    }
    let rgba = image.into_rgba8().into_raw();
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    pixels.make_mut_bytes().copy_from_slice(&rgba);
    Ok(pixels)
}

fn post(generation: u64, seen: Seen) {
    report::post(THREAD, move || landed(generation, seen));
}

/// A frame for the event loop, which takes only the newest of those it has not taken yet.
fn hand_over(waiting: &Waiting, generation: u64, seen: Seen) {
    let first = waiting
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace((generation, seen))
        .is_none();
    if first {
        let waiting = Arc::clone(waiting);
        report::post(THREAD, move || {
            let held = waiting.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some((generation, seen)) = held {
                landed(generation, seen);
            }
        });
    }
}

/// Starts or stops the thread on what the window shows now: the account it follows, if any
/// (`Model::preview_target`). The only way an order goes to the thread. Painting is the
/// caller's.
pub fn sync(ui: &Ui) {
    let order = ui.model.borrow_mut().preview_sync(Instant::now());
    let Some(order) = order else {
        return;
    };
    let sent =
        ui.reading.borrow_mut().as_mut().map_or(Ok(()), |reading| reading.send_preview(order));
    if let Err(why) = sent {
        thread_failed(ui, why);
    }
}

/// The picture thread did not start, or is gone: said in red where the picture is, and in
/// the log. Nothing starts it again this run.
pub fn thread_failed(ui: &Ui, why: String) {
    report::log(THREAD, &format!("no picture thread: {why}"));
    ui.model.borrow_mut().pictures_mut().thread_lost(why);
    paint::picture(ui);
    ui.paint_log_notes();
}

/// UI-3d, before it asks for `alias`'s device for a person: the picture stops reading it
/// from then on, and stays on the last frame.
#[allow(dead_code)] // UI-3d's.
pub fn halt(ui: &Ui, alias: &str) {
    ui.model.borrow_mut().pictures_mut().set_halted(alias, true);
    sync(ui);
    paint::picture(ui);
}

/// UI-3d, once that ask failed or the device is handed back: the picture reads it again.
#[allow(dead_code)] // UI-3d's.
pub fn release(ui: &Ui, alias: &str) {
    ui.model.borrow_mut().pictures_mut().set_halted(alias, false);
    sync(ui);
    paint::picture(ui);
}

/// What the thread handed over, on the event loop. One of another generation is let go: the
/// window shows something else now. Not a panic, which is no finding about an account. One of
/// another owner epoch than the one online is let go too: the next `Status` sees it and
/// follows it.
fn landed(generation: u64, seen: Seen) {
    let Some(ui) = crate::ui::current() else {
        return;
    };
    let current = ui.model.borrow().pictures().generation_now();
    if generation != current && !matches!(seen, Seen::Panicked { .. }) {
        return;
    }
    let online = ui.model.borrow().online_epoch().map(str::to_owned);
    let of_now = |epoch: &str| online.as_deref() == Some(epoch);
    let mut pixels = None;
    match seen {
        Seen::Looked { alias, epoch, found, after } if of_now(epoch.as_str()) => {
            let resumed =
                ui.model.borrow_mut().pictures_mut().looked(&alias, found, after, Instant::now());
            if resumed {
                report::log_quiet(THREAD, &format!("following {alias} again"));
            }
        }
        Seen::Polled { alias, epoch, after, newest, failure } if of_now(epoch.as_str()) => {
            let (resumed, failure) =
                ui.model.borrow_mut().pictures_mut().polled(&alias, after, newest, failure);
            if resumed {
                report::log_quiet(THREAD, &format!("following {alias} again"));
            }
            // The Runtime recorded the failure itself.
            if let Some(failure) = failure {
                let code = failure.code.as_deref().unwrap_or("no outcome code");
                let line = format!("a capture of {alias} failed at {}: {code}", failure.sequence);
                report::log_quiet(THREAD, &line);
            }
        }
        Seen::Frame { alias, epoch, frame, pixels: frame_pixels, overlay }
            if of_now(epoch.as_str()) =>
        {
            let size = (frame_pixels.width(), frame_pixels.height());
            let mut model = ui.model.borrow_mut();
            if model.pictures_mut().frame_shown(alias, epoch, frame, size, overlay) {
                pixels = Some(frame_pixels);
            }
        }
        Seen::Marks { alias, epoch, key, overlay } if of_now(epoch.as_str()) => {
            if !ui.model.borrow_mut().pictures_mut().marks_landed(&alias, &key, overlay) {
                return;
            }
        }
        Seen::Unreadable { alias, epoch, sequence, why } if of_now(epoch.as_str()) => {
            ui.model.borrow_mut().pictures_mut().unreadable(&alias, sequence, why);
        }
        Seen::Looked { .. }
        | Seen::Polled { .. }
        | Seen::Frame { .. }
        | Seen::Marks { .. }
        | Seen::Unreadable { .. } => {
            return;
        }
        Seen::Interrupted { failure } => {
            let line = format!("following interrupted: {}", super::failure_line(failure.failure()));
            let trouble = matches!(failure, Unreached::Trouble(_));
            if ui.model.borrow_mut().pictures_mut().interrupted(failure) {
                // The lamp says a Runtime that is not there, or does not answer.
                if trouble {
                    report::log(THREAD, &line);
                } else {
                    report::log_quiet(THREAD, &line);
                }
            }
        }
        Seen::Panicked { message } => {
            let line = format!("picture read panicked: {message}");
            if ui.model.borrow_mut().pictures_mut().panicked(message) {
                report::log(THREAD, &line);
            }
        }
    }
    // Records that state another size than the frame's: drawn nowhere, and said once a frame.
    let mismatch = ui.model.borrow_mut().pictures_mut().untold_mismatch();
    if let Some(Mismatch { alias, sequence, extent: (width, height), size }) = mismatch {
        let line = format!(
            "frame {sequence} of {alias} is {} x {} pixels, and its recognition states {width} x \
             {height}: nothing drawn on it",
            size.0, size.1
        );
        report::log_quiet(THREAD, &line);
    }
    if let Some(pixels) = pixels {
        paint::picture_pixels(&ui, pixels);
    }
    paint::picture(&ui);
    // The thread may have just made the log.
    ui.paint_log_notes();
}

/// The recognition's boxes and the inputs' points turned on or off on the window, for this
/// run: only what is drawn changes.
pub fn overlays_toggled(ui: &Ui, on: bool) {
    ui.model.borrow_mut().pictures_mut().set_overlays(on);
    paint::picture(ui);
}
