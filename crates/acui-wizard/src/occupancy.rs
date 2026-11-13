// SPDX-License-Identifier: GPL-3.0-only
//! Which emulators another ActingCommand on this machine already uses, so the window can mark
//! them and ask before one is taken. The other roots are found two ways:
//!
//! - each running `actingcommand-actingd.exe` names its root by its own path
//!   (`<root>\runtime\actingcommand-actingd.exe`, the root holding `runtime\BUILD-MANIFEST.json`:
//!   Runtime `apps/actingd/src/config/ac_adb.rs`, the install root);
//! - each registered watchdog task names its root by its action, `<root>\tools\actingwatch.exe`
//!   (Runtime `contracts/runtime-watchdog.md`, "Task": `ActingCommand Runtime watchdog <fp12>` in
//!   the task folder `\`), so a root whose Runtime is stopped is found too. The tasks are read as
//!   the Runtime reads them, through the ScheduledTasks module in a hidden Windows PowerShell
//!   that answers in UTF-8 JSON, read only.
//!
//! Each root's `actingd.config.json` is read, nothing else, and only its instances' binding keys:
//! `instance_index`, `instance_name`, `serial`, `host` and `port`. An emulator is in use when one
//! of them names it: by its index, by its name, or by its adb port (a loopback `serial`
//! `127.0.0.1:<port>` or `emulator-<port - 1>`, or a loopback `host` with `port`). What cannot be
//! read is not known: it is said and logged, and nothing stops.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::mumu::MumuInstance;
use crate::platform;
use crate::text::Said;
use crate::winpath::{drive_absolute, same_path, strip_verbatim};

pub const ACTINGD_EXE: &str = "actingcommand-actingd.exe";
/// The watchdog's launcher, the program its task runs (`contracts/runtime-watchdog.md`,
/// "Launcher").
pub const WATCHDOG_EXE: &str = "actingwatch.exe";
/// The start of every watchdog task's name.
pub const WATCHDOG_TASK_PREFIX: &str = "ActingCommand Runtime watchdog ";
const CONFIG_LIMIT: u64 = 1 << 20;
const TASK_QUERY_TIMEOUT: Duration = Duration::from_secs(60);

/// An instance another root's configuration binds, by the keys it binds it with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsedInstance {
    pub alias: String,
    pub index: Option<u16>,
    pub name: Option<String>,
    pub serial: Option<String>,
    /// The adb port its `serial`, or its `host` and `port`, name on this machine.
    pub port: Option<u16>,
}

/// Another installation: the running process that named it (none when only its watchdog task
/// did), its root when known, the instances it binds when its configuration could be read, and
/// why not otherwise.
#[derive(Debug, Clone)]
pub struct OtherRoot {
    pub pid: Option<u32>,
    pub root: Option<PathBuf>,
    pub instances: Option<Vec<UsedInstance>>,
    pub unknown: Option<Said>,
}

impl OtherRoot {
    /// How a person tells it: its root, else its process.
    pub fn name(&self) -> String {
        match (&self.root, self.pid) {
            (Some(root), _) => root.display().to_string(),
            (None, Some(pid)) => format!("pid {pid}"),
            (None, None) => "?".to_owned(),
        }
    }
}

/// The root of an actingd at `exe`: the folder holding its `runtime\` folder.
pub fn root_of_actingd(exe: &str) -> Option<String> {
    root_of_program(exe, "runtime", ACTINGD_EXE)
}

/// The root of a watchdog task's action `execute`, `<root>\tools\actingwatch.exe`, quoted or
/// not.
pub fn root_of_watchdog_action(execute: &str) -> Option<String> {
    let execute = execute.trim();
    let execute = execute
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(execute);
    root_of_program(execute, "tools", WATCHDOG_EXE)
        .filter(|root| drive_absolute(root) || root.starts_with("\\\\"))
}

/// `<root>` of `<root>\<folder>\<program>`, any case, either separator.
fn root_of_program(path: &str, folder: &str, program: &str) -> Option<String> {
    let path = strip_verbatim(path);
    let (parent, file) = path.rsplit_once(['\\', '/'])?;
    if !file.eq_ignore_ascii_case(program) {
        return None;
    }
    let (root, named) = parent.rsplit_once(['\\', '/'])?;
    (named.eq_ignore_ascii_case(folder) && !root.is_empty()).then(|| root.to_owned())
}

/// One registered watchdog task as the query answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchdogTask {
    pub name: String,
    /// Each action's program and working directory.
    pub actions: Vec<(String, String)>,
}

impl WatchdogTask {
    /// The root its action names, else its working directory when that is an absolute path.
    pub fn root(&self) -> Option<String> {
        self.actions.iter().find_map(|(execute, workdir)| {
            root_of_watchdog_action(execute).or_else(|| {
                let workdir = strip_verbatim(workdir.trim());
                let named = execute.to_ascii_lowercase().trim_matches('"').ends_with(WATCHDOG_EXE);
                (named && drive_absolute(&workdir)).then(|| workdir.trim_end_matches(['\\', '/']).to_owned())
            })
        })
    }
}

/// The tasks in the query's answer (a JSON array of `{name, actions: [{execute,
/// working_directory}]}`, or one such object; a byte-order mark allowed; nothing at all is no
/// task); only those whose name begins with [`WATCHDOG_TASK_PREFIX`].
pub fn parse_watchdog_tasks(stdout: &str) -> Result<Vec<WatchdogTask>, Said> {
    let text = stdout.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let unreadable = || {
        Said::new("看门狗计划任务的列表读不懂", "the list of watchdog tasks does not parse")
            .with(crate::child::clipped(text, 300))
    };
    let document: Value = serde_json::from_str(text).map_err(|_| unreadable())?;
    let rows = match &document {
        Value::Array(rows) => rows.clone(),
        Value::Object(_) => vec![document.clone()],
        Value::Null => Vec::new(),
        _ => return Err(unreadable()),
    };
    let mut tasks = Vec::new();
    for row in rows {
        let name = row["name"].as_str().ok_or_else(unreadable)?.to_owned();
        if !name.starts_with(WATCHDOG_TASK_PREFIX) {
            continue;
        }
        let actions = match &row["actions"] {
            Value::Array(actions) => actions.clone(),
            Value::Null => Vec::new(),
            action @ Value::Object(_) => vec![action.clone()],
            _ => return Err(unreadable()),
        };
        tasks.push(WatchdogTask {
            name,
            actions: actions
                .iter()
                .map(|action| {
                    (
                        action["execute"].as_str().unwrap_or_default().to_owned(),
                        action["working_directory"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
        });
    }
    Ok(tasks)
}

/// The adb port a `serial` names on this machine: `<loopback>:<port>`, or `emulator-<n>` whose
/// adb port is `n + 1`; `None` for any other.
pub fn serial_port(serial: &str) -> Option<u16> {
    let serial = serial.trim();
    if let Some(console) = serial.strip_prefix("emulator-") {
        return console.parse::<u16>().ok()?.checked_add(1);
    }
    let (host, port) = serial.rsplit_once(':')?;
    loopback(host).then(|| port.parse().ok()).flatten()
}

fn loopback(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]"
    )
}

/// The instances a configuration's bytes bind; members it does not know are read past.
pub fn used_instances(bytes: &[u8]) -> Result<Vec<UsedInstance>, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let document: Value = serde_json::from_slice(bytes).map_err(|error| {
        Said::new("另一套的配置读不懂", "the other installation's configuration does not parse").with(error)
    })?;
    let entries = document
        .get("instances")
        .and_then(Value::as_array)
        .ok_or_else(|| Said::new("另一套的配置里没有 instances", "the other installation's configuration has no instances"))?;
    Ok(entries
        .iter()
        .map(|entry| {
            let serial = entry["serial"].as_str().map(str::to_owned);
            let host_port = entry["port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|_| entry["host"].as_str().is_none_or(loopback));
            UsedInstance {
                alias: entry["alias"].as_str().unwrap_or_default().to_owned(),
                index: entry["instance_index"].as_u64().and_then(|index| u16::try_from(index).ok()),
                name: entry["instance_name"].as_str().map(str::to_owned),
                port: serial.as_deref().and_then(serial_port).or(host_port),
                serial,
            }
        })
        .collect())
}

/// Whether `used` names `instance`: by its index, its exact name or its adb port.
pub fn binds(used: &UsedInstance, instance: &MumuInstance) -> bool {
    used.index == Some(instance.index)
        || used.name.as_deref() == Some(instance.name.as_str())
        || (used.port.is_some() && used.port == instance.adb_port)
}

/// The other root that binds `instance` (see [`binds`]), and the alias it has there.
pub fn used_by<'a>(roots: &'a [OtherRoot], instance: &MumuInstance) -> Option<(&'a OtherRoot, &'a str)> {
    roots.iter().find_map(|root| {
        root.instances
            .as_ref()?
            .iter()
            .find(|used| binds(used, instance))
            .map(|used| (root, used.alias.as_str()))
    })
}

/// Where an installation was seen: by its process (with the root its path names, or why none),
/// or by its watchdog task.
#[derive(Debug, Clone)]
pub struct Sighting {
    pub pid: Option<u32>,
    pub root: Result<String, Said>,
}

/// One entry per installation: sightings of one root (compared as Windows compares paths)
/// become one, with the process of the first that has one; a process whose root is not known
/// stays an entry of its own.
pub fn merge_sightings(sightings: Vec<Sighting>) -> Vec<Sighting> {
    let mut merged: Vec<Sighting> = Vec::new();
    for sighting in sightings {
        let same = match &sighting.root {
            Ok(root) => merged
                .iter()
                .position(|known| known.root.as_ref().is_ok_and(|known| same_path(known, root))),
            Err(_) => None,
        };
        match same {
            Some(at) => merged[at].pid = merged[at].pid.or(sighting.pid),
            None => merged.push(sighting),
        }
    }
    merged
}

/// Every other installation with what its root binds (see the module's description). `log`
/// hears each thing that could not be read.
pub fn scan(log: &mut dyn FnMut(Said)) -> Vec<OtherRoot> {
    let mut sightings = Vec::new();
    match platform::processes_named(ACTINGD_EXE) {
        Ok(processes) => {
            for process in processes {
                let root = match &process.path {
                    Err(reason) => Err(Said::new("读不到它的程序路径", "its program path cannot be read").with(reason)),
                    Ok(path) => root_of_actingd(&path.to_string_lossy()).ok_or_else(|| {
                        Said::new("它不在一个安装根的 runtime 文件夹里", "it does not run from an installation root's runtime folder")
                            .with(path.display())
                    }),
                };
                sightings.push(Sighting { pid: Some(process.pid), root });
            }
        }
        Err(problem) => {
            log(Said::new("读不到正在运行的 ActingCommand", "the running ActingCommand cannot be listed").with(problem));
        }
    }
    match watchdog_tasks() {
        Ok(tasks) => {
            for task in tasks {
                match task.root() {
                    Some(root) => sightings.push(Sighting { pid: None, root: Ok(root) }),
                    None => log(
                        Said::new("看门狗计划任务没有指向一个安装根", "a watchdog task names no installation root")
                            .with(&task.name),
                    ),
                }
            }
        }
        Err(problem) => {
            log(Said::new("读不到 ActingCommand 的看门狗计划任务", "the ActingCommand watchdog tasks cannot be listed").with(problem));
        }
    }
    let mut roots = Vec::new();
    for sighting in merge_sightings(sightings) {
        let mut other = OtherRoot {
            pid: sighting.pid,
            root: None,
            instances: None,
            unknown: None,
        };
        let unknown = match sighting.root {
            Err(reason) => Some(reason),
            Ok(root) => {
                let root = PathBuf::from(root);
                let found = read_root(&root);
                other.root = Some(root);
                match found {
                    Ok(instances) => {
                        other.instances = Some(instances);
                        None
                    }
                    Err(reason) => Some(reason),
                }
            }
        };
        if let Some(reason) = unknown {
            log(Said::new(
                format!("不知道另一套 ActingCommand（{}）用着哪些模拟器", other.name()),
                format!("which emulators the ActingCommand {} uses is not known", other.name()),
            )
            .with(&reason));
            other.unknown = Some(reason);
        }
        roots.push(other);
    }
    roots
}

/// The query of the registered watchdog tasks: one line for `powershell.exe -Command`, single
/// quotes only, answering a JSON array on stdout in UTF-8.
const TASK_QUERY: &str = "$ErrorActionPreference = 'Stop'; \
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false; \
    $rows = @(Get-ScheduledTask -TaskPath '\\' | Where-Object { $_.TaskName -like 'ActingCommand Runtime watchdog *' } | \
    ForEach-Object { [pscustomobject]@{ name = [string]$_.TaskName; actions = @($_.Actions | \
    ForEach-Object { [pscustomobject]@{ execute = [string]$_.Execute; working_directory = [string]$_.WorkingDirectory } }) } }); \
    ConvertTo-Json -InputObject $rows -Depth 4 -Compress";

/// The registered watchdog tasks of this user, read only (see the module's description).
fn watchdog_tasks() -> Result<Vec<WatchdogTask>, Said> {
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let powershell = system
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if !cfg!(windows) {
        return Err(Said::new("只在 Windows 上可用", "available on Windows only"));
    }
    let spec = crate::child::Spec::new(&powershell)
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(TASK_QUERY);
    let output = crate::child::run(&spec, TASK_QUERY_TIMEOUT, crate::child::OnTimeout::End)?;
    if output.code != Some(0) {
        return Err(Said::new("查询计划任务的程序报错", "the scheduled task query failed").with(format!(
            "exit {}: {}",
            output.code_text(),
            crate::child::clipped(&output.stderr_text(), 600)
        )));
    }
    parse_watchdog_tasks(&output.stdout_text())
}

fn read_root(root: &Path) -> Result<Vec<UsedInstance>, Said> {
    if !root.join("runtime").join("BUILD-MANIFEST.json").is_file() {
        return Err(Said::new("那里不是单位置安装根", "it is no single-location root").with(root.display()));
    }
    if root.join("install").join("active.json").is_file() {
        return Err(Said::new("那是 A/B 安装，配置不在根下", "it is an A/B installation, whose configuration is not at the root")
            .with(root.display()));
    }
    let path = root.join("actingd.config.json");
    let unreadable = |error: &dyn std::fmt::Display| {
        Said::new("另一套的配置读不出", "the other installation's configuration cannot be read")
            .with(format!("{}: {error}", path.display()))
    };
    let file = fs::File::open(&path).map_err(|error| unreadable(&error))?;
    let mut bytes = Vec::new();
    file.take(CONFIG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(&error))?;
    if bytes.len() as u64 > CONFIG_LIMIT {
        return Err(unreadable(&"larger than 1 MiB"));
    }
    used_instances(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_actingd_names_its_root_by_its_path() {
        assert_eq!(
            root_of_actingd(r"X:\Prod\runtime\actingcommand-actingd.exe").as_deref(),
            Some(r"X:\Prod")
        );
        assert_eq!(
            root_of_actingd(r"\\?\D:\Test Root\Runtime\ActingCommand-Actingd.EXE").as_deref(),
            Some(r"D:\Test Root")
        );
        assert_eq!(root_of_actingd(r"X:\Other\install\builds\c\actingctl.exe"), None);
        assert_eq!(root_of_actingd(r"X:\Other\bin\actingcommand-actingd.exe"), None);
        assert_eq!(root_of_actingd(r"runtime\actingcommand-actingd.exe"), None);
    }

    #[test]
    fn a_watchdog_task_names_its_root_by_its_action() {
        assert_eq!(root_of_watchdog_action(r"X:\Root\tools\actingwatch.exe").as_deref(), Some(r"X:\Root"));
        assert_eq!(root_of_watchdog_action(r#""X:\Root Two\Tools\ActingWatch.EXE""#).as_deref(), Some(r"X:\Root Two"));
        assert_eq!(root_of_watchdog_action(r"X:\Root\runtime\actingwatch.exe"), None);
        assert_eq!(root_of_watchdog_action(r"X:\Root\tools\actingctl.exe"), None);
        assert_eq!(root_of_watchdog_action(r"tools\actingwatch.exe"), None);
        assert_eq!(root_of_watchdog_action(r"Root\tools\actingwatch.exe"), None);
        let task = |execute: &str, workdir: &str| WatchdogTask {
            name: format!("{WATCHDOG_TASK_PREFIX}0123456789ab"),
            actions: vec![(execute.to_owned(), workdir.to_owned())],
        };
        assert_eq!(task(r"X:\Root\tools\actingwatch.exe", r"X:\Root").root().as_deref(), Some(r"X:\Root"));
        // An action of another spelling falls back on its working directory.
        assert_eq!(task(r"%ROOT%\tools\actingwatch.exe", r"X:\Root\").root().as_deref(), Some(r"X:\Root"));
        assert_eq!(task(r"X:\Other\program.exe", r"X:\Root").root(), None);
        assert_eq!(task(r"actingwatch.exe", "relative").root(), None);
    }

    #[test]
    fn the_task_query_answer_is_read_as_the_tasks_it_lists() {
        let answer = "\u{feff}[{\"name\":\"ActingCommand Runtime watchdog 0123456789ab\",\"actions\":[{\"execute\":\"X:\\\\Root\\\\tools\\\\actingwatch.exe\",\"working_directory\":\"X:\\\\Root\"}]},\
            {\"name\":\"Another task\",\"actions\":[]},\
            {\"name\":\"ActingCommand Runtime watchdog ba9876543210\",\"actions\":{\"execute\":\"Y:\\\\AC\\\\tools\\\\actingwatch.exe\",\"working_directory\":\"\"}}]\r\n";
        let tasks = parse_watchdog_tasks(answer).expect("tasks");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].root().as_deref(), Some(r"X:\Root"));
        assert_eq!(tasks[1].root().as_deref(), Some(r"Y:\AC"));
        assert_eq!(parse_watchdog_tasks("").unwrap(), Vec::new());
        assert_eq!(parse_watchdog_tasks("[]").unwrap(), Vec::new());
        // One task alone may come as an object.
        let one = parse_watchdog_tasks(
            "{\"name\":\"ActingCommand Runtime watchdog 0123456789ab\",\"actions\":[{\"execute\":\"X:\\\\R\\\\tools\\\\actingwatch.exe\"}]}",
        )
        .unwrap();
        assert_eq!(one[0].root().as_deref(), Some(r"X:\R"));
        assert!(parse_watchdog_tasks("not json").is_err());
        assert!(parse_watchdog_tasks("[{\"actions\":[]}]").is_err());
        assert!(parse_watchdog_tasks("3").is_err());
    }

    #[test]
    fn a_serial_names_an_adb_port_on_this_machine() {
        assert_eq!(serial_port("127.0.0.1:16416"), Some(16416));
        assert_eq!(serial_port("localhost:5555"), Some(5555));
        assert_eq!(serial_port("[::1]:7555"), Some(7555));
        assert_eq!(serial_port("emulator-5554"), Some(5555));
        assert_eq!(serial_port("192.168.1.9:5555"), None);
        assert_eq!(serial_port("R58M123ABC"), None);
        assert_eq!(serial_port("127.0.0.1:port"), None);
        assert_eq!(serial_port("emulator-65535"), None);
    }

    #[test]
    fn a_configuration_binds_by_index_name_serial_or_port() {
        let config = br#"{"schema_version":"actingcommand.actingd.config.v1","state_root":"X:\\Prod\\state",
            "instances":[
              {"alias":"main","instance_id":"instance_0","instance_index":0,"application_id":"a"},
              {"alias":"named","instance_id":"instance_1","instance_name":"MuMu-3"},
              {"alias":"adb","instance_id":"instance_2","serial":"127.0.0.1:16416"},
              {"alias":"hosted","instance_id":"instance_3","host":"127.0.0.1","port":16480},
              {"alias":"remote","instance_id":"instance_4","host":"192.168.1.9","port":16512},
              {"alias":"handset","instance_id":"instance_5","serial":"R58M123ABC"}
            ],"policy":{}}"#;
        let used = used_instances(config).expect("instances");
        assert_eq!(
            used,
            vec![
                UsedInstance { alias: "main".into(), index: Some(0), ..UsedInstance::default() },
                UsedInstance { alias: "named".into(), name: Some("MuMu-3".into()), ..UsedInstance::default() },
                UsedInstance {
                    alias: "adb".into(),
                    serial: Some("127.0.0.1:16416".into()),
                    port: Some(16416),
                    ..UsedInstance::default()
                },
                UsedInstance { alias: "hosted".into(), port: Some(16480), ..UsedInstance::default() },
                UsedInstance { alias: "remote".into(), ..UsedInstance::default() },
                UsedInstance { alias: "handset".into(), serial: Some("R58M123ABC".into()), ..UsedInstance::default() },
            ]
        );
        let roots = vec![OtherRoot {
            pid: None,
            root: Some(PathBuf::from(r"X:\Prod")),
            instances: Some(used),
            unknown: None,
        }];
        let instance = |index: u16, name: &str, adb_port: Option<u16>| MumuInstance {
            index,
            name: name.to_owned(),
            running: adb_port.is_some(),
            adb_port,
            android_version: None,
        };
        let alias = |instance: MumuInstance| used_by(&roots, &instance).map(|(_, alias)| alias.to_owned());
        assert_eq!(alias(instance(0, "x", None)).as_deref(), Some("main"));
        assert_eq!(alias(instance(3, "MuMu-3", None)).as_deref(), Some("named"));
        // A stopped emulator has no port: only its index or name can name it.
        assert_eq!(alias(instance(1, "MuMu-1", Some(16416))).as_deref(), Some("adb"));
        assert_eq!(alias(instance(1, "MuMu-1", None)), None);
        assert_eq!(alias(instance(3, "MuMu-x", Some(16480))).as_deref(), Some("hosted"));
        assert_eq!(alias(instance(4, "MuMu-4", Some(16512))), None);
        assert_eq!(alias(instance(5, "实验五", Some(16544))), None);
        assert!(used_instances(b"{\"instances\": 3}").is_err());
        assert!(used_instances(b"not json").is_err());
    }

    #[test]
    fn sightings_of_one_root_are_one_installation() {
        let seen = |pid: Option<u32>, root: &str| Sighting { pid, root: Ok(root.to_owned()) };
        let merged = merge_sightings(vec![
            seen(None, r"X:\Root"),
            seen(Some(7), r"x:/root/"),
            seen(None, r"Y:\Other"),
            Sighting { pid: Some(9), root: Err(Said::new("?", "?")) },
            seen(Some(8), r"X:\Root"),
        ]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].pid, Some(7));
        assert_eq!(merged[0].root.as_deref().ok(), Some(r"X:\Root"));
        assert_eq!(merged[1].pid, None);
        assert_eq!(merged[2].pid, Some(9));
        let other = |pid: Option<u32>, root: Option<&str>| OtherRoot {
            pid,
            root: root.map(PathBuf::from),
            instances: None,
            unknown: None,
        };
        assert_eq!(other(Some(7), Some(r"X:\Root")).name(), r"X:\Root");
        assert_eq!(other(Some(7), None).name(), "pid 7");
        assert_eq!(other(None, None).name(), "?");
    }
}
