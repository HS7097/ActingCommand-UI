// SPDX-License-Identifier: GPL-3.0-only
//! The Runtime's `actingctl`, run as a child on `acui-tools`: no window of its own,
//! its output captured, killed after `TOOL_TIMEOUT`, and no more of its answer kept
//! than `MAX_STDOUT`. Two things are asked of it, each only when asked, one after the
//! other.
//!
//! `packs`, for the task names, and the picture proportions each game's standard pack
//! declares it supports (`display.aspect_ratios`, which the live view's hint compares a
//! frame with): the only reader of a game's `ui.json` a client has. It
//! reads the selected configuration from disk, so it neither connects to the Runtime
//! nor writes to the ledger, and it runs at the start of the run and when another
//! Runtime comes up. What it says is of the selected configuration, which a running
//! Runtime may not have loaded yet; good enough for a name or a hint. So a run of it names
//! no owner epoch, and one that was out across a restart counts until the run the restart
//! asked for lands.
//!
//! A task without a name shows its package id: that is ordinary while a game root has
//! no `ui.json`, and says nothing. An answer that could not be had, or that came with
//! warnings, gives no names at all: every task shows its package id, one line on the
//! workbench says so, and the details are in the log.
//!
//! `watchdog status`, for the settings page: whether the scheduled task is there that
//! starts a Runtime gone without a formal close again. It only reads: it writes nothing,
//! takes no lock but a moment's shared probe of the installer's writer lock, and asks a
//! Runtime it finds for its health, which records nothing. It runs as the settings page
//! is gone to, and at no other time; `install`, `uninstall` and `run-once` are never run.
//! It prints its report whatever it decided and exits by the decision's category, so a
//! report is taken at any exit, and only a child that gave none failed. The page shows
//! what the report says of the task; the rest of it goes in the log, so that the window
//! shows where the log is when the rest says something is wrong.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::read::panic_message;
use crate::install::{ToolIssue, ToolPaths};
use crate::launcher::spawn_worker;
use crate::report;
use crate::strings::Language;

/// The thread's name, also what its lines in the technical log are under.
pub const THREAD: &str = "acui-tools";

/// Windows `CREATE_NO_WINDOW`: the output is captured, so a child gets no console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const TOOL_TIMEOUT: Duration = Duration::from_secs(15);
const TOOL_POLL: Duration = Duration::from_millis(50);
/// How long a child's output may take to end once the child has.
const OUTPUT_ENDS: Duration = Duration::from_secs(2);
const MAX_STDOUT: u64 = 8 * 1024 * 1024;
/// stderr holds developer lines, for the log only.
const MAX_STDERR: u64 = 64 * 1024;
/// The most of a child's words one line of the log takes.
const LOG_EXCERPT: usize = 2000;
/// `watchdog`'s one stdout line, and the two attention outcomes of its `status` that say
/// the task is not there, or not as `install` writes it.
const WATCHDOG_REPORT: &str = "actingcommand.watchdog.report.v2";
const TASK_MISSING: &str = "watchdog_task_missing";
const TASK_MISMATCH: &str = "watchdog_task_mismatch";
/// The categories of a decision that says nothing is wrong. `error` and `fatal` exit 1 and 2;
/// `warning` exits 0, as an installer operation left unfinished does when it is the decision.
const ROUTINE: [&str; 2] = ["info", "success"];

/// The locales a language tries, in order, before `en` and then the first written.
/// `ui.json` treats locale names as opaque: these are the spellings expected.
fn locales(language: Language) -> &'static [&'static str] {
    match language {
        Language::Zh => &["zh-Hans", "zh", "zh-CN"],
        Language::En => &["en"],
    }
}

/// The task names of one `packs` answer: by package id, each title per locale in the
/// order the file wrote them. A package two games list keeps the first game's.
pub struct NameTable(BTreeMap<String, Vec<(String, String)>>);

impl NameTable {
    fn len(&self) -> usize {
        self.0.len()
    }

    /// `package`'s title in `language`; `None` when the table has none for it.
    pub fn title(&self, package: &str, language: Language) -> Option<&str> {
        let titles = self.0.get(package)?;
        locales(language)
            .iter()
            .find_map(|locale| title_in(titles, locale))
            .or_else(|| title_in(titles, "en"))
            .or_else(|| titles.first().map(|(_, title)| title.as_str()))
    }
}

fn title_in<'a>(titles: &'a [(String, String)], locale: &str) -> Option<&'a str> {
    titles.iter().find(|(name, _)| name == locale).map(|(_, title)| title.as_str())
}

/// The picture proportions of one `packs` answer: by game, each `<w>:<h>` its `ui.json`
/// declares, in the file's order. A game without the file, or whose file declares no
/// `display`, has none, and no hint is given for it.
pub struct Displays(BTreeMap<String, Vec<(u32, u32)>>);

impl Displays {
    fn len(&self) -> usize {
        self.0.len()
    }

    /// The proportions `game` declares; `None` when it declares none.
    pub fn ratios(&self, game: &str) -> Option<&[(u32, u32)]> {
        self.0.get(game).map(Vec::as_slice)
    }
}

/// What one run of `packs` came to.
pub enum NamesRead {
    /// It answered. `warnings`: each as it wrote it, for the log; with any, the model
    /// uses none of the table. The proportions are used all the same: one is there only
    /// from a `ui.json` that keeps its rules, and a warning says nothing against it.
    Table { table: NameTable, displays: Displays, warnings: Vec<String> },
    /// This run cannot tell where `actingctl` is.
    NoTool(ToolIssue),
    Failed(ToolFailure),
}

impl NamesRead {
    /// The line the technical log gets about it.
    pub fn log_line(&self) -> String {
        match self {
            Self::Table { table, displays, warnings } if warnings.is_empty() => {
                format!("name table: {} entries; displays of {} games", table.len(), displays.len())
            }
            Self::Table { table, displays, warnings } => format!(
                "name table: {} entries; displays of {} games; {} warnings: {}",
                table.len(),
                displays.len(),
                warnings.len(),
                excerpt(warnings.join(" ").as_bytes()),
            ),
            Self::NoTool(issue) => format!("no name table: {issue}"),
            Self::Failed(failure) => format!("no name table: {failure}"),
        }
    }
}

/// What one run of `watchdog status` came to.
pub enum WatchdogRead {
    /// It answered.
    Report(WatchdogReport),
    /// This run cannot tell where `actingctl` is.
    NoTool(ToolIssue),
    Failed(ToolFailure),
}

impl WatchdogRead {
    /// The line the technical log gets about it.
    pub fn log_line(&self) -> String {
        match self {
            Self::Report(report) => {
                let exit = report.exit.map_or_else(|| "-".to_owned(), |exit| exit.to_string());
                let mut line = format!(
                    "watchdog status: {} ({}, exit {exit}), attention [{}], task {}",
                    report.decision,
                    report.category,
                    report.attention.join(", "),
                    report.task.word(),
                );
                if let Some(detail) = &report.detail {
                    line.push_str(": ");
                    line.push_str(detail);
                }
                line
            }
            Self::NoTool(issue) => format!("no watchdog status: {issue}"),
            Self::Failed(failure) => format!("no watchdog status: {failure}"),
        }
    }

    /// Whether something is wrong, so that its line in the log has the window show where the
    /// log is: the settings page says so of the task, or the report says so of the rest,
    /// which the page does not show.
    pub fn is_problem(&self) -> bool {
        match self {
            Self::Report(report) => {
                matches!(report.task, TaskState::Differs | TaskState::Unread) || report.says_wrong()
            }
            Self::NoTool(_) | Self::Failed(_) => true,
        }
    }
}

/// What a report says of the task, and the rest of it, for the log.
pub struct WatchdogReport {
    pub task: TaskState,
    /// The decision's code, its category, and the exit it made.
    decision: String,
    category: String,
    exit: Option<i32>,
    /// Each attention outcome's code.
    attention: Vec<String>,
    /// Of a task that differs, what differs; of one the report could not tell, what it
    /// said of why.
    detail: Option<String>,
}

impl WatchdogReport {
    /// Whether the report says something is wrong besides what it says of the task: a
    /// decision that exited other than 0 or is of another category than `ROUTINE` (a fatal
    /// hold, say, which starts no Runtime until something outside fixes it), or an attention
    /// outcome other than the task's own (a last start that failed, a shutdown's evidence
    /// that cannot be read, …). The page says the task as it is, with an amber line under it.
    pub fn says_wrong(&self) -> bool {
        let of_the_task = |code: &String| [TASK_MISSING, TASK_MISMATCH].contains(&code.as_str());
        self.exit != Some(0)
            || !ROUTINE.contains(&self.category.as_str())
            || !self.attention.iter().all(of_the_task)
    }
}

/// The scheduled task that starts the Runtime again, as a report has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Registered, as `install` writes it.
    On,
    /// Not registered.
    Missing,
    /// Registered, and turned off or not as `install` writes it.
    Differs,
    /// The report cannot tell: the task's query failed, or the report ended before the
    /// task, the watchdog not being able to read the root.
    Unread,
}

impl TaskState {
    fn word(self) -> &'static str {
        match self {
            Self::On => "registered",
            Self::Missing => "missing",
            Self::Differs => "differs",
            Self::Unread => "not read",
        }
    }
}

/// Why a child gave no answer to use.
pub enum ToolFailure {
    /// The command could not be put together.
    NotPrepared(String),
    NotStarted { program: PathBuf, error: String },
    /// A thread this needs did not start, or is gone.
    NoThread(String),
    /// This read panicked.
    Panicked(String),
    TimedOut { reaped: String },
    Wait { error: String, reaped: String },
    Output(String),
    TooMuchOutput,
    /// It exited with a failure: `{outcome}` on stdout, a developer line on stderr.
    Refused { exit: Option<i32>, stdout: String, stderr: String },
    /// It exited 0 with an answer not of the expected shape.
    Malformed(String),
}

impl fmt::Display for ToolFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrepared(error) => write!(formatter, "command not prepared: {error}"),
            Self::NotStarted { program, error } => {
                write!(formatter, "{} did not start: {error}", program.display())
            }
            Self::NoThread(error) => write!(formatter, "no thread: {error}"),
            Self::Panicked(message) => write!(formatter, "read panicked: {message}"),
            Self::TimedOut { reaped } => {
                write!(formatter, "no answer in {} s; {reaped}", TOOL_TIMEOUT.as_secs())
            }
            Self::Wait { error, reaped } => write!(formatter, "wait failed: {error}; {reaped}"),
            Self::Output(error) => write!(formatter, "output not read: {error}"),
            Self::TooMuchOutput => write!(formatter, "output over {MAX_STDOUT} bytes"),
            Self::Refused { exit, stdout, stderr } => {
                write!(formatter, "exit {exit:?}: {stdout} | {stderr}")
            }
            Self::Malformed(what) => write!(formatter, "answer not understood: {what}"),
        }
    }
}

/// What the thread is asked to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// `packs`.
    Names,
    /// `watchdog status`, under the number of its ask: an answer to an ask the event loop
    /// has since let go of is told apart by it.
    Watchdog(u64),
}

/// What one round runs: asks queued behind the first ask for the same runs.
#[derive(Default)]
struct Round {
    names: bool,
    /// The newest ask's number.
    watchdog: Option<u64>,
}

impl Round {
    fn add(&mut self, job: Job) {
        match job {
            Job::Names => self.names = true,
            Job::Watchdog(ask) => self.watchdog = self.watchdog.max(Some(ask)),
        }
    }
}

/// The thread: each round runs what was asked since the last, `packs` first. The sender
/// is what keeps it there. Nothing joins it: letting the sender go while a child runs
/// leaves that child to end on its own.
pub fn spawn(paths: Result<ToolPaths, ToolIssue>) -> io::Result<Sender<Job>> {
    let (sender, commands) = mpsc::channel();
    spawn_worker(THREAD, move || run(&paths, &commands))?;
    Ok(sender)
}

fn run(paths: &Result<ToolPaths, ToolIssue>, commands: &Receiver<Job>) {
    while let Ok(job) = commands.recv() {
        let mut round = Round::default();
        round.add(job);
        loop {
            match commands.try_recv() {
                Ok(job) => round.add(job),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if round.names {
            let started = Instant::now();
            let read = match catch_unwind(AssertUnwindSafe(|| read_names(paths))) {
                Ok(read) => read,
                Err(panic) => NamesRead::Failed(ToolFailure::Panicked(panic_message(&*panic))),
            };
            let took = started.elapsed();
            report::post(THREAD, move || super::names_read(read, took));
        }
        if let Some(ask) = round.watchdog {
            let started = Instant::now();
            let read = match catch_unwind(AssertUnwindSafe(|| read_watchdog(paths))) {
                Ok(read) => read,
                Err(panic) => WatchdogRead::Failed(ToolFailure::Panicked(panic_message(&*panic))),
            };
            let took = started.elapsed();
            report::post(THREAD, move || super::watchdog_read(ask, read, took));
        }
    }
}

fn read_names(paths: &Result<ToolPaths, ToolIssue>) -> NamesRead {
    let paths = match paths {
        Ok(paths) => paths,
        Err(issue) => return NamesRead::NoTool(issue.clone()),
    };
    match packs_answer(paths).and_then(|stdout| {
        parse_packs(&stdout).map_err(ToolFailure::Malformed)
    }) {
        Ok((table, displays, warnings)) => NamesRead::Table { table, displays, warnings },
        Err(failure) => NamesRead::Failed(failure),
    }
}

/// `packs`' stdout. An installed run hands the child its own selection and no
/// `--root`; otherwise `--root` names the root that was checked (`install`).
fn packs_answer(paths: &ToolPaths) -> Result<Vec<u8>, ToolFailure> {
    let mut command = Command::new(&paths.actingctl);
    if let Some(snapshot) = &paths.installation {
        snapshot.apply_to(&mut command).map_err(ToolFailure::NotPrepared)?;
    }
    command.arg("packs");
    if let Some(root) = &paths.root {
        command.arg("--root").arg(root);
    }
    run_tool(command, &paths.actingctl)
}

fn read_watchdog(paths: &Result<ToolPaths, ToolIssue>) -> WatchdogRead {
    let paths = match paths {
        Ok(paths) => paths,
        Err(issue) => return WatchdogRead::NoTool(issue.clone()),
    };
    match watchdog_answer(paths) {
        Ok(report) => WatchdogRead::Report(report),
        Err(failure) => WatchdogRead::Failed(failure),
    }
}

/// `watchdog status --root <root>`, which the command requires. An installed run hands the
/// child its own selection, as for `packs`, and names its installation's root, the one the
/// selection is of; otherwise `--root` names the root that was checked (`install`). A report
/// is taken at any exit; a child that exits with a failure and no report refused.
fn watchdog_answer(paths: &ToolPaths) -> Result<WatchdogReport, ToolFailure> {
    let mut command = Command::new(&paths.actingctl);
    let root = match (&paths.installation, &paths.root) {
        (Some(snapshot), _) => {
            snapshot.apply_to(&mut command).map_err(ToolFailure::NotPrepared)?;
            snapshot.root.clone()
        }
        (None, Some(root)) => root.clone(),
        (None, None) => return Err(ToolFailure::NotPrepared("no root to name".to_owned())),
    };
    command.arg("watchdog").arg("status").arg("--root").arg(root);
    let ran = run_child(command, &paths.actingctl)?;
    match parse_watchdog(&ran.stdout) {
        Ok(report) => Ok(WatchdogReport { exit: ran.status.code(), ..report }),
        Err(_) if !ran.status.success() => Err(ran.refused()),
        Err(what) => Err(ToolFailure::Malformed(what)),
    }
}

/// The child's stdout when it exits 0.
fn run_tool(command: Command, program: &Path) -> Result<Vec<u8>, ToolFailure> {
    let ran = run_child(command, program)?;
    if !ran.status.success() {
        return Err(ran.refused());
    }
    Ok(ran.stdout)
}

/// A child run to its end: how it exited, and what it wrote, as much as is kept.
struct Ran {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Ran {
    /// It exited with a failure: `{outcome}` on stdout, a developer line on stderr.
    fn refused(&self) -> ToolFailure {
        ToolFailure::Refused {
            exit: self.status.code(),
            stdout: excerpt(&self.stdout),
            stderr: excerpt(&self.stderr),
        }
    }
}

/// The child, run to its end within `TOOL_TIMEOUT`, its stdout within `MAX_STDOUT`.
fn run_child(mut command: Command, program: &Path) -> Result<Ran, ToolFailure> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| ToolFailure::NotStarted {
        program: program.to_path_buf(),
        error: error.to_string(),
    })?;
    // Read while it runs, so a full pipe can never stall it.
    let drains = drain(child.stdout.take(), MAX_STDOUT)
        .and_then(|stdout| Ok((stdout, drain(child.stderr.take(), MAX_STDERR)?)));
    let (stdout, stderr) = match drains {
        Ok(drains) => drains,
        Err(error) => {
            let reaped = reap(&mut child);
            return Err(ToolFailure::NoThread(format!("{error}; {reaped}")));
        }
    };
    let deadline = Instant::now() + TOOL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(TOOL_POLL),
            Ok(None) => return Err(ToolFailure::TimedOut { reaped: reap(&mut child) }),
            Err(error) => {
                let error = error.to_string();
                return Err(ToolFailure::Wait { error, reaped: reap(&mut child) });
            }
        }
    };
    let stdout = output_of(&stdout)?;
    let stderr = output_of(&stderr)?;
    if stdout.more {
        return Err(ToolFailure::TooMuchOutput);
    }
    Ok(Ran { status, stdout: stdout.kept, stderr: stderr.kept })
}

/// One pipe's output: what was kept, and whether there was more.
struct Drained {
    kept: Vec<u8>,
    more: bool,
}

/// A thread for one pipe. It reads to the end, so the child never sees a closed pipe;
/// what it keeps is bounded.
fn drain<R: Read + Send + 'static>(
    pipe: Option<R>,
    keep: u64,
) -> io::Result<Receiver<io::Result<Drained>>> {
    let (sender, receiver) = mpsc::channel();
    spawn_worker("acui-tools-pipe", move || {
        if sender.send(read_pipe(pipe, keep)).is_err() {
            report::log(THREAD, "a child's output ended after it was given up on");
        }
    })?;
    Ok(receiver)
}

fn read_pipe<R: Read>(pipe: Option<R>, keep: u64) -> io::Result<Drained> {
    let mut kept = Vec::new();
    let Some(mut pipe) = pipe else {
        return Ok(Drained { kept, more: false });
    };
    pipe.by_ref().take(keep).read_to_end(&mut kept)?;
    let more = io::copy(&mut pipe, &mut io::sink())? > 0;
    Ok(Drained { kept, more })
}

/// A pipe's output once the child is over. Anything else still holding the pipe open
/// is a failure to say, not a wait without end.
fn output_of(pipe: &Receiver<io::Result<Drained>>) -> Result<Drained, ToolFailure> {
    match pipe.recv_timeout(OUTPUT_ENDS) {
        Ok(Ok(drained)) => Ok(drained),
        Ok(Err(error)) => Err(ToolFailure::Output(error.to_string())),
        Err(error) => Err(ToolFailure::Output(error.to_string())),
    }
}

/// Kills and reaps the child; what became of that, for the log.
fn reap(child: &mut Child) -> String {
    match child.kill() {
        Err(error) => format!("kill failed: {error}"),
        Ok(()) => match child.wait() {
            Ok(_) => "killed".to_owned(),
            Err(error) => format!("killed, not reaped: {error}"),
        },
    }
}

/// A child's words for the log: as text, and only so much of them.
fn excerpt(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().chars().take(LOG_EXCERPT).collect()
}

/// `packs`' one line: `standard_packs.games[].ui.packs[]`, by `package_id`, and
/// `standard_packs.games[].display.aspect_ratios`, by `game`, and its `warnings`. No
/// `standard_packs` is a configuration without that root, and a game without `ui` or
/// `display` one without the file, or whose file declares no proportions, as a `packs` from
/// before they were listed answers: all ordinary. Anything else not of this shape is not
/// an answer this build understands.
fn parse_packs(stdout: &[u8]) -> Result<(NameTable, Displays, Vec<String>), String> {
    let document: Value =
        serde_json::from_slice(stdout).map_err(|error| format!("not JSON: {error}"))?;
    if !document.is_object() {
        return Err("not an object".to_owned());
    }
    let warnings = match document.get("warnings") {
        None => Vec::new(),
        Some(Value::Array(warnings)) => warnings.iter().map(|warning| warning.to_string()).collect(),
        Some(_) => return Err("`warnings` is not a list".to_owned()),
    };
    let mut names = BTreeMap::new();
    let mut displays = BTreeMap::new();
    if let Some(section) = document.get("standard_packs") {
        let games = section
            .get("games")
            .and_then(Value::as_array)
            .ok_or("`standard_packs.games` is not a list")?;
        for game in games {
            if let Some(display) = game.get("display") {
                let name =
                    game.get("game").and_then(Value::as_str).ok_or("a game has no `game`")?;
                displays.entry(name.to_owned()).or_insert(display_ratios(display)?);
            }
            let Some(ui) = game.get("ui") else {
                continue;
            };
            let packs =
                ui.get("packs").and_then(Value::as_array).ok_or("a game's `ui.packs` is not a list")?;
            for pack in packs {
                let (package, titles) = pack_titles(pack)?;
                names.entry(package.to_owned()).or_insert(titles);
            }
        }
    }
    Ok((NameTable(names), Displays(displays), warnings))
}

/// A game's `display.aspect_ratios`, each `<w>:<h>` as two whole numbers above zero, which is
/// all the Runtime checks of them.
fn display_ratios(display: &Value) -> Result<Vec<(u32, u32)>, String> {
    let ratios = display
        .get("aspect_ratios")
        .and_then(Value::as_array)
        .ok_or("a game's `display.aspect_ratios` is not a list")?;
    ratios
        .iter()
        .map(|ratio| {
            let sides = ratio.as_str().and_then(|text| text.split_once(':'));
            let side = |text: &str| text.parse::<u32>().ok().filter(|side| *side > 0);
            sides
                .and_then(|(width, height)| Some((side(width)?, side(height)?)))
                .ok_or_else(|| format!("an aspect ratio is not `<w>:<h>`: {ratio}"))
        })
        .collect()
}

fn pack_titles(pack: &Value) -> Result<(&str, Vec<(String, String)>), String> {
    let package =
        pack.get("package_id").and_then(Value::as_str).ok_or("a pack has no `package_id`")?;
    let title = pack.get("title").and_then(Value::as_object).ok_or("a pack has no `title`")?;
    let mut titles = Vec::with_capacity(title.len());
    for (locale, text) in title {
        let text = text.as_str().ok_or("a `title` holds something other than text")?;
        titles.push((locale.clone(), text.to_owned()));
    }
    Ok((package, titles))
}

/// `watchdog status`' one line: the decision as `outcome`, with its category, the
/// `attention` outcomes, and the `task` section. Its attention says a task that is missing
/// or differs; a section with an `error` is a query that failed, one with `registered` true
/// a task as `install` writes it. A report without the section ended before the task, at a
/// root the watchdog could not read: its decision says why.
fn parse_watchdog(stdout: &[u8]) -> Result<WatchdogReport, String> {
    let document: Value =
        serde_json::from_slice(stdout).map_err(|error| format!("not JSON: {error}"))?;
    if document.get("schema_version").and_then(Value::as_str) != Some(WATCHDOG_REPORT) {
        return Err("not a watchdog report".to_owned());
    }
    let decision = document
        .pointer("/outcome/code")
        .and_then(Value::as_str)
        .ok_or("the report's `outcome` has no code")?
        .to_owned();
    let category = document
        .pointer("/outcome/category")
        .and_then(Value::as_str)
        .ok_or("the report's `outcome` has no category")?
        .to_owned();
    let attention = document
        .get("attention")
        .and_then(Value::as_array)
        .ok_or("`attention` is not a list")?
        .iter()
        .map(|outcome| {
            let code = outcome.get("code").and_then(Value::as_str);
            code.map(str::to_owned).ok_or("an `attention` outcome has no code")
        })
        .collect::<Result<Vec<String>, &str>>()?;
    let named = |code: &str| attention.iter().any(|said| said == code);
    let (task, detail) = match document.get("task") {
        None => (TaskState::Unread, Some(excerpt(document["outcome"].to_string().as_bytes()))),
        Some(_) if named(TASK_MISSING) => (TaskState::Missing, None),
        Some(section) if named(TASK_MISMATCH) => {
            (TaskState::Differs, section.get("mismatches").map(Value::to_string))
        }
        Some(section) => match (section.get("error"), section.get("registered")) {
            (Some(error), _) => (TaskState::Unread, Some(excerpt(error.to_string().as_bytes()))),
            (None, Some(Value::Bool(true))) => (TaskState::On, None),
            (None, _) => {
                return Err("`task` is neither registered nor named by an attention".to_owned())
            }
        },
    };
    Ok(WatchdogReport { task, decision, category, exit: None, attention, detail })
}
