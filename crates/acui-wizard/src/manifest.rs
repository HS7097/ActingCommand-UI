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

/// The interfaces between the console (U\*) and the Runtime (R\*) of a fresh install, each with
/// its check (Runtime `contracts/component-interfaces.md`, "Checks per operation"; a fresh
/// install has no previous Runtime): `runtime-client` U\* ∩ R\*; `ledger` R\*.max in U\*;
/// `actingd-config` U\* ∩ R\* (the configuration the wizard writes is the Runtime's own
/// `check-config`'s to judge); `runtime-outcomes` R\*.max in U\*, a side that does not declare it
/// counting as [0, 0] ("Parse rules"). The other interfaces have no edge between these two:
/// `package` is the bundles' and the installer's, `install-control` the installer's, and
/// `install-selection` has none on a single-location root.
pub const CONSOLE_EDGES: [(&str, Edge); 4] = [
    ("runtime-client", Edge::Overlap),
    ("ledger", Edge::RuntimeMaxInConsole),
    ("actingd-config", Edge::Overlap),
    ("runtime-outcomes", Edge::RuntimeMaxInConsole),
];

/// How two ranges of one interface must meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The two ranges share a revision.
    Overlap,
    /// The Runtime's highest revision lies in the console's range.
    RuntimeMaxInConsole,
}

/// The interface an absent name counts as ("Parse rules": only those added after v1 have a
/// default).
fn default_range(interface: &str) -> Option<[u64; 2]> {
    (interface == "runtime-outcomes").then_some([0, 0])
}

/// Whether `declaration` is a v1 declaration at all: its schema and a `speaks` object.
fn declares(declaration: &Value) -> bool {
    declaration.get("schema_version").and_then(Value::as_str) == Some(INTERFACES_SCHEMA)
        && declaration.get("speaks").is_some_and(Value::is_object)
}

/// Every edge of [`CONSOLE_EDGES`] the console's declaration (its `BUILD-MANIFEST.json`
/// `interfaces`) and the Runtime's fail, in a person's words with both ranges; a missing or
/// invalid declaration is one problem.
pub fn console_problems(console: Option<&Value>, runtime: Option<&Value>) -> Vec<Said> {
    let mut problems = Vec::new();
    for (side, declaration) in [("监控台", console), ("Runtime", runtime)] {
        if !declaration.is_some_and(declares) {
            let en = if side == "Runtime" { "the Runtime" } else { "the console" };
            problems.push(Said::new(
                format!("{side}的 BUILD-MANIFEST.json 没有有效的接口声明"),
                format!("{en}'s BUILD-MANIFEST.json declares no valid interfaces"),
            ));
        }
    }
    let (Some(console), Some(runtime)) = (console, runtime) else {
        return problems;
    };
    if !problems.is_empty() {
        return problems;
    }
    for (interface, edge) in CONSOLE_EDGES {
        let range = |declaration: &Value| speaks(declaration, interface).or_else(|| default_range(interface));
        let (Some(ours), Some(theirs)) = (range(console), range(runtime)) else {
            problems.push(
                Said::new(
                    format!("监控台或 Runtime 没有声明 {interface} 接口"),
                    format!("the console or the Runtime declares no {interface} interface"),
                )
                .with(interface),
            );
            continue;
        };
        let met = match edge {
            Edge::Overlap => ours[0] <= theirs[1] && theirs[0] <= ours[1],
            Edge::RuntimeMaxInConsole => ours[0] <= theirs[1] && theirs[1] <= ours[1],
        };
        if !met {
            problems.push(
                Said::new(
                    format!("监控台和这一版 Runtime 的 {interface} 接口不配套"),
                    format!("the console and this Runtime do not meet on {interface}"),
                )
                .with(format!(
                    "console [{}, {}], Runtime [{}, {}]",
                    ours[0], ours[1], theirs[0], theirs[1]
                )),
            );
        }
    }
    problems
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

    fn declaration(speaks: Value) -> Value {
        json!({"schema_version": INTERFACES_SCHEMA, "speaks": speaks})
    }

    /// The UI's and the Runtime's declarations of 0.12.58 (this repository's
    /// crates/acui-setup/component-interfaces.json and the Runtime's
    /// distribution/windows/component-interfaces.json).
    fn console() -> Value {
        declaration(json!({
            "actingd-config": [5, 29], "install-selection": [1, 1], "install-control": [0, 1],
            "ledger": [3, 8], "runtime-client": [3, 8], "package": [1, 2], "runtime-outcomes": [1, 1]
        }))
    }

    fn runtime() -> Value {
        declaration(json!({
            "actingd-config": [5, 29], "install-selection": [1, 1], "install-control": [0, 1],
            "ledger": [3, 8], "runtime-client": [3, 8], "package": [3, 10], "runtime-outcomes": [1, 1]
        }))
    }

    #[test]
    fn the_console_and_the_runtime_meet_on_their_four_edges() {
        // The console's `package` [1, 2] is no edge with the Runtime's [3, 10]: it is the
        // bundles' and the installer's.
        assert_eq!(console_problems(Some(&console()), Some(&runtime())), Vec::<Said>::new());
        fn with(side: fn() -> Value, interface: &str, range: Value) -> Value {
            let mut declaration = side();
            declaration["speaks"][interface] = range;
            declaration
        }
        let fails = |console: Value, runtime: Value, interface: &str| {
            let problems = console_problems(Some(&console), Some(&runtime));
            assert_eq!(problems.len(), 1, "{interface}: {problems:?}");
            assert!(problems[0].en.contains(interface), "{problems:?}");
        };
        // runtime-client and actingd-config: the ranges overlap, or the edge fails.
        fails(with(console, "runtime-client", json!([1, 2])), runtime(), "runtime-client");
        fails(console(), with(runtime, "actingd-config", json!([30, 31])), "actingd-config");
        assert!(console_problems(Some(&with(console, "runtime-client", json!([8, 9]))), Some(&runtime())).is_empty());
        // ledger: the Runtime's highest revision in the console's range, not a mere overlap.
        fails(console(), with(runtime, "ledger", json!([3, 9])), "ledger");
        assert!(console_problems(Some(&with(console, "ledger", json!([8, 8]))), Some(&runtime())).is_empty());
        // runtime-outcomes: undeclared counts as [0, 0].
        let mut undeclared = console();
        undeclared["speaks"].as_object_mut().unwrap().remove("runtime-outcomes");
        fails(undeclared, runtime(), "runtime-outcomes");
        // A v1 interface missing on either side.
        let mut missing = runtime();
        missing["speaks"].as_object_mut().unwrap().remove("ledger");
        fails(console(), missing, "ledger");
    }

    #[test]
    fn a_console_without_a_declaration_is_refused() {
        assert_eq!(console_problems(None, Some(&runtime())).len(), 1);
        assert_eq!(console_problems(Some(&runtime()), None).len(), 1);
        assert_eq!(console_problems(None, None).len(), 2);
        let other = json!({"schema_version": "actingcommand.component-interfaces.v2", "speaks": {}});
        assert_eq!(console_problems(Some(&other), Some(&runtime())).len(), 1);
        assert_eq!(console_problems(Some(&json!({"schema_version": INTERFACES_SCHEMA})), Some(&runtime())).len(), 1);
    }
}
