// SPDX-License-Identifier: GPL-3.0-only
//! Where this run's state root, actingd configuration and actingd executable
//! come from: an installed console takes them from its installation selection,
//! which `--state-root` may only repeat; otherwise `--state-root`, else
//! acui.toml. Neither is a silent default: with both missing the usage is the
//! answer. One function, so that what replaces this derivation replaces it whole.

use std::path::PathBuf;

use acui_installation::Snapshot;

use crate::settings::Settings;
use crate::strings::{fill, Labels};
use crate::Args;

/// What the launcher and the read face start from.
pub struct InstallPaths {
    /// The installation inputs of this process, retained across all UI actions.
    pub installation: Option<Snapshot>,
    pub state_root: PathBuf,
    pub actingd_config: Option<PathBuf>,
    pub actingd_exe: Option<PathBuf>,
}

/// Why there are no paths to start from.
pub enum InstallIssue {
    /// This process's installation selection could not be read, as
    /// acui-installation words it.
    Selection(String),
    /// `--state-root` names another root than the installation selected.
    StateRootDiffers { requested: PathBuf, selected: PathBuf },
    /// acui.toml's `state_root` is not absolute.
    StateRootNotAbsolute(PathBuf),
    /// Neither `--state-root` nor acui.toml names a state root.
    NoStateRoot,
}

impl InstallIssue {
    pub fn text(&self, labels: &Labels) -> String {
        match self {
            Self::Selection(error) => fill(labels.install_selection_failed, &[error]),
            Self::StateRootDiffers { requested, selected } => fill(
                labels.state_root_differs,
                &[&requested.display().to_string(), &selected.display().to_string()],
            ),
            Self::StateRootNotAbsolute(root) => {
                fill(labels.state_root_not_absolute, &[&root.display().to_string()])
            }
            Self::NoStateRoot => labels.usage.to_owned(),
        }
    }
}

pub fn resolve(args: &Args, settings: &Settings) -> Result<InstallPaths, InstallIssue> {
    let installation = Snapshot::for_current_process().map_err(InstallIssue::Selection)?;
    let mut stored_root = settings.state_root.clone();
    let mut actingd_config = settings.actingd_config.clone();
    let mut actingd_exe = settings.actingd_exe.clone();
    if let Some(snapshot) = &installation {
        let state_root = snapshot.state_root().map_err(InstallIssue::Selection)?;
        if let Some(requested) = args.state_root.as_ref().filter(|requested| *requested != &state_root) {
            return Err(InstallIssue::StateRootDiffers {
                requested: requested.clone(),
                selected: state_root,
            });
        }
        stored_root = Some(state_root);
        actingd_config = Some(snapshot.config_path().map_err(InstallIssue::Selection)?);
        actingd_exe = Some(snapshot.slot_root().join("runtime").join("actingcommand-actingd.exe"));
    }
    // `--state-root` is this run's; the file's `state_root` stands in for it.
    let state_root = match &args.state_root {
        Some(root) => root.clone(),
        None => match stored_root {
            Some(root) if root.is_absolute() => root,
            Some(root) => return Err(InstallIssue::StateRootNotAbsolute(root)),
            None => return Err(InstallIssue::NoStateRoot),
        },
    };
    Ok(InstallPaths { installation, state_root, actingd_config, actingd_exe })
}
