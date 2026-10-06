// SPDX-License-Identifier: GPL-3.0-only
//! Prepare the spare slot while the current owner runs, then consume the Host's
//! transition before committing one program/configuration selection.

use std::fs;
use std::path::{Path, PathBuf};

use acui_installation::{InstallSlot, Snapshot};
use serde::Deserialize;

use crate::bundle::Bundle;
use crate::generations::{Plan, Writer};
use crate::install::LaidOut;
use crate::verify::{Report, Verified, MANIFEST, PLATFORM_TOOLS};
use crate::{lifecycle, maintenance, migration, root_tools, slots, vision_migration};

#[derive(Clone)]
pub struct Installed {
    pub runtime_sha: String,
    pub ui_sha: String,
    pub adb: bool,
}

impl Installed {
    pub fn current(&self, wanted: &(String, String)) -> bool {
        self.adb && self.runtime_sha == wanted.0 && self.ui_sha == wanted.1
    }
}

pub fn ac_adb(root: &Path) -> PathBuf {
    root.join("tools").join(PLATFORM_TOOLS).join("adb.exe")
}

#[derive(Deserialize)]
struct Manifest {
    commit_sha: String,
}

pub fn installed(root: &Path) -> Result<Option<Installed>, String> {
    if root
        .join(acui_installation::INSTALL_SELECTION_PATH)
        .try_exists()
        .map_err(|error| format!("Cannot inspect installation selection: {error}"))?
    {
        let snapshot = Snapshot::read(root)?;
        let (runtime_sha, ui_sha) = members(&snapshot)?;
        // AC's adb is the root's own (Workflow #359), not a slot's.
        return Ok(Some(Installed {
            runtime_sha,
            ui_sha,
            adb: ac_adb(root).is_file(),
        }));
    }
    let runtime = root.join("runtime").join(MANIFEST);
    let config = root.join("actingd.config.json");
    match (runtime.try_exists(), config.try_exists()) {
        (Ok(false), Ok(false)) => return Ok(None),
        (Ok(true), Ok(true)) => {},
        (Err(error), _) | (_, Err(error)) => return Err(format!("Cannot inspect installation: {error}")),
        _ => return Err("Existing programs/configuration are incomplete; retained installation materials require recovery before continuing".into()),
    }
    let commit = |path: PathBuf| {
        let bytes = acui_installation::read_bounded(&path, acui_installation::MAX_MATERIAL_BYTES)?;
        serde_json::from_slice::<Manifest>(&bytes)
            .map(|manifest| manifest.commit_sha)
            .map_err(|error| format!("Manifest unreadable: {}: {error}", path.display()))
    };
    Ok(Some(Installed {
        runtime_sha: commit(runtime)?,
        ui_sha: commit(root.join("ui").join(MANIFEST))?,
        adb: ac_adb(root).is_file(),
    }))
}

fn members(snapshot: &Snapshot) -> Result<(String, String), String> {
    crate::verify::members_of(
        std::str::from_utf8(&snapshot.members_bytes()?).map_err(|error| error.to_string())?,
    )
}

pub const INSTALLED_MEMBERS: &str = "installed-members.json";

fn published(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let at = value.get("published_at_utc")?.as_str()?;
    let shaped = at.len() == 20
        && at.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            10 => byte == b'T',
            13 | 16 => byte == b':',
            19 => byte == b'Z',
            _ => byte.is_ascii_digit(),
        });
    shaped.then(|| at.to_string())
}

fn current_members(root: &Path) -> Result<String, String> {
    if root
        .join(acui_installation::INSTALL_SELECTION_PATH)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return String::from_utf8(Snapshot::read(root)?.members_bytes()?)
            .map_err(|error| error.to_string());
    }
    let text = fs::read_to_string(root.join(INSTALLED_MEMBERS))
        .map_err(|error| format!("Cannot read installed release identity: {error}"))?;
    let recorded = crate::verify::members_of(&text)?;
    let actual = installed(root)?.ok_or("No installed release")?;
    if recorded != (actual.runtime_sha, actual.ui_sha) {
        return Err("Installed release record differs from actual manifests".into());
    }
    Ok(text)
}

pub fn downgrade(root: &Path, wanted: &str) -> Option<String> {
    let have = current_members(root).and_then(|text| {
        published(&text).ok_or("Installed release has no comparable publication time".into())
    });
    let want = published(wanted).ok_or("Candidate has no comparable publication time".to_string());
    let which = crate::verify::members_of(wanted)
        .map(|(runtime, ui)| format!("runtime {runtime} · ui {ui}"))
        .unwrap_or_else(|error| format!("Unverified candidate: {error}"));
    match (have, want) {
        (Ok(have), Ok(want)) if want >= have => None,
        (Ok(have), Ok(want)) => Some(format!(
            "此发布件更早 / Earlier release: {which}; {want} < {have}. 切换须通过当前数据的冷态验证 / Switching requires cold verification against current data"
        )),
        (Err(reason), _) | (_, Err(reason)) => Some(format!(
            "发布时间不能确定版本顺序 / Publication order is unproved: {reason}; {which}. 切换须通过当前数据的冷态验证 / Switching requires cold verification against current data"
        )),
    }
}

pub fn record_members(root: &Path, download: &Path) -> Result<(), String> {
    // An A/B install takes its release identity only from the selected MEMBERS.
    let selected = Snapshot::read(root)?;
    let downloaded = acui_installation::read_bounded(
        &download.join("MEMBERS.json"),
        acui_installation::MAX_MATERIAL_BYTES,
    )?;
    if selected.members_bytes()? != downloaded {
        return Err("Selected MEMBERS differs from downloaded release".into());
    }
    Ok(())
}

pub struct Upgraded {
    pub laid_out: LaidOut,
    pub previous: PathBuf,
    pub restarted: Option<PathBuf>,
    pub generation: u64,
}

pub fn upgrade(
    root: &Path,
    verified: &Verified,
    bundles: &[Bundle],
    choose: maintenance::Choose<'_>,
    resolve: maintenance::Resolve<'_>,
    report: Report<'_>,
) -> Result<Upgraded, String> {
    let writer = Writer::acquire(root)?;
    slots::initialize(&writer)?;
    let root = writer.root();
    if !root
        .join(acui_installation::INSTALL_SELECTION_PATH)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return migration::upgrade(&writer, verified, bundles, choose, resolve, report);
    }
    acui_installation::manager_program(root)?;
    let baseline = Snapshot::read(root)?;
    let previous = baseline.slot_root();
    let source_config = baseline.config_path()?;
    let state_root = baseline.state_root()?;
    let (runtime_sha, _) = members(&baseline)?;
    let target = other(baseline.selection.slot);
    let document =
        serde_json::from_slice(&baseline.config_bytes).map_err(|error| error.to_string())?;
    // Every decision is taken before any installation material changes.
    let planned = configuration(
        root,
        &source_config,
        document,
        verified,
        bundles,
        choose,
        resolve,
        report,
    )?;
    let vision = vision_migration::plan(root, &planned.document)?;
    slots::materialize(&writer, target, verified, report)?;
    maintenance::place(&planned.prepared, root, report)?;
    let mut document = planned.document;
    if let Some(vision) = &vision {
        vision.apply(report)?;
        vision.rewrite(&mut document)?;
    }
    let tools = root_tools::plan(root, verified)?;
    // New root tools (platform-tools above all) before the new slot's check-config
    // (review R-F1); replacements wait until the Runtime owner has closed.
    tools.add(root, report)?;
    let mut plan = writer.prepare(
        Some(baseline.clone()),
        target,
        &source_config,
        document,
        planned.qualify,
        report,
    )?;
    baseline.unchanged()?;
    // A tool still running from what the root-tool update moves stops the run
    // here, before the Runtime is closed.
    let owner: Vec<u32> = lifecycle::owner_pid(&state_root).into_iter().collect();
    slots::precheck(&[], &tools.moving(root), &owner)?;
    let closed = lifecycle::close(
        &previous,
        &source_config,
        &state_root,
        Some(&baseline),
        &runtime_sha,
        report,
    )?;
    complete(
        &writer,
        &mut plan,
        closed,
        &verified.members.0,
        previous,
        Some(&tools),
        report,
    )
}

pub fn other(slot: InstallSlot) -> InstallSlot {
    match slot {
        InstallSlot::A => InstallSlot::B,
        InstallSlot::B => InstallSlot::A,
    }
}

/// A configuration plan: the successor document, whether its maintenance
/// chains need full qualification, the staged bundles still to be placed, and
/// the maintenance bindings that differ from the current configuration.
pub struct Planned {
    pub document: serde_json::Value,
    pub qualify: bool,
    pub prepared: Vec<maintenance::Prepared>,
    pub conflicts: Vec<maintenance::Conflict>,
}

/// Everything an upgrade decides, computed from staging alone: nothing under
/// the installation changes here, and the conflicts are left unanswered.
pub fn plan_configuration(
    root: &Path,
    source_config: &Path,
    mut document: serde_json::Value,
    verified: &Verified,
    bundles: &[Bundle],
    choose: maintenance::Choose<'_>,
    report: Report<'_>,
) -> Result<Planned, String> {
    crate::generations::rebase_config(
        &mut document,
        source_config
            .parent()
            .ok_or("Configuration has no parent")?,
    )?;
    let prepared =
        maintenance::prepare(bundles, &verified.staging.join("resource-packages"), report)?;
    let selected = maintenance::upgrade_selections(&mut document, root, &prepared, choose, report)?;
    let old_source = crate::generations::plain(source_config)
        .display()
        .to_string();
    let (qualify, conflicts) =
        maintenance::augment(&mut document, root, &prepared, &selected, &old_source)?;
    Ok(Planned {
        document,
        qualify,
        prepared,
        conflicts,
    })
}

/// `plan_configuration`, then one decision over all its conflicts, written
/// into the document. The caller places `prepared` once the slot is laid out.
#[allow(clippy::too_many_arguments)]
pub fn configuration(
    root: &Path,
    source_config: &Path,
    document: serde_json::Value,
    verified: &Verified,
    bundles: &[Bundle],
    choose: maintenance::Choose<'_>,
    resolve: maintenance::Resolve<'_>,
    report: Report<'_>,
) -> Result<Planned, String> {
    let mut planned =
        plan_configuration(root, source_config, document, verified, bundles, choose, report)?;
    maintenance::decide(&mut planned.document, &planned.conflicts, resolve, report)?;
    Ok(planned)
}

/// `tools`: an upgrade's root-tool update; a switch (rollback) never touches the root.
fn complete(
    writer: &Writer,
    plan: &mut Plan,
    closed: lifecycle::Closed,
    runtime_sha: &str,
    previous: PathBuf,
    tools: Option<&root_tools::Plan>,
    report: Report<'_>,
) -> Result<Upgraded, String> {
    // The same gate also covers explicit rollback. It is evaluated on this
    // transaction's actual data, after the original owner has closed.
    lifecycle::verify_ledger(
        &plan.snapshot.slot_root(),
        &plan.snapshot.config_path()?,
        report,
    )?;
    let applied = match tools {
        Some(tools) => Some(
            tools
                .apply(
                    writer.root(),
                    &writer.root().join(format!(
                        "install/root-tools-{}",
                        plan.snapshot.selection.generation
                    )),
                    report,
                )
                .map_err(|error| format!("{error}; selection unchanged; Runtime remains stopped"))?,
        ),
        None => None,
    };
    let committed = writer.commit(plan).and_then(|()| {
        report.line("安装选择已原子提交 / Installation selection committed atomically")
    });
    if let Err(error) = committed {
        let tools = match applied.as_ref().map(root_tools::Applied::undo) {
            None => String::new(),
            Some(Ok(())) => "; 根工具已复原 / root tools restored".to_string(),
            Some(Err(undo)) => format!("; 根工具复原未完成 / root tools restoration incomplete: {undo}"),
        };
        return Err(match writer.restore_before_start(plan) {
            Ok(()) => format!("{error}{tools}; original selection restored; Runtime remains stopped"),
            Err(restore) => {
                format!("{error}{tools}; selection restoration failed: {restore}; Runtime remains stopped")
            }
        });
    }
    if closed.was_running {
        plan.mark_start_attempt()?;
    }
    let restarted = lifecycle::start(&plan.snapshot, closed, runtime_sha, report)?;
    Ok(outcome(&plan.snapshot, previous, restarted))
}

pub fn outcome(snapshot: &Snapshot, previous: PathBuf, restarted: Option<PathBuf>) -> Upgraded {
    Upgraded {
        laid_out: LaidOut {
            ui_dir: snapshot.root.join("ui"),
            tools_dir: snapshot.root.join("tools"),
            actingd_exe: snapshot.root.join("runtime").join(crate::runtime::ACTINGD),
            acui_exe: snapshot.root.join("ui/acui.exe"),
        },
        previous,
        restarted,
        generation: snapshot.selection.generation,
    }
}

/// Explicit management entry for a retained spare slot. A slot of this layout
/// keeps current business settings in a new generation for the target; a slot
/// whose Runtime predates the vision model folders gets its own last generation
/// back (`Writer::reselect`).
pub fn rollback(root: &Path, report: Report<'_>) -> Result<Upgraded, String> {
    let writer = Writer::acquire(root)?;
    acui_installation::manager_program(writer.root())?;
    let baseline = Snapshot::read(writer.root())?;
    let target = other(baseline.selection.slot);
    let guard =
        actingcommand_contract::installation::InstallSlotLock::try_shared(writer.root(), target)
            .map_err(|error| error.to_string())?;
    let (retained, predates_vision) =
        crate::verify::installed_slot(&writer.root().join(target.as_str()), report)?;
    let config = baseline.config_path()?;
    let programs = baseline.slot_root();
    let state_root = baseline.state_root()?;
    let (runtime_sha, _) = members(&baseline)?;
    let mut plan = if predates_vision {
        // Its Runtime cannot read this configuration (#360 §10.3).
        writer.reselect(baseline.clone(), target, report)?
    } else {
        let document =
            serde_json::from_slice(&baseline.config_bytes).map_err(|error| error.to_string())?;
        writer.prepare(
            Some(baseline.clone()),
            target,
            &config,
            document,
            true,
            report,
        )?
    };
    guard.release().map_err(|error| error.to_string())?;
    baseline.unchanged()?;
    let closed = lifecycle::close(
        &programs,
        &config,
        &state_root,
        Some(&baseline),
        &runtime_sha,
        report,
    )?;
    complete(
        &writer,
        &mut plan,
        closed,
        &retained.0,
        programs,
        None,
        report,
    )
}

pub fn rollback_from_entry() -> Result<(), String> {
    if std::env::args_os().count() != 2 {
        return Err(
            "--rollback takes no arguments; it uses the fixed management entry's installation"
                .into(),
        );
    }
    let root = acui_installation::current_manager_root()?
        .ok_or("Rollback requires the fixed acsetup management entry")?;
    let mut log = crate::log::InstallLog::create(&root, crate::log::unix_ms())
        .map_err(|error| error.to_string())?;
    let mut report = |line: &str| {
        log.line(line)
            .map_err(|error| format!("Rollback log failed: {error}"))
    };
    let done = rollback(&root, &mut report)?;
    println!("Selected installation generation {}", done.generation);
    Ok(())
}
