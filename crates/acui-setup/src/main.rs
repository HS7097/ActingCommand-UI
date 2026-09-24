// SPDX-License-Identifier: GPL-3.0-only
//! acsetup: the install wizard for ActingCommand on Windows. One window, six
//! pages — location, get, verify and lay out, configure, instances, finish. It
//! fetches the release to install from the umbrella repository's Releases, or
//! takes a folder a person filled by hand, checks it against what the umbrella
//! published, lays it out under a per-user install root, writes the Runtime
//! configuration and the console's settings, optionally a Startup-folder
//! launcher and the emulator instances (see `instance_step`), and can open the
//! console. On a root that already holds an installation it upgrades instead:
//! the payload replaced, state and configuration kept (see `upgrade`). Those
//! fetches — the release and a resource package given as a URL — are its only
//! network code; it registers no service and no scheduled task, and touches
//! neither PATH nor the registry.
//!
//! The wizard is Windows-only. Elsewhere it compiles, and the first platform
//! question is the loud stop, before any window is opened.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod fetch;
mod install;
mod instance_step;
mod log;
mod platform;
mod runtime;
mod upgrade;
mod verify;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use slint::ComponentHandle;

use fetch::Release;
use install::{Autostart, Configured, LaidOut};
use instance_step::{Chosen, Found, Settled};
use log::InstallLog;
use slint::{Model, VecModel};
use upgrade::{Installed, Upgraded};

slint::include_modules!();


/// What the steps so far established: one run, one root, one log.
#[derive(Default)]
struct State {
    root: PathBuf,
    download: PathBuf,
    log: Option<InstallLog>,
    /// The release the online path installs, once looked up.
    release: Option<Release>,
    /// What the root already holds: set, the run is an upgrade.
    installed: Option<Installed>,
    upgraded: Option<Upgraded>,
    laid_out: Option<LaidOut>,
    configured: Option<Configured>,
    autostart: Option<Autostart>,
    /// Step 4 touched the configuration and the Runtime: the finish page then
    /// asks how they stand.
    discover_tried: bool,
    settled: Option<Settled>,
    /// The aliases step 4 wrote into the configuration.
    instances: Vec<String>,
    /// The first log write that failed: nothing goes on after it.
    log_error: Option<String>,
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn main() -> Result<()> {
    let root = platform::default_install_root().map_err(anyhow::Error::msg)?;
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
    refresh_preflight(&window);
    install_callbacks(&window, &state);
    window.run()?;
    Ok(())
}

fn install_callbacks(window: &SetupWindow, state: &Shared) {
    {
        let weak = window.as_weak();
        window.on_install_root_edited(move |_| {
            if let Some(window) = weak.upgrade() {
                refresh_preflight(&window);
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
        window.on_open_console(move || {
            if let Some(window) = weak.upgrade() {
                open_console(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
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

/// The two lines under the install root: the room on its volume, and whether
/// a Runtime is already installed there.
fn refresh_preflight(window: &SetupWindow) {
    let root = PathBuf::from(window.get_install_root().as_str());
    let free = match platform::free_space(&root) {
        Ok(bytes) => format!(
            "该卷可用空间 / Free space on this volume: {:.1} GiB",
            bytes as f64 / (1u64 << 30) as f64
        ),
        Err(reason) => format!("可用空间未知 / Free space unknown: {reason}"),
    };
    window.set_free_space_text(free.into());
    let existing = match upgrade::installed(&root) {
        Ok(None) => "此处没有已安装的 Runtime / No installation here yet".to_string(),
        Ok(Some(installed)) => format!(
            "此处已有安装：runtime {} · ui {}；下一步将升级，保留 state、配置与设置 / Installed here: the next steps upgrade it, keeping state, configuration and settings",
            short(&installed.runtime_sha),
            short(&installed.ui_sha)
        ),
        Err(reason) => reason,
    };
    window.set_existing_text(existing.into());
}

fn next(window: &SetupWindow, state: &Shared) {
    match window.get_step() {
        0 => enter_get(window, state),
        1 => begin_get(window, state),
        2 if lock(state).upgraded.is_some() => finish(window, state),
        2 => enter_configure(window, state),
        3 => run_configure(window, state),
        4 => begin_apply(window, state),
        _ => {}
    }
}

/// A line for the log and the page. A log write failing fails the run: the
/// log is the record the person is told to read.
fn reporter(state: &Shared, weak: &slint::Weak<SetupWindow>) -> impl FnMut(&str) -> Result<(), String> {
    let state = Arc::clone(state);
    let weak = weak.clone();
    move |line: &str| {
        write_log(&state, line)?;
        let line = line.to_string();
        let _ = weak.upgrade_in_event_loop(move |window| {
            let mut progress = window.get_progress().to_string();
            progress.push_str(&line);
            progress.push('\n');
            window.set_progress(progress.into());
        });
        Ok(())
    }
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
        Ok(Some(_)) if running_from(&root) => Err(
            "本引导正从要升级的这份安装里运行：请把 acsetup.exe 复制到别处再运行 / This wizard runs from the installation it would upgrade: copy acsetup.exe elsewhere and run it from there"
                .to_string(),
        ),
        other => other,
    };
    let installed = match installed {
        Ok(installed) => installed,
        Err(text) => {
            window.set_note(text.into());
            return;
        }
    };
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
        "acsetup {} · 安装根 / install root: {}{}",
        env!("CARGO_PKG_VERSION"),
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
    if let Err(reason) = write_log(state, &started) {
        fail(state, &window.as_weak(), reason);
        return;
    }
    window.set_note("".into());
    window.set_progress("".into());
    window.set_step(1);
    offline_toggled(window, state, window.get_offline());
}

/// Step 1's choice: a folder is taken as it is; the online path needs a
/// release, looked up once.
fn offline_toggled(window: &SetupWindow, state: &Shared, offline: bool) {
    window.set_note("".into());
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
    window.set_busy(true);
    window.set_can_next(false);
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-release".into()).spawn(move || {
        let have = lock(&state)
            .installed
            .as_ref()
            .map(|installed| (installed.runtime_sha.clone(), installed.ui_sha.clone()));
        // An upgrade reads the release's MEMBERS.json first: the same two
        // commits need nothing fetched.
        let found = fetch::choose().and_then(|release| {
            release.folder()?;
            let wanted = have.as_ref().map(|_| fetch::members(&release)).transpose()?;
            Ok((release, wanted))
        });
        let (line, current) = match &found {
            Ok((release, wanted)) => {
                let mut line = release_line(release);
                if let (Some(have), Some(wanted)) = (&have, wanted) {
                    line.push('\n');
                    line.push_str(&upgrade_line(have, wanted));
                }
                (line, wanted.is_some() && wanted.as_ref() == have.as_ref())
            }
            Err(reason) => (reason.clone(), false),
        };
        if let Err(reason) = write_log(&state, &line) {
            fail(&state, &worker_weak, reason);
            return;
        }
        let found = found.ok().filter(|_| !current).map(|(release, _)| release);
        let ok = found.is_some();
        lock(&state).release = found;
        let _ = worker_weak.upgrade_in_event_loop(move |window| {
            window.set_busy(false);
            window.set_release_text(line.into());
            window.set_can_next(ok || window.get_offline());
            if current {
                window.set_note(UP_TO_DATE.into());
            } else if !ok {
                window.set_note(
                    "可勾选「离线」改用已下载的发布件文件夹；勾上再取消即重新查询 / Tick Offline to use a folder of downloaded release files; tick and untick it to look up again"
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

const UP_TO_DATE: &str = "已是这个发布件的版本，无需升级 / Already at this release's version: nothing to upgrade";

/// What an upgrade replaces with what.
fn upgrade_line(have: &(String, String), wanted: &(String, String)) -> String {
    format!(
        "升级 / Upgrade: runtime {} → {} · ui {} → {}",
        short(&have.0),
        short(&wanted.0),
        short(&have.1),
        short(&wanted.1)
    )
}

fn short(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}

/// Whether this wizard's own executable lies under `root`: its directory could
/// not move aside while it runs.
fn running_from(root: &Path) -> bool {
    let exe = std::env::current_exe().and_then(|exe| exe.canonicalize());
    match (exe, root.canonicalize()) {
        (Ok(exe), Ok(root)) => exe.starts_with(root),
        _ => false,
    }
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

/// Step 1 → 2: offline, the folder is checked; online, the release is fetched
/// into `<root>\downloads\<tag>\` first. Then step 2 starts.
fn begin_get(window: &SetupWindow, state: &Shared) {
    if window.get_offline() {
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
        let have = lock(state)
            .installed
            .as_ref()
            .map(|installed| (installed.runtime_sha.clone(), installed.ui_sha.clone()));
        let mut line = format!("发布件文件夹 / download folder: {}", download.display());
        if let Some(have) = &have {
            let members = download.join("MEMBERS.json");
            let wanted = std::fs::read_to_string(&members)
                .map_err(|error| format!("读取失败 / read failed: {}: {error}", members.display()))
                .and_then(|text| verify::members_of(&text));
            match wanted {
                Ok(wanted) if &wanted == have => {
                    window.set_note(UP_TO_DATE.into());
                    return;
                }
                Ok(wanted) => {
                    line.push_str(" · ");
                    line.push_str(&upgrade_line(have, &wanted));
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
        lock(state).download = download;
        begin_verify(window, state);
        return;
    }
    let (root, release) = {
        let state = lock(state);
        (state.root.clone(), state.release.clone())
    };
    let Some(release) = release else {
        window.set_note("还没有找到可安装的发布件 / No release to install has been found".into());
        return;
    };
    let folder = match release.folder() {
        Ok(folder) => root.join("downloads").join(folder),
        Err(reason) => {
            window.set_note(reason.into());
            return;
        }
    };
    window.set_note("".into());
    window.set_progress("".into());
    window.set_busy(true);
    window.set_can_next(false);
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-fetch".into()).spawn(move || {
        let mut report = reporter(&state, &worker_weak);
        match fetch::fetch(&release, &folder, &mut report) {
            Ok(()) => {
                lock(&state).download = folder;
                let _ = worker_weak.upgrade_in_event_loop(move |window| begin_verify(&window, &state));
            }
            Err(reason) => fail(&state, &worker_weak, reason),
        }
    });
    // Nothing was fetched: the step fails, said so.
    if let Err(error) = spawned {
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// A worker the system refused to start, as the wizard says it.
fn thread_failed(error: &std::io::Error) -> String {
    format!("无法启动工作线程 / The worker thread could not start: {error}")
}

/// Step 2: the folder verified into a staging directory under the root, then
/// laid out. Staging holds nothing that was not in the zips: on a failure it is
/// removed, and said so.
fn begin_verify(window: &SetupWindow, state: &Shared) {
    window.set_note("".into());
    window.set_progress("".into());
    window.set_step(2);
    window.set_busy(true);
    window.set_can_next(false);
    let (root, download) = {
        let state = lock(state);
        (state.root.clone(), state.download.clone())
    };
    let staging = root.join(format!(".staging-{}", log::unix_ms()));
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-verify".into()).spawn(move || {
        let (state, weak) = (state, worker_weak);
        let mut report = reporter(&state, &weak);
        let upgrading = lock(&state).installed.is_some();
        let outcome = verify::run(&download, &staging, &mut report).and_then(|verified| {
            match upgrading {
                true => upgrade::upgrade(&root, &verified, &mut report)
                    .map(|upgraded| (upgraded.laid_out.clone(), Some(upgraded))),
                false => install::lay_out(&root, &verified, &mut report).map(|laid_out| (laid_out, None)),
            }
        });
        match outcome {
            Ok((laid_out, upgraded)) => {
                let mut locked = lock(&state);
                locked.laid_out = Some(laid_out);
                locked.upgraded = upgraded;
                drop(locked);
                let _ = weak.upgrade_in_event_loop(|window| {
                    window.set_busy(false);
                    window.set_can_next(true);
                });
            }
            Err(reason) => {
                if staging.exists() {
                    let _ = report(&match std::fs::remove_dir_all(&staging) {
                        Ok(()) => format!("已删除临时目录 / staging removed: {}", staging.display()),
                        Err(error) => format!(
                            "临时目录未能删除 / staging not removed: {}: {error}",
                            staging.display()
                        ),
                    });
                }
                fail(&state, &weak, reason);
            }
        }
    });
    // Nothing was verified and staging was never made: the step fails, said so.
    if let Err(error) = spawned {
        fail(&shared, &weak, thread_failed(&error));
    }
}

/// The finish page: from step 2 on an upgrade — configuration, settings and
/// the Startup launcher as they were — or from step 4 on a fresh install.
fn finish(window: &SetupWindow, state: &Shared) {
    window.set_summary(summary(state).into());
    window.set_note("".into());
    window.set_step(5);
    window.set_can_next(false);
}

/// Step 2 → 3: the configure page, its state root defaulted once.
fn enter_configure(window: &SetupWindow, state: &Shared) {
    {
        let state = lock(state);
        if state.configured.is_none() {
            window.set_state_root(state.root.join("state").display().to_string().into());
        }
        window.set_configured(state.configured.is_some());
    }
    window.set_note("".into());
    window.set_step(3);
    window.set_can_next(true);
}

/// Step 3 → 4: the configuration and the console's settings, written once,
/// then the optional launcher; the instances page follows.
fn run_configure(window: &SetupWindow, state: &Shared) {
    if lock(state).configured.is_none() {
        let state_root = PathBuf::from(window.get_state_root().as_str());
        if let Err(reason) = install::state_root_usable(&state_root) {
            window.set_note(reason.into());
            return;
        }
        let (root, laid_out) = {
            let state = lock(state);
            (state.root.clone(), state.laid_out.clone())
        };
        let Some(laid_out) = laid_out else {
            fail(state, &window.as_weak(), "尚未铺开，无法配置 / nothing laid out to configure".into());
            return;
        };
        let mut report = |line: &str| write_log(state, line);
        match install::configure(&root, &state_root, &laid_out, &mut report) {
            Ok(configured) => {
                lock(state).configured = Some(configured);
                window.set_configured(true);
            }
            Err(reason) => {
                fail(state, &window.as_weak(), reason);
                return;
            }
        }
    }
    let root = lock(state).root.clone();
    let mut report = |line: &str| write_log(state, line);
    match install::autostart(
        &root,
        window.get_autostart(),
        window.get_autostart_console(),
        &mut report,
    ) {
        Ok(outcome) => {
            lock(state).autostart = Some(outcome);
            window.set_note("".into());
            window.set_progress("".into());
            window.set_step(4);
            window.set_can_next(true);
        }
        Err(reason) => fail(state, &window.as_weak(), reason),
    }
}

/// Step 4's Discover: the MuMu folder written when given, a Runtime running,
/// and the instances it lists shown for picking. A failure is said on the page
/// and in the log; the person may correct the folder, try again, or skip. An
/// empty folder field takes `mumu_root` out of the configuration.
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
    window.set_progress("".into());
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
        let mut report = reporter(&state, &worker_weak);
        let started = instance_step::start(&root, mumu.as_deref(), &mut report);
        match started.and_then(|()| instance_step::discover(&root, &mut report)) {
            Ok(found) => {
                let rows: Vec<InstanceRow> = found.iter().map(instance_row).collect();
                let _ = worker_weak.upgrade_in_event_loop(move |window| {
                    window.set_instances(Rc::new(VecModel::from(rows)).into());
                    window.set_discovered(true);
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
        application: "".into(),
        package: "".into(),
        sha256: "".into(),
    }
}

/// A step-4 failure the page can be used again after: logged, and said on the
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

/// Step 4 → 5: the picked instances written — each with an alias, an
/// application_id and a resource package — then the Runtime restarted on them.
/// A failure before the configuration is replaced leaves the page usable; one
/// after it stops the run.
fn begin_apply(window: &SetupWindow, state: &Shared) {
    let picked: Vec<InstanceRow> = window.get_instances().iter().filter(|row| row.picked).collect();
    if picked.is_empty() {
        window.set_note(
            "没有勾选实例：不配置实例就点「跳过」/ No instance is ticked: Skip to configure none".into(),
        );
        return;
    }
    let mut chosen = Vec::new();
    for row in &picked {
        let missing = [
            (row.alias.trim(), "别名 / alias"),
            (row.application.trim(), "application_id"),
            (row.package.trim(), "资源包 / resource package"),
        ]
        .into_iter()
        .find(|(value, _)| value.is_empty());
        if let Some((_, field)) = missing {
            window.set_note(format!("{}：{field} 未填 / {field} is empty", row.title).into());
            return;
        }
        chosen.push(Chosen {
            index: u16::try_from(row.index).unwrap_or_default(),
            alias: row.alias.trim().to_string(),
            application_id: row.application.trim().to_string(),
            package: row.package.trim().to_string(),
            sha256: row.sha256.trim().to_string(),
        });
    }
    window.set_note("".into());
    window.set_progress("".into());
    window.set_busy(true);
    let root = lock(state).root.clone();
    let weak = window.as_weak();
    let shared = Arc::clone(state);
    let (state, worker_weak) = (Arc::clone(state), weak.clone());
    let spawned = std::thread::Builder::new().name("acsetup-instances".into()).spawn(move || {
        let mut report = reporter(&state, &worker_weak);
        if let Err(reason) = instance_step::write(&root, &chosen, &mut report) {
            retryable(&state, &worker_weak, reason);
            return;
        }
        let aliases = chosen.iter().map(|pick| pick.alias.clone()).collect();
        lock(&state).instances = aliases;
        let restarted = instance_step::restart(&root, &mut report)
            .and_then(|status| report(&format!("actingctl status: {status}")));
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

/// Step 4 → 5. When step 4 touched the configuration and the Runtime, the
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
        let mut report = reporter(&state, &worker_weak);
        let settled = instance_step::settle(&root, &mut report);
        let broken = lock(&state).log_error.clone();
        match (settled, broken) {
            (Err(reason), _) | (Ok(_), Some(reason)) => fail(&state, &worker_weak, reason),
            (Ok(settled), None) => {
                lock(&state).settled = Some(settled);
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
    if let (Some(installed), Some(upgraded)) = (&state.installed, &state.upgraded) {
        lines.push(format!(
            "已升级 / Upgraded from runtime {} · ui {}",
            short(&installed.runtime_sha),
            short(&installed.ui_sha)
        ));
        lines.push(format!("被替换的版本 / Version replaced, kept in: {}", upgraded.previous.display()));
        lines.push(match &upgraded.restarted {
            Some(log) => format!("Runtime 已用新版本重新拉起 / restarted on the new version; 日志 / log: {}", log.display()),
            None => "Runtime 升级前未在运行，未拉起 / was not running, not started".to_string(),
        });
        lines.push("状态根、配置、监控台设置与开机自启保持不变 / State, configuration, console settings and autostart are unchanged".to_string());
    }
    if let Some(laid_out) = &state.laid_out {
        lines.push(format!("Runtime: {}", laid_out.actingd_exe.display()));
        lines.push(format!("监控台 / Console: {}", laid_out.acui_exe.display()));
        lines.push(format!(
            "工具 / Tools: {}（{}）",
            laid_out.tools_dir.display(),
            verify::TOOLS_INSTALLED.join("、")
        ));
    }
    if let Some(configured) = &state.configured {
        lines.push(format!("Runtime 配置 / Config: {}", configured.config_path.display()));
        lines.push(format!("状态根 / State root: {}", configured.state_root.display()));
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
    if fresh && !state.instances.is_empty() {
        lines.push(format!(
            "实例 / Instances: {}（已写入配置并通过检查 / in the configuration, checked）",
            state.instances.join("、")
        ));
    }
    if let Some(settled) = &state.settled {
        lines.extend(settled.mumu_root.as_ref().map(|mumu| format!("MuMu 目录 / MuMu folder (mumu_root): {mumu}")));
        lines.push(match &settled.running {
            Ok(true) => "Runtime 在运行（第 4 步拉起）/ The Runtime is running (started in step 4)".to_string(),
            Ok(false) => "Runtime 未在运行 / The Runtime is not running".to_string(),
            Err(reason) => format!("Runtime 是否在运行未能确认 / Whether the Runtime runs is not confirmed: {reason}"),
        });
    }
    if let Some(log) = &state.log {
        lines.push(format!("日志 / Log: {}", log.path().display()));
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

/// Step 5: the console, detached, and the wizard closed.
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
        Ok(()) => {
            let _ = write_log(state, &format!("已拉起监控台 / console started: {}", exe.display()));
            let _ = slint::quit_event_loop();
        }
        Err(reason) => {
            let _ = write_log(state, &reason);
            window.set_note(reason.into());
        }
    }
}
