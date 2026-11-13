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
//! | the instances | `actinglab --json config replace <full configuration> --sha <its SHA-256>` (rescue row: `rescue.rs` `config_swap` and `commit`; `crates/lab-rescue/src/install/config_swap.rs`) | served row: the live build's check-config first, then, with the Runtime running, its own hold (which keeps a global pause), the close, the swap with the replaced file kept as `actingd.config.json.prev-<ms>`, the start, the post-start checks, `alive` and the release of its own hold only (`contracts/install-cli.md`, "The Lab's rescue module") |
//! | status | `actinglab --json status` (core row: `crates/lab-ops/src/overview.rs`, `daemon.online`) | served row |
//! | watchdog | `actinglab --json watchdog install` (rescue row: `rescue.rs` `watchdog`) | served row |
//!
//! Why the instances come by `config replace`: a global pause stops no run already under way
//! (`contracts/scheduling-pause.md`, "Global pause"), so a Runtime started with its instances
//! could dispatch in the moment between the start and the pause. The wizard starts the Runtime
//! with a configuration without instances, sets the pause, and only then puts the full
//! configuration in place. The pause is a runtime fact that survives a restart; every start that
//! is not installer-held, the module's included, restores it before any physical instance is
//! prepared and before the policy driver runs (`scheduling-pause.md`, "Persistence, no expiry";
//! `install-cli.md`, the module's "Why the reason and the alias"), and `config replace` sets no
//! global pause of its own and hands back only its own (`install-cli.md`, INST-5b).

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
/// A close (its drain bounded by `install.stop.drain_timeout_ms`), a start as long as `up`'s and
/// the wait for `alive` within `install.start.ready_timeout_ms`.
pub const CONFIG_REPLACE_TIMEOUT: Duration = Duration::from_secs(50 * 60);
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

/// `actinglab --json config replace <file> --sha <sha256>`: the root's configuration replaced by
/// `file` (see the module's description).
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
