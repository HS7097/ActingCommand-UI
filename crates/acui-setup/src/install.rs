// SPDX-License-Identifier: GPL-3.0-only
//! The install and configure steps: lay the verified files out under the install root, write
//! the Runtime's configuration and the console's settings, the optional
//! per-user Startup launcher, and start the console.
//!
//! Nothing here touches PATH, the registry, services or scheduled tasks; the
//! configuration template is copied byte for byte and never edited; a state
//! root that already has content is never taken over; no instance is
//! configured — that is the console's, later.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::platform;
use crate::verify::{hex, Report, Staged, Step, Total, Verified, MANIFEST, MISMATCH, TOOLS_INSTALLED};

const CONFIG_SCHEMA_VERSION: &str = "actingcommand.actingd.config.v1";

#[derive(Clone)]
pub struct LaidOut {
    pub ui_dir: PathBuf,
    pub tools_dir: PathBuf,
    pub actingd_exe: PathBuf,
    pub acui_exe: PathBuf,
}

/// `<root>\runtime` and `<root>\ui` get every manifest-bound file plus the
/// manifest; `<root>\tools` gets only the three installed tools. Staging is
/// removed afterwards.
pub fn lay_out(root: &Path, verified: &Verified, report: Report<'_>) -> Result<LaidOut, String> {
    let runtime_dir = root.join("runtime");
    let ui_dir = root.join("ui");
    let tools_dir = root.join("tools");
    // Every manifest-bound file and the two manifests, then the three tools.
    let total = verified.runtime.files.len() + verified.ui.files.len() + 2 + TOOLS_INSTALLED.len();
    report.step(Step::Phase("安装文件 / Installing files", Some(Total::Items(total as u64))))?;
    let mut done = 0;
    copy_all(&verified.runtime, &runtime_dir, &mut done, report)?;
    copy_all(&verified.ui, &ui_dir, &mut done, report)?;
    copy_named(&verified.tools, &tools_dir, TOOLS_INSTALLED, &mut done, report)?;
    fs::remove_dir_all(&verified.staging).map_err(|error| {
        format!(
            "临时目录未能删除 / staging not removed: {}: {error}",
            verified.staging.display()
        )
    })?;
    report.line(&format!("已删除临时目录 / staging removed: {}", verified.staging.display()))?;
    Ok(LaidOut {
        actingd_exe: runtime_dir.join("actingcommand-actingd.exe"),
        acui_exe: ui_dir.join("acui.exe"),
        ui_dir,
        tools_dir,
    })
}

fn copy_all(staged: &Staged, dest: &Path, done: &mut u64, report: Report<'_>) -> Result<(), String> {
    let names: Vec<&str> = staged
        .files
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(MANIFEST))
        .collect();
    copy_named(staged, dest, &names, done, report)
}

fn copy_named(
    staged: &Staged,
    dest: &Path,
    names: &[&str],
    done: &mut u64,
    report: Report<'_>,
) -> Result<(), String> {
    fs::create_dir_all(dest)
        .map_err(|error| format!("无法创建目录 / cannot create: {}: {error}", dest.display()))?;
    for name in names {
        let from = staged.dir.join(name);
        let to = dest.join(name);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("无法创建目录 / cannot create: {}: {error}", parent.display())
            })?;
        }
        let expected = fs::metadata(&from)
            .map(|meta| meta.len())
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", from.display()))?;
        let copied = fs::copy(&from, &to).map_err(|error| {
            format!("复制失败 / copy failed: {} → {}: {error}", from.display(), to.display())
        })?;
        if copied != expected {
            return Err(format!(
                "{MISMATCH}: {} 复制后 {copied} 字节，应为 / expected {expected}",
                to.display()
            ));
        }
        report.line(&format!("已安装 / installed: {}", to.display()))?;
        *done += 1;
        report.step(Step::Done(*done))?;
    }
    Ok(())
}

#[derive(Clone)]
pub struct Configured {
    pub state_root: PathBuf,
    pub config_path: PathBuf,
    pub settings_path: PathBuf,
}

/// Exactly the fields the wizard sets, in this order; the Runtime's parser
/// denies unknown fields and defaults the rest.
#[derive(Serialize)]
struct ActingdConfig<'a> {
    schema_version: &'a str,
    state_root: &'a str,
    bind_host: &'a str,
    bind_port: u16,
    secret_fingerprint_salt: &'a str,
    instances: Vec<()>,
}

/// A state root the wizard may create: absolute, and either absent or an
/// empty directory. One with content is never taken over.
pub fn state_root_usable(state_root: &Path) -> Result<(), String> {
    if !state_root.is_absolute() {
        return Err("状态根必须是绝对路径 / The state root must be an absolute path".into());
    }
    match fs::metadata(state_root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "状态根无法访问 / The state root cannot be read: {}: {error}",
            state_root.display()
        )),
        Ok(meta) if !meta.is_dir() => Err(format!(
            "状态根不是目录 / The state root is not a directory: {}",
            state_root.display()
        )),
        Ok(_) => {
            let mut entries = fs::read_dir(state_root).map_err(|error| {
                format!(
                    "状态根无法访问 / The state root cannot be read: {}: {error}",
                    state_root.display()
                )
            })?;
            if entries.next().is_some() {
                return Err(format!(
                    "状态根已有内容，引导程序不接管已有状态根 / The state root already has content; the wizard never takes over an existing state root: {}",
                    state_root.display()
                ));
            }
            Ok(())
        }
    }
}

/// The state root created, the configuration written with a fresh salt from
/// the OS RNG (never shown, never logged), then the console's settings.
/// `state_root` has passed `state_root_usable`.
pub fn configure(
    root: &Path,
    state_root: &Path,
    laid_out: &LaidOut,
    report: Report<'_>,
) -> Result<Configured, String> {
    fs::create_dir_all(state_root).map_err(|error| {
        format!("无法创建状态根 / cannot create the state root: {}: {error}", state_root.display())
    })?;
    report.line(&format!("状态根已就绪 / state root ready: {}", state_root.display()))?;

    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt)
        .map_err(|error| format!("系统随机源不可用 / OS RNG unavailable: {error}"))?;
    let salt_hex = hex(&salt);
    let state_root_text = state_root.display().to_string();
    let config = ActingdConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        state_root: &state_root_text,
        bind_host: "127.0.0.1",
        bind_port: 0,
        secret_fingerprint_salt: &salt_hex,
        instances: Vec::new(),
    };
    let mut json = serde_json::to_string_pretty(&config)
        .map_err(|error| format!("配置序列化失败 / config serialization failed: {error}"))?;
    json.push('\n');
    let config_path = root.join("actingd.config.json");
    fs::write(&config_path, json)
        .map_err(|error| format!("写入失败 / write failed: {}: {error}", config_path.display()))?;
    report.line(&format!(
        "已写 Runtime 配置 / config written: {}（salt 已生成，不记录 / salt generated, not recorded）",
        config_path.display()
    ))?;

    let settings_path = platform::console_settings_path()?;
    write_console_settings(&settings_path, state_root, &config_path, &laid_out.actingd_exe)?;
    report.line(&format!(
        "已写监控台设置 / console settings written: {}",
        settings_path.display()
    ))?;
    Ok(Configured {
        state_root: state_root.to_path_buf(),
        config_path,
        settings_path,
    })
}

/// The console's own file in the console's own format
/// (`crates/acui-app/src/settings.rs`): `lang` and `text_size` as the file
/// has them, then the three paths as TOML literal strings in single quotes.
/// Nothing else the file may hold is carried over, exactly as the console's
/// own writer does.
fn write_console_settings(
    path: &Path,
    state_root: &Path,
    config: &Path,
    exe: &Path,
) -> Result<(), String> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!("读取失败 / read failed: {}: {error}", path.display()));
        }
    };
    let mut body = String::from("# ActingCommand 监控台 / ActingCommand Console\n");
    for line in existing.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if key == "lang" || key == "text_size" {
                body.push_str(&format!("{key} = {}\n", value.trim()));
            }
        }
    }
    for (key, value) in [
        ("state_root", state_root),
        ("actingd_config", config),
        ("actingd_exe", exe),
    ] {
        let text = value.display().to_string();
        if text.contains('\'') {
            return Err(format!(
                "{key} 含单引号，写不成 TOML 字面量字符串 / contains a single quote and cannot be a TOML literal string: {text}"
            ));
        }
        body.push_str(&format!("{key} = '{text}'\n"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建目录 / cannot create: {}: {error}", parent.display()))?;
    }
    fs::write(path, body)
        .map_err(|error| format!("写入失败 / write failed: {}: {error}", path.display()))
}

pub enum Autostart {
    /// Not asked for; `existing` names a launcher already in the Startup
    /// folder, left as it was.
    NotWanted { existing: Option<PathBuf> },
    Written { path: PathBuf, with_console: bool },
}

/// The per-user Startup-folder launcher, only when asked for: `start ""` of
/// the Runtime with the written configuration, and of the console when that
/// was asked for too. No service, no scheduled task, no registry.
pub fn autostart(
    root: &Path,
    wanted: bool,
    with_console: bool,
    report: Report<'_>,
) -> Result<Autostart, String> {
    let path = platform::startup_launcher_path()?;
    if !wanted {
        let existing = path.is_file().then(|| path.clone());
        report.line(&match &existing {
            Some(existing) => format!(
                "未勾选开机自启；启动文件夹已有 {}，未改动 / autostart not wanted; the existing launcher is left as is",
                existing.display()
            ),
            None => "未勾选开机自启，未写启动项 / autostart not wanted, nothing written".to_string(),
        })?;
        return Ok(Autostart::NotWanted { existing });
    }
    let root_text = root.display().to_string();
    if root_text.contains('%') {
        return Err(format!(
            "安装根含 %，批处理无法原样引用 / the install root contains '%', which a .cmd cannot quote verbatim: {root_text}"
        ));
    }
    let mut body = format!(
        "@echo off\r\nstart \"\" \"{}\" --config \"{}\"\r\n",
        root.join("runtime").join("actingcommand-actingd.exe").display(),
        root.join("actingd.config.json").display()
    );
    if with_console {
        body.push_str(&format!(
            "start \"\" \"{}\"\r\n",
            root.join("ui").join("acui.exe").display()
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建目录 / cannot create: {}: {error}", parent.display()))?;
    }
    fs::write(&path, body)
        .map_err(|error| format!("写入失败 / write failed: {}: {error}", path.display()))?;
    report.line(&format!(
        "已写开机自启 / autostart written: {}{}",
        path.display(),
        if with_console {
            "（含监控台 / with the console）"
        } else {
            ""
        }
    ))?;
    Ok(Autostart::Written { path, with_console })
}

/// The console, detached; never actingd — the console's launcher starts the
/// Runtime.
pub fn open_console(acui_exe: &Path, ui_dir: &Path) -> Result<(), String> {
    platform::spawn_detached(acui_exe, ui_dir).map_err(|error| {
        format!(
            "拉起监控台失败 / cannot start the console: {}: {error}",
            acui_exe.display()
        )
    })
}
