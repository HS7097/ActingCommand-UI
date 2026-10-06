// SPDX-License-Identifier: GPL-3.0-only
//! acsetup without its window (Workflow #359): one installation or upgrade, or
//! its plan, driven by flags. Every question the wizard would put to a person
//! is answered by a flag; a question without its flag stops before the
//! installation changes and names the flag to give. The console shows exactly
//! the install log; the exit code is the result.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acui_installation::InstallSlot;

use crate::fetch::{self, Release};
use crate::log::{self, InstallLog};
use crate::maintenance::{self, Association, Conflict, Side};
use crate::payload::{self, Payload};
use crate::upgrade::{self, Installed};
use crate::verify::{self, Reporter, Step};
use crate::{bundle, install, lifecycle, platform, root_tools, runtime, vision_migration};
use crate::{lock, write_log, Shared, State};

pub const DONE: i32 = 0;
pub const FAILED: i32 = 1;
pub const USAGE: i32 = 2;
pub const CONFLICTS: i32 = 3;
pub const DOWNGRADE: i32 = 4;
pub const ASSOCIATION: i32 = 5;

const HELP: &str = "acsetup — ActingCommand 安装与升级 / installation and upgrade

用法 / Usage:
  acsetup --root <安装根 / install root> (--plan | --yes) [选项 / options]

  --root <路径 / path>      安装根的绝对路径（必填）/ the absolute installation root (required)
  --plan                    只列出将做的改动与全部差异，安装根下不改任何文件
                            / list the changes and every difference; nothing under the root changes
  --yes                     执行安装或升级 / install or upgrade
  --conflicts new|old       维护绑定有差异时：全部采用新值，或全部保留旧值
                            / with maintenance binding differences: every proposed value, or every current one
  --associate <别名 / alias>=<标准包 / bundle>/<服务器 / server>
                            回答该实例的资源关联问题，可重复 / answers that instance's resource association; repeatable
  --allow-downgrade         允许装入更早、或先后无法确定的发布件
                            / allow an earlier release, or one whose order cannot be proved
  --online                  在线版：从伞仓取最新的发布件 / online installer: fetch the newest umbrella release
  --from <文件夹 / folder>  在线版：用已下载的发布件文件夹 / online installer: use a folder of downloaded release files
  --help                    本说明 / this text

离线完整版只用自带的发布件，不接受 --online 与 --from。
The offline full installer uses only the release it carries and takes neither --online nor --from.

退出码 / Exit codes:
  0 完成 / done            1 失败 / failed          2 用法 / usage
  3 有维护绑定差异而未给 --conflicts / differences without --conflicts
  4 降级而未给 --allow-downgrade / a downgrade without --allow-downgrade
  5 资源关联需要选择而未给 --associate / an association without --associate
  2（参数）、3、4、5 都停在安装改动之前 / 2 (arguments), 3, 4 and 5 stop before the installation changes.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plan,
    Execute,
}

enum Source {
    Carried,
    Online,
    From(PathBuf),
}

struct Args {
    root: PathBuf,
    mode: Mode,
    conflicts: Option<Side>,
    allow_downgrade: bool,
    source: Source,
    /// `(alias, "<bundle>/<server>")`, one per alias.
    associate: Vec<(String, String)>,
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

fn say_error(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

fn once(seen: bool, flag: &str) -> Result<(), String> {
    match seen {
        true => Err(format!(
            "{flag} 给了不止一次 / {flag} was given more than once"
        )),
        false => Ok(()),
    }
}

fn value<'a>(arguments: &'a [String], at: &mut usize, flag: &str) -> Result<&'a str, String> {
    *at += 1;
    arguments
        .get(*at)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} 需要一个值 / {flag} needs a value"))
}

/// `None` asks for the help text.
fn parse(arguments: &[String]) -> Result<Option<Args>, String> {
    let mut root: Option<PathBuf> = None;
    let (mut plan, mut yes, mut allow_downgrade, mut online) = (false, false, false, false);
    let mut conflicts: Option<Side> = None;
    let mut from: Option<PathBuf> = None;
    let mut associate: Vec<(String, String)> = Vec::new();
    let mut at = 0;
    while at < arguments.len() {
        let flag = arguments[at].as_str();
        match flag {
            "--help" | "-h" | "/?" => return Ok(None),
            "--root" => {
                once(root.is_some(), flag)?;
                root = Some(PathBuf::from(value(arguments, &mut at, flag)?));
            }
            "--plan" => {
                once(plan, flag)?;
                plan = true;
            }
            "--yes" => {
                once(yes, flag)?;
                yes = true;
            }
            "--conflicts" => {
                once(conflicts.is_some(), flag)?;
                conflicts = Some(match value(arguments, &mut at, flag)? {
                    "new" => Side::New,
                    "old" => Side::Old,
                    other => return Err(format!(
                        "--conflicts 只接受 new 或 old / --conflicts takes new or old, not {other}"
                    )),
                });
            }
            "--allow-downgrade" => {
                once(allow_downgrade, flag)?;
                allow_downgrade = true;
            }
            "--online" => {
                once(online, flag)?;
                online = true;
            }
            "--from" => {
                once(from.is_some(), flag)?;
                from = Some(PathBuf::from(value(arguments, &mut at, flag)?));
            }
            "--associate" => {
                let text = value(arguments, &mut at, flag)?;
                let shaped = text.split_once('=').filter(|(alias, choice)| {
                    !alias.is_empty()
                        && choice.split_once('/').is_some_and(|(bundle, server)| {
                            !bundle.is_empty() && !server.is_empty()
                        })
                });
                let Some((alias, choice)) = shaped else {
                    return Err(format!(
                        "--associate 的格式是 <别名>=<标准包>/<服务器> / --associate takes <alias>=<bundle>/<server>, not {text}"
                    ));
                };
                if associate.iter().any(|(known, _)| known.as_str() == alias) {
                    return Err(format!(
                        "--associate {alias}=… 给了不止一次 / was given more than once"
                    ));
                }
                associate.push((alias.to_string(), choice.to_string()));
            }
            other => return Err(format!("未知参数 / Unknown argument: {other}")),
        }
        at += 1;
    }
    let root = root.ok_or("缺少 --root / --root is required")?;
    if !root.is_absolute() {
        return Err(format!(
            "--root 必须是绝对路径 / --root must be an absolute path: {}",
            root.display()
        ));
    }
    let mode = match (plan, yes) {
        (true, false) => Mode::Plan,
        (false, true) => Mode::Execute,
        _ => {
            return Err(
                "--plan 与 --yes 必须给且只给一个 / give exactly one of --plan and --yes".into(),
            )
        }
    };
    let source = match (online, from) {
        (false, None) => Source::Carried,
        (true, None) => Source::Online,
        (false, Some(dir)) if dir.is_absolute() => Source::From(dir),
        (false, Some(dir)) => {
            return Err(format!(
                "--from 必须是绝对路径 / --from must be an absolute path: {}",
                dir.display()
            ))
        }
        (true, Some(_)) => {
            return Err(
                "--online 与 --from 只能给一个 / give at most one of --online and --from".into(),
            )
        }
    };
    Ok(Some(Args {
        root,
        mode,
        conflicts,
        allow_downgrade,
        source,
        associate,
    }))
}

/// The log and the console: every line in both; a warning also kept for the
/// summary; a phase named once when it begins.
struct ConsoleReport {
    state: Shared,
    phase: String,
}

impl Reporter for ConsoleReport {
    fn line(&mut self, line: &str) -> Result<(), String> {
        write_log(&self.state, line)?;
        say(line);
        Ok(())
    }

    fn step(&mut self, step: Step<'_>) -> Result<(), String> {
        if let Step::Phase(name, _) = step {
            if self.phase != name {
                self.phase = name.to_string();
                say(&format!("— {name}"));
            }
        }
        Ok(())
    }

    fn warn(&mut self, line: &str) -> Result<(), String> {
        write_log(&self.state, &format!("注意 / NOTE: {line}"))?;
        {
            let mut state = lock(&self.state);
            if !state.warnings.iter().any(|kept| kept == line) {
                state.warnings.push(line.to_string());
            }
        }
        say_error(&format!("注意 / NOTE: {line}"));
        Ok(())
    }
}

/// The command line's entry: the exit code of the whole run.
pub fn run() -> i32 {
    let arguments: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|argument| argument.into_string())
        .collect::<Result<_, _>>()
    {
        Ok(arguments) => arguments,
        Err(_) => {
            say_error("参数不是有效的 Unicode / An argument is not valid Unicode");
            return USAGE;
        }
    };
    let args = match parse(&arguments) {
        Ok(Some(args)) => args,
        Ok(None) => {
            say(HELP);
            return DONE;
        }
        Err(reason) => {
            say_error(&format!("{reason}\n\n{HELP}"));
            return USAGE;
        }
    };
    let payload = match payload::detect() {
        Ok(payload) => payload,
        Err(reason) => {
            say_error(&format!("失败 / FAILED: {reason}"));
            return FAILED;
        }
    };
    match (&payload, &args.source) {
        (Some(_), Source::Carried) | (None, Source::Online) | (None, Source::From(_)) => {}
        (Some(_), _) => {
            say_error("离线完整版只用自带的发布件，不接受 --online 与 --from / The offline full installer uses only the release it carries and takes neither --online nor --from");
            return USAGE;
        }
        (None, Source::Carried) => {
            say_error("在线版需要 --online 或 --from <文件夹> / The online installer needs --online or --from <folder>");
            return USAGE;
        }
    }
    match args.mode {
        Mode::Plan => plan(&args, payload),
        Mode::Execute => execute(&args, payload),
    }
}

/// One association answered from `--associate`. Without the flag a real run
/// stops (exit 5) and a plan (`open`) records the question; a flag that names
/// no offered option stops with exit 2. Every message names each option's flag.
fn pick(
    args: &Args,
    association: &Association,
    code: &Cell<Option<i32>>,
    consulted: &RefCell<BTreeSet<String>>,
    open: Option<&RefCell<Vec<String>>>,
) -> Result<Option<usize>, String> {
    let flags = association
        .options
        .iter()
        .map(|option| {
            format!(
                "  --associate {}={}    ({})",
                association.alias,
                option.flag_value(),
                option.label
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let Some((_, wanted)) = args
        .associate
        .iter()
        .find(|(alias, _)| alias.as_str() == association.alias.as_str())
    else {
        let text = format!(
            "实例 / Instance {} 需要选择资源关联，命令行从不替你选择；给出下列之一 / needs a resource association, which the command line never picks; give one of:\n{flags}",
            association.alias
        );
        if let Some(open) = open {
            open.borrow_mut().push(text);
            return Ok(None);
        }
        code.set(Some(ASSOCIATION));
        return Err(format!(
            "{text}\n安装未改动 / The installation was not changed"
        ));
    };
    consulted.borrow_mut().insert(association.alias.clone());
    let hits: Vec<usize> = association
        .options
        .iter()
        .enumerate()
        .filter(|(_, option)| option.flag_value() == *wanted)
        .map(|(at, _)| at)
        .collect();
    match hits.as_slice() {
        [at] => Ok(Some(*at)),
        _ => {
            code.set(Some(USAGE));
            Err(format!(
                "--associate {}={wanted} 不是该实例的可选项之一 / is not one of that instance's options:\n{flags}",
                association.alias
            ))
        }
    }
}

/// Every conflict takes `--conflicts`; without it a real run stops (exit 3)
/// after the list was logged and shown.
fn answer(
    args: &Args,
    conflicts: &[Conflict],
    code: &Cell<Option<i32>>,
) -> Result<Vec<Side>, String> {
    match args.conflicts {
        Some(side) => Ok(vec![side; conflicts.len()]),
        None => {
            code.set(Some(CONFLICTS));
            Err(format!(
                "有 {} 项维护绑定差异（见上）而未给 --conflicts new|old；安装未改动 / {} maintenance binding differences (listed above) and no --conflicts new|old; the installation was not changed",
                conflicts.len(),
                conflicts.len()
            ))
        }
    }
}

/// `--yes`: the installation or upgrade, its log under the root.
fn execute(args: &Args, payload: Option<Payload>) -> i32 {
    let root = &args.root;
    let installed = match upgrade::installed(root) {
        Ok(installed) => installed,
        Err(reason) => {
            say_error(&format!("失败 / FAILED: {reason}"));
            return FAILED;
        }
    };
    if installed.is_some() {
        if let Some((name, dir)) = crate::running_from(root) {
            say_error(&format!(
                "本程序 {name} 正从要升级的这份安装的 {dir}\\ 里运行；请复制到别处再运行 / {name} runs from {dir}\\ of the installation it would upgrade: copy it elsewhere and run it from there"
            ));
            return USAGE;
        }
    } else {
        let usable = install::state_root_usable(&root.join("state")).and_then(|()| {
            match root.display().to_string().contains('\'') {
                true => Err("安装位置含单引号，监控台设置无法原样写入 / The install location contains a single quote, which the console's settings cannot hold".to_string()),
                false => Ok(()),
            }
        });
        if let Err(reason) = usable {
            say_error(&format!("失败 / FAILED: {reason}"));
            return FAILED;
        }
    }
    let log = match std::fs::create_dir_all(root)
        .and_then(|()| InstallLog::create(root, log::unix_ms()))
    {
        Ok(log) => log,
        Err(error) => {
            say_error(&format!(
                "失败 / FAILED: 无法创建安装根或日志 / Cannot create the install root or the log: {}: {error}",
                root.display()
            ));
            return FAILED;
        }
    };
    let log_path = log.path().to_path_buf();
    let state: Shared = Arc::new(Mutex::new(State::default()));
    {
        let mut locked = lock(&state);
        locked.root = root.clone();
        locked.log = Some(log);
        locked.installed = installed.clone();
    }
    let code = Cell::new(None);
    let consulted = RefCell::new(BTreeSet::new());
    let mut report = ConsoleReport {
        state: Arc::clone(&state),
        phase: String::new(),
    };
    match install_or_upgrade(
        args,
        payload,
        installed,
        &state,
        &code,
        &consulted,
        &mut report,
    ) {
        Ok(()) => DONE,
        Err(reason) => {
            let _ = write_log(&state, &format!("失败 / FAILED: {reason}"));
            say_error(&format!(
                "失败 / FAILED: {reason}\n日志 / Log: {}",
                log_path.display()
            ));
            code.get().unwrap_or(FAILED)
        }
    }
}

fn install_or_upgrade(
    args: &Args,
    payload: Option<Payload>,
    installed: Option<Installed>,
    state: &Shared,
    code: &Cell<Option<i32>>,
    consulted: &RefCell<BTreeSet<String>>,
    report: &mut ConsoleReport,
) -> Result<(), String> {
    let root = &args.root;
    report.line(&format!(
        "acsetup {} · 命令行 / command line{} · 安装根 / install root: {}{}",
        env!("CARGO_PKG_VERSION"),
        payload.as_ref().map_or(String::new(), |payload| format!(
            " · 离线版 / offline edition {}",
            payload.tag
        )),
        root.display(),
        installed
            .as_ref()
            .map_or(String::new(), |installed| format!(
                " · 升级 / upgrade from runtime {} · ui {}",
                installed.runtime_sha, installed.ui_sha
            ))
    ))?;
    let log_path = lock(state)
        .log
        .as_ref()
        .map(|log| log.path().display().to_string())
        .unwrap_or_default();
    report.line(&format!("日志 / Log: {log_path}"))?;
    crate::clear_staging(state, root)?;
    let upgrading = installed.is_some();
    // The release: where it comes from and what its MEMBERS.json names.
    let mut carried: Option<Payload> = None;
    let mut release: Option<Release> = None;
    let (download, members_text) = match (&args.source, payload) {
        (Source::Carried, Some(payload)) => {
            let folder = root.join("downloads").join(&payload.tag);
            lock(state).source = format!("自带发布件 / carried release {}", payload.tag);
            let text = payload.members_text.clone();
            carried = Some(payload);
            (folder, text)
        }
        (Source::Online, None) => {
            let found = fetch::choose()?;
            report.line(&crate::release_line(&found))?;
            let folder = root.join("downloads").join(found.folder()?);
            let text = fetch::members(&found)?;
            lock(state).source = found.tag_name.clone();
            release = Some(found);
            (folder, text)
        }
        (Source::From(dir), None) => {
            let members = dir.join("MEMBERS.json");
            let text = std::fs::read_to_string(&members).map_err(|error| {
                format!("读取失败 / read failed: {}: {error}", members.display())
            })?;
            report.line(&format!(
                "发布件文件夹 / download folder: {}",
                dir.display()
            ))?;
            lock(state).source = format!("离线文件夹 / offline folder {}", dir.display());
            (dir.clone(), text)
        }
        _ => return Err(
            "发布件来源与安装包版本不符 / The release source does not fit this installer edition"
                .into(),
        ),
    };
    if let Some(installed) = &installed {
        let wanted = verify::members_of(&members_text)?;
        report.line(&crate::upgrade_line(installed, &wanted))?;
        if let Some(note) = upgrade::downgrade(root, &members_text) {
            if !args.allow_downgrade {
                code.set(Some(DOWNGRADE));
                return Err(format!(
                    "{note}\n需要 --allow-downgrade 才继续；安装未改动 / --allow-downgrade is needed to go on; the installation was not changed"
                ));
            }
            report.warn(&format!(
                "{note}\n已由 --allow-downgrade 确认 / confirmed by --allow-downgrade"
            ))?;
        }
    }
    let staging = root.join(format!(".staging-{}", log::unix_ms()));
    let outcome = (|| -> Result<(), String> {
        match (&release, carried.as_mut()) {
            (Some(release), _) => fetch::fetch(release, &download, report)?,
            (None, Some(payload)) => payload.extract(&download, report)?,
            (None, None) => {}
        }
        let verified = verify::run(&download, &staging, report)?;
        // From here on an interruption would leave files half written.
        let _guard = platform::InterruptGuard::start();
        lock(state).members = Some(verified.members.clone());
        if upgrading {
            let (bundles, problems) = bundle::carried(&download, report)?;
            if !problems.is_empty() {
                return Err(format!(
                    "发布件资源读取失败 / Release resources could not be read:\n{}",
                    problems.join("\n")
                ));
            }
            let mut choose =
                |association: &Association| pick(args, association, code, consulted, None);
            let mut resolve = |conflicts: &[Conflict]| answer(args, conflicts, code);
            let upgraded =
                upgrade::upgrade(root, &verified, &bundles, &mut choose, &mut resolve, report)?;
            if let Err(reason) = upgrade::record_members(root, &download) {
                report.warn(&reason)?;
            }
            let mut locked = lock(state);
            locked.laid_out = Some(upgraded.laid_out.clone());
            locked.upgraded = Some(upgraded);
        } else {
            let (laid_out, configured) = install::fresh(root, &verified, report)?;
            if let Err(reason) = upgrade::record_members(root, &download) {
                report.warn(&reason)?;
            }
            let mut locked = lock(state);
            locked.laid_out = Some(laid_out);
            locked.configured = Some(configured);
        }
        Ok(())
    })();
    if let Err(reason) = outcome {
        // A stop by a missing flag changed nothing worth recovering; anything
        // else says what was kept and where.
        return Err(match code.get() {
            Some(_) => reason,
            None => crate::left_behind(reason, root, &staging, upgrading, [false; 3]),
        });
    }
    for (alias, choice) in &args.associate {
        if !consulted.borrow().contains(alias) {
            report.warn(&format!(
                "--associate {alias}={choice} 未被用到：该实例没有需要选择的资源关联 / was not used: no association question came up for that instance"
            ))?;
        }
    }
    let summary = crate::summary(state);
    report.line(&summary)?;
    if !upgrading {
        report.line("命令行安装未创建快捷方式、开机自启与实例；需要时用向导或监控台添加 / The command line created no shortcut, autostart or instance; add them with the wizard or the console when wanted")?;
    }
    Ok(())
}

/// `--plan`: everything a real run would do and decide, from a scratch copy of
/// the release under %TEMP%; nothing under the root is written.
fn plan(args: &Args, payload: Option<Payload>) -> i32 {
    let stamp = log::unix_ms();
    let temp = std::env::temp_dir();
    let scratch = temp.join(format!("acsetup-plan-{stamp}"));
    let log = match InstallLog::create(&temp, stamp) {
        Ok(log) => log,
        Err(error) => {
            say_error(&format!(
                "失败 / FAILED: 无法创建计划日志 / Cannot create the plan's log in {}: {error}",
                temp.display()
            ));
            return FAILED;
        }
    };
    let log_path = log.path().to_path_buf();
    let state: Shared = Arc::new(Mutex::new(State {
        log: Some(log),
        ..State::default()
    }));
    let code = Cell::new(None);
    let consulted = RefCell::new(BTreeSet::new());
    let open = RefCell::new(Vec::new());
    let mut report = ConsoleReport {
        state: Arc::clone(&state),
        phase: String::new(),
    };
    let outcome = plan_logged(
        args,
        payload,
        &scratch,
        &code,
        &consulted,
        &open,
        &mut report,
    );
    if scratch.exists() {
        match std::fs::remove_dir_all(&scratch) {
            Ok(()) => {
                let _ = report.line(&format!(
                    "计划用的临时目录已删除 / The plan's scratch directory was removed: {}",
                    scratch.display()
                ));
            }
            Err(error) => {
                let _ = report.warn(&format!(
                    "计划用的临时目录未能删除 / The plan's scratch directory was not removed: {}: {error}",
                    scratch.display()
                ));
            }
        }
    }
    match outcome {
        Ok(()) => {
            say(&format!("日志 / Log: {}", log_path.display()));
            DONE
        }
        Err(reason) => {
            let _ = write_log(&state, &format!("失败 / FAILED: {reason}"));
            say_error(&format!(
                "失败 / FAILED: {reason}\n日志 / Log: {}",
                log_path.display()
            ));
            code.get().unwrap_or(FAILED)
        }
    }
}

/// Where a real run probes the current Runtime.
struct Probe {
    programs: PathBuf,
    state_root: PathBuf,
    snapshot: Option<acui_installation::Snapshot>,
}

fn plan_logged(
    args: &Args,
    payload: Option<Payload>,
    scratch: &Path,
    code: &Cell<Option<i32>>,
    consulted: &RefCell<BTreeSet<String>>,
    open: &RefCell<Vec<String>>,
    report: &mut ConsoleReport,
) -> Result<(), String> {
    let root = &args.root;
    report.line(&format!(
        "acsetup {} · 计划 / plan：安装根下不改任何文件 / nothing under the install root changes · 安装根 / install root: {}",
        env!("CARGO_PKG_VERSION"),
        root.display()
    ))?;
    let installed = upgrade::installed(root)?;
    let download = scratch.join("downloads");
    let download = match (&args.source, payload) {
        (Source::Carried, Some(mut payload)) => {
            report.line(&format!("自带发布件 / carried release {}", payload.tag))?;
            payload.extract(&download, report)?;
            download
        }
        (Source::Online, None) => {
            let release = fetch::choose()?;
            report.line(&crate::release_line(&release))?;
            fetch::fetch(&release, &download, report)?;
            download
        }
        (Source::From(dir), None) => dir.clone(),
        _ => return Err(
            "发布件来源与安装包版本不符 / The release source does not fit this installer edition"
                .into(),
        ),
    };
    let members = download.join("MEMBERS.json");
    let members_text = std::fs::read_to_string(&members)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", members.display()))?;
    let wanted = verify::members_of(&members_text)?;
    let mut needed: Vec<String> = vec!["--yes".into()];
    if let Some(installed) = &installed {
        report.line(&crate::upgrade_line(installed, &wanted))?;
        if let Some(note) = upgrade::downgrade(root, &members_text) {
            report.line(&format!(
                "{note}\n真实运行需要 --allow-downgrade / a real run needs --allow-downgrade"
            ))?;
            if !args.allow_downgrade {
                needed.push("--allow-downgrade".into());
            }
        }
    }
    let verified = verify::run(&download, &scratch.join("staging"), report)?;
    let tools = root_tools::plan(root, &verified)?;
    let active = root
        .join(acui_installation::INSTALL_SELECTION_PATH)
        .try_exists()
        .map_err(|error| format!("Cannot inspect installation selection: {error}"))?;
    let mut slots = None;
    if installed.is_none() {
        report.line(&format!(
            "全新安装 / Fresh install: 程序进槽 A / programs into slot A; 状态根 / state root {}; 不建快捷方式、开机自启与实例 / no shortcut, autostart or instance",
            root.join("state").display()
        ))?;
    } else {
        let (canonical, source_config, document, probe) = if active {
            let baseline = acui_installation::Snapshot::read(root)?;
            let target = upgrade::other(baseline.selection.slot);
            slots = Some((baseline.selection.slot, target));
            report.line(&format!(
                "A/B 升级 / A/B upgrade: 当前槽 / selected slot {} → 目标槽 / target slot {}；根工具按下列规则更新 / root tools as below",
                baseline.selection.slot.as_str(),
                target.as_str()
            ))?;
            let document: serde_json::Value = serde_json::from_slice(&baseline.config_bytes)
                .map_err(|error| error.to_string())?;
            let probe = Probe {
                programs: baseline.slot_root(),
                state_root: baseline.state_root()?,
                snapshot: None,
            };
            let source_config = baseline.config_path()?;
            let canonical = baseline.root.clone();
            (
                canonical,
                source_config,
                document,
                Probe {
                    snapshot: Some(baseline),
                    ..probe
                },
            )
        } else {
            let (runtime_sha, ui_sha) = verify::initial_programs(root, report)?;
            let canonical = std::fs::canonicalize(root)
                .map_err(|error| format!("Cannot resolve installation root: {error}"))?;
            let config = canonical.join("actingd.config.json");
            let transaction = maintenance::Transaction::read_config(&config)?;
            let state_root = transaction.state_root()?;
            report.line(&format!(
                "首次迁移 / First migration from the old layout (runtime {runtime_sha} · ui {ui_sha}): runtime\\、ui\\ 与 actingd.config.json 移入 install\\initial-backup-<代际>\\，程序进槽 A / runtime\\, ui\\ and actingd.config.json move into install\\initial-backup-<generation>\\ and the programs into slot A"
            ))?;
            let probe = Probe {
                programs: canonical.clone(),
                state_root,
                snapshot: None,
            };
            (canonical, config, transaction.document, probe)
        };
        let mut status = lifecycle::command(
            &probe.programs.join("runtime").join(runtime::ACTINGCTL),
            probe.snapshot.as_ref(),
        )?;
        status
            .arg("status")
            .arg("--state-root")
            .arg(&probe.state_root);
        report.line(&match runtime::run_observed(&mut status, Duration::from_secs(75)) {
            Ok(output) if output.success => "Runtime 正在运行：真实运行先排空并关闭它，完成后再拉起 / The Runtime is running: a real run drains and closes it first and starts it again afterwards".to_string(),
            Ok(output) => format!(
                "Runtime 未应答（退出 {}）：真实运行先做冷态账本验证 / The Runtime does not answer (exit {}): a real run verifies the ledger cold first",
                output.exit, output.exit
            ),
            Err(reason) => format!("Runtime 状态未能查询 / The Runtime's status could not be asked: {reason}"),
        })?;
        drop(probe);
        let (bundles, problems) = bundle::carried(&download, report)?;
        if !problems.is_empty() {
            return Err(format!(
                "发布件资源读取失败 / Release resources could not be read:\n{}",
                problems.join("\n")
            ));
        }
        let mut choose =
            |association: &Association| pick(args, association, code, consulted, Some(open));
        let planned = upgrade::plan_configuration(
            &canonical,
            &source_config,
            document,
            &verified,
            &bundles,
            &mut choose,
            report,
        )?;
        maintenance::list(&planned.conflicts, report)?;
        match vision_migration::plan(&canonical, &planned.document)? {
            Some(vision) => vision.describe(report)?,
            None => report.line("视觉：配置未引用 v0.3 清单，不迁移 / Vision: the configuration names no v0.3 manifest; nothing to migrate")?,
        }
        if !planned.conflicts.is_empty() {
            match args.conflicts {
                Some(Side::New) => report.line(
                    "--conflicts new：真实运行全部采用新值 / a real run takes every proposed value",
                )?,
                Some(Side::Old) => report.line(
                    "--conflicts old：真实运行全部保留旧值 / a real run keeps every current value",
                )?,
                None => needed.push("--conflicts new|old".into()),
            }
        }
        for text in open.borrow().iter() {
            report.line(text)?;
            needed.push("--associate …（见上 / see above）".into());
        }
    }
    tools.describe(report)?;
    leftovers(root, installed.is_some() && !active, slots, report)?;
    report.line(&format!(
        "真实运行需要 / A real run needs: {}",
        needed.join(" ")
    ))
}

/// What a real run does with each entry already under the root.
fn leftovers(
    root: &Path,
    migration: bool,
    slots: Option<(InstallSlot, InstallSlot)>,
    report: &mut ConsoleReport,
) -> Result<(), String> {
    let list = |dir: &Path| -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        match std::fs::read_dir(dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot read {}: {error}", dir.display())),
            Ok(entries) => {
                for entry in entries {
                    let entry =
                        entry.map_err(|error| format!("Cannot read {}: {error}", dir.display()))?;
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        Ok(names)
    };
    let names = list(root)?;
    if names.is_empty() {
        return Ok(());
    }
    report.line("安装根已有的内容 / What the root already holds:")?;
    for name in names {
        let selected = slots.map(|(selected, _)| selected.as_str());
        let target = slots.map(|(_, target)| target.as_str());
        let fate = match name.as_str() {
            "runtime" | "ui" | "actingd.config.json" if migration => {
                "移入 install\\initial-backup-<代际>\\ / moved into install\\initial-backup-<generation>\\".to_string()
            }
            "runtime" | "ui" => "固定入口，不动 / stable entries, untouched".to_string(),
            "tools" => "按根工具规则更新（见上）/ updated by the root-tool rule above".to_string(),
            "A" if migration => {
                "整体保留为 install\\retained-A-<ms>，再放入新程序 / retained whole as install\\retained-A-<ms>, then the new programs go in".to_string()
            }
            slot if Some(slot) == target => format!(
                "整体保留为 install\\retained-{slot}-<ms>，再放入新程序 / retained whole as install\\retained-{slot}-<ms>, then the new programs go in"
            ),
            slot if Some(slot) == selected => {
                "当前槽，不动 / the selected slot, untouched".to_string()
            }
            staging if staging.starts_with(".staging-") => {
                "真实运行开始时删除并记入日志 / removed, and logged, when a real run starts".to_string()
            }
            _ => "不动 / left as is".to_string(),
        };
        report.line(&format!("  {name}: {fate}"))?;
        if name == "install" {
            for inner in list(&root.join("install"))? {
                report.line(&format!("    install\\{inner}: 不动 / kept as is"))?;
            }
        }
    }
    Ok(())
}
