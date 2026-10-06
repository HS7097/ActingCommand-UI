// SPDX-License-Identifier: GPL-3.0-only
//! acsetup's sole configuration writer and atomic installation selection.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use acui_installation::{
    read_bounded, sha256, InstallFileReference, InstallSelection, InstallSlot, Snapshot,
    INSTALL_SELECTION_PATH, INSTALL_SELECTION_SCHEMA, MAX_CONFIG_BYTES,
    MAX_INSTALL_SELECTION_BYTES, MAX_MATERIAL_BYTES,
};
use serde_json::Value;

use crate::verify::Report;

pub struct Writer {
    root: PathBuf,
    // All acsetup entry points share this lock, including configuration edits.
    _lock: File,
}

pub struct Plan {
    pub snapshot: Snapshot,
    baseline: Option<Snapshot>,
    candidate: PathBuf,
    committed: bool,
    start_attempted: bool,
}

impl Writer {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn acquire(root: &Path) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Installation root must be absolute".into());
        }
        let directory = root.join("install");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("Cannot create installation directory: {error}"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("writer.lock"))
            .map_err(|error| format!("Cannot open acsetup writer lock: {error}"))?;
        lock.try_lock()
            .map_err(|error| format!("Installation/configuration writer is occupied: {error}"))?;
        Ok(Self {
            root: fs::canonicalize(root)
                .map_err(|error| format!("Cannot resolve installation root: {error}"))?,
            _lock: lock,
        })
    }

    /// Planning creates only private, immutable inputs. The active selection is
    /// compared again by commit, after lifecycle work has closed the old owner.
    pub fn prepare(
        &self,
        baseline: Option<Snapshot>,
        slot: InstallSlot,
        source_config: &Path,
        mut document: Value,
        qualify: bool,
        report: Report<'_>,
    ) -> Result<Plan, String> {
        self.check_baseline(baseline.as_ref())?;
        let slot_guard =
            actingcommand_contract::installation::InstallSlotLock::try_shared(&self.root, slot)
                .map_err(|error| error.to_string())?;
        let source_root = source_config
            .parent()
            .ok_or("Configuration has no parent")?;
        rebase_config(&mut document, source_root)?;
        // The v0.3 provider manifest is retired (#360): `vision_migration` turns it
        // into model folders and a `vision` section before a generation is written.
        if document
            .get("vision_provider_manifest")
            .is_some_and(|value| !value.is_null())
        {
            return Err("配置仍引用 vision_provider_manifest，须先迁移到视觉模型文件夹 / The configuration still names a vision_provider_manifest; it must be migrated to the vision model folders first".into());
        }
        let state_root = Path::new(
            document["state_root"]
                .as_str()
                .ok_or("Missing state_root")?,
        );
        let state_location = fs::canonicalize(state_root)
            .map_err(|error| format!("Cannot resolve shared state root: {error}"))?;
        let install_location = fs::canonicalize(&self.root)
            .map_err(|error| format!("Cannot resolve installation root: {error}"))?;
        if !state_root.is_absolute()
            || state_location.starts_with(install_location.join("A"))
            || state_location.starts_with(install_location.join("B"))
        {
            return Err("The shared state root must be absolute and outside both slots".into());
        }
        let next = baseline.as_ref().map_or(Ok(1), |old| {
            old.selection
                .generation
                .checked_add(1)
                .ok_or("Configuration generation exhausted")
        })?;
        let now = u64::try_from(crate::log::unix_ms())
            .map_err(|_| "Configuration generation timestamp overflow")?;
        let generation = next.max(now);
        let relative = format!("install/generations/{generation}");
        let directory = self.root.join(&relative);
        fs::create_dir_all(self.root.join("install/generations"))
            .map_err(|error| format!("Cannot create generations directory: {error}"))?;
        fs::create_dir(&directory).map_err(|error| {
            format!(
                "Cannot create fresh generation {}: {error}",
                directory.display()
            )
        })?;
        let programs = self.root.join(slot.as_str());
        let config_path = directory.join("actingd.config.json");
        if qualify {
            crate::maintenance::validate(&document, &directory, report)?;
        }
        let mut config_bytes =
            serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
        config_bytes.push(b'\n');
        if config_bytes.len() > MAX_CONFIG_BYTES {
            return Err("Configuration exceeds 1 MiB".into());
        }
        write_new(&config_path, &config_bytes)?;
        let members_path = format!("{}/MEMBERS.json", slot.as_str());
        let members_bytes = read_bounded(&self.root.join(&members_path), MAX_MATERIAL_BYTES)?;
        let selection = InstallSelection {
            schema_version: INSTALL_SELECTION_SCHEMA.into(),
            slot,
            generation,
            members: InstallFileReference {
                path: members_path,
                sha256: sha256(&members_bytes),
            },
            config: InstallFileReference {
                path: format!("{relative}/actingd.config.json"),
                sha256: sha256(&config_bytes),
            },
            // No generation carries a provider manifest any more (#360 §10).
            provider: None,
        };
        let mut bytes = serde_json::to_vec_pretty(&selection).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        let snapshot = Snapshot::from_bytes(&self.root, bytes.clone())?;
        slot_guard.release().map_err(|error| error.to_string())?;
        crate::runtime::check_config(
            &programs.join("runtime").join(crate::runtime::ACTINGD),
            &config_path,
        )?;
        let candidate = directory.join("selection.json");
        write_new(&candidate, &bytes)?;
        self.check_baseline(baseline.as_ref())?;
        report.line(&format!(
            "私有配置代际已准备 / Private configuration generation prepared: {generation}"
        ))?;
        Ok(Plan {
            snapshot,
            baseline,
            candidate,
            committed: false,
            start_attempted: false,
        })
    }

    pub fn commit(&self, plan: &mut Plan) -> Result<(), String> {
        if plan.snapshot.root != self.root || plan.committed {
            return Err("Selection plan does not belong to this uncommitted transaction".into());
        }
        self.check_baseline(plan.baseline.as_ref())?;
        let bytes = read_bounded(&plan.candidate, MAX_INSTALL_SELECTION_BYTES)?;
        if bytes != plan.snapshot.selection_bytes {
            return Err("Prepared installation selection changed".into());
        }
        Snapshot::from_bytes(&self.root, bytes.clone())?;
        let temporary = self.root.join(format!(
            "install/active-{}.json",
            plan.snapshot.selection.generation
        ));
        write_new(&temporary, &bytes)?;
        self.check_baseline(plan.baseline.as_ref())?;
        fs::rename(&temporary, self.root.join(INSTALL_SELECTION_PATH))
            .map_err(|error| format!("Atomic installation selection commit failed: {error}"))?;
        plan.committed = true;
        plan.snapshot.unchanged()
    }

    /// Only the transaction that has not attempted a new Runtime start can
    /// restore its own exact selection. Configuration generations remain intact.
    pub fn restore_before_start(&self, plan: &mut Plan) -> Result<(), String> {
        if plan.start_attempted {
            return Err(
                "Runtime start was attempted; automatic slot restoration is forbidden".into(),
            );
        }
        if !plan.committed {
            return self.check_baseline(plan.baseline.as_ref());
        }
        plan.snapshot.unchanged()?;
        let Some(previous) = plan.baseline.as_ref() else {
            let retained = plan.candidate.with_file_name("retained-active.json");
            if retained.try_exists().map_err(|error| error.to_string())? {
                return Err("Retained first selection already exists".into());
            }
            fs::rename(self.root.join(INSTALL_SELECTION_PATH), &retained).map_err(|error| {
                format!(
                    "Cannot retain first selection before restoring the original layout: {error}"
                )
            })?;
            plan.committed = false;
            return self.check_baseline(None);
        };
        Snapshot::from_bytes(&self.root, previous.selection_bytes.clone())?;
        let temporary = self.root.join(format!(
            "install/restore-{}.json",
            plan.snapshot.selection.generation
        ));
        write_new(&temporary, &previous.selection_bytes)?;
        plan.snapshot.unchanged()?;
        fs::rename(&temporary, self.root.join(INSTALL_SELECTION_PATH)).map_err(|error| {
            format!("Atomic installation selection restoration failed: {error}")
        })?;
        plan.committed = false;
        previous.unchanged()
    }

    fn check_baseline(&self, baseline: Option<&Snapshot>) -> Result<(), String> {
        match baseline {
            Some(baseline) if baseline.root == self.root => baseline.unchanged(),
            Some(_) => Err("Configuration baseline belongs to another installation".into()),
            None => match fs::metadata(self.root.join(INSTALL_SELECTION_PATH)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("Cannot inspect installation selection: {error}")),
                Ok(_) => {
                    Err("An installation selection already exists; replan its successor".into())
                }
            },
        }
    }
}

impl Plan {
    pub fn mark_start_attempt(&mut self) -> Result<(), String> {
        if !self.committed {
            return Err("Cannot start an unselected installation generation".into());
        }
        self.start_attempted = true;
        Ok(())
    }
}

/// The console sends its edited document through a pipe. Only acsetup creates
/// the successor generation and compares the inherited baseline before commit.
pub fn commit_from_stdin() -> Result<(), String> {
    if std::env::args_os().count() != 2 {
        return Err(
            "--commit-config takes its input through stdin and the installation snapshot".into(),
        );
    }
    let root = acui_installation::current_manager_root()?
        .ok_or("Configuration editing requires the fixed acsetup management entry")?;
    let baseline = match Snapshot::inherited()? {
        Some(snapshot) if acui_installation::same_install_root(&snapshot.root, &root)? => snapshot,
        Some(_) => {
            return Err(
                "Management entry and configuration proposal belong to different installations"
                    .into(),
            );
        }
        None => Snapshot::read(&root)?,
    };
    let writer = Writer::acquire(&baseline.root)?;
    baseline.unchanged()?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read configuration proposal: {error}"))?;
    if bytes.is_empty() || bytes.len() > MAX_CONFIG_BYTES {
        return Err("Configuration proposal must contain 1..=1 MiB".into());
    }
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Configuration proposal is unreadable: {error}"))?;
    let current: Value =
        serde_json::from_slice(&baseline.config_bytes).map_err(|error| error.to_string())?;
    if document.get("state_root") != current.get("state_root") {
        return Err("Configuration editing cannot move the shared state root".into());
    }
    let mut log = crate::log::InstallLog::create(&baseline.root, crate::log::unix_ms())
        .map_err(|error| format!("Cannot create configuration transaction log: {error}"))?;
    let mut report = |line: &str| {
        log.line(line)
            .map_err(|error| format!("Configuration transaction log failed: {error}"))
    };
    let config = baseline.config_path()?;
    let slot = baseline.selection.slot;
    let mut plan = writer.prepare(
        Some(baseline),
        slot,
        &config,
        document,
        true,
        &mut report,
    )?;
    writer.commit(&mut plan)?;
    let response =
        serde_json::to_vec(&plan.snapshot.selection).map_err(|error| error.to_string())?;
    std::io::stdout().write_all(&response).map_err(|error| {
        format!("Configuration was committed, but its response could not be delivered: {error}")
    })
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Cannot write {}: {error}", path.display()))
}

/// The plain spelling of a path: `\\?\X:\…` as `X:\…` and `\\?\UNC\server\share\…`
/// as `\\server\share\…`; any other path unchanged. The writer's root is canonical
/// (verbatim on Windows), but no installation input ever carries that prefix
/// (Workflow #359): every path acsetup writes into a configuration, a provider
/// manifest or the console's settings goes through here.
pub fn plain(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let head = match prefix.kind() {
        Prefix::VerbatimDisk(disk) => format!("{}:", disk as char),
        Prefix::VerbatimUNC(server, share) => format!(
            r"\\{}\{}",
            server.to_string_lossy(),
            share.to_string_lossy()
        ),
        _ => return path.to_path_buf(),
    };
    let mut plain = PathBuf::from(head);
    for component in components {
        plain.push(component.as_os_str());
    }
    plain
}

fn absolute_field(document: &mut Value, pointer: &str, root: &Path) -> Result<(), String> {
    let Some(value) = document.pointer_mut(pointer) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let text = value
        .as_str()
        .ok_or_else(|| format!("Configuration path is not a string: {pointer}"))?;
    let path = Path::new(text);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let written = plain(&absolute).to_string_lossy().into_owned();
    if written != text {
        *value = Value::from(written);
    }
    Ok(())
}

/// These paths are resolved against config.source_root by the formal Runtime
/// parser. Moving the document into a private generation preserves that meaning.
pub fn rebase_config(document: &mut Value, source_root: &Path) -> Result<(), String> {
    if !source_root.is_absolute() {
        return Err("Configuration source directory must be absolute".into());
    }
    absolute_field(document, "/vision_provider_manifest", source_root)?;
    if let Some(instances) = document.get_mut("instances").and_then(Value::as_array_mut) {
        for instance in instances {
            absolute_field(instance, "/resource_package", source_root)?;
            absolute_field(instance, "/startup_package/package", source_root)?;
        }
    }
    if let Some(packages) = document
        .get_mut("prerequisite_packages")
        .and_then(Value::as_array_mut)
    {
        for package in packages {
            absolute_field(package, "/package_path", source_root)?;
        }
    }
    if let Some(procedures) = document
        .pointer_mut("/policy/procedure_manifest")
        .and_then(Value::as_array_mut)
    {
        for procedure in procedures {
            absolute_field(procedure, "/scheduled_execution/package_path", source_root)?;
        }
    }
    for key in ["tasks", "pools", "activity", "timeline"] {
        absolute_field(document, &format!("/policy/catalog/{key}"), source_root)?;
    }
    Ok(())
}
