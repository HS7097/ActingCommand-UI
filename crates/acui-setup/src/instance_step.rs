// SPDX-License-Identifier: GPL-3.0-only
//! The instances step, on a fresh install, optional. A zero-instance
//! Runtime is started from the install root and `actingctl emulator discover`
//! lists the emulator's instances — it reads the MuMu manager's inventory and
//! starts or stops none. The instances a person picks are written into the
//! configuration as a candidate first, checked by `check-config`, and put in
//! place; the Runtime is then restarted so it binds them. Where MuMu is comes
//! from the Runtime's own `check-config`, and is pinned into the configuration
//! so a second MuMu install cannot take the instances over later. The
//! resources come from `bundle`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::fetch;
use crate::runtime::{self, ACTINGCTL, ACTINGD};
use crate::verify::{hex, Report, Step};

/// The input and capture written for a MuMu instance. Capture stays `adb`
/// until a first frame through `nemu_ipc` without `mumu_root` has been seen on
/// the real machine.
const TOUCH_BACKEND: &str = "adb_shell_input";
const CAPTURE_BACKEND: &str = "adb";

/// The note a Runtime too old to say where MuMu is leaves; taken back once
/// MuMu is pinned after all.
pub const NOT_PINNED: &str =
    "这个 Runtime 版本不回报 MuMu 的位置，MuMu 没有钉住 / This Runtime does not say where MuMu is; MuMu is not pinned";

/// One instance the emulator reports.
pub struct Found {
    pub index: u16,
    pub name: String,
    pub adb: Option<String>,
    pub running: bool,
    pub bound_alias: Option<String>,
    pub android: Option<String>,
}

/// One instance a person picked, and the package name its game runs under.
pub struct Chosen {
    pub index: u16,
    pub alias: String,
    pub application_id: String,
}

struct Paths {
    config: PathBuf,
    state_root: PathBuf,
    actingd: PathBuf,
    actingctl: PathBuf,
}

fn paths(root: &Path) -> Result<Paths, String> {
    let config = root.join("actingd.config.json");
    Ok(Paths {
        state_root: runtime::state_root(&config)?,
        actingd: root.join("runtime").join(ACTINGD),
        actingctl: root.join("runtime").join(ACTINGCTL),
        config,
    })
}

/// `mumu_root` pinned — the folder a person gave, or the one the Runtime's
/// `check-config` finds — and a Runtime running that has read it: one that
/// answers is restarted, since it has no instance yet and which folder it read
/// cannot be asked. Returns the folder pinned; `None` from a Runtime too old to
/// say where MuMu is.
pub fn start(root: &Path, mumu_root: Option<&Path>, report: Report<'_>) -> Result<Option<String>, String> {
    let paths = paths(root)?;
    let pinned = match mumu_root {
        Some(mumu) => Some((mumu.display().to_string(), "person".to_string())),
        None => {
            report.step(Step::Phase("查找 MuMu / Finding MuMu", None))?;
            found_mumu(&probe_mumu(&paths)?)?
        }
    };
    match &pinned {
        Some((path, source)) if source != "config" => {
            report.line(&format!("MuMu 位置 / MuMu is at: {path}（{source}）"))?;
            let path = path.clone();
            set_config(&paths, report, |document| document["mumu_root"] = json!(path))?;
        }
        Some((path, _)) => report.line(&format!("MuMu 位置已钉住 / MuMu is pinned at: {path}"))?,
        None => report.warn(NOT_PINNED)?,
    }
    ensure_running(root, &paths, true, report)?;
    Ok(pinned.map(|(path, _)| path))
}

/// `check-config` asked where MuMu is, with any folder pinned before set
/// aside — an empty field finds MuMu afresh — through a probe candidate that
/// is removed afterwards.
fn probe_mumu(paths: &Paths) -> Result<Value, String> {
    let mut document = read_config(paths)?;
    if document.as_object_mut().and_then(|object| object.remove("mumu_root")).is_none() {
        return runtime::check_config_report(&paths.actingd, &paths.config);
    }
    let probe = paths.config.with_file_name(format!("actingd.config.probe-{}.json", std::process::id()));
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("配置序列化失败 / config serialization failed: {error}"))?;
    if let Err(error) = fs::write(&probe, text) {
        return Err(fetch::discard(&probe, format!("写入失败 / write failed: {}: {error}", probe.display())));
    }
    match runtime::check_config_report(&paths.actingd, &probe) {
        Err(reason) => Err(fetch::discard(&probe, reason)),
        Ok(report) => match fs::remove_file(&probe) {
            Ok(()) => Ok(report),
            Err(error) => Err(format!("探测用的候选配置未能删除 / the probe configuration was not removed: {}: {error}", probe.display())),
        },
    }
}

/// A Windows verbatim path as a plain one: `\\?\C:\x` → `C:\x`,
/// `\\?\UNC\host\share` → `\\host\share`; any other form kept as it is.
fn plain_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => path.to_string(),
    }
}

/// Where `check-config` says MuMu is: `{path, source}`, the Windows verbatim
/// prefix taken off; an error saying why when it found none — two installs
/// said as such; `None` when the report has no such field at all.
fn found_mumu(report: &Value) -> Result<Option<(String, String)>, String> {
    match report.get("mumu_root") {
        None => Ok(None),
        Some(Value::Null) => {
            let why = &report["mumu_root_unresolved"];
            let reason = why["reason"].as_str().unwrap_or("?");
            let hint = match reason {
                "installation_ambiguous" => "找到不止一套 MuMu：填要用的那套的安装目录后「重新查找」/ More than one MuMu install: name the one to use and find again",
                _ => "可以填 MuMu 安装目录后「重新查找」/ Name the MuMu folder and find again",
            };
            Err(format!(
                "无法确定 MuMu 的位置 / MuMu's location is not resolved（{reason}）: {}\n{hint}",
                why["message"].as_str().unwrap_or("—")
            ))
        }
        Some(found) => {
            let path = plain_path(found["path"].as_str().unwrap_or_default());
            let source = found["source"].as_str().unwrap_or("?").to_string();
            match path.is_empty() {
                true => Err(format!("check-config 报告的 MuMu 位置无法读取 / unreadable MuMu location: {found}")),
                false => Ok(Some((path, source))),
            }
        }
    }
}

/// The instances step's outcome as it stands at the end: the configured MuMu folder, and
/// whether a Runtime answers — asked then, not remembered.
pub struct Settled {
    pub mumu_root: Option<String>,
    pub running: Result<bool, String>,
}

pub fn settle(root: &Path, report: Report<'_>) -> Result<Settled, String> {
    let paths = paths(root)?;
    report.step(Step::Phase("确认 Runtime 状态 / Checking whether the Runtime runs", None))?;
    let mumu_root = read_config(&paths)?["mumu_root"].as_str().map(str::to_string);
    // `runtime-info.json` there without an answer may be a Runtime still
    // starting or stuck: not confirmed, rather than not running — which the
    // summary says itself, so here it is a log line, not a second note.
    let mut quiet = |line: &str| report.line(line);
    let running = match runtime::runtime_answers(&paths.actingctl, &paths.state_root, &mut quiet) {
        Ok(Ok(())) => Ok(true),
        Ok(Err(None)) => Ok(false),
        Ok(Err(Some(reason))) | Err(reason) => Err(reason),
    };
    Ok(Settled { mumu_root, running })
}

/// The emulator's instances, as the running Runtime lists them.
pub fn discover(root: &Path, report: Report<'_>) -> Result<Vec<Found>, String> {
    let paths = paths(root)?;
    report.step(Step::Phase("查找模拟器实例 / Finding the emulator's instances", None))?;
    report.line("发现模拟器实例 / discovering emulator instances")?;
    let out = runtime::run(
        Command::new(&paths.actingctl)
            .arg("emulator")
            .arg("discover")
            .arg("--state-root")
            .arg(&paths.state_root),
    )?;
    if !out.success {
        return Err(format!(
            "发现实例失败（退出码 {}）/ discovery failed (exit {}): {}\n检查 MuMu 安装目录后再试 / check the MuMu folder and try again",
            out.exit,
            out.exit,
            out.stderr.trim()
        ));
    }
    let document: Value = serde_json::from_str(out.stdout.trim()).map_err(|error| {
        format!("发现结果无法解析 / discovery output unreadable: {error}: {}", out.stdout.trim())
    })?;
    let listed = document["instances"].as_array().ok_or_else(|| {
        format!("发现结果里没有 instances / no instances in: {}", out.stdout.trim())
    })?;
    let found = listed
        .iter()
        .map(|item| {
            let index = item["instance_index"].as_u64().and_then(|index| u16::try_from(index).ok());
            let index = index.ok_or_else(|| format!("实例序号无法读取 / instance index unreadable: {item}"))?;
            let text = |key: &str| item[key].as_str().map(str::to_string);
            Ok(Found {
                index,
                name: text("instance_name").unwrap_or_default(),
                adb: match (text("adb_host"), item["adb_port"].as_u64()) {
                    (Some(host), Some(port)) => Some(format!("{host}:{port}")),
                    _ => None,
                },
                running: item["running"].as_bool().unwrap_or(false),
                bound_alias: text("bound_alias"),
                android: text("android_version"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    report.line(&format!("发现 {} 个实例 / {} instances found", found.len(), found.len()))?;
    Ok(found)
}

/// The picked instances written into the configuration, each with `package`
/// as its resource package, checked and put in place. On an error the
/// configuration is as it was.
pub fn write(root: &Path, chosen: &[Chosen], package: &Path, report: Report<'_>) -> Result<(), String> {
    let paths = paths(root)?;
    let mut blocks = Vec::new();
    for pick in chosen {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id)
            .map_err(|error| format!("系统随机源不可用 / OS RNG unavailable: {error}"))?;
        blocks.push(json!({
            "alias": pick.alias,
            "instance_id": format!("instance_{}", hex(&id)),
            "instance_index": pick.index,
            "application_id": pick.application_id,
            "touch_backend": TOUCH_BACKEND,
            "capture_backend": CAPTURE_BACKEND,
            "resource_package": package.display().to_string(),
        }));
    }
    set_config(&paths, report, |document| {
        document["instances"] = Value::Array(blocks);
    })
}

/// The Runtime restarted on the configuration `write` put in place, so it
/// binds the instances. Returns what `actingctl status` then says of them.
pub fn restart(root: &Path, report: Report<'_>) -> Result<String, String> {
    let paths = paths(root)?;
    ensure_running(root, &paths, true, report)?;
    let out = runtime::run(Command::new(&paths.actingctl).arg("status").arg("--state-root").arg(&paths.state_root))?;
    match out.success {
        true => {
            report.line("Runtime 已按新配置运行 / the Runtime runs with the new configuration")?;
            Ok(status_line(out.stdout.trim()))
        }
        false => Err(format!(
            "Runtime 已重启，但 status 未应答（退出码 {}）/ restarted, but status did not answer (exit {}): {}",
            out.exit,
            out.exit,
            out.stderr.trim()
        )),
    }
}

/// Each instance `actingctl status` lists, with whether a lease holds it — the
/// rest of its answer (every capability claim) is the console's to show.
/// Output that does not read as a status is kept whole.
fn status_line(stdout: &str) -> String {
    let document: Value = serde_json::from_str(stdout).unwrap_or_default();
    let Some(listed) = document["instances"].as_array() else {
        return stdout.to_string();
    };
    let instances: Vec<String> = listed
        .iter()
        .map(|item| {
            let lease = match item["lease_active"].as_bool() {
                Some(true) => "租约占用 / leased",
                Some(false) => "空闲 / idle",
                None => "租约未知 / lease unknown",
            };
            format!("{}（{lease}）", item["instance_alias"].as_str().unwrap_or("?"))
        })
        .collect();
    format!("{} 个实例 / instances: {}", instances.len(), instances.join("、"))
}

/// The configuration edited as a candidate next to it, checked by the
/// installed Runtime's `check-config`, and put in place only when accepted.
fn set_config(paths: &Paths, report: Report<'_>, edit: impl FnOnce(&mut Value)) -> Result<(), String> {
    let mut document = read_config(paths)?;
    report.step(Step::Phase("检查并写入配置 / Checking and writing the configuration", None))?;
    edit(&mut document);
    let mut candidate_text = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("配置序列化失败 / config serialization failed: {error}"))?;
    candidate_text.push('\n');
    let candidate = paths
        .config
        .with_file_name(format!("actingd.config.candidate-{}.json", std::process::id()));
    fs::write(&candidate, candidate_text).map_err(|error| {
        fetch::discard(&candidate, format!("写入失败 / write failed: {}: {error}", candidate.display()))
    })?;
    if let Err(reason) = runtime::check_config(&paths.actingd, &candidate) {
        let reason = format!("{reason}\n配置未改动 / the configuration was not changed");
        return Err(fetch::discard(&candidate, reason));
    }
    if let Err(error) = fs::rename(&candidate, &paths.config) {
        let reason = format!(
            "无法写入配置，配置未改动 / cannot put the configuration in place, it was not changed: {}: {error}",
            paths.config.display()
        );
        return Err(fetch::discard(&candidate, reason));
    }
    let path = paths.config.display();
    report.line(&format!("配置已更新并通过检查 / configuration updated and checked: {path}"))
        .map_err(|error| format!("{error}\n配置已替换为新内容 / the configuration was replaced: {path}"))
}

fn read_config(paths: &Paths) -> Result<Value, String> {
    let text = fs::read_to_string(&paths.config)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", paths.config.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("配置无法解析 / config unreadable: {error}"))
}

/// A Runtime running on the configuration as it now is: started when none
/// answers, and — when `restart` — one that answers is shut down first.
fn ensure_running(root: &Path, paths: &Paths, restart: bool, report: Report<'_>) -> Result<(), String> {
    match runtime::runtime_answers(&paths.actingctl, &paths.state_root, report)? {
        Ok(()) if !restart => return Ok(()),
        Ok(()) => {
            report.step(Step::Phase("关闭 Runtime / Shutting the Runtime down", None))?;
            report.line("请求 Runtime 关闭，以便按新配置重启 / asking the Runtime to shut down, to restart it on the new configuration")?;
            runtime::request_shutdown(&paths.actingctl, &paths.state_root)?;
        }
        Err(_) => {}
    }
    report.step(Step::Phase("拉起 Runtime / Starting the Runtime", None))?;
    report.line("拉起 Runtime / starting the Runtime")?;
    runtime::restart(root, &paths.actingd, &paths.config, &paths.state_root, report).map(|_| ())
}
