// SPDX-License-Identifier: GPL-3.0-only
//! The install step on a root that already holds an installation: the upgrade. The new
//! Runtime first checks the configuration as it is. A Runtime that answers is
//! asked to shut down and awaited with the new `actingctl` — one started from
//! the console holds `ui\` as its working directory — then the console's, the
//! tools' and the Runtime's directories move aside. The verified payload is
//! laid out in their place and a Runtime that was running starts again on it.
//! Until the new version is laid out, any failure puts every moved directory
//! back, and a Runtime stopped for it starts again on the version still in
//! place. State, the configuration, the console's settings and the downloads
//! stay as they are. The version replaced is kept whole in `previous\`, one
//! version deep: the Runtime ships no state migration and no rollback of its
//! own, so going back stays a person's choice.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::install::{self, LaidOut};
use crate::runtime::{
    check_config, request_shutdown, restart, runtime_answers, state_root, ACTINGCTL, ACTINGD,
};
use crate::verify::{Report, Step, Verified, MANIFEST};

const UNCONFIRMED_NOTE: &str = "Runtime 的关闭未确认，它可能仍在运行或正在退出，未重新拉起：稍后在监控台确认 / The Runtime's shutdown was not confirmed; it may still run or be exiting, and was not started again: check in the console later";
const STOPPED_NOTE: &str = "Runtime 已被请求关闭，可能已停止，未重新拉起：请在监控台点「启动」 / The Runtime was asked to shut down and may have stopped; it was not started again: press Start in the console";

/// What is installed: the commits the two manifests name.
pub struct Installed {
    pub runtime_sha: String,
    pub ui_sha: String,
}

#[derive(Deserialize)]
struct Manifest {
    commit_sha: String,
}

/// `Some` when `root\runtime\BUILD-MANIFEST.json` exists; a manifest that is
/// there but unreadable is an error, never "not installed". So is a payload
/// without the configuration a finished install writes: an install that
/// stopped before it was configured, which an upgrade could not keep.
pub fn installed(root: &Path) -> Result<Option<Installed>, String> {
    let runtime = root.join("runtime").join(MANIFEST);
    if !runtime.is_file() {
        return Ok(None);
    }
    if !root.join("actingd.config.json").is_file() {
        return Err(format!(
            "此处有一份未完成的安装（有程序文件，没有 actingd.config.json）：换一个空的安装位置，或清空 {} 后重新安装 / An unfinished install is here (program files, no actingd.config.json): choose an empty location, or empty {} and install again",
            root.display(),
            root.display()
        ));
    }
    let commit = |path: PathBuf| {
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
        serde_json::from_str::<Manifest>(&text)
            .map(|manifest| manifest.commit_sha)
            .map_err(|error| format!("清单无法解析 / manifest unreadable: {}: {error}", path.display()))
    };
    Ok(Some(Installed {
        runtime_sha: commit(runtime)?,
        ui_sha: commit(root.join("ui").join(MANIFEST))?,
    }))
}

pub struct Upgraded {
    pub laid_out: LaidOut,
    /// Where the version replaced is kept.
    pub previous: PathBuf,
    /// The log of the Runtime started again, when it was running before.
    pub restarted: Option<PathBuf>,
}

pub fn upgrade(root: &Path, verified: &Verified, report: Report<'_>) -> Result<Upgraded, String> {
    let config = root.join("actingd.config.json");
    let state_root = state_root(&config)?;
    report.step(Step::Phase("检查现有配置 / Checking the configuration", None))?;
    check_config(&verified.runtime.dir.join(ACTINGD), &config)
        .map_err(|reason| format!("{reason}\n新版本未装，未做任何改动 / the new version is not installed, nothing was changed"))?;
    report.line("新 Runtime 接受现有配置 / the new Runtime accepts the configuration as it is")?;

    let mut swap = Swap {
        root,
        previous: root.join("previous"),
        older: None,
        moved: Vec::new(),
        stopped: false,
        confirmed: false,
        made_previous: false,
        unanswered: None,
    };
    let laid_out = match swap.lay(verified, &state_root, report) {
        Ok(laid_out) => laid_out,
        Err(reason) => {
            let (mut reason, clean) = swap.undo(reason);
            if swap.stopped {
                reason.push('\n');
                if !swap.confirmed {
                    // The shutdown was not confirmed: a second Runtime would
                    // contend for the same state root.
                    reason.push_str(UNCONFIRMED_NOTE);
                } else if !clean {
                    // What is in `runtime\` may not be the version installed.
                    reason.push_str(STOPPED_NOTE);
                } else {
                    // Everything is back: the Runtime starts again on the
                    // version that stays installed.
                    let actingd = root.join("runtime").join(ACTINGD);
                    let phase = "用原版本重新拉起 Runtime / Starting the Runtime again on the version still installed";
                    // A log that fails here still leaves the reason as it is.
                    let restarted = report
                        .step(Step::Phase(phase, None))
                        .and_then(|()| restart(root, &actingd, &config, &state_root, report));
                    reason.push_str(&match restarted {
                        Ok(log) => format!(
                            "Runtime 已用原版本重新拉起 / the Runtime was started again on the version still installed; 日志 / log: {}",
                            log.display()
                        ),
                        Err(failed) => failed,
                    });
                }
            }
            return Err(reason);
        }
    };
    // The new version is in place: from here nothing is put back, since a
    // Runtime started on it may already have touched state.
    let laid = format!(
        "新版本已铺开；被替换的版本在 / The new version is laid out; the version replaced is in: {}",
        swap.previous.display()
    );
    let restarted = match swap.stopped {
        true => {
            report
                .step(Step::Phase("用新版本重新拉起 Runtime / Starting the Runtime again on the new version", None))
                .map_err(|reason| format!("{reason}\n{laid}"))?;
            Some(
                restart(root, &laid_out.actingd_exe, &config, &state_root, report)
                    .map_err(|reason| format!("{reason}\n{laid}"))?,
            )
        }
        false => None,
    };
    swap.drop_older(report).map_err(|reason| format!("{reason}\n{laid}"))?;
    Ok(Upgraded { laid_out, previous: swap.previous, restarted })
}

/// The directories an upgrade has moved, so a failure can put them back.
struct Swap<'a> {
    root: &'a Path,
    previous: PathBuf,
    /// The version kept from the upgrade before, moved aside until this one
    /// is laid out.
    older: Option<PathBuf>,
    moved: Vec<&'static str>,
    /// Whether the Runtime was asked to shut down, and whether its shutdown
    /// was confirmed.
    stopped: bool,
    confirmed: bool,
    /// Whether this upgrade made `previous\`, so a failure removes it.
    made_previous: bool,
    /// Why a `runtime-info.json` that is there was taken as no Runtime.
    unanswered: Option<String>,
}

impl Swap<'_> {
    fn lay(&mut self, verified: &Verified, state_root: &Path, report: Report<'_>) -> Result<LaidOut, String> {
        self.clear_leftovers(report)?;
        if self.previous.exists() {
            let older = self.root.join(format!("previous.older-{}", crate::log::unix_ms()));
            fs::rename(&self.previous, &older).map_err(|error| {
                format!("无法移开更早的旧版本 / cannot move {} aside: {error}", self.previous.display())
            })?;
            self.older = Some(older);
        }
        fs::create_dir(&self.previous)
            .map_err(|error| format!("无法创建 / cannot create {}: {error}", self.previous.display()))?;
        self.made_previous = true;
        // The Runtime first: one the console started works in `ui\`, which
        // cannot move while it runs.
        let ctl = verified.runtime.dir.join(ACTINGCTL);
        match runtime_answers(&ctl, state_root, report)? {
            Ok(()) => {
                self.stopped = true;
                report.step(Step::Phase("关闭 Runtime / Shutting the Runtime down", None))?;
                report.line("请求 Runtime 关闭并等待 / asking the Runtime to shut down, and waiting")?;
                request_shutdown(&ctl, state_root)?;
                self.confirmed = true;
                report.line("Runtime 已关闭 / the Runtime has shut down")?;
            }
            Err(unanswered) => self.unanswered = unanswered,
        }
        report.step(Step::Phase("移开旧版本 / Moving the old version aside", None))?;
        self.move_aside("ui", "请先关闭监控台 / close the console first", report)?;
        self.move_aside("tools", "多半有程序正从这里运行 / most likely a program runs from it", report)?;
        self.move_aside("runtime", "多半有程序正从这里运行 / most likely a program runs from it", report)?;
        install::lay_out(self.root, verified, report)
    }

    fn move_aside(&mut self, name: &'static str, hint: &str, report: Report<'_>) -> Result<(), String> {
        let from = self.root.join(name);
        if !from.exists() {
            return Ok(());
        }
        fs::rename(&from, self.previous.join(name)).map_err(|error| {
            let mut reason = format!("无法移开 / cannot move {}: {error}\n{hint}", from.display());
            if let Some(unanswered) = &self.unanswered {
                reason.push('\n');
                reason.push_str(unanswered);
            }
            reason
        })?;
        self.moved.push(name);
        report.line(&format!("已移开旧版本 / moved aside: {}", from.display()))
    }

    /// Puts everything moved back where it was, the version kept from before
    /// included, and says what could not be; `true` when everything went back.
    fn undo(&mut self, reason: String) -> (String, bool) {
        let first = reason.len();
        let mut reason = reason;
        for name in self.moved.iter().rev() {
            let placed = self.root.join(name);
            if placed.exists() {
                if let Err(error) = fs::remove_dir_all(&placed) {
                    reason.push_str(&format!(
                        "\n未能删除铺了一半的 / half-laid-out not removed: {}: {error}",
                        placed.display()
                    ));
                    continue;
                }
            }
            if let Err(error) = fs::rename(self.previous.join(name), &placed) {
                reason.push_str(&format!(
                    "\n未能放回 / not put back: {} → {}: {error}",
                    self.previous.join(name).display(),
                    placed.display()
                ));
            }
        }
        if self.made_previous {
            if let Err(error) = fs::remove_dir(&self.previous) {
                reason.push_str(&format!("\n未能删除 / not removed: {}: {error}", self.previous.display()));
            }
        }
        if let Some(older) = &self.older {
            if let Err(error) = fs::rename(older, &self.previous) {
                reason.push_str(&format!(
                    "\n更早的旧版本未能放回 / the older version was not put back: {} → {}: {error}",
                    older.display(),
                    self.previous.display()
                ));
            }
        }
        let clean = reason.len() == first;
        (reason, clean)
    }

    /// The version from the upgrade before, no longer needed once this one is
    /// laid out; one that cannot be removed is said, and removed next time.
    fn drop_older(&self, report: Report<'_>) -> Result<(), String> {
        match &self.older {
            Some(older) => match fs::remove_dir_all(older) {
                Ok(()) => report.line(&format!("已删除更早的旧版本 / older version removed: {}", older.display())),
                Err(error) => report.warn(&format!(
                    "更早的旧版本未能删除，下次升级再删 / older version not removed, the next upgrade retries: {}: {error}",
                    older.display()
                )),
            },
            None => Ok(()),
        }
    }

    /// `previous.older-*` directories an earlier upgrade could not remove.
    fn clear_leftovers(&self, report: Report<'_>) -> Result<(), String> {
        let entries = fs::read_dir(self.root)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", self.root.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let leftover = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("previous.older-"));
            if leftover && path.is_dir() {
                match fs::remove_dir_all(&path) {
                    Ok(()) => report.line(&format!("已删除残留的旧版本 / leftover removed: {}", path.display())),
                    Err(error) => report.warn(&format!(
                        "残留的旧版本仍未能删除 / leftover still not removed: {}: {error}",
                        path.display()
                    )),
                }?;
            }
        }
        Ok(())
    }
}
