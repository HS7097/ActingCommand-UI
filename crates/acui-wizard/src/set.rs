// SPDX-License-Identifier: GPL-3.0-only
//! The install set: one folder holding `install-set.json` (`actingcommand.install-set.v1`, this
//! repository's own format) and every file it names — the Runtime and Tools zips, the console
//! zip, one standard package zip per game with the files that go beside it (its licenses and
//! notices, `bundles[].files`), and the vision files — each with its size and SHA-256. The
//! wizard installs nothing else and downloads nothing.
//!
//! Checking a set reads it only: each file against its size and SHA-256; each program zip
//! against its own `BUILD-MANIFEST.json` (the repository and the commit the set names, every
//! entry's size and SHA-256, no entry it does not list); the console's interfaces against the
//! Runtime's ([`console_problems`]); each standard package zip's `bundle.json`, its
//! `applications.json`, its installed layout, and its `interfaces.json`, whose `package` maximum
//! must lie within the range the Runtime zip's own manifest declares.
//!
//! A standard package's own files go to `<root>\licenses\<game>\<path>`, `path` a plain relative
//! path (each once per game); the ones it must not go without are named in
//! `bundles[].required_files`, and a set that lacks one is refused. The vision set must carry
//! ONNX Runtime's two files exactly as the Runtime pins them ([`ORT_PINS`]), and its licence and
//! notices beside the library ([`ORT_LICENSE`], [`ORT_NOTICES`]). It
//! may carry DirectML ([`DIRECTML_LIBRARY`], only the file the Runtime pins), and then DirectML's
//! own licence files too ([`DIRECTML_LICENSES`]); the configuration still names the CPU provider.
//! The licences of the vision models (`vision.licenses`, at least one with a model) go to
//! `<root>\licenses\vision\<path>`, by the rules of a standard package's own files.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::alias::{alias_limit, longest_item_id};
use crate::apps::{parse_applications, Game};
use crate::archive::{safe_relative, Archive};
use crate::digest::{is_sha256, sha256_file};
use crate::engine::Event;
use crate::layout::{
    parse_bundle, plain_name, plan_layout, BundleIndex, APPLICATIONS_FILE, BUNDLE_FILE,
    INTERFACES_FILE, SCHEDULE_FILE,
};
use crate::manifest::{
    check_identity, console_problems, package_edge, parse_manifest, speaks, BuildManifest, Program,
    MANIFEST_FILE,
};
use crate::text::Said;

pub const SET_FILE: &str = "install-set.json";
pub const SET_SCHEMA: &str = "actingcommand.install-set.v1";
/// The one execution provider the configuration names in v1. A set may already carry DirectML
/// ([`DIRECTML_LIBRARY`]); the configuration that selects it (and its `directml_device`) comes
/// later.
pub const VISION_PROVIDER: &str = "cpu";
/// The file a vision root needs whenever `vision` is configured (Runtime
/// `contracts/vision-model-folders.md`).
pub const ORT_LIBRARY: &str = "ort/onnxruntime.dll";
/// The library ONNX Runtime loads its providers through, needed beside [`ORT_LIBRARY`] for every
/// provider, the CPU's too ("Loading").
pub const ORT_PROVIDERS_SHARED: &str = "ort/onnxruntime_providers_shared.dll";
/// The two ONNX Runtime files as the Runtime pins them (ONNX Runtime 1.24.4, taken from its
/// DirectML build). The Runtime checks both against these sizes and SHA-256 only for a
/// `directml` section, and refuses a vision root whose files differ (`vision_runtime_unavailable`;
/// Runtime `apps/actingd/src/config.rs` `check_runtime_pin`, called on the DirectML branch only);
/// for `cpu` it does not hash them. The set check requires them all the same, so that an install
/// set holds this one pinned build whichever provider is chosen.
pub const ORT_PINS: [(&str, u64, &str); 2] = [
    (ORT_LIBRARY, 17_328_152, "e7eedec6a6f26dc39dc948276a75ef6d2bee3fff944d874ceed0bbd3b97bff40"),
    (ORT_PROVIDERS_SHARED, 22_040, "265c8daf29637cb259cac8be9f08f2cd45f3883f0f0e4949cbfddd5b4cbec3b6"),
];
/// ONNX Runtime's licence and its third-party notices, which go wherever the library goes.
pub const ORT_LICENSE: &str = "ort/licenses/onnxruntime/LICENSE";
pub const ORT_NOTICES: &str = "ort/licenses/onnxruntime/ThirdPartyNotices.txt";
/// DirectML, which a vision root holds for the DirectML provider only: a run on the CPU loads
/// exactly `onnxruntime_providers_shared.dll` and `onnxruntime.dll` whether or not it lies
/// beside them, and neither check-config nor a start looks at it for a `cpu` section (Runtime
/// `contracts/vision-model-folders.md`, "Loading"; `apps/actingd/src/config.rs` `inspect`).
pub const DIRECTML_LIBRARY: &str = "ort/DirectML.dll";
/// The one DirectML a set may carry: DirectML 1.15.4 as the Runtime pins it (Runtime
/// `crates/vision-ffi/src/runtime_pin.rs` `DIRECTML_RUNTIME_PIN`, the `pinned_files` of
/// `directml-1.15.4` in `scripts/windows-tools/windows-tool-sources.v1.json`).
pub const DIRECTML_SIZE: u64 = 18_527_776;
pub const DIRECTML_SHA256: &str = "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1";
/// DirectML's licence files, which go wherever it goes: the three files the Runtime's tool
/// registry has whoever places DirectML copy to `ort/licenses/directml/` (`directml-1.15.4`,
/// `placement_note`; `contracts/vision-model-folders.md`, "Pinned files").
pub const DIRECTML_LICENSES: [&str; 3] = [
    "ort/licenses/directml/LICENSE.txt",
    "ort/licenses/directml/LICENSE-CODE.txt",
    "ort/licenses/directml/ThirdPartyNotices.txt",
];
/// The root's folder for the licence and notice files that go beside what the set installs:
/// one folder per game for a standard package's own files, and [`VISION_LICENSES_DIR`] for the
/// vision models'.
pub const LICENSES_DIR: &str = "licenses";
/// The folder under [`LICENSES_DIR`] for `vision.licenses`.
pub const VISION_LICENSES_DIR: &str = "vision";
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
    /// The files that go beside the package (its licences and notices, which its release hangs
    /// beside the zip): each laid at `<root>\licenses\<game>\<path>`.
    #[serde(default)]
    pub files: Vec<PlacedFile>,
    /// The `path`s among `files` the package must not go without (the licence and attribution
    /// of what it carries from elsewhere, an AGPL-3.0 project's for one): a set that lacks one
    /// is refused.
    #[serde(default)]
    pub required_files: Vec<String>,
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
    /// The licences of the vision models (PP-OCR's, for one), each laid at
    /// `<root>\licenses\vision\<path>`; at least one when the set carries a model.
    #[serde(default)]
    pub licenses: Vec<PlacedFile>,
}

/// A file laid as it is: where it goes (`path`, `/`-separated, below its folder in the root),
/// and the file of the set with its size and SHA-256.
#[derive(Debug, Clone, Deserialize)]
pub struct PlacedFile {
    pub path: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

/// A vision file: where it goes below the vision root.
pub type VisionFile = PlacedFile;

impl PlacedFile {
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
            for placed in &bundle.files {
                files.push((format!("standard package {} file {}", bundle.game, placed.path), placed.file_ref()));
            }
        }
        for vision in &self.vision.files {
            files.push((format!("vision {}", vision.path), vision.file_ref()));
        }
        for licence in &self.vision.licenses {
            files.push((format!("vision licence {}", licence.path), licence.file_ref()));
        }
        files
    }

    /// Whether the set carries DirectML ([`DIRECTML_LIBRARY`]).
    pub fn carries_directml(&self) -> bool {
        self.vision
            .files
            .iter()
            .any(|file| file.path.eq_ignore_ascii_case(DIRECTML_LIBRARY))
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
/// name, each of a game's own files at a plain relative path and each path once in its game,
/// each of its `required_files` among them, a
/// CPU vision set with ONNX Runtime's two files as the Runtime pins them, its licence and notices
/// and at least one model, each vision
/// path once and below `ort/` or `models/<model_ref>/`; DirectML, when carried, the pinned file
/// with its three licence files; at least one vision licence (a model is carried), each at a
/// plain relative path, each once, and no game whose folder would be theirs.
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
        let mut placed = BTreeSet::new();
        for file in &bundle.files {
            if !safe_relative(&file.path) {
                problems.push(
                    Said::new(
                        "标准包随带文件的位置不是普通的相对路径",
                        "a standard package's own file is not placed at a plain relative path",
                    )
                    .with(format!("{}: {}", bundle.game, file.path)),
                );
            } else if !placed.insert(file.path.to_ascii_lowercase()) {
                problems.push(
                    Said::new("标准包随带文件的位置重复", "a standard package's own file is placed twice")
                        .with(format!("{}: {}", bundle.game, file.path)),
                );
            }
        }
        for required in &bundle.required_files {
            if !placed.contains(&required.to_ascii_lowercase()) {
                problems.push(
                    Said::new(
                        "标准包必带的许可或声明文件不在安装集里",
                        "a licence or notice file the standard package must carry is not in the set",
                    )
                    .with(format!("{}: {required}", bundle.game)),
                );
            }
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
    for (pinned, size, sha256) in ORT_PINS {
        match set.vision.files.iter().find(|file| file.path.eq_ignore_ascii_case(pinned)) {
            None => problems.push(Said::new("视觉文件缺少 ONNX Runtime", "the vision set lacks ONNX Runtime").with(pinned)),
            Some(file) if file.size != size || !file.sha256.eq_ignore_ascii_case(sha256) => problems.push(
                Said::new(
                    "ONNX Runtime 不是 Runtime 钉住的那一版（1.24.4）：用显卡时 Runtime 会拒绝它，安装集只收钉住的这一套",
                    "ONNX Runtime is not the one the Runtime pins (1.24.4): the Runtime refuses it for the graphics card, and an install set holds the pinned build only",
                )
                .with(format!("{pinned}: {} bytes {}, pinned {size} bytes {sha256}", file.size, file.sha256)),
            ),
            Some(_) => {}
        }
    }
    for required in [ORT_LICENSE, ORT_NOTICES] {
        if !paths.contains(&required.to_ascii_lowercase()) {
            problems.push(
                Said::new(
                    "视觉文件缺少 ONNX Runtime 的许可文件",
                    "the vision set lacks ONNX Runtime's licence files",
                )
                .with(required),
            );
        }
    }
    if !models {
        problems.push(Said::new("视觉文件里没有模型", "the vision set holds no model"));
    } else if set.vision.licenses.is_empty() {
        problems.push(Said::new(
            "带了识别模型，却没有带模型的许可文件（vision.licenses）",
            "the vision set carries a model without the models' licence files (vision.licenses)",
        ));
    }
    if let Some(directml) = set
        .vision
        .files
        .iter()
        .find(|file| file.path.eq_ignore_ascii_case(DIRECTML_LIBRARY))
    {
        if directml.size != DIRECTML_SIZE || !directml.sha256.eq_ignore_ascii_case(DIRECTML_SHA256) {
            problems.push(
                Said::new(
                    "DirectML.dll 不是 Runtime 钉住的那一版（DirectML 1.15.4）",
                    "DirectML.dll is not the one the Runtime pins (DirectML 1.15.4)",
                )
                .with(format!(
                    "{} bytes {}, pinned {DIRECTML_SIZE} bytes {DIRECTML_SHA256}",
                    directml.size, directml.sha256
                )),
            );
        }
        for required in DIRECTML_LICENSES {
            if !paths.contains(&required.to_ascii_lowercase()) {
                problems.push(
                    Said::new("带了 DirectML，却缺少它的许可文件", "the vision set carries DirectML without its licence files")
                        .with(required),
                );
            }
        }
    }
    let mut licences = BTreeSet::new();
    for licence in &set.vision.licenses {
        if !safe_relative(&licence.path) {
            problems.push(
                Said::new(
                    "视觉模型许可文件的位置不是普通的相对路径",
                    "a vision model's licence file is not placed at a plain relative path",
                )
                .with(&licence.path),
            );
        } else if !licences.insert(licence.path.to_ascii_lowercase()) {
            problems.push(
                Said::new("视觉模型许可文件的位置重复", "a vision model's licence file is placed twice").with(&licence.path),
            );
        }
    }
    if !set.vision.licenses.is_empty() && games.contains(VISION_LICENSES_DIR) {
        problems.push(
            Said::new(
                "有个游戏的文件夹名和视觉模型许可文件的文件夹撞名",
                "a game's folder name is the folder of the vision models' licence files",
            )
            .with(format!("{LICENSES_DIR}/{VISION_LICENSES_DIR}")),
        );
    }
    problems
}

/// What kind of trouble a set check met, for the window to say it in a person's words; the
/// sentence with every detail is [`SetError::said`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetFault {
    /// `install-set.json` cannot be read or breaks a rule of its own, or the set as a whole does
    /// not fit (the Runtime declares no package interface).
    Unusable,
    /// A file of the set (its name in the set folder) is missing, of another size or SHA-256,
    /// or a zip whose contents do not match what it declares.
    Damaged { file: String },
    /// A game's standard package (its display name, else its folder name) speaks a package
    /// interface outside the Runtime's range.
    Mismatch { game: String },
    /// The console and the Runtime of the set do not speak a common revision of an interface
    /// they share (`manifest::console_problems`).
    ConsoleMismatch,
}

/// Why a set check stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetError {
    pub fault: SetFault,
    pub said: Said,
}

impl SetError {
    fn unusable(said: Said) -> Self {
        Self { fault: SetFault::Unusable, said }
    }

    fn damaged(file: &str, said: Said) -> Self {
        Self {
            fault: SetFault::Damaged { file: file.to_owned() },
            said,
        }
    }
}

impl From<SetError> for Said {
    fn from(error: SetError) -> Self {
        error.said
    }
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
    /// The bytes of the vision files and of the vision models' licences.
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
            .map(|bundle| bundle.bytes + bundle.entry.files.iter().map(|file| file.size).sum::<u64>())
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

/// The set file in `dir`, read and parsed but not checked against its files: what the window's
/// first page shows before the check.
pub fn read_set_file(dir: &Path) -> Result<InstallSet, Said> {
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
    parse_install_set(&bytes)
}

/// A game's display name from its standard package zip, read before the set is checked: the
/// `label` of the zip's `applications.json`, one small entry, so the first page can name the
/// game; `None` when it cannot be read, and the folder name stands in. Nothing here is trusted:
/// the check reads the same entry again once the zip's SHA-256 has matched.
pub fn unchecked_label(dir: &Path, entry: &BundleEntry) -> Option<String> {
    if !safe_relative(&entry.file) {
        return None;
    }
    let mut archive = Archive::open(&dir.join(&entry.file)).ok()?;
    let game = parse_applications(&archive.read_small(APPLICATIONS_FILE).ok()?).ok()?;
    (game.game == entry.game).then_some(game.label)
}

/// "3 of 11 files checked", as a person reads it while the files are hashed.
fn files_checked(done: usize, total: usize) -> Said {
    Said::new(format!("已核对 {done}/{total} 个文件"), format!("{done} of {total} files checked"))
}

/// Reads and checks the install set in `dir` (see the module's description); `report` hears a
/// line per file checked (with its path, repository and commit, for the log), the same news in
/// a person's words ([`Event::Plain`], for the window), and the bytes hashed.
pub fn verify_set(dir: &Path, report: &mut dyn FnMut(Event)) -> Result<VerifiedSet, SetError> {
    let set = read_set_file(dir).map_err(SetError::unusable)?;
    let problems = set_problems(&set);
    if !problems.is_empty() {
        let zh: Vec<String> = problems.iter().map(|problem| problem.zh.clone()).collect();
        let en: Vec<String> = problems.iter().map(|problem| problem.en.clone()).collect();
        return Err(SetError::unusable(Said::new(zh.join("；"), en.join("; "))));
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
                SetError::damaged(
                    &file.file,
                    Said::new("安装集缺少文件", "a file of the install set is missing")
                        .with(format!("{what}: {}: {error}", path.display())),
                )
            })?
            .len();
        if size != file.size {
            return Err(SetError::damaged(
                &file.file,
                Said::new("文件大小与安装集不符", "the file size does not match the install set")
                    .with(format!("{what}: {}: expected {}, found {size}", path.display(), file.size)),
            ));
        }
    }
    let total: u64 = files.iter().map(|(_, file)| file.size).sum();
    let mut done = 0u64;
    report(Event::Plain(files_checked(0, files.len())));
    for (index, (what, file)) in files.iter().enumerate() {
        let path = dir.join(&file.file);
        let (sha256, _) = sha256_file(&path, &mut |chunk: u64| {
            done += chunk;
            report(Event::Bytes { done, total });
        })
        .map_err(|error| {
            SetError::damaged(
                &file.file,
                Said::new("文件读不出", "the file cannot be read").with(format!("{}: {error}", path.display())),
            )
        })?;
        if !sha256.eq_ignore_ascii_case(&file.sha256) {
            return Err(SetError::damaged(
                &file.file,
                Said::new("文件的 SHA-256 与安装集不符", "the file's SHA-256 does not match the install set")
                    .with(format!("{what}: {}: expected {}, found {sha256}", path.display(), file.sha256)),
            ));
        }
        report(Event::Line(
            Said::new("已核对", "checked").with(format!("{what}: {} ({} bytes)", file.file, file.size)),
        ));
        report(Event::Plain(files_checked(index + 1, files.len())));
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
        programs.push(verify_program(program, &zip, commit, report).map_err(|said| SetError::damaged(&file.file, said))?);
    }
    let runtime_manifest = &programs[0].manifest;
    let console_manifest = &programs[2].manifest;
    let edges = console_problems(console_manifest.interfaces.as_ref(), runtime_manifest.interfaces.as_ref());
    if !edges.is_empty() {
        let zh: Vec<String> = edges.iter().map(|problem| problem.zh.clone()).collect();
        let en: Vec<String> = edges.iter().map(|problem| problem.en.clone()).collect();
        return Err(SetError {
            fault: SetFault::ConsoleMismatch,
            said: Said::new(zh.join("；"), en.join("; ")).with(format!(
                "console {} {}, Runtime {} {}",
                set.console.tag, set.console.commit, set.runtime.tag, set.runtime.commit
            )),
        });
    }
    report(Event::Line(Said::new(
        "监控台与 Runtime 的接口配套",
        "the console's interfaces match the Runtime's",
    )));
    let package_range = runtime_manifest
        .interfaces
        .as_ref()
        .and_then(|declaration| speaks(declaration, "package"))
        .ok_or_else(|| {
            SetError::unusable(Said::new(
                "Runtime 的 BUILD-MANIFEST.json 没有声明 package 接口",
                "the Runtime's BUILD-MANIFEST.json declares no package interface",
            ))
        })?;

    // Each standard package zip.
    let mut bundles = Vec::new();
    for entry in &set.bundles {
        let zip = dir.join(&entry.file);
        let bundle = verify_bundle(entry, &zip, package_range)?;
        report(Event::Line(
            Said::new("标准包已核对", "standard package checked").with(format!("{} ({})", entry.game, entry.file)),
        ));
        let label = &bundle.game.label;
        report(Event::Plain(Said::new(
            format!("已核对：{label}的游戏资源"),
            format!("Checked: the game resources of {label}"),
        )));
        bundles.push(bundle);
    }
    let vision_bytes = set.vision.files.iter().chain(&set.vision.licenses).map(|file| file.size).sum();
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
    report(Event::Plain(Said::new(
        format!("已核对：{}（{} 个文件）", label.zh, manifest.files.len()),
        format!("Checked: {} ({} files)", label.en, manifest.files.len()),
    )));
    Ok(VerifiedProgram {
        program,
        zip: zip.to_path_buf(),
        manifest,
        bytes,
    })
}

/// One standard package zip: its `bundle.json`, its `applications.json` (the game's name and
/// servers), its `schedule.json` when it carries one (the alias limit), its `interfaces.json`
/// against the Runtime's range, and its installed layout. A package of another range is a
/// [`SetFault::Mismatch`]; anything else wrong with the zip is [`SetFault::Damaged`].
fn verify_bundle(entry: &BundleEntry, zip: &Path, package_range: [u64; 2]) -> Result<VerifiedBundle, SetError> {
    let damaged = |said: Said| SetError::damaged(&entry.file, said.with(format!("{} ({})", entry.game, entry.file)));
    let mut archive = Archive::open(zip).map_err(damaged)?;
    let index = parse_bundle(&archive.read_small(BUNDLE_FILE).map_err(damaged)?).map_err(damaged)?;
    if index.game != entry.game {
        return Err(damaged(
            Said::new("bundle.json 的游戏与安装集不符", "bundle.json names another game than the install set")
                .with(&index.game),
        ));
    }
    let mut game = parse_applications(&archive.read_small(APPLICATIONS_FILE).map_err(damaged)?).map_err(damaged)?;
    if game.game != entry.game {
        return Err(damaged(
            Said::new("applications.json 的游戏与安装集不符", "applications.json names another game than the install set")
                .with(&game.game),
        ));
    }
    if archive.size_of(SCHEDULE_FILE).is_some() {
        let longest = longest_item_id(&archive.read_small(SCHEDULE_FILE).map_err(damaged)?).map_err(damaged)?;
        game.alias_limit = alias_limit(longest);
    }
    let declaration: serde_json::Value = {
        let bytes = archive.read_small(INTERFACES_FILE).map_err(damaged)?;
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes).to_vec();
        serde_json::from_slice(&bytes).map_err(|error| {
            damaged(Said::new("interfaces.json 读不懂", "interfaces.json does not parse").with(error))
        })?
    };
    let package = speaks(&declaration, "package").ok_or_else(|| {
        damaged(Said::new(
            "interfaces.json 没有声明 package 接口",
            "interfaces.json declares no package interface",
        ))
    })?;
    if !package_edge(package, package_range) {
        return Err(SetError {
            fault: SetFault::Mismatch { game: game.label.clone() },
            said: Said::new(
                "标准包的 package 接口不在这个 Runtime 支持的范围内",
                "the standard package's package interface is outside this Runtime's range",
            )
            .with(format!(
                "{} ({}): package [{}, {}], Runtime [{}, {}]",
                entry.game, entry.file, package[0], package[1], package_range[0], package_range[1]
            )),
        });
    }
    for pack in &index.packs {
        if game.server(&pack.server).is_none() {
            return Err(damaged(
                Said::new("包的区服不在 applications.json 里", "a pack's server is not in applications.json")
                    .with(format!("{} → {}", pack.package_id, pack.server)),
            ));
        }
    }
    let layout = plan_layout(&archive.names(), &index).map_err(damaged)?;
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
                 "sha256": sha('d'), "repository": "owner/azurlane-resources", "tag": "v1",
                 "files": [
                    {"path": "pack-files/NOTICE.md", "file": "bundles/azurlane/NOTICE.md", "size": 3, "sha256": sha('e')},
                    {"path": "LICENSES/Upstream/LICENSE", "file": "bundles/azurlane/LICENSE", "size": 4, "sha256": sha('f')}
                 ],
                 "required_files": ["pack-files/NOTICE.md", "LICENSES/Upstream/LICENSE"]}
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
            ], "licenses": [
                {"path": "PaddleOCR/LICENSE", "file": "vision/PaddleOCR-LICENSE", "size": 11376, "sha256": sha('4')}
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
        // Three program zips, one bundle with its two files, seven vision files, the model's
        // licence.
        assert_eq!(set.files().len(), 14);
        assert_eq!(set.bundles[0].required_files.len(), 2);
        let files = set.files();
        let (what, file) = &files[4];
        assert_eq!(what, "standard package azurlane file pack-files/NOTICE.md");
        assert_eq!((file.file.as_str(), file.size), ("bundles/azurlane/NOTICE.md", 3));
        assert_eq!(set.bundles[0].files[1].path, "LICENSES/Upstream/LICENSE");
    }

    /// `good()` with DirectML as the Runtime pins it (Runtime `crates/vision-ffi/src/runtime_pin.rs`)
    /// and its three licence files (sizes and digests of these three are placeholders), and the
    /// OCR model's licence in `vision.licenses`.
    fn with_directml() -> Value {
        let mut set = good();
        let files = set["vision"]["files"].as_array_mut().unwrap();
        files.push(json!({"path": "ort/DirectML.dll", "file": "vision/DirectML.dll", "size": 18527776,
            "sha256": "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1"}));
        for (name, fill) in [("LICENSE.txt", '1'), ("LICENSE-CODE.txt", '2'), ("ThirdPartyNotices.txt", '3')] {
            let path = format!("ort/licenses/directml/{name}");
            let file = format!("vision/directml/{name}");
            files.push(json!({"path": path, "file": file, "size": 100, "sha256": sha(fill)}));
        }
        set["vision"]["licenses"] = json!([
            {"path": "PaddleOCR/LICENSE", "file": "vision/PaddleOCR-LICENSE", "size": 11357, "sha256": sha('4')}
        ]);
        set
    }

    #[test]
    fn directml_with_its_licences_and_the_model_licence_are_taken() {
        assert_eq!(problems(with_directml()), Vec::<Said>::new());
        let set = parse_install_set(&serde_json::to_vec(&with_directml()).unwrap()).unwrap();
        assert!(set.carries_directml());
        assert_eq!(set.vision.provider, VISION_PROVIDER);
        // 14 of good() (its model licence replaced by another), DirectML and its three licence
        // files.
        let files = set.files();
        assert_eq!(files.len(), 18);
        let (what, file) = files.last().unwrap();
        assert_eq!(what, "vision licence PaddleOCR/LICENSE");
        assert_eq!((file.file.as_str(), file.size), ("vision/PaddleOCR-LICENSE", 11357));
        assert_eq!(set.vision.licenses[0].path, "PaddleOCR/LICENSE");
        // Without DirectML, the set reads as before.
        let plain = parse_install_set(&serde_json::to_vec(&good()).unwrap()).unwrap();
        assert!(!plain.carries_directml());
        // The file name is compared as Windows compares names.
        let mut other_case = with_directml();
        other_case["vision"]["files"][7]["path"] = json!("ort/directml.DLL");
        let other_case = parse_install_set(&serde_json::to_vec(&other_case).unwrap()).unwrap();
        assert!(other_case.carries_directml());
        assert_eq!(set_problems(&other_case), Vec::<Said>::new());
    }

    /// Where `with_directml()` lists DirectML among the vision files.
    fn directml_at(set: &Value) -> usize {
        let files = set["vision"]["files"].as_array().unwrap();
        files.iter().position(|file| file["path"] == json!("ort/DirectML.dll")).unwrap()
    }

    #[test]
    fn directml_off_its_pin_or_without_a_licence_file_and_bad_model_licences_are_named() {
        let cases = vec![
            case("directml size", |set| {
                let at = directml_at(set);
                set["vision"]["files"][at]["size"] = json!(18527777)
            }),
            case("directml sha256", |set| {
                let at = directml_at(set);
                set["vision"]["files"][at]["sha256"] = json!(sha('a'))
            }),
            case("no directml licence", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| file["path"] != json!("ort/licenses/directml/LICENSE.txt"));
            }),
            case("no directml code licence", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| file["path"] != json!("ort/licenses/directml/LICENSE-CODE.txt"));
            }),
            case("no directml notices", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| file["path"] != json!("ort/licenses/directml/ThirdPartyNotices.txt"));
            }),
            case("licence escaping", |set| set["vision"]["licenses"][0]["path"] = json!("../LICENSE")),
            case("licence absolute", |set| set["vision"]["licenses"][0]["path"] = json!("C:/LICENSE")),
            case("licence rooted", |set| set["vision"]["licenses"][0]["path"] = json!("/LICENSE")),
            case("licence backslash", |set| set["vision"]["licenses"][0]["path"] = json!("PaddleOCR\\LICENSE")),
            case("licence empty", |set| set["vision"]["licenses"][0]["path"] = json!("")),
            case("licence empty segment", |set| set["vision"]["licenses"][0]["path"] = json!("PaddleOCR//LICENSE")),
            case("licence twice", |set| {
                let copy = json!({"path": "paddleocr/license", "file": "vision/other", "size": 1, "sha256": sha('5')});
                set["vision"]["licenses"].as_array_mut().unwrap().push(copy);
            }),
            case("licence outside the set", |set| set["vision"]["licenses"][0]["file"] = json!("../LICENSE")),
            case("licence sha256", |set| set["vision"]["licenses"][0]["sha256"] = json!("4")),
            case("licence size", |set| set["vision"]["licenses"][0]["size"] = json!(0)),
            case("a game in the licences' folder", |set| set["bundles"][0]["game"] = json!("Vision")),
        ];
        for (name, change) in cases {
            let mut set = with_directml();
            change(&mut set);
            assert!(!problems(set).is_empty(), "{name}");
        }
        // A DirectML licence file alone, without DirectML, is only a file below ort/.
        let mut alone = good();
        alone["vision"]["files"].as_array_mut().unwrap().push(json!(
            {"path": "ort/licenses/directml/LICENSE.txt", "file": "vision/directml/LICENSE.txt", "size": 1, "sha256": sha('6')}
        ));
        assert_eq!(problems(alone), Vec::<Said>::new());
    }

    #[test]
    fn a_bundle_without_files_reads_with_none() {
        let mut bare = good();
        let bundle = bare["bundles"][0].as_object_mut().unwrap();
        bundle.remove("files");
        bundle.remove("required_files");
        let set = parse_install_set(&serde_json::to_vec(&bare).unwrap()).expect("a set");
        assert!(set.bundles[0].files.is_empty() && set.bundles[0].required_files.is_empty());
        assert_eq!(set_problems(&set), Vec::<Said>::new());
        assert_eq!(set.files().len(), 12);
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
            // Both ONNX Runtime files, each exactly as the Runtime pins it.
            case("ort size", |set| set["vision"]["files"][0]["size"] = json!(17328153)),
            case("ort sha256", |set| set["vision"]["files"][0]["sha256"] = json!(sha('a'))),
            case("no providers_shared", |set| set["vision"]["files"][1]["path"] = json!("ort/other_shared.dll")),
            case("providers_shared sha256", |set| set["vision"]["files"][1]["sha256"] = json!(sha('b'))),
            case("no model", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| !file["path"].as_str().unwrap().starts_with("models/"));
            }),
            case("no ort licence", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| file["path"] != json!("ort/licenses/onnxruntime/LICENSE"));
            }),
            case("no ort notices", |set| {
                let files = set["vision"]["files"].as_array_mut().unwrap();
                files.retain(|file| file["path"] != json!("ort/licenses/onnxruntime/ThirdPartyNotices.txt"));
            }),
            case("bundle file escaping", |set| set["bundles"][0]["files"][0]["path"] = json!("../NOTICE.md")),
            case("bundle file absolute", |set| set["bundles"][0]["files"][0]["path"] = json!("C:/NOTICE.md")),
            case("bundle file rooted", |set| set["bundles"][0]["files"][0]["path"] = json!("/NOTICE.md")),
            case("bundle file backslash", |set| set["bundles"][0]["files"][0]["path"] = json!("pack-files\\NOTICE.md")),
            case("bundle file empty", |set| set["bundles"][0]["files"][0]["path"] = json!("")),
            case("bundle file twice", |set| {
                set["bundles"][0]["files"][1]["path"] = json!("Pack-Files/notice.md")
            }),
            case("bundle file outside the set", |set| {
                set["bundles"][0]["files"][1]["file"] = json!("../LICENSE")
            }),
            case("bundle file sha256", |set| set["bundles"][0]["files"][1]["sha256"] = json!("f")),
            case("bundle file size", |set| set["bundles"][0]["files"][1]["size"] = json!(0)),
            // A licence the package must carry, left out of the set; one named but not listed.
            case("required file left out", |set| {
                set["bundles"][0]["files"].as_array_mut().unwrap().remove(1);
            }),
            case("required file not listed", |set| {
                set["bundles"][0]["required_files"].as_array_mut().unwrap().push(json!("LICENSES/Other/LICENSE"));
            }),
            case("no model licence", |set| {
                set["vision"].as_object_mut().unwrap().remove("licenses");
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
