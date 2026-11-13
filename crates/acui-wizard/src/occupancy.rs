// SPDX-License-Identifier: GPL-3.0-only
//! Which emulator instances another ActingCommand on this machine already uses, so the window
//! can mark them and ask before one is taken. Each running `actingcommand-actingd.exe` names its
//! root by its own path (`<root>\runtime\actingcommand-actingd.exe`, the root holding
//! `runtime\BUILD-MANIFEST.json`: Runtime `apps/actingd/src/config/ac_adb.rs`, the install
//! root); that root's `actingd.config.json` is read, nothing else, and only its
//! `instances[].instance_index`, `instance_name` and `alias`. What cannot be read is not known:
//! it is said and logged, and nothing stops.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::mumu::MumuInstance;
use crate::platform;
use crate::text::Said;
use crate::winpath::strip_verbatim;

pub const ACTINGD_EXE: &str = "actingcommand-actingd.exe";
const CONFIG_LIMIT: u64 = 1 << 20;

/// An instance another root's configuration binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsedInstance {
    pub alias: String,
    pub index: Option<u16>,
    pub name: Option<String>,
}

/// Another running Runtime: its process, its root when known, the instances it binds when its
/// configuration could be read, and why not otherwise.
#[derive(Debug, Clone)]
pub struct OtherRoot {
    pub pid: u32,
    pub root: Option<PathBuf>,
    pub instances: Option<Vec<UsedInstance>>,
    pub unknown: Option<Said>,
}

/// The root of an actingd at `exe`: the folder holding its `runtime\` folder.
pub fn root_of_actingd(exe: &str) -> Option<String> {
    let exe = strip_verbatim(exe);
    let (folder, file) = exe.rsplit_once(['\\', '/'])?;
    if !file.eq_ignore_ascii_case(ACTINGD_EXE) {
        return None;
    }
    let (root, runtime) = folder.rsplit_once(['\\', '/'])?;
    (runtime.eq_ignore_ascii_case("runtime") && !root.is_empty()).then(|| root.to_owned())
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
        .map(|entry| UsedInstance {
            alias: entry["alias"].as_str().unwrap_or_default().to_owned(),
            index: entry["instance_index"].as_u64().and_then(|index| u16::try_from(index).ok()),
            name: entry["instance_name"].as_str().map(str::to_owned),
        })
        .collect())
}

/// The other root that binds `instance`, by its index or its exact name, and the alias it has
/// there.
pub fn used_by<'a>(roots: &'a [OtherRoot], instance: &MumuInstance) -> Option<(&'a OtherRoot, &'a str)> {
    roots.iter().find_map(|root| {
        root.instances.as_ref()?.iter().find_map(|used| {
            let same = used.index == Some(instance.index) || used.name.as_deref() == Some(instance.name.as_str());
            same.then(|| (root, used.alias.as_str()))
        })
    })
}

/// Every running actingd with what its root binds (see the module's description). `log` hears
/// each thing that could not be read.
pub fn scan(log: &mut dyn FnMut(Said)) -> Vec<OtherRoot> {
    let processes = match platform::processes_named(ACTINGD_EXE) {
        Ok(processes) => processes,
        Err(problem) => {
            log(Said::new("读不到正在运行的 ActingCommand", "the running ActingCommand cannot be listed").with(problem));
            return Vec::new();
        }
    };
    let mut roots: Vec<OtherRoot> = Vec::new();
    for process in processes {
        let mut other = OtherRoot {
            pid: process.pid,
            root: None,
            instances: None,
            unknown: None,
        };
        let unknown = match &process.path {
            Err(reason) => Some(Said::new("读不到它的程序路径", "its program path cannot be read").with(reason)),
            Ok(path) => match root_of_actingd(&path.to_string_lossy()) {
                None => Some(
                    Said::new("它不在一个安装根的 runtime 文件夹里", "it does not run from an installation root's runtime folder")
                        .with(path.display()),
                ),
                Some(root) => {
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
            },
        };
        if let Some(reason) = unknown {
            log(Said::new(
                format!("不知道 ActingCommand（进程 {}）用着哪些实例", process.pid),
                format!("which instances the ActingCommand of pid {} uses is not known", process.pid),
            )
            .with(&reason));
            other.unknown = Some(reason);
        }
        roots.push(other);
    }
    roots
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
    fn a_configuration_binds_by_index_or_by_name() {
        let config = br#"{"schema_version":"actingcommand.actingd.config.v1","state_root":"X:\\Prod\\state",
            "instances":[
              {"alias":"main","instance_id":"instance_0","instance_index":0,"application_id":"a"},
              {"alias":"named","instance_id":"instance_1","instance_name":"MuMu-3"},
              {"alias":"adb","instance_id":"instance_2","serial":"127.0.0.1:16416"}
            ],"policy":{}}"#;
        let used = used_instances(config).expect("instances");
        assert_eq!(
            used,
            vec![
                UsedInstance { alias: "main".into(), index: Some(0), name: None },
                UsedInstance { alias: "named".into(), index: None, name: Some("MuMu-3".into()) },
                UsedInstance { alias: "adb".into(), index: None, name: None },
            ]
        );
        let roots = vec![OtherRoot {
            pid: 9,
            root: Some(PathBuf::from(r"X:\Prod")),
            instances: Some(used),
            unknown: None,
        }];
        let instance = |index: u16, name: &str| MumuInstance {
            index,
            name: name.to_owned(),
            running: true,
            adb_port: None,
            android_version: None,
        };
        assert_eq!(used_by(&roots, &instance(0, "x")).map(|(_, alias)| alias), Some("main"));
        assert_eq!(used_by(&roots, &instance(3, "MuMu-3")).map(|(_, alias)| alias), Some("named"));
        assert!(used_by(&roots, &instance(5, "实验五")).is_none());
        assert!(used_instances(b"{\"instances\": 3}").is_err());
        assert!(used_instances(b"not json").is_err());
    }
}
