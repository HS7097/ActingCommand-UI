// SPDX-License-Identifier: GPL-3.0-only
//! The launcher block: the one place the console starts a process, and the
//! one place it asks the Runtime to stop.
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
//! After an early exit the log is read back once, for its last `FATAL actingd:`
//! line, shown as written. When that line names `owner_resource_unconfirmed`
//! the block offers `actingd unlock-owner`, run only on a second, confirming
//! press: no console window, output captured, killed and reaped if it outlives
//! its timeout. It appends its own ledger fact, so no client action is recorded
//! for it; an `ok` with exit code 0 presses start once more.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use acui_source::{
    probe_runtime, record_start, request_shutdown, ClientFailure, RuntimeFacts, SHUTDOWN_ATTEMPTS,
};
use serde_json::Value;
use slint::{ComponentHandle, SharedString};

use crate::strings::{fill, Labels};
use crate::{App, AppWindow};

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

const FATAL_PREFIX: &str = "FATAL actingd:";
/// The one code in a fatal line the block answers, with the unlock entry.
const OWNER_RESOURCE_UNCONFIRMED: &str = "owner_resource_unconfirmed";
/// Names the surface, not a person: no user name goes into the ledger.
const UNLOCK_ACTOR: &str = "acui";
const UNLOCK_SCHEMA: &str = "actingcommand.actingd.unlock-owner.v1";
/// After its journal append the command opens the ledger and recovers the
/// exited owner's writer within the Runtime's own 120 s maintenance budget;
/// killing it inside that window would leave the journal record without its
/// ledger fact, so this bound sits well above it.
const UNLOCK_TIMEOUT: Duration = Duration::from_secs(180);
const UNLOCK_POLL: Duration = Duration::from_millis(100);

pub struct Launcher {
    state_root: PathBuf,
    actingd_config: Option<PathBuf>,
    actingd_exe: Option<PathBuf>,
    /// One start in flight at a time: set until its readiness poll ends.
    starting: Arc<AtomicBool>,
    /// Set while unlock-owner runs; a start is refused meanwhile.
    unlocking: Arc<AtomicBool>,
}

impl Launcher {
    pub fn new(
        state_root: PathBuf,
        actingd_config: Option<PathBuf>,
        actingd_exe: Option<PathBuf>,
    ) -> Self {
        Self {
            state_root,
            actingd_config,
            actingd_exe,
            starting: Arc::new(AtomicBool::new(false)),
            unlocking: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub fn install(window: &AppWindow, app: &Rc<App>) {
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_start_runtime(move || {
            if let Some(window) = weak.upgrade() {
                start(&window, &app);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_request_shutdown(move || {
            if let Some(window) = weak.upgrade() {
                shutdown(&window, &app);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_unlock_owner(move || {
            if let Some(window) = weak.upgrade() {
                unlock(&window, &app);
            }
        });
    }
}

/// One probe, painted on the status text of the block.
pub fn refresh_status(window: &AppWindow, app: &App) {
    let probe = probe_runtime(&app.launcher.state_root);
    window.set_runtime_status_text(status_text(app.labels, &probe).into());
}

fn status_text(labels: &Labels, probe: &Result<RuntimeFacts, ClientFailure>) -> String {
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

fn start(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    let launcher = &app.launcher;
    if launcher.unlocking.load(Ordering::SeqCst) {
        window.set_launcher_line(labels.unlock_busy.into());
        return;
    }
    if launcher.starting.load(Ordering::SeqCst) {
        window.set_launcher_line(labels.start_busy.into());
        return;
    }
    window.set_unlock_offered(false);
    window.set_unlock_confirming(false);
    window.set_unlock_line(SharedString::new());
    let probe = probe_runtime(&launcher.state_root);
    window.set_runtime_status_text(status_text(labels, &probe).into());
    if probe.is_ok() {
        window.set_launcher_line(labels.already_running.into());
        let root = launcher.state_root.clone();
        let weak = window.as_weak();
        std::thread::spawn(move || {
            let record = record_text(labels, record_start(&root, false));
            let line = format!("{} · {record}", labels.already_running);
            post(&weak, None, line);
        });
        return;
    }
    let (exe, config) = match actingd_paths(labels, launcher) {
        Ok(paths) => paths,
        Err(text) => {
            window.set_launcher_line(text.into());
            return;
        }
    };
    let log = match log_path(labels) {
        Ok(log) => log,
        Err(text) => {
            window.set_launcher_line(text.into());
            return;
        }
    };
    let child = match spawn(exe, config, &log) {
        Ok(child) => child,
        Err(error) => {
            window.set_launcher_line(
                fill(
                    labels.spawn_failed,
                    &[&exe.display().to_string(), &error.to_string()],
                )
                .into(),
            );
            return;
        }
    };
    let pid = child.id();
    let log_text = log.display().to_string();
    window.set_launcher_line(
        fill(
            labels.start_waiting,
            &[
                &pid.to_string(),
                "0",
                &READY_ATTEMPTS.to_string(),
                &log_text,
            ],
        )
        .into(),
    );

    launcher.starting.store(true, Ordering::SeqCst);
    let starting = Arc::clone(&launcher.starting);
    let root = launcher.state_root.clone();
    let offline = app.source.offline_reason().is_some();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let mut child = child;
        let mut last: Option<ClientFailure> = None;
        let mut outcome: Option<(Option<String>, String)> = None;
        let mut offer_unlock = false;
        for attempt in 1..=READY_ATTEMPTS {
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    let exit = status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| labels.none.to_string());
                    let said = match std::fs::read(&log).map(|bytes| last_fatal(&bytes)) {
                        Ok(Some(line)) => {
                            offer_unlock = line
                                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                                .any(|word| word == OWNER_RESOURCE_UNCONFIRMED);
                            line
                        }
                        Ok(None) => labels.log_no_fatal.to_owned(),
                        Err(error) => fill(labels.log_unreadable, &[&error.to_string()]),
                    };
                    outcome = Some((None, fill(labels.start_exited, &[&exit, &said, &log_text])));
                    break;
                }
                Err(error) => {
                    outcome = Some((
                        None,
                        fill(labels.child_status_failed, &[&error.to_string()]),
                    ));
                    break;
                }
            }
            match probe_runtime(&root) {
                Ok(facts) => {
                    let mut line = fill(
                        labels.start_ready,
                        &[&facts.pid.to_string(), &facts.owner_epoch],
                    );
                    line.push_str(" · ");
                    line.push_str(&record_text(labels, record_start(&root, true)));
                    if offline {
                        line.push_str(" · ");
                        line.push_str(labels.restart_online);
                    }
                    outcome = Some((Some(status_text(labels, &Ok(facts))), line));
                    break;
                }
                Err(failure) => last = Some(failure),
            }
            post(
                &weak,
                None,
                fill(
                    labels.start_waiting,
                    &[
                        &pid.to_string(),
                        &attempt.to_string(),
                        &READY_ATTEMPTS.to_string(),
                        &log_text,
                    ],
                ),
            );
            std::thread::sleep(READY_INTERVAL);
        }
        let (status, line) = outcome.unwrap_or_else(|| {
            let (code, operation) = last
                .as_ref()
                .map(|failure| (failure.code, failure.operation))
                .unwrap_or((labels.none, labels.none));
            (
                None,
                fill(
                    labels.start_not_ready,
                    &[&READY_ATTEMPTS.to_string(), code, operation],
                ),
            )
        });
        // One paint, clearing `starting` with it, so no start press lands
        // between the line and the offer.
        let _ = slint::invoke_from_event_loop(move || {
            starting.store(false, Ordering::SeqCst);
            let Some(window) = weak.upgrade() else {
                return;
            };
            if let Some(status) = status {
                window.set_runtime_status_text(status.into());
            }
            window.set_launcher_line(line.into());
            window.set_unlock_offered(offer_unlock);
        });
        // `child` is dropped here: not killed, not waited on.
    });
}

/// Both configured paths, or the text naming the first one that is missing
/// or not absolute.
fn actingd_paths<'a>(
    labels: &Labels,
    launcher: &'a Launcher,
) -> Result<(&'a Path, &'a Path), String> {
    Ok((
        configured(labels, "actingd_exe", &launcher.actingd_exe)?,
        configured(labels, "actingd_config", &launcher.actingd_config)?,
    ))
}

/// The last line of `output` that starts with `FATAL actingd:`.
fn last_fatal(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .lines()
        .rev()
        .find(|line| line.starts_with(FATAL_PREFIX))
        .map(str::to_owned)
}

/// A configured key, checked to be an absolute path; the text to show otherwise.
fn configured<'a>(
    labels: &Labels,
    key: &str,
    value: &'a Option<PathBuf>,
) -> Result<&'a Path, String> {
    match value {
        None => Err(fill(labels.key_not_configured, &[key])),
        Some(path) if !path.is_absolute() => Err(fill(
            labels.key_not_absolute,
            &[key, &path.display().to_string()],
        )),
        Some(path) => Ok(path),
    }
}

/// `%LOCALAPPDATA%\ActingCommand\logs\actingd-<unix_ms>.log` on Windows,
/// `$XDG_STATE_HOME` (or `$HOME/.local/state`) elsewhere. The directory is
/// created here; the file is created by `spawn`.
fn log_path(labels: &Labels) -> Result<PathBuf, String> {
    let (base, name) = if cfg!(windows) {
        (
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            "%LOCALAPPDATA%",
        )
    } else {
        (
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| PathBuf::from(home).join(".local").join("state"))
                }),
            "$XDG_STATE_HOME / $HOME",
        )
    };
    let Some(base) = base else {
        return Err(fill(labels.log_dir_failed, &[name, labels.none]));
    };
    let dir = base.join("ActingCommand").join("logs");
    std::fs::create_dir_all(&dir).map_err(|error| {
        fill(
            labels.log_dir_failed,
            &[&dir.display().to_string(), &error.to_string()],
        )
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
fn spawn(exe: &Path, config: &Path, log: &Path) -> std::io::Result<Child> {
    let stdout = File::create(log)?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new(exe);
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

/// The confirming press. The entry is withdrawn while unlock-owner runs, back
/// at its first step after any outcome but an unlock, and gone after one.
fn unlock(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    let launcher = &app.launcher;
    window.set_unlock_confirming(false);
    let (exe, config) = match actingd_paths(labels, launcher) {
        Ok((exe, config)) => (exe.to_path_buf(), config.to_path_buf()),
        Err(text) => {
            window.set_unlock_line(text.into());
            return;
        }
    };
    launcher.unlocking.store(true, Ordering::SeqCst);
    window.set_unlock_offered(false);
    window.set_unlock_line(fill(labels.unlock_running, &[UNLOCK_ACTOR]).into());
    let unlocking = Arc::clone(&launcher.unlocking);
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let (line, unlocked) = unlock_outcome(labels, &exe, &config);
        let _ = slint::invoke_from_event_loop(move || {
            unlocking.store(false, Ordering::SeqCst);
            let Some(window) = weak.upgrade() else {
                return;
            };
            if unlocked {
                window.invoke_start_runtime();
            } else {
                window.set_unlock_offered(true);
            }
            // After the start, which clears the unlock line.
            window.set_unlock_line(line.into());
        });
    });
}

/// The line unlock-owner's answer comes to, and whether it unlocked: only an
/// `ok` report with exit code 0 did.
fn unlock_outcome(labels: &Labels, exe: &Path, config: &Path) -> (String, bool) {
    let (status, stdout, stderr) = match run_unlock(labels, exe, config) {
        Ok(output) => output,
        Err(text) => return (text, false),
    };
    let exit = exit_text(labels, status);
    let stdout = stdout.trim_ascii();
    if stdout.is_empty() {
        let text = match last_fatal(&stderr) {
            Some(line) => fill(labels.unlock_fatal, &[&exit, &line]),
            None => fill(labels.unlock_unparseable, &[labels.unlock_no_output, &exit]),
        };
        return (text, false);
    }
    let report = serde_json::from_slice::<Value>(stdout).map_err(|error| error.to_string());
    match report.and_then(|report| unlock_report(labels, &report, &exit, status.success())) {
        Ok(outcome) => outcome,
        Err(why) => (fill(labels.unlock_unparseable, &[&why, &exit]), false),
    }
}

/// The v1 report read field by field; `Err` names the first field that is
/// missing or not as the contract states.
fn unlock_report(
    labels: &Labels,
    report: &Value,
    exit: &str,
    success: bool,
) -> Result<(String, bool), String> {
    let invalid = |pointer: &str| fill(labels.unlock_field_invalid, &[pointer]);
    let text = |pointer: &str| {
        report
            .pointer(pointer)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(pointer))
    };
    if text("/schema_version")? != UNLOCK_SCHEMA {
        return Err(invalid("/schema_version"));
    }
    match text("/status")? {
        "ok" => {
            let epoch = text("/owner_epoch")?;
            let disposition = text("/previous_resource_disposition")?;
            let revision = report
                .pointer("/revision")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("/revision"))?
                .to_string();
            Ok(if success {
                (
                    fill(labels.unlock_ok, &[epoch, disposition, &revision]),
                    true,
                )
            } else {
                (
                    fill(
                        labels.unlock_ok_nonzero,
                        &[exit, epoch, disposition, &revision],
                    ),
                    false,
                )
            })
        }
        "failed" => {
            let appended = report
                .pointer("/journal_appended")
                .and_then(Value::as_bool)
                .ok_or_else(|| invalid("/journal_appended"))?
                .to_string();
            let values = [text("/error/code")?, text("/error/stage")?, &appended, exit];
            Ok((fill(labels.unlock_failed, &values), false))
        }
        _ => Err(invalid("/status")),
    }
}

/// Exactly `<actingd_exe> unlock-owner --config <actingd_config> --actor acui
/// --confirm-resources-released`: its exit status and both output streams, or
/// the text for why there are none.
fn run_unlock(
    labels: &Labels,
    exe: &Path,
    config: &Path,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), String> {
    let mut command = Command::new(exe);
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
        fill(
            labels.unlock_spawn_failed,
            &[&exe.display().to_string(), &error.to_string()],
        )
    })?;
    // Drained while it runs, so a full pipe can never stall it.
    let drains =
        drain(child.stdout.take()).and_then(|stdout| Ok((stdout, drain(child.stderr.take())?)));
    let (stdout, stderr) = match drains {
        Ok(drains) => drains,
        Err(error) => {
            let reaped = reap(labels, &mut child);
            let why = fill(
                labels.unlock_output_failed,
                &[&error.to_string(), labels.none],
            );
            return Err(format!("{why} · {reaped}"));
        }
    };
    let deadline = Instant::now() + UNLOCK_TIMEOUT;
    let stopped = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(UNLOCK_POLL),
            Ok(None) => {
                let seconds = UNLOCK_TIMEOUT.as_secs().to_string();
                break Err(fill(labels.unlock_timeout, &[&seconds]));
            }
            Err(error) => break Err(fill(labels.child_status_failed, &[&error.to_string()])),
        }
    };
    let status = match stopped {
        Ok(status) => status,
        Err(text) => return Err(format!("{text} · {}", reap(labels, &mut child))),
    };
    // The exit code stays on the line: by the contract, 0 alone says the unlock took effect.
    let exit = exit_text(labels, status);
    let output = |pipe: JoinHandle<io::Result<Vec<u8>>>| match pipe.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(fill(
            labels.unlock_output_failed,
            &[&error.to_string(), &exit],
        )),
        Err(_) => Err(fill(labels.unlock_output_failed, &[labels.none, &exit])),
    };
    Ok((status, output(stdout)?, output(stderr)?))
}

fn reap(labels: &Labels, child: &mut Child) -> String {
    match child.kill().and_then(|()| child.wait()) {
        Ok(_) => labels.unlock_killed.to_owned(),
        Err(error) => fill(labels.unlock_kill_failed, &[&error.to_string()]),
    }
}

fn exit_text(labels: &Labels, status: ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| labels.none.to_string())
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

fn shutdown(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    let root = app.launcher.state_root.clone();
    let weak = window.as_weak();
    window.set_launcher_line(format!("{}…", labels.request_shutdown).into());
    std::thread::spawn(move || {
        let total = SHUTDOWN_ATTEMPTS.to_string();
        let outcome = request_shutdown(&root, |attempt| {
            let line = fill(labels.shutdown_busy_retry, &[&attempt.to_string(), &total]);
            post(&weak, None, line);
        });
        let mut line = match outcome.result {
            Ok(accepted) => fill(
                labels.shutdown_accepted,
                &[
                    &accepted.receipt_state,
                    &accepted.request_id,
                    &accepted.action_sequence.to_string(),
                ],
            ),
            Err(failure) => failure_text(labels.shutdown_refused, labels.shutdown_failed, &failure),
        };
        if outcome.attempts > 0 {
            let attempts = outcome.attempts.to_string();
            line.push_str(" · ");
            line.push_str(&fill(labels.shutdown_attempts, &[&attempts, &total]));
        }
        // One probe after the answer: the Runtime stops on its own time, so
        // this may still say running.
        let status = status_text(labels, &probe_runtime(&root));
        post(&weak, Some(status), line);
    });
}

/// Where the start press landed in the ledger, or why it did not.
fn record_text(labels: &Labels, record: Result<u64, ClientFailure>) -> String {
    let (refused, failed) = (labels.start_record_refused, labels.start_record_failed);
    match record {
        Ok(sequence) => fill(labels.start_recorded, &[&sequence.to_string()]),
        Err(failure) => failure_text(refused, failed, &failure),
    }
}

/// The Runtime's refusal code verbatim when it answered with one, then the
/// client's own error code and operation.
fn failure_text(refused: &str, failed: &str, failure: &ClientFailure) -> String {
    match &failure.runtime_code {
        Some(runtime_code) => fill(refused, &[runtime_code, failure.code, failure.operation]),
        None => fill(failed, &[failure.code, failure.operation]),
    }
}

/// Paints the block from a worker thread.
fn post(weak: &slint::Weak<AppWindow>, status: Option<String>, line: String) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if let Some(status) = status {
            window.set_runtime_status_text(status.into());
        }
        window.set_launcher_line(line.into());
    });
}
