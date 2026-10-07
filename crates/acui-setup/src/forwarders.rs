// SPDX-License-Identifier: GPL-3.0-only
//! The installation root's fixed entries — `runtime\actingcommand-actingd.exe`,
//! `runtime\actingctl.exe` and `ui\acui.exe`, each a copy of acforward
//! (`install::stable_entries`) — read `install/active.json` like the programs they forward to.
//! An A/B upgrade therefore replaces every one whose bytes differ from the release's acforward
//! (Workflow #364, review M5), so the entries read what the release's UI reads.
//!
//! An entry in place is renamed into `install\entries-<generation>\` and kept: an entry that
//! runs (a console, an MCP server) can be renamed and keeps running from there. The new bytes
//! come in through `<name>.acsetup-new`, checked against the source's sha256, and are renamed
//! into place. Nothing is deleted but this run's own unfinished copy.

use std::fs;
use std::path::Path;

use crate::verify::{sha256_file, Report, MISMATCH};

/// The fixed entries `install::stable_entries` writes, by component directory.
const ENTRIES: [(&str, &str); 3] = [
    ("runtime", "actingcommand-actingd.exe"),
    ("runtime", "actingctl.exe"),
    ("ui", "acui.exe"),
];

fn hash(path: &Path) -> Result<String, String> {
    sha256_file(path).map_err(|error| format!("Cannot hash {}: {error}", path.display()))
}

/// Each fixed entry whose bytes differ from `acforward` (the new slot's own, verified with it)
/// replaced by it; the previous entry kept under `install\entries-<generation>\`. Runs once the
/// upgrade is committed; a failure leaves that entry as it was and says so.
pub fn refresh(root: &Path, acforward: &Path, generation: u64, report: Report<'_>) -> Result<(), String> {
    let wanted = hash(acforward)?;
    let kept_root = root.join(format!("install/entries-{generation}"));
    let mut refreshed = 0;
    for (component, name) in ENTRIES {
        let entry = root.join(component).join(name);
        if hash(&entry)? == wanted {
            continue;
        }
        let part = entry.with_file_name(format!("{name}.acsetup-new"));
        if part
            .try_exists()
            .map_err(|error| format!("Cannot inspect {}: {error}", part.display()))?
        {
            return Err(format!(
                "上次未完成的固定入口副本挡住了 / An unfinished fixed entry copy is in the way: {}",
                part.display()
            ));
        }
        fs::copy(acforward, &part).map_err(|error| {
            format!(
                "复制失败 / copy failed: {} → {}: {error}",
                acforward.display(),
                part.display()
            )
        })?;
        let copied = hash(&part)?;
        if copied != wanted {
            return Err(crate::fetch::discard(
                &part,
                format!(
                    "{MISMATCH}: {} sha256 应为 / expected {wanted}，实为 / actual {copied}",
                    part.display()
                ),
            ));
        }
        let kept = kept_root.join(component).join(name);
        let ready = (|| {
            if let Some(parent) = kept.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
            }
            if kept
                .try_exists()
                .map_err(|error| format!("Cannot inspect {}: {error}", kept.display()))?
            {
                return Err(format!("Kept fixed entry already exists: {}", kept.display()));
            }
            fs::rename(&entry, &kept).map_err(|error| {
                format!(
                    "固定入口无法移开 / cannot move the fixed entry {} aside to {}: {error}",
                    entry.display(),
                    kept.display()
                )
            })
        })();
        if let Err(reason) = ready {
            return Err(crate::fetch::discard(&part, reason));
        }
        if let Err(error) = fs::rename(&part, &entry) {
            let restored = match fs::rename(&kept, &entry) {
                Ok(()) => "原入口已复原 / the previous entry is back in place".to_string(),
                Err(restore) => format!(
                    "原入口未能复原，保留于 / the previous entry could not be put back and is kept at {}: {restore}",
                    kept.display()
                ),
            };
            return Err(crate::fetch::discard(
                &part,
                format!(
                    "固定入口无法放入 / cannot place the fixed entry {}: {error}; {restored}",
                    entry.display()
                ),
            ));
        }
        report.line(&format!(
            "固定入口已换成本发布件的 acforward / Fixed entry refreshed to this release's acforward: {}；原文件保留于 / previous kept at {}",
            entry.display(),
            kept.display()
        ))?;
        refreshed += 1;
    }
    if refreshed == 0 {
        report.line("固定入口已是本发布件的 acforward / The fixed entries are already this release's acforward")?;
    }
    Ok(())
}
