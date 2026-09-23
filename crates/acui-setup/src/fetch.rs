// SPDX-License-Identifier: GPL-3.0-only
//! Step 1 online: the release to install, taken from the umbrella repository's
//! Releases over HTTPS, and its files fetched into `<root>\downloads\<tag>\`.
//! The newest stable release when there is one, else the newest pre-release
//! (the daily builds). Only `SHA256SUMS`, `MEMBERS.json` and what `SHA256SUMS`
//! lists are fetched; step 2 then verifies that folder exactly as it verifies
//! one a person filled by hand. This is the program's only network code.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::verify::{self, Report};

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
        self.assets.iter().map(|asset| asset.size).sum()
    }

    /// The tag as a folder name under `downloads\`: letters, digits, `.`,
    /// `-` and `_` only, and not a dot name.
    pub fn folder(&self) -> Result<&str, String> {
        let tag = self.tag_name.as_str();
        let plain = !tag.is_empty()
            && !tag.starts_with('.')
            && tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        match plain {
            true => Ok(tag),
            false => Err(format!("发布件标签不能用作文件夹名 / The release tag cannot name a folder: {tag}")),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(USER_AGENT)
        .build()
}

/// The release to install: the newest stable one when there is one, else the
/// newest pre-release, by `published_at`; never a draft.
pub fn choose() -> Result<Release, String> {
    let failed = |error: &dyn std::fmt::Display| {
        format!("查询伞仓发布件失败 / Listing the umbrella releases failed: {error}")
    };
    let text = agent()
        .get(RELEASES)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| failed(&error))?
        .into_string()
        .map_err(|error| failed(&error))?;
    let releases: Vec<Release> = serde_json::from_str(&text).map_err(|error| failed(&error))?;
    let newest = |stable: bool| {
        releases
            .iter()
            .filter(|release| !release.draft && release.prerelease != stable)
            .max_by(|a, b| a.published_at.cmp(&b.published_at))
            .cloned()
    };
    newest(true)
        .or_else(|| newest(false))
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

/// One file of the release, streamed to disk with a progress line per tenth.
fn get(
    agent: &ureq::Agent,
    release: &Release,
    name: &str,
    dir: &Path,
    report: Report<'_>,
) -> Result<(), String> {
    let asset = release.assets.iter().find(|asset| asset.name == name).ok_or_else(|| {
        format!("发布件 {} 缺少 / release {} lacks {name}", release.tag_name, release.tag_name)
    })?;
    let failed = |error: &dyn std::fmt::Display| {
        format!("下载失败 / download failed: {name}: {error}")
    };
    let response = agent.get(&asset.browser_download_url).call().map_err(|error| failed(&error))?;
    let part = dir.join(format!("{name}.part"));
    let mut file = File::create(&part)
        .map_err(|error| format!("无法写入 / cannot write {}: {error}", part.display()))?;
    // One byte past the stated length is enough to know it is too long.
    let mut body = response.into_reader().take(asset.size + 1);
    let mut buffer = vec![0; 64 * 1024];
    let (mut written, mut tenth) = (0u64, 0u64);
    loop {
        let read = body.read(&mut buffer).map_err(|error| failed(&error))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| format!("无法写入 / cannot write {}: {error}", part.display()))?;
        written += read as u64;
        let now = written * 10 / asset.size.max(1);
        if asset.size >= 1 << 20 && now > tenth && now < 10 {
            tenth = now;
            report(&format!("  {name}: {}%", now * 10))?;
        }
    }
    file.sync_all()
        .map_err(|error| format!("无法写入 / cannot write {}: {error}", part.display()))?;
    drop(file);
    if written != asset.size {
        return Err(format!(
            "{}: {name} 长度应为 / length should be {}，实为 / is {written}",
            verify::MISMATCH,
            asset.size
        ));
    }
    let path = dir.join(name);
    fs::rename(&part, &path)
        .map_err(|error| format!("无法改名 / cannot rename {}: {error}", part.display()))?;
    report(&format!("已下载 / fetched: {name}（{written} 字节 / bytes）"))
}
