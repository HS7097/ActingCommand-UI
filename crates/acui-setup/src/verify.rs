// SPDX-License-Identifier: GPL-3.0-only
//! The install step: the release assets in the download folder, checked against what
//! the umbrella published — SHA256SUMS over the zips and MEMBERS.json, then
//! each zip's own BUILD-MANIFEST.json over every file inside it — and
//! unpacked into a staging directory under the install root.
//!
//! Nothing in a zip is ever run here. A mismatch is reported as content that
//! differs from when it was created, a statement about integrity and never
//! about permission.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The one wording for bytes that are not what their creator recorded.
pub const MISMATCH: &str = "内容与创建时不一致";

pub const MANIFEST: &str = "BUILD-MANIFEST.json";
const RUNTIME_REPOSITORY: &str = "HS7097/ActingCommand-Runtime";
const UI_REPOSITORY: &str = "HS7097/ActingCommand-UI";
const RUNTIME_LAYOUT: &str = "distribution-v1";

/// What the runtime zip must hold for anything to be configured, what the
/// console zip must hold to be opened, and the three tools that are
/// installed — the other two in that zip are neither installed nor shown.
const RUNTIME_REQUIRED: &[&str] = &["actingcommand-actingd.exe"];
const UI_REQUIRED: &[&str] = &["acui.exe"];
pub const TOOLS_INSTALLED: &[&str] = &["actinglab.exe", "actingledger.exe", "ac_fastdeploy_ppocr.dll"];

/// Where a step's account goes: every line into the install log, and — for
/// the page — where the work stands and what the person must see. An error
/// (a log write that failed) stops the run.
pub trait Reporter {
    fn line(&mut self, line: &str) -> Result<(), String>;
    /// Where the work stands: the page only, never the log.
    fn step(&mut self, _step: Step<'_>) -> Result<(), String> {
        Ok(())
    }
    /// A line the person must see: logged, and kept on the page and in the
    /// summary.
    fn warn(&mut self, line: &str) -> Result<(), String> {
        self.line(line)
    }
}

impl<F: FnMut(&str) -> Result<(), String>> Reporter for F {
    fn line(&mut self, line: &str) -> Result<(), String> {
        self(line)
    }
}

pub type Report<'a> = &'a mut dyn Reporter;

/// A phase of the work begins — with its size when it is known — or so much
/// of the current phase is done.
pub enum Step<'a> {
    Phase(&'a str, Option<Total>),
    Done(u64),
}

#[derive(Clone, Copy)]
pub enum Total {
    Bytes(u64),
    Items(u64),
}

pub struct Verified {
    pub staging: PathBuf,
    /// The two commits `MEMBERS.json` names: runtime, then ui.
    pub members: (String, String),
    pub runtime: Staged,
    pub ui: Staged,
    pub tools: Staged,
}

/// One unpacked zip: its directory and the manifest's `files[].path`
/// entries, every one of them checked.
pub struct Staged {
    pub dir: PathBuf,
    pub files: Vec<String>,
}

#[derive(Deserialize)]
struct Members {
    runtime_sha: String,
    ui_sha: String,
}

#[derive(Deserialize)]
struct Manifest {
    repository: String,
    commit_sha: String,
    files: Vec<ManifestFile>,
    #[serde(default)]
    runtime_payload_layout: Option<String>,
}

#[derive(Deserialize)]
struct ManifestFile {
    path: String,
    size_bytes: u64,
    sha256: String,
}

/// What one zip must be.
struct Expect<'a> {
    name: String,
    repository: &'a str,
    sha: &'a str,
    required: &'a [&'a str],
    runtime_layout: bool,
}

pub fn run(download: &Path, staging: &Path, report: Report<'_>) -> Result<Verified, String> {
    // SHA256SUMS: every entry it lists, present and matching.
    let sums_path = download.join("SHA256SUMS");
    let sums_text = fs::read_to_string(&sums_path)
        .map_err(|error| format!("缺少文件 / missing: {}: {error}", sums_path.display()))?;
    let entries = parse_sha256sums(&sums_text)?;
    if entries.is_empty() {
        return Err("SHA256SUMS 没有条目 / SHA256SUMS lists nothing".into());
    }
    let mut listed = BTreeSet::new();
    report.step(Step::Phase(
        "核对下载的文件 / Checking the downloaded files",
        Some(Total::Items(entries.len() as u64)),
    ))?;
    for (expected, name) in &entries {
        let path = download.join(name);
        if !path.is_file() {
            return Err(format!(
                "缺少文件 / missing: {}（SHA256SUMS 列出 / listed in SHA256SUMS）",
                path.display()
            ));
        }
        let actual = sha256_file(&path)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
        if &actual != expected {
            return Err(format!(
                "{MISMATCH}: {name} sha256 应为 / expected {expected}，实为 / actual {actual}"
            ));
        }
        report.line(&format!("SHA256SUMS 已核对 / ok: {name}"))?;
        listed.insert(name.clone());
        report.step(Step::Done(listed.len() as u64))?;
    }

    // MEMBERS.json names the two commits, and so the three zips.
    let members_path = download.join("MEMBERS.json");
    let members_text = fs::read_to_string(&members_path)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", members_path.display()))?;
    let (runtime_sha, ui_sha) = members_of(&members_text)?;
    let members = Members { runtime_sha, ui_sha };
    report.line(&format!(
        "MEMBERS.json: runtime {} · ui {}",
        members.runtime_sha, members.ui_sha
    ))?;

    // Unpack under the install root and check each zip against its own manifest.
    fs::create_dir_all(staging).map_err(|error| {
        format!("无法创建临时目录 / cannot create staging: {}: {error}", staging.display())
    })?;
    report.line(&format!("临时目录 / staging: {}", staging.display()))?;
    report.step(Step::Phase("解压并核对内容 / Unpacking and checking", Some(Total::Items(3))))?;
    let runtime = stage(
        download,
        staging,
        &listed,
        &Expect {
            name: format!("actingcommand-runtime-{}.zip", members.runtime_sha),
            repository: RUNTIME_REPOSITORY,
            sha: &members.runtime_sha,
            required: RUNTIME_REQUIRED,
            runtime_layout: true,
        },
        report,
    )?;
    report.step(Step::Done(1))?;
    let ui = stage(
        download,
        staging,
        &listed,
        &Expect {
            name: format!("acui-windows-{}.zip", members.ui_sha),
            repository: UI_REPOSITORY,
            sha: &members.ui_sha,
            required: UI_REQUIRED,
            runtime_layout: false,
        },
        report,
    )?;
    report.step(Step::Done(2))?;
    let tools = stage(
        download,
        staging,
        &listed,
        &Expect {
            name: format!("actingcommand-tools-{}.zip", members.runtime_sha),
            repository: RUNTIME_REPOSITORY,
            sha: &members.runtime_sha,
            required: TOOLS_INSTALLED,
            runtime_layout: false,
        },
        report,
    )?;
    report.step(Step::Done(3))?;
    report.line("校验通过 / verified")?;
    Ok(Verified {
        staging: staging.to_path_buf(),
        members: (members.runtime_sha, members.ui_sha),
        runtime,
        ui,
        tools,
    })
}

/// The two commits a `MEMBERS.json` names, each 40 lowercase hex characters.
pub fn members_of(text: &str) -> Result<(String, String), String> {
    let members: Members = serde_json::from_str(text)
        .map_err(|error| format!("MEMBERS.json 无法解析 / MEMBERS.json unreadable: {error}"))?;
    for (key, sha) in [("runtime_sha", &members.runtime_sha), ("ui_sha", &members.ui_sha)] {
        if sha.len() != 40 || !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!(
                "MEMBERS.json {key} 不是 40 位小写十六进制 / is not 40 lowercase hex characters: {sha}"
            ));
        }
    }
    Ok((members.runtime_sha, members.ui_sha))
}

/// `<hex>  <name>` or `<hex> *<name>`, as sha256sum writes them; blank lines
/// skipped; a name is a plain file name in the same folder.
pub fn parse_sha256sums(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let malformed = || {
            format!(
                "SHA256SUMS 第 {} 行不是 sha256sum 格式 / line {} is not sha256sum format: {line}",
                index + 1,
                index + 1
            )
        };
        if line.len() < 67 || !line.is_char_boundary(64) {
            return Err(malformed());
        }
        let (hash, rest) = line.split_at(64);
        if !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(malformed());
        }
        let name = rest
            .strip_prefix("  ")
            .or_else(|| rest.strip_prefix(" *"))
            .ok_or_else(malformed)?;
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err(malformed());
        }
        entries.push((hash.to_ascii_lowercase(), name.to_string()));
    }
    Ok(entries)
}

/// One zip: listed in SHA256SUMS (so already hashed), unpacked into its own
/// directory under staging, then checked against its manifest.
fn stage(
    download: &Path,
    staging: &Path,
    listed: &BTreeSet<String>,
    expect: &Expect<'_>,
    report: Report<'_>,
) -> Result<Staged, String> {
    let name = &expect.name;
    if !listed.contains(name) {
        return Err(format!("SHA256SUMS 未列出 / not listed in SHA256SUMS: {name}"));
    }
    let dir = staging.join(name.trim_end_matches(".zip"));
    report.line(&format!("正在解压 / unpacking: {name}"))?;
    let count = extract(&download.join(name), name, &dir)?;
    report.line(&format!("已解压 / unpacked: {name}（{count} 个文件 / files）"))?;
    check_manifest(&dir, expect, report)
}

fn extract(zip_path: &Path, name: &str, dir: &Path) -> Result<usize, String> {
    let file = File::open(zip_path)
        .map_err(|error| format!("打开失败 / open failed: {}: {error}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("{MISMATCH}: {name} 不是可读的 zip / is not a readable zip: {error}"))?;
    fs::create_dir_all(dir)
        .map_err(|error| format!("无法创建目录 / cannot create: {}: {error}", dir.display()))?;
    let mut count = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            format!("{MISMATCH}: {name} 第 {index} 个条目读不出 / entry {index} unreadable: {error}")
        })?;
        let Some(relative) = entry.enclosed_name().map(Path::to_path_buf) else {
            return Err(format!(
                "{MISMATCH}: {name} 内有越出目录的条目 / entry escapes the directory: {}",
                entry.name()
            ));
        };
        let target = dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|error| {
                format!("无法创建目录 / cannot create: {}: {error}", target.display())
            })?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("无法创建目录 / cannot create: {}: {error}", parent.display())
            })?;
        }
        let mut out = File::create(&target)
            .map_err(|error| format!("无法创建文件 / cannot create: {}: {error}", target.display()))?;
        let written = io::copy(&mut entry, &mut out).map_err(|error| {
            format!("{MISMATCH}: {name} 内 {} 解压失败 / inflate failed: {error}", entry.name())
        })?;
        if written != entry.size() {
            return Err(format!(
                "{MISMATCH}: {name} 内 {} 大小应为 / expected {} 字节，实为 / actual {written}",
                entry.name(),
                entry.size()
            ));
        }
        count += 1;
    }
    Ok(count)
}

/// The manifest against the files: repository, commit (the MEMBERS sha), the
/// Runtime's payload layout, then every listed file's size and sha256. No
/// file may be present that the manifest does not bind, and the names the
/// wizard needs must be among the bound ones.
fn check_manifest(dir: &Path, expect: &Expect<'_>, report: Report<'_>) -> Result<Staged, String> {
    let zip = &expect.name;
    let manifest: Manifest = read_json(&dir.join(MANIFEST))?;
    if manifest.repository != expect.repository {
        return Err(format!(
            "{MISMATCH}: {zip} 的 {MANIFEST} repository 应为 / expected {}，实为 / actual {}",
            expect.repository, manifest.repository
        ));
    }
    if manifest.commit_sha != expect.sha {
        return Err(format!(
            "{MISMATCH}: {zip} 的 {MANIFEST} commit_sha 应为 / expected {}，实为 / actual {}",
            expect.sha, manifest.commit_sha
        ));
    }
    if expect.runtime_layout && manifest.runtime_payload_layout.as_deref() != Some(RUNTIME_LAYOUT) {
        return Err(format!(
            "{MISMATCH}: {zip} 的 {MANIFEST} runtime_payload_layout 应为 / expected {RUNTIME_LAYOUT}，实为 / actual {}",
            manifest.runtime_payload_layout.as_deref().unwrap_or("（无 / none）")
        ));
    }
    let mut files = Vec::with_capacity(manifest.files.len());
    for entry in &manifest.files {
        let relative = plain_relative(&entry.path).ok_or_else(|| {
            format!(
                "{MISMATCH}: {zip} 的 {MANIFEST} 列出的路径不合法 / lists an invalid path: {}",
                entry.path
            )
        })?;
        let path = dir.join(relative);
        let size = fs::metadata(&path).map(|meta| meta.len()).map_err(|_| {
            format!("{MISMATCH}: {zip} 缺少清单列出的 / lacks the listed {}", entry.path)
        })?;
        if size != entry.size_bytes {
            return Err(format!(
                "{MISMATCH}: {zip} 内 {} 大小应为 / expected {} 字节，实为 / actual {size}",
                entry.path, entry.size_bytes
            ));
        }
        let actual = sha256_file(&path)
            .map_err(|error| format!("读取失败 / read failed: {}: {error}", path.display()))?;
        if actual != entry.sha256.to_ascii_lowercase() {
            return Err(format!(
                "{MISMATCH}: {zip} 内 {} sha256 应为 / expected {}，实为 / actual {actual}",
                entry.path, entry.sha256
            ));
        }
        report.line(&format!("{zip}: {} 已核对 / ok（{size} 字节 / bytes）", entry.path))?;
        files.push(entry.path.clone());
    }
    let mut present = BTreeSet::new();
    walk(dir, dir, &mut present)?;
    let mut bound: BTreeSet<String> = files.iter().cloned().collect();
    bound.insert(MANIFEST.to_string());
    if let Some(extra) = present.difference(&bound).next() {
        return Err(format!(
            "{MISMATCH}: {zip} 内有清单未列出的文件 / holds a file the manifest does not list: {extra}"
        ));
    }
    for name in expect.required {
        if !files.iter().any(|file| file == name) {
            return Err(format!(
                "缺少文件 / missing: {zip} 的清单未列出 / manifest does not list {name}"
            ));
        }
    }
    Ok(Staged {
        dir: dir.to_path_buf(),
        files,
    })
}

/// A manifest path as the wizard accepts it: relative, `/`-separated, no
/// empty, `.` or `..` component, no separator or drive mark inside one.
fn plain_relative(path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." || component.contains(['\\', ':']) {
            return None;
        }
        out.push(component);
    }
    Some(out)
}

/// Every file under `dir`, as a `/`-separated path relative to `base`.
fn walk(base: &Path, dir: &Path, present: &mut BTreeSet<String>) -> Result<(), String> {
    let unreadable = |error: io::Error| format!("读取目录失败 / read_dir failed: {}: {error}", dir.display());
    for entry in fs::read_dir(dir).map_err(unreadable)? {
        let path = entry.map_err(unreadable)?.path();
        if path.is_dir() {
            walk(base, &path, present)?;
            continue;
        }
        let relative = path.strip_prefix(base).map_err(|_| {
            format!("路径不在目录内 / path outside the directory: {}", path.display())
        })?;
        present.insert(
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("缺少文件 / missing: {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} 解析失败 / does not parse: {error}", path.display()))
}

/// Lowercase hex sha256 of a whole file, streamed.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
