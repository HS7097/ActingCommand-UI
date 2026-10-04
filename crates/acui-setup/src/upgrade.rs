// SPDX-License-Identifier: GPL-3.0-only
//! The install step on a root that already holds an installation: the upgrade.
//! Setup first forms the resource/configuration plan. A Runtime that answers is
//! asked to shut down and awaited with the new `actingctl` — one started from
//! the console holds `ui\` as its working directory — then the console's, the
//! tools' and the Runtime's directories move aside. A `ui\` that cannot move
//! because some other process works in it — most likely an adb server that
//! Runtime started — has its entries moved aside one by one instead, once
//! neither the console nor the Runtime is found running; the ADB server
//! itself is never looked at before or during the upgrade. The verified
//! payload and verified resources are laid out in their place. The maintenance
//! plan preserves existing instance identity and resolves binding conflicts in
//! the wizard. Its checked configuration is committed before the ADB check and
//! first new Runtime start. A failure before that start restores configuration
//! and programs; the old Runtime restarts only after complete restoration.
//! State, the console's settings and downloads stay. The version replaced is kept in `previous\`, one
//! version deep: the Runtime ships no state migration and no rollback of its
//! own, so going back stays a person's choice.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::adb_server::{self, AdbServer};
use crate::install::{self, LaidOut};
use crate::{bundle::Bundle, maintenance};
use crate::runtime::{
    request_shutdown, restart, runtime_answers, ACTINGCTL, ACTINGD,
};
use crate::verify::{Report, Step, Verified, MANIFEST, PLATFORM_TOOLS};

const UNCONFIRMED_NOTE: &str = "Runtime 的关闭未确认，它可能仍在运行或正在退出，未重新拉起：稍后在监控台确认 / The Runtime's shutdown was not confirmed; it may still run or be exiting, and was not started again: check in the console later";
const STOPPED_NOTE: &str = "Runtime 已被请求关闭，可能已停止，未重新拉起：请在监控台点「启动」 / The Runtime was asked to shut down and may have stopped; it was not started again: press Start in the console";
const STILL_RUNS: &str = "Runtime 仍在运行，未能确认它已关闭：请先结束它再升级 / The Runtime still runs and its shutdown could not be confirmed: end it first, then upgrade";
/// Said with an older version that could not be removed.
const ADB_HELD: &str = "其中若有 adb.exe / AdbWinApi.dll，多半是仍在运行的 adb 服务占用（它可能也在为 ALAS/MAA 服务）；服务停止后，下次升级会删掉 / If adb.exe or AdbWinApi.dll is among them, most likely an adb server still running holds it (it may also serve ALAS/MAA); once it stops, the next upgrade removes it";
/// Windows' ERROR_SHARING_VIOLATION: a directory some process works in, or
/// an exe that runs, opened for writing.
const SHARING_VIOLATION: i32 = 32;

/// What is installed: the commits the two manifests name, and whether AC's
/// own adb is in place.
#[derive(Clone)]
pub struct Installed {
    pub runtime_sha: String,
    pub ui_sha: String,
    /// Whether `tools\platform-tools\adb.exe` is there: without it, even the
    /// very release installed is laid out again (Workflow #337 §4.3).
    pub adb: bool,
}

impl Installed {
    /// Whether installing the release whose two commits are `wanted` would
    /// change nothing: the same two commits, and AC's adb in place.
    pub fn current(&self, wanted: &(String, String)) -> bool {
        self.adb && self.runtime_sha == wanted.0 && self.ui_sha == wanted.1
    }
}

/// AC's own adb under `root`: `tools\platform-tools\adb.exe`.
pub fn ac_adb(root: &Path) -> PathBuf {
    root.join("tools").join(PLATFORM_TOOLS).join("adb.exe")
}

#[derive(Deserialize)]
struct Manifest {
    commit_sha: String,
}

/// `Some` when `root\runtime\BUILD-MANIFEST.json` and the configuration are
/// both there; a manifest that is there but unreadable is an error, never "not
/// installed". So is either one without the other: the configuration is the
/// last thing a finished install writes, and an upgrade moves the program
/// files aside before laying the new ones out — neither half is installed
/// over, and neither is upgraded.
pub fn installed(root: &Path) -> Result<Option<Installed>, String> {
    let runtime = root.join("runtime").join(MANIFEST);
    let config = root.join("actingd.config.json");
    match (runtime.is_file(), config.is_file()) {
        (false, false) => return Ok(None),
        (false, true) => {
            return Err(format!(
                "此处有 {} 却没有程序文件（多半是一次中断的升级，被替换的版本在 previous\\）：向导不在这里新装，以免覆盖配置 / A configuration is here but no program files (most likely an interrupted upgrade; the version replaced is in previous\\): the wizard does not install over it",
                config.display()
            ))
        }
        (true, false) if has_content(&root.join("state")) || root.join("previous").exists() => {
            return Err(format!(
                "此处有程序文件和状态，但没有 {}：向导只升级配置在此处的安装，请换一个安装位置 / Program files and state are here but no {}: the wizard only upgrades an installation configured here; choose another location",
                config.display(),
                config.display()
            ))
        }
        (true, false) => {
            return Err(format!(
                "此处有一份未完成的安装（有程序文件，没有配置）：删除 {}、{}、{} 后重新安装 / An unfinished install is here (program files, no configuration): remove {}, {} and {} and install again",
                root.join("runtime").display(),
                root.join("ui").display(),
                root.join("tools").display(),
                root.join("runtime").display(),
                root.join("ui").display(),
                root.join("tools").display()
            ))
        }
        (true, true) => {}
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
        adb: ac_adb(root).is_file(),
    }))
}

fn has_content(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// The `MEMBERS.json` of the release last installed or upgraded to at a root,
/// kept for telling an older release from a newer one.
pub const INSTALLED_MEMBERS: &str = "installed-members.json";

/// A `MEMBERS.json`'s `published_at_utc` when it is `YYYY-MM-DDTHH:MM:SSZ`,
/// which sorts as text in time order.
fn published(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let at = value.get("published_at_utc")?.as_str()?;
    let shaped = at.len() == 20
        && at.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            10 => byte == b'T',
            13 | 16 => byte == b':',
            19 => byte == b'Z',
            _ => byte.is_ascii_digit(),
        });
    shaped.then(|| at.to_string())
}

/// Why installing the release whose `MEMBERS.json` is `wanted` over the
/// installation at `root` needs the person's word — it was published before
/// the one installed, or either time is unknown, or the record does not name
/// what the manifests say is installed (an upgrade that failed after laying
/// out, an older wizard, a hand-made rollback) — or `None` when it is as new
/// or newer. The note names the release to install, so a tick given for one
/// release is not taken for another. Publication time is a heuristic: it is
/// said as one.
pub fn downgrade(root: &Path, wanted: &str) -> Option<String> {
    let record = root.join(INSTALLED_MEMBERS);
    let have = match fs::read_to_string(&record) {
        Ok(text) => current_record(root, &text).and_then(|()| {
            published(&text).ok_or_else(|| format!("{} 里没有可比较的发布时间 / has no publication time to compare", record.display()))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "这份安装没有 {INSTALLED_MEMBERS} 记录 / this installation has no {INSTALLED_MEMBERS} record"
        )),
        Err(error) => Err(format!("读取失败 / read failed: {}: {error}", record.display())),
    };
    let want = published(wanted)
        .ok_or_else(|| "要装的 MEMBERS.json 没有可比较的发布时间 / the MEMBERS.json to install has no publication time to compare".to_string());
    let which = match crate::verify::members_of(wanted) {
        Ok((runtime, ui)) => format!("runtime {} · ui {}", &runtime[..8], &ui[..8]),
        Err(_) => "?".to_string(),
    };
    match (have, want) {
        (Ok(have), Ok(want)) if want >= have => None,
        (Ok(have), Ok(want)) => Some(format!(
            "按发布时间判断，看起来是降级：要装的（{which}）发布于 {want}，已装的发布于 {have}。Runtime 没有状态迁移，也没有回滚 / By publication time this looks like a downgrade: the release to install ({which}) was published {want}, the installed one {have}. The Runtime has no state migration and no rollback"
        )),
        (Err(reason), _) | (_, Err(reason)) => Some(format!(
            "无法判断新旧（要装的：{which}）：{reason}。Runtime 没有状态迁移，也没有回滚 / Cannot tell which is newer (to install: {which}). The Runtime has no state migration and no rollback"
        )),
    }
}

/// Whether the record at `root` names the two commits its manifests say are
/// installed.
fn current_record(root: &Path, text: &str) -> Result<(), String> {
    let stale = || {
        format!("{INSTALLED_MEMBERS} 记下的不是现在装着的版本 / {INSTALLED_MEMBERS} does not name the version installed now")
    };
    let recorded = crate::verify::members_of(text).map_err(|_| stale())?;
    match installed(root) {
        Ok(Some(now)) if recorded.0 == now.runtime_sha && recorded.1 == now.ui_sha => Ok(()),
        Ok(_) => Err(stale()),
        Err(reason) => Err(reason),
    }
}

/// The `MEMBERS.json` just installed from `download`, kept at the root as the
/// record `downgrade` reads next time.
pub fn record_members(root: &Path, download: &Path) -> Result<(), String> {
    let (from, to) = (download.join("MEMBERS.json"), root.join(INSTALLED_MEMBERS));
    fs::copy(&from, &to).map(|_| ()).map_err(|error| {
        format!(
            "未能记下所装发布件（下次升级将无法判断新旧）/ could not record the release installed (the next upgrade cannot tell which is newer): {} → {}: {error}",
            from.display(),
            to.display()
        )
    })
}

pub struct Upgraded {
    pub laid_out: LaidOut,
    /// Where the version replaced is kept.
    pub previous: PathBuf,
    /// The log of the Runtime started again, when it was running before.
    pub restarted: Option<PathBuf>,
    /// How the ADB server stands after the check.
    pub adb_server: AdbServer,
    pub configuration_changed: bool,
}

pub fn upgrade(root: &Path, verified: &Verified, bundles: &[Bundle], choose: maintenance::Choose<'_>, report: Report<'_>) -> Result<Upgraded, String> {
    let config = root.join("actingd.config.json");
    let mut transaction = maintenance::Transaction::read(root)?;
    let state_root = transaction.state_root()?;
    let prepared = maintenance::prepare(bundles, &verified.staging.join("resource-packages"), report)?;
    let selected = maintenance::upgrade_selections(&mut transaction.document, root, &prepared, choose, report)?;
    let qualify = maintenance::augment(&mut transaction.document, root, &prepared, &selected, choose)?;
    report.line("配置计划已形成，提交前检查全部实际绑定 / Configuration plan formed; all actual bindings are checked before commit")?;

    let mut swap = Swap {
        root,
        previous: root.join("previous"),
        older: None,
        moved: Vec::new(),
        created: Vec::new(),
        stopped: false,
        confirmed: false,
        made_previous: false,
        unanswered: None,
        ui_entries: None,
        ui_emptied: false,
        ui_names: top_names(&verified.ui.files),
    };
    transaction.unchanged()?;
    let ready = swap.lay(verified, &state_root, report).and_then(|laid_out| {
        maintenance::place(&prepared, root, report)?;
        transaction.commit(&laid_out.actingd_exe, qualify, report)?;
        let adb_server = adb_server::ensure(&ac_adb(root), root, adb_server::PORT, report)?;
        // A failed phase notification still precedes the first new Runtime start.
        if swap.stopped {
            report.step(Step::Phase("用新版本重新拉起 Runtime / Starting the Runtime again on the new version", None))?;
        }
        Ok((laid_out, adb_server))
    });
    let (laid_out, adb_server) = match ready {
        Ok(ready) => ready,
        Err(reason) => {
            let restored = transaction.restore();
            let (mut reason, binaries_restored) = swap.undo(reason);
            let clean = binaries_restored && restored.is_ok();
            if let Err(error) = restored { reason.push_str(&format!("\n{error}")); }
            reason.push_str("\n已放置的资源保留 / Placed resources are retained");
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
    // From the first restart attempt onward, a new Runtime may have written
    // state even when readiness fails. Configuration and binaries then stay.
    let laid = format!(
        "新版本已铺开；被替换的版本在 / The new version is laid out; the version replaced is in: {}",
        swap.previous.display()
    );
    let restarted = match swap.stopped {
        true => {
            Some(
                restart(root, &laid_out.actingd_exe, &config, &state_root, report)
                    .map_err(|reason| format!("{reason}\n{laid}"))?,
            )
        }
        false => None,
    };
    // With no new start, finish at the reversible transaction boundary. An
    // older retained directory is handled by the next upgrade's usual cleanup.
    if swap.stopped {
        swap.drop_older(report).map_err(|reason| format!("{reason}\n{laid}"))?;
    }
    Ok(Upgraded { laid_out, previous: swap.previous, restarted, adb_server, configuration_changed: transaction.changed() })
}

/// The names `lay_out` puts at the top of `ui\`: each file's first segment,
/// and the manifest.
fn top_names(files: &[String]) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .filter_map(|file| file.split('/').next())
        .map(str::to_string)
        .chain(std::iter::once(MANIFEST.to_string()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The directories an upgrade has moved, so a failure can put them back.
struct Swap<'a> {
    root: &'a Path,
    previous: PathBuf,
    /// The version kept from the upgrade before, moved aside until this one
    /// is laid out.
    older: Option<PathBuf>,
    moved: Vec<&'static str>,
    /// Program directories absent before layout, removed on pre-start rollback.
    created: Vec<&'static str>,
    /// Whether the Runtime was asked to shut down, and whether its shutdown
    /// was confirmed.
    stopped: bool,
    confirmed: bool,
    /// Whether this upgrade made `previous\`, so a failure removes it.
    made_previous: bool,
    /// Why a `runtime-info.json` that is there was taken as no Runtime.
    unanswered: Option<String>,
    /// When `ui\` could not move as a whole: the names of its entries moved
    /// into `previous\ui\` one by one, in order, and whether every one of
    /// them was.
    ui_entries: Option<Vec<OsString>>,
    ui_emptied: bool,
    /// The names `lay_out` puts at the top of `ui\`: all an undo removes from
    /// a `ui\` that stayed in place.
    ui_names: Vec<String>,
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
            Err(Some(reason)) => return Err(format!("{UNCONFIRMED_NOTE}\n{reason}")),
            Err(None) => {}
        }
        report.step(Step::Phase("移开旧版本 / Moving the old version aside", None))?;
        self.move_aside("ui", "请先关闭监控台 / close the console first", report)?;
        self.move_aside("tools", "多半有程序正从这里运行 / most likely a program runs from it", report)?;
        self.move_aside("runtime", "多半有程序正从这里运行 / most likely a program runs from it", report)?;
        for name in ["ui", "tools", "runtime"] {
            if !self.moved.contains(&name) && !self.root.join(name).exists() { self.created.push(name); }
        }
        install::lay_out(self.root, verified, report)
    }

    fn move_aside(&mut self, name: &'static str, hint: &str, report: Report<'_>) -> Result<(), String> {
        let from = self.root.join(name);
        if !from.exists() {
            return Ok(());
        }
        match fs::rename(&from, self.previous.join(name)) {
            Ok(()) => {}
            Err(error) if name == "ui" && error.raw_os_error() == Some(SHARING_VIOLATION) => {
                return self.empty_ui(&from, &error, hint, report);
            }
            Err(error) => {
                return Err(self.said(format!("无法移开 / cannot move {}: {error}\n{hint}", from.display())));
            }
        }
        self.moved.push(name);
        report.line(&format!("已移开旧版本 / moved aside: {}", from.display()))
    }

    /// `reason`, with why a `runtime-info.json` was taken as no Runtime, when
    /// one was.
    fn said(&self, mut reason: String) -> String {
        if let Some(unanswered) = &self.unanswered {
            reason.push('\n');
            reason.push_str(unanswered);
        }
        reason
    }

    /// `ui\` some process works in, which keeps it from moving. A running exe
    /// cannot be opened for writing, so `ui\acui.exe` and
    /// `runtime\actingcommand-actingd.exe` are opened that way — nothing
    /// truncated, created or written — and closed at once: either one
    /// running stops the upgrade as before. With neither running, what works
    /// in `ui\` is some other process — most likely an adb server a Runtime
    /// started from the console — and the entries of `ui\` move into
    /// `previous\ui\` one by one, each recorded as it goes; the directory
    /// itself stays, and the new version is laid out into it.
    fn empty_ui(&mut self, ui: &Path, error: &io::Error, hint: &str, report: Report<'_>) -> Result<(), String> {
        let cannot = format!("无法移开 / cannot move {}: {error}", ui.display());
        let console = ui.join("acui.exe");
        let runtime = self.root.join("runtime").join(ACTINGD);
        for (exe, running) in [(&console, hint), (&runtime, STILL_RUNS)] {
            match fs::OpenOptions::new().write(true).open(exe) {
                Ok(_) => {}
                Err(probe) if probe.raw_os_error() == Some(SHARING_VIOLATION) => {
                    return Err(self.said(format!("{cannot}\n{running}")));
                }
                Err(probe) => {
                    return Err(self.said(format!(
                        "{cannot}\n未能确认它是否在运行 / cannot tell whether it runs: {}: {probe}",
                        exe.display()
                    )));
                }
            }
        }
        // Recorded before anything moves, so an undo always looks.
        self.moved.push("ui");
        self.ui_entries = Some(Vec::new());
        let aside = self.previous.join("ui");
        fs::create_dir(&aside)
            .map_err(|error| self.said(format!("无法创建 / cannot create {}: {error}", aside.display())))?;
        let names = fs::read_dir(ui)
            .and_then(|entries| entries.map(|entry| entry.map(|entry| entry.file_name())).collect::<io::Result<Vec<_>>>())
            .map_err(|error| self.said(format!("读取目录失败 / read_dir failed: {}: {error}", ui.display())))?;
        for name in names {
            let from = ui.join(&name);
            if let Err(error) = fs::rename(&from, aside.join(&name)) {
                return Err(self.said(format!("无法移开 / cannot move {}: {error}\n{hint}", from.display())));
            }
            if let Some(moved) = self.ui_entries.as_mut() {
                moved.push(name);
            }
        }
        self.ui_emptied = true;
        report.line(&format!(
            "{} 被另一个进程用作工作目录（多半是 Runtime 拉起的 adb 服务）；监控台与 Runtime 均未运行，逐项移开，目录留在原处 / {} is another process's working directory (most likely an adb server a Runtime started); neither the console nor the Runtime runs, so its entries were moved aside one by one and the directory stays",
            ui.display(),
            ui.display()
        ))
    }

    /// A `ui\` that stayed in place, put back as it was: what this upgrade
    /// laid into it removed — only once every old entry had left, and only
    /// the names `lay_out` puts there — then each entry moved back, the last
    /// first, and the emptied `previous\ui\` removed. What does not go back
    /// is said in `reason`.
    fn put_ui_back(&self, ui: &Path, entries: &[OsString], reason: &mut String) {
        if self.ui_emptied {
            for name in &self.ui_names {
                let laid = ui.join(name);
                let removed = match fs::symlink_metadata(&laid) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => Err(error),
                    Ok(meta) if meta.is_dir() => fs::remove_dir_all(&laid),
                    Ok(_) => fs::remove_file(&laid),
                };
                if let Err(error) = removed {
                    reason.push_str(&format!(
                        "\n未能删除铺了一半的 / half-laid-out not removed: {}: {error}",
                        laid.display()
                    ));
                }
            }
        }
        let aside = self.previous.join("ui");
        for name in entries.iter().rev() {
            let (from, to) = (aside.join(name), ui.join(name));
            if let Err(error) = fs::rename(&from, &to) {
                reason.push_str(&format!("\n未能放回 / not put back: {} → {}: {error}", from.display(), to.display()));
            }
        }
        if aside.exists() {
            if let Err(error) = fs::remove_dir(&aside) {
                reason.push_str(&format!("\n未能删除 / not removed: {}: {error}", aside.display()));
            }
        }
    }

    /// Puts everything moved back where it was, the version kept from before
    /// included, and says what could not be; `true` when everything went back.
    fn undo(&mut self, reason: String) -> (String, bool) {
        let first = reason.len();
        let mut reason = reason;
        for name in &self.created {
            let path = self.root.join(name);
            if path.exists() {
                if let Err(error) = fs::remove_dir_all(&path) {
                    reason.push_str(&format!("\n新程序目录未能移除 / New program directory not removed: {}: {error}", path.display()));
                }
            }
        }
        for name in self.moved.iter().rev() {
            let placed = self.root.join(name);
            if let (&"ui", Some(entries)) = (name, &self.ui_entries) {
                self.put_ui_back(&placed, entries, &mut reason);
                continue;
            }
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
                    "更早的旧版本未能删除，下次升级再删 / older version not removed, the next upgrade retries: {}: {error}\n{ADB_HELD}",
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
                        "残留的旧版本仍未能删除 / leftover still not removed: {}: {error}\n{ADB_HELD}",
                        path.display()
                    )),
                }?;
            }
        }
        Ok(())
    }
}
