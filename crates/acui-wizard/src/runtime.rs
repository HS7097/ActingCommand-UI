// SPDX-License-Identifier: GPL-3.0-only
//! The Runtime's own programs, run from the new root, each through the face the Runtime's route
//! leads to (Runtime `contracts/lab-two-entries.md`: `actingctl` leaves with LAB-7, the Lab's
//! rows take its place): a row of `tools\actinglab.exe` wherever one serves the step, the
//! Runtime's own `runtime\actingcommand-actingd.exe check-config` where none can yet, and
//! `runtime\actingctl.exe request-shutdown` for step 13's close, the form tried on a test machine
//! (the Lab's formal stop, `down`, keeps a hold the console does not lift). Every Lab
//! process the wizard starts is `ACTINGCOMMAND_INITIATOR=Lab-救急` (the same contract, §4.2: the
//! word for "a Lab process the UI starts outside its workbench: the launcher, acsetup, the
//! settings page"). Each answers one JSON line on stdout, read past members it does not know
//! (the `initiator` a later Runtime adds among them), and exits 0 for success, info or warning,
//! 1 for an error and 2 for a fatal outcome (`crates/lab-ops/src/cli_line.rs`).
//!
//! | step | command | why this face |
//! |---|---|---|
//! | configuration check | `runtime\actingcommand-actingd.exe check-config --config <candidate> --install-root <root>` (`apps/actingd/src/check_config.rs` `run`) | the Lab's `check-config` row checks the root's own configuration file and needs it in place to locate the root (`crates/lab-ops/src/locate.rs`; `crates/lab-rescue/src/rescue.rs` `check_config`); the candidate is checked before it is put in place |
//! | standard packages | `actinglab --json standard-pack check --root <game root>` (`apps/actinglab/src/standard_pack_cli.rs`) | the one command; it reads only the game root, no configuration |
//! | adopt | `actinglab --json adopt` (rescue row; `rescue.rs` `installer` → `install adopt --root`) | served row; journals the initiator |
//! | start | `actinglab --json up` (rescue row; on a single-location root `install start`: `rescue.rs` `up`; `crates/lab-rescue/src/install/deploy.rs` `start_command`) | served row; the installer's start with its post-start checks, which a start reports and never refuses on (`deploy.rs`: a `Start` operation ends done whatever `failed` holds), so the wizard reads them ([`judge_up`]) |
//! | first pause | `actinglab --json pause --reason first-start` (core row, global without an instance: `crates/lab-ops/src/scheduling.rs`) | served row; Lab envelopes are taken since 0.12.58 (LAB-6S1b) |
//! | the instances, 1: close | `runtime\actingctl.exe request-shutdown --state-root <root>\state --wait 60` (`apps/actingctl/src/main.rs` `Command::RequestShutdown`; `crates/lab-ops/src/shutdown.rs` `wait`) | the formal close, answered `runtime_shutdown_completed` once the owner closed and its process exited ([`judge_shutdown`]); not `actinglab down`, whose installer stop keeps a hold of its own (`lab-down-<ms>`, `rescue.rs` `down`) that the console does not lift |
//! | the instances, 2: swap | `actinglab --json config replace <full configuration> --sha <its SHA-256>` (rescue row: `rescue.rs` `config_swap` and `commit`; `crates/lab-rescue/src/install/config_swap.rs`) | served row: with no Runtime running, the live build's check-config of the file and the swap alone, the replaced file kept as `actingd.config.json.prev-<ms>`; the answer says `runtime_running` false ([`judge_replace`]) |
//! | the instances, 3: start | `actinglab --json up`, as above | the start reads the new file, restores the pause first, and checks itself |
//! | status | `actinglab --json status` (core row: `crates/lab-ops/src/overview.rs`, `daemon.online`) | served row |
//! | watchdog | `actinglab --json watchdog install` (rescue row: `rescue.rs` `watchdog`) | served row |
//!
//! Why the instances come by a close, a `config replace` and an `up`: a global pause stops no
//! run already under way (`contracts/scheduling-pause.md`, "Global pause"), so a Runtime started
//! with its instances could dispatch in the moment between the start and the pause. The wizard
//! starts the Runtime with a configuration without instances, sets the pause, and only then puts
//! the full configuration in place. The pause is a runtime fact that survives a close; every
//! start that is not installer-held restores it before any physical instance is prepared and
//! before the policy driver runs (`scheduling-pause.md`, "Persistence, no expiry").
//!
//! The full configuration is not put in place on the running Runtime. On a test machine
//! (2026-11-13, 0.12.58), `config replace` on the running Runtime of a newly installed root
//! answered `install_command_refused` with `post_check_failed` (`count` 1), yet the file had been
//! swapped and the Runtime restarted, its pause restored before the instances were prepared
//! (`scheduling_pause_restored` in the actingd log). The row that failed is `no_seeding`
//! (`failed_checks` in the answer): a new root's state is still tagged `seed` on its first
//! restarts, which `up`'s own check passes (`tagged_state` `seed` with `earlier_runtime` false)
//! and the restart inside `config replace` does not. It fails the same with no instance and the
//! same configuration, and on an older root (`tagged_state` `load`) the same replace from no
//! instance to one passed every row; the note `guard_failed` `foreign_pauses` (the first-start
//! pause) comes either way and fails nothing. Closed first, a new root passed every part: the
//! close answered `runtime_shutdown_completed`; `config replace` answered
//! `install_command_done` with `runtime_running` false and kept the replaced file; `up` exited 0
//! with all ten post-start checks passed (`ready`, `open`, `no_seeding`, `bindings`, `packs`,
//! `pauses`, `instructions`, `no_dispatch`, `new_keys`, `config`); `status` showed the global
//! pause still `first-start`, the instances there and the first task waiting on
//! `dispatch_paused_global`. An `up` with the first-start pause already set and no instance
//! exited 0 too.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::child::{self, json_line, outcome_of, outcome_text, OnTimeout, Spec};
use crate::config::STATE_DIR;
use crate::text::Said;
use crate::winpath::strip_verbatim;

pub const ACTINGD_EXE: &str = "actingcommand-actingd.exe";
pub const ACTINGCTL_EXE: &str = "actingctl.exe";
pub const ACTINGLAB_EXE: &str = "actinglab.exe";
/// `request-shutdown --wait`'s seconds: as long as `actingctl restart` waits for its close
/// (Runtime `crates/lab-rescue/src/runtime_start.rs` `SHUTDOWN_WAIT`).
pub const SHUTDOWN_WAIT_SECONDS: u64 = 60;
/// What `request-shutdown --wait` answers once the Runtime closed and its process exited.
pub const SHUTDOWN_COMPLETED: &str = "runtime_shutdown_completed";
pub const INITIATOR_VARIABLE: &str = "ACTINGCOMMAND_INITIATOR";
pub const RESCUE_INITIATOR: &str = "Lab-救急";
pub const CHECK_CONFIG_SCHEMA: &str = "actingcommand.actingd.check-config.v2";
/// The first start's pause reason: a name the Runtime takes (`[a-z0-9_.-]{1,64}`,
/// `crates/actingcommand-contract/src/runtime.rs` `validate_scheduling_pause_reason`) that the
/// console lifts with "put all on duty" (two presses, as for any other tool's pause): it is no
/// deployment's (`acsetup_update`, `install_restart`, `deploy-…`), no Lab rescue operation's
/// (`lab-<word>-<ms>`) and in no default `resume_elsewhere_reasons` (`[]`)
/// (`crates/acui-app/src/ledger/resume.rs` `lifter`).
pub const FIRST_START_REASON: &str = "first-start";

/// Time limits: the checks only read and may be ended; the others change the root and are never
/// ended. `up` waits for readiness up to `install.start.ready_timeout_ms` (900 s by default) after
/// a memory wait of up to `install.start.memory_wait_ms` (120 s) (`distribution/windows/
/// actingd.defaults.json`).
pub const CHECK_CONFIG_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const PACK_CHECK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const ADOPT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const UP_TIMEOUT: Duration = Duration::from_secs(25 * 60);
pub const PAUSE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// With no Runtime running: the live build's check-config of the file (as long as the wizard's
/// own) and the swap.
pub const CONFIG_REPLACE_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// The close: its wait ([`SHUTDOWN_WAIT_SECONDS`]) and room to connect and answer.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3 * 60);
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(3 * 60);
pub const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// What an instance does now that is a task run (`activity.kind`, Runtime
/// `contracts/runtime-interface.md`, "What an instance does now"): a policy run, a client's run
/// and the host's own. A reconnect, a self-check, an emulator control and a recovery ladder are
/// no task.
pub const RUN_ACTIVITIES: [&str; 3] = ["policy_run", "direct_run", "host_run"];

/// One program's answer.
#[derive(Debug, Clone)]
pub struct Answer {
    pub command: String,
    pub code: Option<i32>,
    pub line: Option<Value>,
    pub outcome: child::Outcome,
    pub stdout: String,
    pub stderr: String,
}

impl Answer {
    /// Exit 0: success, info or warning.
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// What it answered, for a person: its exit code, its outcome with every link, its stderr.
    pub fn reason(&self) -> String {
        let mut parts = vec![format!(
            "exit {}",
            self.code.map_or_else(|| "(none)".to_owned(), |code| code.to_string())
        )];
        match &self.line {
            Some(line) => {
                parts.push(outcome_text(&line["outcome"]));
                if let Some(message) = line["message"].as_str() {
                    parts.push(message.to_owned());
                }
            }
            None if !self.stdout.trim().is_empty() => parts.push(child::clipped(&self.stdout, 1500)),
            None => {}
        }
        if !self.stderr.trim().is_empty() {
            parts.push(child::clipped(&self.stderr, 1500));
        }
        child::clipped(&parts.join(" | "), 4000)
    }

    /// The answer's whole stdout as it came (up to 64 Ki characters), for the install log only,
    /// so that whoever reads the log afterwards has every post-start row and `failed_checks` of
    /// a refusal and not only its count; `None` when it wrote nothing. The window shows
    /// [`Answer::reason`].
    pub fn log_stdout(&self) -> Option<String> {
        (!self.stdout.trim().is_empty()).then(|| child::clipped(&self.stdout, 65536))
    }

    /// The line's member `name`.
    pub fn member(&self, name: &str) -> Option<&Value> {
        self.line.as_ref()?.get(name)
    }
}

fn answer(spec: &Spec, timeout: Duration, on_timeout: OnTimeout) -> Result<Answer, Said> {
    let output = child::run(spec, timeout, on_timeout)?;
    let stdout = output.stdout_text();
    let line = json_line(&stdout);
    let outcome = line.as_ref().map(outcome_of).unwrap_or_default();
    Ok(Answer {
        command: spec.text(),
        code: output.code,
        line,
        outcome,
        stdout,
        stderr: output.stderr_text(),
    })
}

pub fn actingd(root: &Path) -> PathBuf {
    root.join("runtime").join(ACTINGD_EXE)
}

pub fn actinglab(root: &Path) -> PathBuf {
    root.join("tools").join(ACTINGLAB_EXE)
}

pub fn actingctl(root: &Path) -> PathBuf {
    root.join("runtime").join(ACTINGCTL_EXE)
}

/// `check-config`'s answer, with the MuMu install root it reports.
#[derive(Debug, Clone)]
pub struct CheckConfig {
    pub answer: Answer,
    /// Exit 0 and a v2 line: `check_config_passed` (a note does not fail it).
    pub passed: bool,
    /// `report.mumu_root.path`: the configured root, else the one the Runtime's own resolution
    /// found (`check_config.rs` `mumu_root_report`); without the verbatim prefix.
    pub mumu_root: Option<String>,
}

/// `<root>\runtime\actingcommand-actingd.exe check-config --config <config> --install-root
/// <root>`: the start's own assembly and checks, no MuMu discovery, no state written
/// (`contracts/actingd-check-config.md`). `--install-root` makes it judge the root's adb and
/// vision root as the root's own Runtime will (`check_config.rs` `install_root_of`).
pub fn check_config(root: &Path, config: &Path) -> Result<CheckConfig, Said> {
    let spec = Spec::new(&actingd(root))
        .arg("check-config")
        .arg("--config")
        .arg(config.as_os_str())
        .arg("--install-root")
        .arg(root.as_os_str())
        .cwd(root);
    let answer = answer(&spec, CHECK_CONFIG_TIMEOUT, OnTimeout::End)?;
    let schema = answer.member("schema_version").and_then(Value::as_str);
    let passed = answer.ok() && schema == Some(CHECK_CONFIG_SCHEMA);
    let mumu_root = answer
        .member("report")
        .and_then(|report| report["mumu_root"]["path"].as_str())
        .map(strip_verbatim);
    Ok(CheckConfig {
        answer,
        passed,
        mumu_root,
    })
}

/// `<root>\tools\actinglab.exe --json <words…>`, as `Lab-救急`, in the root.
pub fn lab(root: &Path, words: &[OsString], timeout: Duration, on_timeout: OnTimeout) -> Result<Answer, Said> {
    let mut spec = Spec::new(&actinglab(root))
        .arg("--json")
        .env(INITIATOR_VARIABLE, RESCUE_INITIATOR)
        .cwd(root);
    for word in words {
        spec = spec.arg(word.clone());
    }
    answer(&spec, timeout, on_timeout)
}

fn words(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// `actinglab --json standard-pack check --root <game root>`: the checks `package bundle` runs on
/// the root it lays out, read only.
pub fn standard_pack_check(root: &Path, game_root: &Path) -> Result<Answer, Said> {
    let mut list = words(&["standard-pack", "check", "--root"]);
    list.push(game_root.as_os_str().to_os_string());
    lab(root, &list, PACK_CHECK_TIMEOUT, OnTimeout::End)
}

/// `actinglab --json adopt`: the installer takes the root (both manifests, every file's
/// SHA-256, a kept copy in `install\builds\<commit>\`, `install\state.json`).
pub fn adopt(root: &Path) -> Result<Answer, Said> {
    lab(root, &words(&["adopt"]), ADOPT_TIMEOUT, OnTimeout::Leave)
}

/// `actinglab --json up`: the installer's start of the root, ready or refused.
pub fn up(root: &Path) -> Result<Answer, Said> {
    lab(root, &words(&["up"]), UP_TIMEOUT, OnTimeout::Leave)
}

/// `actinglab --json pause --reason <reason>`: one global scheduling pause.
pub fn pause_all(root: &Path, reason: &str) -> Result<Answer, Said> {
    lab(root, &words(&["pause", "--reason", reason]), PAUSE_TIMEOUT, OnTimeout::Leave)
}

/// `<root>\runtime\actingctl.exe request-shutdown --state-root <root>\state --wait 60`: the
/// Runtime of the root asked to close, and the wait for its owner to close and its process to
/// exit. actingctl is no Lab process: it takes no initiator, and prints its one line without
/// `--json`. The state root is written as the configuration writes it.
pub fn request_shutdown(root: &Path) -> Result<Answer, Said> {
    let state_root = strip_verbatim(&root.join(STATE_DIR).to_string_lossy());
    let spec = Spec::new(&actingctl(root))
        .arg("request-shutdown")
        .arg("--state-root")
        .arg(state_root)
        .arg("--wait")
        .arg(SHUTDOWN_WAIT_SECONDS.to_string())
        .cwd(root);
    answer(&spec, SHUTDOWN_TIMEOUT, OnTimeout::Leave)
}

/// `actinglab --json config replace <file> --sha <sha256>`: the root's configuration replaced by
/// `file`; the wizard runs it with no Runtime running (see the module's description).
pub fn config_replace(root: &Path, file: &Path, sha256: &str) -> Result<Answer, Said> {
    let mut list = words(&["config", "replace"]);
    list.push(file.as_os_str().to_os_string());
    list.extend(words(&["--sha", sha256]));
    lab(root, &list, CONFIG_REPLACE_TIMEOUT, OnTimeout::Leave)
}

/// `actinglab --json status`: the overview, whose `daemon.online` says whether the Runtime
/// answers.
pub fn status(root: &Path) -> Result<Answer, Said> {
    lab(root, &words(&["status"]), STATUS_TIMEOUT, OnTimeout::End)
}

/// `actinglab --json watchdog install`: the root's watchdog task.
pub fn watchdog_install(root: &Path) -> Result<Answer, Said> {
    lab(root, &words(&["watchdog", "install"]), WATCHDOG_TIMEOUT, OnTimeout::Leave)
}

/// The Runtime's process in `up`'s result line. On a single-location root `up` is the
/// installer's start: its line carries the process as the outcome's `pid` value and as
/// `start.ready.pid` (Runtime `crates/lab-rescue/src/install/deploy.rs`, the start's receipt).
/// Neither may be there (a start that was found already under way).
pub fn started_pid(line: &Value) -> Option<u64> {
    line["outcome"]["values"]["pid"]
        .as_u64()
        .or_else(|| line["start"]["ready"]["pid"].as_u64())
}

/// One post-start check whose `result` is `fail`: its id and its values.
#[derive(Debug, Clone, PartialEq)]
pub struct FailedCheck {
    pub id: String,
    pub values: Value,
    /// The row's `fail_class`: whether the Runtime counts this check as a failure of the start
    /// (`postcheck.rs` `CheckRow::fails_class`). A row that does not say counts as one.
    pub fail_class: bool,
}

/// What the wizard reads of `up`'s result line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UpRead {
    /// [`started_pid`].
    pub pid: Option<u64>,
    /// How many post-start checks the line lists; `None` when it lists none.
    pub checks: Option<usize>,
    /// Each listed check whose `result` is `fail`, in the line's order.
    pub failed: Vec<FailedCheck>,
    /// The top outcome's code when its category is `warning`.
    pub caveat: Option<String>,
}

/// `up`'s line read: on a single-location root its `post_check` is the post-start step's
/// evidence `{commit, checks: [{id, result, fail_class, values}], failed}` (Runtime
/// `crates/lab-rescue/src/install/deploy.rs`, the `Start` answer and the `PostCheck` step;
/// `crates/lab-rescue/src/install/postcheck.rs` `CheckRow`); a bare list of rows, as a refusal
/// carries them, is read the same way.
pub fn read_up(line: &Value) -> UpRead {
    let rows = match &line["post_check"] {
        Value::Array(rows) => Some(rows),
        evidence => evidence["checks"].as_array(),
    };
    let outcome = outcome_of(line);
    let caveat = (outcome.category.as_deref() == Some("warning")).then(|| outcome.code.clone()).flatten();
    UpRead {
        pid: started_pid(line),
        checks: rows.map(Vec::len),
        failed: rows
            .into_iter()
            .flatten()
            .filter(|row| row["result"].as_str() == Some("fail"))
            .map(|row| FailedCheck {
                id: row["id"].as_str().unwrap_or("?").to_owned(),
                values: row.get("values").cloned().unwrap_or(Value::Null),
                fail_class: row["fail_class"].as_bool().unwrap_or(true),
            })
            .collect(),
        caveat,
    }
}

/// The failed checks for a person: each id with its values.
pub fn failed_checks_text(failed: &[FailedCheck]) -> String {
    failed
        .iter()
        .map(|check| format!("{} {}", check.id, check.values))
        .collect::<Vec<_>>()
        .join("; ")
}

fn no_line() -> Said {
    Said::new("答复里没有结果行", "the answer has no result line")
}

fn exit_not_zero(code: Option<i32>) -> Said {
    Said::new("命令没有成功", "the command did not succeed")
        .with(format!("exit {}", code.map_or_else(|| "(none)".to_owned(), |code| code.to_string())))
}

/// `up`'s answer judged: exit 0, the post-start checks listed, and none of the failing class
/// `fail`; else why not, those checks named with their values. A check outside that class (the
/// Runtime's `open`, a slow ledger open) that `fail`s is no failure of the install: it stays in
/// [`UpRead::failed`] for the caller to note.
pub fn judge_up(code: Option<i32>, line: Option<&Value>) -> Result<UpRead, Said> {
    if code != Some(0) {
        return Err(exit_not_zero(code));
    }
    let read = read_up(line.ok_or_else(no_line)?);
    if read.checks.is_none() {
        return Err(Said::new(
            "答复里没有启动后检查（post_check.checks）",
            "the answer lists no post-start checks (post_check.checks)",
        ));
    }
    let blocking: Vec<FailedCheck> = read.failed.iter().filter(|check| check.fail_class).cloned().collect();
    if !blocking.is_empty() {
        return Err(Said::new("启动后检查有不通过的项", "post-start checks failed").with(failed_checks_text(&blocking)));
    }
    Ok(read)
}

/// `request-shutdown --wait`'s answer judged: exit 0, the outcome [`SHUTDOWN_COMPLETED`] and the
/// wait's `shutdown.state` `completed` (Runtime `apps/actingctl/src/main.rs`
/// `Command::RequestShutdown`; `crates/lab-ops/src/shutdown.rs` `wait`); else why not.
pub fn judge_shutdown(code: Option<i32>, line: Option<&Value>) -> Result<(), Said> {
    if code != Some(0) {
        return Err(exit_not_zero(code));
    }
    let line = line.ok_or_else(no_line)?;
    let outcome = outcome_of(line);
    if outcome.code.as_deref() != Some(SHUTDOWN_COMPLETED) {
        return Err(Said::new(
            format!("没有看到关闭完成（{SHUTDOWN_COMPLETED}）"),
            format!("the close was not seen to complete ({SHUTDOWN_COMPLETED})"),
        )
        .with(outcome.code.unwrap_or_else(|| "?".to_owned())));
    }
    if line["shutdown"]["state"].as_str() != Some("completed") {
        return Err(Said::new("关闭的等待没有报告完成", "the wait for the close reports no completion")
            .with(line["shutdown"].to_string()));
    }
    Ok(())
}

/// What `config replace` answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Replaced {
    /// The top outcome's code.
    pub code: Option<String>,
    /// `backup`: where the replaced configuration was kept, without the verbatim prefix.
    pub backup: Option<String>,
}

/// `config replace`'s answer judged for a Runtime the wizard closed: exit 0 and `runtime_running`
/// false (Runtime `crates/lab-rescue/src/rescue.rs` `commit`: without a running Runtime the swap
/// alone); a Runtime running again, or a line that does not say, is no success.
pub fn judge_replace(code: Option<i32>, line: Option<&Value>) -> Result<Replaced, Said> {
    if code != Some(0) {
        return Err(exit_not_zero(code));
    }
    let line = line.ok_or_else(no_line)?;
    match line.get("runtime_running") {
        Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => {
            return Err(Said::new(
                "换配置时 Runtime 又在运行了（向导刚把它停下）",
                "a Runtime was running again when the configuration was swapped (the wizard had just closed it)",
            ))
        }
        _ => {
            return Err(Said::new(
                "答复没有说明 Runtime 是否在运行（runtime_running）",
                "the answer does not say whether a Runtime runs (runtime_running)",
            ))
        }
    }
    Ok(Replaced {
        code: outcome_of(line).code,
        backup: line["backup"].as_str().map(strip_verbatim),
    })
}

/// What the wizard reads of `status`'s result line (the overview, Runtime
/// `crates/lab-ops/src/overview.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusRead {
    /// `daemon.online`.
    pub online: bool,
    /// Each instance row's `alias`, in the line's order.
    pub aliases: Vec<String>,
    /// The global pause's reason (`global_pause.pause_reason`), `?` when the block names none;
    /// `None` without the block (no global pause).
    pub global_pause: Option<String>,
    /// Each instance that runs a task now ([`RUN_ACTIVITIES`]): its alias, the kind and the
    /// package when the row names one.
    pub running: Vec<String>,
}

pub fn read_status(line: &Value) -> StatusRead {
    let rows = line["instances"].as_array().map(Vec::as_slice).unwrap_or_default();
    let alias = |row: &Value| row["alias"].as_str().unwrap_or("?").to_owned();
    StatusRead {
        online: line["daemon"]["online"].as_bool() == Some(true),
        aliases: rows.iter().map(alias).collect(),
        global_pause: line.get("global_pause").filter(|pause| !pause.is_null()).map(|pause| {
            pause["pause_reason"]
                .as_str()
                .or_else(|| pause["reason"].as_str())
                .unwrap_or("?")
                .to_owned()
        }),
        running: rows
            .iter()
            .filter_map(|row| {
                let kind = row["activity"]["kind"].as_str()?;
                RUN_ACTIVITIES.contains(&kind).then(|| match row["activity"]["package_id"].as_str() {
                    Some(package) => format!("{} ({kind} {package})", alias(row)),
                    None => format!("{} ({kind})", alias(row)),
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn up_names_its_process_in_the_outcome_or_in_the_ready_start() {
        assert_eq!(started_pid(&json!({"outcome": {"code": "install_command_done", "values": {"pid": 41}}})), Some(41));
        assert_eq!(started_pid(&json!({"start": {"ready": {"pid": 42}}, "outcome": {"code": "x"}})), Some(42));
        // The outcome's value wins over the receipt's.
        assert_eq!(
            started_pid(&json!({"outcome": {"values": {"pid": 41}}, "start": {"ready": {"pid": 42}}})),
            Some(41)
        );
        assert_eq!(started_pid(&json!({"outcome": {"values": {"pid": "41"}}})), None);
        assert_eq!(started_pid(&json!({"outcome": {"code": "install_command_done"}})), None);
    }

    /// The close as it answered on a test machine (2026-11-13), in the members Runtime
    /// `apps/actingctl/src/main.rs` (`Command::RequestShutdown`) and `crates/lab-ops/src/shutdown.rs`
    /// write; the receipt and the numbers abbreviated.
    fn closed() -> Value {
        json!({
            "receipt": {"request_id": "…", "state": "…"},
            "shutdown": {"state": "completed", "closed_at_unix_ms": 1763035920000u64, "elapsed_ms": 1800},
            "outcome": {"schema_version": "actingcommand.outcome.v1", "code": "runtime_shutdown_completed", "category": "success"}
        })
    }

    #[test]
    fn a_close_counts_only_once_it_completed() {
        assert_eq!(judge_shutdown(Some(0), Some(&closed())), Ok(()));
        assert!(judge_shutdown(Some(1), Some(&closed())).is_err());
        assert!(judge_shutdown(None, Some(&closed())).is_err());
        assert!(judge_shutdown(Some(0), None).is_err());
        // Asked but not waited for.
        let mut requested = closed();
        requested["outcome"]["code"] = json!("runtime_shutdown_requested");
        requested.as_object_mut().unwrap().remove("shutdown");
        let problem = judge_shutdown(Some(0), Some(&requested)).unwrap_err();
        assert!(problem.en.contains("runtime_shutdown_requested"), "{problem}");
        // A wait that reports no completion.
        let mut waiting = closed();
        waiting["shutdown"] = json!({"state": "waiting"});
        assert!(judge_shutdown(Some(0), Some(&waiting)).is_err());
    }

    /// `config replace` with no Runtime running, as it answered on the test machine after the
    /// close (2026-11-13), in the members Runtime `crates/lab-rescue/src/rescue.rs` `commit`
    /// writes; the steps' results and the outcome's values abbreviated.
    fn replaced() -> Value {
        json!({
            "backup": "X:\\Root\\actingd.config.json.prev-1763035921000",
            "runtime_running": false,
            "steps": [
                {"step": "check_config", "outcome": {"code": "install_command_done", "category": "success"}, "result": {}},
                {"step": "swap", "outcome": {"code": "install_command_done", "category": "success"}, "result": {}}
            ],
            "outcome": {"schema_version": "actingcommand.outcome.v1", "code": "install_command_done", "category": "success"}
        })
    }

    #[test]
    fn a_replace_counts_only_with_no_runtime_running() {
        assert_eq!(
            judge_replace(Some(0), Some(&replaced())),
            Ok(Replaced {
                code: Some("install_command_done".into()),
                backup: Some(r"X:\Root\actingd.config.json.prev-1763035921000".into()),
            })
        );
        let mut verbatim = replaced();
        verbatim["backup"] = json!(r"\\?\X:\Root\actingd.config.json.prev-1");
        assert_eq!(
            judge_replace(Some(0), Some(&verbatim)).unwrap().backup.as_deref(),
            Some(r"X:\Root\actingd.config.json.prev-1")
        );
        // A Runtime that runs again, and a line that does not say (the answer for a file equal
        // to the live one carries `unchanged` instead).
        let mut running = replaced();
        running["runtime_running"] = json!(true);
        assert!(judge_replace(Some(0), Some(&running)).is_err());
        let unchanged = json!({"unchanged": true, "steps": [], "outcome": {"code": "install_command_done", "category": "success"}});
        assert!(judge_replace(Some(0), Some(&unchanged)).is_err());
        assert!(judge_replace(Some(0), None).is_err());
        // On the running Runtime the test machine had it refused twice (codes as reported, their
        // nesting abbreviated), though the file was swapped.
        let refused = json!({
            "steps": [],
            "outcome": {"code": "install_command_refused", "category": "error", "causes": [
                {"code": "post_check_failed", "category": "error", "relation": "caused_by", "values": {"count": 1}},
                {"code": "install_caveat_noted", "category": "warning", "relation": "related", "causes": [
                    {"code": "guard_failed", "category": "warning", "relation": "related"},
                    {"code": "foreign_pauses", "category": "warning", "relation": "related"}
                ]}
            ]}
        });
        assert!(judge_replace(Some(1), Some(&refused)).is_err());
    }

    /// `up` as it answered on the test machine after the swap (2026-11-13): exit 0 and ten
    /// post-start checks, all `pass`, in the members Runtime `crates/lab-rescue/src/install/
    /// deploy.rs` (the `Start` answer) and `postcheck.rs` (`CheckRow`) write; the values
    /// abbreviated.
    fn started() -> Value {
        let ids = ["ready", "open", "no_seeding", "bindings", "packs", "pauses", "instructions", "no_dispatch", "new_keys", "config"];
        let checks: Vec<Value> = ids
            .iter()
            .map(|id| json!({"id": id, "result": "pass", "fail_class": *id != "open", "values": {}}))
            .collect();
        json!({
            "operation_id": "…",
            "start": {"ready": {"pid": 4242, "owner_epoch": "…"}, "log": "…"},
            "post_check": {"commit": "ef94d950a102a595fa0bde974b8d77ad54329b48", "checks": checks, "failed": []},
            "outcome": {"schema_version": "actingcommand.outcome.v1", "code": "install_command_done", "category": "success", "values": {"pid": 4242}}
        })
    }

    #[test]
    fn up_counts_only_with_no_post_start_check_failed() {
        let read = judge_up(Some(0), Some(&started())).expect("it passes");
        assert_eq!(read.pid, Some(4242));
        assert_eq!(read.checks, Some(10));
        assert!(read.failed.is_empty());
        assert_eq!(read.caveat, None);
        // `not_evaluated` is no failure.
        let mut unevaluated = started();
        unevaluated["post_check"]["checks"][1]["result"] = json!("not_evaluated");
        assert!(judge_up(Some(0), Some(&unevaluated)).is_ok());
        // A start reports failed checks and still exits 0: each is named with its values.
        let mut failed = started();
        failed["post_check"]["checks"][5] =
            json!({"id": "pauses", "result": "fail", "fail_class": true, "values": {"missing": ["first-start"]}});
        failed["post_check"]["checks"][1] = json!({"id": "open", "result": "fail", "fail_class": false, "values": {"slow": true}});
        failed["post_check"]["failed"] = json!(["pauses"]);
        failed["outcome"]["category"] = json!("warning");
        let read = read_up(&failed);
        assert_eq!(
            read.failed,
            vec![
                FailedCheck { id: "open".into(), values: json!({"slow": true}), fail_class: false },
                FailedCheck { id: "pauses".into(), values: json!({"missing": ["first-start"]}), fail_class: true },
            ]
        );
        assert_eq!(read.caveat.as_deref(), Some("install_command_done"));
        // Only the failing class fails the start; the slow open is not named among the reasons.
        let problem = judge_up(Some(0), Some(&failed)).unwrap_err();
        assert!(problem.en.contains(r#"pauses {"missing":["first-start"]}"#), "{problem}");
        assert!(!problem.en.contains("open"), "{problem}");
        // A check outside the failing class that fails alone is no failure: it is kept to note.
        let mut slow = started();
        slow["post_check"]["checks"][1] = json!({"id": "open", "result": "fail", "fail_class": false, "values": {"slow": true}});
        let read = judge_up(Some(0), Some(&slow)).expect("a slow open alone passes");
        assert_eq!(read.failed, vec![FailedCheck { id: "open".into(), values: json!({"slow": true}), fail_class: false }]);
        // A bare list of rows reads the same; a row without an id, values or class still counts.
        let bare = json!({"post_check": [{"result": "fail"}, {"id": "config", "result": "pass"}]});
        assert_eq!(read_up(&bare).failed, vec![FailedCheck { id: "?".into(), values: Value::Null, fail_class: true }]);
        assert_eq!(read_up(&bare).checks, Some(2));
        // Exit 1, no line, no checks listed.
        assert!(judge_up(Some(1), Some(&started())).is_err());
        assert!(judge_up(Some(0), None).is_err());
        let mut unchecked = started();
        unchecked.as_object_mut().unwrap().remove("post_check");
        assert!(judge_up(Some(0), Some(&unchecked)).is_err());
        assert_eq!(read_up(&unchecked).checks, None);
    }

    #[test]
    fn status_reads_the_daemon_the_pause_and_what_runs() {
        let line = json!({
            "daemon": {"online": true, "owner_epoch": "e"},
            "global_pause": {"revision": 1, "pause_reason": "first-start", "since_unix_ms": 5},
            "instances": [
                {"alias": "ak-5", "lease_active": false, "emulator_state": {"state": "off"}},
                {"alias": "al-6", "activity": {"kind": "policy_run", "request_id": "r", "package_id": "al.daily", "since_unix_ms": 9}},
                {"alias": "al-7", "activity": {"kind": "reconnect", "since_unix_ms": 9}},
                {"alias": "al-8", "activity": {"kind": "host_run", "since_unix_ms": 9}},
                {"alias": "al-9", "activity": {"kind": "emulator_control", "request_id": "q", "since_unix_ms": 9}}
            ],
            "outcome": {"code": "runtime_overview_reported", "category": "success"}
        });
        assert_eq!(
            read_status(&line),
            StatusRead {
                online: true,
                aliases: vec!["ak-5".into(), "al-6".into(), "al-7".into(), "al-8".into(), "al-9".into()],
                global_pause: Some("first-start".into()),
                running: vec!["al-6 (policy_run al.daily)".into(), "al-8 (host_run)".into()],
            }
        );
        // Offline: the daemon block says so, and there are no rows.
        let offline = read_status(&json!({"daemon": {"online": false, "error": {}}}));
        assert_eq!(offline, StatusRead::default());
        // A pause block without a reason still is a pause; a null one is none.
        assert_eq!(read_status(&json!({"global_pause": {"revision": 2}})).global_pause.as_deref(), Some("?"));
        assert_eq!(read_status(&json!({"global_pause": null})).global_pause, None);
        // An older line's `reason`.
        assert_eq!(read_status(&json!({"global_pause": {"reason": "x"}})).global_pause.as_deref(), Some("x"));
        assert!(!read_status(&json!({"daemon": {"online": "yes"}})).online);
    }
}
