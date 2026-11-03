// SPDX-License-Identifier: GPL-3.0-only
//! A person's refresh of the live view (UI-3c-2): "capture one frame", under the picture of the
//! account picked. It is the one write the picture has, and none of the picture thread's: it
//! goes out once, on a thread of its own (`THREAD`), over a fresh person's connection
//! (`observe_frame`), and whatever comes back, nothing is sent again by itself. The frame the
//! Runtime captures shows the way every other frame does: the picture thread reads it, handed
//! over when its record came back with the answer (`preview::offer`), or found by its polls; so
//! the picture has one way in, and the line under the frame says it is the person's refresh.
//!
//! Each account has a ticket of its own (`Pictures::refresh_begin`), apart from the top bar's
//! writes (`actions`): an answer that lands counts no write for the reads (`super::wrote`), since
//! a captured frame changes nothing they show. The button is off while the reads say a refresh
//! would get in a task's or the device's way, or fail (`Model::refresh_view`), while one is out,
//! and for `REFRESH_COOLDOWN` after it lands, whatever it came to, or after a restart let it go;
//! the window never counts that down. Only the account picked has the button: an answer for
//! another lands in its own slot, and shows when that account is picked again.
//!
//! Every answer goes in the log, from the thread, so one that lands after the event loop is not
//! lost: quietly when a frame was captured, as the problem it is otherwise. The end-of-run count
//! is the picture's (`PreviewTally`).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use acui_source::workbench::preview::{observe_frame, FrameSeen, ObserveOutcome};
use slint::Timer;

use super::failure_line;
use super::paint;
use super::picture::{RefreshTicket, Refreshed};
use super::preview::{self, RefreshCount, REFRESH_COOLDOWN};
use super::read::panic_message;
use crate::launcher::spawn_worker;
use crate::report;
use crate::ui::{self, Ui};

/// The refresh's thread, also what its lines in the technical log are under.
pub const THREAD: &str = "acui-act-observe";

/// What one refresh came to, as its thread hands it back.
enum Landing {
    Answer(ObserveOutcome),
    /// The thread panicked: whether the request got out is not known.
    Panicked(String),
    /// The system refused the thread: nothing was sent. The error is the system's.
    NoThread(String),
}

impl Landing {
    /// What it came to, before its words, at `now_ms`; the frame it captured, when its record
    /// came back with the answer; and the request of one that captured a frame.
    fn said(self, now_ms: u64) -> (Refreshed, Option<FrameSeen>, Option<String>) {
        match self {
            Self::Answer(ObserveOutcome::Observed { frame, request, .. }) => {
                let at_ms = frame.as_ref().map_or(now_ms, |frame| frame.captured_at_ms);
                (Refreshed::Observed { at_ms }, frame, Some(request))
            }
            Self::Answer(ObserveOutcome::ObservedUnread(_)) => (Refreshed::Unread, None, None),
            Self::Answer(ObserveOutcome::Refused(failure)) => {
                let host = failure.host.map(|(code, _)| code);
                (Refreshed::Refused { host, class: failure.runtime_code }, None, None)
            }
            Self::Answer(ObserveOutcome::NotSent(_)) => (Refreshed::NotSent, None, None),
            Self::Answer(ObserveOutcome::Unconfirmed(_)) | Self::Panicked(_) => {
                (Refreshed::Unconfirmed, None, None)
            }
            Self::NoThread(error) => (Refreshed::NoThread { error }, None, None),
        }
    }

    fn count(&self) -> RefreshCount {
        match self {
            Self::Answer(ObserveOutcome::Observed { .. }) => RefreshCount::Observed,
            Self::Answer(ObserveOutcome::ObservedUnread(_)) => RefreshCount::Unread,
            Self::Answer(ObserveOutcome::Refused(_)) => RefreshCount::Refused,
            Self::Answer(ObserveOutcome::NotSent(_)) | Self::NoThread(_) => RefreshCount::NotSent,
            Self::Answer(ObserveOutcome::Unconfirmed(_)) | Self::Panicked(_) => {
                RefreshCount::Unconfirmed
            }
        }
    }

    /// Into the technical log, codes and all: a frame captured without lighting up the log's
    /// place on the windows; the rest as the problems they are.
    fn log(&self, alias: &str, ticket: RefreshTicket) {
        let prefix = format!("ticket {} of {alias}", ticket.number());
        let line = match self {
            Self::Answer(ObserveOutcome::Observed { frame, request, width, height }) => {
                let frame = match frame {
                    Some(frame) => {
                        format!("frame {} captured at {} ms", frame.sequence, frame.captured_at_ms)
                    }
                    None => "no record of the frame came back; the polls find it".to_owned(),
                };
                let line =
                    format!("{prefix}: observed {width} x {height}, request {request}, {frame}");
                report::log_quiet(THREAD, &line);
                return;
            }
            Self::Answer(ObserveOutcome::ObservedUnread(failure)) => {
                format!("{prefix}: captured, its records not read back: {}", failure_line(failure))
            }
            Self::Answer(ObserveOutcome::Refused(failure)) => {
                format!("{prefix}: refused: {}", failure_line(failure))
            }
            Self::Answer(ObserveOutcome::NotSent(failure)) => {
                format!("{prefix}: not sent: {}", failure_line(failure))
            }
            Self::Answer(ObserveOutcome::Unconfirmed(failure)) => {
                format!("{prefix}: unconfirmed, not sent again: {}", failure_line(failure))
            }
            Self::Panicked(message) => {
                format!("{prefix}: panicked, unconfirmed, not sent again: {message}")
            }
            Self::NoThread(error) => format!("{prefix}: no thread, not sent: {error}"),
        };
        report::log(THREAD, &line);
    }
}

/// The button: a refresh of the account whose picture shows, once, when it may be pressed now.
/// It is off otherwise, so a press that lands after it went off sends nothing. A thread the
/// system refuses is that press's answer at once, in red; a panic on it is an answer as well:
/// with none at all the button would stay off for good.
pub fn pressed(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let Some(tally) =
        ui.reading.borrow().as_ref().map(|reading| Arc::clone(&reading.preview_tally))
    else {
        return;
    };
    let begun = {
        let mut model = ui.model.borrow_mut();
        let offered = model.refresh_view().is_some_and(|view| view.offered);
        let alias = model.picture_account().map(str::to_owned);
        match alias {
            Some(alias) if offered => {
                model.pictures_mut().refresh_begin(&alias).map(|ticket| (alias, ticket))
            }
            Some(_) | None => None,
        }
    };
    let Some((alias, ticket)) = begun else {
        return;
    };
    let root = launcher.state_root.clone();
    let (worker_alias, worker_tally) = (alias.clone(), Arc::clone(&tally));
    let spawned = spawn_worker(THREAD, move || {
        let alias = worker_alias;
        let landing = match catch_unwind(AssertUnwindSafe(|| observe_frame(&root, &alias))) {
            Ok(outcome) => Landing::Answer(outcome),
            Err(panic) => Landing::Panicked(panic_message(&*panic)),
        };
        landing.log(&alias, ticket);
        worker_tally.refreshed(landing.count());
        report::post(THREAD, move || {
            if let Some(ui) = ui::current() {
                landed(&ui, &alias, ticket, landing);
            }
        });
    });
    match spawned {
        Ok(()) => paint::refresh(ui),
        Err(error) => {
            let landing = Landing::NoThread(error.to_string());
            landing.log(&alias, ticket);
            tally.refreshed(landing.count());
            landed(ui, &alias, ticket, landing);
        }
    }
}

/// An answer, on the event loop. One whose ticket is no longer the one out, which a restart let
/// go, changes nothing shown: only the log had it. A frame it captured goes to the picture
/// thread, which reads it as any other.
fn landed(ui: &Ui, alias: &str, ticket: RefreshTicket, landing: Landing) {
    let (said, frame, request) = landing.said(paint::now_ms());
    let pressed_under =
        ui.model.borrow_mut().pictures_mut().refresh_settle(alias, ticket, said, request);
    let Some(generation) = pressed_under else {
        let line = format!(
            "ticket {} of {alias}: its answer came after a restart let it go, and counts for \
             nothing",
            ticket.number()
        );
        report::log_quiet(THREAD, &line);
        ui.paint_log_notes();
        return;
    };
    cool_down(alias.to_owned(), ticket);
    if let Some(frame) = frame {
        preview::offer(ui, alias, generation, frame);
    }
    // The line under the frame may say now that it is the person's refresh.
    paint::picture(ui);
    ui.paint_log_notes();
}

/// The Runtime started again: every refresh out is let go, says so, and cools down from now.
/// Painting is the caller's.
pub fn restarted(ui: &Ui) {
    let released = ui.model.borrow_mut().pictures_mut().refresh_restarted();
    for (alias, ticket) in released {
        let line = format!(
            "ticket {} of {alias}: let go, the Runtime started again; what came of it is not known",
            ticket.number()
        );
        report::log_quiet(THREAD, &line);
        cool_down(alias, ticket);
    }
}

/// `alias`'s refresh under `ticket` cools down: its button comes back once this runs out.
fn cool_down(alias: String, ticket: RefreshTicket) {
    Timer::single_shot(REFRESH_COOLDOWN, move || {
        let Some(ui) = ui::current() else {
            return;
        };
        let cooled = ui.model.borrow_mut().pictures_mut().refresh_cooled(&alias, ticket);
        if cooled {
            paint::refresh(&ui);
        }
    });
}
