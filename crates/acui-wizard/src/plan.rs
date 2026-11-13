// SPDX-License-Identifier: GPL-3.0-only
//! The install plan (`actingcommand.install-wizard-plan.v1`): what the person chose on the
//! window's pages — the games, each instance with its alias, game, server and backends, and the
//! options. The window builds one; `acsetup install --plan <file>` reads one. Members a later
//! wizard adds are read past.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::alias::{alias_problem, MAX_ALIAS_BYTES};
use crate::apps::Game;
use crate::config::{backend_problem, DEFAULT_BACKEND};
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
    #[serde(default)]
    pub options: PlanOptions,
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

impl Plan {
    pub fn lang(&self) -> Lang {
        Lang::from_wire(&self.language).unwrap_or(Lang::Zh)
    }
}

/// The plan in `bytes` (a UTF-8 byte-order mark allowed), as JSON; its rules are
/// [`plan_problems`].
pub fn parse_plan(bytes: &[u8]) -> Result<Plan, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    serde_json::from_slice(bytes)
        .map_err(|error| Said::new("计划文件读不懂", "the plan file does not parse").with(error))
}

/// Every rule of a plan against the games of the install set: the schema and language; each
/// game in the set and chosen once; each instance once by index and by alias (aliases compared
/// as the Runtime compares them, byte for byte), its alias valid for its game (`alias`: no
/// whitespace, within the game's alias limit), its game chosen, its server one of the game's,
/// its backends a valid pair; and a first-start pause the wizard can set: with
/// the watchdog on, the watchdog starts the Runtime within a minute, so the pause needs the
/// wizard's own start.
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
            problems.push(Said::new("同一台模拟器实例选了两次", "one emulator instance is chosen twice").with(&named));
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
    let options = &plan.options;
    if options.watchdog && options.pause_first_start && !options.start_after_install {
        problems.push(Said::new(
            "开了看门狗时，它会在一分钟内自己拉起 Runtime，首次暂停只能在向导自己启动时挂上：请勾选「装完就启动」，或取消「首次先暂停」",
            "With the watchdog on, it starts the Runtime within a minute, and the first-start pause can only be set when the wizard starts it: choose \"start after install\", or turn the pause off",
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
            // 13 bytes, one over the game's limit; 12 is still fine (below).
            GOOD.replace("日服号", "日服号号a"),
            GOOD.replace("\"server\": \"jp\"", "\"server\": \"kr\""),
            GOOD.replace("[\"azurlane\"]", "[]"),
            GOOD.replace("\"capture_backend\": \"adb\"", "\"capture_backend\": \"auto\""),
            GOOD.replace("\"maatouch\"", "\"nemu_ipc\""),
            GOOD.replace(
                "{\"watchdog\": false}",
                "{\"watchdog\": true, \"start_after_install\": false}",
            ),
        ];
        for text in broken {
            assert!(!plan_problems(&plan(&text), &games()).is_empty(), "{text}");
        }
        // An alias of exactly the game's limit, twelve bytes, is fine.
        assert_eq!(plan_problems(&plan(&GOOD.replace("日服号", "日服号号")), &games()), Vec::<Said>::new());
        // The watchdog without the wizard's start is fine when no pause is asked for.
        let quiet = GOOD.replace(
            "{\"watchdog\": false}",
            "{\"watchdog\": true, \"start_after_install\": false, \"pause_first_start\": false}",
        );
        assert_eq!(plan_problems(&plan(&quiet), &games()), Vec::<Said>::new());
    }
}
