// SPDX-License-Identifier: GPL-3.0-only
//! Step 2 on a root that already holds an installation: the upgrade. The new
//! Runtime first checks the configuration as it is; the console's and the
//! tools' directories move aside, then — the running Runtime asked to shut
//! down and awaited with the new `actingctl` — the Runtime's; the verified
//! payload is laid out in their place, and the Runtime is started again when
//! it was running. State, the configuration, the console's settings and the
//! downloads stay as they are. The version replaced is kept whole in
//! `previous\`, one version deep: the Runtime ships no state migration and no
//! rollback of its own, so going back stays a person's choice.

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
/// How long a restarted Runtime has to write its `runtime-info.json`.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

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

    let previous = root.join("previous");
    if previous.exists() {
        fs::remove_dir_all(&previous).map_err(|error| {
            format!("无法删除更早的旧版本 / cannot remove {}: {error}", previous.display())
        })?;
        report(&format!("已删除更早的旧版本 / older previous version removed: {}", previous.display()))?;
    }
    fs::create_dir_all(&previous)
        .map_err(|error| format!("无法创建 / cannot create {}: {error}", previous.display()))?;

    // The console's and the tools' first: one that cannot move has a program
    // running from it, and nothing has been stopped yet.
    move_aside(root, &previous, &["ui", "tools"], report)?;
    let running = state_root.join("runtime-info.json").is_file();
    if running {
        report("请求 Runtime 关闭并等待 / asking the Runtime to shut down, and waiting")?;
        let ctl = verified.runtime.dir.join(ACTINGCTL);
        if let Err(reason) = request_shutdown(&ctl, &state_root, report) {
            return Err(put_back(root, &previous, &["ui", "tools"], reason));
        }
    } else {
        report("Runtime 未在运行 / the Runtime is not running")?;
    }
    if let Err(reason) = move_aside(root, &previous, &["runtime"], report) {
        let mut reason = put_back(root, &previous, &["ui", "tools"], reason);
        if running {
            reason.push_str("\nRuntime 已关闭，未重新拉起 / the Runtime was shut down and not started again");
        }
        return Err(reason);
    }

    let laid_out = match install::lay_out(root, verified, report) {
        Ok(laid_out) => laid_out,
        Err(reason) => {
            let mut reason = reason;
            for name in ["runtime", "ui", "tools"] {
                let partial = root.join(name);
                if partial.exists() {
                    if let Err(error) = fs::remove_dir_all(&partial) {
                        reason.push_str(&format!(
                            "\n未能删除铺了一半的 / half-laid-out not removed: {}: {error}",
                            partial.display()
                        ));
                    }
                }
            }
            return Err(put_back(root, &previous, &["runtime", "ui", "tools"], reason));
        }
    };
    let restarted = match running {
        true => Some(restart(root, &laid_out.actingd_exe, &config, &state_root, &previous, report)?),
        false => None,
    };
    Ok(Upgraded { laid_out, previous, restarted })
}

/// `state_root` from the configuration; a relative one is resolved against
/// the configuration's folder, as the Runtime resolves it.
fn state_root(config: &Path) -> Result<PathBuf, String> {
    let text = fs::read_to_string(config)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", config.display()))?;
    let document: Value = serde_json::from_str(&text)
        .map_err(|error| format!("配置无法解析 / config unreadable: {}: {error}", config.display()))?;
    let root = document["state_root"].as_str().ok_or_else(|| {
        format!("配置里没有 state_root / no state_root in {}", config.display())
    })?;
    let root = PathBuf::from(root);
    Ok(match (root.is_absolute(), config.parent()) {
        (false, Some(folder)) => folder.join(root),
        _ => root,
    })
}

/// Moves `names` out of `root` into `previous`. A move that fails puts back
/// those already moved and says which directory could not move.
fn move_aside(root: &Path, previous: &Path, names: &[&str], report: Report<'_>) -> Result<(), String> {
    for (index, name) in names.iter().enumerate() {
        let from = root.join(name);
        if !from.exists() {
            continue;
        }
        if let Err(error) = fs::rename(&from, previous.join(name)) {
            let reason = format!(
                "无法移开 / cannot move {}: {error}\n多半有程序正从这里运行：请先关闭监控台 / most likely a program runs from it: close the console first",
                from.display()
            );
            return Err(put_back(root, previous, &names[..index], reason));
        }
        report(&format!("已移开旧版本 / moved aside: {}", from.display()))?;
    }
    Ok(())
}

/// Puts `names` back from `previous` into `root`, appending to `reason` what
/// could not be.
fn put_back(root: &Path, previous: &Path, names: &[&str], mut reason: String) -> String {
    for name in names {
        let kept = previous.join(name);
        if kept.exists() {
            if let Err(error) = fs::rename(&kept, root.join(name)) {
                reason.push_str(&format!(
                    "\n未能放回 / not put back: {} → {}: {error}",
                    kept.display(),
                    root.join(name).display()
                ));
            }
        }
    }
    reason
}

/// `<new actingd> check-config --config <config>`, as the console's instance
/// window runs it.
fn check_config(actingd: &Path, config: &Path) -> Result<(), String> {
    let (success, exit, output) = run(
        Command::new(actingd).arg("check-config").arg("--config").arg(config),
    )?;
    let report: Value = serde_json::from_str(output.trim()).unwrap_or_default();
    let error = &report["error"];
    match (report["status"].as_str(), error["code"].as_str(), error["stage"].as_str()) {
        _ if report["schema_version"] != CHECK_SCHEMA => Err(format!(
            "新 Runtime 的 check-config 输出无法识别（退出码 {exit}）/ unreadable check-config output (exit {exit}): {}",
            output.trim()
        )),
        (Some("ok"), _, _) if success => Ok(()),
        (Some("failed"), Some(code), Some(stage)) => Err(format!(
            "新 Runtime 不接受现有配置，未做任何改动 / the new Runtime refuses the configuration, nothing was changed: {code}（{stage}）"
        )),
        _ => Err(format!(
            "新 Runtime 的 check-config 未通过（退出码 {exit}）/ check-config did not pass (exit {exit}): {}",
            output.trim()
        )),
    }
}

/// `<new actingctl> request-shutdown --state-root <root> --wait <s>`: exit 0
/// once the ownership record is closed and the process is gone.
fn request_shutdown(actingctl: &Path, state_root: &Path, report: Report<'_>) -> Result<(), String> {
    let (success, exit, output) = run(
        Command::new(actingctl)
            .arg("request-shutdown")
            .arg("--state-root")
            .arg(state_root)
            .arg("--wait")
            .arg(SHUTDOWN_WAIT_SECONDS.to_string()),
    )?;
    if !success {
        return Err(format!(
            "Runtime 未能在 {SHUTDOWN_WAIT_SECONDS} 秒内关闭（退出码 {exit}），未做任何改动 / the Runtime did not shut down (exit {exit}), nothing was changed: {}",
            output.trim()
        ));
    }
    report("Runtime 已关闭 / the Runtime has shut down")
}

/// The new Runtime, detached, its output in `<root>\actingd-<unix_ms>.log`,
/// given `READY_TIMEOUT` to write its `runtime-info.json`.
fn restart(
    root: &Path,
    actingd: &Path,
    config: &Path,
    state_root: &Path,
    previous: &Path,
    report: Report<'_>,
) -> Result<PathBuf, String> {
    let log = root.join(format!("actingd-{}.log", crate::log::unix_ms()));
    let stdout = fs::File::create(&log)
        .map_err(|error| format!("无法创建 / cannot create {}: {error}", log.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("无法写入 / cannot write {}: {error}", log.display()))?;
    let mut command = Command::new(actingd);
    command.arg("--config").arg(config).stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(DETACHED_PROCESS);
    }
    command
        .spawn()
        .map_err(|error| format!("无法拉起新 Runtime / cannot start the new Runtime: {error}"))?;
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if state_root.join("runtime-info.json").is_file() {
            report(&format!("新 Runtime 已就绪 / the new Runtime is up; 日志 / log: {}", log.display()))?;
            return Ok(log);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "新 Runtime 在 {} 秒内没有就绪 / the new Runtime was not up within {} s\n日志 / log: {}\n被替换的版本保留在 / the version replaced is kept in: {}",
        READY_TIMEOUT.as_secs(),
        READY_TIMEOUT.as_secs(),
        log.display(),
        previous.display()
    ))
}

/// Runs a child without a window, its output read whole, within
/// `CHILD_TIMEOUT`: whether it succeeded, its exit code as text, and stdout
/// then stderr.
fn run(command: &mut Command) -> Result<(bool, String, String), String> {
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
    let readers = [child.stdout.take().map(drain), child.stderr.take().map(drain)];
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
    let mut output = String::new();
    for reader in readers.into_iter().flatten() {
        match reader.map(std::thread::JoinHandle::join) {
            Ok(Ok(Ok(text))) => output.push_str(&text),
            Ok(Ok(Err(error))) => {
                output.push_str(&format!("\n（输出读取失败 / output unreadable: {error}）"))
            }
            Ok(Err(_)) => output.push_str("\n（输出读取线程失败 / output reader failed）"),
            Err(error) => output.push_str(&format!(
                "\n（输出读取线程未能启动 / output reader not started: {error}）"
            )),
        }
    }
    let exit = status.code().map_or_else(|| "—".to_string(), |code| code.to_string());
    Ok((status.success(), exit, output))
}

type Drained = std::io::Result<std::thread::JoinHandle<std::io::Result<String>>>;

/// A pipe read whole on its own thread, so a long output never stalls the child.
fn drain(mut pipe: impl Read + Send + 'static) -> Drained {
    std::thread::Builder::new().name("acsetup-output".into()).spawn(move || {
        let mut text = String::new();
        pipe.read_to_string(&mut text).map(|_| text)
    })
}

/// Stops a child past its time, and says how that went.
fn stop(child: &mut Child) -> String {
    match child.kill().and_then(|()| child.wait().map(|_| ())) {
        Ok(()) => "；已终止 / stopped".to_string(),
        Err(error) => format!("；未能终止 / could not be stopped: {error}"),
    }
}
