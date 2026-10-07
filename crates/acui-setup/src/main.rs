// SPDX-License-Identifier: GPL-3.0-only
//! acsetup: the install wizard for ActingCommand on Windows. One window, five
//! pages — location, install, options, instances, finish — each showing
//! where its work stands the way an installer does, the full account going to
//! the install log. It fetches the release to install from the umbrella
//! repository's Releases, takes a folder a person filled by hand, or — as the
//! offline edition — extracts the release it carries (see `payload`), checks it
//! against what the umbrella published, lays it out under a per-user install
//! root, writes the Runtime
//! configuration and the console's settings, optionally a Startup-folder
//! launcher and the emulator instances (see `instance_step`), and can open the
//! console. On a root that already holds an installation it upgrades instead:
//! the payload replaced, maintenance bindings planned with the user, then the ADB server
//! checked with the adb just laid out (see `upgrade` and `adb_server`). The
//! fetch of the release — the resource bundles it carries among its files — is
//! its only network code; it registers no service and no scheduled task, and
//! touches neither PATH nor the registry.
//!
//! The wizard is Windows-only. Elsewhere it compiles, and the first platform
//! question is the loud stop, before any window is opened.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod bundle;
mod cli;
mod fetch;
mod forwarders;
mod generations;
mod install;
mod interfaces;
mod lifecycle;
mod instance_step;
mod log;
mod maintenance;
mod migration;
mod payload;
mod platform;
mod resources;
mod root_tools;
mod runtime;
mod slots;
mod upgrade;
mod verify;
mod vision_migration;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;
use slint::ComponentHandle;

use bundle::Bundle;
use fetch::Release;
use install::{Autostart, Configured, LaidOut};
use instance_step::{Chosen, Found, Settled};
use log::InstallLog;
use payload::Payload;
use slint::{Model, VecModel};
use upgrade::{Installed, Upgraded};
use verify::{Report, Reporter, Step, Total};

slint::include_modules!();


/// What the steps so far established: one run, one root, one log.
#[derive(Default)]
struct State {
    root: PathBuf,
    log: Option<InstallLog>,
    /// The release the online path installs, once looked up, and — on an
    /// upgrade — its `MEMBERS.json`.
    release: Option<Release>,
    release_members: Option<String>,
    /// The offline edition's release, checked at start: set, it is the only
    /// source.
    payload: Option<Payload>,
    /// What the root already holds: set, the run is an upgrade.
    installed: Option<Installed>,
    upgraded: Option<Upgraded>,
    laid_out: Option<LaidOut>,
    configured: Option<Configured>,
    autostart: Option<Autostart>,
    /// The console's shortcuts the options wrote.
    shortcuts: Vec<PathBuf>,
    /// The two commits laid out, and where they came from: a release tag or
    /// the offline folder.
    members: Option<(String, String)>,
    source: String,
    /// The instances step touched the configuration and the Runtime: the
    /// finish page then asks how they stand.
    discover_tried: bool,
    settled: Option<Settled>,
    /// The aliases the instances step wrote into the configuration, and for
    /// each what it runs and the resource package it got.
    instances: Vec<String>,
    assigned: Vec<String>,
    /// The folder the release was verified from: its `MEMBERS.json` names the
    /// resource bundles it carries.
    download: Option<PathBuf>,
    /// The bundles the instances step offers — the release's, read once, then
    /// local files added — and what an instance may be given from them.
    bundles: Vec<Bundle>,
    bundles_read: bool,
    /// What of the release's bundles could not be read, one line each.
    bundle_problems: Vec<String>,
    choices: Vec<Choice>,
    /// The first log write that failed: nothing goes on after it.
    log_error: Option<String>,
    /// What the person must see from any step: on the page, and in the summary.
    warnings: Vec<String>,
    /// Closing was asked once while a Runtime the wizard started runs.
    close_warned: bool,
    /// The instances step started a Runtime, which outlives the wizard.
    runtime_started: bool,
    /// A worker is past the point where stopping it midway would leave the
    /// installation or the configuration half changed: the window stays open.
    guarded: bool,
    /// The launcher and shortcuts the options step wrote in this run, so a
    /// retry can take back what it no longer wants.
    options_written: Vec<PathBuf>,
    /// One explicit resource association, answered on the UI thread.
    decision: Option<std::sync::mpsc::SyncSender<Result<usize, String>>>,
    /// The maintenance binding conflicts, all answered at once on one page.
    conflict_decision: Option<std::sync::mpsc::SyncSender<Result<Vec<maintenance::Side>, String>>>,
    /// A resource-only update that completed (Workflow #364): the finish page's summary.
    resources: Option<resources::Outcome>,
}

/// The worker phases the window must not close in: held while laying files
/// out and configuring, upgrading, writing the options, and while the
/// instances step writes or restarts.
struct Held(Shared);

impl Held {
    fn new(state: &Shared) -> Self {
        lock(state).guarded = true;
        Held(Arc::clone(state))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        lock(&self.0).guarded = false;
    }
}

/// The last panic's message, for the worker that ended with it.
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// A worker that panics stops the run loudly, instead of leaving the page busy
/// for good.
struct PanicGuard {
    state: Shared,
    weak: slint::Weak<SetupWindow>,
}

impl PanicGuard {
    fn new(state: &Shared, weak: &slint::Weak<SetupWindow>) -> Self {
        PanicGuard { state: Arc::clone(state), weak: weak.clone() }
    }
}

impl Drop for PanicGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let message = LAST_PANIC
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .unwrap_or_default();
            fail(
                &self.state,
                &self.weak,
                format!("工作线程异常终止 / A worker thread ended abnormally: {message}"),
            );
        }
    }
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn main() -> Result<()> {
    acui_installation::process_slot_lock()
        .map_err(|error| anyhow::Error::msg(error.to_string()))?;
    // The internal entries below never attach to an interactive console while
    // they work, because its window could be closed midway (review
    // CLI-F2-close); `--rollback` and `--replace-manager` show their result
    // there once the work has ended (`internal_entry`, review R3-1).
    platform::private_std_handles();
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--commit-config")
    {
        return generations::commit_from_stdin().map_err(anyhow::Error::msg);
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--rollback")
    {
        std::process::exit(internal_entry(true));
    }
    if std::env::args_os().nth(1).is_some_and(|argument| argument == "--replace-manager") {
        std::process::exit(internal_entry(false));
    }
    // Any other argument means the command line: its output and its failure
    // reach the calling console or script (Workflow #359).
    if std::env::args_os().nth(1).is_some() {
        platform::attach_console();
        std::process::exit(cli::run());
    }
    std::panic::set_hook(Box::new(|info| {
        *LAST_PANIC
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(info.to_string());
    }));
    let installation = acui_installation::Snapshot::inherited().map_err(anyhow::Error::msg)?;
    let manager_root = acui_installation::current_manager_root().map_err(anyhow::Error::msg)?;
    let root = match (&manager_root, &installation) {
        (Some(root), Some(snapshot))
            if !acui_installation::same_install_root(root, &snapshot.root)
                .map_err(anyhow::Error::msg)? =>
        {
            anyhow::bail!("Management entry and inherited installation root disagree")
        }
        (Some(root), _) => root.clone(),
        (None, Some(snapshot)) => snapshot.root.clone(),
        (None, None) => platform::default_install_root().map_err(anyhow::Error::msg)?,
    };
    let download = platform::default_download_dir();
    let state: Shared = Arc::new(Mutex::new(State::default()));
    let window = SetupWindow::new()?;
    window.set_install_root(root.display().to_string().into());
    window.set_download_dir(
        download
            .map(|dir| dir.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    window.set_can_next(true);
    // One choices model for the page's life; see `show_offer`.
    window.set_choices(Rc::new(VecModel::<slint::SharedString>::default()).into());
    // The edition is read before anything else: a carried release that cannot
    // be used stops the wizard here, with nothing written.
    match payload::detect() {
        Ok(payload) => {
            window.set_embedded(payload.is_some());
            lock(&state).payload = payload;
        }
        Err(reason) => {
            window.set_failed(true);
            window.set_failure_text(
                format!("{reason}\n\n尚未写任何文件，也没有日志 / Nothing has been written, and there is no log yet").into(),
            );
        }
    }
    refresh_preflight(&window, &state);
    install_callbacks(&window, &state);
    window.run()?;
    Ok(())
}

/// The installation root an internal entry logs into, known before anything
/// can fail: for `--rollback` the root of the running fixed manager
/// (`<root>\ui\acsetup.exe`), for `--replace-manager` its first argument.
fn entry_root(rollback: bool) -> Option<PathBuf> {
    if rollback {
        let exe = std::env::current_exe().ok()?;
        let ui = exe.parent()?;
        if ui.file_name()? != "ui" {
            return None;
        }
        ui.parent().map(Path::to_path_buf)
    } else {
        let root = PathBuf::from(std::env::args_os().nth(2)?);
        (root.is_absolute() && root.is_dir()).then_some(root)
    }
}

/// `--rollback` and `--replace-manager` (review R3-1). The log is created before
/// anything that can fail and ends with the result or `失败 / FAILED: …`. The
/// work runs attached to no console (CLI-F2-close), with Ctrl+C refused, so
/// closing a window cannot end it midway. Once it has ended, the result and the
/// log path go to the caller's console, attached only now, or to the handles it
/// redirected. The exit code is 0 or 1.
fn internal_entry(rollback: bool) -> i32 {
    use std::io::Write as _;
    let guard = platform::InterruptGuard::start();
    let root = entry_root(rollback);
    let (mut log, mut notes) = match root.as_deref().map(|root| InstallLog::create(root, log::unix_ms())) {
        Some(Ok(log)) => (Some(log), Vec::new()),
        Some(Err(error)) => (None, vec![format!("日志未能创建 / The log could not be created: {error}")]),
        None => (None, vec!["没有日志：安装根未知 / No log: the installation root is unknown".to_string()]),
    };
    let outcome = {
        let mut report = |line: &str| -> Result<(), String> {
            match log.as_mut() {
                Some(log) => log
                    .line(line)
                    .map_err(|error| format!("日志写入失败 / Log write failed: {error}")),
                None => Ok(()),
            }
        };
        if rollback {
            upgrade::rollback_from_entry(&mut report)
        } else {
            install::replace_manager_from_entry(&mut report)
        }
    };
    drop(guard);
    let (code, text) = match outcome {
        Ok(line) => (0, line),
        Err(error) => (1, format!("失败 / FAILED: {error}")),
    };
    if let Some(log) = log.as_mut() {
        match log.line(&text) {
            Ok(()) => notes.push(format!("日志 / Log: {}", log.path().display())),
            Err(error) => notes.push(format!(
                "日志写入失败 / Log write failed: {}: {error}",
                log.path().display()
            )),
        }
    }
    platform::attach_console();
    let shown = std::iter::once(text).chain(notes).collect::<Vec<_>>().join("\n");
    if code == 0 {
        let _ = writeln!(std::io::stdout(), "{shown}");
    } else {
        let _ = writeln!(std::io::stderr(), "{shown}");
    }
    code
}

fn install_callbacks(window: &SetupWindow, state: &Shared) {
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_decide(move |accepted| {
            if let Some(window) = weak.upgrade() {
                let selected = window.get_decision_choice();
                if accepted && selected < 1 { return; }
                let answer = if accepted { Ok((selected - 1) as usize) }
                    else { Err("用户取消配置计划 / Configuration plan cancelled by the user".into()) };
                let sender = lock(&state).decision.take();
                window.set_step(window.get_decision_return_step());
                window.set_busy(true);
                if let Some(sender) = sender {
                    if sender.send(answer).is_err() {
                        fail(&state, &window.as_weak(), "配置选择已过期 / Configuration choice expired".into());
                    }
                }
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_conflicts_decide(move |how| {
            if let Some(window) = weak.upgrade() {
                decide_conflicts(&window, &state, how);
            }
        });
    }
    {
        let weak = window.as_weak();
        window.on_conflict_toggle(move |row, side| {
            if let Some(window) = weak.upgrade() {
                // One box per side: checking one clears the other; checking a
                // checked box leaves the row unchosen.
                let rows = window.get_conflicts();
                if let Some(at) = usize::try_from(row).ok() {
                    if let Some(mut data) = rows.row_data(at) {
                        data.choice = if data.choice == side { 0 } else { side };
                        rows.set_row_data(at, data);
                    }
                }
                refresh_conflict_tally(&window);
            }
        });
    }
    {
        let weak = window.as_weak();
        window.on_conflict_header_toggle(move |side| {
            if let Some(window) = weak.upgrade() {
                // A checked header (every row on that side) clears that side on
                // every row; an unchecked or indeterminate one sets it on every row
                // and so clears the other column.
                let rows = window.get_conflicts();
                let count = rows.row_count();
                let all = count > 0 && rows.iter().all(|row| row.choice == side);
                for at in 0..count {
                    if let Some(mut data) = rows.row_data(at) {
                        let choice = match all {
                            true => 0,
                            false => side,
                        };
                        if data.choice != choice {
                            data.choice = choice;
                            rows.set_row_data(at, data);
                        }
                    }
                }
                refresh_conflict_tally(&window);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_install_root_edited(move |_| {
            if let Some(window) = weak.upgrade() {
                // What was said of the location before is not said of this one.
                window.set_note("".into());
                refresh_preflight(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_next(move || {
            if let Some(window) = weak.upgrade() {
                next(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_offline_toggled(move |offline| {
            if let Some(window) = weak.upgrade() {
                offline_toggled(&window, &state, offline);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_retry_release(move || {
            if let Some(window) = weak.upgrade() {
                find_release(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.window().on_close_requested(move || match weak.upgrade() {
            Some(window) => close_requested(&window, &state),
            None => slint::CloseRequestResponse::HideWindow,
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_open_console(move || {
            if let Some(window) = weak.upgrade() {
                open_console(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_add_bundle({
            let weak = weak.clone();
            let state = Arc::clone(&state);
            move || {
                if let Some(window) = weak.upgrade() {
                    begin_add_bundle(&window, &state);
                }
            }
        });
        window.on_discover(move || {
            if let Some(window) = weak.upgrade() {
                begin_discover(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        window.on_skip_instances(move || {
            if let Some(window) = weak.upgrade() {
                match write_log(&state, "实例：跳过 / instances: skipped") {
                    Ok(()) => settle_and_finish(&window, &state),
                    Err(reason) => fail(&state, &window.as_weak(), reason),
                }
            }
        });
    }
    window.on_close_wizard(|| {
        let _ = slint::quit_event_loop();
    });
}

/// The window's close button: refused while files are laid out and configured,
/// an upgrade swaps versions, the options are written or the instances step
/// writes — work half done there cannot
/// be put back once the wizard is gone; a lookup or a download may be closed.
/// The first time after the instances step started a Runtime, that is said.
fn close_requested(window: &SetupWindow, state: &Shared) -> slint::CloseRequestResponse {
    let mut state = lock(state);
    if state.guarded {
        window.set_note(
            "正在写入，请等它完成再关闭 / Files are being written: wait for it to finish before closing".into(),
        );
        return slint::CloseRequestResponse::KeepWindowShown;
    }
    if state.runtime_started && state.settled.is_none() && !state.close_warned && !window.get_failed() {
        state.close_warned = true;
        window.set_note(
            "实例步拉起过 Runtime，它在向导关闭后可能仍在后台运行，监控台会接上它；再点一次关闭即退出 / The instances step started a Runtime, which may keep running after this wizard closes, and the console attaches to it; close again to exit"
                .into(),
        );
        return slint::CloseRequestResponse::KeepWindowShown;
    }
    slint::CloseRequestResponse::HideWindow
}

/// The two lines under the install root: the room on its volume — and, for the
/// offline edition, what extracting its release takes — and whether a Runtime
/// is already installed there.
fn refresh_preflight(window: &SetupWindow, state: &Shared) {
    let root = PathBuf::from(window.get_install_root().as_str());
    let mut free = match platform::free_space(&root) {
        Ok(bytes) => format!(
            "该卷可用空间 / Free space on this volume: {:.1} GiB",
            bytes as f64 / (1u64 << 30) as f64
        ),
        Err(reason) => format!("可用空间未知 / Free space unknown: {reason}"),
    };
    if let Some(carried) = lock(state).payload.as_ref().map(Payload::total) {
        free.push_str(&format!(
            "；取出自带的发布件需 {:.1} MiB，解压另需空间 / extracting the carried release takes {:.1} MiB, unpacking it more",
            mib(carried),
            mib(carried)
        ));
    }
    window.set_free_space_text(free.into());
    // A resource-only update needs an A/B installation (Workflow #364): only then is it offered.
    let selection = root.join(acui_installation::INSTALL_SELECTION_PATH);
    let ab = root.is_absolute() && selection.is_file();
    let existing = match upgrade::installed(&root) {
        Ok(None) => "此处没有已安装的 Runtime / No installation here yet".to_string(),
        Ok(Some(installed)) => format!(
            "此处已有安装：runtime {} · ui {}；下一步将升级并检查维护配置{} / Installed here: the next steps upgrade it and check maintenance configuration{}",
            short(&installed.runtime_sha),
            short(&installed.ui_sha),
            if ab { "，也可勾选只更新资源" } else { "（旧布局：只更新资源须先做一次完整升级）" },
            if ab {
                ", or tick to update resources only"
            } else {
                " (old layout: a resource-only update needs a full upgrade first)"
            }
        ),
        Err(reason) => reason,
    };
    window.set_existing_text(existing.into());
    window.set_resources_available(ab);
    if !ab {
        window.set_resources_only(false);
    }
}

fn next(window: &SetupWindow, state: &Shared) {
    match window.get_step() {
        0 if window.get_resources_only() => enter_resources(window, state),
        0 => enter_get(window, state),
        1 => begin_install(window, state),
        2 => apply_options(window, state),
        3 => begin_apply(window, state),
        7 => begin_resources(window, state),
        _ => {}
    }
}

/// Step 0 → 7 (Workflow #364): a resource-only update. The checks `enter_get` makes — an
/// absolute root, here an A/B installation, the rule against running from a program folder of
/// it — then the log; no release is looked up. Leftover staging is cleared by the run itself,
/// once it holds the writer lock (`begin_resources`, review F-UI2-1).
fn enter_resources(window: &SetupWindow, state: &Shared) {
    let root = PathBuf::from(window.get_install_root().as_str());
    let checked = match root.is_absolute() {
        true => resources::installation(&root),
        false => Err("安装根必须是绝对路径 / The install root must be an absolute path".to_string()),
    };
    let checked = checked.and_then(|()| match running_from(&root) {
        Some((name, dir)) => Err(format!(
            "本引导 {name} 正从这份安装的 {dir}\\ 里运行：请把 {name} 复制到别处再运行 / This wizard, {name}, runs from {dir}\\ of this installation: copy {name} elsewhere and run it from there"
        )),
        None => Ok(()),
    });
    if let Err(text) = checked {
        window.set_note(text.into());
        return;
    }
    let log = match InstallLog::create(&root, log::unix_ms()) {
        Ok(log) => log,
        Err(error) => {
            window.set_note(
                format!(
                    "无法创建安装日志 / Cannot create the install log: {}: {error}",
                    root.display()
                )
                .into(),
            );
            return;
        }
    };
    window.set_log_path(format!("日志 / Log: {}", log.path().display()).into());
    let carried = lock(state).payload.as_ref().map(|payload| payload.tag.clone());
    {
        let mut state = lock(state);
        state.root = root.clone();
        state.log = Some(log);
    }
    let started = write_log(
        state,
        &format!(
            "acsetup {} · 只更新资源 / resource-only update · 安装根 / install root: {}",
            env!("CARGO_PKG_VERSION"),
            root.display()
        ),
    )
    .and_then(|()| match &carried {
        Some(tag) => write_log(
            state,
            &format!("离线版自带的发布件 {tag} 此时不用 / The offline edition's carried release {tag} is not used"),
        ),
        None => Ok(()),
    });
    if let Err(reason) = started {
        fail(state, &window.as_weak(), reason);
        return;
    }
    window.set_note("".into());
    window.set_notes(lock(state).warnings.join("\n").into());
    window.set_step(7);
    window.set_can_next(true);
}

/// Step 7, one worker from start to end (Workflow #364): the writer lock taken before leftover
/// staging is cleared (model step 1, review F-UI2-1) and held to the end, the bundle checked,
/// staged and admitted, its packs compared with the installed ones, its bindings planned — the
/// association and conflict pages as for an upgrade, where Cancel changes nothing — then the
/// new packs placed and, when bindings change, a new generation committed with the Runtime
/// closed and started again. Success goes to the finish page.
fn begin_resources(window: &SetupWindow, state: &Shared) {
    let zip = PathBuf::from(window.get_resource_zip().trim());
    if !zip.is_absolute() || !zip.is_file() {
        window.set_note(
            format!(
                "标准包须为存在的文件的绝对路径 / The bundle must be the absolute path of an existing file: {}",
                zip.display()
            )
            .into(),
        );
        return;
    }
    let sums = window.get_resource_sums().trim().to_string();
    let sums = (!sums.is_empty()).then(|| PathBuf::from(sums));
    if let Some(sums) = sums.as_ref().filter(|sums| !sums.is_absolute() || !sums.is_file()) {
        window.set_note(
            format!(
                "SHA256SUMS 须为存在的文件的绝对路径 / SHA256SUMS must be the absolute path of an existing file: {}",
                sums.display()
            )
            .into(),
        );
        return;
    }
    window.set_note("".into());
    window.set_busy(true);
    window.set_can_next(false);
    let root = lock(state).root.clone();
    let staging = root.join(format!(".staging-{}", log::unix_ms()));
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-resources".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let mut report = PageReport::new(&state, &worker_weak);
        // The window stays open until the run has ended, its staging removed.
        let held = Held::new(&state);
        // 1. The writer lock before anything under the root is removed: a run that holds the
        // lock never has its staging cleared away by this one (review F-UI2-1).
        let writer = generations::Writer::acquire(&root).and_then(|writer| {
            clear_staging(&state, &root)?;
            Ok(writer)
        });
        let writer = match writer {
            Ok(writer) => writer,
            Err(reason) => {
                drop(held);
                fail(&state, &worker_weak, reason);
                return;
            }
        };
        let outcome = {
            let mut choose = |association: &maintenance::Association| {
                let labels = association.options.iter().map(|option| option.label.clone()).collect();
                ask(&state, &worker_weak, association.question.clone(), labels).map(Some)
            };
            let mut resolve = |conflicts: &[maintenance::Conflict]| resolve_page(&state, &worker_weak, conflicts);
            resources::run(&writer, &zip, sums.as_deref(), &staging, &mut choose, &mut resolve, &mut report)
        };
        // The run has ended: its staging goes, whatever the outcome (Workflow #364 Q5).
        let removed = remove_staging(&staging, &mut report);
        drop(writer);
        drop(held);
        if let Err(reason) = removed {
            fail(&state, &worker_weak, reason);
            return;
        }
        match outcome {
            Ok(outcome) => {
                {
                    let mut locked = lock(&state);
                    // The console opens through the installation's fixed entries (review L6).
                    locked.laid_out = Some(outcome.laid_out.clone());
                    locked.resources = Some(outcome);
                }
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    window.set_busy(false);
                    finish(&window, &state);
                });
            }
            Err(stop) => fail(&state, &worker_weak, String::from(stop)),
        }
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// Worker/UI rendezvous for the installer's normal explicit choices. The worker
/// holds no State lock while waiting; Cancel follows its existing error/rollback path.
fn ask(state: &Shared, weak: &slint::Weak<SetupWindow>, text: String, choices: Vec<String>) -> Result<usize, String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    {
        let mut state = lock(state);
        if state.decision.is_some() { return Err("Another configuration choice is pending".into()); }
        state.decision = Some(sender);
    }
    let shown = weak.upgrade_in_event_loop(move |window| {
        window.set_decision_return_step(window.get_step());
        window.set_decision_text(text.into());
        let labels: Vec<slint::SharedString> = std::iter::once("请选择 / Choose".to_string()).chain(choices).map(Into::into).collect();
        window.set_decision_options(Rc::new(VecModel::from(labels)).into());
        window.set_decision_choice(0);
        window.set_step(5);
    });
    if let Err(error) = shown {
        lock(state).decision = None;
        return Err(format!("Cannot show configuration choice: {error}"));
    }
    let result = receiver.recv_timeout(Duration::from_secs(30 * 60))
        .map_err(|error| format!("配置选择未完成（30 分钟期限）/ Configuration choice not completed (30-minute limit): {error}"));
    lock(state).decision = None;
    let _ = weak.upgrade_in_event_loop(|window| {
        if window.get_step() == 5 { window.set_step(window.get_decision_return_step()); }
    });
    result?
}

/// All maintenance binding conflicts on one page (Workflow #359): every row
/// with its old and new value and their sources; the person takes all new, all
/// old, or decides each row and confirms once. The worker holds no State lock
/// while waiting; Cancel follows its existing error/rollback path.
fn resolve_page(
    state: &Shared,
    weak: &slint::Weak<SetupWindow>,
    conflicts: &[maintenance::Conflict],
) -> Result<Vec<maintenance::Side>, String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    {
        let mut state = lock(state);
        if state.decision.is_some() || state.conflict_decision.is_some() {
            return Err("Another configuration choice is pending".into());
        }
        state.conflict_decision = Some(sender);
    }
    let rows: Vec<ConflictRow> = conflicts
        .iter()
        .map(|conflict| ConflictRow {
            item: conflict.item.clone().into(),
            current: conflict.old_text().into(),
            current_source: conflict.old_source.clone().into(),
            proposed: conflict.new_text().into(),
            proposed_source: conflict.new_source.clone().into(),
            choice: 0,
        })
        .collect();
    let heading = format!(
        "维护绑定差异 / Maintenance binding differences: {}",
        conflicts.len()
    );
    let summary = maintenance::summary(conflicts);
    let shown = weak.upgrade_in_event_loop(move |window| {
        window.set_decision_return_step(window.get_step());
        window.set_conflicts(Rc::new(VecModel::from(rows)).into());
        window.set_conflicts_heading(heading.into());
        window.set_conflicts_summary(summary.into());
        refresh_conflict_tally(&window);
        window.set_step(6);
    });
    if let Err(error) = shown {
        lock(state).conflict_decision = None;
        return Err(format!("Cannot show the maintenance binding differences: {error}"));
    }
    let result = receiver.recv_timeout(Duration::from_secs(30 * 60))
        .map_err(|error| format!("配置选择未完成（30 分钟期限）/ Configuration choice not completed (30-minute limit): {error}"));
    lock(state).conflict_decision = None;
    let _ = weak.upgrade_in_event_loop(|window| {
        if window.get_step() == 6 { window.set_step(window.get_decision_return_step()); }
    });
    result?
}

/// The conflict page's answer: 3 the rows as checked (only when every row has
/// exactly one side), anything else cancels the plan.
fn decide_conflicts(window: &SetupWindow, state: &Shared, how: i32) {
    let rows = window.get_conflicts();
    let answer = match how {
        3 => {
            let sides: Option<Vec<_>> = rows
                .iter()
                .map(|row| match row.choice {
                    1 => Some(maintenance::Side::Old),
                    2 => Some(maintenance::Side::New),
                    _ => None,
                })
                .collect();
            match sides {
                Some(sides) => Ok(sides),
                None => return,
            }
        }
        _ => Err("用户取消配置计划 / Configuration plan cancelled by the user".to_string()),
    };
    let sender = lock(state).conflict_decision.take();
    window.set_step(window.get_decision_return_step());
    window.set_busy(true);
    if let Some(sender) = sender {
        if sender.send(answer).is_err() {
            fail(state, &window.as_weak(), "配置选择已过期 / Configuration choice expired".into());
        }
    }
}

/// How many rows are decided each way, whether all are, and each header box:
/// checked when every row has its side, unchecked when none, else the square.
fn refresh_conflict_tally(window: &SetupWindow) {
    let (mut old, mut new, mut open) = (0, 0, 0);
    for row in window.get_conflicts().iter() {
        match row.choice {
            1 => old += 1,
            2 => new += 1,
            _ => open += 1,
        }
    }
    let total = old + new + open;
    let header = |count: i32| match count {
        0 => 0,
        count if count == total => 1,
        _ => 2,
    };
    window.set_conflicts_header_current(header(old));
    window.set_conflicts_header_proposed(header(new));
    window.set_conflicts_tally(
        format!("新值 / proposed {new} · 旧值 / current {old} · 未选 / undecided {open}").into(),
    );
    window.set_conflicts_complete(open == 0);
}

/// A worker's reporter. Every line goes into the log — a log write failing
/// fails the run, the log being the record the person is told to read. The
/// page shows only where the work stands: the phase, a bar, and the latest
/// line as the one thing being done now; warnings stay on it.
struct PageReport {
    state: Shared,
    weak: slint::Weak<SetupWindow>,
    phase: String,
    total: Option<Total>,
    shown: Option<Instant>,
}

impl PageReport {
    /// A new worker's reporter; what the one before left on the page is cleared.
    fn new(state: &Shared, weak: &slint::Weak<SetupWindow>) -> Self {
        let _ = weak.upgrade_in_event_loop(|window| {
            window.set_stage_text("".into());
            window.set_detail_text("".into());
            window.set_fraction(0.0);
            window.set_indeterminate(true);
        });
        PageReport { state: Arc::clone(state), weak: weak.clone(), phase: String::new(), total: None, shown: None }
    }

    fn show(&self, done: u64) {
        let (stage, fraction) = match self.total {
            None => (format!("{}…", self.phase), 0.0),
            Some(Total::Items(total)) => (format!("{} · {done}/{total}", self.phase), ratio(done, total)),
            Some(Total::Bytes(total)) => (
                format!("{} · {:.1}/{:.1} MiB", self.phase, mib(done), mib(total)),
                ratio(done, total),
            ),
        };
        let indeterminate = self.total.is_none();
        let _ = self.weak.upgrade_in_event_loop(move |window| {
            window.set_stage_text(stage.into());
            window.set_fraction(fraction);
            window.set_indeterminate(indeterminate);
        });
    }
}

impl Reporter for PageReport {
    fn line(&mut self, line: &str) -> Result<(), String> {
        write_log(&self.state, line)?;
        let detail = line.lines().next().unwrap_or_default().trim().to_string();
        let _ = self.weak.upgrade_in_event_loop(move |window| window.set_detail_text(detail.into()));
        Ok(())
    }

    fn step(&mut self, step: Step<'_>) -> Result<(), String> {
        match step {
            // The page only: a marker for the eye never stands in the way of
            // the work, the log carrying the work's own lines.
            Step::Phase(name, total) => {
                self.phase = name.to_string();
                self.total = total;
                self.shown = Some(Instant::now());
                self.show(0);
                let _ = self.weak.upgrade_in_event_loop(|window| window.set_detail_text("".into()));
            }
            Step::Done(done) => {
                // The bar moves at most ten times a second, and always to its end.
                let end = matches!(self.total, Some(Total::Items(total) | Total::Bytes(total)) if done >= total);
                if end || self.shown.is_none_or(|at| at.elapsed() >= Duration::from_millis(100)) {
                    self.shown = Some(Instant::now());
                    self.show(done);
                }
            }
        }
        Ok(())
    }

    fn warn(&mut self, line: &str) -> Result<(), String> {
        write_log(&self.state, &format!("注意 / NOTE: {line}"))?;
        let notes = {
            let mut state = lock(&self.state);
            if !state.warnings.iter().any(|kept| kept == line) {
                state.warnings.push(line.to_string());
            }
            state.warnings.join("\n")
        };
        let _ = self.weak.upgrade_in_event_loop(move |window| window.set_notes(notes.into()));
        Ok(())
    }
}

fn ratio(done: u64, total: u64) -> f32 {
    (done as f64 / total.max(1) as f64).min(1.0) as f32
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 20) as f64
}

fn write_log(state: &Shared, line: &str) -> Result<(), String> {
    let mut state = lock(state);
    let written = match state.log.as_mut() {
        None => Err("日志尚未创建 / The log has not been created".to_string()),
        Some(log) => {
            let path = log.path().display().to_string();
            log.line(line)
                .map_err(|error| format!("日志写入失败 / Log write failed: {path}: {error}"))
        }
    };
    if let Err(error) = &written {
        state.log_error.get_or_insert_with(|| error.clone());
    }
    written
}

/// The loud stop: the reason as the log's last line, then the page that
/// shows only it and the log path.
fn fail(state: &Shared, weak: &slint::Weak<SetupWindow>, reason: String) {
    let mut text = reason.clone();
    if let Err(unlogged) = write_log(state, &format!("失败 / FAILED: {reason}")) {
        text.push('\n');
        text.push_str(&unlogged);
    }
    if let Some(log) = lock(state).log.as_ref() {
        text.push_str(&format!("\n\n日志 / Log: {}", log.path().display()));
    }
    let _ = weak.upgrade_in_event_loop(move |window| {
        window.set_busy(false);
        window.set_failed(true);
        window.set_failure_text(text.into());
    });
}

/// Step 0 → 1: the root checked, then the root and the log created — the
/// first thing the wizard writes — and, online, the release looked up.
fn enter_get(window: &SetupWindow, state: &Shared) {
    let root = PathBuf::from(window.get_install_root().as_str());
    let installed = if root.is_absolute() {
        upgrade::installed(&root)
    } else {
        Err("安装根必须是绝对路径 / The install root must be an absolute path".to_string())
    };
    let installed = match installed {
        Ok(Some(installed)) => match running_from(&root) {
            Some((name, dir)) => Err(format!(
                "本引导 {name} 正从要升级的这份安装的 {dir}\\ 里运行，升级要把它移开：请把 {name} 复制到别处再运行 / This wizard, {name}, runs from {dir}\\ of the installation it would upgrade, which the upgrade moves aside: copy {name} elsewhere and run it from there"
            )),
            None => Ok(Some(installed)),
        },
        other => other,
    };
    let installed = match installed {
        Ok(installed) => installed,
        Err(text) => {
            window.set_note(text.into());
            return;
        }
    };
    // A fresh install writes its state root under the root itself, and the
    // console's settings name the root in TOML literal strings: either one
    // that cannot be is said before anything is fetched.
    if installed.is_none() {
        let usable = install::state_root_usable(&root.join("state")).and_then(|()| {
            match root.display().to_string().contains('\'') {
                true => Err("安装位置含单引号，监控台设置无法原样写入 / The install location contains a single quote, which the console's settings cannot hold".to_string()),
                false => Ok(()),
            }
        });
        if let Err(reason) = usable {
            window.set_note(reason.into());
            return;
        }
    }
    let log = match std::fs::create_dir_all(&root)
        .and_then(|()| InstallLog::create(&root, log::unix_ms()))
    {
        Ok(log) => log,
        Err(error) => {
            window.set_note(
                format!(
                    "无法创建安装根或日志 / Cannot create the install root or the log: {}: {error}",
                    root.display()
                )
                .into(),
            );
            return;
        }
    };
    window.set_log_path(format!("日志 / Log: {}", log.path().display()).into());
    let started = format!(
        "acsetup {}{} · 安装根 / install root: {}{}",
        env!("CARGO_PKG_VERSION"),
        lock(state)
            .payload
            .as_ref()
            .map_or(String::new(), |payload| format!(" · 离线版 / offline edition {}", payload.tag)),
        root.display(),
        installed.as_ref().map_or(String::new(), |installed| format!(
            " · 升级 / upgrade from runtime {} · ui {}",
            installed.runtime_sha, installed.ui_sha
        ))
    );
    {
        let mut state = lock(state);
        state.root = root.clone();
        state.log = Some(log);
        state.installed = installed;
    }
    if let Err(reason) = write_log(state, &started).and_then(|()| clear_staging(state, &root)) {
        fail(state, &window.as_weak(), reason);
        return;
    }
    window.set_note("".into());
    window.set_notes(lock(state).warnings.join("\n").into());
    window.set_upgrading(lock(state).installed.is_some());
    window.set_step(1);
    let carried = lock(state).payload.is_some();
    match carried {
        true => enter_carried(window, state),
        false => offline_toggled(window, state, window.get_offline()),
    }
}

/// Step 1 of the offline edition: the release it carries, checked at start,
/// is the one to install — no lookup, no folder. The resource plan is checked
/// even at the same program commits; an older release shows the downgrade check.
fn enter_carried(window: &SetupWindow, state: &Shared) {
    let (line, members_text, upgrading, current) = {
        let state = lock(state);
        let Some(payload) = state.payload.as_ref() else {
            return;
        };
        let wanted = &payload.members;
        let mut line = format!(
            "本安装包自带发布件 / This installer carries release {}（runtime {} · ui {}）",
            payload.tag,
            short(&wanted.0),
            short(&wanted.1)
        );
        let installed = state.installed.as_ref();
        if let Some(installed) = installed {
            line.push('\n');
            line.push_str(&upgrade_line(installed, wanted));
        }
        let current = installed.is_some_and(|installed| installed.current(wanted));
        (line, payload.members_text.clone(), installed.is_some(), current)
    };
    if let Err(reason) = write_log(state, &line) {
        fail(state, &window.as_weak(), reason);
        return;
    }
    window.set_release_text(line.into());
    window.set_note(if current { UP_TO_DATE } else { "" }.into());
    window.set_can_next(true);
    if upgrading {
        let root = lock(state).root.clone();
        confirm_downgrade(window, upgrade::downgrade(&root, &members_text));
    }
}

/// Whether the release to install may go on over what is installed: `note`
/// says why it needs the person's word, and it goes on once they tick
/// 「确认降级」 for that very note; a different note clears the tick.
fn confirm_downgrade(window: &SetupWindow, note: Option<String>) -> bool {
    let note = note.unwrap_or_default();
    if window.get_downgrade_note().as_str() != note {
        window.set_downgrade_note(note.as_str().into());
        window.set_downgrade_ok(false);
    }
    note.is_empty() || window.get_downgrade_ok()
}

/// On an upgrade, whether the downgrade check stops Install for the release
/// whose `MEMBERS.json` is `wanted`: the check shown, and why said.
fn downgrade_stops(window: &SetupWindow, state: &Shared, wanted: &str) -> bool {
    let root = lock(state).root.clone();
    if confirm_downgrade(window, upgrade::downgrade(&root, wanted)) {
        return false;
    }
    window.set_note("勾选「确认降级」后再继续 / Tick Confirm downgrade to go on".into());
    true
}

/// Staging directories a run closed midway left under the root: removed, and
/// said so; one that cannot be removed is kept as a note for the summary.
fn clear_staging(state: &Shared, root: &Path) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(".staging-"));
        if !stale || !path.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => write_log(state, &format!("已删除上次留下的临时目录 / leftover staging removed: {}", path.display()))?,
            Err(error) => {
                let line = format!("上次留下的临时目录未能删除 / leftover staging not removed: {}: {error}", path.display());
                write_log(state, &line)?;
                lock(state).warnings.push(line);
            }
        }
    }
    Ok(())
}

/// A run's own staging directory, removed once the run has ended — succeeded or failed — and
/// after its last use (Workflow #364, ruling Q5): nothing ever runs from it and nothing recovers
/// from it. A removal that fails is a note for the summary, and the run's result stands; a run
/// that was killed leaves it to `clear_staging` at the next start.
fn remove_staging(staging: &Path, report: Report<'_>) -> Result<(), String> {
    match staging.try_exists() {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(error) => {
            return report.warn(&format!(
                "临时目录无法检查 / staging cannot be inspected: {}: {error}",
                staging.display()
            ))
        }
    }
    match std::fs::remove_dir_all(staging) {
        Ok(()) => report.line(&format!("临时目录已删除 / staging removed: {}", staging.display())),
        Err(error) => report.warn(&format!(
            "临时目录未能删除，下次运行开始时再删 / staging not removed; the next run removes it when it starts: {}: {error}",
            staging.display()
        )),
    }
}

/// The install step's choice: a folder is taken as it is; the online path needs a
/// release, looked up once.
fn offline_toggled(window: &SetupWindow, state: &Shared, offline: bool) {
    window.set_note("".into());
    // A folder's release is judged when Install is pressed; the online one as
    // looked up.
    let online = {
        let state = lock(state);
        match offline {
            true => None,
            false => state.release_members.as_ref().and_then(|text| upgrade::downgrade(&state.root, text)),
        }
    };
    confirm_downgrade(window, online);
    if offline || lock(state).release.is_some() {
        window.set_can_next(true);
    } else {
        find_release(window, state);
    }
}

/// Looks the release up off the event loop. Not finding one is stated on the
/// page and in the log, and leaves the offline folder open; a log write
/// failing stops the run.
fn find_release(window: &SetupWindow, state: &Shared) {
    window.set_release_text("正在查询伞仓发布件… / Looking up the umbrella releases…".into());
    window.set_release_failed(false);
    window.set_note("".into());
    window.set_busy(true);
    window.set_can_next(false);
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-release".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let (root, installed) = {
            let locked = lock(&state);
            (locked.root.clone(), locked.installed.clone())
        };
        // An upgrade reads MEMBERS.json for program identity and the downgrade
        // choice. Matching program commits still need a resource/configuration plan.
        let found = fetch::choose().and_then(|release| {
            release.folder()?;
            let text = installed.as_ref().map(|_| fetch::members(&release)).transpose()?;
            let wanted = text.as_deref().map(verify::members_of).transpose()?;
            Ok((release, wanted, text))
        });
        let (line, current) = match &found {
            Ok((release, wanted, _)) => {
                let mut line = release_line(release);
                if let (Some(installed), Some(wanted)) = (&installed, wanted) {
                    line.push('\n');
                    line.push_str(&upgrade_line(installed, wanted));
                }
                let current = matches!((&installed, wanted), (Some(installed), Some(wanted)) if installed.current(wanted));
                (line, current)
            }
            Err(reason) => (reason.clone(), false),
        };
        if let Err(reason) = write_log(&state, &line) {
            fail(&state, &worker_weak, reason);
            return;
        }
        let found = found.ok().map(|(release, _, text)| (release, text));
        let ok = found.is_some();
        let downgrade = found.as_ref().and_then(|(_, text)| text.as_deref()).and_then(|text| upgrade::downgrade(&root, text));
        {
            let mut locked = lock(&state);
            (locked.release, locked.release_members) = found.map_or((None, None), |(release, text)| (Some(release), text));
        }
        let _ = worker_weak.upgrade_in_event_loop(move |window| {
            window.set_busy(false);
            if !window.get_offline() {
                confirm_downgrade(&window, downgrade);
            }
            window.set_release_text(line.into());
            window.set_can_next(ok || window.get_offline());
            window.set_release_failed(!ok && !current);
            window.set_note("".into());
            if current {
                window.set_note(UP_TO_DATE.into());
            } else if !ok {
                window.set_note(
                    "可以「重新查询」，或勾选「离线」改用已下载的发布件文件夹 / Look up again, or tick Offline to use a folder of downloaded release files"
                        .into(),
                );
            }
        });
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

const UP_TO_DATE: &str = "程序版本相同，继续检查资源与维护配置 / Program versions match; continue to check resources and maintenance configuration";

/// What an upgrade replaces with what — and, at the very release
/// installed, that AC's adb is missing, so it is laid out again.
fn upgrade_line(installed: &Installed, wanted: &(String, String)) -> String {
    let mut line = format!(
        "升级 / Upgrade: runtime {} → {} · ui {} → {}",
        short(&installed.runtime_sha),
        short(&wanted.0),
        short(&installed.ui_sha),
        short(&wanted.1)
    );
    if !installed.adb && installed.runtime_sha == wanted.0 && installed.ui_sha == wanted.1 {
        line.push_str("\n已是这个发布件的版本，但缺少 tools\\platform-tools\\adb.exe：按升级的方式重新铺开 / Already at this release's version, but tools\\platform-tools\\adb.exe is missing: it is laid out again the way an upgrade is");
    }
    line
}

fn short(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}

/// This wizard's own file name and the directory of `root` it runs from, when
/// that is one an upgrade moves aside or replaces — it could not move while
/// the wizard runs. From the root itself or `downloads\` it upgrades as usual.
fn running_from(root: &Path) -> Option<(String, &'static str)> {
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .ok()?;
    let name = exe.file_name()?.to_string_lossy().into_owned();
    if root.join(acui_installation::MANAGER_DIRECTORY).is_dir()
        && root
            .join("ui/acsetup.exe")
            .canonicalize()
            .is_ok_and(|manager| manager == exe)
    {
        return None;
    }
    ["runtime", "ui", "tools", "A", "B", "previous"]
        .into_iter()
        .find(|dir| {
            root.join(dir)
                .canonicalize()
                .is_ok_and(|dir| exe.starts_with(dir))
        })
        .map(|dir| (name, dir))
}

fn release_line(release: &Release) -> String {
    format!(
        "将安装 / To install: {}{} · {} · {} · {:.1} MiB",
        release.tag_name,
        release.name.as_deref().map(|name| format!("（{name}）")).unwrap_or_default(),
        release.published_at.as_deref().unwrap_or("—"),
        if release.prerelease { "预发布 / pre-release" } else { "正式版 / stable" },
        release.size() as f64 / (1u64 << 20) as f64
    )
}

/// Step 1, one worker from start to end: online, the release fetched into
/// `<root>\downloads\<tag>\` — the offline edition's extracted there; the
/// folder verified into a staging directory under the root; then laid out, or
/// the installation upgraded. On success the next page follows by itself:
/// configure on a fresh install, finish on an upgrade. An upgrade to a release
/// published earlier than the one installed waits for the person's word.
fn begin_install(window: &SetupWindow, state: &Shared) {
    let local_bundle = window.get_local_bundle().trim().to_string();
    let (mut fetch_release, mut carried) = (None, None);
    let upgrading = lock(state).installed.is_some();
    let copy = lock(state).payload.as_ref().map(Payload::try_clone);
    let download = if let Some(copy) = copy {
        let payload = match copy {
            Ok(payload) => payload,
            Err(reason) => {
                fail(state, &window.as_weak(), reason);
                return;
            }
        };
        if upgrading && downgrade_stops(window, state, &payload.members_text) {
            return;
        }
        let folder = lock(state).root.join("downloads").join(&payload.tag);
        lock(state).source = format!("自带发布件 / carried release {}", payload.tag);
        carried = Some(payload);
        folder
    } else if window.get_offline() {
        let download = PathBuf::from(window.get_download_dir().as_str());
        if !download.is_dir() {
            window.set_note(
                format!(
                    "发布件文件夹不存在或不是目录 / The download folder does not exist or is not a directory: {}",
                    download.display()
                )
                .into(),
            );
            return;
        }
        let installed = lock(state).installed.clone();
        let mut line = format!("发布件文件夹 / download folder: {}", download.display());
        if let Some(installed) = &installed {
            let members = download.join("MEMBERS.json");
            let text = std::fs::read_to_string(&members)
                .map_err(|error| format!("读取失败 / read failed: {}: {error}", members.display()));
            let wanted = text.as_deref().map_err(Clone::clone).and_then(verify::members_of);
            match wanted {
                Ok(wanted) => {
                    if downgrade_stops(window, state, text.as_deref().unwrap_or_default()) {
                        return;
                    }
                    line.push_str(" · ");
                    line.push_str(&upgrade_line(installed, &wanted));
                }
                Err(reason) => {
                    window.set_note(reason.into());
                    return;
                }
            }
        }
        if let Err(reason) = write_log(state, &line) {
            fail(state, &window.as_weak(), reason);
            return;
        }
        lock(state).source = format!("离线文件夹 / offline folder {}", download.display());
        download
    } else {
        let (root, release, members) = {
            let state = lock(state);
            (state.root.clone(), state.release.clone(), state.release_members.clone())
        };
        let Some(release) = release else {
            window.set_note("还没有找到可安装的发布件 / No release to install has been found".into());
            return;
        };
        if upgrading && downgrade_stops(window, state, members.as_deref().unwrap_or_default()) {
            return;
        }
        let folder = match release.folder() {
            Ok(folder) => root.join("downloads").join(folder),
            Err(reason) => {
                window.set_note(reason.into());
                return;
            }
        };
        lock(state).source = release.tag_name.clone();
        fetch_release = Some(release);
        folder
    };
    window.set_note("".into());
    window.set_installing(true);
    window.set_busy(true);
    window.set_can_next(false);
    lock(state).download = Some(download.clone());
    let root = lock(state).root.clone();
    let staging = root.join(format!(".staging-{}", log::unix_ms()));
    let existed = LAID.map(|name| root.join(name).exists());
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-install".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let mut report = PageReport::new(&state, &worker_weak);
        // An interface refusal changed nothing: its own text says so (Workflow #364).
        let refused = std::cell::Cell::new(false);
        let check = |result: Result<interfaces::Agreed, interfaces::Stop>| {
            result.map_err(|stop| {
                refused.set(matches!(stop, interfaces::Stop::Refused(_)));
                String::from(stop)
            })
        };
        let mut carried = carried;
        let fetched = match (&fetch_release, carried.as_mut()) {
            (Some(release), _) => fetch::fetch(release, &download, &mut report),
            (None, Some(payload)) => payload.extract(&download, &mut report),
            (None, None) => Ok(()),
        };
        let outcome = fetched
            .and_then(|()| verify::run(&download, &staging, &mut report))
            .and_then(|verified| {
                // From here on, stopping midway would leave files half laid out.
                let _held = Held::new(&state);
                let members = verified.members.clone();
                let done = match upgrading {
                    true => {
                        let (mut bundles, problems) = bundle::carried(&download, &mut report)?;
                        if !problems.is_empty() {
                            return Err(format!("发布件资源读取失败 / Release resources could not be read:\n{}", problems.join("\n")));
                        }
                        if !local_bundle.is_empty() { bundles.push(bundle::local(&local_bundle)?); }
                        // The release, the root and this installer must fit before any question
                        // (Workflow #364, review L4).
                        let agreed = check(interfaces::release(&root, &verified, &bundles, &mut report))?;
                        let mut choose = |association: &maintenance::Association| {
                            let labels = association.options.iter().map(|option| option.label.clone()).collect();
                            ask(&state, &worker_weak, association.question.clone(), labels).map(Some)
                        };
                        let mut resolve = |conflicts: &[maintenance::Conflict]| resolve_page(&state, &worker_weak, conflicts);
                        upgrade::upgrade(&root, &verified, &bundles, &agreed, &mut choose, &mut resolve, &mut report)
                            .map(|upgraded| (upgraded.laid_out.clone(), Some(upgraded), members, None))
                    }
                    // A fresh install is configured at once: the state root
                    // under the root, the salt, the console's settings.
                    false => check(interfaces::release(&root, &verified, &[], &mut report))
                        .and_then(|_| install::fresh(&root, &verified, &mut report))
                        .map(|(laid_out, configured)| (laid_out, None, members, Some(configured))),
                }?;
                // What the next upgrade's downgrade check reads; failing to
                // keep it is said, and the installation stands.
                if let Err(reason) = upgrade::record_members(&root, &download) {
                    report.warn(&reason)?;
                }
                Ok(done)
            });
        // The run has ended: its staging goes, whatever the outcome (Workflow #364 Q5).
        if let Err(reason) = remove_staging(&staging, &mut report) {
            fail(&state, &worker_weak, reason);
            return;
        }
        match outcome {
            Ok((laid_out, upgraded, members, configured)) => {
                let mut locked = lock(&state);
                locked.laid_out = Some(laid_out);
                locked.upgraded = upgraded;
                locked.members = Some(members);
                locked.configured = configured;
                drop(locked);
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    window.set_busy(false);
                    match upgrading {
                        true => finish(&window, &state),
                        false => enter_options(&window, &state),
                    }
                });
            }
            Err(reason) if refused.get() => fail(&state, &worker_weak, reason),
            Err(reason) => fail(&state, &worker_weak, left_behind(reason, &root, &staging, upgrading, existed)),
        }
    });
    // Nothing was fetched or verified, and staging was never made: the step
    // fails, said so.
    if let Err(error) = spawned {
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// The directories an install lays out under the root.
const LAID: [&str; 3] = ["runtime", "ui", "tools"];

/// Preserve failed preparation and report the selected installation's recovery boundary.
fn left_behind(mut reason: String, root: &Path, staging: &Path, _upgrading: bool, _existed: [bool; 3]) -> String {
    // The run removed its staging when it ended (`remove_staging`); say what came of it.
    let staging = match staging.try_exists() {
        Ok(false) => format!("本次的临时目录已删除 / this run's staging was removed: {}", staging.display()),
        _ => format!(
            "本次的临时目录未能删除，下次运行开始时删除并记入日志 / this run's staging was not removed; the next run removes it, and logs it, when it starts: {}",
            staging.display()
        ),
    };
    reason.push_str(&format!(
        "\n安装材料与备份保留 / Installation materials and backups retained: {}; {staging}. 当前选择以 install/active.json 为准，启动结果未知时先核实际 owner / Consult the active selection and actual owner before recovery",
        root.display()
    ));
    reason
}

/// A worker the system refused to start, as the wizard says it.
fn thread_failed(error: &std::io::Error) -> String {
    format!("无法启动工作线程 / The worker thread could not start: {error}")
}

/// The finish page: from the completed upgrade transaction or the instances
/// step on a fresh install.
fn finish(window: &SetupWindow, state: &Shared) {
    window.set_summary(summary(state).into());
    window.set_note("".into());
    window.set_step(4);
    window.set_can_next(false);
}

/// The options page of a fresh install, its configuration already written:
/// autostart and the console's shortcuts are the person's to choose.
fn enter_options(window: &SetupWindow, _state: &Shared) {
    window.set_note("".into());
    window.set_step(2);
    window.set_can_next(true);
}

/// Step 2 → 3: the Startup launcher and the shortcuts asked for, written off
/// the event loop; the instances page follows and looks for the emulator at
/// once. A failure is said on the page, which can be used again.
fn apply_options(window: &SetupWindow, state: &Shared) {
    let (root, laid_out) = {
        let state = lock(state);
        (state.root.clone(), state.laid_out.clone())
    };
    let Some(laid_out) = laid_out else {
        fail(state, &window.as_weak(), "尚未铺开 / nothing laid out".into());
        return;
    };
    let (autostart, with_console) = (window.get_autostart(), window.get_autostart_console());
    let (start_menu, desktop) = (window.get_start_menu(), window.get_desktop_shortcut());
    let earlier = lock(state).options_written.clone();
    window.set_note("".into());
    window.set_busy(true);
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-options".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let held = Held::new(&state);
        let mut report = PageReport::new(&state, &worker_weak);
        let mut written = Vec::new();
        // A retry starts from what was there before this run: whatever an
        // earlier attempt wrote goes first, then the options as they are now.
        let outcome = report
            .step(Step::Phase("写入选项 / Writing the options", None))
            .and_then(|()| take_back(&earlier, &mut report))
            .and_then(|()| install::autostart(&root, autostart, with_console, &mut written, &mut report))
            .and_then(|outcome| {
                install::shortcuts(&laid_out, start_menu, desktop, &mut written, &mut report).map(|links| (outcome, links))
            });
        {
            let mut locked = lock(&state);
            locked.options_written = earlier.into_iter().filter(|path| path.exists()).chain(written).collect();
        }
        let written = outcome;
        drop(held);
        match written {
            Ok((outcome, links)) => {
                {
                    let mut locked = lock(&state);
                    locked.autostart = Some(outcome);
                    locked.shortcuts = links;
                }
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    window.set_busy(false);
                    window.set_note("".into());
                    window.set_step(3);
                    window.set_can_next(true);
                    // The emulator is looked for at once: nothing to press first.
                    begin_discover(&window, &state);
                });
            }
            Err(reason) => retryable(&state, &worker_weak, reason),
        }
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// What an earlier options attempt of this run wrote, removed; one that
/// cannot be is the reason to stop.
fn take_back(paths: &[PathBuf], report: Report<'_>) -> Result<(), String> {
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => report.line(&format!("撤回本次早先写出的 / taken back what an earlier attempt wrote: {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "本次早先写出的未能删除 / cannot remove what an earlier attempt wrote: {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

/// The instances step's discovery, run on entering it and again on request:
/// the MuMu folder given, or found by the Runtime's `check-config`, pinned; a
/// Runtime running; and the instances it lists shown for picking — ticked when
/// there is just one. A failure is said on the page and in the log; the person
/// may name the folder, try again, or skip.
fn begin_discover(window: &SetupWindow, state: &Shared) {
    let text = window.get_mumu_root().trim().to_string();
    let mumu = (!text.is_empty()).then(|| PathBuf::from(&text));
    if mumu.as_ref().is_some_and(|mumu| !mumu.is_absolute() || !mumu.is_dir()) {
        window.set_note(
            "MuMu 安装目录必须是存在的文件夹的绝对路径 / The MuMu folder must be the absolute path of an existing folder".into(),
        );
        return;
    }
    // What an earlier discovery listed may not hold for this folder.
    window.set_instances(Rc::new(VecModel::<InstanceRow>::default()).into());
    window.set_discovered(false);
    window.set_note("".into());
    window.set_busy(true);
    let root = {
        let mut state = lock(state);
        state.discover_tried = true;
        state.root.clone()
    };
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-discover".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let held = Held::new(&state);
        let mut report = PageReport::new(&state, &worker_weak);
        let started = instance_step::start(&root, mumu.as_deref(), &mut report);
        let pinned = started.as_ref().ok().cloned().flatten();
        let notes = {
            let mut locked = lock(&state);
            locked.runtime_started |= started.is_ok();
            if pinned.is_some() {
                locked.warnings.retain(|line| line != instance_step::NOT_PINNED);
            }
            locked.warnings.join("\n")
        };
        {
            let pinned = pinned.clone();
            let _ = worker_weak.upgrade_in_event_loop(move |window| {
                window.set_notes(notes.into());
                if let Some(pinned) = pinned {
                    window.set_mumu_root(pinned.into());
                }
            });
        }
        let found = started.and_then(|pinned| instance_step::discover(&root, &mut report).map(|found| (pinned, found)));
        drop(held);
        // The release's bundles are read once, on the first discovery; a
        // failure is said loudly and leaves the local-file way open.
        let unread = {
            let mut locked = lock(&state);
            let unread = !locked.bundles_read;
            locked.bundles_read = true;
            unread.then(|| locked.download.clone()).flatten()
        };
        if let Some(download) = unread {
            let problems = match bundle::carried(&download, &mut report) {
                Ok((carried, problems)) => {
                    // Appended: the choices already offered keep their places.
                    lock(&state).bundles.extend(carried);
                    problems
                }
                Err(reason) => vec![reason],
            };
            for problem in &problems {
                let line = format!("发布件自带的标准包读不出 / a bundle the release carries cannot be read: {problem}");
                if let Err(error) = report.warn(&line) {
                    fail(&state, &worker_weak, error);
                    return;
                }
            }
            lock(&state).bundle_problems = problems;
        }
        let notes = lock(&state).warnings.join("\n");
        match found {
            Ok((pinned, found)) => {
                let mut rows: Vec<InstanceRow> = found.iter().map(instance_row).collect();
                if let [only] = rows.as_mut_slice() {
                    only.picked = true;
                }
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    if let Some(pinned) = pinned {
                        window.set_mumu_root(pinned.into());
                    }
                    window.set_instances(Rc::new(VecModel::from(rows)).into());
                    window.set_discovered(true);
                    window.set_notes(notes.into());
                    show_offer(&window, &state);
                    window.set_note("".into());
                    window.set_busy(false);
                });
            }
            Err(reason) => {
                let offered = Arc::clone(&state);
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    window.set_notes(notes.into());
                    show_offer(&window, &offered);
                });
                retryable(&state, &worker_weak, reason)
            }
        }
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

fn instance_row(found: &Found) -> InstanceRow {
    let mut detail = vec![found.adb.clone().unwrap_or_else(|| "adb —".to_string())];
    detail.push(if found.running { "运行中 / running" } else { "未运行 / not running" }.to_string());
    detail.extend(found.android.as_ref().map(|android| format!("Android {android}")));
    detail.extend(found.bound_alias.as_ref().map(|alias| format!("已绑定 / bound: {alias}")));
    InstanceRow {
        picked: false,
        index: i32::from(found.index),
        title: format!("#{} {}", found.index, found.name).into(),
        detail: detail.join(" · ").into(),
        alias: found.bound_alias.clone().unwrap_or_else(|| format!("mumu-{}", found.index)).into(),
        choice: 0,
    }
}

/// What an instance may be given: one server of one bundle, the package name
/// its game runs under there and that server's default pack, with the sha256
/// `bundle.json` gives it — no hash is asked of the person, but each is logged.
#[derive(Clone)]
struct Choice {
    bundle: usize,
    server: String,
    application_id: String,
    pack: String,
    package_id: String,
    sha256: String,
    label: String,
}

/// What the bundles offer the instances — each server that names both a
/// package name and a default pack — and, for the page, the programs and
/// package names they support, what is not offered and why, and whether any of
/// the release's bundles could not be read.
fn offer(bundles: &[Bundle], problems: &[String]) -> (Vec<Choice>, String) {
    let mut choices = Vec::new();
    let mut lines = Vec::new();
    for (at, bundle) in bundles.iter().enumerate() {
        let mut servers = Vec::new();
        for server in bundle.defaults.keys().filter(|server| !bundle.applications.contains_key(*server)) {
            servers.push(format!("{server}（applications.json 没有它的包名，不可选 / no package name in applications.json, not offered）"));
        }
        for (server, application) in &bundle.applications {
            let pack = bundle.defaults.get(server);
            servers.push(format!(
                "{} {}{}",
                application.label,
                application.application_id,
                if pack.is_some() { "" } else { "（没有默认任务包，不可选 / no default pack, not offered）" }
            ));
            // `bundle::read` holds every default pack to be listed.
            if let Some(listed) = pack.and_then(|pack| bundle.packs.iter().find(|listed| &listed.path == pack)) {
                choices.push(Choice {
                    bundle: at,
                    server: server.clone(),
                    application_id: application.application_id.clone(),
                    pack: listed.path.clone(),
                    package_id: listed.package_id.clone(),
                    sha256: listed.sha256.clone(),
                    label: format!("{} · {} · {}", bundle.name(), application.label, application.application_id),
                });
            }
        }
        lines.push(format!(
            "{}：{}（{}）",
            bundle.name(),
            servers.join("；"),
            if bundle.carried { "发布件自带 / carried by the release" } else { "本机文件 / local file" }
        ));
    }
    let unread = "发布件自带的标准包有读不出的（见注意）/ Some bundles the release carries cannot be read (see the notes)";
    let text = match (lines.is_empty(), problems.is_empty()) {
        (true, true) => "发布件没有带标准包：可在下面加入本机的标准包文件 / The release carries no resource bundle: add a local bundle file below".to_string(),
        (true, false) => format!("{unread}；可在下面加入本机的标准包文件 / add a local bundle file below"),
        (false, true) => format!("支持的程序与包名 / Supported programs and package names:\n{}", lines.join("\n")),
        (false, false) => format!("支持的程序与包名 / Supported programs and package names:\n{}\n{unread}", lines.join("\n")),
    };
    (choices, text)
}

/// What the page's first choice says: an instance is on it until someone
/// chooses. A ComboBox clamps its index into its model, so "nothing chosen"
/// cannot be an index outside it — it is this entry, at 0.
const CHOOSE: &str = "请选择它运行的程序 / Choose what it runs";

/// The page's account of the bundles — what they support, and the choices
/// each ticked instance picks from, made for it when there is only one.
fn show_offer(window: &SetupWindow, state: &Shared) {
    let (choices, text) = {
        let locked = lock(state);
        offer(&locked.bundles, &locked.bundle_problems)
    };
    let single = choices.len() == 1;
    // The page's one choices model only grows, and its first entry is
    // CHOOSE: bundles are only ever appended, so an index already chosen
    // keeps meaning what it meant.
    let labels = || {
        std::iter::once(slint::SharedString::from(CHOOSE))
            .chain(choices.iter().map(|choice| choice.label.clone().into()))
            .collect::<Vec<_>>()
    };
    let model = window.get_choices();
    match model.as_any().downcast_ref::<VecModel<slint::SharedString>>() {
        Some(shown) => {
            for label in labels().into_iter().skip(shown.row_count()) {
                shown.push(label);
            }
        }
        None => window.set_choices(Rc::new(VecModel::from(labels())).into()),
    }
    lock(state).choices = choices;
    window.set_supported_text(text.into());
    // One choice and nothing to weigh: it is made for every instance.
    if single {
        let rows = window.get_instances();
        for at in 0..rows.row_count() {
            if let Some(mut row) = rows.row_data(at).filter(|row| row.choice < 1) {
                row.choice = 1;
                rows.set_row_data(at, row);
            }
        }
    }
}

/// A local bundle file added to what the instances step offers, read off the
/// event loop — for a release that carries none. One for a game already
/// offered is refused; no hash is asked.
fn begin_add_bundle(window: &SetupWindow, state: &Shared) {
    let given = window.get_local_bundle().trim().to_string();
    if given.is_empty() {
        window.set_note("先填本机标准包文件的路径 / Name a local bundle file first".into());
        return;
    }
    window.set_note("".into());
    window.set_busy(true);
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-bundle".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let mut report = PageReport::new(&state, &worker_weak);
        let added = bundle::local(&given).and_then(|bundle| {
            let line = format!("本机标准包 / local bundle: {} → {}", bundle.file.display(), bundle.name());
            {
                let mut locked = lock(&state);
                // `packages\<game>\` is one folder whatever the case.
                if let Some(twin) = locked.bundles.iter().find(|other| other.game.eq_ignore_ascii_case(&bundle.game)) {
                    return Err(format!(
                        "已有 {} 的标准包 / a bundle for {} is already offered: {}",
                        twin.name(),
                        twin.game,
                        twin.file.display()
                    ));
                }
                locked.bundles.push(bundle);
            }
            report.line(&line)
        });
        match added {
            Ok(()) => {
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    show_offer(&window, &state);
                    window.set_local_bundle("".into());
                    window.set_note("".into());
                    window.set_busy(false);
                });
            }
            Err(reason) => retryable(&state, &worker_weak, reason),
        }
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// An instances-step failure the page can be used again after: logged, and said on the
/// page. Any log write that failed, then or before, stops the run instead.
fn retryable(state: &Shared, weak: &slint::Weak<SetupWindow>, reason: String) {
    let broken = lock(state).log_error.clone();
    let logged = match broken {
        Some(error) => Err(error),
        None => write_log(state, &format!("未完成 / not done: {reason}")),
    };
    if let Err(error) = logged {
        let text = if reason.contains(&error) { reason } else { format!("{reason}\n{error}") };
        fail(state, weak, text);
        return;
    }
    let _ = weak.upgrade_in_event_loop(move |window| {
        window.set_busy(false);
        window.set_note(reason.into());
    });
}

/// Step 3 → 4: the picked instances written — each with its alias and what
/// its choice gives, the package name and the resource package — then the
/// Runtime restarted on them. Each bundle used is laid out once, first. A failure
/// before the configuration is replaced leaves the page usable; one after it
/// stops the run.
fn begin_apply(window: &SetupWindow, state: &Shared) {
    if !lock(state).bundle_problems.is_empty() {
        window.set_note("发布件标准包读取失败，配置计划不能提交；请使用完整有效的发布件 / A release bundle failed to read; the configuration plan cannot be committed. Use a complete valid release".into());
        return;
    }
    let picked: Vec<InstanceRow> = window
        .get_instances()
        .iter()
        .filter(|row| row.picked)
        .collect();
    if picked.is_empty() {
        window.set_note(
            "没有勾选实例：不配置实例就点「跳过」/ No instance is ticked: Skip to configure none"
                .into(),
        );
        return;
    }
    if let Some(row) = picked.iter().find(|row| row.alias.trim().is_empty()) {
        window.set_note(format!("{}：别名未填 / the alias is empty", row.title).into());
        return;
    }
    // What each ticked instance runs, and the resource package it gets.
    let (choices, bundles) = {
        let locked = lock(state);
        (locked.choices.clone(), locked.bundles.clone())
    };
    let mut picks = Vec::new();
    for row in &picked {
        // Index 0 is CHOOSE: nothing has been chosen for this instance.
        let Some(choice) = usize::try_from(row.choice - 1)
            .ok()
            .and_then(|at| choices.get(at))
        else {
            window
                .set_note(format!("{}：请选它运行的程序 / choose what it runs", row.title).into());
            return;
        };
        picks.push((
            u16::try_from(row.index).unwrap_or_default(),
            row.alias.trim().to_string(),
            choice.clone(),
        ));
    }
    window.set_note("".into());
    window.set_busy(true);
    let root = lock(state).root.clone();
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-instances".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let held = Held::new(&state);
        let mut report = PageReport::new(&state, &worker_weak);
        // Stage and qualify before touching the installed material. The selected
        // bundle indices remain stable for the plan and the conflict page.
        let staging = root.join(format!(".staging-resources-{}", log::unix_ms()));
        // Every bundle against the selected slot's Runtime and this installer before anything
        // is staged (Workflow #364, review M2); a refusal leaves the page usable.
        let prepared = interfaces::selected(&root, &bundles, &mut report)
            .map_err(String::from)
            .and_then(|_| maintenance::prepare(&bundles, &staging, &mut report));
        let mut restoration_failed = false;
        let written = prepared.and_then(|prepared| {
            let chosen =
            picks
                .iter()
                .map(|(index, alias, choice)| {
                    let package = maintenance::installed_path(&root, &prepared[choice.bundle], &choice.pack)?;
                    report.line(&format!(
                        "实例 / instance {alias}：{} → {} {}（sha256 {}）",
                        choice.label,
                        choice.package_id,
                        package.display(),
                        choice.sha256
                    ))?;
                    Ok(Chosen {
                        index: *index,
                        alias: alias.clone(),
                        application_id: choice.application_id.clone(),
                        resource_package: package,
                    })
                })
                .collect::<Result<Vec<Chosen>, String>>()?;
            let mut transaction = instance_step::plan(&root, &chosen)?;
            let selections: Vec<_> = picks.iter().enumerate().map(|(at, (_, _, choice))| maintenance::Selection {
                instance: at, bundle: choice.bundle, server: choice.server.clone(),
            }).collect();
            let old_source = transaction.config_path().display().to_string();
            let (qualify, conflicts) = maintenance::augment(&mut transaction.document, &root, &prepared, &selections, &old_source)?;
            let mut resolve = |conflicts: &[maintenance::Conflict]| resolve_page(&state, &worker_weak, conflicts);
            maintenance::decide(&mut transaction.document, &conflicts, &mut resolve, &mut report)?;
            let planned_state_root = transaction.state_root()?;
            transaction.unchanged()?;
            let closed = instance_step::stop(&root, &planned_state_root, &mut report)?;
            let committed = maintenance::place(&prepared, &root, bundle::OnDiffers::SetAside, &mut report)
                .and_then(|()| transaction.commit(&root.join("runtime").join(runtime::ACTINGD), qualify, &mut report));
            if let Err(reason) = committed {
                return Err(match transaction.restore() {
                    Ok(()) => format!("{reason}\n原配置保留；Runtime 保持停止；已放置资源保留 / Original configuration retained; Runtime remains stopped; placed resources retained"),
                    Err(restore) => {
                        restoration_failed = true;
                        format!("{reason}\n{restore}\nRuntime 保持停止 / Runtime remains stopped")
                    }
                });
            }
            Ok((chosen, closed))
        });
        // The packs are placed, or the plan stopped: staging has served (Workflow #364 Q5).
        if let Err(reason) = remove_staging(&staging, &mut report) {
            drop(held);
            fail(&state, &worker_weak, reason);
            return;
        }
        let (chosen, closed) = match written {
            Ok(done) => done,
            Err(reason) => {
                drop(held);
                if restoration_failed { fail(&state, &worker_weak, reason); }
                else { retryable(&state, &worker_weak, reason); }
                return;
            }
        };
        {
            let mut locked = lock(&state);
            locked.instances = chosen.iter().map(|pick| pick.alias.clone()).collect();
            locked.assigned = chosen
                .iter()
                .zip(&picks)
                .map(|(pick, (_, _, choice))| format!("{}：{} → {}", pick.alias, choice.label, pick.resource_package.display()))
                .collect();
        }
        let restarted = instance_step::restart(&root, closed, &mut report)
            .and_then(|status| report.line(&format!("actingctl status: {status}")));
        lock(&state).runtime_started |= restarted.is_ok();
        drop(held);
        match restarted {
            Ok(()) => {
                let _ = worker_weak.upgrade_in_event_loop(move |window| settle_and_finish(&window, &state));
            }
            Err(reason) => fail(
                &state,
                &worker_weak,
                format!(
                    "{reason}\n实例已写入配置，但未确认 Runtime 已按新配置运行 / The instances are in the configuration, but it is not confirmed that the Runtime runs on it"
                ),
            ),
        }
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// Step 3 → 4. When the instances step touched the configuration and the Runtime, the
/// configured MuMu folder and whether a Runtime answers are asked now, so the
/// summary says how they stand rather than how they were meant to.
fn settle_and_finish(window: &SetupWindow, state: &Shared) {
    if !lock(state).discover_tried {
        finish(window, state);
        return;
    }
    window.set_busy(true);
    let root = lock(state).root.clone();
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-settle".into()).spawn(move || {
        let _guard = PanicGuard::new(&state, &worker_weak);
        let mut report = PageReport::new(&state, &worker_weak);
        let settled = instance_step::settle(&root, &mut report);
        let broken = lock(&state).log_error.clone();
        match (settled, broken) {
            (Err(reason), _) | (Ok(_), Some(reason)) => fail(&state, &worker_weak, reason),
            (Ok(settled), None) => {
                let mut locked = lock(&state);
                // An earlier "runtime-info.json but no answer" no longer holds.
                if matches!(settled.running, Ok(true)) {
                    locked.warnings.retain(|line| !line.contains("runtime-info.json"));
                }
                locked.settled = Some(settled);
                drop(locked);
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    window.set_busy(false);
                    finish(&window, &state);
                });
            }
        }
    });
    if let Err(error) = spawned {
        window.set_busy(false);
        fail(&shared, &weak, thread_failed(&error));
    }
}

fn summary(state: &Shared) -> String {
    let state = lock(state);
    let mut lines = vec![format!("安装根 / Install root: {}", state.root.display())];
    // A resource-only update (Workflow #364, model §2.2.5).
    if let Some(outcome) = &state.resources {
        lines.extend(resources::summary(outcome));
        if let Some(log) = &state.log {
            lines.push(format!("日志 / Log: {}", log.path().display()));
        }
        if !state.warnings.is_empty() {
            lines.push(String::new());
            lines.push("注意 / Note:".to_string());
            lines.extend(state.warnings.iter().cloned());
        }
        return lines.join("\n");
    }
    if let Some((runtime, ui)) = &state.members {
        lines.push(format!(
            "已安装 / Installed: runtime {} · ui {}（{}）",
            short(runtime),
            short(ui),
            state.source
        ));
    }
    if let (Some(installed), Some(upgraded)) = (&state.installed, &state.upgraded) {
        if let Some(notice) = &upgraded.notice {
            lines.push(format!("！！ 注意 / ATTENTION: {notice}"));
        }
        lines.push(format!(
            "已升级 / Upgraded from runtime {} · ui {}",
            short(&installed.runtime_sha),
            short(&installed.ui_sha)
        ));
        lines.push(format!(
            "被替换的版本 / Version replaced, kept in: {}",
            upgraded.previous.display()
        ));
        lines.push(format!(
            "选中配置代际 / Selected configuration generation: {}",
            upgraded.generation
        ));
        lines.push(match &upgraded.restarted {
            Some(log) => format!(
                "Runtime 已用新版本重新拉起 / restarted on the new version; 日志 / log: {}",
                log.display()
            ),
            None => "Runtime 升级前未在运行，未拉起 / was not running, not started".to_string(),
        });
        lines.push("私有配置按所选计划生成，原代际保留 / Private configuration generated from the selected plan; prior generations retained".into());
        lines.push("状态根、监控台设置与开机自启保留 / State root, console settings and autostart preserved".to_string());
    }
    if let Some(laid_out) = &state.laid_out {
        lines.push(format!("Runtime: {}", laid_out.actingd_exe.display()));
        lines.push(format!("监控台 / Console: {}", laid_out.acui_exe.display()));
        lines.push(format!(
            "工具 / Tools: {}（{}、{}）",
            laid_out.tools_dir.display(),
            verify::TOOLS_INSTALLED
                .iter()
                .filter(|name| !name.contains('/'))
                .copied()
                .collect::<Vec<_>>()
                .join("、"),
            platform_tools(&laid_out.tools_dir)
        ));
    }
    if let Some(configured) = &state.configured {
        lines.push(format!(
            "Runtime 配置 / Config: {}",
            configured.config_path.display()
        ));
        lines.push(format!(
            "状态根 / State root: {}",
            configured.state_root.display()
        ));
        lines.push(format!(
            "监控台设置 / Console settings: {}",
            configured.settings_path.display()
        ));
    }
    let fresh = state.upgraded.is_none();
    lines.extend(fresh.then(|| match &state.autostart {
        Some(Autostart::Written {
            path,
            with_console: true,
        }) => format!(
            "开机自启 / Autostart: {}（含监控台 / with the console）",
            path.display()
        ),
        Some(Autostart::Written {
            path,
            with_console: false,
        }) => format!("开机自启 / Autostart: {}", path.display()),
        Some(Autostart::NotWanted {
            existing: Some(path),
        }) => format!(
            "开机自启 / Autostart: 未启用；已有的 {} 未改动 / not enabled; the existing launcher was left as is",
            path.display()
        ),
        Some(Autostart::NotWanted { existing: None }) | None => {
            "开机自启 / Autostart: 未启用 / not enabled".to_string()
        }
    }));
    if fresh && state.autostart.is_some() {
        lines.push(match state.shortcuts.as_slice() {
            [] => "快捷方式 / Shortcuts: 未创建 / none".to_string(),
            links => format!(
                "快捷方式 / Shortcuts: {}",
                links
                    .iter()
                    .map(|link| link.display().to_string())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        });
    }
    if fresh && !state.instances.is_empty() {
        lines.push(
            "实例 / Instances（已写入配置并通过检查 / in the configuration, checked）:".to_string(),
        );
        lines.extend(state.assigned.iter().map(|line| format!("  {line}")));
    }
    if let Some(settled) = &state.settled {
        lines.extend(
            settled
                .mumu_root
                .as_ref()
                .map(|mumu| format!("MuMu 目录 / MuMu folder (mumu_root): {mumu}")),
        );
        lines.push(match &settled.running {
            Ok(true) => "Runtime 在运行（实例步拉起）/ The Runtime is running (started in the instances step)".to_string(),
            Ok(false) => "Runtime 未在运行 / The Runtime is not running".to_string(),
            Err(reason) => format!("Runtime 是否在运行未能确认 / Whether the Runtime runs is not confirmed: {reason}"),
        });
    }
    if let Some(log) = &state.log {
        lines.push(format!("日志 / Log: {}", log.path().display()));
    }
    if !state.warnings.is_empty() {
        lines.push(String::new());
        lines.push("注意 / Note:".to_string());
        lines.extend(state.warnings.iter().cloned());
    }
    if fresh && state.instances.is_empty() {
        lines.push(String::new());
        lines.push(
            "实例（模拟器 / 设备）未配置，instances 为空：之后点监控台顶栏的「实例配置」按钮添加。"
                .to_string(),
        );
        lines.push(
            "No instance was configured (instances is empty): add them with the Instance Configuration button in the console's top bar."
                .to_string(),
        );
    }
    lines.join("\n")
}

/// `platform-tools` with the revision its own `source.properties` names, as
/// laid out (a file the tools manifest binds, so never guessed).
fn platform_tools(tools_dir: &Path) -> String {
    let properties = tools_dir.join(verify::PLATFORM_TOOLS).join("source.properties");
    let revision = std::fs::read_to_string(properties).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix("Pkg.Revision=").map(|value| value.trim().to_string()))
    });
    match revision {
        Some(revision) => format!("{}（adb {revision}）", verify::PLATFORM_TOOLS),
        None => verify::PLATFORM_TOOLS.to_string(),
    }
}

/// The finish page: the console, detached, and the wizard closed.
fn open_console(window: &SetupWindow, state: &Shared) {
    let console = lock(state)
        .laid_out
        .as_ref()
        .map(|laid_out| (laid_out.acui_exe.clone(), laid_out.ui_dir.clone()));
    let Some((exe, dir)) = console else {
        window.set_note("监控台尚未铺开 / The console has not been laid out".into());
        return;
    };
    match install::open_console(&exe, &dir) {
        Ok(()) => match write_log(state, &format!("已拉起监控台 / console started: {}", exe.display())) {
            Ok(()) => {
                let _ = slint::quit_event_loop();
            }
            Err(unlogged) => window.set_note(format!("已拉起监控台 / console started\n{unlogged}").into()),
        },
        Err(reason) => {
            let text = match write_log(state, &reason) {
                Ok(()) => reason,
                Err(unlogged) => format!("{reason}\n{unlogged}"),
            };
            window.set_note(text.into());
        }
    }
}
