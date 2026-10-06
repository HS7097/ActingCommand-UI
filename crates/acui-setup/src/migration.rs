// SPDX-License-Identifier: GPL-3.0-only
//! First cold migration retains original programs and root configuration as an
//! intact backup, then establishes the fixed management and forwarding entries.
//! The root's `tools\` stays where it is: only files whose content changed are
//! replaced, and the replaced ones join the backup (Workflow #359).

use std::fs;
use std::path::{Path, PathBuf};

use acui_installation::InstallSlot;

use crate::bundle::Bundle;
use crate::generations::Writer;
use crate::verify::{Report, Verified};
use crate::{generations, install, lifecycle, maintenance, root_tools, slots, upgrade, vision_migration};

pub fn upgrade(
    writer: &Writer,
    verified: &Verified,
    bundles: &[Bundle],
    choose: maintenance::Choose<'_>,
    resolve: maintenance::Resolve<'_>,
    report: Report<'_>,
) -> Result<upgrade::Upgraded, String> {
    let root = writer.root();
    // v0.11.0 or v0.11.1 programs in the old layout; only v0.11.0 closes cold.
    let (runtime_sha, _) = crate::verify::initial_programs(root, report)?;
    let config = root.join("actingd.config.json");
    let transaction = maintenance::Transaction::read_config(&config)?;
    let original_config =
        acui_installation::read_bounded(&config, acui_installation::MAX_CONFIG_BYTES)?;
    let state_root = transaction.state_root()?;
    // Every decision is taken before any installation material changes.
    let planned = upgrade::configuration(
        root,
        &config,
        transaction.document.clone(),
        verified,
        bundles,
        choose,
        resolve,
        report,
    )?;
    let vision = vision_migration::plan(root, &planned.document)?;
    let programs = slots::materialize(writer, InstallSlot::A, verified, report)?;
    maintenance::place(&planned.prepared, root, report)?;
    let mut document = planned.document;
    if let Some(vision) = &vision {
        vision.apply(report)?;
        vision.rewrite(&mut document)?;
    }
    let tools = root_tools::plan(root, verified)?;
    let mut plan = writer.prepare(
        None,
        InstallSlot::A,
        &config,
        document,
        planned.qualify,
        report,
    )?;
    transaction.unchanged()?;
    let closed = lifecycle::close(
        root,
        &config,
        &state_root,
        None,
        &runtime_sha,
        report,
    )?;
    lifecycle::verify_ledger(
        &programs
            .actingd_exe
            .parent()
            .and_then(Path::parent)
            .ok_or("Candidate program root missing")?
            .to_path_buf(),
        &plan.snapshot.config_path()?,
        report,
    )?;
    transaction.unchanged()?;
    let backup = root.join(format!(
        "install/initial-backup-{}",
        plan.snapshot.selection.generation
    ));
    fs::create_dir(&backup)
        .map_err(|error| format!("Cannot create original installation backup: {error}"))?;
    let settings = crate::platform::console_settings_path()?;
    let settings_before = optional(&settings)?;
    if let Some(bytes) = &settings_before {
        generations::write_new(&backup.join("acui.toml"), bytes)?;
    }
    let mut migration = Migration {
        root,
        backup,
        moved: Vec::new(),
        settings,
        settings_before,
        settings_written: None,
        manager_created: false,
        tools: None,
    };
    // Native exclusion covers all original program files while their directories
    // move; the root tools that change are held by `root_tools`. Occupied UI/MCP
    // files stop only this migration, with the old layout intact.
    let mut occupancy = Vec::new();
    for name in ["runtime", "ui"] {
        occupancy.push(slots::NativeOccupancy::acquire(&root.join(name))?);
    }
    let changed = (|| {
        transaction.unchanged()?;
        for name in ["runtime", "ui", "actingd.config.json"] {
            fs::rename(root.join(name), migration.backup.join(name))
                .map_err(|error| format!("Cannot retain original {name}: {error}"))?;
            migration.moved.push(name);
        }
        if acui_installation::read_bounded(
            &migration.backup.join("actingd.config.json"),
            acui_installation::MAX_CONFIG_BYTES,
        )? != original_config
        {
            return Err("Original configuration changed during migration".into());
        }
        let backup_tools = migration.backup.join("tools");
        migration.tools = Some(tools.apply(root, &backup_tools, report)?);
        install::stable_entries(root, &programs.ui_dir.join("acforward.exe"), report)?;
        if root
            .join(acui_installation::MANAGER_DIRECTORY)
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Err(
                "An installation manager identity already exists during first migration".into(),
            );
        }
        migration.manager_created = true;
        install::install_manager(root, verified, report)?;
        if optional(&migration.settings)? != migration.settings_before {
            return Err("Console settings changed during migration".into());
        }
        install::write_console_settings(
            &migration.settings,
            &state_root,
            &generations::plain(&config),
            &generations::plain(&root.join("runtime").join(crate::runtime::ACTINGD)),
        )?;
        migration.settings_written = Some(acui_installation::read_bounded(
            &migration.settings,
            acui_installation::MAX_CONFIG_BYTES,
        )?);
        writer.commit(&mut plan)?;
        report.line(&format!("首次安装选择已提交，原配置与程序保留于 / First installation selection committed; original configuration and programs retained at {}", migration.backup.display()))
    })();
    if let Err(error) = changed {
        let selection = writer.restore_before_start(&mut plan);
        // A still-selected A/B installation cannot be put over a root layout.
        if let Err(selection) = selection {
            return Err(format!(
                "{error}; selection restoration failed: {selection}; all materials retained"
            ));
        }
        let restored = migration.restore();
        return Err(match restored {
            Ok(()) => format!(
                "{error}; original installation restored; Runtime remains stopped; candidate materials retained"
            ),
            Err(restore) => format!(
                "{error}; original installation restoration incomplete: {restore}; Runtime remains stopped"
            ),
        });
    }
    drop(occupancy);
    if closed.was_running {
        plan.mark_start_attempt()?;
    }
    let restarted = lifecycle::start(&plan.snapshot, closed, &verified.members.0, report)?;
    Ok(upgrade::outcome(
        &plan.snapshot,
        migration.backup,
        restarted,
    ))
}

fn optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
        Ok(_) => {
            acui_installation::read_bounded(path, acui_installation::MAX_CONFIG_BYTES).map(Some)
        }
    }
}

struct Migration<'a> {
    root: &'a Path,
    backup: PathBuf,
    moved: Vec<&'static str>,
    settings: PathBuf,
    settings_before: Option<Vec<u8>>,
    settings_written: Option<Vec<u8>>,
    manager_created: bool,
    /// The root-tool changes, put back before the original programs return.
    tools: Option<root_tools::Applied>,
}

impl Migration<'_> {
    fn restore(&self) -> Result<(), String> {
        let mut failures = Vec::new();
        if let Some(tools) = &self.tools {
            if let Err(error) = tools.undo() {
                failures.push(format!("Cannot restore the root tools: {error}"));
            }
        }
        if self.manager_created {
            let manager = self.root.join(acui_installation::MANAGER_DIRECTORY);
            match manager.try_exists() {
                Ok(true) => {
                    if let Err(error) = fs::rename(&manager, self.backup.join("retained-manager")) {
                        failures.push(format!("Cannot retain new manager identity: {error}"));
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    failures.push(format!("Cannot inspect new manager identity: {error}"))
                }
            }
        }
        for name in self.moved.iter().rev() {
            let path = self.root.join(name);
            let restored = (|| {
                if path.try_exists().map_err(|error| error.to_string())? {
                    fs::rename(&path, self.backup.join(format!("retained-{name}")))
                        .map_err(|error| format!("Cannot retain new {name}: {error}"))?;
                }
                fs::rename(self.backup.join(name), &path)
                    .map_err(|error| format!("Cannot restore original {name}: {error}"))
            })();
            if let Err(error) = restored {
                failures.push(error);
            }
        }
        if let Some(written) = &self.settings_written {
            let restored = (|| {
                if optional(&self.settings)?.as_ref() != Some(written) {
                    return Err("Console settings changed; original settings are retained in the migration backup".into());
                }
                match &self.settings_before {
                    Some(bytes) => fs::write(&self.settings, bytes)
                        .map_err(|error| format!("Cannot restore console settings: {error}")),
                    None => fs::rename(&self.settings, self.backup.join("retained-acui.toml"))
                        .map_err(|error| {
                            format!("Cannot retain newly created console settings: {error}")
                        }),
                }
            })();
            if let Err(error) = restored {
                failures.push(error);
            }
        } else if optional(&self.settings)? != self.settings_before {
            failures
                .push("Console settings differ from original; backup retained for recovery".into());
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}
