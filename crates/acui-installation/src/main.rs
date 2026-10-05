// SPDX-License-Identifier: GPL-3.0-only
//! Stable product entries forward one invocation to one selected program slot.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use acui_installation::Snapshot;

fn main() {
    match forward() {
        Ok(code) => std::process::exit(code),
        Err(reason) => {
            eprintln!("FATAL acforward: {reason}");
            std::process::exit(1);
        }
    }
}

fn forward() -> Result<i32, String> {
    let entry =
        std::env::current_exe().map_err(|error| format!("Cannot locate stable entry: {error}"))?;
    let filename = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Stable entry has no UTF-8 filename")?;
    let directory = entry.parent().ok_or("Stable entry has no directory")?;
    let (component, expected) = match filename {
        "actingcommand-actingd.exe" | "actingctl.exe" => ("runtime", filename),
        "actinglab.exe"
        | "actingledger.exe"
        | "actingcommand-vision-provider-check.exe"
        | "actingcommand-device-test.exe" => ("tools", filename),
        "acui.exe" | "acsetup.exe" => ("ui", filename),
        _ => return Err(format!("Unknown stable product entry: {filename}")),
    };
    if directory.file_name().and_then(|name| name.to_str()) != Some(component) {
        return Err("Stable product entry is outside its component directory".into());
    }
    let root = directory
        .parent()
        .ok_or("Stable entry has no installation root")?;
    let snapshot = match Snapshot::inherited()? {
        Some(snapshot) if snapshot.root == root => snapshot,
        Some(_) => return Err("Inherited installation root does not match this entry".into()),
        None => Snapshot::read(root)?,
    };
    let target = snapshot.slot_root().join(component).join(expected);
    let mut arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if component == "runtime" {
        resolve_config_argument(&snapshot, &mut arguments)?;
    }
    if filename == "actingcommand-actingd.exe"
        && !arguments.iter().any(|argument| {
            argument == "--config"
                || argument
                    .to_str()
                    .is_some_and(|text| text.starts_with("--config="))
        })
    {
        arguments.push("--config".into());
        arguments.push(snapshot.config_path()?.into_os_string());
    }
    let mut command = Command::new(&target);
    snapshot.apply_to(&mut command)?;
    let status = command
        .args(arguments)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("Cannot run {}: {error}", target.display()))?;
    status
        .code()
        .ok_or_else(|| format!("Child terminated without an exit code: {status}"))
}

fn resolve_config_argument(snapshot: &Snapshot, arguments: &mut [OsString]) -> Result<(), String> {
    let root_config = snapshot.root.join("actingd.config.json");
    for index in 0..arguments.len() {
        if arguments[index] == "--config" {
            let value = arguments
                .get_mut(index + 1)
                .ok_or("--config requires a path")?;
            if same_path(value, &root_config)? {
                *value = snapshot.config_path()?.into_os_string();
            }
        } else if let Some(text) = arguments[index]
            .to_str()
            .and_then(|text| text.strip_prefix("--config="))
        {
            if same_path(&OsString::from(text), &root_config)? {
                let mut value = OsString::from("--config=");
                value.push(snapshot.config_path()?);
                arguments[index] = value;
            }
        }
    }
    Ok(())
}

fn same_path(value: &OsString, expected: &Path) -> Result<bool, String> {
    let actual = std::path::absolute(PathBuf::from(value))
        .map_err(|error| format!("Cannot resolve --config: {error}"))?;
    Ok(actual == expected)
}
