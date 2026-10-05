// SPDX-License-Identifier: GPL-3.0-only
//! acsetup consumes the Host's installation transition and formal shutdown wait.
//! The Host owns admission, tickets, effects and their GlobalLedger facts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use actingcommand_contract::{
    IdentifierIssuer, InstallHeldStartup, InstallTransitionAction as Action,
    InstallTransitionPhase as Phase, InstallTransitionStatus, InstallTransitionTicket,
    RuntimeReceipt, RuntimeResult,
};
use acui_installation::Snapshot;
use serde_json::Value;

use crate::runtime::{self, ACTINGCTL, ACTINGD};
use crate::verify::Report;

pub const COLD_RUNTIME: &str = "b70518949c19d56085afc3a84c49274c4cc041fe";
const HOST_TIMEOUT_MS: u64 = 60_000;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(75);
const POLL: Duration = Duration::from_millis(250);

pub struct Closed {
    pub was_running: bool,
    pub previous: Option<InstallTransitionTicket>,
}

/// Only the named v0.11.0 source uses cold startup. Other releases must answer
/// the current protocol; an unsupported control call is a visible failure.
pub fn cold(runtime_sha: &str) -> bool {
    runtime_sha == COLD_RUNTIME
}

pub fn command(program: &Path, snapshot: Option<&Snapshot>) -> Result<Command, String> {
    let mut command = Command::new(program);
    command
        .env_remove(acui_installation::INSTALL_ROOT_ENV)
        .env_remove(acui_installation::INSTALL_SELECTION_ENV)
        .env_remove(acui_installation::OBSERVER_UI_ENV);
    if let Some(snapshot) = snapshot {
        snapshot.apply_to(&mut command)?;
    }
    Ok(command)
}

fn control(
    programs: &Path,
    state_root: &Path,
    snapshot: Option<&Snapshot>,
    action: &Action,
    wait: bool,
) -> Result<Value, String> {
    action.validate().map_err(|error| error.to_string())?;
    let mut command = command(&programs.join("runtime").join(ACTINGCTL), snapshot)?;
    command
        .arg("install-transition")
        .arg("--state-root")
        .arg(state_root)
        .arg("--action-json")
        .arg(serde_json::to_string(action).map_err(|error| error.to_string())?);
    if wait {
        command.args(["--wait", "60"]);
    }
    parse(
        runtime::run_observed(&mut command, CONTROL_TIMEOUT)?,
        "install-transition",
    )
}

fn parse(output: runtime::Output, operation: &str) -> Result<Value, String> {
    if !output.success {
        return Err(format!(
            "{operation} failed (exit {}): {} {}",
            output.exit,
            output.stdout.trim(),
            output.stderr.trim()
        ));
    }
    serde_json::from_str(&output.stdout).map_err(|error| {
        format!(
            "{operation} output unreadable: {error}; stdout={} stderr={}",
            output.stdout.trim(),
            output.stderr.trim()
        )
    })
}

fn receipt(value: &Value) -> Result<RuntimeReceipt, String> {
    let receipt: RuntimeReceipt = serde_json::from_value(value["receipt"].clone())
        .map_err(|error| format!("Installation control receipt unreadable: {error}; {value}"))?;
    receipt
        .validate()
        .map_err(|error| format!("Invalid installation control receipt: {error}; {value}"))?;
    Ok(receipt)
}

fn status(value: Value, transition: &str) -> Result<InstallTransitionStatus, String> {
    let receipt = receipt(&value)?;
    match receipt.result() {
        Some(RuntimeResult::InstallTransition { status })
            if status.ticket.transition_id == transition =>
        {
            Ok(status.clone())
        }
        _ => Err(format!(
            "Host did not return the requested installation transition: {value}"
        )),
    }
}

fn query(
    programs: &Path,
    state_root: &Path,
    snapshot: Option<&Snapshot>,
    transition: &str,
) -> Result<InstallTransitionStatus, String> {
    status(
        control(
            programs,
            state_root,
            snapshot,
            &Action::Query {
                transition_id: transition.into(),
            },
            false,
        )?,
        transition,
    )
}

fn transition_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("Cannot issue installation transition: {error}"))?;
    Ok(format!("acsetup-{}", crate::verify::hex(&bytes)))
}

fn phase_error(status: &InstallTransitionStatus) -> String {
    format!(
        "Host installation transition stopped: {}",
        serde_json::to_string(status)
            .unwrap_or_else(|error| format!("status encoding failed: {error}"))
    )
}

/// Closed data is proved by the Runtime's existing maintenance gate, including
/// its owner/writer locks. A failed status call alone never proves shutdown.
pub fn verify_ledger(programs: &Path, config: &Path, report: Report<'_>) -> Result<(), String> {
    runtime::check_config(&programs.join("runtime").join(ACTINGD), config)?;
    report.line("冷态账本验证会更新 owner.lock 代际；不启动 Provider / Cold ledger verification updates the owner journal epoch without starting Provider")?;
    let mut command = command(&programs.join("runtime").join(ACTINGD), None)?;
    command
        .args(["ledger-maintenance", "verify", "--config"])
        .arg(config);
    let result = parse(
        runtime::run_observed(&mut command, Duration::from_secs(150))?,
        "ledger-maintenance verify",
    )?;
    if result["schema_version"] != "actingcommand.ledger-maintenance.v1"
        || !matches!(
            result["status"].as_str(),
            Some("verified-sqlite" | "verified-segment")
        )
        || result["activated"] != false
    {
        return Err(format!("Cold compatibility gate is unproved: {result}"));
    }
    report.line(&format!(
        "冷态账本验证通过 / Cold ledger verification completed: {result}"
    ))
}

pub fn close(
    programs: &Path,
    config: &Path,
    state_root: &Path,
    snapshot: Option<&Snapshot>,
    runtime_sha: &str,
    report: Report<'_>,
) -> Result<Closed, String> {
    let mut probe = command(&programs.join("runtime").join(ACTINGCTL), snapshot)?;
    probe.arg("status").arg("--state-root").arg(state_root);
    let observed = runtime::run_observed(&mut probe, CONTROL_TIMEOUT)?;
    if !observed.success {
        report.warn(&format!("Runtime status unavailable (exit {}): {} {}; checking the formal cold owner/writer gate", observed.exit, observed.stdout.trim(), observed.stderr.trim()))?;
        verify_ledger(programs, config, report)?;
        return Ok(Closed {
            was_running: false,
            previous: None,
        });
    }
    if cold(runtime_sha) {
        let mut shutdown = command(&programs.join("runtime").join(ACTINGCTL), snapshot)?;
        shutdown
            .arg("request-shutdown")
            .arg("--state-root")
            .arg(state_root)
            .args(["--wait", "60"]);
        let result = parse(
            runtime::run_observed(&mut shutdown, CONTROL_TIMEOUT)?,
            "cold request-shutdown",
        )?;
        confirm_closed(&result, None)?;
        report.line("原 owner 的原子关闭及进程退出已确认 / Atomic shutdown and process exit of the original owner confirmed")?;
        return Ok(Closed {
            was_running: true,
            previous: None,
        });
    }
    let transition = transition_id()?;
    let deadline = Instant::now() + Duration::from_millis(HOST_TIMEOUT_MS);
    let begun = control(
        programs,
        state_root,
        snapshot,
        &Action::BeginDrain {
            transition_id: transition.clone(),
            timeout_ms: HOST_TIMEOUT_MS,
        },
        false,
    )
    .and_then(|value| status(value, &transition));
    let mut observed = match begun {
        Ok(status) => status,
        Err(error) => {
            report.warn(&format!("Drain submission outcome unresolved: {error}; querying original transition {transition} once, without resubmission"))?;
            query(programs, state_root, snapshot, &transition)
                .map_err(|query| format!("{error}; original transition query failed: {query}"))?
        }
    };
    let ticket = observed.ticket.clone();
    loop {
        if observed.ticket != ticket {
            return Err("Drain query changed the original owner/ticket".into());
        }
        match observed.phase {
            Phase::Drained => break,
            Phase::Draining if Instant::now() < deadline => {}
            _ => return Err(phase_error(&observed)),
        }
        std::thread::sleep(POLL);
        observed = query(programs, state_root, snapshot, &transition)?;
    }
    let result = control(
        programs,
        state_root,
        snapshot,
        &Action::CommitShutdown {
            ticket: ticket.clone(),
        },
        true,
    )
    .and_then(|value| confirm_closed(&value, Some(&ticket)).map(|()| value));
    if let Err(error) = result {
        report.warn(&format!("Shutdown outcome unconfirmed: {error}; selection remains unchanged; querying {transition} without resubmission"))?;
        let query_result = query(programs, state_root, snapshot, &transition);
        return Err(format!(
            "{error}; original transition query: {}; no selection or new owner started",
            match query_result {
                Ok(status) => phase_error(&status),
                Err(query) => query,
            }
        ));
    }
    report.line("Host 排空、原 owner 关闭和进程退出已确认 / Host drain, original owner closure and process exit confirmed")?;
    Ok(Closed {
        was_running: true,
        previous: Some(ticket),
    })
}

fn confirm_closed(value: &Value, ticket: Option<&InstallTransitionTicket>) -> Result<(), String> {
    let receipt = receipt(value)?;
    match receipt.result() {
        Some(RuntimeResult::ShutdownAccepted { target })
            if ticket.is_none_or(|ticket| ticket.target == *target)
                && value["shutdown"]["state"] == "completed"
                && value["shutdown"]["closed_at_unix_ms"].as_u64().is_some() =>
        {
            Ok(())
        }
        _ => Err(format!("Original owner closure is unconfirmed: {value}")),
    }
}

pub fn start(
    snapshot: &Snapshot,
    closed: Closed,
    runtime_sha: &str,
    report: Report<'_>,
) -> Result<Option<PathBuf>, String> {
    if !closed.was_running {
        return Ok(None);
    }
    let transition = match &closed.previous {
        Some(ticket) => ticket.transition_id.clone(),
        None => transition_id()?,
    };
    let request_id = *IdentifierIssuer::new()
        .map_err(|error| error.to_string())?
        .mint_request_id()
        .map_err(|error| error.to_string())?
        .transport();
    let startup = InstallHeldStartup {
        transition_id: transition.clone(),
        request_id,
        timeout_ms: HOST_TIMEOUT_MS,
        previous: closed.previous,
    };
    startup.validate().map_err(|error| error.to_string())?;
    let programs = snapshot.slot_root();
    let state_root = snapshot.state_root()?;
    let log = snapshot
        .root
        .join(format!("actingd-{}.log", crate::log::unix_ms()));
    let stdout = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log)
        .map_err(|error| format!("Cannot create Runtime process log: {error}"))?;
    let stderr = stdout.try_clone().map_err(|error| error.to_string())?;
    let mut command = command(&programs.join("runtime").join(ACTINGD), Some(snapshot))?;
    command.arg("--config").arg(snapshot.config_path()?);
    if !cold(runtime_sha) {
        command
            .arg("--install-held")
            .arg(serde_json::to_string(&startup).map_err(|error| error.to_string())?);
    }
    command
        .current_dir(&snapshot.root)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(runtime::DETACHED_PROCESS);
    }
    let mut child = command.spawn().map_err(|error| {
        format!("Runtime start attempt failed: {error}; selected generation retained")
    })?;
    let deadline = Instant::now() + Duration::from_millis(HOST_TIMEOUT_MS);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Cannot observe started Runtime: {error}"))?
        {
            return Err(format!(
                "Runtime exited during startup: {status}{}; log {}; selected generation retained",
                runtime::fatal_line(&log),
                log.display()
            ));
        }
        let info = state_root.join("runtime-info.json");
        let info = match acui_installation::read_bounded(&info, 1024 * 1024) {
            Ok(bytes) => Some(
                serde_json::from_slice::<Value>(&bytes)
                    .map_err(|error| format!("Runtime discovery input unreadable: {error}"))?,
            ),
            Err(error)
                if !info
                    .try_exists()
                    .map_err(|inspect| format!("{error}; {inspect}"))? =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        if info.as_ref().and_then(|info| info["pid"].as_u64()) == Some(u64::from(child.id())) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Runtime startup observation timed out; pid {}, log {}; selected generation retained, process not terminated",
                child.id(),
                log.display()
            ));
        }
        std::thread::sleep(POLL);
    }
    if cold(runtime_sha) {
        let mut probe = self::command(&programs.join("runtime").join(ACTINGCTL), Some(snapshot))?;
        probe.arg("status").arg("--state-root").arg(&state_root);
        parse(
            runtime::run_observed(&mut probe, CONTROL_TIMEOUT)?,
            "cold Runtime startup status",
        )?;
        report.line("旧槽冷态 Runtime 已响应；该启动包含旧版原有 Provider/设备准备 / Cold Runtime responds after its existing startup preparation")?;
        return Ok(Some(log));
    }
    let held = query(&programs, &state_root, Some(snapshot), &transition)?;
    if held.phase != Phase::Held
        || held.ticket.target.pid != child.id()
        || held.ticket.request_id != request_id
        || startup
            .previous
            .as_ref()
            .is_some_and(|previous| previous.target == held.ticket.target)
    {
        return Err(format!(
            "Started Runtime did not prove the exact held owner: {}",
            phase_error(&held)
        ));
    }
    report.line(
        "准确新 owner 已 held，尚未进入 Provider / Exact new owner is held before Provider",
    )?;
    let ticket = held.ticket;
    let deadline = Instant::now() + Duration::from_millis(HOST_TIMEOUT_MS);
    let released = control(
        &programs,
        &state_root,
        Some(snapshot),
        &Action::Release {
            ticket: ticket.clone(),
            timeout_ms: HOST_TIMEOUT_MS,
        },
        false,
    )
    .and_then(|value| status(value, &transition));
    let mut observed = match released {
        Ok(status) => status,
        Err(error) => {
            report.warn(&format!(
                "Release outcome unresolved: {error}; querying original ticket without resubmission"
            ))?;
            query(&programs, &state_root, Some(snapshot), &transition)
                .map_err(|query| format!("{error}; {query}"))?
        }
    };
    loop {
        if observed.ticket != ticket {
            return Err("Release query changed the exact held owner/ticket".into());
        }
        match observed.phase {
            Phase::Released => break,
            Phase::Preparing if Instant::now() < deadline => {}
            _ => return Err(phase_error(&observed)),
        }
        std::thread::sleep(POLL);
        observed = query(&programs, &state_root, Some(snapshot), &transition)?;
    }
    report.line("Host 已完成准备和本次暂停恢复并放行 / Host completed preparation and this transition's pause restoration and released admission")?;
    Ok(Some(log))
}
