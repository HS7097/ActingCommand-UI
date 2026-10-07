// SPDX-License-Identifier: GPL-3.0-only
//! Program materialization under the shared slot protocol and native occupancy.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use actingcommand_contract::installation::{InstallSelection, InstallSlot, InstallSlotLock};

use crate::generations::Writer;
use crate::install::LaidOut;
use crate::verify::{Report, Verified};

const MAX_SLOT_FILES: usize = 4096;
const MAX_SLOT_DEPTH: usize = 32;

/// Permanent empty installation locators. Existing files are never truncated.
pub fn initialize(writer: &Writer) -> Result<(), String> {
    for slot in [InstallSlot::A, InstallSlot::B] {
        let path = writer
            .root()
            .join(format!("install/slot-{}.lock", slot.as_str()));
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                regular(&path, &metadata)?;
                if metadata.len() != 0 {
                    return Err(format!("Slot locator is not empty: {}", path.display()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|error| {
                        format!("Cannot create slot locator {}: {error}", path.display())
                    })?;
                file.sync_all().map_err(|error| {
                    format!("Cannot seal slot locator {}: {error}", path.display())
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "Cannot inspect slot locator {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

/// The writer is already held. Downloads may precede this call; occupied slots
/// stop here without moving their programs or ending an ADB/shared process.
pub fn materialize(
    writer: &Writer,
    slot: InstallSlot,
    verified: &Verified,
    report: Report<'_>,
) -> Result<LaidOut, String> {
    let root = writer.root();
    let active = root.join(acui_installation::INSTALL_SELECTION_PATH);
    if active
        .try_exists()
        .map_err(|error| format!("Cannot inspect active selection: {error}"))?
    {
        let bytes = acui_installation::read_bounded(
            &active,
            acui_installation::MAX_INSTALL_SELECTION_BYTES,
        )?;
        let selection = InstallSelection::from_json(&bytes).map_err(|error| error.to_string())?;
        if selection.slot == slot {
            return Err("Materialization cannot replace the selected program slot".into());
        }
    }
    let exclusive =
        InstallSlotLock::try_exclusive(root, slot).map_err(|error| error.to_string())?;
    let result = (|| {
        let target = root.join(slot.as_str());
        let occupancy = NativeOccupancy::acquire(&target)?;
        let candidate = root.join(format!(
            "install/programs-{}-{}",
            slot.as_str(),
            crate::log::unix_ms()
        ));
        crate::install::prepare_programs(&candidate, verified, report)?;
        // NTFS refuses to rename a directory while any file beneath it is open,
        // whatever the share mode. The handles carried the Restart Manager answer;
        // they are released just before the move, and a user that appears in
        // between makes the rename fail as occupied, with nothing moved.
        drop(occupancy);
        let retained = if target
            .try_exists()
            .map_err(|error| format!("Cannot inspect target slot: {error}"))?
        {
            let retained = root.join(format!(
                "install/retained-{}-{}",
                slot.as_str(),
                crate::log::unix_ms()
            ));
            if retained.try_exists().map_err(|error| error.to_string())? {
                return Err("Retained slot destination already exists".into());
            }
            rename_unoccupied(&target, &retained).map_err(|error| {
                format!(
                    "Cannot retain previous spare slot: {error}; candidate retained at {}",
                    candidate.display()
                )
            })?;
            Some(retained)
        } else {
            None
        };
        if let Err(error) = fs::rename(&candidate, &target) {
            let reason = format!(
                "Cannot materialize program slot: {error}; candidate retained at {}",
                candidate.display()
            );
            return Err(match retained {
                Some(retained) => match fs::rename(&retained, &target) {
                    Ok(()) => format!("{reason}; previous spare slot restored"),
                    Err(restore) => format!(
                        "{reason}; previous spare slot remains at {}: {restore}",
                        retained.display()
                    ),
                },
                None => reason,
            });
        }
        crate::verify::prepared_programs(&target, verified, report)?;
        if let Some(retained) = retained {
            report.line(&format!(
                "旧备用程序已保留 / Previous spare programs retained: {}",
                retained.display()
            ))?;
        }
        Ok(LaidOut {
            ui_dir: target.join("ui"),
            // The tools are the root's own (Workflow #359).
            tools_dir: root.join("tools"),
            actingd_exe: target.join("runtime").join(crate::runtime::ACTINGD),
            acui_exe: target.join("ui/acui.exe"),
        })
    })();
    match (
        result,
        exclusive.release().map_err(|error| error.to_string()),
    ) {
        (Ok(laid_out), Ok(())) => Ok(laid_out),
        (Err(reason), Ok(())) | (Ok(_), Err(reason)) => Err(reason),
        (Err(reason), Err(release)) => Err(format!("{reason}; {release}")),
    }
}

/// These handles exclude native consumers which do not implement slot locks.
/// They allow the owning transaction to rename the intact directory, but deny
/// new data opens until materialization finishes. Mapped resources are also
/// checked through the native Restart Manager resource list.
pub struct NativeOccupancy {
    _files: Vec<File>,
}

impl NativeOccupancy {
    pub fn acquire(directory: &Path) -> Result<Self, String> {
        let mut paths = Vec::new();
        if directory
            .try_exists()
            .map_err(|error| format!("Cannot inspect native slot occupancy: {error}"))?
        {
            collect(directory, 0, &mut 0, &mut paths)?;
        }
        Self::files(&paths)
    }

    pub fn files(paths: &[PathBuf]) -> Result<Self, String> {
        let mut files = Vec::new();
        for path in paths {
            let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            regular(path, &metadata)?;
            files.push(open_native(path)?);
        }
        crate::platform::slot_files_unused(paths, &[])?;
        Ok(Self { _files: files })
    }
}

/// Restart Manager alone, no handles: whether anything but this process and
/// `allowed` (the Runtime owner the transaction is about to close) uses these
/// directories' files or these files. Run before the Runtime is closed, so a
/// console, an MCP client or a tool still running stops the run with the
/// Runtime untouched.
pub fn precheck(
    directories: &[PathBuf],
    files: &[PathBuf],
    allowed: &[u32],
) -> Result<(), String> {
    let mut paths = files.to_vec();
    for directory in directories {
        if directory
            .try_exists()
            .map_err(|error| format!("Cannot inspect {}: {error}", directory.display()))?
        {
            collect(directory, 0, &mut 0, &mut paths)?;
        }
    }
    crate::platform::slot_files_unused(&paths, allowed).map_err(|error| {
        format!("{error}; 请先关闭它们再运行，Runtime 未被触动 / close them and run again; the Runtime was not touched")
    })
}

/// A directory (or file) moved in one rename. Access denied or a sharing
/// violation means something holds a file beneath it: reported as occupied.
pub fn rename_unoccupied(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|error| match error.raw_os_error() {
        Some(5 | 32) => format!(
            "Program slot is occupied or unavailable; materialization blocked at {}: {error}",
            from.display()
        ),
        _ => format!("Cannot move {} to {}: {error}", from.display(), to.display()),
    })
}

#[cfg(windows)]
fn open_native(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0x0000_0004) // FILE_SHARE_DELETE, for the owning rename.
        .open(path)
        .map_err(|error| {
            format!(
                "Program slot is occupied or unavailable; materialization blocked at {}: {error}",
                path.display()
            )
        })
}

#[cfg(not(windows))]
fn open_native(_path: &Path) -> Result<File, String> {
    Err("Native installation slot occupancy is Windows-only".into())
}

fn collect(
    directory: &Path,
    depth: usize,
    entries: &mut usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("Cannot inspect {}: {error}", directory.display()))?;
    no_link(directory, &metadata)?;
    if !metadata.is_dir() || depth > MAX_SLOT_DEPTH {
        return Err(format!(
            "Program directory is invalid or too deep: {}",
            directory.display()
        ));
    }
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        if *entries == MAX_SLOT_FILES {
            return Err("Program slot exceeds 4096 native occupancy entries".into());
        }
        *entries += 1;
        let path = entry.map_err(|error| error.to_string())?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        no_link(&path, &metadata)?;
        if metadata.is_dir() {
            collect(&path, depth + 1, entries, files)?;
        } else {
            regular(&path, &metadata)?;
            files.push(path);
        }
    }
    Ok(())
}

fn regular(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    no_link(path, metadata)?;
    if !metadata.is_file() {
        return Err(format!(
            "Installation material is not a regular file: {}",
            path.display()
        ));
    }
    Ok(())
}
fn no_link(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(format!(
                "Installation material is a reparse point: {}",
                path.display()
            ));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Installation material is a link: {}",
            path.display()
        ));
    }
    Ok(())
}
