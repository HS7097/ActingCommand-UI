// SPDX-License-Identifier: GPL-3.0-only
//! Setup's in-memory configuration plan. Package facts and chain rules belong to
//! execution-kernel; only the existing actingd configuration is committed here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use actingcommand_contract::{
    ContentDirectory, ContentDirectoryVersion, MaintenanceUse, PackageRef,
};
use actingcommand_execution_kernel::{
    validate_bundle_maintenance, PreparedContainedTask, PrerequisiteChain, TaskPackageDescriptor,
};
use serde_json::{json, Value};

use crate::bundle::Bundle;
use crate::verify::{Report, Step};

const READ_LIMIT: u64 = 16 << 20;
const ADMISSION_BUDGET: Duration = Duration::from_secs(120);

/// One instance whose resource association needs a person: the wizard asks
/// on its resource page, the command line takes `--associate <alias>=<bundle>/<server>`.
pub struct Association {
    pub alias: String,
    pub question: String,
    pub options: Vec<AssociationOption>,
}

/// One answer an association offers: its label for people, and the bundle (its
/// game id) and server the command line names it by.
pub struct AssociationOption {
    pub label: String,
    pub bundle: String,
    pub server: String,
}

impl AssociationOption {
    /// `<bundle>/<server>`, as `--associate` names this option.
    pub fn flag_value(&self) -> String {
        format!("{}/{}", self.bundle, self.server)
    }
}

/// The index of the option taken. No choice is implicit: `None` (only a plan
/// gives it) leaves the instance unassociated in that plan.
pub type Choose<'a> = &'a mut dyn FnMut(&Association) -> Result<Option<usize>, String>;

/// Asks one association and logs the question and the answer.
fn associate(
    association: &Association,
    choose: Choose<'_>,
    report: Report<'_>,
) -> Result<Option<usize>, String> {
    report.line(&format!(
        "资源关联 / Resource association: {}\n{}",
        association.alias, association.question
    ))?;
    let picked = choose(association)?;
    match picked {
        Some(at) => {
            let option = association
                .options
                .get(at)
                .ok_or("Invalid resource association")?;
            report.line(&format!(
                "资源关联 / Resource association: {} → {} ({})",
                association.alias,
                option.flag_value(),
                option.label
            ))?;
        }
        None => report.line(&format!(
            "资源关联未决定 / Resource association left open: {}",
            association.alias
        ))?,
    }
    Ok(picked)
}

pub struct Prepared {
    pub bundle: Bundle,
    pub paths: BTreeMap<String, PathBuf>,
    /// ZIP bundles have mutable pack filenames; isolate each verified bundle so
    /// an upgrade never overwrites a ZIP still named by the old configuration.
    zip_bundle_digest: Option<String>,
}

pub fn content_reference(hash: &str) -> PackageRef {
    PackageRef::ContentDirectory(ContentDirectory {
        schema_version: ContentDirectoryVersion::V1,
        sha256: hash.to_owned(),
    })
}

fn describe(
    path: &Path,
    reference: &PackageRef,
    deadline: Instant,
    report: Report<'_>,
) -> Result<TaskPackageDescriptor, String> {
    report.line(&format!(
        "校验资源身份 / Checking package identity: {}",
        path.display()
    ))?;
    if Instant::now() >= deadline {
        return Err(
            "资源准入超过 120 秒期限 / Package admission exceeded its 120-second deadline".into(),
        );
    }
    PreparedContainedTask::describe_path("acsetup", path, reference, deadline).map_err(|error| {
        format!(
            "资源准入失败 / Package admission failed: {}: {error}",
            path.display()
        )
    })
}

/// All packs, including packs no instance selects, must qualify before a v3
/// bundle can supply any binding. Staging is outside the installed packages.
pub fn prepare(
    bundles: &[Bundle],
    staging: &Path,
    report: Report<'_>,
) -> Result<Vec<Prepared>, String> {
    let deadline = Instant::now() + ADMISSION_BUDGET;
    let mut prepared = Vec::new();
    for (at, bundle) in bundles.iter().enumerate() {
        let paths = bundle.lay_out(&staging.join(at.to_string()), report)?;
        if let Some(index) = &bundle.maintenance {
            let mut actual = Vec::new();
            for pack in &index.packs {
                let path = paths
                    .get(&pack.path)
                    .ok_or_else(|| format!("Missing staged pack: {}", pack.path))?;
                actual.push(describe(
                    path,
                    &content_reference(&pack.digest),
                    deadline,
                    report,
                )?);
            }
            validate_bundle_maintenance(index, &actual).map_err(|error| {
                format!(
                    "维护声明无效 / Invalid maintenance declaration: {}: {error}",
                    bundle.file.display()
                )
            })?;
        }
        let zip_bundle_digest = if bundle.content_directories() {
            None
        } else {
            Some(crate::verify::sha256_file(&bundle.file).map_err(|error| {
                format!("Cannot hash bundle {}: {error}", bundle.file.display())
            })?)
        };
        prepared.push(Prepared {
            bundle: bundle.clone(),
            paths,
            zip_bundle_digest,
        });
    }
    Ok(prepared)
}

pub fn place(prepared: &[Prepared], root: &Path, report: Report<'_>) -> Result<(), String> {
    for item in prepared {
        item.bundle.lay_out(&package_dir(root, item), report)?;
    }
    report.line("已验证的新资源将保留；旧资源保留 / Verified new resources are retained; old resources are kept")
}

pub fn installed_path(root: &Path, item: &Prepared, pack: &str) -> Result<PathBuf, String> {
    let staged = item
        .paths
        .get(pack)
        .ok_or_else(|| format!("Missing staged pack: {pack}"))?;
    let name = staged
        .file_name()
        .ok_or_else(|| format!("Package path has no file name: {}", staged.display()))?;
    // Written into the configuration: never the canonical root's `\\?\` spelling.
    Ok(crate::generations::plain(&package_dir(root, item).join(name)))
}

fn package_dir(root: &Path, item: &Prepared) -> PathBuf {
    let dir = root.join("packages").join(&item.bundle.game);
    match &item.zip_bundle_digest {
        Some(hash) => dir.join("bundles").join(hash),
        None => dir,
    }
}

/// An association applies to an existing array position, preserving instance IDs,
/// backends and all fields that the person did not select for replacement.
pub struct Selection {
    pub instance: usize,
    pub bundle: usize,
    pub server: String,
}

pub fn upgrade_selections(
    document: &mut Value,
    root: &Path,
    prepared: &[Prepared],
    choose: Choose<'_>,
    report: Report<'_>,
) -> Result<Vec<Selection>, String> {
    if !prepared
        .iter()
        .any(|item| item.bundle.maintenance.is_some())
    {
        return Ok(Vec::new());
    }
    let instances = document["instances"]
        .as_array_mut()
        .ok_or("配置缺少 instances / Configuration has no instances array")?;
    let mut selected = Vec::new();
    for (at, instance) in instances.iter_mut().enumerate() {
        let identity =
            resource_descriptor(instance, root, Instant::now() + ADMISSION_BUDGET, report);
        let matches: Vec<_> = match &identity {
            Ok(actual) => prepared
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.bundle.game == actual.game()
                        && item
                            .bundle
                            .packs
                            .iter()
                            .any(|pack| pack.server == actual.server())
                })
                .map(|(bundle, _)| (bundle, actual.server().to_owned()))
                .collect(),
            Err(_) => Vec::new(),
        };
        if let [(bundle, server)] = matches.as_slice() {
            selected.push(Selection {
                instance: at,
                bundle: *bundle,
                server: server.clone(),
            });
            continue;
        }
        if identity.is_ok() && matches.is_empty() {
            // A verified game/server with no new bundle has no declaration to merge.
            continue;
        }
        if identity.is_ok() {
            let association = Association {
                alias: alias_of(instance, at),
                question: format!("实例 / Instance {}：多个标准包匹配已验证的游戏/服务器。请选择维护声明来源；业务资源保持。/ Multiple bundles match the verified game/server. Choose the maintenance source; the business resource stays unchanged.", instance["alias"]),
                options: matches
                    .iter()
                    .map(|(bundle, server)| AssociationOption {
                        label: format!(
                            "{} / {server} — {}",
                            prepared[*bundle].bundle.name(),
                            prepared[*bundle].bundle.file.display()
                        ),
                        bundle: prepared[*bundle].bundle.game.clone(),
                        server: server.clone(),
                    })
                    .collect(),
            };
            let Some(picked) = associate(&association, choose, report)? else {
                continue;
            };
            let (bundle, server) = matches.get(picked).ok_or("Invalid bundle association")?;
            selected.push(Selection {
                instance: at,
                bundle: *bundle,
                server: server.clone(),
            });
            continue;
        }
        let reason = match identity {
            Ok(actual) => format!(
                "已验证归属 / Verified identity: {}/{}; 匹配的标准包数量 / matching bundles: {}",
                actual.game(),
                actual.server(),
                matches.len()
            ),
            Err(reason) => {
                report.warn(&format!(
                    "资源归属未确认 / Resource identity unconfirmed: {reason}"
                ))?;
                reason
            }
        };
        let mut options = Vec::new();
        let mut targets = Vec::new();
        for (bundle, item) in prepared.iter().enumerate() {
            for (server, pack) in &item.bundle.defaults {
                options.push(AssociationOption {
                    label: format!(
                        "{} / {server} — {}",
                        item.bundle.name(),
                        item.bundle.file.display()
                    ),
                    bundle: item.bundle.game.clone(),
                    server: server.clone(),
                });
                targets.push((bundle, server.clone(), pack.clone()));
            }
        }
        if options.is_empty() {
            return Err(format!("{reason}\n没有可明确关联的标准包 / No bundle is available for an explicit association"));
        }
        let association = Association {
            alias: alias_of(instance, at),
            question: format!(
                "实例 / Instance {}\n{reason}\n请选择游戏/服务器。此选择将把该实例的 resource_package 替换为所选默认包；其它实例字段保留。/ Select its game/server. This replaces this instance's resource package with the selected default pack and preserves its other fields.\n原值 / Current: {}",
                instance["alias"], instance["resource_package"]
            ),
            options,
        };
        let Some(picked) = associate(&association, choose, report)? else {
            continue;
        };
        let (bundle, server, pack) = targets.get(picked).ok_or("Invalid resource selection")?;
        instance["resource_package"] = json!(installed_path(root, &prepared[*bundle], pack)?);
        selected.push(Selection {
            instance: at,
            bundle: *bundle,
            server: server.clone(),
        });
    }
    Ok(selected)
}

/// One maintenance binding whose configured value differs from the one the
/// release proposes. Nothing is chosen for it: the caller presents the whole
/// list once and `apply` writes the answers (Workflow #359).
pub struct Conflict {
    pub key: String,
    /// The binding as people read it.
    pub item: String,
    pub old: Value,
    pub old_source: String,
    pub new: Value,
    pub new_source: String,
    target: Target,
}

enum Target {
    Prerequisite(String),
    Startup(usize),
    ReturnHome(String, String),
}

impl Target {
    fn rank(&self) -> u8 {
        match self {
            Target::Prerequisite(_) => 0,
            Target::Startup(_) => 1,
            Target::ReturnHome(..) => 2,
        }
    }
}

/// Which value a conflicting binding keeps.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Old,
    New,
}

/// All conflicts at once, answered row by row in the same order.
pub type Resolve<'a> = &'a mut dyn FnMut(&[Conflict]) -> Result<Vec<Side>, String>;

impl Conflict {
    pub fn old_text(&self) -> String {
        brief(&self.old)
    }

    pub fn new_text(&self) -> String {
        brief(&self.new)
    }
}

/// A binding in one line: its plain path and the first twelve digits of its
/// digest, or its package id.
fn brief(value: &Value) -> String {
    let path = value
        .get("package")
        .or_else(|| value.get("package_path"))
        .and_then(Value::as_str);
    let digest = value
        .get("expected_sha256")
        .and_then(Value::as_str)
        .or_else(|| {
            value.get("package_digest").and_then(|digest| {
                digest
                    .as_str()
                    .or_else(|| digest.get("sha256").and_then(Value::as_str))
            })
        })
        .map(|digest| {
            digest
                .strip_prefix("sha256:")
                .unwrap_or(digest)
                .chars()
                .take(12)
                .collect::<String>()
        });
    let plain = |path: &str| {
        crate::generations::plain(Path::new(path))
            .display()
            .to_string()
    };
    match (path, digest) {
        (Some(path), Some(digest)) => format!("{} · sha256 {digest}", plain(path)),
        (Some(path), None) => plain(path),
        _ => match value.get("package_id").and_then(Value::as_str) {
            Some(id) => format!("package_id {id}"),
            None => value.to_string(),
        },
    }
}

/// An instance's alias, or its place when it has none.
pub fn alias_of(instance: &Value, at: usize) -> String {
    instance["alias"]
        .as_str()
        .map_or_else(|| format!("instances[{at}]"), str::to_owned)
}

/// What one run's release proposes, and which proposals differ from the
/// configuration.
struct Merge<'a> {
    root: &'a Path,
    old_source: &'a str,
    proposed: BTreeMap<String, (Value, String)>,
    conflicts: Vec<Conflict>,
}

impl Merge<'_> {
    /// An empty binding takes the proposal; an equal one stays as it is spelled
    /// (only a verbatim-prefixed path loses that prefix); a different one is a
    /// conflict and stays untouched until `apply`. Two bundles proposing
    /// different values for one binding is a release defect.
    fn propose(
        &mut self,
        slot: &mut Value,
        new: Value,
        key: String,
        item: String,
        target: Target,
        source: &str,
    ) -> Result<(), String> {
        if let Some((earlier, earlier_source)) = self.proposed.get(&key) {
            if normalized(earlier, self.root) == normalized(&new, self.root) {
                return Ok(());
            }
            return Err(format!(
                "两个标准包对同一维护绑定给出不同的值 / Two bundles propose different values for one maintenance binding: {key}: {earlier_source}; {source}"
            ));
        }
        self.proposed
            .insert(key.clone(), (new.clone(), source.to_owned()));
        if slot.is_null() {
            *slot = new;
            return Ok(());
        }
        if normalized(slot, self.root) == normalized(&new, self.root) {
            plain_paths(slot);
            return Ok(());
        }
        self.conflicts.push(Conflict {
            key,
            item,
            old: slot.clone(),
            old_source: self.old_source.to_owned(),
            new,
            new_source: source.to_owned(),
            target,
        });
        Ok(())
    }
}

/// Merge declarations in source order. Equal complete bindings are reused;
/// every different value is returned as a `Conflict`, sorted by kind and key,
/// for one decision over the whole list. `old_source` names the configuration
/// the current values come from.
pub fn augment(
    document: &mut Value,
    root: &Path,
    prepared: &[Prepared],
    selections: &[Selection],
    old_source: &str,
) -> Result<(bool, Vec<Conflict>), String> {
    let mut merge = Merge {
        root,
        old_source,
        proposed: BTreeMap::new(),
        conflicts: Vec::new(),
    };
    let mut registered = BTreeSet::new();
    let mut registered_home = BTreeSet::new();
    let mut active = false;
    for selected in selections {
        let item = &prepared[selected.bundle];
        let Some(index) = &item.bundle.maintenance else {
            continue;
        };
        active = true;
        let file = item
            .bundle
            .file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| item.bundle.file.display().to_string());
        let source = format!("{file} ({})", item.bundle.game);
        for entry in index
            .maintenance
            .iter()
            .filter(|entry| entry.server == selected.server)
        {
            let pack = index
                .packs
                .iter()
                .find(|pack| pack.package_id == entry.package_id)
                .ok_or_else(|| format!("Missing maintenance pack: {}", entry.package_id))?;
            let path = installed_path(root, item, &pack.path)?;
            for purpose in &entry.uses {
                match purpose {
                    MaintenanceUse::Startup => {
                        let key = format!("instances[{}].startup_package", selected.instance);
                        let alias =
                            alias_of(&document["instances"][selected.instance], selected.instance);
                        let value = json!({"package": path, "expected_sha256": pack.digest});
                        let slot = &mut document["instances"][selected.instance]["startup_package"];
                        merge.propose(
                            slot,
                            value,
                            key,
                            format!("实例 / Instance {alias} 启动包 / startup package"),
                            Target::Startup(selected.instance),
                            &source,
                        )?;
                    }
                    MaintenanceUse::Prerequisite | MaintenanceUse::ReturnHome => {
                        if registered.insert((selected.bundle, pack.package_id.clone())) {
                            let value = json!({"package_id": pack.package_id, "package_path": path,
                                "package_digest": content_reference(&pack.digest)});
                            let key = format!("prerequisite_packages[{}]", pack.package_id);
                            let slot = array_slot(
                                document,
                                "prerequisite_packages",
                                &[("package_id", &pack.package_id)],
                            )?;
                            merge.propose(
                                slot,
                                value,
                                key,
                                format!("前置包 / Prerequisite {}", pack.package_id),
                                Target::Prerequisite(pack.package_id.clone()),
                                &source,
                            )?;
                        }
                        if *purpose == MaintenanceUse::ReturnHome
                            && registered_home.insert((selected.bundle, entry.server.clone()))
                        {
                            let value = json!({"game": index.game, "server": entry.server, "package_id": pack.package_id});
                            let key =
                                format!("return_home_packages[{}/{}]", index.game, entry.server);
                            let slot = array_slot(
                                document,
                                "return_home_packages",
                                &[("game", &index.game), ("server", &entry.server)],
                            )?;
                            merge.propose(
                                slot,
                                value,
                                key,
                                format!("回主页包 / Return-home {}/{}", index.game, entry.server),
                                Target::ReturnHome(index.game.clone(), entry.server.clone()),
                                &source,
                            )?;
                        }
                    }
                }
            }
        }
    }
    let mut conflicts = merge.conflicts;
    conflicts.sort_by(|left, right| {
        (left.target.rank(), &left.key).cmp(&(right.target.rank(), &right.key))
    });
    Ok((active, conflicts))
}

/// Counts by kind, and what is not a difference.
pub fn summary(conflicts: &[Conflict]) -> String {
    let count = |rank: u8| {
        conflicts
            .iter()
            .filter(|conflict| conflict.target.rank() == rank)
            .count()
    };
    format!(
        "前置包 / prerequisite {} · 启动包 / startup {} · 回主页包 / return-home {}；仅路径写法不同（带不带 \\\\?\\ 前缀、分隔符或大小写）不算差异，不列出 / a difference in path spelling alone (the \\\\?\\ prefix, separators or case) is not a difference and is not listed",
        count(0),
        count(1),
        count(2)
    )
}

/// Every conflict into the log (and the console): item, both values with
/// their sources, and both values whole.
pub fn list(conflicts: &[Conflict], report: Report<'_>) -> Result<(), String> {
    if conflicts.is_empty() {
        return report.line("维护绑定没有差异 / No maintenance binding differs");
    }
    report.line(&format!(
        "维护绑定差异 / Maintenance binding differences: {}（{}）",
        conflicts.len(),
        summary(conflicts)
    ))?;
    for (at, conflict) in conflicts.iter().enumerate() {
        report.line(&format!(
            "  {}. {} [{}]\n     旧值 / current ({}): {}\n     新值 / proposed ({}): {}\n     旧值全文 / current value: {}\n     新值全文 / proposed value: {}",
            at + 1,
            conflict.item,
            conflict.key,
            conflict.old_source,
            conflict.old_text(),
            conflict.new_source,
            conflict.new_text(),
            conflict.old,
            conflict.new
        ))?;
    }
    Ok(())
}

/// Writes each answer: `New` replaces the binding, `Old` keeps it (a
/// verbatim-prefixed path loses that prefix).
pub fn apply(document: &mut Value, conflicts: &[Conflict], sides: &[Side]) -> Result<(), String> {
    if sides.len() != conflicts.len() {
        return Err("维护绑定的答复与差异数目不符 / The answers do not match the maintenance binding differences".into());
    }
    for (conflict, side) in conflicts.iter().zip(sides) {
        let slot = match &conflict.target {
            Target::Startup(at) => &mut document["instances"][*at]["startup_package"],
            Target::Prerequisite(id) => {
                array_slot(document, "prerequisite_packages", &[("package_id", id.as_str())])?
            }
            Target::ReturnHome(game, server) => array_slot(
                document,
                "return_home_packages",
                &[("game", game.as_str()), ("server", server.as_str())],
            )?,
        };
        match side {
            Side::New => *slot = conflict.new.clone(),
            Side::Old => plain_paths(slot),
        }
    }
    Ok(())
}

/// The whole list logged, one decision over it, the answers logged and written.
pub fn decide(
    document: &mut Value,
    conflicts: &[Conflict],
    resolve: Resolve<'_>,
    report: Report<'_>,
) -> Result<(), String> {
    list(conflicts, report)?;
    if conflicts.is_empty() {
        return Ok(());
    }
    let sides = resolve(conflicts)?;
    let how = if sides.iter().all(|side| *side == Side::New) {
        "全部采用新值 / all proposed"
    } else if sides.iter().all(|side| *side == Side::Old) {
        "全部保留旧值 / all current"
    } else {
        "逐项决定 / decided each"
    };
    report.line(&format!("维护绑定的处理 / Resolution: {how}"))?;
    for (conflict, side) in conflicts.iter().zip(&sides) {
        report.line(&format!(
            "  {} → {}",
            conflict.key,
            match side {
                Side::New => "新值 / proposed",
                Side::Old => "旧值 / current",
            }
        ))?;
    }
    apply(document, conflicts, &sides)
}

fn array_slot<'a>(
    document: &'a mut Value,
    field: &str,
    keys: &[(&str, &str)],
) -> Result<&'a mut Value, String> {
    if document[field].is_null() {
        document[field] = json!([]);
    }
    let array = document[field]
        .as_array_mut()
        .ok_or_else(|| format!("{field} must be an array"))?;
    let found: Vec<_> = array
        .iter()
        .enumerate()
        .filter(|(_, value)| {
            keys.iter()
                .all(|(key, wanted)| value[*key].as_str() == Some(*wanted))
        })
        .map(|(at, _)| at)
        .collect();
    let at = match found.as_slice() {
        [] => {
            array.push(Value::Null);
            array.len() - 1
        }
        [at] => *at,
        _ => {
            return Err(format!(
                "配置存在重复绑定 / Duplicate configuration binding: {field} {keys:?}"
            ))
        }
    };
    Ok(&mut array[at])
}

/// One location however it is spelled: with or without the `\\?\` prefix, with
/// either separator, and in any ASCII case (NTFS compares names that way).
fn comparable(path: &Path) -> String {
    crate::generations::plain(path)
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// A kept value's own path fields lose the `\\?\` prefix; nothing else changes.
fn plain_paths(value: &mut Value) {
    for key in ["package", "package_path"] {
        let plain = value
            .get(key)
            .and_then(Value::as_str)
            .filter(|path| path.starts_with(r"\\?\"))
            .map(|path| {
                crate::generations::plain(Path::new(path))
                    .to_string_lossy()
                    .into_owned()
            });
        if let Some(plain) = plain {
            value[key] = json!(plain);
        }
    }
}

fn normalized(value: &Value, root: &Path) -> Value {
    let mut value = value.clone();
    for key in ["package", "package_path"] {
        if let Some(path) = value[key].as_str() {
            value[key] = json!(comparable(&resolve(root, path)));
        }
    }
    if let Some(hash) = value["package_digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"))
    {
        value["package_digest"] = json!(hash);
    }
    value
}

fn resolve(root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn digest_named(path: &Path) -> Option<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.len() == 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn resource_descriptor(
    instance: &Value,
    root: &Path,
    deadline: Instant,
    report: Report<'_>,
) -> Result<TaskPackageDescriptor, String> {
    let path = resolve(
        root,
        instance["resource_package"]
            .as_str()
            .ok_or("resource_package is missing")?,
    );
    let reference = if let Some(hash) = digest_named(&path) {
        content_reference(hash)
    } else if path.is_file() {
        PackageRef::LegacyZipSha256(
            crate::verify::sha256_file(&path)
                .map_err(|error| format!("Cannot hash {}: {error}", path.display()))?,
        )
    } else {
        return Err(format!(
            "资源没有可验证引用 / Resource has no verifiable reference: {}",
            path.display()
        ));
    };
    describe(&path, &reference, deadline, report)
}

#[derive(serde::Deserialize)]
struct BindingReference {
    #[serde(with = "actingcommand_contract::package::prefixed_reference")]
    package_digest: PackageRef,
}

/// Inspect the actual final candidate, including retained handwritten bindings.
/// check-config does not replace this full shared chain qualification.
pub fn validate(document: &Value, root: &Path, report: Report<'_>) -> Result<(), String> {
    let deadline = Instant::now() + ADMISSION_BUDGET;
    let mut bindings = BTreeMap::new();
    let mut roots = Vec::new();
    for value in array(document, "prerequisite_packages")? {
        let id = value["package_id"]
            .as_str()
            .ok_or("prerequisite package_id missing")?;
        let path = resolve(
            root,
            value["package_path"]
                .as_str()
                .ok_or("prerequisite package_path missing")?,
        );
        let reference: BindingReference = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid prerequisite reference: {error}"))?;
        let actual = describe(&path, &reference.package_digest, deadline, report)?;
        if actual.package_id() != id {
            return Err(format!(
                "Prerequisite identity mismatch: {id} / {}",
                actual.package_id()
            ));
        }
        if let Some(reason) = actual.use_incompatibility(MaintenanceUse::Prerequisite) {
            return Err(format!("Prerequisite {id}: {reason}"));
        }
        if bindings.insert(id.to_owned(), actual.clone()).is_some() {
            return Err(format!("Duplicate prerequisite: {id}"));
        }
        roots.push(actual);
    }
    let mut home = BTreeMap::new();
    for value in array(document, "return_home_packages")? {
        let game = value["game"].as_str().ok_or("return-home game missing")?;
        let server = value["server"]
            .as_str()
            .ok_or("return-home server missing")?;
        let id = value["package_id"]
            .as_str()
            .ok_or("return-home package_id missing")?;
        let actual = bindings
            .get(id)
            .ok_or_else(|| format!("Return-home package unbound: {id}"))?;
        if let Some(reason) =
            actual.prerequisite_incompatibility_with(game, server, actual.resolution(), true)
        {
            return Err(format!("Return-home {id}: {reason}"));
        }
        if let Some(reason) = actual.use_incompatibility(MaintenanceUse::ReturnHome) {
            return Err(format!("Return-home {id}: {reason}"));
        }
        if home
            .insert((game.to_owned(), server.to_owned()), id.to_owned())
            .is_some()
        {
            return Err(format!("Duplicate return-home: {game}/{server}"));
        }
    }
    for instance in array(document, "instances")? {
        roots.push(resource_descriptor(instance, root, deadline, report)?);
        if let Some(startup) = instance
            .get("startup_package")
            .filter(|value| !value.is_null())
        {
            let path = resolve(
                root,
                startup["package"]
                    .as_str()
                    .ok_or("startup package missing")?,
            );
            let hash = startup["expected_sha256"]
                .as_str()
                .ok_or("startup hash missing")?;
            let reference = if digest_named(&path) == Some(hash) {
                content_reference(hash)
            } else {
                PackageRef::LegacyZipSha256(hash.to_owned())
            };
            let actual = describe(&path, &reference, deadline, report)?;
            if let Some(reason) = actual.use_incompatibility(MaintenanceUse::Startup) {
                return Err(format!("Startup: {reason}"));
            }
            roots.push(actual);
        }
    }
    for root in roots {
        if let Some(id) = home.get(&(root.game().to_owned(), root.server().to_owned())) {
            if let Some(reason) = bindings[id].prerequisite_incompatibility_with(
                root.game(),
                root.server(),
                root.resolution(),
                true,
            ) {
                return Err(format!("Return-home {id}: {reason}"));
            }
        }
        let mut chain = PrerequisiteChain::new(root);
        loop {
            let dependent = chain.dependent();
            let fallback = home
                .get(&(dependent.game().to_owned(), dependent.server().to_owned()))
                .map(String::as_str);
            let Some(link) = chain.next(fallback) else {
                break;
            };
            chain
                .begin(&link, bindings.contains_key(&link.package_id))
                .map_err(|error| error.to_string())?;
            chain
                .admit(&link, bindings[&link.package_id].clone())
                .map_err(|error| error.to_string())?;
        }
        chain.finish().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn array<'a>(document: &'a Value, field: &str) -> Result<&'a [Value], String> {
    match document.get(field) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(array)) => Ok(array),
        _ => Err(format!("{field} must be an array")),
    }
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(READ_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > READ_LIMIT {
        return Err(format!("Configuration exceeds 16 MiB: {}", path.display()));
    }
    Ok(bytes)
}

/// Exact baseline bytes stay in memory until an adjacent, checked candidate is
/// atomically renamed. The backup survives both success and rollback.
pub struct Transaction {
    config: PathBuf,
    baseline: Vec<u8>,
    pub document: Value,
    committed: Option<Vec<u8>>,
    installation: Option<acui_installation::Snapshot>,
    selection_plan: Option<crate::generations::Plan>,
}

impl Transaction {
    pub fn read(root: &Path) -> Result<Self, String> {
        if root.join(acui_installation::INSTALL_SELECTION_PATH).try_exists()
            .map_err(|error| format!("Cannot inspect installation selection: {error}"))? {
            let snapshot = acui_installation::Snapshot::read(root)?;
            let mut transaction = Self::read_config(&snapshot.config_path()?)?;
            transaction.installation = Some(snapshot);
            return Ok(transaction);
        }
        Self::read_config(&root.join("actingd.config.json"))
    }

    pub fn read_config(config: &Path) -> Result<Self, String> {
        let baseline = read(config)?;
        let document: Value = serde_json::from_slice(&baseline)
            .map_err(|error| format!("Configuration unreadable: {error}"))?;
        if !document.is_object() {
            return Err("Configuration must be an object".into());
        }
        Ok(Self {
            config: config.to_path_buf(),
            baseline,
            document,
            committed: None,
            installation: None,
            selection_plan: None,
        })
    }

    /// The configuration file this plan was read from.
    pub fn config_path(&self) -> &Path {
        &self.config
    }

    pub fn unchanged(&self) -> Result<(), String> {
        if read(&self.config)? == self.baseline {
            Ok(())
        } else {
            Err("配置在计划后被修改，请重新规划 / Configuration changed after planning; start again".into())
        }
    }

    pub fn state_root(&self) -> Result<PathBuf, String> {
        let root = PathBuf::from(
            self.document["state_root"]
                .as_str()
                .ok_or("Configuration has no state_root")?,
        );
        if root.is_absolute() {
            Ok(root)
        } else {
            Err("Configuration state_root must be absolute".into())
        }
    }

    pub fn commit(
        &mut self,
        actingd: &Path,
        qualify: bool,
        report: Report<'_>,
    ) -> Result<(), String> {
        if let Some(snapshot) = &self.installation {
            self.unchanged()?;
            let writer = crate::generations::Writer::acquire(&snapshot.root)?;
            let mut plan = writer.prepare(
                Some(snapshot.clone()), snapshot.selection.slot, &self.config,
                self.document.clone(), qualify, report,
            )?;
            writer.commit(&mut plan)?;
            self.committed = Some(plan.snapshot.config_bytes.clone());
            self.selection_plan = Some(plan);
            return report.line("私有配置代际已提交 / Private configuration generation committed");
        }
        let root = self.config.parent().ok_or("Configuration has no parent")?;
        report.step(Step::Phase(
            "检查并提交配置 / Checking and committing configuration",
            None,
        ))?;
        if qualify {
            validate(&self.document, root, report)?;
        }
        let mut bytes =
            serde_json::to_vec_pretty(&self.document).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        if bytes.len() as u64 > READ_LIMIT {
            return Err("Candidate configuration exceeds 16 MiB".into());
        }
        let candidate = self.config.with_file_name(format!(
            "actingd.config.candidate-{}-{}.json",
            std::process::id(),
            crate::log::unix_ms()
        ));
        write_new(&candidate, &bytes)?;
        if let Err(reason) = crate::runtime::check_config(actingd, &candidate) {
            return Err(crate::fetch::discard(
                &candidate,
                format!("{reason}\n配置未改动 / Configuration unchanged"),
            ));
        }
        let result = (|| {
            if read(&self.config)? != self.baseline {
                return Err("配置在计划后被修改，请重新规划 / Configuration changed after planning; start again".into());
            }
            let original: Value =
                serde_json::from_slice(&self.baseline).map_err(|error| error.to_string())?;
            if original == self.document {
                fs::remove_file(&candidate)
                    .map_err(|error| format!("Cannot remove unchanged candidate: {error}"))?;
                return report
                    .line("配置相同，复用原文件 / Configuration unchanged; original file reused");
            }
            let backup = self.config.with_file_name(format!(
                "actingd.config.backup-{}-{}.json",
                std::process::id(),
                crate::log::unix_ms()
            ));
            write_new(&backup, &self.baseline)?;
            report.line(&format!(
                "原配置已备份 / Original configuration backed up: {}",
                backup.display()
            ))?;
            if read(&self.config)? != self.baseline {
                return Err("配置在提交前被修改 / Configuration changed before commit".into());
            }
            fs::rename(&candidate, &self.config)
                .map_err(|error| format!("Atomic configuration replacement failed: {error}"))?;
            self.committed = Some(bytes);
            report.line("配置已提交 / Configuration committed")
        })();
        result.map_err(|reason| {
            if self.committed.is_some() {
                format!("{reason}\n配置已提交 / Configuration was committed")
            } else {
                crate::fetch::discard(&candidate, reason)
            }
        })
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if let Some(plan) = &mut self.selection_plan {
            let writer = crate::generations::Writer::acquire(&plan.snapshot.root)?;
            writer.restore_before_start(plan)?;
            self.committed = None;
            return Ok(());
        }
        let Some(committed) = &self.committed else {
            return if read(&self.config)? == self.baseline {
                Ok(())
            } else {
                Err(
                    "原配置已被其它写者修改 / Original configuration changed by another writer"
                        .into(),
                )
            };
        };
        if read(&self.config)? != *committed {
            return Err("配置提交后被修改，无法自动恢复 / Configuration changed after commit; automatic restore refused".into());
        }
        let candidate = self.config.with_file_name(format!(
            "actingd.config.restore-{}-{}.json",
            std::process::id(),
            crate::log::unix_ms()
        ));
        write_new(&candidate, &self.baseline)?;
        if read(&self.config)? != *committed {
            return Err(crate::fetch::discard(
                &candidate,
                "配置恢复前被修改 / Configuration changed before restore".into(),
            ));
        }
        fs::rename(&candidate, &self.config)
            .map_err(|error| format!("Configuration restore failed: {error}"))?;
        if read(&self.config)? != self.baseline {
            return Err("Restored configuration bytes differ".into());
        }
        self.committed = None;
        Ok(())
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Cannot write {}: {error}", path.display()))
}
