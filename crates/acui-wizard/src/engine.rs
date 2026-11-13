// SPDX-License-Identifier: GPL-3.0-only
//! The install, step by step (the UI channel's design, §3): checks that write nothing first —
//! the root, the install set, the plan, the emulators — then the steps that write, in order,
//! each heard as started, finished (with what it did), skipped (with why) or failed (with why).
//! A failed step stops the install; what was written stays as it is, and nothing is deleted.
//!
//! The order departs from the design in one place: the watchdog is installed after the start
//! and the first pause, since its task starts a Runtime that is down within a minute and would
//! start this one before the pause is set (Runtime `contracts/runtime-watchdog.md`).

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::archive::Archive;
use crate::config::{self, ConfigInput, ConfigInstance, CONFIG_FILE, CONFIG_PART, PACKAGES_DIR, STATE_DIR};
use crate::console;
use crate::digest::sha256_file;
use crate::log::InstallLog;
use crate::manifest::{Program, MANIFEST_FILE};
use crate::mumu;
use crate::occupancy;
use crate::plan::{plan_problems, Plan};
use crate::platform;
use crate::root::{check_root, text_issues};
use crate::runtime::{self, FIRST_START_REASON};
use crate::set::{verify_set, VerifiedSet};
use crate::text::{Lang, Said};
use crate::winpath::{last_segment, strip_verbatim};

/// The steps that write, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Prepare,
    Programs,
    Vision,
    Packs,
    PackCheck,
    Config,
    CheckConfig,
    Adopt,
    ConsoleSettings,
    Shortcuts,
    Start,
    Pause,
    Watchdog,
    Status,
}

impl Step {
    pub const ALL: [Step; 14] = [
        Step::Prepare,
        Step::Programs,
        Step::Vision,
        Step::Packs,
        Step::PackCheck,
        Step::Config,
        Step::CheckConfig,
        Step::Adopt,
        Step::ConsoleSettings,
        Step::Shortcuts,
        Step::Start,
        Step::Pause,
        Step::Watchdog,
        Step::Status,
    ];

    /// 1 to 14.
    pub fn number(self) -> usize {
        Step::ALL.iter().position(|step| *step == self).map_or(0, |at| at + 1)
    }

    pub fn title(self) -> Said {
        match self {
            Step::Prepare => Said::new("建安装文件夹和日志", "Create the install folder and the log"),
            Step::Programs => Said::new("铺 Runtime、Tools 和监控台", "Lay out the Runtime, the Tools and the console"),
            Step::Vision => Said::new("放视觉文件", "Place the vision files"),
            Step::Packs => Said::new("铺标准包", "Lay out the standard packages"),
            Step::PackCheck => Said::new("检查标准包", "Check the standard packages"),
            Step::Config => Said::new("生成配置", "Write the configuration"),
            Step::CheckConfig => Said::new("用 Runtime 校验配置", "Check the configuration with the Runtime"),
            Step::Adopt => Said::new("交给 Runtime 的安装器接管", "Hand the root to the Runtime's installer"),
            Step::ConsoleSettings => Said::new("写监控台设置", "Write the console's settings"),
            Step::Shortcuts => Said::new("建快捷方式", "Create the shortcuts"),
            Step::Start => Said::new("启动 Runtime", "Start the Runtime"),
            Step::Pause => Said::new("首次启动先暂停所有号", "Pause every account for the first start"),
            Step::Watchdog => Said::new("装看门狗（开机自动运行）", "Install the watchdog (start at logon)"),
            Step::Status => Said::new("确认 Runtime 在线", "Confirm the Runtime is online"),
        }
    }
}

/// What the install says as it goes.
#[derive(Debug, Clone)]
pub enum Event {
    Started(Step),
    Finished(Step, Said),
    Skipped(Step, Said),
    Failed(Step, Said),
    /// A line of what was done or found (also in the log).
    Line(Said),
    /// Bytes done of the current step's (or the set check's) total.
    Bytes { done: u64, total: u64 },
}

/// Why the install stopped: the step (`None` for the checks before anything is written), the
/// reason, the log when there is one, and whether anything was written.
#[derive(Debug, Clone)]
pub struct Failure {
    pub step: Option<Step>,
    pub reason: Said,
    pub log: Option<PathBuf>,
    pub wrote: bool,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.step {
            Some(step) => write!(
                formatter,
                "第 {} 步「{}」失败 / step {} \"{}\" failed: {}",
                step.number(),
                step.title().zh,
                step.number(),
                step.title().en,
                self.reason
            ),
            None => write!(formatter, "安装前的检查没有通过 / the checks before the install failed: {}", self.reason),
        }
    }
}

/// What an install asks for.
#[derive(Debug, Clone)]
pub struct Request {
    pub set_dir: PathBuf,
    pub root: PathBuf,
    pub plan: Plan,
}

/// A finished install.
#[derive(Debug, Clone)]
pub struct Done {
    pub root: PathBuf,
    pub log: PathBuf,
    pub config: PathBuf,
    pub console: PathBuf,
    pub shortcuts: Vec<PathBuf>,
    pub started: bool,
    pub paused: bool,
    pub watchdog: bool,
    /// What each step did, in order.
    pub summary: Vec<(Step, Said)>,
}

/// One instance as the configuration will bind it.
struct Bound {
    index: u16,
    alias: String,
    game: String,
    server: String,
    application_id: String,
    touch_backend: String,
    capture_backend: String,
    start_emulator: bool,
}

/// What the checks found, for the steps.
struct Checked {
    set: VerifiedSet,
    mumu_root: Option<String>,
    bound: Vec<Bound>,
}

struct Run<'a> {
    log: InstallLog,
    report: &'a mut dyn FnMut(Event),
    root: PathBuf,
    lang: Lang,
    wrote: bool,
    summary: Vec<(Step, Said)>,
}

impl Run<'_> {
    fn say(&mut self, said: Said) -> Result<(), Said> {
        self.note(&said)?;
        (self.report)(Event::Line(said));
        Ok(())
    }

    /// A line for the log only: what the step events already tell whoever listens.
    fn note(&mut self, said: &Said) -> Result<(), Said> {
        self.log
            .line(&said.to_string())
            .map_err(|error| Said::new("日志写不进去", "the log cannot be written").with(error))
    }

    fn fail(&mut self, step: Option<Step>, reason: Said) -> Failure {
        let line = match step {
            Some(step) => format!("FAILED step {} {}: {reason}", step.number(), step.title()),
            None => format!("FAILED checks: {reason}"),
        };
        // The failure is reported whether or not the log takes it.
        if let Err(error) = self.log.line(&line) {
            (self.report)(Event::Line(
                Said::new("日志写不进去", "the log cannot be written").with(error),
            ));
        }
        if let Some(step) = step {
            (self.report)(Event::Failed(step, reason.clone()));
        }
        Failure {
            step,
            reason,
            log: self.log.path().map(Path::to_path_buf),
            wrote: self.wrote,
        }
    }

    /// Runs one step: heard as started, then finished with its summary, or failed. The log
    /// gets a line at each end; listeners hear the events alone, so nothing is said twice.
    fn step(&mut self, step: Step, work: impl FnOnce(&mut Self) -> Result<Said, Said>) -> Result<(), Failure> {
        (self.report)(Event::Started(step));
        if let Err(reason) = self.note(&Said::new(
            format!("开始第 {} 步：{}", step.number(), step.title().zh),
            format!("step {} begins: {}", step.number(), step.title().en),
        )) {
            return Err(self.fail(Some(step), reason));
        }
        match work(&mut *self) {
            Ok(summary) => {
                if let Err(reason) = self.note(&Said::new(
                    format!("第 {} 步完成：{}", step.number(), summary.zh),
                    format!("step {} done: {}", step.number(), summary.en),
                )) {
                    return Err(self.fail(Some(step), reason));
                }
                self.summary.push((step, summary.clone()));
                (self.report)(Event::Finished(step, summary));
                Ok(())
            }
            Err(reason) => Err(self.fail(Some(step), reason)),
        }
    }

    fn skip(&mut self, step: Step, why: Said) -> Result<(), Failure> {
        if let Err(reason) = self.note(&Said::new(
            format!("跳过第 {} 步「{}」：{}", step.number(), step.title().zh, why.zh),
            format!("step {} \"{}\" skipped: {}", step.number(), step.title().en, why.en),
        )) {
            return Err(self.fail(Some(step), reason));
        }
        (self.report)(Event::Skipped(step, why));
        Ok(())
    }

    fn bytes(&mut self, done: u64, total: u64) {
        (self.report)(Event::Bytes { done, total });
    }
}

/// `base` joined with a `/`-separated relative path.
fn under(base: &Path, relative: &str) -> PathBuf {
    relative.split('/').fold(base.to_path_buf(), |path, segment| path.join(segment))
}

fn io_said(zh: &str, en: &str, path: &Path, error: io::Error) -> Said {
    Said::new(zh, en).with(format!("{}: {error}", path.display()))
}

/// Installs `request` (see the module's description); `report` hears every event.
pub fn install(request: &Request, report: &mut dyn FnMut(Event)) -> Result<Done, Failure> {
    let mut run = Run {
        log: InstallLog::pending(),
        report,
        root: request.root.clone(),
        lang: request.plan.lang(),
        wrote: false,
        summary: Vec::new(),
    };
    let checked = match check(request, &mut run) {
        Ok(checked) => checked,
        Err(reason) => return Err(run.fail(None, reason)),
    };
    let plan = &request.plan;
    let root = request.root.clone();

    run.step(Step::Prepare, |run| prepare(run))?;
    run.step(Step::Programs, |run| lay_programs(run, &checked.set))?;
    run.step(Step::Vision, |run| place_vision(run, &checked.set))?;
    run.step(Step::Packs, |run| lay_packs(run, &checked.set, &plan.games))?;
    run.step(Step::PackCheck, |run| check_packs(run, &plan.games))?;
    let mut part = PathBuf::new();
    run.step(Step::Config, |run| {
        let (path, summary) = write_config(run, &checked)?;
        part = path;
        Ok(summary)
    })?;
    let config_path = root.join(CONFIG_FILE);
    run.step(Step::CheckConfig, |run| check_config(run, &part, &config_path, checked.mumu_root.is_some()))?;
    run.step(Step::Adopt, |run| adopt(run))?;
    run.step(Step::ConsoleSettings, |run| {
        let path = console::write_settings(&run.root, run.lang)?;
        Ok(Said::new("已写", "written").with(path.display()))
    })?;
    let mut shortcuts = Vec::new();
    if plan.options.desktop_shortcut || plan.options.start_menu_shortcut {
        run.step(Step::Shortcuts, |run| make_shortcuts(run, plan, &mut shortcuts))?;
    } else {
        run.skip(Step::Shortcuts, Said::new("没有勾选", "not chosen"))?;
    }
    let started = plan.options.start_after_install;
    if started {
        run.step(Step::Start, |run| start(run))?;
    } else {
        run.skip(Step::Start, Said::new("没有勾选「装完就启动」", "\"start after install\" was not chosen"))?;
    }
    let paused = started && plan.options.pause_first_start;
    if paused {
        run.step(Step::Pause, |run| pause(run))?;
    } else if plan.options.pause_first_start {
        run.skip(Step::Pause, Said::new("向导没有启动 Runtime", "the wizard did not start the Runtime"))?;
    } else {
        run.skip(Step::Pause, Said::new("没有勾选", "not chosen"))?;
    }
    if plan.options.watchdog {
        run.step(Step::Watchdog, |run| watchdog(run))?;
    } else {
        run.skip(Step::Watchdog, Said::new("没有勾选（默认关）", "not chosen (off by default)"))?;
    }
    if started {
        run.step(Step::Status, |run| status(run, paused))?;
    } else {
        run.skip(Step::Status, Said::new("向导没有启动 Runtime", "the wizard did not start the Runtime"))?;
    }
    let log = run.log.path().map(Path::to_path_buf).unwrap_or_else(|| root.join(crate::log::LOG_FILE));
    // The log only: the caller says the end itself, from `Done`. A log that takes no more
    // lines now changes nothing of an install that is complete.
    let _ = run.note(&Said::new("安装完成", "the install is complete").with(root.display()));
    Ok(Done {
        console: root.join("ui").join("acui.exe"),
        config: config_path,
        log,
        shortcuts,
        started,
        paused,
        watchdog: plan.options.watchdog,
        summary: run.summary,
        root,
    })
}

/// The checks before anything is written.
fn check(request: &Request, run: &mut Run<'_>) -> Result<Checked, Said> {
    let plan = &request.plan;
    run.say(Said::new("安装开始", "install begins").with(format!(
        "{} → {}",
        request.set_dir.display(),
        request.root.display()
    )))?;
    let issues = text_issues(&request.root.to_string_lossy());
    if !issues.is_empty() {
        return Err(joined(issues.iter().map(|issue| issue.said()).collect()));
    }
    let set = {
        let report = &mut *run.report;
        let log = &mut run.log;
        let mut failed_log: Option<io::Error> = None;
        let verified = verify_set(&request.set_dir, &mut |event: Event| {
            if let Event::Line(said) = &event {
                if let Err(error) = log.line(&said.to_string()) {
                    failed_log.get_or_insert(error);
                }
            }
            report(event);
        });
        if let Some(error) = failed_log {
            return Err(Said::new("日志写不进去", "the log cannot be written").with(error));
        }
        verified?
    };
    let problems = plan_problems(plan, &set.games());
    if !problems.is_empty() {
        return Err(joined(problems));
    }
    let issues = check_root(&request.root, set.space_needed(&plan.games));
    if !issues.is_empty() {
        return Err(joined(issues.iter().map(|issue| issue.said()).collect()));
    }
    run.say(Said::new("安装位置可用", "the install folder is usable").with(request.root.display()))?;

    let mut mumu_root = plan.mumu_root.clone();
    if !plan.instances.is_empty() {
        let mut lines = Vec::new();
        let given = plan.mumu_root.as_deref().map(Path::new);
        let discovery = mumu::discover(given, &mut |said: Said| lines.push(said));
        let scanned = occupancy::scan(&mut |said: Said| lines.push(said));
        for line in lines {
            run.say(line)?;
        }
        if let Some(problem) = discovery.problem {
            return Err(problem);
        }
        let Some(found) = discovery.found else {
            return Err(Said::new("没找到 MuMu 模拟器", "no MuMu emulator was found"));
        };
        mumu_root = Some(strip_verbatim(&found.root.to_string_lossy()));
        run.say(Said::new("MuMu 实例", "MuMu instances").with(
            discovery
                .instances
                .iter()
                .map(|instance| format!("#{} {}{}", instance.index, instance.name, if instance.running { " (running)" } else { "" }))
                .collect::<Vec<_>>()
                .join(", "),
        ))?;
        for wanted in &plan.instances {
            let Some(instance) = discovery.instances.iter().find(|instance| instance.index == wanted.index) else {
                return Err(Said::new(
                    format!("MuMu 里没有序号为 {} 的实例", wanted.index),
                    format!("MuMu has no instance with index {}", wanted.index),
                ));
            };
            if let Some((other, alias)) = occupancy::used_by(&scanned, instance) {
                let whose = other
                    .root
                    .as_ref()
                    .map_or_else(|| format!("pid {}", other.pid), |root| root.display().to_string());
                if !wanted.confirmed_in_use {
                    return Err(Said::new(
                        format!(
                            "实例 #{}「{}」已被别的 ActingCommand 用着（{whose}，别名 {alias}）；确定要用就在计划里写 confirmed_in_use",
                            instance.index, instance.name
                        ),
                        format!(
                            "instance #{} \"{}\" is in use by another ActingCommand ({whose}, alias {alias}); to take it anyway the plan says confirmed_in_use",
                            instance.index, instance.name
                        ),
                    ));
                }
                run.say(Said::new("确认过：使用另一套正在用的实例", "confirmed: an instance another installation uses is taken")
                    .with(format!("#{} ({whose}, {alias})", instance.index)))?;
            }
        }
    }

    let mut bound = Vec::new();
    for wanted in &plan.instances {
        let game = set
            .bundle(&wanted.game)
            .map(|bundle| &bundle.game)
            .ok_or_else(|| Said::new("安装集里没有这个游戏", "the install set has no such game").with(&wanted.game))?;
        let server = game
            .server(&wanted.server)
            .ok_or_else(|| Said::new("这个游戏没有这个区服", "the game has no such server").with(&wanted.server))?;
        bound.push(Bound {
            index: wanted.index,
            alias: wanted.alias.clone(),
            game: wanted.game.clone(),
            server: wanted.server.clone(),
            application_id: server.application_id.clone(),
            touch_backend: wanted.touch_backend.clone(),
            capture_backend: wanted.capture_backend.clone(),
            start_emulator: wanted.start_emulator,
        });
    }
    Ok(Checked {
        set,
        mumu_root,
        bound,
    })
}

fn joined(problems: Vec<Said>) -> Said {
    let zh: Vec<String> = problems.iter().map(|problem| problem.zh.clone()).collect();
    let en: Vec<String> = problems.iter().map(|problem| problem.en.clone()).collect();
    Said::new(zh.join("；"), en.join("; "))
}

fn prepare(run: &mut Run<'_>) -> Result<Said, Said> {
    let root = run.root.clone();
    fs::create_dir_all(&root).map_err(|error| io_said("建不了安装文件夹", "the install folder cannot be created", &root, error))?;
    run.wrote = true;
    run.log
        .open(&root)
        .map_err(|error| io_said("建不了日志", "the log cannot be created", &root, error))?;
    for name in [STATE_DIR, PACKAGES_DIR] {
        let dir = root.join(name);
        fs::create_dir(&dir).map_err(|error| io_said("建不了文件夹", "the folder cannot be created", &dir, error))?;
    }
    let log = run.log.path().map(Path::to_path_buf).unwrap_or_default();
    Ok(Said::new("已建", "created").with(format!("{}, state, packages; log {}", root.display(), log.display())))
}

fn lay_programs(run: &mut Run<'_>, set: &VerifiedSet) -> Result<Said, Said> {
    let total: u64 = Program::ALL.iter().map(|program| set.program(*program).bytes).sum();
    let mut done = 0u64;
    let mut counts = Vec::new();
    for program in Program::ALL {
        let verified = set.program(program);
        let dir = run.root.join(program.dir());
        fs::create_dir(&dir).map_err(|error| io_said("建不了文件夹", "the folder cannot be created", &dir, error))?;
        let mut archive = Archive::open(&verified.zip)?;
        let mut count = 0usize;
        for file in &verified.manifest.files {
            if !program.lays(&file.path) {
                run.say(Said::new("不放（只在 A/B 安装里用）", "not laid (A/B installations only)").with(&file.path))?;
                continue;
            }
            let target = under(&dir, &file.path);
            let written = archive.extract(&file.path, &target)?;
            let (sha256, size) = sha256_file(&target, &mut |_: u64| {})
                .map_err(|error| io_said("读不回刚写的文件", "the file just written cannot be read back", &target, error))?;
            if written != file.size_bytes || size != file.size_bytes || !sha256.eq_ignore_ascii_case(&file.sha256) {
                return Err(Said::new("写出的文件与清单不符", "the file written does not match the manifest").with(format!(
                    "{}: expected {} bytes {}, found {size} bytes {sha256}",
                    target.display(),
                    file.size_bytes,
                    file.sha256
                )));
            }
            done += size;
            count += 1;
            run.bytes(done, total);
        }
        archive.extract(MANIFEST_FILE, &dir.join(MANIFEST_FILE))?;
        run.say(
            Said::new(format!("{}已铺好并逐个核对", program.label().zh), format!("{} laid out and checked", program.label().en))
                .with(format!("{} ({count} files)", dir.display())),
        )?;
        counts.push(format!("{} {count}", program.dir()));
    }
    Ok(Said::new("已铺并核对", "laid out and checked").with(counts.join(", ")))
}

fn place_vision(run: &mut Run<'_>, set: &VerifiedSet) -> Result<Said, Said> {
    let vision = run.root.join("vision");
    let total = set.vision_bytes;
    let mut done = 0u64;
    for file in &set.set.vision.files {
        let source = set.dir.join(&file.file);
        let target = under(&vision, &file.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| io_said("建不了文件夹", "the folder cannot be created", parent, error))?;
        }
        let mut input = fs::File::open(&source).map_err(|error| io_said("读不出视觉文件", "the vision file cannot be read", &source, error))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| io_said("新建不了文件", "the file cannot be created", &target, error))?;
        io::copy(&mut input, &mut output).map_err(|error| io_said("复制失败", "the copy failed", &target, error))?;
        output.sync_all().map_err(|error| io_said("写盘失败", "the file could not be written to disk", &target, error))?;
        drop(output);
        let (sha256, size) = sha256_file(&target, &mut |_: u64| {})
            .map_err(|error| io_said("读不回刚写的文件", "the file just written cannot be read back", &target, error))?;
        if size != file.size || !sha256.eq_ignore_ascii_case(&file.sha256) {
            return Err(Said::new("视觉文件与安装集不符", "the vision file does not match the install set").with(format!(
                "{}: expected {} bytes {}, found {size} bytes {sha256}",
                target.display(),
                file.size,
                file.sha256
            )));
        }
        done += size;
        run.bytes(done, total);
    }
    Ok(Said::new("已放并核对", "placed and checked").with(format!(
        "{} ({} files)",
        vision.display(),
        set.set.vision.files.len()
    )))
}

fn lay_packs(run: &mut Run<'_>, set: &VerifiedSet, games: &[String]) -> Result<Said, Said> {
    let packages = run.root.join(PACKAGES_DIR);
    let chosen: Vec<_> = games.iter().filter_map(|game| set.bundle(game)).collect();
    let total: u64 = chosen.iter().map(|bundle| bundle.bytes).sum();
    let mut done = 0u64;
    let mut laid = Vec::new();
    for bundle in chosen {
        let game_root = packages.join(&bundle.entry.game);
        fs::create_dir(&game_root).map_err(|error| io_said("建不了文件夹", "the folder cannot be created", &game_root, error))?;
        let mut archive = Archive::open(&bundle.zip)?;
        for (name, target) in &bundle.layout {
            let expected = archive.size_of(name).unwrap_or(0);
            let path = under(&game_root, target);
            let written = archive.extract(name, &path)?;
            if written != expected {
                return Err(Said::new("解出的大小与 zip 不符", "the size unpacked does not match the zip")
                    .with(format!("{}: expected {expected}, found {written}", path.display())));
            }
            done += written;
            run.bytes(done, total);
        }
        run.say(Said::new("标准包已铺", "standard package laid out").with(format!(
            "{} ← {} ({} packs)",
            game_root.display(),
            bundle.entry.file,
            bundle.index.packs.len()
        )))?;
        laid.push(bundle.entry.game.clone());
    }
    if laid.is_empty() {
        return Ok(Said::new("没有选游戏，不铺包", "no game chosen, nothing laid out"));
    }
    Ok(Said::new("已铺", "laid out").with(laid.join(", ")))
}

fn check_packs(run: &mut Run<'_>, games: &[String]) -> Result<Said, Said> {
    let root = run.root.clone();
    for game in games {
        let game_root = root.join(PACKAGES_DIR).join(game);
        let answer = runtime::standard_pack_check(&root, &game_root)?;
        run.say(Said::new("standard-pack check 答复", "standard-pack check answered").with(format!(
            "{}: {}",
            game,
            crate::child::clipped(&answer.stdout, 1500)
        )))?;
        if !answer.ok() {
            return Err(Said::new("标准包检查没有通过", "the standard package check failed")
                .with(format!("{game}: {}", answer.reason())));
        }
    }
    if games.is_empty() {
        return Ok(Said::new("没有选游戏", "no game chosen"));
    }
    Ok(Said::new("通过", "passed").with(games.join(", ")))
}

fn write_config(run: &mut Run<'_>, checked: &Checked) -> Result<(PathBuf, Said), Said> {
    let state_root = strip_verbatim(&run.root.join(STATE_DIR).to_string_lossy());
    let mut instances = Vec::new();
    for bound in &checked.bound {
        instances.push(ConfigInstance {
            alias: bound.alias.clone(),
            instance_id: config::mint_instance_id()?,
            instance_index: bound.index,
            application_id: bound.application_id.clone(),
            game: bound.game.clone(),
            server: bound.server.clone(),
            touch_backend: bound.touch_backend.clone(),
            capture_backend: bound.capture_backend.clone(),
            start_emulator: bound.start_emulator,
        });
    }
    let input = ConfigInput {
        state_root,
        salt: config::mint_salt()?,
        mumu_root: checked.mumu_root.clone(),
        instances,
    };
    let part = run.root.join(CONFIG_PART);
    write_new(&part, &config::render(&config::build(&input)))?;
    for instance in &input.instances {
        run.say(Said::new("实例", "instance").with(format!(
            "{} = MuMu #{} {}/{} {} ({} / {}{})",
            instance.alias,
            instance.instance_index,
            instance.game,
            instance.server,
            instance.instance_id,
            instance.touch_backend,
            instance.capture_backend,
            if instance.start_emulator { ", start_emulator" } else { "" }
        )))?;
    }
    Ok((
        part.clone(),
        Said::new("配置候选已写（密钥盐不记日志）", "configuration candidate written (the salt is not logged)").with(format!(
            "{} ({} instances)",
            part.display(),
            input.instances.len()
        )),
    ))
}

fn write_new(path: &Path, text: &str) -> Result<(), Said> {
    use std::io::Write;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_said("新建不了文件", "the file cannot be created", path, error))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| io_said("写不进文件", "the file cannot be written", path, error))
}

fn check_config(run: &mut Run<'_>, part: &Path, config_path: &Path, pinned: bool) -> Result<Said, Said> {
    let root = run.root.clone();
    let mut checked = runtime::check_config(&root, part)?;
    run.say(Said::new("check-config 答复", "check-config answered").with(checked.answer.reason()))?;
    if !checked.passed {
        return Err(Said::new("Runtime 不接受这份配置", "the Runtime refuses the configuration").with(checked.answer.reason()));
    }
    let mut mumu = None;
    if !pinned {
        if let Some(found) = checked.mumu_root.clone() {
            // Pin what the Runtime's own resolution found, and check the file again.
            let text = fs::read_to_string(part).map_err(|error| io_said("读不回配置候选", "the candidate cannot be read back", part, error))?;
            let document: Value = serde_json::from_str(&text)
                .map_err(|error| Said::new("配置候选读不懂", "the candidate does not parse").with(error))?;
            fs::write(part, config::render(&config::with_mumu_root(&document, &found)))
                .map_err(|error| io_said("写不进配置候选", "the candidate cannot be written", part, error))?;
            run.say(Said::new("钉上 Runtime 找到的 MuMu 位置，再校验一次", "pinned the MuMu folder the Runtime found; checking again").with(&found))?;
            checked = runtime::check_config(&root, part)?;
            run.say(Said::new("check-config 答复", "check-config answered").with(checked.answer.reason()))?;
            if !checked.passed {
                return Err(Said::new("Runtime 不接受这份配置", "the Runtime refuses the configuration").with(checked.answer.reason()));
            }
            mumu = Some(found);
        }
    }
    if config_path.exists() {
        return Err(Said::new("配置文件已经在了，向导不覆盖", "a configuration file is already there; the wizard never overwrites one")
            .with(config_path.display()));
    }
    fs::rename(part, config_path).map_err(|error| io_said("配置改不了名", "the configuration cannot be renamed into place", config_path, error))?;
    let mut summary = Said::new("通过并就位", "passed and in place").with(config_path.display());
    if let Some(found) = mumu {
        summary = summary.with(format!("mumu_root {found}"));
    }
    Ok(summary)
}

fn adopt(run: &mut Run<'_>) -> Result<Said, Said> {
    let answer = runtime::adopt(&run.root.clone())?;
    run.say(Said::new("adopt 答复", "adopt answered").with(answer.reason()))?;
    if !answer.ok() {
        return Err(Said::new("Runtime 的安装器没有接管", "the Runtime's installer did not take the root").with(answer.reason()));
    }
    Ok(Said::new("已接管", "taken").with(answer.outcome.code.clone().unwrap_or_default()))
}

fn make_shortcuts(run: &mut Run<'_>, plan: &Plan, made: &mut Vec<PathBuf>) -> Result<Said, Said> {
    let root = run.root.clone();
    let folder = last_segment(&root.to_string_lossy()).unwrap_or("ActingCommand").to_owned();
    let name = match run.lang {
        Lang::Zh => format!("ActingCommand 监控台 ({folder}).lnk"),
        Lang::En => format!("ActingCommand Console ({folder}).lnk"),
    };
    let target = root.join("ui").join("acui.exe");
    let workdir = root.join("ui");
    let mut wanted = Vec::new();
    if plan.options.desktop_shortcut {
        wanted.push(platform::desktop_dir()?);
    }
    if plan.options.start_menu_shortcut {
        wanted.push(platform::start_menu_programs_dir()?);
    }
    let mut kept = Vec::new();
    for dir in wanted {
        let lnk = dir.join(&name);
        if lnk.exists() {
            run.say(Said::new("同名快捷方式已经在了，没有改动它", "a shortcut of that name is already there and was left as it is")
                .with(lnk.display()))?;
            kept.push(lnk.display().to_string());
            continue;
        }
        platform::create_shortcut(&lnk, &target, &workdir, "ActingCommand 监控台 / ActingCommand Console")?;
        run.say(Said::new("已建快捷方式", "shortcut created").with(lnk.display()))?;
        made.push(lnk);
    }
    let mut summary = Said::new("已建", "created").with(
        made.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", "),
    );
    if !kept.is_empty() {
        summary = summary.with(format!("kept {}", kept.join(", ")));
    }
    Ok(summary)
}

fn start(run: &mut Run<'_>) -> Result<Said, Said> {
    let answer = runtime::up(&run.root.clone())?;
    run.say(Said::new("up 答复", "up answered").with(answer.reason()))?;
    if !answer.ok() {
        return Err(Said::new("Runtime 没有起来", "the Runtime did not start").with(answer.reason()));
    }
    // On a single-location root `up` is the installer's start: its result line carries the
    // Runtime's process as the outcome's `pid` value and as `start.ready.pid` (Runtime
    // `crates/lab-rescue/src/install/deploy.rs`, the start's receipt). Neither may be there (a
    // start that was found already under way); then no pid is said.
    let pid = answer
        .member("outcome")
        .and_then(|outcome| outcome["values"]["pid"].as_u64())
        .or_else(|| answer.member("start").and_then(|start| start["ready"]["pid"].as_u64()));
    let mut summary = Said::new("已就绪", "ready");
    if let Some(pid) = pid {
        summary = summary.with(format!("pid {pid}"));
    }
    if answer.outcome.category.as_deref() == Some("warning") {
        summary = summary.with(format!("caveat {}", answer.outcome.code.clone().unwrap_or_default()));
    }
    Ok(summary)
}

fn pause(run: &mut Run<'_>) -> Result<Said, Said> {
    let answer = runtime::pause_all(&run.root.clone(), FIRST_START_REASON)?;
    run.say(Said::new("pause 答复", "pause answered").with(answer.reason()))?;
    if !answer.ok() {
        return Err(Said::new("没能暂停", "the pause was refused").with(answer.reason()));
    }
    Ok(Said::new(
        "已暂停所有号；在监控台确认无误后点「全部开始执勤」",
        "every account is paused; after a look in the console, press \"put all on duty\"",
    )
    .with(format!("reason {FIRST_START_REASON}")))
}

fn watchdog(run: &mut Run<'_>) -> Result<Said, Said> {
    let answer = runtime::watchdog_install(&run.root.clone())?;
    run.say(Said::new("watchdog install 答复", "watchdog install answered").with(answer.reason()))?;
    if !answer.ok() {
        return Err(Said::new("看门狗没装上", "the watchdog was not installed").with(answer.reason()));
    }
    Ok(Said::new("已装", "installed").with(answer.outcome.code.clone().unwrap_or_default()))
}

fn status(run: &mut Run<'_>, paused: bool) -> Result<Said, Said> {
    let answer = runtime::status(&run.root.clone())?;
    run.say(Said::new("status 答复", "status answered").with(crate::child::clipped(&answer.stdout, 3000)))?;
    if !answer.ok() {
        return Err(Said::new("状态读不出", "the status cannot be read").with(answer.reason()));
    }
    let online = answer.member("daemon").and_then(|daemon| daemon["online"].as_bool()) == Some(true);
    if !online {
        return Err(Said::new("Runtime 没有应答", "the Runtime does not answer").with(answer.reason()));
    }
    let instances = answer.member("instances").and_then(Value::as_array).map_or(0, Vec::len);
    let pause = answer.member("global_pause").map(|pause| {
        pause["pause_reason"]
            .as_str()
            .or_else(|| pause["reason"].as_str())
            .unwrap_or("?")
            .to_owned()
    });
    if paused && pause.is_none() {
        return Err(Said::new(
            "Runtime 在线，但状态里看不到首次暂停",
            "the Runtime is online, but its status shows no first-start pause",
        ));
    }
    let mut summary = Said::new("在线", "online").with(format!("{instances} instances"));
    if let Some(reason) = pause {
        summary = summary.with(format!("global pause {reason}"));
    }
    Ok(summary)
}
