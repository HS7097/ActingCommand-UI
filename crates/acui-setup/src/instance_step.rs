// SPDX-License-Identifier: GPL-3.0-only
//! Step 4, on a fresh install: the instances, optional. A zero-instance
//! Runtime is started from the install root and `actingctl emulator discover`
//! lists the emulator's instances — it reads the MuMu manager's inventory and
//! starts or stops none. The instances a person picks are written into the
//! configuration as a candidate first, checked by `check-config`, and put in
//! place; the Runtime is then restarted so it binds them. A resource package
//! given as an `https://` URL is fetched into `<root>\packages\` first; the
//! Runtime only ever sees a local path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::fetch;
use crate::runtime::{self, ACTINGCTL, ACTINGD};
use crate::verify::{hex, Report};

/// The default input and capture for a MuMu instance: `nemu_ipc` capture
/// needs `mumu_root` in the configuration; without it, capture goes through
/// adb.
const TOUCH_BACKEND: &str = "adb_shell_input";
const CAPTURE_WITH_MUMU: &str = "nemu_ipc";
const CAPTURE_WITHOUT_MUMU: &str = "adb";

/// One instance the emulator reports.
pub struct Found {
    pub index: u16,
    pub name: String,
    pub adb: Option<String>,
    pub running: bool,
    pub bound_alias: Option<String>,
    pub android: Option<String>,
}

/// One instance a person picked, as they filled it in.
pub struct Chosen {
    pub index: u16,
    pub alias: String,
    pub application_id: String,
    /// A local path or an `https://` URL.
    pub package: String,
    /// Optional: the package's expected sha256.
    pub sha256: String,
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

/// `mumu_root` written when given, and a Runtime running that has read it:
/// with a folder given, one that answers is restarted — it has no instance
/// yet, and whether it read this folder cannot be asked.
pub fn start(root: &Path, mumu_root: Option<&Path>, report: Report<'_>) -> Result<(), String> {
    let paths = paths(root)?;
    if let Some(mumu) = mumu_root {
        set_config(&paths, report, |document| {
            document["mumu_root"] = json!(mumu.display().to_string());
        })?;
    }
    ensure_running(root, &paths, mumu_root.is_some(), report)
}

/// The emulator's instances, as the running Runtime lists them.
pub fn discover(root: &Path, report: Report<'_>) -> Result<Vec<Found>, String> {
    let paths = paths(root)?;
    report("发现模拟器实例 / discovering emulator instances")?;
    let out = runtime::run(
        Command::new(&paths.actingctl)
            .arg("emulator")
            .arg("discover")
            .arg("--state-root")
            .arg(&paths.state_root),
    )?;
    if !out.success {
        return Err(format!(
            "发现实例失败（退出码 {}）/ discovery failed (exit {}): {}\n填上 MuMu 安装目录再试 / fill in the MuMu folder and try again",
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
    report(&format!("发现 {} 个实例 / {} instances found", found.len(), found.len()))?;
    Ok(found)
}

/// The picked instances written into the configuration — packages given as
/// URLs fetched first — checked and put in place. On an error the
/// configuration is as it was.
pub fn write(root: &Path, chosen: &[Chosen], report: Report<'_>) -> Result<(), String> {
    let paths = paths(root)?;
    let with_mumu = {
        let text = fs::read_to_string(&paths.config)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", paths.config.display()))?;
        let document: Value = serde_json::from_str(&text)
            .map_err(|error| format!("配置无法解析 / config unreadable: {error}"))?;
        document["mumu_root"].is_string()
    };
    let mut blocks = Vec::new();
    for pick in chosen {
        let package = package_path(root, pick, report)?;
        let mut id = [0u8; 16];
        getrandom::fill(&mut id)
            .map_err(|error| format!("系统随机源不可用 / OS RNG unavailable: {error}"))?;
        blocks.push(json!({
            "alias": pick.alias,
            "instance_id": format!("instance_{}", hex(&id)),
            "instance_index": pick.index,
            "application_id": pick.application_id,
            "touch_backend": TOUCH_BACKEND,
            "capture_backend": if with_mumu { CAPTURE_WITH_MUMU } else { CAPTURE_WITHOUT_MUMU },
            "resource_package": package.display().to_string(),
        }));
    }
    set_config(&paths, report, |document| {
        document["instances"] = Value::Array(blocks);
    })
}

/// The Runtime restarted on the configuration `write` put in place, so it
/// binds the instances. Returns what `actingctl status` then says.
pub fn restart(root: &Path, report: Report<'_>) -> Result<String, String> {
    let paths = paths(root)?;
    ensure_running(root, &paths, true, report)?;
    let out = runtime::run(Command::new(&paths.actingctl).arg("status").arg("--state-root").arg(&paths.state_root))?;
    match out.success {
        true => {
            report("Runtime 已按新配置运行 / the Runtime runs with the new configuration")?;
            Ok(out.stdout.trim().to_string())
        }
        false => Err(format!(
            "Runtime 已重启，但 status 未应答（退出码 {}）/ restarted, but status did not answer (exit {}): {}",
            out.exit,
            out.exit,
            out.stderr.trim()
        )),
    }
}

/// A local package as an absolute path that exists; an `https://` one fetched
/// into `<root>\packages\`.
fn package_path(root: &Path, pick: &Chosen, report: Report<'_>) -> Result<PathBuf, String> {
    let given = pick.package.trim();
    let expected = Some(pick.sha256.trim()).filter(|sha| !sha.is_empty());
    if given.starts_with("https://") || given.starts_with("http://") {
        report(&format!("下载资源包 / fetching the resource package for {}: {given}", pick.alias))?;
        return fetch::fetch_url(given, &root.join("packages"), &format!("{}.zip", pick.index), expected, report);
    }
    let path = PathBuf::from(given);
    if !path.is_absolute() || !path.exists() {
        return Err(format!(
            "{} 的资源包路径须为存在的绝对路径 / the resource package of {} must be an existing absolute path: {given}",
            pick.alias, pick.alias
        ));
    }
    if let Some(expected) = expected {
        let actual = crate::verify::sha256_file(&path)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
        if !expected.eq_ignore_ascii_case(&actual) {
            return Err(format!(
                "{}: {} sha256 应为 / expected {expected}，实为 / actual {actual}",
                crate::verify::MISMATCH,
                path.display()
            ));
        }
    }
    Ok(path)
}

/// The configuration edited as a candidate next to it, checked by the
/// installed Runtime's `check-config`, and put in place only when accepted.
fn set_config(paths: &Paths, report: Report<'_>, edit: impl FnOnce(&mut Value)) -> Result<(), String> {
    let text = fs::read_to_string(&paths.config)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", paths.config.display()))?;
    let mut document: Value = serde_json::from_str(&text)
        .map_err(|error| format!("配置无法解析 / config unreadable: {error}"))?;
    edit(&mut document);
    let mut candidate_text = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("配置序列化失败 / config serialization failed: {error}"))?;
    candidate_text.push('\n');
    let candidate = paths
        .config
        .with_file_name(format!("actingd.config.candidate-{}.json", std::process::id()));
    fs::write(&candidate, candidate_text)
        .map_err(|error| format!("写入失败 / write failed: {}: {error}", candidate.display()))?;
    if let Err(reason) = runtime::check_config(&paths.actingd, &candidate) {
        let _ = fs::remove_file(&candidate);
        return Err(format!("{reason}\n配置未改动 / the configuration was not changed"));
    }
    fs::rename(&candidate, &paths.config).map_err(|error| {
        let _ = fs::remove_file(&candidate);
        format!("无法写入配置 / cannot put the configuration in place: {}: {error}", paths.config.display())
    })?;
    report(&format!("配置已更新并通过检查 / configuration updated and checked: {}", paths.config.display()))
}

/// A Runtime running on the configuration as it now is: started when none
/// answers, and — when `restart` — one that answers is shut down first.
fn ensure_running(root: &Path, paths: &Paths, restart: bool, report: Report<'_>) -> Result<(), String> {
    match runtime::runtime_answers(&paths.actingctl, &paths.state_root, report)? {
        Ok(()) if !restart => return Ok(()),
        Ok(()) => {
            report("请求 Runtime 关闭，以便按新配置重启 / asking the Runtime to shut down, to restart it on the new configuration")?;
            runtime::request_shutdown(&paths.actingctl, &paths.state_root)?;
        }
        Err(_) => {}
    }
    report("拉起 Runtime / starting the Runtime")?;
    runtime::restart(root, &paths.actingd, &paths.config, &paths.state_root, report).map(|_| ())
}
