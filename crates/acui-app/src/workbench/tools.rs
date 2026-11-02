// SPDX-License-Identifier: GPL-3.0-only
//! The Runtime's `actingctl`, run as a child on `acui-tools`: no window of its own,
//! its output captured, killed after `TOOL_TIMEOUT`, and no more of its answer kept
//! than `MAX_STDOUT`.
//!
//! The one thing asked of it so far is `packs`, for the task names: the only reader
//! of a game's `ui.json` a client has. It reads the selected configuration from
//! disk, so it neither connects to the Runtime nor writes to the ledger, and it runs
//! only when asked: at the start of the run and when another Runtime comes up. What
//! it says is of the selected configuration, which a running Runtime may not have
//! loaded yet; good enough for a name. So a run of it names no owner epoch, and one
//! that was out across a restart counts until the run the restart asked for lands.
//!
//! A task without a name shows its package id: that is ordinary while a game root has
//! no `ui.json`, and says nothing. An answer that could not be had, or that came with
//! warnings, gives no names at all: every task shows its package id, one line on the
//! workbench says so, and the details are in the log.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::read::{panic_message, Poke};
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

/// What one run of `packs` came to.
pub enum NamesRead {
    /// It answered. `warnings`: each as it wrote it, for the log; with any, the model
    /// uses none of the table.
    Table { table: NameTable, warnings: Vec<String> },
    /// This run cannot tell where `actingctl` is.
    NoTool(ToolIssue),
    Failed(ToolFailure),
}

impl NamesRead {
    /// The line the technical log gets about it.
    pub fn log_line(&self) -> String {
        match self {
            Self::Table { table, warnings } if warnings.is_empty() => {
                format!("name table: {} entries", table.len())
            }
            Self::Table { table, warnings } => format!(
                "name table: {} entries, {} warnings: {}",
                table.len(),
                warnings.len(),
                excerpt(warnings.join(" ").as_bytes()),
            ),
            Self::NoTool(issue) => format!("no name table: {issue}"),
            Self::Failed(failure) => format!("no name table: {failure}"),
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

/// The thread: each poke is one run of `packs`, and pokes queued behind it ask for
/// the same one. The sender is what keeps it there. Nothing joins it: letting the
/// sender go while a child runs leaves that child to end on its own.
pub fn spawn(paths: Result<ToolPaths, ToolIssue>) -> io::Result<Sender<Poke>> {
    let (sender, commands) = mpsc::channel();
    spawn_worker(THREAD, move || run(&paths, &commands))?;
    Ok(sender)
}

fn run(paths: &Result<ToolPaths, ToolIssue>, commands: &Receiver<Poke>) {
    while commands.recv().is_ok() {
        loop {
            match commands.try_recv() {
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let started = Instant::now();
        let read = match catch_unwind(AssertUnwindSafe(|| read_names(paths))) {
            Ok(read) => read,
            Err(panic) => NamesRead::Failed(ToolFailure::Panicked(panic_message(&*panic))),
        };
        let took = started.elapsed();
        report::post(THREAD, move || super::names_read(read, took));
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
        Ok((table, warnings)) => NamesRead::Table { table, warnings },
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

/// The child's stdout when it exits 0.
fn run_tool(mut command: Command, program: &Path) -> Result<Vec<u8>, ToolFailure> {
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
    if !status.success() {
        return Err(ToolFailure::Refused {
            exit: status.code(),
            stdout: excerpt(&stdout.kept),
            stderr: excerpt(&stderr.kept),
        });
    }
    Ok(stdout.kept)
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

/// `packs`' one line: `standard_packs.games[].ui.packs[]`, by `package_id`, and its
/// `warnings`. No `standard_packs` is a configuration without that root, and a game
/// without `ui` one without the file: both are ordinary. Anything else not of this
/// shape is not an answer this build understands.
fn parse_packs(stdout: &[u8]) -> Result<(NameTable, Vec<String>), String> {
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
    if let Some(section) = document.get("standard_packs") {
        let games = section
            .get("games")
            .and_then(Value::as_array)
            .ok_or("`standard_packs.games` is not a list")?;
        for game in games {
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
    Ok((NameTable(names), warnings))
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
