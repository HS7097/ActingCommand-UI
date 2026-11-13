// SPDX-License-Identifier: GPL-3.0-only
//! The MuMu emulator instances on this machine, found before any Runtime runs: the Runtime
//! 0.12.58 has no offline discovery (`actingctl emulator discover` needs a running actingd), so
//! the wizard runs the same two read-only `MuMuManager.exe` queries the Runtime runs, `version`
//! and `info -v all`, each without a window and within 10 seconds (Runtime
//! `crates/device/src/mumu_manager.rs` `discover_mumu_instances`, `query_version`,
//! `query_instances`, `MUMU_MANAGER_COMMAND_TIMEOUT`). It never starts, stops or connects to an
//! emulator.
//!
//! `MuMuManager.exe` is found as the Runtime finds it (`resolve_mumu_manager`), the wizard's own
//! rungs first when the person named a folder: a running `MuMuNxDevice.exe` (its install root is
//! the path before `nx_device` or `nx_main`), then the registry's `MuMuPlayer*` uninstall
//! entries (`InstallLocation`, else `DisplayIcon`'s folder), then the `MuMuPlayer*` folders in
//! Program Files and Program Files (x86), each also under `Netease`; in a root,
//! `nx_main\MuMuManager.exe` (v5, v6) or `shell\MuMuManager.exe`. More than one root on one
//! rung is ambiguous, as for the Runtime.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::child::{self, OnTimeout, Spec};
use crate::platform;
use crate::text::Said;
use crate::winpath::{same_path, strip_verbatim};

pub const MANAGER_EXE: &str = "MuMuManager.exe";
/// The process the Runtime takes a running MuMu's install root from (`crates/device/src/
/// discovery.rs` `is_mumu_device_process`).
pub const DEVICE_PROCESS: &str = "MuMuNxDevice.exe";
/// The Runtime's floor for `MuMuManager version` (`MUMU_MANAGER_MINIMUM_VERSION`).
pub const MINIMUM_VERSION: [u32; 4] = [6, 3, 2, 0];
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NAME_BYTES: usize = 256;
const MAX_SHORT_TEXT_BYTES: usize = 64;

/// One instance as the window lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MumuInstance {
    pub index: u16,
    pub name: String,
    /// `is_process_started && is_android_started`.
    pub running: bool,
    pub adb_port: Option<u16>,
    pub android_version: Option<String>,
}

/// Where the MuMu install root came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MumuSource {
    Given,
    RunningProcess,
    RegistryUninstall,
    VendorEnumeration,
}

impl MumuSource {
    pub fn said(self) -> Said {
        match self {
            Self::Given => Said::new("指定的文件夹", "the folder given"),
            Self::RunningProcess => Said::new("正在运行的 MuMu", "a running MuMu"),
            Self::RegistryUninstall => Said::new("注册表卸载项", "the registry's uninstall entry"),
            Self::VendorEnumeration => Said::new("Program Files 里的 MuMu 文件夹", "the MuMu folder in Program Files"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MumuFound {
    pub root: PathBuf,
    pub manager: PathBuf,
    pub source: MumuSource,
}

/// What a discovery found: the MuMu install, its version, its instances; `problem` says why
/// any of them is missing. `absent` is the one problem that is no failure: no MuMu is installed.
#[derive(Debug, Clone)]
pub struct Discovery {
    pub absent: bool,
    pub found: Option<MumuFound>,
    pub version: Option<String>,
    pub instances: Vec<MumuInstance>,
    pub problem: Option<Said>,
}

/// `MuMuManager version`'s `{"version": "a.b.c.d"}`.
pub fn parse_version(stdout: &str) -> Result<[u32; 4], Said> {
    let unreadable = || Said::new("MuMuManager 的版本读不懂", "MuMuManager's version does not parse").with(stdout.trim());
    let document: Value = serde_json::from_str(stdout.trim_start_matches('\u{feff}').trim()).map_err(|_| unreadable())?;
    let text = document.get("version").and_then(Value::as_str).ok_or_else(unreadable)?;
    let mut parts = [0u32; 4];
    let mut count = 0;
    for part in text.split('.') {
        if count == 4 || part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(unreadable());
        }
        parts[count] = part.parse().map_err(|_| unreadable())?;
        count += 1;
    }
    if count != 4 {
        return Err(unreadable());
    }
    Ok(parts)
}

/// The version as people read it.
pub fn version_text(version: [u32; 4]) -> String {
    format!("{}.{}.{}.{}", version[0], version[1], version[2], version[3])
}

/// Why `version` is too old for the Runtime, or `None`.
pub fn version_problem(version: [u32; 4]) -> Option<Said> {
    (version < MINIMUM_VERSION).then(|| {
        Said::new(
            format!("MuMu 版本 {} 低于 Runtime 要求的 {}", version_text(version), version_text(MINIMUM_VERSION)),
            format!(
                "MuMu {} is older than the {} the Runtime requires",
                version_text(version),
                version_text(MINIMUM_VERSION)
            ),
        )
    })
}

fn errcode(object: &Map<String, Value>) -> Option<String> {
    let code = object.get("errcode")?;
    if code.as_i64() == Some(0) {
        return None;
    }
    Some(format!(
        "errcode={code} errmsg={}",
        object.get("errmsg").map_or_else(|| "<absent>".to_owned(), Value::to_string)
    ))
}

/// The instances of `MuMuManager info -v all`'s stdout, by index (Runtime `mumu_manager.rs`
/// `query_instances` and `parse_instance`): an object keyed by index, or one flat instance; each
/// with its `index` (a string, equal to its key), `name`, `is_process_started` and
/// `is_android_started`; a running instance also with `adb_host_ip` and a non-zero `adb_port`;
/// an `errcode` other than 0 fails.
pub fn parse_info_all(stdout: &str) -> Result<Vec<MumuInstance>, Said> {
    let shape = |detail: String| {
        Said::new("MuMuManager info -v all 的输出读不懂", "MuMuManager info -v all printed what the wizard cannot read")
            .with(detail)
    };
    let document: Value = serde_json::from_str(stdout.trim_start_matches('\u{feff}').trim())
        .map_err(|error| shape(format!("not JSON: {error}")))?;
    let Some(object) = document.as_object() else {
        return Err(shape("not an object keyed by instance index".to_owned()));
    };
    if let Some(envelope) = errcode(object) {
        return Err(shape(envelope));
    }
    let mut instances = Vec::new();
    if object.contains_key("index") {
        instances.push(parse_instance(None, object).map_err(shape)?);
    } else {
        for (key, value) in object {
            let entry = value
                .as_object()
                .ok_or_else(|| shape(format!("entry {key:?} is not an object")))?;
            if let Some(envelope) = errcode(entry) {
                return Err(shape(format!("entry {key:?}: {envelope}")));
            }
            instances.push(parse_instance(Some(key.as_str()), entry).map_err(shape)?);
        }
    }
    instances.sort_by_key(|instance| instance.index);
    Ok(instances)
}

fn parse_instance(key: Option<&str>, entry: &Map<String, Value>) -> Result<MumuInstance, String> {
    let label = key.map_or_else(|| "<flat>".to_owned(), |key| format!("{key:?}"));
    let field = |name: &str| entry.get(name).ok_or_else(|| format!("entry {label} lacks {name:?}"));
    let text = |name: &str, max: usize| -> Result<String, String> {
        field(name)?
            .as_str()
            .filter(|text| !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control))
            .map(str::to_owned)
            .ok_or_else(|| format!("entry {label} {name:?} is no short text"))
    };
    let boolean = |name: &str| -> Result<bool, String> {
        field(name)?
            .as_bool()
            .ok_or_else(|| format!("entry {label} {name:?} is no boolean"))
    };
    let index = field("index")?
        .as_str()
        .and_then(|text| text.parse::<u16>().ok())
        .ok_or_else(|| format!("entry {label} \"index\" is no 16-bit index string"))?;
    if let Some(key) = key {
        if key != index.to_string() {
            return Err(format!("entry {label} \"index\" {index} differs from its key"));
        }
    }
    let name = text("name", MAX_NAME_BYTES)?;
    let process_started = boolean("is_process_started")?;
    let android_started = boolean("is_android_started")?;
    let running = process_started && android_started;
    let reported = |name: &str| running || entry.get(name).is_some_and(|value| !value.is_null());
    if reported("adb_host_ip") {
        text("adb_host_ip", MAX_SHORT_TEXT_BYTES)?;
    }
    let adb_port = if reported("adb_port") {
        Some(
            field("adb_port")?
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| format!("entry {label} \"adb_port\" is no non-zero port"))?,
        )
    } else {
        None
    };
    if reported("player_state") {
        text("player_state", MAX_SHORT_TEXT_BYTES)?;
    }
    let android_version = entry
        .get("android_version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_owned);
    Ok(MumuInstance {
        index,
        name,
        running,
        adb_port,
        android_version,
    })
}

/// The MuMu install root of a running MuMu program's path: the path before its `nx_device` or
/// `nx_main` folder (Runtime `crates/device/src/mumu.rs` `mumu_root_from_path`).
pub fn root_from_process_path(path: &str) -> Option<String> {
    let path = strip_verbatim(path);
    let mut kept: Vec<&str> = Vec::new();
    for segment in path.split(['\\', '/']) {
        if segment.eq_ignore_ascii_case("nx_device") || segment.eq_ignore_ascii_case("nx_main") {
            let root = kept.join("\\");
            return (!root.is_empty() && kept.iter().any(|segment| !segment.is_empty())).then_some(root);
        }
        kept.push(segment);
    }
    None
}

/// Whether a folder name is a MuMu install's (Runtime `mumu.rs` `is_mumu_install_name`).
pub fn is_install_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("mumu player") || lower.starts_with("mumuplayer-") || lower == "mumuplayer"
}

/// The install root an uninstall entry names: `InstallLocation` trimmed and unquoted, else
/// `DisplayIcon`'s folder (its `,<index>` suffix dropped) (Runtime `mumu_manager.rs`
/// `install_root_from`).
pub fn registry_root(install_location: Option<&str>, display_icon: Option<&str>) -> Option<String> {
    if let Some(location) = install_location
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
    {
        return Some(location);
    }
    let icon = display_icon?.trim().trim_matches('"');
    let icon = match icon.rsplit_once(',') {
        Some((path, suffix)) if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) => path,
        _ => icon,
    };
    let icon = icon.trim_matches('"');
    let (folder, _) = icon.rsplit_once(['\\', '/'])?;
    (!folder.is_empty()).then(|| folder.to_owned())
}

/// `MuMuManager.exe` in an install root: `nx_main\` first, then `shell\`.
pub fn manager_in(root: &Path) -> Option<PathBuf> {
    [root.join("nx_main").join(MANAGER_EXE), root.join("shell").join(MANAGER_EXE)]
        .into_iter()
        .find(|path| path.is_file())
}

/// The one root of `roots` (folded as Windows folds paths) that holds a manager, or `None`
/// when none does; more than one is ambiguous.
fn unique(roots: Vec<String>, source: MumuSource, log: &mut dyn FnMut(Said)) -> Result<Option<MumuFound>, Said> {
    let mut kept: Vec<(String, PathBuf)> = Vec::new();
    for root in roots {
        if kept.iter().any(|(seen, _)| same_path(seen, &root)) {
            continue;
        }
        match manager_in(Path::new(&root)) {
            Some(manager) => kept.push((root, manager)),
            None => log(Said::new("这个 MuMu 文件夹里没有 MuMuManager.exe", "this MuMu folder holds no MuMuManager.exe")
                .with(format!("{} ({})", root, source.said().en))),
        }
    }
    match kept.len() {
        0 => Ok(None),
        1 => {
            let (root, manager) = kept.remove(0);
            Ok(Some(MumuFound { root: PathBuf::from(root), manager, source }))
        }
        _ => Err(Said::new(
            "找到不止一套 MuMu，请指定要用的那一套的文件夹",
            "more than one MuMu installation was found; name the folder of the one to use",
        )
        .with(kept.iter().map(|(root, _)| root.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

/// Finds `MuMuManager.exe` (see the module's description); `given` is a folder the person named.
pub fn locate(given: Option<&Path>, log: &mut dyn FnMut(Said)) -> Result<Option<MumuFound>, Said> {
    if let Some(root) = given {
        return match manager_in(root) {
            Some(manager) => Ok(Some(MumuFound { root: root.to_path_buf(), manager, source: MumuSource::Given })),
            None => Err(Said::new("指定的 MuMu 文件夹里没有 MuMuManager.exe", "the MuMu folder given holds no MuMuManager.exe")
                .with(root.display())),
        };
    }
    let mut running = Vec::new();
    match platform::processes_named(DEVICE_PROCESS) {
        Ok(processes) => {
            for process in processes {
                match &process.path {
                    Ok(path) => {
                        if let Some(root) = root_from_process_path(&path.to_string_lossy()) {
                            running.push(root);
                        }
                    }
                    Err(reason) => log(Said::new("读不到正在运行的 MuMu 的路径", "the path of a running MuMu cannot be read")
                        .with(format!("pid {}: {reason}", process.pid))),
                }
            }
        }
        Err(problem) => log(problem),
    }
    if let Some(found) = unique(running, MumuSource::RunningProcess, log)? {
        return Ok(Some(found));
    }
    let mut registered = Vec::new();
    match platform::uninstall_entries("mumuplayer") {
        Ok(entries) => {
            for entry in entries {
                match registry_root(entry.install_location.as_deref(), entry.display_icon.as_deref()) {
                    Some(root) => registered.push(root),
                    None => log(Said::new("MuMu 的卸载项没写安装位置", "a MuMu uninstall entry names no install folder")
                        .with(&entry.key_path)),
                }
            }
        }
        Err(problem) => log(problem),
    }
    if let Some(found) = unique(registered, MumuSource::RegistryUninstall, log)? {
        return Ok(Some(found));
    }
    let mut vendor = Vec::new();
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        let Some(base) = std::env::var_os(variable).map(PathBuf::from) else {
            continue;
        };
        for parent in [base.clone(), base.join("Netease")] {
            let entries = match std::fs::read_dir(&parent) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    log(Said::new("文件夹读不出", "a folder cannot be read")
                        .with(format!("{}: {error}", parent.display())));
                    continue;
                }
            };
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if is_install_name(&name) && entry.path().is_dir() {
                            vendor.push(entry.path().to_string_lossy().into_owned());
                        }
                    }
                    Err(error) => log(Said::new("文件夹读不出", "a folder cannot be read")
                        .with(format!("{}: {error}", parent.display()))),
                }
            }
        }
    }
    unique(vendor, MumuSource::VendorEnumeration, log)
}

fn query(manager: &Path, args: &[&str]) -> Result<String, Said> {
    let mut spec = Spec::new(manager);
    for arg in args {
        spec = spec.arg(*arg);
    }
    let output = child::run(&spec, QUERY_TIMEOUT, OnTimeout::End)?;
    if output.code != Some(0) {
        return Err(Said::new("MuMuManager 报错", "MuMuManager failed").with(format!(
            "{} → exit {}: {} {}",
            spec.text(),
            output.code_text(),
            child::clipped(&output.stdout_text(), 400),
            child::clipped(&output.stderr_text(), 400)
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| Said::new("MuMuManager 的输出不是 UTF-8", "MuMuManager's output is not UTF-8").with(spec.text()))
}

/// Finds MuMu and lists its instances, read only (see the module's description).
pub fn discover(given: Option<&Path>, log: &mut dyn FnMut(Said)) -> Discovery {
    let mut discovery = Discovery {
        absent: false,
        found: None,
        version: None,
        instances: Vec::new(),
        problem: None,
    };
    let found = match locate(given, log) {
        Ok(Some(found)) => found,
        Ok(None) => {
            discovery.absent = true;
            discovery.problem = Some(Said::new(
                "没找到 MuMu 模拟器；可以先装成没有实例的控制面，以后再加",
                "No MuMu emulator was found; the install can go on without instances and add them later",
            ));
            return discovery;
        }
        Err(problem) => {
            discovery.problem = Some(problem);
            return discovery;
        }
    };
    log(Said::new("找到 MuMuManager", "MuMuManager found").with(format!(
        "{} ({})",
        found.manager.display(),
        found.source.said().en
    )));
    let version = match query(&found.manager, &["version"]).and_then(|stdout| parse_version(&stdout)) {
        Ok(version) => version,
        Err(problem) => {
            discovery.found = Some(found);
            discovery.problem = Some(problem);
            return discovery;
        }
    };
    discovery.version = Some(version_text(version));
    if let Some(problem) = version_problem(version) {
        discovery.found = Some(found);
        discovery.problem = Some(problem);
        return discovery;
    }
    match query(&found.manager, &["info", "-v", "all"]).and_then(|stdout| parse_info_all(&stdout)) {
        Ok(instances) => discovery.instances = instances,
        Err(problem) => discovery.problem = Some(problem),
    }
    discovery.found = Some(found);
    discovery
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two shapes Runtime crates/device/src/tests/gate_381b.rs:288-358 reads (observed on
    // MuMu 6.5.7.0): a running instance with its adb endpoint and player state, a stopped one
    // without them.
    const RUNNING: &str = r#"{"index": "0", "name": "fixture", "is_process_started": true,
        "is_android_started": true, "adb_host_ip": "127.0.0.1", "adb_port": 16384,
        "player_state": "start_finished"}"#;
    const STOPPED: &str = r#"{"android_version": "12", "created_timestamp": 1, "disk_size_bytes": 1,
        "error_code": 0, "hyperv_enabled": false, "index": "5", "info_source": "rpc",
        "is_android_started": false, "is_main": false, "is_process_started": false, "name": "实验五"}"#;

    #[test]
    fn info_all_is_read_in_its_map_shape_sorted_by_index() {
        let stdout = format!("\u{feff}{{\"5\": {STOPPED}, \"0\": {RUNNING}}}\r\n");
        let instances = parse_info_all(&stdout).expect("two instances");
        assert_eq!(
            instances,
            vec![
                MumuInstance {
                    index: 0,
                    name: "fixture".into(),
                    running: true,
                    adb_port: Some(16384),
                    android_version: None
                },
                MumuInstance {
                    index: 5,
                    name: "实验五".into(),
                    running: false,
                    adb_port: None,
                    android_version: Some("12".into())
                },
            ]
        );
        // One instance alone may come flat.
        assert_eq!(parse_info_all(RUNNING).expect("one").len(), 1);
        // Started but Android not yet up is not running; its null port is not reported.
        let booting = r#"{"index": "0", "name": "b", "is_process_started": true,
            "is_android_started": false, "adb_port": null, "player_state": null}"#;
        assert!(!parse_info_all(booting).expect("booting")[0].running);
    }

    #[test]
    fn what_the_runtime_refuses_the_wizard_refuses() {
        for refused in [
            r#"{"index": "0", "name": "x", "is_android_started": false}"#,
            r#"{"index": "0", "name": "x", "is_process_started": false}"#,
            r#"{"name": "x", "is_process_started": false, "is_android_started": false}"#,
            r#"{"0": {"index": "1", "name": "x", "is_process_started": false, "is_android_started": false}}"#,
            r#"{"index": "0", "name": "x", "is_process_started": "false", "is_android_started": false}"#,
            r#"{"index": "0", "name": "", "is_process_started": false, "is_android_started": false}"#,
            r#"{"index": "0", "name": "x", "is_process_started": true, "is_android_started": true}"#,
            r#"{"index": "0", "name": "x", "is_process_started": true, "is_android_started": true,
                "adb_host_ip": "127.0.0.1", "adb_port": 0, "player_state": "s"}"#,
            r#"{"errcode": 1, "errmsg": "refused"}"#,
            r#"{"0": {"errcode": -1}}"#,
            r#"[1, 2]"#,
            "not json",
        ] {
            assert!(parse_info_all(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_version_floor_is_the_runtimes() {
        assert_eq!(parse_version("{\"version\": \"6.5.7.0\"}\n"), Ok([6, 5, 7, 0]));
        assert!(parse_version("{\"version\": \"6.5.7\"}").is_err());
        assert!(parse_version("{\"version\": \"6.5.x.0\"}").is_err());
        assert!(parse_version("{}").is_err());
        assert_eq!(version_problem([6, 5, 7, 0]), None);
        assert_eq!(version_problem([6, 3, 2, 0]), None);
        assert!(version_problem([6, 3, 1, 9]).is_some());
        assert!(version_problem([5, 9, 0, 0]).is_some());
    }

    #[test]
    fn install_roots_come_from_process_paths_registry_entries_and_folder_names() {
        assert_eq!(
            root_from_process_path(r"C:\Program Files\Netease\MuMuPlayer\nx_device\12.0\shell\MuMuNxDevice.exe")
                .as_deref(),
            Some(r"C:\Program Files\Netease\MuMuPlayer")
        );
        assert_eq!(
            root_from_process_path(r"\\?\D:\MuMu\NX_MAIN\MuMuNxDevice.exe").as_deref(),
            Some(r"D:\MuMu")
        );
        assert_eq!(root_from_process_path(r"D:\Other\MuMuNxDevice.exe"), None);
        assert_eq!(root_from_process_path(r"nx_device\MuMuNxDevice.exe"), None);
        assert_eq!(
            registry_root(Some(" \"C:\\Program Files\\Netease\\MuMuPlayer\" "), Some("x")).as_deref(),
            Some("C:\\Program Files\\Netease\\MuMuPlayer")
        );
        assert_eq!(
            registry_root(Some(""), Some("\"D:\\MuMuPlayer-12.0\\uninstall.exe\",0")).as_deref(),
            Some("D:\\MuMuPlayer-12.0")
        );
        assert_eq!(registry_root(None, Some("uninstall.exe")), None);
        assert_eq!(registry_root(None, None), None);
        for name in ["MuMuPlayer", "MuMuPlayer-12.0", "MuMu Player 12", "mumuplayer"] {
            assert!(is_install_name(name), "{name}");
        }
        for name in ["MuMuPlayerGlobal-12.0", "MuMu", "Netease"] {
            assert!(!is_install_name(name), "{name}");
        }
    }
}
