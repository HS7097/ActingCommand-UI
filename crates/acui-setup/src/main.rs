// SPDX-License-Identifier: GPL-3.0-only
//! acsetup: the install wizard for ActingCommand on Windows. One window, five
//! pages — location, get, verify and lay out, configure, finish. It fetches
//! the release to install from the umbrella repository's Releases, or takes a
//! folder a person filled by hand, checks it against what the umbrella
//! published, lays it out under a per-user install root, writes the Runtime
//! configuration and the console's settings, optionally a Startup-folder
//! launcher, and can open the console. That fetch is its only network code; it
//! registers no service and no scheduled task, touches neither PATH nor the
//! registry, and configures no instance.
//!
//! The wizard is Windows-only. Elsewhere it compiles, and the first platform
//! question is the loud stop, before any window is opened.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod fetch;
mod install;
mod log;
mod platform;
mod verify;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use slint::ComponentHandle;

use fetch::Release;
use install::{Autostart, Configured, LaidOut};
use log::InstallLog;

slint::include_modules!();

const EXISTING: &str = "此处已有安装（runtime\\BUILD-MANIFEST.json 存在）；v1 没有升级流程，请换一个安装根 / An installation is already here; v1 has no upgrade flow, choose another root";

/// What the steps so far established: one run, one root, one log.
#[derive(Default)]
struct State {
    root: PathBuf,
    download: PathBuf,
    log: Option<InstallLog>,
    /// The release the online path installs, once looked up.
    release: Option<Release>,
    laid_out: Option<LaidOut>,
    configured: Option<Configured>,
    autostart: Option<Autostart>,
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
    window.on_close_wizard(|| {
        let _ = slint::quit_event_loop();
    });
}

fn installed(root: &Path) -> bool {
    root.join("runtime").join("BUILD-MANIFEST.json").is_file()
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
    let existing = if installed(&root) {
        EXISTING
    } else {
        "此处没有已安装的 Runtime / No installation here yet"
    };
    window.set_existing_text(existing.into());
}

fn next(window: &SetupWindow, state: &Shared) {
    match window.get_step() {
        0 => enter_get(window, state),
        1 => begin_get(window, state),
        2 => enter_configure(window, state),
        3 => run_configure(window, state),
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
    let Some(log) = state.log.as_mut() else {
        return Err("日志尚未创建 / The log has not been created".into());
    };
    let path = log.path().display().to_string();
    log.line(line)
        .map_err(|error| format!("日志写入失败 / Log write failed: {path}: {error}"))
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
    let blocked = if !root.is_absolute() {
        Some("安装根必须是绝对路径 / The install root must be an absolute path".to_string())
    } else if installed(&root) {
        Some(EXISTING.to_string())
    } else {
        None
    };
    if let Some(text) = blocked {
        window.set_note(text.into());
        return;
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
    {
        let mut state = lock(state);
        state.root = root.clone();
        state.log = Some(log);
    }
    let started = format!(
        "acsetup {} · 安装根 / install root: {}",
        env!("CARGO_PKG_VERSION"),
        root.display()
    );
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
        let found = fetch::choose().and_then(|release| {
            release.folder()?;
            Ok(release)
        });
        let line = match &found {
            Ok(release) => release_line(release),
            Err(reason) => reason.clone(),
        };
        if let Err(reason) = write_log(&state, &line) {
            fail(&state, &worker_weak, reason);
            return;
        }
        let found = found.ok();
        let ok = found.is_some();
        lock(&state).release = found;
        let _ = worker_weak.upgrade_in_event_loop(move |window| {
            window.set_busy(false);
            window.set_release_text(line.into());
            window.set_can_next(ok || window.get_offline());
            if !ok {
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
        let line = format!("发布件文件夹 / download folder: {}", download.display());
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
        let outcome = verify::run(&download, &staging, &mut report)
            .and_then(|verified| install::lay_out(&root, &verified, &mut report));
        match outcome {
            Ok(laid_out) => {
                lock(&state).laid_out = Some(laid_out);
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
/// then the optional launcher and the summary.
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
            window.set_summary(summary(state).into());
            window.set_note("".into());
            window.set_step(4);
            window.set_can_next(false);
        }
        Err(reason) => fail(state, &window.as_weak(), reason),
    }
}

fn summary(state: &Shared) -> String {
    let state = lock(state);
    let mut lines = vec![format!("安装根 / Install root: {}", state.root.display())];
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
    lines.push(match &state.autostart {
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
    });
    if let Some(log) = &state.log {
        lines.push(format!("日志 / Log: {}", log.path().display()));
    }
    lines.push(String::new());
    lines.push(
        "实例（模拟器 / 设备）本引导未配置，instances 为空：之后点监控台顶栏的「实例配置」按钮添加。"
            .to_string(),
    );
    lines.push(
        "No instance was configured here (instances is empty): add them with the Instance Configuration button in the console's top bar."
            .to_string(),
    );
    lines.join("\n")
}

/// Step 4: the console, detached, and the wizard closed — never actingd.
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
