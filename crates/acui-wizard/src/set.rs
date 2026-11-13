// SPDX-License-Identifier: GPL-3.0-only
//! The install set: one folder holding `install-set.json` (`actingcommand.install-set.v1`, this
//! repository's own format) and every file it names — the Runtime and Tools zips, the console
//! zip, one standard package zip per game and the vision files — each with its size and SHA-256.
//! The wizard installs nothing else and downloads nothing.
//!
//! Checking a set reads it only: each file against its size and SHA-256; each program zip
//! against its own `BUILD-MANIFEST.json` (the repository and the commit the set names, every
//! entry's size and SHA-256, no entry it does not list); each standard package zip's
//! `bundle.json`, its `applications.json`, its installed layout, and its `interfaces.json`,
//! whose `package` maximum must lie within the range the Runtime zip's own manifest declares.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::apps::{parse_applications, Game};
use crate::archive::{safe_relative, Archive};
use crate::digest::{is_sha256, sha256_file};
use crate::engine::Event;
use crate::layout::{
    parse_bundle, plain_name, plan_layout, BundleIndex, APPLICATIONS_FILE, BUNDLE_FILE,
    INTERFACES_FILE,
};
use crate::manifest::{
    check_identity, package_edge, parse_manifest, speaks, BuildManifest, Program, MANIFEST_FILE,
};
use crate::text::Said;

pub const SET_FILE: &str = "install-set.json";
pub const SET_SCHEMA: &str = "actingcommand.install-set.v1";
/// The one provider a set carries in v1 (DirectML later).
pub const VISION_PROVIDER: &str = "cpu";
/// The file a vision root needs whenever `vision` is configured (Runtime
/// `contracts/vision-model-folders.md`).
pub const ORT_LIBRARY: &str = "ort/onnxruntime.dll";
const SET_FILE_LIMIT: u64 = 1 << 20;

#[derive(Debug, Clone, Deserialize)]
pub struct InstallSet {
    pub schema_version: String,
    pub label: String,
    pub runtime: RuntimeEntry,
    pub console: ConsoleEntry,
    pub bundles: Vec<BundleEntry>,
    pub vision: VisionEntry,
}

/// A file of the set: its path in the set folder, its size and its SHA-256.
#[derive(Debug, Clone, Deserialize)]
pub struct FileRef {
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeEntry {
    pub tag: String,
    pub commit: String,
    pub runtime_zip: FileRef,
    pub tools_zip: FileRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleEntry {
    pub tag: String,
    pub commit: String,
    pub zip: FileRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundleEntry {
    /// The game's folder name, which the zip's `bundle.json` must name.
    pub game: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

impl BundleEntry {
    pub fn file_ref(&self) -> FileRef {
        FileRef {
            file: self.file.clone(),
            size: self.size,
            sha256: self.sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VisionEntry {
    pub provider: String,
    pub files: Vec<VisionFile>,
}

/// A vision file: where it goes below the vision root, and the file of the set.
#[derive(Debug, Clone, Deserialize)]
pub struct VisionFile {
    pub path: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

impl VisionFile {
    pub fn file_ref(&self) -> FileRef {
        FileRef {
            file: self.file.clone(),
            size: self.size,
            sha256: self.sha256.clone(),
        }
    }
}

impl InstallSet {
    /// Every file of the set, with what it is for, in the order they are checked.
    pub fn files(&self) -> Vec<(String, FileRef)> {
        let mut files = vec![
            ("Runtime zip".to_owned(), self.runtime.runtime_zip.clone()),
            ("Tools zip".to_owned(), self.runtime.tools_zip.clone()),
            ("console zip".to_owned(), self.console.zip.clone()),
        ];
        for bundle in &self.bundles {
            files.push((format!("standard package {}", bundle.game), bundle.file_ref()));
        }
        for vision in &self.vision.files {
            files.push((format!("vision {}", vision.path), vision.file_ref()));
        }
        files
    }
}

/// The set file in `bytes` (a UTF-8 byte-order mark allowed), as JSON; its rules are
/// [`set_problems`].
pub fn parse_install_set(bytes: &[u8]) -> Result<InstallSet, Said> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    serde_json::from_slice(bytes)
        .map_err(|error| Said::new("install-set.json 读不懂", "install-set.json does not parse").with(error))
}

fn commit_like(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Every rule of a set file that needs no file read: its schema, the two commits, each file
/// plainly inside the set folder with a size and a SHA-256, each game once and a plain folder
/// name, a CPU vision set with ONNX Runtime and at least one model, each vision path once and
/// below `ort/` or `models/<model_ref>/`.
pub fn set_problems(set: &InstallSet) -> Vec<Said> {
    let mut problems = Vec::new();
    if set.schema_version != SET_SCHEMA {
        problems.push(
            Said::new("install-set.json 的格式版本不对", "install-set.json is not actingcommand.install-set.v1")
                .with(&set.schema_version),
        );
    }
    if set.label.trim().is_empty() {
        problems.push(Said::new("install-set.json 缺少 label", "install-set.json has no label"));
    }
    for (what, commit, tag) in [
        ("runtime", &set.runtime.commit, &set.runtime.tag),
        ("console", &set.console.commit, &set.console.tag),
    ] {
        if !commit_like(commit) {
            problems.push(
                Said::new("提交号不是 40 位十六进制", "the commit is not 40 hex digits")
                    .with(format!("{what}.commit {commit}")),
            );
        }
        if tag.trim().is_empty() {
            problems.push(Said::new("缺少版本标签", "the tag is missing").with(format!("{what}.tag")));
        }
    }
    for (what, file) in set.files() {
        if !safe_relative(&file.file) {
            problems.push(
                Said::new("文件名不在安装集文件夹内", "the file is not plainly inside the install set folder")
                    .with(format!("{what}: {}", file.file)),
            );
        }
        if !is_sha256(&file.sha256) {
            problems.push(
                Said::new("SHA-256 不是 64 位十六进制", "the SHA-256 is not 64 hex digits")
                    .with(format!("{what}: {}", file.sha256)),
            );
        }
        if file.size == 0 {
            problems.push(Said::new("文件大小为 0", "the file size is 0").with(what));
        }
    }
    let mut games = BTreeSet::new();
    for bundle in &set.bundles {
        if !plain_name(&bundle.game) {
            problems.push(Said::new("游戏名不是单个文件夹名", "the game is no plain folder name").with(&bundle.game));
        } else if !games.insert(bundle.game.to_ascii_lowercase()) {
            problems.push(Said::new("游戏重复", "the game is listed twice").with(&bundle.game));
        }
    }
    if set.vision.provider != VISION_PROVIDER {
        problems.push(
            Said::new("这一版只装 CPU 视觉", "this version installs the CPU vision set only")
                .with(&set.vision.provider),
        );
    }
    let mut paths = BTreeSet::new();
    let mut models = false;
    for vision in &set.vision.files {
        let segments: Vec<&str> = vision.path.split('/').collect();
        let placed = safe_relative(&vision.path)
            && match segments.first().copied() {
                Some("ort") => segments.len() >= 2,
                Some("models") => segments.len() >= 3,
                _ => false,
            };
        if !placed {
            problems.push(
                Said::new("视觉文件的位置不在 ort/ 或 models/<模型>/ 下", "the vision path is not below ort/ or models/<model_ref>/")
                    .with(&vision.path),
            );
        }
        if !paths.insert(vision.path.to_ascii_lowercase()) {
            problems.push(Said::new("视觉文件重复", "the vision path is listed twice").with(&vision.path));
        }
        models |= placed && segments.first() == Some(&"models");
    }
    if !paths.contains(ORT_LIBRARY) {
        problems.push(Said::new("视觉文件缺少 ONNX Runtime", "the vision set lacks ONNX Runtime").with(ORT_LIBRARY));
    }
    if !models {
        problems.push(Said::new("视觉文件里没有模型", "the vision set holds no model"));
    }
    problems
}

/// A program zip, checked.
#[derive(Debug, Clone)]
pub struct VerifiedProgram {
    pub program: Program,
    pub zip: PathBuf,
    pub manifest: BuildManifest,
    /// The bytes of the files it lays into the root.
    pub bytes: u64,
}

/// A standard package zip, checked, with its laid-out plan.
#[derive(Debug, Clone)]
pub struct VerifiedBundle {
    pub entry: BundleEntry,
    pub zip: PathBuf,
    pub index: BundleIndex,
    pub game: Game,
    pub package: [u64; 2],
    /// Each entry laid, with its path below the game root.
    pub layout: Vec<(String, String)>,
    pub bytes: u64,
}

/// A set whose every file was checked.
#[derive(Debug, Clone)]
pub struct VerifiedSet {
    pub dir: PathBuf,
    pub set: InstallSet,
    pub programs: Vec<VerifiedProgram>,
    /// The Runtime's `package` range, from its own manifest.
    pub package_range: [u64; 2],
    pub bundles: Vec<VerifiedBundle>,
    pub vision_bytes: u64,
}

impl VerifiedSet {
    pub fn program(&self, program: Program) -> &VerifiedProgram {
        self.programs
            .iter()
            .find(|verified| verified.program == program)
            .expect("every program is verified")
    }

    pub fn bundle(&self, game: &str) -> Option<&VerifiedBundle> {
        self.bundles.iter().find(|bundle| bundle.entry.game == game)
    }

    /// The games of the set, as the window lists them.
    pub fn games(&self) -> Vec<Game> {
        self.bundles.iter().map(|bundle| bundle.game.clone()).collect()
    }

    /// The bytes an install of `games` lays down, with the copy of the Runtime and Tools the
    /// Runtime's `install adopt` keeps (`install\builds\<commit>\`) and room for the state.
    pub fn space_needed(&self, games: &[String]) -> u64 {
        const STATE_ROOM: u64 = 512 << 20;
        let programs = self.program(Program::Runtime).bytes * 2
            + self.program(Program::Tools).bytes * 2
            + self.program(Program::Console).bytes;
        let bundles: u64 = self
            .bundles
            .iter()
            .filter(|bundle| games.contains(&bundle.entry.game))
            .map(|bundle| bundle.bytes)
            .sum();
        programs + bundles + self.vision_bytes + STATE_ROOM
    }
}

/// The folder beside this program when it holds an install set (the window looks there
/// first).
pub fn beside_program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    dir.join(SET_FILE).is_file().then_some(dir)
}

/// Reads and checks the install set in `dir` (see the module's description); `report` hears a
/// line per file checked and the bytes hashed.
pub fn verify_set(dir: &Path, report: &mut dyn FnMut(Event)) -> Result<VerifiedSet, Said> {
    let set_path = dir.join(SET_FILE);
    let unreadable = |error: &dyn std::fmt::Display| {
        Said::new("install-set.json 读不出", "install-set.json cannot be read")
            .with(format!("{}: {error}", set_path.display()))
    };
    let length = fs::metadata(&set_path).map_err(|error| unreadable(&error))?.len();
    if length > SET_FILE_LIMIT {
        return Err(unreadable(&"larger than 1 MiB"));
    }
    let bytes = fs::read(&set_path).map_err(|error| unreadable(&error))?;
    let set = parse_install_set(&bytes)?;
    let problems = set_problems(&set);
    if !problems.is_empty() {
        let zh: Vec<String> = problems.iter().map(|problem| problem.zh.clone()).collect();
        let en: Vec<String> = problems.iter().map(|problem| problem.en.clone()).collect();
        return Err(Said::new(zh.join("；"), en.join("; ")));
    }
    report(Event::Line(
        Said::new("读到安装集", "install set read").with(format!(
            "{} (Runtime {} {}, console {} {})",
            set.label, set.runtime.tag, set.runtime.commit, set.console.tag, set.console.commit
        )),
    ));

    // Every file against its size, then its SHA-256.
    let files = set.files();
    for (what, file) in &files {
        let path = dir.join(&file.file);
        let size = fs::metadata(&path)
            .map_err(|error| {
                Said::new("安装集缺少文件", "a file of the install set is missing")
                    .with(format!("{what}: {}: {error}", path.display()))
            })?
            .len();
        if size != file.size {
            return Err(Said::new("文件大小与安装集不符", "the file size does not match the install set")
                .with(format!("{what}: {}: expected {}, found {size}", path.display(), file.size)));
        }
    }
    let total: u64 = files.iter().map(|(_, file)| file.size).sum();
    let mut done = 0u64;
    for (what, file) in &files {
        let path = dir.join(&file.file);
        let (sha256, _) = sha256_file(&path, &mut |chunk: u64| {
            done += chunk;
            report(Event::Bytes { done, total });
        })
        .map_err(|error| {
            Said::new("文件读不出", "the file cannot be read").with(format!("{}: {error}", path.display()))
        })?;
        if !sha256.eq_ignore_ascii_case(&file.sha256) {
            return Err(Said::new("文件的 SHA-256 与安装集不符", "the file's SHA-256 does not match the install set")
                .with(format!("{what}: {}: expected {}, found {sha256}", path.display(), file.sha256)));
        }
        report(Event::Line(
            Said::new("已核对", "checked").with(format!("{what}: {} ({} bytes)", file.file, file.size)),
        ));
    }

    // Each program zip against its own manifest.
    let mut programs = Vec::new();
    for program in Program::ALL {
        let (file, commit) = match program {
            Program::Runtime => (&set.runtime.runtime_zip, &set.runtime.commit),
            Program::Tools => (&set.runtime.tools_zip, &set.runtime.commit),
            Program::Console => (&set.console.zip, &set.console.commit),
        };
        let zip = dir.join(&file.file);
        programs.push(verify_program(program, &zip, commit, report)?);
    }
    let runtime_manifest = &programs[0].manifest;
    let package_range = runtime_manifest
        .interfaces
        .as_ref()
        .and_then(|declaration| speaks(declaration, "package"))
        .ok_or_else(|| {
            Said::new(
                "Runtime 的 BUILD-MANIFEST.json 没有声明 package 接口",
                "the Runtime's BUILD-MANIFEST.json declares no package interface",
            )
        })?;

    // Each standard package zip.
    let mut bundles = Vec::new();
    for entry in &set.bundles {
        let zip = dir.join(&entry.file);
        bundles.push(verify_bundle(entry, &zip, package_range)?);
        report(Event::Line(
            Said::new("标准包已核对", "standard package checked").with(format!("{} ({})", entry.game, entry.file)),
        ));
    }
    let vision_bytes = set.vision.files.iter().map(|file| file.size).sum();
    Ok(VerifiedSet {
        dir: dir.to_path_buf(),
        set,
        programs,
        package_range,
        bundles,
        vision_bytes,
    })
}

fn verify_program(
    program: Program,
    zip: &Path,
    commit: &str,
    report: &mut dyn FnMut(Event),
) -> Result<VerifiedProgram, Said> {
    let label = program.label();
    let mut archive = Archive::open(zip)?;
    let manifest = parse_manifest(&archive.read_small(MANIFEST_FILE)?)?;
    check_identity(program, &manifest, commit)?;
    let mut listed: BTreeSet<String> = manifest.files.iter().map(|file| file.path.clone()).collect();
    listed.insert(MANIFEST_FILE.to_owned());
    let present: BTreeSet<String> = archive.names().into_iter().collect();
    if let Some(extra) = present.difference(&listed).next() {
        return Err(Said::new(
            format!("{}的 zip 里有清单没列的文件", label.zh),
            format!("{}'s zip holds a file its manifest does not list", label.en),
        )
        .with(extra));
    }
    if let Some(missing) = listed.difference(&present).next() {
        return Err(Said::new(
            format!("{}的 zip 缺少清单列出的文件", label.zh),
            format!("{}'s zip lacks a file its manifest lists", label.en),
        )
        .with(missing));
    }
    let mut bytes = 0u64;
    for file in &manifest.files {
        let size = archive.size_of(&file.path).unwrap_or(0);
        let (sha256, hashed) = archive.hash(&file.path)?;
        if size != file.size_bytes || hashed != file.size_bytes || !sha256.eq_ignore_ascii_case(&file.sha256) {
            return Err(Said::new(
                format!("{}的文件与清单不符", label.zh),
                format!("{}'s file does not match its manifest", label.en),
            )
            .with(format!(
                "{}: expected {} bytes {}, found {hashed} bytes {sha256}",
                file.path, file.size_bytes, file.sha256
            )));
        }
        if program.lays(&file.path) {
            bytes += file.size_bytes;
        }
    }
    report(Event::Line(
        Said::new(
            format!("{}的 {} 个文件与清单一致", label.zh, manifest.files.len()),
            format!("{}: {} files match the manifest", label.en, manifest.files.len()),
        )
        .with(format!("{} @ {}", manifest.repository, manifest.commit_sha)),
    ));
    Ok(VerifiedProgram {
        program,
        zip: zip.to_path_buf(),
        manifest,
        bytes,
    })
}

fn verify_bundle(entry: &BundleEntry, zip: &Path, package_range: [u64; 2]) -> Result<VerifiedBundle, Said> {
    let game_said = |said: Said| said.with(format!("{} ({})", entry.game, entry.file));
    let mut archive = Archive::open(zip)?;
    let index = parse_bundle(&archive.read_small(BUNDLE_FILE)?).map_err(game_said)?;
    if index.game != entry.game {
        return Err(game_said(
            Said::new("bundle.json 的游戏与安装集不符", "bundle.json names another game than the install set")
                .with(&index.game),
        ));
    }
    let declaration: serde_json::Value = {
        let bytes = archive.read_small(INTERFACES_FILE)?;
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes).to_vec();
        serde_json::from_slice(&bytes).map_err(|error| {
            game_said(Said::new("interfaces.json 读不懂", "interfaces.json does not parse").with(error))
        })?
    };
    let package = speaks(&declaration, "package").ok_or_else(|| {
        game_said(Said::new(
            "interfaces.json 没有声明 package 接口",
            "interfaces.json declares no package interface",
        ))
    })?;
    if !package_edge(package, package_range) {
        return Err(game_said(
            Said::new(
                "标准包的 package 接口不在这个 Runtime 支持的范围内",
                "the standard package's package interface is outside this Runtime's range",
            )
            .with(format!(
                "package [{}, {}], Runtime [{}, {}]",
                package[0], package[1], package_range[0], package_range[1]
            )),
        ));
    }
    let game = parse_applications(&archive.read_small(APPLICATIONS_FILE)?).map_err(game_said)?;
    if game.game != entry.game {
        return Err(game_said(
            Said::new("applications.json 的游戏与安装集不符", "applications.json names another game than the install set")
                .with(&game.game),
        ));
    }
    for pack in &index.packs {
        if game.server(&pack.server).is_none() {
            return Err(game_said(
                Said::new("包的区服不在 applications.json 里", "a pack's server is not in applications.json")
                    .with(format!("{} → {}", pack.package_id, pack.server)),
            ));
        }
    }
    let layout = plan_layout(&archive.names(), &index).map_err(game_said)?;
    let bytes = layout
        .iter()
        .map(|(name, _)| archive.size_of(name).unwrap_or(0))
        .sum();
    Ok(VerifiedBundle {
        entry: entry.clone(),
        zip: zip.to_path_buf(),
        index,
        game,
        package,
        layout,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn sha(fill: char) -> String {
        fill.to_string().repeat(64)
    }

    /// A set in the shape of the design (UI channel design-wizard §1), with the seven pinned
    /// vision files of Runtime contracts/vision-model-folders.md (CPU: without DirectML.dll).
    fn good() -> Value {
        json!({
            "schema_version": "actingcommand.install-set.v1",
            "label": "ActingCommand 0.12.58 test set",
            "runtime": {
                "tag": "v0.12.58", "commit": "ef94d950a102a595fa0bde974b8d77ad54329b48",
                "runtime_zip": {"file": "actingcommand-runtime-ef94d950.zip", "size": 10, "sha256": sha('a')},
                "tools_zip": {"file": "actingcommand-tools-ef94d950.zip", "size": 10, "sha256": sha('b')}
            },
            "console": {
                "tag": "v0.12.17", "commit": "594d7ee4594d7ee4594d7ee4594d7ee4594d7ee4",
                "zip": {"file": "acui-windows-594d7ee4.zip", "size": 10, "sha256": sha('c')}
            },
            "bundles": [
                {"game": "azurlane", "file": "bundles/azurlane-resources-1234567.zip", "size": 10,
                 "sha256": sha('d'), "repository": "owner/azurlane-resources", "tag": "v1"}
            ],
            "vision": {"provider": "cpu", "files": [
                {"path": "ort/onnxruntime.dll", "file": "vision/onnxruntime.dll", "size": 17328152,
                 "sha256": "e7eedec6a6f26dc39dc948276a75ef6d2bee3fff944d874ceed0bbd3b97bff40"},
                {"path": "ort/onnxruntime_providers_shared.dll", "file": "vision/onnxruntime_providers_shared.dll",
                 "size": 22040, "sha256": "265c8daf29637cb259cac8be9f08f2cd45f3883f0f0e4949cbfddd5b4cbec3b6"},
                {"path": "ort/licenses/onnxruntime/LICENSE", "file": "vision/LICENSE", "size": 1094,
                 "sha256": "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674"},
                {"path": "ort/licenses/onnxruntime/ThirdPartyNotices.txt", "file": "vision/ThirdPartyNotices.txt",
                 "size": 331175, "sha256": "fb0af774b4d7cffc5b9d046f2aaeade2f37df2f80abf8033c95dfffcc77a8866"},
                {"path": "models/PP-OCRv6_medium/det.onnx", "file": "vision/det.onnx", "size": 62032837,
                 "sha256": "eb13b44b25bb36f89528b68720af8a61d9cf381176107f465db1757b65d086e1"},
                {"path": "models/PP-OCRv6_medium/rec.onnx", "file": "vision/rec.onnx", "size": 76554979,
                 "sha256": "9c09abf0957f7968c7586464b7397b84ad2387a0497a351af40e9acc71b673ba"},
                {"path": "models/PP-OCRv6_medium/keys.txt", "file": "vision/keys.txt", "size": 74947,
                 "sha256": "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d"}
            ]}
        })
    }

    fn problems(value: Value) -> Vec<Said> {
        let text = serde_json::to_vec(&value).expect("json");
        set_problems(&parse_install_set(&text).expect("a set"))
    }

    #[test]
    fn the_designed_set_has_no_problem() {
        assert_eq!(problems(good()), Vec::<Said>::new());
        let set = parse_install_set(&serde_json::to_vec(&good()).unwrap()).unwrap();
        // Three program zips, one bundle, seven vision files.
        assert_eq!(set.files().len(), 11);
    }

    type Change = Box<dyn Fn(&mut Value)>;

    fn case(name: &'static str, change: impl Fn(&mut Value) + 'static) -> (&'static str, Change) {
        (name, Box::new(change))
    }

    #[test]
    fn each_broken_rule_is_named() {
        let cases = vec![
            case("schema", |set| set["schema_version"] = json!("actingcommand.install-set.v2")),
            case("commit", |set| set["runtime"]["commit"] = json!("ef94d950")),
            case("tag", |set| set["console"]["tag"] = json!(" ")),
            case("escaping file", |set| set["console"]["zip"]["file"] = json!("../acui.zip")),
            case("absolute file", |set| set["runtime"]["tools_zip"]["file"] = json!("C:/tools.zip")),
            case("sha256", |set| set["bundles"][0]["sha256"] = json!("abc")),
            case("size", |set| set["bundles"][0]["size"] = json!(0)),
            case("game", |set| set["bundles"][0]["game"] = json!("a/b")),
            case("twice", |set| {
                let copy = set["bundles"][0].clone();
                set["bundles"].as_array_mut().unwrap().push(copy);
            }),
            case("provider", |set| set["vision"]["provider"] = json!("directml")),
            case("vision path", |set| set["vision"]["files"][1]["path"] = json!("bin/x.dll")),
            case("model folder", |set| set["vision"]["files"][6]["path"] = json!("models/keys.txt")),
            case("no ort", |set| set["vision"]["files"][0]["path"] = json!("ort/other.dll")),
            case("no model", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| !file["path"].as_str().unwrap().starts_with("models/"));
            }),
        ];
        for (name, change) in cases {
            let mut set = good();
            change(&mut set);
            assert!(!problems(set).is_empty(), "{name}");
        }
    }

    #[test]
    fn a_set_file_must_be_json_of_its_shape() {
        assert!(parse_install_set(b"not json").is_err());
        let mut missing = good();
        missing.as_object_mut().unwrap().remove("vision");
        assert!(parse_install_set(&serde_json::to_vec(&missing).unwrap()).is_err());
        // Members a later wizard adds are read past.
        let mut later = good();
        later["payload"] = json!({"offset": 1});
        assert!(parse_install_set(&serde_json::to_vec(&later).unwrap()).is_ok());
    }
}
