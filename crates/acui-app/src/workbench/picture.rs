// SPDX-License-Identifier: GPL-3.0-only
//! The live view in the workbench's right column (UI-3c): which account's picture it
//! shows, what the picture thread (`preview`) was told to follow and found of each account,
//! and what the picture area says, as a pure function of that. No words and no Slint here:
//! `paint` makes both of a view. The pixels of the frame shown are only in the window.
//!
//! The account is the one a person picked on its card, or that `--account` named at the
//! start; until then, and once the one picked is no longer among the accounts a `Status`
//! lists, the first card's. Which one is picked is not kept past this run.
//!
//! What the thread hands over counts only under the generation of the order it was for:
//! `sync` counts one for every other account to follow, or none, and is the only one that
//! does. Then only for the owner epoch online (`preview::landed`), and a frame only when it is
//! newer than the one shown: a picture never goes back in time. What a task recognized and
//! meant on a frame goes only to that frame, by its key.
//!
//! On a task's frame the window draws the targets its recognition found, solid, and those it
//! did not, dashed, and the points its inputs were meant at, unless a person turned that off
//! for this run; never on an observation's, which has none. It draws none where the frame's
//! size the recognition states is not the frame's own: they would land on the wrong pixels.
//! And under the frame it hints that the account's game's standard pack declares no picture
//! of the frame's proportions, when the pack declares any (`ASPECT_TOLERANCE`).
//!
//! Under all that, the picked account's refresh (UI-3c-2, `refresh`): whether its button can be
//! pressed, why not when it cannot, and what its last refresh came to, as a pure function of
//! what the reads say of the account (`RefreshHold`, the model's) and of each account's own
//! ticket here. A frame the Runtime captured for a refresh of this run is said to be one when it
//! shows, by the request its record names.

use std::collections::BTreeMap;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use acui_source::workbench::preview::{
    CaptureFailure, FrameFailure, FrameKey, FrameOrigin, FrameSeen, Overlay,
};
use acui_source::workbench::Unreached;

use super::model::Fault;
use super::preview::{Order, Start, Target, LOOKUP_AGAIN, STALE_AFTER};

/// How far a frame's proportions may be from one its standard pack declares and still be
/// that one, as a fraction of it: 1920 x 1080, 1280 x 720 and 2560 x 1440 are all 16:9, and a
/// frame a few pixels short of it too. Landscape and portrait are told apart.
const ASPECT_TOLERANCE: f64 = 0.01;

/// Which account's picture shows, and what is known of each.
#[derive(Default)]
pub struct Pictures {
    /// Picked on its card, or named by `--account`; else the first card's, once a
    /// `Status` lists any account.
    selected: Option<String>,
    /// `--account` named `selected`, and no `Status` has listed any account since.
    named: bool,
    /// The frame the window shows, of the account picked.
    shown: Option<Shown>,
    /// How following the account goes.
    follow: Follow,
    /// By alias: what was found of each account this run.
    known: BTreeMap<String, Known>,
    /// The order the thread follows now; shared with it, which stops a read once it changes.
    generation: Arc<AtomicU64>,
    /// The account the thread was last told to follow, if any.
    sent: Option<Target>,
    /// A person turned the recognition's boxes and the inputs' points off, for this run.
    overlays_off: bool,
    /// How many refreshes were pressed this run: the last one's ticket.
    refreshes_issued: u64,
}

/// The frame the window shows.
struct Shown {
    alias: String,
    /// The owner epoch it was read in.
    epoch: String,
    frame: FrameSeen,
    /// Its size in pixels, decoded.
    size: (u32, u32),
    /// What its task recognized and meant on it, as far as that is known.
    overlay: Option<Overlay>,
    /// The log heard that the size the recognition states is not the frame's.
    mismatch_told: bool,
}

/// The size the records of a frame state, which is not the frame's own: for the log.
pub struct Mismatch {
    pub alias: String,
    pub sequence: u64,
    pub extent: (f32, f32),
    pub size: (u32, u32),
}

/// What was found of one account.
#[derive(Default)]
struct Known {
    /// The owner epoch the rest is of, but for the frames: their positions run on across a
    /// restart.
    epoch: Option<String>,
    /// When it was last looked up, and the position the next poll asks after.
    looked_up: Option<Instant>,
    after: Option<u64>,
    /// The last lookup found no frame.
    none_recent: bool,
    /// The newest frame seen of it, read or not.
    newest: Option<FrameSeen>,
    /// The newest frame of it that could not be read, by its position, and why.
    unreadable: Option<(u64, Unread)>,
    /// The newest capture of it that failed after any frame known.
    capture_failed: Option<CaptureFailure>,
    /// UI-3d asked for its device: the picture does not read it (`preview::halt`).
    halted: bool,
    /// Its refresh: the one out, what the last came to, and its cooldown.
    refresh: RefreshSlot,
    /// The request of its last refresh that captured a frame, as the wire spells it: the frame
    /// whose record names it is the person's refresh.
    refreshed: Option<String>,
}

/// The name of one refresh of an account's picture: its answer counts only under the ticket
/// still out for that account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshTicket(u64);

impl RefreshTicket {
    /// For the log.
    pub fn number(self) -> u64 {
        self.0
    }
}

/// One account's refresh: the one out, if any, with the generation it was pressed under; what
/// the last came to; and the one whose cooldown runs, which its timer ends (`refresh_cooled`).
#[derive(Default)]
struct RefreshSlot {
    out: Option<(RefreshTicket, u64)>,
    said: Option<Refreshed>,
    cooling: Option<RefreshTicket>,
}

/// What the last refresh of an account came to, before its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refreshed {
    /// One is out.
    Sending,
    /// The Runtime captured a frame: when, as its record states it, else when the answer came.
    Observed { at_ms: u64 },
    /// It captured one, and its records were not read back: the polls find it.
    Unread,
    /// It captured none: the host's code and the Runtime's, for their words.
    Refused { host: Option<String>, class: Option<String> },
    /// Nothing says what became of it: it may have captured one. It is not sent again.
    Unconfirmed,
    /// No connection: nothing was sent.
    NotSent,
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread { error: String },
    /// One was out across a restart.
    Restarted,
}

/// Why the reads say the account picked cannot be refreshed now: the first of these that
/// applies, in this order (`Model::refresh_view`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshHold {
    /// The main page does not show, or the window is minimized: nothing to say.
    Hidden,
    /// No Runtime answers, or none has yet.
    NotOnline,
    /// Its gate is shut (UI-3d).
    GateShut,
    /// It runs a task, whose frames the picture follows by itself.
    Running,
    /// It does something else the Runtime names.
    Busy(Doing),
    /// The Runtime last read its emulator as not running, at this time (unix ms). That reading
    /// may be stale: an `unknown` one holds nothing back.
    EmulatorOff { at_ms: u64 },
    /// It is fenced, or its recovery ladder holds.
    Fault,
}

/// What an account does, other than run a task, that holds its refresh back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doing {
    Reconnecting,
    SelfChecking,
    ControllingEmulator,
    Climbing,
}

/// Why the refresh's button cannot be pressed: of the reads, or of its own last press, which
/// cools down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshWhyNot {
    Hold(RefreshHold),
    CoolingDown,
}

/// What the picked account's refresh offers now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshView {
    /// The button can be pressed.
    pub offered: bool,
    /// Why it cannot; nothing while one is out, which `said` says.
    pub why_not: Option<RefreshWhyNot>,
    /// What the account's last refresh came to, if one was pressed this run.
    pub said: Option<Refreshed>,
}

impl Known {
    /// `epoch` followed now: what was looked up and polled before is of another.
    fn new_epoch(&mut self, epoch: &str) {
        self.epoch = Some(epoch.to_owned());
        self.looked_up = None;
        self.after = None;
        self.none_recent = false;
    }

    fn saw(&mut self, frame: &FrameSeen) {
        if self.newest.as_ref().map_or(true, |newest| frame.sequence > newest.sequence) {
            self.newest = Some(frame.clone());
        }
    }
}

/// How following the account picked goes.
#[derive(Debug, Clone, Default)]
enum Follow {
    /// Nothing is followed: no account, not shown, no Runtime, or its gate shut.
    #[default]
    Idle,
    /// Its newest frame is being looked up.
    Looking,
    Following,
    /// Its reads did not reach the Runtime; they are tried again a while later.
    Interrupted(Unreached),
    Fault(Fault),
}

/// Whether the picture may read an account now (B.2). In UI-3c only a halt shuts it, which
/// nothing asks for yet; UI-3d fills this from the account's player hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // `Withdrawn`, and when a freeze began: UI-3d's.
pub enum PreviewGate {
    Open,
    /// Not read: the picture stays on its last frame, dimmed.
    Freeze { since_ms: Option<u64> },
    /// Not read, and nothing of it shows.
    Withdrawn,
}

/// Why a frame is not shown, in the classes the picture has words for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unread {
    Missing,
    Evicted,
    TimedOut,
    Integrity,
    Busy,
    TooLarge,
    Undecodable,
    /// Refused, or the connection failed under the read.
    Failed,
}

impl Unread {
    pub fn of(failure: FrameFailure) -> Self {
        match failure {
            FrameFailure::Missing => Self::Missing,
            FrameFailure::Evicted => Self::Evicted,
            FrameFailure::TimedOut => Self::TimedOut,
            FrameFailure::Integrity => Self::Integrity,
            FrameFailure::Busy => Self::Busy,
            FrameFailure::TooLarge => Self::TooLarge,
            FrameFailure::Refused | FrameFailure::Connection => Self::Failed,
        }
    }

    /// For the log.
    pub fn name(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Evicted => "evicted",
            Self::TimedOut => "timed out",
            Self::Integrity => "does not verify",
            Self::Busy => "store busy",
            Self::TooLarge => "too large",
            Self::Undecodable => "does not decode",
            Self::Failed => "read failed",
        }
    }

    /// Something is wrong, rather than a frame cleaned up or a store busy for a moment: the
    /// window points at the log, and says it in red.
    pub fn is_problem(self) -> bool {
        match self {
            Self::Missing | Self::Evicted | Self::Busy => false,
            Self::TimedOut
            | Self::Integrity
            | Self::TooLarge
            | Self::Undecodable
            | Self::Failed => true,
        }
    }
}

/// The Runtime as the picture area tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkNow<'a> {
    /// No `Status` answered yet.
    Pending,
    /// Online, under this owner epoch.
    Online(&'a str),
    Offline(Offline),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offline {
    /// No Runtime runs.
    NotRunning,
    /// One may run, and does not answer, or reading it went wrong.
    Unreachable,
}

/// What the picture area shows, before its words.
#[derive(Debug, Clone, PartialEq)]
pub enum PictureView {
    /// There is no card to show the picture of.
    NoAccount,
    /// The account's gate took the picture down (UI-3d).
    Withdrawn,
    /// No frame shows, and why.
    Empty { why: EmptyWhy },
    /// A frame shows: when it was captured; how long ago, in seconds, once that is longer
    /// than `STALE_AFTER`; what took it; and why it is dimmed, if it is. What is drawn on
    /// it; and the frame's proportions, in lowest terms, when the account's game's standard
    /// pack declares others and not these.
    Frame {
        captured_at_ms: u64,
        age: Option<u64>,
        origin: Origin,
        dim: Option<DimWhy>,
        overlay: OverlayView,
        aspect: Option<(u32, u32)>,
    },
}

/// What is drawn on the frame shown.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayView {
    /// Turned off.
    Off,
    /// What its task recognized and meant on it.
    Shown(Overlay),
    /// Nothing is recorded of it: an observation's frame, or a task's before its
    /// recognition is.
    NotOnThisFrame,
    /// The size the records state is not the frame's: nothing is drawn.
    Mismatch,
}

/// Why no frame shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyWhy {
    /// The account's latest frame is being looked for.
    Looking,
    /// The last lookup found none.
    NoneRecent,
    /// The newest frame could not be shown.
    Unreadable(Unread),
    Offline(Offline),
    /// Following it did not reach the Runtime, and is tried again.
    Interrupted,
    /// Its gate is shut for now (UI-3d).
    Freeze,
    Fault(Fault),
}

/// Why the frame shown is dimmed: it is not the newest, or may not be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimWhy {
    Offline(Offline),
    Freeze,
    Fault(Fault),
    Interrupted,
    /// The Runtime started again, and the newest frame is being looked for.
    Restarted,
    /// A newer capture failed, with its outcome code when the record states one.
    CaptureFailed(Option<String>),
    /// A newer frame could not be shown.
    NewerUnreadable(Unread),
}

/// What took the frame shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Task,
    Observation,
    /// An observation a person's refresh here asked for, this run.
    OwnRefresh,
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
    pub fn account<'a>(&self, aliases: &[&'a str]) -> Option<&'a str> {
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

    /// The generation the thread is to read for, shared with it.
    pub fn generation(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.generation)
    }

    pub fn generation_now(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// What the thread is to do now that the window shows `account`'s picture, and is to
    /// follow `target`, if anything: another account to follow, or none, is a new generation;
    /// the same one only changes the pace. Another account than the one shown, or a gate
    /// that took the picture down, lets go of the frame shown.
    pub fn sync(
        &mut self,
        account: Option<&str>,
        target: Option<Target>,
        withdrawn: bool,
        now: Instant,
    ) -> Option<Order> {
        let other = self.shown.as_ref().is_some_and(|shown| Some(shown.alias.as_str()) != account);
        if withdrawn || other {
            self.shown = None;
        }
        let same = match (&self.sent, &target) {
            (None, None) => true,
            (Some(sent), Some(target)) => sent.same(target),
            (Some(_), None) | (None, Some(_)) => false,
        };
        if same {
            let target = target?;
            let sent = self.sent.as_mut()?;
            if sent.fast == target.fast {
                return None;
            }
            sent.fast = target.fast;
            return Some(Order::Pace(target.fast));
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let thread_lost = matches!(self.follow, Follow::Fault(Fault::NoThread(_)));
        self.sent = target.clone();
        let Some(target) = target else {
            if !thread_lost {
                self.follow = Follow::Idle;
            }
            return Some(Order::Stop);
        };
        let known = self.known.entry(target.alias.clone()).or_default();
        if known.epoch.as_deref() != Some(target.epoch.as_str()) {
            known.new_epoch(&target.epoch);
        }
        let start = match (known.looked_up, known.after) {
            (Some(at), Some(after)) if now.saturating_duration_since(at) < LOOKUP_AGAIN => {
                Start::Known { after, newest: known.newest.clone() }
            }
            _ => Start::Lookup,
        };
        if !thread_lost {
            self.follow = match start {
                Start::Lookup => Follow::Looking,
                Start::Known { .. } => Follow::Following,
            };
        }
        let shown =
            self.shown.as_ref().map(|shown| (shown.frame.sequence, shown.frame.key.clone()));
        Some(Order::Follow { generation, target, start, shown })
    }

    /// Whether the recognition's boxes and the inputs' points are drawn, as a person left it.
    pub fn overlays_on(&self) -> bool {
        !self.overlays_off
    }

    pub fn set_overlays(&mut self, on: bool) {
        self.overlays_off = !on;
    }

    /// Following goes on: whether it had stopped short before, for the log.
    fn resume(&mut self) -> bool {
        let stopped =
            matches!(self.follow, Follow::Interrupted(_) | Follow::Fault(Fault::Panicked(_)));
        if !matches!(self.follow, Follow::Idle | Follow::Fault(Fault::NoThread(_))) {
            self.follow = Follow::Following;
        }
        stopped
    }

    /// A lookup of `alias` found `found`, or nothing; polls go on after `after`. Whether
    /// following had stopped short before.
    pub fn looked(
        &mut self,
        alias: &str,
        found: Option<FrameSeen>,
        after: u64,
        now: Instant,
    ) -> bool {
        let known = self.known.entry(alias.to_owned()).or_default();
        known.looked_up = Some(now);
        known.after = Some(after);
        known.none_recent = found.is_none();
        if let Some(frame) = &found {
            known.saw(frame);
        }
        self.resume()
    }

    /// A poll of `alias` found a new frame, a failed capture, or only that following goes on.
    /// Whether it had stopped short before, and the failed capture when it is newer than any
    /// frame known, for the log.
    pub fn polled(
        &mut self,
        alias: &str,
        after: u64,
        newest: Option<FrameSeen>,
        failure: Option<CaptureFailure>,
    ) -> (bool, Option<CaptureFailure>) {
        let shown = self.shown_sequence(alias);
        let known = self.known.entry(alias.to_owned()).or_default();
        known.after = Some(after);
        if let Some(frame) = &newest {
            known.saw(frame);
            known.none_recent = false;
        }
        let floor = shown.max(known.newest.as_ref().map(|frame| frame.sequence));
        let before = known.capture_failed.as_ref().map(|failed| failed.sequence);
        let failure = failure.filter(|failed| {
            floor.map_or(true, |floor| failed.sequence > floor)
                && before.map_or(true, |before| failed.sequence > before)
        });
        if let Some(failed) = &failure {
            known.capture_failed = Some(failed.clone());
        }
        (self.resume(), failure)
    }

    /// A frame of `alias` read in `epoch`, and decoded to `size`, with what its task
    /// recognized and meant on it as far as that is known: whether it shows, which it does
    /// when it is newer than the one shown. Then the window takes its pixels.
    pub fn frame_shown(
        &mut self,
        alias: String,
        epoch: String,
        frame: FrameSeen,
        size: (u32, u32),
        overlay: Option<Overlay>,
    ) -> bool {
        if self.shown_sequence(&alias).is_some_and(|shown| shown >= frame.sequence) {
            return false;
        }
        let known = self.known.entry(alias.clone()).or_default();
        known.saw(&frame);
        known.none_recent = false;
        if known.unreadable.is_some_and(|(sequence, _)| sequence <= frame.sequence) {
            known.unreadable = None;
        }
        if known.capture_failed.as_ref().is_some_and(|failed| failed.sequence < frame.sequence) {
            known.capture_failed = None;
        }
        self.shown = Some(Shown { alias, epoch, frame, size, overlay, mismatch_told: false });
        self.resume();
        true
    }

    /// What `alias`'s task recognized and meant on its frame `key`, as known now: whether
    /// that changed what is drawn, which it does only on that frame while it shows.
    pub fn marks_landed(&mut self, alias: &str, key: &FrameKey, overlay: Option<Overlay>) -> bool {
        let Some(shown) = self.shown.as_mut() else {
            return false;
        };
        if shown.alias != alias || shown.frame.key != *key || shown.overlay == overlay {
            return false;
        }
        shown.overlay = overlay;
        true
    }

    /// The frame shown, when the size its records state is not its own and the log has not
    /// heard so: once for each frame.
    pub fn untold_mismatch(&mut self) -> Option<Mismatch> {
        let shown = self.shown.as_mut()?;
        let extent = shown.overlay.as_ref()?.extent?;
        if shown.mismatch_told || fits(extent, shown.size) {
            return None;
        }
        shown.mismatch_told = true;
        Some(Mismatch {
            alias: shown.alias.clone(),
            sequence: shown.frame.sequence,
            extent,
            size: shown.size,
        })
    }

    /// The frame of `alias` at `sequence` will not show, and why.
    pub fn unreadable(&mut self, alias: &str, sequence: u64, why: Unread) {
        if self.shown_sequence(alias).is_some_and(|shown| shown >= sequence) {
            return;
        }
        let known = self.known.entry(alias.to_owned()).or_default();
        if known.unreadable.map_or(true, |(before, _)| sequence >= before) {
            known.unreadable = Some((sequence, why));
        }
    }

    /// A round did not reach the Runtime: whether that is news for the log.
    pub fn interrupted(&mut self, failure: Unreached) -> bool {
        if matches!(self.follow, Follow::Fault(Fault::NoThread(_))) {
            return false;
        }
        let news = match &self.follow {
            Follow::Interrupted(before) => {
                mem::discriminant(before) != mem::discriminant(&failure)
                    || before.failure() != failure.failure()
            }
            Follow::Idle | Follow::Looking | Follow::Following | Follow::Fault(_) => true,
        };
        self.follow = Follow::Interrupted(failure);
        news
    }

    /// A round panicked: whether that is news for the log.
    pub fn panicked(&mut self, message: String) -> bool {
        if matches!(self.follow, Follow::Fault(Fault::NoThread(_))) {
            return false;
        }
        let fault = Fault::Panicked(message);
        let news = !matches!(&self.follow, Follow::Fault(before) if *before == fault);
        self.follow = Follow::Fault(fault);
        news
    }

    /// The thread did not start, or is gone: why.
    pub fn thread_lost(&mut self, why: String) {
        self.follow = Follow::Fault(Fault::NoThread(why));
    }

    pub fn halted(&self, alias: &str) -> bool {
        self.known.get(alias).is_some_and(|known| known.halted)
    }

    pub fn set_halted(&mut self, alias: &str, halted: bool) {
        self.known.entry(alias.to_owned()).or_default().halted = halted;
    }

    /// Whether the thread follows `alias` still under `generation`.
    pub fn follows(&self, alias: &str, generation: u64) -> bool {
        self.generation_now() == generation
            && self.sent.as_ref().is_some_and(|sent| sent.alias == alias)
    }

    /// A refresh of `alias` pressed: its ticket, under the generation now, unless one of it is
    /// out or cools down. What it came to is, for now, that it is being sent.
    pub fn refresh_begin(&mut self, alias: &str) -> Option<RefreshTicket> {
        let generation = self.generation_now();
        let slot = &mut self.known.entry(alias.to_owned()).or_default().refresh;
        if slot.out.is_some() || slot.cooling.is_some() {
            return None;
        }
        self.refreshes_issued += 1;
        let ticket = RefreshTicket(self.refreshes_issued);
        slot.out = Some((ticket, generation));
        slot.said = Some(Refreshed::Sending);
        Some(ticket)
    }

    /// What the refresh of `alias` under `ticket` came to: the generation it was pressed under,
    /// when that ticket is still the one out, which is all that makes it count. Its cooldown runs
    /// from now, whatever it came to. `request`: of one that captured a frame, which shows as the
    /// person's refresh when its record names it.
    pub fn refresh_settle(
        &mut self,
        alias: &str,
        ticket: RefreshTicket,
        said: Refreshed,
        request: Option<String>,
    ) -> Option<u64> {
        let known = self.known.get_mut(alias)?;
        let (out, generation) = known.refresh.out?;
        if out != ticket {
            return None;
        }
        known.refresh = RefreshSlot { out: None, said: Some(said), cooling: Some(ticket) };
        if request.is_some() {
            known.refreshed = request;
        }
        Some(generation)
    }

    /// The cooldown of `alias`'s refresh under `ticket` ran out: whether it was the one that ran.
    pub fn refresh_cooled(&mut self, alias: &str, ticket: RefreshTicket) -> bool {
        let Some(known) = self.known.get_mut(alias) else {
            return false;
        };
        if known.refresh.cooling != Some(ticket) {
            return false;
        }
        known.refresh.cooling = None;
        true
    }

    /// The Runtime started again: no refresh out holds across that. Each is let go, says it was
    /// out across a restart, and cools down from now. The ones let go, by alias, for the log and
    /// their cooldowns.
    pub fn refresh_restarted(&mut self) -> Vec<(String, RefreshTicket)> {
        let mut released = Vec::new();
        for (alias, known) in &mut self.known {
            if let Some((ticket, _)) = known.refresh.out.take() {
                known.refresh.said = Some(Refreshed::Restarted);
                known.refresh.cooling = Some(ticket);
                released.push((alias.clone(), ticket));
            }
        }
        released
    }

    /// What `alias`'s refresh offers now, `hold` being why the reads say it cannot be, if they
    /// do: the first of those, then one out, then one cooling down.
    pub fn refresh_view(&self, alias: &str, hold: Option<RefreshHold>) -> RefreshView {
        let slot = self.known.get(alias).map(|known| &known.refresh);
        let out = slot.is_some_and(|slot| slot.out.is_some());
        let cooling = slot.is_some_and(|slot| slot.cooling.is_some());
        let why_not = match hold {
            Some(hold) => Some(RefreshWhyNot::Hold(hold)),
            // The line of what it came to says that one is out.
            None if out => None,
            None if cooling => Some(RefreshWhyNot::CoolingDown),
            None => None,
        };
        RefreshView {
            offered: hold.is_none() && !out && !cooling,
            why_not,
            said: slot.and_then(|slot| slot.said.clone()),
        }
    }

    fn shown_sequence(&self, alias: &str) -> Option<u64> {
        self.shown.as_ref().filter(|shown| shown.alias == alias).map(|shown| shown.frame.sequence)
    }

    /// The picture area of `account`, the one `account` gave, or of none, with the Runtime as
    /// `link` tells it and its gate, at `now_ms`; `ratios`, the picture proportions its game's
    /// standard pack declares, if it declares any.
    pub fn view(
        &self,
        account: Option<&str>,
        link: LinkNow<'_>,
        gate: PreviewGate,
        ratios: Option<&[(u32, u32)]>,
        now_ms: u64,
    ) -> PictureView {
        let Some(alias) = account else {
            return PictureView::NoAccount;
        };
        if gate == PreviewGate::Withdrawn {
            return PictureView::Withdrawn;
        }
        let known = self.known.get(alias);
        let offline = match link {
            LinkNow::Offline(offline) => Some(offline),
            LinkNow::Pending | LinkNow::Online(_) => None,
        };
        let frozen = matches!(gate, PreviewGate::Freeze { .. });
        let fault = match &self.follow {
            Follow::Fault(fault) => Some(fault.clone()),
            Follow::Idle | Follow::Looking | Follow::Following | Follow::Interrupted(_) => None,
        };
        let interrupted = matches!(self.follow, Follow::Interrupted(_));
        let Some(shown) = self.shown.as_ref().filter(|shown| shown.alias == alias) else {
            let unreadable = known.and_then(|known| known.unreadable).map(|(_, why)| why);
            let why = if let Some(offline) = offline {
                EmptyWhy::Offline(offline)
            } else if frozen {
                EmptyWhy::Freeze
            } else if let Some(fault) = fault {
                EmptyWhy::Fault(fault)
            } else if interrupted {
                // Ahead of what earlier reads found: while following stops short, that may no
                // longer be so, and the reads failing would show nowhere else.
                EmptyWhy::Interrupted
            } else if let Some(why) = unreadable {
                EmptyWhy::Unreadable(why)
            } else if known.is_some_and(|known| known.none_recent) {
                EmptyWhy::NoneRecent
            } else {
                EmptyWhy::Looking
            };
            return PictureView::Empty { why };
        };
        let captured_at_ms = shown.frame.captured_at_ms;
        let age_ms = now_ms.saturating_sub(captured_at_ms);
        let age = (u128::from(age_ms) > STALE_AFTER.as_millis()).then_some(age_ms / 1000);
        let refreshed = known.and_then(|known| known.refreshed.as_deref());
        let origin = match shown.frame.origin {
            FrameOrigin::Task { .. } => Origin::Task,
            FrameOrigin::Observation
                if refreshed.is_some() && shown.frame.request.as_deref() == refreshed =>
            {
                Origin::OwnRefresh
            }
            FrameOrigin::Observation => Origin::Observation,
        };
        let restarted = matches!(self.follow, Follow::Looking)
            && matches!(link, LinkNow::Online(epoch) if epoch != shown.epoch);
        let dim = if let Some(offline) = offline {
            Some(DimWhy::Offline(offline))
        } else if frozen {
            Some(DimWhy::Freeze)
        } else if let Some(fault) = fault {
            Some(DimWhy::Fault(fault))
        } else if interrupted {
            Some(DimWhy::Interrupted)
        } else if restarted {
            Some(DimWhy::Restarted)
        } else {
            known.and_then(|known| newer(known, shown.frame.sequence))
        };
        let overlay = match &shown.overlay {
            _ if self.overlays_off => OverlayView::Off,
            // Only a task recognizes, and only on a task's frame is anything drawn.
            _ if origin != Origin::Task => OverlayView::NotOnThisFrame,
            None => OverlayView::NotOnThisFrame,
            Some(overlay) if overlay.extent.map_or(true, |extent| fits(extent, shown.size)) => {
                OverlayView::Shown(overlay.clone())
            }
            Some(_) => OverlayView::Mismatch,
        };
        let aspect = ratios.and_then(|ratios| undeclared(shown.size, ratios));
        PictureView::Frame { captured_at_ms, age, origin, dim, overlay, aspect }
    }
}

/// Whether `extent`, the size a frame's records state, is `size`, the frame's own.
fn fits((width, height): (f32, f32), size: (u32, u32)) -> bool {
    (width - size.0 as f32).abs() < 0.5 && (height - size.1 as f32).abs() < 0.5
}

/// A frame of `size`'s proportions in lowest terms, when they are none of `ratios` within
/// `ASPECT_TOLERANCE`; none when they are one, or there are none to compare with.
fn undeclared((width, height): (u32, u32), ratios: &[(u32, u32)]) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || ratios.is_empty() {
        return None;
    }
    let frame = f64::from(width) / f64::from(height);
    let declared = ratios.iter().any(|&(ratio_width, ratio_height)| {
        let ratio = f64::from(ratio_width) / f64::from(ratio_height);
        (frame / ratio - 1.0).abs() <= ASPECT_TOLERANCE
    });
    if declared {
        return None;
    }
    let divisor = gcd(width, height);
    Some((width / divisor, height / divisor))
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// What is known of an account past the frame shown, at `shown`: the newer of a capture that
/// failed and a frame that could not be read.
fn newer(known: &Known, shown: u64) -> Option<DimWhy> {
    let failed = known.capture_failed.as_ref().filter(|failed| failed.sequence > shown);
    let unreadable = known.unreadable.filter(|(sequence, _)| *sequence > shown);
    match (failed, unreadable) {
        (Some(failed), Some((sequence, _))) if failed.sequence > sequence => {
            Some(DimWhy::CaptureFailed(failed.code.clone()))
        }
        (Some(failed), None) => Some(DimWhy::CaptureFailed(failed.code.clone())),
        (_, Some((_, why))) => Some(DimWhy::NewerUnreadable(why)),
        (None, None) => None,
    }
}
