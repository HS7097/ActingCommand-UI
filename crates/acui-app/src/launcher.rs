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
//! the client, recorded as a client action first; a refusal is shown verbatim
//! and never retried.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use acui_source::{probe_runtime, request_shutdown, ClientFailure, RuntimeFacts};
use slint::ComponentHandle;

use crate::strings::{fill, Labels};
use crate::{App, AppWindow};

/// Readiness is polled: up to this many connect attempts, this far apart.
const READY_ATTEMPTS: u32 = 60;
const READY_INTERVAL: Duration = Duration::from_millis(500);

/// Windows `DETACHED_PROCESS`: the daemon gets no console and does not inherit
/// the console's own, so it outlives the console and never sees its Ctrl+C.
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

pub struct Launcher {
    state_root: PathBuf,
    actingd_config: Option<PathBuf>,
    actingd_exe: Option<PathBuf>,
    /// One start in flight at a time: set until its readiness poll ends.
    starting: Arc<AtomicBool>,
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
    if launcher.starting.load(Ordering::SeqCst) {
        window.set_launcher_line(labels.start_busy.into());
        return;
    }
    let probe = probe_runtime(&launcher.state_root);
    window.set_runtime_status_text(status_text(labels, &probe).into());
    if probe.is_ok() {
        window.set_launcher_line(labels.already_running.into());
        return;
    }
    let (exe, config) = match (
        configured(labels, "actingd_exe", &launcher.actingd_exe),
        configured(labels, "actingd_config", &launcher.actingd_config),
    ) {
        (Ok(exe), Ok(config)) => (exe, config),
        (Err(text), _) | (_, Err(text)) => {
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
        for attempt in 1..=READY_ATTEMPTS {
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    let exit = status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| labels.none.to_string());
                    outcome = Some((None, fill(labels.start_exited, &[&exit, &log_text])));
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
        starting.store(false, Ordering::SeqCst);
        post(&weak, status, line);
        // `child` is dropped here: not killed, not waited on.
    });
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

fn shutdown(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    let root = app.launcher.state_root.clone();
    let weak = window.as_weak();
    window.set_launcher_line(format!("{}…", labels.request_shutdown).into());
    std::thread::spawn(move || {
        let line = match request_shutdown(&root) {
            Ok(accepted) => fill(
                labels.shutdown_accepted,
                &[
                    &accepted.receipt_state,
                    &accepted.request_id,
                    &accepted.action_sequence.to_string(),
                ],
            ),
            Err(failure) => match &failure.runtime_code {
                Some(runtime_code) => fill(
                    labels.shutdown_refused,
                    &[runtime_code, failure.code, failure.operation],
                ),
                None => fill(labels.shutdown_failed, &[failure.code, failure.operation]),
            },
        };
        // One probe after the answer: the Runtime stops on its own time, so
        // this may still say running.
        let status = status_text(labels, &probe_runtime(&root));
        post(&weak, Some(status), line);
    });
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
