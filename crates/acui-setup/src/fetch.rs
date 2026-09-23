// SPDX-License-Identifier: GPL-3.0-only
//! Step 1 online: the release to install, taken from the umbrella repository's
//! Releases over HTTPS, and its files fetched into `<root>\downloads\<tag>\`.
//! The newest stable release when there is one, else the newest pre-release
//! (the daily builds). Only `SHA256SUMS`, `MEMBERS.json` and what `SHA256SUMS`
//! lists are fetched; step 2 then verifies that folder exactly as it verifies
//! one a person filled by hand. This is the program's only network code.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::verify::{self, Report};

/// The newest stable release, as GitHub states it: 404 while there is none.
const LATEST: &str = "https://api.github.com/repos/HS7097/ActingCommand/releases/latest";
/// Newest first; the newest pre-release is on the first page whenever there
/// is no stable release at all.
const RELEASES: &str = "https://api.github.com/repos/HS7097/ActingCommand/releases?per_page=30";
const USER_AGENT: &str = concat!("acsetup/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
}

impl Release {
    /// The bytes the release's files add up to, as it states them.
    pub fn size(&self) -> u64 {
        self.assets.iter().fold(0, |sum, asset| sum.saturating_add(asset.size))
    }

    /// The tag as a folder name under `downloads\`.
    pub fn folder(&self) -> Result<&str, String> {
        match plain(&self.tag_name) {
            true => Ok(&self.tag_name),
            false => Err(format!(
                "发布件标签不能用作文件夹名 / The release tag cannot name a folder: {}",
                self.tag_name
            )),
        }
    }
}

/// A name the wizard writes under `downloads\`: letters, digits, `.`, `-` and
/// `_` only, not starting with a dot, and no Windows device name.
fn plain(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    !name.is_empty()
        && !name.starts_with('.')
        && !device
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// HTTPS only, redirects included. A connection that goes quiet for a minute
/// fails; there is no overall limit, which in ureq 2 would replace that one.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .https_only(true)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(USER_AGENT)
        .build()
}

fn listing_failed(error: &dyn std::fmt::Display) -> String {
    format!("查询伞仓发布件失败 / Listing the umbrella releases failed: {error}")
}

fn read_json<T: DeserializeOwned>(response: ureq::Response) -> Result<T, String> {
    let text = response.into_string().map_err(|error| listing_failed(&error))?;
    serde_json::from_str(&text).map_err(|error| listing_failed(&error))
}

/// The release to install: the newest stable one when there is one, else the
/// newest pre-release by `published_at`; never a draft.
pub fn choose() -> Result<Release, String> {
    let agent = agent();
    let accept = "application/vnd.github+json";
    match agent.get(LATEST).set("Accept", accept).call() {
        Ok(response) => return read_json(response),
        Err(ureq::Error::Status(404, _)) => {}
        Err(error) => return Err(listing_failed(&error)),
    }
    let response = agent
        .get(RELEASES)
        .set("Accept", accept)
        .call()
        .map_err(|error| listing_failed(&error))?;
    let releases: Vec<Release> = read_json(response)?;
    releases
        .into_iter()
        .filter(|release| !release.draft && release.prerelease)
        .max_by(|a, b| a.published_at.cmp(&b.published_at))
        .ok_or_else(|| "伞仓还没有任何发布件 / The umbrella repository has no release yet".into())
}

/// Fetches `release` into `dir`: `SHA256SUMS` and `MEMBERS.json`, then every
/// other file `SHA256SUMS` lists. Each is written to a `.part` file and renamed
/// once its length is the length the release states; a file already in `dir`
/// is fetched again, never trusted.
pub fn fetch(release: &Release, dir: &Path, report: Report<'_>) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|error| format!("无法创建下载目录 / cannot create {}: {error}", dir.display()))?;
    report(&format!("下载目录 / download folder: {}", dir.display()))?;
    let agent = agent();
    let mut fetched = Vec::new();
    for name in ["SHA256SUMS", "MEMBERS.json"] {
        get(&agent, release, name, dir, report)?;
        fetched.push(name.to_string());
    }
    let sums_path = dir.join("SHA256SUMS");
    let sums = fs::read_to_string(&sums_path)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", sums_path.display()))?;
    for (_, name) in verify::parse_sha256sums(&sums)? {
        if !fetched.contains(&name) {
            get(&agent, release, &name, dir, report)?;
            fetched.push(name);
        }
    }
    report("下载完成 / fetched")
}

/// One file of the release. A failed download removes its `.part` file, and
/// says so when it cannot.
fn get(
    agent: &ureq::Agent,
    release: &Release,
    name: &str,
    dir: &Path,
    report: Report<'_>,
) -> Result<(), String> {
    if !plain(name) {
        return Err(format!("发布件文件名不能使用 / The release file name cannot be used: {name}"));
    }
    let asset = release.assets.iter().find(|asset| asset.name == name).ok_or_else(|| {
        format!("发布件 {} 缺少 / release {} lacks {name}", release.tag_name, release.tag_name)
    })?;
    let part = dir.join(format!("{name}.part"));
    let written = match download(agent, asset, &part, report) {
        Ok(written) => written,
        Err(reason) => {
            return Err(match fs::remove_file(&part) {
                Ok(()) => reason,
                Err(error) if error.kind() == io::ErrorKind::NotFound => reason,
                Err(error) => format!(
                    "{reason}\n未完成的文件未能删除 / unfinished file not removed: {}: {error}",
                    part.display()
                ),
            });
        }
    };
    let path = dir.join(name);
    if let Err(error) = fs::rename(&part, &path) {
        let _ = fs::remove_file(&part);
        return Err(format!("无法改名 / cannot rename {}: {error}", part.display()));
    }
    report(&format!("已下载 / fetched: {name}（{written} 字节 / bytes）"))
}

/// Streams one asset into `part`, a progress line per tenth for a file of a
/// MiB or more, and checks the length against what the release states.
fn download(
    agent: &ureq::Agent,
    asset: &Asset,
    part: &Path,
    report: Report<'_>,
) -> Result<u64, String> {
    let name = &asset.name;
    let failed = |error: &dyn std::fmt::Display| {
        format!("下载失败 / download failed: {name}: {error}")
    };
    let cannot_write = |error: io::Error| {
        format!("无法写入 / cannot write {}: {error}", part.display())
    };
    let response = agent.get(&asset.browser_download_url).call().map_err(|error| failed(&error))?;
    let mut file = File::create(part).map_err(cannot_write)?;
    // One byte past the stated length is enough to know it is too long.
    let mut body = response.into_reader().take(asset.size.saturating_add(1));
    let mut buffer = vec![0; 64 * 1024];
    let (mut written, mut tenth) = (0u64, 0u64);
    loop {
        let read = body.read(&mut buffer).map_err(|error| failed(&error))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(cannot_write)?;
        written += read as u64;
        let now = written.saturating_mul(10) / asset.size.max(1);
        if asset.size >= 1 << 20 && now > tenth && now < 10 {
            tenth = now;
            report(&format!("  {name}: {}%", now * 10))?;
        }
    }
    file.sync_all().map_err(cannot_write)?;
    if written != asset.size {
        return Err(format!(
            "{}: {name} 长度应为 / length should be {}，实为 / is {written}",
            verify::MISMATCH,
            asset.size
        ));
    }
    Ok(written)
}
