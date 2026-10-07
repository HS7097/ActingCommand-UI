// SPDX-License-Identifier: GPL-3.0-only
//! Component interfaces (Workflow #364; the Runtime's `contracts/component-interfaces.md`):
//! whether the Runtime, the UI, this installer and the resource bundles a run brings together
//! can work together, judged from what each declares in its build manifest (`interfaces`) or
//! in a bundle's `interfaces.json` — and, for the releases built before declarations, from the
//! known table below — never from a list of release pairs.
//!
//! A range `[min, max]` is what a component reads; a writer writes `max`. Persisted data is
//! checked by containment (the writer's `max` lies in each reader's range), a live exchange by
//! negotiation (the ranges intersect, and the highest common revision is used). Every unmet
//! edge is listed, and the run stops before the installation changes. The existing gates stay:
//! the selected slot's `check-config` and the new Runtime's `ledger-maintenance verify`.
//!
//! Nothing here depends on the Runtime crates this program is built with: the declaration is
//! JSON this module reads itself, and the ranges this build speaks are its own
//! `component-interfaces.json`, which moves with the crates it is built on.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::bundle::Bundle;
use crate::lifecycle::Control;
use crate::verify::{Report, Verified, MANIFEST, RUNTIME_REPOSITORY, UI_REPOSITORY};

pub const SCHEMA: &str = "actingcommand.component-interfaces.v1";
/// This build's own declaration: the very file the UI build writes into its manifest.
const OWN: &str = include_str!("../component-interfaces.json");

/// The six v1 interfaces, in the order every line lists them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Interface {
    ActingdConfig,
    InstallSelection,
    InstallControl,
    Ledger,
    RuntimeClient,
    Package,
}

const ALL: [Interface; 6] = [
    Interface::ActingdConfig,
    Interface::InstallSelection,
    Interface::InstallControl,
    Interface::Ledger,
    Interface::RuntimeClient,
    Interface::Package,
];

impl Interface {
    fn name(self) -> &'static str {
        match self {
            Interface::ActingdConfig => "actingd-config",
            Interface::InstallSelection => "install-selection",
            Interface::InstallControl => "install-control",
            Interface::Ledger => "ledger",
            Interface::RuntimeClient => "runtime-client",
            Interface::Package => "package",
        }
    }

    fn named(name: &str) -> Option<Self> {
        ALL.into_iter().find(|interface| interface.name() == name)
    }
}

/// `[min, max]`: the revisions a component reads; a writer writes `max`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Range {
    min: u32,
    max: u32,
}

impl Range {
    const fn of(min: u32, max: u32) -> Self {
        Range { min, max }
    }

    fn contains(self, revision: u32) -> bool {
        self.min <= revision && revision <= self.max
    }

    /// The highest revision both ranges hold, when they meet.
    fn common(self, other: Range) -> Option<u32> {
        let (low, high) = (self.min.max(other.min), self.max.min(other.max));
        (low <= high).then_some(high)
    }
}

impl fmt::Display for Range {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}, {}]", self.min, self.max)
    }
}

/// One release built before declarations: repository, full commit, tag, and its ranges in the
/// order of `ALL` (`None`: it does not speak that interface). Each row was read against the
/// anchors at its tag (Runtime `contracts/component-interfaces.md`).
struct Known {
    repository: &'static str,
    commit: &'static str,
    tag: &'static str,
    speaks: [Option<Range>; 6],
}

const R11: Option<Range> = Some(Range::of(1, 1));

const KNOWN: &[Known] = &[
    Known {
        repository: RUNTIME_REPOSITORY,
        commit: "b70518949c19d56085afc3a84c49274c4cc041fe",
        tag: "v0.11.0",
        speaks: [R11, None, Some(Range::of(0, 0)), R11, R11, R11],
    },
    Known {
        repository: RUNTIME_REPOSITORY,
        commit: "732a546fb0e1c60538e53c5e52763c4e3e43c66e",
        tag: "v0.11.1",
        speaks: [R11, R11, Some(Range::of(0, 1)), R11, R11, R11],
    },
    Known {
        repository: RUNTIME_REPOSITORY,
        commit: "14b88e04fb31e89f12614192cd38657181218dcb",
        tag: "v0.11.2",
        speaks: [Some(Range::of(2, 2)), R11, Some(Range::of(0, 1)), R11, R11, R11],
    },
    Known {
        repository: UI_REPOSITORY,
        commit: "b0d70e606e3df6ccb5c351b2519e240cb5db4f03",
        tag: "v0.11.0",
        speaks: [R11, None, Some(Range::of(0, 0)), R11, R11, R11],
    },
    Known {
        repository: UI_REPOSITORY,
        commit: "c47bab660b343639bbd67bd5b3a19e8f27fcafdb",
        tag: "v0.11.1",
        speaks: [R11, R11, Some(Range::of(0, 1)), R11, R11, R11],
    },
    Known {
        repository: UI_REPOSITORY,
        commit: "3f08f63978877d68f20b2c86077b1bc7c5e39a83",
        tag: "v0.11.2",
        speaks: [Some(Range::of(1, 2)), R11, Some(Range::of(0, 1)), R11, R11, R11],
    },
];

/// Where a component's ranges come from, as its line and a refusal say it.
#[derive(Clone)]
enum Source {
    Declared,
    Known(&'static str),
    /// A bundle without `interfaces.json`: `package` [1, 1].
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Declared => formatter.write_str("声明 / declared"),
            Source::Known(tag) => write!(formatter, "{tag}，已知表 / known release"),
            Source::Default => formatter.write_str("未声明，按 package [1, 1] / undeclared, package [1, 1]"),
        }
    }
}

/// One party of a run: the installer itself, a Runtime or UI program, or a bundle.
#[derive(Clone)]
pub struct Component {
    label: String,
    /// A program's commit; empty for the installer and a bundle.
    commit: String,
    source: Source,
    speaks: BTreeMap<Interface, Range>,
    /// A Runtime's Tools layout (`tools-layout`, derived), when its Tools manifest is at hand.
    tools_layout: Option<u32>,
    /// A bundle speaks `package` only; its line names nothing else.
    bundle: bool,
}

impl Component {
    fn named(&self) -> String {
        format!("{}（{}）", self.label, self.source)
    }

    /// `接口 / Interfaces: <label> (<source>) actingd-config [2, 2] install-selection [1, 1] …`
    fn line(&self) -> String {
        let mut line = format!("接口 / Interfaces: {}", self.named());
        for interface in ALL {
            match self.speaks.get(&interface) {
                Some(range) => line.push_str(&format!(" {} {range}", interface.name())),
                None if self.bundle => {}
                None => line.push_str(&format!(" {} —", interface.name())),
            }
        }
        if let Some(layout) = self.tools_layout {
            line.push_str(&format!(" tools-layout {layout}"));
        }
        line
    }
}

/// Why a check stopped a run: `Refused` is exit code 6 on the command line — interfaces that
/// do not fit, an invalid declaration, a program neither declared nor known; `Failed` is any
/// other failure (a manifest that cannot be read, a log write).
pub enum Stop {
    Refused(String),
    Failed(String),
}

impl From<String> for Stop {
    fn from(text: String) -> Self {
        Stop::Failed(text)
    }
}

impl From<Stop> for String {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Refused(text) | Stop::Failed(text) => text,
        }
    }
}

fn invalid(what: &str, why: impl fmt::Display) -> Stop {
    Stop::Refused(format!(
        "接口声明无效 / invalid interface declaration: {what}: {why}"
    ))
}

/// One `interfaces` object (v1): its ranges, and a note for each key or interface name it
/// carries that this installer does not check. `required`: the names it must hold; `expected`:
/// further keys beside `schema_version` and `speaks` that are no surprise.
fn parse(
    value: &Value,
    what: &str,
    required: &[Interface],
    expected: &[&str],
) -> Result<(BTreeMap<Interface, Range>, Vec<String>), Stop> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(what, "不是 JSON 对象 / not a JSON object"))?;
    match object.get("schema_version").and_then(Value::as_str) {
        Some(SCHEMA) => {}
        other => {
            return Err(invalid(
                what,
                format!(
                    "schema_version 应为 / should be {SCHEMA}，实为 / is {}",
                    other.unwrap_or("（无 / none）")
                ),
            ))
        }
    }
    let mut notes = Vec::new();
    for key in object.keys() {
        if key != "schema_version" && key != "speaks" && !expected.contains(&key.as_str()) {
            notes.push(format!(
                "{what}: 忽略不认识的键 / ignored an unknown key: {key}"
            ));
        }
    }
    let speaks = object
        .get("speaks")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(what, "缺少 speaks 对象 / no speaks object"))?;
    let mut ranges = BTreeMap::new();
    for (name, given) in speaks {
        let Some(interface) = Interface::named(name) else {
            notes.push(format!(
                "{what}: 忽略本安装程序不检查的接口 / ignored an interface this installer does not check: {name}"
            ));
            continue;
        };
        let range = given
            .as_array()
            .filter(|pair| pair.len() == 2)
            .and_then(|pair| {
                let min = u32::try_from(pair[0].as_u64()?).ok()?;
                let max = u32::try_from(pair[1].as_u64()?).ok()?;
                (min <= max).then_some(Range { min, max })
            })
            .ok_or_else(|| {
                invalid(
                    what,
                    format!("{name} 不是 [min, max]（整数，0 <= min <= max）/ is not [min, max] with integers 0 <= min <= max: {given}"),
                )
            })?;
        ranges.insert(interface, range);
    }
    if let Some(missing) = required
        .iter()
        .find(|interface| !ranges.contains_key(*interface))
    {
        return Err(invalid(
            what,
            format!("缺少接口 / lacks {}", missing.name()),
        ));
    }
    Ok((ranges, notes))
}

/// This acsetup, by its own declaration (`component-interfaces.json`, compiled in).
pub fn installer() -> Result<Component, Stop> {
    let value: Value = serde_json::from_str(OWN).map_err(|error| {
        invalid("本安装程序自带的声明 / this installer's own declaration", error)
    })?;
    let (speaks, _) = parse(&value, "acsetup", &ALL, &[])?;
    Ok(Component {
        label: format!("安装程序 / installer acsetup {}", env!("CARGO_PKG_VERSION")),
        commit: String::new(),
        source: Source::Declared,
        speaks,
        tools_layout: None,
        bundle: false,
    })
}

/// A Runtime or UI program from its build manifest: its own declaration when it has one,
/// else its row of the known table; neither is a refusal. A UI manifest's
/// `installation_selection_schema` must agree with its `install-selection` range.
fn program(manifest: &[u8], role: &str, repository: &str, report: Report<'_>) -> Result<Component, Stop> {
    let value: Value = serde_json::from_slice(manifest).map_err(|error| {
        Stop::Failed(format!("{role} 的 {MANIFEST} 无法解析 / does not parse: {error}"))
    })?;
    let found = value["repository"].as_str().unwrap_or_default();
    if found != repository {
        return Err(Stop::Failed(format!(
            "{role} 的 {MANIFEST} repository 应为 / should be {repository}，实为 / is {found}"
        )));
    }
    let commit = value["commit_sha"]
        .as_str()
        .ok_or_else(|| Stop::Failed(format!("{role} 的 {MANIFEST} 没有 commit_sha / has no commit_sha")))?
        .to_string();
    let label = format!("{role} {}", commit.get(..12).unwrap_or(commit.as_str()));
    let (source, speaks) = match value.get("interfaces").filter(|value| !value.is_null()) {
        Some(declared) => {
            let (speaks, notes) = parse(declared, &label, &ALL, &[])?;
            for note in notes {
                report.line(&note)?;
            }
            (Source::Declared, speaks)
        }
        None => {
            let known = KNOWN
                .iter()
                .find(|known| known.repository == repository && known.commit == commit)
                .ok_or_else(|| {
                    Stop::Refused(format!(
                        "{repository}@{commit} 未声明接口，也不是已知发布件 / declares no interfaces and is not a known release\n安装未改动 / The installation was not changed"
                    ))
                })?;
            let speaks: BTreeMap<Interface, Range> = ALL
                .into_iter()
                .zip(known.speaks)
                .filter_map(|(interface, range)| range.map(|range| (interface, range)))
                .collect();
            (Source::Known(known.tag), speaks)
        }
    };
    if repository == UI_REPOSITORY {
        let schema = value.get("installation_selection_schema");
        let reads = speaks
            .get(&Interface::InstallSelection)
            .is_some_and(|range: &Range| range.contains(1));
        let agrees = match schema {
            None => !reads,
            Some(Value::String(schema)) if schema == acui_installation::INSTALL_SELECTION_SCHEMA => reads,
            Some(_) => false,
        };
        if !agrees {
            return Err(invalid(
                &label,
                format!(
                    "installation_selection_schema（{}）与 install-selection 不一致 / disagrees with install-selection",
                    schema.map_or("—".to_string(), Value::to_string)
                ),
            ));
        }
    }
    Ok(Component {
        label,
        commit,
        source,
        speaks,
        tools_layout: None,
        bundle: false,
    })
}

/// The program a slot (or the old layout's root) holds in `<programs>\<dir>\`.
fn installed(programs: &Path, dir: &str, role: &str, report: Report<'_>) -> Result<Component, Stop> {
    let manifest = acui_installation::read_bounded(
        &programs.join(dir).join(MANIFEST),
        acui_installation::MAX_MATERIAL_BYTES,
    )?;
    let repository = if dir == "ui" { UI_REPOSITORY } else { RUNTIME_REPOSITORY };
    program(&manifest, role, repository, report)
}

/// `tools-layout`, derived from a Tools manifest's `tools_payload_layout`.
fn tools_layout(manifest: &[u8]) -> Result<Option<u32>, Stop> {
    let value: Value = serde_json::from_slice(manifest)
        .map_err(|error| Stop::Failed(format!("tools 的 {MANIFEST} 无法解析 / does not parse: {error}")))?;
    match value.get("tools_payload_layout") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(layout)) if layout == "platform-tools-v1" => Ok(Some(1)),
        Some(Value::String(layout)) if layout == "platform-tools-v2" => Ok(Some(2)),
        Some(other) => Err(Stop::Refused(format!(
            "本安装程序不认识 Tools 的 tools_payload_layout / this installer does not know the Tools layout {other}\n安装未改动 / The installation was not changed"
        ))),
    }
}

/// A bundle: its zip-root `interfaces.json` when it has one (only `package` is required, and
/// `validated_with` is recorded), else `package` [1, 1].
fn bundle(bundle: &Bundle, report: Report<'_>) -> Result<Component, Stop> {
    let label = format!(
        "标准包 / bundle {}",
        bundle
            .file
            .file_name()
            .map_or_else(|| bundle.file.display().to_string(), |name| name.to_string_lossy().into_owned())
    );
    let Some(bytes) = &bundle.interfaces else {
        return Ok(Component {
            label,
            commit: String::new(),
            source: Source::Default,
            speaks: BTreeMap::from([(Interface::Package, Range::of(1, 1))]),
            tools_layout: None,
            bundle: true,
        });
    };
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(&format!("{label}: interfaces.json"), error))?;
    let (speaks, notes) = parse(&value, &label, &[Interface::Package], &["validated_with"])?;
    for note in notes {
        report.line(&note)?;
    }
    if let Some(with) = value.get("validated_with") {
        report.line(&format!(
            "{label}: 资源仓 CI 校验所用 / validated by its CI with {}@{}（只记录，不检查 / recorded, not checked）",
            with["repository"].as_str().unwrap_or("?"),
            with["commit"].as_str().unwrap_or("?")
        ))?;
    }
    // Only `package` is a bundle's edge; any other name it gives is not checked.
    let speaks = speaks
        .into_iter()
        .filter(|(interface, _)| *interface == Interface::Package)
        .collect();
    Ok(Component {
        label,
        commit: String::new(),
        source: Source::Declared,
        speaks,
        tools_layout: None,
        bundle: true,
    })
}

/// Who works together after a run (Runtime `contracts/component-interfaces.md`, "Checks per
/// operation"): the next Runtime and UI (`runtime`, `ui`), the previous Runtime whose
/// configuration and state root the next one takes over, this installer and the bundles.
struct Parties<'a> {
    installer: &'a Component,
    runtime: &'a Component,
    ui: &'a Component,
    previous: Option<&'a Component>,
    /// The release's acsetup becomes the fixed manager, which writes the selection later.
    manager: bool,
    /// The next Runtime is laid into a new slot.
    new_slot: bool,
    bundles: &'a [Component],
}

/// What the checks settled.
pub struct Agreed {
    /// Closing the previous Runtime, when there is one.
    pub close: Option<Control>,
    /// Starting the next one.
    pub start: Control,
    /// The previous Runtime's commit as checked; a run confirms it again under its writer lock.
    previous: Option<String>,
}

impl Agreed {
    /// The control for closing the previous Runtime.
    pub fn closing(&self) -> Result<Control, String> {
        self.close
            .ok_or_else(|| "接口检查没有涉及要关闭的 Runtime / The interface check covered no Runtime to close".to_string())
    }

    /// The Runtime in `<programs>\runtime\` is still the one the check saw.
    pub fn confirm_previous(&self, programs: &Path) -> Result<(), String> {
        let path = programs.join("runtime").join(MANIFEST);
        let bytes = acui_installation::read_bounded(&path, acui_installation::MAX_MATERIAL_BYTES)?;
        let commit = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| value["commit_sha"].as_str().map(str::to_string));
        match (&self.previous, commit) {
            (Some(checked), Some(commit)) if *checked == commit => Ok(()),
            (checked, found) => Err(format!(
                "接口检查之后安装变了，请重新运行 / The installation changed after the interface check; run again: checked {}, found {}",
                checked.as_deref().unwrap_or("—"),
                found.as_deref().unwrap_or("—")
            )),
        }
    }
}

fn silent(failures: &mut Vec<String>, interface: Interface, parties: &[&Component]) {
    for party in parties {
        if !party.speaks.contains_key(&interface) {
            failures.push(format!(
                "  {}: {} 不使用此接口 / does not speak it",
                interface.name(),
                party.named()
            ));
        }
    }
}

/// Negotiation: the highest revision both speak.
fn negotiate(failures: &mut Vec<String>, interface: Interface, left: &Component, right: &Component) -> Option<u32> {
    match (left.speaks.get(&interface), right.speaks.get(&interface)) {
        (Some(a), Some(b)) => {
            let common = a.common(*b);
            if common.is_none() {
                failures.push(format!(
                    "  {}: {} {a} ∩ {} {b} = ∅，没有共同修订 / no common revision",
                    interface.name(),
                    left.named(),
                    right.named()
                ));
            }
            common
        }
        _ => {
            silent(failures, interface, &[left, right]);
            None
        }
    }
}

/// Containment: what `writer` writes, `reader` reads.
fn contain(failures: &mut Vec<String>, interface: Interface, writer: &Component, reader: &Component) {
    match (writer.speaks.get(&interface), reader.speaks.get(&interface)) {
        (Some(written), Some(read)) if read.contains(written.max) => {}
        (Some(written), Some(read)) => failures.push(format!(
            "  {}: {} 写 / writes {} ∉ {} 读 / reads {read}",
            interface.name(),
            writer.named(),
            written.max,
            reader.named()
        )),
        _ => silent(failures, interface, &[writer, reader]),
    }
}

fn control_name(control: Control) -> &'static str {
    match control {
        Control::Cold => "冷态 / cold",
        Control::Transition => "Host 安装过渡 / Host install transition",
    }
}

/// Every edge of the parties, every component logged first. A refusal lists every unmet edge.
fn check(parties: &Parties<'_>, report: Report<'_>) -> Result<Agreed, Stop> {
    let mut seen: Vec<*const Component> = Vec::new();
    for component in [Some(parties.installer), parties.previous, Some(parties.runtime), Some(parties.ui)]
        .into_iter()
        .flatten()
        .chain(parties.bundles)
    {
        if !seen.contains(&(component as *const Component)) {
            seen.push(component as *const Component);
            report.line(&component.line())?;
        }
    }
    let (installer, runtime, ui) = (parties.installer, parties.runtime, parties.ui);
    let mut failures = Vec::new();
    let failures = &mut failures;
    // actingd-config: the installer writes the next Runtime's configuration, the console
    // edits it, and the installer reads the current one.
    let config = negotiate(failures, Interface::ActingdConfig, installer, runtime);
    negotiate(failures, Interface::ActingdConfig, ui, runtime);
    if let Some(previous) = parties.previous {
        negotiate(failures, Interface::ActingdConfig, installer, previous);
    }
    // install-selection: the installer writes active.json; the Runtime and the UI read it.
    contain(failures, Interface::InstallSelection, installer, runtime);
    contain(failures, Interface::InstallSelection, installer, ui);
    if parties.manager {
        contain(failures, Interface::InstallSelection, ui, runtime);
    }
    // install-control: close the previous Runtime, start the next one.
    let close = parties
        .previous
        .map(|previous| negotiate(failures, Interface::InstallControl, installer, previous));
    let start = negotiate(failures, Interface::InstallControl, installer, runtime);
    // ledger: the next Runtime and the console read what the previous one wrote; the console
    // reads what the next one writes.
    if let Some(previous) = parties.previous {
        contain(failures, Interface::Ledger, previous, runtime);
        contain(failures, Interface::Ledger, previous, ui);
    }
    contain(failures, Interface::Ledger, runtime, ui);
    let client = negotiate(failures, Interface::RuntimeClient, ui, runtime);
    // package: the next Runtime runs the packs and this installer admits them.
    for bundle in parties.bundles {
        contain(failures, Interface::Package, bundle, runtime);
        contain(failures, Interface::Package, bundle, installer);
    }
    if parties.new_slot {
        let mut needs = Vec::new();
        if runtime.tools_layout != Some(2) {
            needs.push(format!(
                "tools-layout 2（实为 / is {}）",
                runtime
                    .tools_layout
                    .map_or_else(|| "—".to_string(), |layout| layout.to_string())
            ));
        }
        if !runtime
            .speaks
            .get(&Interface::ActingdConfig)
            .is_some_and(|range| range.contains(2))
        {
            needs.push("actingd-config ∋ 2".to_string());
        }
        if !runtime
            .speaks
            .get(&Interface::InstallSelection)
            .is_some_and(|range| range.contains(1))
        {
            needs.push("install-selection ∋ 1".to_string());
        }
        if !needs.is_empty() {
            failures.push(format!(
                "  新槽 / new slot: {} 需要 / needs {}：槽只含程序核心，更早的 Runtime 在槽里找工具 / a slot is the program core only, and an earlier Runtime looks for its tools in its slot (#359, #360)",
                runtime.named(),
                needs.join("、")
            ));
        }
    }
    let mut listed: Vec<String> = Vec::new();
    for failure in failures.drain(..) {
        if !listed.contains(&failure) {
            listed.push(failure);
        }
    }
    if !listed.is_empty() {
        return Err(Stop::Refused(format!(
            "接口不兼容 / Incompatible interfaces:\n{}\n安装未改动 / The installation was not changed",
            listed.join("\n")
        )));
    }
    let (Some(config), Some(start), Some(client)) = (config, start, client) else {
        return Err(Stop::Failed("接口协商没有结果 / The interface negotiation has no result".into()));
    };
    let start = Control::from_revision(start);
    let close = close.flatten().map(Control::from_revision);
    report.line(&format!(
        "接口协商 / Negotiated: actingd-config {config} · runtime-client {client} · install-control {}启动 / start {}",
        close.map_or_else(String::new, |close| format!("关闭 / close {} · ", control_name(close))),
        control_name(start)
    ))?;
    Ok(Agreed {
        close,
        start,
        previous: parties.previous.map(|previous| previous.commit.clone()),
    })
}

/// A release about to be laid out — a fresh install, a first migration from the old layout,
/// or an A/B upgrade, told apart as `upgrade::installed` does — against what the root holds
/// and this installer, with the bundles the run admits. Read only: it runs right after the
/// release is verified and its bundles read, before any question (review L4).
pub fn release(root: &Path, verified: &Verified, bundles: &[Bundle], report: Report<'_>) -> Result<Agreed, Stop> {
    let installer = installer()?;
    let mut runtime = program(&verified.runtime.manifest, "runtime", RUNTIME_REPOSITORY, report)?;
    runtime.tools_layout = tools_layout(&verified.tools.manifest)?;
    let ui = program(&verified.ui.manifest, "ui", UI_REPOSITORY, report)?;
    let bundles = bundles
        .iter()
        .map(|item| bundle(item, report))
        .collect::<Result<Vec<_>, _>>()?;
    let exists = |path: &Path| {
        path.try_exists()
            .map_err(|error| Stop::Failed(format!("Cannot inspect {}: {error}", path.display())))
    };
    let previous = if exists(&root.join(acui_installation::INSTALL_SELECTION_PATH))? {
        let baseline = acui_installation::Snapshot::read(root)?;
        Some(installed(&baseline.slot_root(), "runtime", "当前 / current runtime", report)?)
    } else if exists(&root.join("runtime").join(MANIFEST))? {
        let (old_runtime, old_ui) = crate::verify::initial_programs(root, report)?;
        // Identified and logged; the old console is replaced, so no edge reads it.
        let old_ui = program(&old_ui.manifest, "旧布局 / old-layout ui", UI_REPOSITORY, report)?;
        report.line(&old_ui.line())?;
        Some(program(&old_runtime.manifest, "旧布局 / old-layout runtime", RUNTIME_REPOSITORY, report)?)
    } else {
        None
    };
    check(
        &Parties {
            installer: &installer,
            runtime: &runtime,
            ui: &ui,
            previous: previous.as_ref(),
            manager: true,
            new_slot: true,
            bundles: &bundles,
        },
        report,
    )
}

/// An explicit rollback: the retained slot's programs against the current Runtime and this
/// installer. No new slot: a retained v0.11.1 slot keeps its own tools.
pub fn rollback(current: &Path, retained: &Path, report: Report<'_>) -> Result<Agreed, Stop> {
    let installer = installer()?;
    let previous = installed(current, "runtime", "当前 / current runtime", report)?;
    let runtime = installed(retained, "runtime", "保留槽 / retained runtime", report)?;
    let ui = installed(retained, "ui", "保留槽 / retained ui", report)?;
    check(
        &Parties {
            installer: &installer,
            runtime: &runtime,
            ui: &ui,
            previous: Some(&previous),
            manager: false,
            new_slot: false,
            bundles: &[],
        },
        report,
    )
}

/// The selected slot's programs against this installer and the bundles a run lays out without
/// changing programs: the instances step and a resource-only update. The Runtime it closes and
/// starts is the selected slot's own.
pub fn selected(root: &Path, bundles: &[Bundle], report: Report<'_>) -> Result<Agreed, Stop> {
    let installer = installer()?;
    let slot = acui_installation::Snapshot::read(root)?.slot_root();
    let runtime = installed(&slot, "runtime", "选中槽 / selected runtime", report)?;
    let ui = installed(&slot, "ui", "选中槽 / selected ui", report)?;
    let bundles = bundles
        .iter()
        .map(|item| bundle(item, report))
        .collect::<Result<Vec<_>, _>>()?;
    check(
        &Parties {
            installer: &installer,
            runtime: &runtime,
            ui: &ui,
            previous: Some(&runtime),
            manager: false,
            new_slot: false,
            bundles: &bundles,
        },
        report,
    )
}
