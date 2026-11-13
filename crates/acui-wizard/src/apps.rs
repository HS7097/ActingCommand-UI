// SPDX-License-Identifier: GPL-3.0-only
//! A game's `applications.json` (`actingcommand.applications.v1`; Runtime
//! `apps/actinglab/src/resource_declarations.rs`): the game's display name and its servers, each
//! with its display name and the Android application id an instance of that server runs. The
//! window lists games and servers from it; the configuration takes each instance's
//! `application_id` from it (Runtime `contracts/runtime-config.md`, `idle.action` `close_app`).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::text::Said;

pub const APPLICATIONS_SCHEMA: &str = "actingcommand.applications.v1";

#[derive(Deserialize)]
struct ApplicationsFile {
    schema_version: String,
    game: String,
    #[serde(default)]
    label: Option<String>,
    servers: BTreeMap<String, ServerFile>,
}

#[derive(Deserialize)]
struct ServerFile {
    application_id: String,
    label: String,
}

/// A game as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// The game's folder name, `bundle.json`'s `game`.
    pub game: String,
    /// Its display name: the table's `label`, else the folder name.
    pub label: String,
    pub servers: Vec<Server>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// The server's key, an instance's `server_id`.
    pub server: String,
    pub label: String,
    pub application_id: String,
}

impl Game {
    pub fn server(&self, server: &str) -> Option<&Server> {
        self.servers.iter().find(|entry| entry.server == server)
    }
}

/// The table in `bytes` (a UTF-8 byte-order mark allowed): its schema, a game, one server at
/// least, each with an application id and a label.
pub fn parse_applications(bytes: &[u8]) -> Result<Game, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let file: ApplicationsFile = serde_json::from_slice(bytes).map_err(|error| {
        Said::new("applications.json 读不懂", "applications.json does not parse").with(error)
    })?;
    if file.schema_version != APPLICATIONS_SCHEMA {
        return Err(Said::new(
            "applications.json 的格式版本不对",
            "applications.json is not actingcommand.applications.v1",
        )
        .with(&file.schema_version));
    }
    if file.game.is_empty() || file.servers.is_empty() {
        return Err(Said::new(
            "applications.json 没有游戏名或区服",
            "applications.json names no game or no server",
        ));
    }
    let mut servers = Vec::with_capacity(file.servers.len());
    for (server, entry) in file.servers {
        if server.is_empty() || entry.application_id.trim().is_empty() || entry.label.is_empty() {
            return Err(Said::new(
                "applications.json 的区服缺少包名或显示名",
                "a server of applications.json lacks its application id or label",
            )
            .with(&server));
        }
        servers.push(Server {
            server,
            label: entry.label,
            application_id: entry.application_id,
        });
    }
    let label = file
        .label
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| file.game.clone());
    Ok(Game {
        game: file.game,
        label,
        servers,
    })
}
