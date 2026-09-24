// SPDX-License-Identifier: GPL-3.0-only
//! The Runtime as the wizard drives it from outside, through its own programs
//! only: `actingd check-config`, `actingctl status`, `request-shutdown` and
//! `emulator discover`, and actingd itself started detached. Every child runs
//! without a window, its output read whole, within a time limit.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::verify::Report;

pub(crate) const ACTINGD: &str = "actingcommand-actingd.exe";
pub(crate) const ACTINGCTL: &str = "actingctl.exe";
pub(crate) const CHECK_SCHEMA: &str = "actingcommand.actingd.check-config.v1";
/// How long `request-shutdown --wait` waits for the Runtime to be gone, and
/// how long any child the wizard runs may take in all.
pub(crate) const SHUTDOWN_WAIT_SECONDS: u64 = 60;
pub(crate) const CHILD_TIMEOUT: Duration = Duration::from_secs(SHUTDOWN_WAIT_SECONDS + 30);
/// How long a restarted Runtime has to write its own `runtime-info.json`.
pub(crate) const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// Windows `CREATE_NO_WINDOW` for the checks, `DETACHED_PROCESS` for the
/// Runtime started again, which outlives the wizard.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
pub(crate) const DETACHED_PROCESS: u32 = 0x0000_0008;

/// `state_root` from the configuration. The wizard writes an absolute one; a
/// relative one would depend on the Runtime's working directory, so it stops.
pub(crate) fn state_root(config: &Path) -> Result<PathBuf, String> {
    let text = fs::read_to_string(config)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", config.display()))?;
    let document: Value = serde_json::from_str(&text)
        .map_err(|error| format!("配置无法解析 / config unreadable: {}: {error}", config.display()))?;
    let root = document["state_root"].as_str().map(PathBuf::from).ok_or_else(|| {
        format!("配置里没有 state_root / no state_root in {}", config.display())
    })?;
    match root.is_absolute() {
        true => Ok(root),
        false => Err(format!(
            "配置的 state_root 不是绝对路径，向导无法确定它 / state_root in {} is not absolute: {}",
            config.display(),
            root.display()
        )),
    }
}

/// Whether a Runtime runs on the state root: `runtime-info.json` there and
/// the new `actingctl status` answered by it (`Ok`). Otherwise `Err`, with the
/// reason when the file is there but no Runtime answered — one that ended
/// without shutting down leaves it — said and taken as not running.
pub(crate) fn runtime_answers(
    actingctl: &Path,
    state_root: &Path,
    report: Report<'_>,
) -> Result<Result<(), Option<String>>, String> {
    if !state_root.join("runtime-info.json").is_file() {
        report.line("Runtime 未在运行 / the Runtime is not running")?;
        return Ok(Err(None));
    }
    let out = run(Command::new(actingctl).arg("status").arg("--state-root").arg(state_root))?;
    if out.success {
        return Ok(Ok(()));
    }
    let line = format!(
        "有 runtime-info.json 但 Runtime 不应答（退出码 {}），按未运行处理 / runtime-info.json is there but no Runtime answers (exit {}), taken as not running: {}",
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
    let out = run(Command::new(actingd).arg("check-config").arg("--config").arg(config))?;
    let report: Value = serde_json::from_str(out.stdout.trim()).unwrap_or_default();
    let error = &report["error"];
    let exit = &out.exit;
    match (report["status"].as_str(), error["code"].as_str(), error["stage"].as_str()) {
        _ if report["schema_version"] != CHECK_SCHEMA => Err(format!(
            "check-config 的输出无法识别（退出码 {exit}）/ unreadable check-config output (exit {exit}): {} {}",
            out.stdout.trim(),
            out.stderr.trim()
        )),
        (Some("ok"), _, _) if out.success => Ok(()),
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

/// `<actingctl> request-shutdown --state-root <root> --wait <s>`: exit 0
/// once the ownership record is closed and the process gone. Anything else
/// may still end with the Runtime stopping, and is said that way.
pub(crate) fn request_shutdown(actingctl: &Path, state_root: &Path) -> Result<(), String> {
    let out = run(
        Command::new(actingctl)
            .arg("request-shutdown")
            .arg("--state-root")
            .arg(state_root)
            .arg("--wait")
            .arg(SHUTDOWN_WAIT_SECONDS.to_string()),
    )?;
    match out.success {
        true => Ok(()),
        false => Err(format!(
            "Runtime 的关闭没有在 {SHUTDOWN_WAIT_SECONDS} 秒内确认（退出码 {}）/ the Runtime's shutdown was not confirmed within {SHUTDOWN_WAIT_SECONDS} s (exit {}): {} {}",
            out.exit,
            out.exit,
            out.stdout.trim(),
            out.stderr.trim()
        )),
    }
}

/// A Runtime, detached, from the install root, its output in
/// `<root>\actingd-<unix_ms>.log`: up once its own `runtime-info.json` names
/// its pid, within `READY_TIMEOUT`; an exit before that is said with its
/// `FATAL` line.
pub(crate) fn restart(
    root: &Path,
    actingd: &Path,
    config: &Path,
    state_root: &Path,
    report: Report<'_>,
) -> Result<PathBuf, String> {
    let log = root.join(format!("actingd-{}.log", crate::log::unix_ms()));
    let stdout = fs::File::create(&log)
        .map_err(|error| format!("无法创建 / cannot create {}: {error}", log.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("无法写入 / cannot write {}: {error}", log.display()))?;
    let mut command = Command::new(actingd);
    command
        .arg("--config")
        .arg(config)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(DETACHED_PROCESS);
    }
    let mut child = command
        .spawn()
        .map_err(|error| {
            format!("无法拉起 Runtime，现在未运行 / cannot start the Runtime, which is not running: {error}")
        })?;
    let info = state_root.join("runtime-info.json");
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "Runtime 启动后退出（{status}），现在未运行 / the Runtime exited on start ({status}) and is not running{}\n日志 / log: {}",
                fatal_line(&log),
                log.display()
            ));
        }
        let pid = fs::read_to_string(&info)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|info| info["pid"].as_u64());
        if pid == Some(u64::from(child.id())) {
            report.line(&format!("Runtime 已就绪 / the Runtime is up; 日志 / log: {}", log.display()))?;
            return Ok(log);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "Runtime 在 {} 秒内没有就绪，可能仍在启动：请看日志，或在监控台确认 / the Runtime was not up within {} s and may still be starting: see the log, or check in the console\n日志 / log: {}",
        READY_TIMEOUT.as_secs(),
        READY_TIMEOUT.as_secs(),
        log.display()
    ))
}

/// The Runtime's own `FATAL actingd:` line from its log, when there is one.
pub(crate) fn fatal_line(log: &Path) -> String {
    fs::read_to_string(log)
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with("FATAL")).map(str::to_string))
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

/// Runs a child without a window within `CHILD_TIMEOUT`.
pub(crate) fn run(command: &mut Command) -> Result<Output, String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
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
                ))
            }
            Err(error) => return Err(format!("{program}: {error}{}", stop(&mut child))),
        }
    };
    Ok(Output {
        success: status.success(),
        exit: status.code().map_or_else(|| "—".to_string(), |code| code.to_string()),
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

type Drained = std::io::Result<std::thread::JoinHandle<std::io::Result<String>>>;

/// A pipe read whole on its own thread, so a long output never stalls the child.
pub(crate) fn drain(mut pipe: impl Read + Send + 'static) -> Drained {
    std::thread::Builder::new().name("acsetup-output".into()).spawn(move || {
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
        Some(Err(error)) => format!("（输出读取线程未能启动 / output reader not started: {error}）"),
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
