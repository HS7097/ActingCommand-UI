// SPDX-License-Identifier: GPL-3.0-only
//! The install, step by step (the UI channel's design, §3): checks that write nothing first —
//! the root, the install set, the plan, the emulators — then the steps that write, in order,
//! each heard as started, finished (with what it did), skipped (with why) or failed (with why).
//! A failed step stops the install; what was written stays as it is, and nothing is deleted.
//! The failure carries what the install left that a person stops or removes first ([`Left`],
//! [`cleanup`]).
//!
//! The order departs from the design in two places:
//!
//! - With instances and the first-start pause, the Runtime first starts with the same
//!   configuration without its instances, and the pause is set: a global pause stops no run
//!   already under way, so a Runtime started with its instances could dispatch before the pause
//!   holds. Step 13 then connects the instances in three parts, each answer checked: the formal
//!   close (`actingctl request-shutdown --wait 60`, `runtime_shutdown_completed`), the full
//!   configuration put in place by the Runtime's `config replace` while no Runtime runs
//!   (`runtime_running` false), and the installer's start again (`actinglab up`, exit 0, no
//!   post-start check of the failing class `fail`). The pause survives the close and is back
//!   before any instance is prepared. `config replace` on the running Runtime of a new root is
//!   refused on its `no_seeding` row although it swapped and restarted (`runtime`'s description,
//!   "Why the instances come by a close, a `config replace` and an `up`").
//! - The watchdog is installed last, after the start, the pause, the instances and the last
//!   read. It never starts a root that never started, nor one closed formally, nor one the
//!   installer is working on; it does start one whose last start ended in an error (Runtime
//!   `contracts/runtime-watchdog.md`, "Decision", rows 3, 5, 6 and 7a). Installed last, it cannot
//!   start again a Runtime whose start the install just reported failed, and an install that
//!   fails earlier leaves no task.
//!
//! The first-start pause is a promise of the options page, so no path lets a first start bring
//! the instances without it: a plan with instances that asks for the pause and leaves the
//! wizard's start out is refused (`plan_problems`), since only a running Runtime takes the
//! pause. Once a step fails, a Runtime of the root that may be running without a pause the
//! install confirmed ([`Pause`]) is closed formally before the failure is told; the failure says
//! whether it was closed, or how to close it by hand.
//!
//! Every command's answer is logged twice: its reading for people, as the window shows it, and
//! its whole stdout (up to 64 Ki characters) on a line of its own, so that a refusal's rows are
//! in the log and not only their count.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::archive::Archive;
use crate::config::{
    self, ConfigInput, ConfigInstance, CONFIG_FILE, CONFIG_PART, FULL_CONFIG, PACKAGES_DIR, STATE_DIR,
};
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
use crate::set::{verify_set, PlacedFile, VerifiedSet, DIRECTML_LIBRARY, LICENSES_DIR, VISION_LICENSES_DIR};
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
    Bind,
    Status,
    Watchdog,
}

impl Step {
    pub const ALL: [Step; 15] = [
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
        Step::Bind,
        Step::Status,
        Step::Watchdog,
    ];

    /// 1 to 15.
    pub fn number(self) -> usize {
        Step::ALL.iter().position(|step| *step == self).map_or(0, |at| at + 1)
    }

    pub fn title(self) -> Said {
        match self {
            Step::Prepare => Said::new("建安装文件夹和日志", "Create the install folder and the log"),
            Step::Programs => Said::new("铺 Runtime、Tools 和监控台", "Lay out the Runtime, the Tools and the console"),
            Step::Vision => Said::new("放视觉文件", "Place the vision files"),
            Step::Packs => Said::new("铺标准包和随带的许可文件", "Lay out the standard packages and their licence files"),
            Step::PackCheck => Said::new("检查标准包", "Check the standard packages"),
            Step::Config => Said::new("生成配置", "Write the configuration"),
            Step::CheckConfig => Said::new("用 Runtime 校验配置", "Check the configuration with the Runtime"),
            Step::Adopt => Said::new("交给 Runtime 的安装器接管", "Hand the root to the Runtime's installer"),
            Step::ConsoleSettings => Said::new("写监控台设置", "Write the console's settings"),
            Step::Shortcuts => Said::new("建快捷方式", "Create the shortcuts"),
            Step::Start => Said::new("启动 Runtime", "Start the Runtime"),
            Step::Pause => Said::new("首次启动先暂停所有实例", "Pause every instance for the first start"),
            Step::Bind => Said::new(
                "接上实例：先停下 Runtime，换上带实例的配置，再启动",
                "Connect the instances: stop the Runtime, put the configuration with the instances in place, start it again",
            ),
            Step::Status => Said::new("确认 Runtime 在线", "Confirm the Runtime is online"),
            Step::Watchdog => Said::new("装看门狗（开机自动运行）", "Install the watchdog (start at logon)"),
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
    /// What the checks before the install just did, or which part of a step of several parts
    /// runs now (step 13's close, swap and start), in a person's words: no path, repository or
    /// commit (the `Line` beside it carries those, for the log and the command line). The
    /// window shows these as they come; nothing writes them down.
    Plain(Said),
    /// Bytes done of the current step's (or the set check's) total.
    Bytes { done: u64, total: u64 },
}

/// The first-start pause as the install last knew it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pause {
    /// Not asked for, or not set yet.
    #[default]
    None,
    /// Set, and no check since found it missing: a runtime fact the root's next start restores.
    Held,
    /// Set, then not confirmed: the start that brought the instances failed (its `pauses` check
    /// among what may have failed), or the last step found no first-start pause or a task under
    /// way. The Runtime may be playing the games.
    Unconfirmed,
}

/// What the install did that outlives its folder or keeps running: what a person stops or
/// removes before the folder can be deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Left {
    /// A Runtime of the root may be running: the wizard ran its start (a start that did not
    /// answer in time, or failed, may have left it running) and has not seen it closed since.
    pub running: bool,
    /// The wizard closed the Runtime it had started (`runtime_shutdown_completed`: step 13's
    /// formal close, or the close after a failure) and has not started it again.
    pub closed: bool,
    /// The close was the one after a failure ([`Run::stop_after_failure`]).
    pub stopped_on_failure: bool,
    /// The first-start pause.
    pub pause: Pause,
    /// The wizard ran `watchdog install`: the root's watchdog task may be registered.
    pub watchdog: bool,
    /// The shortcuts the wizard created.
    pub shortcuts: Vec<PathBuf>,
}

/// What a person does about what a failed install left, in order, each with the command or the
/// button that does it: stop the Runtime it started (the console's "Request Shutdown", or
/// `actingctl request-shutdown`), or, when the wizard closed it itself, that there is none to
/// stop; remove the watchdog task it registered (`actingctl watchdog uninstall`), delete the
/// shortcuts it created; then delete the folder. Only what the install did is named.
pub fn cleanup(root: &Path, left: &Left) -> Vec<Said> {
    let root = strip_verbatim(&root.to_string_lossy()).trim_end_matches(['\\', '/']).to_owned();
    let actingctl = format!("\"{root}\\runtime\\actingctl.exe\"");
    let mut steps = Vec::new();
    if left.running {
        let zh = format!(
            "打开 {root}\\ui\\acui.exe 按「请求关闭」，或者运行 {actingctl} request-shutdown --state-root \"{root}\\state\""
        );
        let en = format!(
            "open {root}\\ui\\acui.exe and press \"Request Shutdown\", or run {actingctl} request-shutdown --state-root \"{root}\\state\""
        );
        steps.push(match left.pause {
            Pause::Held => Said::new(
                format!("ActingCommand 已经启动过，挂着首次暂停，可能还开着。先停下它：{zh}。"),
                format!("ActingCommand has been started, under the first-start pause, and may still be running. Stop it first: {en}."),
            ),
            Pause::Unconfirmed => Said::new(
                format!("ActingCommand 可能还开着，首次暂停没有确认，它可能已经在操作游戏。先停下它：{zh}。"),
                format!("ActingCommand may still be running, and its first-start pause is not confirmed: it may be playing the games already. Stop it first: {en}."),
            ),
            Pause::None => Said::new(
                format!("ActingCommand 可能还开着，而且没有暂停。先停下它：{zh}。"),
                format!("ActingCommand may still be running, and it is not paused. Stop it first: {en}."),
            ),
        });
    } else if left.closed {
        let (zh, en) = if left.stopped_on_failure {
            (
                "ActingCommand 已经停下了（安装没有通过，向导正式关闭了它），不用再停",
                "ActingCommand is already stopped (the install failed, and the wizard closed it formally), so there is nothing to stop",
            )
        } else {
            (
                "ActingCommand 已经停下了（向导换配置之前正式关闭了它），不用再停",
                "ActingCommand is already stopped (the wizard closed it formally before switching the configuration), so there is nothing to stop",
            )
        };
        steps.push(match left.pause {
            Pause::Held => Said::new(
                format!("{zh}；首次暂停记在它的运行记录里，下次启动时会先恢复。"),
                format!("{en}; the first-start pause is kept in its records and comes back first at its next start."),
            ),
            Pause::Unconfirmed => Said::new(
                format!("{zh}；首次暂停没有确认：下次启动以后，先在监控台看它是不是暂停着。"),
                format!("{en}; the first-start pause is not confirmed: after its next start, look in the console whether it is paused."),
            ),
            Pause::None => Said::new(format!("{zh}。"), format!("{en}.")),
        });
    }
    if left.watchdog {
        steps.push(Said::new(
            format!("关掉开机自动运行（看门狗）：运行 {actingctl} watchdog uninstall --root \"{root}\"。"),
            format!("Turn the automatic start (the watchdog) off: run {actingctl} watchdog uninstall --root \"{root}\"."),
        ));
    }
    if !left.shortcuts.is_empty() {
        let list: Vec<String> = left.shortcuts.iter().map(|path| path.display().to_string()).collect();
        steps.push(Said::new(
            format!("删掉向导建的快捷方式：{}。", list.join("、")),
            format!("Delete the shortcuts the wizard created: {}.", list.join(", ")),
        ));
    }
    steps.push(if steps.is_empty() {
        Said::new(
            format!("可以删除整个 {root} 文件夹，再重新运行向导。"),
            format!("The whole {root} folder may be deleted and the wizard run again."),
        )
    } else {
        Said::new(
            format!("做完上面几项，再删除整个 {root} 文件夹，就可以重新运行向导。"),
            format!("Then delete the whole {root} folder and run the wizard again."),
        )
    });
    steps
}

/// The name of the console's shortcut for the root folder named `folder`: attempt 1 is the plain
/// name, each later one adds its number, for when a shortcut of that name is already there.
pub fn shortcut_name(lang: Lang, folder: &str, attempt: u32) -> String {
    let label = if attempt <= 1 { folder.to_owned() } else { format!("{folder} {attempt}") };
    match lang {
        Lang::Zh => format!("ActingCommand 监控台（{label}）.lnk"),
        Lang::En => format!("ActingCommand Console ({label}).lnk"),
    }
}

/// Why the install stopped: the step (`None` for the checks before anything is written), the
/// reason, the log when there is one, whether anything was written, and what was left.
#[derive(Debug, Clone)]
pub struct Failure {
    pub step: Option<Step>,
    pub reason: Said,
    pub log: Option<PathBuf>,
    pub wrote: bool,
    pub left: Left,
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
    /// How many instances the configuration binds.
    pub instances: usize,
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
    left: Left,
    summary: Vec<(Step, Said)>,
}

impl Run<'_> {
    fn say(&mut self, said: Said) -> Result<(), Said> {
        self.note(&said)?;
        (self.report)(Event::Line(said));
        Ok(())
    }

    /// A command's answer: `said` with [`runtime::Answer::reason`] to the log and whoever
    /// listens, then the answer's whole stdout ([`runtime::Answer::log_stdout`]) on a line of its
    /// own in the log only.
    fn answered(&mut self, said: Said, answer: &runtime::Answer) -> Result<(), Said> {
        self.say(said.with(answer.reason()))?;
        if let Some(stdout) = answer.log_stdout() {
            self.log
                .line(&format!("  {} stdout: {stdout}", answer.command))
                .map_err(|error| Said::new("日志写不进去", "the log cannot be written").with(error))?;
        }
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
        self.stop_after_failure();
        Failure {
            step,
            reason,
            log: self.log.path().map(Path::to_path_buf),
            wrote: self.wrote,
            left: self.left.clone(),
        }
    }

    /// After a failure, a Runtime of the root that may be running without a pause the install
    /// confirmed ([`Pause::Held`]) is closed formally (`request-shutdown`, judged as step 13's
    /// close is): a failed install leaves no Runtime playing the games. A close that is refused
    /// or fails leaves `running` set, so the failure says how to stop it by hand ([`cleanup`]).
    fn stop_after_failure(&mut self) {
        if !self.left.running || self.left.pause == Pause::Held {
            return;
        }
        self.plain(Said::new(
            "安装没有通过，正在停下向导启动的 ActingCommand……",
            "The install failed; stopping the ActingCommand the wizard started…",
        ));
        let root = self.root.clone();
        let closed = runtime::request_shutdown(&root).and_then(|closed| {
            if let Err(error) = self.answered(Said::new("request-shutdown 答复", "request-shutdown answered"), &closed) {
                (self.report)(Event::Line(error));
            }
            runtime::judge_shutdown(closed.code, closed.line.as_ref()).map(|_| ()).map_err(|problem| problem.with(closed.reason()))
        });
        let said = match closed {
            Ok(()) => {
                self.left.running = false;
                self.left.closed = true;
                self.left.stopped_on_failure = true;
                Said::new("安装没有通过，向导已经停下它启动的 ActingCommand", "the install failed; the wizard closed the ActingCommand it started")
            }
            Err(problem) => Said::new(
                "安装没有通过，向导也没能停下它启动的 ActingCommand，请照下面的办法手工停",
                "the install failed, and the wizard could not close the ActingCommand it started; stop it by hand as below",
            )
            .with_said(&problem),
        };
        if let Err(error) = self.note(&said) {
            (self.report)(Event::Line(error));
        }
        (self.report)(Event::Line(said));
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

    fn plain(&mut self, said: Said) {
        (self.report)(Event::Plain(said));
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
        left: Left::default(),
        summary: Vec::new(),
    };
    let checked = match check(request, &mut run) {
        Ok(checked) => checked,
        Err(reason) => return Err(run.fail(None, reason)),
    };
    let plan = &request.plan;
    let root = request.root.clone();
    let started = plan.options.start_after_install;
    let paused = started && plan.options.pause_first_start;
    // The Runtime starts without its instances, which come once the pause holds.
    let staged = paused && !checked.bound.is_empty();
    let aliases: Vec<String> = checked.bound.iter().map(|bound| bound.alias.clone()).collect();

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
    let full_path = staged.then(|| root.join(FULL_CONFIG));
    run.step(Step::CheckConfig, |run| {
        check_config(run, &part, &config_path, checked.mumu_root.is_some(), full_path.as_deref())
    })?;
    run.step(Step::Adopt, |run| adopt(run))?;
    run.step(Step::ConsoleSettings, |run| {
        let path = console::write_settings(&run.root, run.lang)?;
        Ok(Said::new("已写", "written").with(path.display()))
    })?;
    if plan.options.desktop_shortcut || plan.options.start_menu_shortcut {
        run.step(Step::Shortcuts, |run| make_shortcuts(run, plan))?;
    } else {
        run.skip(Step::Shortcuts, Said::new("没有勾选", "not chosen"))?;
    }
    if started {
        run.step(Step::Start, |run| start(run))?;
    } else {
        run.skip(Step::Start, Said::new("没有勾选「装完就启动」", "\"start after install\" was not chosen"))?;
    }
    if paused {
        run.step(Step::Pause, |run| pause(run))?;
    } else if plan.options.pause_first_start {
        run.skip(Step::Pause, Said::new("向导没有启动 Runtime", "the wizard did not start the Runtime"))?;
    } else {
        run.skip(Step::Pause, Said::new("没有勾选", "not chosen"))?;
    }
    match &full_path {
        Some(full) => run.step(Step::Bind, |run| bind(run, full, aliases.len()))?,
        None if aliases.is_empty() => run.skip(Step::Bind, Said::new("没有实例", "no instance"))?,
        None => run.skip(
            Step::Bind,
            Said::new(
                format!("实例已随第 {} 步的配置就位", Step::CheckConfig.number()),
                format!("the instances came with the configuration of step {}", Step::CheckConfig.number()),
            ),
        )?,
    }
    if started {
        run.step(Step::Status, |run| status(run, paused, &aliases))?;
    } else {
        run.skip(Step::Status, Said::new("向导没有启动 Runtime", "the wizard did not start the Runtime"))?;
    }
    if plan.options.watchdog {
        run.step(Step::Watchdog, |run| watchdog(run))?;
    } else {
        run.skip(Step::Watchdog, Said::new("没有勾选（默认关）", "not chosen (off by default)"))?;
    }
    let log = run.log.path().map(Path::to_path_buf).unwrap_or_else(|| root.join(crate::log::LOG_FILE));
    // The log only: the caller says the end itself, from `Done`. A log that takes no more
    // lines now changes nothing of an install that is complete.
    let _ = run.note(&Said::new("安装完成", "the install is complete").with(root.display()));
    Ok(Done {
        console: root.join("ui").join("acui.exe"),
        config: config_path,
        log,
        shortcuts: run.left.shortcuts.clone(),
        started,
        paused,
        watchdog: plan.options.watchdog,
        instances: aliases.len(),
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
    run.plain(Said::new("已核对：安装位置可以用", "Checked: the install folder can be used"));

    let mut mumu_root = plan.mumu_root.clone();
    if !plan.instances.is_empty() {
        run.plain(Said::new("正在核对模拟器……", "Checking the emulators…"));
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
        run.say(Said::new("MuMu 里的模拟器", "the emulators in MuMu").with(
            discovery
                .instances
                .iter()
                .map(|instance| {
                    let port = instance.adb_port.map_or_else(String::new, |port| format!(" adb {port}"));
                    format!("#{} {}{}{port}", instance.index, instance.name, if instance.running { " (running)" } else { "" })
                })
                .collect::<Vec<_>>()
                .join(", "),
        ))?;
        for wanted in &plan.instances {
            let Some(instance) = discovery.instances.iter().find(|instance| instance.index == wanted.index) else {
                return Err(Said::new(
                    format!("MuMu 里没有序号为 {} 的模拟器", wanted.index),
                    format!("MuMu has no emulator with index {}", wanted.index),
                ));
            };
            if let Some((other, alias)) = occupancy::used_by(&scanned, instance) {
                let whose = other.name();
                if !wanted.confirmed_in_use {
                    return Err(Said::new(
                        format!(
                            "模拟器 #{}「{}」已被别的 ActingCommand 用着（{whose}，别名 {alias}）；确定要用就在计划里写 confirmed_in_use",
                            instance.index, instance.name
                        ),
                        format!(
                            "emulator #{} \"{}\" is in use by another ActingCommand ({whose}, alias {alias}); to take it anyway the plan says confirmed_in_use",
                            instance.index, instance.name
                        ),
                    ));
                }
                run.say(Said::new("确认过：使用另一套正在用的模拟器", "confirmed: an emulator another installation uses is taken")
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

/// The set's file `placed` copied from `source` to `target`, a new file (its folders created),
/// then read back against the set's size and SHA-256; returns the bytes written.
fn copy_checked(source: &Path, target: &Path, placed: &PlacedFile) -> Result<u64, Said> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| io_said("建不了文件夹", "the folder cannot be created", parent, error))?;
    }
    let mut input = fs::File::open(source).map_err(|error| io_said("读不出安装集里的文件", "a file of the install set cannot be read", source, error))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| io_said("新建不了文件", "the file cannot be created", target, error))?;
    io::copy(&mut input, &mut output).map_err(|error| io_said("复制失败", "the copy failed", target, error))?;
    output.sync_all().map_err(|error| io_said("写盘失败", "the file could not be written to disk", target, error))?;
    drop(output);
    let (sha256, size) = sha256_file(target, &mut |_: u64| {})
        .map_err(|error| io_said("读不回刚写的文件", "the file just written cannot be read back", target, error))?;
    if size != placed.size || !sha256.eq_ignore_ascii_case(&placed.sha256) {
        return Err(Said::new("放好的文件与安装集不符", "the file placed does not match the install set").with(format!(
            "{}: expected {} bytes {}, found {size} bytes {sha256}",
            target.display(),
            placed.size,
            placed.sha256
        )));
    }
    Ok(size)
}

/// The vision files below `<root>\vision` (DirectML's with them when the set carries it), then
/// the vision models' licences below `<root>\licenses\vision`, each read back against the set.
fn place_vision(run: &mut Run<'_>, set: &VerifiedSet) -> Result<Said, Said> {
    let vision = run.root.join("vision");
    let total = set.vision_bytes;
    let mut done = 0u64;
    for file in &set.set.vision.files {
        done += copy_checked(&set.dir.join(&file.file), &under(&vision, &file.path), file)?;
        run.bytes(done, total);
    }
    if set.set.carries_directml() {
        run.say(Said::new(
            "已放 DirectML 和它的许可文件（这一版配置仍用 CPU）",
            "DirectML and its licence files placed (this version's configuration still uses the CPU)",
        )
        .with(under(&vision, DIRECTML_LIBRARY).display()))?;
    }
    let licences = run.root.join(LICENSES_DIR).join(VISION_LICENSES_DIR);
    for licence in &set.set.vision.licenses {
        let target = under(&licences, &licence.path);
        done += copy_checked(&set.dir.join(&licence.file), &target, licence)?;
        run.say(Said::new("已放视觉模型的许可文件", "a vision model's licence file placed").with(format!(
            "{} ← {}",
            target.display(),
            licence.file
        )))?;
        run.bytes(done, total);
    }
    let mut summary = Said::new("已放并核对", "placed and checked").with(format!(
        "{} ({} files)",
        vision.display(),
        set.set.vision.files.len()
    ));
    if !set.set.vision.licenses.is_empty() {
        summary = summary.with(format!("{} ({} licence files)", licences.display(), set.set.vision.licenses.len()));
    }
    Ok(summary)
}

fn lay_packs(run: &mut Run<'_>, set: &VerifiedSet, games: &[String]) -> Result<Said, Said> {
    let packages = run.root.join(PACKAGES_DIR);
    let chosen: Vec<_> = games.iter().filter_map(|game| set.bundle(game)).collect();
    let total: u64 = chosen
        .iter()
        .map(|bundle| bundle.bytes + bundle.entry.files.iter().map(|file| file.size).sum::<u64>())
        .sum();
    let mut done = 0u64;
    let mut laid = Vec::new();
    for bundle in chosen {
        // The zip once more against the set, right before it is laid: the set's folder is not
        // the wizard's, and the check before the install was a while ago.
        let (sha256, size) = sha256_file(&bundle.zip, &mut |_: u64| {})
            .map_err(|error| io_said("读不出标准包 zip", "the standard package zip cannot be read", &bundle.zip, error))?;
        if size != bundle.entry.size || !sha256.eq_ignore_ascii_case(&bundle.entry.sha256) {
            return Err(Said::new(
                "标准包 zip 与安装集不符（核对之后被改过）",
                "the standard package zip does not match the install set (it changed after the check)",
            )
            .with(format!(
                "{}: expected {} bytes {}, found {size} bytes {sha256}",
                bundle.zip.display(),
                bundle.entry.size,
                bundle.entry.sha256
            )));
        }
        run.say(Said::new("标准包 zip 铺之前又核了一次 SHA-256", "the standard package zip's SHA-256 checked again before it is laid")
            .with(format!("{} {sha256}", bundle.entry.file)))?;
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
        let licenses = run.root.join(LICENSES_DIR).join(&bundle.entry.game);
        for file in &bundle.entry.files {
            let target = under(&licenses, &file.path);
            done += copy_checked(&set.dir.join(&file.file), &target, file)?;
            run.say(Said::new("已放标准包随带的文件", "a standard package's own file placed").with(format!(
                "{} ← {}",
                target.display(),
                file.file
            )))?;
            run.bytes(done, total);
        }
        laid.push(match bundle.entry.files.len() {
            0 => bundle.entry.game.clone(),
            count => format!("{} (+{count} licence files)", bundle.entry.game),
        });
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
        run.answered(Said::new("standard-pack check 答复", "standard-pack check answered").with(game), &answer)?;
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

/// `check-config` of the candidate at `part`, with the MuMu folder the Runtime found pinned when
/// none was given, then put in place. With `full` (instances and the first-start pause), the
/// checked candidate is kept there instead, and the same configuration without its instances is
/// checked and put in place for the first start.
fn check_config(run: &mut Run<'_>, part: &Path, config_path: &Path, pinned: bool, full: Option<&Path>) -> Result<Said, Said> {
    let root = run.root.clone();
    let mut checked = runtime::check_config(&root, part)?;
    run.answered(Said::new("check-config 答复", "check-config answered"), &checked.answer)?;
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
            run.answered(Said::new("check-config 答复", "check-config answered"), &checked.answer)?;
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
    let mut summary = match full {
        None => {
            fs::rename(part, config_path)
                .map_err(|error| io_said("配置改不了名", "the configuration cannot be renamed into place", config_path, error))?;
            Said::new("通过并就位", "passed and in place").with(config_path.display())
        }
        Some(full) => {
            if full.exists() {
                return Err(Said::new("完整配置的文件已经在了，向导不覆盖", "the full configuration's file is already there; the wizard never overwrites one")
                    .with(full.display()));
            }
            fs::rename(part, full)
                .map_err(|error| io_said("完整配置改不了名", "the full configuration cannot be renamed", full, error))?;
            run.say(Said::new("带实例的完整配置已通过，先留着，暂停之后再换上", "the full configuration with the instances passed; it waits until the pause holds")
                .with(full.display()))?;
            let text = fs::read_to_string(full).map_err(|error| io_said("读不回完整配置", "the full configuration cannot be read back", full, error))?;
            let document: Value = serde_json::from_str(&text)
                .map_err(|error| Said::new("完整配置读不懂", "the full configuration does not parse").with(error))?;
            write_new(part, &config::render(&config::without_instances(&document)))?;
            let first = runtime::check_config(&root, part)?;
            run.answered(
                Said::new("check-config 答复（首次启动用的配置，不带实例）", "check-config answered (the first start's configuration, without instances)"),
                &first.answer,
            )?;
            if !first.passed {
                return Err(Said::new("Runtime 不接受首次启动用的配置", "the Runtime refuses the first start's configuration").with(first.answer.reason()));
            }
            fs::rename(part, config_path)
                .map_err(|error| io_said("配置改不了名", "the configuration cannot be renamed into place", config_path, error))?;
            Said::new(
                "通过；先放不带实例的配置，带实例的完整配置等暂停之后换上",
                "passed; the configuration without instances is in place, the full one waits for the pause",
            )
            .with(format!("{}, {}", config_path.display(), full.display()))
        }
    };
    if let Some(found) = mumu {
        summary = summary.with(format!("mumu_root {found}"));
    }
    Ok(summary)
}

fn adopt(run: &mut Run<'_>) -> Result<Said, Said> {
    let answer = runtime::adopt(&run.root.clone())?;
    run.answered(Said::new("adopt 答复", "adopt answered"), &answer)?;
    if !answer.ok() {
        return Err(Said::new("Runtime 的安装器没有接管", "the Runtime's installer did not take the root").with(answer.reason()));
    }
    Ok(Said::new("已接管", "taken").with(answer.outcome.code.clone().unwrap_or_default()))
}

/// The shortcuts asked for, each under a name of its own that names the root folder
/// ([`shortcut_name`]): a shortcut already there under a name is never touched, the next name is
/// taken.
fn make_shortcuts(run: &mut Run<'_>, plan: &Plan) -> Result<Said, Said> {
    let root = run.root.clone();
    let folder = last_segment(&root.to_string_lossy()).unwrap_or("ActingCommand").to_owned();
    let target = root.join("ui").join("acui.exe");
    let workdir = root.join("ui");
    let mut wanted = Vec::new();
    if plan.options.desktop_shortcut {
        wanted.push(platform::desktop_dir()?);
    }
    if plan.options.start_menu_shortcut {
        wanted.push(platform::start_menu_programs_dir()?);
    }
    let mut made = Vec::new();
    for dir in wanted {
        let Some(lnk) = (1..=99u32)
            .map(|attempt| dir.join(shortcut_name(run.lang, &folder, attempt)))
            .find(|lnk| fs::symlink_metadata(lnk).is_err())
        else {
            return Err(Said::new("快捷方式的名字都已被占用", "every name for the shortcut is taken").with(dir.display()));
        };
        platform::create_shortcut(&lnk, &target, &workdir, "ActingCommand 监控台 / ActingCommand Console")?;
        run.left.shortcuts.push(lnk.clone());
        run.say(Said::new("已建快捷方式", "shortcut created").with(lnk.display()))?;
        made.push(lnk.display().to_string());
    }
    Ok(Said::new("已建", "created").with(made.join(", ")))
}

/// `actinglab up`, judged by [`runtime::judge_up`]: exit 0 and no post-start check of the
/// failing class `fail`; one outside that class that fails (a slow ledger open) is noted. The
/// Runtime counts as maybe running from the moment it is asked to start.
fn up(run: &mut Run<'_>, failed: Said) -> Result<Said, Said> {
    run.left.running = true;
    run.left.closed = false;
    let answer = runtime::up(&run.root.clone())?;
    run.answered(Said::new("up 答复", "up answered"), &answer)?;
    let read = runtime::judge_up(answer.code, answer.line.as_ref())
        .map_err(|problem| failed.with_said(&problem).with(answer.reason()))?;
    let mut summary = Said::new("已就绪", "ready").with(format!(
        "{} post-start checks, none of the failing class failed",
        read.checks.unwrap_or_default()
    ));
    if !read.failed.is_empty() {
        summary = summary.with(format!("noted: {}", runtime::failed_checks_text(&read.failed)));
    }
    if let Some(pid) = read.pid {
        summary = summary.with(format!("pid {pid}"));
    }
    if let Some(caveat) = read.caveat {
        summary = summary.with(format!("caveat {caveat}"));
    }
    Ok(summary)
}

fn start(run: &mut Run<'_>) -> Result<Said, Said> {
    up(run, Said::new("Runtime 没有正常起来", "the Runtime did not start cleanly"))
}

fn pause(run: &mut Run<'_>) -> Result<Said, Said> {
    let answer = runtime::pause_all(&run.root.clone(), FIRST_START_REASON)?;
    run.answered(Said::new("pause 答复", "pause answered"), &answer)?;
    if !answer.ok() {
        return Err(Said::new("没能暂停", "the pause was refused").with(answer.reason()));
    }
    run.left.pause = Pause::Held;
    Ok(Said::new(
        "已暂停所有实例；在监控台确认无误后点「全部开始执勤」",
        "every instance is paused; after a look in the console, press \"put all on duty\"",
    )
    .with(format!("reason {FIRST_START_REASON}")))
}

/// Step 13 (see the module's description), three parts, each answer judged: the formal close
/// ([`runtime::judge_shutdown`]); the full configuration at `full` put in place by the Runtime's
/// `config replace` (bound to its SHA-256) while no Runtime runs ([`runtime::judge_replace`]),
/// and the file in place read back; the installer's start again ([`runtime::judge_up`]), which
/// restores the pause before any instance is prepared.
fn bind(run: &mut Run<'_>, full: &Path, count: usize) -> Result<Said, Said> {
    let root = run.root.clone();
    let (sha256, _) = sha256_file(full, &mut |_: u64| {})
        .map_err(|error| io_said("读不出完整配置", "the full configuration cannot be read", full, error))?;

    run.plain(Said::new("正在停下 ActingCommand……", "Stopping ActingCommand…"));
    let closed = runtime::request_shutdown(&root)?;
    run.answered(Said::new("request-shutdown 答复", "request-shutdown answered"), &closed)?;
    runtime::judge_shutdown(closed.code, closed.line.as_ref()).map_err(|problem| {
        Said::new("没能先停下 Runtime", "the Runtime could not be closed first")
            .with_said(&problem)
            .with(closed.reason())
    })?;
    run.left.running = false;
    run.left.closed = true;

    run.plain(Said::new("正在换上带实例的设置……", "Switching to the settings with the instances…"));
    let swapped = runtime::config_replace(&root, full, &sha256)?;
    run.answered(Said::new("config replace 答复", "config replace answered"), &swapped)?;
    let replaced = runtime::judge_replace(swapped.code, swapped.line.as_ref()).map_err(|problem| {
        Said::new("没能换上带实例的配置", "the configuration with the instances was not put in place")
            .with_said(&problem)
            .with(swapped.reason())
    })?;
    let live = root.join(CONFIG_FILE);
    let (now, _) = sha256_file(&live, &mut |_: u64| {})
        .map_err(|error| io_said("读不出配置文件", "the configuration cannot be read", &live, error))?;
    if !now.eq_ignore_ascii_case(&sha256) {
        return Err(Said::new("就位的配置不是带实例的那一份", "the configuration in place is not the one with the instances")
            .with(format!("{}: {now}, expected {sha256}", live.display())));
    }
    run.say(Said::new("带实例的配置已换上（Runtime 停着）", "the configuration with the instances is in place (no Runtime running)")
        .with(format!(
            "{}, replaced file kept as {}",
            replaced.code.as_deref().unwrap_or("?"),
            replaced.backup.as_deref().unwrap_or("?")
        )))?;

    run.plain(Said::new("正在重新启动 ActingCommand……", "Starting ActingCommand again…"));
    // The start brings the instances; until its checks pass (`pauses` among them), the pause
    // is not confirmed.
    run.left.pause = Pause::Unconfirmed;
    let started = up(run, Said::new("换上配置以后 Runtime 没有正常起来", "the Runtime did not start cleanly with the new configuration"))?;
    run.left.pause = Pause::Held;
    Ok(Said::new(
        "已停下、换上带实例的配置、再启动",
        "closed, the configuration with the instances put in place, started again",
    )
    .with(format!("{count} instances"))
    .with_said(&started))
}

fn watchdog(run: &mut Run<'_>) -> Result<Said, Said> {
    run.left.watchdog = true;
    let answer = runtime::watchdog_install(&run.root.clone())?;
    run.answered(Said::new("watchdog install 答复", "watchdog install answered"), &answer)?;
    if !answer.ok() {
        return Err(Said::new("看门狗没装上", "the watchdog was not installed").with(answer.reason()));
    }
    Ok(Said::new("已装", "installed").with(answer.outcome.code.clone().unwrap_or_default()))
}

/// The last read: the Runtime answers; with the pause, the pause holds and no instance runs a
/// task (a run under way means the pause came too late: that is a failure, never a success);
/// every instance the configuration binds is there.
fn status(run: &mut Run<'_>, paused: bool, aliases: &[String]) -> Result<Said, Said> {
    // Until this read shows it whole, the pause is not confirmed: a failure here (no answer, no
    // first-start pause, a task under way, an instance missing) closes the Runtime
    // ([`Run::stop_after_failure`]).
    if paused {
        run.left.pause = Pause::Unconfirmed;
    }
    let answer = runtime::status(&run.root.clone())?;
    run.answered(Said::new("status 答复", "status answered"), &answer)?;
    if !answer.ok() {
        return Err(Said::new("状态读不出", "the status cannot be read").with(answer.reason()));
    }
    let Some(line) = answer.line.as_ref() else {
        return Err(Said::new("状态读不懂", "the status does not parse").with(answer.reason()));
    };
    let read = runtime::read_status(line);
    if !read.online {
        return Err(Said::new("Runtime 没有应答", "the Runtime does not answer").with(answer.reason()));
    }
    if paused {
        match read.global_pause.as_deref() {
            Some(FIRST_START_REASON) => {}
            Some(other) => {
                return Err(Said::new(
                    "Runtime 在线，但全局暂停不是首次暂停",
                    "the Runtime is online, but its global pause is not the first-start pause",
                )
                .with(format!("reason {other}")))
            }
            None => {
                return Err(Said::new(
                    "Runtime 在线，但状态里看不到首次暂停",
                    "the Runtime is online, but its status shows no first-start pause",
                ))
            }
        }
    }
    if paused && !read.running.is_empty() {
        return Err(Said::new(
            "暂停之后仍有任务在跑：ActingCommand 已经在操作游戏，首次暂停没有挡住",
            "tasks run after the pause: ActingCommand is already operating the games, and the first-start pause did not hold them",
        )
        .with(read.running.join(", ")));
    }
    let missing: Vec<&str> = aliases
        .iter()
        .filter(|alias| !read.aliases.contains(alias))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(Said::new("Runtime 的状态里缺少这些实例", "the Runtime's status lacks these instances").with(missing.join(", ")));
    }
    if paused {
        run.left.pause = Pause::Held;
    }
    let mut summary = Said::new("在线", "online").with(format!("{} instances", read.aliases.len()));
    if let Some(reason) = read.global_pause {
        summary = summary.with(format!("global pause {reason}"));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_are_numbered_in_their_order() {
        assert_eq!(Step::ALL.len(), 15);
        for (at, step) in Step::ALL.iter().enumerate() {
            assert_eq!(step.number(), at + 1);
            assert!(!step.title().zh.is_empty() && !step.title().en.is_empty());
        }
        // The instances come after the pause, the last read after them, the watchdog last.
        assert!(Step::Pause.number() < Step::Bind.number());
        assert!(Step::Bind.number() < Step::Status.number());
        assert!(Step::Status.number() < Step::Watchdog.number());
    }

    #[test]
    fn a_failure_names_only_what_the_install_left() {
        let root = Path::new(r"\\?\X:\Root\");
        // Nothing but the folder.
        let nothing = cleanup(root, &Left::default());
        assert_eq!(nothing.len(), 1);
        assert!(nothing[0].zh.contains(r"X:\Root 文件夹"), "{}", nothing[0]);
        assert!(!nothing[0].zh.contains(r"\\?\"));
        // Everything: the Runtime, its pause, the watchdog, two shortcuts, then the folder.
        let all = cleanup(
            root,
            &Left {
                running: true,
                closed: false,
                stopped_on_failure: false,
                pause: Pause::Held,
                watchdog: true,
                shortcuts: vec![PathBuf::from(r"X:\Desk\a.lnk"), PathBuf::from(r"X:\Menu\b.lnk")],
            },
        );
        assert_eq!(all.len(), 4);
        assert!(all[0].zh.contains("挂着首次暂停"));
        assert!(all[0].en.contains(r#""X:\Root\runtime\actingctl.exe" request-shutdown --state-root "X:\Root\state""#), "{}", all[0]);
        assert!(all[0].zh.contains(r"X:\Root\ui\acui.exe 按「请求关闭」"));
        assert!(all[1].en.contains(r#""X:\Root\runtime\actingctl.exe" watchdog uninstall --root "X:\Root""#), "{}", all[1]);
        assert!(all[2].zh.contains(r"X:\Desk\a.lnk、X:\Menu\b.lnk"));
        assert!(all[2].en.contains(r"X:\Desk\a.lnk, X:\Menu\b.lnk"));
        assert!(all[3].en.starts_with("Then delete the whole"));
        // A start without the pause says so.
        let running = cleanup(root, &Left { running: true, ..Left::default() });
        assert_eq!(running.len(), 2);
        assert!(running[0].zh.contains("没有暂停") && running[0].en.contains("not paused"));
        // Step 13 closed the Runtime and failed before it started again: nothing to stop, the
        // pause kept; no command to stop it is given.
        let closed = cleanup(root, &Left { closed: true, pause: Pause::Held, ..Left::default() });
        assert_eq!(closed.len(), 2);
        assert!(closed[0].zh.contains("不用再停") && closed[0].en.contains("nothing to stop"), "{}", closed[0]);
        assert!(closed[0].zh.contains("首次暂停记在"));
        assert!(!closed[0].en.contains("request-shutdown"));
        assert!(closed[1].en.starts_with("Then delete the whole"));
        // Started again after the close: the start counts, the close no longer does.
        let again = cleanup(root, &Left { running: true, closed: true, pause: Pause::Held, ..Left::default() });
        assert!(again[0].en.contains("request-shutdown") && again[0].en.contains("may still be running"), "{}", again[0]);
        // The pause not confirmed, and the Runtime not closed: it may be playing; never
        // "under the first-start pause".
        let unconfirmed = cleanup(root, &Left { running: true, pause: Pause::Unconfirmed, ..Left::default() });
        assert!(unconfirmed[0].zh.contains("首次暂停没有确认") && unconfirmed[0].en.contains("may be playing"), "{}", unconfirmed[0]);
        assert!(!unconfirmed[0].zh.contains("挂着首次暂停"));
        assert!(unconfirmed[0].en.contains("request-shutdown"));
        // Closed by the wizard after the failure: nothing to stop, and the pause still not
        // confirmed for the next start.
        let stopped = cleanup(
            root,
            &Left { closed: true, stopped_on_failure: true, pause: Pause::Unconfirmed, ..Left::default() },
        );
        assert!(stopped[0].zh.contains("安装没有通过") && stopped[0].en.contains("nothing to stop"), "{}", stopped[0]);
        assert!(stopped[0].zh.contains("首次暂停没有确认") && !stopped[0].en.contains("request-shutdown"));
        // Shortcuts alone.
        let shortcuts = cleanup(root, &Left { shortcuts: vec![PathBuf::from(r"X:\Desk\a.lnk")], ..Left::default() });
        assert_eq!(shortcuts.len(), 2);
        assert!(!shortcuts.iter().any(|said| said.en.contains("actingctl")));
    }

    #[test]
    fn a_shortcut_names_its_root_folder_and_numbers_a_taken_name() {
        assert_eq!(shortcut_name(Lang::Zh, "AC2", 1), "ActingCommand 监控台（AC2）.lnk");
        assert_eq!(shortcut_name(Lang::Zh, "AC2", 2), "ActingCommand 监控台（AC2 2）.lnk");
        assert_eq!(shortcut_name(Lang::En, "AC2", 1), "ActingCommand Console (AC2).lnk");
        assert_eq!(shortcut_name(Lang::En, "AC2", 3), "ActingCommand Console (AC2 3).lnk");
        assert_eq!(shortcut_name(Lang::En, "AC2", 0), "ActingCommand Console (AC2).lnk");
    }
}
