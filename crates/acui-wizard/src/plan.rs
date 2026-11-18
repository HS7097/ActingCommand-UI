// SPDX-License-Identifier: GPL-3.0-only
//! The install plan (`actingcommand.install-wizard-plan.v1`): what the person chose on the
//! window's pages — the games, each instance with its alias, game, server and backends, each
//! phone's instance likewise (`handsets`), and the options. The window builds one;
//! `acsetup install --plan <file>` reads one. Members a later wizard adds are read past.
//!
//! A plan file holds no phone ([`parse_plan`]): the Lab names a phone by a device handle that
//! holds only for the adb session that listed it (a replug or an adb server restart changes it:
//! Runtime `contracts/device-binding.md`), so a handle written into a file is stale by the time
//! the file is read; and the device key the window names it by is made under the run's own key
//! salt, which no file holds and no later run has. Phones are the window's alone, which lists
//! them and installs them in one run.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::alias::{alias_problem, MAX_ALIAS_BYTES};
use crate::apps::Game;
use crate::config::{
    backend_problem, handset_backend_problem, DEFAULT_BACKEND, HANDSET_ALIAS_LIMIT, HANDSET_CAPTURE_DEFAULT,
    HANDSET_KIND, HANDSET_TOUCH_DEFAULT,
};
use crate::text::{Lang, Said};

pub const PLAN_SCHEMA: &str = "actingcommand.install-wizard-plan.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub schema_version: String,
    /// `zh` or `en`: the wizard's language, which the console's settings file starts with.
    pub language: String,
    /// The games to install, by folder name.
    pub games: Vec<String>,
    /// The MuMu install root, when the person or the discovery named one; else the Runtime's
    /// own resolution (`check-config`'s `mumu_root`) is pinned.
    #[serde(default)]
    pub mumu_root: Option<String>,
    #[serde(default)]
    pub instances: Vec<PlanInstance>,
    /// The phones' instances: the window's alone, never a file's ([`parse_plan`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handsets: Vec<PlanHandset>,
    #[serde(default)]
    pub options: PlanOptions,
}

/// A phone's instance (an Android phone over USB, `config::HandsetInstance`): its alias, game,
/// server and backends, a phone's own (`adb_shell_input` and `adb` unless chosen otherwise). The
/// phone it stands for is the one the window's device list showed in the same run, named by its
/// `device_key` there (`devices`: the keyed digest of its serial under the run's key salt), by
/// which the install's bind finds it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanHandset {
    pub alias: String,
    pub game: String,
    pub server: String,
    #[serde(default = "handset_touch")]
    pub touch_backend: String,
    #[serde(default = "handset_capture")]
    pub capture_backend: String,
    #[serde(default)]
    pub device_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanInstance {
    /// The MuMu instance index (`MuMuManager info -v all`).
    pub index: u16,
    pub alias: String,
    pub game: String,
    pub server: String,
    #[serde(default = "default_backend")]
    pub touch_backend: String,
    #[serde(default = "default_backend")]
    pub capture_backend: String,
    /// The Runtime starts this emulator when it is stopped at the Runtime's start.
    #[serde(default)]
    pub start_emulator: bool,
    /// The person confirmed taking an instance another ActingCommand on this machine uses.
    #[serde(default)]
    pub confirmed_in_use: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOptions {
    #[serde(default = "yes")]
    pub desktop_shortcut: bool,
    #[serde(default = "yes")]
    pub start_menu_shortcut: bool,
    /// The watchdog's scheduled task (start at logon and after a crash); off by default.
    #[serde(default)]
    pub watchdog: bool,
    #[serde(default = "yes")]
    pub start_after_install: bool,
    /// The first start's global pause, which the console lifts ("put all on duty").
    #[serde(default = "yes")]
    pub pause_first_start: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            desktop_shortcut: true,
            start_menu_shortcut: true,
            watchdog: false,
            start_after_install: true,
            pause_first_start: true,
        }
    }
}

fn yes() -> bool {
    true
}

fn default_backend() -> String {
    DEFAULT_BACKEND.to_owned()
}

fn handset_touch() -> String {
    HANDSET_TOUCH_DEFAULT.to_owned()
}

fn handset_capture() -> String {
    HANDSET_CAPTURE_DEFAULT.to_owned()
}

impl Plan {
    pub fn lang(&self) -> Lang {
        Lang::from_wire(&self.language).unwrap_or(Lang::Zh)
    }
}

/// Whether a plan file's JSON names a phone: a `handsets` entry, or an instance whose `device`
/// is a phone's as the configuration writes it (`{"kind": "handset"}`).
fn names_a_phone(document: &Value) -> bool {
    document["handsets"].as_array().is_some_and(|handsets| !handsets.is_empty())
        || document["instances"]
            .as_array()
            .is_some_and(|instances| instances.iter().any(|instance| instance["device"]["kind"] == HANDSET_KIND))
}

/// The plan file in `bytes` (a UTF-8 byte-order mark allowed), as JSON; its rules are
/// [`plan_problems`]. A file that names a phone is refused before anything else: see the
/// module's description.
pub fn parse_plan(bytes: &[u8]) -> Result<Plan, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if serde_json::from_slice::<Value>(bytes).is_ok_and(|document| names_a_phone(&document)) {
        return Err(Said::new(
            "计划文件里不能写手机实例：手机要用向导这一次运行自己的密钥盐算出的设备键来指认，这个盐不在任何文件里，所以从命令行安装不装手机",
            "a plan file cannot hold a phone's instance: a phone is named by the device key made under the key salt of the wizard's own run, which is in no file, so an install from the command line installs no phone",
        ));
    }
    serde_json::from_slice(bytes)
        .map_err(|error| Said::new("计划文件读不懂", "the plan file does not parse").with(error))
}

/// Every rule of a plan against the games of the install set: the schema and language; each
/// game in the set and chosen once; each instance once by index and by alias (aliases compared
/// as the Runtime compares them, byte for byte), its alias valid for its game (`alias`: no
/// whitespace, no leading `-`, within the game's alias limit), its game chosen, its server one
/// of the game's, its backends a valid pair; each phone's instance by the same rules, its alias within
/// [`HANDSET_ALIAS_LIMIT`] bytes too and once among every instance's, its backends a phone's,
/// its phone named by a device key and no phone taken twice (one device drives one instance,
/// Runtime `contracts/device-binding.md`, "The private binding file");
/// with instances, the first-start pause only together with the wizard's start, which alone can
/// set it (a first start with the instances and without the pause the options page promised is
/// never possible). The watchdog needs no rule: it never
/// starts a root that never started (Runtime `contracts/runtime-watchdog.md`, "Decision", row
/// 3), so with the wizard's start left out the root stays down until a person starts it.
pub fn plan_problems(plan: &Plan, games: &[Game]) -> Vec<Said> {
    let mut problems = Vec::new();
    if plan.schema_version != PLAN_SCHEMA {
        problems.push(
            Said::new("计划文件的格式版本不对", "the plan is not actingcommand.install-wizard-plan.v1")
                .with(&plan.schema_version),
        );
    }
    if Lang::from_wire(&plan.language).is_none() {
        problems.push(Said::new("语言只能是 zh 或 en", "the language is zh or en").with(&plan.language));
    }
    let mut chosen = BTreeSet::new();
    for game in &plan.games {
        if !games.iter().any(|known| known.game == *game) {
            problems.push(Said::new("安装集里没有这个游戏", "the install set has no such game").with(game));
        } else if !chosen.insert(game.as_str()) {
            problems.push(Said::new("游戏选了两次", "the game is chosen twice").with(game));
        }
    }
    let mut indexes = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    for instance in &plan.instances {
        let named = format!("#{} {}", instance.index, instance.alias);
        if !indexes.insert(instance.index) {
            problems.push(Said::new("同一台模拟器选了两次", "one emulator is chosen twice").with(&named));
        }
        let limit = games
            .iter()
            .find(|known| known.game == instance.game)
            .map_or(MAX_ALIAS_BYTES, |game| game.alias_limit);
        if let Some(problem) = alias_problem(&instance.alias, limit) {
            problems.push(problem.with(format!("#{}", instance.index)));
        } else if !aliases.insert(instance.alias.as_str()) {
            problems.push(
                Said::new("有两个实例用了同一个名字，请改成不同的", "Two instances have the same name; give them different names")
                    .with(&named),
            );
        }
        if !chosen.contains(instance.game.as_str()) {
            problems.push(
                Said::new("实例的游戏没有勾选安装", "the instance's game is not among the games installed")
                    .with(format!("{named}: {}", instance.game)),
            );
        } else if let Some(game) = games.iter().find(|known| known.game == instance.game) {
            if game.server(&instance.server).is_none() {
                problems.push(
                    Said::new("这个游戏没有这个区服", "the game has no such server")
                        .with(format!("{named}: {}", instance.server)),
                );
            }
        }
        if let Some(problem) = backend_problem(&instance.touch_backend, &instance.capture_backend) {
            problems.push(problem.with(&named));
        }
    }
    let mut phones = BTreeSet::new();
    for handset in &plan.handsets {
        let named = format!("{HANDSET_KIND} {}", handset.alias);
        if handset.device_key.is_empty() {
            problems.push(
                Said::new("手机实例没有对应的手机（缺设备键），绑不上", "a phone's instance names no phone (no device key), so it cannot be bound")
                    .with(&named),
            );
        } else if !phones.insert(handset.device_key.as_str()) {
            problems.push(Said::new("同一部手机选了两次", "one phone is chosen twice").with(&named));
        }
        let limit = games
            .iter()
            .find(|known| known.game == handset.game)
            .map_or(MAX_ALIAS_BYTES, |game| game.alias_limit)
            .min(HANDSET_ALIAS_LIMIT);
        if let Some(problem) = alias_problem(&handset.alias, limit) {
            problems.push(problem.with(HANDSET_KIND));
        } else if !aliases.insert(handset.alias.as_str()) {
            problems.push(
                Said::new("有两个实例用了同一个名字，请改成不同的", "Two instances have the same name; give them different names")
                    .with(&named),
            );
        }
        if !chosen.contains(handset.game.as_str()) {
            problems.push(
                Said::new("实例的游戏没有勾选安装", "the instance's game is not among the games installed")
                    .with(format!("{named}: {}", handset.game)),
            );
        } else if let Some(game) = games.iter().find(|known| known.game == handset.game) {
            if game.server(&handset.server).is_none() {
                problems.push(
                    Said::new("这个游戏没有这个区服", "the game has no such server")
                        .with(format!("{named}: {}", handset.server)),
                );
            }
        }
        if let Some(problem) = handset_backend_problem(&handset.touch_backend, &handset.capture_backend) {
            problems.push(problem.with(&named));
        }
    }
    let instances = !plan.instances.is_empty() || !plan.handsets.is_empty();
    if instances && plan.options.pause_first_start && !plan.options.start_after_install {
        problems.push(Said::new(
            "要首次先暂停，就得勾「装完就启动」：只有向导启动了 ActingCommand 才能挂上暂停；不想启动，就不勾首次暂停（那样第一次启动时实例会直接开始执勤）",
            "The first-start pause needs \"start after install\": only a Runtime the wizard started takes the pause. Not to start it, leave the pause out too (the instances then go on duty at the first start)",
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::Server;

    fn games() -> Vec<Game> {
        vec![Game {
            game: "azurlane".into(),
            label: "碧蓝航线".into(),
            servers: vec![
                Server { server: "cn".into(), label: "国服".into(), application_id: "com.bilibili.azurlane".into() },
                Server { server: "jp".into(), label: "日服".into(), application_id: "com.YoStarJP.AzurLane".into() },
            ],
            // A schedule whose longest id is 115 bytes leaves 12 for an alias.
            alias_limit: crate::alias::alias_limit(115),
        }]
    }

    fn plan(text: &str) -> Plan {
        parse_plan(text.as_bytes()).expect("a plan")
    }

    const GOOD: &str = r#"{
        "schema_version": "actingcommand.install-wizard-plan.v1",
        "language": "zh",
        "games": ["azurlane"],
        "instances": [
            {"index": 5, "alias": "azurlane-5", "game": "azurlane", "server": "cn"},
            {"index": 6, "alias": "日服号", "game": "azurlane", "server": "jp",
             "touch_backend": "maatouch", "capture_backend": "adb", "start_emulator": true}
        ],
        "options": {"watchdog": false}
    }"#;

    #[test]
    fn defaults_fill_what_the_plan_leaves_out() {
        let plan = plan(GOOD);
        assert_eq!(plan_problems(&plan, &games()), Vec::<Said>::new());
        assert_eq!(plan.instances[0].touch_backend, "nemu_ipc");
        assert_eq!(plan.instances[0].capture_backend, "nemu_ipc");
        assert!(!plan.instances[0].start_emulator && !plan.instances[0].confirmed_in_use);
        assert!(plan.options.desktop_shortcut && plan.options.start_menu_shortcut);
        assert!(plan.options.start_after_install && plan.options.pause_first_start);
        assert!(!plan.options.watchdog);
        assert_eq!(plan.mumu_root, None);
        assert_eq!(plan.lang(), Lang::Zh);
    }

    #[test]
    fn each_broken_rule_is_named() {
        let broken = [
            GOOD.replace("install-wizard-plan.v1", "install-wizard-plan.v2"),
            GOOD.replace("\"zh\"", "\"fr\""),
            GOOD.replace("[\"azurlane\"]", "[\"azurlane\", \"arknights\"]"),
            GOOD.replace("[\"azurlane\"]", "[\"azurlane\", \"azurlane\"]"),
            GOOD.replace("\"index\": 6", "\"index\": 5"),
            GOOD.replace("日服号", "azurlane-5"),
            GOOD.replace("日服号", ""),
            GOOD.replace("日服号", "日服 号"),
            GOOD.replace("日服号", "日服\u{3000}号"),
            // A leading `-`, which the Lab could take for an option (`alias`).
            GOOD.replace("日服号", "-日服号"),
            // 13 bytes, one over the game's limit; 12 is still fine (below).
            GOOD.replace("日服号", "日服号号a"),
            GOOD.replace("\"server\": \"jp\"", "\"server\": \"kr\""),
            GOOD.replace("[\"azurlane\"]", "[]"),
            GOOD.replace("\"capture_backend\": \"adb\"", "\"capture_backend\": \"auto\""),
            GOOD.replace("\"maatouch\"", "\"nemu_ipc\""),
        ];
        for text in broken {
            assert!(!plan_problems(&plan(&text), &games()).is_empty(), "{text}");
        }
        // An alias of exactly the game's limit, twelve bytes, is fine.
        assert_eq!(plan_problems(&plan(&GOOD.replace("日服号", "日服号号")), &games()), Vec::<Said>::new());
        // The watchdog leaves a root that never started down, so it needs neither the wizard's
        // start nor the pause turned off.
        for options in [
            "{\"watchdog\": true, \"start_after_install\": false, \"pause_first_start\": false}",
            "{\"watchdog\": true, \"start_after_install\": true, \"pause_first_start\": true}",
            "{\"watchdog\": false, \"start_after_install\": true, \"pause_first_start\": false}",
        ] {
            let text = GOOD.replace("{\"watchdog\": false}", options);
            assert_eq!(plan_problems(&plan(&text), &games()), Vec::<Said>::new(), "{options}");
        }
        // With instances, the pause without the wizard's start (the pause is on by default) is
        // refused: the first start would bring them unpaused.
        for options in [
            "{\"watchdog\": true, \"start_after_install\": false}",
            "{\"start_after_install\": false, \"pause_first_start\": true}",
        ] {
            let text = GOOD.replace("{\"watchdog\": false}", options);
            let problems = plan_problems(&plan(&text), &games());
            assert_eq!(problems.len(), 1, "{options}");
            assert!(problems[0].en.contains("start after install"), "{}", problems[0]);
        }
        // Without instances there is nothing the pause would hold back.
        let none = GOOD
            .replace("{\"watchdog\": false}", "{\"start_after_install\": false}")
            .replace(
                r#"{"index": 5, "alias": "azurlane-5", "game": "azurlane", "server": "cn"},
            {"index": 6, "alias": "日服号", "game": "azurlane", "server": "jp",
             "touch_backend": "maatouch", "capture_backend": "adb", "start_emulator": true}"#,
                "",
            );
        let none = plan(&none);
        assert!(none.instances.is_empty());
        assert_eq!(plan_problems(&none, &games()), Vec::<Said>::new());
    }

    #[test]
    fn a_plan_file_that_names_a_phone_is_refused() {
        let handsets = GOOD.replace(
            "\"options\"",
            r#""handsets": [{"alias": "phone-1", "game": "azurlane", "server": "cn", "device_key": "sha256:aa"}],
        "options""#,
        );
        let device = GOOD.replace(
            r#""server": "cn"}"#,
            r#""server": "cn", "device": {"kind": "handset", "transport": "usb"}}"#,
        );
        for text in [handsets, device] {
            let refused = parse_plan(text.as_bytes()).expect_err("a phone in a plan file");
            assert!(refused.en.starts_with("a plan file cannot hold a phone's instance"), "{refused}");
        }
        // An empty list names no phone, and neither does an instance without a phone's device.
        let empty = GOOD.replace("\"options\"", "\"handsets\": [],\n        \"options\"");
        assert!(plan(&empty).handsets.is_empty());
        let emulator = GOOD.replace(r#""server": "cn"}"#, r#""server": "cn", "device": {"kind": "emulator"}}"#);
        if let Err(said) = parse_plan(emulator.as_bytes()) {
            assert!(!said.en.starts_with("a plan file cannot hold a phone's instance"), "{said}");
        }
    }

    #[test]
    fn each_broken_rule_of_a_phones_instance_is_named() {
        let phone = |alias: &str, key: &str| PlanHandset {
            alias: alias.to_owned(),
            game: "azurlane".to_owned(),
            server: "cn".to_owned(),
            touch_backend: HANDSET_TOUCH_DEFAULT.to_owned(),
            capture_backend: HANDSET_CAPTURE_DEFAULT.to_owned(),
            device_key: key.to_owned(),
        };
        let with = |handsets: Vec<PlanHandset>| Plan { handsets, ..plan(GOOD) };
        // Two phones beside the emulators, each named by its own key: fine.
        let good = with(vec![phone("phone-1", "sha256:aa"), phone("phone-2", "sha256:bb")]);
        assert_eq!(plan_problems(&good, &games()), Vec::<Said>::new());
        let broken = [
            // No device key, one phone taken twice.
            vec![phone("phone-1", "")],
            vec![phone("phone-1", "sha256:aa"), phone("phone-2", "sha256:aa")],
            // An emulator's alias, two phones with one alias, an empty alias, a leading `-`, and
            // one byte over the game's limit of twelve.
            vec![phone("azurlane-5", "sha256:aa")],
            vec![phone("phone-1", "sha256:aa"), phone("phone-1", "sha256:bb")],
            vec![phone("", "sha256:aa")],
            vec![phone("-phone", "sha256:aa")],
            vec![phone("phone-123456a", "sha256:aa")],
            // A game not installed, a server the game does not have, an emulator's backends.
            vec![PlanHandset { game: "arknights".to_owned(), ..phone("phone-1", "sha256:aa") }],
            vec![PlanHandset { server: "kr".to_owned(), ..phone("phone-1", "sha256:aa") }],
            vec![PlanHandset {
                touch_backend: "nemu_ipc".to_owned(),
                capture_backend: "nemu_ipc".to_owned(),
                ..phone("phone-1", "sha256:aa")
            }],
        ];
        for handsets in broken {
            let text = format!("{handsets:?}");
            assert!(!plan_problems(&with(handsets), &games()).is_empty(), "{text}");
        }
        // A phone alone is an instance too: the pause without the wizard's start is refused.
        let mut alone = Plan { instances: Vec::new(), ..with(vec![phone("phone-1", "sha256:aa")]) };
        assert_eq!(plan_problems(&alone, &games()), Vec::<Said>::new());
        alone.options.start_after_install = false;
        let problems = plan_problems(&alone, &games());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].en.contains("start after install"), "{}", problems[0]);
    }
}
