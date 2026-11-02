// SPDX-License-Identifier: GPL-3.0-only
//! Where this run's state root, actingd configuration and actingd executable
//! come from: an installed console takes them from its installation selection,
//! which `--state-root` may only repeat; otherwise `--state-root`, else
//! acui.toml. Neither is a silent default: with both missing the workbench says so
//! in plain words, and the technical log gets the usage. One function, so that what
//! replaces this derivation replaces it whole.
//! So too for where the Runtime's `actingctl` is, and how it is pointed at this
//! run's configuration (`tool_paths`).

use std::fmt;
use std::path::{Path, PathBuf};

use acui_installation::Snapshot;

use crate::settings::Settings;
use crate::strings::{fill, Labels};
use crate::Args;

const ACTINGCTL: &str = "actingctl.exe";
/// The configuration of a root without an installation selection, as `actingctl` finds it.
const ROOT_CONFIG: &str = "actingd.config.json";

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
    /// What the workbench's red bar says of it. No state root at all is said in plain
    /// words; the command line and the keys that name one are the log's (`log_text`).
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
            Self::NoStateRoot => labels.no_state_root.to_owned(),
        }
    }

    /// What the technical log gets: the same, and the usage too when no state root
    /// is named at all.
    pub fn log_text(&self, labels: &Labels) -> String {
        match self {
            Self::NoStateRoot => format!("{}\n{}", labels.no_state_root, labels.usage),
            Self::Selection(_) | Self::StateRootDiffers { .. } | Self::StateRootNotAbsolute(_) => {
                self.text(labels)
            }
        }
    }
}

/// How to run the Runtime's `actingctl` for this run.
pub struct ToolPaths {
    pub actingctl: PathBuf,
    /// `--root`, for a run without an installation selection.
    pub root: Option<PathBuf>,
    /// Handed to the child, which then finds the same selection: `packs` takes no `--root`,
    /// and `watchdog`, which requires one, takes this installation's root.
    pub installation: Option<Snapshot>,
}

/// Why this run cannot tell how to run `actingctl`; for the log.
#[derive(Clone)]
pub enum ToolIssue {
    NoActingdExe,
    /// The actingd executable is not at `<absolute root>\runtime\<file>`.
    NotUnderRuntime(PathBuf),
    NoActingdConfig,
    /// The actingd configuration is not the one `--root` would read.
    OtherConfig { config: PathBuf, expected: PathBuf },
}

impl fmt::Display for ToolIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoActingdExe => write!(formatter, "no actingd executable is set"),
            Self::NotUnderRuntime(exe) => {
                write!(formatter, "{} is not in a runtime folder of a root", exe.display())
            }
            Self::NoActingdConfig => write!(formatter, "no actingd configuration is set"),
            Self::OtherConfig { config, expected } => {
                write!(formatter, "{} is not {}", config.display(), expected.display())
            }
        }
    }
}

/// An installed run uses its own slot's `actingctl`, with its own selection. Any
/// other run goes by where its actingd is: `<root>\runtime\`, `actingctl` beside it,
/// and only when the configuration set is the one `--root <root>` reads, so that the
/// answer is about the configuration this run starts the Runtime with.
pub fn tool_paths(
    installation: Option<&Snapshot>,
    actingd_config: Option<&Path>,
    actingd_exe: Option<&Path>,
) -> Result<ToolPaths, ToolIssue> {
    if let Some(snapshot) = installation {
        return Ok(ToolPaths {
            actingctl: snapshot.slot_root().join("runtime").join(ACTINGCTL),
            root: None,
            installation: Some(snapshot.clone()),
        });
    }
    let exe = actingd_exe.ok_or(ToolIssue::NoActingdExe)?;
    let runtime = exe
        .parent()
        .filter(|folder| folder.file_name().is_some_and(|name| name.eq_ignore_ascii_case("runtime")));
    let root = runtime
        .and_then(Path::parent)
        .filter(|root| root.is_absolute())
        .ok_or_else(|| ToolIssue::NotUnderRuntime(exe.to_path_buf()))?;
    let config = actingd_config.ok_or(ToolIssue::NoActingdConfig)?;
    let expected = root.join(ROOT_CONFIG);
    if config != expected.as_path() {
        return Err(ToolIssue::OtherConfig { config: config.to_path_buf(), expected });
    }
    Ok(ToolPaths {
        actingctl: exe.with_file_name(ACTINGCTL),
        root: Some(root.to_path_buf()),
        installation: None,
    })
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
