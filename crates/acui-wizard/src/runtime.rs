// SPDX-License-Identifier: GPL-3.0-only
//! The Runtime's own programs, run from the new root, each through the face the Runtime's route
//! leads to (Runtime `contracts/lab-two-entries.md`: `actingctl` leaves with LAB-7, the Lab's
//! rows take its place): a row of `tools\actinglab.exe` wherever one serves the step, the
//! Runtime's own `runtime\actingcommand-actingd.exe check-config` where none can yet. Every Lab
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
//! | start | `actinglab --json up` (rescue row; on a single-location root `install start`: `rescue.rs` `up`) | served row; the installer's start with its post-start checks |
//! | first pause | `actinglab --json pause --reason first-start` (core row, global without an instance: `crates/lab-ops/src/scheduling.rs`) | served row; Lab envelopes are taken since 0.12.58 (LAB-6S1b) |
//! | status | `actinglab --json status` (core row: `crates/lab-ops/src/overview.rs`, `daemon.online`) | served row |
//! | watchdog | `actinglab --json watchdog install` (rescue row: `rescue.rs` `watchdog`) | served row |

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::child::{self, json_line, outcome_of, outcome_text, OnTimeout, Spec};
use crate::text::Said;
use crate::winpath::strip_verbatim;

pub const ACTINGD_EXE: &str = "actingcommand-actingd.exe";
pub const ACTINGLAB_EXE: &str = "actinglab.exe";
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
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(3 * 60);
pub const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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

/// `actinglab --json status`: the overview, whose `daemon.online` says whether the Runtime
/// answers.
pub fn status(root: &Path) -> Result<Answer, Said> {
    lab(root, &words(&["status"]), STATUS_TIMEOUT, OnTimeout::End)
}

/// `actinglab --json watchdog install`: the root's watchdog task.
pub fn watchdog_install(root: &Path) -> Result<Answer, Said> {
    lab(root, &words(&["watchdog", "install"]), WATCHDOG_TIMEOUT, OnTimeout::Leave)
}
