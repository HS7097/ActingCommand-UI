// SPDX-License-Identifier: GPL-3.0-only
//! The install and configure steps: lay the verified files out under the install root, write
//! the Runtime's configuration and the console's settings, the optional
//! per-user Startup launcher, and start the console.
//!
//! Program slots and private configuration inputs are prepared before the
//! installation selection is committed. The shared state root stays outside
//! the slots; an existing state root is never taken over by a fresh install.

use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use acui_installation::InstallSlot;
use serde::Serialize;

use crate::platform;
use crate::verify::{
    MANIFEST, MISMATCH, Report, Staged, Step, Total, Verified, hex,
};

const CONFIG_SCHEMA_VERSION: &str = "actingcommand.actingd.config.v1";

#[derive(Clone)]
pub struct LaidOut {
    pub ui_dir: PathBuf,
    pub tools_dir: PathBuf,
    pub actingd_exe: PathBuf,
    pub acui_exe: PathBuf,
}

/// Prepares the program core in a new directory outside the selected slot: the
/// whole runtime and ui zips (Workflow #359, #360). The tools live under the
/// installation root (`root_tools`).
/// Failed preparation leaves its files for inspection; no installed tree is overwritten.
pub fn prepare_programs(
    dir: &Path,
    verified: &Verified,
    report: Report<'_>,
) -> Result<(), String> {
    fs::create_dir(dir).map_err(|error| {
        format!(
            "Cannot create fresh program candidate {}: {error}",
            dir.display()
        )
    })?;
    let total = verified.runtime.files.len() + verified.ui.files.len() + 3;
    report.step(Step::Phase(
        "准备候选程序 / Preparing candidate programs",
        Some(Total::Items(total as u64)),
    ))?;
    let mut done = 0;
    for (name, staged) in [("runtime", &verified.runtime), ("ui", &verified.ui)] {
        copy_all(staged, &dir.join(name), &mut done, report)?;
    }
    let members = dir.join("MEMBERS.json");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&members)
        .map_err(|error| format!("Cannot create {}: {error}", members.display()))?;
    file.write_all(&verified.members_document)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Cannot write {}: {error}", members.display()))?;
    drop(file);
    report.step(Step::Done((done + 1) as u64))?;
    crate::verify::prepared_programs(dir, verified, report)?;
    report.line(&format!(
        "候选程序已核验 / Candidate programs verified: {}",
        dir.display()
    ))
}

fn copy_all(
    staged: &Staged,
    dest: &Path,
    done: &mut u64,
    report: Report<'_>,
) -> Result<(), String> {
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
        // A manifest path is `/`-separated: joined segment by segment, so
        // every path said in the log uses the one separator.
        let relative: PathBuf = name.split('/').collect();
        let from = staged.dir.join(&relative);
        let to = dest.join(&relative);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "无法创建目录 / cannot create: {}: {error}",
                    parent.display()
                )
            })?;
        }
        let expected = fs::metadata(&from)
            .map(|meta| meta.len())
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", from.display()))?;
        let copied = fs::copy(&from, &to).map_err(|error| {
            format!(
                "复制失败 / copy failed: {} → {}: {error}",
                from.display(),
                to.display()
            )
        })?;
        if copied != expected {
            return Err(format!(
                "{MISMATCH}: {} 复制后 {copied} 字节，应为 / expected {expected}",
                to.display()
            ));
        }
        report.line(&format!("已复制 / copied: {}", to.display()))?;
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

/// A first selection has no running installation to drain. Program verification
/// and the checked private generation still precede its atomic publication.
pub fn fresh(
    root: &Path,
    verified: &Verified,
    report: Report<'_>,
) -> Result<(LaidOut, Configured), String> {
    let writer = crate::generations::Writer::acquire(root)?;
    let state_root = root.join("state");
    state_root_usable(&state_root)?;
    crate::slots::initialize(&writer)?;
    let programs = crate::slots::materialize(&writer, InstallSlot::A, verified, report)?;
    let forward = programs.ui_dir.join("acforward.exe");
    if !forward.is_file() {
        return Err(
            "The UI release does not contain its stable product entry acforward.exe".into(),
        );
    }
    let tools = crate::root_tools::plan(root, verified)?;
    tools.add(root, report)?;
    fs::create_dir_all(&state_root)
        .map_err(|error| format!("Cannot create shared state root: {error}"))?;
    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt).map_err(|error| format!("OS RNG unavailable: {error}"))?;
    let document = serde_json::to_value(ActingdConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        state_root: &state_root.to_string_lossy(),
        bind_host: "127.0.0.1",
        bind_port: 0,
        secret_fingerprint_salt: &hex(&salt),
        instances: Vec::new(),
    })
    .map_err(|error| error.to_string())?;
    let mut plan = writer.prepare(
        None,
        InstallSlot::A,
        &root.join("actingd.config.json"),
        document,
        false,
        report,
    )?;
    tools.apply(
        root,
        &root.join(format!(
            "install/root-tools-{}",
            plan.snapshot.selection.generation
        )),
        report,
    )?;
    let laid_out = stable_entries(root, &forward, report)?;
    install_manager(root, verified, report)?;
    let config_path = plan.snapshot.config_path()?;
    let settings_path = platform::console_settings_path()?;
    write_console_settings(
        &settings_path,
        &state_root,
        &root.join("actingd.config.json"),
        &laid_out.actingd_exe,
    )?;
    writer.commit(&mut plan)?;
    report.line("A 槽及私有配置已选中 / Slot A and its private configuration are selected")?;
    Ok((
        laid_out,
        Configured {
            state_root,
            config_path,
            settings_path,
        },
    ))
}

/// One implementation is installed under the fixed product filenames. Each
/// invocation derives its route from that filename and retains one selection.
/// The root's `tools\` holds the real tools, not entries (`root_tools`).
pub fn stable_entries(root: &Path, forward: &Path, report: Report<'_>) -> Result<LaidOut, String> {
    let bytes = acui_installation::read_bounded(forward, 64 * 1024 * 1024)?;
    for (component, names) in [
        (
            "runtime",
            &["actingcommand-actingd.exe", "actingctl.exe"][..],
        ),
        ("ui", &["acui.exe"][..]),
    ] {
        let directory = root.join(component);
        fs::create_dir(&directory).map_err(|error| {
            format!(
                "Cannot create fresh stable entry directory {}: {error}",
                directory.display()
            )
        })?;
        for name in names {
            crate::generations::write_new(&directory.join(name), &bytes)?;
        }
    }
    report.line("固定产品入口已创建 / Stable product entries created")?;
    Ok(LaidOut {
        ui_dir: root.join("ui"),
        tools_dir: root.join("tools"),
        actingd_exe: root.join("runtime").join(crate::runtime::ACTINGD),
        acui_exe: root.join("ui/acui.exe"),
    })
}

/// The fixed installation manager retains its own source identity across
/// business-slot changes. An existing manager is verified and kept in place.
pub fn install_manager(root: &Path, verified: &Verified, report: Report<'_>) -> Result<(), String> {
    let evidence = root.join(acui_installation::MANAGER_DIRECTORY);
    if evidence
        .try_exists()
        .map_err(|error| format!("Cannot inspect management identity: {error}"))?
    {
        acui_installation::manager_program(root)?;
        return report.line(
            "固定安装管理程序已核验并保留 / Fixed installation manager verified and retained",
        );
    }
    let manifest = acui_installation::read_bounded(
        &verified.ui.dir.join(MANIFEST),
        acui_installation::MAX_MATERIAL_BYTES,
    )?;
    acui_installation::verify_manager_material(&verified.members_document, &manifest, &verified.ui.dir.join("acsetup.exe"))?;
    fs::create_dir(&evidence)
        .map_err(|error| format!("Cannot create management identity directory: {error}"))?;
    crate::generations::write_new(&evidence.join("MEMBERS.json"), &verified.members_document)?;
    crate::generations::write_new(&evidence.join(MANIFEST), &manifest)?;
    let source = verified.ui.dir.join("acsetup.exe");
    let mut input = fs::File::open(&source).map_err(|error| {
        format!(
            "Cannot read management program {}: {error}",
            source.display()
        )
    })?;
    let target = root.join("ui/acsetup.exe");
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|error| {
            format!(
                "Cannot create fixed management program {}: {error}",
                target.display()
            )
        })?;
    io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|error| format!("Cannot install fixed management program: {error}"))?;
    drop(output);
    acui_installation::manager_program(root)?;
    report.line(
        "固定安装管理程序及来源已核验 / Fixed installation manager and source identity verified",
    )
}

/// An explicit management update runs from the verified release outside this
/// installation. Native occupancy must prove the fixed manager has exited.
/// `--replace-manager <root> <release folder>`. The caller owns the log
/// (`main::internal_entry`); the returned line is the result to show.
pub fn replace_manager_from_entry(report: Report<'_>) -> Result<String, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 { return Err("Usage: acsetup --replace-manager <absolute-install-root> <absolute-release-directory>".into()); }
    let root = PathBuf::from(&args[1]);
    let download = PathBuf::from(&args[2]);
    if !root.is_absolute() || !download.is_absolute() { return Err("Management replacement requires absolute paths".into()); }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let exe = std::env::current_exe().and_then(fs::canonicalize).map_err(|error| error.to_string())?;
    // Any copy of the release's acsetup but the fixed manager itself may replace
    // it, the new slot's `ui\acsetup.exe` included (review R2-1).
    if fs::canonicalize(root.join("ui/acsetup.exe")).ok().as_deref() == Some(exe.as_path()) { return Err("Management replacement must run from the release's acsetup or a slot's ui\\acsetup.exe, not from the fixed manager it replaces".into()); }
    let writer = crate::generations::Writer::acquire(&root)?;
    let previous = acui_installation::manager_program(writer.root())?;
    let stamp = crate::log::unix_ms();
    let staging = root.join(format!("install/manager-source-{stamp}"));
    let replaced = (|| {
        let verified = crate::verify::run(&download, &staging, report)?;
        let manifest = acui_installation::read_bounded(&verified.ui.dir.join(MANIFEST), acui_installation::MAX_MATERIAL_BYTES)?;
        let candidate = verified.ui.dir.join("acsetup.exe");
        acui_installation::verify_manager_material(&verified.members_document, &manifest, &candidate)?;
        if crate::verify::sha256_file(&exe).map_err(|error| error.to_string())? != crate::verify::sha256_file(&candidate).map_err(|error| error.to_string())? {
            return Err("External installer is not the exact management program in the verified release".into());
        }
        drop(previous);
        replace_manager(&root, &verified, report)
    })();
    // The verified copy has served once the manager is replaced or the run stopped; the new
    // manager and its identity are copies (Workflow #364, review L2).
    crate::remove_staging(&staging, report)?;
    let backup = replaced?;
    Ok(format!(
        "固定管理程序已替换，旧程序保留于 / Fixed management entry replaced; previous source retained at {}",
        backup.display()
    ))
}

/// After an A/B upgrade the fixed manager becomes this release's acsetup
/// (review R-F2): an older one cannot switch to this layout's slots. Nothing
/// happens when it already is. When the fixed manager is the program running
/// this upgrade it cannot replace itself: the upgrade stands and the returned
/// notice gives the exact command to run (review R2-1). Any other running
/// manager, or a failed replacement, is an error.
pub fn refresh_manager(
    root: &Path,
    verified: &Verified,
    slot_root: &Path,
    report: Report<'_>,
) -> Result<Option<String>, String> {
    let current = acui_installation::manager_program(root)?;
    let candidate = verified.ui.dir.join("acsetup.exe");
    let hash = |path: &Path| {
        crate::verify::sha256_file(path)
            .map_err(|error| format!("Cannot hash {}: {error}", path.display()))
    };
    if hash(current.as_path())? == hash(candidate.as_path())? {
        report.line("固定管理程序已是本发布件的 acsetup / The fixed manager is already this release's acsetup")?;
        return Ok(None);
    }
    let running = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| error.to_string())?;
    if fs::canonicalize(&current).map_err(|error| error.to_string())? == running {
        let plain = crate::generations::plain;
        return Ok(Some(format!(
            "固定管理器仍是旧版，未替换 / fixed manager not replaced (running). 先关闭本安装程序窗口（以及其它 ui\\acsetup.exe），再在 PowerShell 运行 / First close this installer window (and any other ui\\acsetup.exe), then run in PowerShell: & \"{}\" --replace-manager \"{}\" \"{}\"",
            plain(&slot_root.join("ui").join("acsetup.exe")).display(),
            plain(root).display(),
            plain(&verified.download).display()
        )));
    }
    replace_manager(root, verified, report).map(|_| None)
}

/// The fixed manager replaced by `verified`'s acsetup. The previous program and
/// its identity are kept under `install/manager-backup-<stamp>` and put back on
/// failure; a running manager blocks (native occupancy). Returns the backup.
pub fn replace_manager(root: &Path, verified: &Verified, report: Report<'_>) -> Result<PathBuf, String> {
    let previous = acui_installation::manager_program(root)?;
    let manifest = acui_installation::read_bounded(&verified.ui.dir.join(MANIFEST), acui_installation::MAX_MATERIAL_BYTES)?;
    let candidate = verified.ui.dir.join("acsetup.exe");
    acui_installation::verify_manager_material(&verified.members_document, &manifest, &candidate)?;
    let occupancy = crate::slots::NativeOccupancy::files(std::slice::from_ref(&previous)).map_err(|error| {
        format!("固定管理程序正在使用 / The fixed manager is in use: {error}")
    })?;
    let stamp = crate::log::unix_ms();
    let backup = root.join(format!("install/manager-backup-{stamp}"));
    fs::create_dir(&backup).map_err(|error| error.to_string())?;
    fs::rename(&previous, backup.join("acsetup.exe")).map_err(|error| format!("Cannot retain previous manager: {error}"))?;
    let identity = root.join(acui_installation::MANAGER_DIRECTORY);
    if let Err(error) = fs::rename(&identity, backup.join("identity")) {
        let restore = fs::rename(backup.join("acsetup.exe"), &previous);
        return Err(format!("Cannot retain management identity: {error}; program restore: {restore:?}"));
    }
    let installed = install_manager(root, verified, report);
    if let Err(error) = installed {
        let restore = (|| {
            if previous.try_exists().map_err(|error| error.to_string())? { fs::rename(&previous, backup.join("retained-candidate.exe")).map_err(|error| error.to_string())?; }
            if identity.try_exists().map_err(|error| error.to_string())? { fs::rename(&identity, backup.join("retained-identity")).map_err(|error| error.to_string())?; }
            fs::rename(backup.join("identity"), &identity).map_err(|error| error.to_string())?;
            fs::rename(backup.join("acsetup.exe"), &previous).map_err(|error| error.to_string())
        })();
        return Err(format!("Management replacement failed: {error}; original manager restoration: {restore:?}"));
    }
    drop(occupancy);
    report.line(&format!("固定管理程序已替换，旧程序保留于 / Fixed management entry replaced; previous source retained at {}", backup.display()))?;
    Ok(backup)
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

/// The console's own file in the console's own format
/// (`crates/acui-app/src/settings.rs`): `lang` and `text_size` as the file
/// has them, then the three paths as TOML literal strings in single quotes.
/// Nothing else the file may hold is carried over, exactly as the console's
/// own writer does.
pub fn write_console_settings(
    path: &Path,
    state_root: &Path,
    config: &Path,
    exe: &Path,
) -> Result<(), String> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "读取失败 / read failed: {}: {error}",
                path.display()
            ));
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
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "无法创建目录 / cannot create: {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, body)
        .map_err(|error| format!("写入失败 / write failed: {}: {error}", path.display()))
}

pub enum Autostart {
    /// Not asked for; `existing` names a launcher already in the Startup
    /// folder, left as it was.
    NotWanted {
        existing: Option<PathBuf>,
    },
    Written {
        path: PathBuf,
        with_console: bool,
    },
}

/// The per-user Startup-folder launcher, only when asked for: `start ""` of
/// the Runtime with the written configuration, and of the console when that
/// was asked for too. No service, no scheduled task, no registry.
pub fn autostart(
    root: &Path,
    wanted: bool,
    with_console: bool,
    written: &mut Vec<PathBuf>,
    report: Report<'_>,
) -> Result<Autostart, String> {
    let path = platform::startup_launcher_path();
    if !wanted {
        // Only a look for one already there: a folder that cannot be found
        // is said in the log, and nothing stops.
        let existing = match &path {
            Ok(path) => path.is_file().then(|| path.clone()),
            Err(reason) => {
                report.line(&format!("启动文件夹无法确定，未查已有的启动项 / the Startup folder is unknown, no existing launcher looked for: {reason}"))?;
                None
            }
        };
        report.line(&match &existing {
            Some(existing) => format!(
                "未勾选开机自启；启动文件夹已有 {}，未改动 / autostart not wanted; the existing launcher is left as is",
                existing.display()
            ),
            None => "未勾选开机自启，未写启动项 / autostart not wanted, nothing written".to_string(),
        })?;
        return Ok(Autostart::NotWanted { existing });
    }
    let path = path?;
    let root_text = root.display().to_string();
    if root_text.contains('%') {
        return Err(format!(
            "安装根含 %，批处理无法原样引用 / the install root contains '%', which a .cmd cannot quote verbatim: {root_text}"
        ));
    }
    let mut body = format!(
        "@echo off\r\nstart \"\" \"{}\" --config \"{}\"\r\n",
        root.join("runtime")
            .join("actingcommand-actingd.exe")
            .display(),
        root.join("actingd.config.json").display()
    );
    if with_console {
        body.push_str(&format!(
            "start \"\" \"{}\"\r\n",
            root.join("ui").join("acui.exe").display()
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "无法创建目录 / cannot create: {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(&path, body)
        .map_err(|error| format!("写入失败 / write failed: {}: {error}", path.display()))?;
    written.push(path.clone());
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

/// Where a shortcut goes, asked of the shell only when it is needed.
type Locate = fn() -> Result<PathBuf, String>;

/// The console's shortcuts a person asked for: in the Start menu and on the
/// desktop, each `ActingCommand.lnk` to `ui\acui.exe`. One not asked for is
/// not written, and one already there is left as it is and said so. Every
/// shortcut written is added to `written` as it lands; returns this call's.
pub fn shortcuts(
    laid_out: &LaidOut,
    start_menu: bool,
    desktop: bool,
    written: &mut Vec<PathBuf>,
    report: Report<'_>,
) -> Result<Vec<PathBuf>, String> {
    let places: [(bool, &str, Locate); 2] = [
        (
            start_menu,
            "开始菜单 / Start menu",
            platform::start_menu_shortcut_path,
        ),
        (desktop, "桌面 / desktop", platform::desktop_shortcut_path),
    ];
    let mut made = Vec::new();
    for (wanted, place, locate) in places {
        if !wanted {
            // Only a look for one already there, never a reason to stop.
            report.line(&match locate() {
                Ok(path) if path.exists() => format!("未勾选{place}快捷方式；已有的 {} 未改动 / not wanted; the existing one is left as is", path.display()),
                Ok(_) => format!("未勾选{place}快捷方式 / {place} shortcut not wanted"),
                Err(reason) => format!("未勾选{place}快捷方式；文件夹无法确定 / not wanted; the folder is unknown: {reason}"),
            })?;
            continue;
        }
        let path = locate()?;
        platform::create_shortcut(
            &path,
            &laid_out.acui_exe,
            &laid_out.ui_dir,
            "ActingCommand 监控台 / console",
        )?;
        written.push(path.clone());
        report.line(&format!(
            "已建快捷方式 / shortcut written: {}",
            path.display()
        ))?;
        made.push(path);
    }
    Ok(made)
}

/// The console, detached; never actingd — the console's launcher starts the
/// Runtime.
pub fn open_console(acui_exe: &Path, ui_dir: &Path) -> Result<(), String> {
    let root = ui_dir.parent().ok_or("Console entry has no installation root")?;
    let snapshot = acui_installation::Snapshot::read(root)?;
    platform::spawn_detached(acui_exe, ui_dir, &snapshot).map_err(|error| {
        format!(
            "拉起监控台失败 / cannot start the console: {}: {error}",
            acui_exe.display()
        )
    })
}
