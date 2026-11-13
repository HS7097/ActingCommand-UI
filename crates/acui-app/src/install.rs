// SPDX-License-Identifier: GPL-3.0-only
//! Where this run's state root, actingd configuration and actingd executable
//! come from, decided once at the start (`place`, then `resolve`), in four tiers:
//!
//! 1. An installation selection (a slot of an A/B installation): all three from it,
//!    which `--state-root` and `--root` may only repeat.
//! 2. `--root <folder>`: a single-location root named on the command line, one that
//!    holds `runtime\BUILD-MANIFEST.json` and no installation selection.
//! 3. Where this program is: `<root>\ui\acui.exe` of such a root. A console that sits
//!    in a root may be given no other one by `--root`.
//! 4. Neither: `--state-root`, else acui.toml's `state_root`, and the two paths from
//!    acui.toml, as ever.
//!
//! In a root all three come from it, as the Runtime's own tools find them: the
//! configuration `<root>\actingd.config.json`, the state root its `state_root` as
//! written, which must be absolute, and actingd `<root>\runtime\actingcommand-actingd.exe`;
//! `--state-root` may only repeat that state root, and acui.toml's path keys are not
//! taken. A root that lacks one of them, or a folder that is only half a root, is said
//! in red, and never stood in for by acui.toml: a console in a root does not reach
//! another installation by itself. Which acui.toml the run reads and writes goes by the
//! same place (`settings::load`).
//!
//! None of it is a silent default: with no state root the workbench says so in plain
//! words, and the technical log gets the usage. One place, so that what replaces this
//! derivation replaces it whole. So too for where the Runtime's `actingctl` is, and how
//! it is pointed at this run's configuration (`tool_paths`).

use std::fmt;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use acui_installation::{Snapshot, INSTALL_SELECTION_PATH, MAX_CONFIG_BYTES};
use serde_json::Value;

use crate::settings::Settings;
use crate::strings::{fill, Labels};
use crate::Args;

const ACTINGCTL: &str = "actingctl.exe";
const ACTINGD: &str = "actingcommand-actingd.exe";
/// The configuration of a root without an installation selection, as `actingctl` finds it.
pub const ROOT_CONFIG: &str = "actingd.config.json";
/// The folder of a root that holds the Runtime's programs, and the manifest in it that
/// marks a single-location root (Runtime contracts/install-cli.md, Layout).
const RUNTIME: &str = "runtime";
const BUILD_MANIFEST: &str = "BUILD-MANIFEST.json";
/// The folder of a root this program sits in.
const UI: &str = "ui";

/// What the launcher and the read face start from.
pub struct InstallPaths {
    /// The installation inputs of this process, retained across all UI actions.
    pub installation: Option<Snapshot>,
    /// The installation folder these come from: the selection's root, or a
    /// single-location root; `None` for `--state-root` and acui.toml. For the
    /// settings page and the log.
    pub root: Option<PathBuf>,
    pub state_root: PathBuf,
    pub actingd_config: Option<PathBuf>,
    pub actingd_exe: Option<PathBuf>,
    /// The tier they came from, for the log.
    pub origin: Origin,
}

/// The tier the paths came from.
pub enum Origin {
    Selected,
    Root(RootBy),
    /// `--state-root` when it was given, else acui.toml.
    Legacy { state_root_given: bool },
}

/// How a single-location root was found: where this program is, or `--root`.
#[derive(Clone, Copy)]
pub enum RootBy {
    Exe,
    Arg,
}

/// Why there are no paths to start from.
pub enum InstallIssue {
    /// This process's installation selection could not be read, as
    /// acui-installation words it.
    Selection(String),
    /// The same, for a program in an installation folder named `A` or `B`, which the
    /// selection reader takes for a slot.
    SelectionSlotNamed(String),
    /// `--state-root` names another root than the installation selected.
    StateRootDiffers { requested: PathBuf, selected: PathBuf },
    /// acui.toml's `state_root` is not absolute.
    StateRootNotAbsolute(PathBuf),
    /// Neither `--state-root` nor acui.toml names a state root.
    NoStateRoot,
    /// `--root` names another folder than the installation this console belongs to.
    RootDiffers { given: PathBuf, root: PathBuf },
    /// `--root` names no single-location root: not absolute, or without
    /// `runtime\BUILD-MANIFEST.json`.
    RootArgNotRoot(PathBuf),
    /// The single-location root this console belongs to lacks what its paths come from.
    RootIncomplete { root: PathBuf, problems: Vec<RootProblem> },
    /// `--state-root` names another state root than the root's configuration.
    RootStateRootDiffers { root: PathBuf, requested: PathBuf, configured: PathBuf },
}

impl InstallIssue {
    /// What the workbench's red bar says of it. No state root at all is said in plain
    /// words; the command line and the keys that name one are the log's (`log_text`).
    pub fn text(&self, labels: &Labels) -> String {
        match self {
            Self::Selection(error) => fill(labels.install_selection_failed, &[error]),
            Self::SelectionSlotNamed(error) => format!(
                "{}\n{}",
                fill(labels.install_selection_failed, &[error]),
                labels.root_named_slot
            ),
            Self::StateRootDiffers { requested, selected } => fill(
                labels.state_root_differs,
                &[&requested.display().to_string(), &selected.display().to_string()],
            ),
            Self::StateRootNotAbsolute(root) => {
                fill(labels.state_root_not_absolute, &[&root.display().to_string()])
            }
            Self::NoStateRoot => labels.no_state_root.to_owned(),
            Self::RootDiffers { given, root } => fill(
                labels.root_differs,
                &[&given.display().to_string(), &root.display().to_string()],
            ),
            Self::RootArgNotRoot(given) => {
                fill(labels.root_arg_not_root, &[&given.display().to_string()])
            }
            Self::RootIncomplete { root, problems } => {
                let parts: Vec<String> = problems.iter().map(|problem| problem.text(labels)).collect();
                fill(
                    labels.root_incomplete,
                    &[&root.display().to_string(), &parts.join(labels.root_problem_separator)],
                )
            }
            Self::RootStateRootDiffers { root, requested, configured } => fill(
                labels.root_state_root_differs,
                &[
                    &root.display().to_string(),
                    &requested.display().to_string(),
                    &configured.display().to_string(),
                ],
            ),
        }
    }

    /// What the technical log gets: the same, and the usage too when no state root
    /// is named at all; for a root that lacks something, each path looked at and
    /// what the system said of it, as written.
    pub fn log_text(&self, labels: &Labels) -> String {
        match self {
            Self::NoStateRoot => format!("{}\n{}", labels.no_state_root, labels.usage),
            Self::RootIncomplete { problems, .. } => {
                let mut text = self.text(labels);
                for problem in problems {
                    text.push('\n');
                    text.push_str(&problem.detail());
                }
                text
            }
            Self::Selection(_)
            | Self::SelectionSlotNamed(_)
            | Self::StateRootDiffers { .. }
            | Self::StateRootNotAbsolute(_)
            | Self::RootDiffers { .. }
            | Self::RootArgNotRoot(_)
            | Self::RootStateRootDiffers { .. } => self.text(labels),
        }
    }
}

/// What a single-location root lacks, with the path it was looked for at and, but
/// for what was read and found wanting, what the system said.
pub enum RootProblem {
    /// `runtime\BUILD-MANIFEST.json`: without it the folder is only half a root.
    NoManifest { path: PathBuf, error: String },
    NoActingd { path: PathBuf, error: String },
    NoConfig { path: PathBuf, error: String },
    ConfigUnreadable { path: PathBuf, error: String },
    /// Larger than actingd reads it up to.
    ConfigTooLarge { path: PathBuf },
    ConfigNotJson { path: PathBuf, error: String },
    /// No string `state_root` (also when the configuration is not a JSON object).
    NoStateRoot { path: PathBuf },
    StateRootNotAbsolute { path: PathBuf, value: PathBuf },
}

impl RootProblem {
    /// One part of the red bar's list.
    fn text(&self, labels: &Labels) -> String {
        match self {
            Self::NoManifest { .. } => labels.root_no_manifest.to_owned(),
            Self::NoActingd { .. } => labels.root_no_actingd.to_owned(),
            Self::NoConfig { .. } => labels.root_no_config.to_owned(),
            Self::ConfigUnreadable { error, .. } => fill(labels.root_config_unreadable, &[error]),
            Self::ConfigTooLarge { .. } => labels.root_config_too_large.to_owned(),
            Self::ConfigNotJson { error, .. } => fill(labels.root_config_not_json, &[error]),
            Self::NoStateRoot { .. } => labels.root_no_state_root.to_owned(),
            Self::StateRootNotAbsolute { value, .. } => {
                fill(labels.root_state_root_not_absolute, &[&value.display().to_string()])
            }
        }
    }

    /// The path and what the system said of it: the log's.
    fn detail(&self) -> String {
        match self {
            Self::NoManifest { path, error }
            | Self::NoActingd { path, error }
            | Self::NoConfig { path, error }
            | Self::ConfigUnreadable { path, error }
            | Self::ConfigNotJson { path, error } => format!("{}: {error}", path.display()),
            Self::ConfigTooLarge { path } => {
                format!("{}: larger than {MAX_CONFIG_BYTES} bytes", path.display())
            }
            Self::NoStateRoot { path } => format!("{}: no string state_root", path.display()),
            Self::StateRootNotAbsolute { path, value } => {
                format!("{}: state_root {} is not absolute", path.display(), value.display())
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

/// Where this run's paths and its settings file come from.
pub struct Place {
    /// The tier, or why none gives paths.
    pub source: Result<Source, InstallIssue>,
    /// The single-location root whose own acui.toml this run may use: the one this
    /// program sits in, whether or not it is whole, else the one `--root` names.
    /// `None` with an installation selection, or one that could not be read, and
    /// outside any root: the per-user acui.toml, as ever.
    pub settings_root: Option<PathBuf>,
}

/// The tier the paths come from, before `--state-root` is weighed.
pub enum Source {
    Selected(Snapshot),
    Root(RootPaths),
    Legacy,
}

/// What a whole single-location root gives.
pub struct RootPaths {
    root: PathBuf,
    by: RootBy,
    config: PathBuf,
    state_root: PathBuf,
    actingd_exe: PathBuf,
}

/// The tier, read once: the installation selection first (`Snapshot::for_current_process`,
/// called here only), then the root `given` by `--root` or the one this program sits
/// in, else acui.toml. `given` is `None` when the command line could not be read: then
/// where this program is decides alone.
pub fn place(given: Option<&Path>) -> Place {
    let selected = match Snapshot::for_current_process() {
        Ok(selected) => selected,
        Err(error) => {
            let issue = if slot_named_root() {
                InstallIssue::SelectionSlotNamed(error)
            } else {
                InstallIssue::Selection(error)
            };
            return Place { source: Err(issue), settings_root: None };
        }
    };
    if let Some(snapshot) = selected {
        let source = match given {
            Some(given) if !same_place(given, &snapshot.root) => Err(InstallIssue::RootDiffers {
                given: given.to_path_buf(),
                root: snapshot.root.clone(),
            }),
            _ => Ok(Source::Selected(snapshot)),
        };
        return Place { source, settings_root: None };
    }
    let (root, by) = match (root_of_exe(), given) {
        (Some(own), Some(given)) if !same_place(given, &own) => {
            let source = Err(InstallIssue::RootDiffers { given: given.to_path_buf(), root: own.clone() });
            return Place { source, settings_root: Some(own) };
        }
        (Some(own), _) => (own, RootBy::Exe),
        (None, Some(given))
            if given.is_absolute() && given.join(RUNTIME).join(BUILD_MANIFEST).is_file() =>
        {
            (given.to_path_buf(), RootBy::Arg)
        }
        (None, Some(given)) => {
            let source = Err(InstallIssue::RootArgNotRoot(given.to_path_buf()));
            return Place { source, settings_root: None };
        }
        (None, None) => return Place { source: Ok(Source::Legacy), settings_root: None },
    };
    Place { source: root_paths(root.clone(), by), settings_root: Some(root) }
}

/// The folder this program belongs to by where it is: `<root>\ui\acui.exe` (the
/// folder's name in any case) of a root that holds `runtime\BUILD-MANIFEST.json`, or
/// of one that is only half a root, with `actingd.config.json` or `runtime\` and no
/// manifest, which is then said rather than passed over. Never an A/B root, one with
/// `install\active.json`: that is the selection's. Judged on the canonical path,
/// given as a person writes it (`plain`).
fn root_of_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().and_then(|exe| exe.canonicalize()).ok()?;
    let root = exe
        .parent()
        .filter(|folder| folder.file_name().is_some_and(|name| name.eq_ignore_ascii_case(UI)))?
        .parent()?;
    if root.join(INSTALL_SELECTION_PATH).is_file() {
        return None;
    }
    let runtime = root.join(RUNTIME);
    let belongs = runtime.join(BUILD_MANIFEST).is_file()
        || present(&root.join(ROOT_CONFIG))
        || runtime.is_dir();
    belongs.then(|| plain(root))
}

/// Whether this program sits in `ui\` of an installation folder named `A` or `B`, one
/// with `runtime\BUILD-MANIFEST.json` and no installation selection above it: the
/// selection reader takes such a folder for a slot (case as it compares it).
fn slot_named_root() -> bool {
    let Ok(exe) = std::env::current_exe().and_then(|exe| exe.canonicalize()) else {
        return false;
    };
    let Some(root) = exe
        .parent()
        .filter(|folder| folder.file_name().and_then(|name| name.to_str()) == Some(UI))
        .and_then(Path::parent)
    else {
        return false;
    };
    matches!(root.file_name().and_then(|name| name.to_str()), Some("A" | "B"))
        && root.join(RUNTIME).join(BUILD_MANIFEST).is_file()
        && root.parent().is_some_and(|above| !above.join(INSTALL_SELECTION_PATH).is_file())
}

/// What `root` gives, each lack said: its manifest, actingd, and the configuration's
/// absolute `state_root`, read whole up to the size actingd reads it to (as the
/// Runtime's `configured_state_root` does), and used as written.
fn root_paths(root: PathBuf, by: RootBy) -> Result<Source, InstallIssue> {
    let mut problems = Vec::new();
    let manifest = root.join(RUNTIME).join(BUILD_MANIFEST);
    if let Err(error) = file_there(&manifest) {
        problems.push(RootProblem::NoManifest { path: manifest, error });
    }
    let actingd_exe = root.join(RUNTIME).join(ACTINGD);
    if let Err(error) = file_there(&actingd_exe) {
        problems.push(RootProblem::NoActingd { path: actingd_exe.clone(), error });
    }
    let config = root.join(ROOT_CONFIG);
    match configured_state_root(&config) {
        Ok(state_root) if problems.is_empty() => {
            Ok(Source::Root(RootPaths { root, by, config, state_root, actingd_exe }))
        }
        Ok(_) => Err(InstallIssue::RootIncomplete { root, problems }),
        Err(problem) => {
            problems.push(problem);
            Err(InstallIssue::RootIncomplete { root, problems })
        }
    }
}

/// `Ok` when `path` is a file; else what the system said, or that it is no file.
fn file_there(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err("not a file".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Whether something is at `path`; a place that cannot be looked at counts as one.
fn present(path: &Path) -> bool {
    !matches!(path.try_exists(), Ok(false))
}

/// The absolute `state_root` of the configuration `config`.
fn configured_state_root(config: &Path) -> Result<PathBuf, RootProblem> {
    let path = config.to_path_buf();
    let file = match File::open(config) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(RootProblem::NoConfig { path, error: error.to_string() });
        }
        Err(error) => return Err(RootProblem::ConfigUnreadable { path, error: error.to_string() }),
    };
    let mut bytes = Vec::new();
    if let Err(error) = file.take(MAX_CONFIG_BYTES as u64 + 1).read_to_end(&mut bytes) {
        return Err(RootProblem::ConfigUnreadable { path, error: error.to_string() });
    }
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(RootProblem::ConfigTooLarge { path });
    }
    let document: Value = match serde_json::from_slice(&bytes) {
        Ok(document) => document,
        Err(error) => return Err(RootProblem::ConfigNotJson { path, error: error.to_string() }),
    };
    let Some(state_root) = document.get("state_root").and_then(Value::as_str) else {
        return Err(RootProblem::NoStateRoot { path });
    };
    let state_root = PathBuf::from(state_root);
    if !state_root.is_absolute() {
        return Err(RootProblem::StateRootNotAbsolute { path, value: state_root });
    }
    Ok(state_root)
}

/// A canonical Windows path without its `\\?\` (or `\\?\UNC\`) prefix, as a person
/// writes it and as the Runtime's tools are given it.
fn plain(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// Whether two paths name the same place: as written, else, both being there, as
/// the system resolves them.
pub fn same_place(left: &Path, right: &Path) -> bool {
    left == right
        || matches!(
            (left.canonicalize(), right.canonicalize()),
            (Ok(left), Ok(right)) if left == right
        )
}

/// The paths of the tier `place` found, with `--state-root` weighed against it.
pub fn resolve(args: &Args, source: Source, settings: &Settings) -> Result<InstallPaths, InstallIssue> {
    let installation = match source {
        Source::Root(found) => return in_root(args, found),
        Source::Selected(snapshot) => Some(snapshot),
        Source::Legacy => None,
    };
    let root = installation.as_ref().map(|snapshot| snapshot.root.clone());
    let origin = match &installation {
        Some(_) => Origin::Selected,
        None => Origin::Legacy { state_root_given: args.state_root.is_some() },
    };
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
        actingd_exe = Some(snapshot.slot_root().join("runtime").join(ACTINGD));
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
    Ok(InstallPaths { installation, root, state_root, actingd_config, actingd_exe, origin })
}

/// A whole single-location root's paths; `--state-root` may only repeat its state root.
fn in_root(args: &Args, found: RootPaths) -> Result<InstallPaths, InstallIssue> {
    let RootPaths { root, by, config, state_root, actingd_exe } = found;
    if let Some(requested) =
        args.state_root.as_ref().filter(|requested| !same_place(requested, &state_root))
    {
        return Err(InstallIssue::RootStateRootDiffers {
            root,
            requested: requested.clone(),
            configured: state_root,
        });
    }
    Ok(InstallPaths {
        installation: None,
        root: Some(root),
        state_root,
        actingd_config: Some(config),
        actingd_exe: Some(actingd_exe),
        origin: Origin::Root(by),
    })
}

impl InstallPaths {
    /// The technical log's one line on where this run's paths came from, and the
    /// settings file it reads and writes.
    pub fn log_line(&self, settings: Option<&Path>) -> String {
        let shown = |path: Option<&PathBuf>| {
            path.map_or_else(|| "none".to_owned(), |path| path.display().to_string())
        };
        let settings = settings
            .map_or_else(|| "none (no config directory)".to_owned(), |path| path.display().to_string());
        match &self.origin {
            Origin::Selected => {
                let slot = self
                    .installation
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), |snapshot| format!("{:?}", snapshot.selection.slot));
                format!(
                    "selection: root={} slot={slot} config={} state_root={} settings={settings}",
                    shown(self.root.as_ref()),
                    shown(self.actingd_config.as_ref()),
                    self.state_root.display(),
                )
            }
            Origin::Root(by) => {
                let by = match by {
                    RootBy::Exe => "exe",
                    RootBy::Arg => "--root",
                };
                // Not an error: a fresh root's state root is made when the Runtime first starts.
                let missing = if self.state_root.is_dir() { "" } else { " (no state root folder there yet)" };
                format!(
                    "root mode ({by}): root={} config={} state_root={}{missing} actingd={} settings={settings}",
                    shown(self.root.as_ref()),
                    shown(self.actingd_config.as_ref()),
                    self.state_root.display(),
                    shown(self.actingd_exe.as_ref()),
                )
            }
            Origin::Legacy { state_root_given } => {
                let from = if *state_root_given { "--state-root" } else { "acui.toml" };
                format!("legacy: state_root from {from}, paths from acui.toml {settings}")
            }
        }
    }
}
