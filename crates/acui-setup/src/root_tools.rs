// SPDX-License-Identifier: GPL-3.0-only
//! The installation root's own tools (Workflow #359): the diagnostic programs
//! and platform-tools live in `<root>\tools\`, outside both program slots, so a
//! slot switch never touches them. An installation or upgrade replaces only a
//! file whose content changed — an unchanged adb, which a running ADB server
//! may be using, is not even opened — and keeps every file it replaces or
//! retires; nothing is deleted. A slot holds only the OCR adapter
//! (`verify::SLOT_TOOL`).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::verify::{sha256_file, Report, Verified, MISMATCH, SLOT_TOOL};

/// What an installation does to `<root>\tools\`, by `/`-separated path.
pub struct Plan {
    source: PathBuf,
    pub unchanged: Vec<String>,
    pub replace: Vec<String>,
    pub add: Vec<String>,
    /// Installer files the root no longer holds: only the OCR adapter, which
    /// the old layout kept here.
    pub retire: Vec<String>,
    /// Files this release does not know: left exactly as they are.
    pub foreign: Vec<String>,
}

/// What `apply` changed, for its own undo before the selection commits.
pub struct Applied {
    tools: PathBuf,
    retained: PathBuf,
    steps: Vec<Change>,
}

enum Change {
    Retained(String),
    Placed(String),
}

fn relative(name: &str) -> PathBuf {
    name.split('/').collect()
}

#[cfg(windows)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn reparse(_metadata: &fs::Metadata) -> bool {
    false
}

/// Whether a regular file is there; anything else at that path stops the plan.
fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
        Ok(metadata)
            if metadata.is_file() && !metadata.file_type().is_symlink() && !reparse(&metadata) =>
        {
            Ok(true)
        }
        Ok(_) => Err(format!(
            "根工具不是普通文件，请移开后重试 / A root tool is not a regular file; move it aside and run again: {}",
            path.display()
        )),
    }
}

fn hash(path: &Path) -> Result<String, String> {
    sha256_file(path).map_err(|error| format!("Cannot hash {}: {error}", path.display()))
}

/// The root's tools against the verified tools zip: every bound file but the
/// OCR adapter belongs under `<root>\tools\`.
pub fn plan(root: &Path, verified: &Verified) -> Result<Plan, String> {
    let tools = root.join("tools");
    match fs::symlink_metadata(&tools) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot inspect {}: {error}", tools.display())),
        Ok(metadata)
            if metadata.is_dir() && !metadata.file_type().is_symlink() && !reparse(&metadata) => {}
        Ok(_) => {
            return Err(format!(
                "安装根的 tools 不是普通文件夹，请移开后重试 / The installation root's tools is not a plain folder; move it aside and run again: {}",
                tools.display()
            ))
        }
    }
    let mut plan = Plan {
        source: verified.tools.dir.clone(),
        unchanged: Vec::new(),
        replace: Vec::new(),
        add: Vec::new(),
        retire: Vec::new(),
        foreign: Vec::new(),
    };
    let wanted: BTreeSet<&str> = verified
        .tools
        .files
        .iter()
        .map(String::as_str)
        .filter(|name| *name != SLOT_TOOL)
        .collect();
    for name in &wanted {
        let installed = tools.join(relative(name));
        if !regular(&installed)? {
            plan.add.push(name.to_string());
        } else if hash(&plan.source.join(relative(name)))? == hash(&installed)? {
            plan.unchanged.push(name.to_string());
        } else {
            plan.replace.push(name.to_string());
        }
    }
    if regular(&tools.join(SLOT_TOOL))? {
        plan.retire.push(SLOT_TOOL.to_string());
    }
    let mut present = Vec::new();
    if tools.is_dir() {
        walk(&tools, &tools, 0, &mut present)?;
    }
    plan.foreign = present
        .into_iter()
        .filter(|name| !wanted.contains(name.as_str()) && name.as_str() != SLOT_TOOL)
        .collect();
    Ok(plan)
}

fn walk(base: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) -> Result<(), String> {
    if depth > 8 {
        return Err(format!("Root tools are nested too deep: {}", dir.display()));
    }
    let unreadable = |error: std::io::Error| format!("Cannot read {}: {error}", dir.display());
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let path = entry.map_err(unreadable)?.path();
        if path.is_dir() {
            walk(base, &path, depth + 1, out)?;
            continue;
        }
        let relative = path
            .strip_prefix(base)
            .map_err(|_| format!("Path outside the root tools: {}", path.display()))?;
        out.push(
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    Ok(())
}

impl Plan {
    /// One line per kind of change, each naming its files.
    pub fn describe(&self, report: Report<'_>) -> Result<(), String> {
        for (label, names) in [
            ("内容相同，不动 / unchanged, not touched", &self.unchanged),
            ("替换，旧文件保留 / replaced, previous kept", &self.replace),
            ("加入 / added", &self.add),
            (
                "移出安装根，保留 / retired from the root, kept",
                &self.retire,
            ),
            (
                "本发布件不认识，原样保留 / unknown to this release, left as is",
                &self.foreign,
            ),
        ] {
            if !names.is_empty() {
                report.line(&format!(
                    "根工具 / Root tools {label}: {}",
                    names.join(", ")
                ))?;
            }
        }
        Ok(())
    }

    /// Replaces, adds and retires as planned. The files that move are held
    /// against other users first (`slots::NativeOccupancy`); each replaced or
    /// retired file is kept under `retained`. On failure everything done here is
    /// put back before the error is returned.
    pub fn apply(
        &self,
        root: &Path,
        retained: &Path,
        report: Report<'_>,
    ) -> Result<Applied, String> {
        let tools = root.join("tools");
        self.describe(report)?;
        let moving: Vec<PathBuf> = self
            .replace
            .iter()
            .chain(&self.retire)
            .map(|name| tools.join(relative(name)))
            .collect();
        let occupancy = crate::slots::NativeOccupancy::files(&moving)?;
        let mut applied = Applied {
            tools,
            retained: retained.to_path_buf(),
            steps: Vec::new(),
        };
        let result = (|| -> Result<(), String> {
            for name in &self.retire {
                applied.retain(name)?;
            }
            for name in &self.replace {
                applied.retain(name)?;
                applied.place(name, &self.source)?;
            }
            for name in &self.add {
                applied.place(name, &self.source)?;
            }
            Ok(())
        })();
        drop(occupancy);
        if let Err(reason) = result {
            return Err(match applied.undo() {
                Ok(()) => format!("{reason}; 根工具已复原 / root tools restored"),
                Err(undo) => {
                    format!(
                        "{reason}; 根工具复原未完成 / root tools restoration incomplete: {undo}"
                    )
                }
            });
        }
        if !self.replace.is_empty() || !self.retire.is_empty() {
            report.line(&format!(
                "根工具的旧文件保留于 / Previous root tool files kept at {}",
                retained.display()
            ))?;
        }
        Ok(applied)
    }
}

impl Applied {
    fn retain(&mut self, name: &str) -> Result<(), String> {
        let from = self.tools.join(relative(name));
        let to = self.retained.join(relative(name));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
        }
        if to
            .try_exists()
            .map_err(|error| format!("Cannot inspect {}: {error}", to.display()))?
        {
            return Err(format!("Kept root tool already exists: {}", to.display()));
        }
        fs::rename(&from, &to).map_err(|error| {
            format!(
                "Cannot keep root tool {} at {}: {error}",
                from.display(),
                to.display()
            )
        })?;
        self.steps.push(Change::Retained(name.to_string()));
        Ok(())
    }

    fn place(&mut self, name: &str, source: &Path) -> Result<(), String> {
        let from = source.join(relative(name));
        let to = self.tools.join(relative(name));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
        }
        let mut part_name = to
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        part_name.push(".acsetup-new");
        let part = to.with_file_name(part_name);
        if part
            .try_exists()
            .map_err(|error| format!("Cannot inspect {}: {error}", part.display()))?
        {
            return Err(format!(
                "Unfinished root tool copy is in the way: {}",
                part.display()
            ));
        }
        fs::copy(&from, &part).map_err(|error| {
            format!(
                "复制失败 / copy failed: {} → {}: {error}",
                from.display(),
                part.display()
            )
        })?;
        let (expected, actual) = (hash(&from)?, hash(&part)?);
        if expected != actual {
            return Err(format!(
                "{MISMATCH}: {} sha256 应为 / expected {expected}，实为 / actual {actual}",
                part.display()
            ));
        }
        fs::rename(&part, &to)
            .map_err(|error| format!("Cannot place root tool {}: {error}", to.display()))?;
        self.steps.push(Change::Placed(name.to_string()));
        Ok(())
    }

    /// Puts back what `apply` did, newest first: a placed file moves aside into
    /// `<retained>\.undone\`, a kept file returns. Nothing is deleted.
    pub fn undo(&self) -> Result<(), String> {
        let mut failures = Vec::new();
        for step in self.steps.iter().rev() {
            let (from, to) = match step {
                Change::Placed(name) => (
                    self.tools.join(relative(name)),
                    self.retained.join(".undone").join(relative(name)),
                ),
                Change::Retained(name) => (
                    self.retained.join(relative(name)),
                    self.tools.join(relative(name)),
                ),
            };
            let moved = to
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::rename(&from, &to));
            if let Err(error) = moved {
                failures.push(format!("{} → {}: {error}", from.display(), to.display()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}
