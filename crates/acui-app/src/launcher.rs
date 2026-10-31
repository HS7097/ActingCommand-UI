// SPDX-License-Identifier: GPL-3.0-only
//! The launcher: the one place the console starts the daemon, and the one
//! place it asks the Runtime to stop. Start and Request Shutdown are on the
//! workbench, which says what the launcher found in plain words; every ledger
//! window shows the technical lines and holds the unlock entry (`paint`).
//!
//! Start spawns `actingcommand-actingd --config <actingd_config>` detached,
//! with its stdout and stderr in a log file under the console's own directory
//! — never inside a state root. The child handle is kept only to notice an
//! early exit through `try_wait`; it is never killed, never waited on, never
//! put in a job object, and is dropped when the readiness poll ends, so the
//! daemon outlives the console. Readiness is one typed connect per attempt,
//! never a parse of the daemon's output. Shutdown is a typed request through
//! the client, recorded as a client action first; a request refused as
//! `runtime_busy` is sent again, a second later, a few times, and any other
//! refusal is shown verbatim. A start press is recorded as a client action too,
//! but only once a Runtime takes a connection: a start that never got ready
//! records nothing.
//!
//! After an early exit the log is read back once, for actingd's terminal
//! outcome: its last `OUTCOME actingd:` line that holds an outcome envelope
//! (runtime-outcomes 1), shown as the developer line written with it. When
//! that outcome's code is `owner_resource_unconfirmed` the launcher offers
//! `actingd unlock-owner`, run only on a second, confirming press in a ledger
//! window: no console window, output captured, killed and reaped if it
//! outlives its timeout. It appends its own ledger fact, so no client action is
//! recorded for it; a v2 report whose outcome is `unlock_owner_completed`, with
//! exit code 0, starts once more, as a start press does.
//!
//! Nothing here waits on the event loop: a probe, a whole start (probe, launch,
//! readiness, record), a shutdown and an unlock each run on a thread of their
//! own, one of each kind at a time, and hand back facts and `Phrase`s, never
//! text. What they said is kept in the `LauncherView` and painted from there in
//! the language shown at that moment: plain lines for a person on the workbench
//! (`LauncherKind`), and the technical ones, with the codes, pids and paths, in
//! every ledger window. A probe that finds a Runtime up is also what offers a
//! ledger window that reads offline for want of one to read it online.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use acui_installation::Snapshot;
use acui_source::{
    probe_runtime, record_start, request_shutdown, ClientFailure, RuntimeFacts, SHUTDOWN_ATTEMPTS,
};
use serde_json::Value;

use crate::install::InstallPaths;
use crate::phrase::{phrase, verbatim, Line, Phrase};
use crate::report;
use crate::strings::{fill, outcome_short, Labels};
use crate::ui::Ui;

/// Readiness is polled: up to this many connect attempts, this far apart.
const READY_ATTEMPTS: u32 = 60;
const READY_INTERVAL: Duration = Duration::from_millis(500);

/// Windows `DETACHED_PROCESS`: the daemon gets no console and does not inherit
/// the console's own, so it outlives the console and never sees its Ctrl+C.
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Windows `CREATE_NO_WINDOW`: unlock-owner's output is captured, so it gets
/// no console of its own rather than a detached one.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// actingd's outcome line (runtime-outcomes 1, Runtime contracts/runtime-outcomes.md):
/// `OUTCOME actingd: <envelope>`, written in one write with its developer line
/// just before it.
const OUTCOME_PREFIX: &str = "OUTCOME actingd: ";
const OUTCOME_SCHEMA: &str = "actingcommand.outcome.v1";
/// The developer line's levels: `<LEVEL> actingd: <code> <text>`.
const DEVELOPER_PREFIXES: [&str; 4] =
    ["INFO actingd:", "WARN actingd:", "ERROR actingd:", "FATAL actingd:"];
/// The one terminal outcome code the launcher answers, with the unlock entry.
const OWNER_RESOURCE_UNCONFIRMED: &str = "owner_resource_unconfirmed";
/// Names the surface, not a person: no user name goes into the ledger.
const UNLOCK_ACTOR: &str = "acui";
const UNLOCK_SCHEMA: &str = "actingcommand.actingd.unlock-owner.v2";
/// The outcome code of a report whose unlock took effect.
const UNLOCK_COMPLETED: &str = "unlock_owner_completed";
/// After its journal append the command opens the ledger and recovers the
/// exited owner's writer within the Runtime's own 120 s maintenance budget;
/// killing it inside that window would leave the journal record without its
/// ledger fact, so this bound sits well above it.
const UNLOCK_TIMEOUT: Duration = Duration::from_secs(180);
const UNLOCK_POLL: Duration = Duration::from_millis(100);

/// The state root and the two launcher paths, as this run resolved them, and
/// the requests in flight.
pub struct Launcher {
    pub state_root: PathBuf,
    pub actingd_config: Option<PathBuf>,
    pub actingd_exe: Option<PathBuf>,
    /// The installation inputs of this process, retained across all UI actions.
    installation: Option<Snapshot>,
    /// One start in flight at a time: set from the press until its thread's
    /// last word is painted.
    starting: Arc<AtomicBool>,
    /// Set while unlock-owner runs; a start is refused meanwhile, and the
    /// workbench does not close.
    unlocking: Arc<AtomicBool>,
    /// One shutdown request in flight at a time.
    shutting_down: Arc<AtomicBool>,
}

impl Launcher {
    pub fn new(paths: InstallPaths) -> Self {
        let InstallPaths { installation, state_root, actingd_config, actingd_exe } = paths;
        Self {
            state_root,
            actingd_config,
            actingd_exe,
            installation,
            starting: Arc::new(AtomicBool::new(false)),
            unlocking: Arc::new(AtomicBool::new(false)),
            shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether unlock-owner runs now.
    pub fn unlocking(&self) -> bool {
        self.unlocking.load(Ordering::SeqCst)
    }
}

/// What a person reads about the launcher, in plain words: no key, code, pid
/// or path; the technical line keeps those. An outcome code shows only as its
/// short words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherKind {
    /// A probe is out, and nothing has answered it yet.
    Checking,
    Starting,
    Running,
    NotRunning,
    /// The probe could not run: no thread for it.
    CheckFailed,
    /// The start brought no Runtime up; the terminal outcome's code when the
    /// log held one.
    StartFailed { code: Option<String> },
    /// The last Runtime left its owner unconfirmed: the unlock entry is offered.
    NeedsUnlock,
    Stopping,
    StopRequested,
    /// The shutdown request was refused or failed: the refusal's code, else
    /// the client's.
    StopFailed { code: Option<String> },
    /// A path the start needs is not set, or not absolute.
    NotConfigured,
    /// A request of the same kind has not finished.
    Busy,
}

impl LauncherKind {
    pub fn human(&self, labels: &Labels) -> String {
        match self {
            Self::Checking => labels.launcher_checking.to_owned(),
            Self::Starting => labels.launcher_starting.to_owned(),
            Self::Running => labels.launcher_running.to_owned(),
            Self::NotRunning => labels.launcher_not_running.to_owned(),
            Self::CheckFailed => labels.launcher_check_failed.to_owned(),
            Self::StartFailed { code } => {
                because(labels, labels.launcher_start_failed, code.as_deref())
            }
            Self::NeedsUnlock => labels.launcher_needs_unlock.to_owned(),
            Self::Stopping => labels.launcher_stopping.to_owned(),
            Self::StopRequested => labels.launcher_stop_requested.to_owned(),
            Self::StopFailed { code } => {
                because(labels, labels.launcher_stop_failed, code.as_deref())
            }
            Self::NotConfigured => labels.launcher_not_configured.to_owned(),
            Self::Busy => labels.launcher_busy.to_owned(),
        }
    }

    /// Whether the plain line says a failure.
    pub fn failed(&self) -> bool {
        matches!(
            self,
            Self::CheckFailed
                | Self::StartFailed { .. }
                | Self::NeedsUnlock
                | Self::StopFailed { .. }
                | Self::NotConfigured
        )
    }
}

/// A plain sentence, then the short words of the code behind it, if any.
fn because(labels: &Labels, sentence: &str, code: Option<&str>) -> String {
    match code.and_then(|code| outcome_short(labels, code)) {
        Some(short) => fill(labels.launcher_because, &[sentence, &short]),
        None => sentence.to_owned(),
    }
}

/// What became of the last request: the plain kind, the technical words, and
/// whether those say a failure.
#[derive(Clone)]
pub struct LauncherLine {
    pub kind: LauncherKind,
    pub technical: Phrase,
    pub failed: bool,
}

impl LauncherLine {
    fn new(kind: LauncherKind, technical: Phrase, failed: bool) -> Self {
        Self { kind, technical, failed }
    }
}

/// What the newest probe said.
enum Status {
    /// This run has no state root: there is nothing to probe.
    Unavailable,
    /// Out, and not answered yet.
    Checking,
    Probed(Result<RuntimeFacts, ClientFailure>),
    /// It could not run: the OS error of its thread.
    Failed(String),
}

/// The launcher as the windows paint it. Workers change it only through jobs
/// handed to the event loop.
pub struct LauncherView {
    status: Status,
    /// The newest probe sent out; only its answer is painted, so a slow one
    /// that a newer press overtook says nothing.
    probe_seq: u64,
    line: Option<LauncherLine>,
    unlock_offered: bool,
    unlock_line: Option<Line>,
}

impl LauncherView {
    /// Before the first probe: checking, or with no state root, nothing to
    /// check.
    pub fn new(installed: bool) -> Self {
        Self {
            status: if installed { Status::Checking } else { Status::Unavailable },
            probe_seq: 0,
            line: None,
            unlock_offered: false,
            unlock_line: None,
        }
    }

    /// Whether the newest probe that answered found a Runtime up.
    pub fn runtime_ready(&self) -> bool {
        matches!(self.status, Status::Probed(Ok(_)))
    }

    /// A new probe: from now on only its answer counts. `checking` puts the
    /// status back to "checking" meanwhile; otherwise the last answer stays
    /// until this one comes.
    fn begin_probe(&mut self, checking: bool) -> u64 {
        self.probe_seq += 1;
        if checking {
            self.status = Status::Checking;
        }
        self.probe_seq
    }

    fn probed(&mut self, seq: u64, probe: Result<RuntimeFacts, ClientFailure>) {
        if seq == self.probe_seq {
            self.status = Status::Probed(probe);
        }
    }

    fn probe_failed(&mut self, seq: u64, error: String) {
        if seq == self.probe_seq {
            self.status = Status::Failed(error);
        }
    }

    /// The status in plain words, for the workbench, and whether they say a
    /// failure.
    fn status_plain(&self, labels: &Labels) -> (String, bool) {
        let kind = match &self.status {
            Status::Unavailable => LauncherKind::NotConfigured,
            Status::Checking => LauncherKind::Checking,
            Status::Probed(Ok(_)) => LauncherKind::Running,
            Status::Probed(Err(_)) => LauncherKind::NotRunning,
            Status::Failed(_) => LauncherKind::CheckFailed,
        };
        (kind.human(labels), kind.failed())
    }

    /// The status with its pid and owner epoch, or the client's code, for the
    /// ledger windows.
    fn status_text(&self, labels: &Labels) -> String {
        match &self.status {
            Status::Unavailable => labels.launcher_not_configured.to_owned(),
            Status::Checking => labels.launcher_checking.to_owned(),
            Status::Probed(probe) => probe_text(labels, probe),
            Status::Failed(error) => fill(labels.thread_failed, &[error]),
        }
    }

    /// What became of the last request in plain words, for the workbench;
    /// nothing before the first. A failure only the technical words say is
    /// pointed at where they are. Red when it says a failure.
    fn line_plain(&self, labels: &Labels) -> (String, bool) {
        let Some(line) = &self.line else {
            return (String::new(), false);
        };
        let mut text = line.kind.human(labels);
        if line.failed && !line.kind.failed() {
            text.push_str(" · ");
            text.push_str(labels.launcher_see_ledger);
        }
        (text, line.failed || line.kind.failed())
    }

    /// The last request's technical words, for the ledger windows; nothing
    /// before the first, or when they have nothing to add. Red when the
    /// request failed.
    fn line_technical(&self, labels: &Labels) -> (String, bool) {
        match &self.line {
            Some(line) => ((line.technical)(labels), line.failed || line.kind.failed()),
            None => (String::new(), false),
        }
    }
}

/// The launcher as the view stands, in the language shown now: the plain
/// lines on the workbench; the technical ones and the unlock entry in every
/// ledger window, with the reopen offer that follows whether a Runtime is up.
pub fn paint(ui: &Ui) {
    let labels = ui.labels();
    let view = ui.launcher_view.borrow();
    let (status, status_failed) = view.status_plain(labels);
    let (line, line_failed) = view.line_plain(labels);
    let technical_status = view.status_text(labels);
    let (technical, technical_failed) = view.line_technical(labels);
    let unlock_offered = view.unlock_offered;
    let (unlock, unlock_failed) = match &view.unlock_line {
        Some(line) => ((line.text)(labels), line.failed),
        None => (String::new(), false),
    };
    drop(view);
    if let Some(window) = ui.workbench() {
        window.set_status_text(status.into());
        window.set_status_failed(status_failed);
        window.set_launcher_line(line.into());
        window.set_launcher_failed(line_failed);
    }
    for handle in ui.ledgers() {
        let window = &handle.window;
        window.set_runtime_status_text(technical_status.as_str().into());
        window.set_launcher_line(technical.as_str().into());
        window.set_launcher_failed(technical_failed);
        window.set_unlock_offered(unlock_offered);
        window.set_unlock_line(unlock.as_str().into());
        window.set_unlock_failed(unlock_failed);
        handle.paint_reopen(ui);
    }
}

/// Every ledger window's unlock entry back at its first step.
fn unconfirm(ui: &Ui) {
    for handle in ui.ledgers() {
        handle.window.set_unlock_confirming(false);
    }
}

/// One probe, on a thread of its own; the windows say it is checking until
/// the answer comes. With no state root there is nothing to probe.
pub fn refresh_status(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    let seq = ui.launcher_view.borrow_mut().begin_probe(true);
    let root = launcher.state_root.clone();
    let probed = spawn_worker("acui-probe", move || {
        let probe = probe_runtime(&root);
        post("acui-probe", move |view| view.probed(seq, probe));
    });
    if let Err(error) = probed {
        ui.launcher_view.borrow_mut().probe_failed(seq, error.to_string());
    }
    paint(ui);
}

fn probe_text(labels: &Labels, probe: &Result<RuntimeFacts, ClientFailure>) -> String {
    match probe {
        Ok(facts) => fill(
            labels.runtime_running,
            &[&facts.pid.to_string(), &facts.owner_epoch],
        ),
        Err(failure) => fill(
            labels.runtime_not_running,
            &[failure.code, failure.operation],
        ),
    }
}

/// A start press on the workbench, or the start after an unlock. The event
/// loop only checks that no unlock and no other start runs, marks this one as
/// running and says it is checking; the whole start runs on `acui-start`. With
/// no state root the button is off and nothing starts.
pub fn start(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    if launcher.unlocking.load(Ordering::SeqCst) {
        let busy = phrase(|labels| labels.unlock_busy.to_owned());
        ui.launcher_view.borrow_mut().line = Some(LauncherLine::new(LauncherKind::Busy, busy, false));
        paint(ui);
        return;
    }
    if launcher.starting.swap(true, Ordering::SeqCst) {
        let busy = phrase(|labels| labels.start_busy.to_owned());
        ui.launcher_view.borrow_mut().line = Some(LauncherLine::new(LauncherKind::Busy, busy, false));
        paint(ui);
        return;
    }
    unconfirm(ui);
    let seq = {
        let mut view = ui.launcher_view.borrow_mut();
        view.unlock_offered = false;
        view.unlock_line = None;
        let nothing = phrase(|_| String::new());
        view.line = Some(LauncherLine::new(LauncherKind::Checking, nothing, false));
        view.begin_probe(true)
    };
    let job = StartJob {
        state_root: launcher.state_root.clone(),
        actingd_exe: launcher.actingd_exe.clone(),
        actingd_config: launcher.actingd_config.clone(),
        installation: launcher.installation.clone(),
        starting: Arc::clone(&launcher.starting),
        probe_seq: seq,
    };
    let started = spawn_worker("acui-start", move || run_start(job));
    // Nothing ran: nothing was probed, launched or recorded.
    if let Err(error) = started {
        launcher.starting.store(false, Ordering::SeqCst);
        let error = error.to_string();
        let mut view = ui.launcher_view.borrow_mut();
        view.probe_failed(seq, error.clone());
        let technical = phrase(move |labels| fill(labels.thread_failed, &[&error]));
        let kind = LauncherKind::StartFailed { code: None };
        view.line = Some(LauncherLine::new(kind, technical, true));
    }
    paint(ui);
}

/// What one start press hands its thread: what it starts from, and the probe
/// its answers count for.
struct StartJob {
    state_root: PathBuf,
    actingd_exe: Option<PathBuf>,
    actingd_config: Option<PathBuf>,
    installation: Option<Snapshot>,
    starting: Arc<AtomicBool>,
    probe_seq: u64,
}

/// The whole start, on `acui-start`: probe; already running, record the press;
/// otherwise check the two paths, make the log, launch, and poll for readiness.
/// Its last word clears `starting`, so no start press lands between that line
/// and the unlock offer.
fn run_start(job: StartJob) {
    let StartJob { state_root, actingd_exe, actingd_config, installation, starting, probe_seq } =
        job;
    // The last word of this start, painted with `starting` cleared.
    let finish = |line: LauncherLine, facts: Option<RuntimeFacts>, offer_unlock: bool| {
        let starting = Arc::clone(&starting);
        post("acui-start", move |view| {
            starting.store(false, Ordering::SeqCst);
            if let Some(facts) = facts {
                view.probed(probe_seq, Ok(facts));
            }
            view.line = Some(line);
            view.unlock_offered = offer_unlock;
        });
    };
    let probe = probe_runtime(&state_root);
    let running = probe.is_ok();
    post("acui-start", move |view| view.probed(probe_seq, probe));
    if running {
        let already = phrase(|labels| labels.already_running.to_owned());
        let line = LauncherLine::new(LauncherKind::Running, already, false);
        post("acui-start", move |view| view.line = Some(line));
        let record = record_start(&state_root, false);
        let failed = record.is_err();
        let record = record_phrase(record);
        let technical =
            phrase(move |labels| format!("{} · {}", labels.already_running, record(labels)));
        finish(LauncherLine::new(LauncherKind::Running, technical, failed), None, false);
        return;
    }
    let (exe, config) = match actingd_paths(&actingd_exe, &actingd_config) {
        Ok(paths) => paths,
        Err(technical) => {
            finish(LauncherLine::new(LauncherKind::NotConfigured, technical, true), None, false);
            return;
        }
    };
    let failed_start = |technical: Phrase| {
        LauncherLine::new(LauncherKind::StartFailed { code: None }, technical, true)
    };
    let log = match log_path() {
        Ok(log) => log,
        Err(technical) => {
            finish(failed_start(technical), None, false);
            return;
        }
    };
    let mut child = match spawn(exe, config, &log, installation.as_ref()) {
        Ok(child) => child,
        Err(error) => {
            let (exe, error) = (exe.display().to_string(), error.to_string());
            let technical = phrase(move |labels| fill(labels.spawn_failed, &[&exe, &error]));
            finish(failed_start(technical), None, false);
            return;
        }
    };
    let pid = child.id();
    let log_text = log.display().to_string();
    let waiting = |attempt: u32| {
        let log_text = log_text.clone();
        let technical = phrase(move |labels| {
            let (pid, attempt) = (pid.to_string(), attempt.to_string());
            fill(labels.start_waiting, &[&pid, &attempt, &READY_ATTEMPTS.to_string(), &log_text])
        });
        let line = LauncherLine::new(LauncherKind::Starting, technical, false);
        post("acui-start", move |view| view.line = Some(line));
    };
    waiting(0);

    let mut last: Option<ClientFailure> = None;
    let mut outcome: Option<(LauncherLine, Option<RuntimeFacts>, bool)> = None;
    for attempt in 1..=READY_ATTEMPTS {
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => {
                let exit = status.code();
                let (said, code) = match std::fs::read(&log).map(|bytes| last_outcome(&bytes)) {
                    Ok(Some((line, code))) => (verbatim(line), Some(code)),
                    Ok(None) => (phrase(|labels| labels.log_no_outcome.to_owned()), None),
                    Err(error) => {
                        let error = error.to_string();
                        (phrase(move |labels| fill(labels.log_unreadable, &[&error])), None)
                    }
                };
                let offer_unlock = code.as_deref() == Some(OWNER_RESOURCE_UNCONFIRMED);
                let kind = if offer_unlock {
                    LauncherKind::NeedsUnlock
                } else {
                    LauncherKind::StartFailed { code }
                };
                let log_text = log_text.clone();
                let technical = phrase(move |labels| {
                    fill(labels.start_exited, &[&exit_text(labels, exit), &said(labels), &log_text])
                });
                outcome = Some((LauncherLine::new(kind, technical, true), None, offer_unlock));
                break;
            }
            Err(error) => {
                let error = error.to_string();
                let technical =
                    phrase(move |labels| fill(labels.child_status_failed, &[&error]));
                outcome = Some((failed_start(technical), None, false));
                break;
            }
        }
        match probe_runtime(&state_root) {
            Ok(facts) => {
                let (pid, epoch) = (facts.pid.to_string(), facts.owner_epoch.clone());
                let record = record_start(&state_root, true);
                let failed = record.is_err();
                let record = record_phrase(record);
                let technical = phrase(move |labels| {
                    format!("{} · {}", fill(labels.start_ready, &[&pid, &epoch]), record(labels))
                });
                let line = LauncherLine::new(LauncherKind::Running, technical, failed);
                outcome = Some((line, Some(facts), false));
                break;
            }
            Err(failure) => last = Some(failure),
        }
        waiting(attempt);
        std::thread::sleep(READY_INTERVAL);
    }
    let (line, facts, offer_unlock) = outcome.unwrap_or_else(|| {
        let code = last.as_ref().map(|failure| failure.code);
        let operation = last.as_ref().map(|failure| failure.operation);
        let technical = phrase(move |labels| {
            let attempts = READY_ATTEMPTS.to_string();
            let (code, operation) = (code.unwrap_or(labels.none), operation.unwrap_or(labels.none));
            fill(labels.start_not_ready, &[&attempts, code, operation])
        });
        (failed_start(technical), None, false)
    });
    finish(line, facts, offer_unlock);
    // `child` is dropped here: not killed, not waited on.
}

/// Both configured paths, or the words naming the first one that is missing
/// or not absolute.
fn actingd_paths<'a>(
    actingd_exe: &'a Option<PathBuf>,
    actingd_config: &'a Option<PathBuf>,
) -> Result<(&'a Path, &'a Path), Phrase> {
    Ok((configured("actingd_exe", actingd_exe)?, configured("actingd_config", actingd_config)?))
}

/// actingd's terminal outcome in `output`: its last `OUTCOME actingd:` line that
/// holds an `actingcommand.outcome.v1` envelope with a code (a line that does
/// not is passed over for an earlier one). Gives the line to show — the
/// developer line written just before it, else the outcome line as written —
/// and the outcome's code.
fn last_outcome(output: &[u8]) -> Option<(String, String)> {
    let text = String::from_utf8_lossy(output);
    let lines: Vec<&str> = text.lines().collect();
    for index in (0..lines.len()).rev() {
        let Some(envelope) = lines[index].strip_prefix(OUTCOME_PREFIX) else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<Value>(envelope) else {
            continue;
        };
        if envelope.pointer("/schema_version").and_then(Value::as_str) != Some(OUTCOME_SCHEMA) {
            continue;
        }
        let Some(code) = envelope.pointer("/code").and_then(Value::as_str) else {
            continue;
        };
        let developer = index
            .checked_sub(1)
            .map(|before| lines[before])
            .filter(|before| DEVELOPER_PREFIXES.iter().any(|level| before.starts_with(*level)));
        return Some((developer.unwrap_or(lines[index]).to_owned(), code.to_owned()));
    }
    None
}

/// The line to show from a subcommand's stderr when its stdout holds no report.
/// A subcommand writes no `OUTCOME actingd:` line, only its developer line
/// (`<LEVEL> actingd: <code> <text>`, Runtime contracts/actingd-unlock-owner.md):
/// the last of those, else the last line that is not blank, so a panic still shows.
fn last_stderr_line(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .rev()
        .find(|line| DEVELOPER_PREFIXES.iter().any(|level| line.starts_with(*level)))
        .or_else(|| text.lines().rev().find(|line| !line.trim().is_empty()))
        .map(str::to_owned)
}

/// A configured key, checked to be an absolute path; the words to show otherwise.
fn configured<'a>(key: &'static str, value: &'a Option<PathBuf>) -> Result<&'a Path, Phrase> {
    match value {
        None => Err(phrase(move |labels| fill(labels.key_not_configured, &[key]))),
        Some(path) if !path.is_absolute() => {
            let shown = path.display().to_string();
            Err(phrase(move |labels| fill(labels.key_not_absolute, &[key, &shown])))
        }
        Some(path) => Ok(path),
    }
}

/// `actingd-<unix_ms>.log` in the console's log directory (`report::logs_dir`:
/// `%LOCALAPPDATA%\ActingCommand\logs` on Windows), beside its own technical
/// log. The directory is created here; the file is created by `spawn`.
fn log_path() -> Result<PathBuf, Phrase> {
    let dir = report::logs_dir()
        .map_err(|names| phrase(move |labels| fill(labels.log_dir_failed, &[names, labels.none])))?;
    std::fs::create_dir_all(&dir).map_err(|error| {
        let (shown, error) = (dir.display().to_string(), error.to_string());
        phrase(move |labels| fill(labels.log_dir_failed, &[&shown, &error]))
    })?;
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    Ok(dir.join(format!("actingd-{unix_ms}.log")))
}

/// Exactly `actingcommand-actingd --config <actingd_config>`, detached, with
/// both output streams in the log file. Nothing is inherited from the console
/// but the environment.
fn spawn(exe: &Path, config: &Path, log: &Path, installation: Option<&Snapshot>) -> std::io::Result<Child> {
    let stdout = File::create(log)?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new(exe);
    if let Some(snapshot) = installation { snapshot.apply_to(&mut command).map_err(std::io::Error::other)?; }
    command
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(DETACHED_PROCESS);
    }
    command.spawn()
}

/// The confirming press, in a ledger window. The entry is withdrawn in every
/// ledger window while unlock-owner runs, back at its first step after any
/// outcome but an unlock, and gone after one; an unlock starts once more, as a
/// start press does. The workbench does not close meanwhile.
pub fn unlock(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    unconfirm(ui);
    let (exe, config) = match actingd_paths(&launcher.actingd_exe, &launcher.actingd_config) {
        Ok((exe, config)) => (exe.to_path_buf(), config.to_path_buf()),
        Err(technical) => {
            ui.launcher_view.borrow_mut().unlock_line = Some(Line::failed(technical));
            paint(ui);
            return;
        }
    };
    launcher.unlocking.store(true, Ordering::SeqCst);
    {
        let mut view = ui.launcher_view.borrow_mut();
        view.unlock_offered = false;
        let running = phrase(|labels| fill(labels.unlock_running, &[UNLOCK_ACTOR]));
        view.unlock_line = Some(Line::said(running));
    }
    let unlocking = Arc::clone(&launcher.unlocking);
    let installation = launcher.installation.clone();
    let ran = spawn_worker("acui-unlock", move || {
        let (text, unlocked) = unlock_outcome(&exe, &config, installation.as_ref());
        report::post("acui-unlock", move || {
            unlocking.store(false, Ordering::SeqCst);
            let Some(ui) = crate::ui::current() else {
                return;
            };
            // The start paints the launcher itself; nothing of the view is
            // held meanwhile.
            if unlocked {
                start(&ui);
            }
            {
                let mut view = ui.launcher_view.borrow_mut();
                if !unlocked {
                    view.unlock_offered = true;
                }
                // After the start, which clears the unlock line.
                view.unlock_line = Some(Line { text, failed: !unlocked });
            }
            // The workbench may close again.
            ui.close_refused.set(false);
            ui.paint_notices();
            paint(&ui);
        });
    });
    // Nothing ran: the entry goes back to its first step, with the reason.
    if let Err(error) = ran {
        launcher.unlocking.store(false, Ordering::SeqCst);
        let error = error.to_string();
        let mut view = ui.launcher_view.borrow_mut();
        view.unlock_offered = true;
        let failed = phrase(move |labels| fill(labels.thread_failed, &[&error]));
        view.unlock_line = Some(Line::failed(failed));
    }
    paint(ui);
}

/// The words unlock-owner's answer comes to, and whether it unlocked: only an
/// `unlock_owner_completed` report with exit code 0 did.
fn unlock_outcome(exe: &Path, config: &Path, installation: Option<&Snapshot>) -> (Phrase, bool) {
    let (status, stdout, stderr) = match run_unlock(exe, config, installation) {
        Ok(output) => output,
        Err(text) => return (text, false),
    };
    let exit = status.code();
    let stdout = stdout.trim_ascii();
    if stdout.is_empty() {
        let text = match last_stderr_line(&stderr) {
            Some(line) => {
                phrase(move |labels| fill(labels.unlock_stderr, &[&exit_text(labels, exit), &line]))
            }
            None => phrase(move |labels| {
                fill(labels.unlock_unparseable, &[labels.unlock_no_output, &exit_text(labels, exit)])
            }),
        };
        return (text, false);
    }
    let report = serde_json::from_slice::<Value>(stdout).map_err(|error| verbatim(error.to_string()));
    match report.and_then(|report| unlock_report(&report, exit, status.success())) {
        Ok(outcome) => outcome,
        Err(why) => {
            let text = phrase(move |labels| {
                fill(labels.unlock_unparseable, &[&why(labels), &exit_text(labels, exit)])
            });
            (text, false)
        }
    }
}

/// The v2 report read field by field (`outcome` is the full
/// `actingcommand.outcome.v1` envelope); `Err` names the first field that is
/// missing or not as the contract states. A failure's stage and
/// `journal_appended` may be absent and are not such a field.
fn unlock_report(
    report: &Value,
    exit: Option<i32>,
    success: bool,
) -> Result<(Phrase, bool), Phrase> {
    let invalid = |pointer: &'static str| {
        phrase(move |labels| fill(labels.unlock_field_invalid, &[pointer]))
    };
    let text = |pointer: &'static str| {
        report
            .pointer(pointer)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(pointer))
    };
    if text("/schema_version")? != UNLOCK_SCHEMA {
        return Err(invalid("/schema_version"));
    }
    let code = text("/outcome/code")?;
    if code == UNLOCK_COMPLETED {
        let epoch = text("/owner_epoch")?.to_owned();
        let disposition = text("/previous_resource_disposition")?.to_owned();
        let revision = report
            .pointer("/revision")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("/revision"))?
            .to_string();
        return Ok(if success {
            let text =
                phrase(move |labels| fill(labels.unlock_ok, &[&epoch, &disposition, &revision]));
            (text, true)
        } else {
            let text = phrase(move |labels| {
                let exit = exit_text(labels, exit);
                fill(labels.unlock_ok_nonzero, &[&exit, &epoch, &disposition, &revision])
            });
            (text, false)
        });
    }
    // A failure refused before the unlock itself (usage, options, the
    // installation check, a report that could not be encoded) carries neither
    // a stage nor `journal_appended`: those show as none, beside its own code.
    let code = code.to_owned();
    let stage = report
        .pointer("/outcome/values/stage")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let appended = report.pointer("/journal_appended").and_then(Value::as_bool);
    let text = phrase(move |labels| {
        let stage = stage.as_deref().unwrap_or(labels.none);
        let appended =
            appended.map_or_else(|| labels.none.to_string(), |appended| appended.to_string());
        fill(labels.unlock_failed, &[&code, stage, &appended, &exit_text(labels, exit)])
    });
    Ok((text, false))
}

/// Exactly `<actingd_exe> unlock-owner --config <actingd_config> --actor acui
/// --confirm-resources-released`: its exit status and both output streams, or
/// the words for why there are none.
fn run_unlock(
    exe: &Path,
    config: &Path,
    installation: Option<&Snapshot>,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), Phrase> {
    let mut command = Command::new(exe);
    if let Some(snapshot) = installation { snapshot.apply_to(&mut command).map_err(verbatim)?; }
    command
        .arg("unlock-owner")
        .arg("--config")
        .arg(config)
        .args(["--actor", UNLOCK_ACTOR, "--confirm-resources-released"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| {
        let (shown, error) = (exe.display().to_string(), error.to_string());
        phrase(move |labels| fill(labels.unlock_spawn_failed, &[&shown, &error]))
    })?;
    // Drained while it runs, so a full pipe can never stall it.
    let drains =
        drain(child.stdout.take()).and_then(|stdout| Ok((stdout, drain(child.stderr.take())?)));
    let (stdout, stderr) = match drains {
        Ok(drains) => drains,
        Err(error) => {
            let reaped = reap(&mut child);
            let error = error.to_string();
            return Err(phrase(move |labels| {
                let why = fill(labels.unlock_output_failed, &[&error, labels.none]);
                format!("{why} · {}", reaped(labels))
            }));
        }
    };
    let deadline = Instant::now() + UNLOCK_TIMEOUT;
    let stopped = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(UNLOCK_POLL),
            Ok(None) => {
                break Err(phrase(|labels| {
                    fill(labels.unlock_timeout, &[&UNLOCK_TIMEOUT.as_secs().to_string()])
                }));
            }
            Err(error) => {
                let error = error.to_string();
                break Err(phrase(move |labels| fill(labels.child_status_failed, &[&error])));
            }
        }
    };
    let status = match stopped {
        Ok(status) => status,
        Err(text) => {
            let reaped = reap(&mut child);
            return Err(phrase(move |labels| format!("{} · {}", text(labels), reaped(labels))));
        }
    };
    // The exit code stays on the line: by the contract, 0 alone says the unlock took effect.
    let exit = status.code();
    let output = |pipe: JoinHandle<io::Result<Vec<u8>>>| match pipe.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => {
            let error = error.to_string();
            Err(phrase(move |labels| {
                fill(labels.unlock_output_failed, &[&error, &exit_text(labels, exit)])
            }))
        }
        Err(_) => Err(phrase(move |labels| {
            fill(labels.unlock_output_failed, &[labels.none, &exit_text(labels, exit)])
        })),
    };
    Ok((status, output(stdout)?, output(stderr)?))
}

/// Kills and reaps the child. A kill that fails leaves it unwaited: it may
/// never exit. A wait that fails after a kill is said as that, not as a kill
/// that failed.
fn reap(child: &mut Child) -> Phrase {
    match child.kill() {
        Err(error) => {
            let error = error.to_string();
            phrase(move |labels| fill(labels.unlock_kill_failed, &[&error]))
        }
        Ok(()) => match child.wait() {
            Ok(_) => phrase(|labels| labels.unlock_killed.to_owned()),
            Err(error) => {
                let error = error.to_string();
                phrase(move |labels| fill(labels.unlock_unreaped, &[&error]))
            }
        },
    }
}

/// An exit code as the lines show it: none when the process has none.
fn exit_text(labels: &Labels, exit: Option<i32>) -> String {
    exit.map(|code| code.to_string()).unwrap_or_else(|| labels.none.to_string())
}

/// A reader thread for one pipe; a thread the system refuses is an error to
/// report, never a panic on a detached worker that would leave the unlock stuck.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> io::Result<JoinHandle<io::Result<Vec<u8>>>> {
    std::thread::Builder::new().spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    })
}

/// A shutdown press on the workbench, on `acui-shutdown`; a second press while
/// one runs is refused, so no two requests go out at once. With no state root
/// the button is off and nothing is sent.
pub fn shutdown(ui: &Ui) {
    let Ok(launcher) = &ui.launcher else {
        return;
    };
    if launcher.shutting_down.swap(true, Ordering::SeqCst) {
        let busy = phrase(|labels| labels.shutdown_busy.to_owned());
        ui.launcher_view.borrow_mut().line = Some(LauncherLine::new(LauncherKind::Busy, busy, false));
        paint(ui);
        return;
    }
    let seq = {
        let mut view = ui.launcher_view.borrow_mut();
        let nothing = phrase(|_| String::new());
        view.line = Some(LauncherLine::new(LauncherKind::Stopping, nothing, false));
        view.begin_probe(false)
    };
    let root = launcher.state_root.clone();
    let shutting_down = Arc::clone(&launcher.shutting_down);
    let sent = spawn_worker("acui-shutdown", move || {
        let outcome = request_shutdown(&root, |attempt| {
            let technical = phrase(move |labels| {
                let (attempt, total) = (attempt.to_string(), SHUTDOWN_ATTEMPTS.to_string());
                fill(labels.shutdown_busy_retry, &[&attempt, &total])
            });
            let line = LauncherLine::new(LauncherKind::Stopping, technical, false);
            post("acui-shutdown", move |view| view.line = Some(line));
        });
        let attempts = outcome.attempts;
        let failed = outcome.result.is_err();
        let (kind, said) = match outcome.result {
            Ok(accepted) => {
                let technical = phrase(move |labels| {
                    let sequence = accepted.action_sequence.to_string();
                    let values: [&str; 3] =
                        [&accepted.receipt_state, &accepted.request_id, &sequence];
                    fill(labels.shutdown_accepted, &values)
                });
                (LauncherKind::StopRequested, technical)
            }
            Err(failure) => {
                let code = failure.runtime_code.clone().unwrap_or_else(|| failure.code.to_owned());
                let technical = phrase(move |labels| {
                    failure_text(labels, labels.shutdown_refused, labels.shutdown_failed, &failure)
                });
                (LauncherKind::StopFailed { code: Some(code) }, technical)
            }
        };
        let technical = phrase(move |labels| {
            let mut line = said(labels);
            if attempts > 0 {
                let (attempts, total) = (attempts.to_string(), SHUTDOWN_ATTEMPTS.to_string());
                line.push_str(" · ");
                line.push_str(&fill(labels.shutdown_attempts, &[&attempts, &total]));
            }
            line
        });
        // One probe after the answer: the Runtime stops on its own time, so
        // this may still say running.
        let probe = probe_runtime(&root);
        let line = LauncherLine::new(kind, technical, failed);
        post("acui-shutdown", move |view| {
            shutting_down.store(false, Ordering::SeqCst);
            view.probed(seq, probe);
            view.line = Some(line);
        });
    });
    // Nothing was sent and nothing recorded.
    if let Err(error) = sent {
        launcher.shutting_down.store(false, Ordering::SeqCst);
        let error = error.to_string();
        let technical = phrase(move |labels| fill(labels.thread_failed, &[&error]));
        let kind = LauncherKind::StopFailed { code: None };
        ui.launcher_view.borrow_mut().line = Some(LauncherLine::new(kind, technical, true));
    }
    paint(ui);
}

/// A named worker thread. A thread the system refuses comes back as the error
/// to show, never as a panic on the event loop.
pub fn spawn_worker(name: &str, work: impl FnOnce() + Send + 'static) -> io::Result<()> {
    std::thread::Builder::new().name(name.to_owned()).spawn(work).map(drop)
}

/// Where the start press landed in the ledger, or why it did not.
fn record_phrase(record: Result<u64, ClientFailure>) -> Phrase {
    match record {
        Ok(sequence) => phrase(move |labels| fill(labels.start_recorded, &[&sequence.to_string()])),
        Err(failure) => phrase(move |labels| {
            failure_text(labels, labels.start_record_refused, labels.start_record_failed, &failure)
        }),
    }
}

/// The Runtime's refusal code verbatim when it answered with one, then the
/// client's own error code and operation, then the host failure behind the
/// refusal when the receipt names one; last, the short words of the refusal
/// code, or of the client's code when there is no refusal.
fn failure_text(labels: &Labels, refused: &str, failed: &str, failure: &ClientFailure) -> String {
    let mut text = match &failure.runtime_code {
        Some(runtime_code) => fill(refused, &[runtime_code, failure.code, failure.operation]),
        None => fill(failed, &[failure.code, failure.operation]),
    };
    if let Some((host_code, operation)) = &failure.host {
        text.push_str(&fill(labels.host_failure, &[host_code, operation]));
    }
    let code = failure.runtime_code.as_deref().unwrap_or(failure.code);
    if let Some(short) = outcome_short(labels, code) {
        text.push_str(" · ");
        text.push_str(&short);
    }
    text
}

/// Hands a worker's news to the event loop: it goes into the launcher's view,
/// and every window paints the launcher again in the language shown at that
/// moment.
fn post(what: &str, update: impl FnOnce(&mut LauncherView) + Send + 'static) {
    report::post(what, move || {
        let Some(ui) = crate::ui::current() else {
            return;
        };
        update(&mut *ui.launcher_view.borrow_mut());
        paint(&ui);
    });
}
