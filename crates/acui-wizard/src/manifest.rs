// SPDX-License-Identifier: GPL-3.0-only
//! The `BUILD-MANIFEST.json` each program zip carries (Runtime `.github/workflows/
//! windows-remote-build.yml` `New-BuildManifest`; this repository's `build.yml`, "Stage Windows
//! artifact with build manifest"): the repository and commit it was built from, every file with
//! its size and SHA-256, and the interface declaration (`contracts/component-interfaces.md`).
//! Members the wizard does not read are left alone, so a later manifest still reads.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;

use crate::archive::safe_relative;
use crate::digest::is_sha256;
use crate::text::Said;

pub const MANIFEST_FILE: &str = "BUILD-MANIFEST.json";
pub const RUNTIME_REPOSITORY: &str = "HS7097/ActingCommand-Runtime";
pub const UI_REPOSITORY: &str = "HS7097/ActingCommand-UI";
/// The Runtime manifest's `runtime_payload_layout` (Runtime `crates/lab-rescue/src/install/
/// manifest.rs` `RUNTIME_LAYOUT`, which `install adopt` requires).
pub const RUNTIME_LAYOUT: &str = "distribution-v1";
/// The Tools manifest's `tools_payload_layout` (the same file's `TOOLS_LAYOUT`).
pub const TOOLS_LAYOUT: &str = "platform-tools-v3";
/// The schema of an interface declaration (the same file's `INTERFACES_SCHEMA`).
pub const INTERFACES_SCHEMA: &str = "actingcommand.component-interfaces.v1";
/// The console zip's A/B forwarding entry: it runs only on an A/B root (it needs
/// `install\active.json`), so it is not laid into a single-location root.
pub const FORWARDER: &str = "acforward.exe";

#[derive(Debug, Clone, Deserialize)]
pub struct BuildManifest {
    pub repository: String,
    pub commit_sha: String,
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub runtime_payload_layout: Option<String>,
    #[serde(default)]
    pub tools_payload_layout: Option<String>,
    #[serde(default)]
    pub interfaces: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// The three program zips and where each goes in the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Runtime,
    Tools,
    Console,
}

impl Program {
    pub const ALL: [Program; 3] = [Program::Runtime, Program::Tools, Program::Console];

    /// The root's folder for it (Runtime `contracts/install-cli.md`, the single-location root;
    /// `ui\` is this wizard's, which the console's root mode reads).
    pub const fn dir(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Tools => "tools",
            Self::Console => "ui",
        }
    }

    pub fn label(self) -> Said {
        match self {
            Self::Runtime => Said::new("Runtime 程序", "the Runtime programs"),
            Self::Tools => Said::new("Tools 工具", "the Tools"),
            Self::Console => Said::new("监控台", "the console"),
        }
    }

    pub const fn repository(self) -> &'static str {
        match self {
            Self::Runtime | Self::Tools => RUNTIME_REPOSITORY,
            Self::Console => UI_REPOSITORY,
        }
    }

    /// The files the wizard and the Runtime's installer need from it: actingd and actingctl
    /// (`install adopt`); the Lab, the watchdog's launcher and the official adb
    /// (`contracts/runtime-watchdog.md`, `install`; `actingd-check-config.md`, the install root's
    /// adb); the console.
    pub const fn required(self) -> &'static [&'static str] {
        match self {
            Self::Runtime => &["actingcommand-actingd.exe", "actingctl.exe"],
            Self::Tools => &[
                "actinglab.exe",
                "actingwatch.exe",
                "platform-tools/adb.exe",
                "platform-tools/AdbWinApi.dll",
                "platform-tools/AdbWinUsbApi.dll",
            ],
            Self::Console => &["acui.exe"],
        }
    }

    /// Whether the manifest's file `path` is laid into the root: every one but the console's
    /// A/B forwarding entry.
    pub fn lays(self, path: &str) -> bool {
        !(self == Self::Console && path == FORWARDER)
    }
}

/// The manifest in `bytes` (a UTF-8 byte-order mark allowed).
pub fn parse_manifest(bytes: &[u8]) -> Result<BuildManifest, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|error| {
        Said::new("BUILD-MANIFEST.json 读不懂", "BUILD-MANIFEST.json does not parse").with(error)
    })
}

/// What a program's manifest must say: its repository, the commit the install set names, its
/// payload layout, every path plain, listed once and with a SHA-256, and the files the install
/// needs among them.
pub fn check_identity(program: Program, manifest: &BuildManifest, commit: &str) -> Result<(), Said> {
    let label = program.label();
    let mismatch = |what: &str, expected: &str, actual: &str| {
        Said::new(
            format!("{}的 BUILD-MANIFEST.json 与安装集不符", label.zh),
            format!("{}'s BUILD-MANIFEST.json does not match the install set", label.en),
        )
        .with(format!("{what}: expected {expected}, found {actual}"))
    };
    if manifest.repository != program.repository() {
        return Err(mismatch("repository", program.repository(), &manifest.repository));
    }
    if !manifest.commit_sha.eq_ignore_ascii_case(commit) {
        return Err(mismatch("commit_sha", commit, &manifest.commit_sha));
    }
    let layout = match program {
        Program::Runtime => Some(("runtime_payload_layout", RUNTIME_LAYOUT, &manifest.runtime_payload_layout)),
        Program::Tools => Some(("tools_payload_layout", TOOLS_LAYOUT, &manifest.tools_payload_layout)),
        Program::Console => None,
    };
    if let Some((key, expected, actual)) = layout {
        if actual.as_deref() != Some(expected) {
            return Err(mismatch(key, expected, actual.as_deref().unwrap_or("(none)")));
        }
    }
    let mut seen = BTreeSet::new();
    for file in &manifest.files {
        if !safe_relative(&file.path) || file.path == MANIFEST_FILE || !is_sha256(&file.sha256) {
            return Err(Said::new(
                format!("{}的 BUILD-MANIFEST.json 有不合规的文件项", label.zh),
                format!("{}'s BUILD-MANIFEST.json lists an invalid file", label.en),
            )
            .with(&file.path));
        }
        if !seen.insert(file.path.to_ascii_lowercase()) {
            return Err(Said::new(
                format!("{}的 BUILD-MANIFEST.json 重复列出文件", label.zh),
                format!("{}'s BUILD-MANIFEST.json lists a file twice", label.en),
            )
            .with(&file.path));
        }
    }
    for required in program.required() {
        if !manifest.files.iter().any(|file| file.path == *required) {
            return Err(Said::new(
                format!("{}缺少必需的文件", label.zh),
                format!("{} lacks a required file", label.en),
            )
            .with(*required));
        }
    }
    Ok(())
}

/// The range `[min, max]` a declaration `{schema_version, speaks}` gives `interface` (Runtime
/// `crates/lab-rescue/src/install/manifest.rs` `parse_declaration`, whose range rules these are);
/// `None` when the declaration is not one or does not name it.
pub fn speaks(declaration: &Value, interface: &str) -> Option<[u64; 2]> {
    if declaration.get("schema_version")?.as_str()? != INTERFACES_SCHEMA {
        return None;
    }
    let pair = declaration.get("speaks")?.get(interface)?.as_array()?;
    let [min, max] = pair.as_slice() else {
        return None;
    };
    let (min, max) = (min.as_u64()?, max.as_u64()?);
    (min <= max).then_some([min, max])
}

/// The `package` edge a standard package must pass to be installed beside a Runtime: the
/// highest revision the bundle declares lies within the Runtime's range (Runtime
/// `crates/lab-rescue/src/install/place.rs` `plan`, `own[0] <= declared[1] && declared[1] <=
/// own[1]`).
pub fn package_edge(bundle: [u64; 2], runtime: [u64; 2]) -> bool {
    runtime[0] <= bundle[1] && bundle[1] <= runtime[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const COMMIT: &str = "ef94d950a102a595fa0bde974b8d77ad54329b48";

    fn manifest(value: Value) -> BuildManifest {
        serde_json::from_value(value).expect("a manifest")
    }

    fn file(path: &str) -> Value {
        json!({"path": path, "size_bytes": 1, "sha256": "a".repeat(64)})
    }

    #[test]
    fn a_runtime_manifest_names_its_repository_commit_layout_and_programs() {
        // The shape New-BuildManifest writes (Runtime windows-remote-build.yml:723-778), with a
        // member the wizard does not read.
        let good = json!({
            "repository": RUNTIME_REPOSITORY, "commit_sha": COMMIT, "tree_sha": "t",
            "runtime_payload_layout": "distribution-v1",
            "files": [file("actingcommand-actingd.exe"), file("actingctl.exe"), file("INSTALL.md")],
            "interfaces": {"schema_version": INTERFACES_SCHEMA, "speaks": {"package": [3, 10]}}
        });
        assert_eq!(check_identity(Program::Runtime, &manifest(good.clone()), COMMIT), Ok(()));
        let mut other_commit = good.clone();
        other_commit["commit_sha"] = json!("0".repeat(40));
        assert!(check_identity(Program::Runtime, &manifest(other_commit), COMMIT).is_err());
        let mut no_layout = good.clone();
        no_layout["runtime_payload_layout"] = Value::Null;
        assert!(check_identity(Program::Runtime, &manifest(no_layout), COMMIT).is_err());
        let mut missing = good.clone();
        missing["files"] = json!([file("actingcommand-actingd.exe")]);
        assert!(check_identity(Program::Runtime, &manifest(missing), COMMIT).is_err());
        let mut escaping = good;
        escaping["files"] = json!([
            file("actingcommand-actingd.exe"), file("actingctl.exe"), file("../x.exe")
        ]);
        assert!(check_identity(Program::Runtime, &manifest(escaping), COMMIT).is_err());
    }

    #[test]
    fn the_console_zip_lays_everything_but_the_forwarding_entry() {
        assert!(!Program::Console.lays("acforward.exe"));
        assert!(Program::Console.lays("acui.exe"));
        assert!(Program::Runtime.lays("acforward.exe"));
    }

    #[test]
    fn the_package_edge_takes_the_bundles_highest_revision() {
        let declaration = json!({
            "schema_version": INTERFACES_SCHEMA,
            "speaks": {"package": [3, 3]},
            "validated_with": {"repository": RUNTIME_REPOSITORY, "commit": COMMIT}
        });
        assert_eq!(speaks(&declaration, "package"), Some([3, 3]));
        assert_eq!(speaks(&declaration, "ledger"), None);
        assert_eq!(speaks(&json!({"speaks": {"package": [3, 3]}}), "package"), None);
        assert_eq!(
            speaks(&json!({"schema_version": INTERFACES_SCHEMA, "speaks": {"package": [4, 3]}}), "package"),
            None
        );
        let runtime = [3, 10];
        assert!(package_edge([3, 3], runtime));
        assert!(package_edge([10, 10], runtime));
        assert!(!package_edge([11, 11], runtime));
        assert!(!package_edge([1, 2], runtime));
    }
}
