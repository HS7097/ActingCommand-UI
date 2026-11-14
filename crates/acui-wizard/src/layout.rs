// SPDX-License-Identifier: GPL-3.0-only
//! A standard package's bundle zip laid cold into `<packages>\<game>\`, the installed layout the
//! Runtime reads (Runtime `contracts/package-reference.md`, "Standard package root";
//! `contracts/resource-verify.md`, the `bundle` step's `installed/<game>/`): the root files as
//! they are, each `packs/<package_id>/` without its `packs/` prefix, `tables/<id>.json` and the
//! `notices/` and `loading/` folders as they are; `interfaces.json` is checked, never laid
//! (`crates/lab-rescue/src/install/place.rs`: "checked, never placed"). The Runtime 0.12.58 has
//! no cold placement of its own (`actingctl install pack` is not built); `actinglab
//! standard-pack check` reads the result afterwards.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::text::Said;

pub const BUNDLE_SCHEMA: &str = "actingcommand.bundle.v4";
pub const BUNDLE_FILE: &str = "bundle.json";
pub const INTERFACES_FILE: &str = "interfaces.json";
pub const APPLICATIONS_FILE: &str = "applications.json";
pub const SCHEDULE_FILE: &str = "schedule.json";
/// The root files a bundle zip may carry (Runtime `place.rs` `ROOT_FILES`; `goals.json` since
/// Runtime 0.12.60, `package` 12, Workflow #455 G-R1).
pub const ROOT_FILES: [&str; 10] = [
    "bundle.json",
    "pages.json",
    "interrupts.json",
    "navigation.json",
    "display-layout.json",
    "loading.json",
    "ui.json",
    "applications.json",
    "schedule.json",
    "goals.json",
];
const PACKS_DIRECTORY: &str = "packs";
const TABLES_DIRECTORY: &str = "tables";
const NOTICES_DIRECTORY: &str = "notices";
const LOADING_DIRECTORY: &str = "loading";
const PACK_CONTROL_FILE: &str = "control.json";

/// What the wizard reads of `bundle.json` (Runtime `crates/actingcommand-contract/src/
/// package.rs` `BundleIndex`): the game, the packs by `package_id`, the tables. The Runtime's own
/// checks of the whole file run in `standard-pack check`.
#[derive(Debug, Clone, Deserialize)]
pub struct BundleIndex {
    pub schema_version: String,
    pub game: String,
    pub packs: Vec<BundlePack>,
    #[serde(default)]
    pub tables: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundlePack {
    pub package_id: String,
    pub server: String,
}

/// The bundle index in `bytes` (a UTF-8 byte-order mark allowed): bundle v4, a game, one pack
/// at least, each `package_id` once.
pub fn parse_bundle(bytes: &[u8]) -> Result<BundleIndex, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let index: BundleIndex = serde_json::from_slice(bytes)
        .map_err(|error| Said::new("bundle.json 读不懂", "bundle.json does not parse").with(error))?;
    if index.schema_version != BUNDLE_SCHEMA {
        return Err(Said::new(
            "bundle.json 不是 bundle v4",
            "bundle.json is not a bundle v4 index",
        )
        .with(&index.schema_version));
    }
    if !plain_name(&index.game) || index.packs.is_empty() {
        return Err(Said::new(
            "bundle.json 的游戏名或包列表无效",
            "bundle.json names no valid game or no pack",
        )
        .with(&index.game));
    }
    let mut ids = BTreeSet::new();
    for pack in &index.packs {
        if !plain_name(&pack.package_id) || !ids.insert(pack.package_id.as_str()) {
            return Err(Said::new(
                "bundle.json 的包名无效或重复",
                "bundle.json names a package invalidly or twice",
            )
            .with(&pack.package_id));
        }
    }
    Ok(index)
}

/// A single folder name: not empty, not `.` or `..`, no separator, `:` or control character.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}

/// Where one zip entry goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Laid at this `/`-separated path below the game root.
    Lay(String),
    /// Read and checked, never laid (`interfaces.json`).
    CheckOnly,
}

/// Where the zip entry `name` goes below the game root, for a bundle with these pack ids and
/// tables; an entry the installed layout has no place for is refused.
pub fn place_of(
    name: &str,
    pack_ids: &BTreeSet<String>,
    tables: &BTreeSet<String>,
) -> Result<Placement, Said> {
    let unknown = || {
        Said::new(
            "标准包 zip 里有已安装布局没有位置的条目",
            "the standard package zip holds an entry the installed layout has no place for",
        )
        .with(name)
    };
    let parts: Vec<&str> = name.split('/').collect();
    match parts.as_slice() {
        [file] if *file == INTERFACES_FILE => Ok(Placement::CheckOnly),
        [file] if ROOT_FILES.contains(file) => Ok(Placement::Lay((*file).to_owned())),
        [packs, id, rest @ ..]
            if *packs == PACKS_DIRECTORY && pack_ids.contains(*id) && !rest.is_empty() =>
        {
            Ok(Placement::Lay(format!("{id}/{}", rest.join("/"))))
        }
        [directory, file]
            if *directory == TABLES_DIRECTORY
                && file
                    .strip_suffix(".json")
                    .is_some_and(|table| tables.contains(table)) =>
        {
            Ok(Placement::Lay(name.to_owned()))
        }
        [directory, rest @ ..] if *directory == NOTICES_DIRECTORY && !rest.is_empty() => {
            // A pack named like the notices folder would take that folder (place.rs).
            if takes_folder(pack_ids, NOTICES_DIRECTORY) {
                return Err(unknown());
            }
            Ok(Placement::Lay(name.to_owned()))
        }
        [directory, _file] if *directory == LOADING_DIRECTORY => {
            // The same for the loading folder: the pack's files and the loading pictures would
            // share one folder of the game root.
            if takes_folder(pack_ids, LOADING_DIRECTORY) {
                return Err(unknown());
            }
            Ok(Placement::Lay(name.to_owned()))
        }
        _ => Err(unknown()),
    }
}

/// Whether a pack of `pack_ids` is named like the game root's folder `folder` (Windows folds
/// case), so that its installed folder would be that one.
fn takes_folder(pack_ids: &BTreeSet<String>, folder: &str) -> bool {
    pack_ids.iter().any(|id| id.eq_ignore_ascii_case(folder))
}

/// The whole layout of a bundle zip with these entries: each entry laid with its target below
/// the game root, sorted by entry; `bundle.json` and `interfaces.json` must be there, each pack
/// must carry its `control.json`, and no two entries may land on one path (Windows paths fold
/// case).
pub fn plan_layout(names: &[String], index: &BundleIndex) -> Result<Vec<(String, String)>, Said> {
    let pack_ids: BTreeSet<String> = index.packs.iter().map(|pack| pack.package_id.clone()).collect();
    let tables: BTreeSet<String> = index.tables.clone().unwrap_or_default().into_iter().collect();
    let mut laid = Vec::new();
    let mut targets: BTreeMap<String, String> = BTreeMap::new();
    let mut interfaces = false;
    for name in names {
        match place_of(name, &pack_ids, &tables)? {
            Placement::CheckOnly => interfaces = true,
            Placement::Lay(target) => {
                if let Some(earlier) = targets.insert(target.to_ascii_lowercase(), name.clone()) {
                    return Err(Said::new(
                        "标准包 zip 里有两项落在同一位置",
                        "two entries of the standard package zip land on one path",
                    )
                    .with(format!("{earlier}, {name}")));
                }
                laid.push((name.clone(), target));
            }
        }
    }
    if !laid.iter().any(|(name, _)| name == BUNDLE_FILE) {
        return Err(Said::new("标准包 zip 里没有 bundle.json", "the standard package zip has no bundle.json"));
    }
    if !interfaces {
        return Err(Said::new(
            "标准包 zip 里没有 interfaces.json",
            "the standard package zip has no interfaces.json",
        ));
    }
    for id in &pack_ids {
        let control = format!("{PACKS_DIRECTORY}/{id}/{PACK_CONTROL_FILE}");
        if !names.iter().any(|name| *name == control) {
            return Err(Said::new(
                "标准包 zip 里的包缺少 control.json",
                "a pack of the standard package zip has no control.json",
            )
            .with(id));
        }
    }
    laid.sort();
    Ok(laid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(tables: &[&str]) -> BundleIndex {
        let text = format!(
            r#"{{"schema_version":"actingcommand.bundle.v4","game":"azurlane",
                "source":{{"repository":"r","commit":"c"}},
                "packs":[{{"package_id":"azurlane.daily","server":"cn","entry_task_id":"t",
                           "path":"packs/azurlane.daily","file_count":2,"byte_count":10}},
                         {{"package_id":"azurlane.home","server":"cn","entry_task_id":"h",
                           "path":"packs/azurlane.home","file_count":1,"byte_count":5}}],
                "tables":{}}}"#,
            serde_json::to_string(tables).unwrap()
        );
        parse_bundle(text.as_bytes()).expect("a bundle v4 index")
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_installed_layout_drops_the_packs_prefix_and_keeps_the_root() {
        // The zip root of `actinglab package bundle` (Runtime contracts/package-reference.md:250-264)
        // against the installed root (the same file, 720-753).
        let zip = names(&[
            "applications.json",
            "bundle.json",
            "interfaces.json",
            "pages.json",
            "schedule.json",
            "loading.json",
            "loading/title.png",
            "tables/drops.json",
            "notices/readme.md",
            "notices/licenses/assets.txt",
            "packs/azurlane.daily/control.json",
            "packs/azurlane.daily/resources/a.png",
            "packs/azurlane.home/control.json",
        ]);
        let laid = plan_layout(&zip, &index(&["drops"])).expect("a layout");
        let laid: BTreeMap<&str, &str> =
            laid.iter().map(|(name, target)| (name.as_str(), target.as_str())).collect();
        assert_eq!(laid.get("interfaces.json"), None);
        assert_eq!(laid["bundle.json"], "bundle.json");
        assert_eq!(laid["applications.json"], "applications.json");
        assert_eq!(laid["loading/title.png"], "loading/title.png");
        assert_eq!(laid["tables/drops.json"], "tables/drops.json");
        assert_eq!(laid["notices/licenses/assets.txt"], "notices/licenses/assets.txt");
        assert_eq!(laid["packs/azurlane.daily/resources/a.png"], "azurlane.daily/resources/a.png");
        assert_eq!(laid["packs/azurlane.home/control.json"], "azurlane.home/control.json");
        assert_eq!(laid.len(), zip.len() - 1);
    }

    #[test]
    fn entries_without_a_place_are_refused() {
        let index = index(&[]);
        let base = [
            "bundle.json",
            "interfaces.json",
            "packs/azurlane.daily/control.json",
            "packs/azurlane.home/control.json",
        ];
        for extra in [
            "README.md",
            "packs/unlisted/control.json",
            "packs/azurlane.daily",
            "tables/unlisted.json",
            "loading/deep/x.png",
            "declaration-exclusions.json",
        ] {
            let mut zip = names(&base);
            zip.push(extra.to_owned());
            assert!(plan_layout(&zip, &index).is_err(), "{extra}");
        }
        assert!(plan_layout(&names(&base), &index).is_ok());
        // bundle.json, interfaces.json and each pack's control.json are required.
        for missing in base {
            let zip: Vec<String> = base
                .iter()
                .filter(|name| **name != missing)
                .map(|name| (*name).to_owned())
                .collect();
            assert!(plan_layout(&zip, &index).is_err(), "{missing}");
        }
    }

    #[test]
    fn two_entries_on_one_path_are_refused() {
        // A pack named `Tables` would land in the tables folder (Windows folds case); the
        // Runtime reserves the name, the wizard only reads `bundle.json` lightly.
        let mut index = index(&["drops"]);
        index.packs.push(BundlePack {
            package_id: "Tables".to_owned(),
            server: "cn".to_owned(),
        });
        let zip = names(&[
            "bundle.json",
            "interfaces.json",
            "tables/drops.json",
            "packs/azurlane.daily/control.json",
            "packs/azurlane.home/control.json",
            "packs/Tables/control.json",
            "packs/Tables/drops.json",
        ]);
        let refused = plan_layout(&zip, &index).expect_err("one path twice");
        assert!(refused.en.contains("land on one path"), "{refused}");
    }

    #[test]
    fn the_notices_and_loading_folders_are_laid_unless_a_pack_takes_their_name() {
        fn ids(list: &[&str]) -> BTreeSet<String> {
            list.iter().map(|id| (*id).to_owned()).collect()
        }
        let tables = BTreeSet::new();
        let packs = ids(&["azurlane.daily"]);
        // The notices folder at any depth, the loading folder one level deep.
        assert_eq!(
            place_of("notices/licenses/assets.txt", &packs, &tables),
            Ok(Placement::Lay("notices/licenses/assets.txt".to_owned()))
        );
        assert_eq!(place_of("notices/readme.md", &packs, &tables), Ok(Placement::Lay("notices/readme.md".to_owned())));
        assert_eq!(place_of("loading/title.png", &packs, &tables), Ok(Placement::Lay("loading/title.png".to_owned())));
        assert!(place_of("loading/deep/title.png", &packs, &tables).is_err());
        assert!(place_of("notices", &packs, &tables).is_err());
        // A pack named like either folder, in any case, would land in it: its entries are refused.
        for taken in ["notices", "Notices", "NOTICES"] {
            let packs = ids(&["azurlane.daily", taken]);
            assert!(place_of("notices/readme.md", &packs, &tables).is_err(), "{taken}");
            assert_eq!(place_of("loading/title.png", &packs, &tables), Ok(Placement::Lay("loading/title.png".to_owned())));
        }
        for taken in ["loading", "Loading"] {
            let packs = ids(&["azurlane.daily", taken]);
            assert!(place_of("loading/title.png", &packs, &tables).is_err(), "{taken}");
            assert_eq!(place_of("notices/readme.md", &packs, &tables), Ok(Placement::Lay("notices/readme.md".to_owned())));
        }
        // A pack named alike in part only takes nothing.
        let packs = ids(&["notices.extra", "loading-screens"]);
        assert!(place_of("notices/readme.md", &packs, &tables).is_ok());
        assert!(place_of("loading/title.png", &packs, &tables).is_ok());
    }

    #[test]
    fn a_bundle_index_is_v4_with_unique_packs() {
        assert!(parse_bundle(br#"{"schema_version":"actingcommand.bundle.v3","game":"g","packs":[]}"#).is_err());
        assert!(parse_bundle(br#"{"schema_version":"actingcommand.bundle.v4","game":"g","packs":[]}"#).is_err());
        assert!(parse_bundle(
            br#"{"schema_version":"actingcommand.bundle.v4","game":"g",
                 "packs":[{"package_id":"a","server":"s"},{"package_id":"a","server":"t"}]}"#
        )
        .is_err());
        assert!(parse_bundle(
            br#"{"schema_version":"actingcommand.bundle.v4","game":"../g",
                 "packs":[{"package_id":"a","server":"s"}]}"#
        )
        .is_err());
        let index = parse_bundle(
            b"\xef\xbb\xbf{\"schema_version\":\"actingcommand.bundle.v4\",\"game\":\"g\",\"packs\":[{\"package_id\":\"a\",\"server\":\"s\"}]}",
        )
        .expect("a byte-order mark is allowed");
        assert_eq!(index.game, "g");
    }
}
