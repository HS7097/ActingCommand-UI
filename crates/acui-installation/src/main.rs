// SPDX-License-Identifier: GPL-3.0-only
//! Stable product entries forward one invocation to one selected program slot.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use acui_installation::{Snapshot, OBSERVER_UI_ENV};

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
    // The tools are the installation root's own files, never forwarded
    // (Workflow #359): a slot holds only the runtime and the ui (#360).
    let (component, expected) = match filename {
        "actingcommand-actingd.exe" | "actingctl.exe" => ("runtime", filename),
        "acui.exe" => ("ui", filename),
        _ => return Err(format!("Unknown stable product entry: {filename}")),
    };
    if directory.file_name().and_then(|name| name.to_str()) != Some(component) {
        return Err("Stable product entry is outside its component directory".into());
    }
    let root = directory
        .parent()
        .ok_or("Stable entry has no installation root")?;
    let snapshot = match Snapshot::inherited()? {
        Some(snapshot) if acui_installation::same_install_root(&snapshot.root, root)? => snapshot,
        Some(_) => return Err("Inherited installation root does not match this entry".into()),
        None => Snapshot::for_launch(root)?,
    };
    let target = snapshot.slot_root().join(component).join(expected);
    let mut arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let observer_ui = if filename == "acui.exe" {
        require_location(&mut arguments, "--state-root", &snapshot.state_root()?)?;
        !snapshot.ui_supports_configuration()?
    } else {
        false
    };
    if filename == "actingctl.exe"
        && arguments
            .first()
            .is_some_and(|argument| argument == "mcp-serve")
        && !arguments.iter().any(|argument| argument == "--list-tools")
    {
        require_location(&mut arguments, "--root", &snapshot.root)?;
        require_location(&mut arguments, "--state-root", &snapshot.state_root()?)?;
    }
    if filename == "actingcommand-actingd.exe" && std::env::var_os(OBSERVER_UI_ENV).is_some() {
        let read_only = arguments
            .first()
            .is_some_and(|argument| argument == "check-config" || argument == "suspended");
        if !read_only {
            return Err("This console is an observation entry. Use the fixed acsetup management entry for a controlled Runtime start or configuration change".into());
        }
    }
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
    if filename == "acui.exe" {
        if observer_ui {
            command.env(OBSERVER_UI_ENV, "1");
        } else {
            command.env_remove(OBSERVER_UI_ENV);
        }
    }
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

/// Location arguments bind the same installation as the inherited selection.
/// A matching relative spelling is made absolute before changing the child's cwd.
fn require_location(
    arguments: &mut Vec<OsString>,
    flag: &str,
    expected: &Path,
) -> Result<(), String> {
    let mut found = false;
    let equals = format!("{flag}=");
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == flag {
            if found {
                return Err(format!("{flag} was supplied more than once"));
            }
            found = true;
            let value = arguments
                .get_mut(index + 1)
                .ok_or_else(|| format!("{flag} requires a path"))?;
            if !same_path(value, expected)? {
                return Err(format!("{flag} conflicts with the selected installation"));
            }
            *value = expected.as_os_str().to_owned();
            index += 1;
        } else if arguments[index]
            .to_str()
            .is_some_and(|value| value.starts_with(&equals))
        {
            return Err(format!(
                "Use {flag} followed by its path as a separate argument"
            ));
        }
        index += 1;
    }
    if !found {
        arguments.extend([OsString::from(flag), expected.as_os_str().to_owned()]);
    }
    Ok(())
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
        .map_err(|error| format!("Cannot resolve entry path: {error}"))?;
    if actual == expected {
        return Ok(true);
    }
    if let (Ok(actual), Ok(expected)) = (actual.canonicalize(), expected.canonicalize()) {
        return Ok(actual == expected);
    }
    // The root-level configuration argument is an alias, so the file is absent.
    match (
        actual.parent(),
        expected.parent(),
        actual.file_name(),
        expected.file_name(),
    ) {
        (Some(left), Some(right), Some(left_name), Some(right_name)) => {
            let same_name = if cfg!(windows) {
                left_name
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right_name.to_string_lossy())
            } else {
                left_name == right_name
            };
            Ok(same_name
                && left.canonicalize().map_err(|error| error.to_string())?
                    == right.canonicalize().map_err(|error| error.to_string())?)
        }
        _ => Ok(false),
    }
}
