// SPDX-License-Identifier: GPL-3.0-only
//! The Runtime as the wizard drives it from outside, through its own programs
//! only: `actingd check-config`, `actingctl status`, `request-shutdown` and
//! `emulator discover`, and actingd itself started detached. Every child runs
//! without a window, its output read whole, within a time limit.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::verify::Report;

pub(crate) const ACTINGD: &str = "actingcommand-actingd.exe";
pub(crate) const ACTINGCTL: &str = "actingctl.exe";
pub(crate) const CHECK_SCHEMA: &str = "actingcommand.actingd.check-config.v1";
/// Checks use a bounded child lifetime. Installation controls use run_observed.
pub(crate) const CHILD_TIMEOUT: Duration = Duration::from_secs(90);
/// Windows `CREATE_NO_WINDOW` for the checks, `DETACHED_PROCESS` for the
/// Runtime started again, which outlives the wizard.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
pub(crate) const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Whether a Runtime runs on the state root: `runtime-info.json` there and
/// the new `actingctl status` answered by it (`Ok`). Otherwise `Err`, with the
/// reason when discovery or the reply is missing. Neither proves shutdown.
pub(crate) fn runtime_answers(
    actingctl: &Path,
    state_root: &Path,
    snapshot: &acui_installation::Snapshot,
    report: Report<'_>,
) -> Result<Result<(), Option<String>>, String> {
    if !state_root.join("runtime-info.json").try_exists().map_err(|error| format!("Cannot inspect Runtime IPC discovery: {error}"))? {
        report.line("尚无 Runtime IPC 定位 / Runtime IPC discovery is absent")?;
        return Ok(Err(None));
    }
    let out = run(crate::lifecycle::command(actingctl, Some(snapshot))?
        .arg("status")
        .arg("--state-root")
        .arg(state_root))?;
    if out.success {
        return Ok(Ok(()));
    }
    let line = format!(
        "有 runtime-info.json 但 Runtime 不应答（退出码 {}），关闭状态未确认 / runtime-info.json is there but no Runtime answers (exit {}), closure is unconfirmed: {}",
        out.exit,
        out.exit,
        out.stderr.trim()
    );
    report.warn(&line)?;
    Ok(Err(Some(line)))
}

/// `<actingd> check-config --config <config>`: the report is stdout; stderr
/// is kept for the failure text.
pub(crate) fn check_config(actingd: &Path, config: &Path) -> Result<(), String> {
    check_config_report(actingd, config).map(|_| ())
}

/// `check_config`, returning the report it accepted.
pub(crate) fn check_config_report(actingd: &Path, config: &Path) -> Result<Value, String> {
    let out = run(Command::new(actingd)
        .env_remove(acui_installation::INSTALL_ROOT_ENV)
        .env_remove(acui_installation::INSTALL_SELECTION_ENV)
        .env_remove(acui_installation::OBSERVER_UI_ENV)
        .arg("check-config")
        .arg("--config")
        .arg(config))?;
    let report: Value = serde_json::from_str(out.stdout.trim()).unwrap_or_default();
    let error = &report["error"];
    let exit = &out.exit;
    match (
        report["status"].as_str(),
        error["code"].as_str(),
        error["stage"].as_str(),
    ) {
        _ if report["schema_version"] != CHECK_SCHEMA => Err(format!(
            "check-config 的输出无法识别（退出码 {exit}）/ unreadable check-config output (exit {exit}): {} {}",
            out.stdout.trim(),
            out.stderr.trim()
        )),
        (Some("ok"), _, _) if out.success => Ok(report.clone()),
        (Some("failed"), Some(code), Some(stage)) => {
            // Only the resource-package stage carries a detail: which
            // instance, which path, and what the package loader said.
            let detail = &error["detail"];
            let loader = match (detail["alias"].as_str(), detail["loader_message"].as_str()) {
                (Some(alias), Some(message)) => format!(
                    "\n{alias}: {} — {message}",
                    detail["path"].as_str().unwrap_or_default()
                ),
                _ => String::new(),
            };
            Err(format!(
                "Runtime 不接受这份配置 / the Runtime refuses the configuration: {code}（{stage}）{loader}{}",
                out.stderr.trim()
            ))
        }
        _ => Err(format!(
            "check-config 未通过（退出码 {exit}）/ check-config did not pass (exit {exit}): {} {}",
            out.stdout.trim(),
            out.stderr.trim()
        )),
    }
}

/// The Runtime's own `FATAL actingd:` line from its log, when there is one.
pub(crate) fn fatal_line(log: &Path) -> String {
    fs::read_to_string(log)
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("FATAL"))
                .map(str::to_string)
        })
        .map(|line| format!("：{line}"))
        .unwrap_or_default()
}

/// A child's outcome: whether it succeeded, its exit code as text, and its two
/// output streams, each read whole.
pub(crate) struct Output {
    pub success: bool,
    pub exit: String,
    pub stdout: String,
    pub stderr: String,
}

/// Installation effects are never killed on a client timeout. The caller must
/// retain the selected generation and treat the operation as unresolved.
pub(crate) fn run_observed(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let program = format!("{:?}", command.get_program());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Cannot run {program}: {error}"))?;
    let stdout = child.stdout.take().map(drain_bounded);
    let stderr = child.stderr.take().map(drain_bounded);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                return Err(format!(
                    "{program} did not finish within {} seconds; pid {} remains unresolved and was not terminated",
                    timeout.as_secs(),
                    child.id()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Cannot observe {program}: {error}; pid {} remains unresolved and was not terminated",
                    child.id()
                ));
            }
        }
    };
    let collect = |reader: Option<Drained>| -> Result<String, String> {
        reader
            .ok_or("Control output pipe is missing")?
            .map_err(|error| format!("Control output reader could not start: {error}"))?
            .join()
            .map_err(|_| "Control output reader panicked".to_string())?
            .map_err(|error| format!("Control output is unreadable: {error}"))
    };
    Ok(Output {
        success: status.success(),
        exit: status.to_string(),
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}

fn drain_bounded(mut pipe: impl Read + Send + 'static) -> Drained {
    std::thread::Builder::new()
        .name("acsetup-control-output".into())
        .spawn(move || {
            const LIMIT: usize = 1024 * 1024;
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 8192];
            let mut overflow = false;
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                if bytes.len().saturating_add(count) > LIMIT {
                    overflow = true;
                }
                if !overflow {
                    bytes.extend_from_slice(&buffer[..count]);
                }
            }
            if overflow {
                return Err(std::io::Error::other("Control output exceeds 1 MiB"));
            }
            String::from_utf8(bytes).map_err(std::io::Error::other)
        })
}

/// Runs a child without a window within `CHILD_TIMEOUT`.
pub(crate) fn run(command: &mut Command) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let program = format!("{:?}", command.get_program());
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法运行 / cannot run {program}: {error}"))?;
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let deadline = Instant::now() + CHILD_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                return Err(format!(
                    "{program} 超过 {} 秒未结束 / did not finish within {} s{}",
                    CHILD_TIMEOUT.as_secs(),
                    CHILD_TIMEOUT.as_secs(),
                    stop(&mut child)
                ));
            }
            Err(error) => return Err(format!("{program}: {error}{}", stop(&mut child))),
        }
    };
    Ok(Output {
        success: status.success(),
        exit: status
            .code()
            .map_or_else(|| "—".to_string(), |code| code.to_string()),
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

type Drained = std::io::Result<std::thread::JoinHandle<std::io::Result<String>>>;

/// A pipe read whole on its own thread, so a long output never stalls the child.
pub(crate) fn drain(mut pipe: impl Read + Send + 'static) -> Drained {
    std::thread::Builder::new()
        .name("acsetup-output".into())
        .spawn(move || {
            let mut text = String::new();
            pipe.read_to_string(&mut text).map(|_| text)
        })
}

/// A drained stream's text, or a line saying why it could not be read.
pub(crate) fn collect(reader: Option<Drained>) -> String {
    match reader.map(|reader| reader.map(std::thread::JoinHandle::join)) {
        Some(Ok(Ok(Ok(text)))) => text,
        Some(Ok(Ok(Err(error)))) => format!("（输出读取失败 / output unreadable: {error}）"),
        Some(Ok(Err(_))) => "（输出读取线程失败 / output reader failed）".to_string(),
        Some(Err(error)) => {
            format!("（输出读取线程未能启动 / output reader not started: {error}）")
        }
        None => String::new(),
    }
}

/// Stops a child past its time, and says how that went.
pub(crate) fn stop(child: &mut Child) -> String {
    match child.kill().and_then(|()| child.wait().map(|_| ())) {
        Ok(()) => "；已终止 / stopped".to_string(),
        Err(error) => format!("；未能终止 / could not be stopped: {error}"),
    }
}
