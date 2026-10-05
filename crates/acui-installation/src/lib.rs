// SPDX-License-Identifier: GPL-3.0-only
//! The installation inputs retained by one UI process. acsetup owns all writes.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use actingcommand_contract::installation::{
    process_slot_lock, InstallFileReference, InstallSelection, InstallSlot, InstalledProcess,
    INSTALL_ROOT_ENV, INSTALL_SELECTION_ENV, INSTALL_SELECTION_PATH, INSTALL_SELECTION_SCHEMA,
    MAX_INSTALL_SELECTION_BYTES,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const MAX_MATERIAL_BYTES: usize = 16 * 1024 * 1024;
pub const MANAGER_DIRECTORY: &str = "install/manager";
pub const OBSERVER_UI_ENV: &str = "ACTINGCOMMAND_OBSERVER_UI";

/// Byte identity is kept as well as the shared DTO: a configuration plan compares
/// the original selection and inputs again before committing its successor.
#[derive(Clone)]
pub struct Snapshot {
    // Clones retain the shared reader's one native slot occupancy.
    process: InstalledProcess,
    pub root: PathBuf,
    pub selection: InstallSelection,
    pub selection_bytes: Vec<u8>,
    pub config_bytes: Vec<u8>,
    pub provider_bytes: Option<Vec<u8>>,
}

impl Snapshot {
    pub fn read(root: &Path) -> Result<Self, String> {
        let process = InstalledProcess::read_active(root)
            .map_err(|error| error.to_string())?
            .ok_or("Installation selection is missing")?;
        Self::from_process(process)
    }

    pub fn from_bytes(root: &Path, selection_bytes: Vec<u8>) -> Result<Self, String> {
        let process = InstalledProcess::from_selection_bytes(root, &selection_bytes)
            .map_err(|error| error.to_string())?;
        Self::from_process(process)
    }

    pub fn for_launch(root: &Path) -> Result<Self, String> {
        let process = InstalledProcess::read(root)
            .map_err(|error| error.to_string())?
            .ok_or("Installation selection is missing")?;
        Self::from_process(process)
    }

    pub fn for_current_process() -> Result<Option<Self>, String> {
        actingcommand_contract::installation::process_installation()
            .map_err(|error| error.to_string())?
            .cloned()
            .map(Self::from_process)
            .transpose()
    }

    fn from_process(process: InstalledProcess) -> Result<Self, String> {
        let root = process.root();
        let selection = process.selection().clone();
        let selection_bytes = process.selection_json().as_bytes().to_vec();
        let config_bytes = process.config_bytes().map_err(|error| error.to_string())?;
        let provider_bytes = selection
            .provider
            .as_ref()
            .map(|reference| read_bounded(&root.join(&reference.path), MAX_CONFIG_BYTES))
            .transpose()?;
        let config: Value = serde_json::from_slice(&config_bytes)
            .map_err(|error| format!("Selected configuration is unreadable: {error}"))?;
        let configured_provider = config
            .get("vision_provider_manifest")
            .filter(|value| !value.is_null());
        match (&selection.provider, configured_provider) {
            (None, None) => {}
            (Some(reference), Some(Value::String(configured))) => {
                let configured = Path::new(configured);
                let config_path = process.config_path();
                let configured = if configured.is_absolute() {
                    configured.to_path_buf()
                } else {
                    config_path
                        .parent()
                        .ok_or("Configuration has no parent")?
                        .join(configured)
                };
                if configured
                    .canonicalize()
                    .map_err(|error| error.to_string())?
                    != root
                        .join(&reference.path)
                        .canonicalize()
                        .map_err(|error| error.to_string())?
                {
                    return Err("Selected provider does not match the configuration input".into());
                }
            }
            _ => return Err("Selected provider and configuration disagree".into()),
        }
        Ok(Self {
            root: root.to_path_buf(),
            process,
            selection,
            selection_bytes,
            config_bytes,
            provider_bytes,
        })
    }

    /// Inheritance never consults active.json: an already running process retains
    /// the complete selection with which it was started.
    pub fn inherited() -> Result<Option<Self>, String> {
        match (
            std::env::var_os(INSTALL_ROOT_ENV),
            std::env::var_os(INSTALL_SELECTION_ENV),
        ) {
            (None, None) => Ok(None),
            (Some(root), Some(_)) => Self::for_launch(Path::new(&root)).map(Some),
            _ => Err("Installation root and selection must be inherited together".into()),
        }
    }

    pub fn config_path(&self) -> Result<PathBuf, String> {
        Ok(self.process.config_path())
    }

    pub fn slot_root(&self) -> PathBuf {
        self.process.program_root()
    }

    pub fn members_bytes(&self) -> Result<Vec<u8>, String> {
        read_bounded(
            &self.root.join(&self.selection.members.path),
            MAX_MATERIAL_BYTES,
        )
    }

    pub fn ui_supports_configuration(&self) -> Result<bool, String> {
        let members = self.members_bytes()?;
        let ui = self.slot_root().join("ui");
        let manifest = read_bounded(&ui.join("BUILD-MANIFEST.json"), MAX_MATERIAL_BYTES)?;
        verify_ui_program(&members, &manifest, &ui.join("acui.exe"), "acui.exe")
    }

    pub fn state_root(&self) -> Result<PathBuf, String> {
        let document: Value =
            serde_json::from_slice(&self.config_bytes).map_err(|error| error.to_string())?;
        let path = PathBuf::from(
            document["state_root"]
                .as_str()
                .ok_or("Missing state_root")?,
        );
        if !path.is_absolute() {
            return Err("Shared state root must be absolute".into());
        }
        let location = path
            .canonicalize()
            .map_err(|error| format!("Shared state root is unavailable: {error}"))?;
        if location.starts_with(self.root.join("A")) || location.starts_with(self.root.join("B")) {
            return Err(
                "The shared state root must be absolute and outside both program slots".into(),
            );
        }
        Ok(path)
    }

    pub fn apply_to(&self, command: &mut Command) -> Result<(), String> {
        let text = std::str::from_utf8(&self.selection_bytes).map_err(|error| error.to_string())?;
        command
            .env(INSTALL_ROOT_ENV, &self.root)
            .env(INSTALL_SELECTION_ENV, text)
            .current_dir(&self.root);
        Ok(())
    }

    pub fn unchanged(&self) -> Result<(), String> {
        let current = Self::read(&self.root)?;
        if current.selection_bytes != self.selection_bytes
            || current.config_bytes != self.config_bytes
            || current.provider_bytes != self.provider_bytes
        {
            return Err(
                "Installation inputs changed after planning; replan against the current generation"
                    .into(),
            );
        }
        Ok(())
    }
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file =
        File::open(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(format!(
            "Installation input must contain 1..={limit} bytes: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn same_install_root(left: &Path, right: &Path) -> Result<bool, String> {
    if left == right {
        return Ok(true);
    }
    Ok(left.canonicalize().map_err(|error| {
        format!(
            "Cannot resolve installation root {}: {error}",
            left.display()
        )
    })? == right.canonicalize().map_err(|error| {
        format!(
            "Cannot resolve installation root {}: {error}",
            right.display()
        )
    })?)
}

/// Management identity is independent of the selected business slot. Its
/// original build manifest and MEMBERS remain beside the installation inputs.
pub fn verify_manager_material(members: &[u8], manifest: &[u8], program: &Path) -> Result<(), String> {
    if verify_ui_program(members, manifest, program, "acsetup.exe")? {
        Ok(())
    } else {
        Err("Management program does not declare the installation selection schema".into())
    }
}

pub fn manager_program(root: &Path) -> Result<PathBuf, String> {
    let evidence = root.join(MANAGER_DIRECTORY);
    let members = read_bounded(&evidence.join("MEMBERS.json"), MAX_MATERIAL_BYTES)?;
    let manifest = read_bounded(&evidence.join("BUILD-MANIFEST.json"), MAX_MATERIAL_BYTES)?;
    let program = root.join("ui/acsetup.exe");
    if !verify_ui_program(&members, &manifest, &program, "acsetup.exe")? {
        return Err(
            "The fixed acsetup does not support this installation selection contract".into(),
        );
    }
    Ok(program)
}

pub fn current_manager_root() -> Result<Option<PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let Some(directory) = executable
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "ui"))
    else {
        return Ok(None);
    };
    let root = directory
        .parent()
        .ok_or("Management executable has no installation root")?;
    if !root
        .join(MANAGER_DIRECTORY)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    let expected = manager_program(root)?;
    if executable
        .canonicalize()
        .map_err(|error| error.to_string())?
        != expected.canonicalize().map_err(|error| error.to_string())?
    {
        return Err("Configuration writes must use the fixed acsetup management entry".into());
    }
    Ok(Some(root.to_path_buf()))
}

fn verify_ui_program(
    members: &[u8],
    manifest: &[u8],
    program: &Path,
    name: &str,
) -> Result<bool, String> {
    let members: Value = serde_json::from_slice(members)
        .map_err(|error| format!("UI source MEMBERS is unreadable: {error}"))?;
    let manifest: Value = serde_json::from_slice(manifest)
        .map_err(|error| format!("UI build manifest is unreadable: {error}"))?;
    let commit = members["ui_sha"]
        .as_str()
        .ok_or("MEMBERS has no UI source")?;
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || manifest["repository"] != "HS7097/ActingCommand-UI"
        || manifest["commit_sha"] != commit
    {
        return Err("UI build manifest does not match its source MEMBERS".into());
    }
    let files = manifest["files"]
        .as_array()
        .ok_or("UI manifest has no files")?;
    let matching: Vec<_> = files.iter().filter(|file| file["path"] == name).collect();
    let [file] = matching.as_slice() else {
        return Err(format!("UI manifest must identify exactly one {name}"));
    };
    let size = file["size_bytes"]
        .as_u64()
        .ok_or("UI program size is missing")?;
    if size == 0 || size > 512 * 1024 * 1024 {
        return Err("UI program size is outside 1 byte..=512 MiB".into());
    }
    let expected_hash = file["sha256"]
        .as_str()
        .ok_or("UI program hash is missing")?;
    let mut input = File::open(program)
        .map_err(|error| format!("Cannot open {}: {error}", program.display()))?
        .take(size.saturating_add(1));
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("Cannot verify {}: {error}", program.display()))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        count += read as u64;
    }
    if count != size || format!("{:x}", hash.finalize()) != expected_hash {
        return Err(format!(
            "UI program differs from its build identity: {}",
            program.display()
        ));
    }
    match manifest.get("installation_selection_schema") {
        None => Ok(false),
        Some(Value::String(schema)) if schema == INSTALL_SELECTION_SCHEMA => Ok(true),
        Some(_) => Err("UI installation selection schema is unsupported".into()),
    }
}
