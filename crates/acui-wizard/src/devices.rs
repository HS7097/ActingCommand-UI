// SPDX-License-Identifier: GPL-3.0-only
//! The phones the root's own Lab sees, and the binding of a phone's instance to one of them
//! (Runtime `contracts/device-binding.md`, "Choosing a device"; `contracts/lab-operations.json`,
//! the rows `devices` and `devices bind`), each through `<root>\tools\actinglab.exe`, the Lab of
//! the install set, which needs no Runtime running and whose adb is the root's own
//! `tools\platform-tools\adb.exe`, the very one the Runtime will use:
//!
//! - **What the Lab can do** ([`ask`]): `actinglab --json help devices` and `actinglab --json help
//!   devices bind` answer the rows of the operation table its build embeds (`command.params`,
//!   each parameter by its `name`) and read nothing else. A Lab whose `devices` takes
//!   [`LIST_PARAMS`] and whose `devices bind` takes [`BIND_PARAMS`] lists before an installation
//!   and binds by key against a candidate configuration; no version is read.
//! - **The list** ([`list`]): `actinglab --json devices --root <root> --key-salt-env
//!   <variable>`, the run's key salt in that variable of the Lab's environment alone
//!   (`key_salt`). On a root that holds only its tools there is no state root, so the answer says
//!   that no binding was read (`bindings_unread`): expected, no failure. No row names a serial;
//!   a row's `device_key` is the keyed digest of the phone's serial under the run's salt, the same
//!   at every listing of the run while a handle changes with each replug ([`row_use`]: which rows
//!   a phone's instance may take).
//! - **The bind** ([`bind`]): `actinglab --json devices bind <alias> --device-key <key>
//!   --key-salt-env <variable> --config <candidate> --bound-via ui --root <root>`, between the
//!   configuration's step and its check: the alias is looked up in the candidate, the phone found
//!   again by its key under the same salt (a row adb reports in another state than `device`, or
//!   not on USB, is refused before anything is written), and its serial written for the alias
//!   into `<state root>\private\device-bindings.json`, the one file that holds it. The bind
//!   creates the state root and its `private` folder when they are missing and writes nothing
//!   else of the state root but that file, written whole as `device-bindings.json.tmp` beside it
//!   and renamed (Runtime `crates/lab-ops/src/devices.rs` `write`: a rename that fails leaves the
//!   `.tmp`, serials and all); check-config, adopt and the first start each accept a state root
//!   that holds only that file (the same contract, "Against a candidate configuration")
//!   ([`judge_bind`]).

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::child::{outcome_of, outcome_text, NoOutput, OnTimeout};
use crate::key_salt::KeySalt;
use crate::runtime::{self, Answer};
use crate::text::Said;

/// The variable of the Lab's environment that holds the run's key salt.
pub const KEY_SALT_VARIABLE: &str = "ACSETUP_DEVICE_KEY_SALT";
/// What `devices` must take to list before an installation with the run's salt.
pub const LIST_PARAMS: [&str; 2] = ["root", "key_salt_env"];
/// What `devices bind` must take to bind by key against the candidate, as the installer.
pub const BIND_PARAMS: [&str; 5] = ["config", "device_key", "root", "key_salt_env", "bound_via"];
/// The channel the binding records: the installer's.
pub const BOUND_VIA: &str = "ui";
/// A bind's success.
pub const BINDING_CHANGED: &str = "device_binding_changed";
/// The state root's folder the bind creates, and its file.
pub const PRIVATE_DIR: &str = "private";
pub const BINDINGS_FILE: &str = "device-bindings.json";
/// A row's transport and state a phone's instance takes (a handset declares `usb`, the one
/// transport of today).
pub const USB: &str = "usb";
pub const DEVICE: &str = "device";
/// `help` reads only the table its build embeds.
pub const HELP_TIMEOUT: Duration = Duration::from_secs(60);
/// A listing runs `adb devices -l` once, which may start the adb server first.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(3 * 60);
/// A bind lists once more and writes: never ended.
pub const BIND_TIMEOUT: Duration = Duration::from_secs(3 * 60);

fn words(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// The names of the parameters a `help <row>` answer's `command.params` lists for the row `row`,
/// or `None` when it answers another entry (`help` names the longest beginning of the words that
/// names one: a Lab without `devices bind` answers `devices` for it) or lists none.
pub fn param_names(line: &Value, row: &str) -> Option<Vec<String>> {
    let command = &line["command"];
    if command["name"].as_str() != Some(row) {
        return None;
    }
    let params = command["params"].as_array()?;
    Some(params.iter().filter_map(|param| param["name"].as_str().map(str::to_owned)).collect())
}

/// What the Lab lacks: each of [`LIST_PARAMS`] not among `list` (the names `devices` takes),
/// then each of [`BIND_PARAMS`] not among `bind` (`devices bind`'s), each named with its row;
/// a row that answered no parameters (`None`) lacks all of its own.
pub fn missing_params(list: Option<&[String]>, bind: Option<&[String]>) -> Vec<String> {
    fn lacking(row: &str, wanted: &[&str], given: Option<&[String]>) -> Vec<String> {
        wanted
            .iter()
            .filter(|name| !given.is_some_and(|given| given.iter().any(|have| have.as_str() == **name)))
            .map(|name| format!("{row} {name}"))
            .collect()
    }
    let mut missing = lacking("devices", &LIST_PARAMS, list);
    missing.extend(lacking("devices bind", &BIND_PARAMS, bind));
    missing
}

/// What [`ask`] found: what the Lab lacks ([`missing_params`]; empty: it lists and binds as the
/// wizard needs), and both answers, for the log.
#[derive(Debug, Clone)]
pub struct Asked {
    pub missing: Vec<String>,
    pub answers: Vec<Answer>,
}

/// The root's Lab asked for its rows `devices` and `devices bind` (see the module's description).
/// An answer that is no success, or names no parameters, counts as a row that takes none.
pub fn ask(root: &Path) -> Result<Asked, Said> {
    let list = runtime::lab(root, &words(&["help", "devices"]), HELP_TIMEOUT, OnTimeout::End)?;
    let bind = runtime::lab(root, &words(&["help", "devices", "bind"]), HELP_TIMEOUT, OnTimeout::End)?;
    let names = |answer: &Answer, row: &str| {
        if answer.ok() {
            answer.line.as_ref().and_then(|line| param_names(line, row))
        } else {
            None
        }
    };
    let missing = missing_params(names(&list, "devices").as_deref(), names(&bind, "devices bind").as_deref());
    Ok(Asked { missing, answers: vec![list, bind] })
}

/// One row of the list, as the Lab answers it (no row names a serial).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    /// The adb server's session number for the device, `<transport>:<id>`; it changes with a
    /// replug or a restart of the server.
    pub handle: Option<String>,
    /// The keyed digest of the serial under the run's salt.
    pub device_key: Option<String>,
    /// `usb` or `tcp`.
    pub transport: String,
    /// As adb reports it: `device`, `unauthorized`, `offline`, `no` (no permissions), …
    pub state: String,
    pub model: Option<String>,
    /// The alias the state root's private file binds it to, when one was read.
    pub bound_alias: Option<String>,
}

/// Whether a phone's instance may take a row, and what to do about one it may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowUse {
    /// On USB, in the state `device`, with its key: a phone's instance may take it.
    Usable,
    /// The phone has not allowed this computer to debug it yet.
    Unauthorized,
    /// No permissions (`no`) or `offline`: the computer lacks the phone's USB driver, or the
    /// phone is not connected well.
    NoDriver,
    /// Not on USB: an emulator's loopback row, or a wireless connection.
    NotHandset,
    /// On USB and in the state `device`, but without a key: it cannot be found again by one.
    NoKey,
    /// Another state, shown as adb reports it.
    Other,
}

/// What a phone's instance may do with `row` (see [`RowUse`]): only a USB row in the state
/// `device` that carries its key is taken; the Lab's bind refuses any other state and any other
/// transport than the instance's `usb` (`device_row_unbindable`), and the wizard binds by key.
pub fn row_use(row: &DeviceRow) -> RowUse {
    if row.transport != USB {
        return RowUse::NotHandset;
    }
    match row.state.as_str() {
        DEVICE if row.device_key.as_deref().is_some_and(|key| !key.is_empty()) => RowUse::Usable,
        DEVICE => RowUse::NoKey,
        "unauthorized" => RowUse::Unauthorized,
        "no" | "offline" => RowUse::NoDriver,
        _ => RowUse::Other,
    }
}

/// A listing as the wizard reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub rows: Vec<DeviceRow>,
    /// `bindings_unread.why`: no binding was read (on a root with only its tools, the state root
    /// is unknown: expected).
    pub bindings_unread: Option<String>,
    /// `device_keys_unmade.why`: the rows carry no key, and why.
    pub keys_unmade: Option<String>,
}

fn text(value: &Value, name: &str) -> Option<String> {
    value[name].as_str().filter(|text| !text.is_empty()).map(str::to_owned)
}

/// `devices`' answer read: exit 0 and its list `devices`, each row's members (one without a
/// transport or a state has them empty, and is no phone's: [`row_use`]); else why not.
pub fn read_listing(code: Option<i32>, line: Option<&Value>) -> Result<Listing, Said> {
    let exit = format!("exit {}", code.map_or_else(|| "(none)".to_owned(), |code| code.to_string()));
    if code != Some(0) {
        let said = Said::new("设备桥没能列出手机", "the device bridge could not list the phones");
        return Err(match line {
            Some(line) => said.with(format!("{exit}; {}", outcome_text(&line["outcome"]))),
            None => said.with(exit),
        });
    }
    let Some(line) = line else {
        return Err(Said::new("找手机的答复里没有结果行", "the device list's answer has no result line").with(exit));
    };
    let Some(rows) = line["devices"].as_array() else {
        return Err(Said::new("找手机的答复里没有设备列表（devices）", "the device list's answer has no list (devices)"));
    };
    Ok(Listing {
        rows: rows
            .iter()
            .map(|row| DeviceRow {
                handle: text(row, "handle"),
                device_key: text(row, "device_key"),
                transport: row["transport"].as_str().unwrap_or_default().to_owned(),
                state: row["state"].as_str().unwrap_or_default().to_owned(),
                model: text(row, "model"),
                bound_alias: text(row, "bound_alias"),
            })
            .collect(),
        bindings_unread: text(&line["bindings_unread"], "why"),
        keys_unmade: text(&line["device_keys_unmade"], "why"),
    })
}

/// `actinglab --json devices --root <root> --key-salt-env <variable>`, the run's salt in that
/// variable alone (see the module's description); read only.
pub fn list(root: &Path, salt: &KeySalt) -> Result<Answer, Said> {
    let mut list = words(&["devices", "--root"]);
    list.push(root.as_os_str().to_os_string());
    list.extend(words(&["--key-salt-env", KEY_SALT_VARIABLE]));
    runtime::lab_salted(root, &list, salt, KEY_SALT_VARIABLE, LIST_TIMEOUT, OnTimeout::End)
        .map_err(|failed| failed.reason)
}

/// `actinglab --json devices bind <alias> --device-key <key> --key-salt-env <variable> --config
/// <candidate> --bound-via ui --root <root>`, the run's salt in that variable alone (see the
/// module's description). It writes: a bind past its time is left running, and says so
/// ([`NoOutput::left_running`]: it may still write in the state root).
pub fn bind(root: &Path, alias: &str, device_key: &str, candidate: &Path, salt: &KeySalt) -> Result<Answer, NoOutput> {
    let mut list = words(&["devices", "bind", alias, "--device-key", device_key, "--key-salt-env", KEY_SALT_VARIABLE, "--config"]);
    list.push(candidate.as_os_str().to_os_string());
    list.extend(words(&["--bound-via", BOUND_VIA, "--root"]));
    list.push(root.as_os_str().to_os_string());
    runtime::lab_salted(root, &list, salt, KEY_SALT_VARIABLE, BIND_TIMEOUT, OnTimeout::Leave)
}

/// A refusal of the bind in a person's words, by its code (and, for a row that cannot be
/// bound, `device_row_problem` and `device_link_state`); the code itself goes after them
/// ([`judge_bind`]).
fn refusal_words(outcome: &Value) -> Said {
    let values = &outcome["values"];
    match outcome["code"].as_str() {
        Some("device_key_unknown") => Said::new(
            "这部手机断开了或换了 USB 口，请插好后重新运行向导",
            "the phone was unplugged or moved to another USB port: plug it in and run the wizard again",
        ),
        Some("device_row_unbindable") => match (values["device_row_problem"].as_str(), values["device_link_state"].as_str()) {
            (Some("state_not_device"), Some("unauthorized")) => Said::new(
                "手机上还没有允许这台电脑做 USB 调试：请在手机上允许，再重新运行向导",
                "the phone has not allowed USB debugging by this computer: allow it on the phone, then run the wizard again",
            ),
            (Some("state_not_device"), Some("offline")) => Said::new(
                "手机离线了：请插好后重新运行向导",
                "the phone is offline: plug it in again, then run the wizard again",
            ),
            (Some("state_not_device"), Some("no_permissions")) => Said::new(
                "电脑缺这台手机的 USB 驱动，或者没连好；驱动要去手机厂商那里装，我们不带",
                "this computer lacks the phone's USB driver, or it is not connected well; the driver comes from the phone's maker and is not included here",
            ),
            (Some("state_not_device"), _) => Said::new(
                "手机现在的连接状态不能绑定（不是「已连接」）",
                "the phone's connection is in a state that cannot be bound (not \"device\")",
            ),
            (Some("transport_mismatch"), _) => Said::new(
                "设备桥找到的这一行不是用 USB 连着的手机（模拟器或无线连接），不能当手机绑定",
                "the row the device bridge found is no phone on USB (an emulator or a wireless connection) and cannot be bound as a phone",
            ),
            _ => Said::new("这部手机现在不能绑定", "the phone cannot be bound now"),
        },
        Some("device_bindings_invalid") => Said::new(
            "绑定文件不接受这次绑定（比如这部手机已经绑给了另一个实例）",
            "the binding file refuses this binding (the phone may be bound to another instance already)",
        ),
        Some("device_list_failed") => Said::new(
            "设备桥的 adb 没能列出设备",
            "the device bridge's adb could not list the devices",
        ),
        _ => Said::new("设备桥拒绝了这次绑定", "the device bridge refused the binding"),
    }
}

/// `devices bind`'s answer judged: exit 0 and `device_binding_changed`; else the refusal in a
/// person's words with its code and values as the Lab answered them (an answer that is no
/// refusal and no success is said as it is).
pub fn judge_bind(code: Option<i32>, line: Option<&Value>) -> Result<(), Said> {
    let exit = format!("exit {}", code.map_or_else(|| "(none)".to_owned(), |code| code.to_string()));
    let Some(line) = line else {
        return Err(Said::new("绑定的答复里没有结果行", "the bind's answer has no result line").with(exit));
    };
    let outcome = outcome_of(line);
    if outcome.code.as_deref() == Some(BINDING_CHANGED) {
        if code == Some(0) {
            return Ok(());
        }
        return Err(Said::new("绑定答复了成功，退出码却不是 0", "the bind answered success, yet its exit code is not 0").with(exit));
    }
    if code == Some(0) {
        return Err(Said::new(
            format!("绑定没有报告成功（{BINDING_CHANGED}）"),
            format!("the bind does not report success ({BINDING_CHANGED})"),
        )
        .with(outcome_text(&line["outcome"])));
    }
    Err(refusal_words(&line["outcome"]).with(format!("{exit}; {}", outcome_text(&line["outcome"]))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A `help <row>` answer for the row `row` with the parameters `names` (Runtime
    /// `apps/actinglab/src/cli_help.rs` `Entry::value`; the other members left out).
    fn help(row: &str, names: &[&str]) -> Value {
        let params: Vec<Value> = names.iter().map(|name| json!({"name": name, "form": "option"})).collect();
        json!({"command": {"name": row, "params": params}, "outcome": {"code": "x", "category": "success"}})
    }

    #[test]
    fn a_lab_lists_and_binds_only_with_every_parameter() {
        // The rows of Runtime 0.12.72 (`contracts/lab-operations.json`).
        let list = param_names(&help("devices", &["config", "root", "key_salt", "key_salt_env"]), "devices");
        let bind_row = help(
            "devices bind",
            &["instance", "handle", "config", "bound_via", "device_key", "key_salt", "key_salt_env", "root"],
        );
        let bind = param_names(&bind_row, "devices bind");
        assert!(missing_params(list.as_deref(), bind.as_deref()).is_empty());
        // One missing on either row is named with its row.
        let older = param_names(&help("devices", &["config", "key_salt", "key_salt_env"]), "devices");
        assert_eq!(missing_params(older.as_deref(), bind.as_deref()), vec!["devices root"]);
        let unbound = help("devices bind", &["instance", "handle", "config", "device_key", "key_salt_env", "root"]);
        assert_eq!(missing_params(list.as_deref(), param_names(&unbound, "devices bind").as_deref()), vec!["devices bind bound_via"]);
        // A row that answered nothing lacks all of its own; a group's answer names no parameters,
        // and neither does the answer for another row (a Lab without `devices bind` answers the
        // row `devices` for those words).
        assert_eq!(missing_params(list.as_deref(), None).len(), BIND_PARAMS.len());
        assert_eq!(missing_params(None, None).len(), LIST_PARAMS.len() + BIND_PARAMS.len());
        assert_eq!(param_names(&json!({"group": "devices", "members": []}), "devices"), None);
        assert_eq!(param_names(&help("devices", &["config", "root", "key_salt_env"]), "devices bind"), None);
    }

    fn row(transport: &str, state: &str, key: Option<&str>) -> DeviceRow {
        DeviceRow {
            handle: Some(format!("{transport}:7")),
            device_key: key.map(str::to_owned),
            transport: transport.to_owned(),
            state: state.to_owned(),
            model: None,
            bound_alias: None,
        }
    }

    #[test]
    fn only_a_usb_phone_in_the_state_device_with_its_key_is_taken() {
        let key = Some("sha256:0000");
        assert_eq!(row_use(&row("usb", "device", key)), RowUse::Usable);
        assert_eq!(row_use(&row("usb", "device", None)), RowUse::NoKey);
        assert_eq!(row_use(&row("usb", "device", Some(""))), RowUse::NoKey);
        assert_eq!(row_use(&row("usb", "unauthorized", key)), RowUse::Unauthorized);
        assert_eq!(row_use(&row("usb", "no", key)), RowUse::NoDriver);
        assert_eq!(row_use(&row("usb", "offline", key)), RowUse::NoDriver);
        assert_eq!(row_use(&row("usb", "recovery", key)), RowUse::Other);
        // An emulator's loopback row is in the state `device` and still no phone.
        assert_eq!(row_use(&row("tcp", "device", key)), RowUse::NotHandset);
        assert_eq!(row_use(&row("tcp", "offline", None)), RowUse::NotHandset);
    }

    #[test]
    fn a_listing_reads_its_rows_and_says_what_it_did_not_read() {
        // As Runtime crates/lab-ops/src/gate_398_db2.rs answers a root with only its tools, with
        // a salt: every row with its key, no serial, the bindings unread.
        let line = json!({
            "devices": [
                {"handle": "usb:7", "device_key": "sha256:aa", "transport": "usb", "state": "device", "model": "Neutral_Phone"},
                {"handle": "tcp:3", "device_key": "sha256:bb", "transport": "tcp", "state": "device", "model": "Neutral_Emulator", "endpoint": "127.0.0.1:5555"},
                {"handle": "usb:8", "device_key": "sha256:cc", "transport": "usb", "state": "unauthorized"}
            ],
            "bindings_unread": {"why": "state_root_unknown", "cause": {"code": "install_state_root_unresolved"}},
            "outcome": {"code": "device_list_reported", "category": "success"}
        });
        let listing = read_listing(Some(0), Some(&line)).expect("a listing");
        assert_eq!(listing.rows.len(), 3);
        assert_eq!(listing.rows[0].model.as_deref(), Some("Neutral_Phone"));
        assert_eq!(listing.rows[2].model, None);
        assert_eq!(
            listing.rows.iter().map(row_use).collect::<Vec<_>>(),
            vec![RowUse::Usable, RowUse::NotHandset, RowUse::Unauthorized]
        );
        assert_eq!(listing.bindings_unread.as_deref(), Some("state_root_unknown"));
        assert_eq!(listing.keys_unmade, None);
        // No phone at all is a listing too.
        let empty = json!({"devices": [], "outcome": {"code": "device_list_reported"}});
        assert!(read_listing(Some(0), Some(&empty)).unwrap().rows.is_empty());
        // A refusal, a line without a list, no line.
        let refused = json!({"outcome": {"code": "device_list_failed", "category": "error"}});
        assert!(read_listing(Some(1), Some(&refused)).unwrap_err().en.contains("device_list_failed"));
        assert!(read_listing(Some(0), Some(&refused)).is_err());
        assert!(read_listing(Some(0), None).is_err());
    }

    #[test]
    fn a_bind_counts_only_as_device_binding_changed_and_a_refusal_keeps_its_code() {
        let bound = json!({"alias": "a", "device_key": "sha256:aa", "handle": "usb:17", "restart_required": true,
            "outcome": {"code": "device_binding_changed", "category": "success"}});
        assert_eq!(judge_bind(Some(0), Some(&bound)), Ok(()));
        assert!(judge_bind(Some(1), Some(&bound)).is_err());
        assert!(judge_bind(Some(0), None).is_err());
        let other = json!({"outcome": {"code": "device_list_reported", "category": "success"}});
        assert!(judge_bind(Some(0), Some(&other)).unwrap_err().en.contains(BINDING_CHANGED));
        // Each refusal in a person's words, its code and values after them.
        let refused = |code: &str, values: Value| json!({"outcome": {"code": code, "category": "error", "values": values}});
        let unknown = judge_bind(Some(1), Some(&refused("device_key_unknown", json!({})))).unwrap_err();
        assert!(unknown.zh.starts_with("这部手机断开了或换了 USB 口") && unknown.zh.contains("device_key_unknown"), "{unknown}");
        let unauthorized = refused(
            "device_row_unbindable",
            json!({"alias": "a", "handle": "usb:8", "device_row_problem": "state_not_device", "device_link_state": "unauthorized"}),
        );
        let said = judge_bind(Some(1), Some(&unauthorized)).unwrap_err();
        assert!(said.en.contains("allowed USB debugging") && said.en.contains("unauthorized"), "{said}");
        let tcp = refused("device_row_unbindable", json!({"alias": "a", "device_row_problem": "transport_mismatch"}));
        assert!(judge_bind(Some(1), Some(&tcp)).unwrap_err().en.contains("transport_mismatch"));
        let later = judge_bind(Some(1), Some(&refused("a_later_code", json!({})))).unwrap_err();
        assert!(later.en.starts_with("the device bridge refused") && later.en.contains("a_later_code"), "{later}");
    }
}
