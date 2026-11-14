// SPDX-License-Identifier: GPL-3.0-only
//! The root's first `actingd.config.json`. The Runtime 0.12.58 has no command that writes a
//! first configuration or a first instance (`actingctl install config` is not built, and the Lab's
//! `instance add` needs a running Runtime and writes no `application_id`), so the wizard writes
//! it and the Runtime's own `actingd check-config` judges it before it is put in place.
//!
//! Every key against Runtime `apps/actingd/src/config.rs` (the file denies unknown members) and
//! `contracts/runtime-config.md`:
//!
//! - `schema_version` `actingcommand.actingd.config.v1`; `state_root` absolute (the Lab, the
//!   installer and the watchdog refuse a relative one); `bind_host` a loopback address;
//!   `bind_port` 0, the system's choice; `secret_fingerprint_salt` 16 to 1024 bytes, here the
//!   hex of 32 bytes of the operating system's random source, never shown or logged;
//! - `mumu_root` absolute, when known; `standard_packs.root` `packages` (relative to the file's
//!   folder; composer mode requires it); `vision` without `root` (the vision root is then
//!   `<root>\vision`): `execution_provider` `cpu`, or, with the graphics card chosen for text
//!   recognition, `directml` with `directml_device.expected_identity` and `gpu_fallback` `cpu`
//!   written out ([`vision_section`]);
//! - `instances[]`: `alias` (1 to 256 bytes, no control character; the wizard holds it to the
//!   schedule's stricter rule, see `alias`), `instance_id`
//!   (`instance_` and 32 lowercase hex digits, minted here), `instance_index` (the MuMu index;
//!   no `instance_name`, `serial` or `port`), `application_id` (required for a discovery binding:
//!   `application_identity_missing`), `touch_backend` and `capture_backend` (required, never
//!   `auto`), `start_emulator` only when true (discovery bindings only); no `adb_path`: the
//!   root's own `tools\platform-tools\adb.exe` is used;
//! - with instances, `policy` in composer mode (no `catalog`): `facts` with one row per alias
//!   (`instance_id` the alias, `server_id`, `game_id`, `host_id` `local`, `available`,
//!   `capability_operation_ids` `["operation.package.run"]`, the entry operation every composed
//!   task names, `preferred_task_ids` `[]`) and `resources.hosts` holding `local`; the alias set
//!   must equal the configured one (`policy_instance_set_mismatch`). Without instances there is
//!   no `policy`: a control-plane configuration.
//! - `lab_module_authoring` and `lab_module_debugging` are left out (only the installer writes
//!   them).

use serde_json::{json, Value};

use crate::digest::hex;
use crate::text::Said;

pub const CONFIG_FILE: &str = "actingd.config.json";
/// The candidate written before `check-config` passes; renamed into place after.
pub const CONFIG_PART: &str = "actingd.config.json.part";
/// The full configuration, with the instances, while the first start runs without them
/// ([`without_instances`]): checked with the other, kept beside it, and put in place by the
/// Runtime's `config replace` once the first-start pause holds and the Runtime is closed. The
/// wizard deletes nothing, so it stays, the same bytes as the configuration then in place.
pub const FULL_CONFIG: &str = "install-wizard.config.json";
pub const CONFIG_SCHEMA: &str = "actingcommand.actingd.config.v1";
pub const STATE_DIR: &str = "state";
pub const PACKAGES_DIR: &str = "packages";
pub const HOST_ID: &str = "local";
/// The entry operation of every task the composer makes (Runtime `crates/policy/src/compose.rs`).
pub const PACKAGE_RUN: &str = "operation.package.run";
/// The touch and capture backends an emulator takes (Runtime `contracts/runtime-config.md`,
/// the backend table; `scrcpy` is a handset's).
pub const TOUCH_BACKENDS: [&str; 4] = ["adb_shell_input", "maatouch", "minitouch", "nemu_ipc"];
pub const CAPTURE_BACKENDS: [&str; 3] = ["adb", "droidcast_raw", "nemu_ipc"];
/// Today's production pair (the same table: "Nemu capture beside any touch backend is today's
/// production"), the wizard's default.
pub const DEFAULT_BACKEND: &str = "nemu_ipc";
/// The execution provider of the CPU, the default, and of the graphics card (`actingd-config`
/// revision 27, inside the range the console and the Runtime declare).
pub const CPU_PROVIDER: &str = "cpu";
pub const DIRECTML_PROVIDER: &str = "directml";
/// What a DirectML section does when the card cannot be used: the work goes on on the CPU, with
/// one warning. The Runtime's default, written out all the same.
pub const GPU_FALLBACK: &str = "cpu";

/// One instance of the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInstance {
    pub alias: String,
    pub instance_id: String,
    pub instance_index: u16,
    pub application_id: String,
    pub game: String,
    pub server: String,
    pub touch_backend: String,
    pub capture_backend: String,
    pub start_emulator: bool,
}

/// Everything the configuration is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInput {
    /// `<root>\state`, absolute.
    pub state_root: String,
    pub salt: String,
    pub mumu_root: Option<String>,
    pub instances: Vec<ConfigInstance>,
    /// The graphics card chosen for text recognition, by its stable identity (`gpu`); `None` for
    /// the CPU.
    pub directml_adapter: Option<String>,
}

/// The `vision` section (Runtime `apps/actingd/src/config.rs` `VisionConfigFile`, which denies
/// unknown members; `contracts/actingd-check-config.md`, "DirectML"): `execution_provider` `cpu`
/// without any DirectML member; or, with a card, `directml`, the card by its stable identity in
/// `directml_device.expected_identity` (no DXGI index, no LUID) and `gpu_fallback` `cpu`.
pub fn vision_section(directml_adapter: Option<&str>) -> Value {
    match directml_adapter {
        None => json!({ "execution_provider": CPU_PROVIDER }),
        Some(identity) => json!({
            "execution_provider": DIRECTML_PROVIDER,
            "directml_device": { "expected_identity": identity },
            "gpu_fallback": GPU_FALLBACK,
        }),
    }
}

/// The configuration document (see the module's description).
pub fn build(input: &ConfigInput) -> Value {
    let entries: Vec<Value> = input.instances.iter().map(instance_entry).collect();
    let mut document = json!({
        "schema_version": CONFIG_SCHEMA,
        "state_root": input.state_root,
        "bind_host": "127.0.0.1",
        "bind_port": 0,
        "secret_fingerprint_salt": input.salt,
        "standard_packs": { "root": PACKAGES_DIR },
        "vision": vision_section(input.directml_adapter.as_deref()),
        "instances": entries,
    });
    if let Some(root) = &input.mumu_root {
        document["mumu_root"] = json!(root);
    }
    if !input.instances.is_empty() {
        let facts: Vec<Value> = input
            .instances
            .iter()
            .map(|instance| {
                json!({
                    "instance_id": instance.alias,
                    "server_id": instance.server,
                    "game_id": instance.game,
                    "host_id": HOST_ID,
                    "available": true,
                    "capability_operation_ids": [PACKAGE_RUN],
                    "preferred_task_ids": [],
                })
            })
            .collect();
        document["policy"] = json!({
            "facts": {
                "ledger_position": 0,
                "fact_snapshot_id": "install",
                "facts": [],
                "outcomes": [],
                "tasks": [],
                "instances": facts,
            },
            "resources": {
                "pools": [],
                "hosts": [{
                    "host_id": HOST_ID,
                    "cpu_available_milli": 1000,
                    "gpu_available_milli": 1000,
                    "io_available_milli": 1000,
                }],
            },
        });
    }
    document
}

fn instance_entry(instance: &ConfigInstance) -> Value {
    let mut entry = json!({
        "alias": instance.alias,
        "instance_id": instance.instance_id,
        "instance_index": instance.instance_index,
        "application_id": instance.application_id,
        "touch_backend": instance.touch_backend,
        "capture_backend": instance.capture_backend,
    });
    if instance.start_emulator {
        entry["start_emulator"] = json!(true);
    }
    entry
}

/// The document as the file holds it: pretty JSON, one line end at the end, no byte-order mark.
pub fn render(document: &Value) -> String {
    let mut text = serde_json::to_string_pretty(document).expect("a JSON value always renders");
    text.push('\n');
    text
}

/// `document` with `mumu_root` set to `root`.
pub fn with_mumu_root(document: &Value, root: &str) -> Value {
    let mut document = document.clone();
    document["mumu_root"] = json!(root);
    document
}

/// `document` without its instances and the policy that names them: the configuration the first
/// start runs with, everything else as it is (the same salt, state root, MuMu folder, packages
/// and vision, the graphics card with it), so that no instance exists before the first-start
/// pause holds.
pub fn without_instances(document: &Value) -> Value {
    let mut document = document.clone();
    document["instances"] = json!([]);
    if let Some(members) = document.as_object_mut() {
        members.remove("policy");
    }
    document
}

/// 32 bytes of the operating system's random source, as 64 hex digits.
pub fn mint_salt() -> Result<String, Said> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| Said::new("取不到系统随机数", "the system random source failed").with(error))?;
    Ok(hex(&bytes))
}

/// A new `instance_<32 lowercase hex>` from 16 random bytes.
pub fn mint_instance_id() -> Result<String, Said> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| Said::new("取不到系统随机数", "the system random source failed").with(error))?;
    Ok(format!("instance_{}", hex(&bytes)))
}

/// Whether `text` is an instance id (Runtime `crates/actingcommand-contract/src/event/ids.rs`).
pub fn is_instance_id(text: &str) -> bool {
    text.strip_prefix("instance_").is_some_and(|digits| {
        digits.len() == 32 && digits.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// What is wrong with a backend pair, or `None`: each one an emulator takes, and Nemu touch only
/// beside Nemu capture (Runtime `contracts/runtime-config.md`, the backend pairing).
pub fn backend_problem(touch: &str, capture: &str) -> Option<Said> {
    if !TOUCH_BACKENDS.contains(&touch) {
        return Some(Said::new("触控后端不可用", "the touch backend is not one an emulator takes").with(touch));
    }
    if !CAPTURE_BACKENDS.contains(&capture) {
        return Some(Said::new("截图后端不可用", "the capture backend is not one an emulator takes").with(capture));
    }
    if touch == "nemu_ipc" && capture != "nemu_ipc" {
        return Some(Said::new(
            "触控 nemu_ipc 必须配截图 nemu_ipc",
            "Nemu touch needs Nemu capture",
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(alias: &str, index: u16, server: &str) -> ConfigInstance {
        ConfigInstance {
            alias: alias.to_owned(),
            instance_id: format!("instance_{}", "0".repeat(31) + &index.to_string()),
            instance_index: index,
            application_id: format!("com.example.{server}"),
            game: "azurlane".to_owned(),
            server: server.to_owned(),
            touch_backend: DEFAULT_BACKEND.to_owned(),
            capture_backend: DEFAULT_BACKEND.to_owned(),
            start_emulator: index == 5,
        }
    }

    fn input(instances: Vec<ConfigInstance>, mumu_root: Option<&str>) -> ConfigInput {
        ConfigInput {
            state_root: "X:\\ActingCommand\\state".to_owned(),
            salt: "ab".repeat(32),
            mumu_root: mumu_root.map(str::to_owned),
            instances,
            directml_adapter: None,
        }
    }

    const CARD: &str = "dxgi-pci:10de:2504:8a1f1043:a1";

    #[test]
    fn the_graphics_card_is_named_by_its_identity_with_the_cpu_to_fall_back_on() {
        assert_eq!(vision_section(None), json!({"execution_provider": "cpu"}));
        assert_eq!(
            vision_section(Some(CARD)),
            json!({
                "execution_provider": "directml",
                "directml_device": {"expected_identity": CARD},
                "gpu_fallback": "cpu",
            })
        );
        let mut chosen = input(vec![instance("azurlane-5", 5, "cn")], Some("C:\\MuMu"));
        chosen.directml_adapter = Some(CARD.to_owned());
        let document = build(&chosen);
        assert_eq!(document["vision"], vision_section(Some(CARD)));
        // No index, no LUID, no root: the Runtime denies what it does not know.
        let device = document["vision"]["directml_device"].as_object().unwrap();
        assert_eq!(device.len(), 1);
        assert_eq!(document["vision"].get("root"), None);
        // The first start runs the same card.
        let start = without_instances(&document);
        assert_eq!(start["vision"], document["vision"]);
        let mut control_plane = input(Vec::new(), Some("C:\\MuMu"));
        control_plane.directml_adapter = Some(CARD.to_owned());
        assert_eq!(start, build(&control_plane));
    }

    #[test]
    fn two_instances_make_the_shape_of_the_runtimes_minimal_configuration() {
        let document = build(&input(
            vec![instance("azurlane-5", 5, "cn"), instance("azurlane-6", 6, "jp")],
            Some("C:\\Program Files\\Netease\\MuMuPlayer"),
        ));
        assert_eq!(document["schema_version"], CONFIG_SCHEMA);
        assert_eq!(document["state_root"], "X:\\ActingCommand\\state");
        assert_eq!(document["bind_host"], "127.0.0.1");
        assert_eq!(document["bind_port"], 0);
        assert_eq!(document["secret_fingerprint_salt"].as_str().unwrap().len(), 64);
        assert_eq!(document["mumu_root"], "C:\\Program Files\\Netease\\MuMuPlayer");
        assert_eq!(document["standard_packs"], json!({"root": "packages"}));
        assert_eq!(document["vision"], json!({"execution_provider": "cpu"}));
        let entries = document["instances"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            json!({
                "alias": "azurlane-5",
                "instance_id": format!("instance_{}5", "0".repeat(31)),
                "instance_index": 5,
                "application_id": "com.example.cn",
                "touch_backend": "nemu_ipc",
                "capture_backend": "nemu_ipc",
                "start_emulator": true,
            })
        );
        assert_eq!(entries[1].get("start_emulator"), None);
        for entry in entries {
            for absent in ["instance_name", "serial", "port", "adb_path", "host", "resource_package"] {
                assert_eq!(entry.get(absent), None, "{absent}");
            }
        }
        // The policy's alias set equals the configured one, each on the one host.
        let facts = &document["policy"]["facts"];
        assert_eq!(facts["ledger_position"], 0);
        let rows = facts["instances"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1],
            json!({
                "instance_id": "azurlane-6",
                "server_id": "jp",
                "game_id": "azurlane",
                "host_id": "local",
                "available": true,
                "capability_operation_ids": ["operation.package.run"],
                "preferred_task_ids": [],
            })
        );
        assert_eq!(document["policy"]["resources"]["hosts"][0]["host_id"], "local");
        assert_eq!(document["policy"].get("catalog"), None);
        for absent in ["lab_module_authoring", "lab_module_debugging"] {
            assert_eq!(document.get(absent), None);
        }
    }

    #[test]
    fn no_instance_is_a_control_plane_without_policy() {
        let document = build(&input(Vec::new(), None));
        assert_eq!(document["instances"], json!([]));
        assert_eq!(document.get("policy"), None);
        assert_eq!(document.get("mumu_root"), None);
        let pinned = with_mumu_root(&document, "C:\\MuMu");
        assert_eq!(pinned["mumu_root"], "C:\\MuMu");
        let text = render(&pinned);
        assert!(text.ends_with("}\n") && !text.starts_with('\u{feff}'));
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), pinned);
    }

    #[test]
    fn the_first_start_runs_the_same_configuration_without_its_instances() {
        let mumu = Some("C:\\MuMu");
        let full = build(&input(vec![instance("azurlane-5", 5, "cn"), instance("azurlane-6", 6, "jp")], mumu));
        let start = without_instances(&full);
        // What a configuration without instances is: the control plane, nothing else changed.
        assert_eq!(start, build(&input(Vec::new(), mumu)));
        assert_eq!(start["instances"], json!([]));
        assert_eq!(start.get("policy"), None);
        for kept in ["schema_version", "state_root", "bind_host", "bind_port", "secret_fingerprint_salt", "mumu_root", "standard_packs", "vision"] {
            assert_eq!(start[kept], full[kept], "{kept}");
        }
        // The full one is left as it was.
        assert_eq!(full["instances"].as_array().map(Vec::len), Some(2));
        assert!(full.get("policy").is_some());
        // A document already without instances stays as it is.
        assert_eq!(without_instances(&start), start);
    }

    #[test]
    fn identifiers_and_backends_follow_the_runtimes_rules() {
        assert!(is_instance_id("instance_0123456789abcdef0123456789abcdef"));
        assert!(!is_instance_id("instance_0123456789ABCDEF0123456789abcdef"));
        assert!(!is_instance_id("instance_0123"));
        assert!(!is_instance_id("0123456789abcdef0123456789abcdef"));
        assert_eq!(backend_problem("nemu_ipc", "nemu_ipc"), None);
        assert_eq!(backend_problem("maatouch", "adb"), None);
        assert!(backend_problem("nemu_ipc", "adb").is_some());
        assert!(backend_problem("auto", "nemu_ipc").is_some());
        assert!(backend_problem("adb_shell_input", "scrcpy").is_some());
    }
}
