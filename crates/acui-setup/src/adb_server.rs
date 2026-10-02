// SPDX-License-Identifier: GPL-3.0-only
//! After an upgrade, the ADB server (owner ruling A5, Workflow #337): the one
//! adb command the wizard runs, and only here — before and during the
//! upgrade the server is left alone. AC's own adb, just laid out, runs
//! `start-server` for 127.0.0.1:5037 within a bounded time, and adb's own
//! version handshake decides: a server that answers with this client's
//! protocol is reused, one with another protocol adb kills and starts again
//! itself, and with none adb starts one. When that attempt does not end within
//! the time — a server is there but does not answer — or ends in failure,
//! every process listening on 127.0.0.1:5037 is ended without a question,
//! each logged with its PID and image path, and `start-server` runs once
//! more. A second failure is said loudly: the upgrade stays in place, and the
//! summary says the server is not ready and why. An adb that cannot be run
//! at all says nothing about the server: then nothing is ended, and that is
//! said as loudly.
//!
//! The adb client runs from the install root. A server it starts takes the
//! client's working directory, and the root is the one directory an upgrade
//! never moves — never `ui\`, which a server working in it holds against the
//! next upgrade.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::platform;
use crate::runtime::stop;
use crate::verify::{Report, Step};

/// The ADB server's port, shared by every adb client on the machine.
pub const PORT: u16 = 5037;
/// How long one `start-server` may take. Starting a server takes seconds; a
/// server that is there but does not answer keeps the client waiting for good.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a process that was ended is waited for to be gone.
const END_WAIT: Duration = Duration::from_secs(10);
/// How long the client's output is waited for once the client is gone: a
/// server it started must never hold the wizard on a pipe.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

/// How the server stands after the check.
pub enum AdbServer {
    /// Ready, and how — adb's own words, and what was ended first, if anything.
    Ready(String),
    /// Not ready after the second attempt, or AC's adb could not be run, and
    /// why.
    NotReady(String),
}

/// Why one `start-server` did not end well.
enum Failed {
    /// The client could not be started: nothing is known of the server.
    NotRun(String),
    /// It ran, and failed or ran out of time.
    Ran(String),
}

/// The check, with `adb` run from `cwd` against 127.0.0.1:`port` (`PORT`
/// but in a test). An error is only a log write that failed; how the server
/// stands is the value.
pub fn ensure(adb: &Path, cwd: &Path, port: u16, report: Report<'_>) -> Result<AdbServer, String> {
    report.step(Step::Phase("检查 ADB 服务 / Checking the ADB server", None))?;
    report.line(&format!(
        "用 AC 自带的 adb 检查 127.0.0.1:{port} 上的 ADB 服务 / checking the ADB server on 127.0.0.1:{port} with AC's own adb: {} start-server（每次至多 {} 秒，工作目录 / at most {} s each, working directory {}）",
        adb.display(),
        ATTEMPT_TIMEOUT.as_secs(),
        ATTEMPT_TIMEOUT.as_secs(),
        cwd.display()
    ))?;
    let first = match start_server(adb, cwd, port) {
        Ok(said) => {
            report.line(&format!("ADB 服务就绪 / the ADB server is ready: {said}"))?;
            return Ok(AdbServer::Ready(said));
        }
        Err(Failed::NotRun(why)) => {
            let why = format!(
                "AC 自带的 adb 无法运行，未结束任何进程 / AC's own adb cannot be run, nothing was ended: {why}"
            );
            return not_ready(why, report);
        }
        Err(Failed::Ran(why)) => why,
    };
    report.line(&format!(
        "第一次检查未通过 / the first check failed: {first}"
    ))?;
    let ended = end_listeners(port, report)?;
    report.line("再次拉起 ADB 服务 / starting the ADB server again")?;
    match start_server(adb, cwd, port) {
        Ok(said) => {
            let how = format!("{ended}；再次拉起 / started again: {said}");
            report.line(&format!("ADB 服务就绪 / the ADB server is ready: {how}"))?;
            Ok(AdbServer::Ready(how))
        }
        Err(Failed::NotRun(second) | Failed::Ran(second)) => not_ready(
            format!("第一次 / first: {first}；{ended}；第二次 / second: {second}"),
            report,
        ),
    }
}

/// The loud end: on the page and in the summary's notes, and the value.
fn not_ready(why: String, report: Report<'_>) -> Result<AdbServer, String> {
    report.warn(&format!(
        "ADB 服务未就绪（升级已完成）/ The ADB server is not ready (the upgrade is in place): {why}"
    ))?;
    Ok(AdbServer::NotReady(why))
}

/// `<adb> start-server` for 127.0.0.1:`port`, without a window, within
/// `ATTEMPT_TIMEOUT`: what adb said when it ended well, or why not.
fn start_server(adb: &Path, cwd: &Path, port: u16) -> Result<String, Failed> {
    let mut command = Command::new(adb);
    command
        .arg("start-server")
        .current_dir(cwd)
        // adb's own way to name the server: the port checked, on loopback.
        .env("ANDROID_ADB_SERVER_PORT", port.to_string())
        .env_remove("ADB_SERVER_SOCKET")
        .env_remove("ANDROID_ADB_SERVER_ADDRESS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::runtime::CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| {
        Failed::NotRun(format!("无法运行 / cannot run {}: {error}", adb.display()))
    })?;
    let (sender, received) = mpsc::channel();
    let mut readers = 0;
    let mut unread = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        match read_into(pipe, "stdout", sender.clone()) {
            Ok(()) => readers += 1,
            Err(error) => unread.push(error),
        }
    }
    if let Some(pipe) = child.stderr.take() {
        match read_into(pipe, "stderr", sender) {
            Ok(()) => readers += 1,
            Err(error) => unread.push(error),
        }
    }
    let deadline = Instant::now() + ATTEMPT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => break None,
            Err(error) => {
                return Err(Failed::Ran(format!(
                    "{}: {error}{}",
                    adb.display(),
                    stop(&mut child)
                )))
            }
        }
    };
    let stopped = match status {
        None => stop(&mut child),
        Some(_) => String::new(),
    };
    let said = gather(&received, readers, unread);
    match status {
        None => Err(Failed::Ran(format!(
            "超过 {} 秒未结束（多半是 127.0.0.1:{port} 上有服务却不应答）/ did not finish within {} s (most likely a server is on 127.0.0.1:{port} but does not answer){stopped}{}",
            ATTEMPT_TIMEOUT.as_secs(),
            ATTEMPT_TIMEOUT.as_secs(),
            said.map(|text| format!("：{text}")).unwrap_or_default()
        ))),
        Some(status) if status.success() => Ok(said.unwrap_or_else(|| {
            "adb 没有输出：已在运行的服务被复用 / adb printed nothing: the server already running was reused".to_string()
        })),
        Some(status) => Err(Failed::Ran(format!(
            "adb 退出码 / adb exit {}{}",
            status.code().map_or_else(|| "—".to_string(), |code| code.to_string()),
            said.map(|text| format!("：{text}")).unwrap_or_default()
        ))),
    }
}

/// A pipe read whole on its own thread, its text sent on `sender` tagged
/// with `which`.
fn read_into(
    mut pipe: impl Read + Send + 'static,
    which: &'static str,
    sender: mpsc::Sender<(&'static str, String)>,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("acsetup-adb-output".into())
        .spawn(move || {
            let mut text = String::new();
            let read = pipe.read_to_string(&mut text);
            let text = match read {
                Ok(_) => text,
                Err(error) => format!("{text}（读取失败 / read failed: {error}）"),
            };
            let _ = sender.send((which, text));
        })
        .map(|_| ())
        .map_err(|error| format!("{which} 读取线程未能启动 / reader not started: {error}"))
}

/// What the client printed, each stream trimmed and tagged, `None` when it
/// printed nothing; a stream still open `OUTPUT_GRACE` after the client is
/// gone is said, not waited for.
fn gather(
    received: &mpsc::Receiver<(&'static str, String)>,
    readers: usize,
    mut said: Vec<String>,
) -> Option<String> {
    let deadline = Instant::now() + OUTPUT_GRACE;
    for _ in 0..readers {
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((which, text)) => {
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    said.push(format!("{which}: {text}"));
                }
            }
            Err(_) => {
                said.push(format!(
                    "输出在 {} 秒内未关闭 / output still open after {} s",
                    OUTPUT_GRACE.as_secs(),
                    OUTPUT_GRACE.as_secs()
                ));
                break;
            }
        }
    }
    (!said.is_empty()).then(|| said.join(" | "))
}

/// Every process listening on 127.0.0.1:`port` ended — never the system's
/// own or this wizard — each logged with its PID and image path before it is
/// ended; returns what was done, said for the summary.
fn end_listeners(port: u16, report: Report<'_>) -> Result<String, String> {
    let pids = match platform::loopback_listeners(port) {
        Ok(pids) => pids,
        Err(reason) => {
            let said = format!(
                "列不出监听 127.0.0.1:{port} 的进程 / cannot list the processes listening on 127.0.0.1:{port}: {reason}"
            );
            report.line(&said)?;
            return Ok(said);
        }
    };
    if pids.is_empty() {
        let said =
            format!("没有进程监听 127.0.0.1:{port} / no process listens on 127.0.0.1:{port}");
        report.line(&said)?;
        return Ok(said);
    }
    let mut done = Vec::new();
    for pid in pids {
        let image = platform::process_image(pid).map_or_else(
            |reason| format!("映像路径未知 / image path unknown: {reason}"),
            |path| path.display().to_string(),
        );
        let outcome = if matches!(pid, 0 | 4) || pid == std::process::id() {
            format!("PID {pid}（{image}）是系统进程或本引导，未结束 / is the system's or this wizard's, not ended")
        } else {
            report.line(&format!(
                "结束监听 127.0.0.1:{port} 的进程 / ending the process listening on 127.0.0.1:{port}: PID {pid} · {image}"
            ))?;
            match platform::end_process(pid, END_WAIT) {
                Ok(()) => format!("已结束 / ended PID {pid}（{image}）"),
                Err(reason) => format!("未能结束 / could not end PID {pid}（{image}）: {reason}"),
            }
        };
        report.line(&outcome)?;
        done.push(outcome);
    }
    Ok(done.join("；"))
}
