// SPDX-License-Identifier: GPL-3.0-only
//! The instance-configuration window: the `instances` of the actingd
//! configuration the launcher starts with, listed, added to and edited.
//!
//! The file is plain JSON here. Only `instances` changes, and in an entry only
//! the keys the form manages; the Runtime's config struct is not mirrored. A
//! save re-reads the file, writes a candidate beside it (relative paths inside
//! resolve against that directory), runs `<actingd_exe> check-config` on it off
//! the event loop, and renames it over the file only on a parsed `ok` with a
//! successful exit. Discovery asks the running Runtime which MuMu instances its
//! provider reports; picking an unbound one starts an entry bound by its index.

use std::cell::RefCell;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use acui_rows::{code, InstanceId};
use acui_source::{discover_instances, probe_runtime, DiscoveredInstance};
use serde_json::{json, Value};
use slint::{ComponentHandle, SharedString};

use crate::launcher::{configured, failure_text, status_text};
use crate::strings::{fill, Labels};
use crate::{
    models, shared, App, AppWindow, ConfigStrings, ConfigWindow, InstanceRow, PortMap, Scale,
};

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
    /// The entry the form is for, found again by its instance_id; `None` when
    /// there is no instance_id to save under.
    editing: RefCell<Option<Editing>>,
    /// The entries as last listed, for a click on a row.
    entries: RefCell<Vec<Value>>,
    /// The last discovery's instances, in the box's order after its
    /// placeholder; the discovery thread writes them before it posts.
    discovered: Arc<Mutex<Vec<DiscoveredInstance>>>,
}

#[derive(Clone)]
struct Editing {
    instance_id: String,
    existing: bool,
}

/// The managed keys in the schema's spelling; `None` removes an optional key.
struct Form {
    kind: usize,
    fields: Vec<(&'static str, Option<Value>)>,
}

pub fn install(window: &AppWindow, config: &ConfigWindow, app: &Rc<App>) {
    fill_strings(window.global::<ConfigStrings>(), app.labels);
    fill_strings(config.global::<ConfigStrings>(), app.labels);
    let editor = Rc::new(Editor {
        app: Rc::clone(app),
        editing: RefCell::new(None),
        entries: RefCell::new(Vec::new()),
        discovered: Arc::new(Mutex::new(Vec::new())),
    });
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
            load_list(&editor, &config);
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
    config.on_reload(on(load_list));
    config.on_discover(on(discover));
    {
        let (weak, editor) = (config.as_weak(), Rc::clone(&editor));
        config.on_discovered_picked(move |index| {
            if let Some(config) = weak.upgrade() {
                pick(&editor, &config, index);
            }
        });
    }
    let weak = config.as_weak();
    config.on_row_clicked(move |index| {
        if let Some(config) = weak.upgrade() {
            edit(&editor, &config, index);
        }
    });
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
    global.set_discover(labels.discover.into());
    global.set_discover_use(labels.discover_use.into());
}

/// Reads the file again and lists it. A reason it cannot be listed takes the
/// count's place, never an empty list that reads as zero instances.
fn load_list(editor: &Editor, config: &ConfigWindow) {
    let labels = editor.app.labels;
    let listed = configured(labels, "actingd_config", &editor.app.launcher.actingd_config)
        .and_then(|path| Ok((path, load(labels, path)?.1)));
    config.set_list_failed(listed.is_err());
    let (note, entries) = match listed {
        Ok((path, entries)) => {
            let count = entries.len().to_string();
            (fill(labels.config_listed, &[&path.display().to_string(), &count]), entries)
        }
        Err(text) => (text, Vec::new()),
    };
    config.set_list_note(note.into());
    let rows = entries.iter().map(|entry| {
        let value = |key| text(entry, key).unwrap_or_else(|| labels.none.to_string());
        let parts = [
            value("instance_id"),
            value("application_id"),
            value("capture_backend"),
            value("touch_backend"),
        ];
        InstanceRow {
            alias: value("alias").into(),
            binding: binding_text(labels, entry).into(),
            detail: fill(labels.instance_detail, &parts.each_ref().map(String::as_str)).into(),
            ledger: id_of(entry)
                .map_or_else(
                    || labels.row_no_id.to_string(),
                    |id| ledger_text(labels, &editor.app.port_map, id),
                )
                .into(),
        }
    });
    config.set_rows(models(rows.collect()));
    // A new entry a save has put in the file is an existing one from now on.
    let mut editing = editor.editing.borrow_mut();
    let position = editing.as_ref().and_then(|editing| {
        entries.iter().position(|entry| id_of(entry) == Some(&editing.instance_id))
    });
    if let (Some(editing), Some(_)) = (editing.as_mut(), position) {
        editing.existing = true;
    }
    config.set_selected_index(position.map_or(-1, |index| index as i32));
    *editor.entries.borrow_mut() = entries;
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
    config.set_selected_index(-1);
    show_form(config, id.as_deref().ok(), &Value::Null);
    *editor.editing.borrow_mut() =
        id.ok().map(|instance_id| Editing { instance_id, existing: false });
}

/// An entry of the list loaded into the form; one `uneditable` refuses is
/// shown with the reason but cannot be saved.
fn edit(editor: &Editor, config: &ConfigWindow, index: i32) {
    let entries = editor.entries.borrow();
    let Some(entry) = entries.get(index.max(0) as usize) else {
        return;
    };
    let failure = uneditable(editor.app.labels, entry);
    let id = id_of(entry).filter(|_| failure.is_none());
    set_outcome(config, failure.is_some(), failure.unwrap_or_default());
    config.set_selected_index(index);
    show_form(config, id, entry);
    *editor.editing.borrow_mut() =
        id.map(|id| Editing { instance_id: id.to_string(), existing: true });
}

/// Why the form cannot save an entry: no string instance_id, or bound in a way
/// no binding kind of the form represents (`fixture_backend`, `serial`, or no
/// binding key at all).
fn uneditable(labels: &Labels, entry: &Value) -> Option<&'static str> {
    let set = |key: &str| field(entry, key).is_some();
    match id_of(entry) {
        None => Some(labels.entry_no_id),
        Some(_) if set("fixture_backend") => Some(labels.entry_fixture),
        Some(_) if set("serial") => Some(labels.entry_serial),
        Some(_) if kind_of(entry).is_none() => Some(labels.entry_no_binding),
        Some(_) => None,
    }
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

/// Asks the running Runtime for its provider's instance discovery, off the
/// event loop, and fills the discovery box with what it reported. The Runtime
/// records the query itself; nothing is bound and no device is touched.
fn discover(editor: &Editor, config: &ConfigWindow) {
    let labels = editor.app.labels;
    config.set_discovering(true);
    config.set_discover_failed(false);
    config.set_discover_note(labels.discovering.into());
    let root = editor.app.launcher.state_root.clone();
    let (weak, store) = (config.as_weak(), Arc::clone(&editor.discovered));
    let worker = move || {
        let (failed, note, items) = match discover_instances(&root) {
            Ok(discovery) => {
                let items: Vec<String> =
                    discovery.instances.iter().map(|found| found_text(labels, found)).collect();
                let count = discovery.instances.len().to_string();
                let sequence = discovery.sequence.to_string();
                let parts = [count.as_str(), &discovery.provider_version, &sequence];
                *store.lock().unwrap_or_else(PoisonError::into_inner) = discovery.instances;
                (false, fill(labels.discovered, &parts), items)
            }
            Err(failure) => {
                store.lock().unwrap_or_else(PoisonError::into_inner).clear();
                let (refused, failed) = (labels.discover_refused, labels.discover_failed);
                (true, failure_text(labels, refused, failed, &failure), Vec::new())
            }
        };
        let _ = weak.upgrade_in_event_loop(move |config| {
            let mut model = vec![SharedString::from(labels.discover_pick)];
            model.extend(items.into_iter().map(SharedString::from));
            config.set_discovered(models(model));
            config.set_discovered_index(0);
            config.set_discover_failed(failed);
            config.set_discover_note(note.into());
            config.set_discovering(false);
        });
    };
    if let Err(error) = std::thread::Builder::new().name("acui-discover".into()).spawn(worker) {
        // No stale result stays pickable under the failure.
        editor.discovered.lock().unwrap_or_else(PoisonError::into_inner).clear();
        config.set_discovered(models(vec![SharedString::from(labels.discover_pick)]));
        config.set_discovered_index(0);
        config.set_discovering(false);
        config.set_discover_failed(true);
        config.set_discover_note(fill(labels.discover_spawn_failed, &[&error.to_string()]).into());
    }
}

/// One discovered instance as the box lists it; the name, up to 256 bytes,
/// comes last, so a long one cannot push the rest out of a narrow box.
fn found_text(labels: &Labels, found: &DiscoveredInstance) -> String {
    let host = found.adb_host.as_deref().unwrap_or(labels.none);
    let port = found.adb_port.map_or_else(|| labels.none.to_string(), |port| port.to_string());
    let running = if found.running { labels.instance_running } else { labels.instance_stopped };
    let mut parts =
        vec![fill(labels.discovered_index, &[&found.index.to_string()]), running.to_string()];
    if let Some(alias) = &found.bound_alias {
        parts.push(fill(labels.bound_to, &[alias]));
    }
    parts.push(format!("{host}:{port}"));
    if let Some(version) = &found.android_version {
        parts.push(fill(labels.android_version, &[version]));
    }
    parts.push(found.name.clone());
    parts.join(" · ")
}

/// Use Selected in the discovery box: an instance neither the file nor the
/// running Runtime binds starts a new entry bound by its MuMu index, the rest of
/// the form left to fill. One the file already binds by that index is pointed
/// at in the list (the running Runtime may not have loaded it yet), and one the
/// Runtime binds is named; neither changes the form.
fn pick(editor: &Editor, config: &ConfigWindow, index: i32) {
    let labels = editor.app.labels;
    let found = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| {
            editor.discovered.lock().unwrap_or_else(PoisonError::into_inner).get(index).cloned()
        });
    let Some(found) = found else {
        return;
    };
    config.set_discovered_index(0);
    let number = found.index.to_string();
    let listed = editor.entries.borrow().iter().find_map(|entry| {
        (field(entry, "instance_index").and_then(Value::as_u64) == Some(u64::from(found.index)))
            .then(|| text(entry, "alias").unwrap_or_else(|| labels.none.to_string()))
    });
    if let Some(alias) = listed {
        set_outcome(config, false, fill(labels.discover_listed, &[&number, &alias]));
        return;
    }
    if let Some(alias) = &found.bound_alias {
        set_outcome(config, false, fill(labels.discover_bound, &[&number, alias]));
        return;
    }
    begin_new(editor, config);
    // Without an instance_id there is nothing to save under; begin_new said why.
    if editor.editing.borrow().is_none() {
        return;
    }
    config.set_form_kind(0);
    config.set_form_index(number.clone().into());
    set_outcome(config, false, fill(labels.discover_applied, &[&number, &found.name]));
}

fn set_outcome(config: &ConfigWindow, failed: bool, text: impl Into<SharedString>) {
    config.set_outcome_failed(failed);
    config.set_outcome(text.into());
}

fn save(editor: &Editor, config: &ConfigWindow) {
    let labels = editor.app.labels;
    let Some(editing) = editor.editing.borrow().clone() else {
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
    let (root, weak) = (launcher.state_root.clone(), config.as_weak());
    std::thread::spawn(move || {
        let (failed, text) = match commit(labels, &path, &exe, &editing, &form) {
            Err(text) => (true, format!("{text}{}", labels.config_unchanged)),
            // One probe says whether a Runtime runs now, to point at the
            // launcher's own buttons; nothing here restarts one.
            Ok(()) => {
                let probe = probe_runtime(&root);
                let next = if probe.is_ok() { labels.saved_running } else { labels.saved_stopped };
                let saved = fill(labels.saved, &[&path.display().to_string()]);
                (false, format!("{saved} · {}", fill(next, &[&status_text(labels, &probe)])))
            }
        };
        let _ = weak.upgrade_in_event_loop(move |config| {
            config.invoke_reload();
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

/// Applies the form to the file as it is now, not as it was listed, and puts
/// the candidate in place only when check-config accepts it.
fn commit(
    labels: &Labels,
    path: &Path,
    exe: &Path,
    editing: &Editing,
    form: &Form,
) -> Result<(), String> {
    let (mut document, mut entries) = load(labels, path)?;
    let id = editing.instance_id.as_str();
    let index = match entries.iter().position(|entry| id_of(entry) == Some(id)) {
        // Another program may have changed the entry since it was loaded.
        Some(index) => match uneditable(labels, &entries[index]) {
            Some(reason) => return Err(reason.to_string()),
            None => index,
        },
        None if editing.existing => return Err(fill(labels.entry_gone, &[id])),
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
        std::thread::Builder::new().spawn(move || {
            let mut text = String::new();
            stdout.read_to_string(&mut text).map(|_| text)
        })
    });
    let reader = reader.transpose().map_err(|error| {
        fill(labels.reader_failed, &[&error.to_string()]) + &stop(labels, &mut child)
    })?;
    let deadline = Instant::now() + CHECK_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            waited => {
                let failure = match waited {
                    Err(error) => fill(labels.child_status_failed, &[&error.to_string()]),
                    _ => fill(labels.check_timeout, &[&CHECK_TIMEOUT.as_secs().to_string()]),
                };
                return Err(failure + &stop(labels, &mut child));
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

/// Kills and reaps the child. One the kill fails on is not waited on: it may never exit.
fn stop(labels: &Labels, child: &mut Child) -> String {
    match child.kill().and_then(|()| child.wait()) {
        Ok(_) => labels.check_stopped.to_string(),
        Err(error) => fill(labels.check_unstoppable, &[&error.to_string()]),
    }
}

/// The file as JSON, and its `instances` taken out — each an object, the one
/// shape the form edits in place. The key stays where it was, holding null.
fn load(labels: &Labels, path: &Path) -> Result<(Value, Vec<Value>), String> {
    let shown = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => fill(labels.config_missing, &[&shown]),
        _ => fill(labels.config_unreadable, &[&shown, &error.to_string()]),
    })?;
    let mut document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| fill(labels.config_not_json, &[&shown, &error.to_string()]))?;
    let Some(Value::Array(entries)) = document.get_mut("instances").map(Value::take) else {
        return Err(fill(labels.config_no_instances, &[&shown]));
    };
    match entries.iter().position(|entry| !entry.is_object()) {
        Some(index) => Err(fill(labels.config_bad_entry, &[&shown, &(index + 1).to_string()])),
        None => Ok((document, entries)),
    }
}

/// A key the entry sets; JSON null reads as unset, as the Runtime reads it.
fn field<'a>(entry: &'a Value, key: &str) -> Option<&'a Value> {
    entry.get(key).filter(|value| !value.is_null())
}

/// A field as the file states it: a string as is, anything else as JSON.
fn text(entry: &Value, key: &str) -> Option<String> {
    field(entry, key).map(|value| value.as_str().map_or_else(|| value.to_string(), str::to_owned))
}

fn id_of(entry: &Value) -> Option<&str> {
    entry.get("instance_id").and_then(Value::as_str)
}

fn kind_of(entry: &Value) -> Option<usize> {
    KINDS.iter().position(|keys| keys.iter().any(|key| field(entry, key).is_some()))
}

fn binding_text(labels: &Labels, entry: &Value) -> String {
    let value = |key| text(entry, key).unwrap_or_else(|| labels.none.to_string());
    match (kind_of(entry), text(entry, "serial")) {
        // The Runtime takes a fixture before any binding key.
        _ if field(entry, "fixture_backend").is_some() => labels.binding_fixture.to_string(),
        (Some(0), _) => fill(labels.binding_index, &[&value("instance_index")]),
        (Some(1), _) => fill(labels.binding_name, &[&value("instance_name")]),
        // An explicit target with a serial is reached by that serial alone.
        (_, Some(serial)) => fill(labels.binding_serial, &[&serial]),
        (Some(_), None) => fill(labels.binding_adb, &[&value("host"), &value("port")]),
        // The Runtime's default address for an entry with neither is its
        // business; this only says what the file holds.
        (None, None) => labels.binding_none.to_string(),
    }
}

/// What the session's port map says about one instance_id: the port its latest
/// binding names, a binding outside the map — serial-configured or portless,
/// which the map does not tell apart — or none; online, unread or with the
/// ledger not opened, it says so.
fn ledger_text(labels: &Labels, port_map: &PortMap, id: &str) -> String {
    let bindings = match port_map {
        PortMap::Read(bindings) => bindings,
        PortMap::Online => return labels.ledger_online.to_string(),
        PortMap::Failed(error) => return fill(labels.ledger_failed, &[&error.to_string()]),
        PortMap::Unopened => return labels.ledger_unopened.to_string(),
    };
    let named = |other: &InstanceId| code(other) == id;
    let members = bindings.ports.iter().flat_map(|entry| &entry.members);
    match bindings.port_of.iter().find(|(other, _)| named(other)) {
        Some((_, port)) => fill(labels.ledger_port, &[&port.to_string()]),
        None if bindings.unported.iter().chain(members).any(named) => {
            labels.ledger_outside_map.to_string()
        }
        None => labels.ledger_none.to_string(),
    }
}
