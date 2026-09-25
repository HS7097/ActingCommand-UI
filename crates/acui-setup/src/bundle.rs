// SPDX-License-Identifier: GPL-3.0-only
//! The instances step's resources: resource repositories' bundles — each one
//! zip holding `applications.json` (the game, its display name when given, and
//! each server's Android package name), `bundle.json` (every pack's path,
//! package id, server, sha256 and size, and each server's default pack) and the
//! sealed packs under `packs/`. The release carries them: its `MEMBERS.json`
//! names each in `bundles[]` with its sha256, and the file sits beside it in
//! the download folder, already checked against `SHA256SUMS`. A local bundle
//! file may be added when the release carries none; no hash is asked of the
//! person. A bundle's packs are laid out under `<root>\packages\<game>\` byte
//! for byte, each checked against `bundle.json` first; the Runtime only ever
//! sees one of them per instance, a local file. The wizard knows no game: the
//! display names and package names come from the bundles.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::fetch;
use crate::verify::{hex, Report, Step, Total, MISMATCH};

const APPLICATIONS_SCHEMA: &str = "actingcommand.applications.v1";
const BUNDLE_SCHEMA: &str = "actingcommand.bundle.v1";
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
    /// A server's default pack, when the bundle names one.
    pub defaults: BTreeMap<String, String>,
    /// Carried by the release, or a local file the person added.
    pub carried: bool,
}

#[derive(Deserialize, Clone)]
pub struct Pack {
    pub path: String,
    pub package_id: String,
    pub server: String,
    pub sha256: String,
    pub byte_count: u64,
}

#[derive(Deserialize, Clone)]
pub struct Application {
    pub application_id: String,
    pub label: String,
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
                report.line(&format!("发布件自带的大包 / carried bundle: {} → {}", entry.asset, bundle.name()))?;
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
        return Err(format!("大包须为存在的文件的绝对路径 / the bundle must be the absolute path of an existing file: {given}"));
    }
    read(&path)
}

/// A zip read as a bundle: both declarations present, their formats known,
/// naming the same game, every pack name usable and unique, every default pack
/// listed.
pub fn read(file: &Path) -> Result<Bundle, String> {
    let mut archive = open(file)?;
    let bundle: BundleFile = match member(&mut archive, file, "bundle.json")? {
        Some(bytes) => parse(&bytes, file, "bundle.json")?,
        None => {
            return Err(format!(
                "不是资源仓大包（根目录没有 bundle.json）/ Not a resource repository bundle (no bundle.json at its root): {}",
                file.display()
            ))
        }
    };
    let applications: ApplicationsFile = match member(&mut archive, file, "applications.json")? {
        Some(bytes) => parse(&bytes, file, "applications.json")?,
        None => return Err(format!("大包缺少 applications.json / the bundle lacks applications.json: {}", file.display())),
    };
    if bundle.schema_version != BUNDLE_SCHEMA || applications.schema_version != APPLICATIONS_SCHEMA {
        return Err(format!(
            "大包的格式版本无法识别 / unknown bundle format: {} · {}（应为 / expected {BUNDLE_SCHEMA} · {APPLICATIONS_SCHEMA}）",
            bundle.schema_version, applications.schema_version
        ));
    }
    if bundle.game != applications.game {
        return Err(format!(
            "大包里两份声明的 game 不一致 / the bundle's two declarations name different games: {} ≠ {}",
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
            return Err(format!("大包里有重名的包 / the bundle lists a pack name twice: {}", pack.path));
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
    })
}

impl Bundle {
    /// The game as people know it: the bundle's display name, else its id.
    pub fn name(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.game.clone())
    }

    /// Every pack laid out under `dir` byte for byte, once its size and sha256
    /// match `bundle.json`; returns where each landed, by its path in the zip.
    pub fn lay_out(&self, dir: &Path, report: Report<'_>) -> Result<BTreeMap<String, PathBuf>, String> {
        let mut archive = open(&self.file)?;
        fs::create_dir_all(dir).map_err(|error| format!("无法创建 / cannot create {}: {error}", dir.display()))?;
        report.step(Step::Phase(
            "放置资源包 / Placing the resource packs",
            Some(Total::Items(self.packs.len() as u64)),
        ))?;
        let mut laid = BTreeMap::new();
        for (done, pack) in self.packs.iter().enumerate() {
            let mut entry = match archive.by_name(&pack.path) {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => {
                    return Err(format!("{MISMATCH}: 大包里缺少 / the bundle lacks {}", pack.path))
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
