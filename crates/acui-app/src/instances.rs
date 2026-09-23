// SPDX-License-Identifier: GPL-3.0-only
//! The instance-configuration window: a new entry added to the `instances` of
//! the actingd configuration the launcher starts with.
//!
//! The file is plain JSON here. Only `instances` changes, and in an entry only
//! the keys the form manages; the Runtime's config struct is not mirrored. A
//! save re-reads the file, writes a candidate beside it (relative paths inside
//! resolve against that directory), runs `<actingd_exe> check-config` on it off
//! the event loop, and renames it over the file only on a parsed `ok` with a
//! successful exit.

use std::cell::RefCell;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use slint::{ComponentHandle, SharedString};

use crate::launcher::configured;
use crate::strings::{fill, Labels};
use crate::{shared, App, AppWindow, ConfigStrings, ConfigWindow, Scale};

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_SCHEMA: &str = "actingcommand.actingd.check-config.v1";

/// Windows `CREATE_NO_WINDOW`: no console window flashes up per save.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The binding kinds in the kind box's order, which is also the Runtime's
/// order of precedence, and the keys each one writes.
const KINDS: [&[&str]; 3] = [&["instance_index"], &["instance_name"], &["host", "port"]];

struct Editor {
    app: Rc<App>,
    /// The instance_id the form saves under; `None` when none could be made.
    editing: RefCell<Option<String>>,
}

/// The managed keys in the schema's spelling; `None` removes an optional key.
struct Form {
    kind: usize,
    fields: Vec<(&'static str, Option<Value>)>,
}

pub fn install(window: &AppWindow, config: &ConfigWindow, app: &Rc<App>) {
    fill_strings(window.global::<ConfigStrings>(), app.labels);
    fill_strings(config.global::<ConfigStrings>(), app.labels);
    let editor = Rc::new(Editor { app: Rc::clone(app), editing: RefCell::new(None) });
    {
        let (main, weak, editor) = (window.as_weak(), config.as_weak(), Rc::clone(&editor));
        window.on_open_config(move || {
            let (Some(main), Some(config)) = (main.upgrade(), weak.upgrade()) else {
                return;
            };
            config.global::<Scale>().set_factor(main.global::<Scale>().get_factor());
            if editor.editing.borrow().is_none() {
                begin_new(&editor, &config);
            }
            if let Err(error) = config.show() {
                main.set_launcher_line(error.to_string().into());
            }
        });
    }
    let on = |action: fn(&Editor, &ConfigWindow)| {
        let (weak, editor) = (config.as_weak(), Rc::clone(&editor));
        move || {
            if let Some(config) = weak.upgrade() {
                action(&editor, &config);
            }
        }
    };
    config.on_add_instance(on(begin_new));
    config.on_save(on(save));
}

fn fill_strings(global: ConfigStrings<'_>, labels: &Labels) {
    global.set_title(labels.instance_config.into());
    global.set_intro(labels.config_intro.into());
    global.set_add(labels.add_instance.into());
    global.set_fields(shared(&labels.form_fields));
    global.set_kinds(shared(&labels.binding_kinds));
    global.set_placeholders(shared(&labels.form_placeholders));
    global.set_optional_note(labels.optional_note.into());
    global.set_save(labels.check_and_save.into());
}

/// A new entry: a fresh instance_id from the OS RNG and an empty form.
fn begin_new(editor: &Editor, config: &ConfigWindow) {
    let mut bytes = [0u8; 16];
    let id = getrandom::fill(&mut bytes).map(|()| {
        bytes.iter().fold("instance_".to_string(), |id, byte| id + &format!("{byte:02x}"))
    });
    let failure =
        id.as_ref().err().map(|error| fill(editor.app.labels.id_failed, &[&error.to_string()]));
    set_outcome(config, failure.is_some(), failure.unwrap_or_default());
    show_form(config, id.as_deref().ok(), &Value::Null);
    *editor.editing.borrow_mut() = id.ok();
}

fn show_form(config: &ConfigWindow, id: Option<&str>, entry: &Value) {
    let value = |key| SharedString::from(text(entry, key).unwrap_or_default());
    config.set_form_ready(id.is_some());
    config.set_form_id(id.unwrap_or_default().into());
    config.set_form_kind(kind_of(entry).unwrap_or(0) as i32);
    config.set_form_alias(value("alias"));
    config.set_form_index(value("instance_index"));
    config.set_form_name(value("instance_name"));
    config.set_form_host(value("host"));
    config.set_form_port(value("port"));
    config.set_form_adb(value("adb_path"));
    config.set_form_nemu(value("nemu_app_index"));
    config.set_form_application(value("application_id"));
    config.set_form_capture(value("capture_backend"));
    config.set_form_touch(value("touch_backend"));
}

fn set_outcome(config: &ConfigWindow, failed: bool, text: impl Into<SharedString>) {
    config.set_outcome_failed(failed);
    config.set_outcome(text.into());
}

fn save(editor: &Editor, config: &ConfigWindow) {
    let labels = editor.app.labels;
    let Some(id) = editor.editing.borrow().clone() else {
        return;
    };
    // The very file and executable Start uses, as this run resolved them.
    let launcher = &editor.app.launcher;
    let (form, path, exe) = match (
        read_form(labels, config),
        configured(labels, "actingd_config", &launcher.actingd_config),
        configured(labels, "actingd_exe", &launcher.actingd_exe),
    ) {
        (Ok(form), Ok(path), Ok(exe)) => (form, path.to_path_buf(), exe.to_path_buf()),
        (Err(text), _, _) | (_, Err(text), _) | (_, _, Err(text)) => {
            set_outcome(config, true, format!("{text}{}", labels.config_unchanged));
            return;
        }
    };
    config.set_saving(true);
    set_outcome(config, false, labels.checking);
    let weak = config.as_weak();
    std::thread::spawn(move || {
        let (failed, text) = match commit(labels, &path, &exe, &id, &form) {
            Err(text) => (true, format!("{text}{}", labels.config_unchanged)),
            Ok(()) => (false, fill(labels.saved, &[&path.display().to_string()])),
        };
        let _ = weak.upgrade_in_event_loop(move |config| {
            config.set_saving(false);
            set_outcome(&config, failed, text);
        });
    });
}

fn read_form(labels: &Labels, config: &ConfigWindow) -> Result<Form, String> {
    let required = |key: &str, value: SharedString| match value.trim() {
        "" => Err(fill(labels.value_missing, &[key])),
        text => Ok(Some(Value::from(text))),
    };
    let number = |key: &str, value: SharedString, largest: u32| match value.trim().parse::<u32>() {
        Ok(number) if number <= largest => Ok(Some(Value::from(number))),
        _ => Err(fill(labels.value_not_number, &[key, &largest.to_string(), value.as_str()])),
    };
    let optional =
        |value: SharedString| Some(value.trim()).filter(|text| !text.is_empty()).map(Value::from);
    let kind = config.get_form_kind().clamp(0, 2) as usize;
    let u16_max = u32::from(u16::MAX);
    let mut fields = vec![("alias", required("alias", config.get_form_alias())?)];
    match kind {
        0 => fields
            .push(("instance_index", number("instance_index", config.get_form_index(), u16_max)?)),
        1 => fields.push(("instance_name", required("instance_name", config.get_form_name())?)),
        _ => fields.extend([
            ("host", required("host", config.get_form_host())?),
            ("port", number("port", config.get_form_port(), u16_max)?),
        ]),
    }
    // Only a MuMu binding has discovery to report adb; an explicit address
    // has nothing else to find it by.
    let adb = config.get_form_adb();
    let adb = if kind == 2 { required("adb_path", adb)? } else { optional(adb) };
    let nemu = config.get_form_nemu();
    let nemu =
        if nemu.trim().is_empty() { None } else { number("nemu_app_index", nemu, u32::MAX)? };
    fields.extend([
        ("adb_path", adb),
        ("nemu_app_index", nemu),
        ("application_id", optional(config.get_form_application())),
        ("capture_backend", optional(config.get_form_capture())),
        ("touch_backend", optional(config.get_form_touch())),
    ]);
    Ok(Form { kind, fields })
}

/// Applies the form to the file as it is now and puts the candidate in place
/// only when check-config accepts it.
fn commit(labels: &Labels, path: &Path, exe: &Path, id: &str, form: &Form) -> Result<(), String> {
    let (mut document, mut entries) = load(labels, path)?;
    // Found only when this form's own earlier save put the entry there.
    let index = match entries.iter().position(|entry| id_of(entry) == Some(id)) {
        Some(index) => index,
        None => {
            entries.push(json!({ "instance_id": id }));
            entries.len() - 1
        }
    };
    apply(&mut entries[index], form);
    document["instances"] = Value::Array(entries);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".candidate-{}", std::process::id()));
    let candidate = path.with_file_name(name);
    let shown = candidate.display().to_string();
    std::fs::write(&candidate, format!("{document:#}\n"))
        .map_err(|error| fill(labels.candidate_failed, &[&shown, &error.to_string()]))
        .and_then(|()| check(labels, exe, &candidate))
        .and_then(|()| {
            std::fs::rename(&candidate, path).map_err(|error| {
                fill(labels.replace_failed, &[&path.display().to_string(), &error.to_string()])
            })
        })
        .map_err(|mut text| {
            match std::fs::remove_file(&candidate) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    text.push_str(&fill(labels.candidate_left, &[&shown, &error.to_string()]))
                }
                _ => {}
            }
            text
        })
}

/// Writes the form's keys into one entry and leaves every other key as it
/// was. A binding kind other than the file's drops the other kinds' keys, so at
/// most one binding is ever written.
fn apply(entry: &mut Value, form: &Form) {
    let changed = kind_of(entry) != Some(form.kind);
    if let Value::Object(entry) = entry {
        entry.retain(|name, _| {
            let other_kind = KINDS
                .iter()
                .enumerate()
                .any(|(kind, keys)| kind != form.kind && keys.contains(&name.as_str()));
            !(changed && other_kind)
                && !form.fields.iter().any(|(key, value)| *key == name.as_str() && value.is_none())
        });
        for (key, value) in &form.fields {
            if let Some(value) = value {
                entry.insert((*key).to_string(), value.clone());
            }
        }
    }
}

/// `<actingd_exe> check-config --config <candidate>`, bounded. Stdout is read
/// whole on its own thread, so a long report can never fill the pipe and stall
/// the child until the timeout.
fn check(labels: &Labels, exe: &Path, candidate: &Path) -> Result<(), String> {
    let mut command = Command::new(exe);
    command.arg("check-config").arg("--config").arg(candidate);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| {
        fill(labels.spawn_failed, &[&exe.display().to_string(), &error.to_string()])
    })?;
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut text = String::new();
            stdout.read_to_string(&mut text).map(|_| text)
        })
    });
    let deadline = Instant::now() + CHECK_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            waited => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match waited {
                    Err(error) => fill(labels.child_status_failed, &[&error.to_string()]),
                    _ => fill(labels.check_timeout, &[&CHECK_TIMEOUT.as_secs().to_string()]),
                });
            }
        }
    };
    let exit = status.code().map_or_else(|| labels.none.to_string(), |code| code.to_string());
    // stdout is piped above, so a missing reader is only ever a panicked one.
    let text = match reader.map(std::thread::JoinHandle::join) {
        Some(Ok(Ok(text))) => text,
        Some(Ok(Err(error))) => {
            return Err(fill(labels.check_read_failed, &[&exit, &error.to_string()]))
        }
        _ => return Err(fill(labels.check_reader_lost, &[&exit])),
    };
    let report: Value = serde_json::from_str(text.trim()).unwrap_or_default();
    let unparsed = || fill(labels.check_unparsed, &[&exit, text.trim()]);
    let error = &report["error"];
    match (report["status"].as_str(), error["code"].as_str(), error["stage"].as_str()) {
        _ if report["schema_version"] != CHECK_SCHEMA => Err(unparsed()),
        (Some("ok"), _, _) if status.success() => Ok(()),
        (Some("ok"), _, _) => Err(fill(labels.check_ok_nonzero, &[&exit])),
        (Some("failed"), Some(code), Some(stage)) => Err(fill(labels.check_failed, &[code, stage])),
        _ => Err(unparsed()),
    }
}

/// The file as JSON, and its `instances` taken out. The key stays where it
/// was, holding null.
fn load(labels: &Labels, path: &Path) -> Result<(Value, Vec<Value>), String> {
    let shown = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => fill(labels.config_missing, &[&shown]),
        _ => fill(labels.config_unreadable, &[&shown, &error.to_string()]),
    })?;
    let mut document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| fill(labels.config_not_json, &[&shown, &error.to_string()]))?;
    match document.get_mut("instances").map(Value::take) {
        Some(Value::Array(entries)) => Ok((document, entries)),
        _ => Err(fill(labels.config_no_instances, &[&shown])),
    }
}

/// A field as the file states it: a string as is, anything else as JSON.
fn text(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).map(|value| value.as_str().map_or_else(|| value.to_string(), str::to_owned))
}

fn id_of(entry: &Value) -> Option<&str> {
    entry.get("instance_id").and_then(Value::as_str)
}

fn kind_of(entry: &Value) -> Option<usize> {
    KINDS.iter().position(|keys| keys.iter().any(|key| entry.get(key).is_some()))
}
