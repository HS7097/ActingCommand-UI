// SPDX-License-Identifier: GPL-3.0-only
//! The installation inputs retained by one UI process. acsetup owns all writes.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use actingcommand_contract::installation::{
    InstallFileReference, InstallSelection, InstallSlot, INSTALL_ROOT_ENV, INSTALL_SELECTION_ENV,
    INSTALL_SELECTION_PATH, INSTALL_SELECTION_SCHEMA, MAX_INSTALL_SELECTION_BYTES,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const MAX_MATERIAL_BYTES: usize = 16 * 1024 * 1024;

/// Byte identity is kept as well as the shared DTO: a configuration plan compares
/// the original selection and inputs again before committing its successor.
#[derive(Clone)]
pub struct Snapshot {
    pub root: PathBuf,
    pub selection: InstallSelection,
    pub selection_bytes: Vec<u8>,
    pub config_bytes: Vec<u8>,
    pub provider_bytes: Option<Vec<u8>>,
}

impl Snapshot {
    pub fn read(root: &Path) -> Result<Self, String> {
        let bytes = read_bounded(
            &root.join(INSTALL_SELECTION_PATH),
            MAX_INSTALL_SELECTION_BYTES,
        )?;
        Self::from_bytes(root, bytes)
    }

    pub fn from_bytes(root: &Path, selection_bytes: Vec<u8>) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Installation root must be absolute".into());
        }
        let selection =
            InstallSelection::from_json(&selection_bytes).map_err(|error| error.to_string())?;
        read_reference(root, &selection.members, MAX_MATERIAL_BYTES)?;
        let config_bytes = read_reference(root, &selection.config, MAX_CONFIG_BYTES)?;
        let provider_bytes = selection
            .provider
            .as_ref()
            .map(|reference| read_reference(root, reference, MAX_MATERIAL_BYTES))
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
                let config_path = reference_path(root, &selection.config)?;
                let configured = if configured.is_absolute() {
                    configured.to_path_buf()
                } else {
                    config_path
                        .parent()
                        .ok_or("Configuration has no parent")?
                        .join(configured)
                };
                if configured != reference_path(root, reference)? {
                    return Err("Selected provider does not match the configuration input".into());
                }
            }
            _ => return Err("Selected provider and configuration disagree".into()),
        }
        Ok(Self {
            root: root.to_path_buf(),
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
            (Some(root), Some(selection)) => {
                let selection = selection
                    .into_string()
                    .map_err(|_| "Inherited installation selection is not UTF-8")?;
                Self::from_bytes(Path::new(&root), selection.into_bytes()).map(Some)
            }
            _ => Err("Installation root and selection must be inherited together".into()),
        }
    }

    pub fn config_path(&self) -> Result<PathBuf, String> {
        reference_path(&self.root, &self.selection.config)
    }

    pub fn slot_root(&self) -> PathBuf {
        self.root.join(self.selection.slot.as_str())
    }

    pub fn state_root(&self) -> Result<PathBuf, String> {
        let document: Value =
            serde_json::from_slice(&self.config_bytes).map_err(|error| error.to_string())?;
        let path = PathBuf::from(
            document["state_root"]
                .as_str()
                .ok_or("Missing state_root")?,
        );
        if !path.is_absolute()
            || path.starts_with(self.root.join("A"))
            || path.starts_with(self.root.join("B"))
        {
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

pub fn reference_path(root: &Path, reference: &InstallFileReference) -> Result<PathBuf, String> {
    reference.resolve(root).map_err(|error| error.to_string())
}

pub fn read_reference(
    root: &Path,
    reference: &InstallFileReference,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let path = reference_path(root, reference)?;
    let bytes = read_bounded(&path, limit)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(format!(
            "Installation input hash mismatch: {}",
            path.display()
        ));
    }
    Ok(bytes)
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
