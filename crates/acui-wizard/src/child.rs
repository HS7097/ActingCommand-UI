// SPDX-License-Identifier: GPL-3.0-only
//! The programs the wizard runs: always without a window, with no input, both output streams
//! read whole on threads of their own, within a time limit. A read-only child past its time is
//! ended; a child that changes the installation is never ended (it is left running and the
//! install stops, saying so). The variables that would make a Runtime program read another
//! installation's selection, change its output's detail or claim another initiator are removed
//! from every child's environment.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::text::Said;

/// Windows `CREATE_NO_WINDOW`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The most bytes kept of one output stream; the rest is read and dropped.
const OUTPUT_LIMIT: usize = 8 << 20;
/// How long the output pipes may stay open after the child exited (a grandchild that inherited
/// them would hold them).
const PIPE_GRACE: Duration = Duration::from_secs(30);

/// Removed from every child's environment: the A/B selection a launcher hands down (Runtime
/// `crates/actingcommand-contract/src/installation.rs` `INSTALL_ROOT_ENV`,
/// `INSTALL_SELECTION_ENV`), the console's observer mode (`acui-installation` `OBSERVER_UI_ENV`),
/// the outcome detail switch (`crates/lab-ops/src/cli_line.rs` `DETAIL_VARIABLE`), and the
/// initiator and its detached-run marker (`contracts/lab-two-entries.md` §4.2).
const CLEARED: [&str; 6] = [
    "ACTINGCOMMAND_INSTALL_ROOT",
    "ACTINGCOMMAND_INSTALL_SELECTION",
    "ACTINGCOMMAND_OBSERVER_UI",
    "ACTINGCOMMAND_DETAIL",
    "ACTINGCOMMAND_INITIATOR",
    "ACTINGCOMMAND_DETACHED_RUN",
];

/// What to do with a child that is still running at its time limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnTimeout {
    /// End it: it only reads.
    End,
    /// Leave it running: it may be changing the installation.
    Leave,
}

/// One program to run.
#[derive(Debug, Clone)]
pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

impl Spec {
    pub fn new(program: &Path) -> Self {
        Self {
            program: program.to_path_buf(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.env.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn cwd(mut self, dir: &Path) -> Self {
        self.cwd = Some(dir.to_path_buf());
        self
    }

    /// The command line as the log shows it.
    pub fn text(&self) -> String {
        let mut text = quoted(&self.program.to_string_lossy());
        for arg in &self.args {
            text.push(' ');
            text.push_str(&quoted(&arg.to_string_lossy()));
        }
        text
    }
}

fn quoted(word: &str) -> String {
    if word.is_empty() || word.contains([' ', '\t', '"']) {
        format!("\"{}\"", word.replace('"', "\\\""))
    } else {
        word.to_owned()
    }
}

/// A finished child: its exit code (`None` when it had none), both streams, its time.
#[derive(Debug, Clone)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
}

impl Output {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// The exit code as a person reads it.
    pub fn code_text(&self) -> String {
        self.code
            .map_or_else(|| "(none)".to_owned(), |code| code.to_string())
    }
}

enum Stream {
    Out,
    Err,
}

/// Runs `spec` (see the module's description).
pub fn run(spec: &Spec, timeout: Duration, on_timeout: OnTimeout) -> Result<Output, Said> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in CLEARED {
        command.env_remove(name);
    }
    for (name, value) in &spec.env {
        command.env(name, value);
    }
    if let Some(dir) = &spec.cwd {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let started = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        Said::new("程序运行不起来", "the program cannot be run").with(format!("{}: {error}", spec.text()))
    })?;
    let (sender, receiver) = mpsc::channel::<(Stream, std::io::Result<Vec<u8>>)>();
    let mut readers = 0;
    if let Some(pipe) = child.stdout.take() {
        readers += reader(pipe, Stream::Out, sender.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        readers += reader(pipe, Stream::Err, sender.clone());
    }
    drop(sender);
    let deadline = started + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let seconds = timeout.as_secs();
                return Err(match on_timeout {
                    OnTimeout::End => {
                        let ended = match child.kill().and_then(|()| child.wait().map(|_| ())) {
                            Ok(()) => "ended".to_owned(),
                            Err(error) => format!("could not be ended: {error}"),
                        };
                        Said::new(
                            format!("程序 {seconds} 秒内没有结束（{ended}）"),
                            format!("the program did not finish within {seconds} s ({ended})"),
                        )
                        .with(spec.text())
                    }
                    OnTimeout::Leave => Said::new(
                        format!("程序 {seconds} 秒内没有结束；它可能还在改动安装，向导没有终止它（进程号 {}），结果未确认", child.id()),
                        format!(
                            "the program did not finish within {seconds} s; it may still be changing the installation, so it was left running (pid {}) and its result is unconfirmed",
                            child.id()
                        ),
                    )
                    .with(spec.text()),
                });
            }
            Err(error) => {
                return Err(Said::new("程序的状态读不出", "the program's state cannot be read")
                    .with(format!("{}: {error}", spec.text())))
            }
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let grace = Instant::now() + PIPE_GRACE;
    for _ in 0..readers {
        let left = grace.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok((stream, Ok(bytes))) => match stream {
                Stream::Out => stdout = bytes,
                Stream::Err => stderr = bytes,
            },
            Ok((_, Err(error))) => {
                return Err(Said::new("程序的输出读不出", "the program's output cannot be read")
                    .with(format!("{}: {error}", spec.text())))
            }
            Err(_) => {
                return Err(Said::new(
                    "程序已结束，但它的输出管道仍被别的进程占着",
                    "the program has ended, but another process still holds its output open",
                )
                .with(spec.text()))
            }
        }
    }
    Ok(Output {
        code: status.code(),
        stdout,
        stderr,
        elapsed: started.elapsed(),
    })
}

/// A thread reading `pipe` whole; 1 when it started, 0 when the system refused the thread (its
/// output is then lost, and the failure shows as the missing stream).
fn reader(
    mut pipe: impl Read + Send + 'static,
    stream: Stream,
    sender: mpsc::Sender<(Stream, std::io::Result<Vec<u8>>)>,
) -> usize {
    let spawned = std::thread::Builder::new()
        .name("acsetup-child-output".into())
        .spawn(move || {
            let mut kept = Vec::new();
            let mut buffer = [0u8; 8192];
            let read = loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break Ok(kept),
                    Ok(count) => {
                        let room = OUTPUT_LIMIT.saturating_sub(kept.len());
                        kept.extend_from_slice(&buffer[..count.min(room)]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => break Err(error),
                }
            };
            let _ = sender.send((stream, read));
        });
    usize::from(spawned.is_ok())
}

/// The last line of `stdout` that is a JSON object (a byte-order mark allowed): a Runtime
/// program's one result line.
pub fn json_line(stdout: &str) -> Option<Value> {
    stdout
        .lines()
        .rev()
        .map(|line| line.trim().trim_start_matches('\u{feff}'))
        .filter(|line| line.starts_with('{'))
        .find_map(|line| serde_json::from_str::<Value>(line).ok().filter(Value::is_object))
}

/// The top outcome of a result line, `{code, category}` of its `outcome` member (an
/// `actingcommand.outcome.v1` envelope); unknown members are read past.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub code: Option<String>,
    pub category: Option<String>,
}

pub fn outcome_of(line: &Value) -> Outcome {
    let outcome = &line["outcome"];
    Outcome {
        code: outcome["code"].as_str().map(str::to_owned),
        category: outcome["category"].as_str().map(str::to_owned),
    }
}

/// An outcome envelope (`{code, category, values?, causes[{code, category, relation, values?,
/// causes}]}`, Runtime `crates/actingcommand-contract/src/outcome.rs` `WireBody`, `WireLink`) as
/// one line for a person: the top code with its category and values, then each linked outcome
/// with its relation, depth first.
pub fn outcome_text(outcome: &Value) -> String {
    fn walk(outcome: &Value, relation: Option<&str>, depth: usize, out: &mut Vec<String>) {
        if depth > 8 || out.len() >= 32 {
            return;
        }
        let mut text = String::new();
        if let Some(relation) = relation {
            text.push_str(relation);
            text.push_str(": ");
        }
        text.push_str(outcome["code"].as_str().unwrap_or("?"));
        if let Some(category) = outcome["category"].as_str() {
            text.push_str(&format!(" [{category}]"));
        }
        if let Some(values) = outcome["values"].as_object().filter(|values| !values.is_empty()) {
            text.push(' ');
            text.push_str(&Value::Object(values.clone()).to_string());
        }
        out.push(text);
        for cause in outcome["causes"].as_array().into_iter().flatten() {
            walk(cause, Some(cause["relation"].as_str().unwrap_or("cause")), depth + 1, out);
        }
    }
    let mut parts = Vec::new();
    walk(outcome, None, 0, &mut parts);
    parts.join("; ")
}

/// A line's text for a person, cut to `limit` characters.
pub fn clipped(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_result_line_is_the_last_json_object_on_stdout() {
        // actingctl and actinglab print one line `{…result, "outcome": …}` (Runtime
        // crates/lab-ops/src/cli_line.rs); a later `initiator` member is read past.
        let stdout = "INFO something\n{\"pid\":7,\"initiator\":\"Lab-救急\",\"outcome\":{\"schema_version\":\"actingcommand.outcome.v1\",\"code\":\"install_command_done\",\"category\":\"success\"}}\r\n\n";
        let line = json_line(stdout).expect("a line");
        assert_eq!(line["pid"], 7);
        assert_eq!(
            outcome_of(&line),
            Outcome {
                code: Some("install_command_done".into()),
                category: Some("success".into())
            }
        );
        assert_eq!(json_line("no json here\n[1,2]\n"), None);
        assert_eq!(json_line("\u{feff}{\"a\":1}"), Some(json!({"a": 1})));
        assert_eq!(outcome_of(&json!({"a": 1})), Outcome::default());
    }
}
