// SPDX-License-Identifier: GPL-3.0-only
//! The instances step's resources: resource repositories' bundles — each one
//! zip holding `applications.json` (the game, its display name when given, and
//! each server's Android package name), `bundle.json` and the packs under
//! `packs/`. Two bundle formats are read, told apart by `bundle.json`'s
//! `schema_version`:
//! - `actingcommand.bundle.v1` lists every sealed pack's path, package id,
//!   server, sha256 and size, and each server's default pack. Each pack zip is
//!   laid out under `<root>\packages\<game>\` byte for byte, checked against
//!   `bundle.json` first.
//! - `actingcommand.bundle.v2` (the contract's `BundleIndexV2`, Workflow #288)
//!   maps each package id to a content directory `packs/<digest>/`, named by
//!   its `content-directory.v1` digest; a server's default package is the one
//!   `applications.json` names (`servers.<server>.default_package_id`). Each
//!   is laid out as `<root>\packages\<game>\<digest>\` only once what was
//!   written has that digest; one already there is reused when it verifies,
//!   and otherwise set aside as `<digest>.broken-<unix>` and kept.
//!
//! The release carries them: its `MEMBERS.json`
//! names each in `bundles[]` with its sha256, and the file sits beside it in
//! the download folder, already checked against `SHA256SUMS`. A local bundle
//! file may be added when the release carries none; no hash is asked of the
//! person. The Runtime only ever sees one pack per instance, a local file
//! (v1) or a digest-named directory (v2). The wizard knows no game: the
//! display names and package names come from the bundles. Nothing here ever
//! deletes an installed pack, file or directory, nor writes either declaration
//! into the install root.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use actingcommand_contract::{content_directory_digest, safe_source_path, BundleIndexV2};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::fetch;
use crate::verify::{hex, Report, Step, Total, MISMATCH};

const APPLICATIONS_SCHEMA: &str = "actingcommand.applications.v1";
const BUNDLE_SCHEMA: &str = "actingcommand.bundle.v1";
/// The wire spelling of the contract's `BundleIndexVersion::V2`.
const BUNDLE_SCHEMA_V2: &str = "actingcommand.bundle.v2";
/// The most a bundle's two declarations may inflate to.
const DECLARATION_LIMIT: u64 = 16 << 20;

#[derive(Clone)]
pub struct Bundle {
    pub file: PathBuf,
    pub game: String,
    /// The game's name for people, when `applications.json` gives one.
    pub label: Option<String>,
    pub packs: Vec<Pack>,
    /// Each server's Android package name.
    pub applications: BTreeMap<String, Application>,
    /// A server's default pack, by its path, when the bundle names one.
    pub defaults: BTreeMap<String, String>,
    /// Carried by the release, or a local file the person added.
    pub carried: bool,
    /// Bundle v2: the packs are content directories, not sealed zips.
    directories: bool,
}

#[derive(Deserialize, Clone)]
pub struct Pack {
    /// v1 `packs/<name>.zip`; v2 `packs/<digest>`.
    pub path: String,
    pub package_id: String,
    pub server: String,
    /// v1 the sealed zip's sha256; v2 the content directory's digest.
    pub sha256: String,
    pub byte_count: u64,
    /// v2: the files the content directory holds; v1 does not state it.
    #[serde(skip)]
    pub file_count: u64,
}

#[derive(Deserialize, Clone)]
pub struct Application {
    pub application_id: String,
    pub label: String,
    /// The server's default package. Read for bundle v2 only: a v1 bundle
    /// names its default packs in `bundle.json`.
    #[serde(default)]
    default_package_id: Option<String>,
}

/// `bundle.json`'s format version, read before the rest of it.
#[derive(Deserialize)]
struct Versioned {
    schema_version: String,
}

#[derive(Deserialize)]
struct BundleFile {
    schema_version: String,
    game: String,
    packs: Vec<Pack>,
    #[serde(default)]
    default_packs: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ApplicationsFile {
    schema_version: String,
    game: String,
    #[serde(default)]
    label: Option<String>,
    servers: BTreeMap<String, Application>,
}

/// What `MEMBERS.json` says of the bundles its release carries.
#[derive(Deserialize)]
struct Members {
    #[serde(default)]
    bundles: Vec<Carried>,
}

#[derive(Deserialize)]
struct Carried {
    asset: String,
    sha256: String,
}

/// The bundles the release in `download` carries, in the order its
/// `MEMBERS.json` names them — each listed in `SHA256SUMS` under the same
/// sha256, present beside it with that sha256, and readable as a bundle —
/// and, one line each, the ones that are not: those are said, the rest still
/// offered. A release that names none carries none; a `MEMBERS.json` or
/// `SHA256SUMS` that cannot be read fails the lot.
pub fn carried(download: &Path, report: Report<'_>) -> Result<(Vec<Bundle>, Vec<String>), String> {
    let path = download.join("MEMBERS.json");
    let text = fs::read_to_string(&path).map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
    let members: Members = serde_json::from_str(&text)
        .map_err(|error| format!("MEMBERS.json 的 bundles 无法解析 / MEMBERS.json bundles do not parse: {error}"))?;
    if members.bundles.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let sums_path = download.join("SHA256SUMS");
    let sums = fs::read_to_string(&sums_path)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", sums_path.display()))?;
    let listed: BTreeMap<String, String> = crate::verify::parse_sha256sums(&sums)?
        .into_iter()
        .map(|(sha, name)| (name, sha))
        .collect();
    let (mut bundles, mut problems): (Vec<Bundle>, Vec<String>) = (Vec::new(), Vec::new());
    for entry in members.bundles {
        let one = (|| {
            if !fetch::plain(&entry.asset) {
                return Err("文件名不能使用 / the name cannot be used".to_string());
            }
            match listed.get(&entry.asset) {
                Some(sha) if sha.eq_ignore_ascii_case(&entry.sha256) => {}
                Some(sha) => {
                    return Err(format!(
                        "{MISMATCH}: SHA256SUMS 记为 / lists {sha}，MEMBERS.json 记为 / names {}",
                        entry.sha256
                    ))
                }
                None => return Err("SHA256SUMS 未列出 / not listed in SHA256SUMS".to_string()),
            }
            let file = download.join(&entry.asset);
            let actual = crate::verify::sha256_file(&file)
                .map_err(|error| format!("读取失败 / read failed: {}: {error}", file.display()))?;
            if !entry.sha256.eq_ignore_ascii_case(&actual) {
                return Err(format!("{MISMATCH}: sha256 应为 / expected {}，实为 / actual {actual}", entry.sha256));
            }
            let mut bundle = read(&file)?;
            bundle.carried = true;
            // `packages\<game>\` is one folder whatever the case.
            if let Some(twin) = bundles.iter().find(|other| other.game.eq_ignore_ascii_case(&bundle.game)) {
                return Err(format!("与 / the same game as {} 同一游戏 {}", twin.file.display(), bundle.game));
            }
            Ok(bundle)
        })();
        match one {
            Ok(bundle) => {
                report.line(&format!("发布件自带的标准包 / carried bundle: {} → {}", entry.asset, bundle.name()))?;
                bundles.push(bundle);
            }
            Err(reason) => problems.push(format!("{}: {reason}", entry.asset)),
        }
    }
    Ok((bundles, problems))
}

/// A local bundle file the person added: the absolute path of an existing
/// file, read as a bundle.
pub fn local(given: &str) -> Result<Bundle, String> {
    let path = PathBuf::from(given);
    if !path.is_absolute() || !path.is_file() {
        return Err(format!("标准包须为存在的文件的绝对路径 / the bundle must be the absolute path of an existing file: {given}"));
    }
    read(&path)
}

/// A zip read as a bundle: both declarations present, their formats known,
/// naming the same game, every pack name usable and unique, every default pack
/// listed.
pub fn read(file: &Path) -> Result<Bundle, String> {
    let mut archive = open(file)?;
    let index = match member(&mut archive, file, "bundle.json")? {
        Some(bytes) => bytes,
        None => {
            return Err(format!(
                "不是资源仓标准包（根目录没有 bundle.json）/ Not a resource repository bundle (no bundle.json at its root): {}",
                file.display()
            ))
        }
    };
    if parse::<Versioned>(&index, file, "bundle.json")?.schema_version == BUNDLE_SCHEMA_V2 {
        return read_v2(&mut archive, file, &index);
    }
    let bundle: BundleFile = parse(&index, file, "bundle.json")?;
    let applications = read_applications(&mut archive, file)?;
    if bundle.schema_version != BUNDLE_SCHEMA || applications.schema_version != APPLICATIONS_SCHEMA {
        return Err(format!(
            "标准包的格式版本无法识别 / unknown bundle format: {} · {}（应为 / expected {BUNDLE_SCHEMA} 或 / or {BUNDLE_SCHEMA_V2} · {APPLICATIONS_SCHEMA}）",
            bundle.schema_version, applications.schema_version
        ));
    }
    if bundle.game != applications.game {
        return Err(format!(
            "标准包里两份声明的 game 不一致 / the bundle's two declarations name different games: {} ≠ {}",
            bundle.game, applications.game
        ));
    }
    if !fetch::plain(&bundle.game) {
        return Err(format!("game 不能用作目录名 / the game cannot name a folder: {}", bundle.game));
    }
    // Names that differ only in case are one file on NTFS.
    let mut names = BTreeSet::new();
    for pack in &bundle.packs {
        if !names.insert(pack_name(pack)?.to_ascii_lowercase()) {
            return Err(format!("标准包里有重名的包 / the bundle lists a pack name twice: {}", pack.path));
        }
    }
    for (server, path) in &bundle.default_packs {
        if !bundle.packs.iter().any(|pack| &pack.path == path && &pack.server == server) {
            return Err(format!("默认包不在包列表里 / a default pack is not listed: {server} → {path}"));
        }
    }
    Ok(Bundle {
        file: file.to_path_buf(),
        game: bundle.game,
        label: applications.label.map(|label| label.trim().to_string()).filter(|label| !label.is_empty()),
        packs: bundle.packs,
        applications: applications.servers,
        defaults: bundle.default_packs,
        carried: false,
        directories: false,
    })
}

/// `applications.json`, which both formats carry.
fn read_applications(archive: &mut zip::ZipArchive<File>, file: &Path) -> Result<ApplicationsFile, String> {
    match member(archive, file, "applications.json")? {
        Some(bytes) => parse(&bytes, file, "applications.json"),
        None => Err(format!("标准包缺少 applications.json / the bundle lacks applications.json: {}", file.display())),
    }
}

/// A bundle v2: the index as the contract decodes and validates it (packs
/// present, package ids and digests unique, every path `packs/<digest>`), the
/// same game in both declarations, and each server's `default_package_id`,
/// where one is given, naming a pack of that server.
fn read_v2(archive: &mut zip::ZipArchive<File>, file: &Path, index: &[u8]) -> Result<Bundle, String> {
    let index: BundleIndexV2 = parse(index, file, "bundle.json")?;
    index.validate().map_err(|error| {
        format!("{} 内 bundle.json 无效 / invalid bundle.json: {}", file.display(), error.code())
    })?;
    let applications = read_applications(archive, file)?;
    if applications.schema_version != APPLICATIONS_SCHEMA {
        return Err(format!(
            "标准包的格式版本无法识别 / unknown bundle format: {BUNDLE_SCHEMA_V2} · {}（应为 / expected {APPLICATIONS_SCHEMA}）",
            applications.schema_version
        ));
    }
    if index.game != applications.game {
        return Err(format!(
            "标准包里两份声明的 game 不一致 / the bundle's two declarations name different games: {} ≠ {}",
            index.game, applications.game
        ));
    }
    if !fetch::plain(&index.game) {
        return Err(format!("game 不能用作目录名 / the game cannot name a folder: {}", index.game));
    }
    let packs: Vec<Pack> = index
        .packs
        .into_iter()
        .map(|pack| Pack {
            path: pack.path,
            package_id: pack.package_id,
            server: pack.server,
            sha256: pack.digest,
            byte_count: pack.byte_count,
            file_count: pack.file_count,
        })
        .collect();
    let mut defaults = BTreeMap::new();
    for (server, application) in &applications.servers {
        let Some(package_id) = &application.default_package_id else {
            continue;
        };
        let pack = packs
            .iter()
            .find(|pack| &pack.package_id == package_id && &pack.server == server)
            .ok_or_else(|| format!("默认包不在包列表里 / a default pack is not listed: {server} → {package_id}"))?;
        defaults.insert(server.clone(), pack.path.clone());
    }
    Ok(Bundle {
        file: file.to_path_buf(),
        game: index.game,
        label: applications.label.map(|label| label.trim().to_string()).filter(|label| !label.is_empty()),
        packs,
        applications: applications.servers,
        defaults,
        carried: false,
        directories: true,
    })
}

impl Bundle {
    /// The game as people know it: the bundle's display name, else its id.
    pub fn name(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.game.clone())
    }

    /// Every pack laid out under `dir` byte for byte, once its size and sha256
    /// match `bundle.json`; returns where each landed, by its path in the zip.
    /// A bundle v2 lays out content directories instead (`lay_out_directories`).
    pub fn lay_out(&self, dir: &Path, report: Report<'_>) -> Result<BTreeMap<String, PathBuf>, String> {
        if self.directories {
            return self.lay_out_directories(dir, report);
        }
        let mut archive = open(&self.file)?;
        fs::create_dir_all(dir).map_err(|error| format!("无法创建 / cannot create {}: {error}", dir.display()))?;
        report.step(Step::Phase(
            "放置任务包 / Placing the resource packs",
            Some(Total::Items(self.packs.len() as u64)),
        ))?;
        let mut laid = BTreeMap::new();
        for (done, pack) in self.packs.iter().enumerate() {
            let mut entry = match archive.by_name(&pack.path) {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => {
                    return Err(format!("{MISMATCH}: 标准包里缺少 / the bundle lacks {}", pack.path))
                }
                Err(error) => return Err(format!("{MISMATCH}: {} 读不出 / unreadable: {error}", pack.path)),
            };
            if entry.size() != pack.byte_count {
                return Err(format!(
                    "{MISMATCH}: {} 应为 / expected {} 字节 / bytes，实为 / actual {}",
                    pack.path,
                    pack.byte_count,
                    entry.size()
                ));
            }
            let target = dir.join(pack_name(pack)?);
            let part = target.with_extension("zip.part");
            // Streamed through a hash, never more than one byte past the size
            // declared, and renamed only once size and sha256 both match.
            let written = (|| {
                let cannot = |error: std::io::Error| format!("无法写入 / cannot write {}: {error}", part.display());
                let mut file = File::create(&part).map_err(cannot)?;
                let mut limited = (&mut entry).take(pack.byte_count + 1);
                let (mut hasher, mut buffer, mut count) = (Sha256::new(), vec![0u8; 64 * 1024], 0u64);
                loop {
                    let read = limited
                        .read(&mut buffer)
                        .map_err(|error| format!("{MISMATCH}: {} 解压失败 / inflate failed: {error}", pack.path))?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                    file.write_all(&buffer[..read]).map_err(cannot)?;
                    count += read as u64;
                }
                file.sync_all().map_err(cannot)?;
                let actual = hex(&hasher.finalize());
                if count != pack.byte_count || !pack.sha256.eq_ignore_ascii_case(&actual) {
                    return Err(format!(
                        "{MISMATCH}: {} 应为 / expected {} 字节 sha256 {}，实为 / actual {count} 字节 sha256 {actual}",
                        pack.path, pack.byte_count, pack.sha256
                    ));
                }
                drop(file);
                fs::rename(&part, &target)
                    .map_err(|error| format!("无法改名 / cannot rename {}: {error}", part.display()))
            })();
            if let Err(reason) = written {
                return Err(fetch::discard(&part, reason));
            }
            report.line(&format!("已放置 / placed: {}", target.display()))?;
            report.step(Step::Done(done as u64 + 1))?;
            laid.insert(pack.path.clone(), target);
        }
        Ok(laid)
    }

    /// Bundle v2: every pack's content directory laid out as `dir\<digest>\`;
    /// returns where each landed, by its path in the zip. A directory already
    /// there is read whole and reused when its digest is the pack's; one that
    /// differs is set aside as `<digest>.broken-<unix>`, kept and said, and the
    /// pack is unpacked again. Each is unpacked into `<digest>.part\` and
    /// renamed only once what was written has the pack's digest; a failed one
    /// is set aside the same way and said. Other directories and zips under
    /// `dir`, older packs among them, are left as they are.
    fn lay_out_directories(&self, dir: &Path, report: Report<'_>) -> Result<BTreeMap<String, PathBuf>, String> {
        let mut archive = open(&self.file)?;
        fs::create_dir_all(dir).map_err(|error| format!("无法创建 / cannot create {}: {error}", dir.display()))?;
        report.step(Step::Phase(
            "放置任务包 / Placing the resource packs",
            Some(Total::Items(self.packs.len() as u64)),
        ))?;
        let mut laid = BTreeMap::new();
        for (done, pack) in self.packs.iter().enumerate() {
            let target = dir.join(&pack.sha256);
            match present(&target, &pack.sha256)? {
                Present::Absent => {}
                Present::Equal => {
                    report.line(&format!(
                        "已存在且与摘要一致，复用 / already there and matching its digest, reused: {}",
                        target.display()
                    ))?;
                    report.step(Step::Done(done as u64 + 1))?;
                    laid.insert(pack.path.clone(), target);
                    continue;
                }
                Present::Differs(why) => {
                    let kept = set_aside(&target, &pack.sha256)?;
                    report.warn(&format!(
                        "{MISMATCH}: {}（{why}）已改名保留为 / set aside and kept as {}；重新解包 / unpacking it again",
                        target.display(),
                        kept.display()
                    ))?;
                }
            }
            let part = dir.join(format!("{}.part", pack.sha256));
            if fs::symlink_metadata(&part).is_ok() {
                let kept = set_aside(&part, &pack.sha256)?;
                report.warn(&format!(
                    "上次未完成的解包 / an unfinished unpack from an earlier run: {} 已改名保留为 / set aside and kept as {}",
                    part.display(),
                    kept.display()
                ))?;
            }
            if let Err(reason) = unpack(&mut archive, pack, &part) {
                return Err(keep_unfinished(&part, &pack.sha256, reason));
            }
            fs::rename(&part, &target).map_err(|error| {
                format!("无法改名 / cannot rename {} → {}: {error}", part.display(), target.display())
            })?;
            report.line(&format!("已放置 / placed: {}", target.display()))?;
            report.step(Step::Done(done as u64 + 1))?;
            laid.insert(pack.path.clone(), target);
        }
        Ok(laid)
    }
}

/// `packs/<digest>/**` of one v2 pack streamed into `part`, a new directory:
/// each file written once and hashed as it is written, the files and bytes
/// held to the index's counts, then the `content-directory.v1` digest of what
/// was written compared with the pack's. Directory entries name no file, and
/// an empty directory is no part of a pack.
fn unpack(archive: &mut zip::ZipArchive<File>, pack: &Pack, part: &Path) -> Result<(), String> {
    fs::create_dir(part).map_err(|error| format!("无法创建 / cannot create {}: {error}", part.display()))?;
    let prefix = format!("{}/", pack.path);
    let (mut files, mut bytes): (Vec<(String, [u8; 32])>, u64) = (Vec::new(), 0);
    for at in 0..archive.len() {
        let mut entry =
            archive.by_index(at).map_err(|error| format!("{MISMATCH}: {} 读不出 / unreadable: {error}", pack.path))?;
        let name = entry.name().to_string();
        let Some(relative) = name.strip_prefix(&prefix) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        if relative == "." || !safe_source_path(relative) {
            return Err(format!("{MISMATCH}: 包内路径不能使用 / a path in the pack cannot be used: {name}"));
        }
        if files.len() as u64 >= pack.file_count {
            return Err(format!(
                "{MISMATCH}: {} 的文件多于 / holds more files than {}",
                pack.path, pack.file_count
            ));
        }
        let path = relative.split('/').fold(part.to_path_buf(), |parent, segment| parent.join(segment));
        let cannot = |error: io::Error| format!("无法写入 / cannot write {}: {error}", path.display());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(cannot)?;
        }
        // A name written twice — or two differing only in case, one file on
        // NTFS — is refused, never overwritten.
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(cannot)?;
        // Never more than one byte past the bytes the index declares.
        let mut limited = (&mut entry).take((pack.byte_count - bytes).saturating_add(1));
        let (mut hasher, mut buffer) = (Sha256::new(), vec![0u8; 64 * 1024]);
        loop {
            let read = limited
                .read(&mut buffer)
                .map_err(|error| format!("{MISMATCH}: {name} 解压失败 / inflate failed: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read]).map_err(cannot)?;
            bytes += read as u64;
        }
        if bytes > pack.byte_count {
            return Err(format!(
                "{MISMATCH}: {} 的字节多于 / holds more bytes than {}",
                pack.path, pack.byte_count
            ));
        }
        file.sync_all().map_err(cannot)?;
        files.push((relative.to_string(), hasher.finalize().into()));
    }
    let actual = content_directory_digest(files.iter().map(|(path, hash)| (path.as_str(), *hash)));
    if files.len() as u64 != pack.file_count || bytes != pack.byte_count || actual != pack.sha256 {
        return Err(format!(
            "{MISMATCH}: {} 应为 / expected {} 个文件 / files {} 字节 / bytes 摘要 / digest {}，实为 / actual {} 个文件 / files {bytes} 字节 / bytes 摘要 / digest {actual}",
            pack.path,
            pack.file_count,
            pack.byte_count,
            pack.sha256,
            files.len()
        ));
    }
    Ok(())
}

/// What is already at a v2 pack's place.
enum Present {
    Absent,
    /// A directory of regular files whose digest is the pack's.
    Equal,
    /// Anything else, and why.
    Differs(String),
}

/// `target` read whole and its `content-directory.v1` digest compared with
/// `digest`. A read that fails is an error, never taken as a difference.
fn present(target: &Path, digest: &str) -> Result<Present, String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Present::Absent),
        Err(error) => return Err(format!("读取失败 / read failed: {}: {error}", target.display())),
    };
    if !metadata.is_dir() {
        return Ok(Present::Differs("不是目录 / not a directory".to_string()));
    }
    let mut files = Vec::new();
    if let Some(why) = hash_tree(target, "", &mut files)? {
        return Ok(Present::Differs(why));
    }
    let actual = content_directory_digest(files.iter().map(|(path, hash)| (path.as_str(), *hash)));
    Ok(match actual == digest {
        true => Present::Equal,
        false => Present::Differs(format!("内容摘要为 / its content digest is {actual}")),
    })
}

/// Every file under `dir`, hashed, by its `/`-joined path below the pack's
/// directory; `Some(why)` for what a content directory cannot hold: a link, a
/// node that is neither file nor directory, a name that is not UTF-8.
fn hash_tree(dir: &Path, prefix: &str, files: &mut Vec<(String, [u8; 32])>) -> Result<Option<String>, String> {
    let unreadable = |error: io::Error| format!("读取目录失败 / read_dir failed: {}: {error}", dir.display());
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Ok(Some(format!("名称不是 UTF-8 / a name is not UTF-8: {}", entry.path().display())));
        };
        let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let kind = entry.file_type().map_err(unreadable)?;
        if kind.is_symlink() {
            return Ok(Some(format!("链接 / a link: {relative}")));
        } else if kind.is_dir() {
            if let Some(why) = hash_tree(&entry.path(), &relative, files)? {
                return Ok(Some(why));
            }
        } else if kind.is_file() {
            let path = entry.path();
            let hash = hash_file(&path).map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
            files.push((relative, hash));
        } else {
            return Ok(Some(format!("不是普通文件 / not a regular file: {relative}")));
        }
    }
    Ok(None)
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let (mut hasher, mut buffer) = (Sha256::new(), vec![0u8; 64 * 1024]);
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// `path` renamed beside itself to `<digest>.broken-<unix>` — `-2`, `-3`, …
/// appended while that name is taken — and kept: nothing here deletes it.
fn set_aside(path: &Path, digest: &str) -> Result<PathBuf, String> {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("系统时钟早于 1970 / the system clock is before 1970: {error}"))?
        .as_secs();
    let parent = path.parent().unwrap_or(Path::new(""));
    for attempt in 1..=100u32 {
        let kept = parent.join(match attempt {
            1 => format!("{digest}.broken-{unix}"),
            n => format!("{digest}.broken-{unix}-{n}"),
        });
        if fs::symlink_metadata(&kept).is_ok() {
            continue;
        }
        return fs::rename(path, &kept).map(|()| kept.clone()).map_err(|error| {
            format!("无法改名保留 / cannot set aside {} as {}: {error}", path.display(), kept.display())
        });
    }
    Err(format!(
        "无法改名保留 / cannot set aside {}: the names {digest}.broken-{unix}[-n] are all taken",
        path.display()
    ))
}

/// `reason`, with an unpack that failed set aside and kept — or, when it
/// could not be, that said too.
fn keep_unfinished(part: &Path, digest: &str, reason: String) -> String {
    if fs::symlink_metadata(part).is_err() {
        return reason;
    }
    match set_aside(part, digest) {
        Ok(kept) => format!("{reason}\n未完成的解包已改名保留 / the unfinished unpack is kept as: {}", kept.display()),
        Err(error) => format!("{reason}\n{error}"),
    }
}

/// A pack's file name: `packs/<plain name>.zip`, nothing else.
fn pack_name(pack: &Pack) -> Result<&str, String> {
    match pack.path.strip_prefix("packs/") {
        Some(name) if fetch::plain(name) && name.ends_with(".zip") => Ok(name),
        _ => Err(format!("包路径不能使用 / the pack path cannot be used: {}", pack.path)),
    }
}

fn open(file: &Path) -> Result<zip::ZipArchive<File>, String> {
    let handle = File::open(file).map_err(|error| format!("打开失败 / open failed: {}: {error}", file.display()))?;
    zip::ZipArchive::new(handle)
        .map_err(|error| format!("不是可读的 zip / not a readable zip: {}: {error}", file.display()))
}

/// One declaration read whole — at most `DECLARATION_LIMIT` — or `None` when
/// the zip has no such name.
fn member(archive: &mut zip::ZipArchive<File>, file: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(format!("{MISMATCH}: {} 内 {name} 读不出 / unreadable: {error}", file.display())),
    };
    let mut bytes = Vec::new();
    entry
        .take(DECLARATION_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{MISMATCH}: {} 内 {name} 解压失败 / inflate failed: {error}", file.display()))?;
    if bytes.len() as u64 > DECLARATION_LIMIT {
        return Err(format!("{MISMATCH}: {} 内 {name} 过大 / too large", file.display()));
    }
    Ok(Some(bytes))
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8], file: &Path, name: &str) -> Result<T, String> {
    serde_json::from_slice(bytes)
        .map_err(|error| format!("{} 内 {name} 无法解析 / does not parse: {error}", file.display()))
}
