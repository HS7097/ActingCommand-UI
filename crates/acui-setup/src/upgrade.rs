// SPDX-License-Identifier: GPL-3.0-only
//! Step 2 on a root that already holds an installation: the upgrade. The new
//! Runtime first checks the configuration as it is. A Runtime that answers is
//! asked to shut down and awaited with the new `actingctl` — one started from
//! the console holds `ui\` as its working directory — then the console's, the
//! tools' and the Runtime's directories move aside. The verified payload is
//! laid out in their place and a Runtime that was running starts again on it.
//! Until the new version is laid out, any failure puts every moved directory
//! back, and a Runtime stopped for it starts again on the version still in
//! place. State, the configuration, the console's settings and the downloads
//! stay as they are. The version replaced is kept whole in `previous\`, one
//! version deep: the Runtime ships no state migration and no rollback of its
//! own, so going back stays a person's choice.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::install::{self, LaidOut};
use crate::verify::{Report, Verified, MANIFEST};

const ACTINGD: &str = "actingcommand-actingd.exe";
const ACTINGCTL: &str = "actingctl.exe";
const CHECK_SCHEMA: &str = "actingcommand.actingd.check-config.v1";
/// How long `request-shutdown --wait` waits for the Runtime to be gone, and
/// how long any child the upgrade runs may take in all.
const SHUTDOWN_WAIT_SECONDS: u64 = 60;
const CHILD_TIMEOUT: Duration = Duration::from_secs(SHUTDOWN_WAIT_SECONDS + 30);
/// How long a restarted Runtime has to write its own `runtime-info.json`.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const UNCONFIRMED_NOTE: &str = "Runtime 的关闭未确认，它可能仍在运行或正在退出，未重新拉起：稍后在监控台确认 / The Runtime's shutdown was not confirmed; it may still run or be exiting, and was not started again: check in the console later";
const STOPPED_NOTE: &str = "Runtime 已被请求关闭，可能已停止，未重新拉起：请在监控台点「启动」 / The Runtime was asked to shut down and may have stopped; it was not started again: press Start in the console";

/// Windows `CREATE_NO_WINDOW` for the checks, `DETACHED_PROCESS` for the
/// Runtime started again, which outlives the wizard.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// What is installed: the commits the two manifests name.
pub struct Installed {
    pub runtime_sha: String,
    pub ui_sha: String,
}

#[derive(Deserialize)]
struct Manifest {
    commit_sha: String,
}

/// `Some` when `root\runtime\BUILD-MANIFEST.json` exists; a manifest that is
/// there but unreadable is an error, never "not installed".
pub fn installed(root: &Path) -> Result<Option<Installed>, String> {
    let runtime = root.join("runtime").join(MANIFEST);
    if !runtime.is_file() {
        return Ok(None);
    }
    let commit = |path: PathBuf| {
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
        serde_json::from_str::<Manifest>(&text)
            .map(|manifest| manifest.commit_sha)
            .map_err(|error| format!("清单无法解析 / manifest unreadable: {}: {error}", path.display()))
    };
    Ok(Some(Installed {
        runtime_sha: commit(runtime)?,
        ui_sha: commit(root.join("ui").join(MANIFEST))?,
    }))
}

pub struct Upgraded {
    pub laid_out: LaidOut,
    /// Where the version replaced is kept.
    pub previous: PathBuf,
    /// The log of the Runtime started again, when it was running before.
    pub restarted: Option<PathBuf>,
}

pub fn upgrade(root: &Path, verified: &Verified, report: Report<'_>) -> Result<Upgraded, String> {
    let config = root.join("actingd.config.json");
    let state_root = state_root(&config)?;
    check_config(&verified.runtime.dir.join(ACTINGD), &config)?;
    report("新 Runtime 接受现有配置 / the new Runtime accepts the configuration as it is")?;

    let mut swap = Swap {
        root,
        previous: root.join("previous"),
        older: None,
        moved: Vec::new(),
        stopped: false,
        confirmed: false,
        made_previous: false,
        unanswered: None,
    };
    let laid_out = match swap.lay(verified, &state_root, report) {
        Ok(laid_out) => laid_out,
        Err(reason) => {
            let (mut reason, clean) = swap.undo(reason);
            if swap.stopped {
                reason.push('\n');
                if !swap.confirmed {
                    // The shutdown was not confirmed: a second Runtime would
                    // contend for the same state root.
                    reason.push_str(UNCONFIRMED_NOTE);
                } else if !clean {
                    // What is in `runtime\` may not be the version installed.
                    reason.push_str(STOPPED_NOTE);
                } else {
                    // Everything is back: the Runtime starts again on the
                    // version that stays installed.
                    let actingd = root.join("runtime").join(ACTINGD);
                    reason.push_str(&match restart(root, &actingd, &config, &state_root, report) {
                        Ok(log) => format!(
                            "Runtime 已用原版本重新拉起 / the Runtime was started again on the version still installed; 日志 / log: {}",
                            log.display()
                        ),
                        Err(failed) => failed,
                    });
                }
            }
            return Err(reason);
        }
    };
    // The new version is in place: from here nothing is put back, since a
    // Runtime started on it may already have touched state.
    let laid = format!(
        "新版本已铺开；被替换的版本在 / The new version is laid out; the version replaced is in: {}",
        swap.previous.display()
    );
    let restarted = match swap.stopped {
        true => Some(
            restart(root, &laid_out.actingd_exe, &config, &state_root, report)
                .map_err(|reason| format!("{reason}\n{laid}"))?,
        ),
        false => None,
    };
    swap.drop_older(report).map_err(|reason| format!("{reason}\n{laid}"))?;
    Ok(Upgraded { laid_out, previous: swap.previous, restarted })
}

/// The directories an upgrade has moved, so a failure can put them back.
struct Swap<'a> {
    root: &'a Path,
    previous: PathBuf,
    /// The version kept from the upgrade before, moved aside until this one
    /// is laid out.
    older: Option<PathBuf>,
    moved: Vec<&'static str>,
    /// Whether the Runtime was asked to shut down, and whether its shutdown
    /// was confirmed.
    stopped: bool,
    confirmed: bool,
    /// Whether this upgrade made `previous\`, so a failure removes it.
    made_previous: bool,
    /// Why a `runtime-info.json` that is there was taken as no Runtime.
    unanswered: Option<String>,
}

impl Swap<'_> {
    fn lay(&mut self, verified: &Verified, state_root: &Path, report: Report<'_>) -> Result<LaidOut, String> {
        self.clear_leftovers(report)?;
        if self.previous.exists() {
            let older = self.root.join(format!("previous.older-{}", crate::log::unix_ms()));
            fs::rename(&self.previous, &older).map_err(|error| {
                format!("无法移开更早的旧版本 / cannot move {} aside: {error}", self.previous.display())
            })?;
            self.older = Some(older);
        }
        fs::create_dir(&self.previous)
            .map_err(|error| format!("无法创建 / cannot create {}: {error}", self.previous.display()))?;
        self.made_previous = true;
        // The Runtime first: one the console started works in `ui\`, which
        // cannot move while it runs.
        let ctl = verified.runtime.dir.join(ACTINGCTL);
        match runtime_answers(&ctl, state_root, report)? {
            Ok(()) => {
                self.stopped = true;
                report("请求 Runtime 关闭并等待 / asking the Runtime to shut down, and waiting")?;
                request_shutdown(&ctl, state_root)?;
                self.confirmed = true;
                report("Runtime 已关闭 / the Runtime has shut down")?;
            }
            Err(unanswered) => self.unanswered = unanswered,
        }
        self.move_aside("ui", "请先关闭监控台 / close the console first", report)?;
        self.move_aside("tools", "多半有程序正从这里运行 / most likely a program runs from it", report)?;
        self.move_aside("runtime", "多半有程序正从这里运行 / most likely a program runs from it", report)?;
        install::lay_out(self.root, verified, report)
    }

    fn move_aside(&mut self, name: &'static str, hint: &str, report: Report<'_>) -> Result<(), String> {
        let from = self.root.join(name);
        if !from.exists() {
            return Ok(());
        }
        fs::rename(&from, self.previous.join(name)).map_err(|error| {
            let mut reason = format!("无法移开 / cannot move {}: {error}\n{hint}", from.display());
            if let Some(unanswered) = &self.unanswered {
                reason.push('\n');
                reason.push_str(unanswered);
            }
            reason
        })?;
        self.moved.push(name);
        report(&format!("已移开旧版本 / moved aside: {}", from.display()))
    }

    /// Puts everything moved back where it was, the version kept from before
    /// included, and says what could not be; `true` when everything went back.
    fn undo(&mut self, reason: String) -> (String, bool) {
        let first = reason.len();
        let mut reason = reason;
        for name in self.moved.iter().rev() {
            let placed = self.root.join(name);
            if placed.exists() {
                if let Err(error) = fs::remove_dir_all(&placed) {
                    reason.push_str(&format!(
                        "\n未能删除铺了一半的 / half-laid-out not removed: {}: {error}",
                        placed.display()
                    ));
                    continue;
                }
            }
            if let Err(error) = fs::rename(self.previous.join(name), &placed) {
                reason.push_str(&format!(
                    "\n未能放回 / not put back: {} → {}: {error}",
                    self.previous.join(name).display(),
                    placed.display()
                ));
            }
        }
        if self.made_previous {
            if let Err(error) = fs::remove_dir(&self.previous) {
                reason.push_str(&format!("\n未能删除 / not removed: {}: {error}", self.previous.display()));
            }
        }
        if let Some(older) = &self.older {
            if let Err(error) = fs::rename(older, &self.previous) {
                reason.push_str(&format!(
                    "\n更早的旧版本未能放回 / the older version was not put back: {} → {}: {error}",
                    older.display(),
                    self.previous.display()
                ));
            }
        }
        let clean = reason.len() == first;
        (reason, clean)
    }

    /// The version from the upgrade before, no longer needed once this one is
    /// laid out; one that cannot be removed is said, and removed next time.
    fn drop_older(&self, report: Report<'_>) -> Result<(), String> {
        match &self.older {
            Some(older) => report(&match fs::remove_dir_all(older) {
                Ok(()) => format!("已删除更早的旧版本 / older version removed: {}", older.display()),
                Err(error) => format!(
                    "更早的旧版本未能删除，下次升级再删 / older version not removed, the next upgrade retries: {}: {error}",
                    older.display()
                ),
            }),
            None => Ok(()),
        }
    }

    /// `previous.older-*` directories an earlier upgrade could not remove.
    fn clear_leftovers(&self, report: Report<'_>) -> Result<(), String> {
        let entries = fs::read_dir(self.root)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", self.root.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let leftover = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("previous.older-"));
            if leftover && path.is_dir() {
                report(&match fs::remove_dir_all(&path) {
                    Ok(()) => format!("已删除残留的旧版本 / leftover removed: {}", path.display()),
                    Err(error) => format!(
                        "残留的旧版本仍未能删除 / leftover still not removed: {}: {error}",
                        path.display()
                    ),
                })?;
            }
        }
        Ok(())
    }
}

/// `state_root` from the configuration. The wizard writes an absolute one; a
/// relative one would depend on the Runtime's working directory, so it stops.
fn state_root(config: &Path) -> Result<PathBuf, String> {
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
fn runtime_answers(
    actingctl: &Path,
    state_root: &Path,
    report: Report<'_>,
) -> Result<Result<(), Option<String>>, String> {
    if !state_root.join("runtime-info.json").is_file() {
        report("Runtime 未在运行 / the Runtime is not running")?;
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
    report(&line)?;
    Ok(Err(Some(line)))
}

/// `<new actingd> check-config --config <config>`: the report is stdout;
/// stderr is kept for the failure text.
fn check_config(actingd: &Path, config: &Path) -> Result<(), String> {
    let out = run(Command::new(actingd).arg("check-config").arg("--config").arg(config))?;
    let report: Value = serde_json::from_str(out.stdout.trim()).unwrap_or_default();
    let error = &report["error"];
    let exit = &out.exit;
    match (report["status"].as_str(), error["code"].as_str(), error["stage"].as_str()) {
        _ if report["schema_version"] != CHECK_SCHEMA => Err(format!(
            "新 Runtime 的 check-config 输出无法识别（退出码 {exit}），未做任何改动 / unreadable check-config output (exit {exit}), nothing was changed: {} {}",
            out.stdout.trim(),
            out.stderr.trim()
        )),
        (Some("ok"), _, _) if out.success => Ok(()),
        (Some("failed"), Some(code), Some(stage)) => Err(format!(
            "新 Runtime 不接受现有配置，未做任何改动 / the new Runtime refuses the configuration, nothing was changed: {code}（{stage}）{}",
            out.stderr.trim()
        )),
        _ => Err(format!(
            "新 Runtime 的 check-config 未通过（退出码 {exit}），未做任何改动 / check-config did not pass (exit {exit}), nothing was changed: {} {}",
            out.stdout.trim(),
            out.stderr.trim()
        )),
    }
}

/// `<new actingctl> request-shutdown --state-root <root> --wait <s>`: exit 0
/// once the ownership record is closed and the process gone. Anything else
/// may still end with the Runtime stopping, and is said that way.
fn request_shutdown(actingctl: &Path, state_root: &Path) -> Result<(), String> {
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
fn restart(
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
            report(&format!("Runtime 已就绪 / the Runtime is up; 日志 / log: {}", log.display()))?;
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
fn fatal_line(log: &Path) -> String {
    fs::read_to_string(log)
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with("FATAL")).map(str::to_string))
        .map(|line| format!("：{line}"))
        .unwrap_or_default()
}

/// A child's outcome: whether it succeeded, its exit code as text, and its two
/// output streams, each read whole.
struct Output {
    success: bool,
    exit: String,
    stdout: String,
    stderr: String,
}

/// Runs a child without a window within `CHILD_TIMEOUT`.
fn run(command: &mut Command) -> Result<Output, String> {
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
fn drain(mut pipe: impl Read + Send + 'static) -> Drained {
    std::thread::Builder::new().name("acsetup-output".into()).spawn(move || {
        let mut text = String::new();
        pipe.read_to_string(&mut text).map(|_| text)
    })
}

/// A drained stream's text, or a line saying why it could not be read.
fn collect(reader: Option<Drained>) -> String {
    match reader.map(|reader| reader.map(std::thread::JoinHandle::join)) {
        Some(Ok(Ok(Ok(text)))) => text,
        Some(Ok(Ok(Err(error)))) => format!("（输出读取失败 / output unreadable: {error}）"),
        Some(Ok(Err(_))) => "（输出读取线程失败 / output reader failed）".to_string(),
        Some(Err(error)) => format!("（输出读取线程未能启动 / output reader not started: {error}）"),
        None => String::new(),
    }
}

/// Stops a child past its time, and says how that went.
fn stop(child: &mut Child) -> String {
    match child.kill().and_then(|()| child.wait().map(|_| ())) {
        Ok(()) => "；已终止 / stopped".to_string(),
        Err(error) => format!("；未能终止 / could not be stopped: {error}"),
    }
}
