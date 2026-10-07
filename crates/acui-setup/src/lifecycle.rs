// SPDX-License-Identifier: GPL-3.0-only
//! acsetup consumes the Host's installation transition and formal shutdown wait.
//! The Host owns admission, tickets, effects and their GlobalLedger facts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

const HOST_TIMEOUT_MS: u64 = 60_000;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(75);
const POLL: Duration = Duration::from_millis(250);

pub struct Closed {
    pub was_running: bool,
    pub previous: Option<InstallTransitionTicket>,
}

/// The pid the Runtime's discovery file names, when there is one: the owner a
/// transaction is about to close, which may use its own programs in a pre-check.
pub fn owner_pid(state_root: &Path) -> Option<u32> {
    let bytes =
        acui_installation::read_bounded(&state_root.join("runtime-info.json"), 1024 * 1024)
            .ok()?;
    let info: Value = serde_json::from_slice(&bytes).ok()?;
    info["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok())
}

/// How acsetup closes and starts a Runtime: the `install-control` revision it and that Runtime
/// both speak (`interfaces`, Workflow #364), never a commit. 0 is the cold protocol
/// (`request-shutdown`, a start without `--install-held`), 1 the Host installation transition.
/// An unsupported control call is a visible failure.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Cold,
    Transition,
}

impl Control {
    pub fn from_revision(revision: u32) -> Self {
        match revision {
            0 => Control::Cold,
            _ => Control::Transition,
        }
    }
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
    how: Control,
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
    if how == Control::Cold {
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
    how: Control,
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
    // cmd.exe and WMI take plain paths only (never the canonical `\\?\` spelling).
    let plain = crate::generations::plain;
    let root = plain(&snapshot.root);
    let log = root.join(format!("actingd-{}.log", crate::log::unix_ms()));
    // The log exists before the Runtime does; a name already taken is a visible failure.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log)
        .map_err(|error| format!("Cannot create Runtime process log: {error}"))?;
    let mut arguments = vec![
        "--config".to_string(),
        plain(&snapshot.config_path()?).display().to_string(),
    ];
    if how == Control::Transition {
        arguments.push("--install-held".to_string());
        arguments.push(serde_json::to_string(&startup).map_err(|error| error.to_string())?);
    }
    // Ruling X3 (Workflow #364): the Runtime starts as the coordinator's restart_actingd.ps1
    // starts it — through WMI, in a hidden window, outside any app job and the caller's job —
    // so nothing that ends acsetup or whoever ran it ends the Runtime, and no window can be
    // closed under it; its output goes to the log. It gets no installation variable: the slot's
    // actingd reads the selection this transaction has just committed, which the writer lock
    // keeps in place, and checks that it is that slot's program.
    let started = crate::platform::start_hidden(
        &plain(&programs.join("runtime").join(ACTINGD)),
        &arguments,
        &root,
        &log,
    )
    .map_err(|error| {
        format!(
            "Runtime start attempt failed: {error}; log {}; selected generation retained",
            log.display()
        )
    })?;
    report.line(&format!(
        "Runtime 已经 WMI 以隐藏窗口拉起，不属于任何作业 / Runtime started hidden through WMI, outside any job: pid {}（cmd {}）；日志 / log {}",
        started.pid,
        started.launcher,
        log.display()
    ))?;
    let deadline = Instant::now() + Duration::from_millis(HOST_TIMEOUT_MS);
    loop {
        if let Some(code) = started.exited()? {
            return Err(format!(
                "Runtime exited during startup: exit code {code}{}; log {}; selected generation retained",
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
        if info.as_ref().and_then(|info| info["pid"].as_u64()) == Some(u64::from(started.pid)) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Runtime startup observation timed out; pid {}, log {}; selected generation retained, process not terminated",
                started.pid,
                log.display()
            ));
        }
        std::thread::sleep(POLL);
    }
    if how == Control::Cold {
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
        || held.ticket.target.pid != started.pid
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
